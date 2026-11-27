// SPDX-License-Identifier: GPL-3.0-only
//! `#![no_std]`, zero-allocation English + SexOS spellchecking, Damerau-Levenshtein
//! suggestions, inline ghost-text completion, and Space-bar autocorrect engine.

#![no_std]

mod lexicon;

use core::fmt;

/// Maximum ASCII/UTF-8 word length handled on the stack.
pub const MAX_WORD_BYTES: usize = 48;

/// Stack-allocated UTF-8 word buffer (zero heap allocations).
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct WordBuf {
    buf: [u8; MAX_WORD_BYTES],
    len: u8,
}

impl Default for WordBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl WordBuf {
    pub const fn new() -> Self {
        Self { buf: [0; MAX_WORD_BYTES], len: 0 }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > MAX_WORD_BYTES {
            return None;
        }
        let mut buf = [0u8; MAX_WORD_BYTES];
        buf[..bytes.len()].copy_from_slice(bytes);
        Some(Self { buf, len: bytes.len() as u8 })
    }

    pub fn as_str(&self) -> &str {
        // Every constructor only copies valid UTF-8 byte slices.
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push_byte(&mut self, b: u8) -> bool {
        let idx = self.len as usize;
        if idx >= MAX_WORD_BYTES {
            return false;
        }
        self.buf[idx] = b;
        self.len += 1;
        true
    }

    fn push_str(&mut self, s: &str) -> bool {
        let bytes = s.as_bytes();
        let idx = self.len as usize;
        if idx + bytes.len() > MAX_WORD_BYTES {
            return false;
        }
        self.buf[idx..idx + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len() as u8;
        true
    }
}

impl fmt::Debug for WordBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("WordBuf").field(&self.as_str()).finish()
    }
}

impl fmt::Display for WordBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Text input context determining whether spellcheck, prediction, and autocorrect apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextMode {
    #[default]
    Prose,
    SingleLineProse,
    Code,
    Password,
    Url,
    Terminal,
}

impl TextMode {
    pub const fn allows_spellcheck(self) -> bool {
        matches!(self, Self::Prose | Self::SingleLineProse)
    }

    pub const fn allows_prediction(self) -> bool {
        matches!(self, Self::Prose | Self::SingleLineProse)
    }

    pub const fn allows_autocorrect(self) -> bool {
        matches!(self, Self::Prose | Self::SingleLineProse)
    }

    pub const fn is_multiline(self) -> bool {
        matches!(self, Self::Prose | Self::Code | Self::Terminal)
    }
}

/// Byte offset span of a token inside text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
}

/// Fixed-capacity ring buffer for user-learned or ignored words (zero allocation).
#[derive(Clone, Debug)]
pub struct RingWordSet<const N: usize> {
    slots: [WordBuf; N],
    head: usize,
    count: usize,
}

impl<const N: usize> Default for RingWordSet<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RingWordSet<N> {
    pub const fn new() -> Self {
        Self { slots: [WordBuf::new(); N], head: 0, count: 0 }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.count = 0;
    }

    pub fn insert(&mut self, word: &str) {
        if N == 0 {
            return;
        }
        let Some(norm) = normalize_ascii_lower(word) else { return };
        if norm.is_empty() || self.contains_norm(norm.as_str()) {
            return;
        }
        if self.count < N {
            let idx = (self.head + self.count) % N;
            self.slots[idx] = norm;
            self.count += 1;
        } else {
            self.slots[self.head] = norm;
            self.head = (self.head + 1) % N;
        }
    }

    pub fn contains(&self, word: &str) -> bool {
        let Some(norm) = normalize_ascii_lower(word) else { return false };
        !norm.is_empty() && self.contains_norm(norm.as_str())
    }

    fn contains_norm(&self, lower: &str) -> bool {
        self.iter().any(|w| w == lower)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        let head = self.head;
        let count = self.count;
        (0..count).map(move |i| self.slots[(head + i) % N].as_str())
    }
}

/// Tracks the most recent Space-bar autocorrect for instant 1-Backspace revert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AutocorrectRevert {
    /// Byte offset where the autocorrected word begins.
    pub word_start: usize,
    /// Byte offset just after the replacement word (underline end).
    pub corrected_end: usize,
    /// Byte offset right after the Space/trigger byte where Backspace reverts.
    pub trigger_end: usize,
    /// Original typed letters before autocorrect.
    pub original: WordBuf,
    /// Corrected text that replaced `original`.
    pub corrected: WordBuf,
    pub active: bool,
}

