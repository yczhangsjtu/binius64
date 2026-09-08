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
	/// M8-B T2：div/divu 的 advice 商（公开 inout，电路断言 q·b+r==a ∧ r<b）。
	pub m_q: Vec<Wire>,
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
	// M8-B T2：div 商 advice（每周期 1 词；rem = a - q·b 从商导出，无需独立 advice）
	let m_q = (0..t_len).map(|_| b.add_inout()).collect::<Vec<_>>();

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
		// M12-T3（M1）：非标 LOAD/STORE funct3 ∈ {3,6,7} 统一为 NOP（interp 同步）——
		// 修复前电路把 funct3=3 当半字 load、interp 直接 panic（同指令两套语义，审计 M1）。
		let f3_low2 = b.band(funct3, b.add_constant_64(3));
		let f3_invalid = b.bor(b.icmp_eq(f3_low2, b.add_constant_64(3)), b.icmp_eq(funct3, b.add_constant_64(6)));
		let f3_valid = b.bnot(f3_invalid);
		let c_is_load = b.band(eq_opcode(OP_LOAD), f3_valid); // M8-B T2：lb/lbu/lh/lhu/lw 全家
		let c_is_store = b.band(eq_opcode(OP_STORE), f3_valid); // M9 T2：sb/sh/sw 全家
		let is_byte_load = b.band(c_is_load, b.bnot(b.icmp_eq(funct3, b.add_constant_64(0x2))));
		// M9 T2：sb/sh（字节地址语义）——同周期「读旧字 + 写新字」双事件
		let is_byte_store = b.band(c_is_store, b.bnot(b.icmp_eq(funct3, b.add_constant_64(0x2))));
		let is_m_ext = b.band(is_risc, b.icmp_eq(funct7, one)); // RV32M：funct7=0x01

		// read register values/versions
		let rs1v = mux(&b, &cur_reg, rs1);
		let rs2v = mux(&b, &cur_reg, rs2);
		let rs1_ver = mux(&b, &cur_ver, rs1);
		let rs2_ver = mux(&b, &cur_ver, rs2);

		// memory address = rs1 + sext(imm): I-imm for lw, S-imm (sign-extended, imm_s) for sw.
		// (S-type offset lives at inst[31:25]+inst[11:7], NOT imm_i whose low 5 bits alias rs2.)
		let ld_byte_addr = b.iadd_32(rs1v, imm_i);
		let ld_word_idx = b.band(b.srl32(ld_byte_addr, 2), b.add_constant_64(0x3f));
		// M8-B T2：lw 沿用「地址即字索引」；字节/半字 load 用 (addr>>2)&0x3f
		let ld_addr_w = b.select(is_byte_load, ld_word_idx, b.band(b.iadd_32(rs1v, imm_i), b.add_constant_64(0x3f)));
		let st_byte_addr = b.iadd_32(rs1v, imm_s);
		let st_word_from_byte = b.band(b.srl32(st_byte_addr, 2), b.add_constant_64(0x3f));
		let st_addr_w = b.select(is_byte_store, st_word_from_byte,
			b.band(b.select(c_is_store, b.iadd_32(rs1v, imm_s), b.iadd_32(rs1v, imm_i)), b.add_constant_64(0x3f))); // S-imm on store, I-imm otherwise (matches witness fallback)
		// M9 T2：sb/sh 周期的读事件地址 = store 字地址（读旧字）；其余周期 = load 地址
		let eff_ld_addr = b.select(is_byte_store, st_addr_w, ld_addr_w);
		let read_ram_ver = mux(&b, &c_ram_ver, eff_ld_addr);
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
		// ---- RV32M 展开验证（M11 F1 修复：审计 S3 三层）----
		// ① MSB 直读：s_ok/uns_ok 是 MSB-bool（band of icmp 输出），bad 归一用
		//   `select(msb, zero, one)` 直读 MSB——**禁止**再经全词 icmp_eq（低 63 位未定义，
		//   归一恒假 → 断言恒真，审计 S3-①）。
		// ② 关系：断言 `r < y`（r = x − q·y 由定义即 q·y+r==x mod 2^32），**非** q·y==x
		//   （旧代码误写整除约束，配合①被掩盖，审计 S3-②）。
		// ③ 环绕防护：断言 imul 高位 == 0（q·y < 2^32），否则 lo 是 mod-2^32 多解
		//   （审计 S3-③）。
		let q = m_q[t];
		let x32 = rs1v;
		let y32 = rs2v;
		let prod_u = b.imul(q, y32);
		let hi_u_is0 = b.icmp_eq(prod_u.0, zero); // 无环绕（③）
		let qy32 = b.band(prod_u.1, b.add_constant_64(0xffffffff));
		let r_u = b.band(b.isub_bin_bout(x32, qy32, zero).0, b.add_constant_64(0xffffffff));
		let y_is0 = b.icmp_eq(y32, zero);
		let q_is_max = b.icmp_eq(q, b.add_constant_64(0xffffffff));
		// 无符号：y≠0 ⇒ 无环绕 ∧ r<y；y=0 ⇒ q=MAX（r=x 自动成立）
		let uns_ok = b.select(y_is0, q_is_max, b.band(hi_u_is0, b.icmp_ult(r_u, y32)));
		let sign_x = b.shl(b.band(b.srl32(x32, 31), one), 63); // 0/1 → MSB-bool（select 条件约定）
		let sign_y = b.shl(b.band(b.srl32(y32, 31), one), 63);
		let neg32 = |v: Wire| b.band(b.iadd_32(b.bnot(v), one), b.add_constant_64(0xffffffff));
		let abs_x = b.select(sign_x, neg32(x32), x32);
		let abs_y = b.select(sign_y, neg32(y32), y32);
		let prod_s = b.imul(q, abs_y);
		let hi_s_is0 = b.icmp_eq(prod_s.0, zero); // 无环绕（③）
		let prod_s_lo = b.band(prod_s.1, b.add_constant_64(0xffffffff));
		let r_s = b.band(b.isub_bin_bout(abs_x, prod_s_lo, zero).0, b.add_constant_64(0xffffffff));
		let abs_y0 = b.icmp_eq(abs_y, zero);
		// 有符号：|y|≠0 ⇒ 无环绕 ∧ r<|y|；|y|=0 ⇒ q=MAX（div→-1 语义，r=|x| 自动）
		let s_ok = b.select(abs_y0, q_is_max, b.band(hi_s_is0, b.icmp_ult(r_s, abs_y)));
		let is_signed_m = b.icmp_eq(b.band(funct3, one), zero); // div(4)/rem(6) 偶，divu/remu 奇
		let s_bad = b.select(s_ok, zero, one); // ① MSB 直读归一
		let uns_bad = b.select(uns_ok, zero, one); // ①
		let m_bad = b.select(is_signed_m, s_bad, uns_bad);
		let is_div_family01 = bool01(&b, b.band(is_m_ext, b.icmp_ult(b.add_constant_64(3), funct3))); // funct3 ∈ 4..7
		b.assert_eq(format!("m_assert[{t}]"), b.band(is_div_family01, m_bad), zero);
		// M11 F1：div_v/rem_v 的导出在上述约束成立时唯一（q 被 r<y 与无环绕唯一化）；
		// 除零分支保持 RISC-V 语义（div→-1、rem→x）；MIN÷−1 溢出在模 2^32 下自动正确。
		let neg_div = b.bxor(sign_x, sign_y);
		let div_v = b.select(y_is0, b.add_constant_64(0xffffffff), b.select(neg_div, neg32(q), q));
		let rem_v = b.select(y_is0, x32, b.select(sign_x, neg32(r_s), r_s));
		let mul_lo = b.band(b.imul(x32, y32).1, b.add_constant_64(0xffffffff));
		let m_res = mux8(&b, &[mul_lo, zero, zero, zero, div_v, q, rem_v, r_u], funct3);
		// ---- M8-B T2：字节/半字 load 提取（RAM 字粒度：事件列记录整字 raw，提取在写回级）----
		let ld_off = b.band(ld_byte_addr, b.add_constant_64(3));
		let byte_shift = b.sll32(ld_off, 3);
		let byte_raw = b.band(shr_var(&b, ld_val[t], byte_shift), b.add_constant_64(0xff));
		let byte_sext = sext_w(&b, byte_raw, 8);
		let half_shift = b.sll32(b.band(b.srl32(ld_off, 1), one), 4);
		let half_raw = b.band(shr_var(&b, ld_val[t], half_shift), b.add_constant_64(0xffff));
		let half_sext = sext_w(&b, half_raw, 16);
		let is_half_load = b.icmp_eq(b.band(funct3, one), one); // lh(1)/lhu(5)
		let is_signed_load = b.icmp_eq(b.srl32(funct3, 2), zero); // lb/lh bit2=0；lbu/lhu bit2=1
		let load_ext = b.select(is_half_load, half_raw, byte_raw);
		let load_ext_sext = b.select(is_half_load, half_sext, byte_sext);
		let load_extracted = b.select(is_signed_load, load_ext_sext, load_ext);
		// lh/lhu 半字对齐断言（addr[0]==0）
		let is_byte01 = bool01(&b, is_byte_load);
		let is_half01 = bool01(&b, is_half_load);
		let off_is_1 = b.select(b.icmp_eq(b.band(ld_byte_addr, one), one), one, zero);
		let lh_misaligned = b.band(b.band(is_byte01, is_half01), off_is_1);
		b.assert_eq(format!("lh_align[{t}]"), lh_misaligned, zero);
		// M12-T3（M2）：sh 半字对齐断言（与 lh 对称；修复前 isa.rs 虚标已做）
		let is_half_store01 = bool01(&b, b.icmp_eq(funct3, b.add_constant_64(F3_SH)));
		let st_off_is_1 = b.select(b.icmp_eq(b.band(st_byte_addr, one), one), one, zero);
		let sh_misaligned = b.band(b.band(bool01(&b, is_byte_store), is_half_store01), st_off_is_1);
		b.assert_eq(format!("sh_align[{t}]"), sh_misaligned, zero);
		// lw 之外的 load 写回 = 提取值；lw 保持整字
		let load_wb = b.select(is_byte_load, load_extracted, ld_val[t]);
		let alu_sum = b.select(is_imm, alu_core,
			b.select(b.band(is_risc, is_m_ext), m_res,
			b.select(is_risc, alu_core,
			b.select(c_is_load, load_wb,
			b.select(is_lui, lui_v,
			b.select(is_auipc, auipc_v,
			b.select(is_jal, jal_rd,
			b.select(is_jalr, jal_rd, zero))))))));

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
		// M9 T2：sb/sh 的合并字 = f(旧字=ld_val[t], rs2, off)——电路可算（旧字经 RAM 论证钉住）
		let sb_off = b.band(st_byte_addr, b.add_constant_64(3));
		let sb_shift = b.sll32(sb_off, 3);
		let sb_mask = shl_var(&b, b.add_constant_64(0xff), sb_shift);
		let sb_ins = shl_var(&b, b.band(rs2v, b.add_constant_64(0xff)), sb_shift);
		let sb_merged = b.bor(b.band(ld_val[t], b.bnot(sb_mask)), sb_ins);
		let sh_shift = b.sll32(b.band(b.srl32(sb_off, 1), one), 4);
		let sh_mask = shl_var(&b, b.add_constant_64(0xffff), sh_shift);
		let sh_ins = shl_var(&b, b.band(rs2v, b.add_constant_64(0xffff)), sh_shift);
		let sh_merged = b.bor(b.band(ld_val[t], b.bnot(sh_mask)), sh_ins);
		let byte_store_merged = b.select(b.icmp_eq(funct3, b.add_constant_64(F3_SB)), sb_merged, sh_merged);
		let is_alu_write_01 = b.select(is_alu_write, one, zero);
		let eff_load = b.bor(c_is_load, is_byte_store); // sb/sh 周期的读旧字事件
		let is_load_01 = b.select(eff_load, one, zero);
		let is_store_01 = b.select(c_is_store, one, zero);
		let rd_ver_now = mux(&b, &cur_ver, rd);
		let rd_ver_after = rd_ver_now; // cur_ver already updated to post-write state
		// M11 F5：版本上界——写前版本必须 ≤ VER_MAX−2（写后 ver+1 ≤ VER_MAX−1，
		// 防止 reg*VER_MAX+ver 别名到相邻寄存器的行；ver_at_max 为 0/1）
		let ver_at_max = bool01(&b, b.icmp_ult(b.add_constant_64((VER_MAX - 1) as u64), rd_ver_now));
		b.assert_eq(format!("ver_bound[{t}]"), b.band(is_alu_write_01, ver_at_max), zero);
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
		b.assert_eq(format!("ld_addr[{t}]"), ld_addr[t], eff_ld_addr);
		b.assert_eq(format!("ld_ver[{t}]"), ld_ver[t], read_ram_ver);
		// M12-T3（M3）：ld_val 的 32 位范围（RAM 词 = u32；高 32 位必须为零）
		b.assert_eq(format!("ld_val_range[{t}]"), b.band(ld_val[t], b.add_constant_64(0xffffffff00000000)), zero);
		b.assert_eq(format!("is_load[{t}]"), is_load[t], is_load_01);
		b.assert_eq(format!("st_addr[{t}]"), st_addr[t], st_addr_w);
		b.assert_eq(format!("st_ver[{t}]"), st_ver[t], store_new_ver);
		b.assert_eq(format!("st_val[{t}]"), st_val[t], b.select(is_byte_store, byte_store_merged, rs2v));
		b.assert_eq(format!("is_store[{t}]"), is_store[t], is_store_01);
	}

	// M11 F4：终止约束——末周期必须执行 HALT 行（pc == HALT_ADDR；防任意截断前缀证明）
	b.assert_eq("final_pc_halt", pc[t_len - 1], b.add_constant_64(HALT_ADDR));
	// T3 init / final / output (verifier-side pinning)
	for r in 0..NREG {
		b.assert_eq(format!("init_regs[{r}]"), init_regs[r], zero);
		b.assert_eq(format!("final_regs[{r}]"), final_regs[r], cur_reg[r]);
	}
	for a in 0..NRAM {
		b.assert_eq(format!("fin_ver[{a}]"), fin_ver[a], c_ram_ver[a]);
	}
	let iref = InoutRefs { inst, pc, rd1_reg, rd1_ver, rd1_val, rd2_reg, rd2_ver, rd2_val, wr_reg, wr_ver, wr_val, wr_iswrite, ld_addr, ld_ver, ld_val, is_load, st_addr, st_ver, st_val, is_store, init_regs, final_regs, fin_ver, m_q };
	(b.build(), iref)
}
