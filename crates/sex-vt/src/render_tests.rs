// SPDX-License-Identifier: GPL-3.0-only
//! The renderer on the host: wide characters, bold and italic cuts, fallback fonts, the
//! glyph cache, and how long a full frame takes.
extern crate std;
use super::render::{cell_for_height, Cache, Faces, Painter};
use super::*;
use alloc::boxed::Box;

fn faces() -> &'static Faces { Box::leak(Faces::embedded().expect("embedded fonts parse")) }
fn cache() -> Box<Cache> { unsafe { Box::<Cache>::new_zeroed().assume_init() } }
fn screen(cols: usize, rows: usize, bytes: &[u8]) -> Screen {
    let mut s = Screen::new(cols, rows);
    let mut p = Parser::new();
    s.feed(&mut p, bytes);
    s.cursor_visible = false;
    s
}
/// How many pixels of the cell at `col, row` differ from the background.
fn ink(pixels: &[u32], stride: usize, cw: usize, ch: usize, col: usize, row: usize, bg: u32) -> usize {
    (0..ch).map(|y| pixels[(row * ch + y) * stride + col * cw..][..cw].iter().filter(|&&p| p != bg).count()).sum()
}

#[test]
fn wide_characters_and_emoji_span_two_cells_with_faces() {
    let (cw, ch) = cell_for_height(18);
    let s = screen(8, 1, "日😀ab".as_bytes());
    let bg = s.theme().bg;
    let (stride, mut px) = (8 * cw, alloc::vec![0u32; 8 * cw * ch]);
    let mut p = Painter::new(cw, ch).with_faces(faces());
    p.draw(&s, &[], &mut cache(), &mut px, stride, false);
    for col in 0..4 { assert!(ink(&px, stride, cw, ch, col, 0, bg) > 4, "cell {col} has no ink"); }
    // Ink crosses the boundary between a wide character's two cells.
    let edge = (0..ch).filter(|y| px[y * stride + cw - 1] != bg || px[y * stride + cw] != bg).count();
    assert!(edge > 0, "日 is drawn in one cell");
}

#[test]
fn without_faces_wide_characters_stay_blank() {
    // A program's own console keeps only Geist Mono, so it does not carry the Noto fonts.
    let (cw, ch) = cell_for_height(18);
    let s = screen(4, 1, "日".as_bytes());
    let bg = s.theme().bg;
    let (stride, mut px) = (4 * cw, alloc::vec![0u32; 4 * cw * ch]);
    Painter::new(cw, ch).draw(&s, &[], &mut cache(), &mut px, stride, false);
    assert_eq!(ink(&px, stride, cw, ch, 0, 0, bg) + ink(&px, stride, cw, ch, 1, 0, bg), 0);
}

#[test]
fn bold_and_italic_use_their_own_cuts() {
    let (cw, ch) = cell_for_height(18);
    let s = screen(3, 1, b"M\x1b[1mM\x1b[0;3mM");
    let (stride, mut px) = (3 * cw, alloc::vec![0u32; 3 * cw * ch]);
    Painter::new(cw, ch).with_faces(faces()).draw(&s, &[], &mut cache(), &mut px, stride, false);
    let cell = |col: usize| -> alloc::vec::Vec<u32> { (0..ch).flat_map(|y| px[y * stride + col * cw..][..cw].iter().copied()).collect() };
    let (regular, bold, italic) = (cell(0), cell(1), cell(2));
    assert_ne!(regular, italic);
    // Bold is brighter too (colour 7 becomes 15): compare where the ink is, not its colour.
    let bg = s.theme().bg;
    let shape = |c: &[u32]| c.iter().map(|&p| p != bg).collect::<alloc::vec::Vec<_>>();
    assert_ne!(shape(&regular), shape(&bold));
    assert!(shape(&bold).iter().filter(|&&b| b).count() > shape(&regular).iter().filter(|&&b| b).count());
}

#[test]
fn a_second_frame_comes_from_the_cache_unchanged() {
    let (cw, ch) = cell_for_height(18);
    let s = screen(20, 2, "Spindle 日本 😀 ok\r\n\x1b[1mbold\x1b[0m \x1b[3mitalic".as_bytes());
    let (stride, mut a) = (20 * cw, alloc::vec![0u32; 20 * cw * 2 * ch]);
    let mut b = a.clone();
    let mut c = cache();
    let mut p = Painter::new(cw, ch).with_faces(faces());
    p.draw(&s, &[], &mut c, &mut a, stride, false);
    p.draw(&s, &[], &mut c, &mut b, stride, false);
    assert_eq!(a, b);
    assert_eq!(c.resets, 0);
}

