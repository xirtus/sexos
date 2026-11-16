// SPDX-License-Identifier: GPL-3.0-only
//! Host test for Whisper, the screen reader in the Shell (servers/silk-shell/src/reader.rs):
//! speech queued for the voice, what changes are spoken, and the accessibility tree
//! programs send. Built against a `sex_pdx` holding only the wires it uses
//! (scripts/service_custody_host_gate.sh makes it).
extern crate alloc;
#[path = "../servers/silk-shell/src/reader.rs"]
mod reader;

use reader::{Applied, Say, Snapshot, Speech, Tree, Watch};
use sex_pdx::{a11y, speech_voice as voice};

fn heard(speech: &mut Speech) -> Vec<Vec<u8>> {
    let mut g = voice::Gather::new();
    let mut out = Vec::new();
    while let Some([a, b, c]) = speech.front() {
        speech.sent();
        match g.push(a, b, c) {
            Some(voice::Request::Speak { text, .. }) => out.push(text.to_vec()),
            Some(voice::Request::Stop) => out.push(Vec::new()),
            None => {}
        }
    }
    out
}

#[test]
fn speech_reaches_the_voice_whole() {
    let mut s = Speech::new();
    assert!(!s.pending());
    s.say(b"Spindle");
    assert!(s.pending());
    assert_eq!(heard(&mut s), vec![b"Spindle".to_vec()]);
    assert!(!s.pending());
    let long = vec![b'a'; 3000];
    s.say(&long);
    assert_eq!(heard(&mut s), vec![vec![b'a'; voice::MAX_TEXT]]);
}

#[test]
fn new_speech_interrupts_old() {
    let mut s = Speech::new();
    s.say(b"A long sentence that takes several pieces to send");
    s.sent();
    s.say(b"Abacus");
    // The newest starts with FIRST, so the voice stops what it was saying.
    assert!(voice::Gather::starts(s.front().unwrap()[0]));
    assert_eq!(heard(&mut s), vec![b"Abacus".to_vec()]);
    s.hush();
    assert_eq!(heard(&mut s), vec![Vec::new()]);
    s.say(b"x");
    s.clear();
    assert!(s.front().is_none());
}

fn snap<'a>(focus: u64, title: Option<&'a [u8]>, launcher: Option<(u8, &'a [u8])>, notices: usize) -> Snapshot<'a> {
    Snapshot { focus, title, title_due: false, now: 0, launcher, notices }
}

#[test]
fn a_program_window_title_is_awaited_briefly() {
    let mut w = Watch::new();
    look(&mut w, snap(153, Some(b"Spindle"), None, 0));
    let due = |now| Snapshot { focus: 41, title: None, title_due: true, now, launcher: None, notices: 0 };
    // Focus moved to a program whose title has not come: wait for it.
    assert!(look(&mut w, due(1000)).is_empty());
    assert!(w.waiting());
    assert!(look(&mut w, due(1000 + reader::TITLE_WAIT_MS - 1)).is_empty());
    // It came: say it, once.
    let named = Snapshot { title: Some(b"Abacus"), title_due: false, ..due(1200) };
    assert_eq!(look(&mut w, named), vec![Say::Title(b"Abacus")]);
    assert!(!w.waiting());
    assert!(look(&mut w, named).is_empty());
    // A program that never names its window is called what Display calls it.
    look(&mut w, snap(153, Some(b"Spindle"), None, 0));
    let late = |now| Snapshot { focus: 42, ..due(now) };
    assert!(look(&mut w, late(5000)).is_empty());
    assert_eq!(look(&mut w, late(5000 + reader::TITLE_WAIT_MS)), vec![Say::Title(reader::UNTITLED)]);
    // Focus left before the title came: the wait is for the new window.
    assert!(look(&mut w, due(9000)).is_empty());
    assert!(look(&mut w, snap(153, Some(b"Spindle"), None, 0)).len() == 1);
    assert!(!w.waiting());
}
fn look<'a>(w: &mut Watch, s: Snapshot<'a>) -> Vec<Say<'a>> {
    let mut out = [None, None];
    w.look(&s, &mut out);
    out.into_iter().flatten().collect()
}

#[test]
fn focus_changes_speak_the_title() {
    let mut w = Watch::new();
    // The first look takes the screen as it is.
    assert!(look(&mut w, snap(153, Some(b"Spindle"), None, 0)).is_empty());
    assert!(look(&mut w, snap(153, Some(b"Spindle"), None, 0)).is_empty());
    assert_eq!(look(&mut w, snap(41, Some(b"Abacus"), None, 0)), vec![Say::Title(b"Abacus")]);
    assert_eq!(look(&mut w, snap(153, Some(b"Spindle"), None, 0)), vec![Say::Title(b"Spindle")]);
    // A surface with no title (a panel) is not named.
    assert!(look(&mut w, snap(7, None, None, 0)).is_empty());
    w.reset();
    assert!(look(&mut w, snap(41, Some(b"Abacus"), None, 0)).is_empty());
}