impl AutocorrectRevert {
    pub const fn new() -> Self {
        Self {
            word_start: 0,
            corrected_end: 0,
            trigger_end: 0,
            original: WordBuf::new(),
            corrected: WordBuf::new(),
            active: false,
        }
    }

    pub fn record(&mut self, word_start: usize, original: &str, corrected: &str, trigger_len: usize) {
        let Some(orig_buf) = WordBuf::from_str(original) else {
            self.clear();
            return;
        };
        let Some(corr_buf) = WordBuf::from_str(corrected) else {
            self.clear();
            return;
        };
        let corrected_end = word_start + corr_buf.len();
        self.word_start = word_start;
        self.corrected_end = corrected_end;
        self.trigger_end = corrected_end + trigger_len;
        self.original = orig_buf;
        self.corrected = corr_buf;
        self.active = true;
    }

    pub fn clear(&mut self) {
        self.active = false;
    }

    /// True when pressing Backspace at `cursor` should revert the autocorrection.
    pub fn should_revert_at(&self, cursor: usize) -> bool {
        self.active && cursor == self.trigger_end
    }

    /// Active blue/indigo underline byte span `(word_start, corrected_end)` while revertible.
    pub fn underline_span(&self) -> Option<(usize, usize)> {
        self.active.then_some((self.word_start, self.corrected_end))
    }
}

/// Top-level spellchecker, suggestion ranker, ghost-text predictor, and autocorrect engine.
#[derive(Clone, Debug, Default)]
pub struct SpellEngine {
    pub learned: RingWordSet<64>,
    pub ignored: RingWordSet<64>,
}

impl SpellEngine {
    pub const fn new() -> Self {
        Self { learned: RingWordSet::new(), ignored: RingWordSet::new() }
    }

    pub fn learn_word(&mut self, word: &str) {
        self.learned.insert(strip_edge_punct(word));
    }

    pub fn ignore_word(&mut self, word: &str) {
        self.ignored.insert(strip_edge_punct(word));
    }

    pub fn is_learned_or_ignored(&self, word: &str) -> bool {
        let trimmed = strip_edge_punct(word);
        self.learned.contains(trimmed) || self.ignored.contains(trimmed)
    }

    /// Returns `true` if `raw` is spelled validly or should be skipped (URLs, code, numbers, etc.).
    pub fn check_word(&self, raw: &str) -> bool {
        let trimmed = strip_edge_punct(raw);
        if trimmed.is_empty() || should_skip_token(trimmed) {
            return true;
        }
        let Some(lower) = normalize_ascii_lower(trimmed) else {
            // Non-ASCII letters/words (accents, non-Latin scripts) or very long tokens are never false-flagged.
            return true;
        };
        if lower.len() < 2 {
            return true;
        }
        if self.learned.contains(lower.as_str()) || self.ignored.contains(lower.as_str()) {
            return true;
        }
        if lookup_autocorrect(lower.as_str()).is_some() {
            return false;
        }
        if is_all_caps_acronym(trimmed) {
            return true;
        }
        self.check_normalized(lower.as_str())
    }

    /// Number of words embedded in the static lexicon.
    pub const fn lexicon_count(&self) -> usize {
        lexicon::LEXICON_COUNT
    }

    /// Compressed ASCII lexicon blob size in bytes.
    pub const fn lexicon_blob_bytes(&self) -> usize {
        lexicon::LEXICON_BLOB_BYTES
    }

