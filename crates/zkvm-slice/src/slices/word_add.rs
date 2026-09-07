//! M2 spike slice (T2/T4): WORD-LEVEL `add rd, rs1, rs2` (32-bit) built with
//! binius-frontend's `iadd_32` gate, proved/verified by the native Binius64
//! prover — versus a BIT-LEVEL 32-bit full-adder chain in spartan R1CS (the
//! W1 approach every earlier slice used).
//!
//! Why this matters: Jolt's uniform per-instruction cost rests on prime-field
//! integer embedding, which fails wholesale in char-2. The word-level path
//! (W2) prices a 32-bit ADD at 1 AND + 1 ZERO constraint regardless of width
//! pressure, while the bit-level path (W1) prices it at ~3 mul per bit.
//!
//! Soundness discipline: the tamper case flips the PUBLIC rd word handed to
//! the verifier (the honest proof stays untouched); it never pokes internal
//! witness wires, which would fail at witness-build time instead of at verify.

use std::time::Instant;

use binius_core::{constraint_system::ValueVec, word::Word};
use binius_field::{Field, Ghash128b as B128, arch::OptimalPackedB128};
use binius_frontend::{Circuit, CircuitBuilder, CircuitStat, Wire};
use binius_hash::StdHashSuite;
use binius_prover::Prover as WordProver;
use binius_spartan_frontend::{
	circuit_builder::{CircuitBuilder as SpartanCircuitBuilder, ConstraintBuilder, InstanceGenerator, WitnessGenerator},
	compiler::compile,
	constraint_system::ConstraintWire,
};
use binius_spartan_prover::Prover as SpartanProver;
use binius_spartan_verifier::Verifier as SpartanVerifier;
use binius_transcript::ProverTranscript;
use binius_verifier::{Verifier as WordVerifier, config::StdChallenger};
use rand::{SeedableRng, rngs::StdRng};

use crate::alu::*;

/// Build-profile label for the timing printouts.
const BUILD_PROFILE: &str = if cfg!(debug_assertions) { "debug" } else { "release" };

/// A compiled word-level `add rd, rs1, rs2` circuit plus its three inout wires.
///
/// Layout: public inout = [rs1_val | rs2_val | rd_val], each one 64-bit word
/// holding a 32-bit value (upper half zero). rd = (rs1 + rs2) mod 2^32.
pub struct WordAddCircuit {
	pub circuit: Circuit,
	pub rs1: Wire,
	pub rs2: Wire,
	pub rd: Wire,
}

/// Builds the word-level add circuit: one `iadd_32` gate + one assert_eq.
/// Shared with the combined slice (word_add_combined.rs).
pub fn build_word_add_circuit() -> WordAddCircuit {
	let builder = CircuitBuilder::new();
	let rs1 = builder.add_inout();
	let rs2 = builder.add_inout();
	let rd = builder.add_inout();
	let sum = builder.iadd_32(rs1, rs2);
	builder.assert_eq("rd = rs1 + rs2 (mod 2^32)", sum, rd);
	WordAddCircuit { circuit: builder.build(), rs1, rs2, rd }
}

/// Fills the witness for `build_word_add_circuit` from concrete 32-bit operand
/// values; the rd word is derived natively as `(a + b) mod 2^32`.
pub fn fill_word_add_witness(wac: &WordAddCircuit, a: u32, b: u32) -> ValueVec {
	let mut w = wac.circuit.new_witness_filler();
	w[wac.rs1] = Word(a as u64);
	w[wac.rs2] = Word(b as u64);
	w[wac.rd] = Word(a.wrapping_add(b) as u64);
	wac.circuit.populate_wire_witness(&mut w).expect("word-add witness fill");
	w.into_value_vec()
}

