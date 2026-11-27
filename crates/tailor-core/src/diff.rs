// SPDX-License-Identifier: GPL-3.0-only
//! Pure `#![no_std]` + `alloc` line diff engine with unified diff formatting.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

const CONTEXT_LINES: usize = 3;
const MAX_LCS_LINES: usize = 2000;
const MAX_LCS_CELLS: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

/// Computes unified diff hunks between `old` and `new` with 3 lines of context.
///
/// Uses an LCS dynamic-programming diff after trimming common prefix/suffix lines,
/// and falls back to a linear replace hunk if the differing middle exceeds 2000 lines
/// so it never OOMs in constrained environments like SexOS.
pub fn compute_diff(old: &str, new: &str) -> Vec<DiffHunk> {
    if old == new {
        return Vec::new();
    }

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Trim common prefix.
    let mut prefix = 0usize;
    let min_len = old_lines.len().min(new_lines.len());
    while prefix < min_len && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }

    // Trim common suffix.
    let mut suffix = 0usize;
    while suffix < (old_lines.len() - prefix)
        && suffix < (new_lines.len() - prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];

    if old_mid.is_empty() && new_mid.is_empty() {
        return Vec::new();
    }

    // Build full edit script with 1-based line numbers.
    let mut all_ops: Vec<DiffLine> = Vec::with_capacity(old_lines.len() + new_lines.len());

    for (i, line) in old_lines[..prefix].iter().enumerate() {
        all_ops.push(DiffLine {
            kind: DiffLineKind::Context,
            old_line: Some(i + 1),
            new_line: Some(i + 1),
            text: String::from(*line),
        });
    }

    let use_fallback = old_mid.len() > MAX_LCS_LINES
        || new_mid.len() > MAX_LCS_LINES
        || old_mid.len().saturating_mul(new_mid.len()) > MAX_LCS_CELLS;

    if use_fallback {
        for (i, line) in old_mid.iter().enumerate() {
            all_ops.push(DiffLine {
                kind: DiffLineKind::Removed,
                old_line: Some(prefix + i + 1),
                new_line: None,
                text: String::from(*line),
            });
        }
        for (j, line) in new_mid.iter().enumerate() {
            all_ops.push(DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(prefix + j + 1),
                text: String::from(*line),
            });
        }
    } else {
        lcs_diff(old_mid, new_mid, prefix, &mut all_ops);
    }

    let old_suffix_start = old_lines.len() - suffix;
    let new_suffix_start = new_lines.len() - suffix;
    for i in 0..suffix {
        all_ops.push(DiffLine {
            kind: DiffLineKind::Context,
            old_line: Some(old_suffix_start + i + 1),
            new_line: Some(new_suffix_start + i + 1),
            text: String::from(old_lines[old_suffix_start + i]),
        });
    }

    group_hunks(&all_ops)
}

fn lcs_diff(old_mid: &[&str], new_mid: &[&str], base: usize, out: &mut Vec<DiffLine>) {
    let m = old_mid.len();
    let n = new_mid.len();
    let cols = n + 1;
    let mut dp = vec![0u32; (m + 1) * cols];

    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i * cols + j] = if old_mid[i] == new_mid[j] {
                dp[(i + 1) * cols + (j + 1)] + 1
            } else {
                dp[(i + 1) * cols + j].max(dp[i * cols + (j + 1)])
            };
        }
    }

    let mut i = 0usize;
    let mut j = 0usize;
    while i < m && j < n {
        if old_mid[i] == new_mid[j] {
            out.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(base + i + 1),
                new_line: Some(base + j + 1),
                text: String::from(old_mid[i]),
            });
            i += 1;
            j += 1;
        } else if dp[(i + 1) * cols + j] >= dp[i * cols + (j + 1)] {
            out.push(DiffLine {
                kind: DiffLineKind::Removed,
                old_line: Some(base + i + 1),
                new_line: None,
                text: String::from(old_mid[i]),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(base + j + 1),
                text: String::from(new_mid[j]),
            });
            j += 1;
        }
    }
    while i < m {
        out.push(DiffLine {
            kind: DiffLineKind::Removed,
            old_line: Some(base + i + 1),
            new_line: None,
            text: String::from(old_mid[i]),
        });
        i += 1;
    }
    while j < n {
        out.push(DiffLine {
            kind: DiffLineKind::Added,
            old_line: None,
            new_line: Some(base + j + 1),
            text: String::from(new_mid[j]),
        });
        j += 1;
    }
}

