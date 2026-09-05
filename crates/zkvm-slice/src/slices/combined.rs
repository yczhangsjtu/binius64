//! Ninth validation slice: COMBINED proof system — logup* instruction-lookup
//! + Spartan state-transition in a SINGLE Fiat-Shamir transcript.
//!
//! A real zkVM needs a LOOKUP layer (program memory: pc -> instruction) and
//! an R1CS layer (ALU/register/PC state machine). Jolt splits these into
//! Lasso + Spartan. Here we prove BOTH over the binary field in ONE proof:
//!
//!   - Spartan layer : execution of `addi x5,x5,1` step,
//!                     (pc,x5) -> (pc+4, x5+1) via integer carry add.
//!   - logup* layer  : program-memory lookup — the executed instruction word
//!                     is claimed to be in the program table at its pc.
//!   - ONE transcript: the Spartan prover observes its public first, then the
//!                     logup* gamma is sampled AFTER it, so the lookup
//!                     challenge depends (Fiat-Shamir) on the state proof.
//!
//! The verifier re-derives both on one channel: the modular Jolt split
//! (lookup subsystem + constraint subsystem) composing into a single proof.

use crate::alu::*;

use binius_compute::GlobalAllocator;
use binius_field::{Field, Ghash128b as B128,
	arch::{OptimalB128, OptimalPackedB128},
};
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

// Spartan uses Ghash128b; logup* uses OptimalB128. Both 128-bit.
type F = B128;
type P = OptimalPackedB128;
type LF = OptimalB128;
type LP = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;



/// Spartan: one `addi x5,x5,1` step. reg' = reg+1, pc' = pc+PC_INC.
/// Segmented inout: [inst(32) | pc(8) | reg(8) | pc_next(8) | reg_next(8)].
fn drive_addi<B: CircuitBuilder<Field = B128>>(
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
	let imm = to_bits(1, BITS);
	let mut cin = b.constant(B128::ZERO);
	for i in 0..BITS {
		let ib = b.constant(imm[i]);
		let a = reg[i];
		let (sum, cout) = fa(b, a, ib, cin);
		b.assert_eq(sum, reg_next[i]);
		cin = cout;
	}
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

pub fn run_combined() {
	// ---- Concrete program: `addi x5,x5,1` at pc = 0x00 ----
	let inst_word: u64 = (1u64 << 20) | (5u64 << 15) | (0u64 << 12) | (5u64 << 7) | 0x13;
	let init_pc: u64 = 0x00;
	let init_x5: u64 = 0xA5;
	let final_x5 = init_x5.wrapping_add(1);
	let final_pc = init_pc + PC_INC;
	println!("program: addi x5,x5,1 @ pc 0x00");
	println!("  state (pc,x5): ({init_pc:#x},{init_x5:#x}) -> ({final_pc:#x},{final_x5:#x})");

	// ======= Spartan layer: build constraint system =======
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let inst_w: Vec<ConstraintWire> = (0..IW).map(|_| cb.alloc_inout()).collect();
	let pc_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let reg_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let pcn_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let regn_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	drive_addi(&mut cb, &inst_w, &pc_w, &reg_w, &pcn_w, &regn_w);
	let (cs, layout) = compile(cb);
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	let mut rng = StdRng::seed_from_u64(0);
	let iw = to_bits(inst_word, IW);
	let ip = to_bits(init_pc, BITS);
	let ix = to_bits(init_x5, BITS);
	let ipn = to_bits(final_pc, BITS);
	let ixn = to_bits(final_x5, BITS);

	let mut wg = WitnessGenerator::new(&layout);
	for k in 0..IW { wg.write_inout(inst_w[k], iw[k]); }
	for k in 0..BITS { wg.write_inout(pc_w[k], ip[k]); wg.write_inout(reg_w[k], ix[k]); wg.write_inout(pcn_w[k], ipn[k]); wg.write_inout(regn_w[k], ixn[k]); }
	let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..IW).map(|k| wg.write_inout(inst_w[k], iw[k])).collect();
	let wp: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[k], ip[k])).collect();
	let wr: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(reg_w[k], ix[k])).collect();
	let wpn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pcn_w[k], ipn[k])).collect();
	let wrn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(regn_w[k], ixn[k])).collect();
	drive_addi(&mut wg, &wi, &wp, &wr, &wpn, &wrn);
	let witness = wg.build().expect("spartan witness");
	cs.validate(&witness);

	let mut ig = InstanceGenerator::new(&layout);
	for k in 0..IW { ig.write_inout(inst_w[k], iw[k]); }
	for k in 0..BITS {
		ig.write_inout(pc_w[k], ip[k]); ig.write_inout(reg_w[k], ix[k]); ig.write_inout(pcn_w[k], ipn[k]); ig.write_inout(regn_w[k], ixn[k]);
	}
	let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..IW).map(|k| ig.write_inout(inst_w[k], iw[k])).collect();
	let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[k], ip[k])).collect();
	let mr: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(reg_w[k], ix[k])).collect();
	let mpn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pcn_w[k], ipn[k])).collect();
	let mrn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(regn_w[k], ixn[k])).collect();
	drive_addi(&mut ig, &mi, &mp, &mr, &mpn, &mrn);
	let public = ig.build();

	// ======= logup* layer: program-memory lookup ========
	// Table over addresses 0..2^m. Index = pc (m-bit). Value = instruction word
	// (or 0 for unoccupied slots). Looker claims T[pc] == inst_word.
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

	// ======= ONE combined transcript =======
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness, &mut rng, &mut pt).expect("spartan prove");
	let gamma = IPProverChannel::<LF>::sample(&mut pt); // AFTER spartan public observed
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
		&alloc,
		gamma,
		[binius_ip_prover::logup_star::TableLookup { table: table_view, lookers: vec![looker] }],
		&mut pt,
	);

	// ======= Combined verification =======
	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("spartan verify");
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

	println!("✅ COMBINED proof: Spartan state-transition + logup* instruction lookup, ONE transcript");
	println!("   Spartan: (pc,x5) ({init_pc:#x},{init_x5:#x}) -> ({final_pc:#x},{final_x5:#x})");
	println!("   logup*:  T[pc=0x{init_pc:x}] = word {inst_word:#010x} found in program table");
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   cross-check: native x5+1 = {final_x5:#x} ✓");

	// ---- Soundness: claim a wrong instruction word in the lookup (verify must fail) ----
	{
		let bad_word = inst_word.wrapping_add(1);
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness, &mut rng, &mut bt).expect("prove(2)");
		let bad_gamma = IPProverChannel::<LF>::sample(&mut bt);
		binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bad_gamma,
			[binius_ip_prover::logup_star::TableLookup {
				table: table_view,
				lookers: vec![Looker { index: &index, eval_point: &[], eval_claim: LF::from(bad_word as u128) }],
			}],
			&mut bt,
		);
		let mut bv = bt.into_verifier();
		verifier.verify(&public, &mut bv).expect("prove(2) valid state");
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
		println!("   soundness: verifier REJECTED a tampered instruction word in lookup ✓");
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn combined() {
		run_combined();
	}
}
