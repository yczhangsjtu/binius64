//! Per-instruction native unit tests (M6 T2a): every RV32I instruction x boundary
//! operand vectors (0 / 1 / -1 / 0x7FFFFFFF / 0x80000000 / shamt 0/31 / negative
//! immediates / x0-write-drop), run through the native interpreter (`run_program`)
//! with a purpose-built micro-program. Independent from the proof pipeline.

use super::*;

fn run1(prog: &[(u64, u64)]) -> [u32; NREG] {
	let mut p = prog.to_vec();
	let next = prog.last().map(|&(a, _)| a + 4).unwrap_or(0x04);
	p.push((next, jal(0, 0xc4 - next))); // jump to halt
	run_program(&[0u32; NRAM], &p, &[], |_| 0).final_regs
}
fn emit_val(val: u32, r: u64, at: u64, out: &mut Vec<(u64, u64)>) {
	let lo = val & 0xfff;
	let mut hi = val >> 12;
	if lo & 0x800 != 0 {
		hi += 1; // addi sign-extends lo
	}
	out.push((at, lhs_lui(r, hi as u64 & 0xfffff)));
	out.push((at + 4, addi(r, r, lo as u64)));
}
fn two(a: u32, b: u32) -> Vec<(u64, u64)> {
	let mut v = Vec::new();
	emit_val(a, 1, 0x00, &mut v); // x1 = a (0x00, 0x04)
	emit_val(b, 2, 0x08, &mut v); // x2 = b (0x08, 0x0c)
	v
}
fn run_inst(inst: u64, rd: usize, a: u32, b: u32) -> u32 {
	let mut p = two(a, b);
	p.push((0x10, inst));
	run1(&p)[rd]
}
fn check(name: &str, rd: usize, a: u32, b: u32, inst: u64, want: u32) {
	let got = run_inst(inst, rd, a, b);
	assert_eq!(got, want, "{name}: rs1={a:08x} rs2={b:08x} got={got:08x} want={want:08x}");
}

const M1: u32 = 0xffff_ffff;
const X80: u32 = 0x8000_0000;
const BOUNDS: [(u32, u32); 5] = [(0, 0), (1, 1), (M1, M1), (0x7fff_ffff, 0x7fff_ffff), (X80, X80)];

#[test]
fn per_inst_r_type() {
	for &(a, _) in &BOUNDS {
		check("add 0", 3, a, 0, add(3, 1, 2), a.wrapping_add(0));
		check("add 1", 3, a, 1, add(3, 1, 2), a.wrapping_add(1));
		check("sub 0", 3, a, 0, sub(3, 1, 2), a.wrapping_sub(0));
		check("sub 1", 3, a, 1, sub(3, 1, 2), a.wrapping_sub(1));
		check("xor", 3, a, 0xffff_ffff, xor(3, 1, 2), a ^ 0xffff_ffff);
		check("or", 3, a, 0x00ff_00ff, or(3, 1, 2), a | 0x00ff_00ff);
		check("and", 3, a, 0x0f0f_0f0f, and(3, 1, 2), a & 0x0f0f_0f0f);
		check("sll 0", 3, a, 0, sll(3, 1, 2), a << 0);
		check("sll 31", 3, a, 31, sll(3, 1, 2), a << 31);
		check("srl 0", 3, a, 0, srl(3, 1, 2), a >> 0);
		check("srl 31", 3, a, 31, srl(3, 1, 2), a >> 31);
		check("sra 0", 3, a, 0, sra(3, 1, 2), ((a as i32) >> 0) as u32);
		check("sra 31", 3, a, 31, sra(3, 1, 2), ((a as i32) >> 31) as u32);
		check("slt", 3, a, 1, slt(3, 1, 2), ((a as i32) < 1) as u32); // signed compare
		check("sltu", 3, a, 1, sltu(3, 1, 2), (a < 1) as u32);
	}
	check("slt -1 < 1", 3, M1, 1, slt(3, 1, 2), 1);
	check("slt 1 < -1", 3, 1, M1, slt(3, 1, 2), 0);
	check("sltu -1 <u 1", 3, M1, 1, sltu(3, 1, 2), 0);
	check("sltu 1 <u -1", 3, 1, M1, sltu(3, 1, 2), 1);
}

