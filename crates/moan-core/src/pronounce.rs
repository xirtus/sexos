// SPDX-License-Identifier: GPL-3.0-only
//! Standalone `#![no_std]` English grapheme-to-phoneme (ARPABET) and Japanese
//! Romaji/Kana pronunciation engines.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Common music and lyric words mapped to standard Hillenbrand/Klatt ARPABET sequences.
/// Kept strictly in ASCII alphabetical order for O(log N) binary search.
pub const COMMON_MUSIC_EXCEPTIONS: &[(&'static str, &'static str)] = &[
    ("a", "AH"),
    ("alive", "AH L AY V"),
    ("all", "AO L"),
    ("alone", "AH L OW N"),
    ("already", "AO L R EH D IY"),
    ("always", "AO L W EY Z"),
    ("am", "AE M"),
    ("an", "AE N"),
    ("and", "AE N D"),
    ("angel", "EY N JH AH L"),
    ("are", "AA R"),
    ("arms", "AA R M Z"),
    ("around", "ER AW N D"),
    ("as", "AE Z"),
    ("at", "AE T"),
    ("away", "AH W EY"),
    ("baby", "B EY B IY"),
    ("back", "B AE K"),
    ("bass", "B EY S"),
    ("beat", "B IY T"),
    ("beautiful", "B Y UW T AH F UH L"),
    ("before", "B IH F AO R"),
    ("believe", "B IH L IY V"),
    ("better", "B EH T ER"),
    ("bitter", "B IH T ER"),
    ("blaze", "B L EY Z"),
    ("blood", "B L AH D"),
    ("blue", "B L UW"),
    ("body", "B AA D IY"),
    ("boy", "B OY"),
    ("break", "B R EY K"),
    ("breathe", "B R IY DH"),
    ("bright", "B R AY T"),
    ("burn", "B ER N"),
    ("but", "B AH T"),
    ("by", "B AY"),
    ("can", "K AE N"),
    ("city", "S IH T IY"),
    ("club", "K L AH B"),
    ("cold", "K OW L D"),
    ("come", "K AH M"),
    ("control", "K AH N T R OW L"),
    ("could", "K UH D"),
    ("crazy", "K R EY Z IY"),
    ("cry", "K R AY"),
    ("cyber", "S AY B ER"),
    ("dance", "D AE N S"),
    ("danger", "D EY N JH ER"),
    ("dark", "D AA R K"),
    ("darkness", "D AA R K N AH S"),
    ("day", "D EY"),
    ("deep", "D IY P"),
    ("desire", "D IH Z AY ER"),
    ("disco", "D IH S K OW"),
    ("do", "D UW"),
    ("down", "D AW N"),
    ("dream", "D R IY M"),
    ("drive", "D R AY V"),
    ("drum", "D R AH M"),
    ("echo", "EH K OW"),
    ("electric", "IH L EH K T R IH K"),
    ("energy", "EH N ER JH IY"),
    ("ever", "EH V ER"),
    ("every", "EH V R IY"),
    ("eyes", "AY Z"),
    ("fade", "F EY D"),
    ("fall", "F AO L"),
    ("faster", "F AE S T ER"),
    ("fear", "F IH R"),
    ("feel", "F IY L"),
    ("feeling", "F IY L IH NG"),
    ("fight", "F AY T"),
    ("fire", "F AY ER"),
    ("fly", "F L AY"),
    ("for", "F AO R"),
    ("forever", "F ER EH V ER"),
    ("forget", "F ER G EH T"),
    ("free", "F R IY"),
    ("freedom", "F R IY D AH M"),
    ("from", "F R AH M"),
    ("future", "F Y UW CH ER"),
    ("game", "G EY M"),
    ("girl", "G ER L"),
    ("give", "G IH V"),
    ("glow", "G L OW"),
    ("go", "G OW"),
    ("gold", "G OW L D"),
    ("good", "G UH D"),
    ("groove", "G R UW V"),
    ("guitar", "G IH T AA R"),
    ("hands", "HH AE N D Z"),
    ("happy", "HH AE P IY"),
    ("harder", "HH AA R D ER"),
    ("have", "HH AE V"),
    ("hear", "HH IH R"),
    ("heart", "HH AA R T"),
    ("heaven", "HH EH V AH N"),
    ("heavy", "HH EH V IY"),
    ("hell", "HH EH L"),
    ("hello", "HH EH L OW"),
    ("high", "HH AY"),
    ("hold", "HH OW L D"),
    ("home", "HH OW M"),
    ("hope", "HH OW P"),
    ("house", "HH AW S"),
    ("human", "HH Y UW M AH N"),
    ("i", "AY"),
    ("in", "IH N"),
    ("into", "IH N T UW"),
    ("is", "IH Z"),
    ("it", "IH T"),
    ("joy", "JH OY"),
    ("jump", "JH AH M P"),
    ("kiss", "K IH S"),
    ("know", "N OW"),
    ("leave", "L IY V"),
    ("life", "L AY F"),
    ("light", "L AY T"),
    ("like", "L AY K"),
    ("lips", "L IH P S"),
    ("live", "L IH V"),
    ("lonely", "L OW N L IY"),
    ("look", "L UH K"),
    ("lose", "L UW Z"),
    ("lost", "L AO S T"),
    ("love", "L AH V"),
    ("low", "L OW"),
    ("machine", "M AH SH IY N"),
    ("magic", "M AE JH IH K"),
    ("make", "M EY K"),
    ("man", "M AE N"),
    ("me", "M IY"),
    ("midnight", "M IH D N AY T"),
    ("mind", "M AY N D"),
    ("moon", "M UW N"),
    ("move", "M UW V"),
    ("music", "M Y UW Z IH K"),
    ("my", "M AY"),
    ("never", "N EH V ER"),
    ("night", "N AY T"),
    ("no", "N OW"),
    ("not", "N AA T"),
    ("now", "N AW"),
    ("ocean", "OW SH AH N"),
    ("of", "AH V"),
    ("on", "AA N"),
    ("one", "W AH N"),
    ("only", "OW N L IY"),
    ("our", "AW ER"),
    ("out", "AW T"),
    ("over", "OW V ER"),
    ("pain", "P EY N"),
    ("paradise", "P AE R AH D AY S"),
    ("party", "P AA R T IY"),
    ("passion", "P AE SH AH N"),
    ("peace", "P IY S"),
    ("people", "P IY P AH L"),
    ("play", "P L EY"),
    ("power", "P AW ER"),
    ("radio", "R EY D IY OW"),
    ("rain", "R EY N"),
    ("ready", "R EH D IY"),
    ("rhythm", "R IH DH AH M"),
    ("ride", "R AY D"),
    ("right", "R AY T"),
    ("rise", "R AY Z"),
    ("river", "R IH V ER"),
    ("road", "R OW D"),
    ("robot", "R OW B AA T"),
    ("rock", "R AA K"),
    ("roll", "R OW L"),
    ("run", "R AH N"),
    ("sad", "S AE D"),
    ("say", "S EY"),
    ("scream", "S K R IY M"),
    ("sea", "S IY"),
    ("secret", "S IY K R AH T"),
    ("see", "S IY"),
    ("shake", "SH EY K"),
    ("shine", "SH AY N"),
    ("shout", "SH AW T"),
    ("silence", "S AY L AH N S"),
    ("silent", "S AY L AH N T"),
    ("sing", "S IH NG"),
    ("singing", "S IH NG IH NG"),
    ("sky", "S K AY"),
    ("smile", "S M AY L"),
    ("so", "S OW"),
    ("some", "S AH M"),
    ("song", "S AO NG"),
    ("soul", "S OW L"),
    ("sound", "S AW N D"),
    ("space", "S P EY S"),
    ("spark", "S P AA R K"),
    ("stand", "S T AE N D"),
    ("star", "S T AA R"),
    ("stars", "S T AA R Z"),
    ("start", "S T AA R T"),
    ("stay", "S T EY"),
    ("stop", "S T AA P"),
    ("storm", "S T AO R M"),
    ("street", "S T R IY T"),
    ("stronger", "S T R AO NG ER"),
    ("sun", "S AH N"),
    ("sweet", "S W IY T"),
    ("synth", "S IH N TH"),
    ("tears", "T IH R Z"),
    ("techno", "T EH K N OW"),
    ("tempo", "T EH M P OW"),
    ("the", "DH AH"),
    ("they", "DH EY"),
    ("through", "TH R UW"),
    ("time", "T AY M"),
    ("to", "T UW"),
    ("together", "T AH G EH DH ER"),
    ("tonight", "T AH N AY T"),
    ("touch", "T AH CH"),
    ("track", "T R AE K"),
    ("truth", "T R UW TH"),
    ("turn", "T ER N"),
    ("voice", "V OY S"),
    ("walk", "W AO K"),
    ("want", "W AA N T"),
    ("war", "W AO R"),
    ("wave", "W EY V"),
    ("we", "W IY"),
    ("whisper", "W IH S P ER"),
    ("wild", "W AY L D"),
    ("win", "W IH N"),
    ("wind", "W IH N D"),
    ("with", "W IH DH"),
    ("without", "W IH DH AW T"),
    ("world", "W ER L D"),
    ("you", "Y UW"),
    ("young", "Y AH NG"),
    ("your", "Y AO R"),
];

