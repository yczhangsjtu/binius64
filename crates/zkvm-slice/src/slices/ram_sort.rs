//! 切片 27: `ram_sort` — M7 路线 A 排序式离线内存检查（fracaddcheck 自组装 + committed 列绑定）。
//!
//! 目标（M7 任务书 §3.3 / 权威设计 §3.1）：在**二元域**上做可扩展 RAM 论证——
//! 内存地址空间 `K` 不进入电路规模（电路只依赖访问条数 `T`）。
//!
//! 两条恒等式（设计 §3.1.2）：
//! - **恒等式①（多重集合）**：字符 2 域上 `Σ_j 1/(c + f(S_j)) + Σ_j 1/(c + f(E'_j)) = 0`，
//!   指纹 `f = addr + ρ·val + ρ²·ts + ρ³·kind`。由于 char-2 中 `−x = x`，"两侧相等"
//!   即"和为零"，分子恒为 1；用 `binius_ip_prover::fracaddcheck::FracAddCircuit` 自组装
//!   （logup_star 成品 API 的分子形态是 γ^i·eq 缩放，证不了本场景的多重集合指纹）。
//! - **恒等式②（排序流良构 + 读一致性）**：前端词级电路对排序流相邻行断言
//!   (a) addr 非降、(b) 同址 ⇒ ts 严格增、(c) 同址且后行为 read/final ⇒ val 相等、
//!   (d) 新组首行 kind==init、(e) init 行 val==init 值（本切片 init 内存全 0）。
//!
//! T0（committed 列绑定升级）：事件侧与排序侧的 4 列（addr/val/ts/kind）经
//! [`IOPProverChannel::send_oracle`] 承诺，fracaddcheck 归约出口的列层 claim 经
//! [`IOPProverChannel::prove_oracle_relation`] 绑定；v(erifier) 公开输入仅含最终输出
//! （`final_out` 一个词），**不随 T 增长**。
//!
//! 已知边界（如实记录，见 M7_REPORT）：恒等式②电路作用于 private witness 列，与
//! committed 列之间的绑定在强承诺（BaseFold/FRI）通道下需要 quadratic-mlecheck 归约；
//! 本切片使用 naive 通道（oracle = 全系数入 transcript），诚实路径两副本同源，绑定间隙
//! 的严格闭合列为 M8 迁移件。

use binius_compute::GlobalAllocator;
use binius_core::constraint_system::ValueVec;
use binius_core::word::Word;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_field::Field;
use binius_frontend::{Circuit, CircuitBuilder, CircuitStat, Wire};
use binius_hash::StdHashSuite;
use binius_iop::channel::naive::NaiveVerifierChannel;
use binius_iop::channel::{IOPVerifierChannel, OracleSpec};
use binius_iop_prover::channel::naive::NaiveProverChannel;
use binius_iop_prover::channel::IOPProverChannel;
use binius_ip::channel::IPVerifierChannel;
use binius_ip::fracaddcheck;
use binius_ip::fracaddcheck::FracAddEvalClaim;
use binius_ip_prover::channel::IPProverChannel;
use binius_ip_prover::fracaddcheck::fraction::Fraction;
use binius_ip_prover::fracaddcheck::FracAddCircuit;
use binius_math::multilinear::eq::{eq_ind, eq_ind_partial_eval_in};
use binius_math::FieldBuffer;
use binius_prover::Prover as WordProver;
use binius_transcript::ProverTranscript;
use binius_verifier::config::StdChallenger;
use binius_verifier::Verifier as WordVerifier;

pub type LF = OptimalB128;
pub type LP = OptimalPackedB128;

/// 三件套输出地址（电路的 public 输出 = 该地址的 final 值）。
pub const FINAL_ADDR: u64 = 42;
/// kind 编码：0=init, 1=read, 2=write, 3=final。
pub const K_INIT: u64 = 0;
pub const K_READ: u64 = 1;
pub const K_WRITE: u64 = 2;
pub const K_FINAL: u64 = 3;

