// SPDX-License-Identifier: GPL-3.0-only
//! External provider packs and clean open catalog defaults for Qupid (`#!(no_std)`, zero
//! dependencies).
//!
//! Public SexOS trees ship only clean, open sources out of the box ([`ProviderPack::public_defaults`]:
//! Cinemeta metadata, Internet Archive open media, Academic Torrents, FOSS announce
//! trackers and the official Kodi Omega add-on mirror). Users who want extra Stremio
//! addons, RSS/JSON swarm indexers, featured feeds or third-party Kodi repositories can
//! either:
//! - bake a gitignored `crates/sex-episodes/qupid_seed_pack.json` into local builds via
//!   `scripts/qupid_pack.sh apply <pack.json>`, or
//! - grant Qupid a `provider-pack.json` capability at runtime (`/cfg/qupid/provider-pack.json`)
//!   and merge it with [`QupidCatalogConfig::apply_pack_json`].

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(qupid_seed_pack)]
const BAKED_SEED_PACK: Option<&str> = Some(include_str!("../qupid_seed_pack.json"));
#[cfg(not(qupid_seed_pack))]
const BAKED_SEED_PACK: Option<&str> = None;

/// Error returned when a `provider-pack.json` document is malformed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    Syntax { byte_offset: usize },
    ExpectedObject,
    DepthExceeded,
}

/// Wire format spoken by a swarm indexer or featured feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwarmFeedKind {
    /// RSS 2.0 / Atom XML with `<enclosure>`, `<link>magnet:...`, or `<*:infoHash>`.
    Rss,
    /// JSON array of `{ name, info_hash, seeders, leechers, size }` objects.
    JsonArrayBtih,
    /// JSON `{ data: { movies: [{ title, year, torrents: [{ hash, quality, seeds, size_bytes }] }] } }`.
    JsonMoviesTorrents,
}

impl SwarmFeedKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SwarmFeedKind::Rss => "rss",
            SwarmFeedKind::JsonArrayBtih => "json_array_btih",
            SwarmFeedKind::JsonMoviesTorrents => "json_movies_torrents",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        if raw.eq_ignore_ascii_case("rss") {
            Some(SwarmFeedKind::Rss)
        } else if raw.eq_ignore_ascii_case("json_array_btih") {
            Some(SwarmFeedKind::JsonArrayBtih)
        } else if raw.eq_ignore_ascii_case("json_movies_torrents") {
            Some(SwarmFeedKind::JsonMoviesTorrents)
        } else {
            None
        }
    }
}

/// A Stremio v3 addon manifest descriptor (`manifest.json` endpoint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StremioAddonEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub transport_url: String,
    pub types: Vec<String>,
    pub resources: Vec<String>,
    pub id_prefixes: Vec<String>,
}

impl StremioAddonEntry {
    /// Base URL without `/manifest.json` nor trailing slash, ready for `/stream/{type}/{id}.json`.
    pub fn base_url(&self) -> &str {
        let trimmed = self.transport_url.trim_end_matches('/');
        trimmed.strip_suffix("/manifest.json").unwrap_or(trimmed)
    }

    pub fn supports_resource(&self, resource: &str) -> bool {
        self.resources.is_empty() || self.resources.iter().any(|r| r.eq_ignore_ascii_case(resource))
    }

    pub fn supports_id(&self, content_type: &str, item_id: &str) -> bool {
        let type_ok = self.types.is_empty() || self.types.iter().any(|t| t.eq_ignore_ascii_case(content_type));
        let prefix_ok = self.id_prefixes.is_empty() || self.id_prefixes.iter().any(|p| item_id.starts_with(p.as_str()));
        type_ok && prefix_ok
    }

    /// Builds `{base}/stream/{content_type}/{encoded_id}.json`.
    pub fn stream_url(&self, content_type: &str, video_id: &str) -> String {
        format!(
            "{}/stream/{}/{}.json",
            self.base_url(),
            url_encode_segment(content_type),
            url_encode_segment(video_id)
        )
    }
}

/// Direct P2P swarm indexer template hydrated from a `ProviderPack`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwarmIndexerEntry {
    pub id: String,
    pub name: String,
    pub kind: SwarmFeedKind,
    pub search_url_template: String,
    pub imdb_url_template: String,
    pub base_urls: Vec<String>,
    pub path_template: String,
    pub default_category: String,
    pub tab_group: String,
    pub adult: bool,
    pub imdb_fallback: bool,
}

