// SPDX-License-Identifier: GPL-3.0-only
//! OOXML `.pptx` presentation importer and generator.
//!
//! Provides memory-bounded, panic-free parsing of OOXML `.pptx` presentations:
//! - [Content_Types].xml and presentation relationships
//! - Slide size <p:sldSz> and aspect ratio
//! - Palette color extraction from ppt/theme/theme1.xml
//! - Slide shapes <p:sp>, text hierarchy, bullets, typography <a:rPr>
//! - Vector shapes <a:prstGeom> (rect, roundRect, ellipse, line, arrow)
//! - Tables <p:graphicFrame> / <a:tbl>
//! - Embedded media pictures <p:pic> via slide rels
//! - Speaker notes <p:notes> from ppt/notesSlides/notesSlideN.xml
//! - Slide transitions <p:transition> (dissolve, push, wipe, zoom, fade)
//!
//! Also provides `export_pptx(&Deck) -> Result<Vec<u8>>` to serialize multi-slide
//! presentations with themes, transitions, shapes, tables, and speaker notes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::{Color, Rect};
use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{
    FitMode, ImageElement, Paragraph, Shape, ShapeKind, SlideElement, StrokeStyle, TableCell,
    TableElement, TextAlign, TextBox, TextRun, Transition,
};
use crate::layout::SlideLayout;
use crate::xml_utils::XmlNode;
use crate::{Error, Result};

/// Maximum allowed uncompressed size per archive member (32 MiB).
pub const MAX_ARCHIVE_MEMBER_BYTES: usize = 32 * 1024 * 1024;
/// Maximum cumulative extracted bytes per archive import (128 MiB).
pub const MAX_ARCHIVE_TOTAL_BYTES: usize = 128 * 1024 * 1024;
/// Maximum number of archive entries allowed to prevent zip DOS.
pub const MAX_ARCHIVE_ENTRIES: usize = 2048;
/// Maximum number of slides parsed per deck.
pub const MAX_SLIDES: usize = 500;

struct BoundedSink<'a> {
    out: &'a mut Vec<u8>,
    member_limit: usize,
    total_extracted: &'a mut usize,
    total_limit: usize,
}

impl<'a> sex_archive::Sink for BoundedSink<'a> {
    fn write_all(&mut self, data: &[u8]) -> core::result::Result<(), sex_archive::IoError> {
        let new_member = self.out.len().saturating_add(data.len());
        if new_member > self.member_limit {
            return Err(sex_archive::IoError("archive member exceeds 32 MiB limit"));
        }
        let new_total = (*self.total_extracted).saturating_add(data.len());
        if new_total > self.total_limit {
            return Err(sex_archive::IoError("total archive extracted bytes exceed 128 MiB limit"));
        }
        *self.total_extracted = new_total;
        self.out.extend_from_slice(data);
        Ok(())
    }
}

/// Extract entry bytes from a zip archive by path (normalizing separators) with defensive size bounds.
pub fn extract_archive_file(
    archive: &sex_archive::Archive<&[u8]>,
    target_path: &str,
    total_extracted: &mut usize,
) -> Option<Vec<u8>> {
    let clean = target_path.replace('\\', "/");
    let trimmed = clean.trim_start_matches('/');

    let extract_idx = |idx: usize, total: &mut usize| -> Option<Vec<u8>> {
        let entry = archive.entries().get(idx)?;
        if entry.size_known && entry.size > MAX_ARCHIVE_MEMBER_BYTES as u64 {
            return None;
        }
        let mut out = Vec::new();
        let mut sink = BoundedSink {
            out: &mut out,
            member_limit: MAX_ARCHIVE_MEMBER_BYTES,
            total_extracted: total,
            total_limit: MAX_ARCHIVE_TOTAL_BYTES,
        };
        if archive.extract(idx, &mut sink, &mut sex_archive::no_progress).is_ok() {
            Some(out)
        } else {
            None
        }
    };

    if let Some(idx) = archive.find(trimmed) {
        if let Some(data) = extract_idx(idx, total_extracted) {
            return Some(data);
        }
    }
    // Case-insensitive fallback
    for (i, entry) in archive.entries().iter().enumerate() {
        if entry.path.eq_ignore_ascii_case(trimmed) {
            if let Some(data) = extract_idx(i, total_extracted) {
                return Some(data);
            }
        }
    }
    None
}

