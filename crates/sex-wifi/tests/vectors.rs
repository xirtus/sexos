//! Known-answer tests from IEEE 802.11-2020 annexes, RFC 3394, 3610, 4493 and
//! hostap's copy of the Annex J.10 SAE vectors.
use sex_wifi::{ccmp, crypto, sae};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}
fn arr<const N: usize>(s: &str) -> [u8; N] { hex(s).try_into().unwrap() }

#[test]
fn passphrase_to_psk_ieee_vector() {
    assert_eq!(crypto::psk_from_passphrase(b"password", b"IEEE").to_vec(),
        hex("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"));
    assert_eq!(crypto::psk_from_passphrase(b"ThisIsAPassword", b"ThisIsASSID").to_vec(),
        hex("0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af"));
}

#[test]
fn prf_sha1_ieee_vector() {
    let mut out = [0u8; 64];
    crypto::prf_sha1(&[0x0b; 20], b"prefix", b"Hi There", &mut out);
    assert_eq!(out.to_vec(), hex("bcd4c650b30b9684951829e0d75f9d54b862175ed9f00606e17d8da35402ffee75df78c3d31e0f889f012120c0862beb67753e7439ae242edb8373698356cf5a"));
}

#[test]
fn kdf_sha256_matches_definition() {
    let mut out = [0u8; 48];
    crypto::kdf_sha256(&[b'k'; 32], b"Pairwise key expansion", &[b'c'; 76], &mut out);
    assert_eq!(out.to_vec(), hex("1339cf6af40846654b662befe20dc2591aa08640c0fca339bfef3d61a16bc67840af502428372fa54344bb0db55f0870"));
}

#[test]
fn aes_key_wrap_rfc3394() {
    let kek: [u8; 16] = arr("000102030405060708090a0b0c0d0e0f");
    let key = hex("00112233445566778899aabbccddeeff");
    let mut wrapped = [0u8; 24];
    crypto::aes_wrap(&kek, &key, &mut wrapped).unwrap();
    assert_eq!(wrapped.to_vec(), hex("1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5"));
    let mut back = [0u8; 16];
    crypto::aes_unwrap(&kek, &wrapped, &mut back).unwrap();
    assert_eq!(back.to_vec(), key);
    wrapped[3] ^= 1;
    assert!(crypto::aes_unwrap(&kek, &wrapped, &mut back).is_err());
    assert_eq!(back, [0; 16], "a failed unwrap leaves no key material");
}

#[test]
fn aes_cmac_rfc4493() {
    let key: [u8; 16] = arr("2b7e151628aed2a6abf7158809cf4f3c");
    assert_eq!(crypto::aes_cmac(&key, &[]).to_vec(), hex("bb1d6929e95937287fa37d129b756746"));
    assert_eq!(crypto::aes_cmac(&key, &[&hex("6bc1bee22e409f96e93d7e117393172a")]).to_vec(), hex("070a16b46b4d4144f79bdd9dd04a287c"));
    let m = hex("6bc1bee22e409f96e93d7e117393172aae2d8a57");
    assert_eq!(crypto::aes_cmac(&key, &[&m[..7], &m[7..]]).to_vec(), hex("7d85449ea6ea19c823a7bf78837dfade"));
}

#[test]
fn ccm_rfc3610_packet_vector_1() {
    let key: [u8; 16] = arr("c0c1c2c3c4c5c6c7c8c9cacbcccdcecf");
    let nonce: [u8; 13] = arr("00000003020100a0a1a2a3a4a5");
    let packet: Vec<u8> = (0u8..0x1f).collect();
    let mut body = packet[8..].to_vec();
    let mic = ccmp::ccm_seal(&key, &nonce, &packet[..8], &mut body);
    let mut out = body.clone();
    out.extend_from_slice(&mic);
    assert_eq!(out, hex("588c979a61c663d2f066d0c2c0f989806d5f6b61dac38417e8d12cfdf926e0"));
    ccmp::ccm_open(&key, &nonce, &packet[..8], &mut body, &mic).unwrap();
    assert_eq!(body, packet[8..].to_vec());
}

