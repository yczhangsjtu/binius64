//! 切片 28: `vm_ram_sort` — M8-A 整合：真实状态机 VM（vm32 语义）× M7 可扩展 RAM 论证
//! × BaseFold 强承诺通道 × **M12 预处理模型 succinct 验证**。
//!
//! 架构（M8-A 任务书 §2 + M12-T1/T2 改造）：
//! - 执行核心：RV32I 子集（vm32 同款译码/执行/寄存器值链）；RAM 版本链删除，
//!   `ld_val` 读语义由内存论证承担。
//! - 事件列：每周期恰一行 (addr, ts=2t/2t+1, val, kind)，kind∈{0=none/PAD,1=read,2=write}。
//! - 排序流：每触及地址（含占位地址）init 首 + 事件按 ts 升序 + final 尾；
//!   恒等式①（fracaddcheck 多重集合）+ 恒等式②（电路词级断言：非降/ts 严增/读一致/init 形状）。
//! - **M12-T1 公开输入 O(1)**：逐周期列（inst/pc、排序流 8 列）全部 committed-only
//!   （BaseFold send_oracle），公开 inout 收敛为 **24 词恒定**（程序哈希 + init 哈希 +
//!   输出 + 输出地址 + χ 挑战 + 6 个 χ-dot 锚定声明）。
//!   witness↔oracle 绑定 = **χ-dot 锚**：验证端 transcript 挑战 χ（承诺后采样），
//!   电路内以 bmul（GF(2^128) 单约束乘法）累加 Σχ^j·witness 列并断言 == 公开词，
//!   验证端对 oracle 列排队同泛函的 oracle relation → 两份承诺在 χ-泛函下绑定。
//! - **M12-T1 uniform 电路**：F3 事件钉扎改为事件侧（d_*，周期序）逐周期统一断言
//!   （行 n_touch+t == 周期 t 派生事件），消除数据依赖的 ev_bindings/init_rows 电路参数
//!   （ verifier 预处理的前提）。排序流内容绑定链 = 电路 d 侧钉扎 → χ-dot → 恒等式①
//!   → χ-dot → 电路 s 侧恒等式②。
//! - **fetch（单 looker 重构）**：inst/pc 列 committed；logup* 单个全点 looker
//!   （eval_point = r_fetch，claim e = Σeq·prog[pc>>2] 经 inst-oracle relation 绑定）；
//!   F2 位置绑定 = index_eval_claim(z) 与 pc-oracle 在 z 点的 oracle relation 对照。
//! - 三件套：init 镜像 = 全 0（默认模式电路断言 init 行 val==0）；final 输出 = 公开词
//!   （OUT_ADDR 的 final 值，M5 唯一性断言防 XOR 相消）。
//! - **M12-T2 预处理拆分**：`vmrs_verifier_setup`（一次性：建电路/CS/编译器）→
//!   `VmRsVerifierKey`；`vmrs_verify_online`（不建电路）。`vmrs_verify` = 兼容包装。
//!
//! 表述纪律（规划文档 §5）：这是**预处理模型下的 succinct 在线验证**
//! （预处理 O(T) 一次性，在线 O(1) 公开输入 + polylog 密码学工作），
//! **不是**无条件 succinct（proof 体积仍随 T 线性，见 KNOWN_BOUNDARIES）。

use binius_compute::GlobalAllocator;
use binius_core::word::Word;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_field::Field;
use binius_frontend::{Circuit, CircuitBuilder, CircuitStat, Wire};
use binius_hash::StdHashSuite;
use binius_iop::basefold::channel::BaseFoldVerifierChannel;
use binius_iop::basefold::compiler::BaseFoldVerifierCompiler;
use binius_hash::hash_serialize;
use binius_ip::channel::IPVerifierChannel;
use binius_ip::fracaddcheck;
use binius_ip::fracaddcheck::FracAddEvalClaim;
use binius_ip::logup_star::LookerClaim;
use binius_iop::channel::{IOPVerifierChannel, OracleSpec};
use binius_iop::fri::{ConstantArityStrategy, calculate_n_test_queries, FRIParams};
use binius_iop::merkle_channel::VerifierMerkleTranscriptChannel;
use binius_iop::merkle_tree::BinaryMerkleTreeScheme;
use binius_iop_prover::basefold::compiler::BaseFoldProverCompiler;
use binius_iop_prover::channel::IOPProverChannel;
use binius_iop_prover::merkle_channel::ProverMerkleTranscriptChannel;
use binius_ip_prover::channel::IPProverChannel;
use binius_ip_prover::logup_star::{Looker as LogupLooker, TableLookup as ProverTableLookup};
use binius_ip_prover::fracaddcheck::fraction::Fraction;
use binius_ip_prover::fracaddcheck::FracAddCircuit;
use binius_math::multilinear::eq::{eq_ind, eq_ind_partial_eval_in};
use binius_math::ntt::domain_context::GaoMateerPreExpanded;
use binius_math::ntt::NeighborsLastMultiThread;
use binius_math::FieldVec;
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
/// 数据数组基址（**字节地址**，M8-C T3；字索引 = BASE>>2）。
pub const BASE: u64 = 0x1000;
/// 排序元素数（主测 16 字——prove 时间预算；缩放点 32/64 可选）。
#[allow(dead_code)]
pub const N: usize = 16;
/// kind 编码：0=init/none, 1=read, 2=write, 3=final。
pub const K_INIT: u64 = 0;
pub const K_READ: u64 = 1;
pub const K_WRITE: u64 = 2;
pub const K_FINAL: u64 = 3;
/// 占位（无访问）地址——组在排序流最末。
pub const PAD_ADDR: u64 = 0xffff;
/// 输出三件套地址 = BASE 的字索引（排序后最小元素）。
pub const OUT_ADDR: u64 = BASE >> 2;
/// ecall（停机）指令编码（M12-T3：末周期指令终止断言）。
pub const ECALL: u64 = 0x00000073;

