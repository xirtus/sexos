// SPDX-License-Identifier: GPL-3.0-only
//! OpenDocument `.odp` presentation importer.

use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::Rect;
use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{
    FitMode, ImageElement, Paragraph, Shape, SlideElement, StrokeStyle,
    TextBox, TextRun,
};
use crate::layout::SlideLayout;
use crate::pptx::extract_archive_file;
use crate::xml_utils::XmlNode;
use crate::{Error, Result};

/// Parse length with unit (cm, mm, in, pt, px) into points.
fn parse_length_to_pt(val: &str) -> f32 {
    let s = val.trim();
    if s.is_empty() {
        return 0.0;
    }
    if s.ends_with("cm") {
        s[..s.len() - 2].trim().parse::<f32>().unwrap_or(0.0) * 28.3465
    } else if s.ends_with("mm") {
        s[..s.len() - 2].trim().parse::<f32>().unwrap_or(0.0) * 2.83465
    } else if s.ends_with("in") {
        s[..s.len() - 2].trim().parse::<f32>().unwrap_or(0.0) * 72.0
    } else if s.ends_with("pt") {
        s[..s.len() - 2].trim().parse::<f32>().unwrap_or(0.0)
    } else if s.ends_with("px") {
        s[..s.len() - 2].trim().parse::<f32>().unwrap_or(0.0) * 0.75
    } else {
        s.parse::<f32>().unwrap_or(0.0)
    }
}

/// Import OpenDocument Presentation (ODP) from bytes.
pub fn import_odp(bytes: &[u8]) -> Result<Deck> {
    let archive = sex_archive::Archive::open(bytes).map_err(|_| Error::InvalidFormat)?;

    let content_bytes = extract_archive_file(&archive, "content.xml").ok_or(Error::MalformedDocument)?;
    let content_str = core::str::from_utf8(&content_bytes).map_err(|_| Error::MalformedDocument)?;
    let content_xml = XmlNode::parse(content_str).map_err(|_| Error::MalformedDocument)?;

    let mut deck = Deck::new(AspectRatio::Ratio16x9);

    // Find all <draw:page> elements
    let mut pages = Vec::new();
    content_xml.find_all("page", &mut pages);

    // Reference page dimension in points: standard 16:9 is 28cm x 15.75cm ~ 793.7pt x 446.5pt
    let page_ref_w = 793.7;
    let page_ref_h = 446.5;

    for (page_idx, page) in pages.iter().enumerate() {
        let mut slide = Slide::new((page_idx + 1) as u64, SlideLayout::Blank);
        if let Some(name) = page.attr("name") {
            slide.title = String::from(name);
        }

        // Speaker notes
        if let Some(notes) = page.child("notes") {
            let mut notes_text = String::new();
            let mut p_nodes = Vec::new();
            notes.find_all("p", &mut p_nodes);
            for p in p_nodes {
                let mut text_nodes = Vec::new();
                p.find_all("span", &mut text_nodes);
                let mut p_str = String::new();
                if text_nodes.is_empty() {
                    p_str = p.text.clone();
                } else {
                    for s in text_nodes {
                        p_str.push_str(&s.text);
                    }
                }
                let trimmed = p_str.trim();
                if !trimmed.is_empty() {
                    if !notes_text.is_empty() {
                        notes_text.push('\n');
                    }
                    notes_text.push_str(trimmed);
                }
            }
            slide.speaker_notes = notes_text;
        }

        // Frames
        let mut frames = Vec::new();
        page.find_all("frame", &mut frames);
        for frame in frames {
            let x_pt = frame.attr("x").map(parse_length_to_pt).unwrap_or(0.0);
            let y_pt = frame.attr("y").map(parse_length_to_pt).unwrap_or(0.0);
            let w_pt = frame.attr("width").map(parse_length_to_pt).unwrap_or(200.0);
            let h_pt = frame.attr("height").map(parse_length_to_pt).unwrap_or(100.0);

            let scale_x = deck.width() / page_ref_w;
            let scale_y = deck.height() / page_ref_h;
            let bounds = Rect::new(x_pt * scale_x, y_pt * scale_y, w_pt * scale_x, h_pt * scale_y);

            // Text box
            if let Some(text_box_node) = frame.child("text-box") {
                let mut paragraphs = Vec::new();
                for p in text_box_node.children_named("p") {
                    let mut paragraph = Paragraph::new();
                    let text = p.text.trim();
                    if !text.is_empty() {
                        paragraph.push_run(TextRun::plain(text, 26.0));
                    }
                    for span in p.children_named("span") {
                        let span_text = span.text.trim();
                        if !span_text.is_empty() {
                            paragraph.push_run(TextRun::plain(span_text, 26.0));
                        }
                    }
                    if !paragraph.runs.is_empty() {
                        paragraphs.push(paragraph);
                    }
                }
                // Bullet lists
                for list in text_box_node.children_named("list") {
                    for item in list.children_named("list-item") {
                        for p in item.children_named("p") {
                            let text = p.text.trim();
                            if !text.is_empty() {
                                paragraphs.push(Paragraph::bullet(text, 26.0, 1));
                            }
                        }
                    }
                }

                if !paragraphs.is_empty() {
                    let mut tb = TextBox::new(bounds);
                    tb.paragraphs = paragraphs;
                    if slide.title.is_empty() {
                        slide.title = tb.plain_text();
                    }
                    slide.add_element(SlideElement::TextBox(tb));
                }
            }

            // Image
            if let Some(img_node) = frame.child("image") {
                if let Some(href) = img_node.attr("href") {
                    let img_path = href.trim_start_matches("./");
                    if let Some(img_data) = extract_archive_file(&archive, img_path) {
                        let elem = ImageElement::new(bounds, img_data, FitMode::Contain);
                        slide.add_element(SlideElement::Image(elem));
                    }
                }
            }
        }

        // Direct shapes in page
        let mut rects = Vec::new();
        page.find_all("rect", &mut rects);
        for r in rects {
            let x_pt = r.attr("x").map(parse_length_to_pt).unwrap_or(0.0);
            let y_pt = r.attr("y").map(parse_length_to_pt).unwrap_or(0.0);
            let w_pt = r.attr("width").map(parse_length_to_pt).unwrap_or(100.0);
            let h_pt = r.attr("height").map(parse_length_to_pt).unwrap_or(100.0);
            let scale_x = deck.width() / page_ref_w;
            let scale_y = deck.height() / page_ref_h;
            let bounds = Rect::new(x_pt * scale_x, y_pt * scale_y, w_pt * scale_x, h_pt * scale_y);

            slide.add_element(SlideElement::Shape(Shape::rect(
                bounds,
                Some(deck.theme.palette.primary_accent),
                Some(StrokeStyle { color: deck.theme.palette.border, width: 2.0 }),
            )));
        }

        deck.slides.push(slide);
    }

    if deck.slides.is_empty() {
        deck.add_slide(SlideLayout::TitleSlide);
    }

    Ok(deck)
}
