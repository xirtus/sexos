// SPDX-License-Identifier: GPL-3.0-only
//! The client side of Torznab (and Newznab, which it extends): asking indexers (Jackett,
//! Prowlarr, or an indexer's own API) for torrents, and merging what many of them say.
//!
//! - [`SearchQuery::url`] builds the request URL (`t=search|tvsearch|movie|music|book`,
//!   `q`, `season`, `ep`, `imdbid`, `tvdbid`, `tmdbid`, `cat`, `limit`, `offset`,
//!   `apikey`, and `extended=1` so indexers send every attribute). [`caps_url`] builds the
//!   `t=caps` request; [`parse_caps`] reads which search modes and parameters an indexer
//!   supports and its categories, and [`SearchQuery::adapt`] rewrites a query an indexer
//!   cannot run (no tv-search: the season and episode go into the text as `S01E02`).
//! - [`parse_results`] reads the RSS answer, including `torznab:attr` seeders, peers,
//!   infohash, magneturl, size, category and download/upload volume factors (freeleech),
//!   and turns `<error code description>` into [`TorznabError::Api`].
//! - [`merge_and_rank`] merges result lists from many indexers: duplicates (same
//!   info-hash, same magnet `btih`, or failing those the same title and size) become one
//!   result that remembers every indexer it came from and the best seeder count; then
//!   results are ranked by title similarity to the query, season/episode agreement,
//!   seeders, size sanity and freeleech.
//!
//! The same API serves Qupid's indexer search. No I/O: the program fetches the URLs.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::rss::{self, xml};
use crate::rules::parse_release;
use crate::InfoHashV1;

/// Which Torznab function to call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SearchMode { #[default] Search, TvSearch, Movie, Music, Book }

impl SearchMode {
    /// The `t=` value.
    pub fn as_str(self) -> &'static str {
        match self { SearchMode::Search => "search", SearchMode::TvSearch => "tvsearch", SearchMode::Movie => "movie", SearchMode::Music => "music", SearchMode::Book => "book" }
    }
    fn caps_name(self) -> &'static str {
        match self { SearchMode::Search => "search", SearchMode::TvSearch => "tv-search", SearchMode::Movie => "movie-search", SearchMode::Music => "music-search", SearchMode::Book => "book-search" }
    }
}

/// A search to send to indexers.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SearchQuery {
    pub mode: SearchMode,
    /// Free text.
    pub q: Option<String>,
    pub season: Option<u32>,
    pub ep: Option<u32>,
    /// IMDb id, with or without the `tt` prefix.
    pub imdbid: Option<String>,
    pub tvdbid: Option<u32>,
    pub tmdbid: Option<u32>,
    /// Newznab category ids (2000 movies, 5000 TV, ...).
    pub cats: Vec<u32>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// The release the person watched last (the previous episode's file name): results
    /// from the same group, resolution and codec rank higher (`sex_episodes::similarity`).
    /// Not sent to indexers.
    pub like: Option<String>,
}

/// Percent-encodes a query value (RFC 3986 unreserved characters kept).
pub fn url_encode(s: &str, out: &mut String) {
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { out.push(b as char); }
        else { out.push_str(&format!("%{:02X}", b)); }
    }
}

fn push_param(url: &mut String, k: &str, v: &str) {
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(k);
    url.push('=');
    url_encode(v, url);
}

/// The URL for `t=caps`.
pub fn caps_url(base: &str, apikey: &str) -> String {
    let mut u = String::from(base);
    push_param(&mut u, "t", "caps");
    if !apikey.is_empty() { push_param(&mut u, "apikey", apikey); }
    u
}

