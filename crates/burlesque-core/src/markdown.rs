// SPDX-License-Identifier: GPL-3.0-only
//! Markdown deck importer and exporter with `---` slide separators and speaker notes.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sex_canvas::Rect;
use crate::animation::{BuildEffect, BuildStep};
use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{CodeBlock, Paragraph, SlideElement, TextBox, TextRun, Transition};
use crate::layout::SlideLayout;
use crate::Result;

/// Import a presentation deck from Markdown format.
pub fn import_markdown(md: &str) -> Result<Deck> {
    let mut deck = Deck::new(AspectRatio::Ratio16x9);

    // Split into slide text chunks by `---`
    let mut slide_chunks = Vec::new();
    let mut current_chunk = String::new();

    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "***" {
            slide_chunks.push(current_chunk);
            current_chunk = String::new();
        } else {
            current_chunk.push_str(line);
            current_chunk.push('\n');
        }
    }
    if !current_chunk.trim().is_empty() || slide_chunks.is_empty() {
        slide_chunks.push(current_chunk);
    }

    for (idx, chunk) in slide_chunks.iter().enumerate() {
        let slide = parse_slide_markdown(chunk, (idx + 1) as u64, deck.width(), deck.height());
        deck.slides.push(slide);
    }

    if deck.slides.is_empty() {
        deck.add_slide(SlideLayout::TitleSlide);
    }

    Ok(deck)
}

fn parse_slide_markdown(content: &str, slide_id: u64, width: f32, height: f32) -> Slide {
    let mut slide = Slide::new(slide_id, SlideLayout::TitleAndBody);
    let mut lines = content.lines().peekable();

    let mut title = String::new();
    let mut paragraphs = Vec::new();
    let mut notes = String::new();

    let mut in_code_block = false;
    let mut code_lang = None;
    let mut code_text = String::new();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        // Comments: <!-- transition: ... -->, <!-- build: ... -->, <!-- timing: ... -->, or <!-- notes: ... -->
        if trimmed.starts_with("<!--") {
            let mut note_buf = String::new();
            let inner_content = if trimmed.contains("-->") {
                trimmed
                    .trim_start_matches("<!--")
                    .trim_end_matches("-->")
                    .trim()
                    .to_string()
            } else {
                let mut buf = String::new();
                buf.push_str(trimmed.trim_start_matches("<!--").trim());
                buf.push('\n');
                while let Some(nline) = lines.next() {
                    let ntrim = nline.trim();
                    if ntrim.contains("-->") {
                        buf.push_str(ntrim.trim_end_matches("-->").trim());
                        break;
                    } else {
                        buf.push_str(ntrim);
                        buf.push('\n');
                    }
                }
                buf
            };

            let clean_inner = inner_content.trim();
            if let Some(trans) = clean_inner.strip_prefix("transition:").or_else(|| clean_inner.strip_prefix("transition ")) {
                slide.transition = Transition::from_str_loose(trans.trim());
                continue;
            } else if let Some(timing) = clean_inner.strip_prefix("timing:").or_else(|| clean_inner.strip_prefix("timing ")) {
                if let Ok(ms) = timing.trim().parse::<u32>() {
                    slide.rehearsed_dwell_ms = Some(ms);
                }
                continue;
            } else if let Some(build_str) = clean_inner.strip_prefix("build:").or_else(|| clean_inner.strip_prefix("build ")) {
                let parts: Vec<&str> = build_str.split_whitespace().collect();
                if !parts.is_empty() {
                    let elem_idx = parts[0].parse::<usize>().unwrap_or(0);
                    let effect = if parts.len() > 1 {
                        BuildEffect::from_str_loose(parts[1])
                    } else {
                        BuildEffect::DissolveIn
                    };
                    let duration = if parts.len() > 2 {
                        parts[2].parse::<u32>().unwrap_or(400)
                    } else {
                        400
                    };
                    slide.add_build(BuildStep::new(elem_idx, effect).with_duration(duration));
                }
                continue;
            } else {
                let clean = strip_notes_prefix(clean_inner);
                note_buf.push_str(clean);
                if !notes.is_empty() {
                    notes.push('\n');
                }
                notes.push_str(note_buf.trim());
                continue;
            }
        }

        // Code block toggle
        if trimmed.starts_with("```") {
            if in_code_block {
                // Finish code block
                let code_elem = CodeBlock::new(
                    Rect::new(width * 0.1, height * 0.35, width * 0.8, height * 0.5),
                    code_text.clone(),
                );
                let mut cb = code_elem;
                if let Some(lang) = code_lang.take() {
                    cb = cb.with_language(lang);
                }
                slide.add_element(SlideElement::Code(cb));
                code_text.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
                let lang_str = trimmed.trim_start_matches("```").trim();
                if !lang_str.is_empty() {
                    code_lang = Some(String::from(lang_str));
                }
            }
            continue;
        }

        if in_code_block {
            code_text.push_str(line);
            code_text.push('\n');
            continue;
        }

        // Titles
        if let Some(t) = trimmed.strip_prefix("# ") {
            title = String::from(t);
            continue;
        } else if let Some(subtitle) = trimmed.strip_prefix("## ") {
            paragraphs.push(Paragraph::from_text(subtitle, 34.0));
            continue;
        }

        // Bullet points
        let leading_spaces = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        let level = (leading_spaces / 2 + 1).min(5) as u8;

        if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            paragraphs.push(Paragraph::bullet(item, 26.0, level));
        } else if !trimmed.is_empty() {
            paragraphs.push(Paragraph::from_text(trimmed, 26.0));
        }
    }

    slide.title = title.clone();
    slide.speaker_notes = notes;

    // Layout elements
    let margin_x = width * 0.08;
    let margin_y = height * 0.08;
    let content_w = width - margin_x * 2.0;

    if !title.is_empty() {
        let title_h = height * 0.14;
        let mut title_tb = TextBox::new(Rect::new(margin_x, margin_y, content_w, title_h));
        let title_run = TextRun::plain(title, 48.0).bold();
        let mut p = Paragraph::new();
        p.push_run(title_run);
        title_tb.paragraphs.push(p);
        slide.add_element(SlideElement::TextBox(title_tb));
    }

    if !paragraphs.is_empty() {
        let body_y = if slide.elements.is_empty() { margin_y } else { margin_y + height * 0.16 };
        let body_h = height - body_y - margin_y;
        let mut body_tb = TextBox::new(Rect::new(margin_x, body_y, content_w, body_h));
        body_tb.paragraphs = paragraphs;
        slide.add_element(SlideElement::TextBox(body_tb));
    }

    slide
}

