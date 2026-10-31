//! Native tests of Quil's production bounded find.
#[path = "../servers/quil/src/find.rs"]
mod find;
use find::{next_match, Find, Match, QUERY_MAX};

fn m(start: usize, end: usize, wrapped: bool) -> Option<Match> { Some(Match { start, end, wrapped }) }

#[test]
fn finds_from_position_then_wraps_once() {
    let text = b"alpha beta alpha";
    assert_eq!(next_match(text, b"alpha", 0), m(0, 5, false));
    assert_eq!(next_match(text, b"alpha", 5), m(11, 16, false));
    assert_eq!(next_match(text, b"alpha", 16), m(0, 5, true));
    assert_eq!(next_match(text, b"beta", 7), m(6, 10, true));
    assert_eq!(next_match(text, b"gamma", 0), None);
}

#[test]
fn exact_case_sensitive_bytes_and_degenerate_queries() {
    assert_eq!(next_match(b"Alpha", b"alpha", 0), None);
    assert_eq!(next_match(b"abc", b"", 0), None);
    assert_eq!(next_match(b"ab", b"abc", 0), None);
    assert_eq!(next_match(b"", b"a", 0), None);
    assert_eq!(next_match(b"aaa", b"aa", 1), m(1, 3, false));
    assert_eq!(next_match(b"abc", b"c", usize::MAX), m(2, 3, true));
}

#[test]
fn matches_never_split_utf8_scalars() {
    let text = "naïve café naïve".as_bytes();
    let found = next_match(text, "ï".as_bytes(), 3).unwrap();
    assert!(std::str::from_utf8(&text[found.start..found.end]).is_ok());
    assert!(std::str::from_utf8(&text[..found.start]).is_ok());
    for (start, _) in text.iter().enumerate() {
        if let Some(found) = next_match(text, b"caf", start) {
            assert!(std::str::from_utf8(&text[..found.start]).is_ok());
        }
    }
}

#[test]
fn query_is_bounded_printable_and_erased_on_exit() {
    let mut f = Find::new();
    assert!(!f.push(b'a'), "inactive find accepts nothing");
    f.begin();
    for byte in [b'\n', 0, 0x7f, 0xc3] { assert!(!f.push(byte)); }
    for _ in 0..QUERY_MAX { assert!(f.push(b'x')); }
    assert!(!f.push(b'y'));
    assert_eq!(f.query().len(), QUERY_MAX);
    let (prompt, len) = f.prompt();
    assert!(prompt[..len].starts_with(b"Find: ") && prompt[..len].ends_with(b"#"));
    assert!(f.pop());
    f.clear();
    assert!(!f.active());
    assert!(f.query().is_empty());
    assert!(!f.pop());
}

#[test]
fn quil_wiring_routes_keys_and_fences_paste() {
    let main = include_str!("../servers/quil/src/main.rs");
    assert!(main.contains("0x21 if !QUIL_NAME_MODE&&!*palette_active=>{"));
    assert!(main.contains("||QUIL_UNICODE_INPUT.active()||QUIL_FIND.active() {"));
    let dispatch = main.split("fn quil_dispatch_key(").nth(1).unwrap();
    let find = dispatch.find("if QUIL_FIND.active() {").unwrap();
    let control = dispatch.find("if key.control {\n            match key.code {").unwrap();
    assert!(find < control);
    assert!(main.contains("QUIL_SELECTION.move_to(source,found.start,false).is_ok()&&QUIL_SELECTION.move_to(source,found.end,true).is_ok()"));
}

#[test]
fn replacement_field_is_separate_bounded_and_shown_in_the_prompt() {
    let mut f = Find::new();
    f.begin();
    for b in *b"cat" { assert!(f.push(b)); }
    f.set_field(find::Field::Replacement);
    assert_eq!(f.field(), find::Field::Replacement);
    for b in *b"dog" { assert!(f.push(b)); }
    assert!(f.pop());
    assert_eq!((f.query(), f.replacement()), (&b"cat"[..], &b"do"[..]));
    let (prompt, len) = f.prompt();
    assert_eq!(&prompt[..len], b"Find: cat > do#");
    f.set_field(find::Field::Query);
    let (prompt, len) = f.prompt();
    assert_eq!(&prompt[..len], b"Find: cat# > do");
    assert!(f.selection_matches(b"cat") && !f.selection_matches(b"ca") && !f.selection_matches(b"cats"));
    for _ in 0..QUERY_MAX { f.push(b'x'); }
    f.set_field(find::Field::Replacement);
    for _ in 0..QUERY_MAX + 3 { f.push(b'y'); }
    let (_, len) = f.prompt();
    assert!(len <= find::PROMPT_MAX);
    f.clear();
    assert!(f.replacement().is_empty() && f.field() == find::Field::Query && !f.active());
}
