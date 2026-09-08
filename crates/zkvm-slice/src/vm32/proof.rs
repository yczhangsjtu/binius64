//! vm32 proof pipeline: `run_machine_full` (prove + 3-table logup* + verify + output
//! triplet), `reverify`/`reverify2`, `claims_from_inout`, wlog builders, fetch-table
//! builder. Mechanically extracted from the former single-file `word_vm32.rs` (M6 T1).

use binius_compute::GlobalAllocator;
use binius_core::word::Word;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_frontend::CircuitStat;
use binius_hash::StdHashSuite;
use binius_ip::logup_star;
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_prover::Prover as WordProver;
use binius_transcript::ProverTranscript;
use binius_verifier::{Verifier as WordVerifier, config::StdChallenger};
use crate::vm32::circuit::build_circuit;
use crate::vm32::isa::*;
use crate::vm32::circuit::*;
use crate::vm32::interp::{Trace, run_program};

pub type LF = OptimalB128;
pub type LP = OptimalPackedB128;

pub fn build_fetch_prog(fetch: fn(u64) -> u64, word_overrides: &[(u64, u64)]) -> Vec<u64> {
	let mut prog = vec![0u64; 1usize << M_FETCH];
	for mm in [0x00u64,0x04,0x08,0x0c,0x10,0x14,0x18,0x1c,0x20,0x24,0x28,0x2c,0x30,0x34,0x38,0x3c,0x40,0x44,0x48,0x4c,0x50,0x54,0x58,0x5c,0x60,0x64,0x68,0x6c,0x70,0x74,0x78,0x7c,0x80,0x84,0x88,0x8c,0x90,0x94,0x98,0x9c,0xa0,0xa4,0xa8,0xac,0xb0,0xb4,0xb8,0xbc,0xc0,0xc4].iter() {
		prog[*mm as usize] = fetch(*mm);
	}
	// program-image override: a word_overrides entry replaces the fetch-table row, so an
	// arbitrary second program image (bubblesort) proves against its own fetch table.
	for &(a, w) in word_overrides {
		prog[a as usize] = w;
	}
	prog
}