/// Helper to test if an ASCII byte is an English vowel.
#[inline]
fn is_vowel(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

/// Convert a single English word token into ARPABET phonemes using NRL-style G2P rules.
fn g2p_word(w: &str) -> String {
    let b = w.as_bytes();
    let n = b.len();
    if n == 0 {
        return String::new();
    }

    let mut out: Vec<&'static str> = Vec::new();
    let mut i = 0;

    while i < n {
        let rem = &b[i..];

        // Vowel before -tion / -sion (e.g. nation -> N EY SH AH N, motion -> M OW SH AH N)
        if matches!(rem[0], b'a' | b'o' | b'u') && rem.len() >= 5 && (&rem[1..5] == b"tion" || &rem[1..5] == b"sion") {
            match rem[0] {
                b'a' => out.push("EY"),
                b'o' => out.push("OW"),
                b'u' => out.push("UW"),
                _ => {}
            }
            i += 1;
            continue;
        }

        // --- Common Multi-letter Suffixes & Digraphs ---
        if rem.starts_with(b"tion") {
            out.push("SH");
            out.push("AH");
            out.push("N");
            i += 4;
            continue;
        }
        if rem.starts_with(b"sion") {
            out.push("ZH");
            out.push("AH");
            out.push("N");
            i += 4;
            continue;
        }
        if rem.starts_with(b"ture") {
            out.push("CH");
            out.push("ER");
            i += 4;
            continue;
        }
        if rem.starts_with(b"ing") && i + 3 == n {
            out.push("IH");
            out.push("NG");
            i += 3;
            continue;
        }
        if rem.starts_with(b"ed") && i + 2 == n {
            let prev = out.last().copied().unwrap_or("");
            if prev == "T" || prev == "D" {
                out.push("IH");
                out.push("D");
            } else if matches!(prev, "P" | "K" | "S" | "SH" | "CH" | "F") {
                out.push("T");
            } else {
                out.push("D");
            }
            i += 2;
            continue;
        }
        if rem.starts_with(b"ight") {
            out.push("AY");
            out.push("T");
            i += 4;
            continue;
        }
        if rem.starts_with(b"igh") {
            out.push("AY");
            i += 3;
            continue;
        }
        if rem.starts_with(b"eigh") {
            out.push("EY");
            i += 4;
            continue;
        }
        if rem.starts_with(b"ough") || rem.starts_with(b"augh") {
            out.push("AO");
            out.push("T");
            i += 4;
            continue;
        }
        if rem.starts_with(b"tch") {
            out.push("CH");
            i += 3;
            continue;
        }
        if rem.starts_with(b"dge") {
            out.push("JH");
            i += 3;
            continue;
        }

        // Silent clusters at start of word
        if i == 0 && (rem.starts_with(b"kn") || rem.starts_with(b"gn") || rem.starts_with(b"pn")) {
            out.push("N");
            i += 2;
            continue;
        }
        if i == 0 && rem.starts_with(b"wr") {
            out.push("R");
            i += 2;
            continue;
        }
        if i == 0 && rem.starts_with(b"ps") {
            out.push("S");
            i += 2;
            continue;
        }

        // 2-letter consonant digraphs
        if rem.starts_with(b"ch") {
            out.push("CH");
            i += 2;
            continue;
        }
        if rem.starts_with(b"sh") {
            out.push("SH");
            i += 2;
            continue;
        }
        if rem.starts_with(b"th") {
            out.push("TH");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ph") {
            out.push("F");
            i += 2;
            continue;
        }
        if rem.starts_with(b"wh") {
            out.push("W");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ck") {
            out.push("K");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ng") {
            out.push("NG");
            i += 2;
            continue;
        }
        if rem.starts_with(b"nk") {
            out.push("NG");
            out.push("K");
            i += 2;
            continue;
        }
        if rem.starts_with(b"qu") {
            out.push("K");
            out.push("W");
            i += 2;
            continue;
        }

        // 2-letter vowel digraphs
        if rem.starts_with(b"ee") || rem.starts_with(b"ea") {
            out.push("IY");
            i += 2;
            continue;
        }
        if rem.starts_with(b"oo") {
            if rem.len() > 2 && rem[2] == b'k' {
                out.push("UH");
            } else {
                out.push("UW");
            }
            i += 2;
            continue;
        }
        if rem.starts_with(b"ai") || rem.starts_with(b"ay") || rem.starts_with(b"ey") {
            out.push("EY");
            i += 2;
            continue;
        }
        if rem.starts_with(b"oi") || rem.starts_with(b"oy") {
            out.push("OY");
            i += 2;
            continue;
        }
        if rem.starts_with(b"oa") {
            out.push("OW");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ou") {
            out.push("AW");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ow") {
            out.push("OW");
            i += 2;
            continue;
        }
        if rem.starts_with(b"au") || rem.starts_with(b"aw") {
            out.push("AO");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ew") {
            out.push("UW");
            i += 2;
            continue;
        }
        if rem.starts_with(b"ie") {
            if i + 2 == n {
                out.push("AY");
            } else {
                out.push("IY");
            }
            i += 2;
            continue;
        }

        // Vowel + R combinations
        if rem.starts_with(b"ar") {
            out.push("AA");
            out.push("R");
            i += 2;
            continue;
        }
        if rem.starts_with(b"er") || rem.starts_with(b"ir") || rem.starts_with(b"ur") {
            out.push("ER");
            i += 2;
            continue;
        }
        if rem.starts_with(b"or") {
            out.push("AO");
            out.push("R");
            i += 2;
            continue;
        }

        // Magic "E" (vowel + single consonant + silent final e)
        let ch = b[i];
        if matches!(ch, b'a' | b'e' | b'i' | b'o' | b'u') && i + 2 < n {
            let next_c = b[i + 1];
            let after_c = b[i + 2];
            let is_single_cons = !is_vowel(next_c) && next_c != b'r' && next_c != b'w' && next_c != b'x';
            let is_magic_e = after_c == b'e' && (i + 3 == n || (i + 4 == n && (b[i + 3] == b's' || b[i + 3] == b'd')));
            let is_v_c_ing = i + 5 == n && &b[i + 2..] == b"ing";
            if is_single_cons && (is_magic_e || is_v_c_ing) {
                match ch {
                    b'a' => out.push("EY"),
                    b'e' => out.push("IY"),
                    b'i' => out.push("AY"),
                    b'o' => out.push("OW"),
                    b'u' => {
                        out.push("Y");
                        out.push("UW");
                    }
                    _ => unreachable!(),
                }
                i += 1;
                continue;
            }
        }

        // Single letter rules
        match ch {
            b'a' => out.push("AE"),
            b'b' => out.push("B"),
            b'c' => {
                if i + 1 < n && matches!(b[i + 1], b'e' | b'i' | b'y') {
                    out.push("S");
                } else {
                    out.push("K");
                }
            }
            b'd' => out.push("D"),
            b'e' => {
                // Silent final 'e'
                if i == n - 1 && n > 1 {
                    // Do nothing
                } else {
                    out.push("EH");
                }
            }
            b'f' => out.push("F"),
            b'g' => {
                if i + 1 < n && matches!(b[i + 1], b'e' | b'i' | b'y') {
                    out.push("JH");
                } else {
                    out.push("G");
                }
            }
            b'h' => out.push("HH"),
            b'i' => out.push("IH"),
            b'j' => out.push("JH"),
            b'k' => out.push("K"),
            b'l' => out.push("L"),
            b'm' => out.push("M"),
            b'n' => out.push("N"),
            b'o' => out.push("AA"),
            b'p' => out.push("P"),
            b'q' => out.push("K"),
            b'r' => out.push("R"),
            b's' => {
                if i > 0 && i + 1 < n && is_vowel(b[i - 1]) && is_vowel(b[i + 1]) {
                    out.push("Z");
                } else {
                    out.push("S");
                }
            }
            b't' => out.push("T"),
            b'u' => out.push("AH"),
            b'v' => out.push("V"),
            b'w' => out.push("W"),
            b'x' => {
                out.push("K");
                out.push("S");
            }
            b'y' => {
                if i == 0 && i + 1 < n && is_vowel(b[i + 1]) {
                    out.push("Y");
                } else if i == n - 1 {
                    out.push("IY");
                } else {
                    out.push("IH");
                }
            }
            b'z' => out.push("Z"),
            _ => {}
        }
        i += 1;
    }

    out.join(" ")
}

/// Convert arbitrary English text into space-separated ARPABET phonemes.
/// Checks the compact 150+ music/lyric exception table first, falling back to
/// Naval Research Lab-style grapheme-to-phoneme rules for any unknown words.
pub fn english_to_arpabet(text: &str) -> String {
    let mut result = String::new();

    for token in text.split(|c: char| !c.is_alphabetic()) {
        let clean = token.trim();
        if clean.is_empty() {
            continue;
        }

        // Lowercase for dictionary lookup and rule evaluation
        let mut lower = String::new();
        for ch in clean.chars() {
            lower.push(ch.to_ascii_lowercase());
        }

        let phone_str: String = match COMMON_MUSIC_EXCEPTIONS
            .binary_search_by_key(&lower.as_str(), |&(k, _)| k)
        {
            Ok(idx) => String::from(COMMON_MUSIC_EXCEPTIONS[idx].1),
            Err(_) => g2p_word(&lower),
        };

        if !phone_str.is_empty() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&phone_str);
        }
    }

    result
}

