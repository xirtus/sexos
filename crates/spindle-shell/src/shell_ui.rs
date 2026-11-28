// SPDX-License-Identifier: GPL-3.0-only
//! Interactive Fish / Zsh shell helpers: inline ghost autosuggestions, word-by-word
//! `Ctrl+F` acceptance, prefix-filtered history navigation (`up-line-or-search`),
//! fuzzy `Ctrl+R` reverse history search, and multi-column Tab completion menus.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::complete::{Completion, CompletionKind};
use crate::engine::Engine;
use crate::host::Host;
use crate::width::{str_width, truncate};

/// A history hit from interactive fuzzy reverse search (`Ctrl+R`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub hist_index: usize,
    pub text: String,
    pub score: i32,
    pub match_positions: Vec<usize>,
}

/// Computes completions including interactive terminal commands, aliases, and `theme` names.
pub fn complete_with_extras(
    line: &str,
    cursor: usize,
    host: &dyn Host,
    engine: &Engine,
    extra_cmds: &[&str],
    theme_names: &[&str],
) -> Vec<Completion> {
    let cursor = cursor.min(line.len());
    let prefix = &line[..cursor];
    // Offer theme names right after `theme `.
    if let Some(rest) = prefix.trim_start().strip_prefix("theme ") {
        if !rest.contains(' ') {
            let start = cursor - rest.len();
            let mut out: Vec<Completion> = theme_names
                .iter()
                .filter(|t| t.starts_with(rest))
                .map(|t| Completion {
                    replace: start..cursor,
                    text: String::from(*t),
                    kind: CompletionKind::Command,
                    description: String::from("theme"),
                })
                .collect();
            out.sort_by(|a, b| a.text.cmp(&b.text));
            return out;
        }
    }

    let mut hits = engine.complete(line, cursor, host);

    let before_word = prefix.trim_end_matches(|c: char| !c.is_whitespace() && c != '|' && c != ';' && c != '(');
    let partial = &prefix[before_word.len()..];
    let at_head = before_word.trim().is_empty()
        || before_word.trim_end().ends_with('|')
        || before_word.trim_end().ends_with(';')
        || before_word.trim_end().ends_with('(');
    if at_head && !partial.starts_with('$') {
        let replace = before_word.len()..cursor;
        for (k, v) in engine.aliases() {
            if k.starts_with(partial) && !hits.iter().any(|h| h.text == *k) {
                hits.push(Completion {
                    replace: replace.clone(),
                    text: k.clone(),
                    kind: CompletionKind::Alias,
                    description: format!("alias -> {}", v),
                });
            }
        }
        for &cmd in extra_cmds {
            if cmd.starts_with(partial) && !hits.iter().any(|h| h.text == cmd) {
                hits.push(Completion {
                    replace: replace.clone(),
                    text: String::from(cmd),
                    kind: CompletionKind::Command,
                    description: String::from("terminal command"),
                });
            }
        }
        hits.sort_by(|a, b| a.text.cmp(&b.text));
        hits.dedup_by(|a, b| a.text == b.text);
    }
    hits
}

