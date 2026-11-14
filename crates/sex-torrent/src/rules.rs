// SPDX-License-Identifier: GPL-3.0-only
//! Automatic download rules: qBittorrent's RSS downloader plus a small Sonarr.
//!
//! A [`Rule`] says which feed items to add and where they go:
//! - [`Pattern`]s for must-contain / must-not-contain. Words separated by spaces must
//!   all appear (AND), `|` separates alternatives (OR), `*` and `?` are wildcards, and
//!   `"quoted phrases"` keep their spaces. Matching ignores case and runs in
//!   O(text x pattern) time: there is no backtracking that could explode.
//! - An [`EpisodeFilter`] in qBittorrent's syntax: `1x2-8;2x;3x1-` means season 1
//!   episodes 2 to 8, all of season 2, and season 3 from episode 1 on.
//! - The smart episode filter: each episode is taken once; a later PROPER/REPACK of it
//!   is taken, and with `upgrade_quality` so is a better resolution or source. When one
//!   refresh offers several copies of one episode, the best one wins.
//! - Category, tags, save path, add paused, resolution bounds, and "ignore matches for
//!   N days after a match".
//!
//! [`parse_release`] reads what a release title says: `S01E02`, `S01E01E02`, `1x02`,
//! `Season 1 Episode 2`, a season pack (`S02`, `Season 2`), a date (`2026.11.14`), an
//! anime absolute number (`[Group] Show - 05 [1080p]`), resolution (480p to 2160p),
//! source (WEB-DL, WEBRip, BluRay, Remux, HDTV, DVD, CAM), codec (x264, x265/HEVC, AV1)
//! and PROPER/REPACK/REAL/v2. [`Engine::evaluate`] runs the rules over candidates and
//! says, for each item it takes, which rule took it and why.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::rss::FeedItem;

const MAX_PATTERN: usize = 1024;

/// A compiled must-contain / must-not-contain expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// Alternatives (OR) of terms (AND), lower-case, as chars.
    alts: Vec<Vec<Vec<char>>>,
}

impl Pattern {
    /// Compiles an expression; `None` when it has no terms (no constraint).
    pub fn parse(src: &str) -> Option<Pattern> {
        let src: String = src.chars().take(MAX_PATTERN).collect::<String>().to_lowercase();
        let mut alts = Vec::new();
        for alt in src.split('|').take(64) {
            let mut terms = Vec::new();
            let mut parts = alt.split('"');
            let mut quoted = false;
            while let Some(part) = parts.next() {
                if quoted {
                    if !part.trim().is_empty() { terms.push(part.chars().collect()); }
                } else {
                    terms.extend(part.split_whitespace().map(|w| w.chars().collect::<Vec<char>>()));
                }
                quoted = !quoted;
            }
            terms.truncate(64);
            if !terms.is_empty() { alts.push(terms); }
        }
        (!alts.is_empty()).then_some(Pattern { alts })
    }
    /// Whether `text` satisfies the expression (case-insensitive).
    pub fn matches(&self, text: &str) -> bool {
        let t: Vec<char> = text.to_lowercase().chars().take(4096).collect();
        self.alts.iter().any(|terms| terms.iter().all(|term| term_matches(term, &t)))
    }
}

fn term_matches(term: &[char], text: &[char]) -> bool {
    if term.iter().any(|&c| c == '*' || c == '?') {
        // Unanchored: `*term*`.
        let mut p = Vec::with_capacity(term.len() + 2);
        p.push('*');
        p.extend_from_slice(term);
        p.push('*');
        glob(&p, text)
    } else {
        term.len() <= text.len() && text.windows(term.len()).any(|w| w == term)
    }
}

/// Iterative glob match with single-star backtracking: O(p x t), never exponential.
pub fn glob_chars(p: &[char], t: &[char]) -> bool { glob(p, t) }

fn glob(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) { pi += 1; ti += 1; }
        else if pi < p.len() && p[pi] == '*' { star = Some((pi, ti)); pi += 1; }
        else if let Some((sp, st)) = star { pi = sp + 1; ti = st + 1; star = Some((sp, st + 1)); }
        else { return false; }
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}