/// Convert a single Japanese Hiragana kana (or digraph) into phonemes.
fn hiragana_char_to_phonemes(c: char, next_c: Option<char>) -> (Option<&'static str>, usize) {
    // Check for yōon (digraphs with small ya, yu, yo)
    if let Some(next) = next_c {
        match (c, next) {
            ('き', 'ゃ') => return (Some("K Y A"), 2),
            ('き', 'ゅ') => return (Some("K Y U"), 2),
            ('き', 'ょ') => return (Some("K Y O"), 2),
            ('し', 'ゃ') => return (Some("SH A"), 2),
            ('し', 'ゅ') => return (Some("SH U"), 2),
            ('し', 'ょ') => return (Some("SH O"), 2),
            ('ち', 'ゃ') => return (Some("CH A"), 2),
            ('ち', 'ゅ') => return (Some("CH U"), 2),
            ('ち', 'ょ') => return (Some("CH O"), 2),
            ('に', 'ゃ') => return (Some("N Y A"), 2),
            ('に', 'ゅ') => return (Some("N Y U"), 2),
            ('に', 'ょ') => return (Some("N Y O"), 2),
            ('ひ', 'ゃ') => return (Some("H Y A"), 2),
            ('ひ', 'ゅ') => return (Some("H Y U"), 2),
            ('ひ', 'ょ') => return (Some("H Y O"), 2),
            ('み', 'ゃ') => return (Some("M Y A"), 2),
            ('み', 'ゅ') => return (Some("M Y U"), 2),
            ('み', 'ょ') => return (Some("M Y O"), 2),
            ('り', 'ゃ') => return (Some("R Y A"), 2),
            ('り', 'ゅ') => return (Some("R Y U"), 2),
            ('り', 'ょ') => return (Some("R Y O"), 2),
            ('ぎ', 'ゃ') => return (Some("G Y A"), 2),
            ('ぎ', 'ゅ') => return (Some("G Y U"), 2),
            ('ぎ', 'ょ') => return (Some("G Y O"), 2),
            ('じ', 'ゃ') => return (Some("J A"), 2),
            ('じ', 'ゅ') => return (Some("J U"), 2),
            ('じ', 'ょ') => return (Some("J O"), 2),
            ('び', 'ゃ') => return (Some("B Y A"), 2),
            ('び', 'ゅ') => return (Some("B Y U"), 2),
            ('び', 'ょ') => return (Some("B Y O"), 2),
            ('ぴ', 'ゃ') => return (Some("P Y A"), 2),
            ('ぴ', 'ゅ') => return (Some("P Y U"), 2),
            ('ぴ', 'ょ') => return (Some("P Y O"), 2),
            _ => {}
        }
    }

    let code = match c {
        'あ' | 'ぁ' => "A",
        'い' | 'ぃ' => "I",
        'う' | 'ぅ' => "U",
        'え' | 'ぇ' => "E",
        'お' | 'ぉ' => "O",
        'か' => "K A",
        'き' => "K I",
        'く' => "K U",
        'け' => "K E",
        'こ' => "K O",
        'さ' => "S A",
        'し' => "SH I",
        'す' => "S U",
        'せ' => "S E",
        'そ' => "S O",
        'た' => "T A",
        'ち' => "CH I",
        'つ' => "TS U",
        'て' => "T E",
        'と' => "T O",
        'な' => "N A",
        'に' => "N I",
        'ぬ' => "N U",
        'ね' => "N E",
        'の' => "N O",
        'は' => "H A",
        'ひ' => "H I",
        'ふ' => "F U",
        'へ' => "H E",
        'ほ' => "H O",
        'ま' => "M A",
        'み' => "M I",
        'む' => "M U",
        'め' => "M E",
        'も' => "M O",
        'や' | 'ゃ' => "Y A",
        'ゆ' | 'ゅ' => "Y U",
        'よ' | 'ょ' => "Y O",
        'ら' => "R A",
        'り' => "R I",
        'る' => "R U",
        'れ' => "R E",
        'ろ' => "R O",
        'わ' => "W A",
        'を' => "O",
        'ん' => "N",
        'が' => "G A",
        'ぎ' => "G I",
        'ぐ' => "G U",
        'げ' => "G E",
        'ご' => "G O",
        'ざ' => "Z A",
        'じ' => "J I",
        'ず' => "Z U",
        'ぜ' => "Z E",
        'ぞ' => "Z O",
        'だ' => "D A",
        'ぢ' => "J I",
        'づ' => "Z U",
        'で' => "D E",
        'ど' => "D O",
        'ば' => "B A",
        'び' => "B I",
        'ぶ' => "B U",
        'べ' => "B E",
        'ぼ' => "B O",
        'ぱ' => "P A",
        'ぴ' => "P I",
        'ぷ' => "P U",
        'ぺ' => "P E",
        'ぽ' => "P O",
        'っ' => "",
        _ => return (None, 1),
    };

    (Some(code), 1)
}

