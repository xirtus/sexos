// SPDX-License-Identifier: GPL-3.0-only
//! Episodes from file names, for Qupid's "Keep watching": which show, season and
//! episode a file is, and which file comes next. Adapted from brootlite's binge
//! matcher (StreamSimilarityMatcher): when two files claim the same next episode, the
//! one most like the file just played wins, by release group, resolution, codec and
//! audio or picture flavour. Names only: nothing here opens a file.
//!
//! Understood: `Show.S01E02`, `show s1e2`, `Show 1x02`, `Show - 02 [Group]` (the anime
//! style), and `Show Episode 12` / `Ep 12`. A name with none of these is not an episode.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Episode {
    /// The show's words, lower case, joined by single spaces: "the expanse".
    pub show: String,
    /// 1 when the name gives only an episode number.
    pub season: u32,
    pub episode: u32,
}

/// The file name without its folders.
pub fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    let name = base(name);
    match name.rfind('.') {
        Some(dot) if name.len() - dot <= 5 && name[dot + 1..].bytes().all(|b| b.is_ascii_alphanumeric()) => &name[..dot],
        _ => name,
    }
}

/// Lower-case words of `text`, separators (`.`, `_`, `-`, spaces) folded to one space,
/// bracketed tags (`[Group]`, `(2019)`) left out.
fn words(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() || c == '\'' => out.extend(c.to_lowercase()),
            _ => if !out.is_empty() && !out.ends_with(' ') { out.push(' ') },
        }
    }
    while out.ends_with(' ') { out.pop(); }
    out
}

fn digits(b: &[u8], at: usize, max: usize) -> Option<(u32, usize)> {
    let n = b[at..].iter().take(max + 1).take_while(|c| c.is_ascii_digit()).count();
    if n == 0 || n > max { return None; }
    let v = core::str::from_utf8(&b[at..at + n]).ok()?.parse().ok()?;
    Some((v, at + n))
}

fn boundary(b: &[u8], at: usize) -> bool {
    at == 0 || !b[at - 1].is_ascii_alphanumeric()
}

/// `S01E02` / `s1e2` (also `S01.E02`): (season, episode, start, end).
fn sxe(b: &[u8]) -> Option<(u32, u32, usize, usize)> {
    for i in 0..b.len() {
        if !(b[i] == b's' || b[i] == b'S') || !boundary(b, i) { continue; }
        let Some((s, mut j)) = digits(b, i + 1, 3) else { continue };
        if j < b.len() && (b[j] == b'.' || b[j] == b' ' || b[j] == b'_') { j += 1; }
        if j >= b.len() || !(b[j] == b'e' || b[j] == b'E') { continue; }
        let Some((e, k)) = digits(b, j + 1, 4) else { continue };
        return Some((s, e, i, k));
    }
    None
}

/// `1x02`.
fn nxm(b: &[u8]) -> Option<(u32, u32, usize, usize)> {
    for i in 0..b.len() {
        if !b[i].is_ascii_digit() || !boundary(b, i) { continue; }
        let Some((s, j)) = digits(b, i, 2) else { continue };
        if j >= b.len() || !(b[j] == b'x' || b[j] == b'X') { continue; }
        let Some((e, k)) = digits(b, j + 1, 3) else { continue };
        if k < b.len() && b[k].is_ascii_alphanumeric() { continue; }
        return Some((s, e, i, k));
    }
    None
}

/// `Episode 12`, `Ep 12`, `Ep.12`, `E12`: (episode, start, end).
fn ep_word(lower: &[u8]) -> Option<(u32, usize, usize)> {
    for word in [&b"episode"[..], b"ep", b"e"] {
        let mut i = 0;
        while let Some(off) = lower[i..].windows(word.len()).position(|w| w == word) {
            let at = i + off;
            i = at + 1;
            if !boundary(lower, at) { continue; }
            let mut j = at + word.len();
            while j < lower.len() && (lower[j] == b' ' || lower[j] == b'.' || lower[j] == b'_') && word.len() > 1 { j += 1; }
            if let Some((e, k)) = digits(lower, j, 4) {
                if k < lower.len() && lower[k].is_ascii_alphanumeric() { continue; }
                return Some((e, at, k));
            }
        }
    }
    None
}

/// `Show - 02 [Group]`: a lone number after " - ", (episode, start).
fn dash_number(b: &[u8]) -> Option<(u32, usize)> {
    let mut i = 0;
    while let Some(off) = b[i..].windows(3).position(|w| w == b" - ") {
        let at = i + off;
        i = at + 1;
        let j = at + 3;
        if let Some((e, k)) = digits(b, j, 4) {
            // A year is not an episode; "v2" after it is a re-release of the same one.
            let mut k2 = k;
            if k2 < b.len() && b[k2] == b'v' && b.get(k2 + 1).is_some_and(|c| c.is_ascii_digit()) { k2 += 2; }
            if (1900..2100).contains(&e) && k - j == 4 { continue; }
            if k2 == b.len() || matches!(b[k2], b' ' | b'[' | b'(' | b'.' | b'_') { return Some((e, at)); }
        }
    }
    None
}

/// The episode a file name names, if it names one.
pub fn parse(path: &str) -> Option<Episode> {
    let name = stem(path);
    let b = name.as_bytes();
    let lower: Vec<u8> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
    let (season, episode, show_end) = if let Some((s, e, at, _)) = sxe(b).or_else(|| nxm(b)) {
        (s, e, at)
    } else if let Some((e, at)) = dash_number(b) {
        (1, e, at)
    } else if let Some((e, at, _)) = ep_word(&lower) {
        (1, e, at)
    } else {
        return None;
    };
    let show = words(&name[..show_end]);
    // "s01e02.mkv" alone: no show words; fine, the folder groups those.
    Some(Episode { show, season, episode })
}

