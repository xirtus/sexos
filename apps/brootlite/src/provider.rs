// SPDX-License-Identifier: GPL-3.0-only
//! BrootLite `provider_pack.json` loader & lightweight `no_std` JSON helpers.
//!
//! Zero private URLs or secrets are bundled in the binary. User-configured
//! Stremio addons, trackers, Swarm indexers, and featured feeds are loaded at
//! runtime from:
//!   1. `/data/brootlite/provider_pack.json`
//!   2. `/data/qupid/provider_pack.json`
//!   3. `/media/Ventoy/provider_pack.json`
//!   4. `provider_pack.json`
//!   5. `/data/provider_pack.json`

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, Window};

#[derive(Clone, Debug)]
pub struct StremioAddon {
    pub id: String,
    pub name: String,
    pub description: String,
    pub transport_url: String,
    /// True if addon manifest/config hints it can provide catalogs.
    pub supports_catalog: bool,
    /// True if addon manifest/config hints it can provide streams (default true).
    pub supports_stream: bool,
}

impl StremioAddon {
    /// Base URL for Stremio v3 endpoints (strips trailing `/manifest.json` or `/`).
    pub fn base_url(&self) -> &str {
        let u = self.transport_url.trim();
        let u = u.strip_suffix("/manifest.json").unwrap_or(u);
        u.trim_end_matches('/')
    }
}

#[derive(Clone, Debug)]
pub struct SwarmIndexer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub search_url: String,
}

impl SwarmIndexer {
    /// Substitutes `{query}` / `%s` in the indexer's search URL template.
    pub fn build_query_url(&self, query: &str) -> String {
        let enc = percent_encode_query(query);
        if self.search_url.contains("{query}") {
            self.search_url.replace("{query}", &enc)
        } else if self.search_url.contains("%s") {
            self.search_url.replace("%s", &enc)
        } else if self.search_url.is_empty() {
            String::new()
        } else {
            let mut out = self.search_url.clone();
            if !out.contains('?') {
                out.push_str("?q=");
            }
            out.push_str(&enc);
            out
        }
    }
}

#[derive(Clone, Debug)]
pub struct FeaturedFeed {
    pub label: String,
    pub url: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DirectSource {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub api_url: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct KodiRepoEntry {
    pub id: String,
    pub name: String,
    pub addons_xml_url: String,
    pub datadir_url: String,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderPack {
    pub name: String,
    pub source_path: String,
    pub stremio_addons: Vec<StremioAddon>,
    pub swarm_indexers: Vec<SwarmIndexer>,
    pub featured_feeds: Vec<FeaturedFeed>,
    pub direct_sources: Vec<DirectSource>,
    pub kodi_repos: Vec<KodiRepoEntry>,
    pub trackers: Vec<String>,
}

/// Search order under SexOS file confinement:
/// 1. `provider_pack.json` — resolves directly to `/data/brootlite/provider_pack.json`
///    inside BrootLite's own confined data folder (no `@` grant required).
/// 2. `@/...` — resolves to explicit granted paths when launched with `@/data/qupid`, etc.
const PACK_PATHS: [&[u8]; 6] = [
    b"provider_pack.json",
    b"@/data/brootlite/provider_pack.json",
    b"@/data/qupid/provider_pack.json",
    b"@/media/Ventoy/provider_pack.json",
    b"@/data/provider_pack.json",
    b"/data/brootlite/provider_pack.json",
];

impl ProviderPack {
    /// Loads the first readable `provider_pack.json` from BrootLite's data folder or granted paths.
    pub fn load(window: &mut Window) -> Self {
        let mut buf = vec![0u8; 128 * 1024];
        for path in PACK_PATHS {
            if let Ok(n) = files::read(window, path, &mut buf) {
                if n > 0 {
                    if let Ok(text) = core::str::from_utf8(&buf[..n]) {
                        let mut pack = parse_pack(text);
                        pack.source_path = if path == b"provider_pack.json" {
                            String::from("/data/brootlite/provider_pack.json")
                        } else {
                            String::from_utf8_lossy(path)
                                .trim_start_matches('@')
                                .into()
                        };
                        sex_pdx::serial_record!(
                            "[brootlite.pack] loaded={} addons={} indexers={} feeds={} direct={} kodi={} trackers={}",
                            pack.source_path,
                            pack.stremio_addons.len(),
                            pack.swarm_indexers.len(),
                            pack.featured_feeds.len(),
                            pack.direct_sources.len(),
                            pack.kodi_repos.len(),
                            pack.trackers.len()
                        );
                        return pack;
                    }
                }
            }
        }
        Self::default()
    }

    pub fn tracker_count(&self) -> usize {
        self.trackers.len()
    }
}

/// Percent-encodes a search query for URL query parameters or Stremio search paths.
pub fn percent_encode_query(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len() + 8);
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xF) as usize] as char);
        }
    }
    out
}