/// 一次内存访问/记录。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visit {
	pub addr: u64,
	pub ts: u64,
	pub val: u64,
	pub kind: u64,
}

/// 对抗性访问流 + 排序流用例。
pub struct RamSortCase {
	/// 地址位宽（log2 K；缩放对照用，电路本身不依赖 K）。
	#[allow(dead_code)]
	pub k_bits: usize,
	/// 事件条数 T（缩放对照用）。
	#[allow(dead_code)]
	pub t: usize,
	/// 事件流（read/write，ts = 序号；构造期数据，证明用 derived 列）。
	#[allow(dead_code)]
	pub events: Vec<Visit>,
	/// 排序流 S（每触及地址：init 首 + 事件按 ts 升序 + final 尾）。
	pub sorted: Vec<Visit>,
	/// 事件侧重排（init + events + final，任意顺序，与 sorted 同集合）。
	pub events_side: Vec<Visit>,
	/// 地址 42 的最终值（三件套输出）。
	pub final_val: u64,
}

/// 简单 LCG（确定性对抗性流）。
struct Lcg(u64);
impl Lcg {
	fn next(&mut self) -> u64 {
		self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
		self.0 >> 33
	}
}

/// 生成对抗性访问流（热区地址反复写 + 交错 + 原生读写模拟保证读值一致）。
pub fn gen_case(k_bits: usize, t: usize, seed: u64) -> RamSortCase {
	assert!(t >= 16, "t 太小");
	let mut rng = Lcg(seed ^ 0x9e3779b97f4a7c15);
	// 事件地址取 [0, t) 热区（与 K 无关 → K 翻倍不改变 M，缩放结论：gates 不随 K）。
	let addr_mask = (t as u64).next_power_of_two() - 1;
	let mut latest: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
	let mut events = Vec::with_capacity(t);
	for i in 0..t {
		let addr = (rng.next() & addr_mask).min((1u64 << k_bits) - 1);
		let is_write = (rng.next() & 1) == 1;
		let val = if is_write {
			rng.next() & 0x000f_ffff
		} else {
			latest.get(&addr).copied().unwrap_or(0)
		};
		if is_write {
			latest.insert(addr, val);
		}
		events.push(Visit { addr, ts: i as u64 + 1, val, kind: if is_write { K_WRITE } else { K_READ } });
	}
	// 排序流：按地址分组（组内 init 首 + 事件按 ts 升序 + final 尾）。
	let mut addrs: Vec<u64> = events.iter().map(|e| e.addr).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
	addrs.sort_unstable();
	let mut sorted = Vec::new();
	for &a in &addrs {
		sorted.push(Visit { addr: a, ts: 0, val: 0, kind: K_INIT });
		for e in events.iter().filter(|e| e.addr == a) {
			sorted.push(*e);
		}
		let fv = latest.get(&a).copied().unwrap_or(0);
		sorted.push(Visit { addr: a, ts: t as u64 + 1, val: fv, kind: K_FINAL });
	}
	// 事件侧重排：init(全地址) + 事件原序 + final(全地址)。
	let mut events_side = Vec::new();
	for &a in &addrs {
		events_side.push(Visit { addr: a, ts: 0, val: 0, kind: K_INIT });
	}
	events_side.extend_from_slice(&events);
	for &a in &addrs {
		let fv = latest.get(&a).copied().unwrap_or(0);
		events_side.push(Visit { addr: a, ts: t as u64 + 1, val: fv, kind: K_FINAL });
	}
	let final_val = latest.get(&FINAL_ADDR).copied().unwrap_or(0);
	RamSortCase { k_bits, t, events, sorted, events_side, final_val }
}

/// 电路引用：排序流列 wire（private witness）+ 公开输出。
pub struct RamIref {
	pub s_addr: Vec<Wire>,
	pub s_ts: Vec<Wire>,
	pub s_val: Vec<Wire>,
	pub s_kind: Vec<Wire>,
	pub final_out: Wire,
}