fn group_hunks(ops: &[DiffLine]) -> Vec<DiffHunk> {
    let mut change_ranges: Vec<(usize, usize)> = Vec::new();
    let mut idx = 0usize;
    while idx < ops.len() {
        if ops[idx].kind != DiffLineKind::Context {
            let start = idx;
            while idx < ops.len() && ops[idx].kind != DiffLineKind::Context {
                idx += 1;
            }
            let end = idx;
            let ctx_start = start.saturating_sub(CONTEXT_LINES);
            let ctx_end = (end + CONTEXT_LINES).min(ops.len());
            if let Some(last) = change_ranges.last_mut() {
                if ctx_start <= last.1 {
                    last.1 = ctx_end;
                    continue;
                }
            }
            change_ranges.push((ctx_start, ctx_end));
        } else {
            idx += 1;
        }
    }

    let mut hunks = Vec::with_capacity(change_ranges.len());
    for (start, end) in change_ranges {
        let slice = &ops[start..end];
        let old_count = slice
            .iter()
            .filter(|l| l.kind != DiffLineKind::Added)
            .count();
        let new_count = slice
            .iter()
            .filter(|l| l.kind != DiffLineKind::Removed)
            .count();

        let old_start = slice
            .iter()
            .find_map(|l| l.old_line)
            .unwrap_or_else(|| {
                // Pure insertion: find preceding old line number or 0.
                ops[..start]
                    .iter()
                    .rev()
                    .find_map(|l| l.old_line)
                    .unwrap_or(0)
            });
        let new_start = slice
            .iter()
            .find_map(|l| l.new_line)
            .unwrap_or_else(|| {
                // Pure deletion: find preceding new line number or 0.
                ops[..start]
                    .iter()
                    .rev()
                    .find_map(|l| l.new_line)
                    .unwrap_or(0)
            });

        hunks.push(DiffHunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: slice.to_vec(),
        });
    }
    hunks
}

/// Formats a standard unified diff string for `path` between `old` and `new`.
/// Returns an empty string if there are no line differences.
pub fn format_unified_diff(path: &str, old: &str, new: &str) -> String {
    let hunks = compute_diff(old, new);
    if hunks.is_empty() {
        return String::new();
    }
    let mut out = format!("--- a/{}\n+++ b/{}\n", path, path);
    for hunk in hunks {
        let _ = writeln!(
            &mut out,
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        );
        for line in hunk.lines {
            let prefix = match line.kind {
                DiffLineKind::Context => ' ',
                DiffLineKind::Added => '+',
                DiffLineKind::Removed => '-',
            };
            let _ = writeln!(&mut out, "{}{}", prefix, line.text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_produce_no_hunks() {
        let text = "line 1\nline 2\nline 3\n";
        assert!(compute_diff(text, text).is_empty());
        assert!(format_unified_diff("a.rs", text, text).is_empty());
    }

    #[test]
    fn computes_single_line_replacement_with_context() {
        let old = "a\nb\nc\nd\ne\nf\ng\n";
        let new = "a\nb\nc\nD\ne\nf\ng\n";
        let hunks = compute_diff(old, new);
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(h.old_start, 1);
        assert_eq!(h.old_count, 7);
        assert_eq!(h.new_start, 1);
        assert_eq!(h.new_count, 7);

        let unified = format_unified_diff("src/lib.rs", old, new);
        assert!(unified.contains("--- a/src/lib.rs\n+++ b/src/lib.rs\n"));
        assert!(unified.contains("@@ -1,7 +1,7 @@\n"));
        assert!(unified.contains("-d\n+D\n"));
    }

    #[test]
    fn separates_distant_hunks() {
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        for i in 1..=20 {
            old_lines.push(format!("line {}", i));
            if i == 2 {
                new_lines.push(String::from("LINE 2"));
            } else if i == 19 {
                new_lines.push(String::from("LINE 19"));
            } else {
                new_lines.push(format!("line {}", i));
            }
        }
        let old = old_lines.join("\n");
        let new = new_lines.join("\n");
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[1].old_start, 16);
    }

    #[test]
    fn handles_pure_addition_and_deletion() {
        let add = format_unified_diff("new.txt", "", "hello\nworld\n");
        assert!(add.contains("+hello\n+world\n"));

        let del = format_unified_diff("old.txt", "hello\nworld\n", "");
        assert!(del.contains("-hello\n-world\n"));
    }

    #[test]
    fn large_diff_uses_linear_fallback_without_oom() {
        let mut old = String::new();
        let mut new = String::new();
        for i in 0..2100 {
            let _ = writeln!(&mut old, "old_{}", i);
            let _ = writeln!(&mut new, "new_{}", i);
        }
        let hunks = compute_diff(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_count, 2100);
        assert_eq!(hunks[0].new_count, 2100);
    }
}