// ---- 指令编码（vm32/isa 同款，简化版；程序用到的子集） ----
#[allow(dead_code)]
fn opcode(x: u64, lo: u32, hi: u32) -> u64 {
	(x >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}
fn i_enc(op: u64, f3: u64, rd: u64, rs1: u64, imm: i64) -> u64 {
	((imm as u64 & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn s_enc(op: u64, f3: u64, rs2: u64, rs1: u64, imm: i64) -> u64 {
	(((imm as u64) & 0xfe0) << 20) | (rs2 << 20) | (rs1 << 15) | (f3 << 12)
		| (((imm as u64) & 0x1f) << 7) | op
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
#[allow(dead_code)]
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
	// 段 1：写数据 word[(BASE>>2)+j]（字节地址 base + 4j；M8-C T3 全局字节地址语义）。
	let mut p: Vec<u64> = Vec::new();
	for j in 0..n {
		let v = data[j];
		// lui + addi 标准组装：低 12 位 >= 0x800 时加 1 到高 20 位、低 12 位取负（addi 符号扩展）。
		let hi = v >> 12;
		let lo = v & 0xfff;
		let (hi, lo) = if lo >= 0x800 { (hi + 1, lo - 0x1000) } else { (hi, lo) };
		p.push(lui(15, hi));
		p.push(addi(15, 15, lo));
		let addr = base + 4 * j as i64;
		p.push(lui(16, addr >> 12));
		p.push(addi(16, 16, addr & 0xfff));
		p.push(sw(15, 16, 0));
	}
	// 段 2：bubblesort（元素地址 = BASE + 4j 字节；x14 = 字节偏移，步 4）。
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
	// M8-C T3 字节地址语义：循环变量（x12 外层 / x14 内层）均为字节偏移，上界 = 4*(n-1)。
	let ub = 4 * (n as i64 - 1);
	let mut s2: Vec<u64> = Vec::new();
	s2.push(lui(12, 0));
	s2.push(lui(15, (ub >> 12) as i64));
	s2.push(addi(15, 15, ub & 0xfff));
	s2.push(bgeu(12, 15, (l_halt - (w + 3)) * 4));
	s2.push(lui(14, 0));
	s2.push(lui(15, (ub >> 12) as i64));
	s2.push(addi(15, 15, ub & 0xfff));
	s2.push(sub(15, 15, 12));
	s2.push(bgeu(14, 15, (l_in_done - (w + 8)) * 4));
	s2.push(lui(16, base >> 12));
	s2.push(addi(16, 16, base & 0xfff));
	s2.push(add(16, 16, 14));
	s2.push(lw(17, 16, 0));
	s2.push(lw(18, 16, 4));
	s2.push(bgeu(18, 17, (l_swap_skip - (w + 14)) * 4));
	s2.push(sw(18, 16, 0));
	s2.push(sw(17, 16, 4));
	s2.push(addi(14, 14, 4));
	s2.push(jal(0, (l_in - (w + 18)) * 4));
	s2.push(addi(12, 12, 4));
	s2.push(jal(0, (l_out - (w + 20)) * 4));
	s2.push(ECALL); // ecall — halt
	p.extend(s2);
	if slot < p.len() {
		p[slot]
	} else {
		ECALL
	}
}

// ---- fetch 表（M8-B T0）：程序镜像作为 committed 表 + 公开镜像哈希 ----
/// 程序表行数的 log2（段 1 每字 5 条 + 段 2 固定 22 条，向上取 2 的幂）。
pub fn m_prog(n: usize) -> usize {
	let slots = 5 * n + 22;
	(usize::BITS - (slots - 1).leading_zeros()) as usize
}

/// committed fetch 表列：T[slot] = prog_image(slot, n)，pad 槽 = ecall。
pub fn prog_col(n: usize) -> Vec<u64> {
	(0..1usize << m_prog(n)).map(|slot| prog_image(slot, n)).collect()
}

/// 公开程序镜像哈希：StdDigest(Sha256) over 序列化的镜像 LF 列 → 4 个 u64（LE）。
/// 这是"执行的程序"的公共输入对照值（承诺绑定见 oracle relation：prog 表 claim）。
pub fn prog_image_hash(n: usize) -> [u64; 4] {
	let elems: Vec<LF> = prog_col(n).iter().map(|&x| LF::from(x as u128)).collect();
	let digest = hash_serialize::<LF, binius_hash::StdDigest>(&elems).expect("hash prog image");
	let mut h = [0u64; 4];
	for (i, w) in h.iter_mut().enumerate() {
		let mut b = [0u8; 8];
		b.copy_from_slice(&digest.as_slice()[i * 8..(i + 1) * 8]);
		*w = u64::from_le_bytes(b);
	}
	h
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
		// M8-C T3：地址语义统一为字节地址——字索引 = (addr >> 2) & (K-1)。
		let addr_calc = ((a.wrapping_add(imm_i)) as u64 >> 2) & (K as u64 - 1);
		let mut load = None;
		let mut store = None;
		if opcode == OP_LOAD && funct3 == 0x2 {
			let v = ramver_native[addr_calc as usize];
			load = Some(MemAccess { addr: addr_calc as usize, ver: v, val: mem[addr_calc as usize] });
		} else if opcode == OP_STORE && funct3 == 0x2 {
			let st_addr = ((a.wrapping_add(imm_s)) >> 2) & (K as u64 - 1);
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
			m_q: 0,
		});
		pc = next_pc;
		if halted { break; }
	}
	let final_regs: [u32; 32] = regs;
	BigTrace { cycles, final_regs, final_mem: mem }
}

/// 排序流：每触及地址（真实 + PAD_ADDR 组）init 首 + 事件升序 + final 尾。
/// `final_ts`：final 行的时间戳（须大于全部事件 ts；单事件布局下 = 2T+1）。
/// `init_mem`：初始内存词表（M8-C init 非零化；None = 全 0）——init 行 val = 词表[addr]。
pub fn build_sorted_with_final(events: &[Visit], final_ts: u64, init_mem: Option<&[u32]>) -> Vec<Visit> {
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
		// M8-C init 非零化：init 行 val = 初始镜像词（d 侧 init 行由电路钉扎，见 build_circuit_vmrs）。
		if a != PAD_ADDR {
			let iv = init_mem.map(|m| m.get(a as usize).copied().unwrap_or(0)).unwrap_or(0);
			sorted.push(Visit { addr: a, ts: 0, val: iv as u64, kind: K_INIT });
		}
		for e in events.iter().filter(|e| e.addr == a) {
			sorted.push(*e);
		}
		let fv = *latest.get(&a).unwrap_or(&0);
		sorted.push(Visit { addr: a, ts: final_ts, val: fv, kind: K_FINAL });
	}
	sorted
}

/// 兼容包装：final ts = 事件数 + 1（单事件布局）、init 全 0。
pub fn build_sorted(events: &[Visit]) -> Vec<Visit> {
	build_sorted_with_final(events, events.len() as u64 + 1, None)
}

// ---- 事件列：每周期恰一行（load ts=2t、store ts=2t+1、PAD ts=2t）----
/// 返回 (事件行, 事件侧全列)。事件侧全列 = [init×n_touch][事件×T][final×n_touch+1]，
/// 布局由形状参数完全决定（M12-T1 uniform 电路的事件侧钉扎基础）。
pub fn event_rows(trace: &BigTrace, init_mem: Option<&[u32]>) -> (Vec<Visit>, Vec<Visit>) {
	let mut rows: Vec<Visit> = Vec::with_capacity(trace.cycles.len());
	for (t, c) in trace.cycles.iter().enumerate() {
		let (t0, t1) = (2 * t as u64, 2 * t as u64 + 1);
		if let Some(ld) = &c.load {
			rows.push(Visit { addr: ld.addr as u64, ts: t0, val: ld.val as u64, kind: K_READ });
		} else if let Some(st) = &c.store {
			rows.push(Visit { addr: st.addr as u64, ts: t1, val: st.val as u64, kind: K_WRITE });
		} else {
			rows.push(Visit { addr: PAD_ADDR, ts: t0, val: 0, kind: K_INIT });
		}
	}
	// 事件侧重排：init(触达地址) + 事件行 + final(触达地址，含 PAD)
	let mut addrs: Vec<u64> = rows.iter().map(|e| e.addr).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
	addrs.sort_unstable();
	let mut side = Vec::new();
	for &a in &addrs {
		// 与 build_sorted 对齐：PAD 组不推 init 行；init 行 val = 初始镜像词（非零化）。
		if a != PAD_ADDR {
			let iv = init_mem.map(|m| m.get(a as usize).copied().unwrap_or(0)).unwrap_or(0);
			side.push(Visit { addr: a, ts: 0, val: iv as u64, kind: K_INIT });
		}
	}
	side.extend_from_slice(&rows);
	let final_ts = 2 * trace.cycles.len() as u64 + 1;
	for &a in &addrs {
		let fv = rows.iter().rev().find(|e| e.addr == a && e.kind == K_WRITE).map(|e| e.val).unwrap_or(0);
		side.push(Visit { addr: a, ts: final_ts, val: fv, kind: K_FINAL });
	}
	(rows, side)
}

// ---- 公开输入布局（M12-T1：O(1)，与 T 无关，23 词恒定） ----
// [prog_hash×4, init_hash×4, final_out, chi(lo,hi),
//  dot_maddr(lo,hi), dot_mval(lo,hi), dot_mts(lo,hi), dot_mkind(lo,hi),
//  dot_inst(lo,hi), dot_pc(lo,hi)]
/// 排序流/事件侧 4 列（sorted ‖ side，nrows 行）的 χ-dot 声明在 IO 中的基址。
pub const IO_PROG_HASH: usize = 0; // 4 词，声明性（hash_ok 对照）
pub const IO_INIT_HASH: usize = 4; // 4 词，声明性外部锚（见 KNOWN_BOUNDARIES init 条目）
pub const IO_FINAL_OUT: usize = 8; // 输出词（电路 XOR + M5 唯一性）
pub const IO_OUT_ADDR: usize = 9;  // 输出字地址（字索引；陈述的一部分："地址 A 的 final 值"）
pub const IO_CHI: usize = 10;      // 2 词，B128 χ 挑战（验证端须 == transcript 采样）
pub const IO_DOT_MADDR: usize = 12;
pub const IO_DOT_MVAL: usize = 14;
pub const IO_DOT_MTS: usize = 16;
pub const IO_DOT_MKIND: usize = 18;
pub const IO_DOT_INST: usize = 20;
pub const IO_DOT_PC: usize = 22;
pub const IO_LEN: usize = 24;

// ---- 电路：执行 + uniform 事件钉扎 + 排序流恒等式② + χ-dot 锚 + 三件套 ----
pub struct VmRsIref {
	pub inst: Vec<Wire>, pub pc: Vec<Wire>,
	pub rd1_reg: Vec<Wire>, pub rd1_val: Vec<Wire>,
	pub rd2_reg: Vec<Wire>, pub rd2_val: Vec<Wire>,
	pub wr_reg: Vec<Wire>, pub wr_val: Vec<Wire>, pub wr_iswrite: Vec<Wire>,
	pub ld_addr: Vec<Wire>, pub ld_val: Vec<Wire>, pub is_load: Vec<Wire>,
	pub st_addr: Vec<Wire>, pub st_val: Vec<Wire>, pub is_store: Vec<Wire>,
	/// 排序半侧（恒等式②承受列；committed-only witness，经 χ-dot 锚到 oracle）。
	pub s_addr: Vec<Wire>, pub s_ts: Vec<Wire>, pub s_val: Vec<Wire>, pub s_kind: Vec<Wire>,
	/// 事件半侧（周期序 uniform 钉扎；committed-only witness）。
	pub d_addr: Vec<Wire>, pub d_ts: Vec<Wire>, pub d_val: Vec<Wire>, pub d_kind: Vec<Wire>,
	pub final_out: Wire,
	/// 公开输出字地址（字索引；陈述："地址 A 的 final 值"）。
	pub out_addr: Wire,
	/// 公开程序镜像哈希（4 词，声明性）。
	pub prog_hash: [Wire; 4],
	/// 公开 init 镜像哈希（4 词，声明性外部锚）。
	pub init_hash: [Wire; 4],
	/// χ 挑战（B128 lo/hi 公开词；验证端预检 == transcript 采样）。
	pub chi: [Wire; 2],
	/// 6 个 χ-dot 锚定声明（B128 lo/hi 公开词）。
	pub dot_claims: [[Wire; 2]; 6],
}

/// 形状参数 → 电路（M12-T1 uniform：只依赖 (t_len, ts, init_zero)）。
/// n_touch = (ts − t_len − 1) / 2（协议结构常数：init/final 各 n_touch、PAD final 1 行）。
#[derive(Clone, Copy, Debug)]
pub struct VmRsShape {
	pub t_len: usize,
	pub ts: usize,
	pub init_zero: bool,
}

impl VmRsShape {
	pub fn n_touch(&self) -> usize {
		(self.ts - self.t_len - 1) / 2
	}
	pub fn l(&self) -> usize {
		(usize::BITS - ((2 * self.ts) - 1).leading_zeros()) as usize
	}
	pub fn li(&self) -> usize {
		(usize::BITS - (self.t_len - 1).leading_zeros()) as usize
	}
}

fn assert_sortedness(b: &CircuitBuilder, sa: &[Wire], sts: &[Wire], sv: &[Wire], sk: &[Wire]) {
	let z = b.add_constant_64(0);
	let o = b.add_constant_64(1);
	let ts = sa.len();
	// init 行 val 的对照：默认模式（init_zero）钉 0；ELF 模式经 d 侧 init 行 +
	// 恒等式① + χ-dot 链绑定（见模块头与 KNOWN_BOUNDARIES init 条目）。
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

pub fn build_circuit_vmrs(t_len: usize, ts: usize, mp: usize, init_zero: bool) -> (Circuit, VmRsIref) {
	let shape = VmRsShape { t_len, ts, init_zero };
	let n_touch = shape.n_touch();
	let b = CircuitBuilder::new();
	let zero = b.add_constant_64(0);
	let one = b.add_constant_64(1);
	// ---- 公开 inout（23 词恒定，M12-T1；声明顺序 = IO_* 布局） ----
	let prog_hash = [b.add_inout(), b.add_inout(), b.add_inout(), b.add_inout()];
	let init_hash = [b.add_inout(), b.add_inout(), b.add_inout(), b.add_inout()];
	let final_out = b.add_inout();
	let out_addr = b.add_inout();
	let chi = [b.add_inout(), b.add_inout()];
	let mut dot_claims = [[zero, zero]; 6];
	for slot in dot_claims.iter_mut() {
		*slot = [b.add_inout(), b.add_inout()];
	}
	// ---- committed-only 逐周期列（M12-T1：原 inout 全部降为 witness） ----
	let inst = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let pc = (0..t_len).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_addr = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_ts = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_val = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_kind = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let d_addr = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let d_ts = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let d_val = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let d_kind = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
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

	// 寄存器值链（内部 wire）
	let mut cur_reg = [zero; 32];
	let mut cur_ver = [zero; 32];
	let mut prev_pc = b.add_constant_64(0); // PC_START = 0
	// 事件行派生（周期 t 的唯一事件行 addr/kind/val/ts）
	let mut ev_row_addr: Vec<Wire> = Vec::with_capacity(t_len);
	let mut ev_row_kind: Vec<Wire> = Vec::with_capacity(t_len);
	let mut ev_row_val: Vec<Wire> = Vec::with_capacity(t_len);
	let mut ev_row_ts: Vec<Wire> = Vec::with_capacity(t_len);
	// pc 槽号（fetch χ-dot 用）
	let mut pc_slot: Vec<Wire> = Vec::with_capacity(t_len);

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
		// M8-C T3：地址语义统一字节地址——字索引 = (addr >> 2) & (K-1)
		let ld_addr_w = b.band(b.srl32(b.iadd_32(rs1v, imm_i), 2), b.add_constant_64(K as u64 - 1));
		let st_addr_w = b.band(b.srl32(b.select(c_is_store, b.iadd_32(rs1v, imm_s), b.iadd_32(rs1v, imm_i)), 2), b.add_constant_64(K as u64 - 1));
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
		// 事件 pinning（执行↔事件 witness；全部 witness 输入）
		let is_alu_write_01 = b.select(is_alu_write, one, zero);
		// 周期 t 的唯一事件行：store → (st_addr, 2t+1, rs2v, WRITE)；
		// load → (ld_addr, 2t, ld_val, READ)；其余 → (PAD, 2t, 0, INIT)。
		let ev_a_load = ld_addr_w;
		let ev_a_none = b.add_constant_64(PAD_ADDR);
		let ev_row_a = b.select(c_is_store, st_addr_w, b.select(c_is_load, ev_a_load, ev_a_none));
		let ev_k_read = b.add_constant_64(K_READ);
		let ev_k_write = b.add_constant_64(K_WRITE);
		let ev_k_none = b.add_constant_64(K_INIT);
		let ev_row_k = b.select(c_is_store, ev_k_write, b.select(c_is_load, ev_k_read, ev_k_none));
		let ev_row_v = b.select(c_is_store, rs2v, b.select(c_is_load, ld_val[t], zero));
		let ev_row_t = b.iadd(b.add_constant_64(2 * t as u64), b.select(c_is_store, one, zero)).0;
		ev_row_addr.push(ev_row_a);
		ev_row_kind.push(ev_row_k);
		ev_row_val.push(ev_row_v);
		ev_row_ts.push(ev_row_t);
		pc_slot.push(b.band(b.srl32(pc[t], 2), b.add_constant_64(0xffff)));
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
	// M12-T3（F4 残余闭合）：显式终止断言——末周期指令必须是 ecall。
	b.assert_eq("final_inst_ecall", inst[t_len - 1], b.add_constant_64(ECALL));
	// 恒等式②（排序流良构 + 读一致性）
	assert_sortedness(&b, &s_addr, &s_ts, &s_val, &s_kind);
	// M12-T1 uniform 事件侧钉扎（替代 M11 F3 的数据依赖 ev_bindings）：
	// 事件半侧布局 = [init×n_touch][事件×T][final×n_touch+1]，全部逐行断言。
	for k in 0..n_touch {
		b.assert_eq(format!("d_init_ts[{k}]"), d_ts[k], zero);
		b.assert_eq(format!("d_init_kind[{k}]"), d_kind[k], b.add_constant_64(K_INIT));
		if init_zero {
			// 默认模式：初始镜像全 0，电路内直接钉扎（ELF 模式见 KNOWN_BOUNDARIES）。
			b.assert_eq(format!("d_init_val[{k}]"), d_val[k], zero);
		}
	}
	for t in 0..t_len {
		let row = n_touch + t;
		b.assert_eq(format!("ev_row_addr[{row}]"), d_addr[row], ev_row_addr[t]);
		b.assert_eq(format!("ev_row_ts[{row}]"), d_ts[row], ev_row_ts[t]);
		b.assert_eq(format!("ev_row_val[{row}]"), d_val[row], ev_row_val[t]);
		b.assert_eq(format!("ev_row_kind[{row}]"), d_kind[row], ev_row_kind[t]);
	}
	for k in 0..n_touch + 1 {
		let row = n_touch + t_len + k;
		b.assert_eq(format!("d_final_ts[{k}]"), d_ts[row], b.add_constant_64(2 * t_len as u64 + 1));
		b.assert_eq(format!("d_final_kind[{k}]"), d_kind[row], b.add_constant_64(K_FINAL));
	}
	// 三件套：final 输出（公开地址 out_addr 的最终值；默认模式 = OUT_ADDR = 排序后最小元素）
	// M12-T3（M5）：hit 计数 == 1 断言——多条 final 行经 XOR 相消可把输出伪造成 0。
	let mut acc = zero;
	let mut hit_sum = zero;
	for j in 0..ts {
		let hit = b.band(b.icmp_eq(s_addr[j], out_addr), b.icmp_eq(s_kind[j], b.add_constant_64(K_FINAL)));
		let hit01 = b.select(hit, one, zero);
		let picked = b.select(hit, s_val[j], zero);
		acc = b.bxor(acc, picked);
		hit_sum = b.iadd(hit_sum, hit01).0;
	}
	b.assert_eq("final_out", final_out, acc);
	b.assert_eq("final_unique", hit_sum, one);

	// ---- M12-T1 χ-dot 锚（witness↔oracle 绑定） ----
	// 泛函：⟨列, χ-幂向量⟩。电路内以 bmul（GF(2^128) 单约束）累加，断言 == 公开词。
	// oracle 侧同名泛函经 prove/verify_oracle_relation 绑定（transparent = χ-幂系数向量）。
	// 记忆 4 列：oracle 行 j ∈ [0, 2ts)：j < ts → s_c[j]，j ∈ [ts, 2ts) → d_c[j−ts]。
	// 列序与 IO_DOT_* 槽位一致：[addr, val, ts, kind]
	let s_cols = [&s_addr, &s_val, &s_ts, &s_kind];
	let d_cols = [&d_addr, &d_val, &d_ts, &d_kind];
	let mut mem_acc: [(Wire, Wire); 4] = [(zero, zero); 4];
	let mut pw = (one, zero); // χ^j，j 从 0 起
	let mut pw_off = (one, zero); // χ^{ts+j}
	for _ in 0..ts {
		pw_off = b.bmul(chi[0], chi[1], pw_off.0, pw_off.1);
	}
	for j in 0..ts {
		for c in 0..4 {
			let p = b.bmul(s_cols[c][j], zero, pw.0, pw.1);
			mem_acc[c].0 = b.bxor(mem_acc[c].0, p.0);
			mem_acc[c].1 = b.bxor(mem_acc[c].1, p.1);
		}
		pw = b.bmul(chi[0], chi[1], pw.0, pw.1);
		for c in 0..4 {
			let p = b.bmul(d_cols[c][j], zero, pw_off.0, pw_off.1);
			mem_acc[c].0 = b.bxor(mem_acc[c].0, p.0);
			mem_acc[c].1 = b.bxor(mem_acc[c].1, p.1);
		}
		pw_off = b.bmul(chi[0], chi[1], pw_off.0, pw_off.1);
	}
	// inst/pc 列：oracle 行 j ∈ [0, 2^L)（L = oracle_log_len(l, mp)，全部 oracle 统一长度），head = witness，tail = pad 常量
	// （inst pad = ecall；pc pad = 最大槽号；二者与 fetch looker 的 index pad 一致，
	//  pad 行经 e-relation 与 index-eval relation 分别锚定）。
	let li_pow2 = 1usize << oracle_log_len(shape.l(), mp);
	let inst_pad = b.add_constant_64(ECALL);
	let pc_pad = b.add_constant_64((1u64 << mp) - 1);
	let mut acc_inst = (zero, zero);
	let mut acc_pc = (zero, zero);
	let mut pwi = (one, zero);
	for j in 0..li_pow2 {
		let iv = if j < t_len { inst[j] } else { inst_pad };
		let pv = if j < t_len { pc_slot[j] } else { pc_pad };
		let pi = b.bmul(iv, zero, pwi.0, pwi.1);
		acc_inst.0 = b.bxor(acc_inst.0, pi.0);
		acc_inst.1 = b.bxor(acc_inst.1, pi.1);
		let pp = b.bmul(pv, zero, pwi.0, pwi.1);
		acc_pc.0 = b.bxor(acc_pc.0, pp.0);
		acc_pc.1 = b.bxor(acc_pc.1, pp.1);
		pwi = b.bmul(chi[0], chi[1], pwi.0, pwi.1);
	}
	// 锚定声明断言（公开词）
	for c in 0..4 {
		b.assert_eq_v(format!("dot_mem{c}"), [mem_acc[c].0, mem_acc[c].1], dot_claims[c]);
	}
	b.assert_eq_v("dot_inst", [acc_inst.0, acc_inst.1], dot_claims[4]);
	b.assert_eq_v("dot_pc", [acc_pc.0, acc_pc.1], dot_claims[5]);

	(
		b.build(),
		VmRsIref { inst, pc, rd1_reg, rd1_val, rd2_reg, rd2_val, wr_reg, wr_val, wr_iswrite, ld_addr, ld_val, is_load, st_addr, st_val, is_store, s_addr, s_ts, s_val, s_kind, final_out, out_addr, prog_hash, init_hash, chi, dot_claims, d_addr, d_ts, d_val, d_kind },
	)
}

// ---- 运行结果 ----
/// M10 T1 / M12-T1：公开 Proof 形态——transcript bytes + 公开 inout（23 词恒定）+ 元数据。
pub struct VmRsProof {
	pub proof_bytes: Vec<u8>,
	pub inout_words: Vec<Word>,
	pub prog_hash: [u64; 4],
	/// M8-C：初始镜像声明哈希（声明性外部锚，与 ELF 加载结果比对；见 KNOWN_BOUNDARIES）。
	pub init_hash: [u64; 4],
	pub n: usize,
	pub t_len: usize,
	pub ts: usize,
	pub l: usize,
	pub mp: usize,
	/// M12-T1：init 模式（true = 默认全 0，电路断言 init 行 val==0）。
	pub init_zero: bool,
	pub stat: CircuitStat,
	pub sorted_ok: bool,
}

/// M10 T1：验证输出（四层标志）。
/// M12-T1 语义：c_ok = 电路（含恒等式②/uniform 钉扎/χ-dot 断言/M5 唯一性/ecall 终止）；
/// l_ok = fetch logup + fracadd + den_check + 全部 oracle relation + finish；
/// hash_ok = 程序镜像哈希对照；s_ok = χ 预检（公开 χ 词 == transcript 挑战）。
pub struct VmRsVerifyOut {
	pub c_ok: bool,
	pub l_ok: bool,
	pub hash_ok: bool,
	pub s_ok: bool,
}

pub struct VmRsRun {
	pub c_ok: bool,
	pub l_ok: bool,
	pub hash_ok: bool,
	pub s_ok: bool,
	pub stat: CircuitStat,
	pub t_len: usize,
	pub ts: usize,
	pub l: usize,
	pub inout_words: Vec<Word>,
	pub sorted_ok: bool,
}

/// 验证端篡改模式（verify 层 soundness，M7 v2 纪律；M12-T1 形态适配）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tamper {
	None,
	/// 例 1（电路层）：篡改公开输出 final_out → `c_ok == false`。
	BadFinalOut,
	/// 例 2（logup 层）：篡改分数和声明 root_den → `l_ok == false`。
	BadRootDen,
	/// 例 3（logup 层）：篡改验证端收到的 r 点 addr 开口声明 → `l_ok == false`。
	BadDenAddr,
	/// 例 4（logup 层）：篡改验证端收到的 r 点 val 开口声明 → `l_ok == false`。
	BadDenVal,
	/// 例 5（fetch 层）：验证端篡改收到的 e（looker claim）→ `l_ok == false`。
	BadFetchClaim,
	/// 例 6（程序公开性，prover 数据坏例）：执行换过编码的程序（word_overrides）
	/// 而承诺表/镜像哈希仍用原镜像 → e-relation 失配 → `l_ok == false`。
	SwapProgram,
	/// 例 7（程序公开性）：公开镜像哈希词与 expected 不符 → `hash_ok == false`。
	BadProgHash,
	/// 例 8（χ-dot 锚，原 BadBridgeWitness）：篡改公开 χ-dot 声明词 → 电路断言拒
	/// （c_ok==false）且 oracle relation 失配（l_ok==false）。
	BadDotClaim,
	/// 例 9（χ 预检）：篡改公开 χ 词 → `s_ok == false`（全部标志拒绝）。
	BadChi,
	// 例 10（BadEventRow）与例 11（DupFinal）为 prover 侧 witness 篡改，
	// 经 vmrs_prove_impl 的 mutant 钩子触发，见测试。
}

/// prove 侧 mutant 钩子（cfg-test 专用坏例；公开 API 恒 None/false）。
#[derive(Clone, Copy, Default, Debug)]
struct ProveMutants {
	/// 例 10（M12 BadEventRow 重构）：仅篡改 witness 排序流一个 PAD 行 val
	/// （oracle 列保持诚实）→ χ-dot 声明与 oracle relation 失配 → l_ok == false，
	/// c_ok 保持 true（电路自洽——这正是 witness↔oracle 绑定存在的证据）。
	bad_event_row: bool,
	/// 例 11（M5 PoC）：复制 OUT_ADDR 组的 final 行（两半侧同步，恒等式①自洽）、
	/// 输出声明改为 XOR 相消值 0 → 修复前全绿（漏洞实证）；修复后 final_unique 拒。
	dup_final: bool,
}

fn log2_ceil(x: usize) -> usize {
	(usize::BITS - (x - 1).leading_zeros()) as usize
}

/// LF → (lo, hi) u64 词（underlier 位序 = 系数序，与 bmul 的 (lo, hi) 约定一致）。
fn lf_words(x: LF) -> (u64, u64) {
	let u: u128 = u128::from(x);
	((u & (u64::MAX as u128)) as u64, (u >> 64) as u64)
}

fn words_lf(lo: u64, hi: u64) -> LF {
	LF::from(((hi as u128) << 64) | lo as u128)
}

/// 电路侧 χ-dot（模拟 build_circuit_vmrs 的累加顺序，供 prove 端声明词）。
/// 记忆列：Σ_{j<ts} χ^j·s[j] + Σ_{j<ts} χ^{ts+j}·d[j]。
fn dot128_mem(s: &[u64], d: &[u64], chi: LF, ts: usize) -> LF {
	let mut acc = LF::ZERO;
	let mut pw = LF::ONE;
	let mut pw_off = LF::ONE;
	for _ in 0..ts {
		pw_off *= chi;
	}
	for j in 0..ts {
		acc += pw * LF::from(s[j] as u128);
		pw *= chi;
		acc += pw_off * LF::from(d[j] as u128);
		pw_off *= chi;
	}
	acc
}

/// inst/pc 列：Σ_{j<2^li} χ^j·col[j]（col 长 t_len，tail = pad 常量，与电路一致）。
fn dot128_padded(col: &[u64], pad: u64, chi: LF, total: usize) -> LF {
	let mut acc = LF::ZERO;
	let mut pw = LF::ONE;
	for j in 0..total {
		let v = if j < col.len() { col[j] } else { pad };
		acc += pw * LF::from(v as u128);
		pw *= chi;
	}
	acc
}

/// 6 个 χ-dot 声明词（与电路累加顺序逐位一致）：[maddr, mval, mts, mkind, inst, pc]。
fn dot_words_from(
	sorted: &[Visit],
	side: &[Visit],
	chi: LF,
	ts: usize,
	li_pow2: usize,
	col_inst: &[u64],
	col_pc: &[u64],
	inst_pad: u64,
	pc_pad: u64,
) -> [u64; 12] {
	let col = |v: fn(&Visit) -> u64| -> (Vec<u64>, Vec<u64>) {
		(sorted.iter().map(|e| v(e)).collect(), side.iter().map(|e| v(e)).collect())
	};
	let (sa, da) = col(|e| e.addr);
	let (sv, dv) = col(|e| e.val);
	let (st, dt) = col(|e| e.ts);
	let (sk, dk) = col(|e| e.kind);
	let dots = [
		dot128_mem(&sa, &da, chi, ts),
		dot128_mem(&sv, &dv, chi, ts),
		dot128_mem(&st, &dt, chi, ts),
		dot128_mem(&sk, &dk, chi, ts),
		dot128_padded(col_inst, inst_pad, chi, li_pow2),
		dot128_padded(col_pc, pc_pad, chi, li_pow2),
	];
	let mut out = [0u64; 12];
	for (i, dv) in dots.iter().enumerate() {
		let (lo, hi) = lf_words(*dv);
		out[2 * i] = lo;
		out[2 * i + 1] = hi;
	}
	out
}

/// oracle 侧 χ-泛函系数向量（transparent buffer）：χ^j for j < support，0 尾。
/// 记忆列 support = 2ts（oracle pad 行 = 0 → 全 χ 泛函 == 电路 head 累加）；
/// inst/pc support = 2^li（pad 常量已计入电路与声明）。
fn chi_transparent_buffer(chi: LF, log_len: usize, support: usize) -> FieldVec<LP, GlobalAllocator> {
	let n = 1usize << log_len;
	let mut v = Vec::with_capacity(n);
	let mut c = LF::ONE;
	for _ in 0..support.min(n) {
		v.push(c);
		c *= chi;
	}
	v.resize(n, LF::ZERO);
	FieldVec::<LP, GlobalAllocator>::from_values(&v)
}

/// 验证端 χ-泛函 transparent：T̃(ρ) = Π_i[(1−ρ_i) + ρ_i·χ^{2^i}]（O(l)）。
/// 与 chi_transparent_buffer 的 MLE 在任意点求值一致。
fn chi_transparent_fn(chi: LF, l: usize) -> Box<dyn Fn(&[LF]) -> LF + 'static> {
	let mut chi_sq = Vec::with_capacity(l + 1);
	let mut c = chi;
	for _ in 0..l {
		chi_sq.push(c);
		c = c * c;
	}
	Box::new(move |p: &[LF]| -> LF {
		let mut acc = LF::ONE;
		for i in 0..l {
			acc = acc * ((LF::ONE - p[i]) + p[i] * chi_sq[i]);
		}
		acc
	})
}

