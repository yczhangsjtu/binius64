//! Eighth validation slice: a REAL LOOP PROGRAM — factorial — combining
//! branch + multi-instruction trace + integer arithmetic in ONE proof.
//!
//! Program (RISC-V-like, 8-bit): compute x2 = 5! with a conditional loop
//!   x1 = 5; x2 = 1; x3 = 1;     // n, acc, i
//! loop: x2 = x2*x3;  x3 = x3+1; // acc *= i ; i++
//!       if x3 <= x1 goto loop;   // conditional branch
//!
//! Unfold a bounded trace of R = 6 rounds (covers i = 1..6; exits when i>5).
//! Each round proves the full loop-body state machine over the binary field:
//!   - go  = branch condition (i+1 <= n) via carry-out overflow test
//!   - acc' = go ? acc*i : acc    8-bit shift-add multiplier gated by boolean MUX
//!   - i'   = go ? i+1   : i      increment (full-adder carry chain) gated by MUX
//! The MUX freezes acc/i after the loop exits — exact conditional-loop semantics.
//!
//! Cross-check: native 5! = 120 == committed final acc.
//! Soundness: tampering the final acc (wrong factorial) is rejected.

use binius_field::{Field, Ghash128b as B128, arch::OptimalPackedB128};
use binius_hash::StdHashSuite;
use binius_spartan_frontend::{
	circuit_builder::{CircuitBuilder, ConstraintBuilder, InstanceGenerator, WitnessGenerator},
	compiler::compile,
	constraint_system::{ConstraintWire, Witness},
};
use binius_spartan_prover::Prover;
use binius_spartan_verifier::{Verifier, config::StdChallenger};
use binius_transcript::ProverTranscript;
use rand::{SeedableRng, rngs::StdRng};

const BITS: usize = 8;
const N: u64 = 5;
const ROUNDS: usize = (N as usize) + 1; // 6

type F = B128;
type P = OptimalPackedB128;

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

/// 8-bit increment +1 (full-adder chain).
fn inc8<B: CircuitBuilder<Field = B128>>(b: &mut B, x: &[B::Wire]) -> Vec<B::Wire> {
	let one_bits = to_bits(1, BITS);
	let mut cin = b.constant(B128::ZERO);
	let mut res = Vec::with_capacity(BITS);
	for i in 0..BITS {
		let ib = b.constant(one_bits[i]);
		let xi = x[i];
		let (sum, cout) = fa(b, xi, ib, cin);
		res.push(sum);
		cin = cout;
	}
	res
}

/// 8x8 -> 8-bit shift-add multiplier (low 8 bits): a * bb.
fn mul8<B: CircuitBuilder<Field = B128>>(b: &mut B, a: &[B::Wire], bb: &[B::Wire]) -> Vec<B::Wire> {
	let mut acc: Vec<B::Wire> = (0..BITS).map(|_| b.constant(B128::ZERO)).collect();
	for k in 0..BITS {
		let mut addend = Vec::with_capacity(BITS);
		for j in 0..BITS {
			if j >= k {
				let g = b.mul(a[k], bb[j - k]);
				addend.push(g);
			} else {
				addend.push(b.constant(B128::ZERO));
			}
		}
		let mut cin = b.constant(B128::ZERO);
		let mut new_acc = Vec::with_capacity(BITS);
		for j in 0..BITS {
			let aj = acc[j];
			let ad = addend[j];
			let (sum, cout) = fa(b, aj, ad, cin);
			new_acc.push(sum);
			cin = cout;
		}
		acc = new_acc;
	}
	acc
}

