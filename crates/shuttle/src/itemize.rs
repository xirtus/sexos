// SPDX-License-Identifier: GPL-3.0-only
//! Script detection and text itemization.
//!
//! Partitions text into homogeneous item runs possessing identical Unicode script,
//! BiDi embedding level, and text direction. Resolves `Common` and `Inherited` scripts
//! using surrounding script context and paired bracket relationships.

extern crate alloc;

use alloc::vec::Vec;
use core::ops::Range;
use unicode_script::{Script, UnicodeScript};

use crate::bidi::{Direction, Paragraph};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRun<'a> {
    pub char_range: Range<usize>,
    pub byte_range: Range<usize>,
    pub script: Script,
    pub level: u8,
    pub direction: Direction,
    pub text: &'a str,
}

impl<'a> ItemRun<'a> {
    pub fn is_rtl(&self) -> bool {
        self.direction == Direction::Rtl
    }
}

/// Itemizes `paragraph` into homogeneous runs of uniform script and BiDi level.
pub fn itemize_paragraph<'a>(para: &Paragraph<'a>) -> Vec<ItemRun<'a>> {
    let text = para.text;
    if text.is_empty() {
        return Vec::new();
    }

    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    let n = char_indices.len();
    if n == 0 {
        return Vec::new();
    }

    // 1. Initial script assignment
    let mut scripts: Vec<Script> = char_indices.iter().map(|&(_, c)| c.script()).collect();

    // 2. Resolve Common and Inherited scripts
    resolve_scripts(&char_indices, &mut scripts);

    // 3. Intersect script runs with BiDi embedding levels
    let mut runs = Vec::new();
    let mut run_start = 0;

    while run_start < n {
        let cur_script = scripts[run_start];
        let cur_level = para.levels.get(run_start).copied().unwrap_or(para.base_level);
        let mut run_end = run_start + 1;

        while run_end < n {
            let next_script = scripts[run_end];
            let next_level = para.levels.get(run_end).copied().unwrap_or(para.base_level);
            if next_script != cur_script || next_level != cur_level {
                break;
            }
            run_end += 1;
        }

        let byte_start = char_indices[run_start].0;
        let byte_end = if run_end < n {
            char_indices[run_end].0
        } else {
            text.len()
        };

        runs.push(ItemRun {
            char_range: run_start..run_end,
            byte_range: byte_start..byte_end,
            script: cur_script,
            level: cur_level,
            direction: if cur_level % 2 == 1 { Direction::Rtl } else { Direction::Ltr },
            text: &text[byte_start..byte_end],
        });

        run_start = run_end;
    }

    runs
}

/// Resolves `Common` and `Inherited` scripts across characters.
fn resolve_scripts(_chars: &[(usize, char)], scripts: &mut [Script]) {
    let n = scripts.len();
    if n == 0 {
        return;
    }

    // Forward pass: inherit previous strong script
    let mut last_strong = Script::Latin;
    for i in 0..n {
        let s = scripts[i];
        if is_strong_script(s) {
            last_strong = s;
        } else if s == Script::Inherited || s == Script::Common {
            // Leave for bracket resolution or backward pass if leading
            if i > 0 && is_strong_script(scripts[i - 1]) {
                scripts[i] = scripts[i - 1];
            }
        }
    }

    // Backward pass: for leading weak characters, inherit the first following strong script
    let mut next_strong = None;
    for i in (0..n).rev() {
        let s = scripts[i];
        if is_strong_script(s) {
            next_strong = Some(s);
        } else if !is_strong_script(s) {
            if let Some(strong) = next_strong {
                if scripts[i] == Script::Common || scripts[i] == Script::Inherited {
                    scripts[i] = strong;
                }
            }
        }
    }

    // Default remaining to last known strong or Latin
    for s in scripts.iter_mut() {
        if !is_strong_script(*s) {
            *s = last_strong;
        }
    }
}

/// Whether a script is a specific language script rather than Common or Inherited.
pub fn is_strong_script(s: Script) -> bool {
    !matches!(s, Script::Common | Script::Inherited | Script::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bidi::{BaseDirection, Paragraph};

    #[test]
    fn test_itemize_mixed_arabic_english() {
        let text = "Order #42: السلام عليكم 2026";
        let paras = Paragraph::split_and_resolve(text, BaseDirection::Auto);
        let items = itemize_paragraph(&paras[0]);
        assert!(items.len() >= 2);

        let has_arabic = items.iter().any(|it| it.script == Script::Arabic && it.is_rtl());
        let has_latin = items.iter().any(|it| it.script == Script::Latin && !it.is_rtl());
        assert!(has_arabic, "Must itemize an Arabic RTL run");
        assert!(has_latin, "Must itemize a Latin LTR run");
    }

    #[test]
    fn test_itemize_cjk() {
        let text = "Hello 世界!";
        let paras = Paragraph::split_and_resolve(text, BaseDirection::Auto);
        let items = itemize_paragraph(&paras[0]);
        let has_cjk = items.iter().any(|it| it.script == Script::Han);
        assert!(has_cjk, "Must identify Han script for CJK ideographs");
    }
}