/// Fish / `zsh-autosuggestions` inline ghost suffix when the cursor sits at end-of-line.
/// Prefers the most recent history entry matching `line` as a strict prefix, then falls
/// back to the top completion candidate for the partial token.
pub fn ghost_suggestion(
    line: &str,
    cursor: usize,
    history: &[String],
    host: &dyn Host,
    engine: &Engine,
    extra_cmds: &[&str],
    theme_names: &[&str],
) -> Option<String> {
    if line.is_empty() || cursor != line.len() {
        return None;
    }
    for entry in history.iter().rev() {
        if entry.len() > line.len() && entry.starts_with(line) {
            return Some(String::from(&entry[line.len()..]));
        }
    }
    let hits = complete_with_extras(line, cursor, host, engine, extra_cmds, theme_names);
    let first = hits.first()?;
    if first.replace.end != cursor || first.replace.start > cursor {
        return None;
    }
    let typed = &line[first.replace.start..cursor];
    if typed.is_empty() && hits.len() > 1 && !line.ends_with(' ') {
        return None;
    }
    first
        .text
        .strip_prefix(typed)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Accepts one word from `ghost` onto `line` (`Ctrl+F` / `Alt+Right` `forward-word`).
/// Returns `true` if `line` grew.
pub fn accept_ghost_word(line: &mut String, cursor: &mut usize, ghost: &str) -> bool {
    if ghost.is_empty() || *cursor != line.len() {
        return false;
    }
    let bytes = ghost.as_bytes();
    let mut take = 0usize;
    while take < bytes.len() && bytes[take].is_ascii_whitespace() {
        take += 1;
    }
    if take < bytes.len() {
        let first = ghost[take..].chars().next().unwrap();
        if is_word_byte(first) {
            while take < ghost.len() {
                let c = ghost[take..].chars().next().unwrap();
                if !is_word_byte(c) {
                    break;
                }
                take += c.len_utf8();
            }
            if ghost[take..].starts_with('/') {
                take += 1;
            }
        } else {
            take += first.len_utf8();
            while take < ghost.len() {
                let c = ghost[take..].chars().next().unwrap();
                if c.is_whitespace() || is_word_byte(c) {
                    break;
                }
                take += c.len_utf8();
            }
        }
    }
    if take == 0 {
        return false;
    }
    line.push_str(&ghost[..take]);
    *cursor = line.len();
    true
}

fn is_word_byte(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Prefix-filtered Up/Down history search (`up-line-or-search` / `down-line-or-search`).
/// Returns `Some(Some(idx))` for a matching entry, `Some(None)` when stepping past the
/// newest match back to the typed prefix, or `None` when there is nothing to move to.
pub fn history_search_prefix(
    history: &[String],
    prefix: &str,
    current: Option<usize>,
    up: bool,
) -> Option<Option<usize>> {
    if history.is_empty() {
        return None;
    }
    if up {
        let start = current.unwrap_or(history.len());
        for idx in (0..start).rev() {
            if history[idx].starts_with(prefix) {
                if current.is_some_and(|c| history[c] == history[idx]) {
                    continue;
                }
                return Some(Some(idx));
            }
        }
        None
    } else {
        let cur = current?;
        for idx in (cur + 1)..history.len() {
            if history[idx].starts_with(prefix) && history[idx] != history[cur] {
                return Some(Some(idx));
            }
        }
        Some(None)
    }
}

/// Fuzzy + substring reverse history search (`Ctrl+R`), newest deduplicated entries ranked first.
pub fn fuzzy_search_history(history: &[String], query: &str, limit: usize) -> Vec<FuzzyMatch> {
    let mut out = Vec::new();
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    let q_lower = query.to_lowercase();
    for (idx, entry) in history.iter().enumerate().rev() {
        if seen.contains_key(entry.as_str()) {
            continue;
        }
        if let Some((score, pos)) = score_fuzzy(entry, &q_lower, history.len() - idx) {
            seen.insert(entry.as_str(), ());
            out.push(FuzzyMatch {
                hist_index: idx,
                text: entry.clone(),
                score,
                match_positions: pos,
            });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| b.hist_index.cmp(&a.hist_index)));
    out.truncate(limit);
    out
}

fn score_fuzzy(hay: &str, q_lower: &str, recency_rank: usize) -> Option<(i32, Vec<usize>)> {
    if q_lower.is_empty() {
        return Some((1000 - recency_rank as i32, Vec::new()));
    }
    let h_lower = hay.to_lowercase();
    if let Some(start) = h_lower.find(q_lower) {
        let chars_before = hay[..start].chars().count();
        let q_chars = q_lower.chars().count();
        let pos: Vec<usize> = (chars_before..chars_before + q_chars).collect();
        let bonus = if start == 0 { 400 } else { 240 };
        return Some((bonus - recency_rank.min(200) as i32, pos));
    }
    let hay_chars: Vec<(usize, char)> = h_lower.chars().enumerate().collect();
    let mut at = 0usize;
    let mut pos = Vec::new();
    let mut score = 80i32 - recency_rank.min(60) as i32;
    let mut prev: Option<usize> = None;
    for qc in q_lower.chars() {
        let found = (at..hay_chars.len()).find(|&i| hay_chars[i].1 == qc)?;
        pos.push(hay_chars[found].0);
        if prev.is_some_and(|p| p + 1 == found) {
            score += 12;
        }
        if found == 0 || !hay_chars[found - 1].1.is_alphanumeric() {
            score += 8;
        }
        prev = Some(found);
        at = found + 1;
    }
    Some((score, pos))
}

/// Renders a Fish-style multi-column Tab completion drawer (`(ansi, rows_used)`).
pub fn format_completion_menu(
    hits: &[Completion],
    selected: usize,
    cols: usize,
    max_rows: usize,
    color: bool,
) -> (String, usize) {
    if hits.is_empty() || cols < 16 || max_rows < 2 {
        return (String::new(), 0);
    }
    let item_max = hits.iter().map(|h| str_width(&h.text) + 5).max().unwrap_or(12).clamp(12, 32);
    let col_w = (item_max + 2).min(cols.saturating_sub(2));
    let num_cols = (cols.saturating_sub(2) / col_w).max(1);
    let grid_rows = max_rows.saturating_sub(1).clamp(1, 6);
    let shown = (num_cols * grid_rows).min(hits.len());
    let actual_rows = shown.div_ceil(num_cols);

    let mut out = String::new();
    for r in 0..actual_rows {
        out.push_str("\r\n");
        for c in 0..num_cols {
            let idx = r + c * actual_rows;
            if idx >= shown {
                continue;
            }
            let h = &hits[idx];
            let icon = completion_badge(h.kind);
            let label = truncate(&h.text, col_w.saturating_sub(6));
            let cell_txt = format!("{} {}", icon, label);
            let pad = col_w.saturating_sub(str_width(&cell_txt));
            let is_sel = idx == selected;
            if color {
                if is_sel {
                    out.push_str("\x1b[48;2;36;40;59m\x1b[1;38;2;34;211;238m");
                } else {
                    out.push_str(completion_color(h.kind));
                }
                out.push_str(&cell_txt);
                out.push_str(&" ".repeat(pad));
                out.push_str("\x1b[0m");
            } else if is_sel {
                out.push('>');
                out.push_str(&cell_txt);
                out.push_str(&" ".repeat(pad.saturating_sub(1)));
            } else {
                out.push_str(&cell_txt);
                out.push_str(&" ".repeat(pad));
            }
        }
        out.push_str("\x1b[K");
    }
    if let Some(sel) = hits.get(selected) {
        let hint = format!("  {} ({}/{}) — Tab next · Enter accept · Esc cancel", sel.description, selected + 1, hits.len());
        let line = truncate(&hint, cols.saturating_sub(1));
        out.push_str("\r\n");
        if color {
            out.push_str(&format!("\x1b[38;2;100;116;139m{}\x1b[0m\x1b[K", line));
        } else {
            out.push_str(&line);
            out.push_str("\x1b[K");
        }
        return (out, actual_rows + 1);
    }
    (out, actual_rows)
}

fn completion_badge(k: CompletionKind) -> &'static str {
    match k {
        CompletionKind::Command => "cmd",
        CompletionKind::Alias => "als",
        CompletionKind::External => "bin",
        CompletionKind::Flag => "flg",
        CompletionKind::Variable => "var",
        CompletionKind::Column => "col",
        CompletionKind::Dir => "dir",
        CompletionKind::File => "fil",
    }
}

fn completion_color(k: CompletionKind) -> &'static str {
    match k {
        CompletionKind::Command => "\x1b[38;2;34;211;238m",
        CompletionKind::Alias => "\x1b[38;2;52;211;153m",
        CompletionKind::External => "\x1b[38;2;16;185;129m",
        CompletionKind::Flag => "\x1b[38;2;125;211;252m",
        CompletionKind::Variable => "\x1b[38;2;192;132;252m",
        CompletionKind::Column => "\x1b[38;2;245;158;11m",
        CompletionKind::Dir => "\x1b[1;38;2;96;165;250m",
        CompletionKind::File => "\x1b[38;2;203;213;225m",
    }
}

