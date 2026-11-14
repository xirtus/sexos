// SPDX-License-Identifier: GPL-3.0-only
//! Text reflowed into pages of `cols` characters by `rows` lines, for a monospace face.
//! Each chapter starts a page. A position in a book is (chapter, character offset in
//! the chapter's text), where a chapter's text is its blocks joined by one separator
//! each; every page knows the offset of its first character, so a position saved at
//! one font size finds the page holding the same text at another.
use crate::{Book, Kind};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style { Body, Heading, Quote, Pre }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line { pub text: String, pub style: Style }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub chapter: usize,
    /// The chapter offset of the first character on the page.
    pub start: usize,
    pub lines: Vec<Line>,
}

/// A place in a book that does not depend on the page size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position { pub chapter: usize, pub offset: usize }

struct Pager { cols: usize, rows: usize, pages: Vec<Page>, chapter: usize }

impl Pager {
    fn line(&mut self, text: String, style: Style, offset: usize) {
        let need_new = self.pages.last().map_or(true, |p| p.chapter != self.chapter || p.lines.len() >= self.rows);
        if need_new {
            // No blank line at the top of a page.
            if text.is_empty() { return; }
            self.pages.push(Page { chapter: self.chapter, start: offset, lines: Vec::new() });
        }
        self.pages.last_mut().unwrap().lines.push(Line { text, style });
    }
}

/// Words wrapped to `cols`, each line with the char index in `text` where it starts.
/// A word longer than a line is cut.
pub fn wrap(text: &str, cols: usize) -> Vec<(usize, String)> {
    let cols = cols.max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i] == ' ' { i += 1; }
        if i >= chars.len() { break; }
        let start = i;
        let mut end = (start + cols).min(chars.len());
        if end < chars.len() && chars[end] != ' ' {
            // Back to the last space inside the line, if there is one.
            if let Some(sp) = (start + 1..end).rev().find(|&k| chars[k] == ' ') { end = sp; }
        }
        let line: String = chars[start..end].iter().collect();
        out.push((start, String::from(line.trim_end())));
        i = end;
    }
    out
}

/// The book's pages at `cols` by `rows` characters.
pub fn paginate(book: &Book, cols: usize, rows: usize) -> Vec<Page> {
    let mut p = Pager { cols: cols.max(8), rows: rows.max(2), pages: Vec::new(), chapter: 0 };
    for (n, chapter) in book.chapters.iter().enumerate() {
        p.chapter = n;
        let mut offset = 0;
        for block in &chapter.blocks {
            let (style, indent, first) = match block.kind {
                Kind::Heading(_) => (Style::Heading, 0, ""),
                Kind::Quote => (Style::Quote, 2, ""),
                Kind::Item => (Style::Body, 2, "- "),
                Kind::Pre => (Style::Pre, 0, ""),
                Kind::Para => (Style::Body, 0, ""),
            };
            if p.pages.last().is_some_and(|pg| pg.chapter == n) { p.line(String::new(), style, offset); }
            if style == Style::Pre {
                let mut at = offset;
                for raw in block.text.split('\n') {
                    let chars: Vec<char> = raw.chars().collect();
                    if chars.is_empty() { p.line(String::from(" "), style, at); }
                    for (k, piece) in chars.chunks(p.cols).enumerate() { p.line(piece.iter().collect(), style, at + k * p.cols); }
                    at += chars.len() + 1;
                }
            } else {
                let width = p.cols - indent;
                for (k, (at, text)) in wrap(&block.text, width).into_iter().enumerate() {
                    let mut line = String::new();
                    for _ in 0..indent.saturating_sub(if k == 0 { first.len() } else { 0 }) { line.push(' '); }
                    if k == 0 { line.push_str(first); }
                    line.push_str(&text);
                    p.line(line, style, offset + at);
                }
            }
            offset += block.text.chars().count() + 1;
        }
        if !p.pages.last().is_some_and(|pg| pg.chapter == n) {
            // An empty chapter still has a page, so the TOC can open it.
            p.pages.push(Page { chapter: n, start: 0, lines: Vec::new() });
        }
    }
    p.pages
}

/// The page holding `pos`: the last page that starts at or before it.
pub fn page_of(pages: &[Page], pos: Position) -> usize {
    pages.iter().rposition(|p| (p.chapter, p.start) <= (pos.chapter, pos.offset)).unwrap_or(0)
}

/// The first page of `chapter` (or the last page before it, if it has none).
pub fn chapter_page(pages: &[Page], chapter: usize) -> usize { page_of(pages, Position { chapter, offset: 0 }) }

impl Page {
    pub fn position(&self) -> Position { Position { chapter: self.chapter, offset: self.start } }
}
