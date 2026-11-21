// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Golden ranking tests. Run with `cargo test -p voyeur` or, without cargo,
//! `rustc --edition=2021 --test crates/voyeur/src/lib.rs` (the host gate).
use crate::abacus::{evaluate, Value};
use crate::rank::*;
use crate::tables::{EMOJI, SETTINGS};
use crate::{Action, Kind};
/// The provider wire protocol, the exact file the Shell and programs compile.
#[path = "../../sex-pdx/src/search.rs"]
mod wire;

fn val(q: &str) -> Option<String> {
    evaluate(q).map(|v: Value| { let mut b = [0u8; 48]; let n = v.format(&mut b); String::from_utf8(b[..n].to_vec()).unwrap() })
}

fn c<'a>(provider: u8, weight: u8, kind: Kind, title: &'a str, keywords: &'a str) -> Candidate<'a> {
    Candidate { provider, weight, kind: kind as u8, title, keywords, hint: 0, hint_cap: HINT_CAP_EXTERNAL }
}

const APPS: &[(&str, &str)] = &[
    ("Open Spindle", "terminal shell console command"),
    ("Open Quil", "editor text notes write"),
    ("Open Linen", "files folders documents"),
    ("Open Atlas", "overview workspaces desktops"),
    ("Open Bell", "notifications alerts"),
    ("Open Collar", "permissions privacy security"),
    ("Open Mesh", "system monitor tasks"),
    ("Restore Minimized", "unhide show window"),
    ("Zoom Toggle", "maximize fullscreen resize"),
    ("Minimize Focused", "hide window"),
    ("Open Browser", "web internet html"),
    ("Run Hello", "hello app program"),
    ("Shut Down", "power off halt"),
    ("Restart", "reboot"),
];

/// Loom's search index: path and the words inside (as `apps/files/src/search.rs` keeps them, sorted).
const LOOM_INDEX: &[(&str, &[&str])] = &[
    ("/home/budget-2026.ods", &[]),
    ("/voyeur/notes.txt", &["for", "lessons", "practice", "the", "tuesday", "xylophone"]),
    ("/voyeur/a-file-whose-name-is-far-too-long-to-show-whole/in-a-row/notes.txt", &["marimba"]),
];
/// Kaleidoscope's history (address, title, visits) and bookmarks (address, title).
const HISTORY: &[(&str, &str, u32)] = &[("https://example.org/platypus.html", "Platypus Field Notes", 3), ("https://sexos.dev/", "SexOS", 1)];
const BOOKMARKS: &[(&str, &str)] = &[("https://example.org/recipes", "Recipes")];

