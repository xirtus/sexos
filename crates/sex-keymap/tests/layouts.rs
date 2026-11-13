// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Host tests for every layout, dead keys, AltGr, Caps Lock, Compose and switching.
use sex_keymap::{code, layouts, Keyboard, Typed};

fn kb(name: &str) -> Keyboard { let mut k = Keyboard::new(); assert!(k.choose([name]), "{name} parses"); k }
/// Presses a key with these modifiers held, releasing them after.
fn with(k: &mut Keyboard, mods: &[u8], key: u8) -> Typed {
    for &m in mods { k.observe(m, 1); }
    let t = k.press(key);
    for &m in mods { k.observe(m, 0); }
    t
}
fn ch(k: &mut Keyboard, key: u8) -> Option<char> { match k.press(key) { Typed::Char(c) => Some(c), _ => None } }
fn shifted(k: &mut Keyboard, key: u8) -> Option<char> { match with(k, &[code::SHIFT_LEFT], key) { Typed::Char(c) => Some(c), _ => None } }
fn altgr(k: &mut Keyboard, key: u8) -> Option<char> { match with(k, &[code::ALT_GR], key) { Typed::Char(c) => Some(c), _ => None } }

#[test]
fn every_layout_parses_and_fills_every_row() {
    for def in &layouts::ALL {
        let mut k = kb(def.name);
        for codes in layouts::ROW_CODES { for &c in codes { assert!(ch(&mut k, c).is_some() || k.pending(), "{} {c:#x}", def.name); if k.pending() { k.press(code::SPACE); } } }
        assert!(def.label.len() <= 3 && !def.title.is_empty());
    }
    assert!(layouts::ALL.len() >= 8);
}

#[test]
fn us_matches_the_table_programs_used_before() {
    let plain: &[u8; 58] = b"\0\x001234567890-=\0\0qwertyuiop[]\0\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
    let upper: &[u8; 58] = b"\0\0!@#$%^&*()_+\0\0QWERTYUIOP{}\0\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";
    let mut k = kb("us");
    for code in 0..58u8 {
        let want = |t: &[u8; 58]| (t[code as usize] != 0).then(|| t[code as usize] as char);
        assert_eq!(ch(&mut k, code), want(plain), "plain {code:#x}");
        assert_eq!(shifted(&mut k, code), want(upper), "shift {code:#x}");
    }
}