/// One `season x episodes` part of an episode filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeasonRange {
    pub season: u32,
    /// First episode; 0 with `to: None` means the whole season.
    pub from: u32,
    /// Last episode, inclusive; `None` is open-ended.
    pub to: Option<u32>,
}

/// qBittorrent's episode filter (`1x2-8;2x;3x1-`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeFilter { pub ranges: Vec<SeasonRange> }

impl EpisodeFilter {
    /// Parses the filter; `None` when any part is malformed.
    pub fn parse(s: &str) -> Option<EpisodeFilter> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        let mut ranges = Vec::new();
        for part in s.split(';').filter(|p| !p.is_empty()).take(256) {
            let (season, eps) = part.split_once(['x', 'X'])?;
            let season: u32 = season.parse().ok()?;
            let r = if eps.is_empty() {
                SeasonRange { season, from: 0, to: None }
            } else if let Some((a, b)) = eps.split_once('-') {
                let from: u32 = a.parse().ok()?;
                let to = if b.is_empty() { None } else { Some(b.parse::<u32>().ok()?) };
                if to.is_some_and(|t| t < from) { return None; }
                SeasonRange { season, from, to }
            } else {
                let e: u32 = eps.parse().ok()?;
                SeasonRange { season, from: e, to: Some(e) }
            };
            ranges.push(r);
        }
        (!ranges.is_empty()).then_some(EpisodeFilter { ranges })
    }
    /// Whether a release passes: a season/episode release when any of its episodes is
    /// in a range; a season pack when its season is wanted from episode 1 on.
    pub fn matches(&self, r: &Release) -> bool {
        let Some(season) = r.season else { return false };
        if r.episodes.is_empty() {
            return self.ranges.iter().any(|g| g.season == season && g.from <= 1 && g.to.is_none());
        }
        r.episodes.iter().any(|&e| self.ranges.iter().any(|g| g.season == season && e >= g.from && g.to.map_or(true, |t| e <= t)))
    }
}

/// Where a release came from, worst to best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Source { Cam, #[default] Unknown, Dvd, Tv, WebRip, WebDl, BluRay, Remux }

/// Video codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Codec { #[default] Unknown, H264, H265, Av1 }

/// What a release title says.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Release {
    /// The show or movie name, lower-case words separated by single spaces.
    pub show: String,
    pub season: Option<u32>,
    /// Episodes (several for `S01E01E02` / `S01E01-E03`); empty for a season pack.
    pub episodes: Vec<u32>,
    /// Air date for daily shows.
    pub date: Option<(u16, u8, u8)>,
    /// Absolute episode number (anime).
    pub absolute: Option<u32>,
    /// Vertical resolution (480, 576, 720, 1080, 2160).
    pub resolution: Option<u16>,
    pub source: Source,
    pub codec: Codec,
    /// PROPER/REPACK/REAL/RERIP count, plus `v2` style revisions.
    pub proper: u8,
}

/// The identity of an episode for "take each episode once".
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EpisodeKey {
    Episode { season: u32, episode: u32 },
    Season(u32),
    Date(u16, u8, u8),
    Absolute(u32),
}

impl core::fmt::Display for EpisodeKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            EpisodeKey::Episode { season, episode } => write!(f, "S{:02}E{:02}", season, episode),
            EpisodeKey::Season(s) => write!(f, "S{:02}", s),
            EpisodeKey::Date(y, m, d) => write!(f, "{:04}-{:02}-{:02}", y, m, d),
            EpisodeKey::Absolute(n) => write!(f, "#{:02}", n),
        }
    }
}

