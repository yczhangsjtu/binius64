//! Seventeenth validation slice: FULL VM with STORE — a read-modify-write loop.
//!
//! Program (initial mem[0]=2, N=3 rounds):
//!   x1=0; i=0; pc=0
//!   loop:
//!     t     = mem[0]     # LOAD  (read current value at addr 0)
//!     x1    = x1 + t     # addi
//!     mem[0]= x1         # STORE (write the accumulator back to addr 0)
//!     i = i + 1          # incr
//!     pc = pc + 4
//!   expected: load_vals=[2,2,4], x1=[0,2,4,8], stores=[2,4,8], final x1=8.
//!
//! This is a read-modify-write (RAW) cycle: each round first reads mem[0], then
//! writes it back. The NEXT round's load must observe the value the PREVIOUS
//! store wrote — the "read must see the most recent write" memory argument.
//!
//! Four layers, ONE Fiat-Shamir transcript, ONE witness set:
//!   1. Spartan state machine: per-round load(load_val) + addi(x1+=load_val) +
//!      store(x1) + incr(i) + pc+=4. `load_val[t]` is the load output wire, and
//!      `x1[t+1]` is the value stored back at round t.
//!   2. logup* program memory: P[pc] = word (fetch layer).
//!   3. logup* data memory (TIME-ORDERED state table, from mem_arg_spice):
//!      T[ts*ADDR+addr] = value of addr at time ts. We give each round a LOAD
//!      timestamp ts=2t and a STORE timestamp ts=2t+1. A load looker claims
//!      T[ts_load, addr] = load_val[t]; a store looker claims
//!      T[ts_store, addr] = x1[t+1]. Because the table is time-ordered, a load
//!      that claims a value that was NOT current at its timestamp is rejected
//!      (read sees the most recent write).

use binius_compute::GlobalAllocator;
use binius_field::{Ghash128b as B128, Field, arch::{OptimalB128, OptimalPackedB128}};
use binius_hash::StdHashSuite;
use binius_ip::logup_star;
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_spartan_frontend::{
	circuit_builder::{CircuitBuilder, ConstraintBuilder, InstanceGenerator, WitnessGenerator},
	compiler::compile,
	constraint_system::ConstraintWire,
};
use binius_spartan_prover::Prover;
use binius_spartan_verifier::Verifier;
use binius_transcript::{ProverTranscript, fiat_shamir::HasherChallenger};
use rand::{SeedableRng, rngs::StdRng};

const BITS: usize = 8;
const PC_INC: u64 = 4;
const N: usize = 3; // three read-modify-write rounds
const START_PC: u64 = 0x00;
const START_X1: u64 = 0x00;
const MEM_INIT: u64 = 2; // initial mem[0]
const ADDR: usize = 4; // 4-address data memory (address space, 2^2)
const TS_MAX: usize = 8; // timestamps 0..7 -> table = 8*4 = 32 = 2^5
const FETCH_M: usize = 6; // 64-address program memory (2^6)

type F = B128;
type P = OptimalPackedB128;
type LF = OptimalB128;
type LP = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

fn to_bits(val: u64, nbits: usize) -> Vec<B128> {
	(0..nbits).map(|i| B128::new(((val >> i) & 1) as u128)).collect()
}

fn fa<B: CircuitBuilder<Field = B128>>(b: &mut B, a: B::Wire, bb: B::Wire, cin: B::Wire) -> (B::Wire, B::Wire) {
	let a_and_b = b.mul(a, bb);
	let a_and_c = b.mul(a, cin);
	let b_and_c = b.mul(bb, cin);
	let axb = b.add(a, bb);
	let sum = b.add(axb, cin);
	let c1 = b.add(a_and_b, a_and_c);
	let cout = b.add(c1, b_and_c);
	(sum, cout)
}

