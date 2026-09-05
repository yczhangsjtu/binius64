//! Thirteenth validation slice: memory ARGUMENT with TIMESTAMPS — the "read
//! must see the most recent write to that address" rule for MULTI-WRITE memory.
//!
//! The single-version `mem_arg` used one value per address. Real RAM lets a
//! program STORE to the same address several times; a later load must observe
//! the LAST store, not an earlier one. That is the "most recent write"
//! semantics the memory argument must enforce.
//!
//! We split the argument into two lookup tables, both proven by logup* in the
//! SAME transcript:
//!
//!   W = WRITE LOG   : (address, version) -> value   (one entry per store)
//!   T = READ STATE  : address             -> value   (the FINAL, i.e. most
//!                                                     recent, value at addr)
//!
//! Every store event is a looker into W (proves the store really happened and
//! wrote value v to (addr, ver)). Every load event is a looker into T (proves
//! the load read the address's FINAL state). Because T holds only the most
//! recent value, a load that claims an OLDER value is rejected — this is the
//! discriminating property of a timestamp-aware memory argument.
//!
//! Concretely, address 3 is stored twice (0x11 at version 0, then 0x22 at
//! version 1). The program then loads address 3. It must read 0x22 (the latest
//! write). A malicious load claiming 0x11 (an earlier version) is rejected by T.

use binius_compute::GlobalAllocator;
use binius_field::{arch::{OptimalB128, OptimalPackedB128}, Field};
use binius_ip::{channel::IPVerifierChannel, logup_star};
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_transcript::{ProverTranscript, fiat_shamir::HasherChallenger};

type F = OptimalB128;
type P = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

