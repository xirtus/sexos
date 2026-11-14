// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! LE Secure Connections cryptography (Core 5.4, Vol 3 Part H, 2.2): AES-CMAC (RFC 4493)
//! on the `aes` crate, the f4/f5/f6/g2 functions, and P-256 ECDH on the `p256` crate.
//!
//! Byte order: every value here is in SMP wire order (least significant byte first), the
//! same order the PDUs carry. The spec defines the functions on most-significant-first
//! strings, so each function reverses its inputs and output around AES-CMAC.

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::{Aes128, Block};
use p256::elliptic_curve::point::AffineCoordinates;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::elliptic_curve::{Field, PrimeField};
use p256::{AffinePoint, EncodedPoint, ProjectivePoint, Scalar};

fn aes(key: &[u8; 16], block: [u8; 16]) -> [u8; 16] {
    let c = Aes128::new(&(*key).into());
    let mut b = Block::from(block);
    c.encrypt_block(&mut b);
    b.into()
}

fn dbl(b: [u8; 16]) -> [u8; 16] {
    let mut o = [0u8; 16];
    let mut carry = 0u8;
    for i in (0..16).rev() { o[i] = (b[i] << 1) | carry; carry = b[i] >> 7; }
    if carry != 0 { o[15] ^= 0x87; }
    o
}

/// AES-CMAC, most significant byte first (RFC 4493).
pub fn cmac(key: &[u8; 16], msg: &[u8]) -> [u8; 16] {
    let l = aes(key, [0; 16]);
    let k1 = dbl(l);
    let k2 = dbl(k1);
    let n = msg.len().div_ceil(16).max(1);
    let complete = !msg.is_empty() && msg.len() % 16 == 0;
    let mut x = [0u8; 16];
    for i in 0..n {
        let mut block = [0u8; 16];
        let chunk = &msg[(16 * i).min(msg.len())..(16 * i + 16).min(msg.len())];
        block[..chunk.len()].copy_from_slice(chunk);
        if i == n - 1 {
            if complete { for j in 0..16 { block[j] ^= k1[j]; } }
            else { block[chunk.len()] = 0x80; for j in 0..16 { block[j] ^= k2[j]; } }
        }
        for j in 0..16 { x[j] ^= block[j]; }
        x = aes(key, x);
    }
    x
}

fn rev<const N: usize>(b: &[u8; N]) -> [u8; N] { let mut o = *b; o.reverse(); o }

/// AES-CMAC with key, message and result in wire order.
fn cmac_le(key: &[u8; 16], msg_le: &[u8]) -> [u8; 16] {
    let mut m = alloc::vec::Vec::from(msg_le);
    m.reverse();
    rev(&cmac(&rev(key), &m))
}

/// f4(U, V, X, Z): confirm values. `u`, `v` are public key X coordinates.
pub fn f4(u: &[u8; 32], v: &[u8; 32], x: &[u8; 16], z: u8) -> [u8; 16] {
    let mut m = [0u8; 65];
    m[0] = z;
    m[1..33].copy_from_slice(v);
    m[33..65].copy_from_slice(u);
    cmac_le(x, &m)
}

const SALT: [u8; 16] = [0xbe, 0x83, 0x60, 0x5a, 0xdb, 0x0b, 0x37, 0x60, 0x38, 0xa5, 0xf5, 0xaa, 0x91, 0x83, 0x88, 0x6c];

/// f5(W, N1, N2, A1, A2): (MacKey, LTK). `a1`/`a2` are address (6 bytes) then type.
pub fn f5(w: &[u8; 32], n1: &[u8; 16], n2: &[u8; 16], a1: &[u8; 7], a2: &[u8; 7]) -> ([u8; 16], [u8; 16]) {
    let t = cmac_le(&SALT, w);
    let mut m = [0u8; 53];
    m[0..2].copy_from_slice(&[0x00, 0x01]); // length 256
    m[2..9].copy_from_slice(a2);
    m[9..16].copy_from_slice(a1);
    m[16..32].copy_from_slice(n2);
    m[32..48].copy_from_slice(n1);
    m[48..52].copy_from_slice(b"eltb"); // "btle"
    m[52] = 0;
    let mackey = cmac_le(&t, &m);
    m[52] = 1;
    (mackey, cmac_le(&t, &m))
}

/// f6(W, N1, N2, R, IOcap, A1, A2): DHKey check values. `io_cap` is (IO capability,
/// OOB flag, AuthReq) as in the pairing PDU.
pub fn f6(w: &[u8; 16], n1: &[u8; 16], n2: &[u8; 16], r: &[u8; 16], io_cap: &[u8; 3], a1: &[u8; 7], a2: &[u8; 7]) -> [u8; 16] {
    let mut m = [0u8; 65];
    m[0..7].copy_from_slice(a2);
    m[7..14].copy_from_slice(a1);
    m[14..17].copy_from_slice(io_cap);
    m[17..33].copy_from_slice(r);
    m[33..49].copy_from_slice(n2);
    m[49..65].copy_from_slice(n1);
    cmac_le(w, &m)
}

/// g2(U, V, X, Y): the six-digit numeric comparison value.
pub fn g2(u: &[u8; 32], v: &[u8; 32], x: &[u8; 16], y: &[u8; 16]) -> u32 {
    let mut m = [0u8; 80];
    m[0..16].copy_from_slice(y);
    m[16..48].copy_from_slice(v);
    m[48..80].copy_from_slice(u);
    let t = cmac_le(x, &m);
    u32::from_le_bytes([t[0], t[1], t[2], t[3]]) % 1_000_000
}

/// A P-256 key pair; the public key in SMP order (X then Y, each least significant first).
pub struct KeyPair { secret: Scalar, pub public: [u8; 64] }

impl KeyPair {
    /// From 32 random bytes (most significant first). None if they are not a valid
    /// private key (zero, or not below the group order): draw again.
    pub fn from_secret(secret: &[u8; 32]) -> Option<KeyPair> {
        let s: Scalar = Option::from(Scalar::from_repr((*secret).into()))?;
        if bool::from(s.is_zero()) { return None; }
        let p = (ProjectivePoint::GENERATOR * s).to_affine();
        Some(KeyPair { secret: s, public: wire_point(&p) })
    }
    /// The DHKey (shared X coordinate, wire order) with a peer's public key. None if
    /// the peer's key is not a point on the curve (which also refuses the
    /// invalid-curve attack).
    pub fn dhkey(&self, peer: &[u8; 64]) -> Option<[u8; 32]> {
        let p = point(peer)?;
        let shared = (ProjectivePoint::from(p) * self.secret).to_affine();
        let x: [u8; 32] = shared.x().into();
        Some(rev(&x))
    }
}

fn wire_point(p: &AffinePoint) -> [u8; 64] {
    let e = p.to_encoded_point(false);
    let mut out = [0u8; 64];
    let (x, y) = (e.x().unwrap(), e.y().unwrap());
    for i in 0..32 { out[i] = x[31 - i]; out[32 + i] = y[31 - i]; }
    out
}

fn point(wire: &[u8; 64]) -> Option<AffinePoint> {
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    for i in 0..32 { x[i] = wire[31 - i]; y[i] = wire[63 - i]; }
    let e = EncodedPoint::from_affine_coordinates(&x.into(), &y.into(), false);
    Option::from(AffinePoint::from_encoded_point(&e))
}
