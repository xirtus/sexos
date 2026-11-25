// SPDX-License-Identifier: GPL-3.0-only
//! Finding words in the document's text: pure, so the host tests it without a window.
//!
//! Each page's text comes from [`crate::text`] as characters in reading order. A hit is
//! a page and the index of its first character there, so the viewer can turn to that
//! page and light up the boxes of exactly those characters.

/// One place the term occurs: page (from 0), first character and source span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit { pub page: usize, pub at: usize, pub len: usize }

/// Folds case one character to one character, so indices into the folded text are
/// indices into the page's own characters (a full `to_lowercase` may change lengths).
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) { (Some(l), None) => l, _ => c }
}

/// Collapse each whitespace run to one space, keeping the range of source
/// characters it replaced so highlights can cover the actual extracted text.
fn normalized(chars: impl IntoIterator<Item = char>, match_case: bool) -> Vec<(char, usize, usize)> {
    let mut out: Vec<(char, usize, usize)> = Vec::new();
    for (at, c) in chars.into_iter().enumerate() {
        if c.is_whitespace() {
            if let Some((last, _, end)) = out.last_mut() {
                if *last == ' ' { *end = at + 1; continue; }
            }
            out.push((' ', at, at + 1));
        } else { out.push((if match_case { c } else { fold(c) }, at, at + 1)); }
    }
    out
}

fn spans(text: &[char], term: &str, match_case: bool) -> Vec<(usize, usize)> {
    let needle: Vec<char> = normalized(term.chars(), match_case).into_iter().map(|x| x.0).collect();
    let first = needle.iter().position(|c| *c != ' ');
    let Some(first) = first else { return Vec::new(); };
    let last = needle.iter().rposition(|c| *c != ' ').unwrap();
    let needle = &needle[first..=last];
    if needle.is_empty() { return Vec::new(); }
    let hay = normalized(text.iter().copied(), match_case);
    if needle.len() > hay.len() { return Vec::new(); }
    (0..=hay.len() - needle.len())
        .filter(|&i| hay[i..i + needle.len()].iter().map(|x| x.0).eq(needle.iter().copied()))
        .map(|i| (hay[i].1, hay[i + needle.len() - 1].2)).collect()
}

/// Where `term` starts in `text`, ignoring case and layout whitespace, in
/// character indices. Overlapping occurrences each count.
pub fn find_all(text: &[char], term: &str) -> Vec<usize> {
    spans(text, term, false).into_iter().map(|(at, _)| at).collect()
}

/// Every hit in the document, in page order.
pub fn hits(pages: &[Vec<char>], term: &str) -> Vec<Hit> {
    hits_with_case(pages, term, false)
}

/// Every hit in page order, optionally retaining the PDF's original letter case.
pub fn hits_with_case(pages: &[Vec<char>], term: &str, match_case: bool) -> Vec<Hit> {
    pages.iter().enumerate().flat_map(|(page, text)| spans(text, term, match_case).into_iter().map(move |(at, end)| Hit { page, at, len: end - at })).collect()
}

/// The hit after `from` (or the first at or after the start of `page` when there is
/// none yet), going round to the first hit after the last. `None` when there are none.
pub fn next(all: &[Hit], from: Option<Hit>, page: usize) -> Option<Hit> {
    let after = |h: &&Hit| match from { Some(f) => **h > f, None => h.page >= page };
    all.iter().find(after).or_else(|| all.first()).copied()
}

/// The hit before `from`, going round to the last. Shift+F3 and Shift+Enter.
pub fn previous(all: &[Hit], from: Option<Hit>, page: usize) -> Option<Hit> {
    let before = |h: &&Hit| match from { Some(f) => **h < f, None => h.page < page };
    all.iter().rev().find(before).or_else(|| all.last()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chars(s: &str) -> Vec<char> { s.chars().collect() }

    #[test]
    fn finds_every_occurrence_ignoring_case() {
        assert_eq!(find_all(&chars("Lantern, lantern; LANTERN"), "lantern"), vec![0, 9, 18]);
        assert_eq!(find_all(&chars("aaa"), "aa"), vec![0, 1]);
        assert!(find_all(&chars("short"), "").is_empty());
        assert!(find_all(&chars("ab"), "abc").is_empty());
        // Indices stay those of the page's own characters after folding.
        assert_eq!(find_all(&chars("ÄÖü x"), "x"), vec![4]);
        let page = chars("First line\n\nSecond line");
        assert_eq!(find_all(&page, "line second"), vec![6]);
        assert_eq!(hits(&[page], "line second"), vec![Hit { page: 0, at: 6, len: 12 }]);
        assert!(find_all(&chars("one two"), "   ").is_empty());
        assert_eq!(find_all(&chars("one\t\ntwo"), " one   two "), vec![0]);
    }

    #[test]
    fn steps_through_pages_and_wraps() {
        let pages = vec![chars("one fox"), chars("nothing"), chars("fox and fox")];
        let all = hits(&pages, "FOX");
        assert_eq!(all, vec![Hit { page: 0, at: 4, len: 3 }, Hit { page: 2, at: 0, len: 3 }, Hit { page: 2, at: 8, len: 3 }]);
        // From the page being read, before any hit was chosen.
        assert_eq!(next(&all, None, 1), Some(Hit { page: 2, at: 0, len: 3 }));
        assert_eq!(next(&all, Some(all[1]), 1), Some(all[2]));
        assert_eq!(next(&all, Some(all[2]), 2), Some(all[0]));
        assert_eq!(previous(&all, Some(all[0]), 0), Some(all[2]));
        assert_eq!(previous(&all, None, 2), Some(all[0]));
        assert_eq!(next(&[], None, 0), None);
    }

    #[test]
    fn match_case_filters_hits_without_changing_source_spans() {
        let pages = vec![chars("Lantern lantern\nLANTERN"), chars("lantern")];
        assert_eq!(hits_with_case(&pages, "Lantern", true), vec![Hit { page: 0, at: 0, len: 7 }]);
        assert_eq!(hits_with_case(&pages, "lantern", true), vec![Hit { page: 0, at: 8, len: 7 }, Hit { page: 1, at: 0, len: 7 }]);
        assert_eq!(hits_with_case(&pages, "lantern", false).len(), 4);
        assert_eq!(hits_with_case(&[chars("A\nB")], "A B", true), vec![Hit { page: 0, at: 0, len: 3 }]);
    }

    #[test]
    fn accented_query_finds_the_matching_pdf_text() {
        let pages = vec![chars("The cafe is closed"), chars("A café appears here")];
        assert_eq!(hits(&pages, "café"), vec![Hit { page: 1, at: 2, len: 4 }]);
    }
}
