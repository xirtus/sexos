//! Host tests for desktop notification chunks: the exact
//! `crates/sex-pdx/src/notify.rs` Bell, the Shell and programs compile.
#[path = "../crates/sex-pdx/src/notify.rs"]
mod notify;
use notify::*;
pub const SLOT_BELL: u64 = 12;
pub static mut CALLS: Vec<(u64, u64, u64, u64)> = Vec::new();
pub fn pdx_call(slot: u64, op: u64, a: u64, b: u64, c: u64) -> (u64, u64) {
    assert_eq!(slot, SLOT_BELL);
    unsafe { (*core::ptr::addr_of_mut!(CALLS)).push((op, a, b, c)); }
    (0, 0)
}

fn roundtrip(text: &[u8]) -> Option<Text> {
    let mut a = Assembler::new();
    let mut out = None;
    for (w, x, y) in chunks(text, URGENCY_NORMAL, 7, 42) { out = a.push(w, x, y); }
    out
}
#[test] fn text_survives_chunking() {
    for len in [1usize, 15, 16, 17, 31, 32, 100, 127, 128] {
        let text: [u8; 128] = core::array::from_fn(|i| b'a' + (i % 26) as u8);
        let t = roundtrip(&text[..len]).unwrap();
        assert_eq!(t.as_bytes(), &text[..len]);
        assert_eq!((t.id, t.sender, t.urgency), (7, 42, URGENCY_NORMAL));
    }
    assert_eq!(roundtrip(&[b'x'; 200]).unwrap().len as usize, TEXT_MAX);
}
#[test] fn summary_and_body() {
    let t = roundtrip(b"Build done\n3 warnings").unwrap();
    assert_eq!(t.summary(), b"Build done");
    assert_eq!(t.body(), b"3 warnings");
    let t = roundtrip(b"Only a summary").unwrap();
    assert_eq!(t.body(), b"");
}
#[test] fn control_bytes_become_question_marks() {
    assert_eq!(roundtrip(b"a\x1b[2Jb\tc").unwrap().as_bytes(), b"a?[2Jb?c");
}
#[test] fn out_of_order_or_forged_chunks_are_dropped() {
    let parts: [(u64, u64, u64); 3] = core::array::from_fn(|i| chunks(&[b'z'; 40], 1, 0, 0).nth(i).unwrap());
    let mut a = Assembler::new();
    assert_eq!(a.push(parts[1].0, parts[1].1, parts[1].2), None, "middle first");
    assert_eq!(a.push(parts[0].0, parts[0].1, parts[0].2), None);
    assert_eq!(a.push(parts[2].0, parts[2].1, parts[2].2), None, "skipped one");
    assert!(!a.pending());
    // A fresh start after a broken one still works.
    for (i, p) in parts.iter().enumerate() { let r = a.push(p.0, p.1, p.2); assert_eq!(r.is_some(), i == 2); }
    // Bad headers: index past the end, wrong last flag, reserved bits.
    assert!(Chunk::decode(Chunk { index: 8, last: true, urgency: 0, len: 128, id: 0, sender: 0 }.word()).is_none());
    assert!(Chunk::decode(Chunk { index: 0, last: false, urgency: 0, len: 10, id: 0, sender: 0 }.word()).is_none());
    assert!(Chunk::decode(Chunk { index: 0, last: true, urgency: 0, len: 10, id: 0, sender: 0 }.word() | 1 << 12).is_none());
    assert!(Chunk::decode(Chunk { index: 0, last: true, urgency: 3, len: 10, id: 0, sender: 0 }.word()).is_none());
}

#[test] fn post_sends_summary_newline_body_in_chunks() {
    unsafe { (*core::ptr::addr_of_mut!(CALLS)).clear(); }
    assert!(post(b"Saved", b"notes.txt to Documents", URGENCY_LOW));
    let calls = unsafe { (*core::ptr::addr_of!(CALLS)).clone() };
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.0 == OP_BELL_POST));
    let mut a = Assembler::new();
    let t = calls.iter().filter_map(|c| a.push(c.1, c.2, c.3)).next().unwrap();
    assert_eq!(t.as_bytes(), b"Saved\nnotes.txt to Documents");
    assert_eq!((t.id, t.sender), (0, 0), "programs never name themselves");
    assert!(!post(b"", b"", URGENCY_LOW));
}