impl SearchQuery {
    /// A free-text search.
    pub fn text(q: &str) -> SearchQuery { SearchQuery { q: Some(String::from(q)), ..SearchQuery::default() } }
    /// A TV episode search.
    pub fn tv(q: &str, season: Option<u32>, ep: Option<u32>) -> SearchQuery {
        SearchQuery { mode: SearchMode::TvSearch, q: Some(String::from(q)), season, ep, ..SearchQuery::default() }
    }
    /// The request URL for an indexer's API endpoint (`.../api`), with its key.
    pub fn url(&self, base: &str, apikey: &str) -> String {
        let mut u = String::from(base);
        push_param(&mut u, "t", self.mode.as_str());
        if !apikey.is_empty() { push_param(&mut u, "apikey", apikey); }
        if let Some(q) = self.q.as_deref().filter(|q| !q.trim().is_empty()) { push_param(&mut u, "q", q.trim()); }
        if let Some(s) = self.season { push_param(&mut u, "season", &format!("{}", s)); }
        if let Some(e) = self.ep { push_param(&mut u, "ep", &format!("{}", e)); }
        if let Some(i) = &self.imdbid {
            let digits: String = i.trim_start_matches("tt").chars().filter(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() { push_param(&mut u, "imdbid", &digits); }
        }
        if let Some(t) = self.tvdbid { push_param(&mut u, "tvdbid", &format!("{}", t)); }
        if let Some(t) = self.tmdbid { push_param(&mut u, "tmdbid", &format!("{}", t)); }
        if !self.cats.is_empty() {
            let c: Vec<String> = self.cats.iter().map(|c| format!("{}", c)).collect();
            push_param(&mut u, "cat", &c.join(","));
        }
        if let Some(l) = self.limit { push_param(&mut u, "limit", &format!("{}", l)); }
        if let Some(o) = self.offset { push_param(&mut u, "offset", &format!("{}", o)); }
        push_param(&mut u, "extended", "1");
        u
    }
    /// Rewrites the query for an indexer's capabilities: unsupported modes fall back to
    /// `search`, unsupported ids are dropped, and season/episode move into the text.
    pub fn adapt(&self, caps: &Caps) -> SearchQuery {
        let mut q = self.clone();
        if !caps.available(q.mode) { q.mode = SearchMode::Search; }
        let ok = |p: &str| caps.supports(q.mode, p);
        if q.imdbid.is_some() && !ok("imdbid") { q.imdbid = None; }
        if q.tvdbid.is_some() && !ok("tvdbid") { q.tvdbid = None; }
        if q.tmdbid.is_some() && !ok("tmdbid") { q.tmdbid = None; }
        if (q.season.is_some() && !ok("season")) || (q.ep.is_some() && !ok("ep")) {
            let mut text = q.q.take().unwrap_or_default();
            match (q.season, q.ep) {
                (Some(s), Some(e)) => text.push_str(&format!(" S{:02}E{:02}", s, e)),
                (Some(s), None) => text.push_str(&format!(" S{:02}", s)),
                _ => {}
            }
            q.q = Some(String::from(text.trim()));
            q.season = None;
            q.ep = None;
        }
        if let Some(max) = caps.limit_max { q.limit = q.limit.map(|l| l.min(max)); }
        q
    }
}

/// One search function an indexer offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCap { pub mode: SearchMode, pub available: bool, pub params: Vec<String> }

/// An indexer category and its subcategories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category { pub id: u32, pub name: String, pub subcats: Vec<Category> }

/// What `t=caps` says.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Caps {
    pub server_title: Option<String>,
    pub limit_max: Option<u32>,
    pub limit_default: Option<u32>,
    pub searching: Vec<SearchCap>,
    pub categories: Vec<Category>,
}

impl Caps {
    /// Whether a mode is available (`search` is assumed when caps do not say).
    pub fn available(&self, m: SearchMode) -> bool {
        match self.searching.iter().find(|s| s.mode == m) { Some(s) => s.available, None => m == SearchMode::Search }
    }
    /// Whether a mode accepts a parameter.
    pub fn supports(&self, m: SearchMode, param: &str) -> bool {
        self.searching.iter().any(|s| s.mode == m && s.available && s.params.iter().any(|p| p.eq_ignore_ascii_case(param)))
    }
}

