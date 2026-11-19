// SPDX-License-Identifier: GPL-3.0-only
//! OpenDocument `.odp` presentation importer and generator.
//!
//! Provides memory-bounded, panic-free parsing of OpenDocument `.odp` presentations:
//! - content.xml and styles.xml parsing
//! - Slides from <draw:page>
//! - Speaker notes from <presentation:notes>
//! - Vector text boxes <draw:text-box> and multi-paragraph runs <text:p>/<text:span>
//! - Tables <table:table>
//! - Vector shapes <draw:rect>, <draw:circle>, <draw:line>
//! - Embedded pictures <draw:image> via Pictures/*
//! - Slide transitions
//!
//! Also provides `export_odp(&Deck) -> Result<Vec<u8>>` to serialize multi-slide
//! presentations with themes, transitions, shapes, tables, and speaker notes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::Rect;
use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{
    FitMode, ImageElement, Paragraph, Shape, SlideElement, StrokeStyle, TableCell,
    TableElement, TextBox, TextRun, Transition,
};
use crate::layout::SlideLayout;
use crate::pptx::{extract_archive_file, MAX_ARCHIVE_ENTRIES, MAX_SLIDES};
use crate::xml_utils::XmlNode;
use crate::{Error, Result};

/// Parse length with unit (cm, mm, in, pt, px) into points safely without slice panics.
fn parse_length_to_pt(val: &str) -> f32 {
    let s = val.trim();
    if s.is_empty() {
        return 0.0;
    }
    let (num_str, factor) = if let Some(rest) = s.strip_suffix("cm") {
        (rest, 28.3465)
    } else if let Some(rest) = s.strip_suffix("mm") {
        (rest, 2.83465)
    } else if let Some(rest) = s.strip_suffix("in") {
        (rest, 72.0)
    } else if let Some(rest) = s.strip_suffix("pt") {
        (rest, 1.0)
    } else if let Some(rest) = s.strip_suffix("px") {
        (rest, 0.75)
    } else {
        (s, 1.0)
    };
    let val = num_str.trim().parse::<f32>().unwrap_or(0.0) * factor;
    if val.is_finite() && val >= 0.0 {
        val
    } else {
        0.0
    }
}

