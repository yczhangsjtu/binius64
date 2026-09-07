//! M2 spike slice (T3): WORD-LEVEL `add rd, rs1, rs2` (binius-frontend `iadd_32`)
//! + logup* program-memory instruction lookup in ONE Fiat-Shamir transcript.
//!
//! This is the spike's core question: the frontend prover
//! (`binius_prover::Prover::prove`, crates/prover/src/prove.rs:452) takes the
//! same `ProverTranscript<HasherChallenger<Sha256>>` the spartan+logup* slices
//! use, so the combined.rs pattern ports directly:
//!
//!   1. frontend word-gate proof of `rd = rs1 + rs2 (mod 2^32)`;
//!   2. `IPProverChannel::sample(&mut transcript)` draws the logup* gamma AFTER
//!      the frontend proof, binding the lookup to it by Fiat-Shamir;
//!   3. logup* proves the executed instruction word sits in the program table
//!      at its pc;
//!   4. the verifier mirrors all three steps on one channel and finalizes.

use binius_compute::GlobalAllocator;
use binius_core::word::Word;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_hash::StdHashSuite;
use binius_ip::logup_star;
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_prover::{OptimalPackedB128 as PackedGhash, Prover as WordProver};
use binius_transcript::ProverTranscript;
use binius_verifier::{Verifier as WordVerifier, config::StdChallenger};

use crate::vm32::isa::enc_add;
use crate::word_add::{build_word_add_circuit, fill_word_add_witness};

// The frontend prover commits over Ghash128b; logup* runs over OptimalB128.
// Both are 128-bit binary fields — the same split combined.rs used.
type LF = OptimalB128;
type LP = OptimalPackedB128;

pub fn run_word_add_combined() {
	// ---- Concrete program: `add x5, x6, x7` at pc = 0x00 ----
	let inst_word = enc_add(5, 6, 7);
	let init_pc: u64 = 0x00;
	let x6: u32 = 0xDEAD_BEEF;
	let x7: u32 = 0x1111_1111;
	let x5 = x6.wrapping_add(x7);
	println!("program: add x5,x6,x7 @ pc 0x00 (word {inst_word:#010x})");
	println!("  x6={x6:#010x} x7={x7:#010x} -> x5={x5:#010x} (mod 2^32)");

	// ======= Word-gate layer: build + witness =======
	let wac = build_word_add_circuit();
	let witness_vec = fill_word_add_witness(&wac, x6, x7);
	let cs = wac.circuit.constraint_system().clone();
	cs.verify(&witness_vec).expect("word-add constraints satisfied natively");
	let inout_words = witness_vec.inout().to_vec();

	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover =
		WordProver::<PackedGhash, StdHashSuite>::setup(verifier.clone()).expect("prover setup");

	// ======= logup* layer: program-memory lookup =======
	let m = 4; // 16 addresses
	let table_size = 1usize << m;
	let mut prog = vec![0u64; table_size];
	prog[init_pc as usize & (table_size - 1)] = inst_word;
	let alloc = GlobalAllocator;
	let table_vals: Vec<LF> = prog.iter().map(|&w| LF::from(w as u128)).collect();
	let table = FieldBuffer::from_values(&table_vals);
	let table_view = table.as_view();

	let index: Vec<usize> = vec![init_pc as usize];
	let looker = Looker { index: &index, eval_point: &[], eval_claim: LF::from(inst_word as u128) };

	// ======= ONE combined transcript: frontend prove -> gamma -> logup* prove =======
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend word-add prove");
	let gamma = IPProverChannel::<LF>::sample(&mut pt); // AFTER frontend proof observed
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
		&alloc,
		gamma,
		[binius_ip_prover::logup_star::TableLookup { table: table_view, lookers: vec![looker] }],
		&mut pt,
	);

	// ======= Combined verification (mirror order) =======
	let mut vt = pt.into_verifier();
	verifier.verify(&inout_words, &mut vt).expect("frontend word-add verify");
	let verifier_gamma = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "both sides draw same lookup challenge");
	let verifier_out = logup_star::verify_reduction::<LF, _>(
		&verifier_gamma,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: vec![logup_star::LookerClaim { eval_point: &[], eval_claim: LF::from(inst_word as u128) }],
		}],
		&mut vt,
	)
	.expect("logup* verify");
	assert_eq!(prover_out, verifier_out, "prover/verifier lookup outputs agree");
	vt.finalize().expect("finalize");

	println!("✅ WORD-ADD COMBINED proof: frontend iadd_32 + logup* fetch, ONE transcript");
	println!("   word-gate: x5 = x6 + x7 = {x5:#010x} (iadd_32, 1 AND + 1 ZERO)");
	println!("   logup*:    T[pc=0x{init_pc:x}] = word {inst_word:#010x} found in program table");
	println!("   transcript: ProverTranscript<HasherChallenger<Sha256>> shared by both layers");

	// ---- Soundness 1: tampered PUBLIC rd word (frontend layer must reject) ----
	{
		let mut bad_inout = inout_words.clone();
		bad_inout[2] = Word((x5 as u64).wrapping_add(1));
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness_vec, &mut bt).expect("prove(2)");
		let bg = IPProverChannel::<LF>::sample(&mut bt);
		binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bg,
			[binius_ip_prover::logup_star::TableLookup {
				table: table_view,
				lookers: vec![Looker { index: &index, eval_point: &[], eval_claim: LF::from(inst_word as u128) }],
			}],
			&mut bt,
		);
		let mut bv = bt.into_verifier();
		let rejected = verifier.verify(&bad_inout, &mut bv).is_err();
		assert!(rejected, "verifier MUST reject a tampered public rd word");
		println!("   soundness(1): verifier REJECTED tampered public rd ✓");
	}

	// ---- Soundness 2: tampered instruction word in the lookup (logup* layer must reject) ----
	{
		let bad_word = inst_word.wrapping_add(1);
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness_vec, &mut bt).expect("prove(3)");
		let bg = IPProverChannel::<LF>::sample(&mut bt);
		binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bg,
			[binius_ip_prover::logup_star::TableLookup {
				table: table_view,
				lookers: vec![Looker { index: &index, eval_point: &[], eval_claim: LF::from(bad_word as u128) }],
			}],
			&mut bt,
		);
		let mut bv = bt.into_verifier();
		verifier.verify(&inout_words, &mut bv).expect("prove(3) valid state");
		let bvg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut bv);
		let rejected = logup_star::verify_reduction::<LF, _>(
			&bvg,
			[logup_star::TableLookup {
				n_vars: m,
				lookers: vec![logup_star::LookerClaim { eval_point: &[], eval_claim: LF::from(bad_word as u128) }],
			}],
			&mut bv,
		)
		.is_err();
		assert!(rejected, "verifier MUST reject a wrong program-table lookup");
		println!("   soundness(2): verifier REJECTED tampered instruction word in lookup ✓");
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn word_add_combined() {
		run_word_add_combined();
	}
}
