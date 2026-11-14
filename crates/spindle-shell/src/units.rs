// SPDX-License-Identifier: GPL-3.0-only
//! Number literals with units (`1mb`, `10KiB`, `5sec`, `100ms`), dates, and how
//! filesizes, durations, dates and floats are shown.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

const NS_PER_SEC: i64 = 1_000_000_000;

/// Filesize units, lower case: the decimal ones count thousands, the `i` ones 1024s.
const SIZE_UNITS: &[(&str, i64)] = &[
    ("b", 1),
    ("kb", 1_000), ("mb", 1_000_000), ("gb", 1_000_000_000), ("tb", 1_000_000_000_000), ("pb", 1_000_000_000_000_000),
    ("kib", 1 << 10), ("mib", 1 << 20), ("gib", 1 << 30), ("tib", 1 << 40), ("pib", 1 << 50),
];

/// Duration units, in nanoseconds.
const DURATION_UNITS: &[(&str, i64)] = &[
    ("ns", 1), ("us", 1_000), ("µs", 1_000), ("ms", 1_000_000), ("sec", NS_PER_SEC), ("min", 60 * NS_PER_SEC),
    ("hr", 3600 * NS_PER_SEC), ("day", 86_400 * NS_PER_SEC), ("wk", 7 * 86_400 * NS_PER_SEC),
];

/// The unit names, for completion and help.
pub fn unit_names() -> Vec<&'static str> {
    SIZE_UNITS.iter().chain(DURATION_UNITS.iter()).map(|(n, _)| *n).collect()
}

/// A word that is a number, a number with a unit, or a date, as a value.
pub fn parse_literal(word: &str) -> Option<Value> {
    match word {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        "null" => return Some(Value::Nothing),
        _ => {}
    }
    if let Some(d) = parse_date(word) { return Some(Value::Date(d)); }
    parse_number(word)
}

/// A number, with an optional filesize or duration unit.
pub fn parse_number(word: &str) -> Option<Value> {
    let bytes = word.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') { i += 1; }
    if i >= bytes.len() || !(bytes[i].is_ascii_digit() || (bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())) {
        return None;
    }
    // Hex, octal and binary integers.
    let digits = &word[i..];
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = digits.strip_prefix(prefix) {
            let clean: String = rest.chars().filter(|c| *c != '_').collect();
            let v = i64::from_str_radix(&clean, radix).ok()?;
            return Some(Value::Int(if bytes[0] == b'-' { -v } else { v }));
        }
    }
    let mut j = i;
    let mut is_float = false;
    while j < bytes.len() {
        let c = bytes[j];
        if c.is_ascii_digit() || c == b'_' { j += 1; }
        else if c == b'.' && !is_float && j + 1 < bytes.len() && bytes[j + 1].is_ascii_digit() { is_float = true; j += 1; }
        else if (c == b'e' || c == b'E') && j + 1 < bytes.len()
            && (bytes[j + 1].is_ascii_digit() || ((bytes[j + 1] == b'-' || bytes[j + 1] == b'+') && j + 2 < bytes.len() && bytes[j + 2].is_ascii_digit())) {
            is_float = true;
            j += 2;
        }
        else { break; }
    }
    let num: String = word[..j].chars().filter(|c| *c != '_').collect();
    let unit = &word[j..];
    if unit.is_empty() {
        return if is_float { num.parse::<f64>().ok().map(Value::Float) } else { num.parse::<i64>().ok().map(Value::Int) };
    }
    let lower = unit.to_ascii_lowercase();
    if let Some(&(_, mult)) = SIZE_UNITS.iter().find(|(n, _)| *n == lower) {
        return Some(Value::Filesize(scale(&num, is_float, mult)?));
    }
    if let Some(&(_, mult)) = DURATION_UNITS.iter().find(|(n, _)| *n == unit) {
        return Some(Value::Duration(scale(&num, is_float, mult)?));
    }
    None
}

fn scale(num: &str, is_float: bool, mult: i64) -> Option<i64> {
    if is_float {
        let f = num.parse::<f64>().ok()? * mult as f64;
        Some(round_f64(f))
    } else {
        num.parse::<i64>().ok()?.checked_mul(mult)
    }
}

/// Rounds half away from zero, without libm.
pub fn round_f64(f: f64) -> i64 {
    if f >= 0.0 { (f + 0.5) as i64 } else { (f - 0.5) as i64 }
}

/// Floor without libm.
pub fn floor_f64(f: f64) -> f64 {
    let t = f as i64 as f64;
    if t > f { t - 1.0 } else { t }
}

/// `1.5 MiB`, `512 B`: binary units with at most one decimal.
pub fn format_filesize(bytes: i64) -> String {
    let neg = bytes < 0;
    let b = (bytes as i128).abs();
    let sign = if neg { "-" } else { "" };
    if b < 1024 { return format!("{}{} B", sign, b); }
    let units = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut unit = 1024i128;
    let mut idx = 0;
    while idx + 1 < units.len() && b >= unit * 1024 {
        unit *= 1024;
        idx += 1;
    }
    let tenths = (b * 10 + unit / 2) / unit;
    if tenths % 10 == 0 { format!("{}{} {}", sign, tenths / 10, units[idx]) }
    else { format!("{}{}.{} {}", sign, tenths / 10, tenths % 10, units[idx]) }
}

/// `2min 30sec`: every unit that is not zero, largest first.
pub fn format_duration(ns: i64) -> String {
    if ns == 0 { return String::from("0sec"); }
    let mut left = (ns as i128).abs();
    let mut parts: Vec<String> = Vec::new();
    for &(name, mult) in DURATION_UNITS.iter().rev() {
        if name == "µs" { continue; }
        let m = mult as i128;
        if left >= m {
            parts.push(format!("{}{}", left / m, name));
            left %= m;
        }
    }
    let joined = parts.join(" ");
    if ns < 0 { format!("-{}", joined) } else { joined }
}

/// A float that always shows a decimal point: `1.0`, `2.5`.
pub fn format_float(f: f64) -> String {
    if f.is_nan() { return String::from("NaN"); }
    if f.is_infinite() { return String::from(if f > 0.0 { "inf" } else { "-inf" }); }
    let s = format!("{}", f);
    if s.contains('.') || s.contains('e') { s } else { format!("{}.0", s) }
}

/// Unix seconds as `2026-11-14T09:30:00Z`.
pub fn format_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, rem / 60 % 60, rem % 60)
}

/// `YYYY-MM-DD`, optionally followed by `THH:MM[:SS]` and `Z`, as unix seconds (UTC).
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' { return None; }
    let num = |r: core::ops::Range<usize>| -> Option<i64> {
        let t = s.get(r)?;
        if !t.bytes().all(|c| c.is_ascii_digit()) { return None; }
        t.parse::<i64>().ok()
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) { return None; }
    let mut secs = days_from_civil(y, m as u32, d as u32) * 86_400;
    let rest = &s[10..];
    if rest.is_empty() { return Some(secs); }
    let rest = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' '))?;
    let rest = rest.strip_suffix('Z').unwrap_or(rest);
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 { return None; }
    let mut mult = 3600;
    for p in parts {
        let whole = p.split('.').next()?;
        if whole.len() != 2 || !whole.bytes().all(|c| c.is_ascii_digit()) { return None; }
        secs += whole.parse::<i64>().ok()? * mult;
        mult /= 60;
    }
    Some(secs)
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) to days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}
