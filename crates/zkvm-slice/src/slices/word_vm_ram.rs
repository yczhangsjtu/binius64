//! Milestone M4: WORD-VM-RAM — RAM memory argument (K=64-word address space).
//!
//! Promotes the M3 register version-chain + write-log mechanism to RAM. The RAM read
//! value is pinned ONLY by logup* (the circuit has NO cross-address value chain — it
//! only maintains K=64 version counters); memory init/final/output are checked by the
//! verifier against the shared write-log table. RAM correctness is carried by the
//! argument, not the circuit — hence the signature layered rejection (circuit OK +
//! logup* reject) in soundness(1).

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
const NRAM: usize = 64; // K = 64-word address space
const VER_MAX: usize = 8; // must exceed max writes to any single register (x6 written 6x in M4 loop) to keep reg*VER_MAX+ver collision-free
const PC_START: u64 = 0x00;
const HALT_ADDR: u64 = 0x28;
const OP_ADDI: u64 = 0x13;
const OP_ADD: u64 = 0x33;
const OP_BEQ: u64 = 0x63;
const OP_LW: u64 = 0x03;
const OP_SW: u64 = 0x23;
const FUNCT3_LW: u64 = 0x02;
const FUNCT3_SW: u64 = 0x02;
const OUT_ADDR: usize = 10; // output unit: mem[10] = accumulator result
const M_FETCH: usize = 6;
const M_W_REG: usize = 6;
const M_W_RAM: usize = 9;

type LF = OptimalB128;
type LP = OptimalPackedB128;

// ---- encoders (M3 self-consistent layout; lw = I-type, sw = S-type) ----
fn enc_addi(rd: u64, rs1: u64, imm7: u64) -> u64 {
	OP_ADDI | (rd << 7) | (0 << 12) | (rs1 << 15) | ((imm7 & 0x7f) << 25)
}
fn enc_add(rd: u64, rs1: u64, rs2: u64) -> u64 {
	OP_ADD | (rd << 7) | (0 << 12) | (rs1 << 15) | (rs2 << 20)
}
fn enc_beq(rs1: u64, rs2: u64, off7: u64) -> u64 {
	OP_BEQ | (0 << 7) | (0 << 12) | (rs1 << 15) | (rs2 << 20) | ((off7 & 0x7f) << 25)
}
fn enc_lw(rd: u64, rs1: u64, imm7: u64) -> u64 {
	OP_LW | (rd << 7) | (FUNCT3_LW << 12) | (rs1 << 15) | ((imm7 & 0x7f) << 25)
}
fn enc_sw(rs2: u64, rs1: u64, imm7: u64) -> u64 {
	OP_SW | (0 << 7) | (FUNCT3_SW << 12) | (rs1 << 15) | (rs2 << 20) | ((imm7 & 0x7f) << 25)
}

#[inline]
fn sgn7(v: u64) -> i64 {
	if v & 0x40 != 0 { (v as i64) - 128 } else { v as i64 }
}

fn fetch_word(addr: u64) -> u64 {
	match addr {
		0x00 => enc_lw(6, 7, 0),       // lw  x6, x7, 0   -> x6 = mem[base]
		0x04 => enc_add(1, 1, 6),       // add x1, x1, x6
		0x08 => enc_addi(6, 6, 1),      // addi x6, x6, 1
		0x0c => enc_sw(6, 7, 0),        // sw  x6, x7, 0   -> mem[base] = x6
		0x10 => enc_lw(2, 7, 1),        // lw  x2, x7, 1   -> x2 = mem[base+1]
		0x14 => enc_add(1, 1, 2),       // add x1, x1, x2
		0x18 => enc_addi(3, 3, 1),      // addi x3, x3, 1  (i++)
		0x1c => enc_beq(3, 4, 8),       // beq x3, x4, +8  (exit)
		0x20 => enc_beq(5, 5, 0x60),    // beq x5, x5, -32 (loop back to 0x00)
		0x24 => enc_sw(1, 7, 2),        // sw  x1, x7, 2   -> mem[base+2] = x1 (output)
		0x28 => enc_addi(0, 0, 0),      // addi x0, x0, 0  (halt)
		_ => 0,
	}
}

fn decode_fields(inst: u64) -> (u64, usize, usize, usize, u64) {
	(inst & 0x7f, ((inst >> 7) & 0x1f) as usize, ((inst >> 15) & 0x1f) as usize,
	 ((inst >> 20) & 0x1f) as usize, (inst >> 25) & 0x7f)
}

