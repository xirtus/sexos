// SPDX-License-Identifier: GPL-3.0-only
//! STUN against the RFC 5769 test vectors, and the framing TURN uses on streams.

use sex_ice::stun::{self, attr, method, Class, Framer, Message};
use std::net::SocketAddr;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

// RFC 5769 2.1: a request with SOFTWARE, PRIORITY, ICE-CONTROLLED, USERNAME,
// MESSAGE-INTEGRITY and FINGERPRINT.
const REQUEST: &str = "
00 01 00 58 21 12 a4 42 b7 e7 a7 01 bc 34 d6 86 fa 87 df ae
80 22 00 10 53 54 55 4e 20 74 65 73 74 20 63 6c 69 65 6e 74
00 24 00 04 6e 00 01 ff
80 29 00 08 93 2f f9 b1 51 26 3b 36
00 06 00 09 65 76 74 6a 3a 68 36 76 59 20 20 20
00 08 00 14 9a ea a7 0c bf d8 cb 56 78 1e f2 b5 b2 d3 f2 49 c1 b5 71 a2
80 28 00 04 e5 7a 3b cf";
const PASSWORD: &str = "VOkJxbRl1RmTxUk/WvJxBt";

// RFC 5769 2.2: the IPv4 response.
const RESPONSE4: &str = "
01 01 00 3c 21 12 a4 42 b7 e7 a7 01 bc 34 d6 86 fa 87 df ae
80 22 00 0b 74 65 73 74 20 76 65 63 74 6f 72 20
00 20 00 08 00 01 a1 47 e1 12 a6 43
00 08 00 14 2b 91 f5 99 fd 9e 90 c3 8c 74 89 f9 2a f9 ba 53 f0 6b e7 d7
80 28 00 04 c0 7d 4c 96";

// RFC 5769 2.3: the IPv6 response.
const RESPONSE6: &str = "
01 01 00 48 21 12 a4 42 b7 e7 a7 01 bc 34 d6 86 fa 87 df ae
80 22 00 0b 74 65 73 74 20 76 65 63 74 6f 72 20
00 20 00 14 00 02 a1 47 01 13 a9 fa a5 d3 f1 79 bc 25 f4 b5 be d2 b9 d9
00 08 00 14 a3 82 95 4e 4b e6 7b f1 17 84 c9 7c 82 92 c2 75 bf e3 ed 41
80 28 00 04 c8 fb 0b 4c";

#[test]
fn rfc5769_request() {
    let raw = hex(REQUEST);
    assert!(stun::is_stun(&raw));
    let m = Message::decode(&raw).unwrap();
    assert_eq!((m.class, m.method), (Class::Request, method::BINDING));
    assert_eq!(m.get_str(attr::SOFTWARE), Some("STUN test client"));
    assert_eq!(m.get_str(attr::USERNAME), Some("evtj:h6vY"));
    assert_eq!(m.get_u32(attr::PRIORITY), Some(0x6e0001ff));
    assert_eq!(m.get_u64(attr::ICE_CONTROLLED), Some(0x932ff9b151263b36));
    assert!(stun::check_integrity(&raw, PASSWORD.as_bytes()));
    assert!(!stun::check_integrity(&raw, b"wrong"));
    assert_eq!(stun::check_fingerprint(&raw), Some(true));
    // Our encoder writes the same bytes (SOFTWARE is padded with spaces in the
    // vector, so give it the same value with its padding as data).
    let mut again = Message::request(method::BINDING, m.transaction);
    again.add_str(attr::SOFTWARE, "STUN test client").add_u32(attr::PRIORITY, 0x6e0001ff)
        .add_u64(attr::ICE_CONTROLLED, 0x932ff9b151263b36).add_str(attr::USERNAME, "evtj:h6vY");
    let enc = again.encode(Some(PASSWORD.as_bytes()), true);
    // The vector pads USERNAME with spaces where we pad with zeros; integrity covers
    // the padding, so compare after copying it.
    let mut enc_padded = enc.clone();
    enc_padded[73..76].copy_from_slice(b"   ");
    assert_eq!(&enc_padded[..76], &raw[..76]);
    assert!(stun::check_integrity(&enc, PASSWORD.as_bytes()));
    assert_eq!(stun::check_fingerprint(&enc), Some(true));
    // A flipped bit breaks both.
    let mut bad = raw.clone();
    bad[30] ^= 1;
    assert!(!stun::check_integrity(&bad, PASSWORD.as_bytes()));
    assert_eq!(stun::check_fingerprint(&bad), Some(false));
}