/// Extracts a quoted JSON string value following `"key":`.
pub fn json_str_field(obj: &str, key: &str) -> Option<String> {
    let mut pat = String::with_capacity(key.len() + 2);
    pat.push('"');
    pat.push_str(key);
    pat.push('"');
    let mut search = obj;
    while let Some(idx) = search.find(&pat) {
        let rest = &search[idx + pat.len()..];
        let trimmed = rest.trim_start();
        if let Some(after_colon) = trimmed.strip_prefix(':') {
            let after = after_colon.trim_start();
            if after.starts_with('"') {
                return parse_json_string(after);
            }
            // Handle numeric values converted to string (e.g., `"imdbRating": 8.4` or `"year": 2024`).
            if after.as_bytes().first().is_some_and(|b| b.is_ascii_digit() || *b == b'-') {
                let end = after
                    .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                    .unwrap_or(after.len());
                if end > 0 {
                    return Some(String::from(&after[..end]));
                }
            }
            return None;
        }
        search = &search[idx + pat.len()..];
    }
    None
}

/// Extracts an unsigned integer JSON field (e.g. `"fileIdx": 0`).
pub fn json_u32_field(obj: &str, key: &str) -> Option<u32> {
    let s = json_str_field(obj, key)?;
    s.trim().parse::<u32>().ok()
}

/// Parses a JSON string literal starting at the opening `"`.
pub fn parse_json_string(s: &str) -> Option<String> {
    if !s.starts_with('"') {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 1usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'"' => {
                    out.push('"');
                    i += 2;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                }
                b'/' => {
                    out.push('/');
                    i += 2;
                }
                b'n' => {
                    out.push('\n');
                    i += 2;
                }
                b'r' => {
                    i += 2;
                }
                b't' => {
                    out.push('\t');
                    i += 2;
                }
                b'u' if i + 6 <= bytes.len() => {
                    if let Ok(hex_str) = core::str::from_utf8(&bytes[i + 2..i + 6]) {
                        if let Ok(code) = u32::from_str_radix(hex_str, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                    }
                    i += 6;
                }
                _ => {
                    out.push(next as char);
                    i += 2;
                }
            }
            continue;
        }
        if b == b'"' {
            return Some(out);
        }
        // Preserve valid UTF-8 sequences cleanly.
        if b < 0x80 {
            out.push(b as char);
            i += 1;
        } else {
            let rem = &s[i..];
            if let Some(ch) = rem.chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        }
    }
    None
}

/// Returns the inner slice of `[ ... ]` for `"array_key": [ ... ]`.
pub fn json_array_slice<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut pat = String::with_capacity(key.len() + 2);
    pat.push('"');
    pat.push_str(key);
    pat.push('"');
    let mut search = text;
    while let Some(idx) = search.find(&pat) {
        let rest = &search[idx + pat.len()..];
        let trimmed = rest.trim_start();
        if let Some(after_colon) = trimmed.strip_prefix(':') {
            let after = after_colon.trim_start();
            if !after.starts_with('[') {
                return None;
            }
            let bytes = after.as_bytes();
            let mut depth = 0i32;
            let mut in_str = false;
            let mut esc = false;
            for i in 0..bytes.len() {
                let b = bytes[i];
                if in_str {
                    if esc {
                        esc = false;
                    } else if b == b'\\' {
                        esc = true;
                    } else if b == b'"' {
                        in_str = false;
                    }
                    continue;
                }
                match b {
                    b'"' => in_str = true,
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&after[1..i]);
                        }
                    }
                    _ => {}
                }
            }
            return None;
        }
        search = &search[idx + pat.len()..];
    }
    None
}

