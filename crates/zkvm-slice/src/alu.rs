//! Shared ALU / bit-level helpers for the zkVM validation crate.
//!
//! These were previously copy-pasted into ~12 of the slice binaries. Consolidating
//! them into one module removes that duplication and gives every slice a single
//! source of truth for the binary-field arithmetic building blocks.

use binius_field::{Field, Ghash128b as B128};
use binius_spartan_frontend::circuit_builder::CircuitBuilder;

/// Bits of `val`, LSB first, as field elements (used for bit-decomposed wires).
/// `nbits` is the bit width (e.g. 8 for our pedagogical data/PC width).
pub fn to_bits(val: u64, nbits: usize) -> Vec<B128> {
	(0..nbits).map(|i| B128::new(((val >> i) & 1) as u128)).collect()
}

/// 1-bit full adder returning (sum, carry_out). Works on any `CircuitBuilder`
/// (constraint / witness / instance) so the same op order lines up derived wires.
pub fn fa<B: CircuitBuilder<Field = B128>>(b: &mut B, a: B::Wire, bb: B::Wire, cin: B::Wire) -> (B::Wire, B::Wire) {
	let a_and_b = b.mul(a, bb);
	let a_and_c = b.mul(a, cin);
	let b_and_c = b.mul(bb, cin);
	let axb = b.add(a, bb);
	let sum = b.add(axb, cin);
	let c1 = b.add(a_and_b, a_and_c);
	let cout = b.add(c1, b_and_c);
	(sum, cout)
}

/// Assert every wire in `wires` is a bit (0/1). Correct projection of boolean
/// constraint; same op order as the builder's allocation.
pub fn assert_bits<B: CircuitBuilder<Field = B128>>(b: &mut B, wires: &[B::Wire]) {
	for &w in wires {
		binius_spartan_frontend::circuits::assert_is_bit(b, w);
	}
}

/// `x + c` via a full-adder chain, returning `out` bits. `c` is a u64 constant.
pub fn add_constant<B: CircuitBuilder<Field = B128>>(b: &mut B, x: &[B::Wire], c: u64) -> Vec<B::Wire> {
	let cb = to_bits(c, x.len());
	let mut cin = b.constant(B128::ZERO);
	let mut out = Vec::with_capacity(x.len());
	for i in 0..x.len() {
		let ib = b.constant(cb[i]);
		let xi = x[i];
		let (sum, cout) = fa(b, xi, ib, cin);
		out.push(sum);
		cin = cout;
	}
	out
}

/// 8-bit increment `x + 1` by a full-adder chain.
pub fn inc8<B: CircuitBuilder<Field = B128>>(b: &mut B, x: &[B::Wire]) -> Vec<B::Wire> {
	add_constant(b, x, 1)
}

/// 8x8 -> low-8-bit shift-add multiplier: `a * b`.
pub fn mul8<B: CircuitBuilder<Field = B128>>(b: &mut B, a: &[B::Wire], bb: &[B::Wire]) -> Vec<B::Wire> {
	let bits = a.len();
	let mut acc: Vec<B::Wire> = (0..bits).map(|_| b.constant(B128::ZERO)).collect();
	for k in 0..bits {
		let mut addend = Vec::with_capacity(bits);
		for j in 0..bits {
			if j >= k {
				addend.push(b.mul(a[k], bb[j - k]));
			} else {
				addend.push(b.constant(B128::ZERO));
			}
		}
		let mut cin = b.constant(B128::ZERO);
		let mut new_acc = Vec::with_capacity(bits);
		for j in 0..bits {
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

/// `go = (x <= limit)`. Overflow test: `x + (2^BITS - (limit+1))` overflows iff
/// `x > limit`; `go = 1 - carry_out`. Used for loop-branch conditions.
pub fn leq8<B: CircuitBuilder<Field = B128>>(b: &mut B, x: &[B::Wire], limit: u64) -> B::Wire {
	let bits = x.len();
	let addend = (1u64 << bits) - (limit + 1);
	let a = to_bits(addend, bits);
	let mut cin = b.constant(B128::ZERO);
	for i in 0..bits {
		let xi = x[i];
		let ib = b.constant(a[i]);
		let (_, cout) = fa(b, xi, ib, cin);
		cin = cout;
	}
	let one = b.constant(B128::ONE);
	b.add(cin, one)
}

/// Native `x ^ y` for the XORI instruction (char-2 linear). Kept as a small helper
/// for the instr_step cross-check.
pub fn native_xor(rs1: u64, imm: u64) -> u64 {
	rs1 ^ imm
}
