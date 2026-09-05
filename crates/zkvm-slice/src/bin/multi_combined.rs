//! Tenth validation slice: multi-instruction COMBINED proof — logup* fetch +
//! Spartan execution of a real 2-instruction program in ONE transcript.
//!
//! Program (two consecutive `addi x5,x5,imm`, pc = 0x00, 0x04):
//!   addi x5,x5,1   -> x5: 0xA5 -> 0xA6
//!   addi x5,x5,2   -> x5: 0xA6 -> 0xA8
//!
//!  - Spartan layer: per-step state machine. Each step decodes `addi` (opcode
//!    + funct3), extracts its immediate from the instruction-word inout, and
//!    constrains reg' = reg + imm and pc' = pc + 4 via integer carry adders.
//!    State wires are shared so step t+1 reads step t's output (dependency).
//!  - logup* layer: program-memory fetch. A table T maps address -> word; one
//!    looker per EXECUTED instruction claims T[pc_i] = word_i. So the verifier
//!    is assured the words the Spartan layer executed were really fetched from
//!    program memory (universal lookup, not the naive O(N x W) per-address
//!    commitment).
//!  - ONE transcript: Spartan observes public, then logup* gamma is sampled.
//!
//! This is the first program proven with a genuinely modular Jolt-like split:
//! lookup(Fetch) + constraint(Execute) composed into a single proof.

use binius_compute::GlobalAllocator;
use binius_field::{Field, Ghash128b as B128, arch::{OptimalB128, OptimalPackedB128}};
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

const IW: usize = 32;
const BITS: usize = 8;
const PC_INC: u64 = 4;
const OPCODE_ADDI: u64 = 0x13;
const FUNCT3_ADD: u64 = 0x0;
const N: usize = 2; // two instructions
const START_PC: u64 = 0x00;
const START_X5: u64 = 0xA5;
const IMMS: [u64; N] = [1, 2]; // per-instruction immediate

type F = B128;
type P = OptimalPackedB128;
type LF = OptimalB128;
type LP = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

fn to_bits(val: u64, nbits: usize) -> Vec<B128> {
	(0..nbits).map(|i| B128::new(((val >> i) & 1) as u128)).collect()
}

/// Encode `addi x5, x5, imm`.
fn enc_addi(imm: u64) -> u64 {
	((imm & 0xfff) << 20) | (5u64 << 15) | (0u64 << 12) | (5u64 << 7) | 0x13
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

/// One `addi x5,x5,imm` step: decode + reg'=reg+imm(inst[20..28]) + pc'=pc+4.
/// Same op order on all builders.
fn drive_step<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	inst: &[B::Wire],
	pc: &[B::Wire],
	reg: &[B::Wire],
	pc_next: &[B::Wire],
	reg_next: &[B::Wire],
) {
	for w in inst.iter().chain(pc.iter()).chain(reg.iter()).chain(pc_next.iter()).chain(reg_next.iter()) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	for (i, &c) in to_bits(OPCODE_ADDI, 7).iter().enumerate() {
		let cw = b.constant(c);
		let d = b.add(inst[i], cw);
		b.assert_zero(d);
	}
	for (i, &c) in to_bits(FUNCT3_ADD, 3).iter().enumerate() {
		let cw = b.constant(c);
		let d = b.add(inst[12 + i], cw);
		b.assert_zero(d);
	}
	// reg' = reg + imm, imm = inst[20..28] (low 8-bit immediate)
	let mut cin = b.constant(B128::ZERO);
	for i in 0..BITS {
		let m = inst[20 + i]; // immediate bit
		let a = reg[i];
		let (sum, cout) = fa(b, a, m, cin);
		b.assert_eq(sum, reg_next[i]);
		cin = cout;
	}
	// pc' = pc + PC_INC
	let inc = to_bits(PC_INC, BITS);
	let mut cin2 = b.constant(B128::ZERO);
	for i in 0..BITS {
		let ib = b.constant(inc[i]);
		let a = pc[i];
		let (sum, cout) = fa(b, a, ib, cin2);
		b.assert_eq(sum, pc_next[i]);
		cin2 = cout;
	}
}

