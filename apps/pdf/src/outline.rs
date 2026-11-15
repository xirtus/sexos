// SPDX-License-Identifier: GPL-3.0-only
//! The document outline (bookmarks): titles and the pages they open, read from the
//! catalog's /Outlines tree with hayro-syntax's objects (hayro has no outline API).
//!
//! An entry's destination is an explicit `[page /XYZ ...]` array, given as /Dest or in
//! a /GoTo action. Named destinations need the name tree and are left without a page:
//! the entry is still listed, it just does not move.
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Object, ObjectIdentifier, String as PdfString};
use hayro::hayro_syntax::Pdf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub depth: usize,
    pub title: String,
    /// The page it opens, from 0.
    pub page: Option<usize>,
}

/// A bound on entries and depth: a malformed file may loop its /Next links.
const MOST: usize = 512;
const DEEPEST: usize = 8;

/// A text string: UTF-16BE after its byte order mark, otherwise PDFDocEncoding, whose
/// printable ASCII and Latin-1 ranges are the same characters.
fn text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
        return char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect();
    }
    bytes.iter().map(|b| *b as char).collect()
}

fn page_of(dest: Option<Array<'_>>, pages: &[Option<ObjectIdentifier>]) -> Option<usize> {
    let first = dest?.raw_iter().next()?;
    match first {
        MaybeRef::Ref(r) => {
            let id: ObjectIdentifier = r.into();
            pages.iter().position(|p| *p == Some(id))
        }
        // Some writers give a page number for a page in this file.
        MaybeRef::NotRef(Object::Number(n)) => Some(n.as_f64() as usize).filter(|n| *n < pages.len()),
        MaybeRef::NotRef(_) => None,
    }
}

/// The outline, flattened in reading order with each entry's depth. Empty when the
/// document has none.
pub fn read(pdf: &Pdf) -> Vec<Entry> {
    let pages: Vec<Option<ObjectIdentifier>> = pdf.pages().iter().map(|p| p.raw().obj_id()).collect();
    let xref = pdf.xref();
    let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) else { return Vec::new() };
    let Some(root) = catalog.get::<Dict<'_>>("Outlines") else { return Vec::new() };
    let mut out = Vec::new();
    // Siblings to visit, deepest last: (item, depth).
    let mut stack: Vec<(Dict<'_>, usize)> = root.get::<Dict<'_>>("First").map(|d| vec![(d, 0)]).unwrap_or_default();
    while let Some((item, depth)) = stack.pop() {
        if out.len() >= MOST { break; }
        let title = item.get::<PdfString<'_>>("Title").map(|s| text(s.as_bytes())).unwrap_or_default();
        let dest = item.get::<Array<'_>>("Dest").or_else(|| item.get::<Dict<'_>>("A").and_then(|a| a.get::<Array<'_>>("D")));
        out.push(Entry { depth, title: title.trim().to_string(), page: page_of(dest, &pages) });
        // The next sibling waits under the first child, which is read first.
        if let Some(next) = item.get::<Dict<'_>>("Next") { stack.push((next, depth)); }
        if depth + 1 < DEEPEST {
            if let Some(child) = item.get::<Dict<'_>>("First") { stack.push((child, depth + 1)); }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_titles_and_pages_from_dest_and_goto() {
        let pdf = Pdf::new(include_bytes!("../../../tests/fixtures/pdf/find10.pdf").to_vec()).expect("parses");
        let entries = read(&pdf);
        let got: Vec<(&str, Option<usize>)> = entries.iter().map(|e| (e.title.as_str(), e.page)).collect();
        assert_eq!(got, vec![("Introduction", Some(0)), ("Middle chapter", Some(4)), ("The island", Some(6)), ("Last page", Some(9))]);
        assert!(entries.iter().all(|e| e.depth == 0));
    }

    #[test]
    fn no_outline_is_empty_and_text_strings_decode() {
        let pdf = Pdf::new(crate::sample::document()).expect("parses");
        assert!(read(&pdf).is_empty());
        assert_eq!(text(&[0xFE, 0xFF, 0, b'H', 0, b'i']), "Hi");
        assert_eq!(text(b"Caf\xe9"), "Café");
    }
}