#[test]
fn rfc5769_responses() {
    for (v, addr) in [(RESPONSE4, "192.0.2.1:32853"), (RESPONSE6, "[2001:db8:1234:5678:11:2233:4455:6677]:32853")] {
        let raw = hex(v);
        let m = Message::decode(&raw).unwrap();
        assert_eq!(m.class, Class::Success);
        assert_eq!(m.mapped(), Some(addr.parse().unwrap()));
        assert!(stun::check_integrity(&raw, PASSWORD.as_bytes()));
        assert_eq!(stun::check_fingerprint(&raw), Some(true));
        // And our encoding of that address is the vector's.
        let mut r = Message::new(Class::Success, method::BINDING, m.transaction);
        r.add_xor_address(attr::XOR_MAPPED_ADDRESS, addr.parse().unwrap());
        assert_eq!(r.attributes[0].1, m.get(attr::XOR_MAPPED_ADDRESS).unwrap());
    }
}

#[test]
fn long_term_key() {
    // MD5 of "user:realm:pass", as RFC 8489 9.2.2 makes it for ASCII credentials.
    let k = stun::long_term_key("user", "realm", "pass");
    assert_eq!(k, [0x84, 0x93, 0xfb, 0xc5, 0x3b, 0xa5, 0x82, 0xfb, 0x4c, 0x04, 0x4c, 0x45, 0x6b, 0xdc, 0x40, 0xeb]);
}

#[test]
fn every_class_and_method() {
    for class in [Class::Request, Class::Indication, Class::Success, Class::Error] {
        for m in [method::BINDING, method::ALLOCATE, method::REFRESH, method::SEND, method::DATA, method::CREATE_PERMISSION, method::CHANNEL_BIND, 0xFFF] {
            let mut msg = Message::new(class, m, [7; 12]);
            msg.add_error(438, "Stale Nonce");
            let back = Message::decode(&msg.encode(None, false)).unwrap();
            assert_eq!((back.class, back.method), (class, m));
            assert_eq!(back.error_code(), Some((438, "Stale Nonce".into())));
        }
    }
    assert_eq!(Message::new(Class::Success, method::BINDING, [0; 12]).encode(None, false)[..2], [0x01, 0x01]);
    assert_eq!(Message::new(Class::Error, method::ALLOCATE, [0; 12]).encode(None, false)[..2], [0x01, 0x13]);
    assert_eq!(Message::new(Class::Indication, method::DATA, [0; 12]).encode(None, false)[..2], [0x00, 0x17]);
}

#[test]
fn refuses_broken_messages() {
    let good = Message::request(method::BINDING, [1; 12]).encode(None, true);
    assert!(Message::decode(&good[..10]).is_err());
    let mut long = good.clone();
    long[3] += 4;
    assert!(Message::decode(&long).is_err());
    let mut cookie = good.clone();
    cookie[5] ^= 1;
    assert!(!stun::is_stun(&cookie));
    // DTLS and RTP first bytes are not STUN.
    assert!(!stun::is_stun(&[22u8; 40]) && !stun::is_stun(&[0x80u8; 40]));
}

#[test]
fn stream_framing() {
    let a = Message::request(method::ALLOCATE, [2; 12]).encode(None, false);
    let c = stun::channel_data(0x4001, b"hello", true);
    assert_eq!(c.len(), 12);
    let mut f = Framer::default();
    let mut all = a.clone();
    all.extend_from_slice(&c);
    // In odd pieces, as TCP delivers them.
    for chunk in all.chunks(3) { f.push(chunk); }
    assert_eq!(f.next().unwrap(), a);
    let got = f.next().unwrap();
    assert_eq!(stun::read_channel_data(&got), Some((0x4001, &b"hello"[..])));
    assert!(f.next().is_none());
    let addr: SocketAddr = "[2001:db8::1]:3478".parse().unwrap();
    let mut m = Message::request(method::CHANNEL_BIND, [3; 12]);
    m.add_xor_address(attr::XOR_PEER_ADDRESS, addr);
    assert_eq!(Message::decode(&m.encode(None, false)).unwrap().xor_address(attr::XOR_PEER_ADDRESS), Some(addr));
}