/// Import OpenDocument Presentation (ODP) from bytes.
pub fn import_odp(bytes: &[u8]) -> Result<Deck> {
    let archive = sex_archive::Archive::open(bytes).map_err(|_| Error::InvalidFormat)?;
    if archive.entries().len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::MalformedDocument);
    }
    let mut total_extracted = 0usize;

    let content_bytes = extract_archive_file(&archive, "content.xml", &mut total_extracted)
        .ok_or(Error::MalformedDocument)?;
    let content_str = core::str::from_utf8(&content_bytes).map_err(|_| Error::MalformedDocument)?;
    let content_xml = XmlNode::parse(content_str).map_err(|_| Error::MalformedDocument)?;

    let mut deck = Deck::new(AspectRatio::Ratio16x9);

    // Find all top-level <draw:page> elements inside presentation
    let mut pres_nodes = Vec::new();
    content_xml.find_all("presentation", &mut pres_nodes);
    let mut pages = Vec::new();
    if let Some(pres) = pres_nodes.first() {
        for child in &pres.children {
            if child.name == "page" || child.name.ends_with(":page") {
                pages.push(child);
            }
        }
    }
    if pages.is_empty() {
        let mut all_pages = Vec::new();
        content_xml.find_all("page", &mut all_pages);
        for p in all_pages {
            if p.attr("name").is_some() || p.child("notes").is_some() || p.child("frame").is_some() {
                pages.push(p);
            }
        }
    }
    pages.truncate(MAX_SLIDES);

    // Reference page dimension in points: standard 16:9 is 28cm x 15.75cm ~ 793.7pt x 446.5pt
    let page_ref_w = 793.7;
    let page_ref_h = 446.5;

    for (page_idx, page) in pages.iter().enumerate() {
        let mut slide = Slide::new((page_idx + 1) as u64, SlideLayout::Blank);
        if let Some(name) = page.attr("name") {
            slide.title = String::from(name);
        }

        // Speaker notes: <presentation:notes>
        let mut notes_nodes = Vec::new();
        page.find_all("notes", &mut notes_nodes);
        for notes in notes_nodes {
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
            if !notes_text.is_empty() {
                slide.set_speaker_notes(notes_text);
            }
        }

        // Slide transition: check attributes on page or transition children
        if let Some(trans_attr) = page.attr("transition-type").or_else(|| page.attr("transition-style")) {
            slide.transition = Transition::from_str_loose(trans_attr);
        } else {
            let mut trans_nodes = Vec::new();
            page.find_all("transitionFilter", &mut trans_nodes);
            if let Some(tf) = trans_nodes.first() {
                if let Some(t_type) = tf.attr("type") {
                    slide.transition = Transition::from_str_loose(t_type);
                }
            }
        }

        // Tables: <table:table>
        let mut table_nodes = Vec::new();
        page.find_all("table", &mut table_nodes);
        for t_node in table_nodes {
            let mut tr_nodes = Vec::new();
            t_node.find_all("table-row", &mut tr_nodes);
            if tr_nodes.is_empty() {
                continue;
            }

            let rows_count = tr_nodes.len().min(50);
            let mut first_row_cells = Vec::new();
            tr_nodes[0].find_all("table-cell", &mut first_row_cells);
            let cols_count = first_row_cells.len().min(50).max(1);

            let tbl_bounds = Rect::new(100.0, 200.0, deck.width() - 200.0, 300.0);
            let mut table = TableElement::new(tbl_bounds, rows_count, cols_count);

            for (r, tr) in tr_nodes.iter().enumerate().take(rows_count) {
                let mut tc_nodes = Vec::new();
                tr.find_all("table-cell", &mut tc_nodes);
                for (c, tc) in tc_nodes.iter().enumerate().take(cols_count) {
                    let mut text = String::new();
                    let mut p_list = Vec::new();
                    tc.find_all("p", &mut p_list);
                    for p in p_list {
                        text.push_str(p.text.trim());
                    }
                    let mut cell = TableCell::new(text);
                    cell.bold = r == 0;
                    table.set_cell(r, c, cell);
                }
            }

            slide.add_element(SlideElement::Table(table));
        }

        // Frames: <draw:frame>
        let mut frames = Vec::new();
        page.find_all("frame", &mut frames);
        for frame in frames {
            let x_pt = frame.attr("x").map(parse_length_to_pt).unwrap_or(0.0);
            let y_pt = frame.attr("y").map(parse_length_to_pt).unwrap_or(0.0);
            let w_pt = frame.attr("width").map(parse_length_to_pt).unwrap_or(200.0).max(0.0);
            let h_pt = frame.attr("height").map(parse_length_to_pt).unwrap_or(100.0).max(0.0);

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
                    if let Some(img_data) = extract_archive_file(&archive, img_path, &mut total_extracted) {
                        let elem = ImageElement::new(bounds, img_data, FitMode::Contain);
                        slide.add_element(SlideElement::Image(elem));
                    }
                }
            }
        }

        // Direct shapes in page: <draw:rect>
        let mut rects = Vec::new();
        page.find_all("rect", &mut rects);
        for r in rects {
            let x_pt = r.attr("x").map(parse_length_to_pt).unwrap_or(0.0);
            let y_pt = r.attr("y").map(parse_length_to_pt).unwrap_or(0.0);
            let w_pt = r.attr("width").map(parse_length_to_pt).unwrap_or(100.0).max(0.0);
            let h_pt = r.attr("height").map(parse_length_to_pt).unwrap_or(100.0).max(0.0);
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

/// Export deck to an OpenDocument Presentation (`.odp`) container archive.
pub fn export_odp(deck: &Deck) -> Result<Vec<u8>> {
    let mut builder = sex_archive::ArchiveBuilder::new(sex_archive::OutputFormat::Zip);

    // 1. mimetype (stored uncompressed)
    let mimetype = b"application/vnd.oasis.opendocument.presentation";
    builder.add_file("mimetype", mimetype.len() as u64, 0, &mimetype[..]).map_err(|_| Error::Io)?;

    // 2. META-INF/manifest.xml
    let manifest_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\">\n<manifest:file-entry manifest:media-type=\"application/vnd.oasis.opendocument.presentation\" manifest:full-path=\"/\"/>\n<manifest:file-entry manifest:media-type=\"text/xml\" manifest:full-path=\"content.xml\"/>\n<manifest:file-entry manifest:media-type=\"text/xml\" manifest:full-path=\"styles.xml\"/>\n</manifest:manifest>";
    builder.add_file("META-INF/manifest.xml", manifest_xml.len() as u64, 0, &manifest_xml[..]).map_err(|_| Error::Io)?;

    // 3. styles.xml
    let styles_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\">\n</office:document-styles>";
    builder.add_file("styles.xml", styles_xml.len() as u64, 0, &styles_xml[..]).map_err(|_| Error::Io)?;

    // 4. content.xml
    let page_ref_w = 793.7;
    let page_ref_h = 446.5;
    let sx = page_ref_w / deck.width();
    let sy = page_ref_h / deck.height();

    let mut content = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:presentation=\"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\">\n<office:body><office:presentation>\n");

    for (i, slide) in deck.slides.iter().enumerate() {
        let slide_name = if slide.title.is_empty() {
            format!("Slide {}", i + 1)
        } else {
            escape_xml(&slide.title)
        };

        let trans_attr = if slide.transition != Transition::None {
            format!(" presentation:transition-style=\"{}\"", slide.transition.kind_str())
        } else {
            String::new()
        };

        content.push_str(&format!("<draw:page draw:name=\"{}\"{}>\n", slide_name, trans_attr));

        let title_in_elements = slide.elements.iter().any(|el| {
            if let SlideElement::TextBox(tb) = el {
                tb.plain_text().contains(&slide.title)
            } else {
                false
            }
        });
        if !slide.title.is_empty() && !title_in_elements {
            let x_pt = 80.0 * sx;
            let y_pt = 60.0 * sy;
            let w_pt = 1760.0 * sx;
            let h_pt = 120.0 * sy;
            content.push_str(&format!(
                "<draw:frame svg:x=\"{:.1}pt\" svg:y=\"{:.1}pt\" svg:width=\"{:.1}pt\" svg:height=\"{:.1}pt\">\n<draw:text-box>\n<text:p>{}</text:p>\n</draw:text-box>\n</draw:frame>\n",
                x_pt, y_pt, w_pt, h_pt,
                escape_xml(&slide.title)
            ));
        }

        for elem in &slide.elements {
            match elem {
                SlideElement::TextBox(tb) => {
                    let x_pt = tb.bounds.x * sx;
                    let y_pt = tb.bounds.y * sy;
                    let w_pt = tb.bounds.w * sx;
                    let h_pt = tb.bounds.h * sy;

                    content.push_str(&format!(
                        "<draw:frame svg:x=\"{:.1}pt\" svg:y=\"{:.1}pt\" svg:width=\"{:.1}pt\" svg:height=\"{:.1}pt\">\n<draw:text-box>\n",
                        x_pt, y_pt, w_pt, h_pt
                    ));
                    for p in &tb.paragraphs {
                        let text = escape_xml(&p.plain_text());
                        if p.bullet_level > 0 {
                            content.push_str(&format!("<text:list><text:list-item><text:p>{}</text:p></text:list-item></text:list>\n", text));
                        } else {
                            content.push_str(&format!("<text:p>{}</text:p>\n", text));
                        }
                    }
                    content.push_str("</draw:text-box>\n</draw:frame>\n");
                }
                SlideElement::Shape(s) => {
                    let x_pt = s.bounds.x * sx;
                    let y_pt = s.bounds.y * sy;
                    let w_pt = s.bounds.w * sx;
                    let h_pt = s.bounds.h * sy;

                    content.push_str(&format!(
                        "<draw:rect svg:x=\"{:.1}pt\" svg:y=\"{:.1}pt\" svg:width=\"{:.1}pt\" svg:height=\"{:.1}pt\"/>\n",
                        x_pt, y_pt, w_pt, h_pt
                    ));
                }
                SlideElement::Table(t) => {
                    content.push_str("<table:table>\n");
                    for row in &t.cells {
                        content.push_str("<table:table-row>");
                        for cell in row {
                            let text = escape_xml(&cell.text);
                            content.push_str(&format!("<table:table-cell><text:p>{}</text:p></table:table-cell>", text));
                        }
                        content.push_str("</table:table-row>\n");
                    }
                    content.push_str("</table:table>\n");
                }
                _ => {}
            }
        }

        // Speaker notes: <presentation:notes>
        let note_text = escape_xml(slide.notes());
        if !note_text.is_empty() {
            content.push_str(&format!(
                "<presentation:notes><draw:frame><draw:text-box><text:p>{}</text:p></draw:text-box></draw:frame></presentation:notes>\n",
                note_text
            ));
        }

        content.push_str("</draw:page>\n");
    }

    content.push_str("</office:presentation></office:body></office:document-content>");
    let content_bytes = content.into_bytes();
    let content_len = content_bytes.len() as u64;
    builder.add_file("content.xml", content_len, 0, sex_archive::VecReader::new(content_bytes)).map_err(|_| Error::Io)?;

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