#[test]
fn ccmp_ieee_annex_vector() {
    let tk: [u8; 16] = arr("c97c1f67ce371185514a8a19f2bdd52f");
    let frame = hex("0848c32c0fd2e128a57c5030f1844408abaea5b8fcba8033f8ba1a55d02f85ae967bb62fb6cda8eb7e78a050");
    let mut out = vec![0u8; frame.len() + 16];
    ccmp::encrypt_frame(&tk, &frame, 24, 0xb5039776e70c, 0, &mut out).unwrap();
    assert_eq!(out, hex("08 48 c3 2c 0f d2 e1 28 a5 7c 50 30 f1 84 44 08 ab ae a5 b8 fc ba 80 33
        0c e7 00 20 76 97 03 b5 f3 d0 a2 fe 9a 3d bf 23 42 a6 43 e4 32 46 e8 0c 3c 04 d0 19
        78 45 ce 0b 16 f9 76 23"));
    let (pn, range) = ccmp::decrypt_frame(&tk, &mut out, 24).unwrap();
    assert_eq!(pn, 0xb5039776e70c);
    assert_eq!(out[range].to_vec(), frame[24..].to_vec());
    let mut bad = hex("0848c32c0fd2e128a57c5030f1844408abaea5b8fcba80330ce70020769703b5f3d0a2fe9a3dbf2342a643e43246e80c3c04d0197845ce0b16f97624");
    assert!(ccmp::decrypt_frame(&tk, &mut bad, 24).is_err(), "a flipped MIC bit is rejected");
}

// IEEE Std 802.11-2020 Annex J.10 (as in hostap src/common/common_module_tests.c).
const ADDR1: [u8; 6] = [0x4d, 0x3f, 0x2f, 0xff, 0xe3, 0x87];
const ADDR2: [u8; 6] = [0xa5, 0xd8, 0xaa, 0x95, 0x8e, 0x3c];
const LOCAL_RAND: &str = "992465fd3daa3c60aa6565b7f62a2a7f2e12dd12f198faf4fbed89d7ff1ace94";
const LOCAL_MASK: &str = "9507a90f777a044d6a0830b91ea3d5dd70bece44e1acffb86983b5e1bf9fb322";
const LOCAL_COMMIT: &str = "13002e2c0f0db52440ad146d967114ce005ce1eab0aa2c2e5c2871b774f6c2575c65d5ad9e00829707aa36ba8b859738fc961d08243505f47c035376d7ac4bc8d7b95083bf43827d0fc31ed778dd3671fd21a46d1091d64b6f9a1e1272621325dbe1";
const PEER_COMMIT: &str = "1300591b96f3397fb945100848e7b550543b6720d88337ee93fc49fd6df7e08b5223e71b9bb048d3873f20556953a96c91536fd8ee6ca9b4a68a148b056a909be03e83ae208f60f8ef5537858074db06687032399862999b511e0a1552a5fea317c2";

#[test]
fn sae_hunting_and_pecking_annex_j10() {
    let pwe = sae::pwe_hunt_and_peck(&ADDR1, &ADDR2, b"mekmitasdigoat").unwrap();
    let mut s = sae::Sae::new(pwe, &arr(LOCAL_RAND), &arr(LOCAL_MASK)).unwrap();
    assert_eq!(s.commit().to_vec(), hex(LOCAL_COMMIT));
    s.process_commit(&hex(PEER_COMMIT)).unwrap();
    assert_eq!(s.kck.to_vec(), hex("1e733f6d9bd53256287304338831b09a39406d121017073a5c30db36f36cb81a"));
    assert_eq!(s.pmk.to_vec(), hex("4e4dfab1a2dd8ac1a91790f953faaa452ae5c6873ab75b63605ba663f8a7fe59"));
    assert_eq!(s.pmkid.to_vec(), hex("8747a600eea3f9f22475df58ca1e5498"));
}

#[test]
fn sae_hash_to_element_annex_j10() {
    let pt = sae::derive_pt(b"byteme", b"mekmitasdigoat", Some(b"psk4internet"));
    let a = [0x00, 0x09, 0x5b, 0x66, 0xec, 0x1e];
    let b = [0x00, 0x0b, 0x6b, 0xd9, 0x02, 0x46];
    let pwe = sae::pwe_from_pt(&pt, &a, &b);
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let e = pwe.to_encoded_point(false);
    assert_eq!(e.x().unwrap().to_vec(), hex("c93049b9e64000f848201649e999f2b5c22dea69b5632c9df4d633b8aa1f6c1e"));
    assert_eq!(e.y().unwrap().to_vec(), hex("73634e94b53d82e7383a8d258199d9dc1a5ee8269d060382ccbf33e614ff59a0"));
}

#[test]
fn sae_two_stations_agree_and_confirm() {
    let pwe_a = sae::pwe_hunt_and_peck(&ADDR1, &ADDR2, b"correct horse").unwrap();
    let pwe_b = sae::pwe_hunt_and_peck(&ADDR2, &ADDR1, b"correct horse").unwrap();
    assert_eq!(pwe_a, pwe_b, "the element does not depend on who is A");
    let mut a = sae::Sae::new(pwe_a, &[0x11; 32], &[0x22; 32]).unwrap();
    let mut b = sae::Sae::new(pwe_b, &[0x33; 32], &[0x44; 32]).unwrap();
    a.process_commit(&b.commit()).unwrap();
    b.process_commit(&a.commit()).unwrap();
    assert_eq!(a.pmk, b.pmk);
    assert_eq!(a.pmkid, b.pmkid);
    b.check_confirm(&a.confirm(1)).unwrap();
    a.check_confirm(&b.confirm(1)).unwrap();
    // A wrong password gives a different PMK and the confirm fails.
    let pwe_c = sae::pwe_hunt_and_peck(&ADDR2, &ADDR1, b"wrong").unwrap();
    let mut c = sae::Sae::new(pwe_c, &[0x33; 32], &[0x44; 32]).unwrap();
    let mut a2 = sae::Sae::new(pwe_a, &[0x11; 32], &[0x22; 32]).unwrap();
    c.process_commit(&a2.commit()).unwrap();
    a2.process_commit(&c.commit()).unwrap();
    assert_eq!(a2.check_confirm(&c.confirm(1)), Err(sae::Error::BadConfirm));
    // Reflecting our own commit back is refused.
    let own = a2.commit();
    let mut a3 = sae::Sae::new(pwe_a, &[0x11; 32], &[0x22; 32]).unwrap();
    assert_eq!(a3.process_commit(&own), Err(sae::Error::Reflection));
}
