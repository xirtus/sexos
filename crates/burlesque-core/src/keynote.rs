// SPDX-License-Identifier: GPL-3.0-only
//! Apple Keynote (`.key`) offline bundle importer.
//!
//! Supports:
//! - Modern iWork '13–14 `.key` ZIP archives containing `Index/Document.iwa`,
//!   `Index/Slide*.iwa`, `Index/Metadata.iwa` (Snappy-framed IWA protobuf streams)
//! - Legacy iWork '09 `.key` ZIP archives containing `index.apxl` / `index.apxl.gz`
//! - Metadata extraction from `Metadata/Properties.plist`
//! - Embedded slide media (`Data/*`) and preview fallback (`preview.jpg`,
//!   `preview-web.jpg`, `QuickLook/Thumbnail.jpg`, `QuickLook/Preview.jpg`)
//!   decoded via `sex-image`

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sex_canvas::Rect;

use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{FitMode, ImageElement, Paragraph, SlideElement, TextBox, TextRun};
use crate::layout::SlideLayout;
use crate::pptx::{extract_archive_file, MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_MEMBER_BYTES, MAX_SLIDES};
use crate::xml_utils::XmlNode;
use crate::{Error, Result};

/// Maximum extracted text blocks per slide to prevent pathological memory growth.
const MAX_TEXT_FRAGMENTS_PER_UNIT: usize = 256;
/// Maximum protobuf recursion depth for IWA extraction.
const MAX_PROTO_DEPTH: usize = 16;

/// Summary of what the Keynote importer recovered from a `.key` archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeynoteImportStats {
    pub slide_count: usize,
    pub text_slides: usize,
    pub extracted_images: usize,
    pub used_preview_fallback: bool,
}

/// Import an Apple Keynote `.key` bundle from raw bytes.
pub fn import_keynote(bytes: &[u8]) -> Result<Deck> {
    let (deck, _) = import_keynote_with_stats(bytes)?;
    Ok(deck)
}

