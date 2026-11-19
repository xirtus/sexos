// SPDX-License-Identifier: GPL-3.0-only
//! Byte-exact tests for both directions.
use crate::client::{Client, Event};
use crate::server::{self, ClientMessage, ZrleEncoder};
use crate::*;
use alloc::vec;
use alloc::vec::Vec;

/// ServerInit for a `w` by `h` screen called "t".
fn init(w: u16, h: u16) -> Vec<u8> { server::server_init(w, h, &PixelFormat::RGB32, b"t") }

/// A client past the handshake (None security, 3.8), its output cleared.
fn connected(w: u16, h: u16) -> Client {
    let mut c = Client::new(None);
    c.feed(b"RFB 003.008\n\x01\x01\x00\x00\x00\x00");
    c.feed(&init(w, h));
    assert_eq!(c.next_event(), Ok(Some(Event::Connected)));
    assert_eq!(c.next_event(), Ok(None));
    c.take_output();
    c
}

fn events(c: &mut Client) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(e) = c.next_event().unwrap() { out.push(e); }
    out
}

fn px(c: u32) -> [u8; 4] { c.to_le_bytes() }

#[test]
fn des_fips_vector() {
    // The classic worked example (key 133457799BBCDFF1).
    let out = des::encrypt_block([0x13, 0x34, 0x57, 0x79, 0x9B, 0xBC, 0xDF, 0xF1], [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]);
    assert_eq!(out, [0x85, 0xE8, 0x13, 0x54, 0x0F, 0x0A, 0xB4, 0x05]);
}

#[test]
fn des_zero_key_vector() {
    // DES with an all-zero key on zero plaintext: 8CA64DE9C1B123A7.
    assert_eq!(des::encrypt_block([0; 8], [0; 8]), [0x8C, 0xA6, 0x4D, 0xE9, 0xC1, 0xB1, 0x23, 0xA7]);
}

#[test]
fn vnc_response_reverses_key_bits() {
    let challenge = [0u8; 16];
    // An empty password is the all-zero key, so each half is DES(0, 0).
    let r = des::vnc_response(b"", &challenge);
    assert_eq!(&r[..8], &[0x8C, 0xA6, 0x4D, 0xE9, 0xC1, 0xB1, 0x23, 0xA7]);
    assert_eq!(&r[..8], &r[8..]);
    // Bits reversed: 0x80 -> 0x01 per byte; passwords longer than 8 bytes are cut.
    let key: Vec<u8> = b"password".iter().map(|b| b.reverse_bits()).collect();
    let mut c = [0u8; 16];
    for (i, b) in c.iter_mut().enumerate() { *b = i as u8; }
    let want = [des::encrypt_block(key.clone().try_into().unwrap(), c[..8].try_into().unwrap()), des::encrypt_block(key.try_into().unwrap(), c[8..].try_into().unwrap())].concat();
    assert_eq!(des::vnc_response(b"password-and-more", &c).to_vec(), want);
}

#[test]
fn handshake_none_38_bytes() {
    let mut c = Client::new(None);
    c.feed(b"RFB 003.008\n");
    assert_eq!(c.next_event(), Ok(None));
    assert_eq!(c.take_output(), b"RFB 003.008\n");
    c.feed(&[2, 2, 1]); // offers VNC auth and None; no password: None
    assert_eq!(c.next_event(), Ok(None));
    assert_eq!(c.take_output(), [1]);
    c.feed(&[0, 0, 0, 0]);
    assert_eq!(c.next_event(), Ok(None));
    assert_eq!(c.take_output(), [1]); // ClientInit, shared
    c.feed(&init(640, 480));
    assert_eq!(c.next_event(), Ok(Some(Event::Connected)));
    assert_eq!((c.fb.width, c.fb.height, &c.name[..]), (640, 480, &b"t"[..]));
    let want: Vec<u8> = [
        &[0u8, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0][..],
        &[2, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0, 5, 0, 0, 0, 2, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x11, 0xFF, 0xFF, 0xFF, 0x21],
        &[3, 0, 0, 0, 0, 0, 2, 0x80, 1, 0xE0],
    ].concat();
    assert_eq!(c.take_output(), want);
}

