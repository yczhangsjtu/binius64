//! Milestone M3: WORD-VM — a minimal but honest single-cycle state machine (v2,
//! binding-rework: logup* claims are rebuilt from the circuit inout, not a native trace).
//!
//! Closes the two semantic gaps left by M1/M2:
//!   - M1: read==write binding (logup* write-log W[(reg,ver)]) had a NATIVE version;
//!   - M2: fetched word did not drive execution.
//! v2 additionally binds the logup* layer to the circuit's public inout so a cheater
//! cannot run program A on the circuit while checking a fabricated trace B on logup*.

use binius_compute::GlobalAllocator;
use binius_core::word::Word;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_frontend::{Circuit, CircuitBuilder, CircuitStat, Wire};
use binius_hash::StdHashSuite;
use binius_ip::logup_star;
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_prover::Prover as WordProver;
use binius_transcript::ProverTranscript;
use binius_verifier::{Verifier as WordVerifier, config::StdChallenger};

const NREG: usize = 8;
const VER_MAX: usize = 4; // max version slots (ver0 initial + up to 3 writes)
const PC_START: u64 = 0x00;
const HALT_ADDR: u64 = 0x14;
const OP_ADDI: u64 = 0x13;
const OP_ADD: u64 = 0x33;
const OP_BEQ: u64 = 0x63;

// logup* field scalars (module-level so nested helpers see them).
type LF = OptimalB128;
type LP = OptimalPackedB128;

fn enc_addi(rd: u64, rs1: u64, imm7: u64) -> u64 {
	OP_ADDI | (rd << 7) | (0 << 12) | (rs1 << 15) | ((imm7 & 0x7f) << 25)
}
fn enc_add(rd: u64, rs1: u64, rs2: u64) -> u64 {
	OP_ADD | (rd << 7) | (0 << 12) | (rs1 << 15) | (rs2 << 20)
}
fn enc_beq(rs1: u64, rs2: u64, off7: u64) -> u64 {
	OP_BEQ | (0 << 7) | (0 << 12) | (rs1 << 15) | (rs2 << 20) | ((off7 & 0x7f) << 25)
}

#[inline]
fn sgn7(v: u64) -> i64 {
	if v & 0x40 != 0 { (v as i64) - 128 } else { v as i64 }
}

fn fetch_word(addr: u64) -> u64 {
	match addr {
		0x00 => enc_addi(1, 1, 1),       // addi x1, x1, 1
		0x04 => enc_add(2, 2, 1),        // add  x2, x2, x1
		0x08 => enc_addi(3, 3, 1),       // addi x3, x3, 1
		0x0c => enc_beq(3, 4, 8),        // beq  x3, x4, +8  (exit)
		0x10 => enc_beq(5, 5, 0x70),     // beq  x5, x5, -16 (loop back)
		0x14 => enc_addi(0, 0, 0),       // addi x0, x0, 0  (halt)
		_ => 0,
	}
}

fn decode_fields(inst: u64) -> (u64, usize, usize, usize, u64) {
	(inst & 0x7f, ((inst >> 7) & 0x1f) as usize, ((inst >> 15) & 0x1f) as usize,
	 ((inst >> 20) & 0x1f) as usize, (inst >> 25) & 0x7f)
}

#[derive(Clone, Copy, Debug)]
struct RegAccess { reg: usize, ver: usize, val: u64 }
#[derive(Clone, Debug)]
struct Cycle {
	pc: u64,
	inst: u64,
	reads: Vec<RegAccess>,
	write: Option<RegAccess>,
	ver: [usize; NREG], // version array BEFORE this cycle's write
}
struct Trace { cycles: Vec<Cycle>, final_regs: [u64; NREG] }

