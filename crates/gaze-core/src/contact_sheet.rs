// SPDX-License-Identifier: GPL-3.0-only
//! Live broadcast video scopes (`Waveform`, `RgbParade`, `Vectorscope`, `Histogram`)
//! powered by `sex_color::scopes` and printable contact sheet renderer (`render_contact_sheet`).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use batik_core::{encode_jpeg, encode_rgba_png, TextLayer};
pub use sex_color::{
    compute_histogram, HistogramData, LinearFrame, ScopeInput, ScopeStandard, ScopeSurface,
    WaveformMode,
};
use sex_image::Image;

use crate::library::PhotoRecord;
use crate::sidecar::{ColorLabel, CullFlag};

/// Active live broadcast scope view in Gaze Develop Studio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    #[default]
    Off,
    WaveformLuma,
    WaveformRgb,
    RgbParade,
    Vectorscope,
    Histogram,
}

impl ScopeKind {
    pub const ALL: [Self; 6] = [
        Self::Off,
        Self::WaveformLuma,
        Self::WaveformRgb,
        Self::RgbParade,
        Self::Vectorscope,
        Self::Histogram,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::WaveformLuma => "Waveform",
            Self::WaveformRgb => "RGB Wave",
            Self::RgbParade => "RGB Parade",
            Self::Vectorscope => "Vectorscope",
            Self::Histogram => "Histogram",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::WaveformLuma => "Wave",
            Self::WaveformRgb => "RGBW",
            Self::RgbParade => "Parade",
            Self::Vectorscope => "Vec",
            Self::Histogram => "Hist",
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::WaveformLuma => "waveform_luma",
            Self::WaveformRgb => "waveform_rgb",
            Self::RgbParade => "rgb_parade",
            Self::Vectorscope => "vectorscope",
            Self::Histogram => "histogram",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::WaveformLuma,
            Self::WaveformLuma => Self::WaveformRgb,
            Self::WaveformRgb => Self::RgbParade,
            Self::RgbParade => Self::Vectorscope,
            Self::Vectorscope => Self::Histogram,
            Self::Histogram => Self::Off,
        }
    }
}

/// Summary exposure/clipping telemetry computed via `sex_color::compute_histogram`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeStats {
    pub mean_ire: f32,
    pub shadow_clip_pct: f32,
    pub highlight_clip_pct: f32,
    pub histogram: HistogramData,
}

/// Computes exposure mean IRE and shadow/highlight clipping percentages from an `Image`.
pub fn analyze_scope_stats(src: &Image, standard: ScopeStandard) -> ScopeStats {
    if src.width == 0 || src.height == 0 || src.pixels.len() < src.width * src.height {
        return ScopeStats {
            mean_ire: 0.0,
            shadow_clip_pct: 0.0,
            highlight_clip_pct: 0.0,
            histogram: HistogramData::default(),
        };
    }
    let input = ScopeInput::argb_u32(src.width, src.height, &src.pixels);
    let hist = compute_histogram(input, standard);
    let total = hist.total_pixels.max(1) as f32;
    let mut sum_luma = 0u64;
    for (bin, &count) in hist.luma.iter().enumerate() {
        sum_luma += (bin as u64) * (count as u64);
    }
    let mean_ire = ((sum_luma as f32 / total) / 255.0) * 100.0;
    let shadow_clip_pct = (hist.luma[0] as f32 / total) * 100.0;
    let highlight_clip_pct = (hist.luma[255] as f32 / total) * 100.0;

    ScopeStats {
        mean_ire,
        shadow_clip_pct,
        highlight_clip_pct,
        histogram: hist,
    }
}

/// Renders `kind` from an sRGB `0xAARRGGBB` [`Image`] into an existing [`ScopeSurface`].
pub fn render_scope_into(
    src: &Image,
    kind: ScopeKind,
    surface: &mut ScopeSurface,
    standard: ScopeStandard,
) {
    if src.width == 0 || src.height == 0 || src.pixels.len() < src.width * src.height {
        surface.clear(0xFF0E_1014);
        return;
    }
    let input = ScopeInput::argb_u32(src.width, src.height, &src.pixels);
    match kind {
        ScopeKind::Off => surface.clear(0xFF0E_1014),
        ScopeKind::WaveformLuma => surface.render_waveform(input, WaveformMode::Luma, standard),
        ScopeKind::WaveformRgb => surface.render_waveform(input, WaveformMode::RgbOverlay, standard),
        ScopeKind::RgbParade => surface.render_rgb_parade(input, standard),
        ScopeKind::Vectorscope => surface.render_vectorscope(input, standard),
        ScopeKind::Histogram => {
            let _ = surface.render_histogram(input, standard);
        }
    }
}

