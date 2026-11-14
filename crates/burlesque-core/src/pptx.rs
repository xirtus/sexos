// SPDX-License-Identifier: GPL-3.0-only
//! OOXML `.pptx` presentation importer.

use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::{Color, Rect};
use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{
    FitMode, ImageElement, Paragraph, Shape, ShapeKind, SlideElement, StrokeStyle,
    TextAlign, TextBox, TextRun,
};
use crate::layout::SlideLayout;
use crate::xml_utils::XmlNode;
use crate::{Error, Result};

/// Extract entry bytes from a zip archive by path (normalizing separators).
pub fn extract_archive_file(archive: &sex_archive::Archive<&[u8]>, target_path: &str) -> Option<Vec<u8>> {
    let clean = target_path.replace('\\', "/");
    let trimmed = clean.trim_start_matches('/');
    if let Some(idx) = archive.find(trimmed) {
        let mut out = Vec::new();
        if archive.extract(idx, &mut out, &mut sex_archive::no_progress).is_ok() {
            return Some(out);
        }
    }
    // Case-insensitive fallback
    for (i, entry) in archive.entries().iter().enumerate() {
        if entry.path.eq_ignore_ascii_case(trimmed) {
            let mut out = Vec::new();
            if archive.extract(i, &mut out, &mut sex_archive::no_progress).is_ok() {
                return Some(out);
            }
        }
    }
    None
}

/// Parse a hex color string like "FF0077" into Color.
pub fn parse_hex_color(hex: &str) -> Option<Color> {
    let clean = hex.trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        Some(Color::rgb(r, g, b))
    } else {
        None
    }
}

/// Convert EMU coordinates to deck logical points.
fn emu_to_pixels(emu: f32, emu_total: f32, logical_total: f32) -> f32 {
    if emu_total > 0.0 {
        (emu / emu_total) * logical_total
    } else {
        emu / 9525.0 // fallback 1 pt ~ 12700 emu, 1 px ~ 9525 emu at 96 dpi
    }
}

/// Import PPTX presentation from raw container bytes.
pub fn import_pptx(bytes: &[u8]) -> Result<Deck> {
    let archive = sex_archive::Archive::open(bytes).map_err(|_| Error::InvalidFormat)?;

    // 1. Read ppt/presentation.xml
    let pres_bytes = extract_archive_file(&archive, "ppt/presentation.xml").ok_or(Error::MalformedDocument)?;
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

    // 2. Read presentation relationships to map rId -> slide path
    let mut slide_rels = Vec::new();
    if let Some(rels_bytes) = extract_archive_file(&archive, "ppt/_rels/presentation.xml.rels") {
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
                            alloc::format!("ppt/{}", target.trim_start_matches('/'))
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
        // Fallback: use all slide relationships sorted or scan slides/
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

    // 3. Parse each slide
    for (slide_idx, slide_path) in ordered_slide_paths.iter().enumerate() {
        let Some(slide_data) = extract_archive_file(&archive, slide_path) else {
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
                let rels_name = alloc::format!("{}.rels", file);
                parts.push(&rels_name);
                parts.join("/")
            } else {
                String::new()
            }
        };

        let mut img_rels = Vec::new();
        let mut notes_slide_path = None;
        if let Some(slide_rels_data) = extract_archive_file(&archive, &slide_rels_path) {
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
            if let Some(notes_data) = extract_archive_file(&archive, &notes_path) {
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
                            // Skip slide number placeholders
                            if !trimmed.is_empty() && trimmed != "<#>" {
                                if !notes_text.is_empty() {
                                    notes_text.push('\n');
                                }
                                notes_text.push_str(trimmed);
                            }
                        }
                        slide.speaker_notes = notes_text;
                    }
                }
            }
        }

        // Parse shapes and pictures from slide XML
        let mut shape_nodes = Vec::new();
        slide_xml.find_all("sp", &mut shape_nodes);
        for sp in shape_nodes {
            parse_pptx_shape(sp, slide_emu_w, slide_emu_h, deck.width(), deck.height(), &mut slide);
        }

        let mut pic_nodes = Vec::new();
        slide_xml.find_all("pic", &mut pic_nodes);
        for pic in pic_nodes {
            parse_pptx_picture(
                pic,
                &img_rels,
                &archive,
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

            bounds = Rect::new(
                emu_to_pixels(x, emu_w, deck_w),
                emu_to_pixels(y, emu_h, deck_h),
                emu_to_pixels(cx, emu_w, deck_w),
                emu_to_pixels(cy, emu_h, deck_h),
            );
        }
    }

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
                    paragraph.bullet_level = lvl + 1; // 1-based bullet
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
                            // sz is in 1/100 pt
                            font_sz = (sz / 100.0) * (deck_h / 720.0);
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
            if slide.title.is_empty() {
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

fn parse_pptx_picture(
    pic: &XmlNode,
    img_rels: &[(String, String)],
    archive: &sex_archive::Archive<&[u8]>,
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
            bounds = Rect::new(
                emu_to_pixels(x, emu_w, deck_w),
                emu_to_pixels(y, emu_h, deck_h),
                emu_to_pixels(cx, emu_w, deck_w),
                emu_to_pixels(cy, emu_h, deck_h),
            );
        }
    }

    let mut img_data = None;
    if let Some(blip_fill) = pic.child("blipFill") {
        if let Some(blip) = blip_fill.child("blip") {
            if let Some(embed_id) = blip.attr("embed") {
                if let Some((_, img_path)) = img_rels.iter().find(|(id, _)| id == embed_id) {
                    if let Some(bytes) = extract_archive_file(archive, img_path) {
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
