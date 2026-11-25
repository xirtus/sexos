// SPDX-License-Identifier: GPL-3.0-only
//! Only an exact strong entity validator and byte range can join a hidden part.
use alloc::{format, string::String, vec::Vec};
use sex_fetch::Head;

pub const MAX_RECORDS: usize = 16;
pub const MAX_FILE: u64 = 1 << 16;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record { pub url: String, pub path: String, pub etag: String, pub total: u64 }

fn safe(r: &Record) -> bool {
    let Some(name) = r.path.strip_prefix("Downloads/") else { return false };
    !name.is_empty() && name.len() <= 256 && !name.starts_with('.')
        && !name.bytes().any(|b| b == b'/' || b == b'\\' || b < 0x20 || b == 0x7f)
        && r.url.len() <= 2048 && (r.url.starts_with("https://") || r.url.starts_with("http://"))
        && !r.url.bytes().any(|b| b < 0x20 || b == 0x7f)
        && r.total > 0 && r.total <= 4u64 << 30 && strong(&r.etag)
}
pub fn strong(etag: &str) -> bool {
    etag.len() >= 2 && etag.len() <= 256 && etag.starts_with('"') && etag.ends_with('"')
        && !etag.bytes().any(|b| b < 0x20 || b == 0x7f)
}
pub fn eligible(head: &Head, url: &str, path: &str) -> Option<Record> {
    if head.status != 200 || head.get("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
        || head.get("transfer-encoding").is_some() || head.all("etag").count() != 1
        || head.all("content-length").count() != 1 { return None; }
    let total: u64 = head.get("content-length")?.parse().ok()?;
    let r = Record { url: String::from(url), path: String::from(path), etag: String::from(head.get("etag")?), total };
    safe(&r).then_some(r)
}
pub fn appendable(head: &Head, r: &Record, offset: u64) -> bool {
    if !safe(r) || offset == 0 || offset >= r.total || head.status != 206 || head.get("etag") != Some(r.etag.as_str())
        || head.get("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
        || head.get("transfer-encoding").is_some() || head.all("etag").count() != 1
        || head.all("content-length").count() != 1 || head.all("content-range").count() != 1 { return false; }
    if head.get("content-length").and_then(|v| v.parse::<u64>().ok()) != Some(r.total - offset) { return false; }
    let Some(range) = head.get("content-range").and_then(|v| v.strip_prefix("bytes ")) else { return false };
    let Some((span, size)) = range.split_once('/') else { return false };
    let Some((start, end)) = span.split_once('-') else { return false };
    start.parse::<u64>().ok() == Some(offset) && end.parse::<u64>().ok() == Some(r.total - 1) && size.parse::<u64>().ok() == Some(r.total)
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
    let kept: Vec<_> = records.iter().filter(|r| safe(r)).rev().take(MAX_RECORDS).collect();
    for r in kept.into_iter().rev() {
        out.push_str(&format!("{}\t{}\t{}\t{}\n", r.total, hex(r.path.as_bytes()), hex(r.url.as_bytes()), hex(r.etag.as_bytes())));
    }
    out
}
pub fn decode(text: &str) -> Result<Vec<Record>, ()> {
    if text.len() as u64 > MAX_FILE { return Err(()); }
    let mut lines = text.lines();
    if lines.next() != Some("v1") { return Err(()); }
    let mut out = Vec::new();
    for line in lines {
        if out.len() == MAX_RECORDS { return Err(()); }
        let mut fields = line.split('\t');
        let total = fields.next().ok_or(())?.parse().map_err(|_| ())?;
        let path = unhex(fields.next().ok_or(())?)?;
        let url = unhex(fields.next().ok_or(())?)?;
        let etag = unhex(fields.next().ok_or(())?)?;
        if fields.next().is_some() { return Err(()); }
        let r = Record { url, path, etag, total };
        if !safe(&r) { return Err(()); }
        out.push(r);
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn head(status: u16, etag: &str, range: Option<&str>, len: u64) -> Head {
        let mut h = Head { status, ..Head::default() };
        h.headers.push(("etag".into(), etag.into()));
        h.headers.push(("content-length".into(), format!("{len}")));
        if let Some(r) = range { h.headers.push(("content-range".into(), r.into())); }
        h
    }
    #[test]
    fn only_exact_strong_entity_and_range_can_append() {
        let r = eligible(&head(200, "\"v1\"", None, 100), "https://example.test/file", "Downloads/file").unwrap();
        assert_eq!(decode(&encode(&[r.clone()])).unwrap(), [r.clone()]);
        assert!(appendable(&head(206, "\"v1\"", Some("bytes 40-99/100"), 60), &r, 40));
        assert!(!appendable(&head(206, "\"v2\"", Some("bytes 40-99/100"), 60), &r, 40));
        assert!(!appendable(&head(206, "\"v1\"", Some("bytes 0-59/100"), 60), &r, 40));
        assert!(!appendable(&head(200, "\"v1\"", None, 100), &r, 40));
        assert!(!appendable(&head(416, "\"v1\"", None, 0), &r, 40));
        assert!(eligible(&head(200, "W/\"v1\"", None, 100), "https://example.test/file", "Downloads/file").is_none());
        let mut dated = head(200, "\"v1\"", None, 100);
        dated.headers.retain(|(k, _)| k != "etag");
        dated.headers.push(("last-modified".into(), "Wed, 25 Nov 2026 00:00:00 GMT".into()));
        assert!(eligible(&dated, "https://example.test/file", "Downloads/file").is_none());
        let mut ambiguous = head(206, "\"v1\"", Some("bytes 40-99/100"), 60);
        ambiguous.headers.push(("etag".into(), "\"v2\"".into()));
        assert!(!appendable(&ambiguous, &r, 40));
    }
}