/// Σ_{j<m} eq_r(j)（O(l)；m ≤ 2^l）。r[i] ↔ j 的 bit i（与现有显式循环同约定）。
fn eq_prefix_sum(r: &[LF], m: usize) -> LF {
	let l = r.len();
	let mut tail = LF::ZERO; // Σ_{j≥m} eq_r(j)
	let mut w = LF::ONE;
	for i in (0..l).rev() {
		let mb = (m >> i) & 1;
		if mb == 0 {
			tail += w * r[i];
		}
		w *= if mb == 1 { r[i] } else { LF::ONE - r[i] };
	}
	tail += w; // j == m 项
	LF::ONE - tail
}

/// 统一 oracle 长度 L = max(l, mp, L_FLOOR)。L_FLOOR：批量开点（组合 FRI）在过小的域上
/// 触及 GaoMateer 基底边界（上游经验边界，实测 L<11 形状 finish 失败；pad 行零值/ecall，
/// 成本可忽略）。电路（inst/pc χ-dot 循环界）与 prove/verify 端共用本函数。
pub const L_FLOOR: usize = 11;

pub fn oracle_log_len(l: usize, mp: usize) -> usize {
	l.max(mp).max(L_FLOOR)
}

/// oracle 列规格：全部 = 统一长度（prog/inst/pc 列 pad 到同长；pad 槽 = ecall / 最大槽号）。
fn oracle_specs(l: usize) -> Vec<OracleSpec> {
	vec![
		OracleSpec { log_msg_len: l, is_zk: false }, // fetch 表（pad ecall）
		OracleSpec { log_msg_len: l, is_zk: false }, // addr
		OracleSpec { log_msg_len: l, is_zk: false }, // val
		OracleSpec { log_msg_len: l, is_zk: false }, // ts
		OracleSpec { log_msg_len: l, is_zk: false }, // kind
		OracleSpec { log_msg_len: l, is_zk: false }, // inst（pad ecall）
		OracleSpec { log_msg_len: l, is_zk: false }, // pc（槽号列，pad 最大槽号）
	]
}

