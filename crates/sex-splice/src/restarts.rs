//! Restarts: a phrase said, then said again a moment later.
//!
//! Adapted from Scarlet's `restarts.ts` (MIT, github.com/Forward-Future/scarlet). A speaker
//! who stumbles starts the sentence over ("there's this like white, I don't know what that
//! is, there's this like white stuff"); a retake can sit beside the take that replaced it.
//! Both leave the same words twice within a few seconds. Whether the first saying should go
//! takes the sentence to tell (it may be a deliberate parallel), so this module only locates
//! them; a clean-up pass offers each one to the editor as a card.
//!
//! What counts, as Scarlet tuned it on real interviews:
//! - three matching words or more (at least two of them different), or two different words
//!   with at most one word between: a shorter match is ordinary speech ("you can ... you can");
//! - the second saying starts within 12 words and 12 seconds of the end of the first;
//! - a whole first attempt said again with one word slipped in or left out after its first
//!   word ("how mechanical watch, how a mechanical watch"), on three words that match;
//! - a contraction compares as the word it starts with ("and then I will ... and then I'll");
//! - each saying is the second of at most one phrase, and a third saying pairs with the second.

use crate::transcript::{norm, SourceCut, Transcript};
use crate::{Micros, SECOND};
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

/// `length` words from `at`, said again from `next`. `again` is the second saying's length
/// when it adds or drops a word; otherwise the second saying is `length` words too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restart {
    pub at: usize,
    pub length: usize,
    pub next: usize,
    pub again: Option<usize>,
}

impl Restart {
    /// The words a clean-up removes: the first saying and anything said before the second
    /// one starts, `at..=next - 1`.
    pub fn first_take(&self) -> (usize, usize) {
        (self.at, self.next - 1)
    }
}

/// How far after the first saying ends the second may begin.
pub const RESTART_WITHIN_SECONDS: Micros = 12 * SECOND;
/// How many words may be said between the two sayings.
pub const RESTART_WITHIN_WORDS: usize = 12;

/// Contractions that compare as the word they start with.
const CONTRACTED: &[(&str, &str)] = &[
    ("ill", "i"),
    ("im", "i"),
    ("ive", "i"),
    ("id", "i"),
    ("youll", "you"),
    ("youre", "you"),
    ("youve", "you"),
    ("youd", "you"),
    ("theyre", "they"),
    ("theyll", "they"),
    ("theyve", "they"),
    ("itll", "it"),
    ("its", "it"),
    ("thats", "that"),
    ("whats", "what"),
    ("theres", "there"),
    ("heres", "here"),
    ("lets", "let"),
    ("weve", "we"),
    ("dont", "do"),
    ("doesnt", "does"),
    ("didnt", "did"),
    ("cant", "can"),
    ("wont", "will"),
    ("isnt", "is"),
];

/// The comparison key of a spoken word: normalised, apostrophes dropped, contraction folded.
pub fn restart_key(text: &str) -> String {
    let k: String = norm(text).chars().filter(|c| *c != '\'').collect();
    match CONTRACTED.iter().find(|(c, _)| *c == k) {
        Some((_, w)) => String::from(*w),
        None => k,
    }
}

fn distinct(keys: &[String]) -> usize {
    keys.iter().collect::<BTreeSet<_>>().len()
}

/// Phrases said twice within a moment, in spoken order. `keys` are [`restart_key`]s (an
/// empty key never matches); `starts` and `ends` are each word's times.
pub fn find_restarts(keys: &[String], starts: &[Micros], ends: &[Micros]) -> Vec<Restart> {
    find_restarts_with(keys, starts, ends, false)
}

