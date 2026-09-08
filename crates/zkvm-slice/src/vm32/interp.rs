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
	/// M8-B T2：div/divu 的 advice 商（非除法周期填 0）。
	pub m_q: u32,
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
		let mut mem_addr = (addr_calc % NRAM as u64) as usize;

				let mut load = None;
		let mut store = None;
		// M8-B T2 地址语义：lw/sw 沿用 M5 的「地址即字索引」（历史程序兼容，imm 步长 1）；
		// 新字节/半字访存（lb/lbu/lh/lhu/sb/sh）按 RISC-V 字节地址：
		// 字索引 = (addr>>2) mod NRAM，字内偏移 = addr & 3（lh/sh 需 addr[0]==0 半字对齐）。
		let byte_word_index = |addr: u64| ((addr >> 2) as usize) % NRAM;
		let mut ld_wb: Option<u32> = None; // 写回值（byte/half load 的提取结果）
		if opcode == OP_LOAD && matches!(funct3, F3_LB | F3_LBU | F3_LH | F3_LHU) {
			// M8-B T2：事件列记录整字 raw（RAM 字粒度，与排序论证 val_cons 一致）；提取在写回级。
			let ld_addr = byte_word_index(addr_calc);
			let v = ramver[ld_addr];
			let w = mem[ld_addr];
			let off = (addr_calc & 3) as u32;
			let val = match funct3 {
				F3_LB => ((((w >> (8 * off)) & 0xff) as u64) as i8 as i32) as u32,
				F3_LBU => (w >> (8 * off)) & 0xff,
				F3_LH => ((((w >> (16 * (off >> 1))) & 0xffff) as u64) as i16 as i32) as u32,
				F3_LHU => (w >> (16 * (off >> 1))) & 0xffff,
				_ => unreachable!(),
			};
			ld_wb = Some(val);
			load = Some(MemAccess { addr: ld_addr, ver: v, val: w });
		} else if opcode == OP_LOAD && funct3 == 0x2 {
			let v = ramver[mem_addr];
			let mut val = mem[mem_addr];
			for &(cyc, ov) in load_overrides { if cyc == cycles.len() { val = ov; } }
			load = Some(MemAccess { addr: mem_addr, ver: v, val });
		} else if opcode == OP_STORE && matches!(funct3, F3_SB | F3_SH) {
			// M9 T2：sb/sh 展开为「读旧字 + 写新字」双事件（同周期 load+store）。
			// 读事件走 RAM 论证的读语义（ver=v），写事件 ver=v+1——版本链与 val_cons
			// （同地址读值一致链：旧字读行 → 新字写行）无需修改排序论证。
			let st_addr = (a.wrapping_add(imm_s as u64)) & 0xffffffff;
			let st_wi = byte_word_index(st_addr);
			let v_old = ramver[st_wi];
			let old = mem[st_wi];
			let v_new = v_old + 1;
			let off = (st_addr & 3) as u32;
			let rs2v = regs[rs2 as usize];
			let merged = match funct3 {
				F3_SB => {
					let shift = 8 * off;
					(old & !(0xffu32 << shift)) | ((rs2v & 0xff) << shift)
				}
				F3_SH => {
					let shift = 16 * (off >> 1);
					(old & !(0xffffu32 << shift)) | ((rs2v & 0xffff) << shift)
				}
				_ => unreachable!(),
			};
			load = Some(MemAccess { addr: st_wi, ver: v_old, val: old });
			store = Some(MemAccess { addr: st_wi, ver: v_new, val: merged });
			mem_addr = st_wi;
		} else if opcode == OP_STORE && funct3 == 0x2 {
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
		let mut m_q: u32 = 0;
		if is_reg {
			let (x, y) = (regs[rs1 as usize] as u32, regs[rs2 as usize] as u32);
			if funct7 == 0x01 {
				// advice 商：div 有符号取 |x|/|y|；divu 取 x/y；mul/rem 无商要求（0 即可，
				// mul 的 m_res 不读 q；rem 的断言走有符号列需要正确 q）
				m_q = match funct3 {
					0x4 => {
						let ax = (x as i32).wrapping_abs();
						let ay = (y as i32).wrapping_abs();
						if ay == 0 { u32::MAX } else { (ax as i64 / ay as i64) as u32 }
					}
					0x5 => {
						if y == 0 { u32::MAX } else { x / y }
					}
					0x6 => {
						let ax = (x as i32).wrapping_abs();
						let ay = (y as i32).wrapping_abs();
						if ay == 0 { u32::MAX } else { (ax as i64 / ay as i64) as u32 }
					}
					0x7 => {
						if y == 0 { u32::MAX } else { x / y }
					}
					_ => 0,
				};
				// M8-B T2：RV32M。RISC-V 语义：除零 div→-1/rem→x、remu→x、
				// 溢出（MIN ÷ -1）div→MIN/rem→0。native 用 wrapping 语义直接表达。
				alu_sum = match funct3 {
					0x0 => x.wrapping_mul(y),
					0x4 => {
						if y == 0 {
							u32::MAX // RISC-V：div 除零 → -1
						} else {
							(x as i32).wrapping_div(y as i32) as u32
						}
					}
					0x5 => {
						if y == 0 { u32::MAX } else { x / y }
					}
					0x6 => {
						if y == 0 {
							x // RISC-V：rem 除零 → 被除数
						} else {
							(x as i32).wrapping_rem(y as i32) as u32
						}
					}
					0x7 => {
						if y == 0 { x } else { x % y }
					}
					_ => 0,
				};
			} else {
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
		} else if opcode == OP_LOAD {
			// M8-B T2：lb/lbu/lh/lhu 写回提取值（ld_wb），lw 写回整字
			alu_sum = ld_wb.unwrap_or_else(|| load.unwrap().val);
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

		cycles.push(Cycle { pc, inst, reads, write, regver: cycle_rv, load, store, ramver: cycle_ramv, mem_addr, m_q });
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