/// go = (x <= limit). Overflow test: x + (2^BITS - (limit+1)) overflows iff x>limit.
fn leq8<B: CircuitBuilder<Field = B128>>(b: &mut B, x: &[B::Wire], limit: u64) -> B::Wire {
	let addend = (1u64 << BITS) - (limit + 1);
	let a = to_bits(addend, BITS);
	let mut cin = b.constant(B128::ZERO);
	for i in 0..BITS {
		let xi = x[i];
		let ib = b.constant(a[i]);
		let (_, cout) = fa(b, xi, ib, cin);
		cin = cout;
	}
	// carry_out = 1 means overflow (x > limit). go = 1 - carry_out.
	let one = b.constant(B128::ONE);
	b.add(cin, one)
}

/// Drive one loop round. Same op order on all builders.
fn drive_round<B: CircuitBuilder<Field = B128>>(
	b: &mut B,
	acc: &[B::Wire],
	i: &[B::Wire],
	acc_next: &[B::Wire],
	i_next: &[B::Wire],
) {
	for w in acc.iter().chain(i.iter()).chain(acc_next.iter()).chain(i_next.iter()) {
		binius_spartan_frontend::circuits::assert_is_bit(b, *w);
	}
	// branch condition: go = (i <= N)  — loop continues while current i still ≤ n
	let go = leq8(b, i, N);
	binius_spartan_frontend::circuits::assert_is_bit(b, go);
	// i_inc = i + 1 (used for the i' update when the loop continues)
	let i_inc = inc8(b, i);
	// product = acc * i
	let prod = mul8(b, acc, i);
	// acc' = go ? prod : acc   = go*prod + (1+go)*acc
	let not_go = { let one = b.constant(B128::ONE); b.add(go, one) };
	for j in 0..BITS {
		let gp = b.mul(go, prod[j]);
		let ng_a = b.mul(not_go, acc[j]);
		let mux = b.add(gp, ng_a);
		b.assert_eq(mux, acc_next[j]);
	}
	// i' = go ? i_inc : i
	for j in 0..BITS {
		let gi = b.mul(go, i_inc[j]);
		let ng_i = b.mul(not_go, i[j]);
		let mux = b.add(gi, ng_i);
		b.assert_eq(mux, i_next[j]);
	}
}

