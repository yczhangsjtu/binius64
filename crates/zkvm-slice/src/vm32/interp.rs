//! vm32 native reference interpreter (`run_program`) and its trace types.
//! Mechanically extracted from the former single-file `word_vm32.rs` (M6 T1); logic identical.

use crate::vm32::isa::*;

pub struct RegAccess { pub reg: usize, pub ver: usize, pub val: u32 }
#[derive(Clone, Copy, Debug)]
pub struct MemAccess { pub addr: usize, pub ver: usize, pub val: u32 }
pub struct Cycle {
	pub pc: u64,
	pub inst: u64,
	pub reads: Vec<RegAccess>,
	pub write: Option<RegAccess>,
	pub regver: [usize; NREG],
	pub load: Option<MemAccess>,
	pub store: Option<MemAccess>,
	pub ramver: [usize; NRAM],
	pub mem_addr: usize,

}
#[allow(dead_code)] // final_mem read by the R5 bubblesort test (cfg(test)) only
pub struct Trace { pub cycles: Vec<Cycle>, pub final_regs: [u32; NREG], pub final_ramver: [usize; NRAM], pub final_mem: [u32; NRAM] }
pub fn run_program(init_mem: &[u32; NRAM], word_overrides: &[(u64, u64)], load_overrides: &[(usize, u32)], fetch: fn(u64) -> u64) -> Trace {
	let mut regs = [0u32; NREG];
	let mut rver = [0usize; NREG];
	let mut mem = *init_mem;
	let mut ramver = [0usize; NRAM];
	let mut pc = PC_START;
	let mut cycles = Vec::new();
	let mut guard = 0;
	let fetch = |addr: u64| -> u64 {
		for &(a, w) in word_overrides { if a == addr { return w; } }
		fetch(addr)
	};
	loop {
		guard += 1;
		if guard > 4096 { panic!("runaway execution"); }
		let inst = fetch(pc);
		let (opcode, rd, rs1, rs2, funct3, funct7) = fn_rv32(inst);
		let imm_i = sext(inst >> 20 & 0xfff, 12);
		let imm_s = sext(((inst >> 25 & 0x7f) << 5) | (inst >> 7 & 0x1f), 12);
		let imm_b = sext(((inst >> 31 & 1) << 12) | ((inst >> 7 & 1) << 11) | ((inst >> 25 & 0x3f) << 5) | ((inst >> 8 & 0x0f) << 1), 13);
		let imm_u = ((inst >> 12) & 0xfffff) << 12;
		let imm_j = sext(((inst >> 31 & 1) << 20) | ((inst >> 12 & 0xff) << 12) | ((inst >> 20 & 1) << 11) | ((inst >> 21 & 0x3ff) << 1), 21);
		let cycle_rv = rver;
		let cycle_ramv = ramver;

		let reads = vec![
			RegAccess { reg: rs1 as usize, ver: rver[rs1 as usize], val: regs[rs1 as usize] },
			RegAccess { reg: rs2 as usize, ver: rver[rs2 as usize], val: regs[rs2 as usize] },
		];
		let a = regs[rs1 as usize] as i64 as u64 & 0xffffffff;

		let addr_calc = ((a.wrapping_add(imm_i as u64)) & 0xffffffff) as u64;
		let mem_addr = (addr_calc % NRAM as u64) as usize;

				let mut load = None;
		let mut store = None;
		if opcode == OP_LOAD && funct3 == 0x2 {
			let v = ramver[mem_addr];
			let mut val = mem[mem_addr];
			for &(cyc, ov) in load_overrides { if cyc == cycles.len() { val = ov; } }
			load = Some(MemAccess { addr: mem_addr, ver: v, val });
		} else if opcode == OP_STORE && funct3 == 0x2 {
			// S-type sign-extended offset (imm_s, at inst[31:25]+inst[11:7]) — NOT imm_i, whose
			// low 5 bits alias rs2 (this was a latent S-type store-address bug before R5).
			let st_addr = (a.wrapping_add(imm_s as u64)) & 0xffffffff;
			let mem_addr = (st_addr % NRAM as u64) as usize;
			let v = ramver[mem_addr] + 1;
			store = Some(MemAccess { addr: mem_addr, ver: v, val: regs[rs2 as usize] });
		}

		let is_branch_cmp = opcode == OP_BRANCH;
		let taken = if is_branch_cmp {
			let rs1v = regs[rs1 as usize];
			let rs2v = regs[rs2 as usize];
			match funct3 {
				0x0 => rs1v == rs2v,
				0x1 => rs1v != rs2v,
				0x4 => (rs1v as i32) < (rs2v as i32),
				0x5 => (rs1v as i32) >= (rs2v as i32),
				0x6 => rs1v < rs2v,
				0x7 => rs1v >= rs2v,
				_ => false,
			}
		} else if opcode == OP_JAL { true }
		else if opcode == OP_JALR { true }
		else { false };

		// ALU result (32-bit) for register write-back
		let mut alu_sum: u32 = 0;
		let mut is_alu_write = false;
		let is_imm = opcode == OP_OPIMM;
		let is_reg = opcode == OP_OP;
		let rd_mask = rd as usize;
		if is_reg {
			let (x, y) = (regs[rs1 as usize] as u32, regs[rs2 as usize] as u32);
			match funct3 {
				0x0 => alu_sum = if funct7 == 0x20 { x.wrapping_sub(y) } else { x.wrapping_add(y) },
				0x1 => alu_sum = x << (y & 0x1f),
				0x2 => alu_sum = ((x as i32) < (y as i32)) as u32,
				0x3 => alu_sum = (x < y) as u32,
				0x4 => alu_sum = x ^ y,
				0x5 => alu_sum = if funct7 == 0x20 { ((x as i32) >> (y & 0x1f)) as u32 } else { x >> (y & 0x1f) },
				0x6 => alu_sum = x | y,
				0x7 => alu_sum = x & y,
				_ => {}
			}
			is_alu_write = true;
		} else if is_imm {
			let x = regs[rs1 as usize] as u32;
			let imm12 = (imm_i as u64) as u32;
			match funct3 {
				0x0 => alu_sum = x.wrapping_add(imm12),
				0x1 => alu_sum = x << ((inst >> 20) & 0x1f),
				0x2 => alu_sum = ((x as i32) < (imm_i as i32)) as u32,
				0x3 => alu_sum = (x < imm12) as u32,
				0x4 => alu_sum = x ^ imm12,
				0x5 => alu_sum = if funct7 & 0x20 != 0 { ((x as i32) >> ((inst >> 20) & 0x1f)) as u32 } else { x >> ((inst >> 20) & 0x1f) },
				0x6 => alu_sum = x | imm12,
				0x7 => alu_sum = x & imm12,
				_ => {}
			}
			is_alu_write = true;
		} else if opcode == OP_LUI {
			alu_sum = imm_u as u32;
			is_alu_write = true;
		} else if opcode == OP_AUIPC {
			alu_sum = (pc as u32).wrapping_add(imm_u as u32);
			is_alu_write = true;
		} else if opcode == OP_JAL || opcode == OP_JALR {
			alu_sum = (pc as u32).wrapping_add(4);
			is_alu_write = true;
		} else if opcode == OP_LOAD && funct3 == 0x2 {
			alu_sum = load.unwrap().val;
			is_alu_write = true;
		}

		let write = if is_alu_write {
			// x0 is hard-zero: writing to x0 is dropped
			if rd_mask == 0 {
				rver[0] += 1; // still count version (but value stays 0) — for soundness(6) we keep value pinned to 0
				regs[0] = 0;
				Some(RegAccess { reg: 0, ver: rver[0], val: 0 })
			} else {
				rver[rd_mask] += 1;
				regs[rd_mask] = alu_sum;
				Some(RegAccess { reg: rd_mask, ver: rver[rd_mask], val: alu_sum })
			}
		} else { None };

		if let Some(s) = &store { mem[s.addr] = s.val; ramver[s.addr] = s.ver; }

		let next_pc = if opcode == OP_JAL {
			(pc as i64 + imm_j as i64) as u64
		} else if opcode == OP_JALR {
			((regs[rs1 as usize] as u64).wrapping_add(imm_i as u64) & !1) & 0xffffffff
		} else if is_branch_cmp && taken {
			(pc as i64 + imm_b as i64) as u64
		} else {
			pc.wrapping_add(4)
		};

		cycles.push(Cycle { pc, inst, reads, write, regver: cycle_rv, load, store, ramver: cycle_ramv, mem_addr });
		if pc == HALT_ADDR { break; }
		pc = next_pc;
	}
	Trace { cycles, final_regs: regs, final_ramver: ramver, final_mem: mem }
}

// bubblesort program image (R5): sorts 8 words at mem[0..8] ascending-unsigned with
// duplicate + 0x80000000 coverage. Loop-carried register writes: x2/x4/x5/x6 hit ~56
// (VER_MAX=128). PC covers 0x00..0x34 then jal to the shared halt row 0xc4; every row the
// PC can reach is overridden, untouched fetch-table rows are never fetched.

#[cfg(test)]
#[path = "per_inst_tests.rs"]
mod per_inst_tests;