#[test]
fn a_full_frame_draws_in_under_eight_milliseconds() {
    // 160 x 48 cells of mixed text, every row dirty, glyphs already cached: the case
    // a scrolling `cat` hits on every frame.
    let (cw, ch) = cell_for_height(18);
    let (cols, rows) = (160, 48);
    let mut bytes = alloc::vec::Vec::new();
    for r in 0..rows { bytes.extend_from_slice(std::format!("\x1b[3{}m{:03} fn main() {{ println!(\"日本 {{}}\", x); }} // ok {}\r\n", r % 8, r, "=".repeat(80)).as_bytes()); }
    let s = screen(cols, rows, &bytes);
    let (stride, mut px) = (cols * cw, alloc::vec![0u32; cols * cw * rows * ch]);
    let mut c = cache();
    let mut p = Painter::new(cw, ch).with_faces(faces());
    p.draw(&s, &[], &mut c, &mut px, stride, false);
    let frames = 20;
    let start = std::time::Instant::now();
    for _ in 0..frames { p.draw(&s, &[], &mut c, &mut px, stride, false); }
    let per = start.elapsed() / frames;
    std::println!("full frame {}x{} cells: {:?}", cols, rows, per);
    // Unoptimised test builds are several times slower; the budget holds for release.
    if !cfg!(debug_assertions) { assert!(per.as_micros() < 8_000, "{per:?} a frame"); }
}

#[test]
fn a_cursor_on_the_second_half_of_a_wide_character_is_drawn() {
    // "ab日" on four columns leaves the cursor on 日's second cell (pending wrap).
    let (cw, ch) = cell_for_height(18);
    let mut s = screen(4, 1, "ab日".as_bytes());
    assert_eq!(s.cursor(), (3, 0));
    let (stride, mut plain) = (4 * cw, alloc::vec![0u32; 4 * cw * ch]);
    let mut with_cursor = plain.clone();
    let mut p = Painter::new(cw, ch).with_faces(faces());
    p.draw(&s, &[], &mut cache(), &mut plain, stride, true);
    s.cursor_visible = true;
    p.draw(&s, &[], &mut cache(), &mut with_cursor, stride, true);
    let pair = |px: &[u32]| -> alloc::vec::Vec<u32> { (0..ch).flat_map(|y| px[y * stride + 2 * cw..][..2 * cw].iter().copied()).collect() };
    assert_ne!(pair(&plain), pair(&with_cursor));
}

#[test]
fn a_glide_starts_on_the_old_frame_and_ends_on_the_new_one() {
    use super::render::Glide;
    // One pixel per row: old rows 10..15; scrolling back two rows brings in 8 and 9.
    let (w, h) = (1, 5);
    let old: alloc::vec::Vec<u32> = (10..15).collect();
    let new: alloc::vec::Vec<u32> = (8..13).collect();
    for (span, from, now) in [(2, &old, &new), (-2, &new, &old)] {
        let mut g = Glide { left: span, span, from: from.clone(), shown: alloc::vec::Vec::new() };
        g.compose(now, w, h);
        assert_eq!(&g.shown, from, "starts where the screen was");
        // Halfway, the rows are still one continuous run of text.
        g.left = span / 2;
        g.compose(now, w, h);
        assert!(g.shown.windows(2).all(|p| p[1] == p[0] + 1), "{:?}", g.shown);
        let mut frames = 0;
        while g.left != 0 { g.step(); frames += 1; }
        assert!(frames <= 8, "{frames} frames");
        g.compose(now, w, h);
        assert_eq!(&g.shown, now, "ends on the new frame");
    }
}

#[test]
fn a_long_glide_eases_out_within_a_few_frames() {
    use super::render::Glide;
    let mut g = Glide { left: 900, span: 900, ..Glide::default() };
    let mut last = g.left;
    let mut frames = 0;
    while g.left != 0 { g.step(); assert!(g.left < last); last = g.left; frames += 1; }
    // About 150 ms at 60 frames a second.
    assert!(frames <= 12, "{frames} frames");
}