impl SwarmIndexerEntry {
    /// Candidate URLs for a text query (multiple entries when mirror `base_urls` are configured).
    pub fn build_search_urls(&self, query: &str, category_override: Option<&str>) -> Vec<String> {
        let q = url_encode_query(query.trim());
        let cat = url_encode_query(category_override.unwrap_or(self.default_category.as_str()));
        let mut out = Vec::new();
        if !self.search_url_template.is_empty() {
            out.push(expand_template(&self.search_url_template, &q, &cat, ""));
        }
        if !self.path_template.is_empty() {
            let path = expand_template(&self.path_template, &q, &cat, "");
            for base in &self.base_urls {
                let b = base.trim_end_matches('/');
                if !b.is_empty() {
                    out.push(format!("{b}{path}"));
                }
            }
        }
        out
    }

    /// Candidate URLs for an IMDb `tt...` lookup when `imdb_fallback` is enabled.
    pub fn build_imdb_urls(&self, imdb_id: &str) -> Vec<String> {
        if !self.imdb_fallback || !imdb_id.starts_with("tt") {
            return Vec::new();
        }
        let tt = url_encode_query(imdb_id.trim());
        let cat = url_encode_query(&self.default_category);
        let mut out = Vec::new();
        if !self.imdb_url_template.is_empty() {
            out.push(expand_template(&self.imdb_url_template, &tt, &cat, &tt));
        } else if !self.path_template.is_empty() {
            let path = expand_template(&self.path_template, &tt, &cat, &tt);
            for base in &self.base_urls {
                let b = base.trim_end_matches('/');
                if !b.is_empty() {
                    out.push(format!("{b}{path}"));
                }
            }
        }
        out
    }
}

/// Pre-configured browse/category swarm feed pill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeaturedFeedEntry {
    pub key: String,
    pub label: String,
    pub kind: SwarmFeedKind,
    pub url: String,
    pub base_urls: Vec<String>,
    pub path: String,
    pub adult: bool,
}

impl FeaturedFeedEntry {
    pub fn candidate_urls(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.url.is_empty() {
            out.push(self.url.clone());
        }
        if !self.path.is_empty() {
            for base in &self.base_urls {
                let b = base.trim_end_matches('/');
                if !b.is_empty() {
                    out.push(format!("{b}{}", self.path));
                }
            }
        }
        out
    }
}

/// Kodi 21 (Omega) repository bootstrap entry for `sex-kodi`.
///
/// Supports both zip-bootstrapped repositories (`zip_url` unpacked by `sex_kodi::package`)
/// and direct `<dir>` mirrors (`addons_xml_url` + `checksum_url` + `datadir_url`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KodiRepositoryEntry {
    pub id: String,
    pub name: String,
    pub zip_url: String,
    pub addons_xml_url: String,
    pub checksum_url: String,
    pub datadir_url: String,
    pub install_addons: Vec<String>,
}

impl KodiRepositoryEntry {
    pub fn has_direct_dir(&self) -> bool {
        !self.addons_xml_url.is_empty() && !self.datadir_url.is_empty()
    }
}

/// Parsed external or built-in Qupid `provider-pack.json` bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderPack {
    pub schema_version: u32,
    pub name: String,
    pub trackers: Vec<String>,
    pub stremio_addons: Vec<StremioAddonEntry>,
    pub swarm_indexers: Vec<SwarmIndexerEntry>,
    pub featured_feeds: Vec<FeaturedFeedEntry>,
    pub kodi_repositories: Vec<KodiRepositoryEntry>,
}