#[test]
fn handshake_vnc_auth() {
    let mut c = Client::new(Some(b"secret"));
    c.feed(b"RFB 003.008\n\x02\x01\x02");
    c.next_event().unwrap();
    assert_eq!(c.take_output(), b"RFB 003.008\n\x02");
    let challenge = [7u8; 16];
    c.feed(&challenge);
    c.next_event().unwrap();
    // Golden from OpenSSL: `openssl enc -des-ecb -nopad -K cea6c64ea62e0000` (the
    // bit-reversed "secret") over sixteen 0x07 bytes.
    let golden = [0x02, 0xB0, 0x48, 0xD0, 0x34, 0x8E, 0x64, 0x76, 0x02, 0xB0, 0x48, 0xD0, 0x34, 0x8E, 0x64, 0x76];
    assert_eq!(des::vnc_response(b"secret", &challenge), golden);
    assert_eq!(c.take_output(), golden);
    c.feed(&server::security_result(Some(b"bad")));
    assert_eq!(c.next_event(), Err(Error::AuthFailed(b"bad".to_vec())));
}

#[test]
fn handshake_33_and_refusals() {
    let mut c = Client::new(None);
    c.feed(b"RFB 003.003\n\x00\x00\x00\x01");
    c.feed(&init(4, 4));
    assert_eq!(c.next_event(), Ok(Some(Event::Connected)));
    assert!(c.take_output().starts_with(b"RFB 003.003\n\x01"));
    let mut c = Client::new(None);
    c.feed(b"RFB 003.008\n\x00\x00\x00\x00\x02no");
    assert_eq!(c.next_event(), Err(Error::Refused(b"no".to_vec())));
    let mut c = Client::new(None);
    c.feed(b"RFB 003.008\n\x01\x02");
    assert_eq!(c.next_event(), Err(Error::NeedPassword));
    let mut c = Client::new(None);
    c.feed(b"RFB 004.000\n");
    assert!(c.next_event().is_err());
    let mut c = Client::new(None);
    c.feed(&server::server_init(5000, 5, &PixelFormat::RGB32, b""));
    let mut c2 = Client::new(None);
    c2.feed(b"RFB 003.008\n\x01\x01\x00\x00\x00\x00");
    c2.feed(&server::server_init(5000, 5, &PixelFormat::RGB32, b""));
    assert_eq!(c2.next_event(), Err(Error::TooLarge));
    let _ = c.next_event();
}

#[test]
fn raw_rect() {
    let mut c = connected(4, 2);
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(1, 0, 2, 2, encoding::RAW));
    for p in [0x112233, 0x445566, 0x778899, 0xAABBCC] { m.extend_from_slice(&px(p)); }
    c.feed(&m);
    assert_eq!(events(&mut c), [Event::Updated { area: Some(Rect { x: 1, y: 0, w: 2, h: 2 }), rects: 1 }]);
    assert_eq!(c.fb.pixels, [0, 0x112233, 0x445566, 0, 0, 0x778899, 0xAABBCC, 0]);
    assert_eq!(c.updates, 1);
}

#[test]
fn copy_rect_overlapping() {
    let mut c = connected(4, 1);
    c.fb.pixels = vec![1, 2, 3, 4];
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(1, 0, 3, 1, encoding::COPY_RECT));
    m.extend_from_slice(&[0, 0, 0, 0]);
    c.feed(&m);
    events(&mut c);
    assert_eq!(c.fb.pixels, [1, 1, 2, 3]);
}