/// Import an Apple Keynote `.key` bundle and return extraction diagnostics.
pub fn import_keynote_with_stats(bytes: &[u8]) -> Result<(Deck, KeynoteImportStats)> {
    let archive = sex_archive::Archive::open(bytes).map_err(|_| Error::InvalidFormat)?;
    if archive.entries().len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::MalformedDocument);
    }

    if !looks_like_keynote_bundle(&archive) {
        return Err(Error::InvalidFormat);
    }

    let mut total_extracted = 0usize;
    let mut deck = Deck::new(AspectRatio::Ratio16x9);

    // 1. Metadata/Properties.plist
    if let Some(plist_bytes) =
        extract_archive_file(&archive, "Metadata/Properties.plist", &mut total_extracted)
    {
        if let Ok(plist_str) = core::str::from_utf8(&plist_bytes) {
            parse_properties_plist(plist_str, &mut deck);
        }
    }

    // Optional Index/Metadata.iwa title/author hints
    if let Some(meta_iwa) =
        extract_archive_file(&archive, "Index/Metadata.iwa", &mut total_extracted)
    {
        let meta_texts = extract_iwa_strings(&meta_iwa);
        if deck.metadata.title.is_empty() {
            if let Some(first) = meta_texts.first() {
                if first.len() <= 120 {
                    deck.metadata.title = first.clone();
                }
            }
        }
    }

    // 2. Legacy iWork '09 index.apxl / index.apxl.gz
    let mut apxl_loaded = false;
    if let Some(apxl_bytes) = extract_archive_file(&archive, "index.apxl", &mut total_extracted) {
        if let Ok(apxl_str) = core::str::from_utf8(&apxl_bytes) {
            apxl_loaded = parse_apxl_into_deck(
                apxl_str,
                &archive,
                &mut total_extracted,
                &mut deck,
            );
        }
    } else if let Some(apxl_gz) =
        extract_archive_file(&archive, "index.apxl.gz", &mut total_extracted)
    {
        if let Some(inflated) = decompress_gzip(&apxl_gz) {
            if let Ok(apxl_str) = core::str::from_utf8(&inflated) {
                apxl_loaded = parse_apxl_into_deck(
                    apxl_str,
                    &archive,
                    &mut total_extracted,
                    &mut deck,
                );
            }
        }
    }

    // 3. Modern IWA slide files: Index/Slide*.iwa (excluding master/theme)
    if !apxl_loaded {
        let mut slide_iwa_paths: Vec<String> = archive
            .entries()
            .iter()
            .filter(|e| !e.is_dir() && is_slide_iwa_path(&e.path))
            .map(|e| e.path.clone())
            .collect();
        slide_iwa_paths.sort_by(|a, b| natural_cmp(a, b));
        slide_iwa_paths.truncate(MAX_SLIDES);

        for (idx, path) in slide_iwa_paths.iter().enumerate() {
            if let Some(iwa_bytes) = extract_archive_file(&archive, path, &mut total_extracted) {
                let texts = extract_iwa_strings(&iwa_bytes);
                if !texts.is_empty() {
                    let slide = build_slide_from_texts(
                        (idx + 1) as u64,
                        &texts,
                        deck.width(),
                        deck.height(),
                        idx == 0,
                    );
                    deck.slides.push(slide);
                }
            }
        }

        // 4. Fallback/primary Index/Document.iwa
        if deck.slides.is_empty() {
            if let Some(doc_iwa) =
                extract_archive_file(&archive, "Index/Document.iwa", &mut total_extracted)
            {
                let groups = extract_iwa_slide_groups(&doc_iwa);
                for (idx, texts) in groups.into_iter().take(MAX_SLIDES).enumerate() {
                    if !texts.is_empty() {
                        let slide = build_slide_from_texts(
                            (idx + 1) as u64,
                            &texts,
                            deck.width(),
                            deck.height(),
                            idx == 0,
                        );
                        deck.slides.push(slide);
                    }
                }
            }
        }
    }

    let text_slides = deck.slides.len();

    // 5. Embedded images from Data/*
    let mut data_img_paths: Vec<String> = archive
        .entries()
        .iter()
        .filter(|e| !e.is_dir() && is_keynote_data_image(&e.path))
        .map(|e| e.path.clone())
        .collect();
    data_img_paths.sort_by(|a, b| natural_cmp(a, b));
    data_img_paths.truncate(64);

    let mut decoded_data_images: Vec<(String, Vec<u8>)> = Vec::new();
    for path in data_img_paths {
        if let Some(img_bytes) = extract_archive_file(&archive, &path, &mut total_extracted) {
            if sex_image::decode(&img_bytes, &sex_image::Limits::PHOTO).is_ok() {
                decoded_data_images.push((path, img_bytes));
            }
        }
    }

    let mut extracted_images = 0usize;
    let w = deck.width();
    let h = deck.height();

    if !decoded_data_images.is_empty() {
        if deck.slides.is_empty() {
            for (idx, (_path, img_bytes)) in decoded_data_images.into_iter().enumerate() {
                let mut slide = Slide::new((idx + 1) as u64, SlideLayout::Blank);
                if idx == 0 && !deck.metadata.title.is_empty() {
                    slide.title = deck.metadata.title.clone();
                } else {
                    slide.title = format!("Slide {}", idx + 1);
                }
                let img = ImageElement::new(
                    Rect::new(w * 0.08, h * 0.12, w * 0.84, h * 0.78),
                    img_bytes,
                    FitMode::Contain,
                );
                slide.add_element(SlideElement::Image(img));
                deck.slides.push(slide);
                extracted_images += 1;
            }
        } else {
            // Attach embedded images cleanly to slides
            for (idx, (_path, img_bytes)) in decoded_data_images.into_iter().enumerate() {
                if idx < deck.slides.len() {
                    let img_bounds = Rect::new(w * 0.56, h * 0.24, w * 0.36, h * 0.64);
                    // Shrink existing body textbox on that slide if it would overlap
                    for elem in &mut deck.slides[idx].elements {
                        if let SlideElement::TextBox(tb) = elem {
                            if tb.bounds.y > h * 0.20 && tb.bounds.w > w * 0.50 {
                                tb.bounds.w = w * 0.44;
                            }
                        }
                    }
                    let img = ImageElement::new(img_bounds, img_bytes, FitMode::Contain);
                    deck.slides[idx].add_element(SlideElement::Image(img));
                    extracted_images += 1;
                } else {
                    let slide_num = deck.slides.len() + 1;
                    let mut slide = Slide::new(slide_num as u64, SlideLayout::Blank);
                    slide.title = format!("Slide {}", slide_num);
                    let img = ImageElement::new(
                        Rect::new(w * 0.08, h * 0.12, w * 0.84, h * 0.78),
                        img_bytes,
                        FitMode::Contain,
                    );
                    slide.add_element(SlideElement::Image(img));
                    deck.slides.push(slide);
                    extracted_images += 1;
                }
            }
        }
    }

    // 6. Preview image fallback (preview.jpg / preview-web.jpg / QuickLook/Thumbnail.jpg / QuickLook/Preview.jpg)
    let mut used_preview_fallback = false;
    if deck.slides.is_empty() {
        let preview_candidates = [
            "preview.jpg",
            "preview.png",
            "preview-web.jpg",
            "preview-web.png",
            "QuickLook/Preview.jpg",
            "QuickLook/Preview.png",
            "QuickLook/Thumbnail.jpg",
            "QuickLook/Thumbnail.png",
            "preview-micro.jpg",
        ];
        for cand in preview_candidates {
            if let Some(prev_bytes) = extract_archive_file(&archive, cand, &mut total_extracted) {
                if sex_image::decode(&prev_bytes, &sex_image::Limits::PHOTO).is_ok() {
                    let mut slide = Slide::new(1, SlideLayout::Blank);
                    slide.title = if deck.metadata.title.is_empty() {
                        String::from("Imported Preview")
                    } else {
                        deck.metadata.title.clone()
                    };
                    let img = ImageElement::new(
                        Rect::new(w * 0.04, h * 0.06, w * 0.92, h * 0.88),
                        prev_bytes,
                        FitMode::Contain,
                    );
                    slide.add_element(SlideElement::Image(img));
                    deck.slides.push(slide);
                    extracted_images += 1;
                    used_preview_fallback = true;
                    break;
                }
            }
        }
    }

    // 7. Guaranteed fallback slide if metadata was present in an otherwise binary-only .key
    if deck.slides.is_empty() {
        let title = if deck.metadata.title.is_empty() {
            String::from("Imported Presentation")
        } else {
            deck.metadata.title.clone()
        };
        let mut slide = build_slide_from_texts(1, &[title], w, h, true);
        if !deck.metadata.description.is_empty() {
            slide.set_speaker_notes(deck.metadata.description.clone());
        }
        deck.slides.push(slide);
    }

    // Populate slide 0 title from metadata if slide 0 lacks one
    if deck.metadata.title.is_empty() {
        if let Some(first) = deck.slides.first() {
            if !first.title.is_empty() {
                deck.metadata.title = first.title.clone();
            }
        }
    }

    let stats = KeynoteImportStats {
        slide_count: deck.slides.len(),
        text_slides,
        extracted_images,
        used_preview_fallback,
    };
    Ok((deck, stats))
}

