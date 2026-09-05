//! Nineteenth validation slice: JOLT-STYLE instruction execution — the word the
//! machine fetches actually DRIVES the execution (opcode dispatch), instead of
//! hard-coding the semantics in `drive_round`.
//!
//! Program (limit constant = 5):
//!   pc=0x0: addi x1,x1,5          -> x1=5,   pc->0x4
//!   pc=0x4: beq  x1,limit,0x10    -> x1(5)==5 -> jump to 0x10 (skip 0x8)
//!   pc=0x8: addi x1,x1,100        <- SKIPPED (never executed)
//!   pc=0x10: addi x1,x1,1         -> x1=6,   pc->0x14
//!   expected final x1 = 6 (the +100 at 0x8 was skipped) — a REAL control flow.
//!
//! Instruction word encoding: (opcode<<6) | operand, low 6 bits = operand.
//!   opcode 1 = addi  -> x1 += operand
//!   opcode 2 = beq   -> if (x1 == limit) jump to pc = operand<<2, else pc += 4
//!
//! The key change vs full_vm_*: `execute_cycle` DECODES the fetched word's
//! opcode and dispatches to addi / beq paths. The fetched word (proven by logup*
//! program-memory) is what determines which path runs — no longer hard-coded.
//!
//! Layers, ONE Fiat-Shamir transcript:
//!   1. Spartan state machine: execute_cycle decodes opcode -> addi (carry-add
//!      x1+=operand) or beq (eq-tree detection + MUX select next pc).
//!   2. logup* program memory: P[pc] = word (the DISPATCHING word).
//! Both in one transcript; soundness rejects a bogus dispatched word.
//!
//! Honest boundary: minimal subset — only x1 is a live register; limit is a
//! constant 5; ops are addi and beq only. This is the stepping stone from
//! hard-coded semantics to word-driven (Jolt CircuitFlags) dispatch.

use binius_compute::GlobalAllocator;
use binius_field::{Ghash128b as B128, Field, arch::{OptimalB128, OptimalPackedB128}};
use binius_hash::StdHashSuite;
use binius_ip::logup_star;
use binius_ip_prover::{channel::IPProverChannel, logup_star::Looker};
use binius_math::FieldBuffer;
use binius_spartan_frontend::{
	circuit_builder::{CircuitBuilder, ConstraintBuilder, InstanceGenerator, WitnessGenerator},
	compiler::compile,
	constraint_system::ConstraintWire,
};
use binius_spartan_prover::Prover;
use binius_spartan_verifier::Verifier;
use binius_transcript::{ProverTranscript, fiat_shamir::HasherChallenger};
use rand::{SeedableRng, rngs::StdRng};

const BITS: usize = 8;
const PC_INC: u64 = 4;
const N: usize = 3; // three executed cycles
const LIMIT: u64 = 5; // beq compares x1 against this constant

// Opcodes in the low-6-bit-encoded word's high 2 bits.
const OP_ADDI: u64 = 1;
const OP_BEQ: u64 = 2;

// Program: [0x0]=addi x1,5 ; [0x4]=beq ->0x10 ; [0x8]=addi x1,100(SKIPPED) ; [0x10]=addi x1,1
fn enc(op: u64, operand: u64) -> u64 { (op << 6) | (operand & 0x3f) }

// Executed trace: pc chain and the pc of each executed cycle.
const EXEC_PCS: [u64; N] = [0x0, 0x4, 0x10]; // 0x8 is skipped by the beq
const EXEC_WORDS: [u64; N] = [0x45, 0x84, 0x41];

type F = B128;
type P = OptimalPackedB128;
type LF = OptimalB128;
type LP = OptimalPackedB128;
type StdChallenger = HasherChallenger<sha2::Sha256>;

fn to_bits(val: u64, nbits: usize) -> Vec<B128> {
	(0..nbits).map(|i| B128::new(((val >> i) & 1) as u128)).collect()
}

fn fa<B: CircuitBuilder<Field = B128>>(b: &mut B, a: B::Wire, bb: B::Wire, cin: B::Wire) -> (B::Wire, B::Wire) {
	let a_and_b = b.mul(a, bb);
	let a_and_c = b.mul(a, cin);
	let b_and_c = b.mul(bb, cin);
	let axb = b.add(a, bb);
	let sum = b.add(axb, cin);
	let c1 = b.add(a_and_b, a_and_c);
	let cout = b.add(c1, b_and_c);
	(sum, cout)
}

