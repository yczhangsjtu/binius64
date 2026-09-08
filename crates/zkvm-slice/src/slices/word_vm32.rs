//! Slice 26 `word_vm32` (M5) — thin layer after the M6 T1 library split.
//! All machine logic now lives in [`crate::vm32`]; this file keeps only the
//! program images (torture `fetch_word`, bubblesort `bubble_words`), the public
//! `run_word_vm32()` entry, and the test suite.

use crate::vm32::isa::*;
use crate::vm32::proof::run_machine_full;

#[allow(dead_code)] // R5 bubblesort image builder, used by word_vm32_bubble test
fn bubble_words() -> Vec<(u64, u64)> {
	let mut w = vec![
		(0x00u64, addi(3, 0, 7)),        // x3 = 7
		(0x04, beq(1, 3, 0x30)),         // OUTER_TOP: i==7 ? done(0x34)  (imm13=0x34-0x04=48)
		(0x08, addi(2, 0, 0)),           // j = 0
		(0x0c, add(4, 0, 2)),            // INNER_TOP: addr = j
		(0x10, lhs_lw(5, 4, 0)),         // a = mem[j]
		(0x14, lhs_lw(6, 4, 1)),         // b = mem[j+1]
		(0x18, bltu(5, 6, 12)),          // a<b ? skip swap (imm13=0x24-0x18=12)
		(0x1c, sw(6, 4, 0)),             // mem[j] = b
		(0x20, sw(5, 4, 1)),             // mem[j+1] = a
		(0x24, addi(2, 2, 1)),           // NOSWAP: j++
		(0x28, bne(2, 3, 0xffffffe4)),   // j!=7 ? inner (imm13=0x0c-0x28=-28)
		(0x2c, addi(1, 1, 1)),           // i++
		(0x30, bne(1, 3, 0xffffffd4)),   // i!=7 ? outer (imm13=0x04-0x30=-44)
		(0x34, jal(0, 144)),             // done: jump halt 0xc4 (imm21=0xc4-0x34=144)
		(0xc4, addi(0, 0, 0)),           // halt row (same word as torture halt)
	];
	w.sort_by_key(|&(a, _)| a);
	w
}
fn fetch_word(addr: u64) -> u64 {
	match addr {
		0x00 => lhs_lui(1, 0xabcde),      // x1 = 0xabcde000
		0x04 => auipc(2, 0x00000),        // x2 = pc (0x04)
		0x08 => addi(1, 0, 0x7ff),        // x1 = 2047 (boundary)
		0x0c => slti(3, 1, 0x7ff),        // x3 = (2047 < 2047) = 0
		0x10 => sltiu(3, 1, 0x7ff),       // x3 = (2047 <u 2047) = 0
		0x14 => xori(4, 1, 0xfff),          // x4 = x1 ^ 0xffffffff (imm=-1)
		0x18 => ori(4, 4, 0x0ff),         // x4 |= 0xff
		0x1c => andi(4, 4, 0x0f0),        // x4 &= 0xf0
		0x20 => slli(5, 4, 31),           // x5 = x4 << 31
		0x24 => srli(5, 5, 0),            // x5 >> 0
		0x28 => srai(5, 5, 31),           // x5 = arith >> 31
		0x2c => add(6, 1, 3),             // x6 = 2047 + 0
		0x30 => sub(7, 1, 3),             // x7 = 2047 - 0
		0x34 => sll(5, 3, 4),             // x5 = 0 << 4 = 0
		0x38 => slt(5, 1, 3),             // x5 = (2047 < 0) = 0
		0x3c => sltu(5, 1, 3),            // x5 = (2047 <u 0) = 0
		0x40 => xor(5, 1, 3),             // x5 = 2047 ^ 0
		0x44 => srl(5, 5, 4),             // x5 >> 4
		0x48 => sra(5, 5, 4),             // x5 arith >> 4
		0x4c => or(5, 1, 3),              // x5 = 2047 | 0
		0x50 => and(5, 1, 3),             // x5 = 2047 & 0
		0x54 => lhs_lw(8, 0, 24),         // x8 = mem[24]
		0x58 => sw(8, 0, 24),             // mem[24] = x8
		0x5c => beq(1, 3, 4),             // not taken (2047 != 0); target 0x60 (= pc+4)
		0x60 => bne(1, 3, 4),             // taken (2047 != 0); target 0x64
		0x64 => blt(3, 1, 4),             // taken (0 < 2047); target 0x68
		0x68 => bge(1, 1, 4),             // taken (2047 >= 2047); target 0x6c
		0x6c => bltu(1, 3, 4),            // not taken (2047 <u 0); target 0x70
		0x70 => bgeu(3, 1, 4),            // not taken (0 >=u 2047); target 0x74
		0x74 => jal(9, 4),                // x9 = 0x78; target 0x78
		0x78 => jalr(9, 0, 0x7c),         // x9 = 0x7c; target = x0+0x7c = 0x7c
		0x7c => addi(10, 0, 1),           // x10 = 1
		0x80 => addi(0, 0, 0x7ff),   // addi x0,x0,2047 (write to x0 dropped)
		0x84 => andi(0, 0, 0xfff),   // transitional (continues to R2 negative-op segment)
		0x88 => lhs_lui(11, 0x80000), // x11 = 0x80000000 (negative)
		0x8c => srai(11, 11, 31),    // 0x80000000 arith>>31 = 0xFFFFFFFF (R2: negative srai)
		0x90 => lhs_lui(12, 0x80000), // x12 = 0x80000000
		0x94 => sra(12, 12, 31),     // 0x80000000 arith>>31 = 0xFFFFFFFF (R-type sra)
		0x98 => lhs_lui(16, 0x80000), // x16 = 0x80000000, rd=16 (bit11=1 -> old is_sra I-type bug path)
		0x9c => srli(16, 16, 31),    // logical 0x80000000>>31 = 1 (must NOT be treated as sra)
		0xa0 => addi(13, 0, 0xfff),  // x13 = -1 (0xFFFFFFFF)
		0xa4 => addi(14, 0, 1),      // x14 = 1
		0xa8 => slt(15, 13, 14),     // -1 < 1 signed = 1
		0xac => slti(15, 13, 1),     // -1 < 1 = 1
		0xb0 => sltu(15, 13, 14),    // 0xFFFFFFFF <u 1 = 0
		0xb4 => sltiu(15, 13, 1),    // 0xFFFFFFFF <u 1 = 0
		0xb8 => addi(15, 0, 0xff),   // x15 = 0xff
		0xbc => sll(15, 15, 4),      // 0xff << 4 = 0xff0 (non-trivial high bits)
		0xc0 => sub(16, 0, 13),      // 0 - (-1) = 1 (is_sub, negative operand)
		0xc4 => addi(0, 0, 0),       // halt
		_ => 0,
	}
}