impl ProviderPack {
    /// Clean open-access sources shipped in public SexOS builds.
    pub fn public_defaults() -> Self {
        Self {
            schema_version: 1,
            name: String::from("Qupid Open Media Defaults"),
            trackers: vec![
                String::from("udp://tracker.opentrackr.org:1337/announce"),
                String::from("udp://open.stealth.si:80/announce"),
                String::from("udp://tracker.torrent.eu.org:451/announce"),
            ],
            stremio_addons: vec![StremioAddonEntry {
                id: String::from("com.linvo.cinemeta"),
                name: String::from("Cinemeta"),
                version: String::from("3.0.13"),
                description: String::from("Official open movie and series metadata catalogue"),
                transport_url: String::from("https://v3-cinemeta.strem.io/manifest.json"),
                types: vec![String::from("movie"), String::from("series")],
                resources: vec![String::from("catalog"), String::from("meta")],
                id_prefixes: vec![String::from("tt")],
            }],
            swarm_indexers: vec![
                SwarmIndexerEntry {
                    id: String::from("internet-archive"),
                    name: String::from("Internet Archive"),
                    kind: SwarmFeedKind::Rss,
                    search_url_template: String::from(
                        "https://archive.org/advancedsearch.php?q={query}+AND+mediatype%3Amovies+AND+format%3A%22Archive+BitTorrent%22&fl[]=identifier,title&rows=30&output=rss",
                    ),
                    imdb_url_template: String::new(),
                    base_urls: Vec::new(),
                    path_template: String::new(),
                    default_category: String::from("movies"),
                    tab_group: String::from("open_archive"),
                    adult: false,
                    imdb_fallback: false,
                },
                SwarmIndexerEntry {
                    id: String::from("academic-torrents"),
                    name: String::from("Academic Torrents"),
                    kind: SwarmFeedKind::Rss,
                    search_url_template: String::from("https://academictorrents.com/browse.php?search={query}&rss=1"),
                    imdb_url_template: String::new(),
                    base_urls: Vec::new(),
                    path_template: String::new(),
                    default_category: String::from("all"),
                    tab_group: String::from("open_archive"),
                    adult: false,
                    imdb_fallback: false,
                },
            ],
            featured_feeds: vec![FeaturedFeedEntry {
                key: String::from("archive_feature_films"),
                label: String::from("Internet Archive · Public Domain Feature Films"),
                kind: SwarmFeedKind::Rss,
                url: String::from(
                    "https://archive.org/advancedsearch.php?q=collection%3Afeature_films+AND+format%3A%22Archive+BitTorrent%22&sort[]=downloads+desc&rows=30&output=rss",
                ),
                base_urls: Vec::new(),
                path: String::new(),
                adult: false,
            }],
            kodi_repositories: vec![KodiRepositoryEntry {
                id: String::from("repository.xbmc.org"),
                name: String::from("Official Kodi Add-on Repository (Omega)"),
                zip_url: String::new(),
                addons_xml_url: String::from("https://mirrors.kodi.tv/addons/omega/addons.xml.gz"),
                checksum_url: String::from("https://mirrors.kodi.tv/addons/omega/addons.xml.gz.sha256"),
                datadir_url: String::from("https://mirrors.kodi.tv/addons/omega/"),
                install_addons: Vec::new(),
            }],
        }
    }

    /// True when local build included a gitignored `qupid_seed_pack.json` at compile time.
    pub fn has_baked_seed() -> bool {
        BAKED_SEED_PACK.is_some()
    }