impl Release {
    /// The episode identity, if the title had one.
    pub fn key(&self) -> Option<EpisodeKey> {
        if let Some(s) = self.season {
            return Some(match self.episodes.first() { Some(&e) => EpisodeKey::Episode { season: s, episode: e }, None => EpisodeKey::Season(s) });
        }
        if let Some((y, m, d)) = self.date { return Some(EpisodeKey::Date(y, m, d)); }
        self.absolute.map(EpisodeKey::Absolute)
    }
    /// A single number ordering quality: resolution, then source, then codec.
    pub fn quality_rank(&self) -> u32 {
        let res = match self.resolution { Some(r) if r >= 2160 => 5, Some(r) if r >= 1080 => 4, Some(r) if r >= 720 => 3, Some(_) => 2, None => 1 };
        res * 100 + self.source as u32 * 10 + self.codec as u32
    }
    /// A short label like `S01E05-E06 1080p WEB-DL`.
    pub fn label(&self) -> String {
        let mut s = match self.key() { Some(k) => format!("{}", k), None => String::from("no episode") };
        if self.episodes.len() > 1 { s.push_str(&format!("-E{:02}", self.episodes[self.episodes.len() - 1])); }
        if let Some(r) = self.resolution { s.push_str(&format!(" {}p", r)); }
        let src = match self.source { Source::Cam => " CAM", Source::Unknown => "", Source::Dvd => " DVD", Source::Tv => " HDTV",
            Source::WebRip => " WEBRip", Source::WebDl => " WEB-DL", Source::BluRay => " BluRay", Source::Remux => " Remux" };
        s.push_str(src);
        if self.proper > 0 { s.push_str(" PROPER"); }
        s
    }
}