/// Parse a hex color string like "FF0077" into Color safely without char-boundary slice panics.
pub fn parse_hex_color(hex: &str) -> Option<Color> {
    let clean = hex.trim_start_matches('#');
    let b = clean.as_bytes();
    if b.len() == 6 && b.iter().all(|c| c.is_ascii_hexdigit()) {
        let r = u8::from_str_radix(core::str::from_utf8(&b[0..2]).ok()?, 16).ok()?;
        let g = u8::from_str_radix(core::str::from_utf8(&b[2..4]).ok()?, 16).ok()?;
        let b_val = u8::from_str_radix(core::str::from_utf8(&b[4..6]).ok()?, 16).ok()?;
        Some(Color::rgb(r, g, b_val))
    } else if b.len() == 8 && b.iter().all(|c| c.is_ascii_hexdigit()) {
        let a = u8::from_str_radix(core::str::from_utf8(&b[0..2]).ok()?, 16).ok()?;
        let r = u8::from_str_radix(core::str::from_utf8(&b[2..4]).ok()?, 16).ok()?;
        let g = u8::from_str_radix(core::str::from_utf8(&b[4..6]).ok()?, 16).ok()?;
        let b_val = u8::from_str_radix(core::str::from_utf8(&b[6..8]).ok()?, 16).ok()?;
        Some(Color::rgba(r, g, b_val, a))
    } else {
        None
    }
}

/// Convert EMU coordinates to deck logical points with overflow and NaN checks.
fn emu_to_pixels(emu: f32, emu_total: f32, logical_total: f32) -> f32 {
    if !emu.is_finite() {
        return 0.0;
    }
    let res = if emu_total.is_finite() && emu_total > 0.0 && logical_total.is_finite() && logical_total > 0.0 {
        (emu / emu_total) * logical_total
    } else {
        emu / 9525.0
    };
    if res.is_finite() {
        res.clamp(-100_000.0, 100_000.0)
    } else {
        0.0
    }
}

