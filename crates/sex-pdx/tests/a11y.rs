// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The accessibility tree transport: `cargo test -p sex-pdx --test a11y`.
//! (Weft's own test checks `scan` against `weft::a11y::encode` and AccessKit's roles.)
use sex_pdx::a11y::*;

fn put_str(o: &mut Vec<u8>, s: &str) { o.extend_from_slice(&(s.len() as u32).to_le_bytes()); o.extend_from_slice(s.as_bytes()); }
fn node(o: &mut Vec<u8>, id: u64, role: u8, flags: u32, label: &str, value: &str, kids: &[u64]) {
    o.extend_from_slice(&id.to_le_bytes());
    o.push(role);
    o.extend_from_slice(&flags.to_le_bytes());
    if flags & (1 << 10) != 0 { o.extend_from_slice(&[0; 16]); }
    for s in [label, value, "", "", ""] { put_str(o, s); }
    if flags & (1 << 11) != 0 { o.extend_from_slice(&[0; 32]); }
    o.extend_from_slice(&[0; 16]);
    o.extend_from_slice(&(kids.len() as u32).to_le_bytes());
    for k in kids { o.extend_from_slice(&k.to_le_bytes()); }
}
fn update(full: bool, focus: u64, nodes: &[(u64, u8, u32, &str, &str, &[u64])]) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(MAGIC);
    o.push(VERSION);
    o.push(full as u8);
    if full { o.extend_from_slice(&1u64.to_le_bytes()); }
    o.extend_from_slice(&focus.to_le_bytes());
    o.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for &(id, role, flags, label, value, kids) in nodes { node(&mut o, id, role, flags, label, value, kids); }
    o
}

#[test]
fn scan_reads_nodes_and_focus() {
    let bytes = update(true, 3, &[(1, 133, 0, "Abacus", "", &[2, 3]), (2, 17, 1 << 10, "Expression", "12+30", &[]), (3, 18, (1 << 10) | (1 << 11), "7", "", &[])]);
    let mut seen = Vec::new();
    let head = scan(&bytes, |n| seen.push((n.id, n.role, n.label.to_vec(), n.value.to_vec()))).unwrap();
    assert_eq!(head, Head { full: true, root: Some(1), focus: 3, nodes: 3 });
    assert_eq!(seen[1], (2, 17, b"Expression".to_vec(), b"12+30".to_vec()));
    assert_eq!(seen[2].2, b"7");
}

#[test]
fn scan_refuses_broken_bytes() {
    let bytes = update(false, 2, &[(2, 18, 0, "OK", "", &[])]);
    assert!(scan(&bytes, |_| {}).is_some());
    for cut in 0..bytes.len() { assert!(scan(&bytes[..cut], |_| {}).is_none(), "cut {cut}"); }
    let mut long = bytes.clone(); long.push(0);
    assert!(scan(&long, |_| {}).is_none());
    let mut bad = bytes.clone(); bad[0] = b'X';
    assert!(scan(&bad, |_| {}).is_none());
}

#[test]
fn pieces_round_trip() {
    let bytes = update(true, 1, &[(1, 133, 0, "A window with a long enough title to need pieces", "", &[])]);
    let mut back = Vec::new();
    let all: Vec<_> = pieces(&bytes).collect();
    assert_eq!(all.len(), bytes.len().div_ceil(PIECE_BYTES));
    for (i, [a, b, c]) in all.iter().copied().enumerate() {
        assert_eq!(kind(a), TREE);
        let p = Piece::decode(a, b, c).unwrap();
        assert_eq!(p.first, i == 0);
        assert_eq!(p.last, i + 1 == all.len());
        back.extend_from_slice(p.data());
    }
    assert_eq!(back, bytes);
    assert!(Piece::decode(ASK << 56, 0, 0).is_none());
    assert!(Piece::decode(TREE << 56 | 17, 0, 0).is_none());
}

#[test]
fn ask_and_state() {
    assert_eq!(kind(ask()[0]), ASK);
    let [a, b, _] = state(true);
    assert_eq!(decode_state(OP_A11Y, a, b), Some(true));
    let [a, b, _] = state(false);
    assert_eq!(decode_state(OP_A11Y, a, b), Some(false));
    assert_eq!(decode_state(0x202, a, b), None);
    assert_eq!(decode_state(OP_A11Y, ask()[0], 1), None);
}

#[test]
fn speech_names_the_role() {
    let mut out = [0u8; 64];
    let say = |n: NodeText, out: &mut [u8; 64]| { let k = speak(&n, out); String::from_utf8(out[..k].to_vec()).unwrap() };
    let n = |role, label: &'static [u8], value: &'static [u8], disabled| NodeText { id: 1, role, hidden: false, disabled, label, value };
    assert_eq!(say(n(18, b"OK", b"", false), &mut out), "OK button");
    assert_eq!(say(n(19, b"=", b"", true), &mut out), "= button dimmed");
    assert_eq!(say(n(17, b"Expression", b"12+30", false), &mut out), "Expression text field 12+30");
    assert_eq!(say(n(3, b"", b"Hello", false), &mut out), "Hello");
    assert_eq!(say(n(15, b"Wrap", b"", false), &mut out), "Wrap check box");
    let mut tiny = [0u8; 4];
    assert_eq!(speak(&n(18, b"OK", b"", false), &mut tiny), 4);
}