pub fn build_reg_wlog(init: &[u32; NREG], trace: &Trace) -> Vec<u64> {
	let mut w = vec![0u64; NREG * VER_MAX];
	for r in 0..NREG { w[r * VER_MAX + 0] = init[r] as u64; }
	for c in &trace.cycles { if let Some(wr) = &c.write { w[wr.reg * VER_MAX + wr.ver] = wr.val as u64; } }
	w
}
pub fn build_ram_wlog(init_mem: &[u32; NRAM], trace: &Trace) -> Vec<u64> {
	let mut w = vec![0u64; NRAM * VER_MAX];
	for a in 0..NRAM { w[a * VER_MAX + 0] = init_mem[a] as u64; }
	for c in &trace.cycles { if let Some(s) = &c.store { w[s.addr * VER_MAX + s.ver] = s.val as u64; } }
	w
}
pub fn claims_from_inout(inout_words: &[Word], t_len: usize) -> (Vec<Vec<usize>>, Vec<u64>, Vec<Vec<usize>>, Vec<u64>, Vec<Vec<usize>>, Vec<u64>) {
	let mut fetch_idxs = Vec::new(); let mut fetch_claims = Vec::new();
	let mut reg_idxs = Vec::new(); let mut reg_claims = Vec::new();
	let mut ram_idxs = Vec::new(); let mut ram_claims = Vec::new();
	for t in 0..t_len {
		let inst = inout_words[io_inst(t_len, t)].0;
		let pc = inout_words[io_pc(t_len, t)].0;
		fetch_idxs.push(vec![pc as usize]); fetch_claims.push(inst);
		reg_idxs.push(vec![(inout_words[io_rd1_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_rd1_ver(t_len, t)].0 as usize]);
		reg_claims.push(inout_words[io_rd1_val(t_len, t)].0);
		reg_idxs.push(vec![(inout_words[io_rd2_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_rd2_ver(t_len, t)].0 as usize]);
		reg_claims.push(inout_words[io_rd2_val(t_len, t)].0);
		if inout_words[io_wr_iswrite(t_len, t)].0 != 0 {
			reg_idxs.push(vec![(inout_words[io_wr_reg(t_len, t)].0 as usize) * VER_MAX + inout_words[io_wr_ver(t_len, t)].0 as usize]);
			reg_claims.push(inout_words[io_wr_val(t_len, t)].0);
		}
		if inout_words[io_is_load(t_len, t)].0 != 0 {
			ram_idxs.push(vec![(inout_words[io_ld_addr(t_len, t)].0 as usize) * VER_MAX + inout_words[io_ld_ver(t_len, t)].0 as usize]);
			ram_claims.push(inout_words[io_ld_val(t_len, t)].0);
		}
		if inout_words[io_is_store(t_len, t)].0 != 0 {
			ram_idxs.push(vec![(inout_words[io_st_addr(t_len, t)].0 as usize) * VER_MAX + inout_words[io_st_ver(t_len, t)].0 as usize]);
			ram_claims.push(inout_words[io_st_val(t_len, t)].0);
		}
	}
	(fetch_idxs, fetch_claims, reg_idxs, reg_claims, ram_idxs, ram_claims)
}
pub struct M5Run {
	pub c_ok: bool,
	pub l_ok: bool,
	pub stat: CircuitStat,
	pub t_len: usize,
	pub inout_words: Vec<Word>,
	pub reg_wlog: Vec<u64>,
	pub ram_wlog: Vec<u64>,
	pub trace: Trace,
	pub prover: WordProver<OptimalPackedB128, StdHashSuite>,
	pub verifier: WordVerifier<StdHashSuite>,
	pub witness: binius_core::constraint_system::ValueVec,
}
pub fn run_machine_full(init: [u32; NREG], init_mem: &[u32; NRAM], word_overrides: &[(u64, u64)], load_overrides: &[(usize, u32)], fetch: fn(u64) -> u64) -> M5Run {
	run_machine_full_impl(init, init_mem, word_overrides, load_overrides, fetch, 0)
}

/// M11 F1：带 m_q advice 篡改偏移的变体（PoC/对照专用；私有实现，测试经 cfg(test) 包装）。
/// `mq_delta` 逐周期加到 advice 商上（0 = 诚实）。
fn run_machine_full_impl(init: [u32; NREG], init_mem: &[u32; NRAM], word_overrides: &[(u64, u64)], load_overrides: &[(usize, u32)], fetch: fn(u64) -> u64, mq_delta: i64) -> M5Run {
	let trace = run_program(init_mem, word_overrides, load_overrides, fetch);
	let t_len = trace.cycles.len();
	let (circuit, iref) = build_circuit(&trace);
	let stat = CircuitStat::collect(&circuit);
	let cs = circuit.constraint_system().clone();
	let mut w = circuit.new_witness_filler();
	for t in 0..t_len {
		let c = &trace.cycles[t];
		w[iref.inst[t]] = Word(c.inst);
		w[iref.pc[t]] = Word(c.pc);
		w[iref.rd1_reg[t]] = Word(c.reads[0].reg as u64);
		w[iref.rd1_ver[t]] = Word(c.reads[0].ver as u64);
		w[iref.rd1_val[t]] = Word(c.reads[0].val as u64);
		w[iref.rd2_reg[t]] = Word(c.reads[1].reg as u64);
		w[iref.rd2_ver[t]] = Word(c.reads[1].ver as u64);
		w[iref.rd2_val[t]] = Word(c.reads[1].val as u64);
		let rd_dec = ((c.inst >> 7) & 0x1f) as usize;
		w[iref.wr_reg[t]] = Word(rd_dec as u64);
		if let Some(wr) = &c.write {
			w[iref.wr_ver[t]] = Word(wr.ver as u64);
			w[iref.wr_val[t]] = Word(wr.val as u64);
			w[iref.wr_iswrite[t]] = Word(1);
		} else {
			w[iref.wr_ver[t]] = Word(c.regver[rd_dec] as u64);
			w[iref.wr_val[t]] = Word(0);
			w[iref.wr_iswrite[t]] = Word(0);
		}
		if let Some(ld) = &c.load {
			w[iref.ld_addr[t]] = Word(ld.addr as u64);
			w[iref.ld_ver[t]] = Word(ld.ver as u64);
			w[iref.ld_val[t]] = Word(ld.val as u64);
			w[iref.is_load[t]] = Word(1);
		} else {
			w[iref.ld_addr[t]] = Word(c.mem_addr as u64);
			w[iref.ld_ver[t]] = Word(c.ramver[c.mem_addr] as u64);
			w[iref.ld_val[t]] = Word(0);
			w[iref.is_load[t]] = Word(0);
		}
		if let Some(st) = &c.store {
			w[iref.st_addr[t]] = Word(st.addr as u64);
			w[iref.st_ver[t]] = Word(st.ver as u64);
			w[iref.st_val[t]] = Word(st.val as u64);
			w[iref.is_store[t]] = Word(1);
		} else {
			w[iref.st_addr[t]] = Word(c.mem_addr as u64);
			w[iref.st_ver[t]] = Word((c.ramver[c.mem_addr] + 1) as u64);
			w[iref.st_val[t]] = Word(c.reads[1].val as u64);
			w[iref.is_store[t]] = Word(0);
		}
	}
	for t in 0..t_len {
		let q = trace.cycles[t].m_q as i64 + mq_delta;
		w[iref.m_q[t]] = Word(q.rem_euclid(1 << 32) as u64);
	}
	for r in 0..NREG { w[iref.init_regs[r]] = Word(init[r] as u64); w[iref.final_regs[r]] = Word(trace.final_regs[r] as u64); }
	for a in 0..NRAM { w[iref.fin_ver[a]] = Word(trace.final_ramver[a] as u64); }
	circuit.populate_wire_witness(&mut w).expect("witness fill");
	let witness_vec = w.into_value_vec();
	let _native_ok = cs.verify(&witness_vec);
	let inout_words = witness_vec.inout().to_vec();
	let (fetch_idxs, fetch_claims, reg_idxs, reg_claims, ram_idxs, ram_claims) = claims_from_inout(&inout_words, t_len);
	let prog = build_fetch_prog(fetch, word_overrides);
	let fv = FieldBuffer::from_values(&prog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let reg_wlog = build_reg_wlog(&init, &trace);
	let rv = FieldBuffer::from_values(&reg_wlog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let ram_wlog = build_ram_wlog(init_mem, &trace);
	let mv = FieldBuffer::from_values(&ram_wlog.iter().map(|&v| LF::from(v as u128)).collect::<Vec<_>>());
	let verifier = WordVerifier::<StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = WordProver::<OptimalPackedB128, StdHashSuite>::setup(verifier.clone()).expect("prover setup");
	let alloc = GlobalAllocator;
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness_vec, &mut pt).expect("frontend prove");
	let f_lookers: Vec<Looker<LF>> = (0..fetch_idxs.len()).map(|i| Looker { index: &fetch_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(fetch_claims[i] as u128) }).collect();
	let r_lookers: Vec<Looker<LF>> = (0..reg_idxs.len()).map(|i| Looker { index: &reg_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(reg_claims[i] as u128) }).collect();
	let m_lookers: Vec<Looker<LF>> = (0..ram_idxs.len()).map(|i| Looker { index: &ram_idxs[i], eval_point: &[] as &[LF], eval_claim: LF::from(ram_claims[i] as u128) }).collect();
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let mut tables = vec![
		binius_ip_prover::logup_star::TableLookup { table: fv.as_view(), lookers: f_lookers },
		binius_ip_prover::logup_star::TableLookup { table: rv.as_view(), lookers: r_lookers },
	];
	if !m_lookers.is_empty() { tables.push(binius_ip_prover::logup_star::TableLookup { table: mv.as_view(), lookers: m_lookers }); }
	let _pout = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(&alloc, gamma, tables, &mut pt);
	let mut vt = pt.into_verifier();
	let circuit_ok = verifier.verify(&inout_words, &mut vt).is_ok();
	let vg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	let logup_ok = if vg == gamma {
		let mut v_tables = vec![
			logup_star::TableLookup { n_vars: M_FETCH, lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
			logup_star::TableLookup { n_vars: M_W_REG, lookers: reg_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
		];
		if !ram_claims.is_empty() {
			v_tables.push(logup_star::TableLookup { n_vars: M_W_RAM, lookers: ram_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() });
		}
		logup_star::verify_reduction::<LF, _>(&vg, v_tables, &mut vt).is_ok()
	} else { false };
	M5Run { c_ok: circuit_ok, l_ok: logup_ok, stat, t_len, inout_words, reg_wlog, ram_wlog, trace, prover, verifier, witness: witness_vec }
}
pub fn reverify2(run: &M5Run, bad_inout: &[Word]) -> (bool, bool) {
	let mut bt = ProverTranscript::new(StdChallenger::default());
	run.prover.prove(&run.witness, &mut bt).expect("re-prove");
	let mut bv = bt.into_verifier();
	let c_ok = run.verifier.verify(bad_inout, &mut bv).is_ok();
	let vg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut bv);
	let (_, fetch_claims, _, reg_claims, _, ram_claims) = claims_from_inout(bad_inout, run.t_len);
	let mut v_tables = vec![
		logup_star::TableLookup { n_vars: M_FETCH, lookers: fetch_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
		logup_star::TableLookup { n_vars: M_W_REG, lookers: reg_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() },
	];
	// M11：RAM 表仅在确有访存 claim 时参与（空 lookers 表触发上游 assert）
	if !ram_claims.is_empty() {
		v_tables.push(logup_star::TableLookup { n_vars: M_W_RAM, lookers: ram_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &[] as &[LF], eval_claim: LF::from(c as u128) }).collect() });
	}
	let l_ok = logup_star::verify_reduction::<LF, _>(&vg, v_tables, &mut bv).is_ok();
	(c_ok, l_ok)
}
pub fn reverify(run: &M5Run, bad_inout: &[Word]) -> bool {
	let (c_ok, l_ok) = reverify2(run, bad_inout);
	!(c_ok && l_ok)
}

#[cfg(test)]
#[path = "bench_tests.rs"]
mod bench_tests;

#[cfg(test)]
pub fn run_machine_full_opt(init: [u32; NREG], init_mem: &[u32; NRAM], word_overrides: &[(u64, u64)], load_overrides: &[(usize, u32)], fetch: fn(u64) -> u64, mq_delta: i64) -> M5Run {
	run_machine_full_impl(init, init_mem, word_overrides, load_overrides, fetch, mq_delta)
}