    /// Parses a `provider-pack.json` string (compatible with brootlite's external pack).
    pub fn from_json(raw: &str) -> Result<Self, PackError> {
        let root = json::parse(raw)?;
        let Some(obj) = root.as_obj() else {
            return Err(PackError::ExpectedObject);
        };
        let schema_version = json::get_u64(obj, "schemaVersion").unwrap_or(1) as u32;
        let name = json::get_str(obj, "name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Custom Provider Pack")
            .to_owned();

        let trackers = json::get_str_vec(obj, "trackers")
            .into_iter()
            .filter(|u| is_safe_tracker_url(u))
            .collect();

        let mut stremio_addons = Vec::new();
        if let Some(arr) = json::get_arr(obj, "stremioAddons") {
            for item in arr {
                let Some(a) = item.as_obj() else { continue };
                let transport_url = json::get_str(a, "transportUrl").unwrap_or("").trim().to_owned();
                if !is_http_url(&transport_url) {
                    continue;
                }
                let id = json::get_str(a, "id").unwrap_or("").trim().to_owned();
                if id.is_empty() {
                    continue;
                }
                stremio_addons.push(StremioAddonEntry {
                    id,
                    name: json::get_str(a, "name").unwrap_or("Addon").to_owned(),
                    version: json::get_str(a, "version").unwrap_or("1.0.0").to_owned(),
                    description: json::get_str(a, "description").unwrap_or("").to_owned(),
                    transport_url,
                    types: json::get_str_vec(a, "types"),
                    resources: json::get_str_vec(a, "resources"),
                    id_prefixes: json::get_str_vec(a, "idPrefixes"),
                });
            }
        }

        let mut swarm_indexers = Vec::new();
        if let Some(arr) = json::get_arr(obj, "swarmIndexers") {
            for item in arr {
                let Some(ix) = item.as_obj() else { continue };
                let id = json::get_str(ix, "id").unwrap_or("").trim().to_owned();
                let Some(kind) = json::get_str(ix, "kind").and_then(SwarmFeedKind::parse) else { continue };
                if id.is_empty() {
                    continue;
                }
                let search_url_template = json::get_str(ix, "searchUrlTemplate").unwrap_or("").trim().to_owned();
                let base_urls: Vec<String> = json::get_str_vec(ix, "baseUrls")
                    .into_iter()
                    .filter(|u| is_http_url(u))
                    .collect();
                if !search_url_template.is_empty() && !is_http_url(&search_url_template) {
                    continue;
                }
                if search_url_template.is_empty() && base_urls.is_empty() {
                    continue;
                }
                swarm_indexers.push(SwarmIndexerEntry {
                    id,
                    name: json::get_str(ix, "name").unwrap_or("Indexer").to_owned(),
                    kind,
                    search_url_template,
                    imdb_url_template: json::get_str(ix, "imdbUrlTemplate").unwrap_or("").trim().to_owned(),
                    base_urls,
                    path_template: json::get_str(ix, "pathTemplate").unwrap_or("").trim().to_owned(),
                    default_category: json::get_str(ix, "defaultCategory").unwrap_or("0_0").to_owned(),
                    tab_group: json::get_str(ix, "tabGroup").unwrap_or("general").to_owned(),
                    adult: json::get_bool(ix, "adult").unwrap_or(false),
                    imdb_fallback: json::get_bool(ix, "imdbFallback").unwrap_or(false),
                });
            }
        }

        let mut featured_feeds = Vec::new();
        if let Some(arr) = json::get_arr(obj, "featuredFeeds") {
            for item in arr {
                let Some(f) = item.as_obj() else { continue };
                let key = json::get_str(f, "key").unwrap_or("").trim().to_owned();
                let Some(kind) = json::get_str(f, "kind").and_then(SwarmFeedKind::parse) else { continue };
                if key.is_empty() {
                    continue;
                }
                let url = json::get_str(f, "url").unwrap_or("").trim().to_owned();
                let base_urls: Vec<String> = json::get_str_vec(f, "baseUrls")
                    .into_iter()
                    .filter(|u| is_http_url(u))
                    .collect();
                if !url.is_empty() && !is_http_url(&url) {
                    continue;
                }
                if url.is_empty() && base_urls.is_empty() {
                    continue;
                }
                featured_feeds.push(FeaturedFeedEntry {
                    key,
                    label: json::get_str(f, "label").unwrap_or("Featured").to_owned(),
                    kind,
                    url,
                    base_urls,
                    path: json::get_str(f, "path").unwrap_or("").trim().to_owned(),
                    adult: json::get_bool(f, "adult").unwrap_or(false),
                });
            }
        }

        let mut kodi_repositories = Vec::new();
        if let Some(arr) = json::get_arr(obj, "kodiRepositories") {
            for item in arr {
                let Some(r) = item.as_obj() else { continue };
                let id = json::get_str(r, "id").unwrap_or("").trim().to_owned();
                if id.is_empty() {
                    continue;
                }
                let zip_url = json::get_str(r, "zipUrl").unwrap_or("").trim().to_owned();
                let addons_xml_url = json::get_str(r, "addonsXmlUrl").unwrap_or("").trim().to_owned();
                if zip_url.is_empty() && addons_xml_url.is_empty() {
                    continue;
                }
                if (!zip_url.is_empty() && !is_http_url(&zip_url))
                    || (!addons_xml_url.is_empty() && !is_http_url(&addons_xml_url))
                {
                    continue;
                }
                kodi_repositories.push(KodiRepositoryEntry {
                    id,
                    name: json::get_str(r, "name").unwrap_or("Kodi Repository").to_owned(),
                    zip_url,
                    addons_xml_url,
                    checksum_url: json::get_str(r, "checksumUrl").unwrap_or("").trim().to_owned(),
                    datadir_url: json::get_str(r, "datadirUrl").unwrap_or("").trim().to_owned(),
                    install_addons: json::get_str_vec(r, "installAddons"),
                });
            }
        }

        Ok(Self {
            schema_version,
            name,
            trackers,
            stremio_addons,
            swarm_indexers,
            featured_feeds,
            kodi_repositories,
        })
    }
}

