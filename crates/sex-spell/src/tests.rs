// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn recognizes_prose_contractions_suffixes_and_sexos_terms() {
    let eng = SpellEngine::new();
    for valid in [
        "hello", "world", "Hello", "WORLD", "preferences", "available", "connection",
        "don't", "can't", "I'm", "we’re", "friend’s", "cities", "stopped", "making",
        "running", "written", "earlier", "happily", "darkness", "movement", "readable",
        "usable", "teeth", "children", "well-known",
        "SexOS", "SilkBar", "Spindle", "Tether", "Tailor", "Quil", "Tatami", "Qupid",
        "Kaleidoscope", "Geist", "silkbar", "sexos",
    ] {
        assert!(eng.check_word(valid), "expected valid: {valid}");
    }
}

#[test]
fn rejects_misspellings_and_respects_orthographic_guards() {
    let eng = SpellEngine::new();
    for bad in [
        "teh", "recieve", "adn", "wiht", "dont", "cant", "im", "seperate",
        "definately", "thier", "wierd", "helllo", "wrold", "speling", "thiss",
        "boxs", "wishs", "TEH", "Recieve",
    ] {
        assert!(!eng.check_word(bad), "expected misspelled: {bad}");
    }
}

#[test]
fn skips_code_urls_numbers_snake_case_and_camel_case() {
    let eng = SpellEngine::new();
    for skip in [
        "https://example.com/weird_slug/foo",
        "http://localhost:8080/api/v1",
        "user@example.org",
        "/usr/local/bin/mytool",
        "snake_case_ident",
        "__private_var",
        "camelCaseVar",
        "myWidgetHelper",
        "RichTextInput",
        "std::vec::Vec",
        "main.rs",
        "v2",
        "0xdeadbeef",
        "12345",
        "UUID",
        "SIMD",
        "`inline_code`",
    ] {
        assert!(eng.check_word(skip), "expected skipped: {skip}");
    }
}

#[test]
fn learned_and_ignored_ring_buffer_overrides_spellcheck_and_autocorrect() {
    let mut eng = SpellEngine::new();
    assert!(!eng.check_word("foozballz"));
    eng.learn_word("Foozballz");
    assert!(eng.check_word("foozballz"));
    assert!(eng.check_word("FOOZBALLZ"));

    eng.ignore_word("teh");
    assert!(eng.check_word("teh"));
    assert_eq!(eng.autocorrect_on_space("teh"), None);

    let mut ring = RingWordSet::<3>::new();
    ring.insert("alpha");
    ring.insert("beta");
    ring.insert("gamma");
    ring.insert("beta"); // duplicate does not evict alpha
    assert!(ring.contains("ALPHA"));
    ring.insert("delta"); // evicts oldest ("alpha")
    assert!(!ring.contains("alpha"));
    assert!(ring.contains("beta"));
    assert!(ring.contains("gamma"));
    assert!(ring.contains("delta"));
}

#[test]
fn suggest_returns_top_ranked_damerau_and_phonetic_fixes() {
    let eng = SpellEngine::new();
    let mut out = [WordBuf::new(); 3];

    let n = eng.suggest("recieve", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "receive");

    let n = eng.suggest("Teh", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "The");

    let n = eng.suggest("helllo", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "hello");

    let n = eng.suggest("wrold", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "world");

    let n = eng.suggest("speling", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "spelling");

    let n = eng.suggest("cant", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "can't");

    let n = eng.suggest("fone", &mut out);
    assert!(n >= 1);
    assert_eq!(out[0].as_str(), "phone");
}

#[test]
fn predict_completion_returns_unambiguous_high_freq_suffixes() {
    let eng = SpellEngine::new();
    assert_eq!(eng.predict_completion("pr"), None, "needs >= 3 chars");
    assert_eq!(eng.predict_completion("pref").map(|w| w).as_ref().map(|w| w.as_str()), Some("erences"));
    assert_eq!(eng.predict_completion("avail").as_ref().map(|w| w.as_str()), Some("able"));
    assert_eq!(eng.predict_completion("connec").as_ref().map(|w| w.as_str()), Some("tion"));
    assert_eq!(eng.predict_completion("settin").as_ref().map(|w| w.as_str()), Some("gs"));
    assert_eq!(eng.predict_completion("Silk").as_ref().map(|w| w.as_str()), Some("Bar"));
    // Already common exact word should not nag with suffix completion
    assert_eq!(eng.predict_completion("the"), None);
    assert_eq!(eng.predict_completion("with"), None);
}

#[test]
fn autocorrect_on_space_covers_common_typos_and_case() {
    let eng = SpellEngine::new();
    let cases = [
        ("teh", "the"),
        ("Teh", "The"),
        ("TEH", "THE"),
        ("recieve", "receive"),
        ("adn", "and"),
        ("wiht", "with"),
        ("dont", "don't"),
        ("Dont", "Don't"),
        ("cant", "can't"),
        ("im", "I'm"),
        ("seperate", "separate"),
        ("definately", "definitely"),
    ];
    for (typo, expected) in cases {
        let got = eng.autocorrect_on_space(typo);
        assert_eq!(got.as_ref().map(|w| w.as_str()), Some(expected), "typo={typo}");
    }
    assert_eq!(eng.autocorrect_on_space("hello"), None);
}

#[test]
fn scan_misspelled_respects_mode_and_finds_exact_byte_spans() {
    let eng = SpellEngine::new();
    let text = "Visit https://example.com/bad_slug for teh recieve snake_case_bad myCamelToken.";
    let mut spans = [WordSpan { start: 0, end: 0 }; 8];
    let mut n = 0usize;
    eng.scan_misspelled(text, TextMode::Prose, |s| {
        if n < spans.len() {
            spans[n] = s;
            n += 1;
        }
    });
    assert_eq!(n, 2);
    assert_eq!(&text[spans[0].start..spans[0].end], "teh");
    assert_eq!(&text[spans[1].start..spans[1].end], "recieve");

    // Disabled in Code / Password / Url / Terminal modes
    for mode in [TextMode::Code, TextMode::Password, TextMode::Url, TextMode::Terminal] {
        let mut count = 0usize;
        eng.scan_misspelled(text, mode, |_| count += 1);
        assert_eq!(count, 0, "mode={mode:?}");
    }
}

#[test]
fn active_word_prefix_and_word_span_at_handle_boundaries() {
    let text = "foo_pre hello pref";
    assert_eq!(active_word_prefix_at(text, 7), None, "skip snake_case suffix");
    assert_eq!(active_word_prefix_at(text, text.len()), Some((14, "pref")));
    assert_eq!(active_word_prefix_at("prefer", 4), None, "mid-word cursor must not predict");

    assert_eq!(word_span_at("hello, recieve!", 9), Some(WordSpan { start: 7, end: 14 }));
    assert_eq!(word_span_at("hello, recieve!", 5), Some(WordSpan { start: 0, end: 5 }));
    assert_eq!(word_span_at("hello  world", 6), None);
}