fn looks_like_keynote_bundle(archive: &sex_archive::Archive<&[u8]>) -> bool {
    let mut has_iwa = false;
    let mut has_apxl = false;
    let mut has_keynote_meta = false;
    let mut has_preview = false;
    let mut has_data_dir = false;
    let mut has_pptx_or_odp = false;

    for e in archive.entries() {
        let p = e.path.as_str();
        if p.eq_ignore_ascii_case("ppt/presentation.xml") || p.eq_ignore_ascii_case("content.xml") {
            has_pptx_or_odp = true;
        }
        if (p.starts_with("Index/") || p.starts_with("index/")) && p.ends_with(".iwa") {
            has_iwa = true;
        }
        if p.eq_ignore_ascii_case("index.apxl") || p.eq_ignore_ascii_case("index.apxl.gz") {
            has_apxl = true;
        }
        if p.eq_ignore_ascii_case("Metadata/Properties.plist")
            || p.eq_ignore_ascii_case("Metadata/BuildVersionHistory.plist")
            || p.eq_ignore_ascii_case("Metadata/DocumentIdentifier")
        {
            has_keynote_meta = true;
        }
        if p.eq_ignore_ascii_case("preview.jpg")
            || p.eq_ignore_ascii_case("preview-web.jpg")
            || p.eq_ignore_ascii_case("QuickLook/Thumbnail.jpg")
            || p.eq_ignore_ascii_case("QuickLook/Preview.jpg")
        {
            has_preview = true;
        }
        if p.starts_with("Data/") || p.starts_with("data/") {
            has_data_dir = true;
        }
    }

    if has_iwa || has_apxl || has_keynote_meta {
        return true;
    }
    if has_preview && has_data_dir && !has_pptx_or_odp {
        return true;
    }
    false
}

