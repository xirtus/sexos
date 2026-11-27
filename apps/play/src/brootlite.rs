// SPDX-License-Identifier: GPL-3.0-only
//! BrootLite-compatible `provider_pack.json` loader & Stremio v3 / Swarm metadata
//! for Qupid. Loads private user configuration from `/data/qupid/provider_pack.json`
//! or `/media/Ventoy/provider_pack.json` (zero private URLs bundled in the binary).

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
}

#[derive(Clone, Debug)]
pub struct SwarmIndexer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub search_url: String,
}

#[derive(Clone, Debug)]
pub struct FeaturedFeed {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderPack {
    pub name: String,
    pub source_path: String,
    pub stremio_addons: Vec<StremioAddon>,
    pub swarm_indexers: Vec<SwarmIndexer>,
    pub featured_feeds: Vec<FeaturedFeed>,
    pub tracker_count: usize,
}

const PACK_PATHS: [&[u8]; 4] = [
    b"/data/qupid/provider_pack.json",
    b"/media/Ventoy/provider_pack.json",
    b"provider_pack.json",
    b"/data/provider_pack.json",
];

impl ProviderPack {
    /// Loads the first readable `provider_pack.json` from `/data/qupid/` or Ventoy.
    pub fn load(window: &mut Window) -> Self {
        let mut buf = vec![0u8; 32 * 1024];
        for path in PACK_PATHS {
            if let Ok(n) = files::read(window, path, &mut buf) {
                if let Ok(text) = core::str::from_utf8(&buf[..n]) {
                    let mut pack = parse_pack(text);
                    pack.source_path = String::from_utf8_lossy(path).into_owned();
                    return pack;
                }
            }
        }
        Self::default()
    }
}

/// Extracts a quoted JSON string value immediately following `"key":`.
fn json_str_field(obj: &str, key: &str) -> Option<String> {
    let mut pat = String::with_capacity(key.len() + 2);
    pat.push('"');
    pat.push_str(key);
    pat.push('"');
    let idx = obj.find(&pat)?;
    let rest = &obj[idx + pat.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    if !after.starts_with('"') {
        return None;
    }
    let bytes = after.as_bytes();
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                _ => out.push(next as char),
            }
            i += 2;
            continue;
        }
        if b == b'"' {
            return Some(out);
        }
        out.push(b as char);
        i += 1;
    }
    None
}

/// Returns the slice inside `[ ... ]` for top-level `"array_key": [ ... ]`.
fn json_array_slice<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut pat = String::with_capacity(key.len() + 2);
    pat.push('"');
    pat.push_str(key);
    pat.push('"');
    let idx = text.find(&pat)?;
    let rest = &text[idx + pat.len()..];
    let open = rest.find('[')?;
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in open..bytes.len() {
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
                    return Some(&rest[open + 1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits top-level `{ ... }` objects inside a JSON array slice.
fn json_objects<'a>(arr: &'a str) -> Vec<&'a str> {
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

pub fn parse_pack(text: &str) -> ProviderPack {
    let mut pack = ProviderPack::default();
    if let Some(name) = json_str_field(text, "name") {
        pack.name = name;
    }
    if let Some(arr) = json_array_slice(text, "trackers") {
        pack.tracker_count = arr.split(',').filter(|s| s.contains("://")).count();
    }
    if let Some(arr) = json_array_slice(text, "stremioAddons") {
        for obj in json_objects(arr) {
            let id = json_str_field(obj, "id").unwrap_or_default();
            let name = json_str_field(obj, "name").unwrap_or_else(|| id.clone());
            let description = json_str_field(obj, "description").unwrap_or_default();
            let transport_url = json_str_field(obj, "transportUrl").unwrap_or_default();
            if !name.is_empty() && !transport_url.is_empty() {
                pack.stremio_addons.push(StremioAddon {
                    id,
                    name,
                    description,
                    transport_url,
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
    pack
}
