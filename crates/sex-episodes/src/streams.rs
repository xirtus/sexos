// SPDX-License-Identifier: GPL-3.0-only
//! Non-truncated stream telemetry, multi-file season-pack episode matching, viability
//! ranking and tracker-enriched magnet building for Qupid and Swingers (`#![no_std]`).
//!
//! Stream titles from Stremio addons and P2P indexers routinely bury the three numbers
//! that decide whether a stream will actually play well — **seed count** (`👤`),
//! **modern codec** (`HEVC` / `AV1`), and **file size** (`💾`) — at the end of a long
//! release name where single-line UIs clip them off. [`StreamCandidate`] extracts every
//! critical field up front and formats a never-truncated [`StreamCandidate::telemetry_header`]
//! paired with the full multi-line [`StreamCandidate::full_release_label`].

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::episodes;
use super::pack::{json, url_encode_query, PackError};

const ZERO_BTIH: &str = "0000000000000000000000000000000000000000";

/// Parsed, non-truncated P2P or direct HTTP stream candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamCandidate {
    /// Addon or indexer display name.
    pub provider_name: String,
    /// Full raw multi-line stream title/description (never truncated).
    pub title: String,
    /// Explicit episode file name inside a multi-file pack (`behaviorHints.filename`).
    pub behavior_filename: String,
    /// Binge continuity key (`behaviorHints.bingeGroup`).
    pub binge_group: String,
    /// Lowercase 40-hex (or 32-char base32) BitTorrent v1 info-hash, empty for HTTP-only.
    pub info_hash: String,
    /// File index inside a multi-file season pack torrent, when specified.
    pub file_idx: Option<u32>,
    /// Direct HTTP/HLS URL when provided by a mirror or debrid/archive stream.
    pub direct_url: String,
    /// Explicit tracker announce URLs attached to this stream.
    pub trackers: Vec<String>,
    /// Active seeder count (`None` for HTTP streams or feeds that omit peer counts).
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    /// Exact or parsed file size in bytes.
    pub size_bytes: Option<u64>,
    /// Vertical picture lines (`2160`, `1440`, `1080`, `720`, `480`, or `0` unknown).
    pub resolution_p: u32,
    /// Normalized video codec badge: `"AV1"`, `"HEVC"`, or `"H.264"`.
    pub video_codec: Option<&'static str>,
    /// Dynamic range badge: `"DV · HDR"`, `"DV"`, `"HDR10+"`, or `"HDR"`.
    pub hdr_badge: Option<&'static str>,
    /// Upstream indexer extracted from `⚙️ ...` metadata when aggregated.
    pub source_indexer: Option<String>,
}

impl StreamCandidate {
    pub fn is_p2p(&self) -> bool {
        !self.info_hash.is_empty()
    }

    pub fn is_direct_http(&self) -> bool {
        !self.direct_url.is_empty()
    }

    /// False when a P2P torrent explicitly reports `0` seeders and has no HTTP fallback.
    pub fn is_viable(&self) -> bool {
        if self.is_direct_http() {
            return true;
        }
        if !self.is_p2p() {
            return false;
        }
        self.seeders != Some(0)
    }

