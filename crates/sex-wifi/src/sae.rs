// SPDX-License-Identifier: GPL-3.0-only
//! WPA3-Personal: Simultaneous Authentication of Equals (IEEE 802.11-2020 12.4)
//! on group 19 (NIST P-256), both password-element methods:
//! hunting-and-pecking (2.4/5 GHz WPA3 and transition networks) and
//! hash-to-element (required on 6 GHz, preferred by modern access points).
use crate::crypto::{hkdf_expand, hkdf_extract, hmac_sha256, kdf_sha256, kdf_sha256_bits, min_max};
use p256::elliptic_curve::bigint::{Encoding, U256};
use p256::elliptic_curve::ff::{Field, PrimeField};
use p256::elliptic_curve::point::AffineCoordinates;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::elliptic_curve::subtle::{ConditionallySelectable, ConstantTimeEq};
use p256::{AffinePoint, EncodedPoint, FieldElement, ProjectivePoint, Scalar};

pub const GROUP: u16 = 19;
pub const COMMIT_LEN: usize = 2 + 32 + 64;
const P_HEX: &str = "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff";
const N_HEX: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";
/// Hunting and pecking always runs at least this many rounds so the time taken
/// does not reveal which round found the element.
const MIN_ROUNDS: u8 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { NoElement, BadCommit, Reflection, BadConfirm }

fn prime() -> U256 { U256::from_be_hex(P_HEX) }
fn order() -> U256 { U256::from_be_hex(N_HEX) }
fn fe(bytes: &[u8; 32]) -> Option<FieldElement> { Option::from(FieldElement::from_bytes(bytes.into())) }
fn curve_b() -> FieldElement {
    fe(&hex32("5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b")).unwrap()
}
fn curve_a() -> FieldElement { FieldElement::ZERO.sub(&FieldElement::from_u64(3)) }

const fn hex32(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(b[2 * i]) << 4) | nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}
const fn nibble(c: u8) -> u8 { if c <= b'9' { c - b'0' } else { c - b'a' + 10 } }

fn rhs(x: &FieldElement) -> FieldElement {
    x.square().multiply(x).add(&curve_a().multiply(x)).add(&curve_b())
}

fn point(x: &FieldElement, y: &FieldElement) -> Option<AffinePoint> {
    let encoded = EncodedPoint::from_affine_coordinates(&x.to_bytes(), &y.to_bytes(), false);
    Option::from(AffinePoint::from_encoded_point(&encoded))
}

fn coordinates(p: &AffinePoint) -> ([u8; 32], [u8; 32]) {
    let e = p.to_encoded_point(false);
    let mut x = [0u8; 32];
    let mut y = [0u8; 32];
    x.copy_from_slice(e.x().unwrap());
    y.copy_from_slice(e.y().unwrap());
    (x, y)
}

/// Password element by hunting and pecking (12.4.4.2.2).
pub fn pwe_hunt_and_peck(a: &[u8; 6], b: &[u8; 6], password: &[u8]) -> Result<AffinePoint, Error> {
    let (lo, hi) = min_max(a, b);
    let mut key = [0u8; 12];
    key[..6].copy_from_slice(hi);
    key[6..].copy_from_slice(lo);
    let p_bytes = prime().to_be_bytes();
    let mut found: Option<(FieldElement, bool)> = None;
    let mut counter: u8 = 1;
    while counter <= MIN_ROUNDS || found.is_none() {
        if counter == u8::MAX { break; }
        let seed = hmac_sha256(&key, &[password, &[counter]]);
        let mut value = [0u8; 32];
        kdf_sha256_bits(&seed, b"SAE Hunting and Pecking", &p_bytes, 256, &mut value);
        counter += 1;
        let Some(x) = fe(&value) else { continue };
        let y2 = rhs(&x);
        let is_square = bool::from(y2.sqrt().is_some());
        if is_square && found.is_none() { found = Some((x, seed[31] & 1 == 1)); }
    }
    let (x, odd) = found.ok_or(Error::NoElement)?;
    let y = Option::<FieldElement>::from(rhs(&x).sqrt()).ok_or(Error::NoElement)?;
    let y = FieldElement::conditional_select(&y, &y.neg(), p256::elliptic_curve::subtle::Choice::from((bool::from(y.is_odd()) != odd) as u8));
    point(&x, &y).ok_or(Error::NoElement)
}