#[test]
fn launcher_selection_wins_over_its_focus() {
    let mut w = Watch::new();
    look(&mut w, snap(153, Some(b"Spindle"), None, 0));
    assert_eq!(look(&mut w, snap(152, None, Some((0, b"Spindle")), 0)), vec![Say::Launcher(b"Spindle")]);
    assert!(look(&mut w, snap(152, None, Some((0, b"Spindle")), 0)).is_empty());
    assert_eq!(look(&mut w, snap(152, None, Some((1, b"Quil")), 0)), vec![Say::Launcher(b"Quil")]);
    // Closing it back to the same window says nothing; to another names it.
    assert!(look(&mut w, snap(152, None, None, 0)).is_empty());
    assert_eq!(look(&mut w, snap(201, Some(b"Quil"), None, 0)), vec![Say::Title(b"Quil")]);
}

#[test]
fn notifications_are_read() {
    let mut w = Watch::new();
    look(&mut w, snap(153, Some(b"Spindle"), None, 3));
    assert_eq!(look(&mut w, snap(153, Some(b"Spindle"), None, 4)), vec![Say::Notice(1)]);
    assert_eq!(look(&mut w, snap(41, Some(b"Abacus"), None, 6)), vec![Say::Notice(2), Say::Title(b"Abacus")]);
    let mut out = [0u8; 64];
    let n = reader::notice_text(b"Battery low", b"10% left\nPlug in soon", &mut out);
    assert_eq!(&out[..n], b"Battery low. 10% left Plug in soon");
    let n = reader::notice_text(b"Saved", b"", &mut out);
    assert_eq!(&out[..n], b"Saved");
}

fn put_str(o: &mut Vec<u8>, s: &str) { o.extend_from_slice(&(s.len() as u32).to_le_bytes()); o.extend_from_slice(s.as_bytes()); }
fn update(full: bool, focus: u64, nodes: &[(u64, u8, &str, &str)]) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(a11y::MAGIC);
    o.push(a11y::VERSION);
    o.push(full as u8);
    if full { o.extend_from_slice(&1u64.to_le_bytes()); }
    o.extend_from_slice(&focus.to_le_bytes());
    o.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for &(id, role, label, value) in nodes {
        o.extend_from_slice(&id.to_le_bytes());
        o.push(role);
        o.extend_from_slice(&0u32.to_le_bytes());
        for s in [label, value, "", "", ""] { put_str(&mut o, s); }
        o.extend_from_slice(&[0; 16]);
        o.extend_from_slice(&0u32.to_le_bytes());
    }
    o
}
fn focused(t: &Tree) -> String { let mut b = [0u8; 128]; let n = t.focused_text(&mut b); String::from_utf8(b[..n].to_vec()).unwrap() }

#[test]
fn tree_follows_focus_across_updates() {
    let mut t = Tree::new();
    let full = update(true, 1, &[(1, 133, "", ""), (2, 17, "Expression", ""), (3, 18, "7", ""), (4, 18, "8", "")]);
    let mut last = None;
    for [a, b, c] in a11y::pieces(&full) { last = t.piece(&a11y::Piece::decode(a, b, c).unwrap()).or(last); }
    // The first tree only says where focus is; it is not a move to announce.
    assert_eq!(last, Some(Applied::Same));
    assert_eq!(t.focus(), 1);
    assert_eq!(t.len(), 4);
    assert_eq!(focused(&t), "");
    // An update with only the focus: the node is remembered from the full tree.
    assert_eq!(t.apply(&update(false, 3, &[])), Applied::FocusMoved);
    assert_eq!(focused(&t), "7 button");
    assert_eq!(t.apply(&update(false, 3, &[])), Applied::Same);
    assert_eq!(t.apply(&update(false, 2, &[(2, 17, "Expression", "12+30")])), Applied::FocusMoved);
    assert_eq!(focused(&t), "Expression text field 12+30");
    // Broken bytes change nothing.
    let mut bad = update(false, 4, &[]);
    bad.pop();
    assert_eq!(t.apply(&bad), Applied::Refused);
    assert_eq!(t.focus(), 2);
    // A new full tree replaces the old one.
    assert_eq!(t.apply(&update(true, 9, &[(1, 133, "", ""), (9, 19, "OK", "")])), Applied::FocusMoved);
    assert_eq!(t.len(), 2);
    assert_eq!(focused(&t), "OK button");
}

#[test]
fn tree_pieces_need_a_start() {
    let mut t = Tree::new();
    let bytes = update(true, 2, &[(2, 18, "OK", "")]);
    let pieces: Vec<_> = a11y::pieces(&bytes).collect();
    assert!(pieces.len() > 1);
    // Without the first piece nothing is gathered.
    for &[a, b, c] in &pieces[1..] { assert_eq!(t.piece(&a11y::Piece::decode(a, b, c).unwrap()), None); }
    assert_eq!(t.len(), 0);
}