/// Renders `kind` directly from a linear-light [`LinearFrame`] into an existing [`ScopeSurface`].
pub fn render_scope_linear(
    frame: &LinearFrame,
    kind: ScopeKind,
    surface: &mut ScopeSurface,
    standard: ScopeStandard,
) {
    match kind {
        ScopeKind::Off => surface.clear(0xFF0E_1014),
        ScopeKind::WaveformLuma => surface.render_waveform(frame, WaveformMode::Luma, standard),
        ScopeKind::WaveformRgb => surface.render_waveform(frame, WaveformMode::RgbOverlay, standard),
        ScopeKind::RgbParade => surface.render_rgb_parade(frame, standard),
        ScopeKind::Vectorscope => surface.render_vectorscope(frame, standard),
        ScopeKind::Histogram => {
            let _ = surface.render_histogram(frame, standard);
        }
    }
}

/// Convenience helper rendering `kind` into a newly allocated [`sex_image::Image`].
pub fn render_scope_image(
    src: &Image,
    kind: ScopeKind,
    width: usize,
    height: usize,
    standard: ScopeStandard,
) -> Image {
    let w = width.max(16);
    let h = height.max(16);
    let mut surface = ScopeSurface::new(w, h);
    render_scope_into(src, kind, &mut surface, standard);
    Image {
        width: surface.width,
        height: surface.height,
        pixels: surface.bgra,
    }
}

/// Layout and styling specification for printable contact sheet rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactSheetSpec {
    pub page_width: usize,
    pub page_height: usize,
    pub columns: usize,
    pub rows: usize,
    pub margin_px: usize,
    pub gap_px: usize,
    pub caption_height_px: usize,
    pub background_argb: u32,
    pub cell_bg_argb: u32,
    pub border_argb: u32,
    pub text_argb: u32,
    pub accent_argb: u32,
    pub title: Option<String>,
    pub show_stars: bool,
    pub show_flag: bool,
    pub show_filename: bool,
}

impl Default for ContactSheetSpec {
    fn default() -> Self {
        Self::dark_slate()
    }
}

impl ContactSheetSpec {
    pub fn dark_slate() -> Self {
        Self {
            page_width: 1600,
            page_height: 1200,
            columns: 3,
            rows: 3,
            margin_px: 40,
            gap_px: 20,
            caption_height_px: 32,
            background_argb: 0xFF0E_1420,
            cell_bg_argb: 0xFF16_2032,
            border_argb: 0xFF28_3854,
            text_argb: 0xFFF1_F5F9,
            accent_argb: 0xFFF5_9E0B,
            title: Some("Gaze Keeper Contact Sheet".to_string()),
            show_stars: true,
            show_flag: true,
            show_filename: true,
        }
    }

    pub fn light_print() -> Self {
        Self {
            page_width: 1600,
            page_height: 1200,
            columns: 3,
            rows: 3,
            margin_px: 48,
            gap_px: 24,
            caption_height_px: 34,
            background_argb: 0xFFF8_FAFC,
            cell_bg_argb: 0xFFFF_FFFF,
            border_argb: 0xFFCB_D5E1,
            text_argb: 0xFF0F_172A,
            accent_argb: 0xFFD9_7706,
            title: Some("Gaze Print Contact Sheet".to_string()),
            show_stars: true,
            show_flag: true,
            show_filename: true,
        }
    }

    pub fn cells_per_page(&self) -> usize {
        self.columns.max(1) * self.rows.max(1)
    }
}

/// Borrowed thumbnail + culling metadata entry placed into a contact sheet cell.
#[derive(Clone, Debug)]
pub struct ContactSheetEntry<'a> {
    pub image: &'a Image,
    pub filename: &'a str,
    pub stars: u8,
    pub flag: CullFlag,
    pub color_label: ColorLabel,
}

