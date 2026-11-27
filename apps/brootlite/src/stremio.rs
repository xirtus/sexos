// SPDX-License-Identifier: GPL-3.0-only
//! Stremio v3 HTTP catalog, search, and stream resolver for BrootLite.
//!
//! Defaults to the standard public Stremio metadata catalog
//! (`https://v3-cinemeta.strem.io`) and queries any user-configured catalog or
//! stream addons from `provider_pack.json`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use sex_app::Window;

use crate::provider::{
    json_array_slice, json_objects, json_str_field, json_u32_field, percent_encode_query,
    ProviderPack,
};

/// Public Stremio v3 metadata catalog (standard open metadata provider).
pub const CINEMETA_BASE: &str = "https://v3-cinemeta.strem.io";
/// Direct Cinemeta CDN catalog endpoint (HTTP 200 without 307 redirect).
pub const CINEMETA_CATALOGS_BASE: &str = "https://cinemeta-catalogs.strem.io";
/// Secondary Cinemeta live endpoint for search fallback.
pub const CINEMETA_LIVE_BASE: &str = "https://cinemeta-live.strem.io";

const MAX_REDIRECTS: usize = 4;
const MAX_HTTP_ATTEMPTS: usize = 2;
/// Cinemeta series catalogs are ~580–610 KiB; 1 MiB fits them with headroom.
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_STREAM_BYTES: usize = 256 * 1024;
const MAX_CATALOG_ITEMS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    Movie,
    Series,
}