    /// Produces up to 3 frequency-ranked corrections within Damerau-Levenshtein distance <= 2
    /// (plus high-confidence autocorrect & phonetic boosts), preserving the input word's casing.
    pub fn suggest(&self, raw: &str, out: &mut [WordBuf; 3]) -> usize {
        *out = [WordBuf::new(); 3];
        let trimmed = strip_edge_punct(raw);
        if trimmed.is_empty() || trimmed.len() >= 30 {
            return 0;
        }
        let Some(lower_buf) = normalize_ascii_lower(trimmed) else { return 0 };
        let lower = lower_buf.as_str();
        let q = lower.as_bytes();
        let q_mask = char_mask(q);

        let mut best_words: [WordBuf; 3] = [WordBuf::new(); 3];
        let mut best_scores: [i32; 3] = [i32::MIN; 3];
        let mut count = 0usize;

        let mut push_cand = |cand: &str, score: i32| {
            let formatted = format_suggestion(trimmed, cand);
            if formatted.as_str() == trimmed {
                return;
            }
            for i in 0..count {
                if best_words[i].as_str() == formatted.as_str() {
                    if score > best_scores[i] {
                        best_scores[i] = score;
                        sort_top3(&mut best_words, &mut best_scores, count);
                    }
                    return;
                }
            }
            if count < 3 {
                best_words[count] = formatted;
                best_scores[count] = score;
                count += 1;
                sort_top3(&mut best_words, &mut best_scores, count);
            } else if score > best_scores[2] {
                best_words[2] = formatted;
                best_scores[2] = score;
                sort_top3(&mut best_words, &mut best_scores, 3);
            }
        };

        // High-confidence typo table entry gets priority boost.
        if let Some(auto) = lookup_autocorrect(lower) {
            push_cand(auto, 90_000);
        }

        // User learned words can also be suggested.
        for lw in self.learned.iter() {
            if let Some(score) = score_candidate(q, q_mask, lw.as_bytes(), 230, true) {
                push_cand(lw, score);
            }
        }

        // Scan static compressed lexicon (length + bitmask filter rejects >99% before OSA DP).
        for &entry in lexicon::LEXICON_ENTRIES {
            let (cand, tier) = entry_word_tier(entry);
            if let Some(score) = score_candidate(q, q_mask, cand.as_bytes(), tier, false) {
                push_cand(cand, score);
            }
        }

        out[..count].copy_from_slice(&best_words[..count]);
        count
    }

    /// Prefix lookup over frequency-ranked common words (>= 3 chars typed).
    /// Returns only the missing suffix (e.g. `"pref"` -> `"erences"`).
    pub fn predict_completion(&self, prefix: &str) -> Option<WordBuf> {
        if prefix.len() < 3 || prefix.len() >= MAX_WORD_BYTES || !prefix.bytes().all(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        let lower_buf = normalize_ascii_lower(prefix)?;
        let lower = lower_buf.as_str();

        // Contiguous binary-search range in the sorted lexicon for words starting with `lower`.
        let start = lexicon::LEXICON_ENTRIES
            .partition_point(|&e| entry_word_tier(e).0 < lower);

        let mut exact_tier: u8 = 0;
        let mut best_word: Option<&str> = None;
        let mut best_tier: u8 = 0;
        let mut best_len: usize = usize::MAX;

        // First check user learned words (treated as tier 235).
        for lw in self.learned.iter() {
            if lw == lower {
                exact_tier = exact_tier.max(235);
            } else if lw.starts_with(lower) && lw.len() > lower.len() {
                if 235 > best_tier || (235 == best_tier && lw.len() < best_len) {
                    best_tier = 235;
                    best_len = lw.len();
                    best_word = Some(lw);
                }
            }
        }

        for &entry in &lexicon::LEXICON_ENTRIES[start..] {
            let (word, tier) = entry_word_tier(entry);
            if !word.starts_with(lower) {
                break;
            }
            if word.len() == lower.len() {
                exact_tier = exact_tier.max(tier);
                continue;
            }
            // Only surface confident high-frequency vocabulary (>= 180) as ghost completions.
            if tier < 180 {
                continue;
            }
            if tier > best_tier || (tier == best_tier && word.len() < best_len) {
                best_tier = tier;
                best_len = word.len();
                best_word = Some(word);
            }
        }

        let full = best_word?;
        // Avoid distracting suffixes when the typed prefix is already a more common complete word.
        if exact_tier >= best_tier {
            return None;
        }

        let suffix_lower = &full[lower.len()..];
        if suffix_lower.is_empty() {
            return None;
        }
        let canon = canonical_case(full).unwrap_or(full);
        let suffix_canon = if canon.len() == full.len() { &canon[lower.len()..] } else { suffix_lower };
        format_prediction_suffix(prefix, suffix_lower, suffix_canon)
    }

    /// High-confidence Space-bar autocorrect (respects learned & ignored words).
    pub fn autocorrect_on_space(&self, raw: &str) -> Option<WordBuf> {
        let trimmed = strip_edge_punct(raw);
        if trimmed.is_empty() {
            return None;
        }
        if self.is_learned_or_ignored(trimmed) {
            return None;
        }
        let lower_buf = normalize_ascii_lower(trimmed)?;
        let repl = lookup_autocorrect(lower_buf.as_str())?;
        let out = format_autocorrect(trimmed, repl);
        if out.as_str() == trimmed { None } else { Some(out) }
    }

    /// Iterates over all misspelled words in `text` for `mode` without heap allocation.
    pub fn scan_misspelled(&self, text: &str, mode: TextMode, mut on_misspelled: impl FnMut(WordSpan)) {
        if !mode.allows_spellcheck() {
            return;
        }
        let bytes = text.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let chunk_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let chunk = &text[chunk_start..i];
            if chunk.is_empty() || chunk_is_nonprose(chunk) {
                continue;
            }
            self.scan_chunk_words(chunk, chunk_start, &mut on_misspelled);
        }
    }