    /// Complete, un-truncated release filename line for multi-line display and similarity matching.
    pub fn full_release_label(&self) -> &str {
        if !self.behavior_filename.is_empty() {
            return &self.behavior_filename;
        }
        for line in self.title.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("👤") && !trimmed.starts_with("💾") {
                return trimmed;
            }
        }
        self.title.trim()
    }

    /// High-signal telemetry line putting seeders, codec (`AV1`/`HEVC`), resolution,
    /// exact size and source indexer first so none of them can ever be clipped off.
    pub fn telemetry_header(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(6);
        if let Some(seeds) = self.seeders {
            parts.push(format!("👤 {seeds}"));
        } else if self.is_direct_http() && !self.is_p2p() {
            parts.push(String::from("HTTP Direct"));
        }
        if self.resolution_p > 0 {
            if self.resolution_p == 2160 {
                parts.push(String::from("4K 2160p"));
            } else {
                parts.push(format!("{}p", self.resolution_p));
            }
        }
        if let Some(codec) = self.video_codec {
            parts.push(String::from(codec));
        }
        if let Some(hdr) = self.hdr_badge {
            parts.push(String::from(hdr));
        }
        if let Some(bytes) = self.size_bytes.filter(|&b| b > 0) {
            parts.push(format!("💾 {}", format_bytes(bytes)));
        }
        if let Some(ix) = self.source_indexer.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("⚙️ {ix}"));
        } else if !self.provider_name.is_empty() {
            parts.push(self.provider_name.clone());
        }
        parts.join(" · ")
    }

    /// Non-truncated two-part `(telemetry_header, full_release_label)` for Qupid's stream list.
    pub fn display_lines(&self) -> (String, String) {
        (self.telemetry_header(), String::from(self.full_release_label()))
    }

    /// Builds a tracker-enriched `magnet:?xt=urn:btih:...` URI merging `self.trackers`
    /// and `extra_trackers` without duplicates, plus `&so={file_idx}` for season packs.
    pub fn build_magnet_uri(&self, extra_trackers: &[String]) -> Option<String> {
        if self.info_hash.is_empty() {
            return None;
        }
        let mut uri = format!("magnet:?xt=urn:btih:{}", self.info_hash);
        let dn = self.full_release_label();
        if !dn.is_empty() {
            uri.push_str("&dn=");
            uri.push_str(&url_encode_query(dn));
        }
        let mut seen: Vec<&str> = Vec::new();
        for tr in self.trackers.iter().chain(extra_trackers.iter()) {
            let t = tr.trim();
            if t.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(t)) {
                continue;
            }
            seen.push(t);
            uri.push_str("&tr=");
            uri.push_str(&url_encode_query(t));
        }
        if let Some(idx) = self.file_idx {
            uri.push_str(&format!("&so={idx}"));
        }
        Some(uri)
    }

    /// True when this stream matches `(season, episode)` either as an individual episode
    /// file or as a multi-file season/series pack covering `season`.
    pub fn matches_episode(&self, season: u32, episode: u32) -> bool {
        if !self.behavior_filename.is_empty() {
            if let Some(ep) = episodes::parse(&self.behavior_filename) {
                return ep.season == season && ep.episode == episode;
            }
        }
        let label = self.full_release_label();
        if let Some(ep) = episodes::parse(label) {
            return ep.season == season && ep.episode == episode;
        }
        // Check other lines in multi-line Stremio pack descriptions (e.g. pack header + episode line).
        for line in self.title.lines() {
            if let Some(ep) = episodes::parse(line) {
                return ep.season == season && ep.episode == episode;
            }
        }
        // Multi-file season or complete-series pack without an explicit episode number in the title.
        is_season_pack_for(label, season) || is_season_pack_for(&self.title, season)
    }
}

/// Ranking preferences for [`rank_streams`].
#[derive(Clone, Debug)]
pub struct RankOptions<'a> {
    /// Drop P2P torrents known to have `0` seeders.
    pub drop_zero_seed_torrents: bool,
    /// Prefer `AV1` and `HEVC` encodes over legacy `H.264` at comparable health.
    pub prefer_modern_codecs: bool,
    /// Optional soft ceiling in bytes; larger files receive a steep penalty so oversized
    /// Blu-ray remuxes sort below streamable `HEVC`/`AV1` encodes.
    pub max_preferred_size_bytes: Option<u64>,
    /// Previous episode's filename for binge release-group/codec continuity.
    pub previous_release: Option<&'a str>,
}

impl<'a> Default for RankOptions<'a> {
    fn default() -> Self {
        Self {
            drop_zero_seed_torrents: true,
            prefer_modern_codecs: true,
            max_preferred_size_bytes: None,
            previous_release: None,
        }
    }
}

/// Filters and sorts stream candidates best-first by viability, binge similarity,
/// modern codec (`AV1`/`HEVC`), resolution, size sanity and swarm seeders.
pub fn rank_streams(candidates: &[StreamCandidate], opts: &RankOptions<'_>) -> Vec<StreamCandidate> {
    let mut scored: Vec<(i64, StreamCandidate)> = Vec::with_capacity(candidates.len());
    for c in candidates {
        if opts.drop_zero_seed_torrents && !c.is_viable() {
            continue;
        }
        let score = score_stream(c, opts);
        scored.push((score, c.clone()));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, c)| c).collect()
}

