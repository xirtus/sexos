//! Security Manager cryptography against published vectors: RFC 4493 AES-CMAC and the
//! Bluetooth Core spec's sample data for f4, f5, f6 and g2 (Vol 3 Part H, Appendix D;
//! here in SMP wire order, least significant byte first), and the spec's P-256 debug key.
use sex_bt::crypto::*;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}
fn a<const N: usize>(s: &str) -> [u8; N] { hex(s).try_into().unwrap() }
fn rev<const N: usize>(mut x: [u8; N]) -> [u8; N] { x.reverse(); x }

#[test]
fn aes_cmac_rfc4493() {
    let k: [u8; 16] = a("2b7e151628aed2a6abf7158809cf4f3c");
    let m = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710");
    assert_eq!(cmac(&k, &[]), a::<16>("bb1d6929e95937287fa37d129b756746"));
    assert_eq!(cmac(&k, &m[..16]), a::<16>("070a16b46b4d4144f79bdd9dd04a287c"));
    assert_eq!(cmac(&k, &m[..40]), a::<16>("dfa66747de9ae63030ca32611497c827"));
    assert_eq!(cmac(&k, &m), a::<16>("51f0bebf7e3b9d92fc49741779363cfe"));
}

const U: &str = "e69d350e480103ccdbfdf4ac1191f4efb9a5f9e9a7832c5e2cbe97f2d203b020";
const V: &str = "fdc57ff449dd4f6bfb7c9df1c29acb592ae7d4eefbfc0a909abbf6323d8b1855";
const X: &str = "abae2b71ecb2ffff3e7377d15484cbd5";
const Y: &str = "cfc43dfff78365216e5fa725cce7e8a6";
const A1: &str = "cebf3737125600";
const A2: &str = "c1cf2d7013a700";

#[test]
fn f4_sample() {
    assert_eq!(f4(&a(U), &a(V), &a(X), 0), a::<16>("2d8774a9bea1edf11cbda907f116c9f2"));
}

#[test]
fn f5_sample() {
    let w: [u8; 32] = a("98a6bf73f3348d86f166f8b4136b79999b7d390aa610103405adc857a33402ec");
    let (mackey, ltk) = f5(&w, &a(X), &a(Y), &a(A1), &a(A2));
    assert_eq!(mackey, a::<16>("206e63ce206a3ffd024a08a176f16529"));
    assert_eq!(ltk, a::<16>("380a7594b5220598 23cdd76911798669"));
}

#[test]
fn f6_sample() {
    let w: [u8; 16] = a("206e63ce206a3ffd024a08a176f16529");
    let r: [u8; 16] = a("c80f2d0cd242da0854bb53b43b34a312");
    let e = f6(&w, &a(X), &a(Y), &r, &[0x02, 0x01, 0x01], &a(A1), &a(A2));
    assert_eq!(e, a::<16>("618f95da090b6cd2c5e8d09c9873c4e3"));
}

#[test]
fn g2_sample() {
    assert_eq!(g2(&a(U), &a(V), &a(X), &a(Y)), 0x2f9ed5ba % 1_000_000);
}

#[test]
fn p256_debug_key_and_ecdh() {
    let secret: [u8; 32] = a("3f49f6d4a3c55f3874c9b3e3d2103f504aff607beb40b7995899b8a6cd3c1abd");
    let k = KeyPair::from_secret(&secret).unwrap();
    let x: [u8; 32] = a("20b003d2f297be2c5e2c83a7e9f9a5b9eff49111acf4fddbcc0301480e359de6");
    let y: [u8; 32] = a("dc809c49652aeb6d63329abf5a52155c766345c28fed3024741c8ed01589d28b");
    assert_eq!(&k.public[..32], &rev(x)[..]);
    assert_eq!(&k.public[32..], &rev(y)[..]);
    let other = KeyPair::from_secret(&[0x11; 32]).unwrap();
    assert_eq!(k.dhkey(&other.public), other.dhkey(&k.public));
    // A point off the curve is refused.
    let mut bad = other.public;
    bad[40] ^= 1;
    assert!(k.dhkey(&bad).is_none());
    assert!(KeyPair::from_secret(&[0; 32]).is_none());
    assert!(KeyPair::from_secret(&[0xff; 32]).is_none());
}