/// Active Qupid catalogue, swarm indexer and Kodi repository state after merging defaults
/// with any compile-time seed pack and runtime capability-granted packs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QupidCatalogConfig {
    pub active_pack_name: String,
    pub show_adult_sources: bool,
    pub trackers: Vec<String>,
    pub stremio_addons: Vec<StremioAddonEntry>,
    pub swarm_indexers: Vec<SwarmIndexerEntry>,
    pub featured_feeds: Vec<FeaturedFeedEntry>,
    pub kodi_repositories: Vec<KodiRepositoryEntry>,
}

impl Default for QupidCatalogConfig {
    fn default() -> Self {
        Self::load_default()
    }
}

impl QupidCatalogConfig {
    /// Starts with [`ProviderPack::public_defaults`] and overlays any baked `qupid_seed_pack.json`.
    pub fn load_default() -> Self {
        let base = ProviderPack::public_defaults();
        let mut cfg = Self {
            active_pack_name: base.name.clone(),
            show_adult_sources: false,
            trackers: base.trackers,
            stremio_addons: base.stremio_addons,
            swarm_indexers: base.swarm_indexers,
            featured_feeds: base.featured_feeds,
            kodi_repositories: base.kodi_repositories,
        };
        if let Some(seed_json) = BAKED_SEED_PACK {
            if let Ok(seed) = ProviderPack::from_json(seed_json) {
                cfg.merge_pack(seed);
            }
        }
        cfg
    }

    /// Parses and merges an external `provider-pack.json` idempotently.
    pub fn apply_pack_json(&mut self, raw_json: &str) -> Result<(), PackError> {
        let pack = ProviderPack::from_json(raw_json)?;
        self.merge_pack(pack);
        Ok(())
    }

    /// Merges `pack` without duplicating existing IDs or tracker announce URLs.
    /// Packed trackers and stream addons are prioritised ahead of baseline defaults.
    pub fn merge_pack(&mut self, pack: ProviderPack) {
        if !pack.name.is_empty() {
            self.active_pack_name = pack.name;
        }
        if pack.swarm_indexers.iter().any(|i| i.adult) || pack.featured_feeds.iter().any(|f| f.adult) {
            self.show_adult_sources = true;
        }

        let mut merged_trackers = Vec::new();
        for tr in pack.trackers.into_iter().chain(self.trackers.drain(..)) {
            if !merged_trackers.iter().any(|e: &String| e.eq_ignore_ascii_case(&tr)) {
                merged_trackers.push(tr);
            }
        }
        self.trackers = merged_trackers;

        for addon in pack.stremio_addons {
            if let Some(existing) = self
                .stremio_addons
                .iter_mut()
                .find(|a| a.id == addon.id || a.transport_url == addon.transport_url)
            {
                *existing = addon;
            } else {
                self.stremio_addons.push(addon);
            }
        }

        for idx in pack.swarm_indexers {
            if let Some(existing) = self.swarm_indexers.iter_mut().find(|i| i.id == idx.id) {
                *existing = idx;
            } else {
                self.swarm_indexers.push(idx);
            }
        }

        for feed in pack.featured_feeds {
            if let Some(existing) = self.featured_feeds.iter_mut().find(|f| f.key == feed.key) {
                *existing = feed;
            } else {
                self.featured_feeds.push(feed);
            }
        }

        for repo in pack.kodi_repositories {
            if let Some(existing) = self.kodi_repositories.iter_mut().find(|r| r.id == repo.id) {
                *existing = repo;
            } else {
                self.kodi_repositories.push(repo);
            }
        }
    }