pub fn run_word_add() {
	// ---- Concrete instance: add x5, x6, x7 with x6=0xFFFFFFFE, x7=5 (carry wraps) ----
	let a: u32 = 0xFFFF_FFFE;
	let b: u32 = 5;
	let d = a.wrapping_add(b);
	println!("word-level add: rs1={a:#010x} rs2={b:#010x} -> rd={d:#010x} (mod 2^32)");

	// ======= Build + witness =======
	let wac = build_word_add_circuit();
	let stat = CircuitStat::collect(&wac.circuit);
	let witness_vec = fill_word_add_witness(&wac, a, b);
	let cs = wac.circuit.constraint_system().clone();
	cs.verify(&witness_vec).expect("word-add constraints satisfied natively");
	let inout_words = witness_vec.inout().to_vec();

	// ======= Prove / verify (timed) =======
	let t0 = Instant::now();
	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<OptimalPackedB128, StdHashSuite>::setup(verifier.clone())
		.expect("prover setup");
	let setup_ms = t0.elapsed().as_secs_f64() * 1e3;

	let t0 = Instant::now();
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("word-add prove");
	let prove_ms = t0.elapsed().as_secs_f64() * 1e3;

	let t0 = Instant::now();
	let mut vt = pt.into_verifier();
	verifier.verify(&inout_words, &mut vt).expect("word-add verify");
	vt.finalize().expect("finalize");
	let verify_ms = t0.elapsed().as_secs_f64() * 1e3;

	println!("✅ WORD-LEVEL add proved & verified (Binius64 native prover, iadd_32 gate)");
	println!(
		"   constraints: ZERO={} AND={} IMUL={} BMUL={} (gates={}, eval-insns={})",
		stat.n_zero_constraints,
		stat.n_and_constraints,
		stat.n_imul_constraints,
		stat.n_bmul_constraints,
		stat.n_gates,
		stat.n_eval_insn
	);
	println!(
		"   values: const={} inout={} witness={} internal={} | committed trace words={}",
		stat.n_const, stat.n_inout, stat.n_witness, stat.n_internal, stat.committed_allocated
	);
	println!("   timing ({BUILD_PROFILE} build): setup={setup_ms:.1}ms prove={prove_ms:.1}ms verify={verify_ms:.1}ms");

	// ---- Soundness: verifier must reject a tampered PUBLIC rd word ----
	{
		let mut bad_inout = inout_words.clone();
		bad_inout[2] = Word((d as u64).wrapping_add(1)); // claim rd = a+b+1
		let mut pt2 = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness_vec, &mut pt2).expect("prove(2)");
		let mut vt2 = pt2.into_verifier();
		let rejected = verifier.verify(&bad_inout, &mut vt2).is_err();
		assert!(rejected, "verifier MUST reject a tampered public rd word");
		println!("   soundness: verifier REJECTED tampered public rd = {:#010x} ✓", d.wrapping_add(1));
	}

	// ======= Control group: BIT-LEVEL 32-bit full-adder chain (spartan R1CS) =======
	run_word_add_bit_control();
}

