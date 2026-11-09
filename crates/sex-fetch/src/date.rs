// SPDX-License-Identifier: GPL-3.0-only
//! HTTP dates (`Expires`), read leniently as RFC 6265 asks: find a time, a day, a
//! month and a year among the tokens, in any order.

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since 1970 for a cookie or HTTP date, or `None`.
pub fn parse_http_date(s: &str) -> Option<u64> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    for tok in s.split(|c: char| !(c.is_ascii_alphanumeric() || c == ':')).filter(|t| !t.is_empty()) {
        if time.is_none() && tok.contains(':') {
            let mut p = tok.split(':').map(|x| x.parse::<u32>().ok());
            if let (Some(Some(h)), Some(Some(m)), Some(Some(sec))) = (p.next(), p.next(), p.next()) { time = Some((h, m, sec)); continue; }
        }
        let digits = tok.bytes().take_while(|b| b.is_ascii_digit()).count();
        if day.is_none() && (1..=2).contains(&digits) && tok.len() == digits { day = tok.parse::<u32>().ok(); continue; }
        if month.is_none() && tok.len() >= 3 {
            let t = tok[..3].to_ascii_lowercase();
            if let Some(i) = MONTHS.iter().position(|m| *m == t) { month = Some(i as u32 + 1); continue; }
        }
        if year.is_none() && (2..=4).contains(&digits) && tok.len() == digits {
            let mut y = tok.parse::<i64>().ok()?;
            if digits == 2 { y += if y >= 70 { 1900 } else { 2000 }; }
            year = Some(y);
        }
    }
    let (h, mi, se) = time?;
    let (d, m, y) = (day?, month?, year?);
    if !(1..=31).contains(&d) || y < 1601 || h > 23 || mi > 59 || se > 59 { return None; }
    let days = days_from_civil(y, m, d);
    let secs = days * 86_400 + (h * 3600 + mi * 60 + se) as i64;
    Some(secs.max(0) as u64)
}