fn score_stream(c: &StreamCandidate, opts: &RankOptions<'_>) -> i64 {
    let mut score = 0i64;

    // 1. Seeders: viability is paramount.
    match c.seeders {
        Some(0) => score -= 5_000,
        Some(s) => {
            // Tiered + bounded linear bonus so healthy swarms dominate dead ones without
            // letting a 480p H.264 rip with 2000 seeds crush a 1080p HEVC encode with 150 seeds.
            let tiered = if s >= 100 {
                260 + (s.min(1_500) as i64) / 12
            } else if s >= 25 {
                190 + (s as i64)
            } else if s >= 5 {
                95 + (s as i64) * 3
            } else {
                25 + (s as i64) * 12
            };
            score += tiered;
        }
        None if c.is_direct_http() => score += 220,
        None => score += 40,
    }

    // 2. Modern codec preference (`AV1` / `HEVC`).
    if opts.prefer_modern_codecs {
        match c.video_codec {
            Some("AV1") => score += 65,
            Some("HEVC") => score += 55,
            Some("H.264") => score += 10,
            _ => {}
        }
    }

    // 3. Resolution quality.
    score += match c.resolution_p {
        2160 => 55,
        1440 => 45,
        1080 => 48,
        720 => 22,
        480 => 5,
        _ => 12,
    };

    // 4. File size streaming sanity cap.
    if let (Some(limit), Some(sz)) = (opts.max_preferred_size_bytes, c.size_bytes) {
        if limit > 0 && sz > limit {
            let over_ratio = ((sz - limit) * 100 / limit).min(500) as i64;
            score -= 220 + over_ratio * 2;
        }
    }

    // 5. Binge release continuity (`sex_episodes::episodes::similarity`).
    if let Some(prev) = opts.previous_release {
        score += (episodes::similarity(prev, c.full_release_label()) as i64) * 4;
    }

    score
}

/// Formats a byte count cleanly without truncation (`842 MB`, `1.85 GB`, `12.4 GB`).
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    const TB: u64 = 1024 * 1024 * 1024 * 1024;

    if bytes >= TB {
        let tenths = (bytes * 100 + TB / 2) / TB;
        format!("{}.{:02} TB", tenths / 100, tenths % 100)
    } else if bytes >= GB {
        let hundredths = (bytes * 100 + GB / 2) / GB;
        if hundredths >= 1000 {
            let tenths = (bytes * 10 + GB / 2) / GB;
            format!("{}.{} GB", tenths / 10, tenths % 10)
        } else {
            format!("{}.{:02} GB", hundredths / 100, hundredths % 100)
        }
    } else if bytes >= MB {
        format!("{} MB", (bytes + MB / 2) / MB)
    } else if bytes >= KB {
        format!("{} KB", (bytes + KB / 2) / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Parses a Stremio v3 `/stream/{type}/{id}.json` response body.
pub fn parse_stremio_streams_json(provider_name: &str, raw_json: &str) -> Result<Vec<StreamCandidate>, PackError> {
    let root = json::parse(raw_json)?;
    let Some(obj) = root.as_obj() else {
        return Err(PackError::ExpectedObject);
    };
    let Some(arr) = json::get_arr(obj, "streams") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(s) = item.as_obj() else { continue };
        let name = json::get_str(s, "name").unwrap_or("").trim();
        let title = json::get_str(s, "title")
            .or_else(|| json::get_str(s, "description"))
            .unwrap_or("")
            .trim();
        let direct_url = json::get_str(s, "url").unwrap_or("").trim().to_owned();
        let mut info_hash = json::get_str(s, "infoHash")
            .and_then(normalize_btih)
            .unwrap_or_default();
        let file_idx = json::get_u64(s, "fileIdx").map(|v| v as u32);

        let hints = json::get_obj(s, "behaviorHints");
        let behavior_filename = hints
            .and_then(|h| json::get_str(h, "filename"))
            .unwrap_or("")
            .trim()
            .to_owned();
        let binge_group = hints
            .and_then(|h| json::get_str(h, "bingeGroup"))
            .unwrap_or("")
            .trim()
            .to_owned();
        let hint_size = hints.and_then(|h| json::get_u64(h, "videoSize")).filter(|&b| b > 0);

        let mut trackers = Vec::new();
        for src in json::get_str_vec(s, "sources") {
            if let Some(tr) = src.strip_prefix("tracker:") {
                if !tr.trim().is_empty() {
                    trackers.push(String::from(tr.trim()));
                }
            } else if src.starts_with("udp://") || src.starts_with("http://") || src.starts_with("https://") {
                trackers.push(src);
            } else if info_hash.is_empty() {
                if let Some(h) = extract_btih_from_magnet(&src) {
                    info_hash = h;
                }
            }
        }
        if info_hash.is_empty() && direct_url.starts_with("magnet:") {
            if let Some(h) = extract_btih_from_magnet(&direct_url) {
                info_hash = h;
            }
            extract_trackers_from_magnet(&direct_url, &mut trackers);
        }
        let clean_direct = if direct_url.starts_with("magnet:") { String::new() } else { direct_url };
        if info_hash.is_empty() && clean_direct.is_empty() {
            continue;
        }

        let combined = format!("{name}\n{title}\n{behavior_filename}");
        let meta = inspect_text_telemetry(&combined);
        let badge_provider = if !name.is_empty() {
            let first = name.lines().next().unwrap_or(provider_name).trim();
            if provider_name.is_empty() || first.contains(provider_name) {
                String::from(first)
            } else {
                format!("{provider_name} · {first}")
            }
        } else {
            String::from(provider_name)
        };

        out.push(StreamCandidate {
            provider_name: badge_provider,
            title: String::from(title),
            behavior_filename,
            binge_group,
            info_hash,
            file_idx,
            direct_url: clean_direct,
            trackers,
            seeders: meta.seeders,
            leechers: None,
            size_bytes: hint_size.or(meta.size_bytes),
            resolution_p: meta.resolution_p,
            video_codec: meta.video_codec,
            hdr_badge: meta.hdr_badge,
            source_indexer: meta.source_indexer,
        });
    }
    Ok(out)
}

/// Parses a `json_array_btih` response (`[{ name, info_hash, seeders, leechers, size }]`),
/// tolerating both string and integer numeric fields and skipping the all-zero empty sentinel.
pub fn parse_btih_array_json(indexer_name: &str, raw_json: &str) -> Result<Vec<StreamCandidate>, PackError> {
    let root = json::parse(raw_json)?;
    let Some(arr) = root.as_arr() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(obj) = item.as_obj() else { continue };
        let Some(info_hash) = json::get_str(obj, "info_hash").and_then(normalize_btih) else { continue };
        if info_hash == ZERO_BTIH {
            continue;
        }
        let name = json::get_str(obj, "name").unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let seeders = json::get_u64(obj, "seeders").map(|v| v as u32);
        let leechers = json::get_u64(obj, "leechers").map(|v| v as u32);
        let size_bytes = json::get_u64(obj, "size").filter(|&b| b > 0);
        let meta = inspect_text_telemetry(name);
        out.push(StreamCandidate {
            provider_name: String::from(indexer_name),
            title: String::from(name),
            behavior_filename: String::new(),
            binge_group: String::new(),
            info_hash,
            file_idx: None,
            direct_url: String::new(),
            trackers: Vec::new(),
            seeders,
            leechers,
            size_bytes: size_bytes.or(meta.size_bytes),
            resolution_p: meta.resolution_p,
            video_codec: meta.video_codec,
            hdr_badge: meta.hdr_badge,
            source_indexer: Some(String::from(indexer_name)),
        });
    }
    Ok(out)
}

