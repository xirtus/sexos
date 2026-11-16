// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The Shell to Whisper voice wire: `cargo test -p sex-pdx --test speech_voice`.
use sex_pdx::speech_voice::*;

fn round_trip(text: &[u8], wpm: u16) -> Option<(Vec<u8>, u16)> {
    let mut g = Gather::new();
    let mut out = None;
    for [a, b, c] in pieces(text, wpm) {
        out = match g.push(a, b, c) {
            Some(Request::Speak { text, wpm }) => Some((text.to_vec(), wpm)),
            Some(Request::Stop) => None,
            None => out,
        };
    }
    out
}

#[test]
fn text_round_trips() {
    assert_eq!(round_trip(b"OK button", 0), Some((b"OK button".to_vec(), 0)));
    let long = "Save changes to \u{201c}Report.odt\u{201d} before closing? Dialog".as_bytes();
    assert_eq!(round_trip(long, 220), Some((long.to_vec(), 220)));
    assert_eq!(round_trip(&[b'x'; 16], 175).unwrap().0.len(), 16);
    assert_eq!(pieces(&[b'x'; 16], 175).count(), 1);
    assert_eq!(round_trip(&[b'y'; 5000], 175).unwrap().0.len(), MAX_TEXT);
}

#[test]
fn empty_is_stop_and_new_utterance_restarts() {
    let p: Vec<_> = pieces(b"", 0).collect();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0][0] & (FIRST | LAST), FIRST | LAST);
    assert_eq!(Gather::new().push(p[0][0], p[0][1], p[0][2]), Some(Request::Stop));
    // Half an utterance, then a new one: only the new one is spoken.
    let mut g = Gather::new();
    let old: Vec<_> = pieces(b"a long label that was interrupted", 0).collect();
    assert!(Gather::starts(old[0][0]));
    assert!(g.push(old[0][0], old[0][1], old[0][2]).is_none());
    let mut last = None;
    for [a, b, c] in pieces(b"Cancel", 0) {
        if let Some(Request::Speak { text, .. }) = g.push(a, b, c) { last = Some(text.to_vec()); }
    }
    assert_eq!(last.as_deref(), Some(&b"Cancel"[..]));
    // A stray continuation is ignored.
    assert!(g.push(3 | LAST, 0x414141, 0).is_none());
}
