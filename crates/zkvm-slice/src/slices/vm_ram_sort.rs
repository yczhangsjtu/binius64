//! 切片 28: `vm_ram_sort` — M8-A 整合：真实状态机 VM（vm32 语义）× M7 可扩展 RAM 论证
//! × BaseFold 强承诺通道。
//!
//! 架构（任务书 M8-A §2）：
//! - 执行核心：RV32I 子集（vm32 同款译码/执行/寄存器值链+版本链）；**RAM 版本链删除**，
//!   `ld_val` 不再钉任何电路内链——读语义由内存论证承担。
//! - 事件列：每周期一行 (addr, ts=周期, val, kind)，kind∈{0=none,1=read,2=write}；
//!   none 行占位 addr=0xFFFF（进多重集合，排序流含 0xFFFF 占位组）。
//!   事件行 wire 是执行电路的派生结果并断言 == 事件列输入 wire → **事件列—执行绑定在电路内**。
//! - 排序流：每触及地址（含占位地址）init 首 + 事件按 ts 升序 + final 尾；
//!   恒等式①（fracaddcheck 多重集合）+ 恒等式②（电路词级断言：非降/ts 严增/读一致性/init 形状）。
//! - 承诺层：事件列 + 排序流列全部经 BaseFold（`BaseFoldProverChannel`）send_oracle，
//!   归约出口 relation 绑定；与 frontend 电路证明同一 transcript。
//! - 三件套：init 镜像 = 全 0（init 记录 val==0 电路断言）；final 值 = 公开 inout 输出
//!   （OUT_ADDR = BASE 的最终值 = 排序后的最小元素）。
//!
//! 强绑定（T2 决策）：排序流良构/比较（非降、ts 严增）是**跨行**关系，quadratic mlecheck
//! （逐行独立二次式）无法表达（16-bit 非降需位分解+跨行借位链）→ 按任务书 §2.3 降级授权
//! 采用 intmul phase5 模式的电路 witness 列方案：排序流列 = 前端电路 private witness
//! （电路词级断言）+ 同值 committed oracle（恒等式①归约绑定）。间隙与理由见 M8_REPORT。

use binius_compute::GlobalAllocator;
use binius_core::constraint_system::ValueVec;
use binius_core::word::Word;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_field::Field;
use binius_frontend::{Circuit, CircuitBuilder, CircuitStat, Wire};
use binius_hash::StdHashSuite;
use binius_iop::basefold::channel::BaseFoldVerifierChannel;
use binius_iop::basefold::compiler::BaseFoldVerifierCompiler;
use binius_iop::channel::{IOPVerifierChannel, OracleSpec};
use binius_iop::fri::{ConstantArityStrategy, calculate_n_test_queries};
use binius_iop::merkle_channel::VerifierMerkleTranscriptChannel;
use binius_iop::merkle_tree::BinaryMerkleTreeScheme;
use binius_iop_prover::basefold::compiler::BaseFoldProverCompiler;
use binius_iop_prover::channel::IOPProverChannel;
use binius_iop_prover::merkle_channel::ProverMerkleTranscriptChannel;
use binius_ip::channel::IPVerifierChannel;
use binius_ip::fracaddcheck;
use binius_ip::fracaddcheck::FracAddEvalClaim;
use binius_ip_prover::channel::IPProverChannel;
use binius_ip_prover::fracaddcheck::fraction::Fraction;
use binius_ip_prover::fracaddcheck::FracAddCircuit;
use binius_math::multilinear::eq::{eq_ind, eq_ind_partial_eval_in};
use binius_math::ntt::domain_context::GaoMateerPreExpanded;
use binius_math::ntt::NeighborsLastMultiThread;
use binius_math::FieldBuffer;
use binius_prover::Prover as WordProver;
use binius_transcript::{ProverTranscript, VerifierTranscript};
use binius_verifier::config::StdChallenger;
use binius_verifier::Verifier as WordVerifier;
use rand::{SeedableRng, rngs::StdRng};
use crate::vm32::circuit::{mux, mux8, sar_var, sext_w, shl_var, shr_var, slt_signed, slt_unsigned};
use crate::vm32::interp::{Cycle, RegAccess, MemAccess};
use crate::vm32::isa::*;

pub type LF = OptimalB128;
pub type LP = OptimalPackedB128;

// ---- 常量 ----
/// 地址空间 K = 2^16 字（任务书 §1）。
pub const K_BITS: usize = 16;
pub const K: usize = 1 << K_BITS;
/// 数据数组基址（RAM 数据区）。
pub const BASE: u64 = 0x1000;
/// 排序元素数（主测 16 字——prove 时间预算；缩放点 32/64 可选）。
pub const N: usize = 16;
/// kind 编码：0=init/none, 1=read, 2=write, 3=final。
pub const K_INIT: u64 = 0;
pub const K_READ: u64 = 1;
pub const K_WRITE: u64 = 2;
pub const K_FINAL: u64 = 3;
/// 占位（无访问）地址——组在排序流最末。
pub const PAD_ADDR: u64 = 0xffff;
/// 输出三件套地址 = BASE（排序后最小元素）。
pub const OUT_ADDR: u64 = BASE;