/// Parses a `json_movies_torrents` response (`{ data: { movies: [{ title_long, torrents: [...] }] } }`).
pub fn parse_movies_torrents_json(indexer_name: &str, raw_json: &str) -> Result<Vec<StreamCandidate>, PackError> {
    let root = json::parse(raw_json)?;
    let Some(obj) = root.as_obj() else {
        return Err(PackError::ExpectedObject);
    };
    let Some(data) = json::get_obj(obj, "data") else {
        return Ok(Vec::new());
    };
    let Some(movies) = json::get_arr(data, "movies") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for m in movies {
        let Some(m_obj) = m.as_obj() else { continue };
        let movie_title = json::get_str(m_obj, "title_long")
            .or_else(|| json::get_str(m_obj, "title"))
            .unwrap_or("Movie");
        let Some(torrents) = json::get_arr(m_obj, "torrents") else { continue };
        for t in torrents {
            let Some(t_obj) = t.as_obj() else { continue };
            let Some(info_hash) = json::get_str(t_obj, "hash").and_then(normalize_btih) else { continue };
            let quality = json::get_str(t_obj, "quality").unwrap_or("");
            let kind = json::get_str(t_obj, "type").unwrap_or("");
            let vcodec = json::get_str(t_obj, "video_codec").unwrap_or("");
            let release = format!("{movie_title} {quality} {kind} {vcodec}").trim().to_owned();
            let meta = inspect_text_telemetry(&release);
            let size_bytes = json::get_u64(t_obj, "size_bytes")
                .filter(|&b| b > 0)
                .or_else(|| json::get_str(t_obj, "size").and_then(parse_human_size));
            out.push(StreamCandidate {
                provider_name: String::from(indexer_name),
                title: release,
                behavior_filename: String::new(),
                binge_group: String::new(),
                info_hash,
                file_idx: None,
                direct_url: String::new(),
                trackers: Vec::new(),
                seeders: json::get_u64(t_obj, "seeds").map(|v| v as u32),
                leechers: json::get_u64(t_obj, "peers").map(|v| v as u32),
                size_bytes: size_bytes.or(meta.size_bytes),
                resolution_p: meta.resolution_p,
                video_codec: meta.video_codec,
                hdr_badge: meta.hdr_badge,
                source_indexer: Some(String::from(indexer_name)),
            });
        }
    }
    Ok(out)
}

