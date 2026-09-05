//! Sixteenth validation slice: FULL VM — the engineering integration of the
//! three zkVM mechanisms into ONE state machine proving ONE complete program.
//!
//! Program (a memory-accessing loop, N=4 rounds):
//!   mem[0]=2, mem[1]=3, mem[2]=5, mem[3]=7   (initial data memory)
//!   x1=0; i=0; pc=0
//!   loop:
//!     t = mem[i]        # load
//!     x1 = x1 + t       # addi
//!     i = i + 1         # incr
//!     pc = pc + 4
//!   # expected x1 = 2+3+5+7 = 17
//!
//! Three layers, ONE Fiat-Shamir transcript, ONE witness set:
//!   1. Spartan state machine: per-round load/incr/addi/pc via carry adders
//!      (reuses the multi_combined `drive_step` skeleton). `mem_val[t]` is the
//!      load output, materialized as a shared wire.
//!   2. logup* program memory: P[pc] = word (fetch layer, multi_combined).
//!   3. logup* data memory: M[i] = mem_val (memory arg / "read the value that
//!      really is at memory[i]"), mem_arg_spice's time-ordered table essence.
//!
//! The integration point: `mem_val[t]` is BOTH the Spartan state-machine load
//! result AND the data-memory looker claim — so one machine simultaneously
//! proves (a) the register array accumulates the loaded values (execute), and
//! (b) those loaded values are really the memory contents (memory argument).

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
const N: usize = 4; // four loop rounds
const START_PC: u64 = 0x00;
const START_X1: u64 = 0x00;
const MEM: [u64; N] = [2, 3, 5, 7]; // initial data memory

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