/// What Loom answers (`App::voyeur` over `Index::lookup`): names first, then words.
fn loom_answer(query: &str) -> Vec<wire::Record> {
    let terms: Vec<String> = query.split(' ').filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase()).collect();
    if terms.is_empty() { return vec![]; }
    let named = |p: &str| { let n = p.rsplit('/').next().unwrap().to_ascii_lowercase(); terms.iter().all(|t| n.contains(t.as_str())) };
    let worded = |w: &[&str]| terms.iter().all(|t| w.iter().any(|w| w.starts_with(t.as_str())));
    let hits = LOOM_INDEX.iter().filter(|(p, _)| named(p)).map(|(p, _)| (*p, false))
        .chain(LOOM_INDEX.iter().filter(|(p, w)| !named(p) && worded(w)).map(|(p, _)| (*p, true)));
    hits.take(wire::RESULTS_MAX).map(|(path, by_words)| {
        let fits = path.len() <= wire::SUBTITLE_MAX;
        let sub = if fits { path.to_string() } else { format!("\u{2026}{}", &path[path.len() - (wire::SUBTITLE_MAX - 3)..]) };
        let (kind, hint) = if by_words { (wire::KIND_CONTENT, 300) } else { (wire::KIND_FILE, 0) };
        let r = wire::Record::new(kind, wire::ACTION_OPEN, hint, 0, path.rsplit('/').next().unwrap(), &sub).unwrap();
        if fits { r.with(&[wire::ACTION_OPEN_WITH, wire::ACTION_REVEAL, wire::ACTION_COPY]) } else { r.with(&[wire::ACTION_OPEN_WITH, wire::ACTION_REVEAL]) }
    }).collect()
}
/// What Kaleidoscope answers (`Browser::voyeur` over `Profile::lookup`).
fn kaleidoscope_answer(query: &str) -> Vec<wire::Record> {
    let terms: Vec<String> = query.split(' ').filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase()).collect();
    if terms.is_empty() { return vec![]; }
    let fits = |u: &str, t: &str| terms.iter().all(|w| u.to_ascii_lowercase().contains(w.as_str()) || t.to_ascii_lowercase().contains(w.as_str()));
    let mut seen: Vec<_> = HISTORY.iter().rev().filter(|(u, t, _)| fits(u, t)).collect();
    seen.sort_by(|a, b| b.2.cmp(&a.2));
    BOOKMARKS.iter().rev().filter(|(u, t)| fits(u, t)).map(|(u, t)| (wire::KIND_BOOKMARK, *u, *t))
        .chain(seen.into_iter().map(|(u, t, _)| (wire::KIND_HISTORY, *u, *t))).take(wire::RESULTS_MAX)
        .map(|(kind, u, t)| wire::Record::new(kind, wire::ACTION_OPEN, 300, 0, if t.is_empty() { u } else { t }, u).unwrap().with(&[wire::ACTION_COPY]))
        .collect()
}
/// A program's answer as the Shell holds it: through the wire and its collector.
fn over_wire(records: &[wire::Record]) -> wire::Collector {
    let mut out = [wire::Packet { opcode: 0, args: [0; 3] }; wire::ANSWER_PACKETS];
    let n = wire::answer_packets(3, records, &mut out);
    let mut c = wire::Collector::new();
    c.begin(3);
    for p in &out[..n] { c.accept(*p).unwrap(); }
    assert!(c.done());
    c
}

/// The Shell's built-in providers as the Shell builds them, plus Abacus,
/// and the program providers Loom and Kaleidoscope over the wire.
struct Fixture { answer: String, clips: Vec<&'static str>, programs: Vec<wire::Collector> }
impl Fixture {
    fn new(query: &str) -> Self {
        Self { answer: val(query).unwrap_or_default(), clips: vec!["meeting at noon", "https://sexos.dev"],
               programs: vec![over_wire(&loom_answer(query)), over_wire(&kaleidoscope_answer(query))] }
    }
    fn candidates(&self) -> Vec<Candidate<'_>> {
        let mut v = Vec::new();
        if !self.answer.is_empty() {
            v.push(Candidate { provider: 0, weight: 100, kind: Kind::Calculator as u8, title: &self.answer, keywords: "", hint: 2000, hint_cap: HINT_CAP_BUILTIN });
        }
        for (t, k) in APPS { v.push(c(1, 100, Kind::App, t, k)); }
        v.push(c(2, 60, Kind::Window, "Quil", "window"));
        for s in SETTINGS { v.push(c(3, 90, Kind::Setting, s.name, s.keywords)); }
        for t in &self.clips { v.push(c(4, 60, Kind::Clipboard, t, "")); }
        for e in EMOJI { v.push(c(5, 50, Kind::Emoji, e.name, e.keywords)); }
        for (slot, p) in self.programs.iter().enumerate() {
            for r in p.records() {
                v.push(Candidate { provider: 6 + slot as u8, weight: 80, kind: r.kind, title: r.title(), keywords: r.subtitle(), hint: r.hint, hint_cap: HINT_CAP_EXTERNAL });
            }
        }
        v
    }
}