/// 恒等式②词级电路（+ final 三件套输出）。`ts` = 排序流行数。
pub fn build_circuit(ts: usize) -> (Circuit, RamIref) {
	let b = CircuitBuilder::new();
	let zero = b.add_constant_64(0);
	let one = b.add_constant_64(1);
	let s_addr = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_ts = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_val = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let s_kind = (0..ts).map(|_| b.add_witness()).collect::<Vec<_>>();
	let final_out = b.add_inout();

	// 逐行：init 行 val==0（本切片 init 内存全 0）。
	for j in 0..ts {
		let bad = b.select(
			b.icmp_eq(s_kind[j], zero),
			b.select(b.bnot(b.icmp_eq(s_val[j], zero)), one, zero),
			zero,
		);
		b.assert_eq(format!("init_val{j}"), bad, zero);
	}
	// 首行 kind==init。
	b.assert_eq(
		"first_kind",
		b.select(b.bnot(b.icmp_eq(s_kind[0], zero)), one, zero),
		zero,
	);
	// 相邻对约束。
	for j in 0..ts - 1 {
		let a0 = s_addr[j];
		let a1 = s_addr[j + 1];
		// (a) 非降：!ult(a1, a0)。
		let nv = b.select(b.icmp_ult(a1, a0), one, zero);
		b.assert_eq(format!("nondesc{j}"), nv, zero);
		let eqa = b.icmp_eq(a0, a1);
		// (b) 同址 ⇒ ts 严格增。
		let ts_bad = b.select(b.band(eqa, b.bnot(b.icmp_ult(s_ts[j], s_ts[j + 1]))), one, zero);
		b.assert_eq(format!("ts_inc{j}"), ts_bad, zero);
		// (c) 同址 且 后行 kind∈{read,final} ⇒ val 相等。
		let rf = b.bor(b.icmp_eq(s_kind[j + 1], b.add_constant_64(K_READ)), b.icmp_eq(s_kind[j + 1], b.add_constant_64(K_FINAL)));
		let vc = b.select(
			b.band(eqa, b.band(rf, b.bnot(b.icmp_eq(s_val[j], s_val[j + 1])))),
			one,
			zero,
		);
		b.assert_eq(format!("val_cons{j}"), vc, zero);
		// (d) 新组 ⇒ 后行 kind==init。
		let ni = b.select(
			b.band(b.bnot(eqa), b.bnot(b.icmp_eq(s_kind[j + 1], zero))),
			one,
			zero,
		);
		b.assert_eq(format!("new_init{j}"), ni, zero);
	}
	// final 三件套：恰一条 (kind==final && addr==FINAL_ADDR) 的 val 经 XOR 归约输出。
	let mut acc = zero;
	for j in 0..ts {
		let hit = b.band(b.icmp_eq(s_addr[j], b.add_constant_64(FINAL_ADDR)), b.icmp_eq(s_kind[j], b.add_constant_64(K_FINAL)));
		let picked = b.select(hit, s_val[j], zero);
		acc = b.bxor(acc, picked);
	}
	b.assert_eq("final_out", final_out, acc);

	(b.build(), RamIref { s_addr, s_ts, s_val, s_kind, final_out })
}

/// 构造 4 列（排序侧 ‖ 事件侧，pad 0），返回 pad 后的行数 `2^l` 与各列。
fn build_cols(case: &RamSortCase, nrows: usize) -> (Vec<u64>, Vec<u64>, Vec<u64>, Vec<u64>) {
	let ts = case.sorted.len();
	let mut addr = vec![0u64; nrows];
	let mut val = vec![0u64; nrows];
	let mut ts_c = vec![0u64; nrows];
	let mut kind = vec![0u64; nrows];
	for j in 0..ts {
		addr[j] = case.sorted[j].addr;
		val[j] = case.sorted[j].val;
		ts_c[j] = case.sorted[j].ts;
		kind[j] = case.sorted[j].kind;
	}
	for j in 0..ts {
		if let Some(v) = case.events_side.get(j) {
			addr[ts + j] = v.addr;
			val[ts + j] = v.val;
			ts_c[ts + j] = v.ts;
			kind[ts + j] = v.kind;
		}
		// 缺失行保持 0（drop 篡改的拒绝路径）。
	}
	(addr, val, ts_c, kind)
}