/// Why a Torznab answer could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorznabError {
    /// Too large or deep to read.
    Xml(xml::Error),
    /// The indexer answered `<error code=".." description=".."/>` (100 bad key, ...).
    Api { code: u32, description: String },
    /// Not a caps document or not an RSS feed.
    Malformed,
}

fn api_error(root: &xml::Element) -> Option<TorznabError> {
    let e = root.child("error")?;
    Some(TorznabError::Api {
        code: e.attr("code").and_then(|c| c.trim().parse().ok()).unwrap_or(0),
        description: String::from(e.attr("description").unwrap_or("")),
    })
}

fn parse_category(e: &xml::Element, depth: usize) -> Option<Category> {
    let id = e.attr("id")?.trim().parse().ok()?;
    let subcats = if depth < 2 { e.children_named("subcat").take(256).filter_map(|s| parse_category(s, depth + 1)).collect() } else { Vec::new() };
    Some(Category { id, name: String::from(e.attr("name").unwrap_or("")), subcats })
}

/// Parses a `t=caps` answer.
pub fn parse_caps(body: &[u8]) -> Result<Caps, TorznabError> {
    let root = xml::parse(body).map_err(TorznabError::Xml)?;
    if let Some(e) = api_error(&root) { return Err(e); }
    let caps = root.child("caps").ok_or(TorznabError::Malformed)?;
    let mut out = Caps::default();
    if let Some(s) = caps.child("server") { out.server_title = s.attr("title").map(String::from); }
    if let Some(l) = caps.child("limits") {
        out.limit_max = l.attr("max").and_then(|v| v.trim().parse().ok());
        out.limit_default = l.attr("default").and_then(|v| v.trim().parse().ok());
    }
    if let Some(s) = caps.child("searching") {
        for m in [SearchMode::Search, SearchMode::TvSearch, SearchMode::Movie, SearchMode::Music, SearchMode::Book] {
            if let Some(e) = s.child(m.caps_name()) {
                out.searching.push(SearchCap {
                    mode: m,
                    available: e.attr("available").is_some_and(|a| a.trim().eq_ignore_ascii_case("yes")),
                    params: e.attr("supportedParams").unwrap_or("").split(',').map(|p| String::from(p.trim())).filter(|p| !p.is_empty()).take(64).collect(),
                });
            }
        }
    }
    if let Some(c) = caps.child("categories") {
        out.categories = c.children_named("category").take(1024).filter_map(|e| parse_category(e, 0)).collect();
    }
    Ok(out)
}

/// One torrent an indexer returned.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct IndexerResult {
    /// The indexer that returned it (the first, after merging).
    pub indexer: String,
    pub title: String,
    pub guid: Option<String>,
    /// The .torrent download URL.
    pub link: Option<String>,
    /// The details page.
    pub details: Option<String>,
    pub magnet: Option<String>,
    pub info_hash: Option<InfoHashV1>,
    pub size: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub grabs: Option<u32>,
    pub categories: Vec<u32>,
    /// Unix seconds.
    pub pub_date: Option<i64>,
    /// 0.0 = freeleech, 1.0 = normal.
    pub download_factor: Option<f32>,
    pub upload_factor: Option<f32>,
    /// Other indexers that returned the same torrent (after merging).
    pub also_on: Vec<String>,
    /// The ranking score [`merge_and_rank`] gave it.
    pub score: i32,
}

impl IndexerResult {
    /// What to hand the engine: the magnet when there is one, else the .torrent URL.
    pub fn torrent_url(&self) -> Option<&str> { self.magnet.as_deref().or(self.link.as_deref()) }
}

