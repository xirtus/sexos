// SPDX-License-Identifier: GPL-3.0-only
//! Generic `no_std` protocol parsers and HTTP fetchers for Swarm indexers,
//! Featured Feeds, and Direct Sources configured in `provider_pack.json`.
//!
//! All parsers are protocol-driven (`json_array_btih`, `json_movies_torrents`,
//! `rss`, `internet_archive`, generic JSON) with zero bundled private URLs.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::Window;

use crate::provider::{
    json_array_slice, json_object_slice, json_objects, json_str_field, json_u32_field,
    json_u64_field, DirectSource, FeaturedFeed, ProviderPack,
};
use crate::stremio::{build_magnet, detect_resolution, http_get};

/// Bounded HTTP response limit for Swarm feeds and direct API endpoints (768 KiB).
const MAX_FEED_BYTES: usize = 768 * 1024;
/// Maximum items parsed from any single feed or indexer response.
const MAX_FEED_ITEMS: usize = 60;
/// Maximum number of Swarm indexers queried in a single multi-indexer search.
const MAX_SEARCH_INDEXERS: usize = 4;

#[derive(Clone, Debug)]
pub struct SwarmResult {
    pub title: String,
    pub source_name: String,
    pub poster_url: String,
    pub info_hash: String,
    pub magnet_uri: String,
    pub direct_url: String,
    pub quality: String,
    pub seeders: Option<u32>,
    pub size_text: String,
    pub year: String,
    #[allow(dead_code)]
    pub imdb_id: String,
}

impl SwarmResult {
    pub fn initials(&self) -> String {
        let mut out = String::new();
        for word in self.title.split_whitespace() {
            if let Some(ch) = word.chars().next() {
                if ch.is_ascii_alphanumeric() {
                    out.push(ch.to_ascii_uppercase());
                    if out.len() >= 2 {
                        break;
                    }
                }
            }
        }
        if out.is_empty() {
            out.push_str("SW");
        }
        out
    }

    pub fn has_magnet(&self) -> bool {
        !self.magnet_uri.is_empty()
    }

    pub fn has_direct(&self) -> bool {
        !self.direct_url.is_empty()
    }

    /// Returns true if `direct_url` appears to point directly at a playable media file or HLS manifest.
    pub fn is_direct_media_stream(&self) -> bool {
        is_media_stream_url(&self.direct_url)
    }
}

/// Returns true if `url` appears to point directly at a playable video/audio file or HLS manifest.
pub fn is_media_stream_url(url: &str) -> bool {
    let base = url
        .split('?')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    for ext in [
        ".mp4", ".mkv", ".webm", ".m3u8", ".mov", ".m4v", ".ts", ".mp3", ".flac", ".ogg",
        ".opus", ".m4a",
    ] {
        if base.ends_with(ext) {
            return true;
        }
    }
    false
}

/// Builds a compact `get <target>` argument string for `swingers` that stays strictly
/// under `sex_pdx::fs::ARGS_MAX` (256 bytes). Prefers the bare 40-char hex `info_hash`
/// (44 bytes total), falling back to a tracker-stripped magnet URI capped at 200 bytes.
#[allow(dead_code)]
pub fn swingers_get_arg(info_hash: &str, magnet_uri: &str) -> String {
    let hash = info_hash.trim();
    if !hash.is_empty() {
        return format!("get {}", hash);
    }
    let mag = magnet_uri.trim();
    let short = mag.split("&tr=").next().unwrap_or(mag);
    let capped = if short.len() > 200 {
        let mut end = 200;
        while !short.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &short[..end]
    } else {
        short
    };
    format!("get {}", capped)
}