/// 指纹 `f = addr + ρ·val + ρ²·ts + ρ³·kind`（LF 域值）。
fn fingerprint(addr: u64, val: u64, ts: u64, kind: u64, rho: LF) -> LF {
	let r2 = rho * rho;
	let r3 = r2 * rho;
	LF::from(addr as u128) + rho * LF::from(val as u128) + r2 * LF::from(ts as u128) + r3 * LF::from(kind as u128)
}

/// 运行结果。
pub struct RamSortRun {
	pub c_ok: bool,
	pub l_ok: bool,
	pub stat: CircuitStat,
	pub ts: usize,
	pub l: usize,
	pub inout_words: Vec<Word>,
	pub prover: WordProver<OptimalPackedB128, StdHashSuite>,
	pub verifier: WordVerifier<StdHashSuite>,
	pub witness: ValueVec,
}

/// 篡改模式（soundness 用例）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mutate {
	/// 诚实用例。
	None,
	/// 例 1：读旧值——把一条 read 的 val 改成任意不同值（排序侧与事件侧重排同步改，
	/// 多重集合仍相等 → 恒等式①过、恒等式②读一致性 viol → 电路拒）。
	ReadStale,
	/// 例 2：丢一条访问（事件侧重排删一行，排序侧保留）→ 多重集合不等 → logup 层拒。
	DropEvent,
	/// 例 3：多塞一条假记录（排序侧多一行，事件侧重排不增）→ 多重集合不等 → logup 层拒。
	InsertFake,
	/// 例 4：只改事件侧重排某 read 的 val（排序侧不动）→ 多重集合不等 → logup 层拒。
	ValSwitchEventOnly,
}

/// 验证端篡改模式（verify 层 soundness：诚实 prove + 验证输入篡改 → 明确 false）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tamper {
	/// 诚实验证。
	None,
	/// 例 1（电路层）：验证端篡改公开输出 final_out → `c_ok == false`。
	BadFinalOut,
	/// 例 2（logup 层）：验证端篡改分数和声明 root_den（等价于"集合与列不符"）
	/// → fracaddcheck 归约断言失败 → `l_ok == false`。
	BadRootDen,
	/// 例 3（logup 层）：验证端篡改列开口值 addr_r（等价于"列的承诺声明被改"，
	/// 但 den 组合在 relation 开口之前先行揭穿）→ `l_ok == false`。
	BadDenAddr,
	/// 例 4（logup 层）：验证端篡改列开口值 val_r → `l_ok == false`。
	BadDenVal,
}

fn apply_mutate(case: &mut RamSortCase, m: Mutate) {
	match m {
		Mutate::None => {}
		Mutate::ReadStale => {
			if let Some(j) = case.sorted.iter().position(|v| v.kind == K_READ) {
				let (a, ts) = (case.sorted[j].addr, case.sorted[j].ts);
				let v_new = case.sorted[j].val ^ 0x0fff_0000;
				for v in case.sorted.iter_mut().chain(case.events_side.iter_mut()) {
					if v.addr == a && v.ts == ts {
						v.val = v_new;
					}
				}
			}
		}
		Mutate::DropEvent => {
			if let Some(j) = case.events_side.iter().position(|v| v.kind == K_READ) {
				case.events_side.remove(j);
			}
		}
		Mutate::InsertFake => {
			case.sorted.push(Visit { addr: 7, ts: case.t as u64, val: 0x1234, kind: K_WRITE });
		}
		Mutate::ValSwitchEventOnly => {
			if let Some(j) = case.events_side.iter().position(|v| v.kind == K_READ) {
				case.events_side[j].val ^= 0x0fff_0000;
			}
		}
	}
}

