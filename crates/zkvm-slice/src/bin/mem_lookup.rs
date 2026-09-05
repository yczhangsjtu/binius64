//! Second validation slice: use Binius64 logup* to prove a memory-consistency
//! lookup — every `lw` (load word) reads the correct value for its address.
//!
//! We model memory as a "(address -> value)" table T, and each load from the
//! RISC-V trace claims `T[index[i]] = load_result_i`, where `index[i]` is the
//! address. This is the "read address -> latest store value" shape that zkVM
//! memory arguments use. Proving it via binary-field logup* demonstrates the
//! same lookup backend carries the memory-consistency sub-problem too.

use binius_compute::GlobalAllocator;
use binius_field::{
	Field,
	arch::{OptimalB128, OptimalPackedB128},
};
use binius_ip::{channel::IPVerifierChannel, logup_star};
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::{FieldBuffer, multilinear::evaluate::evaluate};
use binius_transcript::{ProverTranscript, fiat_shamir::HasherChallenger};

type F = OptimalB128;
type P = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

/// Build a memory table over `nbits`-address space: index = address,
/// value = the value stored there after the trace's stores.
fn build_memory_table(contents: &[(usize, u32)], nbits: usize) -> FieldBuffer<P> {
	let size = 1usize << nbits;
	let mut values = vec![F::ZERO; size];
	for &(addr, val) in contents {
		values[addr] = F::from(val as u128);
	}
	FieldBuffer::from_values(&values)
}

fn main() {
	let alloc = GlobalAllocator;
	let nbits = 3; // 3-bit addresses -> 2^3 = 8 memory slots (m = 3 variables)
	let m = nbits;

	// Program trace of stores then loads to a 3-bit address space.
	// Memory settled by the stores:
	let memory_contents = [
		(0usize, 0xAAu32),
		(1, 0x0F),
		(2, 0x55),
		(3, 0xFF),
	];
	let table = build_memory_table(&memory_contents, nbits);
	let table_view = table.as_view();

	// Load instructions from the trace: (address, loaded_value). Each load
	// claims the value at its address equals what the memory table says.
	let loads: Vec<(usize, u32)> = vec![
		(0, 0xAA),
		(2, 0x55),
		(3, 0xFF),
		(1, 0x0F),
	];

	// Each load is one looker (n=0, claimed at empty point), reading the
	// address -> value table.
	let indexes: Vec<Vec<usize>> = loads.iter().map(|&(addr, _)| vec![addr]).collect();
	let claims: Vec<F> = loads
		.iter()
		.map(|&(_, val)| F::from(val as u128))
		.collect();

	let lookers: Vec<Looker<F>> = indexes
		.iter()
		.zip(&claims)
		.map(|(idx, &claim)| Looker {
			index: idx,
			eval_point: &[],
			eval_claim: claim,
		})
		.collect();

	// Prove: every load reads the value the memory table holds for its address.
	let mut prover_transcript = ProverTranscript::new(StdChallenger::default());
	let gamma = IPProverChannel::<F>::sample(&mut prover_transcript);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
		&alloc,
		gamma,
		[binius_ip_prover::logup_star::TableLookup {
			table: table_view,
			lookers,
		}],
		&mut prover_transcript,
	);

	// Verify.
	let mut verifier_transcript = prover_transcript.into_verifier();
	let verifier_gamma = IPVerifierChannel::<F>::sample(&mut verifier_transcript);
	assert_eq!(verifier_gamma, gamma, "both sides must draw the same challenge");

	let verifier_out = logup_star::verify_reduction::<F, _>(
		&verifier_gamma,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: claims
				.iter()
				.map(|&claim| logup_star::LookerClaim {
					eval_point: &[],
					eval_claim: claim,
				})
				.collect(),
		}],
		&mut verifier_transcript,
	)
	.expect("logup* reduction verifies");

	assert_eq!(prover_out, verifier_out, "prover/verifier outputs agree");

	// ---- Negative test: soundness on the binary field ----
	// A malicious prover claims a WRONG loaded value at address 0. The logup*
	// reduction must reject it.
	{
		let mut wrong_claims = claims.clone();
		wrong_claims[0] = wrong_claims[0] + F::ONE; // false value at addr 0

		let mut bad_transcript = ProverTranscript::new(StdChallenger::default());
		let bad_gamma = IPProverChannel::<F>::sample(&mut bad_transcript);
		let _ = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
			&alloc,
			bad_gamma,
			[binius_ip_prover::logup_star::TableLookup {
				table: table_view,
				lookers: indexes
					.iter()
					.zip(&wrong_claims)
					.map(|(idx, &cl)| Looker {
						index: idx,
						eval_point: &[],
						eval_claim: cl,
					})
					.collect(),
			}],
			&mut bad_transcript,
		);

		let mut bad_verifier = bad_transcript.into_verifier();
		let bad_vgamma = IPVerifierChannel::<F>::sample(&mut bad_verifier);
		let result = logup_star::verify_reduction::<F, _>(
			&bad_vgamma,
			[logup_star::TableLookup {
				n_vars: m,
				lookers: wrong_claims
					.iter()
					.map(|&cl| logup_star::LookerClaim {
						eval_point: &[],
						eval_claim: cl,
					})
					.collect(),
			}],
			&mut bad_verifier,
		);
		assert!(
			result.is_err(),
			"verifier MUST reject a false memory claim (soundness)"
		);
		println!("   soundness: verifier correctly REJECTED a tampered load value ✓");
	}

	// Closing check: reduced memory-table claim equals honest memory MLE.
	let table_point = &prover_out.table_eval_point;
	let expected_table_eval = evaluate(&table_view, &table_point[..m]);
	assert_eq!(
		prover_out.tables[0].eval_claim,
		expected_table_eval,
		"memory-table claim closes against honest memory"
	);

	println!("✅ Memory-consistency lookup proved & verified over binary field");
	println!("   memory: 2^{m} = {} addressable words", table.len());
	println!("   load instructions: {}", loads.len());
	println!("   proof closed: memory-table MLE eval = {expected_table_eval:?}");
	println!();
	println!("=> Binius64 logup* backend carries BOTH instruction-table and");
	println!("   memory-consistency lookups on the binary field.");
}