fn ranked(query: &str, frec: &Frecency, now: u64) -> Vec<(u8, String, u32)> {
    let f = Fixture::new(query);
    let cands = f.candidates();
    let mut out = [Ranked::default(); 16];
    let n = merge(query, now, frec, &cands, &mut out);
    out[..n].iter().map(|r| (cands[r.index].kind, cands[r.index].title.to_string(), r.score)).collect()
}
fn first(query: &str) -> (u8, String) {
    let r = ranked(query, &Frecency::new(), 0);
    let (k, t, _) = r.first().cloned().expect("no result");
    (k, t)
}

// ── Matching tiers ─────────────────────────────────────────────────────────
#[test] fn exact_beats_prefix() { assert!(title_score("quil", "Quil") > title_score("quil", "Quilt")); }
#[test] fn prefix_beats_word_prefix() { assert!(title_score("open", "Open Quil") > title_score("quil", "Open Quil")); }
#[test] fn word_prefix_beats_substring() { assert!(title_score("quil", "Open Quil") > title_score("uil", "Open Quil")); }
#[test] fn substring_beats_keyword_exact() { assert!(SUBSTRING > KEYWORD_EXACT); assert_eq!(title_score("pen", "Open"), SUBSTRING + 74); }
#[test] fn keyword_exact_beats_keyword_prefix() {
    assert_eq!(keyword_score("terminal", "terminal shell"), KEYWORD_EXACT);
    assert_eq!(keyword_score("term", "terminal shell"), KEYWORD_PREFIX);
    assert_eq!(keyword_score("erm", "terminal shell"), 0);
}
#[test] fn fuzzy_needs_a_word_start() {
    assert!(title_score("osp", "Open Spindle") >= FUZZY);
    assert_eq!(title_score("pnd", "Open Spindle"), 0);
}
#[test] fn one_letter_never_fuzzy() { assert_eq!(title_score("z", "Open Quil"), 0); }
#[test] fn ignores_ascii_case() { assert_eq!(title_score("OPEN QUIL", "open quil"), EXACT); }
#[test] fn trims_query_spaces() { assert_eq!(title_score("  quil ", "Quil"), EXACT); }
#[test] fn multi_word_prefixes() { assert!(title_score("op qu", "Open Quil") >= WORD_PREFIX); }
#[test] fn word_prefix_keeps_word_order() { assert!(title_score("qu op", "Open Quil") < WORD_PREFIX); }
#[test] fn no_match_is_zero() { assert_eq!(match_score("xyzzy", "Open Quil", "editor"), 0); }
#[test] fn closer_prefix_ranks_higher() { assert!(title_score("res", "Restart") > title_score("res", "Restore Minimized")); }

// ── Merge ──────────────────────────────────────────────────────────────────
#[test] fn empty_query_has_no_results() { assert!(ranked("   ", &Frecency::new(), 0).is_empty()); }
#[test] fn external_hint_is_capped() {
    let mut x = c(9, 100, Kind::Other, "spam", "");
    x.hint = u16::MAX;
    assert_eq!(score("zzz", 0, &Frecency::new(), &x), HINT_CAP_EXTERNAL as u32);
}
#[test] fn builtin_hint_counts_fully() {
    let a = Candidate { provider: 0, weight: 100, kind: Kind::Calculator as u8, title: "4", keywords: "", hint: 2000, hint_cap: HINT_CAP_BUILTIN };
    assert_eq!(score("2+2", 0, &Frecency::new(), &a), 2000);
}
#[test] fn provider_weight_scales_matches() {
    let full = c(0, 100, Kind::App, "Quil", "");
    let half = c(0, 50, Kind::App, "Quil", "");
    assert_eq!(score("quil", 0, &Frecency::new(), &full), 1000);
    assert_eq!(score("quil", 0, &Frecency::new(), &half), 500);
}
#[test] fn same_result_from_two_providers_appears_once() {
    let cands = [c(1, 60, Kind::App, "Open Quil", ""), c(0, 100, Kind::App, "open quil", "")];
    let mut out = [Ranked::default(); 4];
    assert_eq!(merge("open quil", 0, &Frecency::new(), &cands, &mut out), 1);
    assert_eq!(out[0], Ranked { index: 1, score: 1000 });
}
#[test] fn ties_go_to_earlier_provider_then_shorter_title() {
    let cands = [c(2, 100, Kind::App, "Open Quil", ""), c(1, 100, Kind::Window, "Open Quil", ""), c(1, 100, Kind::Setting, "Open Quilt", "")];
    let mut out = [Ranked::default(); 4];
    let n = merge("quil", 0, &Frecency::new(), &cands, &mut out);
    assert_eq!(n, 3);
    assert_eq!([out[0].index, out[1].index, out[2].index], [1, 0, 2]);
}
#[test] fn out_bounds_the_results() {
    let cands: Vec<_> = APPS.iter().map(|(t, k)| c(0, 100, Kind::App, t, k)).collect();
    let mut out = [Ranked::default(); 3];
    assert_eq!(merge("open", 0, &Frecency::new(), &cands, &mut out), 3);
}

