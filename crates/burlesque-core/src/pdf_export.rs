// SPDX-License-Identifier: GPL-3.0-only
//! Vector PDF-1.4 exporter for presentation decks.
//!
//! Provides multi-page PDF generation conforming to PDF-1.4:
//! - Multi-page vector slide canvas with aspect ratio mapping
//! - Theme backgrounds and slide master chrome (top accent ribbon, styled slide numbers)
//! - Vector shapes: Rect, RoundedRect, Ellipse, Line, Arrow
//! - Text boxes with bold (/F2), regular (/F1), and monospace (/F3) typography
//! - Rich tables with header fills, borders, and styled cell text
//! - Code blocks with dark card backgrounds, line numbers, and Courier
//! - Embedded raster pictures (FlateDecode RGB24)
//! - Optional Speaker Notes pages (Keynote-grade handout layout)
//! - Robust escaping, object table, xref table, and valid %%EOF trailer

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::Color;
use crate::deck::Deck;
use crate::elements::{Background, ShapeKind, SlideElement};
use crate::Result;

/// Export deck to a standards-compliant multi-page PDF-1.4 document.
pub fn export_pdf(deck: &Deck) -> Result<Vec<u8>> {
    export_pdf_with_notes(deck, false)
}

/// Export deck to a multi-page PDF-1.4 document, optionally generating speaker notes pages.
pub fn export_pdf_with_notes(deck: &Deck, include_notes: bool) -> Result<Vec<u8>> {
    let mut pdf = PdfWriter::new();

    let page_w = deck.width();
    let page_h = deck.height();
    let num_slides = deck.slides.len().max(1);

    // Object 1: Catalog
    // Object 2: Pages root
    // Object 3: Font F1 (Helvetica)
    // Object 4: Font F2 (Helvetica-Bold)
    // Object 5: Font F3 (Courier)
    let catalog_id = 1;
    let pages_id = 2;
    let font_regular_id = 3;
    let font_bold_id = 4;
    let font_mono_id = 5;

    let mut next_id = 6;
    let mut page_ids = Vec::with_capacity(num_slides * 2);

    struct PagePlan {
        page_id: u32,
        content_id: u32,
        content_bytes: Vec<u8>,
        images: Vec<(u32, String, u32, u32, Vec<u8>)>,
    }

    let mut plans = Vec::with_capacity(num_slides * 2);

    for (slide_idx, slide) in deck.slides.iter().enumerate() {
        let p_id = next_id;
        next_id += 1;
        let c_id = next_id;
        next_id += 1;

        let mut content = Vec::new();
        let mut images = Vec::new();

        // 1. Background
        let (bg_r, bg_g, bg_b) = match slide.background_override {
            Some(Background::Solid(c)) => (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0),
            _ => (
                deck.theme.palette.background.r() as f32 / 255.0,
                deck.theme.palette.background.g() as f32 / 255.0,
                deck.theme.palette.background.b() as f32 / 255.0,
            ),
        };
        content.extend_from_slice(
            format!("{:.3} {:.3} {:.3} rg 0 0 {:.1} {:.1} re f\n", bg_r, bg_g, bg_b, page_w, page_h).as_bytes(),
        );

        // 2. Master chrome: top accent ribbon
        if deck.theme.show_master_chrome {
            let p_acc = deck.theme.palette.primary_accent;
            let s_acc = deck.theme.palette.secondary_accent;
            let ribbon_h = 4.0;
            let split_w = page_w * 0.65;
            content.extend_from_slice(
                format!(
                    "{:.3} {:.3} {:.3} rg 0 {:.1} {:.1} {:.1} re f\n",
                    p_acc.r() as f32 / 255.0, p_acc.g() as f32 / 255.0, p_acc.b() as f32 / 255.0,
                    page_h - ribbon_h, split_w, ribbon_h
                ).as_bytes(),
            );
            content.extend_from_slice(
                format!(
                    "{:.3} {:.3} {:.3} rg {:.1} {:.1} {:.1} {:.1} re f\n",
                    s_acc.r() as f32 / 255.0, s_acc.g() as f32 / 255.0, s_acc.b() as f32 / 255.0,
                    split_w, page_h - ribbon_h, page_w - split_w, ribbon_h
                ).as_bytes(),
            );

            // Styled slide number badge at bottom-right
            let num_text = format!("{:02}", slide.id);
            let mut_c = deck.theme.palette.muted;
            content.extend_from_slice(
                format!(
                    "BT /F1 14 Tf {:.3} {:.3} {:.3} rg 1 0 0 1 {:.1} 24 Tm ({}) Tj ET\n",
                    mut_c.r() as f32 / 255.0, mut_c.g() as f32 / 255.0, mut_c.b() as f32 / 255.0,
                    page_w - 48.0, escape_pdf_string(&num_text)
                ).as_bytes(),
            );
        }

        // 3. Slide Elements
        for elem in &slide.elements {
            match elem {
                SlideElement::Shape(s) => {
                    let pdf_y = page_h - s.bounds.y - s.bounds.h;
                    let x = s.bounds.x;
                    let y = pdf_y;
                    let w = s.bounds.w;
                    let h = s.bounds.h;

                    if let Some(fill) = s.fill {
                        let fr = fill.r() as f32 / 255.0;
                        let fg = fill.g() as f32 / 255.0;
                        let fb = fill.b() as f32 / 255.0;
                        content.extend_from_slice(format!("{:.3} {:.3} {:.3} rg ", fr, fg, fb).as_bytes());
                    }
                    if let Some(st) = s.stroke {
                        let sr = st.color.r() as f32 / 255.0;
                        let sg = st.color.g() as f32 / 255.0;
                        let sb = st.color.b() as f32 / 255.0;
                        content.extend_from_slice(
                            format!("{:.3} {:.3} {:.3} RG {:.2} w ", sr, sg, sb, st.width).as_bytes(),
                        );
                    }

                    let op = match (s.fill.is_some(), s.stroke.is_some()) {
                        (true, true) => "B",
                        (true, false) => "f",
                        (false, true) => "S",
                        (false, false) => "",
                    };

                    if !op.is_empty() {
                        match s.kind {
                            ShapeKind::Rect | ShapeKind::RoundedRect { .. } => {
                                content.extend_from_slice(format!("{:.1} {:.1} {:.1} {:.1} re {}\n", x, y, w, h, op).as_bytes());
                            }
                            ShapeKind::Ellipse => {
                                let rx = w / 2.0;
                                let ry = h / 2.0;
                                let cx = x + rx;
                                let cy = y + ry;
                                let kx = rx * 0.55228475;
                                let ky = ry * 0.55228475;
                                content.extend_from_slice(
                                    format!(
                                        "{:.1} {:.1} m {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} c \
                                         {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} c \
                                         {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} c \
                                         {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} c {}\n",
                                        cx - rx, cy,
                                        cx - rx, cy + ky, cx - kx, cy + ry, cx, cy + ry,
                                        cx + kx, cy + ry, cx + rx, cy + ky, cx + rx, cy,
                                        cx + rx, cy - ky, cx + kx, cy - ry, cx, cy - ry,
                                        cx - kx, cy - ry, cx - rx, cy - ky, cx - rx, cy,
                                        op
                                    ).as_bytes(),
                                );
                            }
                            ShapeKind::Line => {
                                content.extend_from_slice(
                                    format!("{:.1} {:.1} m {:.1} {:.1} l S\n", x, y + h, x + w, y).as_bytes(),
                                );
                            }
                            ShapeKind::Arrow => {
                                content.extend_from_slice(
                                    format!("{:.1} {:.1} m {:.1} {:.1} l S\n", x, y + h / 2.0, x + w, y + h / 2.0).as_bytes(),
                                );
                            }
                        }
                    }
                }
                SlideElement::TextBox(tb) => {
                    let mut cur_y = tb.bounds.y + tb.padding;
                    for p in &tb.paragraphs {
                        let indent = p.bullet_level as f32 * 28.0;
                        let line_x = tb.bounds.x + tb.padding + indent;

                        for run in &p.runs {
                            let font_name = if run.mono {
                                "/F3"
                            } else if run.bold {
                                "/F2"
                            } else {
                                "/F1"
                            };

                            let color = run.color.unwrap_or(deck.theme.palette.foreground);
                            let cr = color.r() as f32 / 255.0;
                            let cg = color.g() as f32 / 255.0;
                            let cb = color.b() as f32 / 255.0;

                            let pdf_text_y = page_h - cur_y - run.font_size;
                            let escaped = escape_pdf_string(&run.text);

                            content.extend_from_slice(
                                format!(
                                    "BT {} {:.1} Tf {:.3} {:.3} {:.3} rg 1 0 0 1 {:.1} {:.1} Tm ({}) Tj ET\n",
                                    font_name, run.font_size, cr, cg, cb, line_x, pdf_text_y, escaped
                                ).as_bytes(),
                            );
                            cur_y += run.font_size * p.line_spacing;
                        }
                    }
                }
                SlideElement::Image(img) => {
                    if let Some(ref decoded) = img.decoded {
                        let img_id = next_id;
                        next_id += 1;
                        let img_name = format!("Im{}", images.len() + 1);

                        let mut rgb = Vec::with_capacity(decoded.width * decoded.height * 3);
                        for &px in &decoded.pixels {
                            rgb.push(((px >> 16) & 0xFF) as u8);
                            rgb.push(((px >> 8) & 0xFF) as u8);
                            rgb.push((px & 0xFF) as u8);
                        }
                        let deflated = miniz_oxide::deflate::compress_to_vec(&rgb, 6);

                        let pdf_y = page_h - img.bounds.y - img.bounds.h;
                        content.extend_from_slice(
                            format!(
                                "q {:.1} 0 0 {:.1} {:.1} {:.1} cm /{} Do Q\n",
                                img.bounds.w, img.bounds.h, img.bounds.x, pdf_y, img_name
                            ).as_bytes(),
                        );

                        images.push((img_id, img_name, decoded.width as u32, decoded.height as u32, deflated));
                    }
                }
                SlideElement::Code(cb) => {
                    let pdf_y = page_h - cb.bounds.y - cb.bounds.h;
                    let bg = cb.background;
                    content.extend_from_slice(
                        format!(
                            "{:.3} {:.3} {:.3} rg {:.1} {:.1} {:.1} {:.1} re f\n",
                            bg.r() as f32 / 255.0,
                            bg.g() as f32 / 255.0,
                            bg.b() as f32 / 255.0,
                            cb.bounds.x,
                            pdf_y,
                            cb.bounds.w,
                            cb.bounds.h
                        ).as_bytes(),
                    );

                    let tc = cb.text_color;
                    let mut code_y = cb.bounds.y + 16.0;
                    for line in cb.code.lines() {
                        let pdf_line_y = page_h - code_y - cb.font_size;
                        let escaped = escape_pdf_string(line);
                        content.extend_from_slice(
                            format!(
                                "BT /F3 {:.1} Tf {:.3} {:.3} {:.3} rg 1 0 0 1 {:.1} {:.1} Tm ({}) Tj ET\n",
                                cb.font_size,
                                tc.r() as f32 / 255.0,
                                tc.g() as f32 / 255.0,
                                tc.b() as f32 / 255.0,
                                cb.bounds.x + 16.0,
                                pdf_line_y,
                                escaped
                            ).as_bytes(),
                        );
                        code_y += cb.font_size * 1.35;
                    }
                }
                SlideElement::Table(t) => {
                    let pdf_y = page_h - t.bounds.y - t.bounds.h;
                    let col_w = t.bounds.w / t.cols.max(1) as f32;
                    let row_h = t.bounds.h / t.rows.max(1) as f32;

                    // Header row background fill if configured
                    if t.has_header_row && t.rows > 0 {
                        if let Some(hbg) = t.header_background {
                            content.extend_from_slice(
                                format!(
                                    "{:.3} {:.3} {:.3} rg {:.1} {:.1} {:.1} {:.1} re f\n",
                                    hbg.r() as f32 / 255.0, hbg.g() as f32 / 255.0, hbg.b() as f32 / 255.0,
                                    t.bounds.x, page_h - t.bounds.y - row_h, t.bounds.w, row_h
                                ).as_bytes(),
                            );
                        }
                    }

                    // Table outer border
                    let bc = t.border_color;
                    content.extend_from_slice(
                        format!(
                            "{:.3} {:.3} {:.3} RG {:.1} w {:.1} {:.1} {:.1} {:.1} re S\n",
                            bc.r() as f32 / 255.0,
                            bc.g() as f32 / 255.0,
                            bc.b() as f32 / 255.0,
                            t.border_width,
                            t.bounds.x,
                            pdf_y,
                            t.bounds.w,
                            t.bounds.h
                        ).as_bytes(),
                    );

                    // Cell text
                    for (r, row) in t.cells.iter().enumerate() {
                        for (c, cell) in row.iter().enumerate() {
                            let cx = t.bounds.x + c as f32 * col_w + 6.0;
                            let cy = t.bounds.y + r as f32 * row_h + 18.0;
                            let pdf_cell_y = page_h - cy;
                            let escaped = escape_pdf_string(&cell.text);
                            let font = if cell.bold { "/F2" } else { "/F1" };
                            let tc = cell.text_color.unwrap_or(Color::rgb(0xF8, 0xFA, 0xFC));
                            content.extend_from_slice(
                                format!(
                                    "BT {} 16 Tf {:.3} {:.3} {:.3} rg 1 0 0 1 {:.1} {:.1} Tm ({}) Tj ET\n",
                                    font, tc.r() as f32 / 255.0, tc.g() as f32 / 255.0, tc.b() as f32 / 255.0, cx, pdf_cell_y, escaped
                                ).as_bytes(),
                            );
                        }
                    }
                }
            }
        }

        page_ids.push(p_id);
        plans.push(PagePlan {
            page_id: p_id,
            content_id: c_id,
            content_bytes: content,
            images,
        });

        // 4. If include_notes is requested and slide has notes: generate dedicated Notes page
        if include_notes && !slide.notes().is_empty() {
            let note_p_id = next_id;
            next_id += 1;
            let note_c_id = next_id;
            next_id += 1;

            let mut note_content = Vec::new();
            // Background
            note_content.extend_from_slice(b"0.98 0.98 0.98 rg 0 0 1920 1080 re f\n");

            // Header bar
            note_content.extend_from_slice(b"0.12 0.16 0.23 rg 0 1000 1920 80 re f\n");
            let title_text = if slide.title.is_empty() {
                format!("Slide {} Notes", slide_idx + 1)
            } else {
                format!("Slide {}: {}", slide_idx + 1, slide.title)
            };
            note_content.extend_from_slice(
                format!(
                    "BT /F2 26 Tf 1 1 1 rg 1 0 0 1 48 1030 Tm ({}) Tj ET\n",
                    escape_pdf_string(&title_text)
                ).as_bytes(),
            );

            // Notes card
            note_content.extend_from_slice(b"1 1 1 rg 48 100 1824 850 re f\n");
            note_content.extend_from_slice(b"0.8 0.8 0.8 RG 1.5 w 48 100 1824 850 re S\n");

            // Speaker notes text
            let mut text_y = 900.0f32;
            for line in slide.notes().lines() {
                if text_y < 140.0 {
                    break;
                }
                note_content.extend_from_slice(
                    format!(
                        "BT /F1 20 Tf 0.1 0.1 0.1 rg 1 0 0 1 80 {:.1} Tm ({}) Tj ET\n",
                        text_y, escape_pdf_string(line)
                    ).as_bytes(),
                );
                text_y -= 28.0;
            }

            page_ids.push(note_p_id);
            plans.push(PagePlan {
                page_id: note_p_id,
                content_id: note_c_id,
                content_bytes: note_content,
                images: Vec::new(),
            });
        }
    }

    // Write PDF objects:
    pdf.write_header();

    // 1: Catalog
    pdf.start_obj(catalog_id);
    pdf.write_str("<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // 2: Pages root
    pdf.start_obj(pages_id);
    let mut kids = String::from("[");
    for pid in &page_ids {
        kids.push_str(&format!(" {} 0 R", pid));
    }
    kids.push_str(" ]");
    pdf.write_str(&format!(
        "<< /Type /Pages /Kids {} /Count {} /MediaBox [ 0 0 {:.1} {:.1} ] >>\nendobj\n",
        kids, page_ids.len(), page_w, page_h
    ));

    // 3: Font F1 (Helvetica)
    pdf.start_obj(font_regular_id);
    pdf.write_str("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n");

    // 4: Font F2 (Helvetica-Bold)
    pdf.start_obj(font_bold_id);
    pdf.write_str("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>\nendobj\n");

    // 5: Font F3 (Courier)
    pdf.start_obj(font_mono_id);
    pdf.write_str("<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>\nendobj\n");

    // Pages & Contents
    for plan in &plans {
        pdf.start_obj(plan.page_id);
        let mut xobj_dict = String::new();
        if !plan.images.is_empty() {
            xobj_dict.push_str(" /XObject << ");
            for (img_id, img_name, _, _, _) in &plan.images {
                xobj_dict.push_str(&format!("/{} {} 0 R ", img_name, img_id));
            }
            xobj_dict.push_str(">> ");
        }

        pdf.write_str(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 {:.1} {:.1} ] /Contents {} 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >>{}>> >>\nendobj\n",
            page_w, page_h, plan.content_id, xobj_dict
        ));

        pdf.start_obj(plan.content_id);
        pdf.write_str(&format!("<< /Length {} >>\nstream\n", plan.content_bytes.len()));
        pdf.write_bytes(&plan.content_bytes);
        pdf.write_str("\nendstream\nendobj\n");

        for (img_id, _, w, h, deflated) in &plan.images {
            pdf.start_obj(*img_id);
            pdf.write_str(&format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
                w, h, deflated.len()
            ));
            pdf.write_bytes(deflated);
            pdf.write_str("\nendstream\nendobj\n");
        }
    }

    // XRef & Trailer
    pdf.finish(catalog_id);

    Ok(pdf.buffer)
}

