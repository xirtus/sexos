// SPDX-License-Identifier: GPL-3.0-only
//! Known-answer packets from libsrtp's test/srtp_driver.c (srtp_validate and
//! srtp_validate_gcm): master key and salt through the RFC 3711 KDF, SSRC 0xcafebabe, first
//! SRTCP index 1.
use sex_srtp::{Context, Error, Profile};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

const RTP: &str = "800f1234decafbadcafebabe abababababababababababababababab";
const RTCP: &str = "81c8000bcafebabe abababababababababababababababab";

fn cm_key() -> (Vec<u8>, Vec<u8>) {
    (hex("e1f97a0d3e018be0d64fa32c06de4139"), hex("0ec675ad498afeebb6960b3aabe6"))
}

fn gcm_key() -> (Vec<u8>, Vec<u8>) {
    (hex("000102030405060708090a0b0c0d0e0f"), hex("a0a1a2a3a4a5a6a7a8a9aaab"))
}

fn ctx(p: Profile, k: &(Vec<u8>, Vec<u8>)) -> Context {
    let mut c = Context::new(p, &k.0, &k.1).unwrap();
    c.first_srtcp_index = 1;
    c
}

#[test]
fn aes_cm_sha1_80_rtp_and_rtcp() {
    let k = cm_key();
    let mut tx = ctx(Profile::Aes128CmHmacSha1_80, &k);
    let srtp = tx.protect_rtp(&hex(RTP)).unwrap();
    assert_eq!(srtp, hex("800f1234decafbadcafebabe4e55dc4ce79978d88ca4d215949d2402b78d6acc99ea179b8dbb"));
    let srtcp = tx.protect_rtcp(&hex(RTCP)).unwrap();
    assert_eq!(srtcp, hex("81c8000bcafebabe7128035be487b9bdbef89041f977a5a880000001993e08cd54d6c1230798"));

    let mut rx = ctx(Profile::Aes128CmHmacSha1_80, &k);
    assert_eq!(rx.unprotect_rtp(&srtp).unwrap(), hex(RTP));
    assert_eq!(rx.unprotect_rtp(&srtp), Err(Error::Replay));
    assert_eq!(rx.unprotect_rtcp(&srtcp).unwrap(), hex(RTCP));
    assert_eq!(rx.unprotect_rtcp(&srtcp), Err(Error::Replay));

    let mut bad = srtp.clone();
    bad[20] ^= 1;
    let mut rx2 = ctx(Profile::Aes128CmHmacSha1_80, &k);
    assert_eq!(rx2.unprotect_rtp(&bad), Err(Error::Auth));
    // A failed packet does not move the window: the good one still opens.
    assert_eq!(rx2.unprotect_rtp(&srtp).unwrap(), hex(RTP));
}

#[test]
fn aead_aes_128_gcm_rtp_and_rtcp() {
    let k = gcm_key();
    let mut tx = ctx(Profile::AeadAes128Gcm, &k);
    let srtp = tx.protect_rtp(&hex(RTP)).unwrap();
    assert_eq!(
        srtp,
        hex("800f1234decafbadcafebabec5002ede04cfdd2eb91159e0880aa06ed2976826f796b201df3131a127e8a392")
    );
    let srtcp = tx.protect_rtcp(&hex(RTCP)).unwrap();
    assert_eq!(
        srtcp,
        hex("81c8000bcafebabec98b8b5df0392a55852b6c21ac8e7025c52c6fbea2b3b446ea31123ba88ce61e80000001")
    );
    let mut rx = ctx(Profile::AeadAes128Gcm, &k);
    assert_eq!(rx.unprotect_rtp(&srtp).unwrap(), hex(RTP));
    assert_eq!(rx.unprotect_rtcp(&srtcp).unwrap(), hex(RTCP));
    let mut bad = srtcp.clone();
    bad[9] ^= 0x80;
    let mut rx2 = ctx(Profile::AeadAes128Gcm, &k);
    assert_eq!(rx2.unprotect_rtcp(&bad), Err(Error::Auth));
}

fn rtp(seq: u16, ssrc: u32, ext: bool) -> Vec<u8> {
    let mut p = vec![0x80 | if ext { 0x10 } else { 0 }, 96];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&(seq as u32 * 3000).to_be_bytes());
    p.extend_from_slice(&ssrc.to_be_bytes());
    if ext {
        p.extend_from_slice(&[0xbe, 0xde, 0, 1, 0x10, 0xaa, 0, 0]);
    }
    p.extend((0..40).map(|i| i as u8 ^ seq as u8));
    p
}

#[test]
fn rollover_and_reorder_every_profile() {
    for p in [Profile::Aes128CmHmacSha1_80, Profile::Aes128CmHmacSha1_32, Profile::AeadAes128Gcm] {
        let k = if p == Profile::AeadAes128Gcm { gcm_key() } else { cm_key() };
        let mut tx = ctx(p, &k);
        let mut rx = ctx(p, &k);
        let seqs: Vec<u16> = (65500u32..65600).map(|s| s as u16).collect();
        let mut sent: Vec<(u16, Vec<u8>)> = seqs.iter().map(|&s| (s, tx.protect_rtp(&rtp(s, 7, s % 3 == 0)).unwrap())).collect();
        // Swap neighbours to reorder across the wrap.
        for i in (0..sent.len() - 1).step_by(2) {
            sent.swap(i, i + 1);
        }
        for (s, pkt) in &sent {
            assert_eq!(rx.unprotect_rtp(pkt).unwrap(), rtp(*s, 7, s % 3 == 0), "{p:?} seq {s}");
        }
        assert_eq!(rx.roc(7), Some(1));
        assert_eq!(tx.roc(7), Some(1));
        for i in 0..5u32 {
            let mut r = hex(RTCP);
            r[4..8].copy_from_slice(&9u32.to_be_bytes());
            r.push(i as u8);
            r.extend_from_slice(&[0, 0, 0]);
            let s = tx.protect_rtcp(&r).unwrap();
            assert_eq!(rx.unprotect_rtcp(&s).unwrap(), r);
        }
    }
}