#[test]
fn rre_rect() {
    let mut c = connected(3, 3);
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(0, 0, 3, 3, encoding::RRE));
    m.extend_from_slice(&[0, 0, 0, 1]);
    m.extend_from_slice(&px(0x0000FF));
    m.extend_from_slice(&px(0xFF0000));
    m.extend_from_slice(&[0, 1, 0, 1, 0, 1, 0, 1]);
    c.feed(&m);
    events(&mut c);
    assert_eq!(c.fb.pixels, [0xFF, 0xFF, 0xFF, 0xFF, 0xFF0000, 0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn hextile_rect() {
    // 18x2: two tiles, 16x2 and 2x2.
    let mut c = connected(18, 2);
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(0, 0, 18, 2, encoding::HEXTILE));
    // Tile 1: background green, foreground red, one subrect (x 1, y 1, 2x1) in foreground.
    m.push(2 | 4 | 8);
    m.extend_from_slice(&px(0x00FF00));
    m.extend_from_slice(&px(0xFF0000));
    m.extend_from_slice(&[1, 0x11, 0x10]);
    // Tile 2: raw.
    m.push(1);
    for p in [1, 2, 3, 4] { m.extend_from_slice(&px(p)); }
    // Byte by byte, to show half-arrived tiles paint nothing and wait.
    for b in &m { c.feed(&[*b]); let _ = c.next_event().unwrap(); }
    let mut want = vec![0x00FF00; 36];
    want[18 + 1] = 0xFF0000;
    want[18 + 2] = 0xFF0000;
    want[16] = 1; want[17] = 2; want[34] = 3; want[35] = 4;
    assert_eq!(c.fb.pixels, want);
    assert_eq!(c.updates, 1);
}

/// zlib bytes with the data in stored blocks: the header on the first, no end.
fn stored(first: bool, data: &[u8]) -> Vec<u8> {
    let mut out = if first { vec![0x78, 0x01] } else { Vec::new() };
    let n = data.len() as u16;
    out.push(0);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(!n).to_le_bytes());
    out.extend_from_slice(data);
    out
}

#[test]
fn zrle_hand_made_stream() {
    let mut c = connected(4, 2);
    // Rect 1, 4x2: palette RLE with 2 colours: index 0 run 5, index 1 alone, index 0 run 2.
    let tiles1 = [130u8, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0x80, 4, 0x01, 0x80, 1];
    // Rect 2, 1x1 at (3, 1): solid blue-ish (bytes B, G, R).
    let tiles2 = [1u8, 0x30, 0x20, 0x10];
    for (first, rect, tiles) in [(true, (0, 0, 4, 2), &tiles1[..]), (false, (3, 1, 1, 1), &tiles2[..])] {
        let z = stored(first, tiles);
        let mut m = server::update_header(1).to_vec();
        m.extend_from_slice(&server::rect_header(rect.0, rect.1, rect.2, rect.3, encoding::ZRLE));
        m.extend_from_slice(&(z.len() as u32).to_be_bytes());
        m.extend_from_slice(&z);
        c.feed(&m);
        events(&mut c);
    }
    let (r, g) = (0xFF0000, 0x00FF00);
    assert_eq!(c.fb.pixels, [r, r, r, r, r, g, r, 0x102030]);
}

#[test]
fn zrle_packed_palette_and_raw() {
    let mut c = connected(3, 1);
    // Packed palette of 2 (1 bit): 0b010 then raw would be another rect.
    let tiles = [2u8, 1, 0, 0, 2, 0, 0, 0b0100_0000];
    let z = stored(true, &tiles);
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(0, 0, 3, 1, encoding::ZRLE));
    m.extend_from_slice(&(z.len() as u32).to_be_bytes());
    m.extend_from_slice(&z);
    c.feed(&m);
    events(&mut c);
    assert_eq!(c.fb.pixels, [1, 2, 1]);
}