    fn scan_chunk_words(&self, chunk: &str, base_offset: usize, on_misspelled: &mut impl FnMut(WordSpan)) {
        let bytes = chunk.as_bytes();
        let mut p = 0usize;
        while p < bytes.len() {
            while p < bytes.len() && !is_subtoken_start(chunk, p) {
                p = next_utf8_boundary(chunk, p);
            }
            let start = p;
            while p < bytes.len() && is_subtoken_continue(chunk, p) {
                p = next_utf8_boundary(chunk, p);
            }
            let mut end = p;
            // Trim trailing apostrophes/hyphens/dots attached to punctuation like `'word'` or `word.`
            while end > start && matches!(bytes[end - 1], b'\'' | b'-' | b'.') {
                end -= 1;
            }
            while end > start && chunk[start..end].ends_with('\u{2019}') {
                end -= '\u{2019}'.len_utf8();
            }
            let mut trimmed_start = start;
            while trimmed_start < end && matches!(bytes[trimmed_start], b'\'' | b'-' | b'.') {
                trimmed_start += 1;
            }
            if trimmed_start < end {
                let word = &chunk[trimmed_start..end];
                if !self.check_word(word) {
                    on_misspelled(WordSpan {
                        start: base_offset + trimmed_start,
                        end: base_offset + end,
                    });
                }
            }
        }
    }

    fn check_normalized(&self, lower: &str) -> bool {
        if lookup_lexicon_tier(lower).is_some() {
            return true;
        }
        // Possessive forms (`friends'`, `friend's`, `user'`)
        for suf in ["'s", "s'", "'"] {
            if let Some(stem) = lower.strip_suffix(suf) {
                if stem.len() >= 2 && self.check_normalized(stem) {
                    return true;
                }
            }
        }
        // Hyphenated compounds (`well-known`, `multi-step`)
        if lower.contains('-') {
            let mut any = false;
            let mut all_ok = true;
            for part in lower.split('-') {
                if part.is_empty() {
                    all_ok = false;
                    break;
                }
                any = true;
                if part.len() >= 2 && !self.check_normalized(part) {
                    all_ok = false;
                    break;
                }
            }
            if any && all_ok {
                return true;
            }
        }
        check_with_suffixes(lower)
    }
}

/// Finds the prose/identifier word token spanning or directly ending at `byte_idx`.
pub fn word_span_at(text: &str, byte_idx: usize) -> Option<WordSpan> {
    if text.is_empty() {
        return None;
    }
    let idx = byte_idx.min(text.len());
    let is_word_at = |pos: usize| -> bool {
        if pos >= text.len() {
            return false;
        }
        let slice = &text[pos..];
        let Some(ch) = slice.chars().next() else { return false };
        ch.is_alphanumeric() || ch == '_' || ch == '\'' || ch == '\u{2019}'
    };

    let probe = if idx < text.len() && is_word_at(floor_utf8_boundary(text, idx)) {
        floor_utf8_boundary(text, idx)
    } else if idx > 0 {
        let prev = prev_utf8_boundary(text, idx);
        if is_word_at(prev) { prev } else { return None }
    } else {
        return None;
    };

    let mut start = probe;
    while start > 0 {
        let prev = prev_utf8_boundary(text, start);
        if !is_word_at(prev) {
            break;
        }
        start = prev;
    }
    let mut end = next_utf8_boundary(text, probe);
    while end < text.len() && is_word_at(end) {
        end = next_utf8_boundary(text, end);
    }

    // Trim edge apostrophes (`'quoted'`) so clicking quotes doesn't include them in spell words.
    while start < end && matches!(text.as_bytes()[start], b'\'' | b'_') && text[start..].starts_with('\'') {
        start += 1;
    }
    while end > start && text[start..end].ends_with('\'') {
        end -= 1;
    }
    (start < end).then_some(WordSpan { start, end })
}