/// W1 control: the same `rd = rs1 + rs2 (mod 2^32)` semantics as a 32-bit
/// ripple-carry chain of 1-bit full adders (`alu::fa`) in spartan R1CS.
/// Every operand bit is a public inout wire with a booleanity constraint.
fn run_word_add_bit_control() {
	const BITS: usize = 32;
	let a: u32 = 0xFFFF_FFFE;
	let b: u32 = 5;
	let d = a.wrapping_add(b);

	let mut cb: ConstraintBuilder<B128> = ConstraintBuilder::new();
	let a_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let b_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let d_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();

	fn drive<B: SpartanCircuitBuilder<Field = B128>>(
		b: &mut B,
		a_w: &[B::Wire],
		b_w: &[B::Wire],
		d_w: &[B::Wire],
	) {
		assert_bits(b, a_w);
		assert_bits(b, b_w);
		assert_bits(b, d_w);
		let mut cin = b.constant(B128::ZERO);
		for i in 0..a_w.len() {
			let (sum, cout) = fa(b, a_w[i], b_w[i], cin);
			b.assert_eq(sum, d_w[i]);
			cin = cout;
		}
	}
	drive(&mut cb, &a_w, &b_w, &d_w);
	let (cs_raw, layout) = compile(cb);
	// Compiled (pre-padding) mul count: booleanity + fa chain after wire elimination.
	let n_mul_real = cs_raw.mul_constraints().len();
	let verifier = SpartanVerifier::<_, StdHashSuite>::setup(cs_raw, 1).expect("spartan verifier setup");
	let prover =
		SpartanProver::<OptimalPackedB128, StdHashSuite>::setup(&verifier).expect("spartan prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());
	// Prover-side count: setup wraps the system with blinding dummies and pads
	// to a power of two (spartan-verifier ConstraintSystemPadded).
	let n_mul = cs.mul_constraints().len();

	let av = to_bits(a as u64, BITS);
	let bv = to_bits(b as u64, BITS);
	let dv = to_bits(d as u64, BITS);

	let mut wg = WitnessGenerator::new(&layout);
	for k in 0..BITS {
		wg.write_inout(a_w[k], av[k]);
		wg.write_inout(b_w[k], bv[k]);
		wg.write_inout(d_w[k], dv[k]);
	}
	let wa: Vec<_> = (0..BITS).map(|k| wg.write_inout(a_w[k], av[k])).collect();
	let wb: Vec<_> = (0..BITS).map(|k| wg.write_inout(b_w[k], bv[k])).collect();
	let wd: Vec<_> = (0..BITS).map(|k| wg.write_inout(d_w[k], dv[k])).collect();
	drive(&mut wg, &wa, &wb, &wd);
	let witness = wg.build().expect("spartan witness");
	cs.validate(&witness);

	let mut ig = InstanceGenerator::new(&layout);
	for k in 0..BITS {
		ig.write_inout(a_w[k], av[k]);
		ig.write_inout(b_w[k], bv[k]);
		ig.write_inout(d_w[k], dv[k]);
	}
	let ma: Vec<_> = (0..BITS).map(|k| ig.write_inout(a_w[k], av[k])).collect();
	let mb: Vec<_> = (0..BITS).map(|k| ig.write_inout(b_w[k], bv[k])).collect();
	let md: Vec<_> = (0..BITS).map(|k| ig.write_inout(d_w[k], dv[k])).collect();
	drive(&mut ig, &ma, &mb, &md);
	let public = ig.build();

	let mut rng = StdRng::seed_from_u64(0);
	let t0 = Instant::now();
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness, &mut rng, &mut pt).expect("bit-level prove");
	let prove_ms = t0.elapsed().as_secs_f64() * 1e3;

	let t0 = Instant::now();
	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("bit-level verify");
	vt.finalize().expect("finalize");
	let verify_ms = t0.elapsed().as_secs_f64() * 1e3;

	println!("✅ BIT-LEVEL control proved & verified (spartan R1CS, 32-bit full-adder chain)");
	println!(
		"   constraints: mul={} compiled ({} booleanity + {} fa-chain after wire-elim; naive fa = 3 mul/bit), padded/blinded={}",
		n_mul_real,
		BITS * 3,
		n_mul_real - BITS * 3,
		n_mul,
	);
	println!("   timing ({BUILD_PROFILE} build): prove={prove_ms:.1}ms verify={verify_ms:.1}ms");

	// ---- Soundness: tamper one public sum bit ----
	{
		let bad = dv[0] + B128::ONE;
		let mut ig2 = InstanceGenerator::new(&layout);
		for k in 0..BITS {
			ig2.write_inout(a_w[k], av[k]);
			ig2.write_inout(b_w[k], bv[k]);
			ig2.write_inout(d_w[k], if k == 0 { bad } else { dv[k] });
		}
		let qa: Vec<_> = (0..BITS).map(|k| ig2.write_inout(a_w[k], av[k])).collect();
		let qb: Vec<_> = (0..BITS).map(|k| ig2.write_inout(b_w[k], bv[k])).collect();
		let qd: Vec<_> = (0..BITS).map(|k| ig2.write_inout(d_w[k], if k == 0 { bad } else { dv[k] })).collect();
		drive(&mut ig2, &qa, &qb, &qd);
		let bad_public = ig2.build();
		let mut pt2 = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness, &mut rng, &mut pt2).expect("bit-level prove(2)");
		let mut vt2 = pt2.into_verifier();
		let rejected = verifier.verify(&bad_public, &mut vt2).is_err();
		assert!(rejected, "verifier MUST reject a tampered public sum bit");
		println!("   soundness: verifier REJECTED tampered public sum bit ✓");
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn word_add() {
		run_word_add();
	}
}

