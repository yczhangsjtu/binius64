//! vm32 ISA layer: real RV32I (subset) word encoders, decode (`fn_rv32`), and shared
//! machine constants. Mechanically extracted from the former single-file `word_vm32.rs`
//! (M6 T1); logic identical to the pre-split implementation.

pub const NREG: usize = 32;
pub const NRAM: usize = 64;
pub const PC_START: u64 = 0x00;
pub const HALT_ADDR: u64 = 0xc4; // torture program size (R2-expanded): 50 instructions
pub const OP_LUI: u64 = 0x37;
pub const OP_AUIPC: u64 = 0x17;
pub const OP_JAL: u64 = 0x6f;
pub const OP_JALR: u64 = 0x67;
pub const OP_BRANCH: u64 = 0x63;
pub const OP_LOAD: u64 = 0x03;
pub const OP_STORE: u64 = 0x23;
pub const OP_OPIMM: u64 = 0x13;
pub const OP_OP: u64 = 0x33;
pub const M_FETCH: usize = 8;
pub const VER_MAX: usize = 128; // max writes per reg/addr; bubblesort inner-loop regs hit ~56
pub const M_W_REG: usize = 12; // log2(32*128)=log2(4096)
pub const M_W_RAM: usize = 13; // log2(64*128)=log2(8192)
pub fn enc_r(funct7: u64, rs2: u64, rs1: u64, funct3: u64, rd: u64, opcode: u64) -> u64 {
	opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | (funct7 << 25)
}
pub fn enc_i(opcode: u64, funct3: u64, rd: u64, rs1: u64, imm12: u64) -> u64 {
	opcode | (rd << 7) | (funct3 << 12) | (rs1 << 15) | ((imm12 & 0xfff) << 20)
}
pub fn enc_s(opcode: u64, funct3: u64, rs1: u64, rs2: u64, imm12: u64) -> u64 {
	let imm = imm12 & 0xfff;
	opcode | ((imm & 0x1f) << 7) | (funct3 << 12) | (rs1 << 15) | (rs2 << 20) | ((imm >> 5) << 25)
}
pub fn enc_b(opcode: u64, funct3: u64, rs1: u64, rs2: u64, imm13: u64) -> u64 {
	let imm = imm13 & 0x1fff;
	opcode | ((imm >> 1 & 0x0f) << 8) | ((imm >> 5 & 0x3f) << 25) | ((imm >> 11 & 1) << 7) | ((imm >> 12 & 1) << 31)
		| (funct3 << 12) | (rs1 << 15) | (rs2 << 20)
}
pub fn enc_u(opcode: u64, rd: u64, imm20: u64) -> u64 {
	opcode | (rd << 7) | ((imm20 & 0xfffff) << 12)
}
pub fn enc_j(opcode: u64, rd: u64, imm21: u64) -> u64 {
	let imm = imm21 & 0x1fffff;
	opcode | (rd << 7) | ((imm >> 12 & 0xff) << 12) | ((imm >> 11 & 1) << 20) | ((imm >> 1 & 0x3ff) << 21) | ((imm >> 20 & 1) << 31)
}

