//! Seventh validation slice: CONDITIONAL BRANCH — RISC-V `beq`.
//!
//! `beq rs1, rs2, target`: if rs1 == rs2, pc += target_offset; else pc += 4.
//! The zkVM must prove the branch condition AND the conditional PC update.
//!
//! We prove the PC transition for a beq on 8-bit values:
//!   taken = (rs1 == rs2), expressed in char-2 as a bitwise AND of
//!           NOT(XOR) bits:  taken = Π_i (1 ⊕ rs1_i ⊕ rs2_i).
//!   pc_next = taken · target + (1+taken) · (pc+4)   (boolean MUX).
//!   (target is the absolute branch target; standard RISC-V uses a relative
//!    imm — we model the absolute form for clarity of the MUX.)
//!
//! We drive TWO concrete programs:
//!   (a) rs1 == rs2  → taken, pc jumps to target,
//!   (b) rs1 != rs2  → not taken, pc = pc+4.
//! Both are committed as separate states and verified. Soundness: tampering
//! the branch target must be rejected.

use binius_field::{Field, Ghash128b as B128, arch::OptimalPackedB128};
use binius_hash::StdHashSuite;
use binius_spartan_frontend::{
	circuit_builder::{CircuitBuilder, ConstraintBuilder, InstanceGenerator, WitnessGenerator},
	compiler::compile,
	constraint_system::{ConstraintWire, Witness},
};
use binius_spartan_prover::Prover;
use binius_spartan_verifier::{Verifier, config::StdChallenger};
use binius_transcript::ProverTranscript;
use rand::{SeedableRng, rngs::StdRng};

const BITS: usize = 8;
const PC_INC: u64 = 4;
const OPCODE_BEQ: u64 = 0x63;
const FUNCT3_BEQ: u64 = 0x0;

type F = B128;
type P = OptimalPackedB128;

fn to_bits(val: u64, nbits: usize) -> Vec<B128> {
	(0..nbits).map(|i| B128::new(((val >> i) & 1) as u128)).collect()
}

/// Full-adder bit: (sum, carry_out).
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

/// Drive one beq step: decode, compute taken, conditional pc update.
/// Layout (matching allocation): [inst(32) | rs1(8) | rs2(8) | pc(8) | target(8) | pc_next(8)].
fn drive_branch<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	inst: &[B::Wire],
	rs1: &[B::Wire],
	rs2: &[B::Wire],
	pc: &[B::Wire],
	target: &[B::Wire],
	pc_next: &[B::Wire],
) {
	// boolean-ness of all inputs
	for w in inst.iter().chain(rs1.iter()).chain(rs2.iter()).chain(pc.iter()).chain(target.iter()).chain(pc_next.iter()) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	// DECODE opcode = inst[6:0]
	for (i, &c) in to_bits(OPCODE_BEQ, 7).iter().enumerate() {
		let cw = b.constant(c);
		let d = b.add(inst[i], cw);
		b.assert_zero(d);
	}
	// DECODE funct3 = inst[14:12]
	for (i, &c) in to_bits(FUNCT3_BEQ, 3).iter().enumerate() {
		let cw = b.constant(c);
		let d = b.add(inst[12 + i], cw);
		b.assert_zero(d);
	}
	// taken = Π_i (1 ⊕ rs1_i ⊕ rs2_i): equality iff all bits equal.
	// Compute per-bit eq_i = 1 ^ rs1_i ^ rs2_i, then multiply into taken.
	// Use a product tree (log-depth).
	let mut eq: Vec<B::Wire> = Vec::with_capacity(BITS);
	for i in 0..BITS {
		let r1 = rs1[i];
		let r2 = rs2[i];
		let x = b.add(r1, r2); // rs1_i ^ rs2_i
		let one = b.constant(B128::ONE);
		let eqi = b.add(x, one); // 1 ^ rs1_i ^ rs2_i
		eq.push(eqi);
	}
	// product tree reduction
	let taken = product_tree(b, &eq);
	// boolean-ness of taken (derived, guaranteed by product of bits but assert anyway)
	binius_spartan_frontend::circuits::assert_is_bit(b, taken);

	// pc+4 integer addition (full-adder chain)
	let pc_inc = to_bits(PC_INC, BITS);
	let mut cin = b.constant(B128::ZERO);
	let mut pc_plus: Vec<B::Wire> = Vec::with_capacity(BITS);
	for i in 0..BITS {
		let ib = b.constant(pc_inc[i]);
		let pc_i = pc[i];
		let (sum, cout) = fa(b, pc_i, ib, cin);
		pc_plus.push(sum);
		cin = cout;
	}
	// pc_next = taken·target + (1+taken)·(pc+4), bitwise boolean MUX.
	let not_taken = {
		let t = taken;
		let one = b.constant(B128::ONE);
		b.add(t, one) // 1 ^ taken
	};
	for i in 0..BITS {
		// taken·target[i]
		let tt = b.mul(taken, target[i]);
		// (1+taken)·(pc+4)[i]
		let nt = b.mul(not_taken, pc_plus[i]);
		let mux = b.add(tt, nt);
		b.assert_eq(mux, pc_next[i]);
	}
}

