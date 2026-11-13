// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The layouts, one per country or design, written row by row as on the keyboard.
//!
//! Each row lists its keys left to right, separated by one space. A key lists what
//! it types at each level: plain, Shift, AltGr, Shift+AltGr. A combining mark
//! (U+0300..U+036F) at a level makes that level a dead key: `"\u{301}"` is a dead
//! acute. Positions follow the usual names (TLDE AE01..AE12 / AD01..AD12 /
//! AC01..AC11 BKSL / LSGT AB01..AB10); the key codes are in [`ROW_CODES`].
//! The characters each country expects were checked against its national
//! standard layout; the files are our own data, not copied from any keymap source.

/// Which key codes (PS/2 set 1, as Input delivers them) each row's keys are.
pub const ROW_CODES: [&[u8]; 4] = [
    &[0x29, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D],
    &[0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B],
    &[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2B],
    &[0x56, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35],
];

pub struct Def {
    /// Short name, as a person picks it: `de`, `us-dvorak`.
    pub name: &'static str,
    /// Two or three letters for the panel.
    pub label: &'static str,
    pub title: &'static str,
    pub rows: [&'static str; 4],
}

const US_R0: &str = "`~ 1! 2@ 3# 4$ 5% 6^ 7& 8* 9( 0) -_ =+";
const US_R1: &str = "qQ wW eE rR tT yY uU iI oO pP [{ ]}";
const US_R2: &str = "aA sS dD fF gG hH jJ kK lL ;: '\" \\|";
const US_R3: &str = "<> zZ xX cC vV bB nN mM ,< .> /?";
const NORDIC_R3: &str = "<>| zZ xX cC vV bB nN mMµ ,; .: -_";

pub static ALL: [Def; 16] = [
    Def { name: "us", label: "US", title: "English (US)", rows: [US_R0, US_R1, US_R2, US_R3] },
    Def {
        name: "us-intl", label: "USI", title: "English (US, international)",
        rows: [
            "\u{300}\u{303}`~ 1!¡¹ 2@² 3#³ 4$¤£ 5%€ 6\u{302}¼ 7&½ 8*¾ 9(‘ 0)’ -_¥ =+×÷",
            "qQäÄ wWåÅ eEéÉ rR® tTþÞ yYüÜ uUúÚ iIíÍ oOóÓ pPöÖ [{« ]}»",
            "aAáÁ sSß§ dDðÐ fF gG hH jJ kK lLøØ ;:¶° \u{301}\u{308}´¨ \\|¬¦",
            "<>\\| zZæÆ xX cC©¢ vV bB nNñÑ mMµ ,<çÇ .> /?¿",
        ],
    },
    Def {
        name: "gb", label: "GB", title: "English (UK)",
        rows: [
            "`¬¦ 1!¹ 2\"² 3£³ 4$€ 5%½ 6^¾ 7&{ 8*[ 9(] 0)} -_\\ =+",
            "qQ@ wW eEé rR tT yY uUú iIí oOó pP [{ ]}",
            "aAá sS dD fF gG hH jJ kK lL ;: '@ #~",
            "\\| zZ xX cC vV bB nN mM ,< .> /?",
        ],
    },
    Def {
        name: "de", label: "DE", title: "German",
        rows: [
            "\u{302}°′″ 1!¹¡ 2\"²⅛ 3§³£ 4$¼¤ 5%½⅜ 6&¬⅝ 7/{⅞ 8([™ 9)]± 0=}° ß?\\¿ \u{301}\u{300}\u{327}\u{328}",
            "qQ@Ω wWłŁ eE€€ rR¶® tTŧŦ zZ←¥ uU↓↑ iI→ı oOøØ pPþÞ üÜ\u{308}\u{30a} +*~¯",
            "aAæÆ sSſẞ dDðÐ fFđª gGŋŊ hHħĦ jJ\u{323}\u{307} kKĸ& lLłŁ öÖ\u{30b}\u{323} äÄ\u{302}\u{30c} #'’\u{306}",
            "<>| yY»› xX«‹ cC¢© vV„‚ bB“‘ nN”’ mMµº ,;·× .:…÷ -_–—",
        ],
    },
    Def {
        name: "fr", label: "FR", title: "French (AZERTY)",
        rows: [
            "²~¬ &1\u{30c}˛ é2~É \"3#˘ '4{— (5[– -6|„ è7`È _8\\™ ç9^Ç à0@À )°]≠ =+}±",
            "aAæÆ zZâÂ eE€¢ rRêÊ tTþÞ yYÿŸ uUûÛ iIîÎ oOœŒ pPôÔ \u{302}\u{308}\u{303}\u{30a} $£øØ",
            "qQäÄ sSß dDëË fF gG hHðÐ jJ kK lL mM ù%\u{301}Ù *µ\u{300}\u{304}",
            "<>| wW«“ xX»” cC©® vV bB nN¬ ,?¿ ;.× :/÷ !§¡",
        ],
    },
    Def {
        name: "es", label: "ES", title: "Spanish",
        rows: [
            "ºª\\ 1!| 2\"@ 3·# 4$~ 5%½ 6&¬ 7/ 8( 9) 0= '? ¡¿",
            "qQ wW eE€ rR tT yY uU iI oO pP \u{300}\u{302}[ +*]",
            "aA sS dD fF gG hH jJ kK lL ñÑ~ \u{301}\u{308}{ çÇ}",
            "<> zZ xX cC vV bB nN mM ,; .: -_",
        ],
    },
    Def {
        name: "it", label: "IT", title: "Italian",
        rows: [
            "\\|¬ 1!¹ 2\"² 3£³ 4$¼ 5%½ 6&¬ 7/{ 8([ 9)] 0=} '?` ì^~",
            "qQ@ wW eE€ rR tT yY uU iI oO pP èé[{ +*]}",
            "aA sS dD fF gG hH jJ kK lL òç@ à°# ù§",
            "<> zZ xX cC vV bB nN mM ,; .: -_",
        ],
    },
    Def {
        name: "pt", label: "PT", title: "Portuguese",
        rows: [
            "\\| 1! 2\"@ 3#£ 4$§ 5%€ 6& 7/{ 8([ 9)] 0=} '? «»",
            "qQ@ wW eE€ rR tT yY uU iI oO pP +*\u{308} \u{301}\u{300}",
            "aA sS dD fF gG hH jJ kK lL çÇ ºª \u{303}\u{302}",
            "<>\\ zZ xX cC vV bB nN mM ,; .: -_",
        ],
    },
    Def {
        name: "se", label: "SE", title: "Swedish",
        rows: [
            "§½ 1!¡ 2\"@ 3#£ 4¤$ 5%€ 6&¥ 7/{ 8([ 9)] 0=} +?\\ \u{301}\u{300}",
            "qQ wW eE€ rR tT yY uU iI oO pP åÅ \u{308}\u{302}\u{303}",
            "aA sS dD fF gG hH jJ kK lL öÖ äÄ '*",
            NORDIC_R3,
        ],
    },
    Def {
        name: "fi", label: "FI", title: "Finnish",
        rows: [
            "§½ 1!¡ 2\"@ 3#£ 4¤$ 5%€ 6&¥ 7/{ 8([ 9)] 0=} +?\\ \u{301}\u{300}",
            "qQ wW eE€ rR tT yY uU iI oOœŒ pP åÅ \u{308}\u{302}\u{303}",
            "aAəƏ sSšŠ dD fF gG hH jJ kK lL öÖøØ äÄæÆ '*",
            "<>| zZžŽ xX cC vV bB nN mMµ ,; .: -_",
        ],
    },
    Def {
        name: "no", label: "NO", title: "Norwegian",
        rows: [
            "|§ 1!¡ 2\"@ 3#£ 4¤$ 5%€ 6&¥ 7/{ 8([ 9)] 0=} +? \\\u{300}\u{301}",
            "qQ wW eE€ rR tT yY uU iI oO pP åÅ \u{308}\u{302}\u{303}",
            "aA sS dD fF gG hH jJ kK lL øØ æÆ '*",
            NORDIC_R3,
        ],
    },
    Def {
        name: "dk", label: "DK", title: "Danish",
        rows: [
            "½§ 1!¡ 2\"@ 3#£ 4¤$ 5%€ 6&¥ 7/{ 8([ 9)] 0=} +? \u{301}\u{300}|",
            "qQ wW eE€ rR tT yY uU iI oO pP åÅ \u{308}\u{302}\u{303}",
            "aA sS dD fF gG hH jJ kK lL æÆ øØ '*",
            "<>\\ zZ xX cC vV bB nN mMµ ,; .: -_",
        ],
    },
    Def {
        name: "us-dvorak", label: "DV", title: "English (Dvorak)",
        rows: [
            "`~ 1! 2@ 3# 4$ 5% 6^ 7& 8* 9( 0) [{ ]}",
            "'\" ,< .> pP yY fF gG cC rR lL /? =+",
            "aA oO eE uU iI dD hH tT nN sS -_ \\|",
            "<> ;: qQ jJ kK xX bB mM wW vV zZ",
        ],
    },
    Def {
        name: "us-colemak", label: "CM", title: "English (Colemak)",
        rows: [
            US_R0,
            "qQ wW fF pP gG jJ lL uU yY ;: [{ ]}",
            "aA rR sS tT dD hH nN eE iI oO '\" \\|",
            "<> zZ xX cC vV bB kK mM ,< .> /?",
        ],
    },
    // Pinyin: the US keys, with Shell's input method turning letters into Chinese.
    Def { name: "zh-pinyin", label: "PY", title: "Chinese (Pinyin)", rows: [US_R0, US_R1, US_R2, US_R3] },
    Def {
        name: "ru", label: "RU", title: "Russian",
        rows: [
            "ёЁ 1! 2\" 3№ 4; 5% 6: 7? 8* 9( 0) -_ =+",
            "йЙ цЦ уУ кК еЕ нН гГ шШ щЩ зЗ хХ ъЪ",
            "фФ ыЫ вВ аА пП рР оО лЛ дД жЖ эЭ \\/",
            "/| яЯ чЧ сС мМ иИ тТ ьЬ бБ юЮ .,",
        ],
    },
];

/// The key code that types `letter` (a-z) on a US keyboard.
pub fn us_code(letter: char) -> Option<u8> {
    for (row, codes) in [US_R0, US_R1, US_R2, US_R3].iter().zip(ROW_CODES) {
        for (key, &code) in row.split(' ').zip(codes) {
            if key.starts_with(letter) { return Some(code); }
        }
    }
    None
}

pub fn by_name(name: &str) -> Option<usize> { ALL.iter().position(|d| d.name == name) }