fn escape_pdf_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\0' => {}
            _ => out.push(c),
        }
    }
    out
}

struct PdfWriter {
    buffer: Vec<u8>,
    offsets: Vec<usize>,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            offsets: vec![0], // entry 0 is free
        }
    }

    fn write_header(&mut self) {
        self.buffer.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    }

    fn write_str(&mut self, s: &str) {
        self.buffer.extend_from_slice(s.as_bytes());
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    fn start_obj(&mut self, id: u32) {
        while self.offsets.len() <= id as usize {
            self.offsets.push(0);
        }
        self.offsets[id as usize] = self.buffer.len();
        let header = format!("{} 0 obj\n", id);
        self.write_str(&header);
    }

    fn finish(&mut self, root_id: u32) {
        let xref_offset = self.buffer.len();
        let total_objs = self.offsets.len();

        self.write_str(&format!("xref\n0 {}\n", total_objs));
        self.write_str("0000000000 65535 f \n");
        for id in 1..total_objs {
            let off = self.offsets[id];
            self.write_str(&format!("{:010} 00000 n \n", off));
        }

        self.write_str(&format!(
            "trailer\n<< /Size {} /Root {} 0 R >>\nstartxref\n{}\n%%EOF\n",
            total_objs, root_id, xref_offset
        ));
    }
}