/// Parses a search answer from one indexer.
pub fn parse_results(indexer: &str, body: &[u8]) -> Result<Vec<IndexerResult>, TorznabError> {
    let root = xml::parse(body).map_err(TorznabError::Xml)?;
    if let Some(e) = api_error(&root) { return Err(e); }
    let feed = rss::parse_feed(body).map_err(|e| match e { rss::FeedError::Xml(x) => TorznabError::Xml(x), rss::FeedError::NotAFeed => TorznabError::Malformed })?;
    Ok(feed.items.into_iter().map(|it| {
        let mut cats: Vec<u32> = it.attrs.iter().filter(|(k, _)| k == "category").filter_map(|(_, v)| v.trim().parse().ok()).collect();
        for c in &it.categories { if let Ok(n) = c.trim().parse::<u32>() { if !cats.contains(&n) { cats.push(n); } } }
        let f = |n: &str| it.attr(n).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
        let link = it.enclosure.as_ref().map(|e| e.url.clone()).filter(|u| !u.starts_with("magnet:")).or_else(|| it.link.clone().filter(|u| !u.starts_with("magnet:")));
        IndexerResult {
            indexer: String::from(indexer),
            title: it.title.clone(),
            guid: it.guid.clone(),
            link,
            details: it.comments.clone().or_else(|| it.guid.clone().filter(|g| g.starts_with("http"))),
            magnet: it.magnet.clone(),
            info_hash: it.info_hash,
            size: it.size,
            seeders: it.seeders,
            leechers: it.leechers,
            grabs: it.attr("grabs").and_then(|v| v.trim().parse().ok()),
            categories: cats,
            pub_date: it.pub_date,
            download_factor: f("downloadvolumefactor"),
            upload_factor: f("uploadvolumefactor"),
            also_on: Vec::new(),
            score: 0,
        }
    }).collect())
}

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).take(64).map(String::from).collect()
}

/// How much of the query's words appear in the title, 0..=100.
pub fn title_similarity(query: &str, title: &str) -> u32 {
    let q = tokens(query);
    if q.is_empty() { return 100; }
    let t = tokens(title);
    let hit = q.iter().filter(|w| t.contains(w)).count() as u32;
    let mut score = hit * 100 / q.len() as u32;
    // A small penalty when the title has many extra words before the first query word
    // (a different show whose name merely contains the query).
    if let Some(first) = t.iter().position(|w| *w == q[0]) { score = score.saturating_sub((first as u32).min(5) * 4); }
    score
}

fn dedupe_key(r: &IndexerResult) -> String {
    if let Some(h) = r.info_hash.or_else(|| r.magnet.as_deref().and_then(rss::btih_from_magnet)) {
        return crate::to_hex(&h.0);
    }
    let t = tokens(&r.title).join(" ");
    match r.size { Some(s) => format!("t:{}:{}", t, s >> 20), None => format!("t:{}", t) }
}

fn score(r: &IndexerResult, q: &SearchQuery) -> i32 {
    let mut s = title_similarity(q.q.as_deref().unwrap_or(""), &r.title) as i32 * 4;
    if q.season.is_some() || q.ep.is_some() {
        let rel = parse_release(&r.title);
        let season_ok = q.season.map_or(true, |w| rel.season == Some(w));
        let ep_ok = match q.ep { None => true, Some(e) => rel.episodes.contains(&e) || (rel.season.is_some() && rel.episodes.is_empty()) };
        s += if season_ok && ep_ok { if rel.episodes.is_empty() && q.ep.is_some() { 50 } else { 150 } } else { -300 };
    }
    if let Some(prev) = q.like.as_deref() { s += sex_episodes::episodes::similarity(prev, &r.title).clamp(-100, 150); }
    s += match r.seeders {
        Some(0) => -200,
        Some(n) => ((32 - (n.min(1 << 20) + 1).leading_zeros()) as i32 * 30).min(300),
        None => 0,
    };
    let video = matches!(q.mode, SearchMode::TvSearch | SearchMode::Movie);
    s += match r.size {
        None | Some(0) => -50,
        Some(n) if n < 1 << 20 => -300,
        Some(n) if video && n < 50 << 20 => -150,
        Some(n) if n > 200 << 30 => -100,
        Some(_) => 0,
    };
    if r.download_factor == Some(0.0) { s += 25; }
    s
}