// ── Frecency ───────────────────────────────────────────────────────────────
#[test] fn one_fresh_use_adds_ten() {
    let mut f = Frecency::new();
    f.record(key(1, "Open Quil"), 100);
    assert_eq!(f.score(key(1, "open quil"), 100), 10);
    assert_eq!(f.count(key(1, "Open Quil")), 1);
}
#[test] fn uses_decay_with_age() {
    assert_eq!([decay(0), decay(5 * 3600), decay(2 * 86400), decay(10 * 86400), decay(60 * 86400), decay(400 * 86400)], [100, 80, 60, 40, 20, 10]);
    let mut f = Frecency::new();
    for _ in 0..5 { f.record(7, 0); }
    assert_eq!(f.score(7, 0), 50);
    assert_eq!(f.score(7, 2 * 86400), 30);
}
#[test] fn frecency_is_capped() {
    let mut f = Frecency::new();
    for _ in 0..100 { f.record(7, 0); }
    assert_eq!(f.score(7, 0), FRECENCY_MAX);
}
#[test] fn a_full_table_forgets_the_least_valued_use() {
    let mut f = Frecency::new();
    for k in 1..=FRECENCY_SLOTS as u64 {
        if k == 3 { f.record(k, 0); } else { f.record(k, 1_000_000); f.record(k, 1_000_000); }
    }
    f.record(99, 1_000_000);
    assert_eq!(f.count(99), 1);
    assert_eq!(f.count(3), 0);
    assert_eq!((1..=FRECENCY_SLOTS as u64).filter(|k| f.count(*k) != 0).count(), FRECENCY_SLOTS - 1);
}
#[test] fn kind_separates_frecency() { assert_ne!(key(Kind::App as u8, "Quil"), key(Kind::Window as u8, "Quil")); }
#[test] fn frecency_breaks_a_tie() {
    let mut f = Frecency::new();
    for _ in 0..5 { f.record(key(Kind::App as u8, "Open Linen"), 10); }
    let r = ranked("open", &f, 10);
    assert_eq!(r[0].1, "Open Linen");
}
#[test] fn heavy_use_lifts_a_prefix_match_over_an_exact_one() {
    let mut f = Frecency::new();
    let cands = [c(0, 100, Kind::App, "Quil", ""), c(0, 100, Kind::App, "Quilt Editor", "")];
    for _ in 0..30 { f.record(key(Kind::App as u8, "Quilt Editor"), 0); }
    let mut out = [Ranked::default(); 2];
    merge("quil", 0, &f, &cands, &mut out);
    assert_eq!(out[0].index, 1);
}

