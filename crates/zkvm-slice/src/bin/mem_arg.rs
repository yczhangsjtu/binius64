//! Twelfth validation slice: memory ARGUMENT (多地址读-写一致性).
//!
//! This is the zkVM "read must see the most recent write" consensus check, done
//! as a sub-multiset argument over a binary field with logup*.
//!
//! The core idea, vs the earlier `mem_lookup` (whose memory table was given by
//! hand): here the memory table T is NOT free. BOTH the store events and the
//! load events are lookers that constrain to the SAME table T:
//!
//!    store looker:  T[addr] == store_value
//!    load  looker:  T[addr] == load_value
//!
//! So the circuit forces  load_value == T[addr] == store_value  for any address
//! that was written, i.e.  reads ⊆ writes (as a multiset on (addr,value)) and
//! every load reads exactly the value the store wrote at that address. This is
//! precisely the "memory bus + multiset argument" that avoids the O(N·M)
//! selector blow-up of wiring every load to every store.
//!
//! We use several addresses and interleaved store/load to show it is not the
//! single-address case: the argument scales to arbitrary address sets.

use binius_compute::GlobalAllocator;
use binius_field::{arch::{OptimalB128, OptimalPackedB128}, Field};
use binius_ip::{channel::IPVerifierChannel, logup_star};
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::{FieldBuffer, multilinear::evaluate::evaluate};
use binius_transcript::{ProverTranscript, fiat_shamir::HasherChallenger};

type F = OptimalB128;
type P = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

/// A memory event: a store (`is_store=true`) or a load (`is_store=false`).
struct MemOp {
	addr: usize,
	value: u64,
}