/// Product tree reduction of a vector of boolean wires into a single wire.
fn product_tree<B: CircuitBuilder<Field = B128>>(b: &mut B, vals: &[B::Wire]) -> B::Wire {
	if vals.is_empty() {
		return b.constant(B128::ONE);
	}
	let mut cur: Vec<B::Wire> = vals.to_vec();
	while cur.len() > 1 {
		let mut nxt = Vec::new();
		let mut i = 0;
		while i < cur.len() {
			if i + 1 < cur.len() {
				let a = cur[i];
				let bb = cur[i + 1];
				nxt.push(b.mul(a, bb));
				i += 2;
			} else {
				nxt.push(cur[i]);
				i += 1;
			}
		}
		cur = nxt;
	}
	cur[0]
}

fn main() {
	// Two concrete branch cases at pc=0x20.
	// Case (a): rs1 == rs2 == 0x5A → taken → jump to target 0x40.
	let pc_a: u64 = 0x20;
	let target_a: u64 = 0x40;
	let rs1_a: u64 = 0x5A;
	let rs2_a: u64 = 0x5A;
	let exp_pc_a = target_a;
	// Case (b): rs1=0x5A, rs2=0xAB → not taken → pc+4.
	let pc_b: u64 = 0x20;
	let target_b: u64 = 0x40;
	let rs1_b: u64 = 0x5A;
	let rs2_b: u64 = 0xAB;
	let exp_pc_b = pc_b + PC_INC;

	println!("branch cases:");
	println!("  (a) beq {rs1_a:#x}, {rs2_a:#x} → taken → pc {pc_a:#x} -> {exp_pc_a:#x}");
	println!("  (b) beq {rs1_b:#x}, {rs2_b:#x} → not taken → pc {pc_b:#x} -> {exp_pc_b:#x}");

	// ---- Allocate ONCE, segmented layout: [inst(32) | rs1(8) | rs2(8) | pc(8) | target(8) | pc_next(8)] ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let inst_w: Vec<ConstraintWire> = (0..32).map(|_| cb.alloc_inout()).collect();
	let rs1_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let rs2_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let pc_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let tgt_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	let pcn_w: Vec<ConstraintWire> = (0..BITS).map(|_| cb.alloc_inout()).collect();
	drive_branch(&mut cb, &inst_w, &rs1_w, &rs2_w, &pc_w, &tgt_w, &pcn_w);
	let (cs, layout) = compile(cb);

	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	// ---- Witness for case (a): taken ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	// simplest: build beq with rs1=x5(5),rs2=x6(6),imm=0, funct3=0
	let beq: u64 = (6u64 << 20) | (5u64 << 15) | (0u64 << 12) | 0x63;
	let iw = to_bits(beq, 32);
	let r1b = to_bits(rs1_a, BITS);
	let r2b = to_bits(rs2_a, BITS);
	let pcb = to_bits(pc_a, BITS);
	let tgtb = to_bits(target_a, BITS);
	let pcnb = to_bits(exp_pc_a, BITS);
	for k in 0..32 { wg.write_inout(inst_w[k], iw[k]); }
	for k in 0..BITS { wg.write_inout(rs1_w[k], r1b[k]); wg.write_inout(rs2_w[k], r2b[k]); wg.write_inout(pc_w[k], pcb[k]); wg.write_inout(tgt_w[k], tgtb[k]); wg.write_inout(pcn_w[k], pcnb[k]); }
	let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..32).map(|k| wg.write_inout(inst_w[k], iw[k])).collect();
	let wr1: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(rs1_w[k], r1b[k])).collect();
	let wr2: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(rs2_w[k], r2b[k])).collect();
	let wpc: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[k], pcb[k])).collect();
	let wtgt: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(tgt_w[k], tgtb[k])).collect();
	let wpcn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pcn_w[k], pcnb[k])).collect();
	drive_branch(&mut wg, &wi, &wr1, &wr2, &wpc, &wtgt, &wpcn);
	let witness_a = wg.build().expect("witness a");

	// ---- Witness for case (b): not taken ----
	let mut wg2 = WitnessGenerator::new(&layout);
	let r1b2 = to_bits(rs1_b, BITS);
	let r2b2 = to_bits(rs2_b, BITS);
	let pcb2 = to_bits(pc_b, BITS);
	let tgtb2 = to_bits(target_b, BITS);
	let pcnb2 = to_bits(exp_pc_b, BITS);
	for k in 0..32 { wg2.write_inout(inst_w[k], iw[k]); }
	for k in 0..BITS { wg2.write_inout(rs1_w[k], r1b2[k]); wg2.write_inout(rs2_w[k], r2b2[k]); wg2.write_inout(pc_w[k], pcb2[k]); wg2.write_inout(tgt_w[k], tgtb2[k]); wg2.write_inout(pcn_w[k], pcnb2[k]); }
	let wi2: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..32).map(|k| wg2.write_inout(inst_w[k], iw[k])).collect();
	let wr12: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg2.write_inout(rs1_w[k], r1b2[k])).collect();
	let wr22: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg2.write_inout(rs2_w[k], r2b2[k])).collect();
	let wpc2: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg2.write_inout(pc_w[k], pcb2[k])).collect();
	let wtgt2: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg2.write_inout(tgt_w[k], tgtb2[k])).collect();
	let wpcn2: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg2.write_inout(pcn_w[k], pcnb2[k])).collect();
	drive_branch(&mut wg2, &wi2, &wr12, &wr22, &wpc2, &wtgt2, &wpcn2);
	let witness_b = wg2.build().expect("witness b");

	// ---- Instance (verifier recompute) for case (a) ----
	let mut ig = InstanceGenerator::new(&layout);
	let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..32).map(|k| ig.write_inout(inst_w[k], iw[k])).collect();
	let mr1: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(rs1_w[k], r1b[k])).collect();
	let mr2: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(rs2_w[k], r2b[k])).collect();
	let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[k], pcb[k])).collect();
	let mt: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(tgt_w[k], tgtb[k])).collect();
	let mpcn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pcn_w[k], pcnb[k])).collect();
	drive_branch(&mut ig, &mi, &mr1, &mr2, &mp, &mt, &mpcn);
	let public = ig.build();

	// ---- Prove case (a) & verify ----
	cs.validate(&witness_a);
	let t_prove0 = std::time::Instant::now();
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_a, &mut rng, &mut pt).expect("prove a");
	let t_prove = t_prove0.elapsed();
	let t_verify0 = std::time::Instant::now();
	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("verify a");
	vt.finalize().expect("finalize a");
	let t_verify = t_verify0.elapsed();

	// ---- Prove case (b) & verify against the same constraint system (recompute public) ----
	// Rebuild instance for case b
	let mut ig2 = InstanceGenerator::new(&layout);
	let mib: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..32).map(|k| ig2.write_inout(inst_w[k], iw[k])).collect();
	let mrb: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig2.write_inout(rs1_w[k], r1b2[k])).collect();
	let mrb2: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig2.write_inout(rs2_w[k], r2b2[k])).collect();
	let mpb: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig2.write_inout(pc_w[k], pcb2[k])).collect();
	let mtb: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig2.write_inout(tgt_w[k], tgtb2[k])).collect();
	let mpcnb: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig2.write_inout(pcn_w[k], pcnb2[k])).collect();
	drive_branch(&mut ig2, &mib, &mrb, &mrb2, &mpb, &mtb, &mpcnb);
	let public_b = ig2.build();
	cs.validate(&witness_b);
	let mut ptb = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_b, &mut rng, &mut ptb).expect("prove b");
	let mut vtb = ptb.into_verifier();
	verifier.verify(&public_b, &mut vtb).expect("verify b");
	vtb.finalize().expect("finalize b");

	println!("✅ Conditional branch (beq) proved & verified over binary field (Spartan)");
	println!("   taken case:  pc {pc_a:#x} -> {exp_pc_a:#x} (jump to target)");
	println!("   not-taken:   pc {pc_b:#x} -> {exp_pc_b:#x} (pc+4)");
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   timing (taken case): prove={:?} verify={:?}", t_prove, t_verify);

	// ---- Soundness: tamper the branch target (wrong pc_next) ----
	// Segmented layout: [inst(32)|rs1(8)|rs2(8)|pc(8)|target(8)|pc_next(8)].
	let n_const = layout.n_constants();
	let mut bad = witness_a.public().to_vec();
	let pcnext_off = n_const as usize + 32 + BITS + BITS + BITS + BITS;
	bad[pcnext_off] += B128::ONE; // corrupt pc_next low bit
	let tampered = Witness::new(bad, witness_a.precommit().to_vec(), witness_a.private().to_vec());
	let mut bt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&tampered, &mut rng, &mut bt).expect("prove tampered");
	let mut bv = bt.into_verifier();
	let rejected = verifier.verify(&public, &mut bv).is_err();
	assert!(rejected, "verifier MUST reject a tampered branch result");
	println!("   soundness: verifier REJECTED a tampered branch target ✓");
}