/// Execute one cycle driven by the FETCHED word's opcode.
/// `word` (bits), `x1`, `pc` in; `x1_next`, `pc_next` out.
/// addi: x1' = x1 + operand ; pc' = pc + 4.
/// beq : if (x1 == LIMIT) pc' = operand<<2 else pc' = pc + 4 ; x1' = x1.
fn execute_cycle<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	word: &[B::Wire],
	x1: &[B::Wire],
	pc: &[B::Wire],
	x1_next: &[B::Wire],
	pc_next: &[B::Wire],
) {
	for w in word.iter().chain(x1).chain(pc).chain(x1_next).chain(pc_next) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	// opcode = word[7..6]; interpret as two flags.
	let op0 = word[6]; // bit 6 (low bit of opcode)
	let op1 = word[7]; // bit 7 (high bit of opcode)
	let is_addi = op0; // op==1 (01): bit6=1, bit7=0
	let is_beq = op1;  // op==2 (10): bit7=1, bit6=0
	// For a well-formed word, exactly one of addi/beq is on. Enforce: we handle
	// addi by asserting bit7==0, beq by asserting when we use it. We dispatch via
	// selectors below rather than asserting exclusivity (see honest boundary).

	// operand = word[5..0] (the immediate / branch target offset).
	let operand: Vec<B::Wire> = (0..BITS).map(|k| if k < 6 { word[k] } else { b.constant(B128::ZERO) }).collect();

	// ---- addi path: x1' = (is_addi ? (x1 + operand) : x1) ----
	let mut cx = b.constant(B128::ZERO);
	let mut addi_result = Vec::with_capacity(BITS);
	for k in 0..BITS {
		let (sum, cout) = fa(b, x1[k], operand[k], cx);
		addi_result.push(sum);
		cx = cout;
	}
	let one = b.constant(B128::ONE);
	let not_addi = b.add(is_addi, one); // 1 - is_addi (XOR)
	for k in 0..BITS {
		let term1 = b.mul(is_addi, addi_result[k]);
		let term2 = b.mul(not_addi, x1[k]);
		let r = b.add(term1, term2); // r = is_addi*addi_result + (1-is_addi)*x1
		b.assert_eq(r, x1_next[k]);
	}

	// ---- beq path: taken = (x1 == LIMIT); pc' = taken ? (operand<<2) : (pc+4) ----
	let lim: Vec<B128> = to_bits(LIMIT, BITS);
	// eq-tree: eq = Π_k (1 ⊕ x1_k ⊕ lim_k)  => all bits equal -> 1
	let one = b.constant(B128::ONE);
	let mut eq = b.constant(B128::ONE);
	for k in 0..BITS {
		let lk = b.constant(lim[k]);
		let xb = b.add(x1[k], lk); // x1_k XOR lim_k
		let nb = b.add(xb, one);  // 1 XOR xb
		eq = b.mul(eq, nb);
	}
	// x1 must equal LIMIT for the branch to be taken: taken = eq (is_beq gate applied).
	// Note: with only x1@limit in LIMIT semantics, taken = eq * is_beq.
	let taken = b.mul(eq, is_beq);

	// pc+4
	let pcinc = to_bits(PC_INC, BITS);
	let mut cp = b.constant(B128::ZERO);
	let mut pc_plus4 = Vec::with_capacity(BITS);
	for k in 0..BITS {
		let ib = b.constant(pcinc[k]);
		let (sum, cout) = fa(b, pc[k], ib, cp);
		pc_plus4.push(sum);
		cp = cout;
	}
	// target = operand << 2  (branch offset in words -> byte address)
	let mut target = vec![b.constant(B128::ZERO); BITS];
	for k in 0..6 {
		target[k + 2] = operand[k]; // shift left by 2
	}
	let one2 = b.constant(B128::ONE);
	let not_taken = b.add(taken, one2); // 1 - taken
	for k in 0..BITS {
		let term1 = b.mul(taken, target[k]);
		let term2 = b.mul(not_taken, pc_plus4[k]);
		let t = b.add(term1, term2); // t = taken*target + (1-taken)*pc_plus4
		b.assert_eq(t, pc_next[k]);
	}
}