fn strip_notes_prefix(s: &str) -> &str {
    let lower = s.trim();
    if let Some(rest) = lower.strip_prefix("notes:").or_else(|| lower.strip_prefix("Notes:")) {
        rest.trim()
    } else if let Some(rest) = lower.strip_prefix("note:").or_else(|| lower.strip_prefix("Note:")) {
        rest.trim()
    } else {
        lower
    }
}

/// Export a deck into Markdown presentation format.
pub fn export_markdown(deck: &Deck) -> String {
    let mut out = String::new();

    for (i, slide) in deck.slides.iter().enumerate() {
        if i > 0 {
            out.push_str("\n---\n\n");
        }

        if !slide.title.is_empty() {
            out.push_str("# ");
            out.push_str(&slide.title);
            out.push_str("\n\n");
        }

        for elem in &slide.elements {
            match elem {
                SlideElement::TextBox(tb) => {
                    for p in &tb.paragraphs {
                        let text = p.plain_text();
                        if p.bullet_level > 0 {
                            let indent = "  ".repeat((p.bullet_level.saturating_sub(1)) as usize);
                            out.push_str(&indent);
                            out.push_str("- ");
                            out.push_str(&text);
                            out.push('\n');
                        } else if !text.is_empty() && text != slide.title {
                            out.push_str(&text);
                            out.push_str("\n\n");
                        }
                    }
                }
                SlideElement::Code(cb) => {
                    out.push_str("```");
                    if let Some(ref lang) = cb.language {
                        out.push_str(lang);
                    }
                    out.push('\n');
                    out.push_str(&cb.code);
                    if !cb.code.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n\n");
                }
                _ => {}
            }
        }

        if slide.transition != Transition::None {
            out.push_str("\n<!-- transition: ");
            out.push_str(slide.transition.kind_str());
            out.push_str(" -->\n");
        }
        if let Some(dwell) = slide.rehearsed_dwell_ms {
            out.push_str("<!-- timing: ");
            out.push_str(&alloc::format!("{}", dwell));
            out.push_str(" -->\n");
        }
        for b in &slide.builds {
            out.push_str("<!-- build: ");
            out.push_str(&alloc::format!("{} {} {}", b.element_index, b.effect.kind_str(), b.duration_ms));
            out.push_str(" -->\n");
        }

        if !slide.speaker_notes.is_empty() {
            out.push_str("\n<!-- notes:\n");
            out.push_str(&slide.speaker_notes);
            out.push_str("\n-->\n");
        }
    }

    out
}