pub fn run_word_vm32() {
	let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
	let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
	println!("word_vm32 honest: cycles={} t={} | c_ok={} l_ok={} | n_gates={}", run.trace.cycles.len(), run.t_len, run.c_ok, run.l_ok, run.stat.n_gates);
	println!("word_vm32 reg_wlog_len={} ram_wlog_len={}", run.reg_wlog.len(), run.ram_wlog.len());
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::vm32::circuit::*;
	use crate::vm32::interp::*;
	use crate::vm32::proof::*;
	#[test]
	fn word_vm32_native() {
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let t = run_program(&init, &[], &[], fetch_word);
		println!("cycles={} final x1={:08x} x2={:08x} x3={:08x} x4={:08x} x5={:08x} x6={:08x} x7={:08x} x8={:08x} x9={:08x} x10={:08x}",
			t.cycles.len(), t.final_regs[1], t.final_regs[2], t.final_regs[3], t.final_regs[4], t.final_regs[5], t.final_regs[6], t.final_regs[7], t.final_regs[8], t.final_regs[9], t.final_regs[10]);
	}
	#[test]
	fn word_vm32_prove() {
		// honest + 5 soundness (prints to stdout)
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
		println!("honest: cycles={} c_ok={} l_ok={} n_gates={} n_zero={} n_and={} n_imul={} n_bmul={}",
			run.trace.cycles.len(), run.c_ok, run.l_ok, run.stat.n_gates, run.stat.n_zero_constraints, run.stat.n_and_constraints, run.stat.n_imul_constraints, run.stat.n_bmul_constraints);
		assert!(run.c_ok && run.l_ok, "honest proof must pass");
			run_word_vm32();
		}
			#[test]
			fn word_vm32_rework() {
				// R2/R7 verification of the F1 fix is covered by word_vm32_prove (honest now passes on
				// negative sra/srai/srli/sub cases that failed before the is_sra/is_sub fix).
				let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
				// instruction coverage table (R6): distinct (opcode,funct3,funct7) across torture image
				let mut cover = std::collections::BTreeSet::new();
				for mm in [0x00u64,0x04,0x08,0x0c,0x10,0x14,0x18,0x1c,0x20,0x24,0x28,0x2c,0x30,0x34,
					0x38,0x3c,0x40,0x44,0x48,0x4c,0x50,0x54,0x58,0x5c,0x60,0x64,0x68,0x6c,0x70,
					0x74,0x78,0x7c,0x80,0x84,0x88,0x8c,0x90,0x94,0x98,0x9c,0xa0,0xa4,0xa8,0xac,
					0xb0,0xb4,0xb8,0xbc,0xc0,0xc4] {
					let w = fetch_word(mm);
					cover.insert((w & 0x7f, (w >> 12) & 0x7, (w >> 20) & 0x1f, (w >> 25) & 0x7f, (w>>31)&1));
				}
				// R3 (a): stale-RAM-read layered reject — circuit self-consistent, but logup RAM claim fails
				let lr_a = run_machine_full([0u32; NREG], &init, &[], &[(21, 0x11111111u32)], fetch_word);
				println!("[R3a stale-RAM-read] c_ok={} l_ok={} (want c_ok=true, l_ok=false)", lr_a.c_ok, lr_a.l_ok);
				assert!(lr_a.c_ok && !lr_a.l_ok, "R3a layered reject failed");
				// R3 (b): layered reject, 2nd example on the bubblesort program (RAM logup* layer).
				// A load-override injects 0x22222222 into the first mem[j] read; the native trace
				// stays self-consistent (circuit passes), while the RAM logup* row claim rejects.
				// (Direct witness tampering can never yield c_ok=true: final_regs/fin_ver asserts pin
				// the value/version chains, so any changed value must be a *native* truth — which is
				// exactly what load_overrides provides. Note: after R5, word overrides became a legal
				// program image that also lands in the fetch table, so the v2-style fetch-layer
				// mismatch (override not in table) no longer exists; bad fetch is covered by
				// R4/soundness-5 at the constraint layer.)
				let mut ib = [0u32; NRAM];
				let data_b = [0x80000000u32, 5, 5, 0xffffffff, 3, 3, 0x80000000, 1];
				for (i, &v) in data_b.iter().enumerate() { ib[i] = v; }
				let wb = bubble_words();
				let ntb = run_program(&ib, &wb, &[], fetch_word);
				let mut first_load_cyc = 0;
				for (t, c) in ntb.cycles.iter().enumerate() { if c.load.is_some() { first_load_cyc = t; break; } }
				let lr_b = run_machine_full([0u32; NREG], &ib, &wb, &[(first_load_cyc, 0x22222222u32)], fetch_word);
				println!("[R3b bubble load-override] c_ok={} l_ok={} (want c_ok=true, l_ok=false)", lr_b.c_ok, lr_b.l_ok);
				assert!(lr_b.c_ok && !lr_b.l_ok, "R3b layered reject failed");
				// R4: decode tamper — change add x6 to sub x6 (same table row semantics swap), keep add result;
				// the circuit must reject because decode now computes sub != claimed wr_val.
				let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
				let mut x = run.inout_words.clone();
				let mut t6 = false;
				for t in 0..run.t_len {
					let inst = run.inout_words[io_inst(run.t_len, t)].0;
					if inst & 0x7f == OP_OP && (inst >> 12) & 0x7 == 0x0 && (inst >> 25) & 0x7f == 0x00 && (inst >> 7) & 0x1f == 6 {
						x[io_inst(run.t_len, t)].0 = enc_r(0x20, (inst >> 20) & 0x1f, (inst >> 15) & 0x1f, 0x0, 6, OP_OP);
						t6 = reverify(&run, &x);
						break;
					}
				}
				println!("[R4 decode-tamper add->sub] rejected={} (want true)", t6);
				assert!(t6, "R4 decode tamper must be rejected");
				println!("[R6] instruction coverage: {} distinct (opcode,funct3,funct7) table rows in torture", cover.len());
				}
				#[test]
				fn word_vm32_bubble() {
				// R5: bubblesort end-to-end. Parameterized program-image mirror (bubble_words ->
				// word_overrides) drives the exact same prove/verify/logup* pipeline as torture.
				let mut init = [0u32; NRAM];
				let data = [0x80000000u32, 5, 5, 0xffffffff, 3, 3, 0x80000000, 1]; // dup + MSB-set coverage
				for (i, &v) in data.iter().enumerate() { init[i] = v; }
				let mut exp: Vec<u32> = data.to_vec();
				exp.sort(); // independent public expectation (unsigned ascending), not derived from trace
				let words = bubble_words();
				// (1) native run + memory 对拍
				let nt = run_program(&init, &words, &[], fetch_word);
				let got: Vec<u32> = nt.final_mem[0..8].to_vec();
				println!("[R5 native] cycles={} match_exp={} got={:08x?}", nt.cycles.len(), got == exp, got);
				assert_eq!(got, exp, "native bubblesort must produce the sorted array");
				// (2) prove + verify + logup* honest (full gate circuit over the bubble trace)
				let run = run_machine_full([0u32; NREG], &init, &words, &[], fetch_word);
				println!("[R5 prove] cycles={} c_ok={} l_ok={} n_gates={} n_and={} n_bmul={}",
				run.trace.cycles.len(), run.c_ok, run.l_ok, run.stat.n_gates,
				run.stat.n_and_constraints, run.stat.n_bmul_constraints);
				assert!(run.c_ok && run.l_ok, "bubblesort honest proof must pass");
				// (3) output-triplet cross-check: ram_wlog top row (addr, final_ramver) == expectation
				for a in 0..8 {
				let v = run.trace.final_ramver[a];
				let top = run.ram_wlog[a * VER_MAX + v];
				assert_eq!(top as u32, exp[a], "ram_wlog top row addr {a} (ver {v})");
				}
				println!("[R5 output] ram_wlog top-row checks 8/8 match expectation");
				}
					#[test]
	fn soundness_tamper_alu_wr() {
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
		let mut x = run.inout_words.clone();
		let mut hit = false;
		for t in 0..run.t_len {
			let inst = run.inout_words[io_inst(run.t_len, t)].0;
			if inst & 0x7f == OP_OP && (inst >> 12) & 0x7 == 0x0 && ((inst >> 7) & 0x1f) == 6 && run.inout_words[io_wr_iswrite(run.t_len, t)].0 != 0 {
				x[io_wr_val(run.t_len, t)].0 = run.inout_words[io_wr_val(run.t_len, t)].0.wrapping_add(1);
				hit = reverify(&run, &x); break;
			}
		}
		assert!(hit, "soundness 1 (ALU wr-val tamper) must be rejected");
	}
	#[test]
	fn soundness_tamper_wr_ver() {
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
		let mut x = run.inout_words.clone();
		let mut hit = false;
		for t in 0..run.t_len {
			if run.inout_words[io_wr_iswrite(run.t_len, t)].0 != 0 {
				x[io_wr_ver(run.t_len, t)].0 = run.inout_words[io_wr_ver(run.t_len, t)].0.wrapping_add(1);
				hit = reverify(&run, &x); break;
			}
		}
		assert!(hit, "soundness 2 (wr-ver tamper) must be rejected");
	}
	#[test]
	fn soundness_tamper_ld_val() {
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
		let mut x = run.inout_words.clone();
		let mut hit = false;
		for t in 0..run.t_len {
			if run.inout_words[io_is_load(run.t_len, t)].0 != 0 {
				x[io_ld_val(run.t_len, t)].0 = run.inout_words[io_ld_val(run.t_len, t)].0.wrapping_add(1);
				hit = reverify(&run, &x); break;
			}
		}
		assert!(hit, "soundness 3 (ld-val tamper) must be rejected");
	}
	#[test]
	fn soundness_tamper_x0_write() {
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
		let mut x = run.inout_words.clone();
		let mut hit = false;
		for t in 0..run.t_len {
			if run.inout_words[io_wr_reg(run.t_len, t)].0 == 0 && run.inout_words[io_wr_iswrite(run.t_len, t)].0 != 0 {
				x[io_wr_val(run.t_len, t)].0 = 0xdeadbeef;
				hit = reverify(&run, &x); break;
			}
		}
		assert!(hit, "soundness 4 (x0 hard-zero tamper) must be rejected");
	}
	#[test]
	fn soundness_tamper_fetch() {
		let mut init = [0u32; NRAM]; init[24] = 0xdeadbeef;
		let run = run_machine_full([0u32; NREG], &init, &[], &[], fetch_word);
		let mut x = run.inout_words.clone();
		let mut hit = false;
		for t in 0..run.t_len {
			if run.inout_words[io_wr_iswrite(run.t_len, t)].0 != 0 {
				x[io_inst(run.t_len, t)].0 = fetch_word(run.inout_words[io_pc(run.t_len, t)].0) ^ 1;
				hit = reverify(&run, &x); break;
			}
		}
		assert!(hit, "soundness 5 (fetch tamper) must be rejected");
	}

}