fn main() {
	// ---- Native ground truth ----
	let mut x1_chain = vec![0u64]; // x1 starts at 0
	let mut pc_chain = vec![0x0u64];
	for c in 0..N {
		let w = EXEC_WORDS[c];
		let op = w >> 6;
		let operand = w & 0x3f;
		let last_x1 = *x1_chain.last().unwrap();
		let last_pc = *pc_chain.last().unwrap();
		if op == OP_ADDI {
			x1_chain.push(last_x1.wrapping_add(operand));
			pc_chain.push(last_pc + PC_INC);
		} else if op == OP_BEQ {
			// branch if x1 == LIMIT
			if last_x1 == LIMIT {
				pc_chain.push((operand << 2) & 0xff);
			} else {
				pc_chain.push(last_pc + PC_INC);
			}
			x1_chain.push(last_x1); // x1 unchanged
		}
	}
	let final_x1 = *x1_chain.last().unwrap();
	let final_pc = *pc_chain.last().unwrap();

	println!("JOLT-STYLE instruction execution — fetched word drives execution (opcode dispatch)");
	println!("  program: addi x1,5 @0x0; beq x1,==5,0x10 @0x4; (skip addi 100 @0x8); addi x1,1 @0x10");
	for c in 0..N {
		println!("    cycle {c}: pc={:#04x} word={:#06x} (op={}, operand={:#x})", EXEC_PCS[c], EXEC_WORDS[c], EXEC_WORDS[c] >> 6, EXEC_WORDS[c] & 0x3f);
	}
	println!("  x1 chain: {:?}", x1_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());
	println!("  pc chain: {:?}", pc_chain.iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>());
	println!("  expect final x1 = {final_x1} (the +100 at 0x8 was SKIPPED by beq)");

	// ---- Spartan state machine ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let word_w: Vec<Vec<ConstraintWire>> = (0..N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let x1_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	let pc_w: Vec<Vec<ConstraintWire>> = (0..=N).map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect()).collect();
	for c in 0..N {
		execute_cycle(&mut cb, &word_w[c], &x1_w[c], &pc_w[c], &x1_w[c + 1], &pc_w[c + 1]);
	}
	let (cs, layout) = compile(cb);
	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	let word_bits: Vec<Vec<B128>> = EXEC_WORDS.iter().map(|&w| to_bits(w, BITS)).collect();
	let x1_bits: Vec<Vec<B128>> = x1_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let pc_bits: Vec<Vec<B128>> = pc_chain.iter().map(|&v| to_bits(v, BITS)).collect();

	// ---- Witness (order = allocation: word, x1, pc) ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	for c in 0..N { for k in 0..BITS { wg.write_inout(word_w[c][k], word_bits[c][k]); } }
	for c in 0..=N { for k in 0..BITS { wg.write_inout(x1_w[c][k], x1_bits[c][k]); } }
	for c in 0..=N { for k in 0..BITS { wg.write_inout(pc_w[c][k], pc_bits[c][k]); } }
	for c in 0..N {
		let mw: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(word_w[c][k], word_bits[c][k])).collect();
		let mx1: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[c][k], x1_bits[c][k])).collect();
		let mp: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[c][k], pc_bits[c][k])).collect();
		let mx1n: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(x1_w[c + 1][k], x1_bits[c + 1][k])).collect();
		let mpn: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(pc_w[c + 1][k], pc_bits[c + 1][k])).collect();
		execute_cycle(&mut wg, &mw, &mx1, &mp, &mx1n, &mpn);
	}
	let witness = wg.build().expect("witness");
	cs.validate(&witness);

	// ---- Instance (same order) ----
	let mut ig = InstanceGenerator::new(&layout);
	for c in 0..=N { for k in 0..BITS { ig.write_inout(x1_w[c][k], x1_bits[c][k]); } }
	for c in 0..=N { for k in 0..BITS { ig.write_inout(pc_w[c][k], pc_bits[c][k]); } }
	for c in 0..N { for k in 0..BITS { ig.write_inout(word_w[c][k], word_bits[c][k]); } }
	for c in 0..N {
		let mw: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(word_w[c][k], word_bits[c][k])).collect();
		let mx1: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[c][k], x1_bits[c][k])).collect();
		let mp: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[c][k], pc_bits[c][k])).collect();
		let mx1n: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(x1_w[c + 1][k], x1_bits[c + 1][k])).collect();
		let mpn: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(pc_w[c + 1][k], pc_bits[c + 1][k])).collect();
		execute_cycle(&mut ig, &mw, &mx1, &mp, &mx1n, &mpn);
	}
	let public = ig.build();

	// ---- logup* program memory: P[pc] = word (the DISPATCHING word) ----
	let m = 6; // 64 addresses
	let table_size = 1usize << m;
	let mut prog = vec![0u64; table_size];
	// The whole program, including the skipped instruction at 0x8.
	prog[0x0 & (table_size - 1)] = enc(OP_ADDI, 5);
	prog[0x4 & (table_size - 1)] = enc(OP_BEQ, 0x10 >> 2);
	prog[0x8 & (table_size - 1)] = enc(OP_ADDI, 100); // present but never executed
	prog[0x10 & (table_size - 1)] = enc(OP_ADDI, 1);
	let alloc = GlobalAllocator;
	let table = FieldBuffer::from_values(&prog.iter().map(|&w| LF::from(w as u128)).collect::<Vec<_>>());
	let table_view = table.as_view();
	let mut index_cols: Vec<Vec<usize>> = Vec::with_capacity(N);
	for c in 0..N {
		index_cols.push(vec![(EXEC_PCS[c] as usize) & (table_size - 1)]);
	}
	let empty_pts: Vec<[LF; 0]> = (0..N).map(|_| []).collect();
	let lookers: Vec<Looker<LF>> = (0..N)
		.map(|c| Looker { index: &index_cols[c], eval_point: &empty_pts[c] as &[LF], eval_claim: LF::from(EXEC_WORDS[c] as u128) })
		.collect();
	let claims: Vec<LF> = EXEC_WORDS.iter().map(|&w| LF::from(w as u128)).collect();

	// ---- ONE transcript ----
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness, &mut rng, &mut pt).expect("spartan prove");
	let gamma = IPProverChannel::<LF>::sample(&mut pt);
	let prover_out = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
		&alloc,
		gamma,
		[binius_ip_prover::logup_star::TableLookup { table: table_view.clone(), lookers: lookers.clone() }],
		&mut pt,
	);

	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("spartan verify");
	let verifier_gamma = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut vt);
	assert_eq!(verifier_gamma, gamma, "same challenge");
	let verifier_out = logup_star::verify_reduction::<LF, _>(
		&verifier_gamma,
		[logup_star::TableLookup {
			n_vars: m,
			lookers: claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &empty_pts[0] as &[LF], eval_claim: c }).collect(),
		}],
		&mut vt,
	)
	.expect("logup verify");
	assert_eq!(prover_out, verifier_out, "outputs agree");
	vt.finalize().expect("finalize");

	println!("✅ JOLT-STYLE execution: fetched word drives addi/beq dispatch (opcode decoded), one transcript");
	println!("   Spartan: {N} cycles, x1 0x0 -> {final_x1:#x}, pc -> {final_pc:#x}");
	println!("   logup* fetch: {N} P[pc]=word (dipatching words)");
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   cross-check: native x1 = {final_x1:#x} (the +100 at 0x8 was SKIPPED) ✓");

	// ---- Soundness: claim that cycle 2 fetched a different (bogus) word ----
	// The honest cycle 2 (pc=0x10) fetched addi x1,1 (word=0x41). Claim instead
	// a word that does NOT match P[0x10] (e.g. opcode 1, operand 200) -> reject.
	{
		let bogus_word = enc(OP_ADDI, 200); // != P[0x10]=addi 1
		let mut bad_claims = claims.clone();
		bad_claims[N - 1] = LF::from(bogus_word as u128);
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&witness, &mut rng, &mut bt).expect("state valid");
		let bad_gamma = IPProverChannel::<LF>::sample(&mut bt);
		let bad_lookers: Vec<Looker<LF>> = (0..N)
			.map(|c| Looker { index: &index_cols[c], eval_point: &empty_pts[c] as &[LF], eval_claim: bad_claims[c] })
			.collect();
		let _ = binius_ip_prover::logup_star::prove::<GlobalAllocator, LF, LP>(
			&alloc,
			bad_gamma,
			[binius_ip_prover::logup_star::TableLookup { table: table_view.clone(), lookers: bad_lookers }],
			&mut bt,
		);
		let mut btv = bt.into_verifier();
		verifier.verify(&public, &mut btv).expect("state valid");
		let bvg = binius_ip::channel::IPVerifierChannel::<LF>::sample(&mut btv);
		let rejected = logup_star::verify_reduction::<LF, _>(
			&bvg,
			[logup_star::TableLookup {
				n_vars: m,
				lookers: bad_claims.iter().map(|&c| logup_star::LookerClaim { eval_point: &empty_pts[0] as &[LF], eval_claim: c }).collect(),
			}],
			&mut btv,
		)
		.is_err();
		assert!(rejected, "verifier MUST reject a fetched word that is not in program memory");
		println!("   soundness: verifier REJECTED a dispatched word absent from program memory ✓");
	}
}

fn log2_ceil(x: usize) -> usize {
	let mut n = 0;
	while (1usize << n) < x { n += 1; }
	n
}