/// Import PPTX presentation from raw container bytes.
pub fn import_pptx(bytes: &[u8]) -> Result<Deck> {
    let archive = sex_archive::Archive::open(bytes).map_err(|_| Error::InvalidFormat)?;
    if archive.entries().len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::MalformedDocument);
    }
    let mut total_extracted = 0usize;

    // 1. Read ppt/presentation.xml
    let pres_bytes = extract_archive_file(&archive, "ppt/presentation.xml", &mut total_extracted)
        .ok_or(Error::MalformedDocument)?;
    let pres_str = core::str::from_utf8(&pres_bytes).map_err(|_| Error::MalformedDocument)?;
    let pres_xml = XmlNode::parse(pres_str).map_err(|_| Error::MalformedDocument)?;

    // Parse slide size <p:sldSz cx="..." cy="..."/>
    let (slide_emu_w, slide_emu_h) = if let Some(sld_sz) = pres_xml.child("sldSz") {
        let cx = sld_sz.attr("cx").and_then(|v| v.parse::<f32>().ok()).unwrap_or(12192000.0);
        let cy = sld_sz.attr("cy").and_then(|v| v.parse::<f32>().ok()).unwrap_or(6858000.0);
        (cx, cy)
    } else {
        (12192000.0, 6858000.0) // 16:9 standard
    };

    let ratio = if slide_emu_h > 0.0 { slide_emu_w / slide_emu_h } else { 16.0 / 9.0 };
    let aspect_ratio = if (ratio - 16.0 / 9.0).abs() < 0.08 {
        AspectRatio::Ratio16x9
    } else if (ratio - 16.0 / 10.0).abs() < 0.08 {
        AspectRatio::Ratio16x10
    } else if (ratio - 4.0 / 3.0).abs() < 0.08 {
        AspectRatio::Ratio4x3
    } else {
        AspectRatio::Custom {
            width: 1920.0,
            height: (1920.0 / ratio).max(400.0),
        }
    };

    let mut deck = Deck::new(aspect_ratio);

    // Parse theme colors if present: ppt/theme/theme1.xml
    if let Some(theme_bytes) = extract_archive_file(&archive, "ppt/theme/theme1.xml", &mut total_extracted) {
        if let Ok(theme_str) = core::str::from_utf8(&theme_bytes) {
            if let Ok(theme_xml) = XmlNode::parse(theme_str) {
                let mut schemes = Vec::new();
                theme_xml.find_all("clrScheme", &mut schemes);
                if let Some(scheme) = schemes.first() {
                    let parse_node_clr = |node_name: &str| -> Option<Color> {
                        let node = scheme.child(node_name)?;
                        if let Some(srgb) = node.child("srgbClr") {
                            if let Some(val) = srgb.attr("val") {
                                return parse_hex_color(val);
                            }
                        }
                        None
                    };
                    if let Some(c) = parse_node_clr("dk1") {
                        deck.theme.palette.foreground = c;
                    }
                    if let Some(c) = parse_node_clr("lt1") {
                        deck.theme.palette.background = c;
                    }
                    if let Some(c) = parse_node_clr("accent1") {
                        deck.theme.palette.primary_accent = c;
                    }
                    if let Some(c) = parse_node_clr("accent2") {
                        deck.theme.palette.secondary_accent = c;
                    }
                }
            }
        }
    }

    // 2. Read presentation relationships to map rId -> slide path
    let mut slide_rels = Vec::new();
    if let Some(rels_bytes) = extract_archive_file(&archive, "ppt/_rels/presentation.xml.rels", &mut total_extracted) {
        if let Ok(rels_str) = core::str::from_utf8(&rels_bytes) {
            if let Ok(rels_xml) = XmlNode::parse(rels_str) {
                for rel in rels_xml.children_named("Relationship") {
                    let id = rel.attr("Id").unwrap_or("");
                    let target = rel.attr("Target").unwrap_or("");
                    let rel_type = rel.attr("Type").unwrap_or("");
                    if rel_type.ends_with("/slide") || target.contains("slide") {
                        let full_target = if target.starts_with('/') {
                            String::from(target.trim_start_matches('/'))
                        } else if target.starts_with("ppt/") {
                            String::from(target)
                        } else {
                            format!("ppt/{}", target.trim_start_matches('/'))
                        };
                        slide_rels.push((String::from(id), full_target));
                    }
                }
            }
        }
    }

    // Determine slide order from <p:sldIdLst>
    let mut ordered_slide_paths = Vec::new();
    if let Some(sld_id_lst) = pres_xml.child("sldIdLst") {
        for sld_id in sld_id_lst.children_named("sldId") {
            if let Some(rid) = sld_id.attr("id") {
                if let Some((_, target)) = slide_rels.iter().find(|(id, _)| id == rid) {
                    ordered_slide_paths.push(target.clone());
                }
            }
        }
    }

    if ordered_slide_paths.is_empty() {
        for (_, target) in &slide_rels {
            ordered_slide_paths.push(target.clone());
        }
        if ordered_slide_paths.is_empty() {
            for entry in archive.entries() {
                if entry.path.starts_with("ppt/slides/slide") && entry.path.ends_with(".xml") {
                    ordered_slide_paths.push(entry.path.clone());
                }
            }
            ordered_slide_paths.sort();
        }
    }

    ordered_slide_paths.truncate(MAX_SLIDES);

    // 3. Parse each slide
    for (slide_idx, slide_path) in ordered_slide_paths.iter().enumerate() {
        let Some(slide_data) = extract_archive_file(&archive, slide_path, &mut total_extracted) else {
            continue;
        };
        let Ok(slide_xml_str) = core::str::from_utf8(&slide_data) else {
            continue;
        };
        let Ok(slide_xml) = XmlNode::parse(slide_xml_str) else {
            continue;
        };

        let mut slide = Slide::new((slide_idx + 1) as u64, SlideLayout::Blank);

        // Slide rels for images and notes
        let slide_rels_path = {
            let mut parts: Vec<&str> = slide_path.split('/').collect();
            if let Some(file) = parts.pop() {
                parts.push("_rels");
                let rels_name = format!("{}.rels", file);
                parts.push(&rels_name);
                parts.join("/")
            } else {
                String::new()
            }
        };

        let mut img_rels = Vec::new();
        let mut notes_slide_path = None;
        if let Some(slide_rels_data) = extract_archive_file(&archive, &slide_rels_path, &mut total_extracted) {
            if let Ok(slide_rels_str) = core::str::from_utf8(&slide_rels_data) {
                if let Ok(slide_rels_xml) = XmlNode::parse(slide_rels_str) {
                    for rel in slide_rels_xml.children_named("Relationship") {
                        let id = rel.attr("Id").unwrap_or("");
                        let target = rel.attr("Target").unwrap_or("");
                        let rel_type = rel.attr("Type").unwrap_or("");
                        if rel_type.ends_with("/image") || target.contains("media/") {
                            let clean_target = resolve_relative_path(slide_path, target);
                            img_rels.push((String::from(id), clean_target));
                        } else if rel_type.ends_with("/notesSlide") || target.contains("notesSlide") {
                            notes_slide_path = Some(resolve_relative_path(slide_path, target));
                        }
                    }
                }
            }
        }

        // Speaker notes
        if let Some(notes_path) = notes_slide_path {
            if let Some(notes_data) = extract_archive_file(&archive, &notes_path, &mut total_extracted) {
                if let Ok(notes_str) = core::str::from_utf8(&notes_data) {
                    if let Ok(notes_xml) = XmlNode::parse(notes_str) {
                        let mut notes_text = String::new();
                        let mut paragraphs = Vec::new();
                        notes_xml.find_all("p", &mut paragraphs);
                        for p in paragraphs {
                            let mut text_nodes = Vec::new();
                            p.find_all("t", &mut text_nodes);
                            let mut p_text = String::new();
                            for t in text_nodes {
                                p_text.push_str(&t.text);
                            }
                            let trimmed = p_text.trim();
                            if !trimmed.is_empty() && trimmed != "<#>" {
                                if !notes_text.is_empty() {
                                    notes_text.push('\n');
                                }
                                notes_text.push_str(trimmed);
                            }
                        }
                        slide.set_speaker_notes(notes_text);
                    }
                }
            }
        }

        // Parse slide transition <p:transition>
        let mut trans_nodes = Vec::new();
        slide_xml.find_all("transition", &mut trans_nodes);
        if let Some(trans) = trans_nodes.first() {
            if trans.child("dissolve").is_some() {
                slide.transition = Transition::Dissolve;
            } else if let Some(push) = trans.child("push") {
                if push.attr("dir") == Some("r") {
                    slide.transition = Transition::PushRight;
                } else {
                    slide.transition = Transition::PushLeft;
                }
            } else if trans.child("wipe").is_some() {
                slide.transition = Transition::WipeRight;
            } else if trans.child("zoom").is_some() {
                slide.transition = Transition::ZoomIn;
            } else if trans.child("fade").is_some() {
                slide.transition = Transition::Fade;
            }
        }

        // Parse shapes and text boxes <p:sp>
        let mut shape_nodes = Vec::new();
        slide_xml.find_all("sp", &mut shape_nodes);
        for sp in shape_nodes {
            parse_pptx_shape(sp, slide_emu_w, slide_emu_h, deck.width(), deck.height(), &mut slide);
        }

        // Parse tables <p:graphicFrame> / <a:tbl>
        let mut gf_nodes = Vec::new();
        slide_xml.find_all("graphicFrame", &mut gf_nodes);
        for gf in gf_nodes {
            parse_pptx_graphic_frame(gf, slide_emu_w, slide_emu_h, deck.width(), deck.height(), &mut slide);
        }

        // Parse pictures <p:pic>
        let mut pic_nodes = Vec::new();
        slide_xml.find_all("pic", &mut pic_nodes);
        for pic in pic_nodes {
            parse_pptx_picture(
                pic,
                &img_rels,
                &archive,
                &mut total_extracted,
                slide_emu_w,
                slide_emu_h,
                deck.width(),
                deck.height(),
                &mut slide,
            );
        }

        deck.slides.push(slide);
    }

    if deck.slides.is_empty() {
        deck.add_slide(SlideLayout::TitleSlide);
    }

    Ok(deck)
}

