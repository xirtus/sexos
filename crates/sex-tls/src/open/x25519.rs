// SPDX-License-Identifier: GPL-3.0-only
//! X25519 (RFC 7748) on fiat-crypto's verified field arithmetic. Plain 64-bit integer
//! code, so the network service needs no vector registers for it, and constant time:
//! the ladder swaps with masks, never branches on the secret.
use fiat_crypto::curve25519_64::*;

type Tight = fiat_25519_tight_field_element;
type Loose = fiat_25519_loose_field_element;

fn relax(a: &Tight) -> Loose { let mut o = fiat_25519_loose_field_element([0; 5]); fiat_25519_relax(&mut o, a); o }
fn add(a: &Tight, b: &Tight) -> Loose { let mut o = fiat_25519_loose_field_element([0; 5]); fiat_25519_add(&mut o, a, b); o }
fn sub(a: &Tight, b: &Tight) -> Loose { let mut o = fiat_25519_loose_field_element([0; 5]); fiat_25519_sub(&mut o, a, b); o }
fn mul(a: &Loose, b: &Loose) -> Tight { let mut o = fiat_25519_tight_field_element([0; 5]); fiat_25519_carry_mul(&mut o, a, b); o }
fn square(a: &Loose) -> Tight { let mut o = fiat_25519_tight_field_element([0; 5]); fiat_25519_carry_square(&mut o, a); o }
fn swap(bit: u8, a: &mut Tight, b: &mut Tight) {
    let (x, y) = (a.0, b.0);
    let (mut p, mut q) = ([0u64; 5], [0u64; 5]);
    fiat_25519_selectznz(&mut p, bit, &x, &y);
    fiat_25519_selectznz(&mut q, bit, &y, &x);
    a.0 = p; b.0 = q;
}
/// a^(p-2), the inverse, by the usual addition chain.
fn invert(z: &Tight) -> Tight {
    let sq_n = |a: &Tight, n: usize| { let mut t = *a; for _ in 0..n { t = square(&relax(&t)); } t };
    let m = |a: &Tight, b: &Tight| mul(&relax(a), &relax(b));
    let z2 = square(&relax(z));
    let z9 = m(&sq_n(&z2, 2), z);
    let z11 = m(&z9, &z2);
    let z2_5_0 = m(&square(&relax(&z11)), &z9);
    let z2_10_0 = m(&sq_n(&z2_5_0, 5), &z2_5_0);
    let z2_20_0 = m(&sq_n(&z2_10_0, 10), &z2_10_0);
    let z2_40_0 = m(&sq_n(&z2_20_0, 20), &z2_20_0);
    let z2_50_0 = m(&sq_n(&z2_40_0, 10), &z2_10_0);
    let z2_100_0 = m(&sq_n(&z2_50_0, 50), &z2_50_0);
    let z2_200_0 = m(&sq_n(&z2_100_0, 100), &z2_100_0);
    let z2_250_0 = m(&sq_n(&z2_200_0, 50), &z2_50_0);
    m(&sq_n(&z2_250_0, 5), &z11)
}

/// The scalar multiple of the point with u-coordinate `u` (RFC 7748 section 5).
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248; k[31] &= 127; k[31] |= 64;
    let mut u = *u;
    u[31] &= 127;
    let mut x1 = fiat_25519_tight_field_element([0; 5]);
    fiat_25519_from_bytes(&mut x1, &u);
    let one = { let mut t = fiat_25519_tight_field_element([0; 5]); t.0[0] = 1; t };
    let (mut x2, mut z2, mut x3, mut z3) = (one, fiat_25519_tight_field_element([0; 5]), x1, one);
    let mut swapped = 0u8;
    for t in (0..255).rev() {
        let bit = (k[t / 8] >> (t % 8)) & 1;
        swapped ^= bit;
        swap(swapped, &mut x2, &mut x3);
        swap(swapped, &mut z2, &mut z3);
        swapped = bit;
        let a = add(&x2, &z2);
        let aa = square(&a);
        let b = sub(&x2, &z2);
        let bb = square(&b);
        let e = sub(&aa, &bb);
        let c = add(&x3, &z3);
        let d = sub(&x3, &z3);
        let da = mul(&d, &a);
        let cb = mul(&c, &b);
        x3 = square(&add(&da, &cb));
        z3 = mul(&relax(&x1), &relax(&square(&sub(&da, &cb))));
        x2 = mul(&relax(&aa), &relax(&bb));
        let mut e121665 = fiat_25519_tight_field_element([0; 5]);
        fiat_25519_carry_scmul_121666(&mut e121665, &e);
        // RFC 7748 uses a24 = 121665 with z2 = E * (AA + a24 * E); with 121666 the
        // same value is BB + 121666 * E.
        z2 = mul(&e, &add(&bb, &e121665));
    }
    swap(swapped, &mut x2, &mut x3);
    swap(swapped, &mut z2, &mut z3);
    let mut out = [0u8; 32];
    fiat_25519_to_bytes(&mut out, &mul(&relax(&x2), &relax(&invert(&z2))));
    out
}
pub const BASE: [u8; 32] = { let mut b = [0u8; 32]; b[0] = 9; b };

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> [u8; 32] { let mut o = [0u8; 32]; for i in 0..32 { o[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap(); } o }
    #[test]
    fn rfc7748_vectors() {
        assert_eq!(x25519(&hex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"), &hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")),
            hex("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"));
        assert_eq!(x25519(&hex("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d"), &hex("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493")),
            hex("95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957"));
        // Diffie-Hellman (section 6.1).
        let (a, b) = (hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"), hex("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"));
        let (pa, pb) = (x25519(&a, &BASE), x25519(&b, &BASE));
        assert_eq!(pa, hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"));
        assert_eq!(pb, hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"));
        assert_eq!(x25519(&a, &pb), hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"));
        assert_eq!(x25519(&b, &pa), x25519(&a, &pb));
    }
    #[test]
    fn iterated_thousand() {
        let (mut k, mut u) = (BASE, BASE);
        for _ in 0..1000 { let r = x25519(&k, &u); u = k; k = r; }
        assert_eq!(k, hex("684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51"));
    }
}
