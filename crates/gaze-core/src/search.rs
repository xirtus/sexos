// SPDX-License-Identifier: GPL-3.0-only
//! Faces-free smart search engine by date, place, album, and edit state.
//!
//! Gaze strictly rejects facial recognition or person-clustering queries (`face:`, `person:`).
//! Smart queries combine tokens with logical AND so queries like `"2026-05 Paris"`,
//! `"autumn Japan"`, `"2025..2026 Europe"`, or `"edited France"` work instantaneously offline.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::develop::film_stock_slug;
use crate::library::PhotoRecord;
use crate::sidecar::{ColorLabel, CullFlag};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    pub fn contains_month(self, month: u8) -> bool {
        match self {
            Season::Spring => matches!(month, 3..=5),
            Season::Summer => matches!(month, 6..=8),
            Season::Autumn => matches!(month, 9..=11),
            Season::Winter => matches!(month, 12 | 1 | 2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryToken {
    Year(u16),
    YearMonth(u16, u8),
    ExactDate(u16, u8, u8),
    YearRange(u16, u16),
    YearMonthRange((u16, u8), (u16, u8)),
    Month(u8),
    Season(Season),
    Place(String),
    HasGps,
    Album(String),
    Favorite,
    Edited,
    StarsMin(u8),
    Flag(CullFlag),
    Keepers,
    Label(ColorLabel),
    Keyword(String),
    Film(String),
    Text(String),
}

/// Parsed faces-free smart query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmartQuery {
    pub raw: String,
    pub tokens: Vec<QueryToken>,
    /// True if a `face:` / `person:` / `biometric:` token was entered; Gaze refuses biometric matching.
    pub rejected_biometric: bool,
}

impl SmartQuery {
    pub fn parse(raw: &str) -> Self {
        let mut tokens = Vec::new();
        let mut rejected_biometric = false;

        for word in raw.split_whitespace() {
            let clean = word.trim_matches(|c: char| matches!(c, ',' | ';'));
            if clean.is_empty() {
                continue;
            }
            let lower = to_ascii_lower(clean);

            // Faces-free guardrail: explicitly refuse face/person/biometric queries
            if lower.starts_with("face:")
                || lower.starts_with("faces:")
                || lower.starts_with("person:")
                || lower.starts_with("people:")
                || lower.starts_with("biometric:")
            {
                rejected_biometric = true;
                continue;
            }

            if let Some(rest) = lower.strip_prefix("date:") {
                if let Some(tok) = parse_date_or_range(rest) {
                    tokens.push(tok);
                    continue;
                }
            }

            if let Some(rest) = lower
                .strip_prefix("place:")
                .or_else(|| lower.strip_prefix("loc:"))
                .or_else(|| lower.strip_prefix("location:"))
            {
                if !rest.is_empty() {
                    tokens.push(QueryToken::Place(rest.to_string()));
                }
                continue;
            }

            if let Some(rest) = lower.strip_prefix("album:") {
                if !rest.is_empty() {
                    tokens.push(QueryToken::Album(rest.to_string()));
                }
                continue;
            }

            if let Some(rest) = lower
                .strip_prefix("stars:")
                .or_else(|| lower.strip_prefix("star:"))
                .or_else(|| lower.strip_prefix("rating:"))
                .or_else(|| lower.strip_prefix("stars>="))
            {
                let digits = rest.trim_start_matches(">=");
                if let Ok(s) = digits.parse::<u8>() {
                    tokens.push(QueryToken::StarsMin(s.min(5)));
                    continue;
                }
            }

            if let Some(rest) = lower.strip_prefix("flag:") {
                tokens.push(QueryToken::Flag(CullFlag::from_slug(rest)));
                continue;
            }

            if let Some(rest) = lower.strip_prefix("label:") {
                tokens.push(QueryToken::Label(ColorLabel::from_slug(rest)));
                continue;
            }

            if let Some(rest) = lower
                .strip_prefix("kw:")
                .or_else(|| lower.strip_prefix("tag:"))
            {
                if !rest.is_empty() {
                    tokens.push(QueryToken::Keyword(rest.to_string()));
                }
                continue;
            }

            if let Some(rest) = lower.strip_prefix("film:") {
                if !rest.is_empty() {
                    tokens.push(QueryToken::Film(rest.to_string()));
                }
                continue;
            }

            if matches!(lower.as_str(), "pick" | "picked") {
                tokens.push(QueryToken::Flag(CullFlag::Pick));
                continue;
            }

            if matches!(lower.as_str(), "reject" | "rejected") {
                tokens.push(QueryToken::Flag(CullFlag::Reject));
                continue;
            }

            if matches!(lower.as_str(), "keeper" | "keepers") {
                tokens.push(QueryToken::Keepers);
                continue;
            }

            if matches!(lower.as_str(), "has:gps" | "has:place" | "located" | "gps") {
                tokens.push(QueryToken::HasGps);
                continue;
            }

            if matches!(lower.as_str(), "fav" | "favorite" | "favorites" | "starred" | "has:fav") {
                tokens.push(QueryToken::Favorite);
                continue;
            }

            if matches!(lower.as_str(), "edited" | "has:edits" | "has:edit") {
                tokens.push(QueryToken::Edited);
                continue;
            }

            if let Some(tok) = parse_date_or_range(&lower) {
                tokens.push(tok);
                continue;
            }

            if let Some(m) = parse_month_name(&lower) {
                tokens.push(QueryToken::Month(m));
                continue;
            }

            if let Some(season) = parse_season(&lower) {
                tokens.push(QueryToken::Season(season));
                continue;
            }

            tokens.push(QueryToken::Text(lower));
        }

        Self {
            raw: raw.to_string(),
            tokens,
            rejected_biometric,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty() && !self.rejected_biometric
    }

    /// Evaluates whether `record` (and its `album_names`) satisfies every token in `self`.
    pub fn matches(&self, record: &PhotoRecord, album_names: &[&str]) -> bool {
        if self.rejected_biometric {
            return false;
        }
        for token in &self.tokens {
            if !match_token(token, record, album_names) {
                return false;
            }
        }
        true
    }
}

fn match_token(token: &QueryToken, record: &PhotoRecord, album_names: &[&str]) -> bool {
    match token {
        QueryToken::Year(y) => record.meta.date.map_or(false, |d| d.year == *y),
        QueryToken::YearMonth(y, m) => record.meta.date.map_or(false, |d| d.year == *y && d.month == *m),
        QueryToken::ExactDate(y, m, day) => {
            record.meta.date.map_or(false, |d| d.year == *y && d.month == *m && d.day == *day)
        }
        QueryToken::YearRange(y0, y1) => {
            let (lo, hi) = if y0 <= y1 { (*y0, *y1) } else { (*y1, *y0) };
            record.meta.date.map_or(false, |d| d.year >= lo && d.year <= hi)
        }
        QueryToken::YearMonthRange(a, b) => {
            let (lo, hi) = if a <= b { (*a, *b) } else { (*b, *a) };
            record.meta.date.map_or(false, |d| {
                let ym = (d.year, d.month);
                ym >= lo && ym <= hi
            })
        }
        QueryToken::Month(m) => record.meta.date.map_or(false, |d| d.month == *m),
        QueryToken::Season(s) => record.meta.date.map_or(false, |d| s.contains_month(d.month)),
        QueryToken::Place(needle) => {
            contains_ci(record.meta.place.as_deref(), needle)
                || contains_ci(record.meta.place_region.as_deref(), needle)
        }
        QueryToken::HasGps => record.meta.gps.is_some() || record.meta.place.is_some(),
        QueryToken::Album(needle) => album_names.iter().any(|a| contains_ci(Some(a), needle)),
        QueryToken::Favorite => record.favorite,
        QueryToken::Edited => record.is_edited(),
        QueryToken::StarsMin(min_s) => record.stars >= *min_s,
        QueryToken::Flag(flag) => record.flag == *flag,
        QueryToken::Keepers => record.is_keeper(),
        QueryToken::Label(lbl) => record.color_label == *lbl,
        QueryToken::Keyword(needle) => record.keywords.iter().any(|k| contains_ci(Some(k), needle)),
        QueryToken::Film(needle) => record.develop.film_stock.map_or(false, |f| {
            contains_ci(Some(f.name()), needle) || contains_ci(Some(film_stock_slug(f)), needle)
        }),
        QueryToken::Text(needle) => {
            contains_ci(record.meta.place.as_deref(), needle)
                || contains_ci(record.meta.place_region.as_deref(), needle)
                || contains_ci(Some(&record.name), needle)
                || contains_ci(record.meta.camera.as_deref(), needle)
                || contains_ci(record.meta.lens.as_deref(), needle)
                || contains_ci(Some(&record.meta.format_name), needle)
                || record.keywords.iter().any(|k| contains_ci(Some(k), needle))
                || record
                    .develop
                    .film_stock
                    .map_or(false, |f| contains_ci(Some(f.name()), needle))
                || album_names.iter().any(|a| contains_ci(Some(a), needle))
                || record
                    .meta
                    .date
                    .map_or(false, |d| contains_ci(Some(&d.format_ymd()), needle))
        }
    }
}

fn contains_ci(haystack: Option<&str>, needle_lower: &str) -> bool {
    let Some(h) = haystack else { return false };
    if needle_lower.is_empty() {
        return true;
    }
    to_ascii_lower(h).contains(needle_lower)
}

fn to_ascii_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn parse_u16_exact(s: &str, len: usize) -> Option<u16> {
    if s.len() != len || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u16>().ok()
}

fn parse_date_or_range(s: &str) -> Option<QueryToken> {
    if let Some((left, right)) = s.split_once("..") {
        if let (Some(y0), Some(y1)) = (parse_u16_exact(left, 4), parse_u16_exact(right, 4)) {
            if (1900..=2199).contains(&y0) && (1900..=2199).contains(&y1) {
                return Some(QueryToken::YearRange(y0, y1));
            }
        }
        if let (Some(ym0), Some(ym1)) = (parse_ym(left), parse_ym(right)) {
            return Some(QueryToken::YearMonthRange(ym0, ym1));
        }
        return None;
    }

    if let Some(y) = parse_u16_exact(s, 4) {
        if (1900..=2199).contains(&y) {
            return Some(QueryToken::Year(y));
        }
    }
    if let Some((y, m)) = parse_ym(s) {
        return Some(QueryToken::YearMonth(y, m));
    }
    if let Some((y, m, d)) = parse_ymd(s) {
        return Some(QueryToken::ExactDate(y, m, d));
    }
    None
}

fn parse_ym(s: &str) -> Option<(u16, u8)> {
    if s.len() != 7 {
        return None;
    }
    let b = s.as_bytes();
    if !matches!(b[4], b'-' | b'/' | b':') {
        return None;
    }
    let y = parse_u16_exact(&s[0..4], 4)?;
    let m = parse_u16_exact(&s[5..7], 2)? as u8;
    if (1900..=2199).contains(&y) && (1..=12).contains(&m) {
        Some((y, m))
    } else {
        None
    }
}

fn parse_ymd(s: &str) -> Option<(u16, u8, u8)> {
    if s.len() != 10 {
        return None;
    }
    let b = s.as_bytes();
    if !matches!(b[4], b'-' | b'/' | b':') || !matches!(b[7], b'-' | b'/' | b':') {
        return None;
    }
    let y = parse_u16_exact(&s[0..4], 4)?;
    let m = parse_u16_exact(&s[5..7], 2)? as u8;
    let d = parse_u16_exact(&s[8..10], 2)? as u8;
    if (1900..=2199).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d) {
        Some((y, m, d))
    } else {
        None
    }
}

fn parse_month_name(lower: &str) -> Option<u8> {
    match lower {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_season(lower: &str) -> Option<Season> {
    match lower {
        "spring" => Some(Season::Spring),
        "summer" => Some(Season::Summer),
        "autumn" | "fall" => Some(Season::Autumn),
        "winter" => Some(Season::Winter),
        _ => None,
    }
}