// ── Abacus ─────────────────────────────────────────────────────────────────
#[test] fn abacus_adds() { assert_eq!(val("2+2").as_deref(), Some("4")); }
#[test] fn abacus_precedence() { assert_eq!(val("2+3*4").as_deref(), Some("14")); assert_eq!(val("(2+3)*4").as_deref(), Some("20")); }
#[test] fn abacus_divides_to_decimals() { assert_eq!(val("10/4").as_deref(), Some("2.5")); assert_eq!(val("1/3").as_deref(), Some("0.333333")); }
#[test] fn abacus_refuses_division_by_zero() { assert_eq!(val("1/0"), None); assert_eq!(val("5%0"), None); }
#[test] fn abacus_needs_an_operator() { assert_eq!(val("42"), None); assert_eq!(val("-5"), None); assert_eq!(val("hello"), None); assert_eq!(val("2+"), None); assert_eq!(val("x"), None); }
#[test] fn abacus_powers() { assert_eq!(val("2^10").as_deref(), Some("1024")); assert_eq!(val("2^3^2").as_deref(), Some("512")); assert_eq!(val("2^0.5"), None); }
#[test] fn abacus_signs() { assert_eq!(val("-3+5").as_deref(), Some("2")); assert_eq!(val("1-2.5").as_deref(), Some("-1.5")); assert_eq!(val("2*-3").as_deref(), Some("-6")); }
#[test] fn abacus_accepts_spaces_x_and_equals() { assert_eq!(val(" 3 x 4 =").as_deref(), Some("12")); assert_eq!(val("7%3").as_deref(), Some("1")); }
#[test] fn abacus_decimals_are_exact() { assert_eq!(val("0.1+0.2").as_deref(), Some("0.3")); }
#[test] fn abacus_refuses_overflow_and_depth() {
    assert_eq!(val("999999999999999999*999999999999999999"), None);
    assert_eq!(val(&("(".repeat(40) + "1+1" + &")".repeat(40))), None);
}

// ── Golden first hits over the Shell's built-in providers ─────────────────
#[test] fn golden_two_plus_two_is_four_first() { assert_eq!(first("2+2"), (Kind::Calculator as u8, "4".into())); }
#[test] fn golden_bell_merges_three_providers() {
    let r = ranked("bell", &Frecency::new(), 0);
    assert_eq!(r[0].1, "Open Bell");
    for (kind, title) in [(Kind::Setting, "Notifications"), (Kind::Emoji, "bell")] {
        assert!(r.iter().any(|(k, t, _)| *k == kind as u8 && t == title), "{title} missing from {r:?}");
    }
}
#[test] fn golden_setting_name_is_first() { assert_eq!(first("wallpaper"), (Kind::Setting as u8, "Wallpaper".into())); }
#[test] fn golden_setting_word_finds_its_page() { assert_eq!(first("brightness"), (Kind::Setting as u8, "Display".into())); }
#[test] fn golden_display_setting() { assert_eq!(first("display").1, "Display"); }
#[test] fn golden_term_finds_spindle() { assert_eq!(first("term").1, "Open Spindle"); }
#[test] fn golden_web_finds_the_browser_not_an_emoji() { assert_eq!(first("web").1, "Open Browser"); }
#[test] fn golden_qui_finds_quil() { assert_eq!(first("qui").1, "Open Quil"); }
#[test] fn golden_emoji_by_keyword() { assert_eq!(first("smile"), (Kind::Emoji as u8, "grinning face".into())); }
#[test] fn golden_clipboard_history_text() { assert_eq!(first("meeting"), (Kind::Clipboard as u8, "meeting at noon".into())); }
#[test] fn golden_window_by_app_name() {
    let r = ranked("quil", &Frecency::new(), 0);
    assert_eq!(r[0].1, "Open Quil");
    assert!(r.iter().any(|(k, t, _)| *k == Kind::Window as u8 && t == "Quil"));
}
#[test] fn golden_reboot_finds_restart() { assert_eq!(first("reboot").1, "Restart"); }