fn is_slide_iwa_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("index/") || !lower.ends_with(".iwa") {
        return false;
    }
    let file = &lower["index/".len()..];
    file.starts_with("slide") && !file.contains("master") && !file.contains("theme")
}

fn is_keynote_data_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("data/") {
        return false;
    }
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
}

fn parse_properties_plist(xml: &str, deck: &mut Deck) {
    let Ok(root) = XmlNode::parse(xml) else {
        return;
    };
    let mut dicts = Vec::new();
    root.find_all("dict", &mut dicts);
    for dict in dicts {
        let mut current_key: Option<String> = None;
        for child in &dict.children {
            if child.name == "key" {
                current_key = Some(child.text.trim().to_string());
            } else if let Some(key) = current_key.take() {
                if child.name == "string" {
                    let val = child.text.trim();
                    if val.is_empty() {
                        continue;
                    }
                    let lk = key.to_ascii_lowercase();
                    if lk == "title" || lk == "ksfwpdocumenttitle" || lk == "kmditemtitle" {
                        if deck.metadata.title.is_empty() {
                            deck.metadata.title = val.to_string();
                        }
                    } else if lk == "author"
                        || lk == "creator"
                        || lk == "kmditemauthors"
                    {
                        if deck.metadata.author.is_empty() {
                            deck.metadata.author = val.to_string();
                        }
                    } else if lk == "comment"
                        || lk == "description"
                        || lk == "kmditemdescription"
                    {
                        if deck.metadata.description.is_empty() {
                            deck.metadata.description = val.to_string();
                        }
                    } else if lk == "date" || lk == "creationdate" {
                        if deck.metadata.date.is_empty() {
                            deck.metadata.date = val.to_string();
                        }
                    }
                }
            }
        }
    }
}

fn parse_apxl_into_deck(
    apxl_str: &str,
    archive: &sex_archive::Archive<&[u8]>,
    total_extracted: &mut usize,
    deck: &mut Deck,
) -> bool {
    let Ok(root) = XmlNode::parse(apxl_str) else {
        return false;
    };
    let mut slide_nodes = Vec::new();
    root.find_all("slide", &mut slide_nodes);
    if slide_nodes.is_empty() {
        return false;
    }

    let w = deck.width();
    let h = deck.height();

    for (idx, snode) in slide_nodes.iter().take(MAX_SLIDES).enumerate() {
        let mut texts = Vec::new();
        let mut p_nodes = Vec::new();
        snode.find_all("p", &mut p_nodes);
        for p in p_nodes {
            let t = p.text.trim();
            if !t.is_empty() {
                texts.push(t.to_string());
            }
        }
        if texts.is_empty() {
            if let Some(t) = snode.attr("title").or_else(|| snode.attr("name")) {
                if !t.trim().is_empty() {
                    texts.push(t.trim().to_string());
                }
            }
        }

        let mut slide = build_slide_from_texts((idx + 1) as u64, &texts, w, h, idx == 0);

        // Speaker notes in APXL
        let mut notes_nodes = Vec::new();
        snode.find_all("notes", &mut notes_nodes);
        snode.find_all("speaker-notes", &mut notes_nodes);
        let mut notes_acc = String::new();
        for nn in notes_nodes {
            let direct = nn.text.trim();
            if !direct.is_empty() {
                if !notes_acc.is_empty() {
                    notes_acc.push('\n');
                }
                notes_acc.push_str(direct);
            }
            let mut np = Vec::new();
            nn.find_all("p", &mut np);
            for p in np {
                let pt = p.text.trim();
                if !pt.is_empty() {
                    if !notes_acc.is_empty() {
                        notes_acc.push('\n');
                    }
                    notes_acc.push_str(pt);
                }
            }
        }
        if !notes_acc.is_empty() {
            slide.set_speaker_notes(notes_acc);
        }

        // Image references in APXL
        let mut img_nodes = Vec::new();
        snode.find_all("image", &mut img_nodes);
        for inode in img_nodes {
            if let Some(href) = inode.attr("href").or_else(|| inode.attr("src")).or_else(|| inode.attr("path")) {
                let clean = href.trim_start_matches("./");
                if let Some(data) = extract_archive_file(archive, clean, total_extracted) {
                    let elem = ImageElement::new(
                        Rect::new(w * 0.56, h * 0.24, w * 0.36, h * 0.62),
                        data,
                        FitMode::Contain,
                    );
                    slide.add_element(SlideElement::Image(elem));
                }
            }
        }

        deck.slides.push(slide);
    }

    !deck.slides.is_empty()
}