/// Renders the `Ctrl+R` fuzzy reverse history search box below the prompt (`(ansi, rows_used)`).
pub fn format_reverse_search(
    query: &str,
    matches: &[FuzzyMatch],
    selected: usize,
    cols: usize,
    max_rows: usize,
    color: bool,
) -> (String, usize) {
    let inner = cols.saturating_sub(4).max(16);
    let list_rows = max_rows.saturating_sub(2).clamp(1, 6);
    let shown = matches.len().min(list_rows);
    let mut out = String::new();
    let header = truncate(&format!("bck-i-search: {}_  ({} matches)", query, matches.len()), inner);
    out.push_str("\r\n");
    if color {
        out.push_str(&format!("\x1b[38;2;244;63;94m╭─ \x1b[1;38;2;236;239;244m{}\x1b[0m\x1b[K", header));
    } else {
        out.push_str(&format!("╭─ {}\x1b[K", header));
    }
    if matches.is_empty() {
        out.push_str("\r\n");
        if color {
            out.push_str("\x1b[38;2;100;116;139m│  (no history matches)\x1b[0m\x1b[K");
        } else {
            out.push_str("│  (no history matches)\x1b[K");
        }
        return (out, 2);
    }
    for (idx, m) in matches.iter().take(shown).enumerate() {
        let sel = idx == selected;
        let line = truncate(&m.text, inner.saturating_sub(4));
        out.push_str("\r\n");
        if color {
            let prefix = if sel { "\x1b[38;2;244;63;94m│ \x1b[48;2;36;40;59m▶ " } else { "\x1b[38;2;100;116;139m│   " };
            out.push_str(prefix);
            for (ci, ch) in line.chars().enumerate() {
                if m.match_positions.contains(&ci) {
                    out.push_str("\x1b[1;38;2;34;211;238m");
                    out.push(ch);
                    out.push_str(if sel { "\x1b[0m\x1b[48;2;36;40;59m\x1b[38;2;236;239;244m" } else { "\x1b[0m\x1b[38;2;203;213;225m" });
                } else {
                    out.push_str(if sel { "\x1b[38;2;236;239;244m" } else { "\x1b[38;2;203;213;225m" });
                    out.push(ch);
                }
            }
            out.push_str("\x1b[0m\x1b[K");
        } else {
            out.push_str(&format!("│ {} {}\x1b[K", if sel { ">" } else { " " }, line));
        }
    }
    (out, shown + 1)
}