/// If `cursor` sits at the immediate end of an alphabetic prose prefix of length >= 3,
/// returns `(word_start, &text[word_start..cursor])`. Returns `None` mid-word or on code identifiers.
pub fn active_word_prefix_at(text: &str, cursor: usize) -> Option<(usize, &str)> {
    if cursor > text.len() || cursor < 3 || !text.is_char_boundary(cursor) {
        return None;
    }
    let bytes = text.as_bytes();
    // Do not predict while cursor is before another word character.
    if cursor < bytes.len() && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'_' | b'\'')) {
        return None;
    }
    let mut start = cursor;
    while start > 0 && bytes[start - 1].is_ascii_alphabetic() {
        start -= 1;
    }
    if cursor - start < 3 {
        return None;
    }
    // Skip snake_case segments, URLs, file paths, domains, and numbers immediately before the prefix.
    if start > 0 && (bytes[start - 1].is_ascii_digit()
        || matches!(bytes[start - 1], b'_' | b'/' | b'\\' | b'.' | b'@' | b':' | b'\'' | b'-'))
    {
        return None;
    }
    Some((start, &text[start..cursor]))
}

#[inline]
fn entry_word_tier(entry: u32) -> (&'static str, u8) {
    let offset = (entry >> 13) as usize;
    let len = ((entry >> 8) & 0x1F) as usize;
    let tier = (entry & 0xFF) as u8;
    (&lexicon::LEXICON_BLOB[offset..offset + len], tier)
}

fn lookup_lexicon_tier(lower: &str) -> Option<u8> {
    let idx = lexicon::LEXICON_ENTRIES
        .binary_search_by(|&e| entry_word_tier(e).0.cmp(lower))
        .ok()?;
    Some(entry_word_tier(lexicon::LEXICON_ENTRIES[idx]).1)
}

fn canonical_case(lower: &str) -> Option<&'static str> {
    let idx = lexicon::CANONICAL_CASE
        .binary_search_by(|&(k, _)| k.cmp(lower))
        .ok()?;
    Some(lexicon::CANONICAL_CASE[idx].1)
}

fn lookup_autocorrect(lower: &str) -> Option<&'static str> {
    let idx = lexicon::AUTOCORRECT_PAIRS
        .binary_search_by(|&(k, _)| k.cmp(lower))
        .ok()?;
    Some(lexicon::AUTOCORRECT_PAIRS[idx].1)
}

fn has_lexicon_word(s: &str) -> bool {
    lookup_lexicon_tier(s).is_some()
}

fn has_with_appended(stem: &str, extra: &str) -> bool {
    let mut buf = WordBuf::new();
    buf.push_str(stem) && buf.push_str(extra) && has_lexicon_word(buf.as_str())
}

fn undouble_and_lookup(stem: &str) -> bool {
    let b = stem.as_bytes();
    b.len() >= 3 && b[b.len() - 1] == b[b.len() - 2] && has_lexicon_word(&stem[..b.len() - 1])
}