fn build_slide_from_texts(
    slide_id: u64,
    texts: &[String],
    width: f32,
    height: f32,
    is_first: bool,
) -> Slide {
    let layout = if is_first && texts.len() <= 2 {
        SlideLayout::TitleSlide
    } else {
        SlideLayout::TitleAndBody
    };
    let mut slide = Slide::new(slide_id, layout);

    let margin_x = width * 0.08;
    let margin_y = height * 0.08;
    let content_w = width - margin_x * 2.0;

    if let Some(title) = texts.first() {
        slide.title = title.clone();
        let mut title_tb = TextBox::new(Rect::new(margin_x, margin_y, content_w, height * 0.16));
        let mut p = Paragraph::new();
        p.push_run(TextRun::plain(title.clone(), 48.0).bold());
        title_tb.paragraphs.push(p);
        slide.add_element(SlideElement::TextBox(title_tb));
    }

    if texts.len() > 1 {
        let body_y = margin_y + height * 0.20;
        let body_h = height - body_y - margin_y;
        let mut body_tb = TextBox::new(Rect::new(margin_x, body_y, content_w, body_h));
        for line in &texts[1..] {
            if let Some(bullet) = line.strip_prefix("- ").or_else(|| line.strip_prefix("• ")) {
                body_tb.paragraphs.push(Paragraph::bullet(bullet, 26.0, 1));
            } else {
                body_tb.paragraphs.push(Paragraph::from_text(line.clone(), 26.0));
            }
        }
        slide.add_element(SlideElement::TextBox(body_tb));
    }

    slide
}

/// Decompress gzip payload (used for `index.apxl.gz`).
fn decompress_gzip(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 18 || data[0] != 0x1F || data[1] != 0x8B || data[2] != 8 {
        return None;
    }
    let flags = data[3];
    let mut pos = 10usize;
    if (flags & 0x04) != 0 {
        if pos + 2 > data.len() {
            return None;
        }
        let xlen = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos = pos.checked_add(2 + xlen)?;
    }
    if (flags & 0x08) != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if (flags & 0x10) != 0 {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        pos += 1;
    }
    if (flags & 0x02) != 0 {
        pos = pos.checked_add(2)?;
    }
    if pos + 8 > data.len() {
        return None;
    }
    let deflate_slice = &data[pos..data.len() - 8];
    miniz_oxide::inflate::decompress_to_vec_with_limit(deflate_slice, MAX_ARCHIVE_MEMBER_BYTES).ok()
}

/// Decompress Apple IWA Snappy-framed chunks (`[0x00, len_lo, len_mid, len_hi] + raw_snappy`).
/// Falls back to returning `raw` if not framed (e.g. uncompressed protobuf stream).
pub(crate) fn decompress_iwa_payload(raw: &[u8]) -> Vec<u8> {
    if raw.len() >= 4 && raw[0] == 0x00 {
        let mut out = Vec::new();
        let mut pos = 0usize;
        let mut decoded_any = false;
        while pos + 4 <= raw.len() {
            if raw[pos] != 0x00 {
                break;
            }
            let chunk_len =
                (raw[pos + 1] as usize) | ((raw[pos + 2] as usize) << 8) | ((raw[pos + 3] as usize) << 16);
            pos += 4;
            if chunk_len == 0 || pos.checked_add(chunk_len).map_or(true, |end| end > raw.len()) {
                decoded_any = false;
                break;
            }
            if let Some(chunk_out) = decompress_snappy(&raw[pos..pos + chunk_len]) {
                if out.len().saturating_add(chunk_out.len()) > MAX_ARCHIVE_MEMBER_BYTES {
                    break;
                }
                out.extend_from_slice(&chunk_out);
                decoded_any = true;
                pos += chunk_len;
            } else {
                decoded_any = false;
                break;
            }
        }
        if decoded_any && !out.is_empty() {
            return out;
        }
    }
    raw.to_vec()
}

