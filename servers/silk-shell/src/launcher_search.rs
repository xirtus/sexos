//! Type-to-search for the launcher: a short query, fuzzy scoring against a
//! row's name and its search words, and the ranked order of matching rows.
//! Pure and host-tested (`tests/launcher_search.rs`); the shell owns rendering.

/// Longest query the launcher keeps.
pub const QUERY_MAX: usize = 24;

/// The ASCII character of an unshifted set-1 key the launcher accepts:
/// letters, digits and space. Other keys keep their launcher meanings.
pub fn key_char(code: u8) -> Option<u8> {
    const TOP: &[u8; 10] = b"qwertyuiop";
    const HOME: &[u8; 9] = b"asdfghjkl";
    const BOTTOM: &[u8; 7] = b"zxcvbnm";
    match code {
        0x02..=0x0A => Some(b'1' + (code - 0x02)),
        0x0B => Some(b'0'),
        0x10..=0x19 => Some(TOP[(code - 0x10) as usize]),
        0x1E..=0x26 => Some(HOME[(code - 0x1E) as usize]),
        0x2C..=0x32 => Some(BOTTOM[(code - 0x2C) as usize]),
        0x39 => Some(b' '),
        _ => None,
    }
}

/// Like [`key_char`], plus the symbols arithmetic needs so Voyeur's Abacus
/// answers `2+2`: - = . / and, with Shift, + * ( ) ^ %; keypad + - *.
/// Shifted letters give the letter (the query is lowercase).
pub fn key_char_shifted(code: u8, shift: bool) -> Option<u8> {
    match (code, shift) {
        (0x0D, true) => Some(b'+'),
        (0x09, true) | (0x37, _) => Some(b'*'),
        (0x0A, true) => Some(b'('),
        (0x0B, true) => Some(b')'),
        (0x07, true) => Some(b'^'),
        (0x06, true) => Some(b'%'),
        (0x4E, _) => Some(b'+'),
        (0x0C, false) | (0x4A, _) => Some(b'-'),
        (0x0D, false) => Some(b'='),
        (0x34, false) => Some(b'.'),
        (0x35, false) => Some(b'/'),
        (0x02..=0x0B, true) => None,
        _ => key_char(code),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query { buf: [u8; QUERY_MAX], len: u8 }
impl Query {
    pub const fn new() -> Self { Self { buf: [0; QUERY_MAX], len: 0 } }
    /// Adds one character; a leading space or a full query refuses it.
    pub fn push(&mut self, c: u8) -> bool {
        if self.len as usize >= QUERY_MAX || !(32..=126).contains(&c) || (c == b' ' && self.len == 0) { return false; }
        self.buf[self.len as usize] = c.to_ascii_lowercase();
        self.len += 1;
        true
    }
    pub fn pop(&mut self) -> bool { if self.len == 0 { return false; } self.len -= 1; true }
    pub fn clear(&mut self) { self.len = 0; }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn as_str(&self) -> &str { core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("") }
}

/// Higher is better; None means the query is not in `text` in order.
/// Every query character must appear in order (spaces in the query are
/// skipped). Matches at word starts and runs of adjacent matches score more,
/// and a match that starts the text scores most, so "sp" finds "Open Spindle"
/// before "Shut Down" ... "Prompt".
fn score_text(query: &str, text: &str) -> Option<u32> {
    let t = text.as_bytes();
    let mut score = 0u32;
    let mut at = 0usize;
    let mut last: Option<usize> = None;
    let mut first: Option<usize> = None;
    let q: &[u8] = query.as_bytes();
    for (k, &q0) in q.iter().enumerate().filter(|(_, b)| **b != b' ') {
        let found = (at..t.len()).find(|&i| t[i].to_ascii_lowercase() == q0)?;
        // Prefer a later word start over a mid-word hit when the query is not
        // in a run, as long as the rest of the query still fits after it.
        let found = if last.map_or(true, |l| l + 1 != found) && !word_start(t, found) {
            (found..t.len()).find(|&i| word_start(t, i) && t[i].to_ascii_lowercase() == q0 && fits(&q[k + 1..], t, i + 1))
                .unwrap_or(found)
        } else { found };
        score += 1;
        if word_start(t, found) { score += 8; }
        if last.is_some_and(|l| l + 1 == found) { score += 5; }
        first.get_or_insert(found);
        last = Some(found);
        at = found + 1;
    }
    if first == Some(0) { score += 6; }
    // Shorter texts win ties: "Restart" before "Restore Minimized".
    Some(score * 64 + 63u32.saturating_sub(t.len() as u32))
}
fn fits(rest: &[u8], t: &[u8], mut at: usize) -> bool {
    for &q in rest.iter().filter(|b| **b != b' ') {
        match (at..t.len()).find(|&i| t[i].to_ascii_lowercase() == q) { Some(i) => at = i + 1, None => return false }
    }
    true
}
fn word_start(t: &[u8], i: usize) -> bool { i == 0 || !t[i - 1].is_ascii_alphanumeric() }

/// A row's score: its name counts fully; its search words count less.
pub fn score(query: &str, name: &str, words: &str) -> Option<u32> {
    if query.bytes().all(|b| b == b' ') { return Some(0); }
    let by_name = score_text(query, name).map(|s| s * 2);
    let by_word = words.split(' ').filter_map(|w| score_text(query, w)).max();
    by_name.max(by_word)
}

/// Writes the indexes of the rows that match `query` into `out`, best first
/// (equal scores keep list order), and returns how many there are. An empty
/// query keeps every row in list order.
pub fn rank(query: &str, rows: &[(&str, &str)], out: &mut [usize]) -> usize {
    let mut n = 0;
    let mut scores = [0u32; 64];
    for (i, (name, words)) in rows.iter().enumerate().take(out.len().min(scores.len())) {
        let Some(s) = score(query, name, words) else { continue };
        // Insertion sort, stable: a new row goes after rows that score as well.
        let mut j = n;
        while j > 0 && scores[j - 1] < s { out[j] = out[j - 1]; scores[j] = scores[j - 1]; j -= 1; }
        out[j] = i;
        scores[j] = s;
        n += 1;
    }
    n
}

/// Moves `selected` (a row index) by `step` within `order`, wrapping around.
/// A selection outside the order goes to the first match.
pub fn step(order: &[usize], selected: usize, step: i32) -> Option<usize> {
    if order.is_empty() { return None; }
    let Some(at) = order.iter().position(|&i| i == selected) else { return Some(order[0]) };
    let n = order.len() as i32;
    Some(order[(at as i32 + step).rem_euclid(n) as usize])
}