fn run_program(init: [u64; NREG], overrides: &[(u64, u64)]) -> Trace {
	let mut regs = init;
	let mut ver = [0usize; NREG];
	let mut cycles = Vec::new();
	let mut pc = PC_START;
	let mut guard = 0;
	let fetch = |addr: u64| -> u64 {
		for &(a, w) in overrides {
			if a == addr { return w; }
		}
		fetch_word(addr)
	};
	loop {
		guard += 1;
		if guard > 64 { panic!("runaway execution"); }
		let inst = fetch(pc);
		let (opcode, rd, rs1, rs2, imm) = decode_fields(inst);
		let mut reads = Vec::new();
		reads.push(RegAccess { reg: rs1, ver: ver[rs1], val: regs[rs1] });
		reads.push(RegAccess { reg: rs2, ver: ver[rs2], val: regs[rs2] });
		let cycle_ver = ver;
		let write = match opcode {
			OP_ADDI => { let v = regs[rs1].wrapping_add(sgn7(imm) as u64); ver[rd] += 1; regs[rd] = v;
				Some(RegAccess { reg: rd, ver: ver[rd], val: v }) }
			OP_ADD => { let v = regs[rs1].wrapping_add(regs[rs2]); ver[rd] += 1; regs[rd] = v;
				Some(RegAccess { reg: rd, ver: ver[rd], val: v }) }
			_ => None,
		};
		let next_pc = if opcode == OP_BEQ && regs[rs1] == regs[rs2] {
			(pc as i64 + sgn7(imm)) as u64
		} else {
			pc.wrapping_add(4)
		};
		cycles.push(Cycle { pc, inst, reads, write, ver: cycle_ver });
		if pc == HALT_ADDR { break; }
		pc = next_pc;
	}
	Trace { cycles, final_regs: regs }
}

fn mux8(b: &CircuitBuilder, inputs: &[Wire], sel: Wire) -> Wire {
	let n = inputs.len();
	if n == 0 { return b.add_constant_64(0); }
	let num_sel_bits = usize::BITS - (n - 1).leading_zeros();
	let mut level = inputs.to_vec();
	for bit in 0..num_sel_bits {
		let sel_bit = b.shl(sel, (63 - bit) as u32);
		let next: Vec<Wire> = level.chunks(2)
			.map(|p| if let [l, r] = p { b.select(sel_bit, *r, *l) } else { p[0] })
			.collect();
		level = next;
	}
	level[0]
}

#[derive(Clone)]
struct InoutRefs {
	inst: Vec<Wire>,
	pc: Vec<Wire>,
	// per-cycle read events ×2 (rs1, rs2): (reg, ver, val) bound to the value/version chain.
	rd1_reg: Vec<Wire>,
	rd1_ver: Vec<Wire>,
	rd1_val: Vec<Wire>,
	rd2_reg: Vec<Wire>,
	rd2_ver: Vec<Wire>,
	rd2_val: Vec<Wire>,
	// per-cycle write event ×1: (reg, ver, val, is_write).
	wr_reg: Vec<Wire>,
	wr_ver: Vec<Wire>,
	wr_val: Vec<Wire>,
	wr_iswrite: Vec<Wire>,
	init_regs: Vec<Wire>,
	final_regs: Vec<Wire>,
}

// flat inout layout — the builder allocates in BLOCKS (all inst, then all pc, then
// each read/write field, then init/final). Total = 12*t_len + 2*NREG. The verifier
// rebuilds the logup* claims from inout_words via these block offsets.
fn io_inst(_t_len: usize, t: usize) -> usize { t }
fn io_pc(t_len: usize, t: usize) -> usize { t_len + t }
fn io_rd1_reg(t_len: usize, t: usize) -> usize { 2 * t_len + t }
fn io_rd1_ver(t_len: usize, t: usize) -> usize { 3 * t_len + t }
fn io_rd1_val(t_len: usize, t: usize) -> usize { 4 * t_len + t }
fn io_rd2_reg(t_len: usize, t: usize) -> usize { 5 * t_len + t }
fn io_rd2_ver(t_len: usize, t: usize) -> usize { 6 * t_len + t }
fn io_rd2_val(t_len: usize, t: usize) -> usize { 7 * t_len + t }
fn io_wr_reg(t_len: usize, t: usize) -> usize { 8 * t_len + t }
fn io_wr_ver(t_len: usize, t: usize) -> usize { 9 * t_len + t }
fn io_wr_val(t_len: usize, t: usize) -> usize { 10 * t_len + t }
fn io_wr_iswrite(t_len: usize, t: usize) -> usize { 11 * t_len + t }
fn io_final(t_len: usize, r: usize) -> usize { 12 * t_len + NREG + r }