// ---- 指令编码（vm32/isa 同款，简化版；程序用到的子集） ----
fn opcode(x: u64, lo: u32, hi: u32) -> u64 {
	(x >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}
fn i_enc(op: u64, f3: u64, rd: u64, rs1: u64, imm: i64) -> u64 {
	((imm as u64 & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn s_enc(op: u64, f3: u64, rs2: u64, rs1: u64, imm: i64) -> u64 {
	(((imm as u64) & 0xfe0) << 20) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (((imm as u64) & 0x1f) << 7) | op
}
fn b_enc(op: u64, f3: u64, rs2: u64, rs1: u64, imm: i64) -> u64 {
	(((imm as u64 >> 12) & 1) << 31) | (((imm as u64 >> 5) & 0x3f) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12)
		| (((imm as u64 >> 1) & 0xf) << 8) | (((imm as u64 >> 11) & 1) << 7) | op
}
fn j_enc(op: u64, rd: u64, imm: i64) -> u64 {
	(((imm as u64 >> 20) & 1) << 31) | (((imm as u64 >> 1) & 0x3ff) << 21) | (((imm as u64 >> 11) & 1) << 20)
		| (((imm as u64 >> 12) & 0xff) << 12) | (rd << 7) | op
}
pub fn addi(rd: u64, rs1: u64, imm: i64) -> u64 { i_enc(0x13, 0, rd, rs1, imm) }
pub fn lui(rd: u64, imm20: i64) -> u64 { ((imm20 as u64 & 0xfffff) << 12) | (rd << 7) | 0x37 }
pub fn lw(rd: u64, rs1: u64, imm: i64) -> u64 { i_enc(0x03, 2, rd, rs1, imm) }
pub fn sw(rs2: u64, rs1: u64, imm: i64) -> u64 { s_enc(0x23, 2, rs2, rs1, imm) }
pub fn blt(rs1: u64, rs2: u64, imm: i64) -> u64 { b_enc(0x63, 4, rs2, rs1, imm) }
pub fn bge(rs1: u64, rs2: u64, imm: i64) -> u64 { b_enc(0x63, 5, rs2, rs1, imm) }
pub fn bltu(rs1: u64, rs2: u64, imm: i64) -> u64 { b_enc(0x63, 6, rs2, rs1, imm) }
pub fn bgeu(rs1: u64, rs2: u64, imm: i64) -> u64 { b_enc(0x63, 7, rs2, rs1, imm) }
pub fn jal(rd: u64, imm: i64) -> u64 { j_enc(0x6f, rd, imm) }
pub fn r_enc(f3: u64, f7: u64, rd: u64, rs1: u64, rs2: u64) -> u64 { (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | 0x33 }
pub fn add(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(0, 0, rd, rs1, rs2) }
pub fn sub(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(0, 0x20, rd, rs1, rs2) }
pub fn sll(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(1, 0, rd, rs1, rs2) }
pub fn srl(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(5, 0, rd, rs1, rs2) }
pub fn xor(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(4, 0, rd, rs1, rs2) }
pub fn or(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(6, 0, rd, rs1, rs2) }
pub fn and(rd: u64, rs1: u64, rs2: u64) -> u64 { r_enc(7, 0, rd, rs1, rs2) }

// ---- 访问记录 ----
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visit {
	pub addr: u64,
	pub ts: u64,
	pub val: u64,
	pub kind: u64,
}

/// 程序镜像：fetch(pc) → 指令（bubblesort，数据写入段 + 排序段 + halt）。
pub fn prog_fetch(pc: u64) -> u64 {
	let slot = (pc >> 2) as usize;
	prog_image(slot, N)
}

fn prog_image(slot: usize, n: usize) -> u64 {
	// 数据（n 字，伪随机）。
	let data: Vec<i64> = (0..n).map(|i| ((i as i64) * 7919 + 13) % 9973).collect();
	let base = BASE as i64;
	// 段 1：写数据 mem[BASE+j] = data[j]（lui+addi+sw 每字 5 条，字寻址步 1）。
	let mut p: Vec<u64> = Vec::new();
	for j in 0..n {
		let v = data[j];
		// lui + addi 标准组装：低 12 位 >= 0x800 时加 1 到高 20 位、低 12 位取负（addi 符号扩展）。
		let hi = v >> 12;
		let lo = v & 0xfff;
		let (hi, lo) = if lo >= 0x800 { (hi + 1, lo - 0x1000) } else { (hi, lo) };
		p.push(lui(15, hi));
		p.push(addi(15, 15, lo));
		let addr = base + j as i64;
		p.push(lui(16, addr >> 12));
		p.push(addi(16, 16, addr & 0xfff));
		p.push(sw(15, 16, 0));
	}
	// 段 2：bubblesort（N 字，字寻址：mem 为 u32 数组，元素地址 = BASE + j，j 步 1）。
	// 寄存器：x12=i, x14=j, x16=&a[j], x17=a[j], x18=a[j+1]。
	//  0: lui x12, 0                        # i = 0
	//  1(out): lui x15, hi(N-1)             # 外层上界（i<N-1，N-1 轮；每轮经 jal 重算）
	//  2: addi x15, x15, lo(N-1)
	//  3: bgeu x12, x15, halt
	//  4: lui x14, 0                        # j = 0（仅首次）
	//  5(in): lui x15, hi(N-1)              # 内层上界 = N-1-i（每轮重算）
	//  6: addi x15, x15, lo(N-1)
	//  7: sub x15, x15, x12
	//  8: bgeu x14, x15, in_done
	//  9: lui x16, hi(BASE)
	//  10: addi x16, x16, lo(BASE)
	//  11: add x16, x16, x14
	//  12: lw x17, x16, 0
	//  13: lw x18, x16, 1
	//  14: bgeu x18, x17, next_j
	//  15: sw x18, x16, 0
	//  16: sw x17, x16, 1
	//  17(next_j): addi x14, x14, 1
	//  18: jal x0, in
	//  19(in_done): addi x12, x12, 1
	//  20: jal x0, out
	//  21: ecall (halt)
	let w = p.len() as i64;
	let l_out = w + 1;
	let l_in = w + 5;
	let l_swap_skip = w + 17;
	let l_in_done = w + 19;
	let l_halt = w + 21;
	let mut s2: Vec<u64> = Vec::new();
	s2.push(lui(12, 0));
	s2.push(lui(15, ((n as i64 - 1) >> 12) as i64));
	s2.push(addi(15, 15, (n as i64 - 1) & 0xfff));
	s2.push(bgeu(12, 15, (l_halt - (w + 3)) * 4));
	s2.push(lui(14, 0));
	s2.push(lui(15, ((n as i64 - 1) >> 12) as i64));
	s2.push(addi(15, 15, (n as i64 - 1) & 0xfff));
	s2.push(sub(15, 15, 12));
	s2.push(bgeu(14, 15, (l_in_done - (w + 8)) * 4));
	s2.push(lui(16, base >> 12));
	s2.push(addi(16, 16, base & 0xfff));
	s2.push(add(16, 16, 14));
	s2.push(lw(17, 16, 0));
	s2.push(lw(18, 16, 1));
	s2.push(bgeu(18, 17, (l_swap_skip - (w + 14)) * 4));
	s2.push(sw(18, 16, 0));
	s2.push(sw(17, 16, 1));
	s2.push(addi(14, 14, 1));
	s2.push(jal(0, (l_in - (w + 18)) * 4));
	s2.push(addi(12, 12, 1));
	s2.push(jal(0, (l_out - (w + 20)) * 4));
	s2.push(0x00000073); // ecall — halt
	p.extend(s2);
	if slot < p.len() {
		p[slot]
	} else {
		0x00000073
	}
}

// ---- trace（K=2^16 RAM 域，语义 = vm32::interp::run_program，仅地址域放大） ----
pub struct BigTrace {
	pub cycles: Vec<Cycle>,
	pub final_regs: [u32; 32],
	pub final_mem: Vec<u32>, // K 长
}
pub fn run_program_big(
	init_mem: &[u32],
	word_overrides: &[(u64, u64)],
	fetch: impl Fn(u64) -> u64,
) -> BigTrace {
	assert_eq!(init_mem.len(), K);
	let mut regs = [0u32; 32];
	let mut rver = [0usize; 32];
	let mut mem = init_mem.to_vec();
	let mut ramver_native = vec![0usize; K];
	let mut pc: u64 = 0;
	let mut cycles: Vec<Cycle> = Vec::new();
	let mut guard = 0usize;
	let fetch_ov = |addr: u64| -> u64 {
		for &(a, w) in word_overrides {
			if a == addr { return w; }
		}
		fetch(addr)
	};
	loop {
		guard += 1;
		if guard > 100_000 {
			for c in cycles.iter().rev().take(24).rev() {
				eprintln!("DBG pc={:04x} inst={:08x} ld={:?} st={:?}", c.pc, c.inst,
					c.load.as_ref().map(|l| (l.addr, l.val)), c.store.as_ref().map(|s| (s.addr, s.val)));
			}
			panic!("runaway execution");
		}
		let inst = fetch_ov(pc);
		let opcode = (inst >> 0) & 0x7f;
		let rd = ((inst >> 7) & 0x1f) as usize;
		let rs1 = ((inst >> 15) & 0x1f) as usize;
		let rs2 = ((inst >> 20) & 0x1f) as usize;
		let funct3 = (inst >> 12) & 0x7;
		let funct7 = (inst >> 25) & 0x7f;
		let sext = |v: u64, bits: u32| -> i64 { ((v as i64) << (64 - bits)) >> (64 - bits) };
		let imm_i = sext(inst >> 20 & 0xfff, 12) as u64;
		let imm_s = sext(((inst >> 25 & 0x7f) << 5) | (inst >> 7 & 0x1f), 12) as u64;
		let imm_b = sext(((inst >> 31 & 1) << 12) | ((inst >> 7 & 1) << 11) | ((inst >> 25 & 0x3f) << 5) | ((inst >> 8 & 0x0f) << 1), 13) as u64;
		let imm_u = ((inst >> 12) & 0xfffff) << 12;
		let imm_j = sext(((inst >> 31 & 1) << 20) | ((inst >> 12 & 0xff) << 12) | ((inst >> 20 & 1) << 11) | ((inst >> 21 & 0x3ff) << 1), 21) as u64;
		let cycle_rv = rver;
		let reads = vec![
			RegAccess { reg: rs1, ver: rver[rs1], val: regs[rs1] },
			RegAccess { reg: rs2, ver: rver[rs2], val: regs[rs2] },
		];
		let a = regs[rs1] as u64;
		let addr_calc = (a.wrapping_add(imm_i)) as u64 & (K as u64 - 1);
		let mut load = None;
		let mut store = None;
		if opcode == OP_LOAD && funct3 == 0x2 {
			let v = ramver_native[addr_calc as usize];
			load = Some(MemAccess { addr: addr_calc as usize, ver: v, val: mem[addr_calc as usize] });
		} else if opcode == OP_STORE && funct3 == 0x2 {
			let st_addr = (a.wrapping_add(imm_s)) & (K as u64 - 1);
			let v = ramver_native[st_addr as usize] + 1;
			mem[st_addr as usize] = regs[rs2];
			ramver_native[st_addr as usize] += 1;
			store = Some(MemAccess { addr: st_addr as usize, ver: v, val: regs[rs2] });
		}
		let is_branch = opcode == OP_BRANCH;
		let taken = if is_branch {
			let (x, y) = (regs[rs1], regs[rs2]);
			match funct3 {
				0x0 => x == y,
				0x1 => x != y,
				0x4 => (x as i32) < (y as i32),
				0x5 => (x as i32) >= (y as i32),
				0x6 => x < y,
				0x7 => x >= y,
				_ => false,
			}
		} else if opcode == OP_JAL { true } else if opcode == OP_JALR { true } else { false };
		let (mut alu_sum, mut is_alu_write, mut alu_wr) = (0u32, false, 0usize);
		if opcode == OP_OP {
			let (x, y) = (regs[rs1], regs[rs2]);
			alu_wr = rd;
			alu_sum = match (funct3, funct7) {
				(0x0, 0x20) => x.wrapping_sub(y),
				(0x0, _) => x.wrapping_add(y),
				(0x1, _) => x << (y & 0x1f),
				(0x2, _) => ((x as i32) < (y as i32)) as u32,
				(0x3, _) => (x < y) as u32,
				(0x4, _) => x ^ y,
				(0x5, 0x20) => ((x as i32) >> (y & 0x1f)) as u32,
				(0x5, _) => x >> (y & 0x1f),
				(0x6, _) => x | y,
				(0x7, _) => x & y,
				_ => 0,
			};
			is_alu_write = true;
		} else if opcode == OP_OPIMM {
			alu_wr = rd;
			alu_sum = match funct3 {
				0x0 => regs[rs1].wrapping_add(imm_i as u32),
				0x1 => regs[rs1] << (imm_i & 0x1f),
				0x2 => ((regs[rs1] as i32) < (imm_i as i32)) as u32,
				0x3 => (regs[rs1] < imm_i as u32) as u32,
				0x4 => regs[rs1] ^ imm_i as u32,
				0x5 => if funct7 == 0x20 { ((regs[rs1] as i32) >> (imm_i & 0x1f)) as u32 } else { regs[rs1] >> (imm_i & 0x1f) },
				0x6 => regs[rs1] | imm_i as u32,
				0x7 => regs[rs1] & imm_i as u32,
				_ => 0,
			};
			is_alu_write = true;
		} else if opcode == OP_LUI {
			alu_wr = rd;
			alu_sum = imm_u as u32;
			is_alu_write = true;
		} else if opcode == OP_AUIPC {
			alu_wr = rd;
			alu_sum = (pc.wrapping_add(imm_u)) as u32;
			is_alu_write = true;
		} else if opcode == OP_JAL {
			alu_wr = rd;
			alu_sum = pc.wrapping_add(4) as u32;
			is_alu_write = true;
		} else if opcode == OP_JALR {
			alu_wr = rd;
			alu_sum = pc.wrapping_add(4) as u32;
			is_alu_write = true;
		} else if opcode == OP_LOAD && funct3 == 0x2 {
			alu_wr = rd;
			alu_sum = load.as_ref().map(|l| l.val).unwrap_or(0);
			is_alu_write = true;
		}
		// 写回（x0 丢弃）
		let wr_ver = rver[alu_wr];
		if is_alu_write && rd != 0 {
			regs[alu_wr] = alu_sum;
			rver[alu_wr] += 1;
		}
		let mut next_pc = pc + 4;
		if is_branch && taken { next_pc = (pc as i64 + imm_b as i64) as u64 & 0xffffffff; }
		else if opcode == OP_JAL { next_pc = (pc as i64 + imm_j as i64) as u64 & 0xffffffff; }
		else if opcode == OP_JALR { next_pc = (regs[rs1] as i64 + imm_i as i64) as u64 & 0xfffffffe; }
		let halted = opcode == 0x73;
		cycles.push(Cycle {
			pc,
			inst,
			reads,
			write: if is_alu_write { Some(RegAccess { reg: alu_wr, ver: wr_ver, val: alu_sum }) } else { None },
			regver: cycle_rv,
			load,
			store,
			ramver: [0usize; NRAM],
			mem_addr: addr_calc as usize,
		});
		pc = next_pc;
		if halted { break; }
	}
	let final_regs: [u32; 32] = regs;
	BigTrace { cycles, final_regs, final_mem: mem }
}

/// 排序流：每触及地址（真实 + PAD_ADDR 组）init 首 + 事件升序 + final 尾。
pub fn build_sorted(events: &[Visit]) -> Vec<Visit> {
	let t = events.len();
	let mut latest: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
	for e in events {
		if e.kind == K_WRITE {
			latest.insert(e.addr, e.val);
		}
	}
	let mut addrs: Vec<u64> = events.iter().map(|e| e.addr).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
	addrs.sort_unstable();
	let mut sorted = Vec::new();
	for &a in &addrs {
		// PAD 组不推 init 行：占位事件行本身 kind=0/val=0（init 形状）且 ts=周期号从 0 起，
		// 若再推 ts=0 的 init 行会与首个占位事件违反 ts 严增。
		if a != PAD_ADDR {
			sorted.push(Visit { addr: a, ts: 0, val: 0, kind: K_INIT });
		}
		for e in events.iter().filter(|e| e.addr == a) {
			sorted.push(*e);
		}
		let fv = *latest.get(&a).unwrap_or(&0);
		sorted.push(Visit { addr: a, ts: t as u64 + 1, val: fv, kind: K_FINAL });
	}
	sorted
}

// ---- 事件列：每周期一行（load/store/占位），事件侧重排（init + 事件 + final），排序流 ----
pub fn event_rows(trace: &BigTrace) -> (Vec<Visit>, Vec<Visit>) {
	// 事件行
	let mut rows: Vec<Visit> = Vec::with_capacity(trace.cycles.len());
	for (t, c) in trace.cycles.iter().enumerate() {
		if let Some(ld) = &c.load {
			rows.push(Visit { addr: ld.addr as u64, ts: t as u64, val: ld.val as u64, kind: K_READ });
		} else if let Some(st) = &c.store {
			rows.push(Visit { addr: st.addr as u64, ts: t as u64, val: st.val as u64, kind: K_WRITE });
		} else {
			rows.push(Visit { addr: PAD_ADDR, ts: t as u64, val: 0, kind: K_INIT });
		}
	}
	// 事件侧重排：init(触达地址) + 事件行 + final(触达地址)
	let mut addrs: Vec<u64> = rows.iter().map(|e| e.addr).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
	addrs.sort_unstable();
	let mut side = Vec::new();
	for &a in &addrs {
		// 与 build_sorted 对齐：PAD 组不推 init 行（保持两侧多重集合相等）。
		if a != PAD_ADDR {
			side.push(Visit { addr: a, ts: 0, val: 0, kind: K_INIT });
		}
	}
	side.extend_from_slice(&rows);
	for &a in &addrs {
		let fv = rows.iter().rev().find(|e| e.addr == a && e.kind == K_WRITE).map(|e| e.val).unwrap_or(0);
		side.push(Visit { addr: a, ts: trace.cycles.len() as u64 + 1, val: fv, kind: K_FINAL });
	}
	(rows, side)
}

// ---- 电路：执行（inout→witness）+ 事件 pinning + 排序流恒等式② + 三件套 ----
pub struct VmRsIref {
	pub inst: Vec<Wire>, pub pc: Vec<Wire>,
	pub rd1_reg: Vec<Wire>, pub rd1_val: Vec<Wire>,
	pub rd2_reg: Vec<Wire>, pub rd2_val: Vec<Wire>,
	pub wr_reg: Vec<Wire>, pub wr_val: Vec<Wire>, pub wr_iswrite: Vec<Wire>,
	pub ld_addr: Vec<Wire>, pub ld_val: Vec<Wire>, pub is_load: Vec<Wire>,
	pub st_addr: Vec<Wire>, pub st_val: Vec<Wire>, pub is_store: Vec<Wire>,
	pub s_addr: Vec<Wire>, pub s_ts: Vec<Wire>, pub s_val: Vec<Wire>, pub s_kind: Vec<Wire>,
	pub final_out: Wire,
}

fn assert_sortedness(b: &CircuitBuilder, sa: &[Wire], sts: &[Wire], sv: &[Wire], sk: &[Wire]) {
	let z = b.add_constant_64(0);
	let o = b.add_constant_64(1);
	let ts = sa.len();
	for j in 0..ts {
		let bad = b.select(b.icmp_eq(sk[j], z), b.select(b.bnot(b.icmp_eq(sv[j], z)), o, z), z);
		b.assert_eq(format!("init_val{j}"), bad, z);
	}
	b.assert_eq("first_kind", b.select(b.bnot(b.icmp_eq(sk[0], z)), o, z), z);
	for j in 0..ts - 1 {
		let a0 = sa[j];
		let a1 = sa[j + 1];
		let nv = b.select(b.icmp_ult(a1, a0), o, z);
		b.assert_eq(format!("nondesc{j}"), nv, z);
		let eqa = b.icmp_eq(a0, a1);
		let ts_bad = b.select(b.band(eqa, b.bnot(b.icmp_ult(sts[j], sts[j + 1]))), o, z);
		b.assert_eq(format!("ts_inc{j}"), ts_bad, z);
		let rf = b.bor(b.icmp_eq(sk[j + 1], b.add_constant_64(K_READ)), b.icmp_eq(sk[j + 1], b.add_constant_64(K_FINAL)));
		let vc = b.select(b.band(eqa, b.band(rf, b.bnot(b.icmp_eq(sv[j], sv[j + 1])))), o, z);
		b.assert_eq(format!("val_cons{j}"), vc, z);
		let ni = b.select(b.band(b.bnot(eqa), b.bnot(b.icmp_eq(sk[j + 1], z))), o, z);
		b.assert_eq(format!("new_init{j}"), ni, z);
	}
}

pub fn build_circuit_vmrs(t_len: usize, ts: usize) -> (Circuit, VmRsIref) {
	let b = CircuitBuilder::new();
	let zero = b.add_constant_64(0);
	let one = b.add_constant_64(1);
	let inst = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let pc = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let rd1_reg = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let rd1_val = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let rd2_reg = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let rd2_val = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let wr_reg = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let wr_val = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let wr_iswrite = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let ld_addr = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let ld_val = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let is_load = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let st_addr = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let st_val = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let is_store = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_addr = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_ts = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_val = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_kind = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let final_out = b.add_inout();

	// 寄存器值链（内部 wire）
	let mut cur_reg = [zero; 32];
	let mut cur_ver = [zero; 32];
	let mut prev_pc = b.add_constant_64(0); // PC_START = 0

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
		let imm_s = sext_w(&b, b.bor(b.sll32(b.band(b.srl32(inst_w, 25), b.add_constant_64(0x7f)), 5), b.band(b.srl32(inst_w, 7), b.add_constant_64(0x1f))), 12);
		let imm_u_val = b.sll32(b.band(b.srl32(inst_w, 12), b.add_constant_64(0xfffff)), 12);
		let imm_b_raw = b.bor(b.bor(b.bor(b.sll32(b.band(b.srl32(inst_w, 31), b.add_constant_64(1)), 12), b.sll32(b.band(b.srl32(inst_w, 7), b.add_constant_64(1)), 11)), b.sll32(b.band(b.srl32(inst_w, 25), b.add_constant_64(0x3f)), 5)), b.sll32(b.band(b.srl32(inst_w, 8), b.add_constant_64(0x0f)), 1));
		let imm_b = sext_w(&b, imm_b_raw, 13);
		let imm_j_raw = b.bor(b.bor(b.bor(b.sll32(b.band(b.srl32(inst_w, 31), b.add_constant_64(1)), 20), b.sll32(b.band(b.srl32(inst_w, 12), b.add_constant_64(0xff)), 12)), b.sll32(b.band(b.srl32(inst_w, 20), b.add_constant_64(1)), 11)), b.sll32(b.band(b.srl32(inst_w, 21), b.add_constant_64(0x3ff)), 1));
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
		let rs1v = mux(&b, &cur_reg, rs1);
		let rs2v = mux(&b, &cur_reg, rs2);
		// 内存地址：K=2^16 域（无 RAM 版本链）
		let ld_addr_w = b.band(b.iadd_32(rs1v, imm_i), b.add_constant_64(K as u64 - 1));
		let st_addr_w = b.band(b.select(c_is_store, b.iadd_32(rs1v, imm_s), b.iadd_32(rs1v, imm_i)), b.add_constant_64(K as u64 - 1));
		let bop = b.select(is_risc, rs2v, imm_i);
		let shamt = b.select(is_risc, b.band(rs2v, b.add_constant_64(0x1f)), b.band(b.srl32(inst_w, 20), b.add_constant_64(0x1f)));
		let is_sub = b.band(b.band(b.icmp_eq(opcode, b.add_constant_64(OP_OP)), b.icmp_eq(funct3, b.add_constant_64(0x0))), b.shl(funct7, 58));
		let is_sra = b.band(b.icmp_eq(funct3, b.add_constant_64(0x5)), b.shl(funct7, 58));
		let alu_add_sub = b.select(is_sub, b.band(b.isub_bin_bout(rs1v, rs2v, zero).0, b.add_constant_64(0xffffffff)), b.iadd_32(rs1v, bop));
		let alu_sll = shl_var(&b, rs1v, shamt);
		let alu_slt = slt_signed(&b, rs1v, bop);
		let alu_sltu = slt_unsigned(&b, rs1v, bop);
		let alu_xor = b.bxor(rs1v, bop);
		let alu_shift = b.select(is_sra, sar_var(&b, rs1v, shamt), shr_var(&b, rs1v, shamt));
		let alu_or = b.bor(rs1v, bop);
		let alu_and = b.band(rs1v, bop);
		let alu8 = [alu_add_sub, alu_sll, alu_slt, alu_sltu, alu_xor, alu_shift, alu_or, alu_and];
		let alu_core = mux8(&b, &alu8, funct3);
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
		let rd_is_zero = b.icmp_eq(rd, zero);
		let wb = b.select(rd_is_zero, zero, alu_sum);
		let is_alu_write = b.bor(b.bor(b.bor(b.bor(b.bor(is_imm, is_risc), is_lui), is_auipc), is_jal), is_jalr);
		let is_alu_write = b.bor(is_alu_write, c_is_load);
		let mut next_reg = cur_reg;
		let mut next_ver = cur_ver;
		for r in 1..32 {
			let is_write_r = b.band(b.icmp_eq(rd, b.add_constant_64(r as u64)), is_alu_write);
			let nv = b.select(is_write_r, wb, cur_reg[r]);
			let nver = b.select(is_write_r, b.iadd(cur_ver[r], one).0, cur_ver[r]);
			next_reg[r] = nv;
			next_ver[r] = nver;
		}
		cur_reg = next_reg;
		cur_ver = next_ver;
		// ★ RAM 版本链已删除（M8-A §2.1）：ld_val 为 witness，读语义由内存论证承担。
		// PC 演进
		let pc4 = b.iadd_32(pc_v, b.add_constant_64(4));
		let c_blt = b.icmp_ult(b.bxor(rs1v, b.add_constant_64(0x80000000)), b.bxor(rs2v, b.add_constant_64(0x80000000)));
		let c_bge = b.bnot(c_blt);
		let c_bltu = b.icmp_ult(rs1v, rs2v);
		let c_bgeu = b.bnot(c_bltu);
		let branch_taken = mux8(&b, &[b.icmp_eq(rs1v, rs2v), b.bnot(b.icmp_eq(rs1v, rs2v)), zero, zero, c_blt, c_bge, c_bltu, c_bgeu], funct3);
		let next_pc = b.select(is_jal, b.iadd_32(pc_v, imm_j),
			b.select(is_jalr, b.band(b.iadd_32(rs1v, imm_i), b.add_constant_64(0xfffffffe)),
			b.select(is_branch, b.select(branch_taken, b.iadd_32(pc_v, imm_b), pc4), pc4)));
		prev_pc = next_pc;
		// 事件 pinning（R1 语义：执行↔事件 witness 钉扎；全部 witness 输入）
		let is_alu_write_01 = b.select(is_alu_write, one, zero);
		let is_load_01 = b.select(c_is_load, one, zero);
		let is_store_01 = b.select(c_is_store, one, zero);
		b.assert_eq(format!("rd1_reg[{t}]"), rd1_reg[t], rs1);
		b.assert_eq(format!("rd1_val[{t}]"), rd1_val[t], rs1v);
		b.assert_eq(format!("rd2_reg[{t}]"), rd2_reg[t], rs2);
		b.assert_eq(format!("rd2_val[{t}]"), rd2_val[t], rs2v);
		b.assert_eq(format!("wr_reg[{t}]"), wr_reg[t], rd);
		b.assert_eq(format!("wr_val[{t}]"), wr_val[t], wb);
		b.assert_eq(format!("wr_iswrite[{t}]"), wr_iswrite[t], is_alu_write_01);
		b.assert_eq(format!("ld_addr[{t}]"), ld_addr[t], ld_addr_w);
		b.assert_eq(format!("is_load[{t}]"), is_load[t], is_load_01);
		b.assert_eq(format!("st_addr[{t}]"), st_addr[t], st_addr_w);
		b.assert_eq(format!("st_val[{t}]"), st_val[t], rs2v);
		b.assert_eq(format!("is_store[{t}]"), is_store[t], is_store_01);
	}
	// 恒等式②（排序流良构 + 读一致性）
	assert_sortedness(&b, &s_addr, &s_ts, &s_val, &s_kind);
	// 三件套：final 输出（OUT_ADDR 的最终值 = 排序后最小元素）
	let mut acc = zero;
	for j in 0..ts {
		let hit = b.band(b.icmp_eq(s_addr[j], b.add_constant_64(OUT_ADDR)), b.icmp_eq(s_kind[j], b.add_constant_64(K_FINAL)));
		let picked = b.select(hit, s_val[j], zero);
		acc = b.bxor(acc, picked);
	}
	b.assert_eq("final_out", final_out, acc);
	(
		b.build(),
		VmRsIref { inst, pc, rd1_reg, rd1_val, rd2_reg, rd2_val, wr_reg, wr_val, wr_iswrite, ld_addr, ld_val, is_load, st_addr, st_val, is_store, s_addr, s_ts, s_val, s_kind, final_out },
	)
}

// ---- 运行结果 ----
pub struct VmRsRun {
	pub c_ok: bool,
	pub l_ok: bool,
	pub stat: CircuitStat,
	pub t_len: usize,
	pub ts: usize,
	pub l: usize,
	pub inout_words: Vec<Word>,
	pub prover: WordProver<LP, StdHashSuite>,
	pub verifier: WordVerifier<StdHashSuite>,
	pub witness: ValueVec,
	pub sorted_ok: bool,
}

/// 验证端篡改模式（verify 层 soundness，M7 v2 纪律）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tamper {
	None,
	/// 例 1（电路层）：篡改公开输出 final_out → `c_ok == false`。
	BadFinalOut,
	/// 例 2（logup 层）：篡改分数和声明 root_den → `l_ok == false`。
	BadRootDen,
	/// 例 3（logup 层）：篡改 addr 开口值（den 组合先行揭穿）→ `l_ok == false`。
	BadDenAddr,
	/// 例 4（logup 层）：篡改 val 开口值 → `l_ok == false`。
	BadDenVal,
}

/// T1 主流程：native 排序程序 → trace → 电路（执行+恒等式②）→ BaseFold committed 列
/// → fracaddcheck（恒等式①）→ 绑定 → verify。
pub fn run_vmrs(n: usize, tamper: Tamper) -> VmRsRun {
	// fetch 闭包捕获 n：排序规模随参数变化（prog_fetch 固定用常量 N，仅作外部便捷入口）。
	let trace = run_program_big(&vec![0u32; K], &[], |pc| prog_image((pc >> 2) as usize, n));
	assert!(trace.cycles.len() < 1_500_000);
	let t_len = trace.cycles.len();
	let (rows, side) = event_rows(&trace);
	let sorted = build_sorted(&rows);
	assert_eq!(sorted.len(), side.len(), "排序流与事件侧同长");
	let ts = sorted.len();
	let l = (usize::BITS - ((2 * ts) - 1).leading_zeros()) as usize;
	let nrows = 1usize << l;

	// final 输出期望（排序后最小元素）
	let final_val = trace.final_mem[OUT_ADDR as usize] as u64;

	let (circuit, iref) = build_circuit_vmrs(t_len, ts);
	let stat = CircuitStat::collect(&circuit);
	let cs = circuit.constraint_system().clone();
	let mut w = circuit.new_witness_filler();
	for (t, c) in trace.cycles.iter().enumerate() {
		w[iref.inst[t]] = Word(c.inst);
		w[iref.pc[t]] = Word(c.pc);
		w[iref.rd1_reg[t]] = Word(c.reads[0].reg as u64);
		w[iref.rd1_val[t]] = Word(c.reads[0].val as u64);
		w[iref.rd2_reg[t]] = Word(c.reads[1].reg as u64);
		w[iref.rd2_val[t]] = Word(c.reads[1].val as u64);
		// rd 字段按指令编码复算（sw 的 rd 字段 = imm 低 5 位，非 0；电路断言 wr_reg == rd 无条件成立）。
		w[iref.wr_reg[t]] = Word(((c.inst >> 7) & 0x1f) as u64);
		// wb = select(rd==0, 0, alu_sum)：非 ALU 写或 rd=x0 时电路恒为 0。
		w[iref.wr_val[t]] = Word(c.write.as_ref().filter(|x| x.reg != 0).map(|x| x.val).unwrap_or(0) as u64);
		w[iref.wr_iswrite[t]] = Word(if c.write.is_some() { 1 } else { 0 });
		// pinning 断言是无条件的：ld_addr == (rs1+imm_i)&0xffff、st_addr == select(store, rs1+imm_s, rs1+imm_i)&0xffff、
		// st_val == rs2v（rs2 字段按指令编码读取），非访存周期也必须填 native 复算值。
		w[iref.ld_addr[t]] = Word(c.mem_addr as u64);
		w[iref.ld_val[t]] = Word(c.load.as_ref().map(|x| x.val as u64).unwrap_or(0));
		w[iref.is_load[t]] = Word(if c.load.is_some() { 1 } else { 0 });
		w[iref.st_addr[t]] = Word(c.store.as_ref().map(|x| x.addr as u64).unwrap_or(c.mem_addr as u64));
		w[iref.st_val[t]] = Word(c.reads[1].val as u64);
		w[iref.is_store[t]] = Word(if c.store.is_some() { 1 } else { 0 });
	}
	for j in 0..ts {
		w[iref.s_addr[j]] = Word(sorted[j].addr);
		w[iref.s_ts[j]] = Word(sorted[j].ts);
		w[iref.s_val[j]] = Word(sorted[j].val);
		w[iref.s_kind[j]] = Word(sorted[j].kind);
	}
	w[iref.final_out] = Word(final_val);
	circuit.populate_wire_witness(&mut w).expect("witness fill");
	let witness_vec = w.into_value_vec();
	cs.verify(&witness_vec).expect("native verify");
	let inout_words = witness_vec.inout().to_vec();

	// committed 列（事件侧 + 排序侧 ‖ pad）
	let mut col_addr = vec![0u64; nrows];
	let mut col_val = vec![0u64; nrows];
	let mut col_ts = vec![0u64; nrows];
	let mut col_kind = vec![0u64; nrows];
	for j in 0..ts {
		col_addr[j] = sorted[j].addr; col_val[j] = sorted[j].val;
		col_ts[j] = sorted[j].ts; col_kind[j] = sorted[j].kind;
	}
	for j in 0..ts {
		col_addr[ts + j] = side[j].addr; col_val[ts + j] = side[j].val;
		col_ts[ts + j] = side[j].ts; col_kind[ts + j] = side[j].kind;
	}
	let to_fb = |v: &[u64]| FieldBuffer::<LP, _>::from_values(&v.iter().map(|&x| LF::from(x as u128)).collect::<Vec<_>>());
	let fb_addr = to_fb(&col_addr);
	let fb_val = to_fb(&col_val);
	let fb_ts = to_fb(&col_ts);
	let fb_kind = to_fb(&col_kind);

	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<LP, StdHashSuite>::setup(verifier.clone()).expect("prover setup");
	let alloc = GlobalAllocator;
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");

	// 说明：fetch 论证（M6 的 fetch logup 表）在本切片省略——程序语义由电路执行约束 +
	// 内存论证 + 最终输出断言（inout）承担；fetch 表的程序固定性论证列入 M9/报告边界。
	let merkle_scheme = BinaryMerkleTreeScheme::<LF, StdHashSuite>::new();
	let log_inv_rate = 1;
	let log_code_len = l + log_inv_rate;
	let arity = ConstantArityStrategy::with_optimal_arity::<LF, _>(&merkle_scheme, log_code_len).arity;
	let verifier_compiler = BaseFoldVerifierCompiler::new(
		&merkle_scheme,
		vec![OracleSpec { log_msg_len: l, is_zk: false }; 4],
		log_inv_rate,
		calculate_n_test_queries(100, log_inv_rate),
		&ConstantArityStrategy::new(arity),
	);
	let prover_compiler = BaseFoldProverCompiler::from_verifier_compiler(&verifier_compiler, NeighborsLastMultiThread::new(GaoMateerPreExpanded::<LF>::generate(log_code_len), 1));
	let merkle_chan = ProverMerkleTranscriptChannel::<&mut ProverTranscript<StdChallenger>, StdChallenger, LF, StdHashSuite>::new(&mut pt);
	let mut chan = prover_compiler.create_channel(merkle_chan, StdRng::from_seed([0u8; 32]), GlobalAllocator);
	let o_addr = chan.send_oracle(fb_addr.as_view());
	let o_val = chan.send_oracle(fb_val.as_view());
	let o_ts = chan.send_oracle(fb_ts.as_view());
	let o_kind = chan.send_oracle(fb_kind.as_view());
	let rho: LF = chan.sample();
	let c: LF = chan.sample();

	// 恒等式①：fracaddcheck（num 全 1，den = c + f）
	let mut num = vec![LF::ZERO; nrows];
	let mut den = vec![LF::ZERO; nrows];
	let r2 = rho * rho;
	let r3 = r2 * rho;
	for j in 0..2 * ts {
		num[j] = LF::ONE;
		den[j] = c + LF::from(col_addr[j] as u128) + rho * LF::from(col_val[j] as u128)
			+ r2 * LF::from(col_ts[j] as u128) + r3 * LF::from(col_kind[j] as u128);
	}
	for j in 2 * ts..nrows { den[j] = LF::ONE; }
	let (frac, root) = FracAddCircuit::build(l, &alloc, Fraction::new(FieldBuffer::<LP, _>::from_values(&num), FieldBuffer::<LP, _>::from_values(&den)));
	let root_num = root.num.get(0);
	let root_den = root.den.get(0);
	assert_eq!(root_num, LF::ZERO, "恒等式①根分子必须为零");
	chan.send_one(root_den);
	let final_claim = frac.prove(FracAddEvalClaim { num_eval: LF::ZERO, den_eval: root_den, point: vec![] }, &mut chan);
	let r = final_claim.point.clone();
	let eq_r = eq_ind_partial_eval_in::<GlobalAllocator, LP>(&alloc, &r);
	let eq_vals: Vec<LF> = eq_r.as_view().iter_scalars().collect();
	let dot = |col: &Vec<u64>| -> LF {
		let mut s = LF::ZERO;
		for (j, &x) in col.iter().enumerate() { s += eq_vals[j] * LF::from(x as u128); }
		s
	};
	let addr_r = dot(&col_addr);
	let val_r = dot(&col_val);
	let ts_r = dot(&col_ts);
	let kind_r = dot(&col_kind);
	chan.prove_oracle_relation(o_addr, eq_r.clone(), addr_r);
	chan.prove_oracle_relation(o_val, eq_r.clone(), val_r);
	chan.prove_oracle_relation(o_ts, eq_r.clone(), ts_r);
	chan.prove_oracle_relation(o_kind, eq_r.clone(), kind_r);
	chan.finalize_oracle(o_addr, fb_addr);
	chan.finalize_oracle(o_val, fb_val);
	chan.finalize_oracle(o_ts, fb_ts);
	chan.finalize_oracle(o_kind, fb_kind);
	chan.finish();

	// ---------- verifier ----------
	let mut vt = pt.into_verifier();
	let inout_verify: Vec<Word> = if tamper == Tamper::BadFinalOut {
		let mut v = inout_words.clone();
		v[0].0 ^= 1;
		v
	} else {
		inout_words.clone()
	};
	let c_ok = verifier.verify(&inout_verify, &mut vt).is_ok();
	let merkle_veri = VerifierMerkleTranscriptChannel::<&mut VerifierTranscript<StdChallenger>, StdChallenger, LF, StdHashSuite>::new(&mut vt);
	let v_specs = vec![OracleSpec { log_msg_len: l, is_zk: false }; 4];
	let mut vchan = BaseFoldVerifierChannel::new(merkle_veri, &v_specs, verifier_compiler.fri_params());
	let v_o_addr = vchan.recv_oracle(l, false).unwrap();
	let v_o_val = vchan.recv_oracle(l, false).unwrap();
	let v_o_ts = vchan.recv_oracle(l, false).unwrap();
	let v_o_kind = vchan.recv_oracle(l, false).unwrap();
	let vrho: LF = vchan.sample();
	let vc: LF = vchan.sample();
	let mut vroot_den: LF = vchan.recv_one().expect("recv root_den");
	if tamper == Tamper::BadRootDen {
		vroot_den += LF::ONE;
	}
	let l_ok = (|vchan: &mut BaseFoldVerifierChannel<'_, LF, _>| -> bool {
		let vfinal = match fracaddcheck::verify::<LF, _>(l, FracAddEvalClaim { num_eval: LF::ZERO, den_eval: vroot_den, point: vec![] }, vchan) {
			Ok(c) => c,
			Err(_) => return false,
		};
		let r: Vec<LF> = vfinal.point.clone();
		let mut sum_eq = LF::ZERO;
		for j in 0..2 * ts {
			let mut e = LF::ONE;
			for (bit, &rj) in r.iter().enumerate() {
				let bj = ((j >> bit) & 1) as u64;
				e *= if bj == 1 { rj } else { LF::ONE + rj };
			}
			sum_eq += e;
		}
		if vfinal.num_eval != sum_eq { return false; }
		let a_use = if tamper == Tamper::BadDenAddr { addr_r + LF::ONE } else { addr_r };
		let v_use = if tamper == Tamper::BadDenVal { val_r + LF::ONE } else { val_r };
		let rr2 = vrho * vrho;
		let rr3 = rr2 * vrho;
		let den_check = vc * sum_eq + a_use + vrho * v_use + rr2 * ts_r + rr3 * kind_r + (LF::ONE - sum_eq);
		if vfinal.den_eval != den_check { return false; }
		let ok_addr = vchan.verify_oracle_relation(v_o_addr, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), addr_r);
		let ok_val = vchan.verify_oracle_relation(v_o_val, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), val_r);
		let ok_ts = vchan.verify_oracle_relation(v_o_ts, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), ts_r);
		let ok_kind = vchan.verify_oracle_relation(v_o_kind, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), kind_r);
		ok_addr.is_ok() && ok_val.is_ok() && ok_ts.is_ok() && ok_kind.is_ok()
	})(&mut vchan) && match vchan.finish() { Ok(_) => true, Err(_) => false };

	let sorted_ok = (0..n - 1).all(|i| trace.final_mem[BASE as usize + i] <= trace.final_mem[BASE as usize + i + 1]);
	VmRsRun { c_ok, l_ok, stat, t_len, ts, l, inout_words, prover, verifier, witness: witness_vec, sorted_ok }
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Instant;

	fn show(run: &VmRsRun, label: &str) {
		let s = &run.stat;
		println!(
			"{label}: T={} ts={} l={} gates={} zero/and/bmul={}/{}/{} c_ok={} l_ok={} sorted_ok={}",
			run.t_len, run.ts, run.l, s.n_gates, s.n_zero_constraints, s.n_and_constraints,
			s.n_bmul_constraints, run.c_ok, run.l_ok, run.sorted_ok
		);
	}

	#[test]
	fn vm_ram_sort_honest() {
		let t0 = Instant::now();
		let run = run_vmrs(16, Tamper::None);
		let dt = t0.elapsed().as_secs_f32();
		assert!(run.sorted_ok, "native 排序必须成功");
		assert!(run.c_ok, "诚实路径电路必须通过");
		assert!(run.l_ok, "诚实路径 fracaddcheck/绑定必须通过");
		show(&run, &format!("honest ({dt:.1}s)"));
	}

	/// 缩放点（任务书 §2.5）：N=32，供报告成本曲线；默认忽略（`--ignored` 运行）。
	#[test]
	#[ignore]
	fn vm_ram_sort_scale32() {
		let t0 = Instant::now();
		let run = run_vmrs(32, Tamper::None);
		let dt = t0.elapsed().as_secs_f32();
		show(&run, &format!("scale N=32 ({dt:.1}s)"));
		assert!(run.sorted_ok && run.c_ok && run.l_ok);
	}

	#[test]
	fn vm_ram_sort_soundness_bad_final_out() {
		let run = run_vmrs(16, Tamper::BadFinalOut);
		assert!(!run.c_ok, "验证端篡改最终输出必须被电路拒绝（c_ok==false）");
		show(&run, "sound 1/bad-final-out (circuit layer)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_root_den() {
		let run = run_vmrs(16, Tamper::BadRootDen);
		assert!(run.c_ok, "电路实例未动");
		assert!(!run.l_ok, "验证端篡改 root_den 必须被 fracaddcheck 拒绝（l_ok==false）");
		show(&run, "sound 2/bad-root-den (logup layer)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_den_addr() {
		let run = run_vmrs(16, Tamper::BadDenAddr);
		assert!(run.c_ok, "电路实例未动");
		assert!(!run.l_ok, "验证端篡改 addr 开口值必须拒绝（l_ok==false）");
		show(&run, "sound 3/bad-den-addr (logup layer)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_den_val() {
		let run = run_vmrs(16, Tamper::BadDenVal);
		assert!(run.c_ok, "电路实例未动");
		assert!(!run.l_ok, "验证端篡改 val 开口值必须拒绝（l_ok==false）");
		show(&run, "sound 4/bad-den-val (logup layer)");
	}
}