impl<'a> ContactSheetEntry<'a> {
    pub fn from_record(rec: &'a PhotoRecord, image: &'a Image) -> Self {
        Self {
            image,
            filename: &rec.name,
            stars: rec.stars,
            flag: rec.flag,
            color_label: rec.color_label,
        }
    }
}

/// Renders a single contact sheet page (`entries` up to `spec.columns * spec.rows`) into a
/// `sex_image::Image` ready for PNG/JPEG/PDF export.
pub fn render_contact_sheet(
    entries: &[ContactSheetEntry<'_>],
    spec: &ContactSheetSpec,
) -> Result<Image, gaze_codecs::Error> {
    if spec.page_width < 64 || spec.page_height < 64 || spec.columns == 0 || spec.rows == 0 {
        return Err(gaze_codecs::Error::TooLarge);
    }
    let total_px = spec
        .page_width
        .checked_mul(spec.page_height)
        .ok_or(gaze_codecs::Error::TooLarge)?;
    if total_px > 64_000_000 {
        return Err(gaze_codecs::Error::TooLarge);
    }

    let bg = spec.background_argb | 0xFF00_0000;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(total_px)
        .map_err(|_| gaze_codecs::Error::OutOfMemory)?;
    pixels.resize(total_px, bg);

    let margin = spec.margin_px.min(spec.page_width / 6).min(spec.page_height / 6);
    let mut grid_top = margin;

    if let Some(ref title) = spec.title {
        if !title.is_empty() && spec.page_height >= 120 {
            let title_h = ((spec.page_height / 28).clamp(20, 36)) as f32;
            TextLayer::new(title, margin as i32, margin as i32, title_h * 0.72, spec.text_argb | 0xFF00_0000)
                .with_bold(true)
                .render_into_slice(&mut pixels, spec.page_width, spec.page_height);

            let count_txt = format!("{} items", entries.len().min(spec.cells_per_page()));
            let count_x = (spec.page_width.saturating_sub(margin + 110)) as i32;
            TextLayer::new(
                &count_txt,
                count_x.max(margin as i32),
                margin as i32 + 4,
                (title_h * 0.50).max(11.0),
                spec.accent_argb | 0xFF00_0000,
            )
            .render_into_slice(&mut pixels, spec.page_width, spec.page_height);

            grid_top = margin + (title_h as usize) + 12;
        }
    }

    let usable_w = spec.page_width.saturating_sub(margin * 2);
    let usable_h = spec.page_height.saturating_sub(grid_top + margin);
    let cols = spec.columns.max(1);
    let rows = spec.rows.max(1);
    let gap = spec.gap_px.min(usable_w / (cols * 3).max(1)).min(usable_h / (rows * 3).max(1));

    let cell_w = usable_w.saturating_sub(gap * (cols - 1)) / cols;
    let cell_h = usable_h.saturating_sub(gap * (rows - 1)) / rows;
    if cell_w < 16 || cell_h < 16 {
        return Err(gaze_codecs::Error::TooLarge);
    }

    let max_cells = cols * rows;
    let caption_h = if spec.show_filename || spec.show_stars || spec.show_flag {
        spec.caption_height_px.min(cell_h / 2)
    } else {
        0
    };

    for (idx, entry) in entries.iter().take(max_cells).enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cx = margin + col * (cell_w + gap);
        let cy = grid_top + row * (cell_h + gap);

        let card_bg = spec.cell_bg_argb | 0xFF00_0000;
        let border_col = match entry.flag {
            CullFlag::Pick => 0xFF10_B981,
            CullFlag::Reject => 0xFFF4_3F5E,
            CullFlag::Unflagged => spec.border_argb | 0xFF00_0000,
        };

        fill_rect_argb(&mut pixels, spec.page_width, spec.page_height, cx, cy, cell_w, cell_h, card_bg);
        stroke_rect_argb(&mut pixels, spec.page_width, spec.page_height, cx, cy, cell_w, cell_h, border_col);

        let pad = 6usize.min(cell_w / 8).min(cell_h / 8);
        let img_box_x = cx + pad;
        let img_box_y = cy + pad;
        let img_box_w = cell_w.saturating_sub(pad * 2);
        let img_box_h = cell_h.saturating_sub(pad * 2 + caption_h);

        if img_box_w > 0 && img_box_h > 0 {
            blit_fitted_argb(
                &mut pixels,
                spec.page_width,
                spec.page_height,
                entry.image,
                img_box_x,
                img_box_y,
                img_box_w,
                img_box_h,
                card_bg,
            );
        }

        if caption_h >= 12 {
            let cap_y = cy + cell_h - caption_h;
            let mut text_x = cx + pad;

            if entry.color_label != ColorLabel::None && caption_h >= 14 {
                let dot_sz = 8usize.min(caption_h - 4);
                fill_rect_argb(
                    &mut pixels,
                    spec.page_width,
                    spec.page_height,
                    text_x,
                    cap_y + (caption_h - dot_sz) / 2,
                    dot_sz,
                    dot_sz,
                    entry.color_label.argb_u32(),
                );
                text_x += dot_sz + 6;
            }

            // Right-aligned star rating icons
            let stars = entry.stars.min(5);
            let stars_w = if spec.show_stars { 5 * 11 + 6 } else { 0 };
            if spec.show_stars && cell_w > stars_w + 24 {
                let sx0 = cx + cell_w - pad - 5 * 11;
                let sy0 = cap_y + (caption_h.saturating_sub(9)) / 2;
                for s in 0..5u8 {
                    let color = if s < stars {
                        spec.accent_argb | 0xFF00_0000
                    } else {
                        spec.border_argb | 0xFF00_0000
                    };
                    draw_star_glyph(
                        &mut pixels,
                        spec.page_width,
                        spec.page_height,
                        sx0 + (s as usize) * 11,
                        sy0,
                        color,
                    );
                }
            }

            if spec.show_filename || spec.show_flag {
                let mut label = String::new();
                if spec.show_flag && entry.flag == CullFlag::Pick {
                    label.push_str("[P] ");
                } else if spec.show_flag && entry.flag == CullFlag::Reject {
                    label.push_str("[X] ");
                }
                if spec.show_filename {
                    label.push_str(entry.filename);
                }

                let avail_w = (cx + cell_w).saturating_sub(text_x + pad + stars_w);
                if avail_w >= 16 && !label.is_empty() {
                    let font_px = ((caption_h as f32) * 0.48).clamp(10.0, 16.0);
                    let mut cell_buf = vec![card_bg; avail_w * caption_h];
                    TextLayer::new(&label, 0, ((caption_h as i32 - font_px as i32) / 2).max(1), font_px, spec.text_argb | 0xFF00_0000)
                        .render_into_slice(&mut cell_buf, avail_w, caption_h);
                    for row_i in 0..caption_h {
                        let dy = cap_y + row_i;
                        if dy < spec.page_height {
                            let dst_off = dy * spec.page_width + text_x;
                            let src_off = row_i * avail_w;
                            pixels[dst_off..dst_off + avail_w].copy_from_slice(&cell_buf[src_off..src_off + avail_w]);
                        }
                    }
                }
            }
        }
    }

    Ok(Image {
        width: spec.page_width,
        height: spec.page_height,
        pixels,
    })
}