/// Builds a compact `stream <target> --port <port>` argument string for `swingers`
/// strictly under `sex_pdx::fs::ARGS_MAX` (256 bytes). Prefers the bare 40-char hex
/// `info_hash` (59 bytes total), falling back to a tracker-stripped magnet URI capped
/// at 180 bytes.
pub fn swingers_stream_arg(info_hash: &str, magnet_uri: &str, port: u16) -> String {
    let hash = info_hash.trim();
    if !hash.is_empty() {
        return format!("stream {} --port {}", hash, port);
    }
    let mag = magnet_uri.trim();
    let short = mag.split("&tr=").next().unwrap_or(mag);
    let capped = if short.len() > 180 {
        let mut end = 180;
        while !short.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &short[..end]
    } else {
        short
    };
    format!("stream {} --port {}", capped, port)
}

/// Formats a byte count into a concise human-readable string (e.g. `1.4 GB`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return String::new();
    }
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if bytes >= GB {
        let whole = bytes / GB;
        let tenths = ((bytes % GB) * 10) / GB;
        format!("{}.{} GB", whole, tenths)
    } else if bytes >= MB {
        let whole = bytes / MB;
        format!("{} MB", whole)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// Formats a size field that may already be human-readable (`"1.4 GB"`) or raw byte count (`"1503238553"`).
fn normalize_size(raw: Option<String>, raw_bytes: Option<u64>) -> String {
    if let Some(s) = raw {
        let t = s.trim();
        if !t.is_empty() {
            if let Ok(b) = t.parse::<u64>() {
                return format_bytes(b);
            }
            return String::from(t);
        }
    }
    if let Some(b) = raw_bytes {
        return format_bytes(b);
    }
    String::new()
}

fn is_valid_btih(hash: &str) -> bool {
    let h = hash.trim();
    if h.len() != 40 && h.len() != 32 {
        return false;
    }
    if h.bytes().all(|b| b == b'0') {
        return false;
    }
    h.bytes()
        .all(|b| b.is_ascii_hexdigit() || (h.len() == 32 && b.is_ascii_alphanumeric()))
}

/// Parses a top-level JSON array of torrent objects:
/// `[ { "name", "info_hash", "seeders", "size", "imdb" }, ... ]`.
pub fn parse_json_array_btih(
    text: &str,
    source_name: &str,
    trackers: &[String],
) -> Vec<SwarmResult> {
    let mut out = Vec::new();
    let trimmed = text.trim();
    let slice = if trimmed.starts_with('[') {
        trimmed
    } else if let Some(arr) = json_array_slice(trimmed, "results")
        .or_else(|| json_array_slice(trimmed, "items"))
        .or_else(|| json_array_slice(trimmed, "data"))
    {
        arr
    } else {
        return out;
    };

    for obj in json_objects(slice) {
        if out.len() >= MAX_FEED_ITEMS {
            break;
        }
        let title = json_str_field(obj, "name")
            .or_else(|| json_str_field(obj, "title"))
            .unwrap_or_default();
        let info_hash = json_str_field(obj, "info_hash")
            .or_else(|| json_str_field(obj, "infoHash"))
            .or_else(|| json_str_field(obj, "hash"))
            .unwrap_or_default();
        if title.is_empty() || !is_valid_btih(&info_hash) {
            continue;
        }
        let seeders = json_u32_field(obj, "seeders").or_else(|| json_u32_field(obj, "seeds"));
        let size_text = normalize_size(
            json_str_field(obj, "size"),
            json_u64_field(obj, "size_bytes"),
        );
        let imdb_id = json_str_field(obj, "imdb")
            .or_else(|| json_str_field(obj, "imdb_id"))
            .unwrap_or_default();
        let year = json_str_field(obj, "year").unwrap_or_default();
        let quality = detect_resolution(&title);
        let magnet_uri = build_magnet(&info_hash, &title, trackers);

        out.push(SwarmResult {
            title,
            source_name: String::from(source_name),
            poster_url: String::new(),
            info_hash,
            magnet_uri,
            direct_url: String::new(),
            quality,
            seeders,
            size_text,
            year,
            imdb_id,
        });
    }
    out
}

/// Parses a movie-catalog torrent JSON response:
/// `{ "data": { "movies": [ { "title", "year", "rating", "medium_cover_image",
///   "imdb_code", "torrents": [ { "hash", "quality", "seeds", "size" } ] } ] } }`.
pub fn parse_json_movies_torrents(
    text: &str,
    source_name: &str,
    trackers: &[String],
) -> Vec<SwarmResult> {
    let mut out = Vec::new();
    let Some(movies_arr) = json_array_slice(text, "movies") else {
        return out;
    };

    for movie_obj in json_objects(movies_arr) {
        if out.len() >= MAX_FEED_ITEMS {
            break;
        }
        let title = json_str_field(movie_obj, "title")
            .or_else(|| json_str_field(movie_obj, "title_english"))
            .or_else(|| json_str_field(movie_obj, "title_long"))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let year = json_str_field(movie_obj, "year").unwrap_or_default();
        let poster_url = json_str_field(movie_obj, "medium_cover_image")
            .or_else(|| json_str_field(movie_obj, "large_cover_image"))
            .or_else(|| json_str_field(movie_obj, "small_cover_image"))
            .unwrap_or_default();
        let imdb_id = json_str_field(movie_obj, "imdb_code").unwrap_or_default();

        // Pick the best (highest-seeded) valid torrent for this movie.
        let mut best: Option<(String, String, Option<u32>, String)> = None;
        if let Some(t_arr) = json_array_slice(movie_obj, "torrents") {
            for t_obj in json_objects(t_arr) {
                let hash = json_str_field(t_obj, "hash").unwrap_or_default();
                if !is_valid_btih(&hash) {
                    continue;
                }
                let quality = json_str_field(t_obj, "quality").unwrap_or_default();
                let seeds = json_u32_field(t_obj, "seeds").or_else(|| json_u32_field(t_obj, "seeders"));
                let size_text = normalize_size(
                    json_str_field(t_obj, "size"),
                    json_u64_field(t_obj, "size_bytes"),
                );
                let replace = match &best {
                    None => true,
                    Some((_, _, cur_seeds, _)) => seeds.unwrap_or(0) > cur_seeds.unwrap_or(0),
                };
                if replace {
                    best = Some((hash, quality, seeds, size_text));
                }
            }
        }

        let Some((info_hash, raw_quality, seeders, size_text)) = best else {
            continue;
        };
        let quality = if raw_quality.eq_ignore_ascii_case("2160p") {
            String::from("4K")
        } else if !raw_quality.is_empty() {
            raw_quality
        } else {
            detect_resolution(&title)
        };
        let magnet_uri = build_magnet(&info_hash, &title, trackers);

        out.push(SwarmResult {
            title,
            source_name: String::from(source_name),
            poster_url,
            info_hash,
            magnet_uri,
            direct_url: String::new(),
            quality,
            seeders,
            size_text,
            year,
            imdb_id,
        });
    }
    out
}

/// Lightweight `no_std` XML `<item>` scanner for RSS / Torznab swarm feeds.
/// Extracts `<title>`, namespaced or plain `<*:infoHash>` (or `btih:` hex in
/// `<link>`/`<guid>`/`<description>`), `<*:seeders>`, and `<*:size>`.
pub fn parse_rss_swarm(xml: &str, source_name: &str, trackers: &[String]) -> Vec<SwarmResult> {
    let mut out = Vec::new();
    let mut rest = xml;

    while let Some(item_start) = find_ascii_ci(rest, "<item") {
        if out.len() >= MAX_FEED_ITEMS {
            break;
        }
        let after_open = &rest[item_start + 5..];
        let Some(tag_end) = after_open.find('>') else {
            break;
        };
        let body_start = &after_open[tag_end + 1..];
        let Some(item_end) = find_ascii_ci(body_start, "</item>") else {
            break;
        };
        let item_xml = &body_start[..item_end];
        rest = &body_start[item_end + 7..];

        let title = xml_tag_text(item_xml, "title").unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        let link = xml_tag_text(item_xml, "link").unwrap_or_default();
        let guid = xml_tag_text(item_xml, "guid").unwrap_or_default();
        let desc = xml_tag_text(item_xml, "description").unwrap_or_default();

        let mut info_hash = xml_tag_text(item_xml, "infohash").unwrap_or_default();
        if !is_valid_btih(&info_hash) {
            info_hash = extract_btih(&link)
                .or_else(|| extract_btih(&guid))
                .or_else(|| extract_btih(&desc))
                .or_else(|| extract_btih(item_xml))
                .unwrap_or_default();
        }

        let magnet_uri = if link.starts_with("magnet:?") {
            link.clone()
        } else if is_valid_btih(&info_hash) {
            build_magnet(&info_hash, &title, trackers)
        } else {
            String::new()
        };

        let direct_url = if magnet_uri.is_empty()
            && (link.starts_with("http://") || link.starts_with("https://"))
        {
            link
        } else {
            String::new()
        };

        if magnet_uri.is_empty() && direct_url.is_empty() {
            continue;
        }

        let seeders = xml_tag_text(item_xml, "seeders").and_then(|s| s.trim().parse::<u32>().ok());
        let size_text = normalize_size(xml_tag_text(item_xml, "size"), None);
        let quality = detect_resolution(&title);

        out.push(SwarmResult {
            title,
            source_name: String::from(source_name),
            poster_url: String::new(),
            info_hash,
            magnet_uri,
            direct_url,
            quality,
            seeders,
            size_text,
            year: String::new(),
            imdb_id: String::new(),
        });
    }
    out
}

/// Parses an Internet Archive advanced-search JSON response:
/// `{ "response": { "docs": [ { "identifier", "title", "year", "description" } ] } }`.
pub fn parse_internet_archive(text: &str, source_name: &str) -> Vec<SwarmResult> {
    let mut out = Vec::new();
    let Some(docs_arr) = json_array_slice(text, "docs") else {
        return out;
    };

    for obj in json_objects(docs_arr) {
        if out.len() >= MAX_FEED_ITEMS {
            break;
        }
        let identifier = json_str_field(obj, "identifier").unwrap_or_default();
        if identifier.is_empty() {
            continue;
        }
        let title = json_str_field(obj, "title").unwrap_or_else(|| identifier.clone());
        let year = json_str_field(obj, "year").unwrap_or_default();
        let direct_url = format!("https://archive.org/details/{}", identifier);
        let poster_url = format!("https://archive.org/services/img/{}", identifier);

        out.push(SwarmResult {
            title,
            source_name: String::from(source_name),
            poster_url,
            info_hash: String::new(),
            magnet_uri: String::new(),
            direct_url,
            quality: String::from("ARCHIVE"),
            seeders: None,
            size_text: String::new(),
            year,
            imdb_id: String::new(),
        });
    }
    out
}

/// Fallback generic JSON scanner for `videos` / `results` / `items` / `data` arrays.
pub fn parse_generic_direct_json(text: &str, source_name: &str) -> Vec<SwarmResult> {
    let mut out = Vec::new();
    let trimmed = text.trim();
    let slice = json_array_slice(trimmed, "videos")
        .or_else(|| json_array_slice(trimmed, "results"))
        .or_else(|| json_array_slice(trimmed, "items"))
        .or_else(|| json_array_slice(trimmed, "data"))
        .or_else(|| trimmed.starts_with('[').then_some(trimmed));
    let Some(arr) = slice else {
        return out;
    };

    for obj in json_objects(arr) {
        if out.len() >= MAX_FEED_ITEMS {
            break;
        }
        let title = json_str_field(obj, "title")
            .or_else(|| json_str_field(obj, "name"))
            .unwrap_or_default();
        let direct_url = json_str_field(obj, "hls_url")
            .or_else(|| json_str_field(obj, "stream_url"))
            .or_else(|| json_str_field(obj, "url"))
            .or_else(|| json_str_field(obj, "embed"))
            .or_else(|| json_str_field(obj, "link"))
            .unwrap_or_default();
        if title.is_empty() || direct_url.is_empty() {
            continue;
        }
        let poster_url = json_object_slice(obj, "default_thumb")
            .and_then(|t| json_str_field(t, "src"))
            .or_else(|| json_str_field(obj, "thumb"))
            .or_else(|| json_str_field(obj, "thumbnail"))
            .or_else(|| json_str_field(obj, "poster"))
            .or_else(|| json_str_field(obj, "image"))
            .or_else(|| json_str_field(obj, "cover"))
            .unwrap_or_default();
        let quality = json_str_field(obj, "quality")
            .or_else(|| json_str_field(obj, "resolution"))
            .unwrap_or_else(|| detect_resolution(&title));
        let size_text = json_str_field(obj, "length_min")
            .or_else(|| json_str_field(obj, "duration"))
            .unwrap_or_default();
        let year = json_str_field(obj, "year").unwrap_or_default();

        out.push(SwarmResult {
            title,
            source_name: String::from(source_name),
            poster_url,
            info_hash: String::new(),
            magnet_uri: String::new(),
            direct_url,
            quality,
            seeders: None,
            size_text,
            year,
            imdb_id: String::new(),
        });
    }
    out
}

/// Dispatches a raw response body to the appropriate generic protocol parser based on `kind`,
/// with automatic format detection when `kind` is empty or unrecognized.
fn parse_by_kind(
    kind: &str,
    text: &str,
    source_name: &str,
    trackers: &[String],
) -> Vec<SwarmResult> {
    let k = kind.trim().to_ascii_lowercase();
    match k.as_str() {
        "json_array_btih" => parse_json_array_btih(text, source_name, trackers),
        "json_movies_torrents" => parse_json_movies_torrents(text, source_name, trackers),
        "rss" | "rss_xml" | "xml" | "torznab" => parse_rss_swarm(text, source_name, trackers),
        "internet_archive" | "archive" => parse_internet_archive(text, source_name),
        _ => {
            let trimmed = text.trim_start();
            if trimmed.starts_with('<') {
                parse_rss_swarm(text, source_name, trackers)
            } else if trimmed.contains("\"movies\"") && trimmed.contains("\"torrents\"") {
                parse_json_movies_torrents(text, source_name, trackers)
            } else if trimmed.contains("\"docs\"") && trimmed.contains("\"identifier\"") {
                parse_internet_archive(text, source_name)
            } else if trimmed.starts_with('[') && trimmed.contains("info_hash") {
                parse_json_array_btih(text, source_name, trackers)
            } else {
                parse_generic_direct_json(text, source_name)
            }
        }
    }
}

/// Fetches and parses a configured `FeaturedFeed` live over HTTP/HTTPS.
pub fn fetch_feed(
    window: &mut Window,
    feed: &FeaturedFeed,
    trackers: &[String],
) -> Result<Vec<SwarmResult>, String> {
    sex_pdx::serial_record!("[brootlite.feed] GET {} kind={}", feed.url, feed.kind);
    let body = http_get(window, &feed.url, MAX_FEED_BYTES)?;
    let text = core::str::from_utf8(&body).map_err(|_| String::from("invalid UTF-8 feed"))?;
    let items = parse_by_kind(&feed.kind, text, &feed.label, trackers);
    sex_pdx::serial_record!("[brootlite.feed] items={}", items.len());
    Ok(items)
}

/// Fetches and parses a configured `DirectSource` live over HTTP/HTTPS.
pub fn fetch_direct_source(
    window: &mut Window,
    src: &DirectSource,
) -> Result<Vec<SwarmResult>, String> {
    sex_pdx::serial_record!("[brootlite.direct] GET {} kind={}", src.api_url, src.kind);
    let body = http_get(window, &src.api_url, MAX_FEED_BYTES)?;
    let text = core::str::from_utf8(&body).map_err(|_| String::from("invalid UTF-8 source"))?;
    let items = parse_by_kind(&src.kind, text, &src.name, &[]);
    sex_pdx::serial_record!("[brootlite.direct] items={}", items.len());
    Ok(items)
}

/// Queries up to `MAX_SEARCH_INDEXERS` configured Swarm indexers in `provider_pack.json`,
/// parses each according to its `kind`, and sorts combined results by seeders descending.
pub fn search_swarm_indexers(
    window: &mut Window,
    pack: &ProviderPack,
    query: &str,
) -> (Vec<SwarmResult>, String) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return (Vec::new(), String::from("Enter a query to search Swarm indexers"));
    }
    let active_indexers: Vec<_> = pack
        .swarm_indexers
        .iter()
        .filter(|ix| !ix.search_url.is_empty())
        .take(MAX_SEARCH_INDEXERS)
        .collect();
    if active_indexers.is_empty() {
        return (
            Vec::new(),
            String::from("No Swarm indexers configured in provider_pack.json"),
        );
    }

    let mut combined = Vec::new();
    let mut queried = 0usize;
    let mut errors = 0usize;
    let mut last_err = String::new();

    for ix in active_indexers {
        let url = ix.build_query_url(trimmed);
        if url.is_empty() {
            continue;
        }
        queried += 1;
        sex_pdx::serial_record!("[brootlite.swarm] GET {} kind={}", url, ix.kind);
        match http_get(window, &url, MAX_FEED_BYTES) {
            Ok(body) => {
                if let Ok(text) = core::str::from_utf8(&body) {
                    let parsed = parse_by_kind(&ix.kind, text, &ix.name, &pack.trackers);
                    combined.extend(parsed.into_iter().take(24));
                }
            }
            Err(e) => {
                errors += 1;
                last_err = format!("{}: {}", ix.name, e);
            }
        }
    }

    combined.sort_by(|a, b| b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0)));

    let status = if !combined.is_empty() {
        format!(
            "{} Swarm result(s) across {} indexer(s)",
            combined.len(),
            queried
        )
    } else if errors == queried && queried > 0 {
        format!("Swarm error ({})", last_err)
    } else {
        format!("No Swarm results across {} indexer(s)", queried)
    };
    (combined, status)
}

