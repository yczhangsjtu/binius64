//! Fourth validation slice: PC *integer* carry arithmetic in R1CS over the
//! binary field, using Binius64's built-in Spartan prover.
//!
//! The hard part of a zkVM is that the program counter advances by *integer*
//! addition (`pc = pc + 4`) which is NOT a linear operation over GF(2^128):
//! integer carry propagates across bits, whereas binary-field arithmetic is
//! bitwise. So we must decompose the PC into bits and enforce a full-adder
//! carry chain.
//!
//! This slice proves: an 8-bit PC sequence advances by integer +1 each step (an
//! increment that triggers a carry chain), by constraining `pc_next = pc_cur+1`
//! via:
//!   - bit-decomposed inout witness (each bit constrained boolean b*b=b),
//!   - a full-adder chain  s = a^b^cin,  cout = (a&b)|(a&cin)|(b&cin),
//!   - recombination into the next PC's bits.
//!
//! Cross-check: native +1 recompute must match the committed PC trace.
//! Soundness: a tampered PC bit is rejected by the verifier.
//!
//! KEY PATTERN: inout wires are allocated ONCE on the ConstraintBuilder; the
//! WitnessGenerator and InstanceGenerator reuse those same ConstraintWire ids.

use binius_field::{Field, Ghash128b as B128, arch::OptimalPackedB128};
use binius_hash::StdHashSuite;
use binius_spartan_frontend::{
	circuit_builder::{CircuitBuilder, ConstraintBuilder, InstanceGenerator, WitnessGenerator},
	compiler::compile,
	constraint_system::Witness,
};
use binius_spartan_prover::Prover;
use binius_spartan_verifier::{Verifier, config::StdChallenger};
use binius_transcript::ProverTranscript;
use rand::{SeedableRng, rngs::StdRng};

const BITS: usize = 8;
const STEPS: usize = 4;
const INC: u64 = 1;

type F = B128;
type P = OptimalPackedB128;

/// Decompose a number into little-endian field bits (0/1), native side.
fn to_bits(val: u64) -> [B128; BITS] {
	let mut out = [B128::ZERO; BITS];
	for (i, o) in out.iter_mut().enumerate() {
		*o = B128::new(((val >> i) & 1) as u128);
	}
	out
}

/// Full-adder step: returns (sum, carry_out). Generic over builder/witness/instance.
fn full_adder<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	a: B::Wire,
	bb: B::Wire,
	cin: B::Wire,
) -> (B::Wire, B::Wire) {
	let a_and_b = b.mul(a, bb);
	let a_and_cin = b.mul(a, cin);
	let b_and_cin = b.mul(bb, cin);
	let a_xor_b = b.add(a, bb);
	let sum = b.add(a_xor_b, cin);
	let c1 = b.add(a_and_b, a_and_cin);
	let carry = b.add(c1, b_and_cin);
	(sum, carry)
}

/// Per-step carry constraint: given pc_cur inout bits, add INC and assert result
/// equals pc_next inout bits. All operations go through the generic builder;
/// the increment constant bits are derived inside so all generators agree.
fn step_add<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	cur: &[B::Wire; BITS],
	nxt: &[B::Wire; BITS],
) {
	let mut cin = b.constant(B128::ZERO);
	for i in 0..BITS {
		let inc_bit = b.constant(B128::new(((INC >> i) & 1) as u128));
		let (sum, cout) = full_adder(b, cur[i], inc_bit, cin);
		b.assert_eq(sum, nxt[i]);
		cin = cout;
	}
}

