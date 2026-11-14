//! Host tests for the launcher's type-to-search: the exact
//! `servers/silk-shell/src/launcher_search.rs` the shell compiles.
#[path = "../servers/silk-shell/src/launcher_search.rs"]
mod launcher_search;
use launcher_search::*;

const ROWS: &[(&str, &str)] = &[
    ("Open Spindle", "terminal shell console command"),
    ("Open Quil", "editor text notes write"),
    ("Open Linen", "files folders documents"),
    ("Open Atlas", "overview workspaces desktops"),
    ("Open Bell", "notifications alerts"),
    ("Restore Minimized", "unhide show window"),
    ("Open Browser", "web internet html"),
    ("Shut Down", "power off halt"),
    ("Restart", "reboot"),
];

fn top(q: &str) -> Option<&'static str> {
    let mut out = [0usize; 16];
    let n = rank(q, ROWS, &mut out);
    (n > 0).then(|| ROWS[out[0]].0)
}

#[test]
fn empty_query_keeps_list_order() {
    let mut out = [0usize; 16];
    assert_eq!(rank("", ROWS, &mut out), ROWS.len());
    assert!(out[..ROWS.len()].iter().enumerate().all(|(i, &r)| i == r));
}

#[test]
fn name_word_starts_win() {
    assert_eq!(top("spin"), Some("Open Spindle"));
    assert_eq!(top("sp"), Some("Open Spindle"));
    assert_eq!(top("br"), Some("Open Browser"));
    assert_eq!(top("ob"), Some("Open Bell"));
    assert_eq!(top("sd"), Some("Shut Down"));
    assert_eq!(top("rest"), Some("Restart"));
    assert_eq!(top("rem"), Some("Restore Minimized"));
}

#[test]
fn search_words_find_rows_by_purpose() {
    assert_eq!(top("term"), Some("Open Spindle"));
    assert_eq!(top("web"), Some("Open Browser"));
    assert_eq!(top("files"), Some("Open Linen"));
    assert_eq!(top("reboot"), Some("Restart"));
    assert_eq!(top("power"), Some("Shut Down"));
}

#[test]
fn letters_must_appear_in_order() {
    assert_eq!(top("zzz"), None);
    assert_eq!(top("lq"), None);
    assert!(score("nd", "Open Spindle", "").is_some());
    // A later word start is taken only when the rest still fits after it.
    assert!(score("ab", "xab a", "").is_some());
}

#[test]
fn query_edits_and_keys() {
    let mut q = Query::new();
    assert!(!q.push(b' '), "no leading space");
    for c in b"Web" { assert!(q.push(*c)); }
    assert_eq!(q.as_str(), "web");
    assert!(q.pop());
    assert_eq!(q.as_str(), "we");
    q.clear();
    assert!(q.is_empty() && !q.pop());
    for _ in 0..40 { q.push(b'a'); }
    assert_eq!(q.as_str().len(), QUERY_MAX);
    assert_eq!(key_char(0x10), Some(b'q'));
    assert_eq!(key_char(0x26), Some(b'l'));
    assert_eq!(key_char(0x32), Some(b'm'));
    assert_eq!(key_char(0x02), Some(b'1'));
    assert_eq!(key_char(0x0B), Some(b'0'));
    assert_eq!(key_char(0x39), Some(b' '));
    assert_eq!(key_char(0x1C), None);
    assert_eq!(key_char(0x01), None);
}

#[test]
fn selection_steps_within_matches() {
    let order = [4, 1, 7];
    assert_eq!(step(&order, 4, 1), Some(1));
    assert_eq!(step(&order, 7, 1), Some(4));
    assert_eq!(step(&order, 4, -1), Some(7));
    assert_eq!(step(&order, 2, 1), Some(4));
    assert_eq!(step(&[], 0, 1), None);
}

#[test]
fn shifted_keys_type_arithmetic_for_voyeur() {
    // 2 Shift+= 2 is "2+2"; Shift+digits that are not operators type nothing.
    assert_eq!(key_char_shifted(0x03, false), Some(b'2'));
    assert_eq!(key_char_shifted(0x0D, true), Some(b'+'));
    assert_eq!(key_char_shifted(0x0D, false), Some(b'='));
    assert_eq!(key_char_shifted(0x09, true), Some(b'*'));
    assert_eq!(key_char_shifted(0x35, false), Some(b'/'));
    assert_eq!(key_char_shifted(0x0A, true), Some(b'('));
    assert_eq!(key_char_shifted(0x02, true), None);
    assert_eq!(key_char_shifted(0x1E, true), Some(b'a'));
    assert_eq!(key_char_shifted(0x4E, false), Some(b'+'));
    let mut q = Query::new();
    for c in [0x03, 0x0D, 0x03].iter().zip([false, true, false]).filter_map(|(&k, s)| key_char_shifted(k, s)) { q.push(c); }
    assert_eq!(q.as_str(), "2+2");
}