fn num(s: &str) -> Option<u32> { (!s.is_empty() && s.len() <= 4 && s.bytes().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten() }

/// `s01e02`, `s01e02e03`; returns (season, episodes).
fn sxxeyy(t: &str) -> Option<(u32, Vec<u32>)> {
    let rest = t.strip_prefix('s')?;
    let e = rest.find('e')?;
    let season = num(&rest[..e]).filter(|_| e <= 2)?;
    let mut eps = Vec::new();
    for part in rest[e + 1..].split('e') { eps.push(num(part).filter(|_| part.len() <= 3)?); }
    Some((season, eps))
}

/// Reads what a release title says. Never fails: unknown parts stay `None`.
pub fn parse_release(title: &str) -> Release {
    let mut r = Release::default();
    let mut t: String = title.chars().take(512).collect::<String>().to_lowercase();
    // A leading [Group] tag is not part of the name.
    if t.starts_with('[') { if let Some(e) = t.find(']') { t = String::from(t[e + 1..].trim_start()); } }
    let clean: String = t.chars().map(|c| if c == '.' || c == '_' { ' ' } else { c }).collect();
    let toks: Vec<&str> = clean.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();
    let mut marker: Option<usize> = None;
    let mut i = 0;
    while i < toks.len() {
        let tk = toks[i];
        if marker.is_none() {
            if let Some((s, mut eps)) = sxxeyy(tk) {
                // S01E01-E03 / S01E01-03 range.
                if let Some(n) = toks.get(i + 1).and_then(|n| num(n.strip_prefix('e').unwrap_or(n))) {
                    let last = *eps.last().unwrap_or(&0);
                    if n > last && n - last <= 50 && (toks[i + 1].starts_with('e') || clean.contains(&format!("{}-{}", tk, toks[i + 1]))) {
                        eps.extend(last + 1..=n);
                    }
                }
                r.season = Some(s); r.episodes = eps; marker = Some(i);
            } else if let Some((a, b)) = tk.split_once('x').filter(|(a, b)| a.len() <= 2 && b.len() <= 3) {
                if let (Some(s), Some(e)) = (num(a), num(b)) { r.season = Some(s); r.episodes = alloc::vec![e]; marker = Some(i); }
            } else if tk == "season" || (tk.len() <= 3 && tk.starts_with('s') && num(&tk[1..]).is_some()) {
                let s = if tk == "season" { toks.get(i + 1).and_then(|n| num(n)) } else { num(&tk[1..]) };
                if let Some(s) = s {
                    r.season = Some(s); marker = Some(i);
                    let j = if tk == "season" { i + 2 } else { i + 1 };
                    if toks.get(j) == Some(&"episode") { if let Some(e) = toks.get(j + 1).and_then(|n| num(n)) { r.episodes = alloc::vec![e]; } }
                    else if let Some(e) = toks.get(j).and_then(|n| n.strip_prefix('e')).and_then(num) { r.episodes = alloc::vec![e]; }
                }
            } else if tk.len() == 4 && i + 2 < toks.len() && toks[i + 1].len() == 2 && toks[i + 2].len() == 2 {
                if let (Some(y), Some(m), Some(d)) = (num(tk), num(toks[i + 1]), num(toks[i + 2])) {
                    if (1900..=2100).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d) {
                        r.date = Some((y as u16, m as u8, d as u8)); marker = Some(i);
                    }
                }
            } else if (tk == "ep" || tk == "episode") && i > 0 {
                if let Some(e) = toks.get(i + 1).and_then(|n| num(n)) { r.absolute = Some(e); marker = Some(i); }
            }
        }
        match tk {
            "480p" => r.resolution = Some(480), "576p" => r.resolution = Some(576), "720p" => r.resolution = Some(720),
            "1080p" | "1080i" => r.resolution = Some(1080), "2160p" | "4k" | "uhd" => r.resolution = Some(2160),
            "1920x1080" => r.resolution = Some(1080), "1280x720" => r.resolution = Some(720), "3840x2160" => r.resolution = Some(2160),
            "remux" => r.source = Source::Remux,
            "bluray" | "bdrip" | "brrip" | "bd" | "bdremux" => if r.source < Source::BluRay { r.source = Source::BluRay },
            "webdl" | "amzn" | "nf" | "dsnp" | "hmax" | "atvp" => if r.source < Source::WebDl { r.source = Source::WebDl },
            "web" => if r.source < Source::WebDl { r.source = if toks.get(i + 1) == Some(&"rip") { Source::WebRip } else { Source::WebDl } },
            "webrip" => if r.source < Source::WebRip { r.source = Source::WebRip },
            "hdtv" | "pdtv" | "sdtv" | "dsr" => if r.source < Source::Tv { r.source = Source::Tv },
            "dvdrip" | "dvd" | "dvd5" | "dvd9" => if r.source < Source::Dvd { r.source = Source::Dvd },
            "cam" | "hdcam" | "ts" | "telesync" | "hdts" => r.source = Source::Cam,
            "x264" | "h264" | "avc" => r.codec = r.codec.max(Codec::H264),
            "x265" | "h265" | "hevc" => r.codec = r.codec.max(Codec::H265),
            "av1" => r.codec = Codec::Av1,
            "h" => match toks.get(i + 1) { Some(&"264") => r.codec = r.codec.max(Codec::H264), Some(&"265") => r.codec = r.codec.max(Codec::H265), _ => {} },
            "proper" | "repack" | "rerip" => r.proper = r.proper.saturating_add(1),
            "real" if marker.is_some() => r.proper = r.proper.saturating_add(1),
            _ => {}
        }
        i += 1;
    }
    // Anime: "Show - 05 [1080p]", "Show - 05v2".
    let mut anime_at = None;
    if marker.is_none() {
        let mut from = 0;
        while let Some(p) = t[from..].find(" - ").map(|p| p + from) {
            let after = &t[p + 3..];
            let digits = after.bytes().take_while(|c| c.is_ascii_digit()).count();
            if (1..=4).contains(&digits) {
                let next = after[digits..].chars().next();
                let mut ok = matches!(next, None | Some(' ' | '[' | '(' | '.'));
                if next == Some('v') {
                    if let Some(v) = after[digits + 1..].chars().next().and_then(|c| c.to_digit(10)) { r.proper = r.proper.max(v.saturating_sub(1) as u8); ok = true; }
                }
                if ok { r.absolute = after[..digits].parse().ok(); anime_at = Some(p); break; }
            }
            from = p + 3;
        }
    }
    let name_part: String = match (marker, anime_at) {
        (Some(m), _) => toks[..m].join(" "),
        (None, Some(p)) => t[..p].split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "),
        _ => toks.iter().take_while(|w| !w.ends_with('p') || num(&w[..w.len() - 1]).is_none()).copied().collect::<Vec<_>>().join(" "),
    };
    r.show = name_part;
    r
}

