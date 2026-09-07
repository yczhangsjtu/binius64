//! Per-instruction cost benchmark (M6 T3): each RV32I instruction repeated N=16 times
//! in a fixed-operand micro-program, proved through the full `run_machine_full` pipeline.
//! Run with: `cargo test -p binius-zkvm-slice --lib -- --ignored --nocapture bench`.
//! Numbers in `zkvm-project/BENCHMARKS.md` are pasted from this test's real output.

use super::*;
use std::time::Instant;

const N: usize = 16; // instruction repeats per micro-program (>= 8)
const SETUP_SLOTS: u64 = 0x14; // 0x00..0x10: x1 = 0x80000000, x2 = 0x7FFFFFFF

fn setup() -> Vec<(u64, u64)> {
	vec![
		(0x00, lhs_lui(1, 0x80000)),      // x1 = 0x80000000
		(0x04, addi(1, 1, 0)),
		(0x08, lhs_lui(2, 0x7ffff)),      // x2 = 0x7FFFF000
		(0x0c, addi(2, 2, 0x0ff)),        //      + 0x0FF
		(0x10, addi(2, 2, 0xf00)),        //      + 0xF00  -> 0x7FFFFFFF
	]
}
fn micro(mut p: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
	let next = p.last().map(|&(a, _)| a + 4).unwrap();
	p.push((next, jal(0, 0xc4 - next)));
	p
}
fn same_inst(inst_gen: &dyn Fn(u64) -> u64) -> Vec<(u64, u64)> {
	let mut p = setup();
	for i in 0..N {
		p.push((SETUP_SLOTS + (i as u64) * 4, inst_gen(SETUP_SLOTS + (i as u64) * 4)));
	}
	micro(p)
}
fn bench_one(name: &str, prog: Vec<(u64, u64)>) {
	let init = [0u32; NRAM];
	let t0 = Instant::now();
	let run = run_machine_full([0u32; NREG], &init, &prog, &[], |_| 0);
	let dt = t0.elapsed().as_micros();
	let s = &run.stat;
	let cyc = run.trace.cycles.len();
	assert!(run.c_ok && run.l_ok, "{name}: honest proof failed");
	println!(
		"{name:<10} cyc={cyc:<4} gates={:<9} zero={:<7} and={:<8} imul={:<6} bmul={:<8} g/cyc={:<7} t={}us",
		s.n_gates, s.n_zero_constraints, s.n_and_constraints, s.n_imul_constraints, s.n_bmul_constraints,
		s.n_gates / cyc, dt
	);
}

#[test]
#[ignore = "benchmark: run with -- --ignored --nocapture to reproduce BENCHMARKS.md"]
fn bench_instruction_costs() {
	println!("M6 T3 per-instruction cost benchmark (N={N} repeats/instruction, debug profile, i5-12400F AVX2)");
	println!("{:<10} {:<6} {:<9} {:<7} {:<8} {:<6} {:<8} {:<7} {:>6}", "inst", "cyc", "gates", "zero", "and", "imul", "bmul", "g/cyc", "t(us)");
	// R-type (x1=0x80000000, x2=0x7FFFFFFF -> rd=x3)
	let r = |f: fn(u64, u64, u64) -> u64| same_inst(&|_| f(3, 1, 2));
	bench_one("add", r(add));
	bench_one("sub", r(sub));
	bench_one("sll", r(sll));
	bench_one("slt", r(slt));
	bench_one("sltu", r(sltu));
	bench_one("xor", r(xor));
	bench_one("srl", r(srl));
	bench_one("sra", r(sra));
	bench_one("or", r(or));
	bench_one("and", r(and));
	// I-type
	let i = |f: fn(u64, u64, u64) -> u64| same_inst(&|_| f(3, 1, 0x1ff));
	bench_one("addi", i(addi));
	bench_one("slti", i(slti));
	bench_one("sltiu", i(sltiu));
	bench_one("xori", i(xori));
	bench_one("ori", i(ori));
	bench_one("andi", i(andi));
	let sh = |f: fn(u64, u64, u64) -> u64| same_inst(&|_| f(3, 1, 9));
	bench_one("slli", sh(slli));
	bench_one("srli", sh(srli));
	bench_one("srai", sh(srai));
	// U / J
	bench_one("lui", same_inst(&|_| lhs_lui(3, 0xabcde)));
	bench_one("auipc", same_inst(&|_| auipc(3, 0x00001)));
	bench_one("jal", {
		// linear chain: each jal rd, +4 -> target = next slot
		let mut p = setup();
		for i in 0..N {
			p.push((SETUP_SLOTS + (i as u64) * 4, jal(3, 4)));
		}
		micro(p)
	});
	bench_one("jalr", {
		// Register-indirect jumps cannot form a static linear chain of jalr-only (target =
		// x1+imm is loop-invariant), so we interleave `addi x1,x1,8; jalr x3,x1,0` pairs:
		// each jalr targets the *next* addi slot and every jalr executes exactly once.
		let mut p = vec![(0x00, addi(1, 0, 0x14)), (0x04, addi(1, 1, 0))];
		for i in 0..N {
			let s = SETUP_SLOTS + (i as u64) * 8;
			p.push((s, addi(1, 1, 8)));
			p.push((s + 4, jalr(3, 1, 0)));
		}
		micro(p)
	});
	// B-type (all configured NOT-taken so the chain stays linear)
	let b = |f: fn(u64, u64, u64) -> u64| same_inst(&|_| f(1, 2, 4));
	bench_one("beq", b(beq));
	bench_one("bne", same_inst(&|_| bne(2, 2, 4)));
	bench_one("blt", same_inst(&|_| blt(2, 1, 4)));
	bench_one("bge", b(bge));
	bench_one("bltu", b(bltu));
	bench_one("bgeu", same_inst(&|_| bgeu(2, 1, 4)));
	// memory
	bench_one("lw", same_inst(&|_| lhs_lw(3, 0, 24)));
	bench_one("sw", same_inst(&|_| sw(2, 0, 24)));
}