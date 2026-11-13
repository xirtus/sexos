//! Every script the fallback chain promises has a font that draws ink for it.
use sex_font::fallback::{Fallback, Source};
use sex_font::{draw_cell, raster::scratch_len, Font, GEIST_MONO};

fn ink(f: &Fallback, scalar: u32, w: usize, h: usize) -> usize {
    let mut acc = vec![0; scratch_len(w, h)];
    let mut out = vec![0u8; w * h];
    assert!(f.draw_cell(scalar, w, h, &mut acc, &mut out), "{scalar:#x} drawn");
    out.iter().filter(|&&c| c > 64).count()
}

#[test]
fn every_script_reaches_a_font() {
    let f = Fallback::embedded().expect("embedded fonts parse");
    let geist = Font::new(GEIST_MONO).unwrap();
    let cases: &[(char, Source)] = &[
        ('Ж', Source::Mono), ('ж', Source::Mono), ('Ω', Source::Mono), ('ő', Source::Mono), ('ŋ', Source::Mono),
        ('日', Source::Cjk), ('本', Source::Cjk), ('语', Source::Cjk), ('你', Source::Cjk), ('好', Source::Cjk),
        ('あ', Source::Cjk), ('カ', Source::Cjk), ('한', Source::Cjk), ('國', Source::Cjk), ('。', Source::Cjk),
        ('😀', Source::Emoji), ('👍', Source::Emoji), ('❤', Source::Emoji), ('🚀', Source::Emoji),
        ('♞', Source::Symbols), ('⣿', Source::Symbols),
        ('ب', Source::Script(0)), ('\u{FEB3}', Source::Script(0)), ('ש', Source::Script(1)), ('न', Source::Script(2)), ('ก', Source::Script(3)),
    ];
    for &(c, want) in cases {
        let scalar = c as u32;
        assert!(geist.glyph(scalar).is_none() || want != Source::Cjk, "{c} is Geist's");
        assert_eq!(f.source(scalar), Some(want), "{c}");
        for (w, h) in [(12, 18), (24, 36), (48, 72)] {
            let n = ink(&f, scalar, w, h);
            assert!(n * 50 >= w * h, "{c} at {w}x{h}: {n} inked pixels");
        }
    }
    assert_eq!(f.source(0x10FFFD), None);
}

#[test]
fn geist_still_draws_ascii_first() {
    let geist = Font::new(GEIST_MONO).unwrap();
    let mut acc = vec![0; scratch_len(12, 18)];
    let mut out = vec![0u8; 12 * 18];
    for c in 0x21..0x7F { assert!(draw_cell(&geist, c, 12, 18, &mut acc, &mut out)); }
}