/// M12-T2：verifier 预处理（一次性）——建电路/CS/BaseFold 编译器 → VerifierKey。
/// Online 验证（[`vmrs_verify_online`]）不再调用 build_circuit。
pub struct VmRsVerifierKey {
	pub n: usize,
	pub t_len: usize,
	pub ts: usize,
	pub l: usize,
	/// 统一 oracle 长度 = max(l, mp)
	pub big_l: usize,
	pub mp: usize,
	pub init_zero: bool,
	pub n_touch: usize,
	word_verifier: WordVerifier<StdHashSuite>,
	specs: Vec<OracleSpec>,
	fri_params: FRIParams<LF>,
}

impl VmRsVerifierKey {
	pub fn shape(&self) -> VmRsShape {
		VmRsShape { t_len: self.t_len, ts: self.ts, init_zero: self.init_zero }
	}
}

pub fn vmrs_verifier_setup(n: usize, t_len: usize, ts: usize, init_zero: bool) -> VmRsVerifierKey {
	let mp = m_prog(n);
	let (circuit, _) = build_circuit_vmrs(t_len, ts, mp, init_zero);
	let word_verifier = WordVerifier::<StdHashSuite>::setup(circuit.constraint_system().clone(), 1)
		.expect("verifier setup");
	let shape = VmRsShape { t_len, ts, init_zero };
	let l = shape.l();
	let big_l = oracle_log_len(l, mp);
	let specs = oracle_specs(big_l);
	let merkle_scheme = BinaryMerkleTreeScheme::<LF, StdHashSuite>::new();
	let log_inv_rate = 1;
	let log_code_len = big_l + log_inv_rate;
	let arity = ConstantArityStrategy::with_optimal_arity::<LF, _>(&merkle_scheme, log_code_len).arity;
	let compiler = BaseFoldVerifierCompiler::new(
		&merkle_scheme,
		specs.clone(),
		log_inv_rate,
		calculate_n_test_queries(100, log_inv_rate),
		&ConstantArityStrategy::new(arity),
	);
	VmRsVerifierKey {
		n, t_len, ts, l, big_l, mp, init_zero,
		n_touch: shape.n_touch(),
		word_verifier,
		specs,
		fri_params: compiler.fri_params().clone(),
	}
}

/// T1 主流程（M12-T1 重构）：native 排序程序 → trace → committed 列（7 oracle）
/// → χ 挑战（承诺后采样）→ 电路（uniform + χ-dot 锚）→ 单 looker logup fetch
/// → fracaddcheck（恒等式①）→ 11 个 oracle relation → frontend prove（最后）。
///
/// M10 v2 公开 API：prove(program) -> Proof。初始内存 = 全 0（非零初始镜像用
/// [`vmrs_prove_with_init`]）。
pub fn vmrs_prove(n: usize, program: Option<&[u64]>) -> VmRsProof {
	vmrs_prove_impl(n, &[], program, false, None, OUT_ADDR, ProveMutants::default())
}

/// M8-C：非零初始镜像版 prove（ELF 加载结果作为公共初始镜像）。
/// init 绑定链：init 行 val（d 侧）＝电路内 init 行钉扎 + 恒等式① + χ-dot；
/// 外部锚 = proof.init_hash（声明性，与 ELF 加载结果比对；见 KNOWN_BOUNDARIES）。
pub fn vmrs_prove_with_init(n: usize, program: Option<&[u64]>, init_mem: &[u32], out_addr: u64) -> VmRsProof {
	vmrs_prove_impl(n, &[], program, false, Some(init_mem), out_addr, ProveMutants::default())
}