#[test]
fn pseudo_encodings_bell_and_cut_text() {
    let mut c = connected(2, 2);
    let mut m = server::update_header(2).to_vec();
    m.extend_from_slice(&server::rect_header(0, 0, 3, 1, encoding::DESKTOP_SIZE));
    m.extend_from_slice(&server::rect_header(1, 0, 2, 1, encoding::CURSOR));
    m.extend_from_slice(&px(0xABCDEF));
    m.extend_from_slice(&px(0x123456));
    m.push(0b1000_0000);
    m.push(2);
    m.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 2, b'h', b'i']);
    c.feed(&m);
    assert_eq!(events(&mut c), [
        Event::Resized { width: 3, height: 1 },
        Event::Cursor,
        Event::Updated { area: Some(Rect { x: 0, y: 0, w: 3, h: 1 }), rects: 2 },
        Event::Bell,
        Event::CutText(b"hi".to_vec()),
    ]);
    let cur = c.cursor.clone().unwrap();
    assert_eq!((cur.hot_x, cur.hot_y, cur.pixels), (1, 0, vec![0xFFAB_CDEF, 0]));
    assert_eq!(c.fb.pixels.len(), 3);
}

#[test]
fn rejects_bad_rects() {
    let mut c = connected(2, 2);
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(1, 1, 2, 2, encoding::RAW));
    m.extend_from_slice(&[0; 16]);
    c.feed(&m);
    assert!(c.next_event().is_err());
    let mut c = connected(2, 2);
    let mut m = server::update_header(1).to_vec();
    m.extend_from_slice(&server::rect_header(0, 0, 1, 1, 7));
    c.feed(&m);
    assert_eq!(c.next_event(), Err(Error::Encoding(7)));
}

#[test]
fn client_messages_bytes() {
    let mut c = connected(8, 8);
    c.request_update(true);
    c.tap(keysym::RETURN, true);
    c.pointer(1, 300, 2);
    c.cut_text(b"ok");
    let want: Vec<u8> = [
        &[3u8, 1, 0, 0, 0, 0, 0, 8, 0, 8][..],
        &[4, 1, 0, 0, 0, 0, 0xFF, 0xE3, 4, 1, 0, 0, 0, 0, 0xFF, 0x0D, 4, 0, 0, 0, 0, 0, 0xFF, 0x0D, 4, 0, 0, 0, 0, 0, 0xFF, 0xE3],
        &[5, 1, 1, 44, 0, 2],
        &[6, 0, 0, 0, 0, 0, 0, 2, b'o', b'k'],
    ].concat();
    let out = c.take_output();
    assert_eq!(out, want);
    // And the server half parses them back.
    let mut at = 0;
    let mut got = Vec::new();
    while let Some((m, n)) = server::parse_client_message(&out[at..]).unwrap() { got.push(m); at += n; }
    assert_eq!(at, out.len());
    assert_eq!(got[0], ClientMessage::UpdateRequest { incremental: true, x: 0, y: 0, w: 8, h: 8 });
    assert_eq!(got[2], ClientMessage::Key { down: true, keysym: keysym::RETURN });
    assert_eq!(got[5], ClientMessage::Pointer { buttons: 1, x: 300, y: 2 });
    assert_eq!(got[6], ClientMessage::CutText(b"ok".to_vec()));
    assert_eq!(server::parse_client_message(&out[..3]), Ok(None));
}

#[test]
fn server_handshake_bytes() {
    assert_eq!(server::VERSION, b"RFB 003.008\n");
    assert_eq!(server::security_types(&[1, 2]), [2, 1, 2]);
    assert_eq!(server::security_result(None), [0, 0, 0, 0]);
    assert_eq!(server::security_result(Some(b"x")), [0, 0, 0, 1, 0, 0, 0, 1, b'x']);
    assert_eq!(server::server_init(800, 600, &PixelFormat::RGB32, b"Lurk"), [
        3, 32, 2, 88, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0, 0, 0, 0, 4, b'L', b'u', b'r', b'k',
    ]);
    assert_eq!(server::rect_header(1, 2, 3, 4, encoding::ZRLE), [0, 1, 0, 2, 0, 3, 0, 4, 0, 0, 0, 16]);
    assert_eq!(server::rect_header(0, 0, 0, 0, encoding::DESKTOP_SIZE)[8..], [0xFF, 0xFF, 0xFF, 0x21]);
    let mut raw = Vec::new();
    server::encode_raw(&[0x112233, 0xFF445566, 7, 8], 2, 0, 0, 2, 1, &mut raw);
    assert_eq!(raw, [0x33, 0x22, 0x11, 0, 0x66, 0x55, 0x44, 0]);
    let mut pf = PixelFormat::RGB32.to_bytes();
    assert_eq!(PixelFormat::from_bytes(&pf), PixelFormat::RGB32);
    pf[0] = 16;
    assert_ne!(PixelFormat::from_bytes(&pf), PixelFormat::RGB32);
}