// ── Lightweight XML helpers (no_std) ─────────────────────────────────────────

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() || nb.len() > hb.len() {
        return None;
    }
    for i in 0..=(hb.len() - nb.len()) {
        if hb[i..i + nb.len()].eq_ignore_ascii_case(nb) {
            return Some(i);
        }
    }
    None
}

/// Extracts inner text for `<tag>...</tag>` or `<ns:tag>...</ns:tag>` (case-insensitive local name).
fn xml_tag_text(xml: &str, local_name: &str) -> Option<String> {
    let bytes = xml.as_bytes();
    let target = local_name.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let open_start = i + 1;
        if open_start >= bytes.len() || bytes[open_start] == b'/' || bytes[open_start] == b'!' {
            i += 1;
            continue;
        }
        let Some(gt_rel) = xml[open_start..].find('>') else {
            break;
        };
        let tag_header = &xml[open_start..open_start + gt_rel];
        let full_name = tag_header
            .split(|c: char| c.is_ascii_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let local = full_name.rsplit(':').next().unwrap_or(full_name);
        if local.as_bytes().eq_ignore_ascii_case(target) && !tag_header.ends_with('/') {
            let content_start = open_start + gt_rel + 1;
            let close_pat = format!("</{}>", full_name);
            if let Some(close_rel) = find_ascii_ci(&xml[content_start..], &close_pat) {
                let raw = &xml[content_start..content_start + close_rel];
                return Some(clean_xml_text(raw));
            }
        }
        i = open_start + gt_rel + 1;
    }
    None
}

fn clean_xml_text(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(stripped) = s.strip_prefix("<![CDATA[") {
        if let Some(inner) = stripped.strip_suffix("]]>") {
            s = inner.trim();
        }
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

/// Scans text for a `btih:<hex>` info-hash token (e.g. inside a magnet URI or link).
fn extract_btih(s: &str) -> Option<String> {
    let pos = find_ascii_ci(s, "btih:")?;
    let after = &s[pos + 5..];
    let token: String = after
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if is_valid_btih(&token) {
        Some(token)
    } else {
        None
    }
}
