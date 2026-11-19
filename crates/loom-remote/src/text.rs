// SPDX-License-Identifier: GPL-3.0-only
//! Small text codecs the protocols share: percent-encoding, base64, hex and dates.
use alloc::{string::String, vec::Vec};

fn unreserved(b: u8) -> bool { b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') }

/// Percent-encodes everything but RFC 3986 unreserved bytes (and `/` when `keep_slash`).
pub fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if unreserved(b) || (keep_slash && b == b'/') { o.push(b as char); } else { o.push('%'); o.push(HEX_UP[(b >> 4) as usize] as char); o.push(HEX_UP[(b & 15) as usize] as char); }
    }
    o
}

/// Decodes `%XX` escapes; bad escapes stay as they are.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hexval(b[i + 1]), hexval(b[i + 2])) { o.push(h << 4 | l); i += 3; continue; }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}
fn hexval(c: u8) -> Option<u8> { (c as char).to_digit(16).map(|d| d as u8) }

const HEX_UP: &[u8; 16] = b"0123456789ABCDEF";
const HEX_LOW: &[u8; 16] = b"0123456789abcdef";
pub fn hex(b: &[u8]) -> String { let mut s = String::with_capacity(b.len() * 2); for x in b { s.push(HEX_LOW[(x >> 4) as usize] as char); s.push(HEX_LOW[(x & 15) as usize] as char); } s }

pub fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { if i <= c.len() { s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char); } else { s.push('='); } }
    }
    s
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// `YYYYMMDDTHHMMSSZ`, the form SigV4 signs.
pub fn amz_date(t: u64) -> String {
    let (y, mo, d) = civil_from_days((t / 86400) as i64);
    let s = t % 86400;
    alloc::format!("{y:04}{mo:02}{d:02}T{:02}{:02}{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

fn num(s: &str) -> Option<u32> { if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) { return None; } s.parse().ok() }
fn secs(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<u64> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 { return None; }
    let t = days_from_civil(y, mo, d) * 86400 + (h * 3600 + mi * 60 + s) as i64;
    u64::try_from(t).ok()
}

/// ISO 8601 UTC as S3 gives it: `2009-10-12T17:50:30.000Z` (fraction optional).
pub fn parse_iso8601(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let (y, mo, d) = (num(dp.next()?)?, num(dp.next()?)?, num(dp.next()?)?);
    let time = time.strip_suffix('Z')?;
    let time = time.split('.').next()?;
    let mut tp = time.split(':');
    secs(y as i64, mo, d, num(tp.next()?)?, num(tp.next()?)?, num(tp.next()?)?)
}

/// An HTTP date (RFC 9110 IMF-fixdate): `Sun, 06 Nov 1994 08:49:37 GMT`.
pub fn parse_http_date(s: &str) -> Option<u64> {
    let mut p = s.trim().split_whitespace();
    let _day = p.next()?;
    let d = num(p.next()?)?;
    let mo = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| Some(*m) == p.clone().next())? as u32 + 1;
    p.next();
    let y = num(p.next()?)?;
    let mut tp = p.next()?.split(':');
    secs(y as i64, mo, d, num(tp.next()?)?, num(tp.next()?)?, num(tp.next()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_round_trip_and_parse() {
        assert_eq!(amz_date(1440938160), "20150830T123600Z");
        assert_eq!(parse_iso8601("2015-08-30T12:36:00.000Z"), Some(1440938160));
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784111777));
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 25:49:37 GMT"), None);
        for z in [-800000i64, -1, 0, 59, 60, 11016, 20000, 2932896] { let (y, m, d) = civil_from_days(z); assert_eq!(days_from_civil(y, m, d), z); }
    }
    #[test]
    fn codecs() {
        assert_eq!(base64(b"user:pa ss"), "dXNlcjpwYSBzcw==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(percent_encode("a b/ü~", true), "a%20b/%C3%BC~");
        assert_eq!(percent_encode("a/b", false), "a%2Fb");
        assert_eq!(percent_decode("a%20b%2f%C3%BC%zz%"), "a b/ü%zz%");
    }
}