/// One automation rule.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub must_contain: Option<Pattern>,
    pub must_not_contain: Option<Pattern>,
    pub episode_filter: Option<EpisodeFilter>,
    /// Take each episode once (PROPER/REPACK excepted).
    pub smart_filter: bool,
    /// With the smart filter: also take an episode again in better quality.
    pub upgrade_quality: bool,
    pub min_resolution: Option<u16>,
    pub max_resolution: Option<u16>,
    /// Feed URLs this rule applies to; empty means all.
    pub feeds: Vec<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub save_path: Option<String>,
    pub add_paused: Option<bool>,
    /// After a match, ignore further matches for this many days (0 = never ignore).
    pub ignore_days: u32,
}

impl Rule {
    /// An enabled rule with a must-contain expression.
    pub fn new(name: &str, must_contain: &str) -> Rule {
        Rule { name: String::from(name), enabled: true, must_contain: Pattern::parse(must_contain), ..Rule::default() }
    }
    /// Whether the rule's text, feed and quality conditions accept a title (the smart
    /// filter and ignore-days are applied by [`Engine::evaluate`]).
    pub fn accepts(&self, feed: &str, title: &str, rel: &Release) -> bool {
        self.enabled
            && (self.feeds.is_empty() || self.feeds.iter().any(|f| f == feed))
            && self.must_contain.as_ref().map_or(true, |p| p.matches(title))
            && !self.must_not_contain.as_ref().is_some_and(|p| p.matches(title))
            && self.episode_filter.as_ref().map_or(true, |f| f.matches(rel))
            && self.min_resolution.map_or(true, |m| rel.resolution.is_some_and(|r| r >= m))
            && self.max_resolution.map_or(true, |m| rel.resolution.map_or(true, |r| r <= m))
    }
}

/// One item offered to the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub feed: String,
    pub title: String,
    /// Magnet or .torrent URL.
    pub url: String,
}

impl Candidate {
    /// From a feed item; `None` when the item has nothing to download.
    pub fn from_item(feed: &str, item: &FeedItem) -> Option<Candidate> {
        Some(Candidate { feed: String::from(feed), title: item.title.clone(), url: String::from(item.torrent_url()?) })
    }
}

/// What to add, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Index into the candidates passed to [`Engine::evaluate`].
    pub index: usize,
    pub rule: String,
    pub url: String,
    pub title: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub save_path: Option<String>,
    pub paused: Option<bool>,
    pub episode: Option<EpisodeKey>,
    /// Human-readable explanation.
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Grab { quality: u32, proper: u8 }

/// What a rule remembers between evaluations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleState {
    /// Unix seconds of the last match.
    pub last_match: Option<i64>,
    episodes: BTreeMap<(String, EpisodeKey), Grab>,
}

impl RuleState {
    /// How many episodes the smart filter remembers.
    pub fn episodes_len(&self) -> usize { self.episodes.len() }
}

/// Remembered episodes per rule are capped at this many (oldest keys dropped).
pub const MAX_EPISODES: usize = 20_000;

/// The rules and their state.
#[derive(Clone, Debug, Default)]
pub struct Engine {
    /// Evaluated in order; an item is taken by the first rule that takes it.
    pub rules: Vec<Rule>,
    state: BTreeMap<String, RuleState>,
}

impl Engine {
    pub fn new(rules: Vec<Rule>) -> Engine { Engine { rules, state: BTreeMap::new() } }
    /// A rule's state.
    pub fn state(&self, rule: &str) -> Option<&RuleState> { self.state.get(rule) }
    /// Forgets a rule's episode history and last match ("clear downloaded episodes").
    pub fn reset(&mut self, rule: &str) { self.state.remove(rule); }