fn build_circuit(trace: &Trace) -> (Circuit, InoutRefs) {
	let b = CircuitBuilder::new();
	let t_len = trace.cycles.len();
	let iref = InoutRefs {
		inst: (0..t_len).map(|_| b.add_inout()).collect(),
		pc: (0..t_len).map(|_| b.add_inout()).collect(),
		rd1_reg: (0..t_len).map(|_| b.add_inout()).collect(),
		rd1_ver: (0..t_len).map(|_| b.add_inout()).collect(),
		rd1_val: (0..t_len).map(|_| b.add_inout()).collect(),
		rd2_reg: (0..t_len).map(|_| b.add_inout()).collect(),
		rd2_ver: (0..t_len).map(|_| b.add_inout()).collect(),
		rd2_val: (0..t_len).map(|_| b.add_inout()).collect(),
		wr_reg: (0..t_len).map(|_| b.add_inout()).collect(),
		wr_ver: (0..t_len).map(|_| b.add_inout()).collect(),
		wr_val: (0..t_len).map(|_| b.add_inout()).collect(),
		wr_iswrite: (0..t_len).map(|_| b.add_inout()).collect(),
		init_regs: (0..NREG).map(|_| b.add_inout()).collect(),
		final_regs: (0..NREG).map(|_| b.add_inout()).collect(),
	};
	let zero = b.add_constant_64(0);
	let one = b.add_constant_64(1);

	// value chain: reg_cur[r] = current value of register r; starts at init inouts.
	let mut reg_cur: Vec<Wire> = iref.init_regs.clone();
	// version chain: ver[t][r] = #writes to r before cycle t; ver[0][r] = 0.
	let mut ver: Vec<Vec<Wire>> = vec![(0..NREG).map(|_| zero).collect()];

	for t in 0..t_len {
		let inst_w = iref.inst[t];
		// ---- WORD-GATE decode ----
		let opcode = b.band(inst_w, b.add_constant_64(0x7f));
		let rd = b.band(b.srl32(inst_w, 7), b.add_constant_64(0x1f));
		let funct3 = b.band(b.srl32(inst_w, 12), b.add_constant_64(0x7));
		let rs1 = b.band(b.srl32(inst_w, 15), b.add_constant_64(0x1f));
		let rs2 = b.band(b.srl32(inst_w, 20), b.add_constant_64(0x1f));
		let imm7 = b.band(b.srl32(inst_w, 25), b.add_constant_64(0x7f));

		let is_addi = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_ADDI)), b.icmp_eq(funct3, zero));
		let is_add = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_ADD)), b.icmp_eq(funct3, zero));
		let is_beq = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_BEQ)), b.icmp_eq(funct3, zero));
		let is_alu_write = b.bor(is_addi, is_add);

		// ---- read register operands (dynamic index from the decoded word) ----
		let a_val = mux8(&b, &reg_cur, rs1);
		let b_val = mux8(&b, &reg_cur, rs2);
		let b_operand = b.select(is_addi, imm7, b_val);
		let sum = b.iadd_32(a_val, b_operand);

		// ---- R1: bind read/write event inouts to the actual circuit wires ----
		let rd1_ver_w = mux8(&b, &ver[t], rs1);
		let rd2_ver_w = mux8(&b, &ver[t], rs2);
		let rd_ver_w = mux8(&b, &ver[t], rd);
		let wr_ver_w = b.iadd(rd_ver_w, b.select(is_alu_write, one, zero)).0; // ver[t][rd]+is_write
		let is_write_01 = b.select(is_alu_write, one, zero);
		b.assert_eq(format!("rd1.reg==rs1[{t}]"), iref.rd1_reg[t], rs1);
		b.assert_eq(format!("rd1.ver==ver[{t}]"), iref.rd1_ver[t], rd1_ver_w);
		b.assert_eq(format!("rd1.val==a[{t}]"), iref.rd1_val[t], a_val);
		b.assert_eq(format!("rd2.reg==rs2[{t}]"), iref.rd2_reg[t], rs2);
		b.assert_eq(format!("rd2.ver==ver[{t}]"), iref.rd2_ver[t], rd2_ver_w);
		b.assert_eq(format!("rd2.val==b[{t}]"), iref.rd2_val[t], b_val);
		b.assert_eq(format!("wr.reg==rd[{t}]"), iref.wr_reg[t], rd);
		b.assert_eq(format!("wr.ver==ver+iswr[{t}]"), iref.wr_ver[t], wr_ver_w);
		b.assert_eq(format!("wr.val==sum[{t}]"), iref.wr_val[t], sum);
		b.assert_eq(format!("wr.iswrite[{t}]"), iref.wr_iswrite[t], is_write_01);

		// ---- version chain + value chain write-back ----
		let mut next_reg = Vec::with_capacity(NREG);
		let mut next_ver = Vec::with_capacity(NREG);
		for r in 0..NREG {
			let rword = b.add_constant_64(r as u64);
			let is_rd_r = b.band(b.icmp_eq(rd, rword), is_alu_write);
			next_reg.push(b.select(is_rd_r, sum, reg_cur[r]));
			next_ver.push(b.iadd(ver[t][r], b.select(is_rd_r, one, zero)).0);
		}
		reg_cur = next_reg;
		ver.push(next_ver);

		// ---- PC advance ----
		let pc_plus4 = b.iadd(iref.pc[t], b.add_constant_64(4)).0;
		let beq_taken = b.band(is_beq, b.icmp_eq(a_val, b_val));
		let imm_sext = b.select(
			b.icmp_eq(b.band(imm7, b.add_constant_64(0x40)), zero),
			imm7,
			b.bxor(imm7, b.add_constant_64(0xFFFF_FFFF_FFFF_FF80)),
		);
		let target = b.iadd(iref.pc[t], imm_sext).0;
		let next_pc_w = b.select(beq_taken, target, pc_plus4);
		if t == 0 {
			b.assert_eq("pc[0]==start", iref.pc[0], b.add_constant_64(PC_START));
		}
		if t + 1 < t_len {
			b.assert_eq(format!("pc[{}]==next", t + 1), iref.pc[t + 1], next_pc_w);
		} else {
			b.assert_eq("last_pc==halt", iref.pc[t], b.add_constant_64(HALT_ADDR));
		}
	}

	// ---- final register outputs ----
	for r in 0..NREG {
		b.assert_eq(format!("final_reg[{r}]"), iref.final_regs[r], reg_cur[r]);
	}

	let circuit = b.build();
	(circuit, iref)
}

