// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! OpenType shaping with rustybuzz (a Rust port of HarfBuzz): the glyphs a run of
//! text becomes in a font, with ligatures, mark positioning, Arabic joining and
//! Indic reordering. Positions are in font units.
use alloc::vec::Vec;
pub use rustybuzz::Direction;
use rustybuzz::{Face, UnicodeBuffer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub id: u16,
    /// Byte offset in the text of the cluster this glyph belongs to.
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// Shapes `text` in the font `data` (face 0). Direction, script and language are
/// guessed from the text, as `hb-shape` does by default. `None` if the font does not parse.
pub fn shape(data: &[u8], text: &str) -> Option<Vec<Glyph>> {
    let face = Face::from_slice(data, 0)?;
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let out = rustybuzz::shape(&face, &[], buffer);
    Some(out.glyph_infos().iter().zip(out.glyph_positions()).map(|(i, p)| Glyph {
        id: i.glyph_id as u16, cluster: i.cluster, x_advance: p.x_advance, y_advance: p.y_advance,
        x_offset: p.x_offset, y_offset: p.y_offset,
    }).collect())
}