/// Reduce a 48-byte big-endian number modulo p.
fn reduce48(bytes: &[u8; 48]) -> FieldElement {
    let mut hi = [0u8; 32];
    hi[16..].copy_from_slice(&bytes[..16]);
    let mut lo = [0u8; 32];
    lo.copy_from_slice(&bytes[16..]);
    let mut lo_int = U256::from_be_slice(&lo);
    if lo_int >= prime() { lo_int = lo_int.wrapping_sub(&prime()); }
    let lo_fe = fe(&lo_int.to_be_bytes()).unwrap();
    let hi_fe = fe(&hi).unwrap();
    // 2^256 mod p = 2^256 - p.
    let r = U256::ZERO.wrapping_sub(&prime());
    let r_fe = fe(&r.to_be_bytes()).unwrap();
    hi_fe.multiply(&r_fe).add(&lo_fe)
}

/// Simplified SWU map for P-256 (RFC 9380 6.6.2), z = -10, constant-time.
fn sswu(u: &FieldElement) -> AffinePoint {
    let a = curve_a();
    let b = curve_b();
    let z = FieldElement::ZERO.sub(&FieldElement::from_u64(10));
    let zu2 = z.multiply(&u.square());
    let tv1 = zu2.square().add(&zu2);
    let tv1_inv = Option::<FieldElement>::from(tv1.invert()).unwrap_or(FieldElement::ZERO);
    let neg_b_over_a = b.neg().multiply(&Option::<FieldElement>::from(a.invert()).unwrap());
    let x1_general = neg_b_over_a.multiply(&FieldElement::ONE.add(&tv1_inv));
    let x1_exceptional = b.multiply(&Option::<FieldElement>::from(z.multiply(&a).invert()).unwrap());
    let x1 = FieldElement::conditional_select(&x1_general, &x1_exceptional, tv1.is_zero());
    let gx1 = rhs(&x1);
    let x2 = zu2.multiply(&x1);
    let gx2 = rhs(&x2);
    let gx1_square = gx1.sqrt().is_some();
    let x = FieldElement::conditional_select(&x2, &x1, gx1_square);
    let gx = FieldElement::conditional_select(&gx2, &gx1, gx1_square);
    let y = Option::<FieldElement>::from(gx.sqrt()).unwrap();
    let y = FieldElement::conditional_select(&y, &y.neg(), !u.is_odd().ct_eq(&y.is_odd()));
    point(&x, &y).expect("SSWU lands on the curve")
}

/// The password-derived point PT for hash-to-element (12.4.4.2.3). Computed once
/// per SSID and password; `identifier` is the optional password identifier.
pub fn derive_pt(ssid: &[u8], password: &[u8], identifier: Option<&[u8]>) -> AffinePoint {
    let seed = match identifier {
        Some(id) => hkdf_extract(ssid, &[password, id]),
        None => hkdf_extract(ssid, &[password]),
    };
    let mut u1 = [0u8; 48];
    hkdf_expand(&seed, b"SAE Hash to Element u1 P1", &mut u1);
    let mut u2 = [0u8; 48];
    hkdf_expand(&seed, b"SAE Hash to Element u2 P2", &mut u2);
    let p1 = sswu(&reduce48(&u1));
    let p2 = sswu(&reduce48(&u2));
    (ProjectivePoint::from(p1) + ProjectivePoint::from(p2)).to_affine()
}

/// PWE from PT and the two MAC addresses (hash-to-element).
pub fn pwe_from_pt(pt: &AffinePoint, a: &[u8; 6], b: &[u8; 6]) -> AffinePoint {
    let (lo, hi) = min_max(a, b);
    let val = hkdf_extract(&[0u8; 32], &[hi, lo]);
    // val mod (n - 1) + 1; 2^256 < 2(n - 1), so one subtraction reduces it.
    let n1 = order().wrapping_sub(&U256::ONE);
    let mut v = U256::from_be_slice(&val);
    if v >= n1 { v = v.wrapping_sub(&n1); }
    let v = v.wrapping_add(&U256::ONE);
    let scalar = Option::<Scalar>::from(Scalar::from_repr(v.to_be_bytes().into())).unwrap();
    (ProjectivePoint::from(*pt) * scalar).to_affine()
}

/// One side of an SAE exchange.
pub struct Sae {
    pwe: AffinePoint,
    rand: Scalar,
    scalar: Scalar,
    element: AffinePoint,
    peer_scalar: Option<Scalar>,
    peer_element: Option<AffinePoint>,
    pub kck: [u8; 32],
    pub pmk: [u8; 32],
    pub pmkid: [u8; 16],
}

fn scalar_from(bytes: &[u8; 32]) -> Option<Scalar> { Option::from(Scalar::from_repr((*bytes).into())) }

