// SPDX-License-Identifier: GPL-3.0-only
//! The WebRTC media path end to end in memory: DTLS-SRTP handshake, SRTP keys, a VP8 key
//! frame packetized by sex-rtp, protected by sex-srtp, shuffled, opened, put through the
//! jitter buffer and depacketized intact; a PLI back over SRTCP.
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use sex_dtls::{Certificate, Config, Dtls, Role, SrtpProfile};
use sex_rtp::codec::Vp8;
use sex_rtp::{Depacketizer, JitterBuffer, JitterConfig, Packet, Packetizer, Rtcp, RtpStream};

fn connect(profiles: Vec<SrtpProfile>) -> (Dtls, Dtls) {
    let cc = Certificate::generate(&mut ChaCha20Rng::from_seed([21; 32]));
    let sc = Certificate::generate(&mut ChaCha20Rng::from_seed([22; 32]));
    let mut c = Config::new(Role::Client, cc.clone(), sc.fingerprint());
    let mut s = Config::new(Role::Server, sc, cc.fingerprint());
    c.srtp_profiles = profiles.clone();
    s.srtp_profiles = profiles;
    let (mut c, mut s) = (Dtls::new(c, [1; 32]), Dtls::new(s, [2; 32]));
    c.start(0);
    for _ in 0..10 {
        while let Some(d) = c.poll_transmit() {
            s.handle_datagram(0, &d).unwrap();
        }
        while let Some(d) = s.poll_transmit() {
            c.handle_datagram(0, &d).unwrap();
        }
    }
    assert!(c.is_connected() && s.is_connected());
    (c, s)
}

#[test]
fn vp8_frame_over_dtls_srtp() {
    for profile in [SrtpProfile::AeadAes128Gcm, SrtpProfile::Aes128CmHmacSha1_80] {
        let (c, s) = connect(vec![profile]);
        let (mut c_tx, mut c_rx) = c.srtp_keys().unwrap().contexts();
        let (mut s_tx, mut s_rx) = s.srtp_keys().unwrap().contexts();
        assert_eq!(c_tx.profile(), profile);

        // A 12 KB "key frame" (VP8 frame tag P bit clear).
        let frame: Vec<u8> = (0..12_000u32).map(|i| if i == 0 { 0x10 } else { (i * 31 % 253) as u8 }).collect();
        let mut vp8 = Vp8::default();
        let mtu = 1200 - 12 - profile.rtp_overhead();
        let payloads = vp8.packetize(&frame, mtu);
        let mut stream = RtpStream::new(0x1234_5678, 96, 65530);
        let mut wire: Vec<Vec<u8>> = stream
            .packets(payloads, 90_000)
            .into_iter()
            .map(|p| c_tx.protect_rtp(&p.write()).unwrap())
            .collect();
        assert!(wire.iter().all(|w| w.len() <= 1200));
        assert!(wire.len() > 10);
        // The network reorders, and a copy of one packet arrives twice.
        wire.reverse();
        wire.swap(0, 5);
        wire.push(wire[3].clone());

        let mut jb = JitterBuffer::new(JitterConfig::video());
        let mut replays = 0;
        for w in &wire {
            match s_rx.unprotect_rtp(w) {
                Ok(b) => jb.push(Packet::parse(&b).unwrap(), 0),
                Err(sex_srtp::Error::Replay) => replays += 1,
                Err(e) => panic!("{e:?}"),
            }
        }
        assert_eq!(replays, 1);
        let pkts = jb.pop(0).unwrap();
        let mut rx = Vp8::default();
        let mut got = None;
        for p in &pkts {
            got = got.or(rx.push(&p.payload, p.marker));
        }
        let got = got.unwrap();
        assert!(got.keyframe);
        assert_eq!(got.data, frame);

        // The receiver asks for a key frame over SRTCP.
        let pli = Rtcp::compound(&[Rtcp::Pli { sender_ssrc: 1, media_ssrc: 0x1234_5678 }]);
        let sp = s_tx.protect_rtcp(&pli).unwrap();
        assert!(sex_rtp::is_rtcp(&sp));
        let back = c_rx.unprotect_rtcp(&sp).unwrap();
        assert_eq!(Rtcp::parse_compound(&back).unwrap(), vec![Rtcp::Pli { sender_ssrc: 1, media_ssrc: 0x1234_5678 }]);
        // Keys are per direction: the sender cannot open its own packets with its rx keys.
        assert!(c_rx.unprotect_rtp(&wire[0]).is_err());
    }
}
