// SPDX-License-Identifier: GPL-3.0-only
//! The words a program was started with, as `argv`.
//!
//! Shell passes what was typed after the program's name as one line. It splits on
//! spaces, and a part in double quotes stays one word (`run pdf "My file.pdf"`), with
//! `\"` and `\\` inside quotes for a quote and a backslash.

/// The words of `line`, in order.
pub fn split(line: &[u8]) -> Vec<Vec<u8>> {
    let mut words = Vec::new();
    let mut word: Option<Vec<u8>> = None;
    let mut quoted = false;
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        match b {
            b'"' => { quoted = !quoted; word.get_or_insert_with(Vec::new); }
            b'\\' if quoted && matches!(line.get(i + 1), Some(b'"' | b'\\')) => { i += 1; word.get_or_insert_with(Vec::new).push(line[i]); }
            b' ' | b'\t' | b'\n' | b'\r' if !quoted => { if let Some(w) = word.take() { words.push(w); } }
            _ => word.get_or_insert_with(Vec::new).push(b),
        }
        i += 1;
    }
    if let Some(w) = word { words.push(w); }
    words
}

#[cfg(test)]
mod tests {
    use super::split;
    #[test]
    fn words_and_quotes() {
        assert_eq!(split(b"  a  bc d "), vec![b"a".to_vec(), b"bc".to_vec(), b"d".to_vec()]);
        assert_eq!(split(br#"open "My file.pdf" -p 3"#), vec![b"open".to_vec(), b"My file.pdf".to_vec(), b"-p".to_vec(), b"3".to_vec()]);
        assert_eq!(split(br#""" x"#), vec![b"".to_vec(), b"x".to_vec()]);
        assert_eq!(split(br#""a\"b\\""#), vec![br#"a"b\"#.to_vec()]);
        assert!(split(b"").is_empty());
    }
}
