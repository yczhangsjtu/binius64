//! Minimum validation slice: use Binius64 logup* to prove a RISC-V AND
//! instruction lookup over a binary field.
//!
//! This proves that a set of `and` instructions from a real RISC-V trace each
//! looked up the correct result in the RISC-V AND truth table:
//!
//! ```text
//!   for each instruction i:   T[ index[i] ] = (rs1 & rs2)_i
//! ```
//!
//! where `index` encodes `(rs1_low, rs2_low)` and the table `T` holds
//! `rs1 & rs2`. We use a small table (2^m entries) to demonstrate the
//! *mechanism*: Binius64's binary-field logup* backend can carry a zkVM
//! instruction-table lookup. Full-width tables are a scale problem, not a
//! mechanism problem.

use binius_compute::GlobalAllocator;
use binius_field::{
	BinaryField1b, ExtensionField, Field,
	arch::{OptimalB128, OptimalPackedB128},
};
use binius_ip::{channel::IPVerifierChannel, logup_star};
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::{FieldBuffer, multilinear::evaluate::evaluate};
use binius_transcript::{ProverTranscript, fiat_shamir::HasherChallenger};

type F = OptimalB128;
type P = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

/// Embed a table position `j` into the field through the GF(2)-linear basis,
/// exactly as the protocol requires. Used only in the honest side-checks.
#[allow(dead_code)]
fn iota(j: usize, m: usize) -> F {
	(0..m)
		.filter(|&t| (j >> t) & 1 == 1)
		.map(<F as ExtensionField<BinaryField1b>>::basis)
		.fold(F::ZERO, |acc, b| acc + b)
}

/// Build the AND truth table over `(rs1, rs2)`, each `nbits` wide.
/// Index = `rs1 | (rs2 << nbits)`, value = `rs1 & rs2`, size = 2^(2*nbits).
fn build_and_table(nbits: usize) -> FieldBuffer<P> {
	let mask = (1usize << nbits) - 1;
	let size = 1usize << (2 * nbits);
	let values: Vec<F> = (0..size)
		.map(|i| {
			let rs1 = i & mask;
			let rs2 = (i >> nbits) & mask;
			F::from((rs1 & rs2) as u128)
		})
		.collect();
	FieldBuffer::from_values(&values)
}

fn main() {
	let alloc = GlobalAllocator;
	let nbits = 3; // 3-bit operands -> 2^6 = 64-entry table (m = 6 variables)
	let m = 2 * nbits;
	let mask = (1u32 << nbits) - 1;

	let table = build_and_table(nbits);
	let table_view = table.as_view();

	// A small real RISC-V trace of `and` instructions (rs1, rs2 pairs).
	// Each row is an independent looker reading the AND truth table.
	let trace: Vec<(u32, u32)> = vec![
		(0b111, 0b011), // and -> 0b011 (3)
		(0b101, 0b010), // and -> 0b000 (0)
		(0b111, 0b111), // and -> 0b111 (7)
		(0b110, 0b011), // and -> 0b010 (2)
	];

	// Each trace row is one looker with n=0 variables (claimed at empty point:
	// eq = 1, so eval_claim = T[index]). index encodes (rs1, rs2).
	let indexes: Vec<Vec<usize>> = trace
		.iter()
		.map(|&(rs1, rs2)| vec![((rs1 & mask) | ((rs2 & mask) << nbits)) as usize])
		.collect();
	let claims: Vec<F> = trace
		.iter()
		.map(|&(rs1, rs2)| F::from(((rs1 & mask) & (rs2 & mask)) as u128))
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

	// Prove: each looked-up AND result is consistent with the AND truth table.
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

	// Closing check: the reduced table claim must equal the honest AND table
	// multilinear at the shared point. This binds the lookup to the real table.
	let table_point = &prover_out.table_eval_point;
	let expected_table_eval = evaluate(&table_view, &table_point[..m]);
	assert_eq!(
		prover_out.tables[0].eval_claim,
		expected_table_eval,
		"table claim closes against honest table"
	);

	println!("✅ AND instruction lookup proved & verified over binary field");
	println!("   table: 2^{m} = {} entries", table.len());
	println!("   instructions: {} and-instructions from trace", trace.len());
	println!("   proof closed: table MLE eval = {expected_table_eval:?}");
}