#[cfg(test)]
mod m11_tests {
	use super::*;
	use crate::vm32::isa::*;
	use crate::vm32::proof::run_machine_full;

	/// divu 7/2 微程序镜像（word_overrides 机制）。
	fn divu_prog() -> Vec<(u64, u64)> {
		let mut p: Vec<(u64, u64)> = Vec::new();
		let mut at = 0x00u64;
		p.push((at, lhs_lui(1, 0)));
		at += 4;
		p.push((at, addi(1, 1, 7)));
		at += 4;
		p.push((at, addi(2, 0, 2)));
		at += 4;
		p.push((at, divu(3, 1, 2)));
		at += 4;
		p.push((at, jal(0, 0xc4 - at)));
		p
	}
	fn fetch_halt(_: u64) -> u64 { 0x00000073 }

	/// M11 F1 PoC：advice 商 +1（4 而非 3）。
	/// 修复前：断言恒真 → 全绿（漏洞实证）；
	/// 修复后：verify 层拒绝（m_assert populate 失败 → c_ok=false）。
	#[test]
	fn m11_f1_div_mq_tamper_rejected() {
		let prog = divu_prog();
		// 诚实基线
		let honest = run_machine_full([0u32; NREG], &[0u32; NRAM], &prog, &[], fetch_halt);
		assert!(honest.c_ok && honest.l_ok, "诚实 divu 7/2 必须通过");
		assert_eq!(honest.trace.final_regs[3], 3, "7/2 商 = 3");
		// PoC/用例：验证端篡改公开 inout 的 advice 商（+1）
		// 修复前：m_assert 恒真 → reverify 双绿（漏洞实证）；
		// 修复后：verify 层拒绝（c_ok=false，无 panic）。
		let mut bad_inout = honest.inout_words.clone();
		let mq_base = 20 * honest.trace.cycles.len() + 2 * NREG + NRAM; // 布局：20T+init+final+fin_ver 之后
		bad_inout[mq_base + 3].0 = (bad_inout[mq_base + 3].0 + 1) & 0xffffffff;
		let (c_ok, l_ok) = crate::vm32::proof::reverify2(&honest, &bad_inout);
		assert!(!c_ok, "M11 F1：篡改 advice 商必须被 verify 层拒绝（修复前全绿=漏洞实证）");
	}
}