/// Convert an ASCII Romaji syllable slice to Japanese phonemes.
fn romaji_syllable(b: &[u8]) -> Option<(&'static str, usize)> {
    if b.is_empty() {
        return None;
    }

    // 3-letter combos
    if b.starts_with(b"kya") {
        return Some(("K Y A", 3));
    }
    if b.starts_with(b"kyu") {
        return Some(("K Y U", 3));
    }
    if b.starts_with(b"kyo") {
        return Some(("K Y O", 3));
    }
    if b.starts_with(b"sha") {
        return Some(("SH A", 3));
    }
    if b.starts_with(b"shu") {
        return Some(("SH U", 3));
    }
    if b.starts_with(b"sho") {
        return Some(("SH O", 3));
    }
    if b.starts_with(b"cha") {
        return Some(("CH A", 3));
    }
    if b.starts_with(b"chu") {
        return Some(("CH U", 3));
    }
    if b.starts_with(b"cho") {
        return Some(("CH O", 3));
    }
    if b.starts_with(b"tsu") {
        return Some(("TS U", 3));
    }
    if b.starts_with(b"shi") {
        return Some(("SH I", 3));
    }
    if b.starts_with(b"chi") {
        return Some(("CH I", 3));
    }
    if b.starts_with(b"nya") {
        return Some(("N Y A", 3));
    }
    if b.starts_with(b"nyu") {
        return Some(("N Y U", 3));
    }
    if b.starts_with(b"nyo") {
        return Some(("N Y O", 3));
    }
    if b.starts_with(b"hya") {
        return Some(("H Y A", 3));
    }
    if b.starts_with(b"hyu") {
        return Some(("H Y U", 3));
    }
    if b.starts_with(b"hyo") {
        return Some(("H Y O", 3));
    }
    if b.starts_with(b"mya") {
        return Some(("M Y A", 3));
    }
    if b.starts_with(b"myu") {
        return Some(("M Y U", 3));
    }
    if b.starts_with(b"myo") {
        return Some(("M Y O", 3));
    }
    if b.starts_with(b"rya") {
        return Some(("R Y A", 3));
    }
    if b.starts_with(b"ryu") {
        return Some(("R Y U", 3));
    }
    if b.starts_with(b"ryo") {
        return Some(("R Y O", 3));
    }
    if b.starts_with(b"gya") {
        return Some(("G Y A", 3));
    }
    if b.starts_with(b"gyu") {
        return Some(("G Y U", 3));
    }
    if b.starts_with(b"gyo") {
        return Some(("G Y O", 3));
    }
    if b.starts_with(b"bya") {
        return Some(("B Y A", 3));
    }
    if b.starts_with(b"byu") {
        return Some(("B Y U", 3));
    }
    if b.starts_with(b"byo") {
        return Some(("B Y O", 3));
    }
    if b.starts_with(b"pya") {
        return Some(("P Y A", 3));
    }
    if b.starts_with(b"pyu") {
        return Some(("P Y U", 3));
    }
    if b.starts_with(b"pyo") {
        return Some(("P Y O", 3));
    }

    // 2-letter combos
    if b.starts_with(b"ka") {
        return Some(("K A", 2));
    }
    if b.starts_with(b"ki") {
        return Some(("K I", 2));
    }
    if b.starts_with(b"ku") {
        return Some(("K U", 2));
    }
    if b.starts_with(b"ke") {
        return Some(("K E", 2));
    }
    if b.starts_with(b"ko") {
        return Some(("K O", 2));
    }

    if b.starts_with(b"sa") {
        return Some(("S A", 2));
    }
    if b.starts_with(b"su") {
        return Some(("S U", 2));
    }
    if b.starts_with(b"se") {
        return Some(("S E", 2));
    }
    if b.starts_with(b"so") {
        return Some(("S O", 2));
    }

    if b.starts_with(b"ta") {
        return Some(("T A", 2));
    }
    if b.starts_with(b"te") {
        return Some(("T E", 2));
    }
    if b.starts_with(b"to") {
        return Some(("T O", 2));
    }

    if b.starts_with(b"na") {
        return Some(("N A", 2));
    }
    if b.starts_with(b"ni") {
        return Some(("N I", 2));
    }
    if b.starts_with(b"nu") {
        return Some(("N U", 2));
    }
    if b.starts_with(b"ne") {
        return Some(("N E", 2));
    }
    if b.starts_with(b"no") {
        return Some(("N O", 2));
    }

    if b.starts_with(b"ha") {
        return Some(("H A", 2));
    }
    if b.starts_with(b"hi") {
        return Some(("H I", 2));
    }
    if b.starts_with(b"fu") {
        return Some(("F U", 2));
    }
    if b.starts_with(b"he") {
        return Some(("H E", 2));
    }
    if b.starts_with(b"ho") {
        return Some(("H O", 2));
    }

    if b.starts_with(b"ma") {
        return Some(("M A", 2));
    }
    if b.starts_with(b"mi") {
        return Some(("M I", 2));
    }
    if b.starts_with(b"mu") {
        return Some(("M U", 2));
    }
    if b.starts_with(b"me") {
        return Some(("M E", 2));
    }
    if b.starts_with(b"mo") {
        return Some(("M O", 2));
    }

    if b.starts_with(b"ya") {
        return Some(("Y A", 2));
    }
    if b.starts_with(b"yu") {
        return Some(("Y U", 2));
    }
    if b.starts_with(b"yo") {
        return Some(("Y O", 2));
    }

    if b.starts_with(b"ra") {
        return Some(("R A", 2));
    }
    if b.starts_with(b"ri") {
        return Some(("R I", 2));
    }
    if b.starts_with(b"ru") {
        return Some(("R U", 2));
    }
    if b.starts_with(b"re") {
        return Some(("R E", 2));
    }
    if b.starts_with(b"ro") {
        return Some(("R O", 2));
    }

    if b.starts_with(b"wa") {
        return Some(("W A", 2));
    }
    if b.starts_with(b"wo") {
        return Some(("O", 2));
    }

    if b.starts_with(b"ga") {
        return Some(("G A", 2));
    }
    if b.starts_with(b"gi") {
        return Some(("G I", 2));
    }
    if b.starts_with(b"gu") {
        return Some(("G U", 2));
    }
    if b.starts_with(b"ge") {
        return Some(("G E", 2));
    }
    if b.starts_with(b"go") {
        return Some(("G O", 2));
    }

    if b.starts_with(b"za") {
        return Some(("Z A", 2));
    }
    if b.starts_with(b"ji") {
        return Some(("J I", 2));
    }
    if b.starts_with(b"zu") {
        return Some(("Z U", 2));
    }
    if b.starts_with(b"ze") {
        return Some(("Z E", 2));
    }
    if b.starts_with(b"zo") {
        return Some(("Z O", 2));
    }

    if b.starts_with(b"da") {
        return Some(("D A", 2));
    }
    if b.starts_with(b"de") {
        return Some(("D E", 2));
    }
    if b.starts_with(b"do") {
        return Some(("D O", 2));
    }

    if b.starts_with(b"ba") {
        return Some(("B A", 2));
    }
    if b.starts_with(b"bi") {
        return Some(("B I", 2));
    }
    if b.starts_with(b"bu") {
        return Some(("B U", 2));
    }
    if b.starts_with(b"be") {
        return Some(("B E", 2));
    }
    if b.starts_with(b"bo") {
        return Some(("B O", 2));
    }

    if b.starts_with(b"pa") {
        return Some(("P A", 2));
    }
    if b.starts_with(b"pi") {
        return Some(("P I", 2));
    }
    if b.starts_with(b"pu") {
        return Some(("P U", 2));
    }
    if b.starts_with(b"pe") {
        return Some(("P E", 2));
    }
    if b.starts_with(b"po") {
        return Some(("P O", 2));
    }

    // 1-letter vowels & nasal 'n'
    match b[0] {
        b'a' => Some(("A", 1)),
        b'i' => Some(("I", 1)),
        b'u' => Some(("U", 1)),
        b'e' => Some(("E", 1)),
        b'o' => Some(("O", 1)),
        b'n' => Some(("N", 1)),
        _ => None,
    }
}