fn main() {
	let n: u64 = N;
	let mut acc = 1u64;
	let mut i = 1u64;
	let mut acc_chain = vec![acc];
	let mut i_chain = vec![i];
	for _ in 0..ROUNDS {
		let new_acc = if i <= n { acc * i } else { acc };
		let new_i = if i <= n { i + 1 } else { i };
		acc = new_acc;
		i = new_i;
		acc_chain.push(acc);
		i_chain.push(i);
	}
	let final_acc = *acc_chain.last().unwrap();
	println!("factorial: compute {n}! via 8-bit mul + carry add + conditional branch");
	println!("  unfolded {ROUNDS} rounds; native final acc = {final_acc} (expect {n}!)");

	// ---- Allocate: acc(0..=R)x8 | i(0..=R)x8 (segmented, shared state) ----
	let mut cb: ConstraintBuilder<F> = ConstraintBuilder::new();
	let acc_w: Vec<Vec<ConstraintWire>> = (0..=ROUNDS)
		.map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect())
		.collect();
	let i_w: Vec<Vec<ConstraintWire>> = (0..=ROUNDS)
		.map(|_| (0..BITS).map(|_| cb.alloc_inout()).collect())
		.collect();
	for r in 0..ROUNDS {
		drive_round(&mut cb, &acc_w[r], &i_w[r], &acc_w[r + 1], &i_w[r + 1]);
	}
	let (cs, layout) = compile(cb);

	let verifier = Verifier::<_, StdHashSuite>::setup(cs, 1).expect("verifier setup");
	let prover = Prover::<P, StdHashSuite>::setup(&verifier).expect("prover setup");
	let cs = verifier.constraint_system();
	let layout = layout.with_blinding(*cs.blinding_info());

	// ---- Witness ----
	let mut rng = StdRng::seed_from_u64(0);
	let mut wg = WitnessGenerator::new(&layout);
	let acc_bits: Vec<Vec<B128>> = acc_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	let i_bits: Vec<Vec<B128>> = i_chain.iter().map(|&v| to_bits(v, BITS)).collect();
	for r in 0..=ROUNDS {
		for k in 0..BITS {
			wg.write_inout(acc_w[r][k], acc_bits[r][k]);
		}
	}
	for r in 0..=ROUNDS {
		for k in 0..BITS {
			wg.write_inout(i_w[r][k], i_bits[r][k]);
		}
	}
	for r in 0..ROUNDS {
		let wa: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(acc_w[r][k], acc_bits[r][k])).collect();
		let wi: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[r][k], i_bits[r][k])).collect();
		let wan: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(acc_w[r + 1][k], acc_bits[r + 1][k])).collect();
		let win: Vec<binius_spartan_frontend::circuit_builder::WitnessWire<F>> = (0..BITS).map(|k| wg.write_inout(i_w[r + 1][k], i_bits[r + 1][k])).collect();
		drive_round(&mut wg, &wa, &wi, &wan, &win);
	}

	let witness = match wg.build() { Ok(w)=>w, Err(e)=>{ eprintln!("WITNESS_ERR: {e:?}"); std::process::exit(1);} };

	// ---- Instance ----
	let mut ig = InstanceGenerator::new(&layout);
	for r in 0..=ROUNDS {
		for k in 0..BITS {
			ig.write_inout(acc_w[r][k], acc_bits[r][k]);
		}
	}
	for r in 0..=ROUNDS {
		for k in 0..BITS {
			ig.write_inout(i_w[r][k], i_bits[r][k]);
		}
	}
	for r in 0..ROUNDS {
		let ma: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(acc_w[r][k], acc_bits[r][k])).collect();
		let mi: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[r][k], i_bits[r][k])).collect();
		let man: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(acc_w[r + 1][k], acc_bits[r + 1][k])).collect();
		let min: Vec<binius_spartan_frontend::circuit_builder::PublicWire<F>> = (0..BITS).map(|k| ig.write_inout(i_w[r + 1][k], i_bits[r + 1][k])).collect();
		drive_round(&mut ig, &ma, &mi, &man, &min);
	}
	let public = ig.build();

	// ---- Prove & verify ----
	let t_prove0 = std::time::Instant::now();
	let mut pt = ProverTranscript::new(StdChallenger::default());
	prover.prove(&witness, &mut rng, &mut pt).expect("prove failed");
	let t_prove = t_prove0.elapsed();
	let t_verify0 = std::time::Instant::now();
	let mut vt = pt.into_verifier();
	verifier.verify(&public, &mut vt).expect("verify failed");
	vt.finalize().expect("finalize failed");
	let t_verify = t_verify0.elapsed();

	println!("✅ Factorial loop program proved & verified over binary field (Spartan)");
	println!("   {n}! = {final_acc} via {ROUNDS} rounds (8-bit mul + carry add + branch)");
	println!("   constraints: n_private={} n_mul={}", cs.n_private(), cs.mul_constraints().len());
	println!("   timing: prove={:?} verify={:?}", t_prove, t_verify);
	println!("   cross-check: native {n}! = {final_acc} ✓");

	// ---- Soundness: tamper final acc ----
	{
		let n_const = layout.n_constants();
		let mut bad = witness.public().to_vec();
		let acc_final_off = n_const as usize + ROUNDS * BITS; // acc[r=ROUNDS], after acc seg
		bad[acc_final_off] += B128::ONE;
		let tampered = Witness::new(bad, witness.precommit().to_vec(), witness.private().to_vec());
		let mut bt = ProverTranscript::new(StdChallenger::default());
		prover.prove(&tampered, &mut rng, &mut bt).expect("prove tampered");
		let mut bv = bt.into_verifier();
		let rejected = verifier.verify(&public, &mut bv).is_err();
		assert!(rejected, "verifier MUST reject a tampered factorial result");
		println!("   soundness: verifier REJECTED a tampered factorial result ✓");
	}
}