/// Merges result lists from several indexers and ranks them, best first. Results that
/// name the same torrent become one, keeping the highest seeder/leecher counts, any
/// missing magnet/hash/size/link from the others, and the other indexers in `also_on`.
pub fn merge_and_rank(lists: Vec<Vec<IndexerResult>>, query: &SearchQuery) -> Vec<IndexerResult> {
    let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: Vec<IndexerResult> = Vec::new();
    for r in lists.into_iter().flatten() {
        let k = dedupe_key(&r);
        match by_key.get(&k) {
            None => { by_key.insert(k, out.len()); out.push(r); }
            Some(&i) => {
                let m = &mut out[i];
                if r.seeders.unwrap_or(0) > m.seeders.unwrap_or(0) {
                    // Prefer the better-seeded copy as the face of the result.
                    let mut r = r;
                    core::mem::swap(m, &mut r);
                    m.also_on.append(&mut r.also_on);
                    absorb(m, r);
                } else {
                    absorb(m, r);
                }
            }
        }
    }
    for r in &mut out { r.score = score(r, query); }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(b.seeders.cmp(&a.seeders)).then_with(|| a.title.cmp(&b.title)));
    out
}

fn absorb(m: &mut IndexerResult, r: IndexerResult) {
    if r.indexer != m.indexer && !m.also_on.contains(&r.indexer) { m.also_on.push(r.indexer); }
    m.seeders = m.seeders.max(r.seeders);
    m.leechers = m.leechers.max(r.leechers);
    m.grabs = m.grabs.max(r.grabs);
    if m.magnet.is_none() { m.magnet = r.magnet; }
    if m.info_hash.is_none() { m.info_hash = r.info_hash; }
    if m.size.is_none() { m.size = r.size; }
    if m.link.is_none() { m.link = r.link; }
    if m.pub_date.is_none() { m.pub_date = r.pub_date; }
    m.download_factor = match (m.download_factor, r.download_factor) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
    for c in r.categories { if !m.categories.contains(&c) { m.categories.push(c); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const CAPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <server version="1.0" title="Jackett" />
  <limits max="100" default="50" />
  <searching>
    <search available="yes" supportedParams="q" />
    <tv-search available="yes" supportedParams="q,season,ep,imdbid,tvdbid" />
    <movie-search available="yes" supportedParams="q,imdbid" />
    <music-search available="no" supportedParams="q" />
    <book-search available="no" supportedParams="q" />
  </searching>
  <categories>
    <category id="2000" name="Movies"><subcat id="2040" name="Movies/HD" /><subcat id="2045" name="Movies/UHD" /></category>
    <category id="5000" name="TV"><subcat id="5040" name="TV/HD" /></category>
  </categories>
</caps>"#;

    const RESULTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:torznab="http://torznab.com/schemas/2015/feed">
  <channel>
    <atom:link href="http://127.0.0.1:9117/" rel="self" type="application/rss+xml" />
    <title>AggregateSearch</title>
    <item>
      <title>Show.Name.S01E02.1080p.WEB-DL.x264-GRP</title>
      <guid>https://tracker.example/details/111</guid>
      <jackettindexer id="ex">Example</jackettindexer>
      <comments>https://tracker.example/details/111</comments>
      <pubDate>Fri, 13 Nov 2026 20:00:00 +0000</pubDate>
      <size>1610612736</size>
      <link>http://127.0.0.1:9117/dl/ex/?jackett_apikey=k&amp;path=abc&amp;file=Show</link>
      <category>5040</category>
      <enclosure url="http://127.0.0.1:9117/dl/ex/?jackett_apikey=k&amp;path=abc&amp;file=Show" length="1610612736" type="application/x-bittorrent" />
      <torznab:attr name="category" value="5000" />
      <torznab:attr name="category" value="5040" />
      <torznab:attr name="seeders" value="120" />
      <torznab:attr name="peers" value="130" />
      <torznab:attr name="grabs" value="900" />
      <torznab:attr name="infohash" value="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" />
      <torznab:attr name="downloadvolumefactor" value="0" />
      <torznab:attr name="uploadvolumefactor" value="1" />
    </item>
    <item>
      <title>Show Name S01E02 720p HDTV</title>
      <guid>https://other.example/t/9</guid>
      <size>400000000</size>
      <torznab:attr name="seeders" value="3" />
      <torznab:attr name="peers" value="4" />
      <torznab:attr name="magneturl" value="magnet:?xt=urn:btih:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB&amp;dn=x" />
    </item>
  </channel>
</rss>"#;

    #[test]
    fn urls() {
        let q = SearchQuery { mode: SearchMode::TvSearch, q: Some(String::from("Show & Name")), season: Some(1), ep: Some(2),
            imdbid: Some(String::from("tt0944947")), tvdbid: Some(121361), cats: vec![5000, 5040], limit: Some(50), offset: Some(100), ..SearchQuery::default() };
        assert_eq!(q.url("http://127.0.0.1:9117/api/v2.0/indexers/all/results/torznab/api", "k3y"),
            "http://127.0.0.1:9117/api/v2.0/indexers/all/results/torznab/api?t=tvsearch&apikey=k3y&q=Show%20%26%20Name&season=1&ep=2&imdbid=0944947&tvdbid=121361&cat=5000%2C5040&limit=50&offset=100&extended=1");
        assert_eq!(SearchQuery::text("x").url("https://i/api?x=1", ""), "https://i/api?x=1&t=search&q=x&extended=1");
        assert_eq!(caps_url("https://i/api", "k"), "https://i/api?t=caps&apikey=k");
    }

    #[test]
    fn caps() {
        let c = parse_caps(CAPS.as_bytes()).unwrap();
        assert_eq!(c.server_title.as_deref(), Some("Jackett"));
        assert_eq!((c.limit_max, c.limit_default), (Some(100), Some(50)));
        assert!(c.available(SearchMode::TvSearch) && !c.available(SearchMode::Music));
        assert!(c.supports(SearchMode::TvSearch, "season") && !c.supports(SearchMode::Movie, "season"));
        assert_eq!(c.categories.len(), 2);
        assert_eq!(c.categories[0].subcats[1].id, 2045);
        // A movie query with season info: season moves into the text, limit clamps.
        let q = SearchQuery { mode: SearchMode::Movie, q: Some(String::from("Show")), season: Some(3), ep: Some(4), tvdbid: Some(1), limit: Some(500), ..SearchQuery::default() };
        let a = q.adapt(&c);
        assert_eq!((a.mode, a.q.as_deref(), a.season, a.tvdbid, a.limit), (SearchMode::Movie, Some("Show S03E04"), None, None, Some(100)));
        let a = SearchQuery { mode: SearchMode::Book, ..SearchQuery::default() }.adapt(&c);
        assert_eq!(a.mode, SearchMode::Search);
    }

    #[test]
    fn errors() {
        assert_eq!(parse_results("x", br#"<?xml version="1.0"?><error code="100" description="Incorrect user credentials"/>"#),
            Err(TorznabError::Api { code: 100, description: String::from("Incorrect user credentials") }));
        assert_eq!(parse_caps(b"<error code='201' description='Incorrect parameter'/>").unwrap_err(),
            TorznabError::Api { code: 201, description: String::from("Incorrect parameter") });
        assert_eq!(parse_caps(b"<html/>"), Err(TorznabError::Malformed));
        assert_eq!(parse_results("x", b"not xml"), Err(TorznabError::Malformed));
        let _ = parse_results("x", &[0xff; 100]);
    }

    #[test]
    fn results() {
        let r = parse_results("Jackett", RESULTS.as_bytes()).unwrap();
        assert_eq!(r.len(), 2);
        let a = &r[0];
        assert_eq!(a.size, Some(1610612736));
        assert_eq!((a.seeders, a.leechers, a.grabs), (Some(120), Some(10), Some(900)));
        assert_eq!(a.categories, vec![5000, 5040]);
        assert_eq!(a.download_factor, Some(0.0));
        assert_eq!(a.upload_factor, Some(1.0));
        assert_eq!(a.info_hash.unwrap().0, [0xaa; 20]);
        assert!(a.link.as_deref().unwrap().contains("path=abc&file=Show"));
        assert_eq!(a.details.as_deref(), Some("https://tracker.example/details/111"));
        let b = &r[1];
        assert_eq!(b.info_hash.unwrap().0, [0xbb; 20]);
        assert!(b.torrent_url().unwrap().starts_with("magnet:"));
        assert_eq!(b.leechers, Some(1));
    }

    fn res(ix: &str, title: &str, hash: u8, seeders: u32, size: u64) -> IndexerResult {
        IndexerResult { indexer: String::from(ix), title: String::from(title), info_hash: (hash != 0).then_some(InfoHashV1([hash; 20])),
            seeders: Some(seeders), size: Some(size), ..IndexerResult::default() }
    }

    #[test]
    fn merging_and_ranking() {
        let q = SearchQuery::tv("Show Name", Some(1), Some(2));
        let a = vec![
            res("A", "Show.Name.S01E02.1080p.WEB-DL", 1, 50, 1 << 30),
            res("A", "Show.Name.S01E03.1080p.WEB-DL", 2, 500, 1 << 30),
            res("A", "Totally Different Thing", 3, 9000, 1 << 30),
        ];
        let mut dup = res("B", "Show Name S01E02 1080p WEB-DL", 0, 80, 1 << 30);
        dup.magnet = Some(format!("magnet:?xt=urn:btih:{}", crate::to_hex(&[1; 20])));
        let b = vec![dup,
            res("B", "Show Name S01E02 fake", 4, 1000, 1000),
            res("B", "Show Name S01E02 dead", 5, 0, 1 << 30),
            res("C", "Show Name S01 Complete 1080p", 6, 40, 20 << 30)];
        let out = merge_and_rank(vec![a, b, vec![]], &q);
        assert_eq!(out.len(), 6);
        // The duplicate merged: better-seeded copy is the face, the other indexer listed.
        assert_eq!(out[0].indexer, "B");
        assert_eq!(out[0].also_on, vec![String::from("A")]);
        assert_eq!(out[0].seeders, Some(80));
        assert_eq!(out[0].info_hash, Some(InfoHashV1([1; 20])));
        // The season pack ranks next, then the wrong episode and the unrelated title rank low.
        assert_eq!(out[1].title, "Show Name S01 Complete 1080p");
        let pos = |t: &str| out.iter().position(|r| r.title == t).unwrap();
        assert!(pos("Show Name S01E02 fake") > pos("Show Name S01 Complete 1080p"));
        assert!(pos("Totally Different Thing") > 2);
        assert!(pos("Show.Name.S01E03.1080p.WEB-DL") > 1);
        // Title-only dedupe when no hash: same title and size merge.
        let x = vec![res("A", "Same Title 2026", 0, 1, 5 << 30)];
        let y = vec![res("B", "Same.Title.2026", 0, 2, 5 << 30)];
        assert_eq!(merge_and_rank(vec![x, y], &SearchQuery::text("same title")).len(), 1);
    }

    #[test]
    fn similarity() {
        assert_eq!(title_similarity("show name", "Show.Name.S01E01"), 100);
        assert_eq!(title_similarity("show name", "Show.S01E01"), 50);
        assert!(title_similarity("name", "A Much Longer Different Show Name") < 100);
        assert_eq!(title_similarity("", "x"), 100);
    }

    #[test]
    fn like_prefers_the_same_release_group() {
        let mk = |t: &str| IndexerResult { title: String::from(t), seeders: Some(50), ..IndexerResult::default() };
        let q = SearchQuery { like: Some(String::from("Show.S01E01.1080p.WEB.h264-GRP")), ..SearchQuery::tv("Show", Some(1), Some(2)) };
        let out = merge_and_rank(alloc::vec![alloc::vec![mk("Show.S01E02.720p.HDTV.x265-OTHER"), mk("Show.S01E02.1080p.WEB.h264-GRP")]], &q);
        assert_eq!(out[0].title, "Show.S01E02.1080p.WEB.h264-GRP");
    }

}