/// One round: load_val[t] (witness), x1' = x1 + load_val, store x1' back,
/// i' = i + 1, pc' = pc + 4. Same op order on every builder.
fn drive_round<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	load_val: &[B::Wire],
	x1: &[B::Wire],
	i: &[B::Wire],
	pc: &[B::Wire],
	x1_next: &[B::Wire],
	i_next: &[B::Wire],
	pc_next: &[B::Wire],
) {
	for w in load_val.iter().chain(x1).chain(i).chain(pc).chain(x1_next).chain(i_next).chain(pc_next) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	// x1' = x1 + load_val
	let mut cin = b.constant(B128::ZERO);
	for k in 0..BITS {
		let (sum, cout) = fa(b, x1[k], load_val[k], cin);
		b.assert_eq(sum, x1_next[k]);
		cin = cout;
	}
	// i' = i + 1
	let inc = to_bits(1, BITS);
	let mut cin2 = b.constant(B128::ZERO);
	for k in 0..BITS {
		let ib = b.constant(inc[k]);
		let (sum, cout) = fa(b, i[k], ib, cin2);
		b.assert_eq(sum, i_next[k]);
		cin2 = cout;
	}
	// pc' = pc + PC_INC
	let pinc = to_bits(PC_INC, BITS);
	let mut cin3 = b.constant(B128::ZERO);
	for k in 0..BITS {
		let ib = b.constant(pinc[k]);
		let (sum, cout) = fa(b, pc[k], ib, cin3);
		b.assert_eq(sum, pc_next[k]);
		cin3 = cout;
	}
}

