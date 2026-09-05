//! Shared RISC-V-style instruction-word encoding helpers.
//!
//! Real RV32I layout (subset): [31:20] imm | [19:15] rs1 | [14:12] funct3 |
//! [11:7] rd | [6:0] opcode. We model the low 8/32 bits for our subset. These
//! were previously duplicated in multi_inst / multi_combined / zkvm.

/// Opcodes (low 7 bits, real RISC-V).
pub const OP_ADDI: u64 = 0x13;
pub const OP_ADD: u64 = 0x33;
pub const OP_LW: u64 = 0x03;
pub const OP_SW: u64 = 0x23;
pub const OP_BEQ: u64 = 0x63;

/// funct3 constants.
pub const F3_ADD: u64 = 0x0;
pub const F3_LW: u64 = 0x2;
pub const F3_SW: u64 = 0x2;
pub const F3_BEQ: u64 = 0x0;

/// Register numbers (our small file; x0 is the zero register).
pub const REG_X0: u64 = 0;
pub const REG_X1: u64 = 1;
pub const REG_X2: u64 = 2;
pub const REG_X3: u64 = 3;
pub const REG_X5: u64 = 5;
pub const REG_X6: u64 = 6;

/// Extract fields from a word (subsets). `imm` is the top bits for I-type.
pub fn word_opcode(w: u64) -> u64 { w & 0x7f }
pub fn word_rd(w: u64) -> u64 { (w >> 7) & 0x1f }
pub fn word_funct3(w: u64) -> u64 { (w >> 12) & 0x7 }
pub fn word_rs1(w: u64) -> u64 { (w >> 15) & 0x1f }
pub fn word_rs2(w: u64) -> u64 { (w >> 20) & 0x1f }
pub fn word_imm(w: u64) -> u64 { (w >> 25) & 0x7f }
pub fn word_bimm(w: u64) -> u64 { (w >> 25) & 0x7f }

pub fn enc_addi(imm: u64, rd: u64, rs1: u64) -> u64 {
	(OP_ADDI & 0x7f) | ((rd & 0x1f) << 7) | ((F3_ADD & 0x7) << 12) | ((rs1 & 0x1f) << 15) | ((imm & 0x7f) << 25)
}
pub fn enc_add(rd: u64, rs1: u64, rs2: u64) -> u64 {
	(OP_ADD & 0x7f) | ((rd & 0x1f) << 7) | ((F3_ADD & 0x7) << 12) | ((rs1 & 0x1f) << 15) | ((rs2 & 0x1f) << 20)
}
pub fn enc_lw(rd: u64, rs1: u64) -> u64 {
	(OP_LW & 0x7f) | ((rd & 0x1f) << 7) | ((F3_LW & 0x7) << 12) | ((rs1 & 0x1f) << 15)
}
pub fn enc_sw(rs1: u64, rs2: u64) -> u64 {
	(OP_SW & 0x7f) | ((F3_SW & 0x7) << 12) | ((rs1 & 0x1f) << 15) | ((rs2 & 0x1f) << 20)
}
pub fn enc_beq(rs1: u64, rs2: u64, bimm: u64) -> u64 {
	(OP_BEQ & 0x7f) | ((rs1 & 0x1f) << 7) | ((rs2 & 0x1f) << 12) | ((F3_BEQ & 0x7) << 15) | ((bimm & 0x7f) << 25)
}
