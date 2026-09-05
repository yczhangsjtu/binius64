//! Eighteenth validation slice: FULL VM over MULTIPLE addresses with interleaved
//! read/write — the strongest memory-argument test. Program alternates between
//! two memory addresses (addr 0 and 1), each read-modify-written across rounds,
//! and each load must observe that address's most recent write.
//!
//! Program (initial mem[0]=1, mem[1]=5, N=4 rounds):
//!   loop { t = mem[i&1];  x1 += t;  mem[i&1] = x1+1;  i++;  pc += 4 }
//!   round 0: addr 0, load mem[0]=1  -> x1=1,  store mem[0]=2
//!   round 1: addr 1, load mem[1]=5  -> x1=6,  store mem[1]=7
//!   round 2: addr 0, load mem[0]=2  -> x1=8,  store mem[0]=9
//!   round 3: addr 1, load mem[1]=7  -> x1=15, store mem[1]=16
//!   loads=[1,5,2,7] stores=[2,7,9,16] x1:0->15 (=1+5+2+7)
//!
//! The interleaved read-modify-write (RAW) is the key: address 0 is written at
//! rounds 0 and 2; address 1 at rounds 1 and 3. Round 2's load of mem[0] must
//! read 2 (round 0's write), NOT the initial 1; round 3's load of mem[1] must
//! read 7 (round 1's write), not the initial 5. That is "read sees most recent
//! write" across multiple addresses.
//!
//! Four layers, ONE Fiat-Shamir transcript, ONE witness set:
//!   1. Spartan state machine: per-round load(load_val) + addi(x1+=load_val) +
//!      store(mem[i&1] = x1+1) + incr(i) + pc+=4. `load_val[t]` and `store_val[t]`
//!      carry the memory values; `store_val[t] = x1[t+1]+1` is constrained.
//!   2. logup* program memory: P[pc] = word (fetch).
//!   3. logup* data memory (TIME-ORDERED state table): T[ts*ADDR+addr] = value of
//!      addr at time ts. Each round r gets a LOAD ts=2r and a STORE ts=2r+1, both
//!      at addr = i[r]&1 = r&1. A load claims T[ts_load, addr]; a store claims
//!      T[ts_store, addr]. Time ordering rejects a load claiming a stale value.

use crate::alu::*;

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
const N: usize = 4; // four rounds
const START_PC: u64 = 0x00;
const START_X1: u64 = 0x00;
const MEM_INIT: [u64; 2] = [1, 5]; // mem[0]=1, mem[1]=5
const ADDR: usize = 4; // 4-address data memory (address space 0..3, 2^2)
const TS_MAX: usize = 8; // timestamps 0..7 -> table = 8*4 = 32 = 2^5
const FETCH_M: usize = 6; // 64-address program memory (2^6)

type F = B128;
type P = OptimalPackedB128;
type LF = OptimalB128;
type LP = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

/// The address accessed in round r (i[r]&1). Since i starts at 0 and increments,
/// i[r] = r, so the address is r&1.
fn r_addr(r: usize) -> usize { r & 1 }



/// One round: x1' = x1 + load_val; store_val = x1' + 1; i' = i + 1; pc' = pc + 4.
/// `store_val` wire is constrained to x1_next + 1 (the value written back).
/// Same op order on every builder (witness/instance mirror).
fn drive_round<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	load_val: &[B::Wire],
	store_val: &[B::Wire],
	x1: &[B::Wire],
	i: &[B::Wire],
	pc: &[B::Wire],
	x1_next: &[B::Wire],
	i_next: &[B::Wire],
	pc_next: &[B::Wire],
) {
	for w in load_val.iter().chain(store_val).chain(x1).chain(i).chain(pc).chain(x1_next).chain(i_next).chain(pc_next) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	// x1' = x1 + load_val
	let mut cin = b.constant(B128::ZERO);
	let mut x1n_bits = Vec::with_capacity(BITS);
	for k in 0..BITS {
		let (sum, cout) = fa(b, x1[k], load_val[k], cin);
		b.assert_eq(sum, x1_next[k]);
		x1n_bits.push(sum);
		cin = cout;
	}
	// store_val = x1' + 1
	let inc = to_bits(1, BITS);
	let mut cin2 = b.constant(B128::ZERO);
	for k in 0..BITS {
		let ib = b.constant(inc[k]);
		let (sum, cout) = fa(b, x1_next[k], ib, cin2);
		b.assert_eq(sum, store_val[k]);
		cin2 = cout;
	}
	// i' = i + 1
	let iinc = to_bits(1, BITS);
	let mut cin3 = b.constant(B128::ZERO);
	for k in 0..BITS {
		let ib = b.constant(iinc[k]);
		let (sum, cout) = fa(b, i[k], ib, cin3);
		b.assert_eq(sum, i_next[k]);
		cin3 = cout;
	}
	// pc' = pc + PC_INC
	let pinc = to_bits(PC_INC, BITS);
	let mut cin4 = b.constant(B128::ZERO);
	for k in 0..BITS {
		let ib = b.constant(pinc[k]);
		let (sum, cout) = fa(b, pc[k], ib, cin4);
		b.assert_eq(sum, pc_next[k]);
		cin4 = cout;
	}
}