/// Parses RSS 2.0 / Torznab / namespace-tagged torrent feeds (`<item>` blocks).
pub fn parse_rss_swarm_xml(indexer_name: &str, raw_xml: &str) -> Vec<StreamCandidate> {
    let mut out = Vec::new();
    let mut rest = raw_xml;
    while let Some(start) = find_ascii_case_insensitive(rest, "<item") {
        let after_open = &rest[start..];
        let Some(gt) = after_open.find('>') else { break };
        let body_and_more = &after_open[gt + 1..];
        let Some(end) = find_ascii_case_insensitive(body_and_more, "</item>") else { break };
        let item_xml = &body_and_more[..end];
        rest = &body_and_more[end + 7..];

        let title = xml_unescape(&extract_xml_tag_text(item_xml, "title").unwrap_or_default());
        if title.trim().is_empty() {
            continue;
        }
        let link = xml_unescape(&extract_xml_tag_text(item_xml, "link").unwrap_or_default());
        let enclosure_url = extract_enclosure_url(item_xml).map(|s| xml_unescape(&s)).unwrap_or_default();
        let magnet_tag = xml_unescape(&extract_xml_tag_suffix(item_xml, "magneturi").unwrap_or_default());

        let mut trackers = Vec::new();
        let mut info_hash = extract_xml_tag_suffix(item_xml, "infohash")
            .and_then(|s| normalize_btih(s.trim()))
            .unwrap_or_default();

        for cand in [&magnet_tag, &link, &enclosure_url] {
            if cand.starts_with("magnet:") {
                if info_hash.is_empty() {
                    if let Some(h) = extract_btih_from_magnet(cand) {
                        info_hash = h;
                    }
                }
                extract_trackers_from_magnet(cand, &mut trackers);
            }
        }

        let direct_url = if info_hash.is_empty() {
            if enclosure_url.starts_with("http://") || enclosure_url.starts_with("https://") {
                enclosure_url
            } else if link.starts_with("http://") || link.starts_with("https://") {
                link
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if info_hash.is_empty() && direct_url.is_empty() {
            continue;
        }

        let seeders = extract_xml_tag_suffix(item_xml, "seeders")
            .and_then(|s| s.trim().parse::<u32>().ok())
            .or_else(|| extract_torznab_attr(item_xml, "seeders").and_then(|s| s.parse::<u32>().ok()));
        let leechers = extract_xml_tag_suffix(item_xml, "leechers")
            .and_then(|s| s.trim().parse::<u32>().ok())
            .or_else(|| extract_torznab_attr(item_xml, "peers").and_then(|s| s.parse::<u32>().ok()));
        let size_bytes = extract_xml_tag_suffix(item_xml, "size")
            .and_then(|s| s.trim().parse::<u64>().ok().or_else(|| parse_human_size(&s)))
            .or_else(|| extract_torznab_attr(item_xml, "size").and_then(|s| s.parse::<u64>().ok()))
            .or_else(|| extract_enclosure_length(item_xml));

        let desc = xml_unescape(&extract_xml_tag_text(item_xml, "description").unwrap_or_default());
        let combined = format!("{title}\n{desc}");
        let meta = inspect_text_telemetry(&combined);

        out.push(StreamCandidate {
            provider_name: String::from(indexer_name),
            title: String::from(title.trim()),
            behavior_filename: String::new(),
            binge_group: String::new(),
            info_hash,
            file_idx: None,
            direct_url,
            trackers,
            seeders: seeders.or(meta.seeders),
            leechers,
            size_bytes: size_bytes.or(meta.size_bytes),
            resolution_p: meta.resolution_p,
            video_codec: meta.video_codec,
            hdr_badge: meta.hdr_badge,
            source_indexer: Some(String::from(indexer_name)),
        });
    }
    out
}

#[derive(Default)]
struct TextTelemetry {
    seeders: Option<u32>,
    size_bytes: Option<u64>,
    resolution_p: u32,
    video_codec: Option<&'static str>,
    hdr_badge: Option<&'static str>,
    source_indexer: Option<String>,
}

fn inspect_text_telemetry(text: &str) -> TextTelemetry {
    let lower = text.to_ascii_lowercase();
    let resolution_p = detect_resolution(&lower);
    let video_codec = detect_codec(&lower);
    let hdr_badge = detect_hdr(&lower);
    let seeders = extract_after_marker_u32(text, "👤")
        .or_else(|| extract_after_ascii_label_u32(&lower, "seeders:"))
        .or_else(|| extract_after_ascii_label_u32(&lower, "seeds:"))
        .or_else(|| extract_after_ascii_label_u32(&lower, "s:"));
    let size_bytes = extract_after_marker_size(text, "💾").or_else(|| scan_any_human_size(text));
    let source_indexer = extract_source_indexer(text);

    TextTelemetry {
        seeders,
        size_bytes,
        resolution_p,
        video_codec,
        hdr_badge,
        source_indexer,
    }
}

fn detect_resolution(lower: &str) -> u32 {
    for (needle, p) in [
        ("2160p", 2160),
        ("4k", 2160),
        ("uhd", 2160),
        ("1440p", 1440),
        ("1080p", 1080),
        ("1080i", 1080),
        ("720p", 720),
        ("576p", 576),
        ("480p", 480),
    ] {
        if lower.contains(needle) {
            return p;
        }
    }
    0
}

fn detect_codec(lower: &str) -> Option<&'static str> {
    if lower.contains("av1") {
        Some("AV1")
    } else if ["hevc", "x265", "h265", "h.265"].iter().any(|n| lower.contains(n)) {
        Some("HEVC")
    } else if ["x264", "h264", "h.264", "avc"].iter().any(|n| lower.contains(n)) {
        Some("H.264")
    } else {
        None
    }
}

fn detect_hdr(lower: &str) -> Option<&'static str> {
    let dv = lower.contains("dolby vision") || lower.contains(".dv.") || lower.contains(" dv ") || lower.contains("dovi");
    let hdr10p = lower.contains("hdr10+") || lower.contains("hdr10plus");
    let hdr = hdr10p || lower.contains("hdr");
    match (dv, hdr10p, hdr) {
        (true, _, true) => Some("DV · HDR"),
        (true, _, false) => Some("DV"),
        (false, true, _) => Some("HDR10+"),
        (false, false, true) => Some("HDR"),
        _ => None,
    }
}