fn check_with_suffixes(w: &str) -> bool {
    let len = w.len();
    if let Some(stem) = w.strip_suffix("ies") {
        if stem.len() >= 2 && has_with_appended(stem, "y") {
            return true;
        }
    }
    if let Some(stem) = w.strip_suffix("ves") {
        if stem.len() >= 2 && (has_with_appended(stem, "f") || has_with_appended(stem, "fe")) {
            return true;
        }
    }
    if let Some(stem) = w.strip_suffix("es") {
        if stem.len() >= 2
            && ((stem.ends_with('s')
                || stem.ends_with('x')
                || stem.ends_with('z')
                || stem.ends_with("ch")
                || stem.ends_with("sh")
                || stem.ends_with('o'))
                && has_lexicon_word(stem))
        {
            return true;
        }
    }
    if let Some(stem) = w.strip_suffix('s') {
        // Orthographic guard: avoid passing `"thiss"` / `"boxs"` by checking English -s constraints.
        if stem.len() >= 3
            && !stem.ends_with('s')
            && !stem.ends_with('x')
            && !stem.ends_with('z')
            && !stem.ends_with("ch")
            && !stem.ends_with("sh")
            && has_lexicon_word(stem)
        {
            return true;
        }
    }
    if let Some(stem) = w.strip_suffix("ied") {
        if stem.len() >= 2 && has_with_appended(stem, "y") {
            return true;
        }
    }
    if len > 4 {
        if let Some(stem) = w.strip_suffix("ed") {
            if has_lexicon_word(stem)
                || has_with_appended(stem, "e")
                || undouble_and_lookup(stem)
            {
                return true;
            }
        }
    }
    if let Some(stem) = w.strip_suffix("ying") {
        if !stem.is_empty() && has_with_appended(stem, "ie") {
            return true;
        }
    }
    if len > 5 {
        if let Some(stem) = w.strip_suffix("ing") {
            if has_lexicon_word(stem)
                || has_with_appended(stem, "e")
                || undouble_and_lookup(stem)
            {
                return true;
            }
        }
    }
    if let Some(stem) = w.strip_suffix("ier") {
        if stem.len() >= 2 && has_with_appended(stem, "y") {
            return true;
        }
    }
    if len > 4 {
        if let Some(stem) = w.strip_suffix("er") {
            if has_lexicon_word(stem)
                || has_with_appended(stem, "e")
                || undouble_and_lookup(stem)
            {
                return true;
            }
        }
    }
    if let Some(stem) = w.strip_suffix("iest") {
        if stem.len() >= 2 && has_with_appended(stem, "y") {
            return true;
        }
    }
    if len > 5 {
        if let Some(stem) = w.strip_suffix("est") {
            if has_lexicon_word(stem)
                || has_with_appended(stem, "e")
                || undouble_and_lookup(stem)
            {
                return true;
            }
        }
    }
    if let Some(stem) = w.strip_suffix("ily") {
        if stem.len() >= 2 && has_with_appended(stem, "y") {
            return true;
        }
    }
    if let Some(stem) = w.strip_suffix("bly") {
        if stem.len() >= 2 && has_with_appended(stem, "ble") {
            return true;
        }
    }
    if let Some(stem) = w.strip_suffix("ally") {
        if stem.len() >= 2 && (has_lexicon_word(stem) || has_with_appended(stem, "al")) {
            return true;
        }
    }
    if len > 4 {
        if let Some(stem) = w.strip_suffix("ly") {
            if has_lexicon_word(stem) {
                return true;
            }
        }
    }
    if let Some(stem) = w.strip_suffix("iness") {
        if stem.len() >= 2 && has_with_appended(stem, "y") {
            return true;
        }
    }
    if len > 6 {
        if let Some(stem) = w.strip_suffix("ness") {
            if has_lexicon_word(stem) {
                return true;
            }
        }
        if let Some(stem) = w.strip_suffix("ment") {
            if has_lexicon_word(stem) {
                return true;
            }
        }
        if let Some(stem) = w.strip_suffix("able") {
            if has_lexicon_word(stem) || has_with_appended(stem, "e") {
                return true;
            }
        }
    }
    false
}

fn should_skip_token(raw: &str) -> bool {
    if raw.len() > 40 {
        return true;
    }
    // URLs, emails, paths, code operators, numbers, snake_case
    if raw.contains("://")
        || raw.starts_with("www.")
        || raw.starts_with("mailto:")
        || raw.contains('/')
        || raw.contains('\\')
        || raw.contains('@')
        || raw.contains('_')
        || raw.contains("::")
        || raw.contains("->")
        || raw.contains("=>")
        || raw.bytes().any(|b| b.is_ascii_digit() || matches!(b, b'<' | b'>' | b'{' | b'}' | b'[' | b']' | b'=' | b'+' | b'*' | b'&' | b'|' | b'^' | b'%' | b'#' | b'$' | b'~' | b'`'))
    {
        return true;
    }
    // Dotted identifiers / filenames (`main.rs`, `example.com`)
    if raw.contains('.') {
        return true;
    }
    // camelCase / internal PascalCase unless present in known canonical SexOS/brand terms
    let bytes = raw.as_bytes();
    let has_camel_transition = bytes.windows(2).any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
    if has_camel_transition {
        if let Some(lower) = normalize_ascii_lower(raw) {
            if lookup_lexicon_tier(lower.as_str()).is_none() {
                return true;
            }
        }
    }
    false
}

