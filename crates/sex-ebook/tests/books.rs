// SPDX-License-Identifier: GPL-3.0-only
//! The fixture books (made by make_fixtures.py) read, paginate and find their place.
use sex_ebook::layout::{chapter_page, page_of, paginate, wrap, Position, Style};
use sex_ebook::store::{Positions, Settings};
use sex_ebook::{open, zip::Zip, Format, Kind};

const EPUB: &[u8] = include_bytes!("tiny.epub");
const FB2: &[u8] = include_bytes!("tiny.fb2");
const CBZ: &[u8] = include_bytes!("tiny.cbz");

#[test]
fn epub_package_spine_and_toc() {
    let b = open("tiny.epub", EPUB).unwrap();
    assert_eq!(b.format, Format::Epub);
    assert_eq!(b.title, "A Tiny Book");
    assert_eq!(b.author, "Bodice Fixture");
    assert_eq!(b.chapters.len(), 3);
    let labels: Vec<(&str, usize)> = b.toc.iter().map(|t| (t.label.as_str(), t.chapter)).collect();
    assert_eq!(labels, [("Chapter 1", 0), ("Chapter 2", 1), ("Chapter 3", 2)]);
    let c2 = &b.chapters[1];
    assert_eq!(c2.title, "Chapter 2");
    assert_eq!(c2.blocks[0].kind, Kind::Heading(1));
    assert_eq!(c2.blocks.len(), 13);
    assert!(c2.blocks[1].text.starts_with("Paragraph 1 of chapter 2: "));
    // Neither the <title> nor the <style> leaks into the text.
    assert!(!c2.blocks.iter().any(|b| b.text.contains("margin")));
}

#[test]
fn epub2_ncx_alone() {
    // Without the nav document's properties the NCX gives the TOC.
    let z = Zip::open(EPUB).unwrap();
    let opf = String::from_utf8(z.read("OEBPS/content.opf", 1 << 20).unwrap()).unwrap();
    assert!(opf.contains("properties=\"nav\""));
    let rebuilt = rezip(&[("OEBPS/content.opf", opf.replace(" properties=\"nav\"", "").replace("version=\"3.0\"", "version=\"2.0\"").into_bytes())]);
    let b = open("old.epub", &rebuilt).unwrap();
    assert_eq!(b.toc.len(), 3);
    assert_eq!(b.toc[2].label, "Chapter 3");
    assert_eq!(b.toc[2].chapter, 2);
}

/// The fixture EPUB with some members replaced, written as a stored ZIP.
fn rezip(replace: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let z = Zip::open(EPUB).unwrap();
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in &z.entries {
        let data = replace.iter().find(|(n, _)| *n == e.name).map(|(_, d)| d.clone()).unwrap_or_else(|| z.read_entry(e, 1 << 20).unwrap());
        let crc = 0u32; // Not checked by the reader.
        let at = out.len() as u32;
        let name = e.name.as_bytes();
        let head = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central { h.extend_from_slice(&20u16.to_le_bytes()); }
            h.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            if central { h.extend_from_slice(&[0; 10]); h.extend_from_slice(&at.to_le_bytes()); }
            h.extend_from_slice(name);
            h
        };
        out.extend(head(0x0403_4b50, false));
        out.extend_from_slice(&data);
        central.extend(head(0x0201_4b50, true));
    }
    let dir_at = out.len() as u32;
    let n = z.entries.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&dir_at.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
fn pages_and_positions_across_font_sizes() {
    let b = open("tiny.epub", EPUB).unwrap();
    let big = paginate(&b, 78, 25);
    assert!(big.len() >= 6, "{} pages", big.len());
    // Each chapter starts a page, headed by its heading.
    for n in 0..3 {
        let p = &big[chapter_page(&big, n)];
        assert_eq!((p.chapter, p.start), (n, 0));
        assert_eq!(p.lines[0].style, Style::Heading);
        assert_eq!(p.lines[0].text, format!("Chapter {}", n + 1));
    }
    for p in &big {
        assert!(p.lines.len() <= 25);
        assert!(p.lines.iter().all(|l| l.text.chars().count() <= 78));
        assert!(!p.lines.is_empty() && !p.lines[0].text.is_empty());
    }
    // A place on page 4 is found again on the same page, and at another size on the
    // page that holds the same text.
    let pos = big[3].position();
    assert_eq!(page_of(&big, pos), 3);
    let small = paginate(&b, 40, 12);
    let i = page_of(&small, pos);
    assert_eq!(small[i].chapter, pos.chapter);
    assert!(small[i].start <= pos.offset && small.get(i + 1).map_or(true, |n| n.chapter != pos.chapter || n.start > pos.offset));
}

#[test]
fn wrapping() {
    assert_eq!(wrap("aaa bbb ccc", 7), [(0, "aaa bbb".to_string()), (8, "ccc".to_string())]);
    assert_eq!(wrap("abcdefghij", 4), [(0, "abcd".to_string()), (4, "efgh".to_string()), (8, "ij".to_string())]);
}

#[test]
fn fictionbook() {
    let b = open("tiny.fb2", FB2).unwrap();
    assert_eq!(b.format, Format::Fb2);
    assert_eq!(b.title, "A Tiny FictionBook");
    assert_eq!(b.author, "Bodice Fixture");
    // The body's own title, three sections and the notes.
    assert_eq!(b.chapters.len(), 5);
    assert_eq!(b.chapters[2].title, "Chapter 2");
    assert_eq!(b.chapters[2].blocks[0].kind, Kind::Heading(1));
    assert_eq!(b.chapters[2].blocks.len(), 4);
    assert_eq!(b.chapters[4].blocks[0].text, "A note & nothing more.");
    let labels: Vec<&str> = b.toc.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["Chapter 1", "Chapter 2", "Chapter 3", "Notes"]);
}

#[test]
fn comic_pages_in_natural_order() {
    let b = open("tiny.cbz", CBZ).unwrap();
    assert_eq!(b.format, Format::Cbz);
    assert_eq!(b.images, ["page1.png", "page2.png", "page10.png"]);
    let z = Zip::open(CBZ).unwrap();
    let colours: Vec<u32> = b.images.iter().map(|n| {
        let img = sex_image::decode(&z.read(n, 1 << 20).unwrap(), &sex_image::Limits::SURFACE).unwrap();
        assert_eq!((img.width, img.height), (8, 6));
        img.pixels[0] & 0x00FF_FFFF
    }).collect();
    assert_eq!(colours, [0xFF0000, 0x00FF00, 0x0000FF]);
}

#[test]
fn not_books() {
    assert!(open("x.txt", b"hello").is_err());
    assert!(open("x.epub", &EPUB[..EPUB.len() / 2]).is_err());
}

#[test]
fn saved_places_and_settings() {
    let mut p = Positions::default();
    p.set("tiny.epub", Position { chapter: 1, offset: 812 });
    p.set("b.fb2", Position { chapter: 0, offset: 0 });
    p.set("tiny.epub", Position { chapter: 2, offset: 5 });
    let again = Positions::parse(&p.serialize());
    assert_eq!(again.get("tiny.epub"), Some(Position { chapter: 2, offset: 5 }));
    assert_eq!(again.books.len(), 2);
    assert_eq!(Positions::parse("junk\nx\t1\n").books.len(), 0);
    let s = Settings { font: 3, night: true };
    assert_eq!(Settings::parse(&s.serialize()), s);
    assert_eq!(Settings::parse(""), Settings::default());
}