/// Renders one or more contact sheet pages if `entries.len() > spec.cells_per_page()`.
pub fn render_contact_sheet_pages(
    entries: &[ContactSheetEntry<'_>],
    spec: &ContactSheetSpec,
) -> Result<Vec<Image>, gaze_codecs::Error> {
    let per_page = spec.cells_per_page();
    if entries.is_empty() {
        return Ok(vec![render_contact_sheet(&[], spec)?]);
    }
    let mut pages = Vec::new();
    let total_pages = (entries.len() + per_page - 1) / per_page;
    for page_idx in 0..total_pages {
        let start = page_idx * per_page;
        let end = (start + per_page).min(entries.len());
        let mut page_spec = spec.clone();
        if total_pages > 1 {
            if let Some(ref t) = spec.title {
                page_spec.title = Some(format!("{} (Page {}/{})", t, page_idx + 1, total_pages));
            }
        }
        pages.push(render_contact_sheet(&entries[start..end], &page_spec)?);
    }
    Ok(pages)
}

/// Renders and encodes a contact sheet page directly as RGBA PNG bytes.
pub fn export_contact_sheet_png(
    entries: &[ContactSheetEntry<'_>],
    spec: &ContactSheetSpec,
) -> Result<Vec<u8>, gaze_codecs::Error> {
    let img = render_contact_sheet(entries, spec)?;
    encode_rgba_png(&img.pixels, img.width, img.height).map_err(|_| gaze_codecs::Error::OutOfMemory)
}

/// Renders and encodes a contact sheet page directly as baseline JPEG bytes (`quality` in `1..=100`).
pub fn export_contact_sheet_jpeg(
    entries: &[ContactSheetEntry<'_>],
    spec: &ContactSheetSpec,
    quality: u8,
) -> Result<Vec<u8>, gaze_codecs::Error> {
    let img = render_contact_sheet(entries, spec)?;
    encode_jpeg(&img.pixels, img.width, img.height, quality.clamp(1, 100))
        .map_err(|_| gaze_codecs::Error::OutOfMemory)
}

fn fill_rect_argb(
    pixels: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    let x0 = x.min(fb_w);
    let y0 = y.min(fb_h);
    let x1 = x.saturating_add(w).min(fb_w);
    let y1 = y.saturating_add(h).min(fb_h);
    for yy in y0..y1 {
        pixels[yy * fb_w + x0..yy * fb_w + x1].fill(color);
    }
}

fn stroke_rect_argb(
    pixels: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    if w == 0 || h == 0 {
        return;
    }
    fill_rect_argb(pixels, fb_w, fb_h, x, y, w, 1, color);
    fill_rect_argb(pixels, fb_w, fb_h, x, y + h - 1, w, 1, color);
    fill_rect_argb(pixels, fb_w, fb_h, x, y, 1, h, color);
    fill_rect_argb(pixels, fb_w, fb_h, x + w - 1, y, 1, h, color);
}

#[allow(clippy::too_many_arguments)]
fn blit_fitted_argb(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    src: &Image,
    dx: usize,
    dy: usize,
    dw: usize,
    dh: usize,
    bg_argb: u32,
) {
    if src.width == 0 || src.height == 0 || src.pixels.len() < src.width * src.height || dw == 0 || dh == 0 {
        return;
    }
    let (tw, th) = if src.width * dh >= src.height * dw {
        (dw, (src.height * dw / src.width).max(1))
    } else {
        ((src.width * dh / src.height).max(1), dh)
    };
    let ox = dx + (dw - tw) / 2;
    let oy = dy + (dh - th) / 2;

    for y in 0..th {
        let py = oy + y;
        if py >= fb_h {
            continue;
        }
        let sy = (y * src.height / th).min(src.height - 1);
        let src_row = &src.pixels[sy * src.width..(sy + 1) * src.width];
        let fb_row = py * fb_w;
        for x in 0..tw {
            let px = ox + x;
            if px >= fb_w {
                continue;
            }
            let sx = (x * src.width / tw).min(src.width - 1);
            let p = src_row[sx];
            let a = (p >> 24) & 0xFF;
            let blended = if a == 255 {
                p
            } else {
                let mix = |shift: u32| {
                    ((((p >> shift) & 0xFF) * a + ((bg_argb >> shift) & 0xFF) * (255 - a)) / 255)
                        << shift
                };
                0xFF00_0000 | mix(16) | mix(8) | mix(0)
            };
            fb[fb_row + px] = blended;
        }
    }
}

/// Draws a crisp 9x9 star icon at `(x, y)` in `0xAARRGGBB`.
fn draw_star_glyph(pixels: &mut [u32], fb_w: usize, fb_h: usize, x: usize, y: usize, color: u32) {
    const STAR_9X9: [&[u8; 9]; 9] = [
        b"....#....",
        b"....#....",
        b"...###...",
        b"#########",
        b".#######.",
        b"..#####..",
        b"..##.##..",
        b".#.....#.",
        b".........",
    ];
    for (ry, row) in STAR_9X9.iter().enumerate() {
        let py = y + ry;
        if py >= fb_h {
            continue;
        }
        for (rx, &ch) in row.iter().enumerate() {
            let px = x + rx;
            if px < fb_w && ch == b'#' {
                pixels[py * fb_w + px] = color;
            }
        }
    }
}