fn resolve_relative_path(base_file: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = base_file.split('/').collect();
    parts.pop(); // remove filename
    for seg in relative.split('/') {
        if seg == "." {
            continue;
        } else if seg == ".." {
            parts.pop();
        } else {
            parts.push(seg);
        }
    }
    parts.join("/")
}

fn parse_pptx_shape(
    sp: &XmlNode,
    emu_w: f32,
    emu_h: f32,
    deck_w: f32,
    deck_h: f32,
    slide: &mut Slide,
) {
    // 1. Transform / Bounds
    let mut bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
    if let Some(sp_pr) = sp.child("spPr") {
        if let Some(xfrm) = sp_pr.child("xfrm") {
            let x = xfrm
                .child("off")
                .and_then(|o| o.attr("x"))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let y = xfrm
                .child("off")
                .and_then(|o| o.attr("y"))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            let cx = xfrm
                .child("ext")
                .and_then(|e| e.attr("cx"))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1000.0);
            let cy = xfrm
                .child("ext")
                .and_then(|e| e.attr("cy"))
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1000.0);

            let w = emu_to_pixels(cx, emu_w, deck_w).max(0.0);
            let h = emu_to_pixels(cy, emu_h, deck_h).max(0.0);
            bounds = Rect::new(
                emu_to_pixels(x, emu_w, deck_w),
                emu_to_pixels(y, emu_h, deck_h),
                w,
                h,
            );
        }
    }

    let is_title_ph = sp
        .child("nvSpPr")
        .and_then(|n| n.child("nvPr"))
        .and_then(|n| n.child("ph"))
        .map(|ph| {
            let t = ph.attr("type").unwrap_or("title");
            t == "title" || t == "ctrTitle"
        })
        .unwrap_or(false);

    // 2. Shape geometry, fill and stroke
    let mut shape_kind = None;
    let mut fill_color = None;
    let mut stroke = None;

    if let Some(sp_pr) = sp.child("spPr") {
        if let Some(geom) = sp_pr.child("prstGeom") {
            if let Some(prst) = geom.attr("prst") {
                shape_kind = Some(match prst {
                    "roundRect" => ShapeKind::RoundedRect { radius: 16.0 },
                    "ellipse" => ShapeKind::Ellipse,
                    "line" | "straightConnector1" => ShapeKind::Line,
                    "rightArrow" => ShapeKind::Arrow,
                    _ => ShapeKind::Rect,
                });
            }
        }

        // Fill
        if let Some(solid_fill) = sp_pr.child("solidFill") {
            if let Some(srgb) = solid_fill.child("srgbClr") {
                if let Some(val) = srgb.attr("val") {
                    fill_color = parse_hex_color(val);
                }
            }
        }

        // Stroke
        if let Some(ln) = sp_pr.child("ln") {
            let width_emu = ln.attr("w").and_then(|v| v.parse::<f32>().ok()).unwrap_or(12700.0);
            let stroke_w = emu_to_pixels(width_emu, emu_w, deck_w).max(1.0);
            let mut stroke_color = Color::BLACK;
            if let Some(solid_fill) = ln.child("solidFill") {
                if let Some(srgb) = solid_fill.child("srgbClr") {
                    if let Some(val) = srgb.attr("val") {
                        if let Some(c) = parse_hex_color(val) {
                            stroke_color = c;
                        }
                    }
                }
            }
            stroke = Some(StrokeStyle {
                color: stroke_color,
                width: stroke_w,
            });
        }
    }

    // 3. Text Body
    let mut text_box = None;
    if let Some(tx_body) = sp.child("txBody") {
        let mut paragraphs = Vec::new();
        for p in tx_body.children_named("p") {
            let mut paragraph = Paragraph::new();

            // Paragraph properties
            if let Some(p_pr) = p.child("pPr") {
                if let Some(lvl) = p_pr.attr("lvl").and_then(|v| v.parse::<u8>().ok()) {
                    paragraph.bullet_level = lvl.min(8).saturating_add(1);
                }
                if let Some(algn) = p_pr.attr("algn") {
                    paragraph.alignment = match algn {
                        "ctr" => TextAlign::Center,
                        "r" => TextAlign::Right,
                        "just" => TextAlign::Justify,
                        _ => TextAlign::Left,
                    };
                }
            }

            // Runs
            for r in p.children_named("r") {
                let mut run_text = String::new();
                if let Some(t) = r.child("t") {
                    run_text = t.text.clone();
                }

                if !run_text.is_empty() {
                    let mut font_sz = 24.0;
                    let mut bold = false;
                    let mut italic = false;
                    let mut text_color = None;

                    if let Some(r_pr) = r.child("rPr") {
                        if let Some(sz) = r_pr.attr("sz").and_then(|v| v.parse::<f32>().ok()) {
                            let computed = (sz / 100.0) * (deck_h / 720.0);
                            if computed.is_finite() && computed > 0.0 {
                                font_sz = computed.clamp(4.0, 400.0);
                            }
                        }
                        bold = r_pr.attr("b") == Some("1");
                        italic = r_pr.attr("i") == Some("1");
                        if let Some(solid_fill) = r_pr.child("solidFill") {
                            if let Some(srgb) = solid_fill.child("srgbClr") {
                                if let Some(val) = srgb.attr("val") {
                                    text_color = parse_hex_color(val);
                                }
                            }
                        }
                    }

                    let mut run = TextRun::plain(run_text, font_sz);
                    if bold {
                        run = run.bold();
                    }
                    if italic {
                        run = run.italic();
                    }
                    if let Some(c) = text_color {
                        run = run.with_color(c);
                    }
                    paragraph.push_run(run);
                }
            }

            if !paragraph.runs.is_empty() {
                paragraphs.push(paragraph);
            }
        }

        if !paragraphs.is_empty() {
            let mut tb = TextBox::new(bounds);
            tb.paragraphs = paragraphs;
            if slide.title.is_empty() || is_title_ph {
                slide.title = tb.plain_text();
            }
            text_box = Some(tb);
        }
    }

    // Add Shape if geometric fill or stroke is specified
    if let Some(kind) = shape_kind {
        if fill_color.is_some() || stroke.is_some() {
            slide.add_element(SlideElement::Shape(Shape {
                kind,
                bounds,
                fill: fill_color,
                stroke,
            }));
        }
    }

    // Add TextBox
    if let Some(tb) = text_box {
        slide.add_element(SlideElement::TextBox(tb));
    }
}

