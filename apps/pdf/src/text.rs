// SPDX-License-Identifier: GPL-3.0-only
//! A page's text and where each character sits, read by running hayro's interpreter
//! into a device that keeps glyphs and ignores everything drawn.
//!
//! hayro has no text extraction of its own, but its interpreter hands every device the
//! glyphs it shows, each with its Unicode value (from ToUnicode, the glyph's name or the
//! standard encodings) and its transform. That is the same font machinery that draws
//! the page, so what search finds is what the eye sees, including text in forms.
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Point, Rect};
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, TransformExt,
};
use hayro::hayro_syntax::page::Page;

/// A page's characters in content order, with one box per character in points from the
/// page's top left (as it is shown, rotation applied). Spaces and line breaks the
/// extractor infers from gaps carry the box of the character before them.
#[derive(Clone, Debug, Default)]
pub struct PageText {
    pub chars: Vec<char>,
    pub boxes: Vec<Rect>,
}

/// Glyph space is 1000 units per em; a box from the descender to the ascender and
/// about half an em wide is close enough to light up a word.
const GLYPH_BOX: Rect = Rect::new(0.0, -220.0, 560.0, 780.0);

struct Collect {
    out: PageText,
    /// The last glyph's origin and em size, to tell a gap or a new line.
    last: Option<(Point, f64)>,
}

impl Collect {
    fn push(&mut self, c: char, at: Rect) {
        self.out.chars.push(c);
        self.out.boxes.push(at);
    }

    fn glyph(&mut self, transform: Affine, text: Option<BfString>) {
        let origin = transform * Point::ORIGIN;
        // The em, in points: how far one em of glyph space reaches on the page.
        let em = (transform * Point::new(0.0, 1000.0) - origin).hypot().max(0.1);
        let at = transform.transform_rect_bbox(GLYPH_BOX);
        if let Some((last, last_em)) = self.last {
            let prev = self.out.boxes.last().copied().unwrap_or(at);
            let size = em.max(last_em);
            let ends_blank = self.out.chars.last().is_some_and(|c| c.is_whitespace());
            if (origin.y - last.y).abs() > size * 0.5 {
                if !ends_blank { self.push('\n', prev); }
            } else if origin.x - last.x > size * 0.85 && !ends_blank {
                // Wider than any one glyph's advance: words placed apart, not spaced.
                self.push(' ', prev);
            }
        }
        match text {
            Some(BfString::Char(c)) => self.push(c, at),
            Some(BfString::String(s)) => for c in s.chars() { self.push(c, at) },
            // A glyph with no known meaning still takes a place, so words stay apart.
            None => self.push('\u{FFFD}', at),
        }
        self.last = Some((origin, em));
    }
}

impl<'a> Device<'a> for Collect {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for glyph in run.glyphs() {
            self.glyph(props.transform * glyph.transform(), glyph.as_unicode());
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The text of one page.
pub fn page_text(page: &Page<'_>) -> PageText {
    let (w, h) = page.render_dimensions();
    let transform = page.initial_transform(true).to_kurbo();
    let cache = InterpreterCache::new();
    let mut context = Context::new(transform, Rect::new(0.0, 0.0, w as f64, h as f64), &cache, page.xref(), InterpreterSettings::default());
    let mut device = Collect { out: PageText::default(), last: None };
    interpret_page(page, &mut context, &mut device);
    device.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro::hayro_syntax::Pdf;

    const FIND10: &[u8] = include_bytes!("../../../tests/fixtures/pdf/find10.pdf");

    #[test]
    fn reads_every_pages_words_with_positions() {
        let pdf = Pdf::new(FIND10.to_vec()).expect("parses");
        let texts: Vec<PageText> = pdf.pages().iter().map(|p| page_text(p)).collect();
        assert_eq!(texts.len(), 10);
        for (i, t) in texts.iter().enumerate() {
            let s: String = t.chars.iter().collect();
            assert!(s.contains(&format!("Page {} of 10", i + 1)), "page {}: {s:?}", i + 1);
            assert!(s.contains("lazy dog"), "{s:?}");
            assert_eq!(t.chars.len(), t.boxes.len());
        }
        let pages: Vec<Vec<char>> = texts.iter().map(|t| t.chars.clone()).collect();
        let all = crate::search::hits(&pages, "zanzibar");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].page, 6);
        // The hit's box is where the line was drawn: 72 pt from the left, and below the
        // first lines (Td 72 720 with 20 pt leading: the sixth line, y = 620 from the
        // bottom, so 172 from the top on a 792 pt page).
        let b = texts[6].boxes[all[0].at];
        assert!(b.x0 > 72.0 && b.x0 < 300.0, "{b:?}");
        assert!(b.y0 > 150.0 && b.y1 < 190.0, "{b:?}");
        assert_eq!(crate::search::hits(&pages, "lantern").len(), 10);
    }

    #[test]
    fn the_welcome_document_has_text() {
        let pdf = Pdf::new(crate::sample::document()).expect("parses");
        let s: String = page_text(&pdf.pages()[0]).chars.iter().collect();
        assert!(s.contains("PDF on SexOS"), "{s:?}");
    }
}