#[derive(Clone, Copy, Debug)]
struct RegAccess { reg: usize, ver: usize, val: u64 }
#[derive(Clone, Copy, Debug)]
struct MemAccess { addr: usize, ver: usize, val: u64 }
#[derive(Clone, Debug)]
struct Cycle {
	pc: u64,
	inst: u64,
	reads: Vec<RegAccess>,
	write: Option<RegAccess>,
	regver: [usize; NREG],
	load: Option<MemAccess>,
	store: Option<MemAccess>,
	ramver: [usize; NRAM],
	mem_addr: usize,
}
#[derive(Clone, Debug)]
struct Trace { cycles: Vec<Cycle>, final_regs: [u64; NREG], final_ramver: [usize; NRAM] }

// word_overrides: pc -> replacement word (for soundness illegal-op / uninit-addr);
// load_overrides: cycle index -> replacement load VALUES (for the expired/uninit load).
fn run_program(init: [u64; NREG], init_mem: &[u64; NRAM],
	word_overrides: &[(u64, u64)], load_overrides: &[(usize, u64)]) -> Trace {
	let mut regs = init;
	let mut rver = [0usize; NREG];
	let mut mem = *init_mem;
	let mut ramver = [0usize; NRAM];
	let mut cycles = Vec::new();
	let mut pc = PC_START;
	let mut guard = 0;
	let fetch = |addr: u64| -> u64 {
		for &(a, w) in word_overrides { if a == addr { return w; } }
		fetch_word(addr)
	};
	loop {
		guard += 1;
		if guard > 96 { panic!("runaway execution"); }
		let inst = fetch(pc);
		let (opcode, rd, rs1, rs2, imm) = decode_fields(inst);
		let funct3 = (inst >> 12) & 0x7;
		let is_load = opcode == OP_LW && funct3 == FUNCT3_LW;
		let is_store = opcode == OP_SW && funct3 == FUNCT3_SW;
		let mut reads = Vec::new();
		reads.push(RegAccess { reg: rs1, ver: rver[rs1], val: regs[rs1] });
		reads.push(RegAccess { reg: rs2, ver: rver[rs2], val: regs[rs2] });
		let addr_calc = (regs[rs1] as u32).wrapping_add(sgn7(imm) as u32) as u64;
		let cycle_rv = rver;
		let cycle_ramv = ramver;

		let load = if is_load {
			let a = (addr_calc % NRAM as u64) as usize;
			let v = ramver[a];
			let mut val = mem[a];
			for &(cyc, ov) in load_overrides { if cyc == cycles.len() { val = ov; } }
			Some(MemAccess { addr: a, ver: v, val })
		} else { None };
		let store = if is_store {
			let a = (addr_calc % NRAM as u64) as usize;
			let v = ramver[a] + 1;
			Some(MemAccess { addr: a, ver: v, val: regs[rs2] })
		} else { None };

		let is_alu = opcode == OP_ADDI || opcode == OP_ADD;
		let write = if is_load {
			let v = load.unwrap().val; // the (possibly overridden) load value
			rver[rd] += 1;
			regs[rd] = v;
			Some(RegAccess { reg: rd, ver: rver[rd], val: v })
		} else if is_alu {
			let opadd = opcode == OP_ADD;
			let b = if opadd { regs[rs2] } else { sgn7(imm) as u64 };
			let v = (regs[rs1] as u32).wrapping_add(b as u32) as u64;
			rver[rd] += 1;
			regs[rd] = v;
			Some(RegAccess { reg: rd, ver: rver[rd], val: v })
		} else { None };

		if let Some(s) = &store { mem[s.addr] = s.val; ramver[s.addr] = s.ver; }

		let next_pc = if opcode == OP_BEQ && regs[rs1] == regs[rs2] {
			(pc as i64 + sgn7(imm)) as u64
		} else {
			pc.wrapping_add(4)
		};
		cycles.push(Cycle { pc, inst, reads, write, regver: cycle_rv, load, store, ramver: cycle_ramv, mem_addr: (addr_calc % NRAM as u64) as usize });
		if pc == HALT_ADDR { break; }
		pc = next_pc;
	}
	Trace { cycles, final_regs: regs, final_ramver: ramver }
}