/// [`find_restarts`]; with `cut_off_words`, a first attempt may also stop on a word the next
/// one says longer ("so perfect, so perfectly realistic", "we want, we wanted"): a word of
/// three letters or more that the second saying's word starts with, when it is the last word
/// before the second saying (Scarlet, 2026-11-20).
pub fn find_restarts_with(keys: &[String], starts: &[Micros], ends: &[Micros], cut_off_words: bool) -> Vec<Restart> {
    let n = keys.len().min(starts.len()).min(ends.len());
    let same = |a: usize, b: usize| !keys[a].is_empty() && keys[a] == keys[b];
    let mut found = Vec::new();
    let mut paired = BTreeSet::new();
    let mut i = 0;
    while i + 1 < n {
        let mut best: Option<Restart> = None;
        if !keys[i].is_empty() {
            let mut j = i + 2;
            while j < n && j <= i + 2 * RESTART_WITHIN_WORDS + 2 {
                if !same(i, j) || paired.contains(&j) {
                    j += 1;
                    continue;
                }
                let mut length = 0;
                while i + length < j && j + length < n && same(i + length, j + length) {
                    length += 1;
                }
                if cut_off_words && length > 0 && i + length + 1 == j && j + length < n {
                    let (cut, full) = (&keys[i + length], &keys[j + length]);
                    if cut.chars().count() >= 3 && full.len() > cut.len() && full.starts_with(cut.as_str()) {
                        length += 1;
                    }
                }
                let between = j - (i + length);
                let d = distinct(&keys[i..i + length]);
                if between <= RESTART_WITHIN_WORDS
                    && starts[j] - ends[i + length - 1] <= RESTART_WITHIN_SECONDS
                    && ((length >= 3 && d >= 2) || (length == 2 && d == 2 && between <= 1))
                {
                    if best.map_or(true, |b| length > b.length) {
                        best = Some(Restart { at: i, length, next: j, again: None });
                    }
                    j += 1;
                    continue;
                }
                // The whole first attempt again, a word slipped in or left out after its first word.
                for (skip_a, skip_b) in [(0, 1), (1, 0)] {
                    let (mut a, mut b, mut more) = (i + length + skip_a, j + length + skip_b, 0);
                    while a < j && b < n && same(a, b) {
                        a += 1;
                        b += 1;
                        more += 1;
                    }
                    if a != j
                        || length + more < 3
                        || more < 1
                        || distinct(&keys[i..j]) < 2
                        || starts[j] - ends[j - 1] > RESTART_WITHIN_SECONDS
                    {
                        continue;
                    }
                    if best.map_or(true, |r| j - i > r.length) {
                        best = Some(Restart { at: i, length: j - i, next: j, again: Some(b - j) });
                    }
                    break;
                }
                j += 1;
            }
        }
        match best {
            Some(r) => {
                found.push(r);
                paired.insert(r.next);
                i = r.at + r.length;
            }
            None => i += 1,
        }
    }
    found
}

impl Transcript {
    /// The transcript's restarts (see the module notes), a first try cut off mid-word
    /// included, since each one goes to the editor as a card to take or skip.
    pub fn restarts(&self) -> Vec<Restart> {
        let keys: Vec<String> = self.words.iter().map(|w| restart_key(&w.text)).collect();
        let starts: Vec<Micros> = self.words.iter().map(|w| w.source.start).collect();
        let ends: Vec<Micros> = self.words.iter().map(|w| w.source.end).collect();
        find_restarts_with(&keys, &starts, &ends, true)
    }

    /// A word said twice straight off ("the the"), outside a restart: the index of the
    /// first saying, which is the one to take out, so the word plays once in the cleaner
    /// second delivery (Scarlet, 2026-11-20). Doubles that are often meant ("had had", "that
    /// that", "no no", "bye bye") are left alone, as are doubles with a pause between.
    pub fn doubled_words(&self) -> Vec<usize> {
        const MEANT: &[&str] = &["had", "that", "no", "yes", "yeah", "bye", "very", "so", "really", "go", "ha", "oh", "is", "do"];
        // Plain words, contractions not folded: "can't, can" is not a double.
        let keys: Vec<String> = self.words.iter().map(|w| norm(&w.text).replace('\'', "")).collect();
        (0..keys.len().saturating_sub(1))
            .filter(|&i| {
                !keys[i].is_empty()
                    && keys[i] == keys[i + 1]
                    && !MEANT.contains(&keys[i].as_str())
                    && self.words[i + 1].source.start - self.words[i].source.end < SECOND / 2
                    && (i + 2 >= keys.len() || keys[i + 2] != keys[i])
                    && (i == 0 || keys[i - 1] != keys[i])
            })
            .collect()
    }