fn build_write_log(init: &[u64; NREG], trace: &Trace) -> Vec<u64> {
	let mut w = vec![0u64; NREG * VER_MAX];
	for r in 0..NREG {
		w[r * VER_MAX + 0] = init[r];
	}
	for c in &trace.cycles {
		if let Some(wr) = &c.write {
			w[wr.reg * VER_MAX + wr.ver] = wr.val;
		}
	}
	w
}

fn build_fetch_prog() -> Vec<u64> {
	let m_fetch = 5;
	let mut prog = vec![0u64; 1usize << m_fetch];
	for mm in [0x00u64, 0x04, 0x08, 0x0c, 0x10, 0x14].iter() {
		prog[*mm as usize] = fetch_word(*mm);
	}
	prog
}

/// Native ALU sum for a cycle (matching the circuit's `iadd_32(a_val, b_operand)`).
fn cycle_sum(c: &Cycle) -> u64 {
	let op = c.inst & 0x7f;
	let a = c.reads[0].val;
	let b = c.reads[1].val;
	let imm = (c.inst >> 25) & 0x7f;
	let (x, y) = if op == OP_ADDI { (a, sgn7(imm) as u64) } else { (a, b) };
	(x as u32).wrapping_add(y as u32) as u64
}

/// Rebuild every logup* claim (fetch + register read/write) from the flat `inout_words`
/// slice — NOT from a native trace. This binds the logup* layer to the same public
/// statement the circuit commits to (Task §3.2).
fn claims_from_inout(inout_words: &[Word], t_len: usize) -> (Vec<Vec<usize>>, Vec<u64>, Vec<Vec<usize>>, Vec<u64>) {
	let mut fetch_idxs: Vec<Vec<usize>> = Vec::new();
	let mut fetch_claims: Vec<u64> = Vec::new();
	let mut rd_idxs: Vec<Vec<usize>> = Vec::new();
	let mut rd_claims: Vec<u64> = Vec::new();
	for t in 0..t_len {
		let inst = inout_words[io_inst(t_len, t)].0;
		let pc = inout_words[io_pc(t_len, t)].0;
		fetch_idxs.push(vec![pc as usize]);
		fetch_claims.push(inst);
		// read rs1
		rd_idxs.push(vec![(inout_words[io_rd1_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_rd1_ver(t_len, t)].0 as usize]);
		rd_claims.push(inout_words[io_rd1_val(t_len, t)].0);
		// read rs2
		rd_idxs.push(vec![(inout_words[io_rd2_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_rd2_ver(t_len, t)].0 as usize]);
		rd_claims.push(inout_words[io_rd2_val(t_len, t)].0);
		// write (only when is_write != 0)
		if inout_words[io_wr_iswrite(t_len, t)].0 != 0 {
			rd_idxs.push(vec![(inout_words[io_wr_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_wr_ver(t_len, t)].0 as usize]);
			rd_claims.push(inout_words[io_wr_val(t_len, t)].0);
		}
	}
	(fetch_idxs, fetch_claims, rd_idxs, rd_claims)
}

/// Run the whole machine for `init` / `overrides` and verify BOTH layers. Returns
/// (frontend-verify-ok, logup*-verify-ok, stat, t_len, inout_words). Used by soundness
/// case 3 (a program whose instruction word is not in the program table: circuit OK,
/// logup* fetch rejects).
fn run_machine(init: [u64; NREG], overrides: &[(u64, u64)]) -> (bool, bool, CircuitStat, usize, Vec<Word>) {
	let trace = run_program(init, overrides);
	let t_len = trace.cycles.len();
	let (circuit, iref) = build_circuit(&trace);
	let stat = CircuitStat::collect(&circuit);
	let cs = circuit.constraint_system().clone();
	let mut w = circuit.new_witness_filler();
	for t in 0..t_len {
		let c = &trace.cycles[t];
		w[iref.inst[t]] = Word(c.inst);
		w[iref.pc[t]] = Word(c.pc);
		w[iref.rd1_reg[t]] = Word(c.reads[0].reg as u64);
		w[iref.rd1_ver[t]] = Word(c.reads[0].ver as u64);
		w[iref.rd1_val[t]] = Word(c.reads[0].val);
		w[iref.rd2_reg[t]] = Word(c.reads[1].reg as u64);
		w[iref.rd2_ver[t]] = Word(c.reads[1].ver as u64);
		w[iref.rd2_val[t]] = Word(c.reads[1].val);
		let rd_dec = ((c.inst >> 7) & 0x1f) as usize;
		w[iref.wr_reg[t]] = Word(rd_dec as u64);
		if let Some(wr) = &c.write {
			w[iref.wr_ver[t]] = Word(wr.ver as u64);
			w[iref.wr_val[t]] = Word(wr.val);
			w[iref.wr_iswrite[t]] = Word(1);
		} else {
			w[iref.wr_ver[t]] = Word(c.ver[rd_dec] as u64);
			w[iref.wr_val[t]] = Word(cycle_sum(c));
			w[iref.wr_iswrite[t]] = Word(0);
		}
	}
	for r in 0..NREG {
		w[iref.init_regs[r]] = Word(init[r]);
		w[iref.final_regs[r]] = Word(trace.final_regs[r]);
	}
	circuit.populate_wire_witness(&mut w).expect("word-vm witness fill");
	let witness_vec = w.into_value_vec();
	let _native_ok = cs.verify(&witness_vec).is_ok();
	let inout_words = witness_vec.inout().to_vec();
	let (fetch_idxs, fetch_claims, rd_idxs, rd_claims) = claims_from_inout(&inout_words, t_len);

	let prog = build_fetch_prog();
	let fetch_table = FieldBuffer::from_values(&prog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let fetch_view = fetch_table.as_view();
	let wlog = build_write_log(&init, &trace);
	let w_table = FieldBuffer::from_values(&wlog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let w_view = w_table.as_view();
	let m_fetch: usize = 5;
	let m_w = (usize::BITS - (NREG * VER_MAX - 1).leading_zeros()) as usize;

	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<OptimalPackedB128, StdHashSuite>::setup(verifier.clone()).expect("prover setup");
	let alloc = GlobalAllocator;
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");
	let fetch_lookers: Vec<Looker<LF>> = (0..fetch_idxs.len())
		.map(|i| Looker { index: &fetch_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(fetch_claims[i] as u128) })
		.collect();
	let rd_lookers: Vec<Looker<LF>> = (0..rd_idxs.len())
		.map(|i| Looker { index: &rd_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(rd_claims[i] as u128) })
		.collect();
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let _prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(&alloc, gamma, [
		binius_ip_prover::logup_star::TableLookup { table: fetch_view, lookers: fetch_lookers },
		binius_ip_prover::logup_star::TableLookup { table: w_view, lookers: rd_lookers },
	], &mut pt);

	let mut vt = pt.into_verifier();
	let circuit_ok = verifier.verify(&inout_words, &mut vt).is_ok();
	let verifier_gamma = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	let logup_ok = if verifier_gamma == gamma {
		logup_star::verify_reduction::<LF, _>(&verifier_gamma, [
			logup_star::TableLookup { n_vars: m_fetch,
				lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
			logup_star::TableLookup { n_vars: m_w,
				lookers: rd_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
		], &mut vt).is_ok()
	} else {
		false
	};
	(circuit_ok, logup_ok, stat, t_len, inout_words)
}

pub fn run_word_vm() {
	let init: [u64; NREG] = [0, 0, 0, 0, 2, 0, 0, 0]; // x0..x7; x4 = limit = 2
	let trace = run_program(init, &[]);
	let t_len = trace.cycles.len();
	let (circuit, iref) = build_circuit(&trace);
	let stat = CircuitStat::collect(&circuit);
	let cs = circuit.constraint_system().clone();
	println!("== M3 (WORD-VM): single-cycle state machine (binding-rework v2) ==");
	println!("   program: addi x1; add x2,x2,x1; addi x3; beq x3,x4,+8; beq x5,x5,-16; halt");
	println!("   cycles={t_len} final x1={} x2={} x3={} (cross-check native)",
		trace.final_regs[1], trace.final_regs[2], trace.final_regs[3]);
	println!("   constraints: ZERO={} AND={} IMUL={} BMUL={} (gates={})",
		stat.n_zero_constraints, stat.n_and_constraints, stat.n_imul_constraints, stat.n_bmul_constraints, stat.n_gates);

	// ---- honest witness fill (read/write events are public inouts, R1) ----
	let mut w = circuit.new_witness_filler();
	for t in 0..t_len {
		let c = &trace.cycles[t];
		w[iref.inst[t]] = Word(c.inst);
		w[iref.pc[t]] = Word(c.pc);
		w[iref.rd1_reg[t]] = Word(c.reads[0].reg as u64);
		w[iref.rd1_ver[t]] = Word(c.reads[0].ver as u64);
		w[iref.rd1_val[t]] = Word(c.reads[0].val);
		w[iref.rd2_reg[t]] = Word(c.reads[1].reg as u64);
		w[iref.rd2_ver[t]] = Word(c.reads[1].ver as u64);
		w[iref.rd2_val[t]] = Word(c.reads[1].val);
		let rd_dec = ((c.inst >> 7) & 0x1f) as usize;
		w[iref.wr_reg[t]] = Word(rd_dec as u64);
		if let Some(wr) = &c.write {
			w[iref.wr_ver[t]] = Word(wr.ver as u64);
			w[iref.wr_val[t]] = Word(wr.val);
			w[iref.wr_iswrite[t]] = Word(1);
		} else {
			w[iref.wr_ver[t]] = Word(c.ver[rd_dec] as u64);
			w[iref.wr_val[t]] = Word(cycle_sum(c));
			w[iref.wr_iswrite[t]] = Word(0);
		}
	}
	for r in 0..NREG {
		w[iref.init_regs[r]] = Word(init[r]);
		w[iref.final_regs[r]] = Word(trace.final_regs[r]);
	}
	circuit.populate_wire_witness(&mut w).expect("word-vm witness fill");
	let witness_vec = w.into_value_vec();
	assert!(cs.verify(&witness_vec).is_ok(), "word-vm constraints satisfied natively");
	let inout_words = witness_vec.inout().to_vec();

	// ---- logup* claims rebuilt from the SAME public statement (R2), not a native trace ----
	let (fetch_idxs, fetch_claims, rd_idxs, rd_claims) = claims_from_inout(&inout_words, t_len);
	let prog = build_fetch_prog();
	let fetch_table = FieldBuffer::from_values(&prog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let fetch_view = fetch_table.as_view();
	let wlog = build_write_log(&init, &trace);
	let w_table = FieldBuffer::from_values(&wlog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let w_view = w_table.as_view();
	let m_fetch: usize = 5;
	let m_w = (usize::BITS - (NREG * VER_MAX - 1).leading_zeros()) as usize;

	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<OptimalPackedB128, StdHashSuite>::setup(verifier.clone()).expect("prover setup");
	let alloc = GlobalAllocator;
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");
	let fetch_lookers: Vec<Looker<LF>> = (0..fetch_idxs.len())
		.map(|i| Looker { index: &fetch_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(fetch_claims[i] as u128) })
		.collect();
	let rd_lookers: Vec<Looker<LF>> = (0..rd_idxs.len())
		.map(|i| Looker { index: &rd_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(rd_claims[i] as u128) })
		.collect();
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(&alloc, gamma, [
		binius_ip_prover::logup_star::TableLookup { table: fetch_view, lookers: fetch_lookers },
		binius_ip_prover::logup_star::TableLookup { table: w_view, lookers: rd_lookers },
	], &mut pt);

	let mut vt = pt.into_verifier();
	assert!(verifier.verify(&inout_words, &mut vt).is_ok(), "word-vm frontend verify");
	let verifier_gamma = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "both sides draw same lookup challenge");
	let verifier_out = logup_star::verify_reduction::<LF, _>(&verifier_gamma, [
		logup_star::TableLookup { n_vars: m_fetch,
			lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
		logup_star::TableLookup { n_vars: m_w,
			lookers: rd_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
	], &mut vt).expect("word-vm logup* verify");
	assert_eq!(prover_out, verifier_out, "prover/verifier lookup outputs agree");
	vt.finalize().expect("finalize");
	println!("✅ WORD-VM COMBINED proof: frontend + logup*(fetch+write-log) ONE transcript; claims rebuilt from inout");

	// ---- R3 soundness: each case tampers a public inout and re-verifies (verify-layer is_err) ----
	// (1) 过期读: rewrite a read value inout to its older-version value.
	{
		let (bad, _) = tamper_read_val(&inout_words, t_len, false);
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness_vec, &mut bt).expect("re-prove");
		let mut bv = bt.into_verifier();
		assert!(verifier.verify(&bad, &mut bv).is_err(), "soundness(1) MUST reject expired read value");
		println!("   soundness(1): verifier REJECTED an expired read value ✓ (frontend public-match; R1 binds rd.val==a_val)");
	}
	// (2) 版本篡改: rewrite a read version inout to a wrong index.
	{
	let mut bad = inout_words.clone();
	let rd1_ver_6 = io_rd1_ver(t_len, 6);
	bad[rd1_ver_6] = Word(bad[rd1_ver_6].0 + 1); // wrong version index -> wrong claim index
	let mut bt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut bt).expect("re-prove");
	let mut bv = bt.into_verifier();
	assert!(verifier.verify(&bad, &mut bv).is_err(), "soundness(2) MUST reject tampered version index");
	println!("   soundness(2): verifier REJECTED a tampered version index ✓ (frontend public-match; R1 binds rd.ver==mux8(ver[t],rs))");
	}
	// (3) 非法取指: circuit executes a different (still-valid) word NOT in the program table.
	//     Circuit is OK (decodes/executes it); logup* fetch rejects -> proves executed==fetched.
	{
		let bad_word = enc_addi(2, 1, 1); // addi x2,x1,1 — a valid instruction NOT in the program table
		let (c_ok, l_ok, _, _, _) = run_machine(init, &[(0x00, bad_word)]);
		assert!(c_ok, "soundness(3): altered program is a consistent execution -> circuit must pass");
		assert!(!l_ok, "soundness(3): executed word not in program table -> logup* must reject");
		println!("   soundness(3): circuit PASS (executes addi x1,x1,2) but logup* REJECTS (not in table) ✓ == proves executed==fetched");
	}
	// (4) 结果篡改: tamper the final register public output.
	{
		let mut bad = inout_words.clone();
		let fin_x1 = io_final(t_len, 1);
		bad[fin_x1] = Word(trace.final_regs[1].wrapping_add(1));
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness_vec, &mut bt).expect("re-prove");
		let mut bv = bt.into_verifier();
		assert!(verifier.verify(&bad, &mut bv).is_err(), "soundness(4) MUST reject tampered final register output");
		println!("   soundness(4): verifier REJECTED tampered final x1 output ✓ (frontend public-match)");
	}
}

/// Pick a read with `ver >= 1` and rewrite its VALUE inout to the stale (older) value.
/// Returns (tampered_inout, cycle_idx). `is_second` selects the rs2 read vs rs1 read.
fn tamper_read_val(inout_words: &[Word], t_len: usize, is_second: bool) -> (Vec<Word>, usize) {
	let mut bad = inout_words.to_vec();
	for t in 0..t_len {
		let (reg_idx, ver_idx, val_idx) = if is_second {
			(io_rd2_reg(t_len, t), io_rd2_ver(t_len, t), io_rd2_val(t_len, t))
		} else {
			(io_rd1_reg(t_len, t), io_rd1_ver(t_len, t), io_rd1_val(t_len, t))
		};
		let ver = bad[ver_idx].0;
		if ver >= 1 && bad[reg_idx].0 == 2 {
			// x2 at a version >= 1: the stale value is the version-0 initial (x2=0).
			bad[val_idx] = Word(0);
			return (bad, t);
		}
	}
	panic!("expected an x2 read at version >= 1 to tamper");
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn word_vm() {
		run_word_vm();
	}
}
