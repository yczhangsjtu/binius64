//! vm32 gate-circuit builder (`build_circuit`): decode/execute, 32-register value+version
//! chains, RAM version chain, event pinning, flat inout layout, MSB-bool helpers.
//! Mechanically extracted from the former single-file `word_vm32.rs` (M6 T1); logic identical.

use binius_frontend::{Circuit, CircuitBuilder, Wire};
use crate::vm32::isa::*;
use crate::vm32::interp::Trace;

pub fn mux(b: &CircuitBuilder, inputs: &[Wire], sel: Wire) -> Wire {
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

// 8-way mux by funct3 (3-bit selector)
pub fn mux8(b: &CircuitBuilder, inputs: &[Wire; 8], sel: Wire) -> Wire {
	mux(b, &inputs[..], sel)
}

// variable logical left shift (barrel, 5 stages of constant sll32)
pub fn shl_var(b: &CircuitBuilder, v: Wire, shamt: Wire) -> Wire {
	let mut x = v;
	for (i, s) in [1u32, 2, 4, 8, 16].iter().enumerate() {
		let bit = b.shl(shamt, 63 - i as u32); // MSB-bool condition (select needs it)
		x = b.select(bit, b.sll32(x, *s), x);
	}
	x
}
// variable logical right shift
pub fn shr_var(b: &CircuitBuilder, v: Wire, shamt: Wire) -> Wire {
	let mut x = v;
	for (i, s) in [1u32, 2, 4, 8, 16].iter().enumerate() {
		let bit = b.shl(shamt, 63 - i as u32); // MSB-bool
		x = b.select(bit, b.srl32(x, *s), x);
	}
	x
}
// variable arithmetic right shift (sign-extend each stage from current bit31)
pub fn sar_var(b: &CircuitBuilder, v: Wire, shamt: Wire) -> Wire {
	let mut x = v;
	for (i, s) in [1u32, 2, 4, 8, 16].iter().enumerate() {
		let bit = b.shl(shamt, 63 - i as u32); // MSB-bool
		let msb = b.shl(x, 32); // x bit31 -> bit63, MSB-bool
		let mask = b.add_constant_64((!0u64 << (32 - s)) & 0xffffffff);
		let fill = b.select(msb, mask, b.add_constant_64(0));
		let shifted = b.srl32(x, *s);
		x = b.select(bit, b.bor(shifted, fill), x);
	}
	x
}

// sign-extend a wire's low `bits` to 32-bit lane using 64-bit arithmetic shift:
// place the sign bit at bit 63 via shl then arith-shift right to fill the high bits.
pub fn sext_w(b: &CircuitBuilder, raw: Wire, bits: u32) -> Wire {
	let shift = 64 - bits;
	b.band(b.sar(b.shl(raw, shift), shift), b.add_constant_64(0xffffffff))
}

// struct of a ±n compare producing 1/0 wire
pub fn bool01(b: &CircuitBuilder, cond: Wire) -> Wire {
	let one = b.add_constant_64(1);
	let zero = b.add_constant_64(0);
	b.select(cond, one, zero)
}
pub fn slt_signed(b: &CircuitBuilder, a: Wire, b0: Wire) -> Wire {
	let sign = b.add_constant_64(0x80000000);
	bool01(b, b.icmp_ult(b.bxor(a, sign), b.bxor(b0, sign)))
}
pub fn slt_unsigned(b: &CircuitBuilder, a: Wire, b0: Wire) -> Wire {
	bool01(b, b.icmp_ult(a, b0))
}

#[derive(Clone)]
pub struct InoutRefs {
	pub inst: Vec<Wire>,
	pub pc: Vec<Wire>,
	pub rd1_reg: Vec<Wire>,
	pub rd1_ver: Vec<Wire>,
	pub rd1_val: Vec<Wire>,
	pub rd2_reg: Vec<Wire>,
	pub rd2_ver: Vec<Wire>,
	pub rd2_val: Vec<Wire>,
	pub wr_reg: Vec<Wire>,
	pub wr_ver: Vec<Wire>,
	pub wr_val: Vec<Wire>,
	pub wr_iswrite: Vec<Wire>,
	pub ld_addr: Vec<Wire>,
	pub ld_ver: Vec<Wire>,
	pub ld_val: Vec<Wire>,
	pub is_load: Vec<Wire>,
	pub st_addr: Vec<Wire>,
	pub st_ver: Vec<Wire>,
	pub st_val: Vec<Wire>,
	pub is_store: Vec<Wire>,
	pub init_regs: Vec<Wire>,
	pub final_regs: Vec<Wire>,
	pub fin_ver: Vec<Wire>,
}

// flat inout layout (block): 20 fields/cycle, then init_regs[32], final_regs[32], fin_ver[64].
pub fn io_inst(_t: usize, t: usize) -> usize { t }
pub fn io_pc(t_len: usize, t: usize) -> usize { t_len + t }
pub fn io_rd1_reg(t_len: usize, t: usize) -> usize { 2 * t_len + t }
pub fn io_rd1_ver(t_len: usize, t: usize) -> usize { 3 * t_len + t }
pub fn io_rd1_val(t_len: usize, t: usize) -> usize { 4 * t_len + t }
pub fn io_rd2_reg(t_len: usize, t: usize) -> usize { 5 * t_len + t }
pub fn io_rd2_ver(t_len: usize, t: usize) -> usize { 6 * t_len + t }
pub fn io_rd2_val(t_len: usize, t: usize) -> usize { 7 * t_len + t }
pub fn io_wr_reg(t_len: usize, t: usize) -> usize { 8 * t_len + t }
pub fn io_wr_ver(t_len: usize, t: usize) -> usize { 9 * t_len + t }
pub fn io_wr_val(t_len: usize, t: usize) -> usize { 10 * t_len + t }
pub fn io_wr_iswrite(t_len: usize, t: usize) -> usize { 11 * t_len + t }
pub fn io_ld_addr(t_len: usize, t: usize) -> usize { 12 * t_len + t }
pub fn io_ld_ver(t_len: usize, t: usize) -> usize { 13 * t_len + t }
pub fn io_ld_val(t_len: usize, t: usize) -> usize { 14 * t_len + t }
pub fn io_is_load(t_len: usize, t: usize) -> usize { 15 * t_len + t }
pub fn io_st_addr(t_len: usize, t: usize) -> usize { 16 * t_len + t }
pub fn io_st_ver(t_len: usize, t: usize) -> usize { 17 * t_len + t }
pub fn io_st_val(t_len: usize, t: usize) -> usize { 18 * t_len + t }
pub fn io_is_store(t_len: usize, t: usize) -> usize { 19 * t_len + t }
pub fn build_circuit(trace: &Trace) -> (Circuit, InoutRefs) {
	let b = CircuitBuilder::new();
	let t_len = trace.cycles.len();
	let zero = b.add_constant_64(0);
	let one = b.add_constant_64(1);

	let inst = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let pc = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let rd1_reg = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let rd1_ver = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let rd1_val = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let rd2_reg = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let rd2_ver = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let rd2_val = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let wr_reg = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let wr_ver = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let wr_val = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let wr_iswrite = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let ld_addr = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let ld_ver = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let ld_val = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let is_load = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let st_addr = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let st_ver = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let st_val = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let is_store = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();
	let init_regs = (0..NREG).map(|_| b.add_inout()).collect::<Vec<_>>();
	let final_regs = (0..NREG).map(|_| b.add_inout()).collect::<Vec<_>>();
	let fin_ver = (0..NRAM).map(|_| b.add_inout()).collect::<Vec<_>>();

	// registers (value + version chains), 32 each, per cycle
	let mut reg: Vec<[Wire; NREG]> = Vec::new();
	let mut ver: Vec<[Wire; NREG]> = Vec::new();
	let mut cur_reg = [zero; NREG];
	let mut cur_ver = [zero; NREG];
	// RAM version chain
	let mut ramver: Vec<[Wire; NRAM]> = Vec::new();
	let mut c_ram_ver = [zero; NRAM];
	let mut prev_pc = b.add_constant_64(PC_START);

	for t in 0..t_len {
		b.assert_eq(format!("pc[{t}]"), pc[t], prev_pc);
		let inst_w = inst[t];
		let opcode = b.band(inst_w, b.add_constant_64(0x7f));
		let rd = b.band(b.srl32(inst_w, 7), b.add_constant_64(0x1f));
		let rs1 = b.band(b.srl32(inst_w, 15), b.add_constant_64(0x1f));
		let rs2 = b.band(b.srl32(inst_w, 20), b.add_constant_64(0x1f));
		let funct3 = b.band(b.srl32(inst_w, 12), b.add_constant_64(0x7));
		let funct7 = b.band(b.srl32(inst_w, 25), b.add_constant_64(0x7f));

		let imm_i = sext_w(&b, b.band(b.srl32(inst_w, 20), b.add_constant_64(0xfff)), 12);
		let imm_s = sext_w(&b, b.bor(b.sll32(b.band(b.srl32(inst_w, 25), b.add_constant_64(0x7f)), 5), b.band(b.srl32(inst_w, 7), b.add_constant_64(0x1f))), 12); // S-type sign-extended offset (sw)
		let imm_u = b.band(b.srl32(inst_w, 12), b.add_constant_64(0xfffff)); // U-imm[31:12] -> value = (srl32(inst,12)<<12) but keep 20 bits shifted
		let imm_u_val = b.sll32(imm_u, 12); // place at [31:12]
		let imm_b_raw = b.bor(b.bor(b.bor(b.sll32(b.band(b.srl32(inst_w, 31), b.add_constant_64(1)), 12),
			b.sll32(b.band(b.srl32(inst_w, 7), b.add_constant_64(1)), 11)),
			b.sll32(b.band(b.srl32(inst_w, 25), b.add_constant_64(0x3f)), 5)),
			b.sll32(b.band(b.srl32(inst_w, 8), b.add_constant_64(0x0f)), 1));
		let imm_b = sext_w(&b, imm_b_raw, 13);
		let imm_j_raw = b.bor(b.bor(b.bor(b.sll32(b.band(b.srl32(inst_w, 31), b.add_constant_64(1)), 20),
			b.sll32(b.band(b.srl32(inst_w, 12), b.add_constant_64(0xff)), 12)),
			b.sll32(b.band(b.srl32(inst_w, 20), b.add_constant_64(1)), 11)),
			b.sll32(b.band(b.srl32(inst_w, 21), b.add_constant_64(0x3ff)), 1));
		let imm_j = sext_w(&b, imm_j_raw, 21);

		let eq_opcode = |c: u64| b.icmp_eq(opcode, b.add_constant_64(c));
		let is_risc = eq_opcode(OP_OP);
		let is_imm = eq_opcode(OP_OPIMM);
		let is_lui = eq_opcode(OP_LUI);
		let is_auipc = eq_opcode(OP_AUIPC);
		let is_jal = eq_opcode(OP_JAL);
		let is_jalr = eq_opcode(OP_JALR);
		let is_branch = eq_opcode(OP_BRANCH);
		let c_is_load = b.band(eq_opcode(OP_LOAD), b.icmp_eq(funct3, b.add_constant_64(0x2)));
		let c_is_store = b.band(eq_opcode(OP_STORE), b.icmp_eq(funct3, b.add_constant_64(0x2)));

		// read register values/versions
		let rs1v = mux(&b, &cur_reg, rs1);
		let rs2v = mux(&b, &cur_reg, rs2);
		let rs1_ver = mux(&b, &cur_ver, rs1);
		let rs2_ver = mux(&b, &cur_ver, rs2);

		// memory address = rs1 + sext(imm): I-imm for lw, S-imm (sign-extended, imm_s) for sw.
		// (S-type offset lives at inst[31:25]+inst[11:7], NOT imm_i whose low 5 bits alias rs2.)
		let ld_addr_w = b.band(b.iadd_32(rs1v, imm_i), b.add_constant_64(0x3f));
		let st_addr_w = b.band(b.select(c_is_store, b.iadd_32(rs1v, imm_s), b.iadd_32(rs1v, imm_i)), b.add_constant_64(0x3f)); // S-imm on store, I-imm otherwise (matches witness fallback)
		let read_ram_ver = mux(&b, &c_ram_ver, ld_addr_w);
		let st_ram_ver = mux(&b, &c_ram_ver, st_addr_w);
		let store_new_ver = b.iadd(st_ram_ver, one).0;

		// ALU common operands
		let bop = b.select(is_risc, rs2v, imm_i);
		let shamt = b.select(is_risc, b.band(rs2v, b.add_constant_64(0x1f)), b.band(b.srl32(inst_w, 20), b.add_constant_64(0x1f)));
		let is_sub = b.band(b.band(b.icmp_eq(opcode, b.add_constant_64(OP_OP)), b.icmp_eq(funct3, b.add_constant_64(0x0))), b.shl(funct7, 58)); // sub: R-type only (addi's imm[11:5] must not count), funct7 bit5 -> MSB
		let is_sra = b.band(b.icmp_eq(funct3, b.add_constant_64(0x5)), b.shl(funct7, 58)); // R&I sra marker both at inst bit30/funct7 bit5 (F1 fix); funct3==5 gates to srai/srli only

		// funct3-candidate ALU results (shared for R and I where applicable)
		let alu_add_sub = b.select(is_sub, b.band(b.isub_bin_bout(rs1v, rs2v, zero).0, b.add_constant_64(0xffffffff)), b.iadd_32(rs1v, bop)); // mask sub result to low 32 bits
		let alu_sll = shl_var(&b, rs1v, shamt);
		let alu_slt = slt_signed(&b, rs1v, bop);
		let alu_sltu = slt_unsigned(&b, rs1v, bop);
		let alu_xor = b.bxor(rs1v, bop);
		let alu_shift = b.select(is_sra, sar_var(&b, rs1v, shamt), shr_var(&b, rs1v, shamt));
		let alu_or = b.bor(rs1v, bop);
		let alu_and = b.band(rs1v, bop);

		let alu8 = [alu_add_sub, alu_sll, alu_slt, alu_sltu, alu_xor, alu_shift, alu_or, alu_and];
		let alu_core = mux8(&b, &alu8, funct3);
		// final alu_sum selection across opcode classes
		let pc_v = prev_pc;
		let lui_v = imm_u_val;
		let auipc_v = b.iadd_32(pc_v, imm_u_val);
		let jal_rd = b.iadd_32(pc_v, b.add_constant_64(4));
		let alu_sum = b.select(is_imm, alu_core,
			b.select(is_risc, alu_core,
			b.select(c_is_load, ld_val[t],
			b.select(is_lui, lui_v,
			b.select(is_auipc, auipc_v,
			b.select(is_jal, jal_rd,
			b.select(is_jalr, jal_rd, zero)))))));

		// x0 hard-zero write-back
		let rd_is_zero = b.icmp_eq(rd, zero);
		let wb = b.select(rd_is_zero, zero, alu_sum);
		let is_alu_write = b.bor(b.bor(b.bor(b.bor(b.bor(is_imm, is_risc), is_lui), is_auipc), is_jal), is_jalr);
		let is_alu_write = b.bor(is_alu_write, c_is_load);

		// reg/ver chain update
		let mut next_reg = cur_reg;
		let mut next_ver = cur_ver;
		for r in 0..NREG {
			let is_write_r = b.band(b.icmp_eq(rd, b.add_constant_64(r as u64)), is_alu_write);
			// wb already forced to 0 for r0 by rd_is_zero
			let nv = b.select(is_write_r, wb, cur_reg[r]);
			let nver = b.select(is_write_r, b.iadd(cur_ver[r], one).0, cur_ver[r]);
			next_reg[r] = nv;
			next_ver[r] = nver;
		}
		reg.push(cur_reg); ver.push(cur_ver);
		cur_reg = next_reg; cur_ver = next_ver;

		// RAM version chain
		let mut next_ram = c_ram_ver;
		for a in 0..NRAM {
			let is_st_a = b.band(c_is_store, b.icmp_eq(st_addr_w, b.add_constant_64(a as u64)));
			next_ram[a] = b.select(is_st_a, b.iadd(c_ram_ver[a], one).0, c_ram_ver[a]);
		}
		ramver.push(c_ram_ver);
		c_ram_ver = next_ram;

		// PC advance
		let pc4 = b.iadd_32(pc_v, b.add_constant_64(4));
		// branch conditions (MSB-bool from icmp_*)
		let c_beq = b.icmp_eq(rs1v, rs2v);
		let c_bne = b.bnot(c_beq);
		let c_blt = b.icmp_ult(b.bxor(rs1v, b.add_constant_64(0x80000000)), b.bxor(rs2v, b.add_constant_64(0x80000000)));
		let c_bge = b.bnot(c_blt);
		let c_bltu = b.icmp_ult(rs1v, rs2v);
		let c_bgeu = b.bnot(c_bltu);
		let branch_taken = mux8(&b, &[c_beq, c_bne, zero, zero, c_blt, c_bge, c_bltu, c_bgeu], funct3);
		let next_pc = b.select(is_jal, b.iadd_32(pc_v, imm_j),
			b.select(is_jalr, b.band(b.iadd_32(rs1v, imm_i), b.add_constant_64(0xfffffffe)),
			b.select(is_branch, b.select(branch_taken, b.iadd_32(pc_v, imm_b), pc4), pc4)));
		prev_pc = next_pc;

		// R1: pin read / write / load / store event inouts to the circuit wires
		let is_alu_write_01 = b.select(is_alu_write, one, zero);
		let is_load_01 = b.select(c_is_load, one, zero);
		let is_store_01 = b.select(c_is_store, one, zero);
		let rd_ver_now = mux(&b, &cur_ver, rd);
		let rd_ver_after = rd_ver_now; // cur_ver already updated to post-write state
		b.assert_eq(format!("rd1_reg[{t}]"), rd1_reg[t], rs1);
		b.assert_eq(format!("rd1_ver[{t}]"), rd1_ver[t], rs1_ver);
		b.assert_eq(format!("rd1_val[{t}]"), rd1_val[t], rs1v);
		b.assert_eq(format!("rd2_reg[{t}]"), rd2_reg[t], rs2);
		b.assert_eq(format!("rd2_ver[{t}]"), rd2_ver[t], rs2_ver);
		b.assert_eq(format!("rd2_val[{t}]"), rd2_val[t], rs2v);
		b.assert_eq(format!("wr_reg[{t}]"), wr_reg[t], rd);
		b.assert_eq(format!("wr_ver[{t}]"), wr_ver[t], rd_ver_after);
		b.assert_eq(format!("wr_val[{t}]"), wr_val[t], wb);
		b.assert_eq(format!("wr_iswrite[{t}]"), wr_iswrite[t], is_alu_write_01);
		b.assert_eq(format!("ld_addr[{t}]"), ld_addr[t], ld_addr_w);
		b.assert_eq(format!("ld_ver[{t}]"), ld_ver[t], read_ram_ver);
		b.assert_eq(format!("is_load[{t}]"), is_load[t], is_load_01);
		b.assert_eq(format!("st_addr[{t}]"), st_addr[t], st_addr_w);
		b.assert_eq(format!("st_ver[{t}]"), st_ver[t], store_new_ver);
		b.assert_eq(format!("st_val[{t}]"), st_val[t], rs2v);
		b.assert_eq(format!("is_store[{t}]"), is_store[t], is_store_01);
	}

	// T3 init / final / output (verifier-side pinning)
	for r in 0..NREG {
		b.assert_eq(format!("init_regs[{r}]"), init_regs[r], zero);
		b.assert_eq(format!("final_regs[{r}]"), final_regs[r], cur_reg[r]);
	}
	for a in 0..NRAM {
		b.assert_eq(format!("fin_ver[{a}]"), fin_ver[a], c_ram_ver[a]);
	}
	let iref = InoutRefs { inst, pc, rd1_reg, rd1_ver, rd1_val, rd2_reg, rd2_ver, rd2_val, wr_reg, wr_ver, wr_val, wr_iswrite, ld_addr, ld_ver, ld_val, is_load, st_addr, st_ver, st_val, is_store, init_regs, final_regs, fin_ver };
	(b.build(), iref)
}