/// 私有实现（含测试钩子参数；公开签名见 [`vmrs_prove`]）。
fn vmrs_prove_impl(
	n: usize,
	word_overrides: &[(u64, u64)],
	program: Option<&[u64]>,
	bad_hash: bool,
	init_mem: Option<&[u32]>,
	out_addr: u64,
	mutants: ProveMutants,
) -> VmRsProof {
	// 程序镜像可注入（None = 内置 bubblesort(n)）；fetch 闭包统一从镜像槽读取。
	let image: Vec<u64> = program.map(|p| p.to_vec()).unwrap_or_else(|| (0..1usize << m_prog(n)).map(|slot| prog_image(slot, n)).collect());
	let image_for_table = image.clone();
	let image_for_hash = image.clone();
	let fetch_prog = move |pc: u64| image.get((pc >> 2) as usize).copied().unwrap_or(ECALL);
	// M8-C：prove 内部 trace 与排序流共享同一初始镜像（init 非零化）
	let zero_mem = vec![0u32; K];
	let mem0: &[u32] = init_mem.unwrap_or(&zero_mem);
	let trace = run_program_big(mem0, word_overrides, fetch_prog);
	assert!(trace.cycles.len() < 1_500_000);
	let t_len = trace.cycles.len();
	let (rows, side) = event_rows(&trace, init_mem);
	let sorted = build_sorted_with_final(&rows, 2 * trace.cycles.len() as u64 + 1, init_mem);
	assert_eq!(sorted.len(), side.len(), "排序流与事件侧同长");
	let ts = sorted.len();
	let shape = VmRsShape { t_len, ts, init_zero: init_mem.is_none() };
	let l = shape.l();
	// committed fetch 表（程序镜像）——跟随注入的镜像；pad 槽 = ecall；列长统一 2^L
	let mp = m_prog(n);
	// 统一 oracle 长度 L = max(l, mp, L_FLOOR)：fetch 表须容纳全部镜像槽（2^mp），记忆列
	// pad 到同长。L_FLOOR：批量开点（组合 FRI）在过小的域上触及 GaoMateer 基底边界
	// （上游经验边界，实测 L<9 形状 finish 失败；pad 行零值，成本可忽略）。
	const L_FLOOR: usize = 11;
	let big_l = l.max(mp).max(L_FLOOR);
	let l_pow2 = 1usize << big_l;
	let nrows = l_pow2;

	// M13 根因修复：协议不变量「inst pad = ECALL = prog_table[pad_slot]」必须按构造成立。
	// 注入镜像可能长于 2^mp（ELF 单 PT_LOAD 覆盖 .text+.sdata 间隙时，parse_elf32 的
	// img.text 含零填充词，覆盖 resize 的 ecall 填充——fib 形状 completeness 缺口根因）。
	let pad_slot = (1u64 << mp) - 1;
	let prog_table = {
		let mut t = image_for_table;
		t.resize(l_pow2, ECALL);
		t[pad_slot as usize] = ECALL;
		t
	};
	// 公开程序哈希 = 规范镜像（2^mp 槽）的哈希，与 oracle 列的 2^l pad 无关
	let img_hash = {
		let canonical: Vec<u64> = {
			let mut t = image_for_hash;
			t.resize(1 << mp, ECALL);
			t
		};
		let elems: Vec<LF> = canonical.iter().map(|&x| LF::from(x as u128)).collect();
		let digest = hash_serialize::<LF, binius_hash::StdDigest>(&elems).expect("hash prog image");
		let mut h = [0u64; 4];
		for (i, w) in h.iter_mut().enumerate() {
			let mut b = [0u8; 8];
			b.copy_from_slice(&digest.as_slice()[i * 8..(i + 1) * 8]);
			*w = u64::from_le_bytes(b);
		}
		h
	};

	// final 输出期望（公开输出地址 out_addr 的 final 值；默认 = OUT_ADDR = 排序后最小元素）
	let final_val = trace.final_mem[out_addr as usize] as u64;

	// M11 F3：事件绑定表——本引擎每周期恰一行事件；行号 = n_touch + t（uniform）。
	// （n_touch 由形状推导；此表仅供 native 对照，不进电路。）
	let init_hash = {
		let init_vals_w: Vec<u64> = side.iter().filter(|e| e.kind == K_INIT && e.addr != PAD_ADDR).map(|e| e.val).collect();
		let elems: Vec<LF> = init_vals_w.iter().map(|&x| LF::from(x as u128)).collect();
		let digest = hash_serialize::<LF, binius_hash::StdDigest>(&elems).expect("hash init words");
		let mut h = [0u64; 4];
		for (i, w) in h.iter_mut().enumerate() {
			let mut b = [0u8; 8];
			b.copy_from_slice(&digest.as_slice()[i * 8..(i + 1) * 8]);
			*w = u64::from_le_bytes(b);
		}
		h
	};

	// M12-T1：committed 列（事件侧 + 排序侧 ‖ pad；inst/pc 槽号列 pad 见下）
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
	// inst 列：pad = ecall；pc 列 = 槽号（pc>>2），pad = 最大槽号（fetch looker 的 index pad 一致）
	let mut col_inst = vec![ECALL; l_pow2];
	let mut col_pc = vec![pad_slot; l_pow2];
	for (t, c) in trace.cycles.iter().enumerate() {
		col_inst[t] = c.inst;
		col_pc[t] = (c.pc >> 2) & 0xffff;
	}
	let to_fb = |v: &[u64]| FieldVec::<LP, GlobalAllocator>::from_values(&v.iter().map(|&x| LF::from(x as u128)).collect::<Vec<_>>());
	let fb_prog = to_fb(&prog_table);
	let fb_addr = to_fb(&col_addr);
	let fb_val = to_fb(&col_val);
	let fb_ts = to_fb(&col_ts);
	let fb_kind = to_fb(&col_kind);
	let fb_inst = to_fb(&col_inst);
	let fb_pc = to_fb(&col_pc);

	// ---- transcript：承诺 → χ → 电路/prove → fetch/内存论证 → relations → finish ----
	let mut pt = ProverTranscript::new(StdChallenger::default());
	let merkle_scheme = BinaryMerkleTreeScheme::<LF, StdHashSuite>::new();
	let log_inv_rate = 1;
	let log_code_len = big_l + log_inv_rate;
	let arity = ConstantArityStrategy::with_optimal_arity::<LF, _>(&merkle_scheme, log_code_len).arity;
	let verifier_compiler = BaseFoldVerifierCompiler::new(
		&merkle_scheme,
		oracle_specs(big_l),
		log_inv_rate,
		calculate_n_test_queries(100, log_inv_rate),
		&ConstantArityStrategy::new(arity),
	);
	let prover_compiler = BaseFoldProverCompiler::from_verifier_compiler(&verifier_compiler, NeighborsLastMultiThread::new(GaoMateerPreExpanded::<LF>::generate(log_code_len), 1));
	let merkle_chan = ProverMerkleTranscriptChannel::<&mut ProverTranscript<StdChallenger>, StdChallenger, LF, StdHashSuite>::new(&mut pt);
	let mut chan = prover_compiler.create_channel(merkle_chan, StdRng::from_seed([0u8; 32]), GlobalAllocator);
	// 承诺先于 χ（soundness：χ 不可预测于 oracle 列数据之前）
	let o_prog = chan.send_oracle(fb_prog.as_view());
	let o_addr = chan.send_oracle(fb_addr.as_view());
	let o_val = chan.send_oracle(fb_val.as_view());
	let o_ts = chan.send_oracle(fb_ts.as_view());
	let o_kind = chan.send_oracle(fb_kind.as_view());
	let o_inst = chan.send_oracle(fb_inst.as_view());
	let o_pc = chan.send_oracle(fb_pc.as_view());
	let chi: LF = chan.sample();

	// mutant witness 排序流（先于声明词计算：声明 = witness 侧 χ-dot）
	// 例 10：仅篡改 witness 排序流一个 PAD 行 val（oracle 列保持诚实）
	let mut sorted_w: Vec<Visit> = sorted.clone();
	let mut final_out_word = final_val;
	if mutants.bad_event_row {
		if let Some(row) = sorted_w.iter().position(|e| e.addr == PAD_ADDR && e.kind == K_INIT) {
			sorted_w[row].val ^= 1;
			eprintln!("[mutant] bad_event_row: 篡改 witness PAD 行 {row} val");
		}
	}
	if mutants.dup_final {
		// M5 PoC：把 OUT_ADDR 组最后一个写行改为第二条 final（②仍满足），输出声明 = XOR 相消 0。
		// 拒绝层：c_ok（final_unique 唯一性断言，native 期即拒）+ l_ok（χ/①与诚实 oracle 失配）。
		let fa = sorted_w.iter().position(|e| e.addr == OUT_ADDR && e.kind == K_FINAL).expect("OUT_ADDR final row");
		let wt = sorted_w[..fa].iter().rposition(|e| e.addr == OUT_ADDR && e.kind == K_WRITE).expect("OUT_ADDR write row");
		sorted_w[wt].kind = K_FINAL;
		final_out_word = 0;
		eprintln!("[mutant] dup_final: OUT_ADDR 写行 {wt} 改 kind=FINAL，输出声明改为 0");
	}

	// 公开 χ-dot 声明（与电路累加顺序一致；mutant 时取 witness 侧）
	let t_build = std::time::Instant::now();
	let (circuit, iref) = build_circuit_vmrs(t_len, ts, mp, shape.init_zero);
	let stat = CircuitStat::collect(&circuit);
	eprintln!("[phase] build_circuit+stat: {:?}", t_build.elapsed());
	let dot_words: [u64; 12] = dot_words_from(&sorted_w, &side, chi, ts, l_pow2, &col_inst, &col_pc, ECALL, pad_slot);

	// ---- witness 填充 ----
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
		// pinning 断言是无条件的：非访存周期也必须填 native 复算值。
		w[iref.ld_addr[t]] = Word(c.mem_addr as u64);
		w[iref.ld_val[t]] = Word(c.load.as_ref().map(|x| x.val as u64).unwrap_or(0));
		w[iref.is_load[t]] = Word(if c.load.is_some() { 1 } else { 0 });
		w[iref.st_addr[t]] = Word(c.store.as_ref().map(|x| x.addr as u64).unwrap_or(c.mem_addr as u64));
		w[iref.st_val[t]] = Word(c.reads[1].val as u64);
		w[iref.is_store[t]] = Word(if c.store.is_some() { 1 } else { 0 });
	}
	for j in 0..ts {
		w[iref.s_addr[j]] = Word(sorted_w[j].addr);
		w[iref.s_ts[j]] = Word(sorted_w[j].ts);
		w[iref.s_val[j]] = Word(sorted_w[j].val);
		w[iref.s_kind[j]] = Word(sorted_w[j].kind);
		w[iref.d_addr[j]] = Word(side[j].addr);
		w[iref.d_ts[j]] = Word(side[j].ts);
		w[iref.d_val[j]] = Word(side[j].val);
		w[iref.d_kind[j]] = Word(side[j].kind);
	}
	// 公开词（inout）
	let mut hash_words = img_hash;
	if bad_hash {
		hash_words[0] ^= 1;
	}
	for i in 0..4 {
		w[iref.prog_hash[i]] = Word(hash_words[i]);
		w[iref.init_hash[i]] = Word(init_hash[i]);
	}
	// mutant 例 11 的输出声明（XOR 相消值 0）已在上面的 final_out_word 处理
	w[iref.final_out] = Word(final_out_word);
	w[iref.out_addr] = Word(out_addr);
	let (chi_lo, chi_hi) = lf_words(chi);
	w[iref.chi[0]] = Word(chi_lo);
	w[iref.chi[1]] = Word(chi_hi);
	for (i, dw) in dot_words.iter().enumerate() {
		w[iref.dot_claims[i / 2][i % 2]] = Word(*dw);
	}
	let t_fill = std::time::Instant::now();
	circuit.populate_wire_witness(&mut w).expect("witness fill");
	eprintln!("[phase] witness_fill: {:?}", t_fill.elapsed());
	let witness_vec = w.into_value_vec();
	cs.verify(&witness_vec).expect("native verify");
	let inout_words = witness_vec.inout().to_vec();

	// fetch 单 looker（M12-T1）：全点 looker，claim e 绑定 inst-oracle relation
	let r_fetch: Vec<LF> = (0..big_l).map(|_| chan.sample()).collect();
	let eq_rf = eq_ind_partial_eval_in::<GlobalAllocator, LP>(&GlobalAllocator, &r_fetch);
	let eq_rf_vals: Vec<LF> = eq_rf.as_view().iter_scalars().collect();
	// looker index 列 = 槽号（pc>>2），pad = 最大槽号；prog_table pad 到 2^l（全 ecall 尾）
	let mut look_idx: Vec<usize> = Vec::with_capacity(l_pow2);
	for j in 0..l_pow2 {
		let slot = if j < t_len { col_pc[j] } else { pad_slot };
		look_idx.push(slot as usize);
	}
	let e_claim: LF = {
		let mut acc = LF::ZERO;
		for j in 0..l_pow2 {
			acc += eq_rf_vals[j] * LF::from(prog_table[look_idx[j]] as u128);
		}
		acc
	};
	chan.send_one(e_claim);

	// indexed logup*：单个全点 looker（eval_point = r_fetch）
	let gamma: LF = chan.sample();
	let lookers = vec![LogupLooker { index: &look_idx, eval_point: &r_fetch, eval_claim: e_claim }];
	let logup_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
		&GlobalAllocator,
		gamma,
		vec![ProverTableLookup { table: fb_prog.as_view(), lookers }],
		&mut chan,
	);

	let rho: LF = chan.sample();
	let c: LF = chan.sample();

	// 恒等式①：fracaddcheck（num 全 1，den = c + f）；GKR 变量数 = big_l
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
	let (frac, root) = FracAddCircuit::build(big_l, &GlobalAllocator, Fraction::new(FieldVec::<LP, GlobalAllocator>::from_values(&num), FieldVec::<LP, GlobalAllocator>::from_values(&den)));
	let root_num = root.num.get(0);
	let root_den = root.den.get(0);
	assert_eq!(root_num, LF::ZERO, "恒等式①根分子必须为零");
	chan.send_one(root_den);
	let final_claim = frac.prove(FracAddEvalClaim { num_eval: LF::ZERO, den_eval: root_den, point: vec![] }, &mut chan);
	let r = final_claim.point.clone();
	let eq_r = eq_ind_partial_eval_in::<GlobalAllocator, LP>(&GlobalAllocator, &r);
	let eq_vals: Vec<LF> = eq_r.as_view().iter_scalars().collect();
	let dot = |col: &[u64]| -> LF {
		let mut s = LF::ZERO;
		for (j, &x) in col.iter().enumerate() { s += eq_vals[j] * LF::from(x as u128); }
		s
	};
	// r 点开口声明（验证端 den_check 数据源；oracle relation 绑定）
	let addr_r = dot(&col_addr);
	let val_r = dot(&col_val);
	let ts_r = dot(&col_ts);
	let kind_r = dot(&col_kind);
	chan.send_one(addr_r);
	chan.send_one(val_r);
	chan.send_one(ts_r);
	chan.send_one(kind_r);

	// ---- oracle relations（11 个）----
	// fetch 表 claim 绑定承诺（M8-B T0 形态不变）
	let tep = logup_out.table_eval_point.clone();
	let prog_claim = logup_out.tables[0].eval_claim;
	let eq_tep = eq_ind_partial_eval_in::<GlobalAllocator, LP>(&GlobalAllocator, &tep);
	chan.prove_oracle_relation(o_prog, eq_tep, prog_claim);
	// 记忆 4 列：r 点开口 + χ 泛函（声明词）
	chan.prove_oracle_relation(o_addr, eq_r.clone(), addr_r);
	chan.prove_oracle_relation(o_val, eq_r.clone(), val_r);
	chan.prove_oracle_relation(o_ts, eq_r.clone(), ts_r);
	chan.prove_oracle_relation(o_kind, eq_r.clone(), kind_r);
	// 记忆列 transparent = 全 χ-幂向量（2^l 行）：oracle pad 行为 0 → 泛函值 == 电路 head 累加；
	// 两侧 transparent 必须逐位一致（verifier 侧为同一向量的 O(l) 乘积式 MLE）。
	chan.prove_oracle_relation(o_addr, chi_transparent_buffer(chi, big_l, nrows),
		dot128_mem(&sorted_w.iter().map(|e| e.addr).collect::<Vec<_>>(), &side.iter().map(|e| e.addr).collect::<Vec<_>>(), chi, ts));
	chan.prove_oracle_relation(o_val, chi_transparent_buffer(chi, big_l, nrows),
		dot128_mem(&sorted_w.iter().map(|e| e.val).collect::<Vec<_>>(), &side.iter().map(|e| e.val).collect::<Vec<_>>(), chi, ts));
	chan.prove_oracle_relation(o_ts, chi_transparent_buffer(chi, big_l, nrows),
		dot128_mem(&sorted_w.iter().map(|e| e.ts).collect::<Vec<_>>(), &side.iter().map(|e| e.ts).collect::<Vec<_>>(), chi, ts));
	chan.prove_oracle_relation(o_kind, chi_transparent_buffer(chi, big_l, nrows),
		dot128_mem(&sorted_w.iter().map(|e| e.kind).collect::<Vec<_>>(), &side.iter().map(|e| e.kind).collect::<Vec<_>>(), chi, ts));
	// inst：r_fetch 点 e-claim + χ 泛函（声明词；oracle pad = ecall，与声明一致）
	chan.prove_oracle_relation(o_inst, eq_rf, e_claim);
	chan.prove_oracle_relation(o_inst, chi_transparent_buffer(chi, big_l, l_pow2),
		dot128_padded(&col_inst, ECALL, chi, l_pow2));
	// pc：z 点 index-claim + χ 泛函（oracle pad = 最大槽号）
	let z_point = logup_out.index_eval_point.clone();
	let pc_index_claim = logup_out.tables[0].index_eval_claims[0];
	let eq_z = eq_ind_partial_eval_in::<GlobalAllocator, LP>(&GlobalAllocator, &z_point);
	chan.prove_oracle_relation(o_pc, eq_z, pc_index_claim);
	chan.prove_oracle_relation(o_pc, chi_transparent_buffer(chi, big_l, l_pow2),
		dot128_padded(&col_pc, pad_slot, chi, l_pow2));
	// finalize + finish（BaseFold 批量开点）
	chan.finalize_oracle(o_prog, fb_prog);
	chan.finalize_oracle(o_addr, fb_addr);
	chan.finalize_oracle(o_val, fb_val);
	chan.finalize_oracle(o_ts, fb_ts);
	chan.finalize_oracle(o_kind, fb_kind);
	chan.finalize_oracle(o_inst, fb_inst);
	chan.finalize_oracle(o_pc, fb_pc);
	chan.finish();
	// frontend prove（M12-T1 顺序：BaseFold 全部先于 frontend；χ 已在填充前采样）
	let verifier_for_prover = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<LP, StdHashSuite>::setup(verifier_for_prover).expect("prover setup");
	let t_fprove = std::time::Instant::now();
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");
	eprintln!("[phase] frontend_prove: {:?}", t_fprove.elapsed());

	let sorted_ok = program.is_some()
		|| (0..n - 1).all(|i| trace.final_mem[(BASE >> 2) as usize + i] <= trace.final_mem[(BASE >> 2) as usize + i + 1]);
	if !sorted_ok {
		let b = (BASE >> 2) as usize;
		eprintln!("DBG n={} T={} head={:x?} cycles_tail={:x?}", n, trace.cycles.len(),
			&trace.final_mem[b..b + n], trace.cycles.last().map(|c| c.pc));
	}
	let proof_bytes = pt.finalize();
	eprintln!("[phase] proof_bytes={}", proof_bytes.len());
	eprintln!("[phase] public_io_words={} (IO_LEN={IO_LEN})", inout_words.len());

	VmRsProof { proof_bytes, inout_words, prog_hash: hash_words, init_hash, n, t_len, ts, l, mp, init_zero: shape.init_zero, stat, sorted_ok }
}