/// Pure-Rust `#![no_std]` Snappy raw block decompressor (bounded to `MAX_ARCHIVE_MEMBER_BYTES`).
pub(crate) fn decompress_snappy(input: &[u8]) -> Option<Vec<u8>> {
    let (uncomp_len, mut pos) = read_varint(input, 0)?;
    let target_len = usize::try_from(uncomp_len).ok()?;
    if target_len > MAX_ARCHIVE_MEMBER_BYTES {
        return None;
    }

    let mut out = Vec::with_capacity(target_len.min(64 * 1024));
    while pos < input.len() {
        let tag = input[pos];
        pos += 1;
        match tag & 0x03 {
            0x00 => {
                // Literal
                let lit_code = (tag >> 2) as usize;
                let lit_len = if lit_code < 60 {
                    lit_code + 1
                } else {
                    let extra_bytes = lit_code - 59; // 1..=4
                    if pos + extra_bytes > input.len() {
                        return None;
                    }
                    let mut n = 0usize;
                    for i in 0..extra_bytes {
                        n |= (input[pos + i] as usize) << (8 * i);
                    }
                    pos += extra_bytes;
                    n.checked_add(1)?
                };
                if pos.checked_add(lit_len)? > input.len() {
                    return None;
                }
                if out.len().saturating_add(lit_len) > target_len {
                    return None;
                }
                out.extend_from_slice(&input[pos..pos + lit_len]);
                pos += lit_len;
            }
            0x01 => {
                // Copy with 1-byte offset
                let len = ((tag >> 2) & 0x07) as usize + 4;
                if pos >= input.len() {
                    return None;
                }
                let offset = (((tag >> 5) as usize) << 8) | (input[pos] as usize);
                pos += 1;
                copy_snappy_backref(&mut out, offset, len, target_len)?;
            }
            0x02 => {
                // Copy with 2-byte offset
                let len = (tag >> 2) as usize + 1;
                if pos + 2 > input.len() {
                    return None;
                }
                let offset = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                pos += 2;
                copy_snappy_backref(&mut out, offset, len, target_len)?;
            }
            _ => {
                // 0x03: Copy with 4-byte offset
                let len = (tag >> 2) as usize + 1;
                if pos + 4 > input.len() {
                    return None;
                }
                let offset =
                    u32::from_le_bytes([input[pos], input[pos + 1], input[pos + 2], input[pos + 3]])
                        as usize;
                pos += 4;
                copy_snappy_backref(&mut out, offset, len, target_len)?;
            }
        }
    }

    if out.len() != target_len {
        return None;
    }
    Some(out)
}

fn copy_snappy_backref(
    out: &mut Vec<u8>,
    offset: usize,
    len: usize,
    target_len: usize,
) -> Option<()> {
    if offset == 0 || offset > out.len() || out.len().saturating_add(len) > target_len {
        return None;
    }
    for _ in 0..len {
        let b = out[out.len() - offset];
        out.push(b);
    }
    Some(())
}

fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut val = 0u64;
    let mut shift = 0u32;
    for _ in 0..10 {
        if pos >= buf.len() {
            return None;
        }
        let b = buf[pos];
        pos += 1;
        val |= ((b & 0x7F) as u64) << shift;
        if (b & 0x80) == 0 {
            return Some((val, pos));
        }
        shift += 7;
    }
    None
}

/// Extract clean user-facing text strings from an `.iwa` file.
pub(crate) fn extract_iwa_strings(raw_iwa: &[u8]) -> Vec<String> {
    let stream = decompress_iwa_payload(raw_iwa);
    let mut out = Vec::new();
    for msg in iter_iwa_messages(&stream) {
        scan_proto_strings(msg, 0, &mut out);
        if out.len() >= MAX_TEXT_FRAGMENTS_PER_UNIT {
            break;
        }
    }
    if out.is_empty() {
        scan_proto_strings(&stream, 0, &mut out);
    }
    dedup_adjacent(out)
}

/// Group `Index/Document.iwa` into per-slide text vectors where possible.
fn extract_iwa_slide_groups(raw_iwa: &[u8]) -> Vec<Vec<String>> {
    let stream = decompress_iwa_payload(raw_iwa);
    let msgs = iter_iwa_messages(&stream);
    if msgs.len() > 1 {
        let mut groups = Vec::new();
        for msg in msgs {
            let mut texts = Vec::new();
            scan_proto_strings(msg, 0, &mut texts);
            let clean = dedup_adjacent(texts);
            if !clean.is_empty() {
                groups.push(clean);
            }
        }
        if !groups.is_empty() {
            return groups;
        }
    }
    let flat = extract_iwa_strings(raw_iwa);
    if flat.is_empty() {
        Vec::new()
    } else {
        vec![flat]
    }
}

