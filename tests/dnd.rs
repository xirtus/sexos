//! Host test for drag and drop (`crates/sex-pdx/src/dnd.rs`).
#[path = "../crates/sex-pdx/src/dnd.rs"]
mod dnd;
use dnd::*;

fn carry(kind: u8, payload: &[u8]) -> Option<Payload> {
    let Message::Begin { kind, len, extra } = Message::decode(Message::Begin { kind, len: payload.len(), extra: at(-3, 40) }.wire())? else { return None };
    let mut p = Payload::new(kind, len, extra);
    for piece in pieces(payload) {
        let Some(Message::Data { offset, count, bytes }) = Message::decode(piece.wire()) else { return None };
        if !p.take(offset, count, &bytes) { return None; }
    }
    Some(p)
}

#[test]
fn a_path_and_text_cross_whole() {
    let p = carry(FILE, b"/0gate/note.txt").unwrap();
    assert!(p.valid());
    assert_eq!(p.as_bytes(), b"/0gate/note.txt");
    assert_eq!(position(p.extra), (-3, 40));
    let long = "é".repeat(200);
    let t = carry(TEXT, long.as_bytes()).unwrap();
    assert!(t.valid() && t.as_bytes() == long.as_bytes());
}

#[test]
fn bad_paths_and_pieces_are_refused() {
    for bad in [&b"relative"[..], b"/a/../b", b"/a b", b"/a\nb", b"/./x"] { assert!(!carry(FILE, bad).unwrap().valid(), "{:?}", bad); }
    assert!(!carry(TEXT, &[0xff, 0xfe]).unwrap().valid());
    assert!(Message::decode(Message::Begin { kind: FILE, len: MAX_BYTES + 1, extra: 0 }.wire()).is_none());
    assert!(Message::decode(Message::Begin { kind: 9, len: 3, extra: 0 }.wire()).is_none());
    // Out of order, past the end, or a short piece in the middle.
    let mut p = Payload::new(TEXT, 40, 0);
    let piece = [b'a'; PIECE];
    assert!(!p.take(16, 16, &piece));
    assert!(!p.take(0, 8, &piece));
    assert!(p.take(0, 16, &piece) && p.take(16, 16, &piece) && !p.complete());
    assert!(!p.take(32, 16, &piece));
    assert!(p.take(32, 8, &piece) && p.complete());
    // Bytes past the count must be zero.
    let mut w = Message::Data { offset: 0, count: 4, bytes: [0; PIECE] }.wire();
    w[2] = 1;
    assert!(Message::decode(w).is_none());
}

#[test]
fn done_and_hash() {
    assert_eq!(Message::decode(Message::Done { target: 52 }.wire()), Some(Message::Done { target: 52 }));
    assert_eq!(Message::decode(Message::End.wire()), Some(Message::End));
    assert_ne!(hash(b"/a"), hash(b"/b"));
}