fn parse_pptx_graphic_frame(
    gf: &XmlNode,
    emu_w: f32,
    emu_h: f32,
    deck_w: f32,
    deck_h: f32,
    slide: &mut Slide,
) {
    let mut bounds = Rect::new(100.0, 100.0, 600.0, 300.0);
    if let Some(xfrm) = gf.child("xfrm") {
        let x = xfrm.child("off").and_then(|o| o.attr("x")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let y = xfrm.child("off").and_then(|o| o.attr("y")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let cx = xfrm.child("ext").and_then(|e| e.attr("cx")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1000.0);
        let cy = xfrm.child("ext").and_then(|e| e.attr("cy")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1000.0);
        bounds = Rect::new(
            emu_to_pixels(x, emu_w, deck_w),
            emu_to_pixels(y, emu_h, deck_h),
            emu_to_pixels(cx, emu_w, deck_w).max(20.0),
            emu_to_pixels(cy, emu_h, deck_h).max(20.0),
        );
    }

    let mut tbl_nodes = Vec::new();
    gf.find_all("tbl", &mut tbl_nodes);
    let Some(tbl) = tbl_nodes.first() else {
        return;
    };

    let mut tr_nodes = Vec::new();
    tbl.find_all("tr", &mut tr_nodes);
    if tr_nodes.is_empty() {
        return;
    }

    let rows_count = tr_nodes.len().min(50);
    let cols_count = tr_nodes[0].children_named("tc").count().min(50).max(1);

    let mut table = TableElement::new(bounds, rows_count, cols_count);

    for (r, tr) in tr_nodes.iter().enumerate().take(rows_count) {
        let mut tc_nodes = Vec::new();
        tr.find_all("tc", &mut tc_nodes);
        for (c, tc) in tc_nodes.iter().enumerate().take(cols_count) {
            let mut text_nodes = Vec::new();
            tc.find_all("t", &mut text_nodes);
            let mut cell_text = String::new();
            for t in text_nodes {
                cell_text.push_str(&t.text);
            }
            let is_header = r == 0;
            let mut cell = TableCell::new(cell_text.trim());
            cell.bold = is_header;
            table.set_cell(r, c, cell);
        }
    }

    slide.add_element(SlideElement::Table(table));
}

fn parse_pptx_picture(
    pic: &XmlNode,
    img_rels: &[(String, String)],
    archive: &sex_archive::Archive<&[u8]>,
    total_extracted: &mut usize,
    emu_w: f32,
    emu_h: f32,
    deck_w: f32,
    deck_h: f32,
    slide: &mut Slide,
) {
    let mut bounds = Rect::new(0.0, 0.0, 300.0, 200.0);
    if let Some(sp_pr) = pic.child("spPr") {
        if let Some(xfrm) = sp_pr.child("xfrm") {
            let x = xfrm.child("off").and_then(|o| o.attr("x")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            let y = xfrm.child("off").and_then(|o| o.attr("y")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            let cx = xfrm.child("ext").and_then(|e| e.attr("cx")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1000.0);
            let cy = xfrm.child("ext").and_then(|e| e.attr("cy")).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1000.0);
            let w = emu_to_pixels(cx, emu_w, deck_w).max(0.0);
            let h = emu_to_pixels(cy, emu_h, deck_h).max(0.0);
            bounds = Rect::new(
                emu_to_pixels(x, emu_w, deck_w),
                emu_to_pixels(y, emu_h, deck_h),
                w,
                h,
            );
        }
    }

    let mut img_data = None;
    if let Some(blip_fill) = pic.child("blipFill") {
        if let Some(blip) = blip_fill.child("blip") {
            if let Some(embed_id) = blip.attr("embed") {
                if let Some((_, img_path)) = img_rels.iter().find(|(id, _)| id == embed_id) {
                    if let Some(bytes) = extract_archive_file(archive, img_path, total_extracted) {
                        img_data = Some(bytes);
                    }
                }
            }
        }
    }

    if let Some(data) = img_data {
        let img_elem = ImageElement::new(bounds, data, FitMode::Contain);
        slide.add_element(SlideElement::Image(img_elem));
    }
}

/// Export deck to an OOXML `.pptx` container archive.
pub fn export_pptx(deck: &Deck) -> Result<Vec<u8>> {
    let mut builder = sex_archive::ArchiveBuilder::new(sex_archive::OutputFormat::Zip);

    // 1. [Content_Types].xml
    let mut ct = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n<Default Extension=\"xml\" ContentType=\"application/xml\"/>\n<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\n<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\n");
    for i in 1..=deck.slide_count() {
        ct.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\n",
            i
        ));
        ct.push_str(&format!(
            "<Override PartName=\"/ppt/notesSlides/notesSlide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>\n",
            i
        ));
    }
    ct.push_str("</Types>");
    let ct_bytes = ct.into_bytes();
    let ct_len = ct_bytes.len() as u64;
    builder.add_file("[Content_Types].xml", ct_len, 0, sex_archive::VecReader::new(ct_bytes)).map_err(|_| Error::Io)?;

    // 2. _rels/.rels
    let root_rels = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\n</Relationships>";
    builder.add_file("_rels/.rels", root_rels.len() as u64, 0, &root_rels[..]).map_err(|_| Error::Io)?;

    // 3. ppt/presentation.xml
    let (emu_w, emu_h) = match deck.aspect_ratio {
        AspectRatio::Ratio16x9 => (12192000, 6858000),
        AspectRatio::Ratio16x10 => (12192000, 7620000),
        AspectRatio::Ratio4x3 => (9144000, 6858000),
        AspectRatio::Custom { width, height } => ((width * 9525.0) as u64, (height * 9525.0) as u64),
    };

    let mut pres = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\n<p:sldSz cx=\"{}\" cy=\"{}\"/>\n<p:sldIdLst>\n",
        emu_w, emu_h
    );
    for i in 1..=deck.slide_count() {
        pres.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>\n", 255 + i, i));
    }
    pres.push_str("</p:sldIdLst>\n</p:presentation>");
    let pres_bytes = pres.into_bytes();
    let pres_len = pres_bytes.len() as u64;
    builder.add_file("ppt/presentation.xml", pres_len, 0, sex_archive::VecReader::new(pres_bytes)).map_err(|_| Error::Io)?;

    // 4. ppt/_rels/presentation.xml.rels
    let mut pres_rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n");
    for i in 1..=deck.slide_count() {
        pres_rels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>\n",
            i, i
        ));
    }
    pres_rels.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>\n</Relationships>",
        deck.slide_count() + 1
    ));
    let pres_rels_bytes = pres_rels.into_bytes();
    let pres_rels_len = pres_rels_bytes.len() as u64;
    builder.add_file("ppt/_rels/presentation.xml.rels", pres_rels_len, 0, sex_archive::VecReader::new(pres_rels_bytes)).map_err(|_| Error::Io)?;

    // 5. ppt/theme/theme1.xml with deck theme colors
    let pal = &deck.theme.palette;
    let theme_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"{}\">\n<a:themeElements><a:clrScheme name=\"Burlesque\">\n<a:dk1><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:dk1>\n<a:lt1><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:lt1>\n<a:accent1><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:accent1>\n<a:accent2><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:accent2>\n</a:clrScheme></a:themeElements></a:theme>",
        deck.theme.name,
        pal.foreground.r(), pal.foreground.g(), pal.foreground.b(),
        pal.background.r(), pal.background.g(), pal.background.b(),
        pal.primary_accent.r(), pal.primary_accent.g(), pal.primary_accent.b(),
        pal.secondary_accent.r(), pal.secondary_accent.g(), pal.secondary_accent.b()
    );
    let theme_bytes = theme_xml.into_bytes();
    let theme_len = theme_bytes.len() as u64;
    builder.add_file("ppt/theme/theme1.xml", theme_len, 0, sex_archive::VecReader::new(theme_bytes)).map_err(|_| Error::Io)?;

    // 6. Slides & notes
    for (i, slide) in deck.slides.iter().enumerate() {
        let slide_num = i + 1;

        // Transition tag
        let trans_tag = match slide.transition {
            Transition::Dissolve => "<p:transition><p:dissolve/></p:transition>\n",
            Transition::PushLeft => "<p:transition><p:push dir=\"l\"/></p:transition>\n",
            Transition::PushRight => "<p:transition><p:push dir=\"r\"/></p:transition>\n",
            Transition::WipeRight => "<p:transition><p:wipe dir=\"r\"/></p:transition>\n",
            Transition::ZoomIn => "<p:transition><p:zoom/></p:transition>\n",
            Transition::Fade => "<p:transition><p:fade/></p:transition>\n",
            _ => "",
        };

        let mut slide_xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\n<p:cSld><p:spTree>\n<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\n<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\n"
        );

        let mut sp_id = 2;

        let title_in_elements = slide.elements.iter().any(|el| {
            if let SlideElement::TextBox(tb) = el {
                tb.plain_text().contains(&slide.title)
            } else {
                false
            }
        });
        if !slide.title.is_empty() && !title_in_elements {
            let off_x = (80.0 * 9525.0) as i64;
            let off_y = (60.0 * 9525.0) as i64;
            let ext_x = (1760.0 * 9525.0) as i64;
            let ext_y = (120.0 * 9525.0) as i64;
            let pal = &deck.theme.palette;
            slide_xml.push_str(&format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{}\" name=\"Title {}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\n<p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm></p:spPr>\n<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr sz=\"4400\" b=\"1\"><a:solidFill><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>\n",
                sp_id, sp_id, off_x, off_y, ext_x, ext_y,
                pal.foreground.r(), pal.foreground.g(), pal.foreground.b(),
                escape_xml(&slide.title)
            ));
            sp_id += 1;
        }

        for elem in &slide.elements {
            match elem {
                SlideElement::TextBox(tb) => {
                    let off_x = (tb.bounds.x * 9525.0) as i64;
                    let off_y = (tb.bounds.y * 9525.0) as i64;
                    let ext_x = (tb.bounds.w * 9525.0) as i64;
                    let ext_y = (tb.bounds.h * 9525.0) as i64;

                    slide_xml.push_str(&format!(
                        "<p:sp><p:nvSpPr><p:cNvPr id=\"{}\" name=\"TextBox {}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\n<p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm></p:spPr>\n<p:txBody><a:bodyPr/><a:lstStyle/>\n",
                        sp_id, sp_id, off_x, off_y, ext_x, ext_y
                    ));
                    sp_id += 1;

                    for p in &tb.paragraphs {
                        let lvl_attr = if p.bullet_level > 0 { format!(" lvl=\"{}\"", p.bullet_level.saturating_sub(1)) } else { String::new() };
                        slide_xml.push_str(&format!("<a:p><a:pPr{}/>", lvl_attr));
                        for r in &p.runs {
                            let bold_attr = if r.bold { " b=\"1\"" } else { "" };
                            let italic_attr = if r.italic { " i=\"1\"" } else { "" };
                            let sz_attr = format!(" sz=\"{}\"", (r.font_size * 100.0) as u32);
                            let clr_tag = if let Some(c) = r.color {
                                format!("<a:solidFill><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:solidFill>", c.r(), c.g(), c.b())
                            } else {
                                String::new()
                            };
                            let escaped = escape_xml(&r.text);
                            slide_xml.push_str(&format!(
                                "<a:r><a:rPr{}{}{}>{}</a:rPr><a:t>{}</a:t></a:r>",
                                sz_attr, bold_attr, italic_attr, clr_tag, escaped
                            ));
                        }
                        slide_xml.push_str("</a:p>\n");
                    }
                    slide_xml.push_str("</p:txBody></p:sp>\n");
                }
                SlideElement::Shape(s) => {
                    let off_x = (s.bounds.x * 9525.0) as i64;
                    let off_y = (s.bounds.y * 9525.0) as i64;
                    let ext_x = (s.bounds.w * 9525.0) as i64;
                    let ext_y = (s.bounds.h * 9525.0) as i64;

                    let prst = match s.kind {
                        ShapeKind::RoundedRect { .. } => "roundRect",
                        ShapeKind::Ellipse => "ellipse",
                        ShapeKind::Line => "line",
                        ShapeKind::Arrow => "rightArrow",
                        _ => "rect",
                    };

                    let fill_tag = if let Some(fill) = s.fill {
                        format!("<a:solidFill><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:solidFill>", fill.r(), fill.g(), fill.b())
                    } else {
                        String::new()
                    };

                    let stroke_tag = if let Some(st) = s.stroke {
                        format!(
                            "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{:02X}{:02X}{:02X}\"/></a:solidFill></a:ln>",
                            (st.width * 12700.0) as u64,
                            st.color.r(), st.color.g(), st.color.b()
                        )
                    } else {
                        String::new()
                    };

                    slide_xml.push_str(&format!(
                        "<p:sp><p:nvSpPr><p:cNvPr id=\"{}\" name=\"Shape {}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>\n<p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>{}{}</p:spPr>\n</p:sp>\n",
                        sp_id, sp_id, off_x, off_y, ext_x, ext_y, prst, fill_tag, stroke_tag
                    ));
                    sp_id += 1;
                }
                SlideElement::Table(t) => {
                    let off_x = (t.bounds.x * 9525.0) as i64;
                    let off_y = (t.bounds.y * 9525.0) as i64;
                    let ext_x = (t.bounds.w * 9525.0) as i64;
                    let ext_y = (t.bounds.h * 9525.0) as i64;

                    slide_xml.push_str(&format!(
                        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{}\" name=\"Table {}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>\n<p:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm>\n<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\">\n<a:tbl><a:tblPr/><a:tblGrid>",
                        sp_id, sp_id, off_x, off_y, ext_x, ext_y
                    ));
                    sp_id += 1;

                    let col_w = (ext_x / t.cols.max(1) as i64).max(1000);
                    for _ in 0..t.cols {
                        slide_xml.push_str(&format!("<a:gridCol w=\"{}\"/>", col_w));
                    }
                    slide_xml.push_str("</a:tblGrid>\n");

                    for row in &t.cells {
                        slide_xml.push_str("<a:tr h=\"370840\">");
                        for cell in row {
                            let esc = escape_xml(&cell.text);
                            let bold_attr = if cell.bold { " b=\"1\"" } else { "" };
                            slide_xml.push_str(&format!(
                                "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr{}><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p></a:txBody></a:tc>",
                                bold_attr, esc
                            ));
                        }
                        slide_xml.push_str("</a:tr>\n");
                    }
                    slide_xml.push_str("</a:tbl></a:graphicData></a:graphic></p:graphicFrame>\n");
                }
                _ => {}
            }
        }

        slide_xml.push_str("</p:spTree></p:cSld>\n");
        slide_xml.push_str(trans_tag);
        slide_xml.push_str("</p:sld>");

        let slide_bytes = slide_xml.into_bytes();
        let slide_len = slide_bytes.len() as u64;
        let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
        builder.add_file(&slide_path, slide_len, 0, sex_archive::VecReader::new(slide_bytes)).map_err(|_| Error::Io)?;

        // Slide rels -> notesSlide
        let notes_rel = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide\" Target=\"../notesSlides/notesSlide{}.xml\"/>\n</Relationships>",
            slide_num
        );
        let notes_rel_bytes = notes_rel.into_bytes();
        let notes_rel_len = notes_rel_bytes.len() as u64;
        let notes_rel_path = format!("ppt/slides/_rels/slide{}.xml.rels", slide_num);
        builder.add_file(&notes_rel_path, notes_rel_len, 0, sex_archive::VecReader::new(notes_rel_bytes)).map_err(|_| Error::Io)?;

        // notesSlide{i}.xml
        let note_text = escape_xml(slide.notes());
        let notes_slide_xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:notes xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\n<p:cSld><p:spTree>\n<p:sp><p:txBody><a:bodyPr/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>\n</p:spTree></p:cSld>\n</p:notes>",
            note_text
        );
        let notes_slide_bytes = notes_slide_xml.into_bytes();
        let notes_slide_len = notes_slide_bytes.len() as u64;
        let notes_slide_path = format!("ppt/notesSlides/notesSlide{}.xml", slide_num);
        builder.add_file(&notes_slide_path, notes_slide_len, 0, sex_archive::VecReader::new(notes_slide_bytes)).map_err(|_| Error::Io)?;
    }

    let mut out = Vec::new();
    builder.write(&mut out, &mut sex_archive::no_progress).map_err(|_| Error::Io)?;
    Ok(out)
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