    /// Runs every rule over `cands` at unix time `now`; returns what to add, in
    /// candidate order, and records the matches.
    pub fn evaluate(&mut self, cands: &[Candidate], now: i64) -> Vec<Decision> {
        let parsed: Vec<Release> = cands.iter().map(|c| parse_release(&c.title)).collect();
        let mut taken = alloc::vec![false; cands.len()];
        let mut out = Vec::new();
        for rule in &self.rules {
            let st = self.state.entry(rule.name.clone()).or_default();
            if rule.ignore_days > 0 && st.last_match.is_some_and(|t| now - t < rule.ignore_days as i64 * 86_400) { continue; }
            let hits: Vec<usize> = (0..cands.len()).filter(|&i| !taken[i] && rule.accepts(&cands[i].feed, &cands[i].title, &parsed[i])).collect();
            let mut chosen: Vec<(usize, String)> = Vec::new();
            if rule.smart_filter {
                // Best copy per episode in this batch.
                let mut best: BTreeMap<(String, EpisodeKey), usize> = BTreeMap::new();
                for &i in &hits {
                    let r = &parsed[i];
                    match r.key() {
                        None => chosen.push((i, String::from("no episode number"))),
                        Some(k) => {
                            let e = best.entry((r.show.clone(), k)).or_insert(i);
                            let cur = &parsed[*e];
                            if (r.proper, r.quality_rank()) > (cur.proper, cur.quality_rank()) { *e = i; }
                        }
                    }
                }
                for (key, i) in best {
                    let r = &parsed[i];
                    let g = Grab { quality: r.quality_rank(), proper: r.proper };
                    let why = match st.episodes.get(&key) {
                        None => Some(format!("new episode {}", r.label())),
                        Some(old) if g.proper > old.proper && g.quality >= old.quality => Some(format!("PROPER/REPACK of {}", r.label())),
                        Some(old) if rule.upgrade_quality && g.quality > old.quality => Some(format!("quality upgrade to {}", r.label())),
                        Some(_) => None,
                    };
                    if let Some(why) = why {
                        if st.episodes.len() >= MAX_EPISODES && !st.episodes.contains_key(&key) { st.episodes.pop_first(); }
                        st.episodes.insert(key, g);
                        chosen.push((i, why));
                    }
                }
                chosen.sort_by_key(|(i, _)| *i);
            } else {
                chosen = hits.into_iter().map(|i| (i, format!("matched ({})", parsed[i].label()))).collect();
            }
            if rule.ignore_days > 0 { chosen.truncate(1); }
            if !chosen.is_empty() { st.last_match = Some(now); }
            for (i, why) in chosen {
                taken[i] = true;
                out.push(Decision {
                    index: i, rule: rule.name.clone(), url: cands[i].url.clone(), title: cands[i].title.clone(),
                    category: rule.category.clone(), tags: rule.tags.clone(), save_path: rule.save_path.clone(),
                    paused: rule.add_paused, episode: parsed[i].key(),
                    reason: format!("rule '{}': {}", rule.name, why),
                });
            }
        }
        out.sort_by_key(|d| d.index);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn cand(t: &str) -> Candidate { Candidate { feed: String::from("f"), title: String::from(t), url: format!("magnet:?dn={}", t) } }

    #[test]
    fn patterns() {
        let p = Pattern::parse("show name 1080p").unwrap();
        assert!(p.matches("Show.Name.S01E01.1080p.WEB"));
        assert!(!p.matches("Show.Name.S01E01.720p.WEB"));
        let p = Pattern::parse("frieren | \"sousou no\"").unwrap();
        assert!(p.matches("[SubsPlease] Sousou no Frieren - 05"));
        assert!(p.matches("Frieren S01E05"));
        let p = Pattern::parse("\"sousou no\"").unwrap();
        assert!(!p.matches("Sousou-no"));
        let p = Pattern::parse("s0?e* *265").unwrap();
        assert!(p.matches("Show.S01E02.x265"));
        assert!(!p.matches("Show.S10E02.x265"));
        assert!(Pattern::parse("  | ").is_none());
        // Pathological glob stays fast.
        let p = Pattern::parse(&"*a".repeat(200)).unwrap();
        assert!(!p.matches(&"a".repeat(150)));
        assert!(glob_chars(&['a', '*', 'c'], &['a', 'b', 'b', 'c']));
    }

    #[test]
    fn episode_filter() {
        let f = EpisodeFilter::parse("1x2-8;2x;3x1-; 4x5").unwrap();
        let ep = |s, e| Release { season: Some(s), episodes: vec![e], ..Release::default() };
        assert!(!f.matches(&ep(1, 1)));
        assert!(f.matches(&ep(1, 2)) && f.matches(&ep(1, 8)));
        assert!(!f.matches(&ep(1, 9)));
        assert!(f.matches(&ep(2, 77)));
        assert!(f.matches(&ep(3, 1000)));
        assert!(f.matches(&ep(4, 5)) && !f.matches(&ep(4, 6)));
        assert!(!f.matches(&ep(5, 1)));
        let pack = |s| Release { season: Some(s), ..Release::default() };
        assert!(f.matches(&pack(2)) && f.matches(&pack(3)) && !f.matches(&pack(1)));
        assert!(!f.matches(&Release::default()));
        for bad in ["", "x1", "1x8-2", "1y2", "ax1", "1x2-b"] { assert!(EpisodeFilter::parse(bad).is_none(), "{}", bad); }
    }

    #[test]
    fn release_parsing() {
        let r = parse_release("Show.Name.S01E02.1080p.WEB-DL.DDP5.1.H.264-GRP");
        assert_eq!((r.show.as_str(), r.season, r.episodes.clone()), ("show name", Some(1), vec![2]));
        assert_eq!((r.resolution, r.source, r.codec, r.proper), (Some(1080), Source::WebDl, Codec::H264, 0));
        let r = parse_release("Show Name S01E01E02 720p HDTV x264");
        assert_eq!(r.episodes, vec![1, 2]);
        assert_eq!(r.source, Source::Tv);
        let r = parse_release("Show.Name.S03E01-E03.2160p.BluRay.REMUX.HEVC");
        assert_eq!(r.episodes, vec![1, 2, 3]);
        assert_eq!((r.resolution, r.source, r.codec), (Some(2160), Source::Remux, Codec::H265));
        let r = parse_release("Show Name 1x02 PROPER WEBRip");
        assert_eq!((r.season, r.episodes.clone(), r.proper, r.source), (Some(1), vec![2], 1, Source::WebRip));
        let r = parse_release("Show Name - Season 1 Episode 2 - Title");
        assert_eq!((r.show.as_str(), r.season, r.episodes.clone()), ("show name", Some(1), vec![2]));
        let r = parse_release("Show.Name.S02.COMPLETE.1080p.BluRay.x265");
        assert_eq!((r.season, r.episodes.len()), (Some(2), 0));
        assert_eq!(r.key(), Some(EpisodeKey::Season(2)));
        let r = parse_release("The.Daily.Show.2026.11.14.Guest.720p.WEB.h264");
        assert_eq!((r.show.as_str(), r.date), ("the daily show", Some((2026, 11, 14))));
        let r = parse_release("[SubsPlease] Sousou no Frieren - 05 (1080p) [A1B2C3D4].mkv");
        assert_eq!((r.show.as_str(), r.absolute, r.resolution), ("sousou no frieren", Some(5), Some(1080)));
        let r = parse_release("[Group] One Piece - 1100v2 [1080p]");
        assert_eq!((r.absolute, r.proper), (Some(1100), 1));
        let r = parse_release("Movie Title 2026 1920x1080 BluRay");
        assert_eq!((r.season, r.resolution, r.key()), (None, Some(1080), None));
        let r = parse_release("Show S01E05 REPACK 1080p");
        assert_eq!(r.label(), "S01E05 1080p PROPER");
        // Hostile input does not panic.
        for t in ["", "s", "sxe", "S999999E999999", "- - -", " - 9999999999", "[", "]", "s01e", "1x", "ü - 05v", "season", "2026.13.40"] { let _ = parse_release(t); }
        assert!(parse_release("Show S01E02").quality_rank() < parse_release("Show S01E02 1080p WEB-DL").quality_rank());
    }

    #[test]
    fn smart_filter_takes_each_episode_once() {
        let mut rule = Rule::new("Show", "show name");
        rule.smart_filter = true;
        rule.category = Some(String::from("tv"));
        rule.tags = vec![String::from("auto")];
        rule.save_path = Some(String::from("/media/tv/Show Name"));
        let mut eng = Engine::new(vec![rule]);
        let batch = vec![
            cand("Show.Name.S01E01.720p.HDTV.x264"),
            cand("Show.Name.S01E01.1080p.WEB-DL.x264"),
            cand("Other.Show.S01E01.1080p"),
            cand("Show.Name.S01E02.720p.HDTV.x264"),
        ];
        let d = eng.evaluate(&batch, 1000);
        assert_eq!(d.iter().map(|d| d.index).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(d[0].category.as_deref(), Some("tv"));
        assert_eq!(d[0].episode, Some(EpisodeKey::Episode { season: 1, episode: 1 }));
        assert!(d[0].reason.contains("new episode S01E01 1080p WEB-DL"), "{}", d[0].reason);
        // Same episodes again: nothing.
        assert!(eng.evaluate(&batch, 2000).is_empty());
        // A REPACK of the same quality is taken; a lower-quality PROPER is not.
        let d = eng.evaluate(&[cand("Show.Name.S01E01.REPACK.1080p.WEB-DL.x264"), cand("Show.Name.S01E02.PROPER.480p")], 3000);
        assert_eq!(d.len(), 1);
        assert!(d[0].reason.contains("PROPER/REPACK"));
        // Quality upgrades only when asked.
        assert!(eng.evaluate(&[cand("Show.Name.S01E02.2160p.BluRay")], 4000).is_empty());
        eng.rules[0].upgrade_quality = true;
        assert_eq!(eng.evaluate(&[cand("Show.Name.S01E02.2160p.BluRay")], 5000).len(), 1);
        assert_eq!(eng.state("Show").unwrap().episodes_len(), 2);
        eng.reset("Show");
        assert_eq!(eng.evaluate(&[cand("Show.Name.S01E02.720p")], 6000).len(), 1);
    }

    #[test]
    fn must_not_filter_resolution_and_feeds() {
        let mut rule = Rule::new("Frieren", "frieren");
        rule.must_not_contain = Pattern::parse("batch | 480p");
        rule.min_resolution = Some(720);
        rule.feeds = vec![String::from("https://nyaa.si/?page=rss")];
        rule.add_paused = Some(true);
        let mut eng = Engine::new(vec![rule]);
        let mk = |feed: &str, t: &str| Candidate { feed: String::from(feed), title: String::from(t), url: String::from("u") };
        let d = eng.evaluate(&[
            mk("https://nyaa.si/?page=rss", "[SubsPlease] Frieren - 06 (1080p)"),
            mk("https://nyaa.si/?page=rss", "[SubsPlease] Frieren - 06 (480p)"),
            mk("https://nyaa.si/?page=rss", "[SubsPlease] Frieren (01-28) (1080p) [Batch]"),
            mk("https://other/rss", "[SubsPlease] Frieren - 07 (1080p)"),
            mk("https://nyaa.si/?page=rss", "[SubsPlease] Frieren - 08"),
        ], 0);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].index, 0);
        assert_eq!(d[0].paused, Some(true));
    }

