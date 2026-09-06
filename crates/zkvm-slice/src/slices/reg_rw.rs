//! Milestone M1: REGISTER read-write matrix (ReadWriteChecking) — TRUE version.
//!
//! This is the register read-write consistency argument, reworked per the
//! acceptance spec to put the HARD part (the version/temporal binding) into the
//! argument itself instead of a native precomputed time-snapshot table.
//!
//! Design (replaces the previous native time-ordered table `T[ts*NREG+reg]`):
//!   - A WRITE-LOG table `W[(reg, ver)] -> value`, built ONLY from write events
//!     (ver = 1 + number of prior writes to that register, so each write gets a
//!     fresh, monotonically increasing version for that register).
//!   - Each WRITE event is a looker: index = (reg, ver), claim = written value.
//!   - Each READ event is a looker: index = (reg, ver_at_read), claim = read
//!     value. Because the index is the register's CURRENT version at the read,
//!     logup* forces  read_value == W[(reg, ver_at_read)] — i.e. the read sees
//!     the value of the MOST RECENT write to that register. THAT is the proof of
//!     "read sees most-recent write".
//!
//! Program (3 live registers, shows read-after-write on x1):
//!   addi x1, x0, 5      # x1=5   (write x1 ver1)
//!   addi x2, x0, 3      # x2=3   (write x2 ver1)
//!   add  x1, x1, x2     # x1=8   (read x1@v1=5, x2@v1=3; write x1 ver2)
//!   addi x2, x0, 7      # x2=7   (write x2 ver2, overwrites 3)
//!   add  x1, x1, x2     # x1=15  (read x1@v2=8, x2@v2=7; write x1 ver3)
//!   addi x5, x1, 1      # x5=16  (read x1@v3=15; write x5 ver1)
//!
//! Expected: x1=15, x2=7, x5=16. Soundness: any read claiming a value that is
//! NOT the value of the register's current-version write must be REJECTED.
//! All three tamper cases are exercised: wrong-version value, tampered version
//! wire, never-written value.
//!
//! HONEST BOUNDARY (do not over-claim): the per-register `version` counter is
//! still computed by the native `run_program()` (`ver[reg] += 1`), NOT yet by a
//! Spartan constraint. logup* DOES force `read_value == W[(reg, version)]`
//! (the read==write binding, which is the proof that a read observes its
//! register's most-recent write), but it does NOT separately prove that the
//! version chain increments correctly (`ver[rd]' = ver[rd] + 1`) inside the
//! circuit. Making the version chain a real constraint requires a Spartan state
//! machine over the rows — out of scope for this standalone slice (M2 target).

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
/// Max writes to a single register (x1 is written 3x here). 4 is a power of 2,
/// so the write-log table is NREG * VER_MAX = 8 * 4 = 32 cells = 2^5.
const VER_MAX: usize = 4;

/// A register access event.
///
/// For a write: `version` = this registration's write index (1 = first write).
/// For a read:  `version` = the register's current version AT THE TIME of this
/// read (which is what the reader observes). Both use the same `reg`/`version`
/// to index the write-log table, so a read claims exactly the most-recent write.
#[derive(Clone, Copy)]
struct Access {
	reg: usize,
	version: usize,
	val: u64,
	is_write: bool,
}