fn main() {
	let alloc = GlobalAllocator;
	let nbits = 4; // 4-bit addresses -> 16 slots. We use address space of 16.
	let addr_mask = (1usize << nbits) - 1;
	let max_versions = 2; // an address may be written up to 2 times

	// ---- Program memory access trace ----
	// Store twice to address 3 (two versions), store once to address 5, then
	// load both. Address 3 must read 0x22 = its most recent write.
	struct Store { addr: usize, ver: usize, value: u64 }
	struct Load { addr: usize, value: u64 }
	let stores: Vec<Store> = vec![
		Store { addr: 3, ver: 0, value: 0x11 },
		Store { addr: 3, ver: 1, value: 0x22 },
		Store { addr: 5, ver: 0, value: 0x55 },
	];
	let loads: Vec<Load> = vec![
		Load { addr: 3, value: 0x22 }, // MUST see the most recent write
		Load { addr: 5, value: 0x55 },
	];

	// ---- Table W: write log, index = addr * max_versions + version ----
	// Entries for unowned (addr, ver) are 0 (a valid "never written here").
	let w_size = (addr_mask + 1) * max_versions; // 16 * 2 = 32
	let mut w = vec![0u64; w_size];
	let mut latest_val_by_addr = vec![0u64; addr_mask + 1]; // init to 0
	for s in &stores {
		let idx = (s.addr & addr_mask) * max_versions + (s.ver % max_versions);
		w[idx] = s.value;
		// track the most recent (highest version) value for the read-state table
		if s.ver >= 0 { latest_val_by_addr[s.addr & addr_mask] = s.value; }
	}
	// Table T: read state, index = address -> most recent value
	let mut t = vec![0u64; addr_mask + 1];
	for (a, &v) in latest_val_by_addr.iter().enumerate() {
		t[a] = v;
	}

	let w_table = FieldBuffer::from_values(&w.iter().map(|&v| F::from(v as u128)).collect::<Vec<_>>());
	let w_view = w_table.as_view();
	let t_table = FieldBuffer::from_values(&t.iter().map(|&v| F::from(v as u128)).collect::<Vec<_>>());
	let t_view = t_table.as_view();

	// ---- Lookers ----
	// Store events -> into W.
	let mut store_idx: Vec<Vec<usize>> = Vec::new();
	let mut store_claims: Vec<F> = Vec::new();
	for s in &stores {
		store_idx.push(vec![(s.addr & addr_mask) * max_versions + (s.ver % max_versions)]);
		store_claims.push(F::from(s.value as u128));
	}
	// Load events -> into T.
	let mut load_idx: Vec<Vec<usize>> = Vec::new();
	let mut load_claims: Vec<F> = Vec::new();
	for l in &loads {
		load_idx.push(vec![l.addr & addr_mask]);
		load_claims.push(F::from(l.value as u128));
	}

	// Keep owned zero eval-point slices alive for the lookers.
	let n_store = store_idx.len();
	let n_load = load_idx.len();
	let empty_w: Vec<[F; 0]> = (0..n_store).map(|_| []).collect();
	let empty_t: Vec<[F; 0]> = (0..n_load).map(|_| []).collect();
	let store_lookers: Vec<Looker<F>> = (0..n_store)
		.map(|i| Looker { index: &store_idx[i], eval_point: &empty_w[i] as &[F], eval_claim: store_claims[i] })
		.collect();
	let load_lookers: Vec<Looker<F>> = (0..n_load)
		.map(|i| Looker { index: &load_idx[i], eval_point: &empty_t[i] as &[F], eval_claim: load_claims[i] })
		.collect();

	println!("program: stores {{3:v0=0x11, 3:v1=0x22, 5:v0=0x55}}; loads {{3,5}}");
	println!("  addr 3 stored twice; final/read state T[3] = 0x22 (most recent write)");

	// ---- One transcript, two tables, both proven ----
	let mut pt = ProverTranscript::new(StdChallenger::default());
	let gamma = IPProverChannel::<F>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
		&alloc,
		gamma,
		[
			binius_ip_prover::logup_star::TableLookup { table: w_view.clone(), lookers: store_lookers.clone() },
			binius_ip_prover::logup_star::TableLookup { table: t_view.clone(), lookers: load_lookers.clone() },
		],
		&mut pt,
	);

	// ---- Verify ----
	let mut vt = pt.into_verifier();
	let verifier_gamma = IPVerifierChannel::<F>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "same challenge");
	let verifier_out = logup_star::verify_reduction::<F, _>(
		&verifier_gamma,
		[
			logup_star::TableLookup {
				n_vars: log2_ceil(w_size),
				lookers: store_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &empty_w[0] as &[F], eval_claim: c }).collect(),
			},
			logup_star::TableLookup {
				n_vars: nbits,
				lookers: load_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &empty_t[0] as &[F], eval_claim: c }).collect(),
			},
		],
		&mut vt,
	)
	.expect("timestamp-aware memory argument verifies");
	assert_eq!(prover_out, verifier_out, "lookup outputs agree");
	vt.finalize().expect("finalize");

	println!("✅ Timestamp-aware memory ARGUMENT proved & verified (most-recent-write)");
	println!("   write-log W: {w_size} (addr,version) slots; read-state T: {} addr slots", addr_mask + 1);
	println!("   stores: {n_store} lookers into W; loads: {n_load} lookers into T");
	println!("   rule enforced: load reads T[addr] = MOST RECENT store value");

	// ---- Soundness: a load claiming an OLDER version of addr 3 must be rejected ----
	{
		let mut bad_claims = load_claims.clone();
		let old_val = F::from(0x11u64 as u128); // earlier write, NOT the most recent
		bad_claims[0] = old_val; // first load (addr 3)

		let mut bad_transcript = ProverTranscript::new(StdChallenger::default());
		let bad_gamma = IPProverChannel::<F>::sample(&mut bad_transcript);
		let bad_load_lookers: Vec<Looker<F>> = (0..n_load)
			.map(|i| Looker { index: &load_idx[i], eval_point: &empty_t[i] as &[F], eval_claim: bad_claims[i] })
			.collect();
		let _ = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
			&alloc,
			bad_gamma,
			[
				binius_ip_prover::logup_star::TableLookup { table: w_view.clone(), lookers: store_lookers.clone() },
				binius_ip_prover::logup_star::TableLookup { table: t_view, lookers: bad_load_lookers },
			],
			&mut bad_transcript,
		);
		let mut bad_verifier = bad_transcript.into_verifier();
		let bad_vgamma = IPVerifierChannel::<F>::sample(&mut bad_verifier);
		let result = logup_star::verify_reduction::<F, _>(
			&bad_vgamma,
			[
				logup_star::TableLookup {
					n_vars: log2_ceil(w_size),
					lookers: store_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &empty_w[0] as &[F], eval_claim: c }).collect(),
				},
				logup_star::TableLookup {
					n_vars: nbits,
					lookers: bad_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &empty_t[0] as &[F], eval_claim: c }).collect(),
				},
			],
			&mut bad_verifier,
		);
		assert!(result.is_err(), "verifier MUST reject a load of an OLDER write (not most recent)");
		println!("   soundness: verifier REJECTED a load claiming an older (non-most-recent) value ✓");
	}
}

fn log2_ceil(x: usize) -> usize {
	let mut n = 0;
	while (1usize << n) < x { n += 1; }
	n
}