fn mux(b: &CircuitBuilder, inputs: &[Wire], sel: Wire) -> Wire {
	if inputs.is_empty() { return b.add_constant_64(0); }
	let num_sel_bits = usize::BITS - (inputs.len() - 1).leading_zeros();
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
	inst: Vec<Wire>, pc: Vec<Wire>,
	rd1_reg: Vec<Wire>, rd1_ver: Vec<Wire>, rd1_val: Vec<Wire>,
	rd2_reg: Vec<Wire>, rd2_ver: Vec<Wire>, rd2_val: Vec<Wire>,
	wr_reg: Vec<Wire>, wr_ver: Vec<Wire>, wr_val: Vec<Wire>, wr_iswrite: Vec<Wire>,
	ld_addr: Vec<Wire>, ld_ver: Vec<Wire>, ld_val: Vec<Wire>, is_load: Vec<Wire>,
	st_addr: Vec<Wire>, st_ver: Vec<Wire>, st_val: Vec<Wire>, is_store: Vec<Wire>,
	init_regs: Vec<Wire>, final_regs: Vec<Wire>, fin_ver: Vec<Wire>,
}

// flat inout layout (block): 20 fields/cycle, then init_regs[8], final_regs[8], fin_ver[64].
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
fn io_ld_addr(t_len: usize, t: usize) -> usize { 12 * t_len + t }
fn io_ld_ver(t_len: usize, t: usize) -> usize { 13 * t_len + t }
fn io_ld_val(t_len: usize, t: usize) -> usize { 14 * t_len + t }
fn io_is_load(t_len: usize, t: usize) -> usize { 15 * t_len + t }
fn io_st_addr(t_len: usize, t: usize) -> usize { 16 * t_len + t }
fn io_st_ver(t_len: usize, t: usize) -> usize { 17 * t_len + t }
fn io_st_val(t_len: usize, t: usize) -> usize { 18 * t_len + t }
fn io_is_store(t_len: usize, t: usize) -> usize { 19 * t_len + t }
fn io_fin_ver(t_len: usize, a: usize) -> usize { 20 * t_len + 2 * NREG + a }

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
		ld_addr: (0..t_len).map(|_| b.add_inout()).collect(),
		ld_ver: (0..t_len).map(|_| b.add_inout()).collect(),
		ld_val: (0..t_len).map(|_| b.add_inout()).collect(),
		is_load: (0..t_len).map(|_| b.add_inout()).collect(),
		st_addr: (0..t_len).map(|_| b.add_inout()).collect(),
		st_ver: (0..t_len).map(|_| b.add_inout()).collect(),
		st_val: (0..t_len).map(|_| b.add_inout()).collect(),
		is_store: (0..t_len).map(|_| b.add_inout()).collect(),
		init_regs: (0..NREG).map(|_| b.add_inout()).collect(),
		final_regs: (0..NREG).map(|_| b.add_inout()).collect(),
		fin_ver: (0..NRAM).map(|_| b.add_inout()).collect(),
	};
	let zero = b.add_constant_64(0);
	let one = b.add_constant_64(1);
	let mask_ram = b.add_constant_64(0x3f);

	// register value chain (M3 scheme) + register version chain.
	let mut reg_cur: Vec<Wire> = iref.init_regs.clone();
	let mut rver: Vec<Vec<Wire>> = vec![(0..NREG).map(|_| zero).collect()];
	// RAM version chain (K=64 counters) — NO RAM value chain (the M4 signature).
	let mut ramver: Vec<Vec<Wire>> = vec![(0..NRAM).map(|_| zero).collect()];

	for t in 0..t_len {
		let inst_w = iref.inst[t];
		let opcode = b.band(inst_w, b.add_constant_64(0x7f));
		let rd = b.band(b.srl32(inst_w, 7), b.add_constant_64(0x1f));
		let funct3 = b.band(b.srl32(inst_w, 12), b.add_constant_64(0x7));
		let rs1 = b.band(b.srl32(inst_w, 15), b.add_constant_64(0x1f));
		let rs2 = b.band(b.srl32(inst_w, 20), b.add_constant_64(0x1f));
		let imm7 = b.band(b.srl32(inst_w, 25), b.add_constant_64(0x7f));

		let is_addi = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_ADDI)), b.icmp_eq(funct3, zero));
		let is_add = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_ADD)), b.icmp_eq(funct3, zero));
		let is_beq = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_BEQ)), b.icmp_eq(funct3, zero));
		let is_load = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_LW)), b.icmp_eq(funct3, b.add_constant_64(FUNCT3_LW)));
		let is_store = b.band(b.icmp_eq(opcode, b.add_constant_64(OP_SW)), b.icmp_eq(funct3, b.add_constant_64(FUNCT3_SW)));
		let is_alu_write = b.bor(b.bor(is_addi, is_add), is_load);
		let is_alu_write_01 = b.select(is_alu_write, one, zero);

		// ---- register operands (value chain, M3) ----
		let a_val = mux(&b, &reg_cur, rs1);
		let b_val = mux(&b, &reg_cur, rs2);
		let b_operand = b.select(is_addi, imm7, b_val);
		let alu_sum = b.iadd_32(a_val, b_operand);

		// ---- address calc: (rs1 + sext(imm)) masked to K=64 word ----
		let imm_sext = b.select(
			b.icmp_eq(b.band(imm7, b.add_constant_64(0x40)), zero),
			imm7,
			b.bxor(imm7, b.add_constant_64(0xFFFF_FFFF_FFFF_FF80)),
		);
		let addr_wire = b.iadd_32(a_val, imm_sext);
		let mem_addr = b.band(addr_wire, mask_ram);

		// ---- RAM version read (mux64 over the K counters) & load version ----
		let read_ram_ver = mux(&b, &ramver[t], mem_addr);
		let store_new_ver = b.iadd(read_ram_ver, one).0;

		// ---- load/store event pinning ----
		let is_load_01 = b.select(is_load, one, zero);
		let is_store_01 = b.select(is_store, one, zero);
		// load val == register write-back (the value x[rd] receives).
		let write_back = b.select(is_load, iref.ld_val[t], alu_sum);
		b.assert_eq(format!("ld.addr==addr[{t}]"), iref.ld_addr[t], mem_addr);
		b.assert_eq(format!("ld.ver==mux64[{t}]"), iref.ld_ver[t], read_ram_ver);
		b.assert_eq(format!("ld.val==writeback[{t}]"), iref.ld_val[t], write_back);
		b.assert_eq(format!("is_load[{t}]"), iref.is_load[t], is_load_01);
		b.assert_eq(format!("st.addr==addr[{t}]"), iref.st_addr[t], mem_addr);
		b.assert_eq(format!("st.ver==newver[{t}]"), iref.st_ver[t], store_new_ver);
		b.assert_eq(format!("st.val==rs2[{t}]"), iref.st_val[t], b_val);
		b.assert_eq(format!("is_store[{t}]"), iref.is_store[t], is_store_01);

		// ---- register version chain + value chain write-back ----
		let rd_ver_w = mux(&b, &rver[t], rd);
		let wr_ver_w = b.iadd(rd_ver_w, is_alu_write_01).0; // register version after write (any of addi/add/load)
		let mut next_reg = Vec::with_capacity(NREG);
		let mut next_rver = Vec::with_capacity(NREG);
		for r in 0..NREG {
			let rword = b.add_constant_64(r as u64);
			let is_write_r = b.band(b.icmp_eq(rd, rword), is_alu_write);
			next_reg.push(b.select(is_write_r, write_back, reg_cur[r]));
			next_rver.push(b.iadd(rver[t][r], b.select(is_write_r, one, zero)).0);
		}
		// register write event pinning.
		b.assert_eq(format!("wr.reg==rd[{t}]"), iref.wr_reg[t], rd);
		b.assert_eq(format!("wr.ver==rv+1[{t}]"), iref.wr_ver[t], wr_ver_w);
		b.assert_eq(format!("wr.val==writeback[{t}]"), iref.wr_val[t], write_back);
		b.assert_eq(format!("wr.iswrite[{t}]"), iref.wr_iswrite[t], is_alu_write_01);

		// ---- RAM version chain advance: ver[t+1][a] = ver[t][a] + (is_store & addr==a) ----
		let mut next_ramver = Vec::with_capacity(NRAM);
		for a in 0..NRAM {
			let aw = b.add_constant_64(a as u64);
			let is_store_a = b.band(b.icmp_eq(mem_addr, aw), is_store);
			next_ramver.push(b.iadd(ramver[t][a], b.select(is_store_a, one, zero)).0);
		}

		reg_cur = next_reg;
		rver.push(next_rver);
		ramver.push(next_ramver);

		// ---- PC advance ----
		let pc_plus4 = b.iadd(iref.pc[t], b.add_constant_64(4)).0;
		let beq_taken = b.band(is_beq, b.icmp_eq(a_val, b_val));
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

	// final registers + final RAM versions.
	for r in 0..NREG {
		b.assert_eq(format!("final_reg[{r}]"), iref.final_regs[r], reg_cur[r]);
	}
	for a in 0..NRAM {
		b.assert_eq(format!("fin_ver[{a}]"), iref.fin_ver[a], ramver[t_len][a]);
	}
	let circuit = b.build();
	(circuit, iref)
}

