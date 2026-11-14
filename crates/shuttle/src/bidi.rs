// SPDX-License-Identifier: GPL-3.0-only
//! Arbitrary-length multi-paragraph Unicode Bidirectional Algorithm (UAX #9).
//!
//! Provides paragraph splitting, base direction resolution, full rule evaluation
//! (weak types, neutral types, paired brackets, implicit levels), and visual run
//! reordering (L2) with character mirroring for text of any length.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use sex_shape::bidi::{class, mirror, Class};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseDirection {
    Auto,
    Ltr,
    Rtl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRun {
    pub char_range: Range<usize>,
    pub level: u8,
    pub direction: Direction,
}

#[derive(Clone, Debug)]
pub struct Paragraph<'a> {
    pub text: &'a str,
    pub byte_range: Range<usize>,
    pub base_level: u8,
    pub levels: Vec<u8>,
}

impl<'a> Paragraph<'a> {
    /// The direction of this paragraph based on its base embedding level.
    pub fn direction(&self) -> Direction {
        if self.base_level % 2 == 1 {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }

    /// Splits `text` into paragraphs and computes character embedding levels for each.
    pub fn split_and_resolve(text: &'a str, base_dir: BaseDirection) -> Vec<Paragraph<'a>> {
        let mut paras = Vec::new();
        let mut start = 0;
        for (i, c) in text.char_indices() {
            if c == '\n' || c == '\r' || c == '\u{2029}' {
                let end = i;
                let p_text = &text[start..end];
                let (base_level, levels) = resolve_levels(p_text, base_dir);
                paras.push(Paragraph {
                    text: p_text,
                    byte_range: start..end,
                    base_level,
                    levels,
                });
                start = i + c.len_utf8();
            }
        }

        if start <= text.len() {
            let p_text = &text[start..text.len()];
            let (base_level, levels) = resolve_levels(p_text, base_dir);
            paras.push(Paragraph {
                text: p_text,
                byte_range: start..text.len(),
                base_level,
                levels,
            });
        }

        paras
    }

    /// Resolves visual runs for a given character slice `char_range` within this paragraph.
    pub fn visual_runs(&self, char_range: Range<usize>) -> Vec<VisualRun> {
        if char_range.is_empty() || char_range.start >= self.levels.len() {
            return Vec::new();
        }
        let end = char_range.end.min(self.levels.len());
        let slice = &self.levels[char_range.start..end];
        let n = slice.len();

        let mut runs: Vec<VisualRun> = Vec::new();
        let mut run_start = 0;
        while run_start < n {
            let lvl = slice[run_start];
            let mut run_end = run_start + 1;
            while run_end < n && slice[run_end] == lvl {
                run_end += 1;
            }
            runs.push(VisualRun {
                char_range: (char_range.start + run_start)..(char_range.start + run_end),
                level: lvl,
                direction: if lvl % 2 == 1 { Direction::Rtl } else { Direction::Ltr },
            });
            run_start = run_end;
        }

        reorder_visual_runs(&mut runs);
        runs
    }
}

/// Resolves character embedding levels for an arbitrary-length single line/paragraph of text.
pub fn resolve_levels(line: &str, base_dir: BaseDirection) -> (u8, Vec<u8>) {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    if n == 0 {
        let base = match base_dir {
            BaseDirection::Rtl => 1,
            _ => 0,
        };
        return (base, Vec::new());
    }

    let orig_types: Vec<Class> = chars.iter().map(|&c| class(c)).collect();
    let mut t = orig_types.clone();

    // Determine base paragraph level (P2 / P3)
    let base_level: u8 = match base_dir {
        BaseDirection::Ltr => 0,
        BaseDirection::Rtl => 1,
        BaseDirection::Auto => {
            orig_types
                .iter()
                .find(|&&c| matches!(c, Class::L | Class::R | Class::AL))
                .map_or(0, |&c| if c == Class::L { 0 } else { 1 })
        }
    };

    let e = if base_level == 1 { Class::R } else { Class::L };
    let live = |i: usize, types: &[Class]| types[i] != Class::BN;

    // W1: NSM takes type of previous character
    let mut prev = e;
    for i in 0..n {
        if !live(i, &t) {
            continue;
        }
        if t[i] == Class::NSM {
            t[i] = prev;
        }
        prev = t[i];
    }

    // W2: EN after AL becomes AN
    let mut strong = e;
    for i in 0..n {
        match t[i] {
            Class::L | Class::R | Class::AL => strong = t[i],
            Class::EN if strong == Class::AL => t[i] = Class::AN,
            _ => {}
        }
    }

    // W3: AL becomes R
    for i in 0..n {
        if t[i] == Class::AL {
            t[i] = Class::R;
        }
    }

    // W4: Single ES or CS between two digits joins them
    let prev_live = |i: usize, types: &[Class]| (0..i).rev().find(|&j| live(j, types)).map(|j| types[j]);
    let next_live = |i: usize, types: &[Class]| (i + 1..n).find(|&j| live(j, types)).map(|j| types[j]);

    for i in 0..n {
        let p = prev_live(i, &t);
        let q = next_live(i, &t);
        match t[i] {
            Class::ES if p == Some(Class::EN) && q == Some(Class::EN) => t[i] = Class::EN,
            Class::CS if p == Some(Class::EN) && q == Some(Class::EN) => t[i] = Class::EN,
            Class::CS if p == Some(Class::AN) && q == Some(Class::AN) => t[i] = Class::AN,
            _ => {}
        }
    }

    // W5: Runs of ET adjacent to EN become EN
    let mut i = 0;
    while i < n {
        if t[i] == Class::ET || (t[i] == Class::BN && i > 0 && t[i - 1] == Class::ET) {
            let start = i;
            while i < n && (t[i] == Class::ET || t[i] == Class::BN) {
                i += 1;
            }
            let before = prev_live(start, &t) == Some(Class::EN);
            let after = i < n && t[i] == Class::EN;
            if before || after {
                for k in start..i {
                    if t[k] == Class::ET {
                        t[k] = Class::EN;
                    }
                }
            }
        } else {
            i += 1;
        }
    }

    // W6: Remaining separators become ON
    for i in 0..n {
        if matches!(t[i], Class::ES | Class::ET | Class::CS) {
            t[i] = Class::ON;
        }
    }

    // W7: EN after L becomes L
    let mut strong_dir = e;
    for i in 0..n {
        match t[i] {
            Class::L | Class::R => strong_dir = t[i],
            Class::EN if strong_dir == Class::L => t[i] = Class::L,
            _ => {}
        }
    }

    let dir = |c: Class| match c {
        Class::L => Some(Class::L),
        Class::R | Class::EN | Class::AN => Some(Class::R),
        _ => None,
    };

    // N0: Paired brackets take direction of enclosed text
    let opening = |c: char| matches!(c, '(' | '[' | '{' | '<' | '«' | '‹');
    let closing_for = |c: char| match c {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        '«' => '»',
        '‹' => '›',
        _ => '\0',
    };

    let mut stack: Vec<(usize, char)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();

    for i in 0..n {
        if t[i] != Class::ON {
            continue;
        }
        let c = chars[i];
        if opening(c) {
            stack.push((i, closing_for(c)));
        } else if let Some(pos) = stack.iter().rposition(|&(_, expected)| expected == c) {
            pairs.push((stack[pos].0, i));
            stack.truncate(pos);
        }
    }

    pairs.sort_unstable_by_key(|&(open, _)| open);

    for &(open, close) in &pairs {
        let inside = (open + 1..close).filter_map(|j| dir(t[j]));
        let mut found = None;
        for d in inside {
            if d == e {
                found = Some(e);
                break;
            }
            found = Some(d);
        }

        let to = match found {
            Some(d) if d == e => Some(e),
            Some(d) => {
                let before = (0..open).rev().find_map(|j| dir(t[j])).unwrap_or(e);
                Some(if before == d { d } else { e })
            }
            None => None,
        };

        if let Some(d) = to {
            t[open] = d;
            t[close] = d;
            for k in [open, close] {
                let mut j = k + 1;
                while j < n && orig_types[j] == Class::NSM {
                    t[j] = d;
                    j += 1;
                }
            }
        }
    }

    // N1 / N2: Neutrals take surrounding direction if consistent, else base direction
    let neutral = |c: Class| matches!(c, Class::B | Class::S | Class::WS | Class::ON | Class::BN);
    let mut i = 0;
    while i < n {
        if !neutral(t[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && neutral(t[i]) {
            i += 1;
        }
        let before = if start == 0 {
            e
        } else {
            dir(t[start - 1]).unwrap_or(e)
        };
        let after = if i < n { dir(t[i]).unwrap_or(e) } else { e };
        let to = if before == after { before } else { e };
        for k in start..i {
            t[k] = to;
        }
    }

    // I1 / I2: Implicit levels
    let mut out = vec![0u8; n];
    for i in 0..n {
        out[i] = match (base_level, t[i]) {
            (0, Class::R) => 1,
            (0, Class::AN | Class::EN) => 2,
            (0, _) => 0,
            (_, Class::L | Class::EN | Class::AN) => 2,
            _ => 1,
        };
    }

    // L1: Segment separators & trailing whitespace reset to paragraph level
    let mut trailing = true;
    for i in (0..n).rev() {
        match orig_types[i] {
            Class::S | Class::B => {
                out[i] = base_level;
                trailing = true;
            }
            Class::WS | Class::BN if trailing => {
                out[i] = base_level;
            }
            _ => {
                trailing = false;
            }
        }
    }

    (base_level, out)
}

/// Reorders visual runs in-place by embedding level (UAX #9 Rule L2).
pub fn reorder_visual_runs(runs: &mut [VisualRun]) {
    if runs.len() <= 1 {
        return;
    }

    let high = runs.iter().map(|r| r.level).max().unwrap_or(0);
    let low_odd = runs
        .iter()
        .map(|r| r.level)
        .filter(|&l| l % 2 == 1)
        .min()
        .unwrap_or(high + 1);

    let mut level = high;
    while level >= low_odd && level > 0 {
        let mut i = 0;
        while i < runs.len() {
            if runs[i].level >= level {
                let start = i;
                while i < runs.len() && runs[i].level >= level {
                    i += 1;
                }
                runs[start..i].reverse();
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
}

/// Applies character mirroring for right-to-left runs.
pub fn mirror_char(c: char, is_rtl: bool) -> char {
    if is_rtl {
        mirror(c)
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn test_arbitrary_length_bidi() {
        let long_english = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        let long_arabic = "السماء صافية والشمس مشرقة اليوم. ".repeat(10);
        let combined = format!("{long_english} {long_arabic} {long_english}");
        assert!(combined.chars().count() > 600);

        let (base, levels) = resolve_levels(&combined, BaseDirection::Auto);
        assert_eq!(base, 0);
        assert_eq!(levels.len(), combined.chars().count());

        let paras = Paragraph::split_and_resolve(&combined, BaseDirection::Auto);
        assert_eq!(paras.len(), 1);
        let runs = paras[0].visual_runs(0..levels.len());
        assert!(!runs.is_empty());
    }

    #[test]
    fn test_mixed_hebrew_english() {
        let text = "Hello שלום World";
        let (base, levels) = resolve_levels(text, BaseDirection::Auto);
        assert_eq!(base, 0);
        let chars: Vec<char> = text.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if ('\u{0590}'..='\u{05FF}').contains(c) {
                assert_eq!(levels[i], 1, "Hebrew character {c} should have level 1");
            }
        }
    }
}
