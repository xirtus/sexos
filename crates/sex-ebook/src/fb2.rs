// SPDX-License-Identifier: GPL-3.0-only
//! FictionBook 2: the title-info, and each body's top-level sections as chapters.
//! Nested section titles become headings and table of contents entries; poems,
//! epigraphs and citations become paragraphs and quotes. Pictures are left out.
use crate::{Block, Book, Chapter, Error, Format, Kind, TocEntry};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_xml::{collapse, parse, text_of, Child, Node};

pub fn read(data: &[u8]) -> Result<Book, Error> {
    let doc = parse(&text_of(data));
    let fb = doc.child("FictionBook").ok_or(Error::NotBook("no FictionBook element"))?;
    let info = fb.child("description").and_then(|d| d.child("title-info"));
    let title = info.map(|i| i.child_text("book-title")).unwrap_or_default();
    let author = info.and_then(|i| i.child("author")).map(|a| {
        let parts: Vec<String> = ["first-name", "middle-name", "last-name"].iter().map(|n| a.child_text(n)).filter(|s| !s.is_empty()).collect();
        if parts.is_empty() { a.child_text("nickname") } else { parts.join(" ") }
    }).unwrap_or_default();

    let mut chapters = Vec::new();
    let mut toc = Vec::new();
    for body in fb.children_named("body") {
        let sections: Vec<&Node> = body.children_named("section").collect();
        let notes = body.attr("name").is_some_and(|n| n == "notes");
        if sections.is_empty() || notes {
            let mut c = Chapter::default();
            flow(body, 1, &mut c.blocks, &mut toc, chapters.len());
            c.title = if notes { String::from("Notes") } else { first_heading(&c).unwrap_or_else(|| title.clone()) };
            if notes { toc.push(TocEntry { label: c.title.clone(), chapter: chapters.len(), depth: 0 }); }
            if !c.blocks.is_empty() { chapters.push(c); }
            continue;
        }
        // Anything before the first section (the body's title, an epigraph) is a chapter of its own.
        let mut lead = Chapter::default();
        for c in &body.children {
            if let Child::Element(e) = c { if !e.is("section") { element(e, 1, &mut lead.blocks, &mut Vec::new(), 0); } }
        }
        if !lead.blocks.is_empty() { lead.title = first_heading(&lead).unwrap_or_else(|| title.clone()); chapters.push(lead); }
        for s in sections {
            let mut c = Chapter::default();
            let n = chapters.len();
            let label = s.child("title").map(Node::text_trim).unwrap_or_default();
            if !label.is_empty() { toc.push(TocEntry { label: label.clone(), chapter: n, depth: 0 }); }
            flow(s, 1, &mut c.blocks, &mut toc, n);
            c.title = if label.is_empty() { format!("Part {}", n + 1) } else { label };
            chapters.push(c);
        }
    }
    if chapters.is_empty() { return Err(Error::NotBook("the FictionBook has no text")); }
    Ok(Book { format: Format::Fb2, title, author, chapters, toc, images: Vec::new() })
}

fn first_heading(c: &Chapter) -> Option<String> {
    c.blocks.iter().find(|b| matches!(b.kind, Kind::Heading(_))).map(|b| b.text.clone())
}

/// The children of a section or body, `level` deep.
fn flow(n: &Node, level: u8, out: &mut Vec<Block>, toc: &mut Vec<TocEntry>, chapter: usize) {
    for c in &n.children {
        if let Child::Element(e) = c { element(e, level, out, toc, chapter); }
    }
}

fn push(out: &mut Vec<Block>, kind: Kind, text: String) {
    let text = collapse(&text);
    if !text.is_empty() { out.push(Block { kind, text }); }
}

fn element(e: &Node, level: u8, out: &mut Vec<Block>, toc: &mut Vec<TocEntry>, chapter: usize) {
    let name = sex_xml::local(&e.name);
    match name {
        "title" => {
            let lines: Vec<String> = e.children_named("p").map(Node::text_trim).filter(|s| !s.is_empty()).collect();
            push(out, Kind::Heading(level.min(6)), if lines.is_empty() { e.text() } else { lines.join(" ") });
        }
        "subtitle" => push(out, Kind::Heading((level + 1).min(6)), e.text()),
        "p" | "v" | "text-author" => push(out, Kind::Para, e.text()),
        "epigraph" | "cite" | "annotation" => {
            let mut inner = Vec::new();
            flow(e, level, &mut inner, toc, chapter);
            for mut b in inner { if b.kind == Kind::Para { b.kind = Kind::Quote; } out.push(b); }
        }
        "section" => {
            if level >= 1 {
                let label = e.child("title").map(Node::text_trim).unwrap_or_default();
                if !label.is_empty() { toc.push(TocEntry { label, chapter, depth: level.min(255) }); }
            }
            flow(e, level + 1, out, toc, chapter);
        }
        "poem" | "stanza" | "table" | "tr" => flow(e, level, out, toc, chapter),
        "td" | "th" => push(out, Kind::Para, e.text()),
        "empty-line" | "image" | "binary" => {}
        _ => push(out, Kind::Para, e.text()),
    }
}