impl Sae {
    /// `rand` and `mask` are 32 random bytes each from the system entropy service;
    /// values outside 2..n-1 are rejected and the caller draws again.
    pub fn new(pwe: AffinePoint, rand: &[u8; 32], mask: &[u8; 32]) -> Option<Self> {
        let r = scalar_from(rand)?;
        let m = scalar_from(mask)?;
        if bool::from(r.is_zero()) || bool::from(m.is_zero()) || r == Scalar::ONE || m == Scalar::ONE { return None; }
        let scalar = r + m;
        if bool::from(scalar.is_zero()) || scalar == Scalar::ONE { return None; }
        let element = (-(ProjectivePoint::from(pwe) * m)).to_affine();
        Some(Self { pwe, rand: r, scalar, element, peer_scalar: None, peer_element: None,
            kck: [0; 32], pmk: [0; 32], pmkid: [0; 16] })
    }

    /// Commit body: group, scalar, element (no anti-clogging token).
    pub fn commit(&self) -> [u8; COMMIT_LEN] {
        let mut out = [0u8; COMMIT_LEN];
        out[..2].copy_from_slice(&GROUP.to_le_bytes());
        out[2..34].copy_from_slice(&self.scalar.to_repr());
        let (x, y) = coordinates(&self.element);
        out[34..66].copy_from_slice(&x);
        out[66..98].copy_from_slice(&y);
        out
    }

    /// Processes the peer's commit body and derives KCK, PMK and PMKID.
    pub fn process_commit(&mut self, body: &[u8]) -> Result<(), Error> {
        if body.len() < COMMIT_LEN || u16::from_le_bytes([body[0], body[1]]) != GROUP { return Err(Error::BadCommit); }
        let mut s = [0u8; 32];
        s.copy_from_slice(&body[2..34]);
        let peer_scalar = scalar_from(&s).ok_or(Error::BadCommit)?;
        if bool::from(peer_scalar.is_zero()) || peer_scalar == Scalar::ONE { return Err(Error::BadCommit); }
        let mut x = [0u8; 32];
        let mut y = [0u8; 32];
        x.copy_from_slice(&body[34..66]);
        y.copy_from_slice(&body[66..98]);
        let peer_element = point(&fe(&x).ok_or(Error::BadCommit)?, &fe(&y).ok_or(Error::BadCommit)?).ok_or(Error::BadCommit)?;
        if peer_scalar == self.scalar && peer_element == self.element { return Err(Error::Reflection); }
        let k = (ProjectivePoint::from(self.pwe) * peer_scalar + ProjectivePoint::from(peer_element)) * self.rand;
        let k = k.to_affine();
        if bool::from(k.is_identity()) { return Err(Error::BadCommit); }
        let kx: [u8; 32] = k.x().into();
        let keyseed = hmac_sha256(&[0u8; 32], &[&kx]);
        let sum = (self.scalar + peer_scalar).to_repr();
        let mut keys = [0u8; 64];
        kdf_sha256(&keyseed, b"SAE KCK and PMK", &sum, &mut keys);
        self.kck.copy_from_slice(&keys[..32]);
        self.pmk.copy_from_slice(&keys[32..]);
        self.pmkid.copy_from_slice(&sum[..16]);
        self.peer_scalar = Some(peer_scalar);
        self.peer_element = Some(peer_element);
        Ok(())
    }

    fn confirm_value(&self, send_confirm: u16, own_first: bool) -> [u8; 32] {
        let own = self.commit();
        let ps = self.peer_scalar.unwrap().to_repr();
        let (px, py) = coordinates(&self.peer_element.unwrap());
        let sc = send_confirm.to_le_bytes();
        if own_first {
            hmac_sha256(&self.kck, &[&sc, &own[2..34], &own[34..98], &ps, &px, &py])
        } else {
            hmac_sha256(&self.kck, &[&sc, &ps, &px, &py, &own[2..34], &own[34..98]])
        }
    }

    /// Our confirm body: send-confirm counter and the confirm hash.
    pub fn confirm(&self, send_confirm: u16) -> [u8; 34] {
        let mut out = [0u8; 34];
        out[..2].copy_from_slice(&send_confirm.to_le_bytes());
        out[2..].copy_from_slice(&self.confirm_value(send_confirm, true));
        out
    }

    /// Checks the peer's confirm body.
    pub fn check_confirm(&self, body: &[u8]) -> Result<(), Error> {
        if body.len() < 34 || self.peer_scalar.is_none() { return Err(Error::BadConfirm); }
        let sc = u16::from_le_bytes([body[0], body[1]]);
        let expect = self.confirm_value(sc, false);
        if crate::crypto::ct_eq(&expect, &body[2..34]) { Ok(()) } else { Err(Error::BadConfirm) }
    }
}