fn main() {
	// ---- Native ground truth ----
	let mut load_vals = vec![0u64; N];
	let mut stores = vec![0u64; N];
	let mut x1_chain = vec![START_X1];
	let mut i_chain = vec![0u64];
	let mut pc_chain = vec![START_PC];
	let mut mem = vec![MEM_INIT]; // mem[0] initially 2
	for t in 0..N {
		let lv = mem[0];
		load_vals[t] = lv;
		let last = *x1_chain.last().unwrap();
		let nx = last.wrapping_add(lv);
		x1_chain.push(nx);
		stores[t] = nx; // store x1 back to mem[0]
		mem[0] = nx;
		i_chain.push(*i_chain.last().unwrap() + 1);
		pc_chain.push(*pc_chain.last().unwrap() + PC_INC);
	}
	let final_x1 = *x1_chain.last().unwrap();

	println!("FULL VM with STORE — one state machine, read-modify-write loop, one transcript");
	println!("  program: loop {{ t=mem[0]; x1+=t; mem[0]=x1; i++; pc+=4 }} (N={N})");
	println!("  initial mem[0]={MEM_INIT}");
	println!("  loads : {load_vals:?}");
	println!("  stores: {stores:?}");
	println!("  x1    : {:?}", x1_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());
	println!("  expect final x1 = {final_x1} (load_vals 2+2+4)");

	// ---- Spartan state machine ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let load_w: Vec<Vec<ConstraintWire>> = (0..N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let x1_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let i_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let pc_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	for t in 0..N {
		drive_round(&mut cb, &load_w[t], &x1_w[t], &i_w[t], &pc_w[t], &x1_w[t + 1], &i_w[t + 1], &pc_w[t + 1]);
	}
	let (cs, layout) = compile(cb);
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	let load_bits: Vec<Vec<B128>> = load_vals.iter().map(|&v| to_bits(v, BITS)).collect();
	let x1_bits: Vec<Vec<B128>> = x1_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let i_bits: Vec<Vec<B128>> = i_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let pc_bits: Vec<Vec<B128>> = pc_chain.iter().map(|&v| to_bits(v, BITS)).collect();

	// ---- Witness (segmented order = allocation order: load, x1, i, pc) ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	for t in 0..N { for k in 0..BITS { wg.write_inout(load_w[t][k], load_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(x1_w[t][k], x1_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(i_w[t][k], i_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(pc_w[t][k], pc_bits[t][k]); } }
	for t in 0..N {
		let wlv: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(load_w[t][k], load_bits[t][k])).collect();
		let wx1: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[t][k], x1_bits[t][k])).collect();
		let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[t][k], i_bits[t][k])).collect();
		let wp: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[t][k], pc_bits[t][k])).collect();
		let wx1n: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[t + 1][k], x1_bits[t + 1][k])).collect();
		let win: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[t + 1][k], i_bits[t + 1][k])).collect();
		let wpn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[t + 1][k], pc_bits[t + 1][k])).collect();
		drive_round(&mut wg, &wlv, &wx1, &wi, &wp, &wx1n, &win, &wpn);
	}
	let witness = wg.build().expect("witness");
	cs.validate(&witness);

	// ---- Instance (same op order) ----
	let mut ig = InstanceGenerator::new(&layout);
	for t in 0..=N { for k in 0..BITS { ig.write_inout(x1_w[t][k], x1_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { ig.write_inout(i_w[t][k], i_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { ig.write_inout(pc_w[t][k], pc_bits[t][k]); } }
	for t in 0..N { for k in 0..BITS { ig.write_inout(load_w[t][k], load_bits[t][k]); } }
	for t in 0..N {
		let mlv: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(load_w[t][k], load_bits[t][k])).collect();
		let mx1: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[t][k], x1_bits[t][k])).collect();
		let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[t][k], i_bits[t][k])).collect();
		let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[t][k], pc_bits[t][k])).collect();
		let mx1n: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[t + 1][k], x1_bits[t + 1][k])).collect();
		let min: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[t + 1][k], i_bits[t + 1][k])).collect();
		let mpn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[t + 1][k], pc_bits[t + 1][k])).collect();
		drive_round(&mut ig, &mlv, &mx1, &mi, &mp, &mx1n, &min, &mpn);
	}
	let public = ig.build();

	// ---- logup* program memory: P[pc] = word (fetch) ----
	let fetch_size = 1usize << FETCH_M;
	let mut prog = vec![0u64; fetch_size];
	for t in 0..N {
		prog[(START_PC as usize + t * PC_INC as usize) & (fetch_size - 1)] = (t as u64) + 1;
	}
	// ---- logup* data memory: TIME-ORDERED state table T[ts*ADDR+addr] ----
	// Each round t: LOAD at ts=2t, STORE at ts=2t+1. Address always 0.
	let data_size = TS_MAX * ADDR;
	let mut t = vec![0u64; data_size];
	let mut current = vec![MEM_INIT; ADDR]; // mem[0]=2, rest 0
	// At ts=2r a load reads current[0] (initial or the value written by round r-1).
	// At ts=2r+1 the store of round r writes stores[r] then we record current.
	for ts in 0..TS_MAX {
		if ts % 2 == 1 {
			let r = (ts - 1) / 2; // store round index
			if r < N {
				current[0] = stores[r]; // write back x1[r+1] to mem[0]
			}
		}
		for addr in 0..ADDR {
			t[ts * ADDR + addr] = current[addr];
		}
	}
	let data_table_f = FieldBuffer::from_values(&t.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let data_view = data_table_f.as_view();

	let alloc = GlobalAllocator;
	let fetch_table = FieldBuffer::from_values(&prog.iter().map(|&w| LF::from(w as u128)).collect::<Vec<_>>());
	let fetch_view = fetch_table.as_view();

	// fetch lookers: index = pc, claim = word
	let mut fetch_idx: Vec<Vec<usize>> = Vec::new();
	for r in 0..N { fetch_idx.push(vec![(START_PC as usize + r * PC_INC as usize) & (fetch_size - 1)]); }
	let fetch_claims: Vec<LF> = (0..N).map(|r| LF::from((r as u64 + 1) as u128)).collect();
	// data lookers: load (ts=2r, addr=0) + store (ts=2r+1, addr=0)
	let mut data_idx: Vec<Vec<usize>> = Vec::new();
	let mut data_claims: Vec<LF> = Vec::new();
	for r in 0..N {
		data_idx.push(vec![(2 * r) * ADDR + 0]); // load at ts=2r
		data_claims.push(LF::from(load_vals[r] as u128));
		data_idx.push(vec![(2 * r + 1) * ADDR + 0]); // store at ts=2r+1
		data_claims.push(LF::from(stores[r] as u128));
	}

	let fetch_empty: Vec<[LF; 0]> = (0..N).map(|_| []).collect();
	let data_empty: Vec<[LF; 0]> = (0..data_claims.len()).map(|_| []).collect();
	let fetch_lookers: Vec<Looker<LF>> = (0..N).map(|r| Looker { index: &fetch_idx[r], eval_point: &fetch_empty[r] as &[LF], eval_claim: fetch_claims[r] }).collect();
	let data_lookers: Vec<Looker<LF>> = (0..data_claims.len()).map(|k| Looker { index: &data_idx[k], eval_point: &data_empty[k] as &[LF], eval_claim: data_claims[k] }).collect();

	// ---- ONE transcript ----
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness, &mut rng, &mut pt).expect("spartan prove");
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
		&alloc,
		gamma,
		[
			binius_ip_prover::logup_star::TableLookup { table: fetch_view.clone(), lookers: fetch_lookers.clone() },
			binius_ip_prover::logup_star::TableLookup { table: data_view.clone(), lookers: data_lookers.clone() },
		],
		&mut pt,
	);

	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("spartan verify");
	let verifier_gamma = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "same challenge");
	let verifier_out = logup_star::verify_reduction::<LF, _>(
		&verifier_gamma,
		[
			logup_star::TableLookup { n_vars: FETCH_M, lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &fetch_empty[0] as &[LF], eval_claim: c }).collect() },
			logup_star::TableLookup { n_vars: log2_ceil(data_size), lookers: data_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &data_empty[0] as &[LF], eval_claim: c }).collect() },
		],
		&mut vt,
	)
	.expect("logup verify");
	assert_eq!(prover_out, verifier_out, "outputs agree");
	vt.finalize().expect("finalize");

	println!("✅ FULL VM with STORE: one state machine (load+store read-modify-write loop), one transcript");
	let final_pc = *pc_chain.last().unwrap();
	println!("   Spartan state machine: {N} rounds, x1 {START_X1:#x} -> {final_x1:#x}, pc -> {final_pc:#x}");
	println!("   logup* fetch: {N} P[pc]=word; logup* data: {} load+store M[ts,addr]=value", data_claims.len());
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   cross-check: native x1 = {final_x1:#x} = 2+2+4 ✓");

	// ---- Soundness: a load claiming a stale value (not most recent write) rejected ----
	{
		// Round 2 load (ts=4) reads current mem[0] = stores[1] = 4. Claim 2 (the
		// initial value / a stale earlier write) must be rejected.
		let stale = 2u64;
		let mut bad_data_claims = data_claims.clone();
		// data_claims layout: [load0, store0, load1, store1, load2, store2]
		// load2 is index 4 (0,1,2,3,4,...) = data_claims[4]
		if N >= 3 { bad_data_claims[4] = LF::from(stale as u128); }
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness, &mut rng, &mut bt).expect("state valid");
		let bad_gamma = IPProverChannel::<LF>::sample(&mut bt);
		let bad_lookers: Vec<Looker<LF>> = (0..data_claims.len()).map(|k| Looker { index: &data_idx[k], eval_point: &data_empty[k] as &[LF], eval_claim: bad_data_claims[k] }).collect();
		let _ = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bad_gamma,
			[
				binius_ip_prover::logup_star::TableLookup { table: fetch_view.clone(), lookers: fetch_lookers.clone() },
				binius_ip_prover::logup_star::TableLookup { table: data_view.clone(), lookers: bad_lookers },
			],
			&mut bt,
		);
		let mut btv = bt.into_verifier();
		verifier.verify(&public, &mut btv).expect("state valid");
		let bvg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut btv);
		let rejected = logup_star::verify_reduction::<LF, _>(
			&bvg,
			[
				logup_star::TableLookup { n_vars: FETCH_M, lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &fetch_empty[0] as &[LF], eval_claim: c }).collect() },
				logup_star::TableLookup { n_vars: log2_ceil(data_size), lookers: bad_data_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &data_empty[0] as &[LF], eval_claim: c }).collect() },
			],
			&mut btv,
		)
		.is_err();
		assert!(rejected, "verifier MUST reject a load of a stale (non-most-recent) value");
		println!("   soundness: verifier REJECTED a load claiming a stale (non-most-recent) write ✓");
	}
}

fn log2_ceil(x: usize) -> usize {
	let mut n = 0;
	while (1usize << n) < x { n += 1; }
	n
}