/// Convert Japanese Romaji, Hiragana, and Katakana text into a sequence of phonemes,
/// prefixed with the `[bank=ja]` selector token.
pub fn japanese_to_phonemes(text: &str) -> String {
    let mut out: Vec<&'static str> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let num_chars = chars.len();
    let mut i = 0;

    let mut ascii_buf: Vec<u8> = Vec::new();

    let flush_ascii = |buf: &mut Vec<u8>, out: &mut Vec<&'static str>| {
        let mut idx = 0;
        let len = buf.len();
        while idx < len {
            // Check double consonants (sokuon) e.g. "kk", "tt", "ss", "pp"
            if idx + 1 < len
                && buf[idx] == buf[idx + 1]
                && !matches!(buf[idx], b'a' | b'i' | b'u' | b'e' | b'o' | b'n')
            {
                // Skip the first consonant (mimicking sokuon gemination)
                idx += 1;
                continue;
            }

            if let Some((syl, adv)) = romaji_syllable(&buf[idx..]) {
                out.push(syl);
                idx += adv;
            } else {
                idx += 1;
            }
        }
        buf.clear();
    };

    while i < num_chars {
        let c = chars[i];

        // Prolonged sound mark (ー) extends previous vowel if available
        if c == 'ー' || c == '-' {
            if let Some(&last) = out.last() {
                // Find trailing vowel in last token
                if let Some(vowel) = last.split_whitespace().last() {
                    match vowel {
                        "A" | "I" | "U" | "E" | "O" => out.push(vowel),
                        _ => {}
                    }
                }
            }
            i += 1;
            continue;
        }

        // Check if character is Katakana (U+30A1..=U+30F6), convert to Hiragana offset (-0x60)
        let norm_c = match c {
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        };

        if ('\u{3041}'..='\u{3096}').contains(&norm_c) {
            // Flush any pending ASCII Romaji buffer
            if !ascii_buf.is_empty() {
                flush_ascii(&mut ascii_buf, &mut out);
            }

            let next_c = if i + 1 < num_chars {
                let n_c = chars[i + 1];
                match n_c {
                    '\u{30A1}'..='\u{30F6}' => char::from_u32(n_c as u32 - 0x60),
                    _ => Some(n_c),
                }
            } else {
                None
            };

            let (phones, consumed) = hiragana_char_to_phonemes(norm_c, next_c);
            if let Some(p) = phones {
                if !p.is_empty() {
                    out.push(p);
                }
            }
            i += consumed;
        } else if c.is_ascii_alphabetic() {
            ascii_buf.push(c.to_ascii_lowercase() as u8);
            i += 1;
        } else {
            // Non-alphabetic delimiter (space, punctuation)
            if !ascii_buf.is_empty() {
                flush_ascii(&mut ascii_buf, &mut out);
            }
            i += 1;
        }
    }

    if !ascii_buf.is_empty() {
        flush_ascii(&mut ascii_buf, &mut out);
    }

    let mut result = String::from("[bank=ja]");
    for item in out {
        for phone in item.split_whitespace() {
            result.push(' ');
            result.push_str(phone);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_music_exceptions_sorted() {
        for i in 1..COMMON_MUSIC_EXCEPTIONS.len() {
            assert!(
                COMMON_MUSIC_EXCEPTIONS[i - 1].0 < COMMON_MUSIC_EXCEPTIONS[i].0,
                "Dictionary not strictly sorted: {} vs {}",
                COMMON_MUSIC_EXCEPTIONS[i - 1].0,
                COMMON_MUSIC_EXCEPTIONS[i].0
            );
        }
        assert!(COMMON_MUSIC_EXCEPTIONS.len() >= 150);
    }

    #[test]
    fn test_english_dictionary_words() {
        assert_eq!(english_to_arpabet("love"), "L AH V");
        assert_eq!(english_to_arpabet("heart"), "HH AA R T");
        assert_eq!(english_to_arpabet("night"), "N AY T");
        assert_eq!(english_to_arpabet("fire"), "F AY ER");
        assert_eq!(english_to_arpabet("dream"), "D R IY M");
        assert_eq!(english_to_arpabet("music"), "M Y UW Z IH K");
        assert_eq!(english_to_arpabet("machine"), "M AH SH IY N");
        assert_eq!(english_to_arpabet("future"), "F Y UW CH ER");
        assert_eq!(english_to_arpabet("dance"), "D AE N S");
        assert_eq!(english_to_arpabet("voice"), "V OY S");
        assert_eq!(english_to_arpabet("better"), "B EH T ER");
        assert_eq!(english_to_arpabet("faster"), "F AE S T ER");
        assert_eq!(english_to_arpabet("stronger"), "S T R AO NG ER");
        assert_eq!(english_to_arpabet("harder"), "HH AA R D ER");
        assert_eq!(english_to_arpabet("alive"), "AH L AY V");
    }

    #[test]
    fn test_english_rule_engine_arbitrary_words() {
        // Test NRL rule engine with arbitrary words not in the exception table
        let res = english_to_arpabet("robot nation shining");
        assert!(res.contains("SH AH N")); // -tion
        assert!(res.contains("SH AY N")); // shine/shining
    }

    #[test]
    fn test_english_punctuation_and_casing() {
        let input = "I love music! Faster, Harder, Stronger.";
        let res = english_to_arpabet(input);
        assert_eq!(
            res,
            "AY L AH V M Y UW Z IH K F AE S T ER HH AA R D ER S T R AO NG ER"
        );
    }

    #[test]
    fn test_japanese_romaji() {
        let res = japanese_to_phonemes("karaoke");
        assert_eq!(res, "[bank=ja] K A R A O K E");

        let res2 = japanese_to_phonemes("tokyo");
        assert_eq!(res2, "[bank=ja] T O K Y O");

        let res3 = japanese_to_phonemes("sushi");
        assert_eq!(res3, "[bank=ja] S U SH I");

        let res4 = japanese_to_phonemes("tsunami");
        assert_eq!(res4, "[bank=ja] TS U N A M I");

        let res5 = japanese_to_phonemes("fuji");
        assert_eq!(res5, "[bank=ja] F U J I");
    }

    #[test]
    fn test_japanese_kana() {
        // Hiragana: こんにちは
        let res = japanese_to_phonemes("こんにちは");
        assert_eq!(res, "[bank=ja] K O N N I CH I H A");

        // Katakana: カラオケ
        let res2 = japanese_to_phonemes("カラオケ");
        assert_eq!(res2, "[bank=ja] K A R A O K E");

        // Yōon: とうきょう (Toukyou)
        let res3 = japanese_to_phonemes("とうきょう");
        assert_eq!(res3, "[bank=ja] T O U K Y O U");
    }

    #[test]
    fn test_english_edge_cases() {
        assert_eq!(english_to_arpabet(""), "");
        assert_eq!(english_to_arpabet("   ... ??? !!! "), "");
        // Magic E words
        assert_eq!(english_to_arpabet("gate"), "G EY T");
        assert_eq!(english_to_arpabet("hope"), "HH OW P");
        assert_eq!(english_to_arpabet("fine"), "F AY N");
        assert_eq!(english_to_arpabet("cube"), "K Y UW B");
    }

    #[test]
    fn test_japanese_geminates_and_prolonged() {
        // Geminate consonant "chotto" -> skips double 't'
        let res1 = japanese_to_phonemes("chotto");
        assert_eq!(res1, "[bank=ja] CH O T O");

        // Katakana with prolonged mark: ラーメン -> R A A M E N
        let res2 = japanese_to_phonemes("ラーメン");
        assert_eq!(res2, "[bank=ja] R A A M E N");
    }
}
