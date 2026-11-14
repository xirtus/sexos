// SPDX-License-Identifier: GPL-3.0-only
//! EPUB 2 and 3: container, OPF package and table of contents.
use crate::zip::Zip;
use crate::{folder, resolve, Book, Chapter, Error, Format, TocEntry, MEMBER_MAX};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_xml::{html, parse, text_of, Node};

struct Item { id: String, href: String, media: String, properties: String }

fn read_text(z: &Zip, path: &str) -> Result<String, Error> { Ok(text_of(&z.read(path, MEMBER_MAX)?)) }

pub fn read(z: &Zip) -> Result<Book, Error> {
    let container = parse(&read_text(z, "META-INF/container.xml")?);
    let opf_path = container.find("rootfile").and_then(|r| r.attr("full-path")).map(String::from)
        .or_else(|| z.entries.iter().find(|e| e.name.ends_with(".opf")).map(|e| e.name.clone()))
        .ok_or(Error::NotBook("the EPUB names no package document"))?;
    let opf = parse(&read_text(z, &opf_path)?);
    let base = folder(&opf_path);
    let package = opf.child("package").ok_or(Error::NotBook("the package document has no package element"))?;
    let meta = package.child("metadata");
    let title = meta.and_then(|m| m.find("title")).map(Node::text_trim).unwrap_or_default();
    let author = meta.and_then(|m| m.find("creator")).map(Node::text_trim).unwrap_or_default();

    let mut items = Vec::new();
    if let Some(manifest) = package.child("manifest") {
        for it in manifest.children_named("item") {
            items.push(Item {
                id: String::from(it.attr("id").unwrap_or("")),
                href: resolve(base, it.attr("href").unwrap_or("")),
                media: String::from(it.attr("media-type").unwrap_or("")),
                properties: String::from(it.attr("properties").unwrap_or("")),
            });
        }
    }
    let spine_node = package.child("spine");
    let spine: Vec<&Item> = spine_node.map(|s| s.children_named("itemref").filter_map(|r| {
        let id = r.attr("idref")?;
        items.iter().find(|i| i.id == id)
    }).collect()).unwrap_or_default();
    if spine.is_empty() { return Err(Error::NotBook("the EPUB's spine is empty")); }

    let mut chapters = Vec::with_capacity(spine.len());
    for item in &spine {
        let blocks = match z.read(&item.href, MEMBER_MAX) {
            Ok(bytes) => html::blocks(&text_of(&bytes)),
            Err(_) => Vec::new(),
        };
        chapters.push(Chapter { title: String::new(), blocks });
    }
    let chapter_of = |path: &str| spine.iter().position(|i| i.href == path);

    // The EPUB 3 navigation document, else the NCX (spine's toc, or any NCX item).
    let mut toc = Vec::new();
    if let Some(nav) = items.iter().find(|i| i.properties.split(' ').any(|p| p == "nav")) {
        if let Ok(text) = read_text(z, &nav.href) { nav_toc(&parse(&text), folder(&nav.href), &chapter_of, &mut toc); }
    }
    if toc.is_empty() {
        let ncx = spine_node.and_then(|s| s.attr("toc")).and_then(|id| items.iter().find(|i| i.id == id))
            .or_else(|| items.iter().find(|i| i.media == "application/x-dtbncx+xml"));
        if let Some(ncx) = ncx {
            if let Ok(text) = read_text(z, &ncx.href) {
                let doc = parse(&text);
                if let Some(map) = doc.find("navMap") { ncx_points(map, folder(&ncx.href), 0, &chapter_of, &mut toc); }
            }
        }
    }
    // Chapter titles: the first TOC entry for it, else its first heading.
    for (n, c) in chapters.iter_mut().enumerate() {
        c.title = toc.iter().find(|t| t.chapter == n).map(|t| t.label.clone())
            .or_else(|| c.blocks.iter().find(|b| matches!(b.kind, html::Kind::Heading(_))).map(|b| b.text.clone()))
            .unwrap_or_else(|| format!("Part {}", n + 1));
    }
    if toc.is_empty() {
        toc = chapters.iter().enumerate().filter(|(_, c)| !c.blocks.is_empty()).map(|(n, c)| TocEntry { label: c.title.clone(), chapter: n, depth: 0 }).collect();
    }
    Ok(Book { format: Format::Epub, title, author, chapters, toc, images: Vec::new() })
}

fn nav_toc(doc: &Node, base: &str, chapter_of: &dyn Fn(&str) -> Option<usize>, toc: &mut Vec<TocEntry>) {
    let mut navs = Vec::new();
    doc.find_all("nav", &mut navs);
    let Some(nav) = navs.iter().find(|n| n.attr("epub:type").is_some_and(|t| t.split(' ').any(|w| w == "toc"))).or(navs.first()) else { return };
    if let Some(ol) = nav.find("ol") { nav_list(ol, base, 0, chapter_of, toc); }
}

fn nav_list(ol: &Node, base: &str, depth: u8, chapter_of: &dyn Fn(&str) -> Option<usize>, toc: &mut Vec<TocEntry>) {
    for li in ol.children_named("li") {
        let link = li.child("a").or_else(|| li.child("span"));
        if let Some(a) = link {
            let label = a.text_trim();
            if let Some(chapter) = a.attr("href").and_then(|h| chapter_of(&resolve(base, h))) {
                if !label.is_empty() { toc.push(TocEntry { label, chapter, depth }); }
            }
        }
        if let Some(sub) = li.child("ol") { nav_list(sub, base, depth.saturating_add(1), chapter_of, toc); }
    }
}

fn ncx_points(parent: &Node, base: &str, depth: u8, chapter_of: &dyn Fn(&str) -> Option<usize>, toc: &mut Vec<TocEntry>) {
    for p in parent.children_named("navPoint") {
        let label = p.child("navLabel").map(Node::text_trim).unwrap_or_default();
        if let Some(chapter) = p.child("content").and_then(|c| c.attr("src")).and_then(|s| chapter_of(&resolve(base, s))) {
            if !label.is_empty() { toc.push(TocEntry { label, chapter, depth }); }
        }
        ncx_points(p, base, depth.saturating_add(1), chapter_of, toc);
    }
}