const IGNORED_TAGS: &[&str] = &[
    "2160p", "1440p", "1080p", "720p", "480p", "4k", "uhd", "fhd", "hd", "hevc", "x265", "h265",
    "x264", "h264", "avc", "av1", "10bit", "10-bit", "hdr", "hdr10", "dv", "web-dl", "webrip",
    "bluray", "bdrip", "remux", "aac", "eac3", "atmos", "multi", "dual-audio",
];

fn is_ignored(tag: &str) -> bool {
    IGNORED_TAGS.iter().any(|t| t.eq_ignore_ascii_case(tag)) || tag.bytes().all(|b| b.is_ascii_digit())
}

/// Who released the file: `[SubsPlease]` at the front, or `-NTb` at the end. Lower case.
/// Brackets elsewhere (`[rarbg]`, a checksum) are where a file came from, not who made it.
pub fn release_group(path: &str) -> Option<String> {
    let name = base(path);
    if let Some(rest) = name.strip_prefix('[') {
        let tag = rest[..rest.find(']')?].trim();
        if (2..=28).contains(&tag.len()) && !is_ignored(tag) {
            return Some(tag.to_ascii_lowercase());
        }
    }
    let s = stem(name).trim_end();
    // Drop trailing brackets like "[rarbg]".
    let mut s = s;
    while s.ends_with(']') {
        match s.rfind('[') { Some(i) => s = s[..i].trim_end(), None => break }
    }
    let dash = s.rfind('-')?;
    let tag = &s[dash + 1..];
    if (2..=18).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphanumeric()) && !is_ignored(tag) && !s[..dash].ends_with(' ') {
        return Some(tag.to_ascii_lowercase());
    }
    None
}

/// Picture height a name claims: 2160, 1080, 720...
pub fn resolution(path: &str) -> u32 {
    let lower = base(path).to_ascii_lowercase();
    for (needle, h) in [("2160p", 2160), ("4k", 2160), ("uhd", 2160), ("1440p", 1440), ("1080p", 1080), ("1080i", 1080), ("720p", 720), ("576p", 576), ("480p", 480)] {
        if lower.contains(needle) { return h; }
    }
    0
}

/// Video codec a name claims.
pub fn codec(path: &str) -> Option<&'static str> {
    let l = base(path).to_ascii_lowercase();
    if ["hevc", "x265", "h265", "h.265"].iter().any(|n| l.contains(n)) { Some("hevc") }
    else if l.contains("av1") { Some("av1") }
    else if ["x264", "h264", "h.264", "avc"].iter().any(|n| l.contains(n)) { Some("h264") }
    else { None }
}

/// Sound and picture flavours a name claims, as a bit set.
pub fn flavours(path: &str) -> u32 {
    let l = base(path).to_ascii_lowercase();
    let mut set = 0u32;
    for (i, needles) in [
        &["dolby vision", ".dv.", " dv "][..], &["hdr"], &["10bit", "10-bit"], &["atmos"], &["truehd"],
        &["eac3", "ddp"], &["flac"], &["dual audio", "dual-audio"], &["multi-sub", "multisub"], &["dub"],
    ].iter().enumerate() {
        if needles.iter().any(|n| l.contains(n)) { set |= 1 << i; }
    }
    set
}

/// How much `cand` looks like it came from the same release as `prev`; higher is more.
pub fn similarity(prev: &str, cand: &str) -> i32 {
    let mut score = 0i32;
    if let (Some(a), Some(b)) = (release_group(prev), release_group(cand)) {
        if a == b { score += 65; }
    }
    let (rp, rc) = (resolution(prev), resolution(cand));
    if rp > 0 && rc > 0 {
        if rp == rc { score += 30 } else { score -= (rp.abs_diff(rc) as i32) / 60; }
    }
    if let (Some(a), Some(b)) = (codec(prev), codec(cand)) {
        if a == b { score += 20; }
    }
    score += 8 * (flavours(prev) & flavours(cand)).count_ones() as i32;
    let (ep, ec) = (base(prev).rsplit('.').next(), base(cand).rsplit('.').next());
    if ep.is_some() && ep == ec { score += 5; }
    score
}

/// The file in `candidates` that plays after `current`: the same show's next episode,
/// in (season, episode) order. Specials (season 0) are skipped unless one is playing.
/// Among copies of that episode the one most like `current` wins. None when nothing
/// follows or `current` is not an episode.
pub fn next_episode(current: &str, candidates: &[&str]) -> Option<usize> {
    let now = parse(current)?;
    let mut best: Option<(u32, u32, i32, usize)> = None;
    for (i, cand) in candidates.iter().enumerate() {
        if *cand == current { continue; }
        let Some(ep) = parse(cand) else { continue };
        if ep.show != now.show && !(ep.show.is_empty() || now.show.is_empty()) { continue; }
        if (ep.season, ep.episode) <= (now.season, now.episode) { continue; }
        if ep.season == 0 && now.season != 0 { continue; }
        let sim = similarity(current, cand);
        let better = match best {
            None => true,
            Some((s, e, bs, _)) => (ep.season, ep.episode) < (s, e) || ((ep.season, ep.episode) == (s, e) && sim > bs),
        };
        if better { best = Some((ep.season, ep.episode, sim, i)); }
    }
    best.map(|b| b.3)
}

/// "S01E04" for a card.
pub fn label(ep: &Episode) -> String {
    alloc::format!("S{:02}E{:02}", ep.season, ep.episode)
}