/// M10 v2 / M12-T2 兼容入口：setup + online 两段串跑（每次调用重建 VerifierKey）。
/// 热路径请用 [`vmrs_verifier_setup`] + [`vmrs_verify_online`]（同 key 复用）。
pub fn vmrs_verify(proof: &VmRsProof, expected_hash: Option<[u64; 4]>) -> VmRsVerifyOut {
	let key = vmrs_verifier_setup(proof.n, proof.t_len, proof.ts, proof.init_zero);
	vmrs_verify_impl(&key, proof, Tamper::None, expected_hash)
}

/// M12-T2：在线验证——不重建电路（电路/CS/BaseFold 形状全部来自预处理 VerifierKey）。
pub fn vmrs_verify_online(
	key: &VmRsVerifierKey,
	proof: &VmRsProof,
	expected_hash: Option<[u64; 4]>,
) -> VmRsVerifyOut {
	vmrs_verify_impl(key, proof, Tamper::None, expected_hash)
}

/// 私有实现（含 tamper 钩子；公开签名见 [`vmrs_verify`] / [`vmrs_verify_online`]）。
fn vmrs_verify_impl(
	key: &VmRsVerifierKey,
	proof: &VmRsProof,
	tamper: Tamper,
	expected_hash: Option<[u64; 4]>,
) -> VmRsVerifyOut {
	let VmRsProof { proof_bytes, inout_words, prog_hash: hash_words, .. } = proof;
	let ts = key.ts;
	let mut inout_verify: Vec<Word> = inout_words.clone();
	// 公开输入形状预检：M12-T1 公开输入必须恒为 IO_LEN 词（succinct 的结构前提）。
	if inout_verify.len() != IO_LEN {
		return VmRsVerifyOut { c_ok: false, l_ok: false, hash_ok: false, s_ok: false };
	}
	// 公开哈希对照（M8-B T0）：Proof 携带的镜像哈希词 vs 验证端期望
	let hash_ok = match expected_hash {
		Some(exp) => *hash_words == exp,
		None => true,
	};
	if tamper == Tamper::BadFinalOut {
		inout_verify[IO_FINAL_OUT].0 ^= 1;
	}
	if tamper == Tamper::BadDotClaim {
		inout_verify[IO_DOT_MVAL].0 ^= 1;
	}
	if tamper == Tamper::BadChi {
		inout_verify[IO_CHI].0 ^= 1;
	}

	let mut vt = VerifierTranscript::new(StdChallenger::default(), proof_bytes.clone());
	let merkle_veri = VerifierMerkleTranscriptChannel::<&mut VerifierTranscript<StdChallenger>, StdChallenger, LF, StdHashSuite>::new(&mut vt);
	let mut vchan = BaseFoldVerifierChannel::new(merkle_veri, &key.specs, &key.fri_params);
	let v_o_prog = vchan.recv_oracle(key.big_l, false).unwrap();
	let v_o_addr = vchan.recv_oracle(key.big_l, false).unwrap();
	let v_o_val = vchan.recv_oracle(key.big_l, false).unwrap();
	let v_o_ts = vchan.recv_oracle(key.big_l, false).unwrap();
	let v_o_kind = vchan.recv_oracle(key.big_l, false).unwrap();
	let v_o_inst = vchan.recv_oracle(key.big_l, false).unwrap();
	let v_o_pc = vchan.recv_oracle(key.big_l, false).unwrap();
	let chi_v: LF = vchan.sample();
	// 公开 χ 词 == transcript 挑战（χ 在 oracle 承诺之后采样，两侧同序）
	let s_ok = words_lf(inout_verify[IO_CHI].0, inout_verify[IO_CHI + 1].0) == chi_v;

	// fetch 单 looker：claim e 来自 channel（经 oracle relation + logup 双重绑定）
	let r_fetch: Vec<LF> = (0..key.big_l).map(|_| vchan.sample()).collect();
	let mut e_for_tables = match vchan.recv_one() {
		Ok(e) => e,
		Err(_) => return VmRsVerifyOut { c_ok: false, l_ok: false, hash_ok, s_ok },
	};
	if tamper == Tamper::BadFetchClaim {
		e_for_tables += LF::ONE;
	}

	// indexed logup*：单个全点 looker。归约失败也必须继续消费 transcript（relations
	// 以哑点排队，finish 统一拒绝）——保证 verify 全程无 panic 单独成立。
	let vgamma: LF = vchan.sample();
	let v_tables = vec![binius_ip::logup_star::TableLookup {
		n_vars: key.big_l, // fetch 表列长统一 2^big_l（prog pad ecall）
		lookers: vec![LookerClaim { eval_point: &r_fetch, eval_claim: e_for_tables }],
	}];
	let mut fetch_ok = true;
	let (z_point, tep, ic, tclaim) = match binius_ip::logup_star::verify_reduction::<LF, _>(&vgamma, v_tables.clone(), &mut vchan) {
		Ok(o) => (o.index_eval_point.clone(), o.table_eval_point.clone(), o.tables[0].index_eval_claims[0], o.tables[0].eval_claim),
		Err(_) => { fetch_ok = false; (vec![LF::ZERO; key.big_l], vec![LF::ZERO; key.big_l], LF::ZERO, LF::ZERO) }
	};
	// F2 位置绑定（M12 形态）：index claim 须 == pc-oracle 在 z 点的开口；
	// inst-oracle 在 r_fetch 点的开口 == e；prog 表 claim 绑定承诺（M8-B T0 形态不变）。
	let rf = r_fetch.clone();
	let _ = vchan.verify_oracle_relation(v_o_pc, Box::new(move |p: &[LF]| eq_ind(&z_point, p)), ic);
	let _ = vchan.verify_oracle_relation(v_o_inst, Box::new(move |p: &[LF]| eq_ind(&rf, p)), e_for_tables);
	let _ = vchan.verify_oracle_relation(v_o_prog, Box::new(move |p: &[LF]| eq_ind(&tep, p)), tclaim);

	let vrho: LF = vchan.sample();
	let vc: LF = vchan.sample();
	let mut vroot_den = vchan.recv_one().unwrap_or(LF::ZERO);
	if tamper == Tamper::BadRootDen {
		vroot_den += LF::ONE;
	}
	let mut mem_ok = true;
	let (r, num_eval, den_eval) = match fracaddcheck::verify::<LF, _>(key.big_l, FracAddEvalClaim { num_eval: LF::ZERO, den_eval: vroot_den, point: vec![] }, &mut vchan) {
		Ok(c) => (c.point.clone(), c.num_eval, c.den_eval),
		Err(_) => { mem_ok = false; (vec![LF::ZERO; key.big_l], LF::ZERO, LF::ZERO) }
	};
	// r 点开口声明（M7 v2 形态：验证端数据源 = 关系绑定的开口，不再线性重算）
	let mut addr_r = vchan.recv_one().unwrap_or(LF::ZERO);
	let mut val_r = vchan.recv_one().unwrap_or(LF::ZERO);
	let ts_r = vchan.recv_one().unwrap_or(LF::ZERO);
	let kind_r = vchan.recv_one().unwrap_or(LF::ZERO);
	if tamper == Tamper::BadDenAddr { addr_r += LF::ONE; }
	if tamper == Tamper::BadDenVal { val_r += LF::ONE; }
	// den_check：Σ_{j<2ts} eq_r(j) 用 O(l) 尾和公式（替代旧的 O(T) 显式循环）
	let sum_eq = eq_prefix_sum(&r, 2 * ts);
	let num_ok = num_eval == sum_eq;
	let rr2 = vrho * vrho;
	let rr3 = rr2 * vrho;
	let den_check = vc * sum_eq + addr_r + vrho * val_r + rr2 * ts_r + rr3 * kind_r + (LF::ONE - sum_eq);
	let den_ok = num_ok && den_eval == den_check;
	mem_ok = mem_ok && den_ok;

	// 记忆 4 列 r 点开口 + χ 泛函 relation；inst/pc 的 fetch relation 已排队。
	// 全部无条件排队（transparent 与 prove 端逐位一致；失败统一在 finish 显现）。
	let _ = vchan.verify_oracle_relation(v_o_addr, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), addr_r);
	let _ = vchan.verify_oracle_relation(v_o_val, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), val_r);
	let _ = vchan.verify_oracle_relation(v_o_ts, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), ts_r);
	let _ = vchan.verify_oracle_relation(v_o_kind, Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }), kind_r);
	let claim_lf = |base: usize| words_lf(inout_verify[base].0, inout_verify[base + 1].0);
	for (o, base, ll) in [
		(v_o_addr, IO_DOT_MADDR, key.big_l),
		(v_o_val, IO_DOT_MVAL, key.big_l),
		(v_o_ts, IO_DOT_MTS, key.big_l),
		(v_o_kind, IO_DOT_MKIND, key.big_l),
		(v_o_inst, IO_DOT_INST, key.big_l),
		(v_o_pc, IO_DOT_PC, key.big_l),
	] {
		let cl = claim_lf(base);
		let _ = vchan.verify_oracle_relation(o, chi_transparent_fn(chi_v, ll), cl);
	}

	// finish（BaseFold 批量开点）→ frontend verify（顺序与 prove 严格同序）
	let t_verify = std::time::Instant::now();
	let finish_ok = match vchan.finish() { Ok(_) => true, Err(e) => { eprintln!("DBG finish: {e:?}"); false } };
	let c_ok = key.word_verifier.verify(&inout_verify, &mut vt).is_ok();
	eprintln!("[phase] online_verify(fetch logup + fracadd + relations + finish + frontend): {:?}（无 build_circuit）", t_verify.elapsed());

	VmRsVerifyOut { c_ok, l_ok: fetch_ok && mem_ok && finish_ok, hash_ok, s_ok }
}
/// M10 T1 兼容包装（仅测试使用）：prove + verify 两段串跑，逐数字一致。
#[cfg(test)]
pub(crate) fn run_vmrs(n: usize, tamper: Tamper, expected_hash: Option<[u64; 4]>, word_overrides: &[(u64, u64)]) -> VmRsRun {
	let proof = vmrs_prove_impl(n, word_overrides, None, tamper == Tamper::BadProgHash, None, OUT_ADDR, ProveMutants::default());
	let sorted_ok = proof.sorted_ok;
	let key = vmrs_verifier_setup(proof.n, proof.t_len, proof.ts, proof.init_zero);
	let v = vmrs_verify_impl(&key, &proof, tamper, expected_hash);
	VmRsRun { c_ok: v.c_ok, l_ok: v.l_ok, hash_ok: v.hash_ok, s_ok: v.s_ok, stat: proof.stat, t_len: proof.t_len, ts: proof.ts, l: proof.l, inout_words: proof.inout_words, sorted_ok }
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Instant;

	fn show(run: &VmRsRun, label: &str) {
		let s = &run.stat;
		println!(
			"{label}: T={} ts={} l={} gates={} zero/and/bmul={}/{}/{} io_words={} c_ok={} l_ok={} hash_ok={} s_ok={}",
			run.t_len, run.ts, run.l, s.n_gates, s.n_zero_constraints, s.n_and_constraints,
			s.n_bmul_constraints, run.inout_words.len(), run.c_ok, run.l_ok, run.hash_ok, run.s_ok
		);
	}

	#[test]
	fn vm_ram_sort_honest() {
		let t0 = Instant::now();
		let run = run_vmrs(16, Tamper::None, Some(prog_image_hash(16)), &[]);
		let dt = t0.elapsed().as_secs_f32();
		assert!(run.sorted_ok, "native 排序必须成功");
		assert!(run.c_ok, "诚实路径电路必须通过");
		assert!(run.l_ok, "诚实路径 fracaddcheck/绑定必须通过");
		assert!(run.hash_ok, "公开镜像哈希必须对照通过");
		assert!(run.s_ok, "χ 预检必须通过");
		assert_eq!(run.inout_words.len(), IO_LEN, "M12-T1 硬指标：公开输入词数恒定");
		show(&run, &format!("honest ({dt:.1}s)"));
	}

	/// M8-B T1 例 8 → M12 形态：篡改公开 χ-dot 声明词 → 电路断言拒 + relation 失配。
	#[test]
	fn vm_ram_sort_soundness_bad_dot_claim() {
		let run = run_vmrs(16, Tamper::BadDotClaim, Some(prog_image_hash(16)), &[]);
		assert!(!run.c_ok, "公开声明词被电路 assert 绑定 → 篡改即拒");
		assert!(!run.l_ok, "χ-oracle relation 必须失配拒绝（l_ok==false）");
		show(&run, "sound 8/bad-dot-claim (chi-dot anchor)");
	}

	/// 例 9：篡改公开 χ 词 → 预检拒（s_ok == false，全部拒绝）。
	#[test]
	fn vm_ram_sort_soundness_bad_chi() {
		let run = run_vmrs(16, Tamper::BadChi, Some(prog_image_hash(16)), &[]);
		assert!(!run.s_ok, "公开 χ 词必须 == transcript 挑战");
		assert!(!run.c_ok, "χ 不符 → 电路 dot 断言失配拒绝（证明被整体拒绝）");
		show(&run, "sound 9/bad-chi (transcript challenge check)");
	}

	/// 缩放点（任务书 §2.5）：N=32，供报告成本曲线 + 公开词数恒定实测；默认忽略。
	#[test]
	#[ignore]
	fn vm_ram_sort_scale32() {
		let t0 = Instant::now();
		let run = run_vmrs(32, Tamper::None, Some(prog_image_hash(32)), &[]);
		let dt = t0.elapsed().as_secs_f32();
		assert_eq!(run.inout_words.len(), IO_LEN, "N=32 公开词数必须恒定");
		show(&run, &format!("scale N=32 ({dt:.1}s)"));
		assert!(run.sorted_ok && run.c_ok && run.l_ok && run.hash_ok && run.s_ok);
	}

	/// M9 T0/T1：N=64（M8-A 时电路构建 OOM/SIGKILL 的规模）。
	#[test]
	#[ignore]
	fn vm_ram_sort_scale64() {
		let t0 = Instant::now();
		let run = run_vmrs(64, Tamper::None, Some(prog_image_hash(64)), &[]);
		let dt = t0.elapsed().as_secs_f32();
		assert_eq!(run.inout_words.len(), IO_LEN, "N=64 公开词数必须恒定");
		show(&run, &format!("scale N=64 ({dt:.1}s)"));
		assert!(run.sorted_ok && run.c_ok && run.l_ok && run.hash_ok && run.s_ok);
	}

	/// M10 T1：公共 API 端到端 + M12-T2 VerifierKey 复用（同 key 连验两个不同 proof，
	/// online 路径不调用 build_circuit）。
	#[test]
	fn vm_ram_sort_api_end_to_end() {
		// 内置程序：prove → bytes → verify 全绿
		let proof = vmrs_prove(16, None);
		assert!(proof.sorted_ok);
		let h = proof.prog_hash;
		let key = vmrs_verifier_setup(proof.n, proof.t_len, proof.ts, proof.init_zero);
		let v = vmrs_verify_online(&key, &proof, Some(h));
		assert!(v.c_ok && v.l_ok && v.hash_ok && v.s_ok, "API 端到端（bytes 重建）必须全绿");
		// 同一 VerifierKey 连验第二个 proof（注入镜像）
		let img: Vec<u64> = prog_col(16);
		let proof2 = vmrs_prove(16, Some(&img));
		let v2 = vmrs_verify_online(&key, &proof2, Some(proof2.prog_hash));
		assert!(v2.c_ok && v2.l_ok && v2.hash_ok && v2.s_ok, "同 key 第二 proof 必须全绿（VerifierKey 复用）");
		// 哈希不符拒
		let v3 = vmrs_verify_online(&key, &proof, Some([1, 2, 3, 4]));
		assert!(!v3.hash_ok, "API verify 的哈希对照必须拒绝不符值");
	}

	/// M8-C T4：真实编译 C 程序端到端——ELF → 加载 → tracer → prove → verify + 独立对拍。
	#[test]
	fn vm_ram_sort_elf_bubble16_e2e() {
		use crate::vm32::elf::{elf_fetch, parse_elf32};
		let elf = include_bytes!("../../testdata/bubble16.elf");
		let img = parse_elf32(elf).expect("parse bubble16.elf");
		assert_eq!(img.entry, 0);
		// 初始内存：LOAD 段词（.data 含 16 元素输入数组）
		let mut init_mem = vec![0u32; K];
		for (k, w) in &img.init_words {
			init_mem[*k] = *w;
		}
		// 定位输入数组（.data 首个连续 16 词块 = elf 声明的 0x1054 >> 2）
		let data_base = 0x1054usize >> 2;
		let input: Vec<u32> = (0..16).map(|i| init_mem[data_base + i]).collect();
		assert_eq!(input[0], 0x8000_0000, "输入数组必须含边界值（.data 加载正确）");

		// tracer 执行（字节地址语义、ecall halt）
		let trace = run_program_big(&init_mem, &[], elf_fetch(&img));
		assert!(trace.cycles.len() < 100_000);
		// 独立参考对拍：Rust 直接排序 + flag 字
		let mut want = input.clone();
		want.sort_unstable();
		assert_eq!(&trace.final_mem[data_base..data_base + 16], &want[..], "排序输出与独立参考一致");
		assert_eq!(trace.final_mem[0x400 >> 2], 0xC0DE_600D, "完成标志字");

		// 端到端 prove → verify（公共 API，非零初始镜像）
		let text: Vec<u64> = img.text.iter().map(|&w| w as u64).collect();
		let proof = vmrs_prove_with_init(16, Some(&text), &init_mem, 0x1054 >> 2);
		assert!(proof.sorted_ok);
		assert!(!proof.init_zero, "ELF 模式 init_zero == false");
		let v = vmrs_verify(&proof, Some(proof.prog_hash));
		assert!(v.c_ok && v.l_ok && v.hash_ok && v.s_ok, "C 冒泡端到端 prove→verify 必须全绿");
		// 输出对拍：公开 final_out == 排序后最小元素（独立参考）
		assert_eq!(proof.inout_words[IO_FINAL_OUT].0, want[0] as u64, "公开输出 == 独立参考最小元素");
		println!(
			"elf bubble16: cycles={} ts={} l={} gates={} io_words={} init_hash={:x?}",
			trace.cycles.len(), proof.ts, proof.l, proof.stat.n_gates, proof.inout_words.len(), proof.init_hash
		);

		// soundness：换一个不同编译产物（fib）的哈希对照 → 拒
		let elf_fib = include_bytes!("../../testdata/fib.elf");
		let img_fib = parse_elf32(elf_fib).expect("parse fib.elf");
		let text_fib: Vec<u64> = img_fib.text.iter().map(|&w| w as u64).collect();
		// fib 的输出 = 完成标志字（byte 0x400 → word 0x100）；数组地址 fib 不触碰。
		let proof_fib = vmrs_prove_with_init(16, Some(&text_fib), &init_mem, 0x400 >> 2);
		let v_wrong = vmrs_verify(&proof_fib, Some(proof.prog_hash));
		assert!(!v_wrong.hash_ok, "不同编译产物的哈希对照必须拒绝");
		// M13 修复后：fib 端到端恢复（根因 = pad 槽 ECALL 不变量被长镜像破坏，见 M13_REPORT）
		let v_right = vmrs_verify(&proof_fib, Some(proof_fib.prog_hash));
		assert!(v_right.c_ok && v_right.l_ok, "fib 端到端（同 init）必须通过");

		// soundness（M12 形态）：篡改公开 χ-dot 声明词 → 电路断言拒
		let hash_main = proof.prog_hash;
		let mut bad = proof;
		bad.inout_words[IO_DOT_MVAL].0 ^= 1;
		let v_bad = vmrs_verify(&bad, Some(hash_main));
		assert!(!v_bad.c_ok && !v_bad.l_ok, "篡改 χ-dot 声明必须被拒绝");
	}

	/// M12 例 10（BadEventRow 重构）：prove 端仅篡改 witness 排序流一个 PAD 行 val
	/// （oracle 列诚实）→ 电路自洽（c_ok==true——证明 witness 不再经 inout 公开），
	/// 但 χ-dot 声明与 oracle relation 失配 → l_ok == false（witness↔oracle 绑定成立的直接证据）。
	#[test]
	fn vm_ram_sort_soundness_bad_event_row() {
		let proof = vmrs_prove_impl(16, &[], None, false, None, OUT_ADDR, ProveMutants { bad_event_row: true, dup_final: false });
		let key = vmrs_verifier_setup(proof.n, proof.t_len, proof.ts, proof.init_zero);
		let v = vmrs_verify_impl(&key, &proof, Tamper::None, Some(proof.prog_hash));
		assert!(
			!v.l_ok,
			"M12：篡改 witness 事件行必须被 χ-oracle relation 拒绝（修复前 witness↔oracle 无绑定=漏洞，M8 报告间隙条目）"
		);
		// 注：M12 顺序下 finish 失败使 frontend 段失配，c_ok 不再可观测（单流 transcript）。
		println!("sound 10/bad-event-row: c_ok={} l_ok={} hash_ok={} s_ok={}", v.c_ok, v.l_ok, v.hash_ok, v.s_ok);
	}

	/// M12 例 11（M5 PoC）：OUT_ADDR 组写行改第二条 final + 输出声明 = XOR 相消 0。
	/// 修复前（无 final_unique）：②允许（ts 严增/val 一致）、输出断言 XOR 相消为 0 通过 = 漏洞形态。
	/// 修复后：final_unique（hit 计数==1）在 prove 期 native 断言即拒（catch_unwind 捕获）。
	#[test]
	fn vm_ram_sort_soundness_dup_final() {
		let result = std::panic::catch_unwind(|| {
			vmrs_prove_impl(16, &[], None, false, None, OUT_ADDR, ProveMutants { bad_event_row: false, dup_final: true })
		});
		assert!(result.is_err(), "M5：重复 final 行 + XOR 相消必须被 final_unique 断言拒绝（修复前此形态全绿=漏洞实证）");
		println!("sound 11/dup-final: prove 期 native 断言拒绝（final_unique）");
	}

	/// M11 F2 PoC：周期 0 执行表中另一槽位（slot 5）的指令字——成员关系满足、位置不符。
	/// 修复前：fetch 只证成员 → 全绿（漏洞实证）；修复后：位置绑定链 → 拒（l_ok=false）。
	#[test]
	fn vm_ram_sort_soundness_fetch_position() {
		let slot5_word = prog_image(5, 16);
		let run = run_vmrs(16, Tamper::SwapProgram, Some(prog_image_hash(16)), &[(0x0, slot5_word)]);
		assert!(
			!run.l_ok,
			"M11 F2：执行另一槽位的指令必须被位置绑定拒绝（修复前全绿=漏洞实证）"
		);
		show(&run, "sound fetch-position (index binding)");
	}

	/// M8-B T0 例 5：验证端篡改 e（looker claim）→ logup 归约/relation 拒。
	#[test]
	fn vm_ram_sort_soundness_bad_fetch_claim() {
		let run = run_vmrs(16, Tamper::BadFetchClaim, Some(prog_image_hash(16)), &[]);
		assert!(!run.l_ok, "篡改取指 claim 必须被 logup 归约拒绝（l_ok==false）");
		show(&run, "sound bad-fetch-claim (fetch layer)");
	}

	/// M8-B T0 例 6：执行换过编码的程序、承诺表/镜像哈希仍用原镜像 → e-relation 失配。
	#[test]
	fn vm_ram_sort_soundness_swap_program() {
		// slot 0：lui x15, hi(13) 换成语义等价但编码不同的 addi x15, x0, 13。
		let run = run_vmrs(16, Tamper::SwapProgram, Some(prog_image_hash(16)), &[(0x0, addi(15, 0, 13))]);
		assert!(!run.l_ok, "执行的指令 ≠ 承诺表值必须被 fetch 归约拒绝（l_ok==false）");
		show(&run, "sound swap-program (program binding)");
	}

	/// M8-B T0 例 7：公开镜像哈希词与 expected 不符 → hash_ok == false（verify 层对照）。
	#[test]
	fn vm_ram_sort_soundness_bad_prog_hash() {
		let run = run_vmrs(16, Tamper::BadProgHash, Some(prog_image_hash(16)), &[]);
		assert!(!run.hash_ok, "镜像哈希不符必须被公共输入对照拒绝（hash_ok==false）");
		show(&run, "sound bad-prog-hash (program binding)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_final_out() {
		let run = run_vmrs(16, Tamper::BadFinalOut, Some(prog_image_hash(16)), &[]);
		assert!(!run.c_ok, "验证端篡改最终输出必须被电路拒绝（c_ok==false）");
		show(&run, "sound bad-final-out (circuit layer)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_root_den() {
		let run = run_vmrs(16, Tamper::BadRootDen, Some(prog_image_hash(16)), &[]);
		// M12 顺序下 transcript 为单流：l 层篡改使 frontend 段失配（c_ok 不再隔离可观测）。
		assert!(!run.l_ok, "验证端篡改 root_den 必须被 fracaddcheck 拒绝（l_ok==false）");
		show(&run, "sound bad-root-den (logup layer)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_den_addr() {
		let run = run_vmrs(16, Tamper::BadDenAddr, Some(prog_image_hash(16)), &[]);
		assert!(!run.l_ok, "验证端篡改 addr 开口声明必须拒绝（l_ok==false）");
		show(&run, "sound bad-den-addr (logup layer)");
	}

	#[test]
	fn vm_ram_sort_soundness_bad_den_val() {
		let run = run_vmrs(16, Tamper::BadDenVal, Some(prog_image_hash(16)), &[]);
		assert!(!run.l_ok, "验证端篡改 val 开口声明必须拒绝（l_ok==false）");
		show(&run, "sound bad-den-val (logup layer)");
	}
}


#[cfg(test)]
mod scratch_small {
	use super::*;
	#[test]
	fn debug_small_program() {
		// 4 指令：lui x16, hi(BASE)；addi x15, x0, 42；sw x15, x16, 0；ecall
		let img = vec![lui(16, 1), addi(15, 0, 42), sw(15, 16, 0), ECALL];
		let proof = vmrs_prove(16, Some(&img));
		let key = vmrs_verifier_setup(proof.n, proof.t_len, proof.ts, proof.init_zero);
		let v = vmrs_verify_online(&key, &proof, Some(proof.prog_hash));
		println!("small: T={} ts={} l={} c={} l_ok={} h={} s={}", proof.t_len, proof.ts, proof.l, v.c_ok, v.l_ok, v.hash_ok, v.s_ok);
		assert!(v.c_ok && v.l_ok && v.s_ok);
	}
}