/// Parse IWA `ArchiveInfo` framing (`varint header_len` + `ArchiveInfo` + message payloads).
fn iter_iwa_messages<'a>(stream: &'a [u8]) -> Vec<&'a [u8]> {
    let mut messages = Vec::new();
    let mut pos = 0usize;

    while pos < stream.len() && messages.len() < MAX_SLIDES * 8 {
        let Some((info_len_u64, after_len)) = read_varint(stream, pos) else {
            break;
        };
        let Ok(info_len) = usize::try_from(info_len_u64) else {
            break;
        };
        if info_len == 0 || after_len.saturating_add(info_len) > stream.len() {
            break;
        }
        let info_slice = &stream[after_len..after_len + info_len];
        let payload_lengths = parse_archive_info_lengths(info_slice);
        if payload_lengths.is_empty() {
            break;
        }
        let mut cursor = after_len + info_len;
        let mut ok = true;
        for plen in payload_lengths {
            if cursor.saturating_add(plen) > stream.len() {
                ok = false;
                break;
            }
            messages.push(&stream[cursor..cursor + plen]);
            cursor += plen;
        }
        if !ok || cursor <= pos {
            break;
        }
        pos = cursor;
    }

    if pos != stream.len() && messages.is_empty() {
        messages.push(stream);
    }
    messages
}

/// Extract `MessageInfo.length` fields from `TSP.ArchiveInfo` protobuf.
fn parse_archive_info_lengths(info: &[u8]) -> Vec<usize> {
    let mut lengths = Vec::new();
    let mut pos = 0usize;
    while pos < info.len() {
        let Some((tag, next)) = read_varint(info, pos) else {
            return Vec::new();
        };
        pos = next;
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;
        match wire_type {
            0 => {
                let Some((_, n)) = read_varint(info, pos) else {
                    return Vec::new();
                };
                pos = n;
            }
            1 => {
                if pos + 8 > info.len() {
                    return Vec::new();
                }
                pos += 8;
            }
            2 => {
                let Some((len_u64, n)) = read_varint(info, pos) else {
                    return Vec::new();
                };
                let Ok(len) = usize::try_from(len_u64) else {
                    return Vec::new();
                };
                pos = n;
                if pos.saturating_add(len) > info.len() {
                    return Vec::new();
                }
                if field_num == 2 {
                    if let Some(msg_len) = parse_message_info_length(&info[pos..pos + len]) {
                        lengths.push(msg_len);
                    }
                }
                pos += len;
            }
            5 => {
                if pos + 4 > info.len() {
                    return Vec::new();
                }
                pos += 4;
            }
            _ => return Vec::new(),
        }
    }
    lengths
}

fn parse_message_info_length(msg_info: &[u8]) -> Option<usize> {
    let mut pos = 0usize;
    let mut msg_len = None;
    while pos < msg_info.len() {
        let (tag, next) = read_varint(msg_info, pos)?;
        pos = next;
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;
        match wire_type {
            0 => {
                let (val, n) = read_varint(msg_info, pos)?;
                pos = n;
                if field_num == 3 {
                    msg_len = usize::try_from(val).ok();
                }
            }
            1 => {
                if pos + 8 > msg_info.len() {
                    return None;
                }
                pos += 8;
            }
            2 => {
                let (len_u64, n) = read_varint(msg_info, pos)?;
                let len = usize::try_from(len_u64).ok()?;
                pos = n.checked_add(len)?;
                if pos > msg_info.len() {
                    return None;
                }
            }
            5 => {
                if pos + 4 > msg_info.len() {
                    return None;
                }
                pos += 4;
            }
            _ => return None,
        }
    }
    msg_len
}

fn scan_proto_strings(buf: &[u8], depth: usize, out: &mut Vec<String>) {
    if depth > MAX_PROTO_DEPTH || out.len() >= MAX_TEXT_FRAGMENTS_PER_UNIT {
        return;
    }
    let mut pos = 0usize;
    while pos < buf.len() && out.len() < MAX_TEXT_FRAGMENTS_PER_UNIT {
        let Some((tag, next)) = read_varint(buf, pos) else {
            break;
        };
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;
        if field_num == 0 {
            break;
        }
        pos = next;
        match wire_type {
            0 => {
                let Some((_, n)) = read_varint(buf, pos) else {
                    break;
                };
                pos = n;
            }
            1 => {
                if pos + 8 > buf.len() {
                    break;
                }
                pos += 8;
            }
            2 => {
                let Some((len_u64, n)) = read_varint(buf, pos) else {
                    break;
                };
                let Ok(len) = usize::try_from(len_u64) else {
                    break;
                };
                pos = n;
                if pos.saturating_add(len) > buf.len() {
                    break;
                }
                let slice = &buf[pos..pos + len];
                pos += len;

                if is_likely_submessage(slice) {
                    scan_proto_strings(slice, depth + 1, out);
                } else if let Ok(s) = core::str::from_utf8(slice) {
                    let clean = s.trim();
                    if is_meaningful_slide_text(clean) {
                        for line in clean.lines() {
                            let l = line.trim();
                            if is_meaningful_slide_text(l) {
                                out.push(l.to_string());
                            }
                        }
                    }
                }
            }
            5 => {
                if pos + 4 > buf.len() {
                    break;
                }
                pos += 4;
            }
            _ => break,
        }
    }
}