/// Splits top-level `{ ... }` objects inside a JSON array slice.
pub fn json_objects<'a>(arr: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let bytes = arr.as_bytes();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for i in 0..bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    out.push(&arr[start..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Parses a JSON array of strings (e.g., `"trackers": ["udp://...", "https://..."]`).
fn json_string_list(arr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = arr.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if let Some(val) = parse_json_string(&arr[i..]) {
                out.push(val);
            }
            // Advance past closing quote.
            i += 1;
            let mut esc = false;
            while i < bytes.len() {
                if esc {
                    esc = false;
                } else if bytes[i] == b'\\' {
                    esc = true;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

pub fn parse_pack(text: &str) -> ProviderPack {
    let mut pack = ProviderPack::default();
    if let Some(name) = json_str_field(text, "name") {
        pack.name = name;
    }
    if let Some(arr) = json_array_slice(text, "trackers") {
        for t in json_string_list(arr) {
            if t.contains("://") {
                pack.trackers.push(t);
            }
        }
    }
    if let Some(arr) = json_array_slice(text, "stremioAddons") {
        for obj in json_objects(arr) {
            let id = json_str_field(obj, "id").unwrap_or_default();
            let name = json_str_field(obj, "name").unwrap_or_else(|| id.clone());
            let description = json_str_field(obj, "description").unwrap_or_default();
            let transport_url = json_str_field(obj, "transportUrl").unwrap_or_default();
            let resources = json_array_slice(obj, "resources").unwrap_or("");
            let supports_catalog = resources.contains("catalog")
                || json_array_slice(obj, "catalogs").is_some_and(|c| !c.trim().is_empty());
            let supports_stream = resources.is_empty() || resources.contains("stream");
            if !name.is_empty() && !transport_url.is_empty() {
                pack.stremio_addons.push(StremioAddon {
                    id,
                    name,
                    description,
                    transport_url,
                    supports_catalog,
                    supports_stream,
                });
            }
        }
    }
    if let Some(arr) = json_array_slice(text, "swarmIndexers") {
        for obj in json_objects(arr) {
            let id = json_str_field(obj, "id").unwrap_or_default();
            let name = json_str_field(obj, "name").unwrap_or_else(|| id.clone());
            let kind = json_str_field(obj, "kind").unwrap_or_default();
            let search_url = json_str_field(obj, "searchUrlTemplate")
                .or_else(|| json_str_field(obj, "pathTemplate"))
                .unwrap_or_default();
            if !name.is_empty() {
                pack.swarm_indexers.push(SwarmIndexer {
                    id,
                    name,
                    kind,
                    search_url,
                });
            }
        }
    }
    if let Some(arr) = json_array_slice(text, "featuredFeeds") {
        for obj in json_objects(arr) {
            let label = json_str_field(obj, "label").unwrap_or_default();
            let url = json_str_field(obj, "url").unwrap_or_default();
            if !label.is_empty() && !url.is_empty() {
                pack.featured_feeds.push(FeaturedFeed { label, url });
            }
        }
    }
    if let Some(arr) = json_array_slice(text, "directSources") {
        for obj in json_objects(arr) {
            let id = json_str_field(obj, "id").unwrap_or_default();
            let name = json_str_field(obj, "name").unwrap_or_else(|| id.clone());
            let kind = json_str_field(obj, "kind").unwrap_or_default();
            let api_url = json_str_field(obj, "apiUrl").unwrap_or_default();
            if !name.is_empty() && !api_url.is_empty() {
                pack.direct_sources.push(DirectSource {
                    id,
                    name,
                    kind,
                    api_url,
                });
            }
        }
    }
    if let Some(arr) = json_array_slice(text, "kodiRepositories") {
        for obj in json_objects(arr) {
            let id = json_str_field(obj, "id").unwrap_or_default();
            let name = json_str_field(obj, "name").unwrap_or_else(|| id.clone());
            let addons_xml_url = json_str_field(obj, "addonsXmlUrl").unwrap_or_default();
            let datadir_url = json_str_field(obj, "datadirUrl").unwrap_or_default();
            if !name.is_empty() && !addons_xml_url.is_empty() {
                pack.kodi_repos.push(KodiRepoEntry {
                    id,
                    name,
                    addons_xml_url,
                    datadir_url,
                });
            }
        }
    }
    pack
}