fn is_all_caps_acronym(raw: &str) -> bool {
    let mut count = 0usize;
    for b in raw.bytes() {
        if b.is_ascii_uppercase() {
            count += 1;
        } else if b.is_ascii_alphabetic() {
            return false;
        }
    }
    (2..=6).contains(&count)
}

fn chunk_is_nonprose(chunk: &str) -> bool {
    chunk.contains("://")
        || chunk.starts_with("www.")
        || chunk.starts_with("mailto:")
        || chunk.starts_with('/')
        || chunk.starts_with("~/")
        || (chunk.contains('@') && chunk.contains('.'))
}

fn strip_edge_punct(mut s: &str) -> &str {
    const TRIM_CHARS: &[char] = &[
        ' ', '\t', '\r', '\n', ',', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}',
        '<', '>', '"', '`', '*', '~',
    ];
    s = s.trim_matches(TRIM_CHARS);
    s = s.trim_matches(|c: char| matches!(c, '\u{201C}' | '\u{201D}' | '\u{2018}' | '\''));
    // Strip trailing sentence dot if not preceded by another dot
    while s.ends_with('.') && !s.ends_with("..") {
        s = &s[..s.len() - 1];
    }
    s
}

fn normalize_ascii_lower(raw: &str) -> Option<WordBuf> {
    let mut out = WordBuf::new();
    for ch in raw.chars() {
        if matches!(ch, '\u{2018}' | '\u{2019}' | '\u{02BC}') {
            if !out.push_byte(b'\'') {
                return None;
            }
            continue;
        }
        if !ch.is_ascii() {
            return None;
        }
        let b = (ch as u8).to_ascii_lowercase();
        if !out.push_byte(b) {
            return None;
        }
    }
    Some(out)
}

fn is_subtoken_start(s: &str, byte_idx: usize) -> bool {
    let Some(ch) = s[byte_idx..].chars().next() else { return false };
    ch.is_alphanumeric() || ch == '_' || ch == '\'' || ch == '\u{2019}'
}

fn is_subtoken_continue(s: &str, byte_idx: usize) -> bool {
    let Some(ch) = s[byte_idx..].chars().next() else { return false };
    ch.is_alphanumeric() || matches!(ch, '_' | '\'' | '\u{2019}' | '-' | '.')
}

fn next_utf8_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    s[idx..].chars().next().map_or(s.len(), |c| idx + c.len_utf8())
}

fn prev_utf8_boundary(s: &str, mut idx: usize) -> usize {
    if idx == 0 {
        return 0;
    }
    idx -= 1;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn floor_utf8_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[inline]
fn char_mask(bytes: &[u8]) -> u32 {
    let mut mask = 0u32;
    for &b in bytes {
        let bit = match b {
            b'a'..=b'z' => b - b'a',
            b'\'' => 26,
            _ => 27,
        };
        mask |= 1u32 << bit;
    }
    mask
}

/// Optimal String Alignment (restricted Damerau-Levenshtein) bounded by `max_dist`.
pub fn damerau_levenshtein(a: &[u8], b: &[u8]) -> usize {
    const CAP: usize = 32;
    let n = a.len().min(CAP - 1);
    let m = b.len().min(CAP - 1);
    let mut prev2 = [0u8; CAP];
    let mut prev = [0u8; CAP];
    let mut curr = [0u8; CAP];
    for j in 0..=m {
        prev[j] = j as u8;
    }
    for i in 1..=n {
        curr[0] = i as u8;
        for j in 1..=m {
            let cost = u8::from(a[i - 1] != b[j - 1]);
            let mut best = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(prev2[j - 2] + cost);
            }
            curr[j] = best;
        }
        prev2[..=m].copy_from_slice(&prev[..=m]);
        prev[..=m].copy_from_slice(&curr[..=m]);
    }
    prev[m] as usize
}