#[test]
fn golden_keys_per_layout() {
    // (layout, code, plain, shift, altgr)
    let golden: &[(&str, u8, char, char, Option<char>)] = &[
        ("gb", 0x04, '3', '£', None), ("gb", 0x28, '\'', '@', None), ("gb", 0x2B, '#', '~', None),
        ("de", 0x15, 'z', 'Z', Some('←')), ("de", 0x2C, 'y', 'Y', Some('»')), ("de", 0x1A, 'ü', 'Ü', None),
        ("de", 0x27, 'ö', 'Ö', None), ("de", 0x28, 'ä', 'Ä', None), ("de", 0x0C, 'ß', '?', Some('\\')),
        ("de", 0x10, 'q', 'Q', Some('@')), ("de", 0x12, 'e', 'E', Some('€')), ("de", 0x08, '7', '/', Some('{')),
        ("fr", 0x10, 'a', 'A', None), ("fr", 0x1E, 'q', 'Q', None), ("fr", 0x11, 'z', 'Z', None),
        ("fr", 0x02, '&', '1', None), ("fr", 0x03, 'é', '2', Some('~')), ("fr", 0x0B, 'à', '0', Some('@')),
        ("fr", 0x27, 'm', 'M', None), ("fr", 0x32, ',', '?', None),
        ("es", 0x27, 'ñ', 'Ñ', None), ("es", 0x03, '2', '"', Some('@')), ("es", 0x2B, 'ç', 'Ç', Some('}')),
        ("it", 0x1A, 'è', 'é', Some('[')), ("it", 0x27, 'ò', 'ç', Some('@')), ("it", 0x28, 'à', '°', Some('#')),
        ("pt", 0x27, 'ç', 'Ç', None), ("pt", 0x28, 'º', 'ª', None), ("pt", 0x03, '2', '"', Some('@')),
        ("se", 0x1A, 'å', 'Å', None), ("se", 0x27, 'ö', 'Ö', None), ("se", 0x28, 'ä', 'Ä', None), ("se", 0x05, '4', '¤', Some('$')),
        ("fi", 0x27, 'ö', 'Ö', Some('ø')), ("fi", 0x1F, 's', 'S', Some('š')),
        ("no", 0x27, 'ø', 'Ø', None), ("no", 0x28, 'æ', 'Æ', None),
        ("dk", 0x27, 'æ', 'Æ', None), ("dk", 0x28, 'ø', 'Ø', None),
        ("us-dvorak", 0x10, '\'', '"', None), ("us-dvorak", 0x1F, 'o', 'O', None), ("us-dvorak", 0x2C, ';', ':', None),
        ("us-colemak", 0x12, 'f', 'F', None), ("us-colemak", 0x1F, 'r', 'R', None), ("us-colemak", 0x31, 'k', 'K', None),
        ("ru", 0x10, 'й', 'Й', None), ("ru", 0x1E, 'ф', 'Ф', None), ("ru", 0x04, '3', '№', None),
        ("us-intl", 0x12, 'e', 'E', Some('é')), ("us-intl", 0x31, 'n', 'N', Some('ñ')),
    ];
    for &(name, code, plain, shift, alt) in golden {
        let mut k = kb(name);
        assert_eq!(ch(&mut k, code), Some(plain), "{name} {code:#x}");
        assert_eq!(shifted(&mut k, code), Some(shift), "{name} shift {code:#x}");
        if let Some(a) = alt { assert_eq!(altgr(&mut k, code), Some(a), "{name} altgr {code:#x}"); }
    }
}

#[test]
fn dead_keys_make_accented_letters() {
    let mut k = kb("de");
    assert_eq!(k.press(0x0D), Typed::Pending); // ´
    assert_eq!(ch(&mut k, 0x12), Some('é'));
    assert_eq!(with(&mut k, &[code::SHIFT_LEFT], 0x0D), Typed::Pending); // `
    assert_eq!(shifted(&mut k, 0x1E), Some('À'));
    assert_eq!(k.press(0x29), Typed::Pending); // ^
    assert_eq!(ch(&mut k, 0x16), Some('û'));
    // Space and a second press give the accent itself; an unknown pair gives the letter.
    k.press(0x29); assert_eq!(ch(&mut k, code::SPACE), Some('^'));
    k.press(0x0D); assert_eq!(ch(&mut k, 0x0D), Some('´'));
    k.press(0x0D); assert_eq!(ch(&mut k, 0x2D), Some('x'));
    // A non-typing key ends the wait.
    k.press(0x0D); assert_eq!(k.press(0x1C), Typed::None); assert!(!k.pending());
    let mut fr = kb("fr");
    fr.press(0x1A); assert_eq!(ch(&mut fr, 0x12), Some('ê'));
    assert_eq!(with(&mut fr, &[code::SHIFT_LEFT], 0x1A), Typed::Pending); assert_eq!(ch(&mut fr, 0x12), Some('ë'));
    let mut es = kb("es");
    es.press(0x28); assert_eq!(ch(&mut es, 0x18), Some('ó'));
    let mut intl = kb("us-intl");
    intl.press(0x28); assert_eq!(ch(&mut intl, 0x2E), Some('ć'));
    intl.press(0x28); assert_eq!(ch(&mut intl, code::SPACE), Some('´'));
    let mut se = kb("se");
    se.press(0x1B); assert_eq!(ch(&mut se, 0x16), Some('ü'));
}

