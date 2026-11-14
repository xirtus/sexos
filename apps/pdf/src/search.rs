// SPDX-License-Identifier: GPL-3.0-only
//! Finding words in the document's text: pure, so the host tests it without a window.
//!
//! Each page's text comes from [`crate::text`] as characters in reading order. A hit is
//! a page and the index of its first character there, so the viewer can turn to that
//! page and light up the boxes of exactly those characters.

/// One place the term occurs: page (from 0) and its first character on that page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit { pub page: usize, pub at: usize }

/// Folds case one character to one character, so indices into the folded text are
/// indices into the page's own characters (a full `to_lowercase` may change lengths).
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) { (Some(l), None) => l, _ => c }
}

/// Where `term` starts in `text`, ignoring case, in character indices. Overlapping
/// occurrences each count: "aa" occurs twice in "aaa", as a reader's eye would find them.
pub fn find_all(text: &[char], term: &str) -> Vec<usize> {
    let needle: Vec<char> = term.chars().map(fold).collect();
    if needle.is_empty() || needle.len() > text.len() { return Vec::new(); }
    let hay: Vec<char> = text.iter().map(|c| fold(*c)).collect();
    (0..=hay.len() - needle.len()).filter(|&i| hay[i..i + needle.len()] == needle[..]).collect()
}

/// Every hit in the document, in page order.
pub fn hits(pages: &[Vec<char>], term: &str) -> Vec<Hit> {
    pages.iter().enumerate().flat_map(|(page, text)| find_all(text, term).into_iter().map(move |at| Hit { page, at })).collect()
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
    }

    #[test]
    fn steps_through_pages_and_wraps() {
        let pages = vec![chars("one fox"), chars("nothing"), chars("fox and fox")];
        let all = hits(&pages, "FOX");
        assert_eq!(all, vec![Hit { page: 0, at: 4 }, Hit { page: 2, at: 0 }, Hit { page: 2, at: 8 }]);
        // From the page being read, before any hit was chosen.
        assert_eq!(next(&all, None, 1), Some(Hit { page: 2, at: 0 }));
        assert_eq!(next(&all, Some(all[1]), 1), Some(all[2]));
        assert_eq!(next(&all, Some(all[2]), 2), Some(all[0]));
        assert_eq!(previous(&all, Some(all[0]), 0), Some(all[2]));
        assert_eq!(previous(&all, None, 2), Some(all[0]));
        assert_eq!(next(&[], None, 0), None);
    }
}