fn extract_after_marker_u32(text: &str, marker: &str) -> Option<u32> {
    let idx = text.find(marker)?;
    let rest = text[idx + marker.len()..].trim_start();
    parse_leading_u32(rest)
}

fn extract_after_ascii_label_u32(lower: &str, label: &str) -> Option<u32> {
    let mut search = lower;
    while let Some(idx) = search.find(label) {
        let ok_boundary = idx == 0 || !search.as_bytes()[idx - 1].is_ascii_alphanumeric();
        let after = &search[idx + label.len()..];
        if ok_boundary {
            if let Some(v) = parse_leading_u32(after.trim_start()) {
                return Some(v);
            }
        }
        search = after;
    }
    None
}

fn parse_leading_u32(s: &str) -> Option<u32> {
    let mut val: u64 = 0;
    let mut digits = 0usize;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            val = val.saturating_mul(10).saturating_add((b - b'0') as u64);
            digits += 1;
        } else if b == b',' && digits > 0 {
            continue;
        } else {
            break;
        }
    }
    if digits == 0 { None } else { Some(val.min(u32::MAX as u64) as u32) }
}

fn extract_after_marker_size(text: &str, marker: &str) -> Option<u64> {
    let idx = text.find(marker)?;
    parse_human_size(text[idx + marker.len()..].trim_start())
}

fn scan_any_human_size(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            if let Some(sz) = parse_human_size(&text[i..]) {
                return Some(sz);
            }
        }
        i += 1;
    }
    None
}