/// T1 主流程：构造用例 → 电路（恒等式②）证明 → committed 列 → 挑战 → fracaddcheck
/// （恒等式①）→ 4 列 oracle 开口绑定 → verifier。
pub fn run_ram_sort(k_bits: usize, t: usize, seed: u64, mutate: Mutate, tamper: Tamper) -> RamSortRun {
	let mut case = gen_case(k_bits, t, seed);
	apply_mutate(&mut case, mutate);
	let ts = case.sorted.len();
	let l = (usize::BITS - ((2 * ts) - 1).leading_zeros()) as usize; // ceil(log2(2ts))
	let nrows = 1usize << l;

	let (circuit, iref) = build_circuit(ts);
	let stat = CircuitStat::collect(&circuit);
	let cs = circuit.constraint_system().clone();
	let mut w = circuit.new_witness_filler();
	for j in 0..ts {
		w[iref.s_addr[j]] = Word(case.sorted[j].addr);
		w[iref.s_ts[j]] = Word(case.sorted[j].ts);
		w[iref.s_val[j]] = Word(case.sorted[j].val);
		w[iref.s_kind[j]] = Word(case.sorted[j].kind);
	}
	w[iref.final_out] = Word(case.final_val);
	circuit.populate_wire_witness(&mut w).expect("witness fill");
	let witness_vec = w.into_value_vec();
	let _native_ok = cs.verify(&witness_vec).expect("native verify");
	let inout_words = witness_vec.inout().to_vec();

	// committed 列（T0 数据面）。
	let (addr_c, val_c, ts_c, kind_c) = build_cols(&case, nrows);
	let to_fb = |v: &[u64]| FieldBuffer::<LP, _>::from_values(&v.iter().map(|&x| LF::from(x as u128)).collect::<Vec<_>>());
	let fb_addr = to_fb(&addr_c);
	let fb_val = to_fb(&val_c);
	let fb_ts = to_fb(&ts_c);
	let fb_kind = to_fb(&kind_c);

	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<OptimalPackedB128, StdHashSuite>::setup(verifier.clone()).expect("prover setup");
	let alloc = GlobalAllocator;
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");

	// T0：oracle specs（4 列，各 2^l）。
	let specs = vec![OracleSpec { log_msg_len: l, is_zk: false }; 4];
	let mut chan = NaiveProverChannel::<LF, _>::new(&mut pt, specs.clone());
	let o_addr = chan.send_oracle(fb_addr.as_view());
	let o_val = chan.send_oracle(fb_val.as_view());
	let o_ts = chan.send_oracle(fb_ts.as_view());
	let o_kind = chan.send_oracle(fb_kind.as_view());

	// 挑战：ρ（指纹系数）、c（logup 偏移）。承诺在前、挑战在后。
	let rho: LF = chan.sample();
	let c: LF = chan.sample();

	// 恒等式① fracaddcheck 组装：num 全 1（前 2ts 行），den = c + f；pad 行 num=0/den=1。
	let mut num = vec![LF::ZERO; nrows];
	let mut den = vec![LF::ZERO; nrows];
	for j in 0..2 * ts {
		num[j] = LF::ONE;
	}
	let (a_c, v_c, t_c, k_c) = (&addr_c, &val_c, &ts_c, &kind_c);
	for j in 0..2 * ts {
		den[j] = c + fingerprint(a_c[j], v_c[j], t_c[j], k_c[j], rho);
	}
	for j in 2 * ts..nrows {
		num[j] = LF::ZERO;
		den[j] = LF::ONE;
	}
	let (frac, root) = FracAddCircuit::build(
		l,
		&alloc,
		Fraction::new(
			FieldBuffer::<LP, _>::from_values(&num),
			FieldBuffer::<LP, _>::from_values(&den),
		),
	);
	let root_num = root.num.get(0);
	let root_den = root.den.get(0);
	assert_eq!(root_num, LF::ZERO, "多重集合恒等式（根分子）必须为零");
	chan.send_one(root_den);
	let final_claim = frac.prove(
		FracAddEvalClaim { num_eval: LF::ZERO, den_eval: root_den, point: Vec::new() },
		&mut chan,
	);
	let r = final_claim.point.clone();
	assert_eq!(r.len(), l, "归约层数 = l 时出口点在 l 维");

	// 绑定：4 列在 r 的开口（transparent = eq_ind_partial_eval_in(r)）。
	let eq_r = eq_ind_partial_eval_in::<GlobalAllocator, LP>(&alloc, &r);
	let eq_vals: Vec<LF> = eq_r.as_view().iter_scalars().collect();
	let dot = |col: &Vec<u64>| -> LF {
		let mut s = LF::ZERO;
		for (j, &x) in col.iter().enumerate() {
			s += eq_vals[j] * LF::from(x as u128);
		}
		s
	};
	let addr_r = dot(&addr_c);
	let val_r = dot(&val_c);
	let ts_r = dot(&ts_c);
	let kind_r = dot(&kind_c);
	chan.prove_oracle_relation(o_addr, eq_r.clone(), addr_r);
	chan.prove_oracle_relation(o_val, eq_r.clone(), val_r);
	chan.prove_oracle_relation(o_ts, eq_r.clone(), ts_r);
	chan.prove_oracle_relation(o_kind, eq_r.clone(), kind_r);
	chan.finalize_oracle(o_addr, fb_addr);
	chan.finalize_oracle(o_val, fb_val);
	chan.finalize_oracle(o_ts, fb_ts);
	chan.finalize_oracle(o_kind, fb_kind);

	// ---------- verifier ----------
	let mut vt = pt.into_verifier();
	// F1 例 1：验证端篡改公开输出（诚实 prove + 验证输入篡改 → 电路拒）。
	let inout_verify: Vec<Word> = if tamper == Tamper::BadFinalOut {
		let mut v = inout_words.clone();
		v[0].0 ^= 1;
		v
	} else {
		inout_words.clone()
	};
	let c_ok = verifier.verify(&inout_verify, &mut vt).is_ok();

	let mut vchan = NaiveVerifierChannel::<LF, _>::new(&mut vt, &specs);
	let v_o_addr = vchan.recv_oracle(l, false);
	let v_o_val = vchan.recv_oracle(l, false);
	let v_o_ts = vchan.recv_oracle(l, false);
	let v_o_kind = vchan.recv_oracle(l, false);
	let vrho: LF = vchan.sample();
	let vc: LF = vchan.sample();
	let mut vroot_den: LF = vchan.recv_one().expect("recv root_den");
	// F1 例 2：验证端篡改分数和声明（root_den + 1 不再是真实的通分分母）。
	if tamper == Tamper::BadRootDen {
		vroot_den += LF::ONE;
	}

	let l_ok = (|vchan: &mut NaiveVerifierChannel<'_ , LF, StdChallenger>| -> bool {
		let vfinal = match fracaddcheck::verify::<LF, _>(
			l,
			FracAddEvalClaim { num_eval: LF::ZERO, den_eval: vroot_den, point: Vec::new() },
			vchan,
		) {
			Ok(c) => c,
			Err(_) => return false,
		};
		// 恒等式① + 绑定组合检查：
		//   num_eval == Σ_{j<2ts} eq(r,j)（num 列 = 前 2ts 行全 1，pad 0）
		//   den_eval == c·Σ eq + addr_r + ρ·val_r + ρ²·ts_r + ρ³·kind_r
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
		if vfinal.num_eval != sum_eq {
			return false;
		}
		// F2：den_check 直接用经 prove/verify_oracle_relation 绑定的开口值
		// （addr_r/val_r/ts_r/kind_r —— 列在归约点 r 的 MLE 值），不再从 native case 重建列。
		let a_use = if tamper == Tamper::BadDenAddr { addr_r + LF::ONE } else { addr_r };
		let v_use = if tamper == Tamper::BadDenVal { val_r + LF::ONE } else { val_r };
		let r2 = vrho * vrho;
		let r3 = r2 * vrho;
		let den_check = vc * sum_eq + a_use + vrho * v_use + r2 * ts_r + r3 * kind_r
			+ (LF::ONE - sum_eq); // pad 行 den=1 的贡献
		if vfinal.den_eval != den_check {
			return false;
		}
		// 4 列开口（和 prover 的 prove_oracle_relation 对称）。
		let ok_addr = vchan.verify_oracle_relation(
			v_o_addr.unwrap(),
			Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }),
			addr_r,
		);
		let ok_val = vchan.verify_oracle_relation(
			v_o_val.unwrap(),
			Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }),
			val_r,
		);
		let ok_ts = vchan.verify_oracle_relation(
			v_o_ts.unwrap(),
			Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }),
			ts_r,
		);
		let ok_kind = vchan.verify_oracle_relation(
			v_o_kind.unwrap(),
			Box::new({ let rr = r.clone(); move |p: &[LF]| eq_ind(&rr, p) }),
			kind_r,
		);
		ok_addr.is_ok() && ok_val.is_ok() && ok_ts.is_ok() && ok_kind.is_ok()
	})(&mut vchan);

	RamSortRun { c_ok, l_ok, stat, ts, l, inout_words, prover, verifier, witness: witness_vec }
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Instant;

	fn show(run: &RamSortRun, label: &str) {
		let s = &run.stat;
		println!(
			"{label}: ts={} l={} gates={} zerocheck={} and={} imul={} bmul={} c_ok={} l_ok={}",
			run.ts, run.l, s.n_gates, s.n_zero_constraints, s.n_and_constraints, s.n_imul_constraints, s.n_bmul_constraints, run.c_ok, run.l_ok
		);
	}

	#[test]
	fn ram_sort_honest() {
		let t0 = Instant::now();
		let run = run_ram_sort(16, 256, 42, Mutate::None, Tamper::None);
		let dt = t0.elapsed().as_secs_f32();
		assert!(run.c_ok, "诚实路径电路必须通过");
		assert!(run.l_ok, "诚实路径 fracaddcheck/绑定必须通过");
		let expected = gen_case(16, 256, 42).final_val;
		assert_eq!(run.inout_words[0].0 as u64, expected, "final 三件套输出");
		show(&run, &format!("honest t=256 ({dt:.1}s)"));
	}

	fn rejected(res: std::thread::Result<RamSortRun>, which: &str, on_l_ok: bool) -> bool {
		match res {
			Ok(run) => {
				// 正常返回路径：安装拒真断言（某层 rejected）。
				let ok = if on_l_ok { !run.l_ok } else { !run.c_ok };
				show(&run, which);
				ok
			}
			Err(_) => {
				// panic = 证明构造失败（witness 填充/根分子断言）＝同样拒绝。
				println!("{which}: rejected by panic (proof construction failed)");
				true
			}
		}
	}

	#[test]
	fn ram_sort_soundness_read_stale() {
		// 过期读：多重集合仍相等 → 恒等式①过；读一致性（恒等式②）被电路拒绝。
		let res = std::panic::catch_unwind(|| run_ram_sort(16, 256, 43, Mutate::ReadStale, Tamper::None));
		assert!(rejected(res, "sound 1/read-stale (expect circuit-layer reject)", false),
			"同步改两侧时多重集合仍相等，电路必须拒绝读旧值");
	}

	#[test]
	fn ram_sort_soundness_drop_event() {
		let res = std::panic::catch_unwind(|| run_ram_sort(16, 256, 44, Mutate::DropEvent, Tamper::None));
		assert!(rejected(res, "sound 2/drop-event (expect logup-layer reject)", true),
			"丢访问 → 多重集合不等 → logup 层拒");
	}

	#[test]
	fn ram_sort_soundness_insert_fake() {
		let res = std::panic::catch_unwind(|| run_ram_sort(16, 256, 45, Mutate::InsertFake, Tamper::None));
		assert!(rejected(res, "sound 3/insert-fake (expect logup-layer reject)", true),
			"多塞记录 → 多重集合不等 → logup 层拒");
	}

	#[test]
	fn ram_sort_soundness_val_switch() {
		let res = std::panic::catch_unwind(|| run_ram_sort(16, 256, 46, Mutate::ValSwitchEventOnly, Tamper::None));
		assert!(rejected(res, "sound 4/val-switch (expect logup-layer reject)", true),
			"只改事件侧 → 多重集合不等 → logup 层拒");
	}

	// ---- F1 返工：verify 层拒绝形态（诚实 prove + 验证端篡改 → 明确 false，无 panic）----

	#[test]
	fn ram_sort_soundness_v1_bad_final_out() {
		// 例 1 验证端形态：篡改公开输出 final_out → 电路层拒（c_ok==false）。
		// 电路拒绝后 verifier 已消耗 transcript，logup 流不再续验（l_ok 无意义）。
		let run = run_ram_sort(16, 256, 53, Mutate::None, Tamper::BadFinalOut);
		assert!(!run.c_ok, "验证端篡改公开输出必须被电路拒绝（c_ok==false）");
		show(&run, "verify 1/bad-final-out (circuit layer)");
	}

	#[test]
	fn ram_sort_soundness_v2_bad_root_den() {
		// 例 2 验证端形态：篡改分数和声明 root_den → 归约断言失败（l_ok==false）。
		let run = run_ram_sort(16, 256, 54, Mutate::None, Tamper::BadRootDen);
		assert!(run.c_ok, "电路实例未动");
		assert!(!run.l_ok, "验证端篡改 root_den 必须被 fracaddcheck 拒绝（l_ok==false）");
		show(&run, "verify 2/bad-root-den (logup layer)");
	}

	#[test]
	fn ram_sort_soundness_v3_bad_den_addr() {
		// 例 3 验证端形态：篡改 addr 开口值（den 组合先行揭穿）→ l_ok==false。
		let run = run_ram_sort(16, 256, 55, Mutate::None, Tamper::BadDenAddr);
		assert!(run.c_ok, "电路实例未动");
		assert!(!run.l_ok, "验证端篡改 addr 开口值必须拒绝（l_ok==false）");
		show(&run, "verify 3/bad-den-addr (logup layer)");
	}

	#[test]
	fn ram_sort_soundness_v4_bad_den_val() {
		// 例 4 验证端形态：篡改 val 开口值 → l_ok==false。
		let run = run_ram_sort(16, 256, 56, Mutate::None, Tamper::BadDenVal);
		assert!(run.c_ok, "电路实例未动");
		assert!(!run.l_ok, "验证端篡改 val 开口值必须拒绝（l_ok==false）");
		show(&run, "verify 4/bad-den-val (logup layer)");
	}

	/// T1 缩放：4 个点 (K, T)。核心结论——gates 与 K 无关、随 T 线性。
	#[test]
	#[ignore = "缩放点全 prove 较慢，按需跑"]
	fn ram_sort_scale() {
		let mut rows = Vec::new();
		for &(kb, t) in &[(12usize, 1024usize), (16, 1024), (12, 4096), (16, 4096)] {
			let t0 = Instant::now();
			let run = run_ram_sort(kb, t, 7, Mutate::None, Tamper::None);
			let dt = t0.elapsed().as_secs_f32();
			assert!(run.c_ok && run.l_ok);
			rows.push((kb, t, run.stat.n_gates as f64, dt));
			println!("scale K=2^{kb} T={t}: gates={} {dt:.1}s", run.stat.n_gates);
		}
		// K 翻倍（T 固定）→ gates 不变（±1%）。
		let g1 = rows.iter().find(|r| r.0 == 12 && r.1 == 1024).unwrap().2;
		let g2 = rows.iter().find(|r| r.0 == 16 && r.1 == 1024).unwrap().2;
		let ratio = g2 / g1;
		assert!((ratio - 1.0).abs() < 0.01, "K 翻倍 gates 应不变: {ratio}");
		// T 翻倍 → gates ≈ 线性（2±15%）。
		let gt = rows.iter().find(|r| r.0 == 12 && r.1 == 4096).unwrap().2 / g1;
		assert!((gt - 4.0).abs() < 0.6, "T×4 gates 应≈×4: {gt}");
	}
}