fn score_candidate(q: &[u8], q_mask: u32, cand: &[u8], tier: u8, learned: bool) -> Option<i32> {
    if q.len().abs_diff(cand.len()) > 2 {
        return None;
    }
    let c_mask = char_mask(cand);
    // Phonetic `f` <-> `ph` or `kn` <-> `n` bypasses strict 2-char mask filter only slightly, so check mask <= 2
    // unless `f`/`p`/`h` phonetic case applies.
    let phonetic_bonus = phonetic_similarity_bonus(q, cand);
    if phonetic_bonus == 0 {
        if (q_mask & !c_mask).count_ones() > 2 || (c_mask & !q_mask).count_ones() > 2 {
            return None;
        }
    }
    let dist = damerau_levenshtein(q, cand);
    if dist > 2 {
        return None;
    }
    let mut score = (3 - dist as i32) * 10_000 + (tier as i32) * 25 + phonetic_bonus;
    if q.first() == cand.first() {
        score += 350;
    }
    if q.len() == cand.len() {
        score += 120;
    }
    if learned {
        score += 600;
    }
    Some(score)
}

fn phonetic_similarity_bonus(q: &[u8], cand: &[u8]) -> i32 {
    if (q.starts_with(b"f") && cand.starts_with(b"ph")) || (q.starts_with(b"ph") && cand.starts_with(b"f")) {
        return 14_000;
    }
    if (q.starts_with(b"n") && cand.starts_with(b"kn")) || (q.starts_with(b"r") && cand.starts_with(b"wr")) {
        return 10_000;
    }
    if (contains_pair(q, b"ie") && contains_pair(cand, b"ei"))
        || (contains_pair(q, b"ei") && contains_pair(cand, b"ie"))
    {
        return 1_500;
    }
    0
}

fn contains_pair(s: &[u8], pair: &[u8; 2]) -> bool {
    s.windows(2).any(|w| w == pair)
}

fn sort_top3(words: &mut [WordBuf; 3], scores: &mut [i32; 3], count: usize) {
    for i in 1..count {
        let mut j = i;
        while j > 0 && scores[j] > scores[j - 1] {
            scores.swap(j, j - 1);
            words.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn format_suggestion(orig: &str, cand_lower: &str) -> WordBuf {
    if let Some(canon) = canonical_case(cand_lower) {
        return WordBuf::from_str(canon).unwrap_or_default();
    }
    format_autocorrect(orig, cand_lower)
}

fn format_autocorrect(orig: &str, repl: &str) -> WordBuf {
    // Preserve explicit canonical uppercase starts like `"I'm"` / `"I've"`.
    if repl.starts_with('I') {
        return WordBuf::from_str(repl).unwrap_or_default();
    }
    let alpha_count = orig.bytes().filter(|b| b.is_ascii_alphabetic()).count();
    let all_upper = alpha_count >= 2 && orig.bytes().filter(|b| b.is_ascii_alphabetic()).all(|b| b.is_ascii_uppercase());
    let first_upper = orig.bytes().next().is_some_and(|b| b.is_ascii_uppercase());

    let mut out = WordBuf::new();
    if all_upper {
        for b in repl.bytes() {
            out.push_byte(b.to_ascii_uppercase());
        }
        return out;
    }
    if first_upper {
        for (i, b) in repl.bytes().enumerate() {
            out.push_byte(if i == 0 { b.to_ascii_uppercase() } else { b });
        }
        return out;
    }
    let _ = out.push_str(repl);
    out
}

fn format_prediction_suffix(prefix: &str, suffix_lower: &str, suffix_canon: &str) -> Option<WordBuf> {
    let all_upper = prefix.len() >= 2 && prefix.bytes().all(|b| b.is_ascii_uppercase());
    let first_upper = prefix.bytes().next().is_some_and(|b| b.is_ascii_uppercase());
    let mut out = WordBuf::new();
    if all_upper {
        for b in suffix_lower.bytes() {
            out.push_byte(b.to_ascii_uppercase());
        }
    } else if first_upper {
        out.push_str(suffix_canon);
    } else {
        out.push_str(suffix_lower);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests;