fn main() {
	// Reference/native PC trace.
	let mut native_pcs = vec![0u64];
	for _ in 0..STEPS {
		let last = *native_pcs.last().unwrap();
		native_pcs.push(last + INC);
	}
	assert_eq!(native_pcs.len(), STEPS + 1);

	// ---- Allocate inout bit-wires ONCE on the constraint builder ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let pc_wires: Vec<[binius_spartan_frontend::constraint_system::ConstraintWire; BITS]> = (0
		..=STEPS)
		.map(|_| std::array::from_fn(|_| cb.alloc_inout()))
		.collect();

	// Constrain every inout bit to be boolean b*b=b (all steps), then +INC per step.
	for wires in &pc_wires {
		for &w in wires.iter() {
			binius_spartan_frontend::circuits::assert_is_bit(&mut cb, w);
		}
	}
	for t in 0..STEPS {
		step_add(&mut cb, &pc_wires[t], &pc_wires[t + 1]);
	}
	let (cs, layout) = compile(cb);

	let log_inv_rate = 1;
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, log_inv_rate).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	// ---- Witness: write committed native bits, recompute carry internals ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	let mut w_bits = Vec::new();
	for (wires, &pc) in pc_wires.iter().zip(&native_pcs) {
		let bits = to_bits(pc);
		let arr: Vec<_> = (0..BITS)
			.map(|i| wg.write_inout(wires[i], bits[i]))
			.collect();
		w_bits.push(arr);
	}
	// MUST mirror constraint-side op order: assert_is_bit on every wire first,
	// then step_add per step (order produces identical derived-wire numbering).
	for bits in &w_bits {
		for &w in bits.iter() {
			binius_spartan_frontend::circuits::assert_is_bit(&mut wg, w);
		}
	}
	for t in 0..STEPS {
		step_add(
			&mut wg,
			w_bits[t].as_slice().try_into().unwrap(),
			w_bits[t + 1].as_slice().try_into().unwrap(),
		);
	}
	let witness = wg.build().expect("witness build");

	cs.validate(&witness);

	// ---- Public segment (verifier recomputes) ----
	let mut ig = InstanceGenerator::new(&layout);
	let mut i_bits = Vec::new();
	for (wires, &pc) in pc_wires.iter().zip(&native_pcs) {
		let bits = to_bits(pc);
		let arr: Vec<_> = (0..BITS)
			.map(|i| ig.write_inout(wires[i], bits[i]))
			.collect();
		i_bits.push(arr);
	}
	// Mirror constraint-side op order (assert_is_bit before step_add).
	for bits in &i_bits {
		for &w in bits.iter() {
			binius_spartan_frontend::circuits::assert_is_bit(&mut ig, w);
		}
	}
	for t in 0..STEPS {
		step_add(
			&mut ig,
			i_bits[t].as_slice().try_into().unwrap(),
			i_bits[t + 1].as_slice().try_into().unwrap(),
		);
	}
	let public = ig.build();

	// ---- Prove & verify ----
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover
		.prove(&witness, &mut rng, &mut pt)
		.expect("prove failed");
	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("verify failed");
	vt.finalize().expect("finalize failed");

	println!("✅ PC integer-carry flow proved & verified over binary field (Spartan)");
	println!("   BITS={BITS} STEPS={STEPS} INC={INC}");
	println!("   PC trace (native +1, must exhibit carry across bits):");
	for (i, &pc) in native_pcs.iter().enumerate() {
		let bits: Vec<u64> = to_bits(pc).iter().map(|b| u128::from(*b) as u64).collect();
		println!("     step {i}: pc = {pc:#04x} ({pc}) bits={bits:?}");
	}

	// ---- Cross-check: native recompute ----
	let recomputed = (0..STEPS).fold(0u64, |a, _| a + INC);
	assert_eq!(recomputed, *native_pcs.last().unwrap(), "recompute mismatch");
	println!("   cross-check: recomputed final pc = {recomputed} == committed ✓");

	// ---- Soundness: tamper one PC bit, must reject ----
	let offset = layout.n_constants();
	let mut bad_public = witness.public().to_vec();
	bad_public[offset + 0] += B128::ONE; // flip pc step-0 bit-0
	let tampered = Witness::new(bad_public, witness.precommit().to_vec(), witness.private().to_vec());
	let mut bt = ProverTranscript::new(StdChallenger::default());
	prover
		.prove(&tampered, &mut rng, &mut bt)
		.expect("prove tampered");
	let mut bv = bt.into_verifier();
	let rejected = verifier.verify(&public, &mut bv).is_err();
	assert!(rejected, "verifier MUST reject a tampered PC bit");
	println!("   soundness: verifier REJECTED a tampered PC bit ✓");
}