/// Parses `"1.42 GB"`, `"850 MiB"`, `"12.5 GiB"` into bytes.
pub fn parse_human_size(s: &str) -> Option<u64> {
    let b = s.trim_start().as_bytes();
    let mut i = 0;
    let mut whole: u64 = 0;
    let mut saw_digit = false;
    while i < b.len() && b[i].is_ascii_digit() {
        whole = whole.saturating_mul(10).saturating_add((b[i] - b'0') as u64);
        saw_digit = true;
        i += 1;
    }
    if !saw_digit {
        return None;
    }
    let mut frac_num: u64 = 0;
    let mut frac_div: u64 = 1;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            if frac_div < 1_000_000 {
                frac_num = frac_num * 10 + (b[i] - b'0') as u64;
                frac_div *= 10;
            }
            i += 1;
        }
    }
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    let unit_start = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == unit_start {
        return None;
    }
    let unit = core::str::from_utf8(&b[unit_start..i]).ok()?;
    let mult: u64 = if unit.eq_ignore_ascii_case("tb") || unit.eq_ignore_ascii_case("tib") {
        1024 * 1024 * 1024 * 1024
    } else if unit.eq_ignore_ascii_case("gb") || unit.eq_ignore_ascii_case("gib") {
        1024 * 1024 * 1024
    } else if unit.eq_ignore_ascii_case("mb") || unit.eq_ignore_ascii_case("mib") {
        1024 * 1024
    } else if unit.eq_ignore_ascii_case("kb") || unit.eq_ignore_ascii_case("kib") {
        1024
    } else {
        return None;
    };
    Some(whole.saturating_mul(mult).saturating_add(frac_num.saturating_mul(mult) / frac_div))
}

fn extract_source_indexer(text: &str) -> Option<String> {
    let idx = text.find('⚙')?;
    let mut rest = &text[idx + '⚙'.len_utf8()..];
    if let Some(stripped) = rest.strip_prefix('\u{fe0f}') {
        rest = stripped;
    }
    let line = rest.lines().next()?.trim();
    let token = line.split(['·', '|', '\t']).next()?.trim();
    if token.is_empty() { None } else { Some(String::from(token)) }
}

fn is_season_pack_for(text: &str, target_season: u32) -> bool {
    let lower = text.to_ascii_lowercase();
    let b = lower.as_bytes();

    // Check `S01-S05` / `S1-S4` / `S01 - S03` or standalone `S01` without `E..`.
    for i in 0..b.len() {
        if b[i] == b's' && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            if let Some((s1, j)) = read_u32_digits(b, i + 1, 2) {
                let mut k = j;
                while k < b.len() && (b[k] == b' ' || b[k] == b'.') {
                    k += 1;
                }
                if k < b.len() && (b[k] == b'-' || b[k] == b'~') {
                    k += 1;
                    while k < b.len() && (b[k] == b' ' || b[k] == b's') {
                        k += 1;
                    }
                    if let Some((s2, _)) = read_u32_digits(b, k, 2) {
                        if (s1..=s2).contains(&target_season) {
                            return true;
                        }
                    }
                } else if k >= b.len() || (b[k] != b'e' && !b[k].is_ascii_alphanumeric()) {
                    if s1 == target_season {
                        return true;
                    }
                }
            }
        }
    }

    // Check `"season 2"` / `"season 02"`.
    if let Some(pos) = lower.find("season") {
        let after = lower[pos + 6..].trim_start_matches([' ', '.', '_', '-']);
        if let Some((s1, _)) = read_u32_digits(after.as_bytes(), 0, 2) {
            if s1 == target_season {
                return true;
            }
        }
    }

    lower.contains("complete series") || lower.contains("complete collection")
}

fn read_u32_digits(b: &[u8], start: usize, max_len: usize) -> Option<(u32, usize)> {
    let mut i = start;
    let mut val = 0u32;
    while i < b.len() && i - start <= max_len && b[i].is_ascii_digit() {
        val = val * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    let count = i - start;
    if count == 0 || count > max_len { None } else { Some((val, i)) }
}

/// Validates and lowercases a 40-hex or 32-base32 BitTorrent info-hash.
pub fn normalize_btih(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s.to_ascii_lowercase())
    } else if s.len() == 32 && s.bytes().all(|b| b.is_ascii_alphabetic() || (b'2'..=b'7').contains(&b)) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}

pub fn extract_btih_from_magnet(magnet: &str) -> Option<String> {
    let lower = magnet.to_ascii_lowercase();
    let idx = lower.find("urn:btih:")?;
    let after = &magnet[idx + 9..];
    let end = after.find(['&', '#', ' ', '"', '\'']).unwrap_or(after.len());
    normalize_btih(&after[..end])
}