impl MediaType {
    pub const fn as_str(self) -> &'static str {
        match self {
            MediaType::Movie => "movie",
            MediaType::Series => "series",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            MediaType::Movie => "Movies",
            MediaType::Series => "Series",
        }
    }

    pub const fn badge(self) -> &'static str {
        match self {
            MediaType::Movie => "MOVIE",
            MediaType::Series => "SERIES",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogKind {
    Popular,
    TopRated,
}

impl CatalogKind {
    pub const fn catalog_id(self) -> &'static str {
        match self {
            CatalogKind::Popular => "top",
            CatalogKind::TopRated => "imdbRating",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            CatalogKind::Popular => "Popular",
            CatalogKind::TopRated => "Top Rated",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CatalogItem {
    pub id: String,
    pub media_type: String,
    pub name: String,
    pub poster: String,
    pub release_info: String,
    pub imdb_rating: String,
    pub description: String,
}

impl CatalogItem {
    pub fn initials(&self) -> String {
        let mut out = String::new();
        for word in self.name.split_whitespace() {
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
            out.push_str("BL");
        }
        out
    }

    pub fn type_badge(&self) -> &'static str {
        if self.media_type.eq_ignore_ascii_case("series") {
            "SERIES"
        } else {
            "MOVIE"
        }
    }
}

#[derive(Clone, Debug)]
pub struct StreamItem {
    pub addon_name: String,
    pub name: String,
    pub title: String,
    pub url: String,
    pub info_hash: String,
    pub file_idx: Option<u32>,
    pub magnet_uri: String,
    pub resolution: String,
    pub seeders: Option<u32>,
    pub is_direct: bool,
}

impl StreamItem {
    pub fn display_title(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if !self.name.is_empty() {
            &self.name
        } else {
            &self.addon_name
        }
    }
}

/// Performs a bounded HTTP/HTTPS GET request following up to `MAX_REDIRECTS`,
/// retrying once on transient network/DNS errors.
pub fn http_get(window: &mut Window, initial_url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut last_err = String::new();
    for _attempt in 0..MAX_HTTP_ATTEMPTS {
        let mut url = initial_url.to_string();
        let mut attempt_err: Option<String> = None;
        for _ in 0..=MAX_REDIRECTS {
            let req = sex_fetch::Request {
                method: "GET",
                url: &url,
                headers: &[("Accept", "application/json, image/*;q=0.8, */*;q=0.5")],
                body: &[],
                trusted_origin: false,
                referrer: None,
            };
            let resp = match sex_app::net::request(window, &req, None, max_bytes) {
                Ok(r) => r,
                Err(e) => {
                    attempt_err = Some(format!("net error: {e:?}"));
                    break;
                }
            };
            if resp.head.is_redirect() {
                let loc = resp.head.get("location").unwrap_or("");
                match sex_fetch::join(&url, loc) {
                    Some(next) => {
                        url = next;
                        continue;
                    }
                    None => return Err(String::from("invalid redirect")),
                }
            }
            if resp.head.status < 200 || resp.head.status >= 300 {
                return Err(format!("HTTP {}", resp.head.status));
            }
            return Ok(resp.body);
        }
        match attempt_err {
            Some(e) => last_err = e,
            None => return Err(String::from("too many redirects")),
        }
    }
    Err(last_err)
}

/// Parses a Stremio v3 catalog response (`{"metas": [...]}`).
pub fn parse_catalog_json(text: &str, fallback_type: &str) -> Vec<CatalogItem> {
    let mut items = Vec::new();
    let Some(arr) = json_array_slice(text, "metas") else {
        return items;
    };
    for obj in json_objects(arr) {
        if items.len() >= MAX_CATALOG_ITEMS {
            break;
        }
        let id = json_str_field(obj, "id").unwrap_or_default();
        let name = json_str_field(obj, "name").unwrap_or_default();
        if id.is_empty() || name.is_empty() {
            continue;
        }
        let media_type = json_str_field(obj, "type").unwrap_or_else(|| String::from(fallback_type));
        let poster = json_str_field(obj, "poster").unwrap_or_default();
        let release_info = json_str_field(obj, "releaseInfo")
            .or_else(|| json_str_field(obj, "year"))
            .unwrap_or_default();
        let imdb_rating = json_str_field(obj, "imdbRating").unwrap_or_default();
        let description = json_str_field(obj, "description").unwrap_or_default();
        items.push(CatalogItem {
            id,
            media_type,
            name,
            poster,
            release_info,
            imdb_rating,
            description,
        });
    }
    items
}

/// Fetches a Stremio v3 catalog from `base_url` (`/catalog/{type}/{id}.json`).
/// For default Cinemeta, tries the direct CDN host (`cinemeta-catalogs.strem.io`)
/// first to skip the 307 redirect, falling back to `v3-cinemeta.strem.io`.
pub fn fetch_catalog(
    window: &mut Window,
    base_url: &str,
    media_type: MediaType,
    catalog_id: &str,
) -> Result<Vec<CatalogItem>, String> {
    let base = base_url.trim_end_matches('/');
    let body = if base == CINEMETA_BASE {
        let direct_url = format!(
            "{}/{}/catalog/{}/{}.json",
            CINEMETA_CATALOGS_BASE,
            catalog_id,
            media_type.as_str(),
            catalog_id
        );
        sex_pdx::serial_record!("[brootlite.catalog] GET {}", direct_url);
        match http_get(window, &direct_url, MAX_CATALOG_BYTES) {
            Ok(b) => b,
            Err(_) => {
                let fallback_url = format!("{}/catalog/{}/{}.json", base, media_type.as_str(), catalog_id);
                sex_pdx::serial_record!("[brootlite.catalog] fallback GET {}", fallback_url);
                http_get(window, &fallback_url, MAX_CATALOG_BYTES)?
            }
        }
    } else {
        let url = format!("{}/catalog/{}/{}.json", base, media_type.as_str(), catalog_id);
        sex_pdx::serial_record!("[brootlite.catalog] GET {}", url);
        http_get(window, &url, MAX_CATALOG_BYTES)?
    };
    let text = core::str::from_utf8(&body).map_err(|_| String::from("invalid UTF-8 catalog"))?;
    let items = parse_catalog_json(text, media_type.as_str());
    sex_pdx::serial_record!("[brootlite.catalog] items={}", items.len());
    Ok(items)
}

/// Searches the public Cinemeta catalog for `query` across movies & series.
pub fn search_catalog(
    window: &mut Window,
    media_type: MediaType,
    query: &str,
) -> Result<Vec<CatalogItem>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let enc = percent_encode_query(trimmed);
    let primary_url = format!(
        "{}/catalog/{}/top/search={}.json",
        CINEMETA_BASE,
        media_type.as_str(),
        enc
    );
    sex_pdx::serial_record!("[brootlite.search] GET {}", primary_url);
    let body = match http_get(window, &primary_url, MAX_CATALOG_BYTES) {
        Ok(b) => b,
        Err(_) => {
            let fallback_url = format!(
                "{}/catalog/{}/top/search={}.json",
                CINEMETA_LIVE_BASE,
                media_type.as_str(),
                enc
            );
            sex_pdx::serial_record!("[brootlite.search] fallback GET {}", fallback_url);
            http_get(window, &fallback_url, MAX_CATALOG_BYTES)?
        }
    };
    let text = core::str::from_utf8(&body).map_err(|_| String::from("invalid UTF-8 search"))?;
    Ok(parse_catalog_json(text, media_type.as_str()))
}

/// Fetches streams for `(media_type, item_id)` across all configured Stremio stream addons
/// in `provider_pack.json`.
pub fn fetch_streams(
    window: &mut Window,
    pack: &ProviderPack,
    media_type: &str,
    item_id: &str,
    item_name: &str,
) -> (Vec<StreamItem>, String) {
    let mut out = Vec::new();
    let stream_addons: Vec<_> = pack
        .stremio_addons
        .iter()
        .filter(|a| a.supports_stream && !a.base_url().is_empty())
        .collect();
    if stream_addons.is_empty() {
        return (
            out,
            String::from(
                "No Stremio stream addons configured. Place provider_pack.json in /data/brootlite/.",
            ),
        );
    }

    let mut queried = 0usize;
    let mut errors = 0usize;
    let mut last_err = String::new();
    // For series without episode suffix, default to S1E1 so Stremio stream addons return results.
    let lookup_id = if media_type.eq_ignore_ascii_case("series") && !item_id.contains(':') {
        format!("{}:1:1", item_id)
    } else {
        String::from(item_id)
    };

    for addon in stream_addons.iter().take(6) {
        let base = addon.base_url();
        let url = format!("{}/stream/{}/{}.json", base, media_type, lookup_id);
        queried += 1;
        sex_pdx::serial_record!("[brootlite.stream] GET {}", url);
        match http_get(window, &url, MAX_STREAM_BYTES) {
            Ok(body) => {
                if let Ok(text) = core::str::from_utf8(&body) {
                    parse_streams_into(text, &addon.name, item_name, &pack.trackers, &mut out);
                }
            }
            Err(e) => {
                errors += 1;
                last_err = format!("{}: {}", addon.name, e);
            }
        }
    }

    let status = if !out.is_empty() {
        format!("Found {} streams from {} addon(s)", out.len(), queried)
    } else if errors == queried {
        format!("Addon error ({}) — click Retry or check +net", last_err)
    } else {
        format!("No streams returned by {} addon(s)", queried)
    };
    (out, status)
}

fn parse_streams_into(
    text: &str,
    addon_name: &str,
    item_name: &str,
    trackers: &[String],
    out: &mut Vec<StreamItem>,
) {
    let Some(arr) = json_array_slice(text, "streams") else {
        return;
    };
    for obj in json_objects(arr) {
        if out.len() >= 48 {
            break;
        }
        let name = json_str_field(obj, "name").unwrap_or_default();
        let title = json_str_field(obj, "title")
            .or_else(|| json_str_field(obj, "description"))
            .unwrap_or_default();
        let url = json_str_field(obj, "url")
            .or_else(|| json_str_field(obj, "externalUrl"))
            .unwrap_or_default();
        let info_hash = json_str_field(obj, "infoHash").unwrap_or_default();
        let file_idx = json_u32_field(obj, "fileIdx");

        let is_direct = url.starts_with("http://") || url.starts_with("https://");
        let is_magnet = url.starts_with("magnet:?") || !info_hash.is_empty();
        if !is_direct && !is_magnet {
            continue;
        }

        let magnet_uri = if url.starts_with("magnet:?") {
            url.clone()
        } else if !info_hash.is_empty() {
            build_magnet(&info_hash, item_name, trackers)
        } else {
            String::new()
        };

        let combined = format!("{} {}", name, title);
        let resolution = detect_resolution(&combined);
        let seeders = detect_seeders(&combined);
        let clean_title = title.replace('\n', " · ");
        let clean_name = name.replace('\n', " ");

        out.push(StreamItem {
            addon_name: String::from(addon_name),
            name: clean_name,
            title: clean_title,
            url,
            info_hash,
            file_idx,
            magnet_uri,
            resolution,
            seeders,
            is_direct,
        });
    }
}

fn build_magnet(info_hash: &str, display_name: &str, trackers: &[String]) -> String {
    let mut mag = format!(
        "magnet:?xt=urn:btih:{}&dn={}",
        info_hash.trim(),
        percent_encode_query(display_name)
    );
    for tr in trackers.iter().take(6) {
        mag.push_str("&tr=");
        mag.push_str(&percent_encode_query(tr));
    }
    mag
}

fn detect_resolution(s: &str) -> String {
    let up = s.to_ascii_uppercase();
    if up.contains("2160P") || up.contains("4K") || up.contains("UHD") {
        String::from("4K")
    } else if up.contains("1080P") || up.contains("FHD") {
        String::from("1080p")
    } else if up.contains("720P") {
        String::from("720p")
    } else if up.contains("480P") || up.contains("SD") {
        String::from("480p")
    } else {
        String::new()
    }
}

fn detect_seeders(s: &str) -> Option<u32> {
    // Common Stremio stream formatting: "👤 142", "S:142", "Seeds: 142", "seeders: 142".
    for marker in ["👤", "S:", "Seeds:", "seeds:", "Seeders:", "seeders:"] {
        if let Some(pos) = s.find(marker) {
            let after = s[pos + marker.len()..].trim_start();
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}