#[test]
fn caps_lock_shifts_letters_only() {
    let mut k = kb("de");
    k.observe(code::CAPS_LOCK, 1); k.observe(code::CAPS_LOCK, 0);
    assert!(k.caps_lock());
    assert_eq!(ch(&mut k, 0x1A), Some('Ü'));
    assert_eq!(ch(&mut k, 0x02), Some('1'));
    assert_eq!(shifted(&mut k, 0x1E), Some('a'));
    k.observe(code::CAPS_LOCK, 1);
    assert_eq!(ch(&mut k, 0x1E), Some('a'));
}

#[test]
fn shortcuts_type_nothing() {
    let mut k = kb("us");
    assert_eq!(with(&mut k, &[code::CTRL_LEFT], 0x1F), Typed::None);
    assert_eq!(with(&mut k, &[code::CTRL_RIGHT], 0x1F), Typed::None);
    assert_eq!(with(&mut k, &[code::ALT_LEFT], 0x1F), Typed::None);
    assert_eq!(ch(&mut k, 0x1F), Some('s'));
    assert_eq!(k.press(0x3B), Typed::None); // F1
}

#[test]
fn compose_sequences() {
    let mut k = kb("de");
    let seq = |k: &mut Keyboard, keys: &[(u8, bool)]| -> Vec<Typed> {
        k.observe(code::MENU, 1); k.observe(code::MENU, 0);
        keys.iter().map(|&(c, s)| if s { with(k, &[code::SHIFT_LEFT], c) } else { k.press(c) }).collect()
    };
    // DE: o is 0x18, " is Shift+2, = is Shift+0, e is 0x12, - is 0x35, s is 0x1F.
    assert_eq!(seq(&mut k, &[(0x18, false), (0x03, true)]), [Typed::Pending, Typed::Char('ö')]);
    assert_eq!(seq(&mut k, &[(0x0B, true), (0x12, false)]), [Typed::Pending, Typed::Char('€')]);
    assert_eq!(seq(&mut k, &[(0x35, false), (0x35, false), (0x35, false)]), [Typed::Pending, Typed::Pending, Typed::Char('—')]);
    assert_eq!(seq(&mut k, &[(0x1F, false), (0x1F, false)]), [Typed::Pending, Typed::Char('ß')]);
    // An unknown sequence types nothing and ends Compose.
    assert_eq!(seq(&mut k, &[(0x10, false), (0x10, false)]), [Typed::Pending, Typed::Char('q')]);
    assert_eq!(ch(&mut k, 0x2D), Some('x'));
    // US has no AltGr levels, so right Alt is Compose there: AltGr ' e -> é.
    let mut us = kb("us");
    us.observe(code::ALT_GR, 1); us.observe(code::ALT_GR, 0);
    assert_eq!(us.press(0x28), Typed::Pending);
    assert_eq!(ch(&mut us, 0x12), Some('é'));
    us.observe(code::ALT_GR, 1); us.observe(code::ALT_GR, 0);
    us.press(0x2E); assert_eq!(ch(&mut us, 0x35), Some('¢'));
    us.observe(code::ALT_GR, 1); us.observe(code::ALT_GR, 0);
    us.press(0x0D); assert_eq!(ch(&mut us, 0x12), Some('€'));
}

#[test]
fn switching_cycles_the_chosen_layouts() {
    let mut k = Keyboard::new();
    assert_eq!(ch(&mut k, 0x15), Some('y')); // US until told otherwise
    assert!(k.choose(["us", "nope", "de", "fr", "de"]));
    assert_eq!(k.chosen().count(), 3);
    assert_eq!(k.layout().label, "US");
    assert_eq!(k.next_layout().label, "DE");
    assert_eq!(ch(&mut k, 0x15), Some('z'));
    assert_eq!(k.next_layout().label, "FR");
    assert_eq!(ch(&mut k, 0x10), Some('a'));
    assert_eq!(k.next_layout().label, "US");
    assert!(!k.choose(["xx"]));
    assert_eq!(k.layout().label, "US");
}