// U/J
pub fn lhs_lui(rd: u64, imm20: u64) -> u64 { enc_u(OP_LUI, rd, imm20) }
pub fn auipc(rd: u64, imm20: u64) -> u64 { enc_u(OP_AUIPC, rd, imm20) }
pub fn jal(rd: u64, imm21: u64) -> u64 { enc_j(OP_JAL, rd, imm21) }
pub fn jalr(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_JALR, 0, rd, rs1, imm12) }
// B
pub fn beq(rs1: u64, rs2: u64, imm13: u64) -> u64 { enc_b(OP_BRANCH, 0x0, rs1, rs2, imm13) }
pub fn bne(rs1: u64, rs2: u64, imm13: u64) -> u64 { enc_b(OP_BRANCH, 0x1, rs1, rs2, imm13) }
pub fn blt(rs1: u64, rs2: u64, imm13: u64) -> u64 { enc_b(OP_BRANCH, 0x4, rs1, rs2, imm13) }
pub fn bge(rs1: u64, rs2: u64, imm13: u64) -> u64 { enc_b(OP_BRANCH, 0x5, rs1, rs2, imm13) }
pub fn bltu(rs1: u64, rs2: u64, imm13: u64) -> u64 { enc_b(OP_BRANCH, 0x6, rs1, rs2, imm13) }
pub fn bgeu(rs1: u64, rs2: u64, imm13: u64) -> u64 { enc_b(OP_BRANCH, 0x7, rs1, rs2, imm13) }
// memory
pub fn lhs_lw(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_LOAD, 0x2, rd, rs1, imm12) }
pub fn sw(rs2: u64, rs1: u64, imm12: u64) -> u64 { enc_s(OP_STORE, 0x2, rs1, rs2, imm12) }
// M8-B T2：字节/半字访存
pub fn lb(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_LOAD, F3_LB, rd, rs1, imm12) }
pub fn lh(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_LOAD, F3_LH, rd, rs1, imm12) }
pub fn lbu(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_LOAD, F3_LBU, rd, rs1, imm12) }
pub fn lhu(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_LOAD, F3_LHU, rd, rs1, imm12) }
pub fn sb(rs2: u64, rs1: u64, imm12: u64) -> u64 { enc_s(OP_STORE, F3_SB, rs1, rs2, imm12) }
pub fn sh(rs2: u64, rs1: u64, imm12: u64) -> u64 { enc_s(OP_STORE, F3_SH, rs1, rs2, imm12) }
// I
pub fn addi(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_OPIMM, 0x0, rd, rs1, imm12) }
pub fn slti(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_OPIMM, 0x2, rd, rs1, imm12) }
pub fn sltiu(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_OPIMM, 0x3, rd, rs1, imm12) }
pub fn xori(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_OPIMM, 0x4, rd, rs1, imm12) }
pub fn ori(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_OPIMM, 0x6, rd, rs1, imm12) }
pub fn andi(rd: u64, rs1: u64, imm12: u64) -> u64 { enc_i(OP_OPIMM, 0x7, rd, rs1, imm12) }
pub fn slli(rd: u64, rs1: u64, shamt: u64) -> u64 { enc_i(OP_OPIMM, 0x1, rd, rs1, shamt & 0x1f) }
pub fn srli(rd: u64, rs1: u64, shamt: u64) -> u64 { enc_i(OP_OPIMM, 0x5, rd, rs1, shamt & 0x1f) }
pub fn srai(rd: u64, rs1: u64, shamt: u64) -> u64 { enc_i(OP_OPIMM, 0x5, rd, rs1, 0x400 | (shamt & 0x1f)) }
// R
pub fn add(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x0, rd, OP_OP) }
pub fn sub(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x20, rs2, rs1, 0x0, rd, OP_OP) }
pub fn sll(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x1, rd, OP_OP) }
pub fn slt(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x2, rd, OP_OP) }
pub fn sltu(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x3, rd, OP_OP) }
pub fn xor(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x4, rd, OP_OP) }
pub fn srl(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x5, rd, OP_OP) }
pub fn sra(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x20, rs2, rs1, 0x5, rd, OP_OP) }
pub fn or(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x6, rd, OP_OP) }
pub fn and(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x00, rs2, rs1, 0x7, rd, OP_OP) }

// ---- M8-B T2：RV32M（mul/div/rem，funct7=0x01）----
pub fn mul(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x01, rs2, rs1, 0x0, rd, OP_OP) }
pub fn div(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x01, rs2, rs1, 0x4, rd, OP_OP) }
pub fn divu(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x01, rs2, rs1, 0x5, rd, OP_OP) }
pub fn rem(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x01, rs2, rs1, 0x6, rd, OP_OP) }
pub fn remu(rd: u64, rs1: u64, rs2: u64) -> u64 { enc_r(0x01, rs2, rs1, 0x7, rd, OP_OP) }

// ---- M8-B T2：字节/半字访存（LOAD/STORE 扩展 funct3）----
// 语义（设计详案 §2.7）：字粒度 RAM，地址低 2 位 = 字内字节偏移，字索引 = (addr>>2) mod NRAM；
// lh/lhu/sh 半字对齐断言（addr[0]==0；M12-T3 M2 补齐 sh——修复前本注释虚标「已做」）：
// 违者 interp panic、电路断言拒绝。lw/sw 无对齐语义（本引擎地址即字索引，imm 步长 1）。
// M12-T3（M1）：非标 LOAD/STORE funct3 ∈ {3,6,7} 两层统一为 NOP。
pub const F3_LB: u64 = 0x0;
pub const F3_LH: u64 = 0x1;
pub const F3_LW: u64 = 0x2;
pub const F3_LBU: u64 = 0x4;
pub const F3_LHU: u64 = 0x5;
pub const F3_SB: u64 = 0x0;
pub const F3_SH: u64 = 0x1;
pub const F3_SW: u64 = 0x2;

#[inline]
pub fn sext(v: u64, bits: u32) -> u64 {
	// sign-extend the low `bits` of v to 64 bits
	let m = 1u64 << (bits - 1);
	let v = v & ((1u64 << bits) - 1);
	if v & m != 0 { v | (!0u64 << bits) } else { v }
}
pub fn fn_rv32(inst: u64) -> (u64, u64, u64, u64, u64, u64) {
	(inst & 0x7f, (inst >> 7 & 0x1f), (inst >> 15 & 0x1f), (inst >> 20 & 0x1f), (inst >> 12 & 0x7), (inst >> 25 & 0x7f))
}

/// Legacy `enc_add` retained for the word_add_combined slice (formerly in src/encode.rs).
pub fn enc_add(rd: u64, rs1: u64, rs2: u64) -> u64 {
	(OP_OP & 0x7f) | ((rd & 0x1f) << 7) | ((0x0 & 0x7) << 12) | ((rs1 & 0x1f) << 15) | ((rs2 & 0x1f) << 20)
}