pub fn run_full_vm_multi() {
	// ---- Native ground truth (interleaved RAW over 2 addresses) ----
	let mut mem = [0u64; 4];
	mem[0] = MEM_INIT[0];
	mem[1] = MEM_INIT[1];
	let mut load_vals = vec![0u64; N];
	let mut store_vals = vec![0u64; N];
	let mut x1_chain = vec![START_X1];
	let mut i_chain = vec![0u64];
	let mut pc_chain = vec![START_PC];
	for r in 0..N {
		let addr = r_addr(r);
		let lv = mem[addr];
		load_vals[r] = lv;
		let last = *x1_chain.last().unwrap();
		let nx = last.wrapping_add(lv);
		x1_chain.push(nx);
		store_vals[r] = nx + 1; // write back x1+1
		mem[addr] = nx + 1;
		i_chain.push(*i_chain.last().unwrap() + 1);
		pc_chain.push(*pc_chain.last().unwrap() + PC_INC);
	}
	let final_x1 = *x1_chain.last().unwrap();
	let final_pc = *pc_chain.last().unwrap();
	let mut all_loads_sum = 0u64;
	for &v in &load_vals { all_loads_sum += v; }

	println!("FULL VM over MULTIPLE addresses, interleaved read-modify-write, one transcript");
	println!("  program: loop {{ t=mem[i&1]; x1+=t; mem[i&1]=x1+1; i++; pc+=4 }} (N={N})");
	println!("  initial mem[0]={} mem[1]={}", MEM_INIT[0], MEM_INIT[1]);
	for r in 0..N {
		println!("    round {r}: addr {} load mem={} x1={} store back={}", r_addr(r), load_vals[r], x1_chain[r], store_vals[r]);
	}
	println!("  loads : {load_vals:?}");
	println!("  stores: {store_vals:?}");
	println!("  x1    : {:?}", x1_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());
	println!("  expect final x1 = {final_x1} ({}+{}+{}+{})", load_vals[0], load_vals[1], load_vals[2], load_vals[3]);

	// ---- Spartan state machine ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let load_w: Vec<Vec<ConstraintWire>> = (0..N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let store_w: Vec<Vec<ConstraintWire>> = (0..N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let x1_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let i_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let pc_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	for r in 0..N {
		drive_round(&mut cb, &load_w[r], &store_w[r], &x1_w[r], &i_w[r], &pc_w[r], &x1_w[r + 1], &i_w[r + 1], &pc_w[r + 1]);
	}
	let (cs, layout) = compile(cb);
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	let load_bits: Vec<Vec<B128>> = load_vals.iter().map(|&v| to_bits(v, BITS)).collect();
	let store_bits: Vec<Vec<B128>> = store_vals.iter().map(|&v| to_bits(v, BITS)).collect();
	let x1_bits: Vec<Vec<B128>> = x1_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let i_bits: Vec<Vec<B128>> = i_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let pc_bits: Vec<Vec<B128>> = pc_chain.iter().map(|&v| to_bits(v, BITS)).collect();

	// ---- Witness (segmented order = allocation order: load, store, x1, i, pc) ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	for r in 0..N { for k in 0..BITS { wg.write_inout(load_w[r][k], load_bits[r][k]); } }
	for r in 0..N { for k in 0..BITS { wg.write_inout(store_w[r][k], store_bits[r][k]); } }
	for r in 0..=N { for k in 0..BITS { wg.write_inout(x1_w[r][k], x1_bits[r][k]); } }
	for r in 0..=N { for k in 0..BITS { wg.write_inout(i_w[r][k], i_bits[r][k]); } }
	for r in 0..=N { for k in 0..BITS { wg.write_inout(pc_w[r][k], pc_bits[r][k]); } }
	for r in 0..N {
		let wlv: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(load_w[r][k], load_bits[r][k])).collect();
		let wsv: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(store_w[r][k], store_bits[r][k])).collect();
		let wx1: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[r][k], x1_bits[r][k])).collect();
		let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[r][k], i_bits[r][k])).collect();
		let wp: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[r][k], pc_bits[r][k])).collect();
		let wx1n: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[r + 1][k], x1_bits[r + 1][k])).collect();
		let win: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[r + 1][k], i_bits[r + 1][k])).collect();
		let wpn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[r + 1][k], pc_bits[r + 1][k])).collect();
		drive_round(&mut wg, &wlv, &wsv, &wx1, &wi, &wp, &wx1n, &win, &wpn);
	}
	let witness = wg.build().expect("witness");
	cs.validate(&witness);

	// ---- Instance (same op order as witness) ----
	let mut ig = InstanceGenerator::new(&layout);
	for r in 0..=N { for k in 0..BITS { ig.write_inout(x1_w[r][k], x1_bits[r][k]); } }
	for r in 0..=N { for k in 0..BITS { ig.write_inout(i_w[r][k], i_bits[r][k]); } }
	for r in 0..=N { for k in 0..BITS { ig.write_inout(pc_w[r][k], pc_bits[r][k]); } }
	for r in 0..N { for k in 0..BITS { ig.write_inout(load_w[r][k], load_bits[r][k]); } }
	for r in 0..N { for k in 0..BITS { ig.write_inout(store_w[r][k], store_bits[r][k]); } }
	for r in 0..N {
		let mlv: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(load_w[r][k], load_bits[r][k])).collect();
		let msv: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(store_w[r][k], store_bits[r][k])).collect();
		let mx1: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[r][k], x1_bits[r][k])).collect();
		let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[r][k], i_bits[r][k])).collect();
		let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[r][k], pc_bits[r][k])).collect();
		let mx1n: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[r + 1][k], x1_bits[r + 1][k])).collect();
		let min: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[r + 1][k], i_bits[r + 1][k])).collect();
		let mpn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[r + 1][k], pc_bits[r + 1][k])).collect();
		drive_round(&mut ig, &mlv, &msv, &mx1, &mi, &mp, &mx1n, &min, &mpn);
	}
	let public = ig.build();

	// ---- logup* program memory: P[pc] = word (fetch) ----
	let fetch_size = 1usize << FETCH_M;
	let mut prog = vec![0u64; fetch_size];
	for r in 0..N {
		prog[(START_PC as usize + r * PC_INC as usize) & (fetch_size - 1)] = (r as u64) + 1;
	}
	// ---- logup* data memory: TIME-ORDERED state table T[ts*ADDR+addr] ----
	let data_size = TS_MAX * ADDR;
	let mut t = vec![0u64; data_size];
	let mut current = vec![0u64; ADDR];
	current[0] = MEM_INIT[0];
	current[1] = MEM_INIT[1];
	for ts in 0..TS_MAX {
		if ts % 2 == 1 {
			let r = (ts - 1) / 2; // store round index
			if r < N {
				current[r_addr(r)] = store_vals[r]; // write back x1+1 to addr r&1
			}
		}
		for a in 0..ADDR {
			t[ts * ADDR + a] = current[a];
		}
	}
	let data_table_f = FieldBuffer::from_values(&t.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let data_view = data_table_f.as_view();

	let alloc = GlobalAllocator;
	let fetch_table = FieldBuffer::from_values(&prog.iter().map(|&w| LF::from(w as u128)).collect::<Vec<_>>());
	let fetch_view = fetch_table.as_view();

	// fetch lookers
	let mut fetch_idx: Vec<Vec<usize>> = Vec::new();
	for r in 0..N { fetch_idx.push(vec![(START_PC as usize + r * PC_INC as usize) & (fetch_size - 1)]); }
	let fetch_claims: Vec<LF> = (0..N).map(|r| LF::from((r as u64 + 1) as u128)).collect();
	// data lookers: round r load (ts=2r) + store (ts=2r+1), both at addr r&1
	let mut data_idx: Vec<Vec<usize>> = Vec::new();
	let mut data_claims: Vec<LF> = Vec::new();
	for r in 0..N {
		let a = r_addr(r);
		data_idx.push(vec![(2 * r) * ADDR + a]); // load at ts=2r
		data_claims.push(LF::from(load_vals[r] as u128));
		data_idx.push(vec![(2 * r + 1) * ADDR + a]); // store at ts=2r+1
		data_claims.push(LF::from(store_vals[r] as u128));
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

	println!("✅ FULL VM over MULTIPLE addresses: interleaved read-modify-write, one transcript");
	println!("   Spartan: {N} rounds, x1 {START_X1:#x} -> {final_x1:#x}, pc -> {final_pc:#x}");
	println!("   logup* fetch: {N} P[pc]=word; logup* data: {} load+store M[ts,addr]=value (2 addresses)", data_claims.len());
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   cross-check: native x1 = {final_x1:#x} = {all_loads_sum} = sum of loads ✓");

	// ---- Soundness: a load claiming a STALE value (initial, not most-recent write) ----
	// Round 3 load of addr 1 (ts=6) reads mem[1]=7 (written at round 1). Claim
	// the INITIAL mem[1]=5 -> reject (read must see most recent write).
	{
		let stale = MEM_INIT[1]; // 5, the initial value at addr 1
		let mut bad_data_claims = data_claims.clone();
		// data_claims layout: [load0,store0,load1,store1,load2,store2,load3,store3]
		// load3 is index 6 -> data_claims[6]
		if N >= 4 { bad_data_claims[6] = LF::from(stale as u128); }
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
		assert!(rejected, "verifier MUST reject a load claiming a stale (initial) value across multi-address");
		println!("   soundness: verifier REJECTED a load of a stale initial value (multi-address, read-sees-most-recent-write) ✓");
	}
}

fn log2_ceil(x: usize) -> usize {
	let mut n = 0;
	while (1usize << n) < x { n += 1; }
	n
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn full_vm_multi() {
		run_full_vm_multi();
	}
}
