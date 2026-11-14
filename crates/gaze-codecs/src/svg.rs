//! SVG through resvg/usvg/tiny-skia (Apache-2.0 OR MIT; tiny-skia
//! BSD-3-Clause). They need std, so this module is behind the `svg` feature.
//!
//! SexOS has no system font directory a confined program could read, so the
//! caller passes font files (such as sex-font's bundled `GEIST_SANS`); with
//! none, text is left out and everything else still renders. Files the SVG
//! names by path are never opened; `data:` URLs of nested SVG still work,
//! embedded raster images (PNG/JPEG/GIF/WebP) are skipped.

use alloc::sync::Arc;
use alloc::vec::Vec;

use resvg::tiny_skia;
use resvg::usvg;
use sex_image::Image;

use crate::Error;

/// Largest side `render_svg` draws, to bound memory.
pub const SVG_MAX_SIDE: u32 = 8192;

/// The SVG's intrinsic size in CSS pixels (its `width`/`height`, else its `viewBox`).
pub fn svg_size(svg: &[u8]) -> Result<(f32, f32), Error> {
    let tree = parse(svg, &[])?;
    let s = tree.size();
    Ok((s.width(), s.height()))
}

fn parse(svg: &[u8], fonts: &[&[u8]]) -> Result<usvg::Tree, Error> {
    let mut opt = usvg::Options::default();
    // Never read files the SVG names.
    opt.resources_dir = None;
    opt.image_href_resolver.resolve_string = alloc::boxed::Box::new(|_, _| None);
    let mut db = usvg::fontdb::Database::new();
    for f in fonts {
        db.load_font_data(f.to_vec());
    }
    // Map every generic family (and the default) to the first font given.
    let first = db.faces().next().and_then(|f| f.families.first()).map(|(n, _)| n.clone());
    if let Some(name) = first {
        db.set_serif_family(name.clone());
        db.set_sans_serif_family(name.clone());
        db.set_monospace_family(name.clone());
        db.set_cursive_family(name.clone());
        db.set_fantasy_family(name.clone());
        opt.font_family = name;
    }
    opt.fontdb = Arc::new(db);
    usvg::Tree::from_data(svg, &opt).map_err(|_| Error::Svg)
}

/// Renders `svg` into a `width` x `height` picture in sex-image's layout
/// (0xAARRGGBB, straight alpha). The drawing is scaled uniformly to fit and
/// centred; uncovered pixels are transparent. `fonts` are TrueType/OpenType
/// files for text; the first one also stands in for every generic family.
pub fn render_svg(svg: &[u8], width: u32, height: u32, fonts: &[&[u8]]) -> Result<Image, Error> {
    if width == 0 || height == 0 || width > SVG_MAX_SIDE || height > SVG_MAX_SIDE {
        return Err(Error::TooLarge);
    }
    let tree = parse(svg, fonts)?;
    let size = tree.size();
    let scale = (width as f32 / size.width()).min(height as f32 / size.height());
    let dx = (width as f32 - size.width() * scale) / 2.0;
    let dy = (height as f32 - size.height() * scale) / 2.0;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(Error::OutOfMemory)?;
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(width as usize * height as usize).map_err(|_| Error::OutOfMemory)?;
    pixels.extend(pixmap.pixels().iter().map(|p| {
        let c = p.demultiply();
        u32::from_be_bytes([c.alpha(), c.red(), c.green(), c.blue()])
    }));
    Ok(Image { width: width as usize, height: height as usize, pixels })
}

/// The `width` x `height` that fits the SVG's intrinsic size inside
/// `max_width` x `max_height` (scaling up as well as down), at least 1x1.
pub fn svg_fit(svg: &[u8], max_width: u32, max_height: u32) -> Result<(u32, u32), Error> {
    let (w, h) = svg_size(svg)?;
    let scale = (max_width as f32 / w).min(max_height as f32 / h);
    let fit = |v: f32| ((v * scale + 0.5) as u32).clamp(1, SVG_MAX_SIDE);
    Ok((fit(w), fit(h)))
}
