// SPDX-License-Identifier: GPL-3.0-only
//! Splitting a run of Pinyin letters into syllables: `nihao` is `ni` `hao`,
//! `xian` is one syllable, `xi'an` would be typed `xi` then a choice.
use crate::lookup;

/// Every Mandarin syllable is at most six letters (`zhuang`, `shuang`).
const LONGEST: usize = 6;

pub fn is_syllable(s: &str) -> bool { !s.is_empty() && s.len() <= LONGEST && !s.contains('-') && lookup(s).is_some() }

/// Ends of the syllables of `letters` into `ends` (fewest syllables, preferring a
/// longer first syllable); returns how many. A trailing part that is no syllable
/// yet (still being typed) counts as one.
pub fn split(letters: &str, ends: &mut [usize]) -> usize {
    let n = letters.len();
    if n == 0 || n > 48 || ends.is_empty() { return 0; }
    // best[i]: fewest syllables covering letters[i..]; next[i]: end of the first.
    let mut best = [u8::MAX; 49];
    let mut next = [0usize; 49];
    best[n] = 0;
    for i in (0..n).rev() {
        for len in (1..=LONGEST.min(n - i)).rev() {
            let j = i + len;
            if best[j] != u8::MAX && is_syllable(&letters[i..j]) && best[j] + 1 < best[i] {
                best[i] = best[j] + 1;
                next[i] = j;
            }
        }
    }
    let mut count = 0;
    let mut i = 0;
    while i < n && count < ends.len() {
        if best[i] == u8::MAX {
            // The rest is not (yet) syllables: one unfinished part.
            ends[count] = n; count += 1; break;
        }
        ends[count] = next[i]; count += 1; i = next[i];
    }
    count
}