    pub fn visible_indexers(&self) -> Vec<&SwarmIndexerEntry> {
        self.swarm_indexers
            .iter()
            .filter(|i| self.show_adult_sources || !i.adult)
            .collect()
    }

    pub fn visible_featured_feeds(&self) -> Vec<&FeaturedFeedEntry> {
        self.featured_feeds
            .iter()
            .filter(|f| self.show_adult_sources || !f.adult)
            .collect()
    }
}

fn is_http_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

fn is_safe_tracker_url(s: &str) -> bool {
    s.starts_with("udp://")
        || s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("wss://")
        || s.starts_with("ws://")
}

fn expand_template(tpl: &str, query: &str, cat: &str, imdb: &str) -> String {
    tpl.replace("{query}", query)
        .replace("{cat}", cat)
        .replace("{imdb}", imdb)
}

/// Percent-encodes a URL query parameter (`RFC 3986` unreserved set).
pub fn url_encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(hex_upper(b >> 4));
            out.push(hex_upper(b & 0x0f));
        }
    }
    out
}

/// Percent-encodes a path segment (preserving `:` only when safe? No: encodes `:` as `%3A`
/// so Stremio series IDs like `tt1234567:1:2` resolve cleanly on strict CDNs).
pub fn url_encode_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_upper(b >> 4));
            out.push(hex_upper(b & 0x0f));
        }
    }
    out
}

#[inline]
fn hex_upper(nibble: u8) -> char {
    match nibble & 0x0f {
        0..=9 => (b'0' + (nibble & 0x0f)) as char,
        n => (b'A' + (n - 10)) as char,
    }
}

/// Small, bounded `#![no_std]` RFC-8259 JSON reader shared with `streams.rs`.
pub(crate) mod json {
    use alloc::string::String;
    use alloc::vec::Vec;
    use super::PackError;

    const MAX_DEPTH: usize = 20;

    #[derive(Clone, Debug, PartialEq)]
    pub enum Val {
        Null,
        Bool(bool),
        Num(i64, Option<u64>),
        Str(String),
        Arr(Vec<Val>),
        Obj(Vec<(String, Val)>),
    }

    impl Val {
        pub fn as_obj(&self) -> Option<&[(String, Val)]> {
            match self {
                Val::Obj(entries) => Some(entries),
                _ => None,
            }
        }

        pub fn as_arr(&self) -> Option<&[Val]> {
            match self {
                Val::Arr(items) => Some(items),
                _ => None,
            }
        }

        pub fn as_str(&self) -> Option<&str> {
            match self {
                Val::Str(s) => Some(s),
                _ => None,
            }
        }

        pub fn as_u64(&self) -> Option<u64> {
            match self {
                Val::Num(_, Some(u)) => Some(*u),
                Val::Num(i, None) if *i >= 0 => Some(*i as u64),
                Val::Str(s) => s.trim().parse::<u64>().ok(),
                _ => None,
            }
        }

        pub fn as_bool(&self) -> Option<bool> {
            match self {
                Val::Bool(b) => Some(*b),
                _ => None,
            }
        }
    }