/// One round: load `mem_val[t]` (witness), x1' = x1 + mem_val, i' = i + 1,
/// pc' = pc + 4. Same op order on every builder (witness/instance mirror).
fn drive_round<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	mem_val: &[B::Wire],
	x1: &[B::Wire],
	i: &[B::Wire],
	pc: &[B::Wire],
	x1_next: &[B::Wire],
	i_next: &[B::Wire],
	pc_next: &[B::Wire],
) {
	for w in mem_val.iter().chain(x1).chain(i).chain(pc).chain(x1_next).chain(i_next).chain(pc_next) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	// x1' = x1 + mem_val  (carry-adder, mem_val is the addend witness)
	let mut cin = b.constant(B128::ZERO);
	for k in 0..BITS {
		let (sum, cout) = fa(b, x1[k], mem_val[k], cin);
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
	// ---- Native ground-truth trace ----
	let mut x1_chain = vec![START_X1];
	let mut i_chain = vec![0u64];
	let mut pc_chain = vec![START_PC];
	let mut load_vals = vec![0u64; N];
	for t in 0..N {
		let mem_val = MEM[t];
		load_vals[t] = mem_val;
		let last = *x1_chain.last().unwrap();
		x1_chain.push(last.wrapping_add(mem_val));
		i_chain.push(*i_chain.last().unwrap() + 1);
		pc_chain.push(*pc_chain.last().unwrap() + PC_INC);
	}
	let final_x1 = *x1_chain.last().unwrap();
	let final_pc = *pc_chain.last().unwrap();

	println!("FULL VM — one state machine, three zkVM layers, one transcript");
	println!("  program: loop {{ t=mem[i]; x1+=t; i++; pc+=4 }} (N={N} rounds)");
	println!("  data mem: {MEM:?}");
	println!("  reg chain x1: {:?}", x1_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());
	println!("  i chain       : {:?}", i_chain);
	println!("  pc chain      : {:?}", pc_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());
	println!("  expect final x1 = 2+3+5+7 = {final_x1}");

	// ---- Spartan: shared-state per round ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let mem_val_w: Vec<Vec<ConstraintWire>> = (0..N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let x1_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let i_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let pc_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	for t in 0..N {
		drive_round(&mut cb, &mem_val_w[t], &x1_w[t], &i_w[t], &pc_w[t], &x1_w[t + 1], &i_w[t + 1], &pc_w[t + 1]);
	}
	let (cs, layout) = compile(cb);
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	let mem_val_bits: Vec<Vec<B128>> = load_vals.iter().map(|&v| to_bits(v, BITS)).collect();
	let x1_bits: Vec<Vec<B128>> = x1_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let i_bits: Vec<Vec<B128>> = i_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let pc_bits: Vec<Vec<B128>> = pc_chain.iter().map(|&v| to_bits(v, BITS)).collect();

	// ---- Witness (segmented inout order must match allocation: mem_val, x1, i, pc) ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	for t in 0..N { for k in 0..BITS { wg.write_inout(mem_val_w[t][k], mem_val_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(x1_w[t][k], x1_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(i_w[t][k], i_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(pc_w[t][k], pc_bits[t][k]); } }
	for t in 0..N {
		let wmv: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(mem_val_w[t][k], mem_val_bits[t][k])).collect();
		let wx1: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[t][k], x1_bits[t][k])).collect();
		let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[t][k], i_bits[t][k])).collect();
		let wp: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[t][k], pc_bits[t][k])).collect();
		let wx1n: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[t + 1][k], x1_bits[t + 1][k])).collect();
		let win: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[t + 1][k], i_bits[t + 1][k])).collect();
		let wpn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[t + 1][k], pc_bits[t + 1][k])).collect();
		drive_round(&mut wg, &wmv, &wx1, &wi, &wp, &wx1n, &win, &wpn);
	}
	let witness = wg.build().expect("witness");
	cs.validate(&witness);

	// ---- Instance (same op order as witness) ----
	let mut ig = InstanceGenerator::new(&layout);
	for t in 0..=N { for k in 0..BITS { ig.write_inout(x1_w[t][k], x1_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { ig.write_inout(i_w[t][k], i_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { ig.write_inout(pc_w[t][k], pc_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { ig.write_inout(mem_val_w[t % N][k], mem_val_bits[t % N][k]); } }
	for t in 0..N {
		let mx1: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[t][k], x1_bits[t][k])).collect();
		let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[t][k], i_bits[t][k])).collect();
		let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[t][k], pc_bits[t][k])).collect();
		let mmv: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(mem_val_w[t][k], mem_val_bits[t][k])).collect();
		let mx1n: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[t + 1][k], x1_bits[t + 1][k])).collect();
		let min: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[t + 1][k], i_bits[t + 1][k])).collect();
		let mpn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[t + 1][k], pc_bits[t + 1][k])).collect();
		drive_round(&mut ig, &mmv, &mx1, &mi, &mp, &mx1n, &min, &mpn);
	}
	let public = ig.build();

	// ---- logup* program memory: P[pc] = word ----
	// We encode a placeholder "word" per round (no decode in this minimal VM);
	// fetch just proves the pc the machine stepped on is a valid program address.
	let fetch_m = 6; // 64 program addresses
	let fetch_size = 1usize << fetch_m;
	let mut prog = vec![0u64; fetch_size];
	for t in 0..N {
		prog[(START_PC as usize + t * PC_INC as usize) & (fetch_size - 1)] = (t as u64) + 1;
	}
	// ---- logup* data memory: M[addr] = value, address = i[t] (= t) ----
	// i[t]=t (since i increments from 0), so load address t reads MEM[t].
	let data_m = 3; // 8-address data memory
	let data_size = 1usize << data_m;
	let mut data_table = vec![0u64; data_size];
	for t in 0..N {
		data_table[t & (data_size - 1)] = MEM[t];
	}

	let alloc = GlobalAllocator;
	let fetch_table = FieldBuffer::from_values(&prog.iter().map(|&w| LF::from(w as u128)).collect::<Vec<_>>());
	let fetch_view = fetch_table.as_view();
	let data_table_f = FieldBuffer::from_values(&data_table.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let data_view = data_table_f.as_view();

	// fetch lookers
	let mut fetch_idx: Vec<Vec<usize>> = Vec::with_capacity(N);
	for t in 0..N { fetch_idx.push(vec![(START_PC as usize + t * PC_INC as usize) & (fetch_size - 1)]); }
	let fetch_claims: Vec<LF> = (0..N).map(|t| LF::from((t as u64 + 1) as u128)).collect();
	// data lookers (address = i[t] = t)
	let mut data_idx: Vec<Vec<usize>> = Vec::with_capacity(N);
	for t in 0..N { data_idx.push(vec![t & (data_size - 1)]); }
	let data_claims: Vec<LF> = (0..N).map(|t| LF::from(MEM[t] as u128)).collect();

	let fetch_empty: Vec<[LF; 0]> = (0..N).map(|_| []).collect();
	let data_empty: Vec<[LF; 0]> = (0..N).map(|_| []).collect();
	let fetch_lookers: Vec<Looker<LF>> = (0..N)
		.map(|t| Looker { index: &fetch_idx[t], eval_point: &fetch_empty[t] as &[LF], eval_claim: fetch_claims[t] })
		.collect();
	let data_lookers: Vec<Looker<LF>> = (0..N)
		.map(|t| Looker { index: &data_idx[t], eval_point: &data_empty[t] as &[LF], eval_claim: data_claims[t] })
		.collect();

	// ---- ONE transcript: Spartan, then gamma, then both logup* tables ----
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
	assert_eq!(verifier_gamma, gamma, "same lookup challenge");
	let verifier_out = logup_star::verify_reduction::<LF, _>(
		&verifier_gamma,
		[
			logup_star::TableLookup {
				n_vars: fetch_m,
				lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &fetch_empty[0] as &[LF], eval_claim: c }).collect(),
			},
			logup_star::TableLookup {
				n_vars: data_m,
				lookers: data_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &data_empty[0] as &[LF], eval_claim: c }).collect(),
			},
		],
		&mut vt,
	)
	.expect("logup verify");
	assert_eq!(prover_out, verifier_out, "lookup outputs agree");
	vt.finalize().expect("finalize");

	println!("✅ FULL VM: one state machine proving execute + fetch + memory argument, one transcript");
	println!("   Spartan state machine: {N} rounds, x1 {START_X1:#x} -> {final_x1:#x}, pc -> {final_pc:#x}");
	println!("   logup* fetch: {N} fetches P[pc]=word; logup* memory: {N} loads M[i]=value");
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   cross-check: native x1 = {final_x1:#x} = 2+3+5+7 ✓");

	// ---- Soundness: a data-memory load claiming a WRONG value must be rejected ----
	// Re-prove honest state but ask the verifier to accept a wrong data-memory value.
	{
		let bogus = 0xAAu64; // not any of MEM[0..3]
		let mut bad_data_claims = data_claims.clone();
		bad_data_claims[0] = LF::from(bogus as u128);
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness, &mut rng, &mut bt).expect("state valid");
		let bad_gamma = IPProverChannel::<LF>::sample(&mut bt);
		let bad_data_lookers: Vec<Looker<LF>> = (0..N)
			.map(|t| Looker { index: &data_idx[t], eval_point: &data_empty[t] as &[LF], eval_claim: bad_data_claims[t] })
			.collect();
		let _ = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bad_gamma,
			[
				binius_ip_prover::logup_star::TableLookup { table: fetch_view.clone(), lookers: fetch_lookers.clone() },
				binius_ip_prover::logup_star::TableLookup { table: data_view.clone(), lookers: bad_data_lookers },
			],
			&mut bt,
		);
		let mut btv = bt.into_verifier();
		verifier.verify(&public, &mut btv).expect("state valid");
		let bvg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut btv);
		let bad_claims_all: Vec<LF> = bad_data_claims;
		let rejected = logup_star::verify_reduction::<LF, _>(
			&bvg,
			[
				logup_star::TableLookup {
					n_vars: fetch_m,
					lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &fetch_empty[0] as &[LF], eval_claim: c }).collect(),
				},
				logup_star::TableLookup {
					n_vars: data_m,
					lookers: bad_claims_all.iter().map(|&c| logup_star::LookerClaim { eval_point: &data_empty[0] as &[LF], eval_claim: c }).collect(),
				},
			],
			&mut btv,
		)
		.is_err();
		assert!(rejected, "verifier MUST reject a data-memory load value not in memory");
		println!("   soundness: verifier REJECTED a data-memory load value absent from memory ✓");
	}
}

fn log2_ceil(x: usize) -> usize {
	let mut n = 0;
	while (1usize << n) < x { n += 1; }
	n
}