/// Native execution: build the register state over time + the access trace.
/// Returns `(final_regs, per-access events)`.
///
/// NOTE: `version` here is computed natively only to NAME the write-log entries
/// (so we can build the table the same way the constraint would). The *proof*
/// that read_value == W[(reg, version)] happens in logup*, not in this loop.
fn run_program() -> ([u64; NREG], Vec<Access>) {
	let mut regs = [0u64; NREG];
	let mut ver = [0usize; NREG]; // per-register write count (current version)
	let mut acc: Vec<Access> = Vec::new();

	// helper to record a write (bumps the register's version)
	macro_rules! write_reg {
		($rd:expr, $val:expr) => {{
			let reg = $rd as usize;
			ver[reg] += 1;
			regs[reg] = $val;
			acc.push(Access { reg, version: ver[reg], val: $val, is_write: true });
		}};
	}
	// helper to record a read (version = register's current version at this point)
	macro_rules! read_reg {
		($rs:expr, $val:expr) => {{
			let reg = $rs as usize;
			acc.push(Access { reg, version: ver[reg], val: $val, is_write: false });
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
	// add x1, x1, x2 -> x1 = 8 + 7 = 15 (reads UPDATED x1@v2=8 and x2@v2=7)
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

/// Build the write-log table `W[(reg, ver)] = value` from the write events.
/// Cells for registers/versions never written hold 0 (registers start at 0).
fn build_write_log(acc: &[Access]) -> Vec<u64> {
	let table_size = NREG * VER_MAX;
	let mut w = vec![0u64; table_size];
	for a in acc {
		if a.is_write {
			let cell = a.reg * VER_MAX + a.version;
			w[cell] = a.val;
		}
	}
	w
}

/// The looker index for an access: (reg, version) packed into a table cell.
fn cell_of(a: &Access) -> usize {
	a.reg * VER_MAX + a.version
}

pub fn run_reg_rw() {
	let (regs, acc) = run_program();

	println!("== M1 (TRUE): Register read-write matrix — write-log + version binding ==");
	println!("   program: addi x1,5; addi x2,3; add x1,x1,x2; addi x2,7; add x1,x1,x2; addi x5,x1,1");
	println!("   final regs: x1={} x2={} x5={} (cross-check native)", regs[REG_X1 as usize], regs[REG_X2 as usize], regs[REG_X5 as usize]);
	println!("   access trace (reg, ver, val, op):");
	for a in &acc {
		println!("     reg=x{} ver={} val={} {}", a.reg, a.version, a.val, if a.is_write { "WRITE" } else { "READ " });
	}

	// ---- Write-log table W[(reg, ver)], built ONLY from write events ----
	let w = build_write_log(&acc);
	let w_table = FieldBuffer::from_values(&w.iter().map(|&v| F::from(v as u128)).collect::<Vec<_>>());
	let w_view = w_table.as_view();

	// ---- Lookers: every access (read or write) is a claim on W[(reg,ver)] ----
	let idxs: Vec<Vec<usize>> = acc.iter().map(|a| vec![cell_of(a)]).collect();
	let claims: Vec<F> = acc.iter().map(|a| F::from(a.val as u128)).collect();
	let n = idxs.len();
	let empties: Vec<[F; 0]> = (0..n).map(|_| []).collect();
	let lookers: Vec<Looker<F>> = (0..n)
		.map(|i| Looker { index: &idxs[i], eval_point: &empties[i] as &[F], eval_claim: claims[i] })
		.collect();

	println!("   write-log table W[reg*{} + ver], {} events", VER_MAX, n);

	// ---- Prove & verify (one transcript) ----
	let alloc = GlobalAllocator;
	let m = 5; // log2(NREG*VER_MAX) = log2(32) = 5
	let mut pt = ProverTranscript::new(StdChallenger::default());
	let gamma = IPProverChannel::<F>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
		&alloc, gamma,
		[binius_ip_prover::logup_star::TableLookup { table: w_view, lookers }],
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

	println!("   ✅ read==write binding verified: each read sees its register's most-recent write");

	// cross-check: native per-register committed values match trace.
	assert_eq!(regs[REG_X1 as usize], 15, "x1 should be 8+7=15");
	assert_eq!(regs[REG_X2 as usize], 7, "x2 should be 7 (overwritten 3)");
	assert_eq!(regs[REG_X5 as usize], 16, "x5 should be 15+1=16");
	println!("   cross-check: native x1=15 x2=7 x5=16 ✓");

	// ================= SOUNDNESS: three tamper cases =================
	// All use the existing rule: tamper a witness/public-derived value (or a
	// claim) WITHOUT re-driving / breaking the decode, then prove and verify.

	let _ = run_soundness_cases(&acc, &w, alloc);
}

/// Run the three required soundness rejection cases.
fn run_soundness_cases(acc: &[Access], w: &[u64], alloc: GlobalAllocator) -> u64 {
	// helper to prove+verify a given access list against the SAME write-log.
	fn try_verify(bad: &[Access], w: &[u64], alloc: GlobalAllocator) -> bool {
		let w_table = FieldBuffer::from_values(&w.iter().map(|&v| F::from(v as u128)).collect::<Vec<_>>());
		let w_view = w_table.as_view();
		let idxs: Vec<Vec<usize>> = bad.iter().map(|a| vec![cell_of(a)]).collect();
		let claims: Vec<F> = bad.iter().map(|a| F::from(a.val as u128)).collect();
		let ne = idxs.len();
		let empties: Vec<[F; 0]> = (0..ne).map(|_| []).collect();
		let lookers: Vec<Looker<F>> = (0..ne)
			.map(|i| Looker { index: &idxs[i], eval_point: &empties[i] as &[F], eval_claim: claims[i] })
			.collect();
		let mut bt = ProverTranscript::new(StdChallenger::default());
		let bg = IPProverChannel::<F>::sample(&mut bt);
		let _bo = binius_ip_prover::logup_star::prove::<GlobalAllocator, F, P>(
			&alloc, bg,
			[binius_ip_prover::logup_star::TableLookup { table: w_view, lookers }],
			&mut bt,
		);
		let mut btv = bt.into_verifier();
		let bvg = IPVerifierChannel::<F>::sample(&mut btv);
		logup_star::verify_reduction::<F, _>(
			&bvg,
			[logup_star::TableLookup {
				n_vars: 5,
				lookers: claims.iter().map(|&c| logup_star::LookerClaim {
					eval_point: &empties[0] as &[F],
					eval_claim: c,
				}).collect(),
			}],
			&mut btv,
		).is_err()
	}

	// Case (a): a READ claims a value of the WRONG version (in-table but stale/incorrect).
	//   Replace the read of x1@ver2 (which is 8) with the ver1 value 5. The looker
	//   index is still (x1, ver2), but the claim is now a value from a different
	//   version. W[(x1,ver2)] = 8 != 5 => reject.
	let mut a1: Vec<Access> = acc.to_vec();
	let mut ok = false;
	for a in a1.iter_mut() {
		if !a.is_write && a.reg == REG_X1 as usize && a.version == 2 && a.val == 8 {
			a.val = 5; // claim the ver-1 value 5 at ver-2 index -> WRONG version value
			ok = true;
			break;
		}
	}
	assert!(ok, "soundness-a: expected a read of x1@ver2=8 to tamper");
	let rej_a = try_verify(&a1, w, alloc);
	assert!(rej_a, "soundness-a: MUST reject a read claiming the wrong-version value");
	println!("   soundness(a): REJECTED read x1@v2 claiming ver-1 value 5 (wrong-version value) ✓");

	// Case (b): tamper the VERSION wire of a read (read with the WRONG version index).
	//   Take the read of x1 that observes 15 at ver3, but give it ver1's index
	//   (the cell that holds 5). Its claim 15 is not in W[(x1,ver1)]=5 => reject.
	let mut a2: Vec<Access> = acc.to_vec();
	let mut ok = false;
	for a in a2.iter_mut() {
		if !a.is_write && a.reg == REG_X1 as usize && a.version == 3 && a.val == 15 {
			a.version = 1; // wrong version wire: index (x1, ver1) instead of (x1,ver3)
			ok = true;
			break;
		}
	}
	assert!(ok, "soundness-b: expected a read of x1@v3=15 to tamper");
	let rej_b = try_verify(&a2, w, alloc);
	assert!(rej_b, "soundness-b: MUST reject a read using the wrong version wire");
	println!("   soundness(b): REJECTED read x1@v3 claiming value at wrong version-1 index ✓");

	// Case (c): a READ claims a value NEVER written to that register.
	//   Take the last read of x1 (15, ver3) and claim 99 (never written). The
	//   cell W[(x1,ver3)] = 15 != 99 => reject.
	let mut a3: Vec<Access> = acc.to_vec();
	let mut ok = false;
	for a in a3.iter_mut() {
		if !a.is_write && a.reg == REG_X1 as usize && a.val == 15 {
			a.val = 99; // never written
			ok = true;
			break;
		}
	}
	assert!(ok, "soundness-c: expected a read of x1=15 to tamper");
	let rej_c = try_verify(&a3, w, alloc);
	assert!(rej_c, "soundness-c: MUST reject a read claiming a never-written value");
	println!("   soundness(c): REJECTED read x1 claiming never-written value 99 ✓");

	// Positive check (control): the UNTAMPERED list must verify (already proven above).
	0
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn reg_rw() {
		run_reg_rw();
	}
}