    #[test]
    fn ignore_days_and_rule_order() {
        let mut a = Rule::new("A", "movie");
        a.ignore_days = 2;
        let b = Rule::new("B", "movie 2160p");
        let mut eng = Engine::new(vec![a, b]);
        let d = eng.evaluate(&[cand("Movie 2026 1080p"), cand("Movie 2026 2160p")], 0);
        // A takes the first only (ignore-days), B takes the rest.
        assert_eq!(d.iter().map(|d| (d.index, d.rule.as_str())).collect::<Vec<_>>(), vec![(0, "A"), (1, "B")]);
        // Within 2 days A ignores matches.
        let d = eng.evaluate(&[cand("Movie Two 720p")], 86_400);
        assert!(d.is_empty());
        let d = eng.evaluate(&[cand("Movie Two 720p")], 3 * 86_400);
        assert_eq!(d.len(), 1);
        eng.rules[0].enabled = false;
        assert!(eng.evaluate(&[cand("Movie Three")], 10 * 86_400).is_empty());
    }

    #[test]
    fn candidate_from_feed_item() {
        let it = FeedItem { title: String::from("x"), link: Some(String::from("https://a/t.torrent")), ..FeedItem::default() };
        assert_eq!(Candidate::from_item("f", &it).unwrap().url, "https://a/t.torrent");
        assert!(Candidate::from_item("f", &FeedItem::default()).is_none());
    }
}
