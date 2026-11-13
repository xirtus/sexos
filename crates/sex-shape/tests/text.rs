//! Bidirectional order and Arabic joining, as Display uses them for text cells.
use sex_shape::{arabic, bidi};

fn order(line: &str) -> Option<Vec<usize>> {
    let mut o = [0u16; bidi::MAX];
    bidi::visual_order_into(line, &mut o).map(|n| o[..n].iter().map(|&i| i as usize).collect())
}
fn visual(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    match order(line) { Some(order) => order.iter().map(|&i| chars[i]).collect(), None => line.into() }
}
/// The order the `unicode-bidi` crate gives (full UAX #9).
fn reference(line: &str) -> Vec<usize> {
    let info = unicode_bidi::BidiInfo::new(line, None);
    let para = &info.paragraphs[0];
    let starts: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
    let (levels, runs) = info.visual_runs(para, para.range.clone());
    let mut out = vec![];
    for run in runs {
        let (a, b) = (starts.partition_point(|&s| s < run.start), starts.partition_point(|&s| s < run.end));
        if levels[run.start].is_rtl() { out.extend((a..b).rev()) } else { out.extend(a..b) }
    }
    out
}

#[test]
fn mixed_hebrew_and_english_lines() {
    assert_eq!(order("plain English (1)"), None);
    assert_eq!(visual("abc שלום def"), "abc םולש def");
    assert_eq!(visual("שלום world"), "world םולש");
    assert_eq!(visual("שלום 123 עולם"), "םלוע 123 םולש");
    assert_eq!(visual("ספר SexOS חדש"), "שדח SexOS רפס");
    assert!(bidi::is_rtl("שלום world"));
    assert!(!bidi::is_rtl("hello שלום"));
    let mut l = [0u8; 8];
    assert_eq!(bidi::levels_into("a ש", &mut l), Some(3));
    assert_eq!(&l[..3], &[0, 0, 1]);
    assert_eq!(bidi::mirror('('), ')');
}

#[test]
fn arabic_letters_take_their_joining_forms() {
    // سلام: seen initial, lam medial, alef final (alef joins only back), meem isolated.
    let word = "سلام";
    let forms: Vec<u32> = word.char_indices().map(|(i, _)| arabic::form_at(word.as_bytes(), i).unwrap()).collect();
    assert_eq!(forms, vec![0xFEB3, 0xFEE0, 0xFE8E, 0xFEE1]);
    // Alone, a letter is isolated; harakat do not break joining.
    assert_eq!(arabic::form(0, 0x0628, 0), 0xFE8F);
    let marked = "بِب";
    assert_eq!(arabic::form_at(marked.as_bytes(), 0), Some(0xFE91));
    assert_eq!(arabic::form_at(marked.as_bytes(), 4), Some(0xFE90));
    // Latin is untouched; a newline ends the word.
    assert_eq!(arabic::form_at(b"a", 0), Some('a' as u32));
    let two = "ب\nب";
    assert_eq!(arabic::form_at(two.as_bytes(), 0), Some(0xFE8F));
}

#[test]
fn display_cells_reorder_per_row_and_keep_newlines() {
    // Two rows: "ab שלום\n" and "עולם x"; cells are byte offsets of each character.
    let text = "ab שלום\nעולם x";
    let mut cells: Vec<u16> = text.char_indices().map(|(i, _)| i as u16).collect();
    let lines = [0u16, 8];
    let n = cells.len();
    sex_shape::cells::reorder_lines(text.as_bytes(), &mut cells, &lines, n);
    let shown: String = cells.iter().map(|&b| text[b as usize..].chars().next().unwrap()).collect();
    assert_eq!(shown, "ab םולש\nx םלוע");
    // Plain rows are untouched.
    let plain = "hello\nworld";
    let mut cells: Vec<u16> = plain.char_indices().map(|(i, _)| i as u16).collect();
    let before = cells.clone();
    sex_shape::cells::reorder_lines(plain.as_bytes(), &mut cells, &[0, 6], 11);
    assert_eq!(cells, before);
    assert_eq!(sex_shape::cells::shown("سلام".as_bytes(), 0, 0x0633), 0xFEB3);
    assert_eq!(sex_shape::cells::shown(b"a", 0, 'a' as u32), 'a' as u32);
}

#[test]
fn line_algorithm_matches_unicode_bidi() {
    let lines = [
        "abc שלום def", "שלום world", "שלום 123 עולם", "ספר SexOS חדש", "hello, שלום!", "שלום, hello!",
        "مرحبا بالعالم", "العربية 2024 نص", "قیمت ۱۲۳ تومان", "price: 12.50 שקל", "שקל 12.50", "a (b) ג (ד) e",
        "גרסה 1.2.3 של SexOS", "תאריך 13/11/2026", "email: a@b.c בדיקה", "  שלום  ", "x - y = 5% מ", "ש 5% ב",
        "ملف test.txt جديد", "١٢٣ عربي", "Hebrew: עברית, Arabic: عربي.", "עברית\tטאב", "a\u{200F}b", "1 + 2 = ש",
    ];
    for line in lines {
        let line = line.replace("\\t", "\t").replace("\\u{200F}", "\u{200F}");
        let mine = order(&line).unwrap_or_else(|| (0..line.chars().count()).collect());
        assert_eq!(mine, reference(&line), "{line}");
    }
}