    /// The cut that removes a restart's first saying, so the second one plays.
    pub fn restart_cut(&self, r: &Restart) -> Option<SourceCut> {
        let (first, last) = r.first_take();
        self.cut_words(first, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Word;
    use crate::Range;
    use std::vec;

    fn said(text: &str) -> Transcript {
        let words = text
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| {
                let t = i as Micros * 300_000;
                Word { text: w.into(), source: Range::new(t, t + 250_000) }
            })
            .collect();
        Transcript { media: 1, words }
    }

    fn quote(t: &Transcript, a: usize, len: usize) -> String {
        t.words[a..a + len].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn a_restart_with_words_between_is_found_and_its_cut_ends_at_the_second_saying() {
        let t = said("there's this like white, I don't know what that is, there's this like white stuff");
        let r = t.restarts();
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(quote(&t, r[0].at, r[0].length), "there's this like white,");
        assert_eq!(r[0].next, 10);
        let cut = t.restart_cut(&r[0]).unwrap();
        assert_eq!(cut.range, Range::new(0, t.words[10].source.start));
    }

    #[test]
    fn two_words_count_only_with_at_most_one_between() {
        assert_eq!(said("you can you can see it").restarts().len(), 1);
        assert_eq!(said("you can do it and you can").restarts(), vec![]);
        // Two of the same word is not a phrase.
        assert_eq!(said("no no no no").restarts(), vec![]);
    }

    #[test]
    fn a_contraction_matches_the_word_it_starts_with() {
        let t = said("and then I will, and then I'll copy it");
        let r = t.restarts();
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!((r[0].at, r[0].length, r[0].next), (0, 3, 4));
    }

    #[test]
    fn a_first_attempt_with_a_word_added_or_dropped_is_one_restart() {
        let t = said("how mechanical watch, how a mechanical watch works");
        let r = t.restarts();
        assert_eq!(r, vec![Restart { at: 0, length: 3, next: 3, again: Some(4) }]);
        let t = said("how a mechanical watch, how mechanical watch works");
        let r = t.restarts();
        assert_eq!(r, vec![Restart { at: 0, length: 4, next: 4, again: Some(3) }]);
    }

    #[test]
    fn too_far_apart_in_words_or_seconds_is_ordinary_speech() {
        let filler = "a b c d e f g h i j k l m";
        assert_eq!(said(&std::format!("we went there {filler} we went there")).restarts(), vec![]);
        let mut t = said("we went there and we went there");
        for w in &mut t.words[4..] {
            w.source.start += 20 * SECOND;
            w.source.end += 20 * SECOND;
        }
        assert_eq!(t.restarts(), vec![]);
    }

    #[test]
    fn a_first_try_cut_off_on_a_word_the_retake_says_longer() {
        let t = said("Okay. So perfect. So perfectly realistic.");
        assert_eq!(t.restarts(), vec![Restart { at: 1, length: 2, next: 3, again: None }]);
        let keys: Vec<String> = t.words.iter().map(|w| restart_key(&w.text)).collect();
        let times: Vec<Micros> = t.words.iter().map(|w| w.source.start).collect();
        assert_eq!(find_restarts(&keys, &times, &times), vec![], "only with cut-off words");
        // Two letters is too short to say it was cut off.
        assert_eq!(said("so we go so we gone").restarts().len(), 1, "\"so we\" matches on its own");
        assert_eq!(said("we want, we wanted it").restarts(), vec![Restart { at: 0, length: 2, next: 2, again: None }]);
    }

    #[test]
    fn a_doubled_word_takes_out_the_first() {
        assert_eq!(said("I went to the the shop").doubled_words(), vec![3]);
        assert_eq!(said("what he had had was gone").doubled_words(), vec![]);
        assert_eq!(said("no no no").doubled_words(), vec![]);
        assert_eq!(said("you can't can you").doubled_words(), vec![], "a contraction is its own word here");
        let mut t = said("to the the shop");
        t.words[2].source.start += SECOND;
        assert_eq!(t.doubled_words(), vec![], "a pause between is a choice");
    }

    #[test]
    fn a_third_saying_pairs_with_the_second() {
        let t = said("I think that I think that I think that is right");
        let r = t.restarts();
        assert_eq!(r.iter().map(|r| (r.at, r.next)).collect::<Vec<_>>(), vec![(0, 3), (3, 6)]);
    }
}