fn build_fetch_prog() -> Vec<u64> {
	let mut prog = vec![0u64; 1usize << M_FETCH];
	for mm in [0x00u64, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x18, 0x1c, 0x20, 0x24, 0x28].iter() {
		prog[*mm as usize] = fetch_word(*mm);
	}
	prog
}
fn build_reg_wlog(init: &[u64; NREG], trace: &Trace) -> Vec<u64> {
	let mut w = vec![0u64; NREG * VER_MAX];
	for r in 0..NREG { w[r * VER_MAX + 0] = init[r]; }
	for c in &trace.cycles {
		if let Some(wr) = &c.write { w[wr.reg * VER_MAX + wr.ver] = wr.val; }
	}
	w
}
fn build_ram_wlog(init_mem: &[u64; NRAM], trace: &Trace) -> Vec<u64> {
	let mut w = vec![0u64; NRAM * VER_MAX];
	for a in 0..NRAM { w[a * VER_MAX + 0] = init_mem[a]; }
	for c in &trace.cycles {
		if let Some(s) = &c.store { w[s.addr * VER_MAX + s.ver] = s.val; }
	}
	w
}

/// Rebuild all three claim groups from the flat inout words (R2 discipline; no native trace).
fn claims_from_inout(inout_words: &[Word], t_len: usize) -> (Vec<Vec<usize>>, Vec<u64>, Vec<Vec<usize>>, Vec<u64>, Vec<Vec<usize>>, Vec<u64>) {
	let mut fetch_idxs = Vec::new(); let mut fetch_claims = Vec::new();
	let mut reg_idxs = Vec::new(); let mut reg_claims = Vec::new();
	let mut ram_idxs = Vec::new(); let mut ram_claims = Vec::new();
	for t in 0..t_len {
		let inst = inout_words[io_inst(t_len, t)].0;
		let pc = inout_words[io_pc(t_len, t)].0;
		fetch_idxs.push(vec![pc as usize]);
		fetch_claims.push(inst);
		// register reads rs1/rs2
		reg_idxs.push(vec![(inout_words[io_rd1_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_rd1_ver(t_len, t)].0 as usize]);
		reg_claims.push(inout_words[io_rd1_val(t_len, t)].0);
		reg_idxs.push(vec![(inout_words[io_rd2_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_rd2_ver(t_len, t)].0 as usize]);
		reg_claims.push(inout_words[io_rd2_val(t_len, t)].0);
		// register write (when iswrite != 0)
		if inout_words[io_wr_iswrite(t_len, t)].0 != 0 {
			reg_idxs.push(vec![(inout_words[io_wr_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_wr_ver(t_len, t)].0 as usize]);
			reg_claims.push(inout_words[io_wr_val(t_len, t)].0);
		}
		// RAM load (when is_load != 0)
		if inout_words[io_is_load(t_len, t)].0 != 0 {
			ram_idxs.push(vec![(inout_words[io_ld_addr(t_len, t)].0 as usize) * VER_MAX + inout_words[io_ld_ver(t_len, t)].0 as usize]);
			ram_claims.push(inout_words[io_ld_val(t_len, t)].0);
		}
		// RAM store (when is_store != 0)
		if inout_words[io_is_store(t_len, t)].0 != 0 {
			ram_idxs.push(vec![(inout_words[io_st_addr(t_len, t)].0 as usize) * VER_MAX + inout_words[io_st_ver(t_len, t)].0 as usize]);
			ram_claims.push(inout_words[io_st_val(t_len, t)].0);
		}
	}
	(fetch_idxs, fetch_claims, reg_idxs, reg_claims, ram_idxs, ram_claims)
}

/// Verifier-side explicit checks (init / final / output), operating on the shared W tables
/// + the public inout-derived fin_ver.
fn check_init(w_ram: &[u64], init_mem: &[u64; NRAM]) -> bool {
	(0..NRAM).all(|a| w_ram[a * VER_MAX + 0] == init_mem[a])
}
fn check_final_output(w_ram: &[u64], fin_ver: &[usize; NRAM], out_addr: usize, expected: u64) -> bool {
	w_ram[out_addr * VER_MAX + fin_ver[out_addr]] == expected
}

fn cycle_alu_sum(c: &Cycle) -> u64 {
	let op = c.inst & 0x7f;
	let a = c.reads[0].val;
	let b = c.reads[1].val;
	let imm = (c.inst >> 25) & 0x7f;
	let (x, y) = if op == OP_ADDI { (a, sgn7(imm) as u64) } else { (a, b) };
	(x as u32).wrapping_add(y as u32) as u64
}

/// Result of a full machine run; keeps the prover/verifier/witness alive so the
/// soundness cases can re-prove with a tampered public inout.
struct M4Run {
	c_ok: bool,
	l_ok: bool,
	stat: CircuitStat,
	t_len: usize,
	inout_words: Vec<Word>,
	ram_wlog: Vec<u64>,
	trace: Trace,
	prover: WordProver<OptimalPackedB128, StdHashSuite>,
	verifier: WordVerifier<StdHashSuite>,
	witness: binius_core::constraint_system::ValueVec,
}

fn run_machine_full(init: [u64; NREG], init_mem: &[u64; NRAM],
	word_overrides: &[(u64, u64)], load_overrides: &[(usize, u64)]) -> M4Run {
	let trace = run_program(init, init_mem, word_overrides, load_overrides);
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
		let alu = cycle_alu_sum(c);
		if let Some(wr) = &c.write {
			w[iref.wr_ver[t]] = Word(wr.ver as u64);
			w[iref.wr_val[t]] = Word(wr.val);
			w[iref.wr_iswrite[t]] = Word(1);
		} else {
			w[iref.wr_ver[t]] = Word(c.regver[rd_dec] as u64);
			w[iref.wr_val[t]] = Word(alu);
			w[iref.wr_iswrite[t]] = Word(0);
		}
		if let Some(ld) = &c.load {
			w[iref.ld_addr[t]] = Word(ld.addr as u64);
			w[iref.ld_ver[t]] = Word(ld.ver as u64);
			w[iref.ld_val[t]] = Word(ld.val);
			w[iref.is_load[t]] = Word(1);
		} else {
			w[iref.ld_addr[t]] = Word(c.mem_addr as u64);
			w[iref.ld_ver[t]] = Word(c.ramver[c.mem_addr] as u64);
			w[iref.ld_val[t]] = Word(alu);
			w[iref.is_load[t]] = Word(0);
		}
		if let Some(st) = &c.store {
			w[iref.st_addr[t]] = Word(st.addr as u64);
			w[iref.st_ver[t]] = Word(st.ver as u64);
			w[iref.st_val[t]] = Word(st.val);
			w[iref.is_store[t]] = Word(1);
		} else {
			w[iref.st_addr[t]] = Word(c.mem_addr as u64);
			w[iref.st_ver[t]] = Word((c.ramver[c.mem_addr] + 1) as u64);
			w[iref.st_val[t]] = Word(c.reads[1].val);
			w[iref.is_store[t]] = Word(0);
		}
	}
	for r in 0..NREG {
		w[iref.init_regs[r]] = Word(init[r]);
		w[iref.final_regs[r]] = Word(trace.final_regs[r]);
	}
	for a in 0..NRAM {
		w[iref.fin_ver[a]] = Word(trace.final_ramver[a] as u64);
	}
	circuit.populate_wire_witness(&mut w).expect("witness fill");
	let witness_vec = w.into_value_vec();
	let _native_ok = cs.verify(&witness_vec).is_ok();
	let inout_words = witness_vec.inout().to_vec();
	let (fetch_idxs, fetch_claims, reg_idxs, reg_claims, ram_idxs, ram_claims) = claims_from_inout(&inout_words, t_len);

	let prog = build_fetch_prog();
	let fetch_table = FieldBuffer::from_values(&prog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let fetch_view = fetch_table.as_view();
	let reg_wlog = build_reg_wlog(&init, &trace);
	let reg_table = FieldBuffer::from_values(&reg_wlog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let reg_view = reg_table.as_view();
	let ram_wlog = build_ram_wlog(init_mem, &trace);
	let ram_table = FieldBuffer::from_values(&ram_wlog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let ram_view = ram_table.as_view();

	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<OptimalPackedB128, StdHashSuite>::setup(verifier.clone()).expect("prover setup");
	let alloc = GlobalAllocator;
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");
	let f_lookers: Vec<Looker<LF>> = (0..fetch_idxs.len())
		.map(|i| Looker { index: &fetch_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(fetch_claims[i] as u128) }).collect();
	let r_lookers: Vec<Looker<LF>> = (0..reg_idxs.len())
		.map(|i| Looker { index: &reg_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(reg_claims[i] as u128) }).collect();
	let m_lookers: Vec<Looker<LF>> = (0..ram_idxs.len())
		.map(|i| Looker { index: &ram_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(ram_claims[i] as u128) }).collect();
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let _pout = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(&alloc, gamma, [
		binius_ip_prover::logup_star::TableLookup { table: fetch_view, lookers: f_lookers },
		binius_ip_prover::logup_star::TableLookup { table: reg_view, lookers: r_lookers },
		binius_ip_prover::logup_star::TableLookup { table: ram_view, lookers: m_lookers },
	], &mut pt);
	let mut vt = pt.into_verifier();
	let circuit_ok = verifier.verify(&inout_words, &mut vt).is_ok();
	let vg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	let logup_ok = if vg == gamma {
		logup_star::verify_reduction::<LF, _>(&vg, [
			logup_star::TableLookup { n_vars: M_FETCH,
				lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
			logup_star::TableLookup { n_vars: M_W_REG,
				lookers: reg_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
			logup_star::TableLookup { n_vars: M_W_RAM,
				lookers: ram_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
		], &mut vt).is_ok()
	} else { false };
	M4Run { c_ok: circuit_ok, l_ok: logup_ok, stat, t_len, inout_words, ram_wlog, trace, prover, verifier, witness: witness_vec }
}

/// Re-prove the honest witness but verify a (possibly tampered) public inout; true iff rejected.
fn reverify(run: &M4Run, bad_inout: &[Word]) -> bool {
	let mut bt = ProverTranscript::new(StdChallenger::default());
	run.prover.prove(&run.witness, &mut bt).expect("re-prove");
	let mut bv = bt.into_verifier();
	run.verifier.verify(bad_inout, &mut bv).is_err()
}

pub fn run_word_vm_ram() {
	let init: [u64; NREG] = [0, 0, 0, 0, 3, 0, 0, 8]; // x4=limit=3, x7=base=8
	let init_mem: [u64; NRAM] = {
		let mut m = [0u64; NRAM];
		m[8] = 5;
		m[9] = 7;
		m
	};
	let run = run_machine_full(init, &init_mem, &[], &[]);
	assert!(run.c_ok, "honest frontend verify must pass");
	assert!(run.l_ok, "honest logup* verify must pass");
	let t_len = run.t_len;
	let trace = &run.trace;
	let ram_wlog = &run.ram_wlog;
	let inout_words = &run.inout_words;
	println!("== M4 (WORD-VM-RAM): RAM memory argument (K=64) ==");
	println!("   program: lw/add/addi/sw/lw/add/addi/beq/beq/sw/halt (mem[8]=5, mem[9]=7, out=mem[10])");
	println!("   cycles={t_len} final x1={} ramver[8]={} ramver[9]={} ramver[10]={}",
		trace.final_regs[1], trace.final_ramver[8], trace.final_ramver[9], trace.final_ramver[10]);
	println!("   constraints: ZERO={} AND={} IMUL={} BMUL={} (gates={})",
		run.stat.n_zero_constraints, run.stat.n_and_constraints, run.stat.n_imul_constraints, run.stat.n_bmul_constraints, run.stat.n_gates);
	println!("✅ WORD-VM-RAM COMBINED proof: frontend + logup*(fetch+reg-wlog+ram-wlog) ONE transcript");
	println!("   RAM version chain = O(K·T) = {NRAM} counters × {t_len} cycles (T1 known boundary)");

	// ---- T3 init / final / output (verifier-side explicit checks) ----
	assert!(check_init(ram_wlog, &init_mem), "init: W_ram[addr*VER_MAX+0] must equal public init_mem (verifier assertion)");
	let mut fin_ver = [0usize; NRAM];
	for a in 0..NRAM { fin_ver[a] = inout_words[io_fin_ver(t_len, a)].0 as usize; }
	let expected_out = trace.cycles.iter().filter(|c| c.store.is_some()).last().map(|c| c.store.unwrap().val).unwrap();
	assert!(check_final_output(ram_wlog, &fin_ver, OUT_ADDR, expected_out), "final/output: M_final[out] must equal the store value (verifier assertion)");
	println!("   ✅ init/final/output checks pass (verifier asserts init + M_final[{OUT_ADDR}]={expected_out})");

	// ---- T4 soundness (5 cases) ----
	// (1) 过期读 —— M4 招牌 layered rejection: a load reads an OLD-version value.
	{
		let cyc = trace.cycles.iter().position(|c| c.load.map(|l| l.ver >= 1).unwrap_or(false));
		let cyc = cyc.expect("expected a load at version >= 1");
		let lv = trace.cycles[cyc].load.unwrap();
		let stale = ram_wlog[lv.addr * VER_MAX + (lv.ver - 1)]; // older version's stored value
		let r2 = run_machine_full(init, &init_mem, &[], &[(cyc, stale)]);
		assert!(r2.c_ok, "soundness(1): altered (expired-load) machine must still be circuit-consistent");
		assert!(!r2.l_ok, "soundness(1): expired load must be REJECTED by logup*");
		println!("   soundness(1): expired load → circuit PASS + logup* REJECT ✓ (RAM read carried by argument, not circuit)");
	}
	// (2) 版本篡改 —— tamper a load event's ver inout; frontend public-match rejects.
	{
		let cyc = trace.cycles.iter().position(|c| c.load.is_some()).expect("a load");
		let mut bad = inout_words.clone();
		let idx = io_ld_ver(t_len, cyc);
		bad[idx] = Word(bad[idx].0 + 1);
		assert!(reverify(&run, &bad), "soundness(2) MUST reject a tampered load-version inout");
		println!("   soundness(2): verifier REJECTED a tampered load version inout ✓ (frontend public-match; ld.ver==mux64(ver[t],addr))");
	}
	// (3) 越界/未初始化读 —— a load claims a NONZERO value at an address never written & not in init image.
	{
		// Redirect the lw at 0x10 to touch an address (x7+0x20=40) that is init 0 and never written,
		// then override the loaded value to a nonzero (not in W). Circuit OK, logup* (ram-wlog) rejects.
		let w_ov = vec![(0x10u64, enc_lw(2, 7, 0x20))];
		let l_ov = vec![(11usize, 99u64)]; // 11 = the first redirected lw cycle index (empirically a load)
		let r3 = run_machine_full(init, &init_mem, &w_ov, &l_ov);
		assert!(r3.c_ok, "soundness(3): uninit-addr machine must still be circuit-consistent");
		assert!(!r3.l_ok, "soundness(3): a load of a nonzero value from an uninitialized address must be REJECTED by logup*");
		println!("   soundness(3): uninitialized-address non-zero load → circuit PASS + logup* REJECT ✓");
	}
	// (4) 初始镜像篡改 —— W_ram ver=0 row disagrees with public init_mem -> verifier init check rejects.
	{
		let mut bad_init = init_mem;
		bad_init[8] = bad_init[8].wrapping_add(1); // tamper the public init image at a touched address
		assert!(!check_init(ram_wlog, &bad_init), "soundness(4): tampered public init image must fail the verifier init check");
		println!("   soundness(4): public init image tamper → verifier init check REJECTS ✓");
	}
	// (5) 结果篡改 —— tamper a final RAM-version inout (output depends on it); frontend public-match rejects.
	{
		let mut bad = inout_words.clone();
		let idx = io_fin_ver(t_len, OUT_ADDR);
		bad[idx] = Word(bad[idx].0 + 1);
		assert!(reverify(&run, &bad), "soundness(5) MUST reject a tampered final RAM version (output) inout");
		println!("   soundness(5): verifier REJECTED a tampered final RAM version (output) inout ✓ (frontend public-match)");
	}
}



#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn word_vm_ram() {
		run_word_vm_ram();
	}
}