// ── R10: program providers (files and their words, browser history) ───────
#[test] fn wire_kinds_and_actions_are_voyeurs() {
    for (k, v) in [(Kind::App, wire::KIND_APP), (Kind::File, wire::KIND_FILE), (Kind::Content, wire::KIND_CONTENT), (Kind::History, wire::KIND_HISTORY), (Kind::Bookmark, wire::KIND_BOOKMARK)] {
        assert_eq!(k as u8, v);
    }
    for (a, v) in [(Action::Open, wire::ACTION_OPEN), (Action::OpenWith, wire::ACTION_OPEN_WITH), (Action::Reveal, wire::ACTION_REVEAL), (Action::Copy, wire::ACTION_COPY)] {
        assert_eq!(a as u8, v);
    }
    assert_eq!(Action::decode(wire::ACTIONS).map(Action::name), Some("run"));
    assert_eq!(Action::decode(wire::ACTIONS + 1), None);
}
/// The R10 gate on the host: `2+2`, a word inside a file and a setting
/// name each put the right result first with every provider answering.
#[test] fn golden_r10_first_hits() {
    assert_eq!(first("2+2"), (Kind::Calculator as u8, "4".into()));
    assert_eq!(first("xylophone"), (Kind::Content as u8, "notes.txt".into()));
    assert_eq!(first("wallpaper"), (Kind::Setting as u8, "Wallpaper".into()));
    assert_eq!(first("platypus"), (Kind::History as u8, "Platypus Field Notes".into()));
}
#[test] fn golden_file_name_is_first() { assert_eq!(first("budget"), (Kind::File as u8, "budget-2026.ods".into())); }
#[test] fn golden_bookmark_by_title() { assert_eq!(first("recipes"), (Kind::Bookmark as u8, "Recipes".into())); }
#[test] fn golden_history_by_address_word() { assert_eq!(first("example.org/platy"), (Kind::History as u8, "Platypus Field Notes".into())); }
#[test] fn a_word_prefix_inside_a_file_is_found() { assert_eq!(first("xylo").1, "notes.txt"); }
#[test] fn file_words_do_not_bury_a_setting_keyword() {
    // "practice" is only inside notes.txt; "brightness" names Display by keyword.
    assert_eq!(first("brightness").1, "Display");
    assert_eq!(first("practice").1, "notes.txt");
}
#[test] fn program_results_carry_their_actions_and_preview() {
    let f = Fixture::new("xylophone");
    let r = f.programs[0].records()[0];
    assert_eq!(r.subtitle(), "/voyeur/notes.txt");
    assert_eq!(r.actions().collect::<Vec<_>>(), vec![wire::ACTION_OPEN, wire::ACTION_OPEN_WITH, wire::ACTION_REVEAL, wire::ACTION_COPY]);
    // A path too long to show whole is cut at its start and cannot be copied.
    let f = Fixture::new("marimba");
    let r = f.programs[0].records()[0];
    assert!(r.subtitle().starts_with('\u{2026}') && r.subtitle().ends_with("/in-a-row/notes.txt"), "{}", r.subtitle());
    assert!(r.actions().all(|a| a != wire::ACTION_COPY));
    let f = Fixture::new("platypus");
    assert_eq!(f.programs[1].records()[0].actions().collect::<Vec<_>>(), vec![wire::ACTION_OPEN, wire::ACTION_COPY]);
}
#[test] fn other_actions_cross_the_wire_and_bad_ones_are_dropped() {
    let r = wire::Record::new(wire::KIND_FILE, wire::ACTION_REVEAL, 0, 9, "a", "/a").unwrap().with(&[wire::ACTION_OPEN, wire::ACTION_REVEAL]);
    assert_eq!(r.actions().collect::<Vec<_>>(), vec![wire::ACTION_REVEAL, wire::ACTION_OPEN]);
    assert_eq!(over_wire(&[r]).records(), &[r]);
    let mut out = [wire::Packet { opcode: 0, args: [0; 3] }; wire::ANSWER_PACKETS];
    wire::answer_packets(3, &[r], &mut out);
    out[0].args[0] |= 1 << 63; // an action past the last one
    let mut c = wire::Collector::new();
    c.begin(3);
    assert_eq!(c.accept(out[0]), Err(wire::CollectError::Malformed));
}
#[test] fn programs_only_answer_from_the_query() {
    // Nothing a provider answers depends on anything but the query text.
    assert_eq!(loom_answer("xylophone"), loom_answer("xylophone"));
    assert!(loom_answer("").is_empty() && kaleidoscope_answer("  ").is_empty());
}
