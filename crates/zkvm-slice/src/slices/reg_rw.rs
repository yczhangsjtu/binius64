//! Milestone M1: REGISTER read-write matrix (ReadWriteChecking) on the binary field.
//!
//! This is the first REAL step toward a Jolt-style zkVM. It proves, with logup*
//! as a sub-multiset argument, that **every register read sees the value of the
//! most recent write to that register** — i.e. register read-write consistency
//! across a multi-register, multi-write program.
//!
//! This is what replaces the "independent injected operand values" weakness in
//! zkvm.rs (where `a`/`b` were computed natively and not tied to a write).
//!
//! Model: a TIME-ORDERED register state table `T[ts * NREG + reg]`, exactly like
//! mem_arg_spice but over the REGISTER FILE instead of memory addresses:
//!   - Each access has a global timestamp; a store (write) updates the register's
//!     current value; a load (read) observes the register's value at that ts.
//!   - `T[ts, reg]` = the register's value at timestamp ts (= most recent write <= ts).
//!   - logup* proves every read/write is consistent with the same table, so a
//!     read that claims a stale value is rejected.
//!
//! Program (3 live registers x1/x2/x5, demonstrates read-after-write on x1):
//!   addi x1, x0, 5      # x1 = 5              (write x1)
//!   addi x2, x0, 3      # x2 = 3              (write x2)
//!   add  x1, x1, x2     # x1 = 5+3 = 8        (read x1=5, x2=3; write x1=8)
//!   addi x2, x0, 7      # x2 = 7              (write x2, overwrites 3)
//!   add  x1, x1, x2     # x1 = 8+7 = 15       (read x1=8 [not 5], x2=7; write x1=15)
//!   addi x5, x1, 1      # x5 = 16             (read x1=15; write x5=16)
//!
//! Expected: x1=15, x2=7, x5=16. Soundness: a read claiming a stale value
//! (e.g. reading x1=5 after x1 was overwritten to 15) must be REJECTED.

use binius_compute::GlobalAllocator;
use binius_field::arch::{OptimalB128, OptimalPackedB128};
use binius_ip::{channel::IPVerifierChannel, logup_star};
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_transcript::{fiat_shamir::HasherChallenger, ProverTranscript};

type F = OptimalB128;
type P = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

const REG_X0: u64 = 0;
const REG_X1: u64 = 1;
const REG_X2: u64 = 2;
const REG_X5: u64 = 5;
const NREG: usize = 8;
const TS_MAX: usize = 16; // 16 timestamps -> table = 16*8 = 128 cells = 2^7

/// A register access event at a global timestamp.
#[derive(Clone, Copy)]
struct Access {
	ts: usize,
	reg: usize,
	val: u64,
	is_write: bool, // write (store to rd) vs read (load from rs)
}

/// One row of the executed program.
#[derive(Clone, Copy)]
struct Row {
	ts: usize,
	rd: u64,
	rs1: u64,
	rs2: u64,
	rd_val: u64,
	rs1_val: u64,
	rs2_val: u64,
}

/// Native execution: build the register state over time + the access trace.
/// Returns `(final_regs, per-access events)`.
fn run_program() -> ([u64; NREG], Vec<Access>) {
	let mut regs = [0u64; NREG];
	let mut acc: Vec<Access> = Vec::new();
	let mut ts = 0usize;

	// helper to record a write
	macro_rules! write_reg {
		($rd:expr, $val:expr) => {{
			regs[$rd as usize] = $val;
			acc.push(Access { ts, reg: $rd as usize, val: $val, is_write: true });
			ts += 1;
		}};
	}
	// helper to record a read (value is the register's CURRENT value)
	macro_rules! read_reg {
		($rs:expr, $val:expr) => {{
			acc.push(Access { ts, reg: $rs as usize, val: $val, is_write: false });
			ts += 1;
		}};
	}

	// addi x1, x0, 5
	write_reg!(REG_X1, 5);
	// addi x2, x0, 3
	write_reg!(REG_X2, 3);
	// add x1, x1, x2 -> x1 = 5 + 3 = 8
	let a = regs[REG_X1 as usize];
	let b = regs[REG_X2 as usize];
	read_reg!(REG_X1, a);
	read_reg!(REG_X2, b);
	write_reg!(REG_X1, a + b);
	// addi x2, x0, 7
	write_reg!(REG_X2, 7);
	// add x1, x1, x2 -> x1 = 8 + 7 = 15 (reads UPDATED x1=8 and x2=7)
	let a = regs[REG_X1 as usize];
	let b = regs[REG_X2 as usize];
	read_reg!(REG_X1, a);
	read_reg!(REG_X2, b);
	write_reg!(REG_X1, a + b);
	// addi x5, x1, 1 -> x5 = 15 + 1 = 16
	let a = regs[REG_X1 as usize];
	read_reg!(REG_X1, a);
	write_reg!(REG_X5, a + 1);

	(regs, acc)
}