#[cfg(test)]
mod m8b_tests {
	use super::*;
	use crate::vm32::interp::run_program;

	/// M8-B T2 端到端：mul/divu/rem/lb/lbu/lh/lhu/sb/sh 微程序，native 对拍 + prove→verify。
	#[test]
	fn word_vm32_m8b_isa_prove() {
		let mut init = [0u32; NRAM];
		init[4] = 0xa5c3_1234; // 字节地址 16..20
		// 程序（地址=词索引语义的 lw/sw 与字节地址语义的 lb/lh 共存，见 interp 注释）：
		//  x1=7, x2=3 → x3=mul(x1,x2)=21；x4=divu(x1,x2)=2；x5=remu(x1,x2)=1
		//  x6=div(-7,2)=-3；x7=rem(-7,2)=-1
		//  x8=lb 字节地址 19（字 4 字节 3 = 0xa5，负）→ 0xffffffa5
		//  x9=lbu 字节地址 16 → 0x34；x10=lhu 字节地址 18 → 0xa5c3 零扩展
		//  sb 字节地址 17 ← 0x5e（写 mem[4] 字节 1）→ lw 字 4 读回 0xa5c35e34
		let mut prog: Vec<(u64, u64)> = Vec::new();
		let mut at = 0x00u64;
		let mut push = |inst: u64, prog: &mut Vec<(u64, u64)>, at: &mut u64| {
			prog.push((*at, inst));
			*at += 4;
		};
		push(lhs_lui(1, 0), &mut prog, &mut at);
		push(addi(1, 1, 7), &mut prog, &mut at);
		push(addi(2, 0, 3), &mut prog, &mut at);
		push(mul(3, 1, 2), &mut prog, &mut at);
		push(divu(4, 1, 2), &mut prog, &mut at);
		push(remu(5, 1, 2), &mut prog, &mut at);
		push(addi(8, 0, 0xff9), &mut prog, &mut at); // x8 = -7（imm 符号扩展）
		push(addi(9, 0, 2), &mut prog, &mut at); // x9 = 2
		push(div(6, 8, 9), &mut prog, &mut at);
		push(rem(7, 8, 9), &mut prog, &mut at);
		push(lhs_lui(10, 0), &mut prog, &mut at);
		push(addi(10, 10, 19), &mut prog, &mut at); // 字节地址 19
		push(lb(11, 10, 0), &mut prog, &mut at);
		push(addi(10, 10, 0xfffd), &mut prog, &mut at); // 回到 16
		push(lbu(12, 10, 0), &mut prog, &mut at);
		push(addi(10, 10, 2), &mut prog, &mut at); // 18
		push(lhu(13, 10, 0), &mut prog, &mut at);
		push(addi(14, 0, 0x5e), &mut prog, &mut at);
		push(addi(15, 10, 0xffff), &mut prog, &mut at); // x15 = 18-1 = 字节地址 17（字 4 字节 1）
		push(sb(14, 15, 0), &mut prog, &mut at); // M9 T2：sb 双事件（读旧字+写新字）
		push(addi(15, 15, 1), &mut prog, &mut at); // x15 = 18（字 4 半字 1，对齐）
		push(sh(14, 15, 0), &mut prog, &mut at); // sh 半字 1 ← 0x005e
		push(lhs_lw(15, 0, 4), &mut prog, &mut at); // lw 字索引 4
		push(jal(0, 0xc4 - at), &mut prog, &mut at);
		fn fetch_halt(_: u64) -> u64 { 0x00000073 }
		// native 对拍（程序镜像经 word_overrides 传入，M5 机制）
		let tr = run_program(&init, &prog, &[], fetch_halt);
		let fr = &tr.final_regs;
		assert_eq!(fr[3], 21, "mul");
		assert_eq!(fr[4], 2, "divu");
		assert_eq!(fr[5], 1, "remu");
		assert_eq!(fr[6], (-3i32) as u32, "div");
		assert_eq!(fr[7], (-1i32) as u32, "rem");
		assert_eq!(fr[11], 0xffff_ffa5, "lb");
		assert_eq!(fr[12], 0x34, "lbu");
		assert_eq!(fr[13], 0xa5c3, "lhu");
				// sb 写字节 1（0x5e）、sh 写半字 1（0x005e）：mem[4] = 0x005e5e34
		assert_eq!(fr[15], 0x005e_5e34, "sb/sh 双事件合并后 lw 读回");
		// 端到端 prove→verify
		let run = run_machine_full([0u32; NREG], &init, &prog, &[], fetch_halt);
		println!("m8b isa e2e: cycles={} c_ok={} l_ok={} gates={} imul={} and={}",
			run.trace.cycles.len(), run.c_ok, run.l_ok, run.stat.n_gates, run.stat.n_imul_constraints, run.stat.n_and_constraints);
		assert!(run.c_ok && run.l_ok, "M8-B ISA 微程序端到端 prove→verify 必须通过");
	}
}