fn extract_trackers_from_magnet(magnet: &str, out: &mut Vec<String>) {
    let Some(qpos) = magnet.find('?') else { return };
    for param in magnet[qpos + 1..].split('&') {
        if let Some(raw_tr) = param.strip_prefix("tr=") {
            let decoded = percent_decode(raw_tr);
            if !decoded.is_empty() && !out.iter().any(|e| e.eq_ignore_ascii_case(&decoded)) {
                out.push(decoded);
            }
        }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut bytes = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (from_hex(b[i + 1]), from_hex(b[i + 2])) {
                bytes.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        } else if b[i] == b'+' {
            bytes.push(b' ');
            i += 1;
            continue;
        }
        bytes.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn find_ascii_case_insensitive(haystack: &str, needle_lower: &str) -> Option<usize> {
    let hb = haystack.as_bytes();
    let nb = needle_lower.as_bytes();
    if nb.is_empty() || hb.len() < nb.len() {
        return None;
    }
    hb.windows(nb.len())
        .position(|w| w.iter().zip(nb.iter()).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

fn strip_cdata(s: &str) -> &str {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")) {
        inner
    } else {
        t
    }
}

fn extract_xml_tag_text(xml: &str, tag_lower: &str) -> Option<String> {
    let open = format!("<{tag_lower}");
    let close = format!("</{tag_lower}>");
    let start = find_ascii_case_insensitive(xml, &open)?;
    let after = &xml[start + open.len()..];
    if !after.starts_with('>') && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let gt = after.find('>')?;
    let content = &after[gt + 1..];
    let end = find_ascii_case_insensitive(content, &close)?;
    Some(String::from(strip_cdata(&content[..end])))
}

/// Matches `<prefix:local>` or `<local>` (for `<ns:seeders>`, `<ns:infoHash>`, `<ns:size>`).
fn extract_xml_tag_suffix(xml: &str, local_lower: &str) -> Option<String> {
    let mut search = xml;
    while let Some(lt) = search.find('<') {
        let rest = &search[lt + 1..];
        if rest.starts_with('/') || rest.starts_with('!') || rest.starts_with('?') {
            search = rest;
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag_head = rest[..gt].split_whitespace().next().unwrap_or("");
        let local = tag_head.rsplit(':').next().unwrap_or(tag_head);
        if local.eq_ignore_ascii_case(local_lower) {
            let close = format!("</{tag_head}>");
            let body = &rest[gt + 1..];
            if let Some(end) = find_ascii_case_insensitive(body, &close.to_ascii_lowercase()) {
                return Some(String::from(strip_cdata(&body[..end])));
            }
        }
        search = &rest[gt + 1..];
    }
    None
}

fn extract_torznab_attr(xml: &str, attr_name: &str) -> Option<String> {
    let mut search = xml;
    while let Some(pos) = find_ascii_case_insensitive(search, "torznab:attr") {
        let after = &search[pos..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        if let (Some(n), Some(v)) = (extract_xml_attr(tag, "name"), extract_xml_attr(tag, "value")) {
            if n.eq_ignore_ascii_case(attr_name) {
                return Some(v);
            }
        }
        search = &after[gt + 1..];
    }
    None
}

fn extract_enclosure_url(xml: &str) -> Option<String> {
    let pos = find_ascii_case_insensitive(xml, "<enclosure")?;
    let after = &xml[pos..];
    let gt = after.find('>')?;
    extract_xml_attr(&after[..gt], "url")
}

fn extract_enclosure_length(xml: &str) -> Option<u64> {
    let pos = find_ascii_case_insensitive(xml, "<enclosure")?;
    let after = &xml[pos..];
    let gt = after.find('>')?;
    extract_xml_attr(&after[..gt], "length")?.parse::<u64>().ok().filter(|&n| n > 0)
}

fn extract_xml_attr(tag: &str, attr_lower: &str) -> Option<String> {
    let needle_dq = format!("{attr_lower}=\"");
    if let Some(p) = find_ascii_case_insensitive(tag, &needle_dq) {
        let rest = &tag[p + needle_dq.len()..];
        let end = rest.find('"')?;
        return Some(String::from(&rest[..end]));
    }
    let needle_sq = format!("{attr_lower}='");
    if let Some(p) = find_ascii_case_insensitive(tag, &needle_sq) {
        let rest = &tag[p + needle_sq.len()..];
        let end = rest.find('\'')?;
        return Some(String::from(&rest[..end]));
    }
    None
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}