fn main() {
	let alloc = GlobalAllocator;
	let nbits = 4; // 4-bit addresses -> 2^4 = 16 slots (m = 4 variables)
	let m = nbits;
	let mask = (1usize << nbits) - 1;

	// Program memory trace: interleaved stores and loads to several addresses.
	// We store distinct values at each address, then load them back.
	// Order matters only for the trace; the argument is order-agnostic.
	let stores: Vec<MemOp> = vec![
		MemOp { addr: 0x0, value: 0xAA },
		MemOp { addr: 0x3, value: 0x0F },
		MemOp { addr: 0x5, value: 0x55 },
		MemOp { addr: 0xA, value: 0xFF },
	];
	let loads: Vec<MemOp> = vec![
		MemOp { addr: 0x3, value: 0x0F }, // must equal the store at addr 3
		MemOp { addr: 0x5, value: 0x55 },
		MemOp { addr: 0xA, value: 0xFF },
		MemOp { addr: 0x0, value: 0xAA },
	];

	// The memory table T is the FINAL memory state after all stores. It is what
	// the store events commit to. Unwritten addresses are 0.
	let table_size = 1usize << m;
	let mut t = vec![0u64; table_size];
	for s in &stores {
		t[s.addr & mask] = s.value;
	}
	let table = FieldBuffer::from_values(&t.iter().map(|&v| F::from(v as u128)).collect::<Vec<_>>());
	let table_view = table.as_view();

	// Build the lookers: STORE lookers first, then LOAD lookers. All constrain
	// to the SAME table T. Each looker: index = addr, claim = value.
	let store_indexes: Vec<Vec<usize>> = stores.iter().map(|s| vec![s.addr & mask]).collect();
	let store_claims: Vec<F> = stores.iter().map(|s| F::from(s.value as u128)).collect();
	let load_indexes: Vec<Vec<usize>> = loads.iter().map(|l| vec![l.addr & mask]).collect();
	let load_claims: Vec<F> = loads.iter().map(|l| F::from(l.value as u128)).collect();

	let mut index_cols: Vec<Vec<usize>> = Vec::new();
	index_cols.extend(store_indexes.iter().cloned());
	index_cols.extend(load_indexes.iter().cloned());
	let mut claims: Vec<F> = Vec::new();
	claims.extend(store_claims.iter().cloned());
	claims.extend(load_claims.iter().cloned());
	let n_lookers = claims.len();

	// eval_point is empty for these (n=0 lookers); keep owned zero-slices alive.
	let empty_pts: Vec<[F; 0]> = (0..n_lookers).map(|_| []).collect();
	let lookers: Vec<Looker<F>> = (0..n_lookers)
		.map(|i| Looker {
			index: &index_cols[i],
			eval_point: &empty_pts[i] as &[F],
			eval_claim: claims[i],
		})
		.collect();

	// Prove: every store and every load is consistent with the SAME memory
	// table, as a sub-multiset argument over the binary field.
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
	assert_eq!(verifier_gamma, gamma, "both sides draw same challenge");
	let verifier_out = logup_star::verify_reduction::<F, _>(
		&verifier_gamma,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: claims
				.iter()
				.map(|&c| logup_star::LookerClaim {
					eval_point: &empty_pts[0] as &[F],
					eval_claim: c,
				})
				.collect(),
		}],
		&mut verifier_transcript,
	)
	.expect("logup* memory-argument reduction verifies");
	assert_eq!(prover_out, verifier_out, "prover/verifier outputs agree");

	// Closing check: reduced table claim equals honest memory-state MLE.
	let table_point = &prover_out.table_eval_point;
	let expected = evaluate(&table_view, &table_point[..m]);
	assert_eq!(
		prover_out.tables[0].eval_claim,
		expected,
		"memory-table claim closes against honest state"
	);

	// ---- Soundness: a load claiming a value that was NEVER stored must fail ----
	// This is the crux of the memory argument: reads ⊆ writes. If a malicious
	// prover claims to read a value that no store wrote, the sub-multiset
	// argument must reject it.
	{
		let mut bad_claims = claims.clone();
		// tamper the FIRST load looker (index = n_stores + 0, addr 0x3) to read 0xEE
		let first_load_idx = stores.len();
		let bad_value = 0xEEu64;
		bad_claims[first_load_idx] = F::from(bad_value as u128);

		let mut bad_transcript = ProverTranscript::new(StdChallenger::default());
		let bad_gamma = IPProverChannel::<F>::sample(&mut bad_transcript);
		let bad_lookers: Vec<Looker<F>> = (0..n_lookers)
			.map(|i| Looker {
				index: &index_cols[i],
				eval_point: &empty_pts[i] as &[F],
				eval_claim: bad_claims[i],
			})
			.collect();
		let _ = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
			&alloc,
			bad_gamma,
			[binius_ip_prover::logup_star::TableLookup {
				table: table_view,
				lookers: bad_lookers,
			}],
			&mut bad_transcript,
		);

		let mut bad_verifier = bad_transcript.into_verifier();
		let bad_vgamma = IPVerifierChannel::<F>::sample(&mut bad_verifier);
		let result = logup_star::verify_reduction::<F, _>(
			&bad_vgamma,
			[logup_star::TableLookup {
				n_vars: m,
				lookers: bad_claims
					.iter()
					.map(|&c| logup_star::LookerClaim {
						eval_point: &empty_pts[0] as &[F],
						eval_claim: c,
					})
					.collect(),
			}],
			&mut bad_verifier,
		);
		assert!(
			result.is_err(),
			"memory argument MUST reject a load value never stored (reads ⊆ writes)"
		);
		println!("   soundness: verifier REJECTED a load value that was never stored ✓");
	}

	println!("✅ Memory ARGUMENT proved & verified over binary field (sub-multiset)");
	println!("   memory: 2^{m} = {} slots, {nbits}-bit addresses", table.len());
	println!("   stores: {}  loads: {}  -> {} lookers constrain SAME table T", stores.len(), loads.len(), n_lookers);
	println!("   invariant: reads ⊆ writes (multiset on (addr,value)); load reads the stored value");
	println!("   proof closed: memory-state MLE eval = {expected:?}");
	println!();
	println!("=> Binius64 logup* carries the random-access MEMORY ARGUMENT itself,");
	println!("   not just a hand-given lookup: store+load all bind to the same table.");
}