#[test]
fn zrle_tile_bytes() {
    // 3x1, one colour: solid.
    let mut out = Vec::new();
    server::zrle_tiles(&[0x102030; 3], 3, 0, 0, 3, 1, &mut out);
    assert_eq!(out, [1, 0x30, 0x20, 0x10]);
    // 3x2, three colours: packed palette, 2 bits, rows padded to a byte.
    out.clear();
    server::zrle_tiles(&[1, 2, 3, 3, 2, 1], 3, 0, 0, 3, 2, &mut out);
    assert_eq!(out, [3, 1, 0, 0, 2, 0, 0, 3, 0, 0, 0b00_01_10_00, 0b10_01_00_00]);
    // 17 colours: raw cpixels.
    let many: Vec<u32> = (0..17).collect();
    out.clear();
    server::zrle_tiles(&many, 17, 0, 0, 17, 1, &mut out);
    assert_eq!(out.len(), 1 + 17 * 3);
    assert_eq!(&out[..7], [0, 0, 0, 0, 1, 0, 0]);
}

#[test]
fn zrle_server_to_client_roundtrip() {
    // 130x70 crosses tile edges (64) in both directions.
    let (w, h) = (130usize, 70usize);
    let src: Vec<u32> = (0..w * h).map(|i| {
        let (x, y) = (i % w, i / w);
        match (x < 65, y < 35) { (true, true) => 0xFF0000, (false, true) => 0x00FF00, (true, false) => 0x0000FF, _ => ((x * 7 + y * 13) as u32) * 0x010203 & 0xFFFFFF }
    }).collect();
    let mut enc = ZrleEncoder::new();
    let mut c = connected(w as u16, h as u16);
    // Two updates over the same zlib stream: the whole screen, then a corner.
    for (x, y, rw, rh) in [(0, 0, w, h), (100, 50, 30, 20)] {
        let mut m = server::update_header(1).to_vec();
        m.extend_from_slice(&server::rect_header(x as u16, y as u16, rw as u16, rh as u16, encoding::ZRLE));
        enc.encode(&src, w, x, y, rw, rh, &mut m);
        c.feed(&m);
        events(&mut c);
    }
    assert_eq!(c.updates, 2);
    assert_eq!(c.fb.pixels, src);
}

#[test]
fn keysyms() {
    assert_eq!(keysym::from_key(0x1C, Some('\n')), Some(keysym::RETURN));
    assert_eq!(keysym::from_key(0x1E, Some('a')), Some(0x61));
    assert_eq!(keysym::from_key(0x1E, Some('A')), Some(0x41));
    assert_eq!(keysym::from_key(0x1A, Some('ü')), Some(0xFC));
    assert_eq!(keysym::from_key(0x12, Some('€')), Some(0x0100_20AC));
    assert_eq!(keysym::from_key(0x4B, None), Some(keysym::LEFT));
    assert_eq!(keysym::from_key(0x69, None), Some(keysym::LEFT));
    assert_eq!(keysym::from_key(0x3B, None), Some(keysym::F1));
    assert_eq!(keysym::from_key(0x58, None), Some(keysym::F1 + 11));
    assert_eq!(keysym::from_key(0x2A, None), None);
}