fn is_likely_submessage(slice: &[u8]) -> bool {
    if slice.len() < 2 {
        return false;
    }
    // Check if it looks like clean printable UTF-8 first — pure text shouldn't be misread as protobuf
    if let Ok(s) = core::str::from_utf8(slice) {
        if !s.is_empty() && s.chars().all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t') {
            return false;
        }
    }
    // Verify it parses cleanly to exact end as protobuf wire fields
    let mut pos = 0usize;
    let mut fields = 0usize;
    while pos < slice.len() {
        let Some((tag, next)) = read_varint(slice, pos) else {
            return false;
        };
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;
        if field_num == 0 || field_num > 10_000 {
            return false;
        }
        pos = next;
        match wire_type {
            0 => {
                let Some((_, n)) = read_varint(slice, pos) else {
                    return false;
                };
                pos = n;
            }
            1 => {
                if pos + 8 > slice.len() {
                    return false;
                }
                pos += 8;
            }
            2 => {
                let Some((len_u64, n)) = read_varint(slice, pos) else {
                    return false;
                };
                let Ok(len) = usize::try_from(len_u64) else {
                    return false;
                };
                pos = n;
                if pos.saturating_add(len) > slice.len() {
                    return false;
                }
                pos += len;
            }
            5 => {
                if pos + 4 > slice.len() {
                    return false;
                }
                pos += 4;
            }
            _ => return false,
        }
        fields += 1;
    }
    pos == slice.len() && fields > 0
}

fn is_meaningful_slide_text(s: &str) -> bool {
    if s.is_empty() || s.len() > 8192 {
        return false;
    }
    if s.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') {
        return false;
    }
    if !s.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }
    // Filter Apple internal schema/style/font/UUID noise
    let prefixes = [
        "TSP.", "TSWP.", "TSS.", "TSD.", "TSCH.", "TSCE.", "KN.", "TSK.", "com.apple.",
        "Helvetica", "SFPro", "PingFang", "TimesNewRoman", "CourierNew", "Arial",
        "LucidaGrande", "AppleColorEmoji",
    ];
    if prefixes.iter().any(|p| s.starts_with(p)) {
        return false;
    }
    if is_uuid_like(s) {
        return false;
    }
    true
}

fn is_uuid_like(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && b.iter().all(|&c| c == b'-' || c.is_ascii_hexdigit())
}

fn dedup_adjacent(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if out.last().map_or(true, |prev| prev != &item) {
            out.push(item);
        }
    }
    out
}

fn natural_cmp(a: &str, b: &str) -> core::cmp::Ordering {
    let mut ia = a.as_bytes();
    let mut ib = b.as_bytes();
    while !ia.is_empty() && !ib.is_empty() {
        if ia[0].is_ascii_digit() && ib[0].is_ascii_digit() {
            let na_len = ia.iter().take_while(|c| c.is_ascii_digit()).count();
            let nb_len = ib.iter().take_while(|c| c.is_ascii_digit()).count();
            let va = core::str::from_utf8(&ia[..na_len]).ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let vb = core::str::from_utf8(&ib[..nb_len]).ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            match va.cmp(&vb) {
                core::cmp::Ordering::Equal => {
                    ia = &ia[na_len..];
                    ib = &ib[nb_len..];
                }
                ord => return ord,
            }
        } else {
            match ia[0].cmp(&ib[0]) {
                core::cmp::Ordering::Equal => {
                    ia = &ia[1..];
                    ib = &ib[1..];
                }
                ord => return ord,
            }
        }
    }
    ia.len().cmp(&ib.len())
}