    pub fn get<'a>(obj: &'a [(String, Val)], key: &str) -> Option<&'a Val> {
        obj.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_str<'a>(obj: &'a [(String, Val)], key: &str) -> Option<&'a str> {
        get(obj, key).and_then(Val::as_str)
    }

    pub fn get_u64(obj: &[(String, Val)], key: &str) -> Option<u64> {
        get(obj, key).and_then(Val::as_u64)
    }

    pub fn get_bool(obj: &[(String, Val)], key: &str) -> Option<bool> {
        get(obj, key).and_then(Val::as_bool)
    }

    pub fn get_arr<'a>(obj: &'a [(String, Val)], key: &str) -> Option<&'a [Val]> {
        get(obj, key).and_then(Val::as_arr)
    }

    pub fn get_obj<'a>(obj: &'a [(String, Val)], key: &str) -> Option<&'a [(String, Val)]> {
        get(obj, key).and_then(Val::as_obj)
    }

    pub fn get_str_vec(obj: &[(String, Val)], key: &str) -> Vec<String> {
        get_arr(obj, key)
            .map(|arr| {
                arr.iter()
                    .filter_map(Val::as_str)
                    .map(|s| String::from(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn parse(src: &str) -> Result<Val, PackError> {
        let bytes = src.as_bytes();
        // Strip UTF-8 BOM if present.
        let mut pos = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        let v = parse_val(bytes, &mut pos, 0)?;
        skip_ws(bytes, &mut pos);
        if pos != bytes.len() {
            return Err(PackError::Syntax { byte_offset: pos });
        }
        Ok(v)
    }

    fn skip_ws(b: &[u8], pos: &mut usize) {
        while *pos < b.len() && matches!(b[*pos], b' ' | b'\t' | b'\r' | b'\n') {
            *pos += 1;
        }
    }

    fn parse_val(b: &[u8], pos: &mut usize, depth: usize) -> Result<Val, PackError> {
        if depth > MAX_DEPTH {
            return Err(PackError::DepthExceeded);
        }
        skip_ws(b, pos);
        if *pos >= b.len() {
            return Err(PackError::Syntax { byte_offset: *pos });
        }
        match b[*pos] {
            b'"' => parse_str(b, pos).map(Val::Str),
            b'{' => parse_obj(b, pos, depth + 1),
            b'[' => parse_arr(b, pos, depth + 1),
            b't' if b[*pos..].starts_with(b"true") => {
                *pos += 4;
                Ok(Val::Bool(true))
            }
            b'f' if b[*pos..].starts_with(b"false") => {
                *pos += 5;
                Ok(Val::Bool(false))
            }
            b'n' if b[*pos..].starts_with(b"null") => {
                *pos += 4;
                Ok(Val::Null)
            }
            b'-' | b'0'..=b'9' => parse_num(b, pos),
            _ => Err(PackError::Syntax { byte_offset: *pos }),
        }
    }

    fn parse_obj(b: &[u8], pos: &mut usize, depth: usize) -> Result<Val, PackError> {
        *pos += 1; // '{'
        skip_ws(b, pos);
        let mut entries = Vec::new();
        if *pos < b.len() && b[*pos] == b'}' {
            *pos += 1;
            return Ok(Val::Obj(entries));
        }
        loop {
            skip_ws(b, pos);
            if *pos >= b.len() || b[*pos] != b'"' {
                return Err(PackError::Syntax { byte_offset: *pos });
            }
            let key = parse_str(b, pos)?;
            skip_ws(b, pos);
            if *pos >= b.len() || b[*pos] != b':' {
                return Err(PackError::Syntax { byte_offset: *pos });
            }
            *pos += 1;
            let val = parse_val(b, pos, depth)?;
            entries.push((key, val));
            skip_ws(b, pos);
            if *pos >= b.len() {
                return Err(PackError::Syntax { byte_offset: *pos });
            }
            match b[*pos] {
                b',' => *pos += 1,
                b'}' => {
                    *pos += 1;
                    return Ok(Val::Obj(entries));
                }
                _ => return Err(PackError::Syntax { byte_offset: *pos }),
            }
        }
    }

    fn parse_arr(b: &[u8], pos: &mut usize, depth: usize) -> Result<Val, PackError> {
        *pos += 1; // '['
        skip_ws(b, pos);
        let mut items = Vec::new();
        if *pos < b.len() && b[*pos] == b']' {
            *pos += 1;
            return Ok(Val::Arr(items));
        }
        loop {
            let val = parse_val(b, pos, depth)?;
            items.push(val);
            skip_ws(b, pos);
            if *pos >= b.len() {
                return Err(PackError::Syntax { byte_offset: *pos });
            }
            match b[*pos] {
                b',' => *pos += 1,
                b']' => {
                    *pos += 1;
                    return Ok(Val::Arr(items));
                }
                _ => return Err(PackError::Syntax { byte_offset: *pos }),
            }
        }
    }

    fn parse_num(b: &[u8], pos: &mut usize) -> Result<Val, PackError> {
        let start = *pos;
        if b[*pos] == b'-' {
            *pos += 1;
        }
        let digits_start = *pos;
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
        if *pos == digits_start {
            return Err(PackError::Syntax { byte_offset: start });
        }
        let int_end = *pos;
        // Consume fractional or exponent suffix if present so floats don't break parsing.
        if *pos < b.len() && b[*pos] == b'.' {
            *pos += 1;
            let frac_start = *pos;
            while *pos < b.len() && b[*pos].is_ascii_digit() {
                *pos += 1;
            }
            if *pos == frac_start {
                return Err(PackError::Syntax { byte_offset: start });
            }
        }
        if *pos < b.len() && (b[*pos] == b'e' || b[*pos] == b'E') {
            *pos += 1;
            if *pos < b.len() && (b[*pos] == b'+' || b[*pos] == b'-') {
                *pos += 1;
            }
            let exp_start = *pos;
            while *pos < b.len() && b[*pos].is_ascii_digit() {
                *pos += 1;
            }
            if *pos == exp_start {
                return Err(PackError::Syntax { byte_offset: start });
            }
        }
        let int_slice = core::str::from_utf8(&b[start..int_end]).map_err(|_| PackError::Syntax { byte_offset: start })?;
        let signed = int_slice.parse::<i64>().unwrap_or(0);
        let unsigned = int_slice.parse::<u64>().ok();
        Ok(Val::Num(signed, unsigned))
    }

    fn parse_str(b: &[u8], pos: &mut usize) -> Result<String, PackError> {
        *pos += 1; // opening '"'
        let mut out = String::new();
        while *pos < b.len() {
            let c = b[*pos];
            if c == b'"' {
                *pos += 1;
                return Ok(out);
            }
            if c == b'\\' {
                *pos += 1;
                if *pos >= b.len() {
                    return Err(PackError::Syntax { byte_offset: *pos });
                }
                match b[*pos] {
                    b'"' => { out.push('"'); *pos += 1; }
                    b'\\' => { out.push('\\'); *pos += 1; }
                    b'/' => { out.push('/'); *pos += 1; }
                    b'b' => { out.push('\u{0008}'); *pos += 1; }
                    b'f' => { out.push('\u{000C}'); *pos += 1; }
                    b'n' => { out.push('\n'); *pos += 1; }
                    b'r' => { out.push('\r'); *pos += 1; }
                    b't' => { out.push('\t'); *pos += 1; }
                    b'u' => {
                        *pos += 1;
                        let hi = parse_hex4(b, pos)?;
                        if (0xD800..=0xDBFF).contains(&hi)
                            && *pos + 2 <= b.len()
                            && &b[*pos..*pos + 2] == b"\\u"
                        {
                            *pos += 2;
                            let lo = parse_hex4(b, pos)?;
                            let cp = 0x10000 + (((hi as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        } else {
                            out.push(char::from_u32(hi as u32).unwrap_or('\u{FFFD}'));
                        }
                    }
                    _ => return Err(PackError::Syntax { byte_offset: *pos }),
                }
            } else if c < 0x20 {
                return Err(PackError::Syntax { byte_offset: *pos });
            } else {
                // Decode valid UTF-8 sequence starting at `*pos`.
                let rest = &b[*pos..];
                let ch_len = utf8_char_len(c);
                if ch_len == 0 || rest.len() < ch_len {
                    return Err(PackError::Syntax { byte_offset: *pos });
                }
                let s = core::str::from_utf8(&rest[..ch_len]).map_err(|_| PackError::Syntax { byte_offset: *pos })?;
                out.push_str(s);
                *pos += ch_len;
            }
        }
        Err(PackError::Syntax { byte_offset: *pos })
    }

    fn parse_hex4(b: &[u8], pos: &mut usize) -> Result<u16, PackError> {
        if *pos + 4 > b.len() {
            return Err(PackError::Syntax { byte_offset: *pos });
        }
        let mut val = 0u16;
        for _ in 0..4 {
            let d = match b[*pos] {
                b'0'..=b'9' => b[*pos] - b'0',
                b'a'..=b'f' => b[*pos] - b'a' + 10,
                b'A'..=b'F' => b[*pos] - b'A' + 10,
                _ => return Err(PackError::Syntax { byte_offset: *pos }),
            };
            val = (val << 4) | d as u16;
            *pos += 1;
        }
        Ok(val)
    }

    fn utf8_char_len(first: u8) -> usize {
        if first < 0x80 { 1 }
        else if (first & 0xE0) == 0xC0 { 2 }
        else if (first & 0xF0) == 0xE0 { 3 }
        else if (first & 0xF8) == 0xF0 { 4 }
        else { 0 }
    }
}
