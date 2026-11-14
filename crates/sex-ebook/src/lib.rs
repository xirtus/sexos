// SPDX-License-Identifier: GPL-3.0-only
//! The e-book model behind Bodice. [`open`] reads a book's bytes, whatever the
//! format, into a [`Book`]: its title and author, chapters of text blocks, a table of
//! contents pointing at chapters, and for a comic the pictures that are its pages.
//!
//! * EPUB 2 and 3 ([`epub`]): the ZIP container, `META-INF/container.xml`, the OPF
//!   package (metadata, manifest, spine), and the table of contents from the EPUB 3
//!   navigation document or the EPUB 2 NCX. Chapters are XHTML, turned into text by
//!   [`sex_xml::html`].
//! * FictionBook 2 ([`fb2`]), plain or zipped: sections become chapters.
//! * Comic book archives (CBZ, [`cbz`]): the pictures in name order.
//!
//! [`layout`] reflows chapters into pages of a given width and height in characters
//! and finds a page again from a saved position, so a position survives a change of
//! font size. [`store`] keeps positions and settings as text files.
#![no_std]

extern crate alloc;

pub mod cbz;
pub mod epub;
pub mod fb2;
pub mod layout;
pub mod store;
pub mod zip;

use alloc::string::String;
use alloc::vec::Vec;
pub use sex_xml::html::{Block, Kind};

/// The most any one member of an archive may unpack to.
pub const MEMBER_MAX: usize = 16 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Epub, Fb2, Cbz }

impl Format {
    pub fn name(self) -> &'static str { match self { Format::Epub => "epub", Format::Fb2 => "fb2", Format::Cbz => "cbz" } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Zip(zip::Error),
    /// The bytes are not a book this crate reads, and why.
    NotBook(&'static str),
}

impl From<zip::Error> for Error { fn from(e: zip::Error) -> Error { Error::Zip(e) } }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chapter {
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    pub label: String,
    /// The chapter (spine position) it opens.
    pub chapter: usize,
    /// 0 for the top level.
    pub depth: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Book {
    pub format: Format,
    pub title: String,
    pub author: String,
    pub chapters: Vec<Chapter>,
    pub toc: Vec<TocEntry>,
    /// A comic's pages: the archive members holding its pictures, in reading order.
    pub images: Vec<String>,
}

impl Book {
    /// How many pages a comic has, or how many chapters a text book has.
    pub fn parts(&self) -> usize { if self.format == Format::Cbz { self.images.len() } else { self.chapters.len() } }
}

/// True for a file name Bodice lists in its library.
pub fn is_book_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".epub", ".fb2", ".fb2.zip", ".fbz", ".cbz"].iter().any(|e| lower.ends_with(e))
}

/// Reads a book from its bytes. `name` (the file name) only breaks ties: a ZIP with
/// an EPUB container is an EPUB, one holding a `.fb2` is FictionBook, any other with
/// pictures is a comic.
pub fn open(name: &str, data: &[u8]) -> Result<Book, Error> {
    if zip::is_zip(data) {
        let z = zip::Zip::open(data)?;
        if z.entry("META-INF/container.xml").is_some() || z.read("mimetype", 64).is_ok_and(|m| m.starts_with(b"application/epub+zip")) {
            return epub::read(&z);
        }
        if let Some(e) = z.entries.iter().find(|e| e.name.to_ascii_lowercase().ends_with(".fb2")) {
            return fb2::read(&z.read_entry(e, MEMBER_MAX)?);
        }
        let book = cbz::read(&z, name);
        return if book.images.is_empty() { Err(Error::NotBook("no pictures or book inside the archive")) } else { Ok(book) };
    }
    let head = &data[..data.len().min(1024)];
    if head.windows(11).any(|w| w == b"FictionBook") {
        return fb2::read(data);
    }
    Err(Error::NotBook("not an EPUB, FictionBook or comic archive"))
}

/// A path inside an archive: `href` taken relative to the folder `base` (which ends in
/// `/` or is empty), with `.` and `..` resolved, `%XX` decoded and any `#fragment` cut.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') { Vec::new() } else { base.split('/').filter(|p| !p.is_empty()).collect() };
    for p in href.split('/') {
        match p { "" | "." => {} ".." => { parts.pop(); } _ => parts.push(p) }
    }
    parts.join("/")
}

/// The folder part of an archive path, with its trailing `/` (empty at the top).
pub fn folder(path: &str) -> &str { path.rfind('/').map_or("", |i| &path[..=i]) }

fn percent_decode(s: &str) -> String {
    if !s.contains('%') { return String::from(s); }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) { out.push((h * 16 + l) as u8); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths() {
        assert_eq!(resolve("OEBPS/", "text/ch1.xhtml#p3"), "OEBPS/text/ch1.xhtml");
        assert_eq!(resolve("OEBPS/text/", "../images/a%20b.png"), "OEBPS/images/a b.png");
        assert_eq!(resolve("", "./x.html"), "x.html");
        assert_eq!(folder("OEBPS/content.opf"), "OEBPS/");
        assert_eq!(folder("content.opf"), "");
        assert!(is_book_name("A.EPUB") && is_book_name("b.fb2.zip") && !is_book_name("notes.txt"));
    }
}
