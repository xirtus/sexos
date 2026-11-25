// SPDX-License-Identifier: GPL-3.0-only
//! Bounded, versioned history for Kaleidoscope's completed downloads.
//! URLs and names are hex encoded so a site cannot add fake rows with tabs or newlines.
use alloc::{format, string::String, vec::Vec};

pub const MAX_RECORDS: usize = 128;
pub const MAX_FILE: u64 = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub path: String,
    pub bytes: u64,
    pub url: String,
    /// Kept in the current window only; never written to the profile.
    pub private: bool,
}

fn valid(r: &Record) -> bool {
    let Some(name) = r.path.strip_prefix("Downloads/") else { return false };
    !name.is_empty() && name.len() <= 256 && !name.starts_with('.')
        && !name.bytes().any(|b| b == b'/' || b == b'\\' || b < 0x20 || b == 0x7f)
        && r.url.len() <= 2048 && (r.url.starts_with("https://") || r.url.starts_with("http://"))
        && !r.url.bytes().any(|b| b < 0x20 || b == 0x7f)
}

fn hex(data: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for &b in data { out.push(DIGITS[(b >> 4) as usize] as char); out.push(DIGITS[(b & 15) as usize] as char); }
    out
}

fn unhex(s: &str) -> Result<String, ()> {
    if s.len() % 2 != 0 { return Err(()); }
    fn digit(b: u8) -> Result<u8, ()> { match b { b'0'..=b'9' => Ok(b - b'0'), b'a'..=b'f' => Ok(b - b'a' + 10), _ => Err(()) } }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks_exact(2) { out.push(digit(pair[0])? << 4 | digit(pair[1])?); }
    String::from_utf8(out).map_err(|_| ())
}

pub fn encode(records: &[Record]) -> String {
    let mut out = String::from("v1\n");
    let durable: Vec<_> = records.iter().filter(|r| !r.private && valid(r)).rev().take(MAX_RECORDS).collect();
    for r in durable.into_iter().rev() {
        out.push_str(&format!("{}\t{}\t{}\n", r.bytes, hex(r.path.as_bytes()), hex(r.url.as_bytes())));
    }
    out
}

/// Reject the whole file if any row is malformed; the caller may use a previous copy.
pub fn decode(text: &str) -> Result<Vec<Record>, ()> {
    if text.len() as u64 > MAX_FILE { return Err(()); }
    let mut lines = text.lines();
    if lines.next() != Some("v1") { return Err(()); }
    let mut records = Vec::new();
    for line in lines {
        if records.len() == MAX_RECORDS { return Err(()); }
        let mut fields = line.split('\t');
        let bytes = fields.next().ok_or(())?.parse().map_err(|_| ())?;
        let path = unhex(fields.next().ok_or(())?)?;
        let url = unhex(fields.next().ok_or(())?)?;
        if fields.next().is_some() { return Err(()); }
        let r = Record { path, bytes, url, private: false };
        if !valid(&r) { return Err(()); }
        records.push(r);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_round_trip_bounds_and_excludes_private_urls() {
        let public = Record { path: "Downloads/report (2).pdf".into(), bytes: 4012,
            url: "https://example.test/a?x=1&y=2".into(), private: false };
        let secret = Record { path: "Downloads/private.pdf".into(), bytes: 7,
            url: "https://secret.test/item".into(), private: true };
        assert_eq!(decode(&encode(&[public.clone(), secret])).unwrap(), [public]);
        let many = (0..140).map(|i| Record { path: format!("Downloads/{i}.bin"), bytes: i,
            url: "https://example.test/file".into(), private: false }).collect::<Vec<_>>();
        let kept = decode(&encode(&many)).unwrap();
        assert_eq!(kept.len(), MAX_RECORDS);
        assert_eq!(kept.first().unwrap().path, "Downloads/12.bin");
        assert_eq!(kept.last().unwrap().path, "Downloads/139.bin");
    }

    #[test]
    fn malformed_or_forged_rows_are_rejected() {
        for text in ["v2\n", "v1\n1\t2f\t00\n", "v1\n1\t44\t55\textra\n",
            "v1\n1\t446f776e6c6f6164732f2e2e2f6576696c\t68747470733a2f2f612e74657374\n"] {
            assert!(decode(text).is_err(), "{text}");
        }
        let bad = Record { path: "Downloads/evil\nname".into(), bytes: 1,
            url: "https://example.test/".into(), private: false };
        assert_eq!(decode(&encode(&[bad])).unwrap().len(), 0);
    }
}
