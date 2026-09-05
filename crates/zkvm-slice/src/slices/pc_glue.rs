//! Third validation slice: R1CS glue for program state-flow, using Binius64's
//! built-in Spartan prover.
//!
//! Proves correct register flow across a short RISC-V instruction sequence.
//! Each instruction is a real `xori` (XOR-immediate): in characteristic-2 the
//! instruction `xori x5,x5,imm` is exactly one linear add `out = in + imm`,
//! which Spartan R1CS carries as a single mul constraint against the constant-1
//! wire. This is the "register-file / ALU state-flow" half of the R1CS glue that
//! Jolt uses to join instruction lookups together.
//!
//! HONEST BOUNDARY: PC *integer* (+4) carry propagation is NOT expressible as a
//! binary-field linear chain (that needs the IMUL/carry unit, a later stage).
//! This slice covers the ALU/register glue only; PC carry arithmetic is deferred.

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

// Program: three consecutive `xori` on register x5, starting from X0.
//   xori x5,x5,1  then  xori x5,x5,3  then  xori x5,x5,5
const X0: u64 = 0xa5a5a5a5a5a5a5a5;
const IMMS: [u64; 3] = [0x1, 0x3, 0x5];

/// One `xori` step: `out = in + imm` (XOR). Keep as `B::Wire` (the generic wire).
fn xori_step<B: CircuitBuilder<Field = B128>>(b: &mut B, input: B::Wire, imm: u64) -> B::Wire {
	let imm_wire = b.constant(B128::new(imm as u128));
	b.add(input, imm_wire)
}

pub fn run_pc_glue() {
	// Allocate the committed register-trace wires ONCE on the constraint side.
	// Edge wires: [initial, after-step-1, after-step-2, after-step-3].
	let mut cb = ConstraintBuilder::new();
	let edge: Vec<_> = (0..=IMMS.len()).map(|_| cb.alloc_inout()).collect();

	// Constrain each step: sum(edge[i], imm) == edge[i+1].
	{
		let mut state = edge[0];
		for (i, &imm) in IMMS.iter().enumerate() {
			let sum = xori_step(&mut cb, state, imm);
			cb.assert_eq(sum, edge[i + 1]);
			state = edge[i + 1];
		}
	}

	let (cs, layout) = compile(cb);

	let log_inv_rate = 1;
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, log_inv_rate).expect("verifier setup");
	let prover = Prover::<OptimalPackedB128, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	// Expected register trace (native execution of the xori chain).
	let expected: Vec<u64> = IMMS
		.iter()
		.scan(X0, |acc, &imm| {
			*acc ^= imm;
			Some(*acc)
		})
		.collect();
	let expected: Vec<u64> = std::iter::once(X0).chain(expected).collect();

	// ---- Witness ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	let mut state_w = wg.write_inout(edge[0], B128::new(expected[0] as u128));
	for (i, &imm) in IMMS.iter().enumerate() {
		let sum = xori_step(&mut wg, state_w, imm);
		let out_w = wg.write_inout(edge[i + 1], B128::new(expected[i + 1] as u128));
		wg.assert_eq(sum, out_w);
		state_w = out_w;
	}
	let witness = wg.build().expect("witness build");
	cs.validate(&witness);

	// ---- Public segment: verifier recomputes derived chain ----
	let mut ig = InstanceGenerator::new(&layout);
	let mut state_i = ig.write_inout(edge[0], B128::new(expected[0] as u128));
	for (i, &imm) in IMMS.iter().enumerate() {
		let sum = xori_step(&mut ig, state_i, imm);
		let out_i = ig.write_inout(edge[i + 1], B128::new(expected[i + 1] as u128));
		ig.assert_eq(sum, out_i);
		state_i = out_i;
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

	let final_val = *expected.last().unwrap();
	println!("✅ R1CS glue proved & verified over binary field (Spartan)");
	println!("   program: 3 × xori on register x5 (start {:#018x})", X0);
	println!("   final x5 = {:#018x}", final_val);
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   public segment (register trace):");
	for (i, &v) in expected.iter().enumerate() {
		println!("     step {i}: x5 = {:#018x}", v);
	}

	// ---- Soundness: tamper a committed register value, must reject ----
	{
		let mut tampered_public = witness.public().to_vec();
		let offset = layout.n_constants();
		tampered_public[offset + 1] += B128::ONE; // corrupt the step-1 register value
		let tampered = Witness::new(
			tampered_public,
			witness.precommit().to_vec(),
			witness.private().to_vec(),
		);
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover
			.prove(&tampered, &mut rng, &mut bt)
			.expect("prove tampered");
		let mut bv = bt.into_verifier();
		let rejected = verifier.verify(&public, &mut bv).is_err();
		assert!(rejected, "verifier MUST reject a tampered register value");
		println!("   soundness: verifier REJECTED a tampered register value ✓");
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn pc_glue() {
		run_pc_glue();
	}
}