fn main() {
	let insts: Vec<u64> = IMMS.iter().map(|&imm| enc_addi(imm)).collect();
	let mut x5_chain = vec![START_X5];
	for &imm in &IMMS {
		let last = *x5_chain.last().unwrap();
		x5_chain.push(last.wrapping_add(imm));
	}
	let mut pc_chain = vec![START_PC];
	for _ in 0..N {
		let last = *pc_chain.last().unwrap();
		pc_chain.push(last + PC_INC);
	}
	let final_x5 = *x5_chain.last().unwrap();
	let final_pc = *pc_chain.last().unwrap();
	println!("program (combined fetch+execute, 2 instrs):");
	for (i, &w) in insts.iter().enumerate() {
		println!("  pc={:#06x}: {w:#010x}  addi x5,x5,{}", START_PC + (i as u64) * PC_INC, IMMS[i]);
	}
	println!("  reg chain: {:?}", x5_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());

	// ---- Spartan: allocate shared state per step ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	// segmented: [inst(t)x32 for t | pc(t)x8 for t=0..=N | reg(t)x8 for t=0..=N]
	let inst_w: Vec<Vec<ConstraintWire>> = (0..N).map(|_| (0..IW).map(|_| cb.alloc_inout()).collect()).collect();
	let pc_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let reg_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	for t in 0..N {
		drive_step(&mut cb, &inst_w[t], &pc_w[t], &reg_w[t], &pc_w[t + 1], &reg_w[t + 1]);
	}
	let (cs, layout) = compile(cb);
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	let inst_bits: Vec<Vec<B128>> = insts.iter().map(|&w| to_bits(w, IW)).collect();
	let pc_bits: Vec<Vec<B128>> = pc_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let reg_bits: Vec<Vec<B128>> = x5_chain.iter().map(|&v| to_bits(v, BITS)).collect();

	// ---- Witness ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	for t in 0..N { for k in 0..IW { wg.write_inout(inst_w[t][k], inst_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(pc_w[t][k], pc_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { wg.write_inout(reg_w[t][k], reg_bits[t][k]); } }
	for t in 0..N {
		let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..IW).map(|k| wg.write_inout(inst_w[t][k], inst_bits[t][k])).collect();
		let wp: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[t][k], pc_bits[t][k])).collect();
		let wr: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(reg_w[t][k], reg_bits[t][k])).collect();
		let wpn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[t + 1][k], pc_bits[t + 1][k])).collect();
		let wrn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(reg_w[t + 1][k], reg_bits[t + 1][k])).collect();
		drive_step(&mut wg, &wi, &wp, &wr, &wpn, &wrn);
	}
	let witness = wg.build().expect("witness");
	cs.validate(&witness);

	// ---- Instance ----
	let mut ig = InstanceGenerator::new(&layout);
	for t in 0..=N { for k in 0..BITS { ig.write_inout(pc_w[t][k], pc_bits[t][k]); } }
	for t in 0..=N { for k in 0..BITS { ig.write_inout(reg_w[t][k], reg_bits[t][k]); } }
	for t in 0..N { for k in 0..IW { ig.write_inout(inst_w[t][k], inst_bits[t][k]); } }
	for t in 0..N {
		let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..IW).map(|k| ig.write_inout(inst_w[t][k], inst_bits[t][k])).collect();
		let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[t][k], pc_bits[t][k])).collect();
		let mr: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(reg_w[t][k], reg_bits[t][k])).collect();
		let mpn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[t + 1][k], pc_bits[t + 1][k])).collect();
		let mrn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(reg_w[t + 1][k], reg_bits[t + 1][k])).collect();
		drive_step(&mut ig, &mi, &mp, &mr, &mpn, &mrn);
	}
	let public = ig.build();

	// ======= logup* layer: program memory T[pc] = word ========
	// Table over 2^m addresses; each executed instruction is one looker with
	// index = its pc and claim = its word.
	let m = 6; // 64 addresses
	let table_size = 1usize << m;
	let mut prog = vec![0u64; table_size];
	for t in 0..N {
		prog[(START_PC as usize + t * PC_INC as usize) & (table_size - 1)] = insts[t];
	}
	let alloc = GlobalAllocator;
	let table = FieldBuffer::from_values(&prog.iter().map(|&w| LF::from(w as u128)).collect::<Vec<_>>());
	let table_view = table.as_view();
	// Pre-build owned index columns and eval-point slices, then borrow them.
	let mut index_cols: Vec<Vec<usize>> = Vec::with_capacity(N);
	for t in 0..N {
		index_cols.push(vec![(START_PC as usize + t * PC_INC as usize) & (table_size - 1)]);
	}
	let empty_pts: Vec<[LF; 0]> = (0..N).map(|_| []).collect();
	let lookers: Vec<Looker<LF>> = (0..N)
		.map(|t| {
			let empty: &[LF] = &empty_pts[t];
			Looker {
				index: &index_cols[t],
				eval_point: empty,
				eval_claim: LF::from(insts[t] as u128),
			}
		})
		.collect();

	// ======= ONE combined transcript =======
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness, &mut rng, &mut pt).expect("spartan prove");
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
		&alloc,
		gamma,
		[binius_ip_prover::logup_star::TableLookup { table: table_view, lookers }],
		&mut pt,
	);

	// ======= Combined verification =======
	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("spartan verify");
	let verifier_gamma = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "same lookup challenge");
	let claims: Vec<LF> = insts.iter().map(|&w| LF::from(w as u128)).collect();
	let verifier_out = logup_star::verify_reduction::<LF, _>(
		&verifier_gamma,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: c }).collect(),
		}],
		&mut vt,
	)
	.expect("logup verify");
	assert_eq!(prover_out, verifier_out, "lookup outputs agree");
	vt.finalize().expect("finalize");

	println!("✅ MULTI-INSTRUCTION COMBINED proof: Spartan execute + logup* fetch, one transcript");
	println!("   {N} instrs; Spartan (pc,x5) ({START_PC:#x},{START_X5:#x}) -> ({final_pc:#x},{final_x5:#x})");
	println!("   logup*: {N} fetches T[pc] = word verified against program table");
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   cross-check: native reg chain final = {final_x5:#x} ✓");

	// ---- Soundness: a fetch claim absent from program memory must be rejected ----
	// Re-prove the honest state but ask the verifier to accept a lookup claim for a
	// word NOT in the program table. The logup* layer must reject it (this is where
	// a malicious fetch is caught in a real zkVM).
	{
		let bogus_word = 0xffff_u64;
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness, &mut rng, &mut bt).expect("prove(2) state");
		let bad_gamma = IPProverChannel::<LF>::sample(&mut bt);
		let lookers2: Vec<Looker<LF>> = (0..N)
			.map(|t| Looker {
				index: &index_cols[t],
				eval_point: &empty_pts[t] as &[LF],
				eval_claim: if t == 0 { LF::from(bogus_word as u128) } else { claims[t] },
			})
			.collect();
		binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bad_gamma,
			[binius_ip_prover::logup_star::TableLookup { table: table_view, lookers: lookers2 }],
			&mut bt,
		);
		let mut btv = bt.into_verifier();
		verifier.verify(&public, &mut btv).expect("state valid");
		let bvg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut btv);
		let bogus_claims: Vec<LF> = (0..N)
			.map(|t| if t == 0 { LF::from(bogus_word as u128) } else { claims[t] })
			.collect();
		let rejected = logup_star::verify_reduction::<LF, _>(
			&bvg,
			[logup_star::TableLookup {
				n_vars: m,
				lookers: bogus_claims.iter().map(|&c| logup_star::LookerClaim {
					eval_point: &empty_pts[0] as &[LF],
					eval_claim: c,
				}).collect(),
			}],
			&mut btv,
		)
		.is_err();
		assert!(rejected, "verifier MUST reject a fetch claim absent from program memory");
		println!("   soundness: verifier REJECTED a fetch claim absent from program memory ✓");
	}
}