#[test]
fn per_inst_i_type() {
	for &(a, _) in &BOUNDS {
		check("addi 1", 3, a, 0, addi(3, 1, 1), a.wrapping_add(1));
		check("addi -1", 3, a, 0, addi(3, 1, 0xfff), a.wrapping_add(M1));
		check("addi 0x7ff", 3, a, 0, addi(3, 1, 0x7ff), a.wrapping_add(0x7ff));
		check("xori", 3, a, 0, xori(3, 1, 0x0aa), a ^ 0x0000_00aa);
		check("xori -imm", 3, a, 0, xori(3, 1, 0xfff), a ^ 0xffff_ffff);
		check("ori", 3, a, 0, ori(3, 1, 0x0f0), a | 0x0f0);
		check("andi", 3, a, 0, andi(3, 1, 0x0ff), a & 0x0ff);
		check("slli 0", 3, a, 0, slli(3, 1, 0), a << 0);
		check("slli 31", 3, a, 0, slli(3, 1, 31), a << 31);
		check("srli 0", 3, a, 0, srli(3, 1, 0), a >> 0);
		check("srli 31", 3, a, 0, srli(3, 1, 31), a >> 31);
		check("srai 0", 3, a, 0, srai(3, 1, 0), ((a as i32) >> 0) as u32);
		check("srai 31", 3, a, 0, srai(3, 1, 31), ((a as i32) >> 31) as u32);
		check("slti", 3, a, 0, slti(3, 1, 0), ((a as i32) < 0) as u32); // signed imm compare
		check("sltiu", 3, a, 0, sltiu(3, 1, 0), (a < 0) as u32);
	}
	let mut p = two(5, 0);
	p.push((0x10, addi(0, 0, 7)));
	let r = run1(&p);
	assert_eq!(r[0], 0, "x0 hard zero");
	check("srli rd=16 bug path", 16, X80, 0, srli(16, 1, 31), 1);
	check("srai rd=16 bug path", 16, X80, 0, srai(16, 1, 31), M1);
}

#[test]
fn per_inst_u_j_branch() {
	for &(a, _) in &BOUNDS {
		check("lui", 3, a, 0, lhs_lui(3, 0xabcde), 0xabcde000);
		check("auipc", 3, a, 0, auipc(3, 0x00001), (0x10 + 0x00001000) & 0xffff_ffff);
	}
	let mut p = two(0, 0);
	p.push((0x10, jal(3, 0x24)));
	p.push((0x14, addi(4, 0, 99)));
	for i in 0..7 {
		p.push((0x18 + i * 4, addi(4, 0, 100)));
	}
	let r = run1(&p);
	assert_eq!(r[3], 0x14, "jal rd = pc+4");
	assert_eq!(r[4], 0, "jal taken: 99/100 path all skipped");
	let mut p2 = two(0, 0);
	p2.push((0x10, jalr(3, 1, 0x24)));
	let r2 = run1(&p2);
	assert_eq!(r2[3], 0x14, "jalr rd = pc+4");
	let mk = |x: u32, y: u32, b: u64| {
		let mut p = two(x, y);
		p.push((0x10, b));
		p.push((0x14, addi(4, 0, 1))); // fall-through marker (executed iff branch NOT taken)
		p.push((0x18, jal(0, 0xc4 - 0x18)));
		p
	}; // branch imm = 8: taken jumps over 0x14 to 0x18
	let r = run1(&mk(1, 2, beq(1, 2, 8)));
	assert_eq!(r[4], 1, "beq not-taken falls through");
	let r = run1(&mk(1, 1, beq(1, 1, 8)));
	assert_eq!(r[4], 0, "beq taken skips");
	let r = run1(&mk(1, 2, bne(1, 2, 8)));
	assert_eq!(r[4], 0, "bne taken");
	let r = run1(&mk(1, 1, blt(1, 1, 8)));
	assert_eq!(r[4], 1, "blt not-taken");
	let r = run1(&mk(2, 1, bltu(1, 2, 8))); // x1=2, x2=1: 2 <u 1 false
	assert_eq!(r[4], 1, "bltu not-taken (2 <u 1 false)");
	let r = run1(&mk(1, 2, bltu(1, 2, 8))); // x1=1, x2=2: 1 <u 2 true
	assert_eq!(r[4], 0, "bltu taken (1 <u 2 true)");
	let r = run1(&mk(1, 1, bge(1, 1, 8)));
	assert_eq!(r[4], 0, "bge taken");
	let r = run1(&mk(2, 1, bgeu(1, 2, 8))); // x1=2 >=u x2=1 true
	assert_eq!(r[4], 0, "bgeu taken");
}

#[test]
fn per_inst_load_store() {
	let mut prog = two(8, 0xdead_beef);
	prog.push((0x10, sw(2, 1, 0)));
	prog.push((0x14, lhs_lw(3, 1, 0)));
	let r = run1(&prog);
	assert_eq!(r[3], 0xdead_beef, "store->load round trip");
	let mut prog2 = two(0, 0);
	prog2.push((0x10, lhs_lw(3, 0, 24)));
	let r2 = run1(&prog2);
	assert_eq!(r2[3], 0, "lw default mem");
}

#[test]
fn per_inst_shift_shamt_boundaries() {
	for &(a, _) in &BOUNDS {
		check("slli 1", 3, a, 0, slli(3, 1, 1), a << 1);
		check("srai 1", 3, a, 0, srai(3, 1, 1), ((a as i32) >> 1) as u32);
	}
}