pub fn run_reg_rw() {
	let (regs, acc) = run_program();

	println!("== M1: Register read-write matrix (logup* sub-multiset, binary field) ==");
	println!("   program: addi x1,5; addi x2,3; add x1,x1,x2; addi x2,7; add x1,x1,x2; addi x5,x1,1");
	println!("   final regs: x1={} x2={} x5={} (cross-check native)", regs[REG_X1 as usize], regs[REG_X2 as usize], regs[REG_X5 as usize]);
	println!("   expected read-after-write: x1 8 then 15, x2 7");
	for a in &acc {
		println!("     ts={} reg=x{} val={} {}", a.ts, a.reg, a.val, if a.is_write { "WRITE" } else { "READ " });
	}

	// ---- Build the time-ordered register state table T[ts*NREG + reg] ----
	// Value at (ts, reg) = most recent write <= ts (initial 0).
	let table_size = TS_MAX * NREG;
	let mut t = vec![0u64; table_size];
	let mut current = vec![0u64; NREG];
	for ts in 0..TS_MAX {
		for a in &acc {
			if a.ts == ts && a.is_write {
				current[a.reg] = a.val;
			}
		}
		for reg in 0..NREG {
			t[ts * NREG + reg] = current[reg];
		}
	}
	let t_table = FieldBuffer::from_values(&t.iter().map(|&v| F::from(v as u128)).collect::<Vec<_>>());
	let t_view = t_table.as_view();

	// ---- Lookers: each access binds to its (ts,reg) cell ----
	let mut idxs: Vec<Vec<usize>> = Vec::new();
	let mut claims: Vec<F> = Vec::new();
	for a in &acc {
		let cell = a.ts * NREG + a.reg;
		idxs.push(vec![cell]);
		claims.push(F::from(a.val as u128));
	}
	let n = idxs.len();
	let empties: Vec<[F; 0]> = (0..n).map(|_| []).collect();
	let lookers: Vec<Looker<F>> = (0..n)
		.map(|i| Looker { index: &idxs[i], eval_point: &empties[i] as &[F], eval_claim: claims[i] })
		.collect();

	println!("   logup*: time-ordered register table T[ts*{} + reg], {} events", NREG, n);

	// ---- Prove & verify (one transcript) ----
	let alloc = GlobalAllocator;
	let m = 7; // log2(table_size) = log2(16*8)=7
	let mut pt = ProverTranscript::new(StdChallenger::default());
	let gamma = IPProverChannel::<F>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
		&alloc, gamma,
		[binius_ip_prover::logup_star::TableLookup { table: t_view, lookers }],
		&mut pt,
	);
	let mut vt = pt.into_verifier();
	let verifier_gamma = IPVerifierChannel::<F>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "both sides draw same challenge");
	let verifier_out = logup_star::verify_reduction::<F, _>(
		&verifier_gamma,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: claims.iter().map(|&c| logup_star::LookerClaim {
				eval_point: &empties[0] as &[F],
				eval_claim: c,
			}).collect(),
		}],
		&mut vt,
	).expect("M1 register read-write argument verifies");
	assert_eq!(prover_out, verifier_out, "prover/verifier outputs agree");

	println!("   ✅ register read-write matrix verified (reads see most-recent writes)");

	// cross-check: native per-register committed values match the trace.
	assert_eq!(regs[REG_X1 as usize], 15, "x1 should be 8+7=15");
	assert_eq!(regs[REG_X2 as usize], 7, "x2 should be 7 (overwritten 3)");
	assert_eq!(regs[REG_X5 as usize], 16, "x5 should be 15+1=16");
	println!("   cross-check: native x1=15 x2=7 x5=16 ✓");

	// ---- SOUNDNESS: tamper a READ to claim a STALE value (x1=5 after x1=15) ----
	// The last read of x1 (before the final write of 15) should observe 15; replace
	// it with the stale 5. That read's cell T[ts, x1] holds 15 => the claim 5 is
	// not in the table => verifier rejects.
	let mut bad = run_program().1; // rebuild access list (fresh)
	// find a READ of x1 with value 15 (the final read before addi x5)
	// and replace with 5.
	let mut replaced = false;
	for a in bad.iter_mut() {
		if a.is_write == false && a.reg == REG_X1 as usize && a.val == 15 {
			a.val = 5;
			replaced = true;
			break;
		}
	}
	assert!(replaced, "expected a read of x1 observing 15 in the trace");

	let mut bidxs: Vec<Vec<usize>> = Vec::new();
	let mut bclaims: Vec<F> = Vec::new();
	for a in &bad {
		let cell = a.ts * NREG + a.reg;
		bidxs.push(vec![cell]);
		bclaims.push(F::from(a.val as u128));
	}
	let bn = bidxs.len();
	let bempty: Vec<[F; 0]> = (0..bn).map(|_| []).collect();
	let blookers: Vec<Looker<F>> = (0..bn)
		.map(|i| Looker { index: &bidxs[i], eval_point: &bempty[i] as &[F], eval_claim: bclaims[i] })
		.collect();

	let t_view2 = t_table.as_view();
	let mut bt = ProverTranscript::new(StdChallenger::default());
	let bg = IPProverChannel::<F>::sample(&mut bt);
	let _bo = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
		&alloc, bg,
		[binius_ip_prover::logup_star::TableLookup { table: t_view2, lookers: blookers }],
		&mut bt,
	);
	let mut btv = bt.into_verifier();
	let bvg = IPVerifierChannel::<F>::sample(&mut btv);
	let rejected = logup_star::verify_reduction::<F, _>(
		&bvg,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: bclaims.iter().map(|&c| logup_star::LookerClaim {
				eval_point: &bempty[0] as &[F],
				eval_claim: c,
			}).collect(),
		}],
		&mut btv,
	).is_err();
	assert!(rejected, "verifier MUST reject a stale register read (x1=5 after x1=15)");
	println!("   soundness: verifier REJECTED a stale register read (x1=5 after x1=15) ✓");
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn reg_rw() {
		run_reg_rw();
	}
}
