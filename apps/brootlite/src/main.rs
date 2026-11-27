// SPDX-License-Identifier: GPL-3.0-only
//! BrootLite: Dedicated native Stremio v3 / Swarm P2P / Kodi media discovery app for SexOS.
//!
//! Launch with network:
//!   `run +net brootlite`
//!
//! Zero private URLs or secrets are bundled. User-configured Stremio addons,
//! trackers, Swarm indexers, and featured feeds are loaded from:
//!   - `/data/brootlite/provider_pack.json`
//!   - `/data/qupid/provider_pack.json`
//!   - `/media/Ventoy/provider_pack.json`
//!
//! The standard public Stremio metadata catalog (`https://v3-cinemeta.strem.io`)
//! is used as the default Discover catalog.
#![no_std]
#![no_main]

extern crate alloc;

mod draw;
mod poster_cache;
mod provider;
mod stremio;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, launch, Event, Window};
use sex_kodi::manager::{Manager as KodiManager, Net as KodiNet, Store as KodiStore};
use sex_pdx::app_pointer::Kind as PtrKind;

use crate::draw::{
    blit_image_cover, dim_backdrop, draw_badge, draw_card, draw_pill, fill_rect, poster_swatch,
    Rect, TextPainter, ACCENT_AMBER, ACCENT_AMBER_DIM, ACCENT_CYAN, ACCENT_CYAN_DIM,
    ACCENT_EMERALD, ACCENT_EMERALD_DIM, BG_CANVAS, BG_CARD, BG_ELEVATED, BG_HOVER, BG_SIDEBAR,
    BORDER_ACTIVE, BORDER_DEFAULT, TEXT_MUTED, TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::poster_cache::PosterCache;
use crate::provider::ProviderPack;
use crate::stremio::{
    fetch_catalog, fetch_streams, http_get, search_catalog, CatalogItem, CatalogKind, MediaType,
    StreamItem, CINEMETA_BASE,
};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[brootlite.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Board,
    Discover,
    Swarm,
    Kodi,
    Library,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Board,
        Tab::Discover,
        Tab::Swarm,
        Tab::Kodi,
        Tab::Library,
    ];

    const fn label(self) -> &'static str {
        match self {
            Tab::Board => "Board",
            Tab::Discover => "Discover",
            Tab::Swarm => "Search / Swarm",
            Tab::Kodi => "Kodi Add-ons",
            Tab::Library => "Library",
        }
    }

    const fn short_label(self) -> &'static str {
        match self {
            Tab::Board => "Board",
            Tab::Discover => "Discover",
            Tab::Swarm => "Swarm",
            Tab::Kodi => "Kodi",
            Tab::Library => "Library",
        }
    }

    fn next(self) -> Self {
        match self {
            Tab::Board => Tab::Discover,
            Tab::Discover => Tab::Swarm,
            Tab::Swarm => Tab::Kodi,
            Tab::Kodi => Tab::Library,
            Tab::Library => Tab::Board,
        }
    }
}

/// Interactive click targets recorded during each frame's render pass.
#[derive(Clone, Debug)]
enum HitAction {
    SwitchTab(Tab),
    SetMediaType(MediaType),
    SetCatalogKind(CatalogKind),
    SetAddonCatalog(Option<usize>),
    RefreshDiscover,
    OpenDetail(usize),       // index into active catalog list (Discover)
    OpenSearchDetail(usize), // index into search_results
    CloseDetail,
    RefreshStreams,
    PlayStream(usize),
    FocusSearch,
    RunSearch,
    LaunchFeed(usize),
    LaunchSwarmIndexer(usize),
    RefreshKodi,
    LaunchKodiUi,
    PlayLibraryFile(usize),
    RescanLibrary,
}

#[derive(Clone, Debug)]
struct HitZone {
    rect: Rect,
    action: HitAction,
}

struct DetailOverlay {
    item: CatalogItem,
    streams: Vec<StreamItem>,
    stream_status: String,
    scroll_y: i32,
}

#[derive(Clone, Debug)]
struct LocalMedia {
    name: String,
    path: String,
    size: u64,
    is_video: bool,
}

struct KodiFileStore {
    window: *mut Window,
}

impl KodiStore for KodiFileStore {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let window = unsafe { &mut *self.window };
        let p = format!("kodi/{path}");
        let len = files::size(window, p.as_bytes()).ok()? as usize;
        let mut out = vec![0u8; len];
        let mut at = 0;
        while at < len {
            match files::read_at(window, p.as_bytes(), at as u64, &mut out[at..]) {
                Ok(n) if n > 0 => at += n,
                _ => return None,
            }
        }
        Some(out)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let window = unsafe { &mut *self.window };
        let p = format!("kodi/{path}");
        let mut at = 0;
        while let Some(k) = p[at..].find('/') {
            let _ = files::mkdir(window, p[..at + k].as_bytes());
            at += k + 1;
        }
        files::write(window, p.as_bytes(), data)
            .map_err(|_| String::from("write failed"))
    }
}

struct KodiHttpNet {
    window: *mut Window,
}

impl KodiNet for KodiHttpNet {
    fn get(&mut self, url: &str, max: usize) -> Result<Vec<u8>, String> {
        let window = unsafe { &mut *self.window };
        http_get(window, url, max)
    }
}

struct App {
    tab: Tab,
    pack: ProviderPack,
    posters: PosterCache,
    tp: TextPainter,

    // Discover state
    media_type: MediaType,
    catalog_kind: CatalogKind,
    /// `None` = Cinemeta default; `Some(i)` = catalog addon `pack.stremio_addons[i]`.
    selected_addon: Option<usize>,
    catalog: Vec<CatalogItem>,
    catalog_status: String,
    scroll_y: i32,
    visible_poster_urls: Vec<String>,

    // Detail & Stream Picker overlay
    detail: Option<DetailOverlay>,

    // Search / Swarm state
    search_query: String,
    search_focused: bool,
    search_results: Vec<CatalogItem>,
    search_status: String,
    swarm_scroll_y: i32,

    // Kodi Add-ons state
    kodi: KodiManager,
    kodi_status: String,

    // Library state
    library_files: Vec<LocalMedia>,
    library_scanned: bool,

    // Interaction & status
    mouse_x: i32,
    mouse_y: i32,
    hits: Vec<HitZone>,
    status_toast: String,
    launch_cookie: u32,
    dirty: bool,
}

impl App {
    fn new(window: &mut Window) -> Self {
        let pack = ProviderPack::load(window);
        let mut store = KodiFileStore { window };
        let kodi = KodiManager::load(&mut store);
        let kodi_status = format!(
            "{} installed add-on(s), {} repository source(s)",
            kodi.installed.len(),
            kodi.sources.len()
        );
        let toast = if pack.source_path.is_empty() {
            String::from("Public Cinemeta mode · Add provider_pack.json for custom addons/Swarm")
        } else {
            format!(
                "Loaded {} ({} addons, {} Swarm indexers)",
                pack.source_path,
                pack.stremio_addons.len(),
                pack.swarm_indexers.len()
            )
        };
        Self {
            tab: Tab::Discover,
            pack,
            posters: PosterCache::new(),
            tp: TextPainter::new(),
            media_type: MediaType::Movie,
            catalog_kind: CatalogKind::Popular,
            selected_addon: None,
            catalog: Vec::new(),
            catalog_status: String::from("Loading Cinemeta catalog..."),
            scroll_y: 0,
            visible_poster_urls: Vec::new(),
            detail: None,
            search_query: String::new(),
            search_focused: false,
            search_results: Vec::new(),
            search_status: String::from("Type a movie/series title and press Enter to search"),
            swarm_scroll_y: 0,
            kodi,
            kodi_status,
            library_files: Vec::new(),
            library_scanned: false,
            mouse_x: -1,
            mouse_y: -1,
            hits: Vec::new(),
            status_toast: toast,
            launch_cookie: 100,
            dirty: true,
        }
    }

    fn reload_discover(&mut self, window: &mut Window) {
        let base = match self.selected_addon.and_then(|i| self.pack.stremio_addons.get(i)) {
            Some(addon) => addon.base_url().to_string(),
            None => String::from(CINEMETA_BASE),
        };
        let source_label = match self.selected_addon.and_then(|i| self.pack.stremio_addons.get(i)) {
            Some(addon) => addon.name.clone(),
            None => String::from("Cinemeta"),
        };
        self.catalog_status = format!(
            "Fetching {} · {} ({})...",
            source_label,
            self.media_type.label(),
            self.catalog_kind.label()
        );
        self.dirty = true;
        self.render(window);

        match fetch_catalog(window, &base, self.media_type, self.catalog_kind.catalog_id()) {
            Ok(items) => {
                self.catalog_status = format!(
                    "{} · {} {} ({} titles)",
                    source_label,
                    self.catalog_kind.label(),
                    self.media_type.label(),
                    items.len()
                );
                self.catalog = items;
                self.scroll_y = 0;
            }
            Err(err) => {
                self.catalog_status = format!(
                    "Offline or fetch failed ({}) — run with `+net` for live catalogs",
                    err
                );
            }
        }
        self.dirty = true;
    }

    fn open_detail(&mut self, window: &mut Window, item: CatalogItem) {
        let overlay = DetailOverlay {
            item: item.clone(),
            streams: Vec::new(),
            stream_status: String::from("Querying Stremio stream addons..."),
            scroll_y: 0,
        };
        self.detail = Some(overlay);
        self.dirty = true;
        self.render(window);

        // Fetch hero poster immediately if uncached.
        if !item.poster.is_empty() && self.posters.needs_fetch(&item.poster) {
            self.posters.fetch_one(window, &item.poster);
        }

        let (streams, status) = fetch_streams(
            window,
            &self.pack,
            &item.media_type,
            &item.id,
            &item.name,
        );
        if let Some(d) = self.detail.as_mut() {
            d.streams = streams;
            d.stream_status = status;
        }
        self.dirty = true;
    }

    fn run_search(&mut self, window: &mut Window) {
        let q = self.search_query.trim().to_string();
        if q.is_empty() {
            self.search_results.clear();
            self.search_status = String::from("Enter a title to search Cinemeta & Swarm indexers");
            self.dirty = true;
            return;
        }
        self.search_status = format!("Searching Cinemeta for \"{}\"...", q);
        self.dirty = true;
        self.render(window);

        let mut combined = Vec::new();
        if let Ok(movies) = search_catalog(window, MediaType::Movie, &q) {
            combined.extend(movies.into_iter().take(12));
        }
        if let Ok(series) = search_catalog(window, MediaType::Series, &q) {
            combined.extend(series.into_iter().take(12));
        }
        self.search_status = if combined.is_empty() {
            format!("No catalog matches for \"{}\" (check +net)", q)
        } else {
            format!("{} catalog result(s) for \"{}\"", combined.len(), q)
        };
        self.search_results = combined;
        self.dirty = true;
    }

    fn refresh_kodi(&mut self, window: &mut Window) {
        self.kodi_status = String::from("Refreshing Kodi repositories...");
        self.dirty = true;
        self.render(window);

        let mut store = KodiFileStore { window };
        self.kodi = KodiManager::load(&mut store);
        let mut net = KodiHttpNet { window };
        let refreshed = self.kodi.refresh(&mut net);
        let total_cat = self.kodi.catalogue.entries.len();
        self.kodi_status = format!(
            "{} installed · {} repo(s) refreshed · {} catalogue add-on(s)",
            self.kodi.installed.len(),
            refreshed.len(),
            total_cat
        );
        self.dirty = true;
    }

    fn scan_library(&mut self, window: &mut Window) {
        self.library_files.clear();
        const ROOTS: [&[u8]; 4] = [
            b"/media/Ventoy",
            b"/data/media",
            b"/data/music",
            b"/Movies",
        ];
        for root in ROOTS {
            if let Ok(entries) = files::list(window, root) {
                let root_str = core::str::from_utf8(root).unwrap_or("");
                for entry in entries {
                    if entry.dir || self.library_files.len() >= 120 {
                        continue;
                    }
                    let name = String::from_utf8_lossy(&entry.name).into_owned();
                    if let Some(is_video) = classify_media_ext(&name) {
                        self.library_files.push(LocalMedia {
                            path: format!("{}/{}", root_str, name),
                            name,
                            size: entry.size,
                            is_video,
                        });
                    }
                }
            }
        }
        self.library_scanned = true;
        self.dirty = true;
    }

    fn launch_playback(&mut self, stream: &StreamItem) {
        self.launch_cookie = self.launch_cookie.wrapping_add(1);
        let cookie = self.launch_cookie;
        if stream.is_direct && !stream.url.is_empty() {
            match launch::run(b"qupid", stream.url.as_bytes(), true, cookie) {
                Ok(()) => {
                    self.status_toast = format!("Launched Qupid -> {}", truncate(&stream.url, 48));
                }
                Err(_) => {
                    self.status_toast = format!(
                        "Direct HTTP ready (run +net qupid {})",
                        truncate(&stream.url, 42)
                    );
                }
            }
        } else if !stream.magnet_uri.is_empty() {
            let args = format!("get {}", stream.magnet_uri);
            match launch::run(b"swingers", args.as_bytes(), true, cookie) {
                Ok(()) => {
                    self.status_toast =
                        format!("Launched Swingers P2P -> {}", truncate(&stream.display_title(), 40));
                }
                Err(_) => {
                    self.status_toast = format!(
                        "Magnet ready (run +net swingers get {})",
                        truncate(&stream.info_hash, 20)
                    );
                }
            }
        } else {
            self.status_toast = String::from("Stream has no playable URL or infoHash");
        }
        self.dirty = true;
    }

    fn launch_url_or_file(&mut self, program: &[u8], arg: &str, label: &str) {
        self.launch_cookie = self.launch_cookie.wrapping_add(1);
        match launch::run(program, arg.as_bytes(), true, self.launch_cookie) {
            Ok(()) => {
                self.status_toast = format!("Launched {} ({})", label, truncate(arg, 44));
            }
            Err(_) => {
                self.status_toast = format!("{} target: {}", label, truncate(arg, 50));
            }
        }
        self.dirty = true;
    }

    fn handle_click(&mut self, window: &mut Window, x: i32, y: i32) {
        // Check hit zones in reverse order so top-most overlays win.
        let action = self
            .hits
            .iter()
            .rev()
            .find(|hz| hz.rect.contains(x, y))
            .map(|hz| hz.action.clone());

        let Some(action) = action else {
            self.search_focused = false;
            return;
        };

        match action {
            HitAction::SwitchTab(t) => {
                self.tab = t;
                self.detail = None;
                self.search_focused = t == Tab::Swarm;
                if t == Tab::Library && !self.library_scanned {
                    self.scan_library(window);
                }
                self.dirty = true;
            }
            HitAction::SetMediaType(mt) => {
                if self.media_type != mt {
                    self.media_type = mt;
                    self.reload_discover(window);
                }
            }
            HitAction::SetCatalogKind(ck) => {
                if self.catalog_kind != ck || self.selected_addon.is_some() {
                    self.catalog_kind = ck;
                    self.selected_addon = None;
                    self.reload_discover(window);
                }
            }
            HitAction::SetAddonCatalog(idx) => {
                if self.selected_addon != idx {
                    self.selected_addon = idx;
                    self.reload_discover(window);
                }
            }
            HitAction::RefreshDiscover => {
                self.posters.clear_failed();
                self.reload_discover(window);
            }
            HitAction::OpenDetail(idx) => {
                if let Some(item) = self.catalog.get(idx).cloned() {
                    self.open_detail(window, item);
                }
            }
            HitAction::OpenSearchDetail(idx) => {
                if let Some(item) = self.search_results.get(idx).cloned() {
                    self.open_detail(window, item);
                }
            }
            HitAction::CloseDetail => {
                self.detail = None;
                self.dirty = true;
            }
            HitAction::RefreshStreams => {
                if let Some(item) = self.detail.as_ref().map(|d| d.item.clone()) {
                    self.open_detail(window, item);
                }
            }
            HitAction::PlayStream(idx) => {
                let chosen = self
                    .detail
                    .as_ref()
                    .and_then(|d| d.streams.get(idx).cloned());
                if let Some(stream) = chosen {
                    self.launch_playback(&stream);
                }
            }
            HitAction::FocusSearch => {
                self.search_focused = true;
                self.dirty = true;
            }
            HitAction::RunSearch => {
                self.run_search(window);
            }
            HitAction::LaunchFeed(idx) => {
                if let Some(feed) = self.pack.featured_feeds.get(idx).cloned() {
                    self.launch_url_or_file(b"kaleidoscope", &feed.url, &feed.label);
                }
            }
            HitAction::LaunchSwarmIndexer(idx) => {
                if let Some(ix) = self.pack.swarm_indexers.get(idx).cloned() {
                    let q = if self.search_query.trim().is_empty() {
                        "1080p"
                    } else {
                        self.search_query.trim()
                    };
                    let url = ix.build_query_url(q);
                    self.launch_url_or_file(b"kaleidoscope", &url, &ix.name);
                }
            }
            HitAction::RefreshKodi => {
                self.refresh_kodi(window);
            }
            HitAction::LaunchKodiUi => {
                self.launch_url_or_file(b"qupid", "--addons", "Qupid Kodi Add-ons");
            }
            HitAction::PlayLibraryFile(idx) => {
                if let Some(f) = self.library_files.get(idx).cloned() {
                    let arg = format!("@{}", f.path);
                    self.launch_url_or_file(b"qupid", &arg, &f.name);
                }
            }
            HitAction::RescanLibrary => {
                self.scan_library(window);
            }
        }
    }

    fn handle_wheel(&mut self, steps: i8) {
        let delta = -(steps as i32) * 48;
        if let Some(d) = self.detail.as_mut() {
            d.scroll_y = (d.scroll_y + delta).max(0);
        } else if self.tab == Tab::Discover {
            self.scroll_y = (self.scroll_y + delta).max(0);
        } else if self.tab == Tab::Swarm {
            self.swarm_scroll_y = (self.swarm_scroll_y + delta).max(0);
        }
        self.dirty = true;
    }

    fn handle_key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        // Esc closes detail overlay.
        if key.code == 0x01 && self.detail.is_some() {
            self.detail = None;
            self.dirty = true;
            return;
        }
        // Tab cycles main navigation when detail overlay is closed.
        if key.code == 0x0F && self.detail.is_none() {
            self.tab = self.tab.next();
            self.search_focused = self.tab == Tab::Swarm;
            if self.tab == Tab::Library && !self.library_scanned {
                self.scan_library(window);
            }
            self.dirty = true;
            return;
        }

        // Search box text entry on Swarm tab.
        if self.detail.is_none() && self.tab == Tab::Swarm && self.search_focused {
            match key.code {
                0x0E => {
                    // Backspace
                    self.search_query.pop();
                    self.dirty = true;
                    return;
                }
                0x1C => {
                    // Enter
                    self.run_search(window);
                    return;
                }
                _ => {
                    if let Some(ch) = key.char() {
                        if !ch.is_control() && self.search_query.len() < 80 {
                            self.search_query.push(ch);
                            self.dirty = true;
                            return;
                        }
                    }
                }
            }
        }

        // Scroll & refresh shortcuts.
        match key.code {
            0x48 => self.handle_wheel(2),  // Up
            0x50 => self.handle_wheel(-2), // Down
            0x49 => self.handle_wheel(6),  // Page Up
            0x51 => self.handle_wheel(-6), // Page Down
            0x13 if self.detail.is_none() && self.tab == Tab::Discover => {
                // R refreshes Discover
                self.posters.clear_failed();
                self.reload_discover(window);
            }
            0x0E if self.detail.is_some() => {
                // Backspace closes Detail
                self.detail = None;
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// Incrementally decodes at most one visible uncached poster per frame tick
    /// so the UI stays snappy while posters stream in.
    fn pump_one_poster(&mut self, window: &mut Window) -> bool {
        if let Some(url) = self
            .visible_poster_urls
            .iter()
            .find(|u| self.posters.needs_fetch(u))
            .cloned()
        {
            if self.posters.fetch_one(window, &url) {
                self.dirty = true;
                return true;
            }
        }
        false
    }

    fn render(&mut self, window: &mut Window) {
        if !self.dirty {
            return;
        }
        let (bw, bh) = window.size();
        if bw < 64 || bh < 64 {
            return;
        }
        let buf = window.frame(bw, bh);
        self.hits.clear();
        self.visible_poster_urls.clear();

        // Fill Xirtus Dark-Glass canvas.
        fill_rect(buf, bw, bh, 0, 0, bw as i32, bh as i32, BG_CANVAS);

        let compact = bw < 680;
        let status_h = 26i32;
        let content_rect = if compact {
            let nav_h = 46i32;
            self.render_top_nav(buf, bw, bh, nav_h);
            Rect::new(0, nav_h, bw as i32, bh as i32 - nav_h - status_h)
        } else {
            let side_w = 184i32;
            self.render_sidebar(buf, bw, bh, side_w, bh as i32 - status_h);
            Rect::new(side_w, 0, bw as i32 - side_w, bh as i32 - status_h)
        };

        match self.tab {
            Tab::Board => self.render_board(buf, bw, bh, content_rect),
            Tab::Discover => self.render_discover(buf, bw, bh, content_rect),
            Tab::Swarm => self.render_swarm(buf, bw, bh, content_rect),
            Tab::Kodi => self.render_kodi(buf, bw, bh, content_rect),
            Tab::Library => self.render_library(buf, bw, bh, content_rect),
        }

        // Bottom status strip.
        let sy = bh as i32 - status_h;
        fill_rect(buf, bw, bh, 0, sy, bw as i32, 1, BORDER_DEFAULT);
        fill_rect(buf, bw, bh, 0, sy + 1, bw as i32, status_h - 1, BG_SIDEBAR);
        self.tp.draw(
            buf,
            bw,
            bh,
            12,
            sy + 5,
            bw as i32 - 210,
            12,
            &self.status_toast,
            TEXT_SECONDARY,
            false,
        );
        let cache_lbl = format!(
            "Posters {}/{} · Trackers {}",
            self.posters.len(),
            poster_cache::MAX_POSTERS,
            self.pack.tracker_count()
        );
        let cw = self.tp.measure(&cache_lbl, 11, false);
        self.tp.draw(
            buf,
            bw,
            bh,
            bw as i32 - cw - 12,
            sy + 6,
            cw + 8,
            11,
            &cache_lbl,
            TEXT_MUTED,
            false,
        );

        // Detail & Stream Picker modal overlay on top.
        if self.detail.is_some() {
            self.render_detail_overlay(buf, bw, bh);
        }

        window.show_frame(bw, bh);
        self.dirty = false;
    }

    fn render_sidebar(&mut self, buf: &mut [u32], bw: usize, bh: usize, w: i32, h: i32) {
        fill_rect(buf, bw, bh, 0, 0, w, h, BG_SIDEBAR);
        fill_rect(buf, bw, bh, w - 1, 0, 1, h, BORDER_DEFAULT);

        // Brand header.
        draw_card(
            buf,
            bw,
            bh,
            Rect::new(14, 14, 28, 28),
            7,
            ACCENT_CYAN_DIM,
            ACCENT_CYAN,
        );
        self.tp.draw(buf, bw, bh, 21, 19, 20, 14, "B", ACCENT_CYAN, true);
        self.tp
            .draw(buf, bw, bh, 50, 15, w - 60, 15, "BrootLite", TEXT_PRIMARY, true);
        self.tp
            .draw(buf, bw, bh, 50, 32, w - 60, 10, "SexOS Native", TEXT_MUTED, false);

        // Navigation items.
        let mut y = 64i32;
        for tab in Tab::ALL {
            let r = Rect::new(10, y, w - 20, 34);
            let active = self.tab == tab;
            let hovered = self.detail.is_none() && r.contains(self.mouse_x, self.mouse_y);
            let (fill, border, fg) = if active {
                (ACCENT_CYAN_DIM, BORDER_ACTIVE, ACCENT_CYAN)
            } else if hovered {
                (BG_HOVER, BORDER_ACTIVE, TEXT_PRIMARY)
            } else {
                (BG_SIDEBAR, BG_SIDEBAR, TEXT_SECONDARY)
            };
            draw_card(buf, bw, bh, r, 8, fill, border);
            if active {
                fill_rect(buf, bw, bh, r.x + 2, r.y + 8, 3, r.h - 16, ACCENT_CYAN);
            }
            self.tp.draw(
                buf,
                bw,
                bh,
                r.x + 14,
                r.y + 9,
                r.w - 20,
                13,
                tab.label(),
                fg,
                active,
            );
            self.hits.push(HitZone {
                rect: r,
                action: HitAction::SwitchTab(tab),
            });
            y += 40;
        }

        // Provider Pack summary card at bottom of sidebar.
        let card_h = 84i32;
        let card_y = (h - card_h - 12).max(y + 12);
        let card = Rect::new(10, card_y, w - 20, card_h);
        draw_card(buf, bw, bh, card, 8, BG_CARD, BORDER_DEFAULT);
        let pack_title = if self.pack.name.is_empty() {
            "Public Mode"
        } else {
            &self.pack.name
        };
        let status_dot = if self.pack.stremio_addons.is_empty() {
            ACCENT_AMBER
        } else {
            ACCENT_EMERALD
        };
        fill_rect(buf, bw, bh, card.x + 10, card.y + 13, 6, 6, status_dot);
        self.tp.draw(
            buf,
            bw,
            bh,
            card.x + 22,
            card.y + 9,
            card.w - 30,
            12,
            pack_title,
            TEXT_PRIMARY,
            true,
        );
        let s1 = format!("Stremio addons: {}", self.pack.stremio_addons.len());
        let s2 = format!("Swarm indexers: {}", self.pack.swarm_indexers.len());
        let s3 = format!("Featured feeds: {}", self.pack.featured_feeds.len());
        self.tp
            .draw(buf, bw, bh, card.x + 10, card.y + 28, card.w - 16, 11, &s1, TEXT_SECONDARY, false);
        self.tp
            .draw(buf, bw, bh, card.x + 10, card.y + 44, card.w - 16, 11, &s2, TEXT_SECONDARY, false);
        self.tp
            .draw(buf, bw, bh, card.x + 10, card.y + 60, card.w - 16, 11, &s3, TEXT_SECONDARY, false);
    }

    fn render_top_nav(&mut self, buf: &mut [u32], bw: usize, bh: usize, h: i32) {
        fill_rect(buf, bw, bh, 0, 0, bw as i32, h, BG_SIDEBAR);
        fill_rect(buf, bw, bh, 0, h - 1, bw as i32, 1, BORDER_DEFAULT);
        self.tp
            .draw(buf, bw, bh, 12, 13, 96, 15, "BrootLite", ACCENT_CYAN, true);
        let mut x = 104i32;
        for tab in Tab::ALL {
            let lbl = tab.short_label();
            let pw = (self.tp.measure(lbl, 12, true) + 22).max(60);
            let r = Rect::new(x, 9, pw, 28);
            let hovered = self.detail.is_none() && r.contains(self.mouse_x, self.mouse_y);
            draw_pill(buf, bw, bh, &mut self.tp, r, lbl, self.tab == tab, hovered);
            self.hits.push(HitZone {
                rect: r,
                action: HitAction::SwitchTab(tab),
            });
            x += pw + 6;
        }
    }

    fn render_board(&mut self, buf: &mut [u32], bw: usize, bh: usize, area: Rect) {
        let pad = 20i32;
        let x = area.x + pad;
        let mut y = area.y + pad;
        let w = (area.w - pad * 2).max(200);

        self.tp
            .draw(buf, bw, bh, x, y, w, 20, "Board", TEXT_PRIMARY, true);
        self.tp.draw(
            buf,
            bw,
            bh,
            x,
            y + 26,
            w,
            12,
            "Native Stremio v3, Swarm P2P & Kodi Hub for SexOS",
            TEXT_SECONDARY,
            false,
        );
        y += 52;

        // 4 KPI stat cards.
        let gap = 12i32;
        let cols = if w >= 680 { 4 } else { 2 };
        let card_w = (w - gap * (cols - 1)) / cols;
        let stats: [(&str, String, u32); 4] = [
            ("Catalog Titles", format!("{}", self.catalog.len()), ACCENT_CYAN),
            (
                "Stremio Addons",
                format!("{}", self.pack.stremio_addons.len()),
                ACCENT_EMERALD,
            ),
            (
                "Swarm Indexers",
                format!("{}", self.pack.swarm_indexers.len()),
                ACCENT_AMBER,
            ),
            (
                "Kodi Add-ons",
                format!("{}", self.kodi.installed.len()),
                ACCENT_CYAN,
            ),
        ];
        for (i, (label, val, accent)) in stats.iter().enumerate() {
            let col = (i as i32) % cols;
            let row = (i as i32) / cols;
            let cr = Rect::new(x + col * (card_w + gap), y + row * 78, card_w, 66);
            draw_card(buf, bw, bh, cr, 10, BG_CARD, BORDER_DEFAULT);
            self.tp
                .draw(buf, bw, bh, cr.x + 14, cr.y + 12, cr.w - 24, 11, label, TEXT_MUTED, false);
            self.tp
                .draw(buf, bw, bh, cr.x + 14, cr.y + 30, cr.w - 24, 20, val, *accent, true);
        }
        y += ((4 + cols - 1) / cols) * 78 + 10;

        // Provider Pack status banner.
        let banner = Rect::new(x, y, w, 74);
        draw_card(buf, bw, bh, banner, 10, BG_CARD, BORDER_DEFAULT);
        if self.pack.source_path.is_empty() {
            self.tp.draw(
                buf,
                bw,
                bh,
                banner.x + 16,
                banner.y + 12,
                banner.w - 32,
                14,
                "No provider_pack.json detected (Public Cinemeta Catalog Active)",
                ACCENT_AMBER,
                true,
            );
            self.tp.draw(
                buf,
                bw,
                bh,
                banner.x + 16,
                banner.y + 36,
                banner.w - 32,
                12,
                "Place your private provider_pack.json at /data/brootlite/provider_pack.json, /data/qupid/provider_pack.json, or /media/Ventoy/provider_pack.json.",
                TEXT_SECONDARY,
                false,
            );
        } else {
            let hdr = format!("Provider Pack Active: {}", self.pack.source_path);
            self.tp.draw(
                buf,
                bw,
                bh,
                banner.x + 16,
                banner.y + 12,
                banner.w - 32,
                14,
                &hdr,
                ACCENT_EMERALD,
                true,
            );
            let sub = format!(
                "{} Stremio addons · {} Swarm indexers · {} featured feeds · {} trackers",
                self.pack.stremio_addons.len(),
                self.pack.swarm_indexers.len(),
                self.pack.featured_feeds.len(),
                self.pack.tracker_count()
            );
            self.tp.draw(
                buf,
                bw,
                bh,
                banner.x + 16,
                banner.y + 36,
                banner.w - 32,
                12,
                &sub,
                TEXT_SECONDARY,
                false,
            );
        }
        y += 92;

        // Quick-access preview strip of Discover posters.
        self.tp.draw(
            buf,
            bw,
            bh,
            x,
            y,
            w,
            15,
            "Featured on Discover",
            TEXT_PRIMARY,
            true,
        );
        y += 28;

        let preview_cols = (w / 140).clamp(3, 6);
        let p_gap = 12i32;
        let pw = (w - p_gap * (preview_cols - 1)) / preview_cols;
        let ph = (pw * 3) / 2 + 46;
        if y + ph <= area.y + area.h {
            for i in 0..(preview_cols as usize).min(self.catalog.len()) {
                let cr = Rect::new(x + (i as i32) * (pw + p_gap), y, pw, ph);
                self.render_poster_card(buf, bw, bh, cr, i, false);
            }
        }
    }

    fn render_discover(&mut self, buf: &mut [u32], bw: usize, bh: usize, area: Rect) {
        let pad = 18i32;
        let x = area.x + pad;
        let mut y = area.y + 14;
        let w = (area.w - pad * 2).max(200);

        // Top filter toolbar: Movies | Series  ·  Popular | Top Rated  ·  [Addon Catalogs]  ·  ↻ Refresh
        let mut px = x;
        for mt in [MediaType::Movie, MediaType::Series] {
            let pw = 74i32;
            let r = Rect::new(px, y, pw, 28);
            let hovered = self.detail.is_none() && r.contains(self.mouse_x, self.mouse_y);
            draw_pill(
                buf,
                bw,
                bh,
                &mut self.tp,
                r,
                mt.label(),
                self.media_type == mt,
                hovered,
            );
            self.hits.push(HitZone {
                rect: r,
                action: HitAction::SetMediaType(mt),
            });
            px += pw + 6;
        }

        px += 10;
        for ck in [CatalogKind::Popular, CatalogKind::TopRated] {
            let pw = if ck == CatalogKind::Popular { 78 } else { 90 };
            let r = Rect::new(px, y, pw, 28);
            let active = self.selected_addon.is_none() && self.catalog_kind == ck;
            let hovered = self.detail.is_none() && r.contains(self.mouse_x, self.mouse_y);
            draw_pill(buf, bw, bh, &mut self.tp, r, ck.label(), active, hovered);
            self.hits.push(HitZone {
                rect: r,
                action: HitAction::SetCatalogKind(ck),
            });
            px += pw + 6;
        }

        // Optional catalog addons from provider_pack.json.
        let catalog_addons: Vec<(usize, String)> = self
            .pack
            .stremio_addons
            .iter()
            .enumerate()
            .filter(|(_, a)| a.supports_catalog)
            .take(3)
            .map(|(i, a)| (i, a.name.clone()))
            .collect();
        for (addon_idx, name) in catalog_addons {
            let short = truncate(&name, 14);
            let pw = (self.tp.measure(&short, 12, true) + 20).max(72);
            if px + pw + 100 < x + w {
                let r = Rect::new(px, y, pw, 28);
                let active = self.selected_addon == Some(addon_idx);
                let hovered = self.detail.is_none() && r.contains(self.mouse_x, self.mouse_y);
                draw_pill(buf, bw, bh, &mut self.tp, r, &short, active, hovered);
                self.hits.push(HitZone {
                    rect: r,
                    action: HitAction::SetAddonCatalog(Some(addon_idx)),
                });
                px += pw + 6;
            }
        }

        // Right-aligned ↻ Refresh button.
        let ref_w = 88i32;
        let ref_r = Rect::new(x + w - ref_w, y, ref_w, 28);
        let ref_hov = self.detail.is_none() && ref_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(buf, bw, bh, &mut self.tp, ref_r, "↻ Refresh", false, ref_hov);
        self.hits.push(HitZone {
            rect: ref_r,
            action: HitAction::RefreshDiscover,
        });

        y += 38;
        self.tp.draw(
            buf,
            bw,
            bh,
            x,
            y,
            w,
            12,
            &self.catalog_status,
            TEXT_MUTED,
            false,
        );
        y += 22;

        // 6-column responsive poster gallery!
        let grid_top = y;
        let grid_bottom = area.y + area.h - 6;
        if self.catalog.is_empty() {
            let empty_box = Rect::new(x, grid_top + 12, w, 96);
            draw_card(buf, bw, bh, empty_box, 10, BG_CARD, BORDER_DEFAULT);
            self.tp.draw(
                buf,
                bw,
                bh,
                empty_box.x + 18,
                empty_box.y + 20,
                empty_box.w - 36,
                14,
                "No catalog titles loaded yet",
                TEXT_PRIMARY,
                true,
            );
            self.tp.draw(
                buf,
                bw,
                bh,
                empty_box.x + 18,
                empty_box.y + 46,
                empty_box.w - 36,
                12,
                "Click ↻ Refresh or press R. Ensure BrootLite was started with network (`run +net brootlite`).",
                TEXT_SECONDARY,
                false,
            );
            return;
        }

        let cols: i32 = if w >= 780 {
            6
        } else if w >= 600 {
            5
        } else if w >= 450 {
            4
        } else {
            3
        };
        let gap = 12i32;
        let card_w = (w - gap * (cols - 1)) / cols;
        let poster_h = (card_w * 3) / 2; // ~2:3 aspect ratio
        let card_h = poster_h + 50;
        let row_h = card_h + gap;

        let total_rows = ((self.catalog.len() as i32) + cols - 1) / cols;
        let max_scroll = (total_rows * row_h - (grid_bottom - grid_top)).max(0);
        if self.scroll_y > max_scroll {
            self.scroll_y = max_scroll;
        }

        for i in 0..self.catalog.len() {
            let col = (i as i32) % cols;
            let row = (i as i32) / cols;
            let cx = x + col * (card_w + gap);
            let cy = grid_top + row * row_h - self.scroll_y;
            if cy + card_h < grid_top || cy > grid_bottom {
                continue;
            }
            // Clip cards that start above grid_top cleanly so they don't overlap the toolbar.
            if cy < grid_top - 4 {
                continue;
            }
            let cr = Rect::new(cx, cy, card_w, card_h);
            self.render_poster_card(buf, bw, bh, cr, i, false);
        }
    }

    fn render_poster_card(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        cr: Rect,
        idx: usize,
        from_search: bool,
    ) {
        let item = if from_search {
            self.search_results.get(idx).cloned()
        } else {
            self.catalog.get(idx).cloned()
        };
        let Some(item) = item else {
            return;
        };

        let hovered = self.detail.is_none() && cr.contains(self.mouse_x, self.mouse_y);
        let fill = if hovered { BG_HOVER } else { BG_CARD };
        let border = if hovered { BORDER_ACTIVE } else { BORDER_DEFAULT };
        draw_card(buf, bw, bh, cr, 9, fill, border);

        let poster_r = Rect::new(cr.x + 2, cr.y + 2, cr.w - 4, (cr.h - 52).max(40));
        if !item.poster.is_empty() {
            self.visible_poster_urls.push(item.poster.clone());
        }

        if let Some(img) = self.posters.get(&item.poster) {
            blit_image_cover(buf, bw, bh, poster_r, img);
        } else {
            // Graceful dark-glass fallback: deterministic swatch + large initials + type badge.
            let swatch = poster_swatch(&item.name);
            draw_card(buf, bw, bh, poster_r, 7, swatch, BORDER_DEFAULT);
            let init = item.initials();
            let iw = self.tp.measure(&init, 24, true);
            let ix = poster_r.x + ((poster_r.w - iw) / 2).max(4);
            let iy = poster_r.y + (poster_r.h - 28) / 2;
            self.tp
                .draw(buf, bw, bh, ix, iy, poster_r.w - 8, 24, &init, TEXT_PRIMARY, true);
            draw_badge(
                buf,
                bw,
                bh,
                &mut self.tp,
                poster_r.x + 6,
                poster_r.y + 6,
                item.type_badge(),
                BG_CANVAS,
                ACCENT_CYAN,
            );
        }

        // Metadata footer: Title, ★ imdbRating (amber), releaseInfo (muted).
        let ty = poster_r.y + poster_r.h + 7;
        let title_str = truncate(&item.name, ((cr.w - 14) / 7).max(8) as usize);
        self.tp.draw(
            buf,
            bw,
            bh,
            cr.x + 8,
            ty,
            cr.w - 14,
            12,
            &title_str,
            TEXT_PRIMARY,
            true,
        );

        let meta_y = ty + 18;
        if !item.imdb_rating.is_empty() {
            let rating = format!("★ {}", item.imdb_rating);
            self.tp.draw(
                buf,
                bw,
                bh,
                cr.x + 8,
                meta_y,
                cr.w / 2,
                11,
                &rating,
                ACCENT_AMBER,
                true,
            );
        }
        if !item.release_info.is_empty() {
            let rel = truncate(&item.release_info, 9);
            let rw = self.tp.measure(&rel, 11, false);
            self.tp.draw(
                buf,
                bw,
                bh,
                cr.x + cr.w - rw - 8,
                meta_y,
                rw + 4,
                11,
                &rel,
                TEXT_MUTED,
                false,
            );
        }

        let action = if from_search {
            HitAction::OpenSearchDetail(idx)
        } else {
            HitAction::OpenDetail(idx)
        };
        self.hits.push(HitZone { rect: cr, action });
    }

    fn render_detail_overlay(&mut self, buf: &mut [u32], bw: usize, bh: usize) {
        dim_backdrop(buf, bw, bh);
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        let item = detail.item.clone();
        let streams = detail.streams.clone();
        let stream_status = detail.stream_status.clone();
        let scroll_y = detail.scroll_y;

        // Centered modal panel.
        let margin_x = (bw as i32 / 14).clamp(18, 72);
        let margin_y = (bh as i32 / 14).clamp(18, 48);
        let modal = Rect::new(
            margin_x,
            margin_y,
            bw as i32 - margin_x * 2,
            bh as i32 - margin_y * 2,
        );
        draw_card(buf, bw, bh, modal, 12, BG_ELEVATED, BORDER_ACTIVE);

        // Top bar inside modal: ← Back | Title | ↻ Reload Streams.
        let back_r = Rect::new(modal.x + 16, modal.y + 14, 82, 28);
        let back_hov = back_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(buf, bw, bh, &mut self.tp, back_r, "← Back", false, back_hov);
        self.hits.push(HitZone {
            rect: back_r,
            action: HitAction::CloseDetail,
        });

        let reload_r = Rect::new(modal.x + modal.w - 128, modal.y + 14, 112, 28);
        let reload_hov = reload_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(
            buf,
            bw,
            bh,
            &mut self.tp,
            reload_r,
            "↻ Streams",
            false,
            reload_hov,
        );
        self.hits.push(HitZone {
            rect: reload_r,
            action: HitAction::RefreshStreams,
        });

        let body_y = modal.y + 54;
        let poster_w = (modal.w / 4).clamp(120, 180);
        let poster_h = (poster_w * 3) / 2;
        let poster_r = Rect::new(modal.x + 18, body_y, poster_w, poster_h);

        if let Some(img) = self.posters.get(&item.poster) {
            draw_card(
                buf,
                bw,
                bh,
                Rect::new(poster_r.x - 1, poster_r.y - 1, poster_r.w + 2, poster_r.h + 2),
                8,
                BG_CARD,
                BORDER_DEFAULT,
            );
            blit_image_cover(buf, bw, bh, poster_r, img);
        } else {
            draw_card(buf, bw, bh, poster_r, 8, poster_swatch(&item.name), BORDER_DEFAULT);
            let init = item.initials();
            let iw = self.tp.measure(&init, 30, true);
            self.tp.draw(
                buf,
                bw,
                bh,
                poster_r.x + (poster_r.w - iw) / 2,
                poster_r.y + (poster_r.h - 32) / 2,
                poster_r.w - 8,
                30,
                &init,
                TEXT_PRIMARY,
                true,
            );
        }

        // Right column: title, badges, description, stream list.
        let rx = poster_r.x + poster_r.w + 20;
        let rw = (modal.x + modal.w - rx - 18).max(160);
        let mut ry = body_y;

        self.tp
            .draw(buf, bw, bh, rx, ry, rw, 20, &item.name, TEXT_PRIMARY, true);
        ry += 28;

        let mut bx = rx;
        bx += draw_badge(
            buf,
            bw,
            bh,
            &mut self.tp,
            bx,
            ry,
            item.type_badge(),
            ACCENT_CYAN_DIM,
            ACCENT_CYAN,
        ) + 8;
        if !item.imdb_rating.is_empty() {
            let r_lbl = format!("★ {}", item.imdb_rating);
            bx += draw_badge(
                buf,
                bw,
                bh,
                &mut self.tp,
                bx,
                ry,
                &r_lbl,
                ACCENT_AMBER_DIM,
                ACCENT_AMBER,
            ) + 8;
        }
        if !item.release_info.is_empty() {
            bx += draw_badge(
                buf,
                bw,
                bh,
                &mut self.tp,
                bx,
                ry,
                &item.release_info,
                BG_CARD,
                TEXT_SECONDARY,
            ) + 8;
        }
        draw_badge(
            buf,
            bw,
            bh,
            &mut self.tp,
            bx,
            ry,
            &item.id,
            BG_CARD,
            TEXT_MUTED,
        );
        ry += 30;

        let desc = if item.description.is_empty() {
            "No synopsis available from catalog metadata."
        } else {
            &item.description
        };
        ry = self.tp.draw_wrapped(
            buf,
            bw,
            bh,
            rx,
            ry,
            rw,
            12,
            5,
            4,
            desc,
            TEXT_SECONDARY,
        );
        ry += 10;

        // Divider + Streams header.
        fill_rect(buf, bw, bh, rx, ry, rw, 1, BORDER_DEFAULT);
        ry += 10;
        self.tp
            .draw(buf, bw, bh, rx, ry, rw, 14, "Available Streams", TEXT_PRIMARY, true);
        self.tp.draw(
            buf,
            bw,
            bh,
            rx,
            ry + 20,
            rw,
            11,
            &stream_status,
            TEXT_MUTED,
            false,
        );
        ry += 40;

        let list_bottom = modal.y + modal.h - 14;
        if streams.is_empty() {
            let empty = Rect::new(rx, ry, rw, 64);
            draw_card(buf, bw, bh, empty, 8, BG_CARD, BORDER_DEFAULT);
            self.tp.draw_wrapped(
                buf,
                bw,
                bh,
                empty.x + 12,
                empty.y + 12,
                empty.w - 24,
                12,
                4,
                3,
                "Configure Stremio stream addons in /data/brootlite/provider_pack.json (or /data/qupid/provider_pack.json, /media/Ventoy/provider_pack.json) to resolve Direct HTTP & P2P Magnet streams.",
                TEXT_SECONDARY,
            );
            return;
        }

        let row_h = 54i32;
        for (i, st) in streams.iter().enumerate() {
            let sy = ry + (i as i32) * (row_h + 8) - scroll_y;
            if sy < ry - 4 || sy + row_h > list_bottom {
                continue;
            }
            let rr = Rect::new(rx, sy, rw, row_h);
            let hovered = rr.contains(self.mouse_x, self.mouse_y);
            draw_card(
                buf,
                bw,
                bh,
                rr,
                8,
                if hovered { BG_HOVER } else { BG_CARD },
                if hovered { BORDER_ACTIVE } else { BORDER_DEFAULT },
            );

            // Badges row inside stream card.
            let mut sx = rr.x + 10;
            let (t_lbl, t_bg, t_fg) = if st.is_direct {
                ("Direct HTTP", ACCENT_EMERALD_DIM, ACCENT_EMERALD)
            } else {
                ("P2P Magnet", ACCENT_CYAN_DIM, ACCENT_CYAN)
            };
            sx += draw_badge(buf, bw, bh, &mut self.tp, sx, rr.y + 8, t_lbl, t_bg, t_fg) + 6;
            if !st.resolution.is_empty() {
                sx += draw_badge(
                    buf,
                    bw,
                    bh,
                    &mut self.tp,
                    sx,
                    rr.y + 8,
                    &st.resolution,
                    BG_ELEVATED,
                    TEXT_PRIMARY,
                ) + 6;
            }
            if let Some(seeds) = st.seeders {
                let s_lbl = format!("S:{}", seeds);
                sx += draw_badge(
                    buf,
                    bw,
                    bh,
                    &mut self.tp,
                    sx,
                    rr.y + 8,
                    &s_lbl,
                    ACCENT_AMBER_DIM,
                    ACCENT_AMBER,
                ) + 6;
            }
            self.tp.draw(
                buf,
                bw,
                bh,
                sx + 2,
                rr.y + 11,
                rr.x + rr.w - sx - 12,
                11,
                &st.addon_name,
                TEXT_MUTED,
                false,
            );
            self.tp.draw(
                buf,
                bw,
                bh,
                rr.x + 10,
                rr.y + 33,
                rr.w - 20,
                12,
                st.display_title(),
                TEXT_PRIMARY,
                false,
            );

            self.hits.push(HitZone {
                rect: rr,
                action: HitAction::PlayStream(i),
            });
        }
    }

    fn render_swarm(&mut self, buf: &mut [u32], bw: usize, bh: usize, area: Rect) {
        let pad = 18i32;
        let x = area.x + pad;
        let mut y = area.y + 16;
        let w = (area.w - pad * 2).max(200);

        self.tp
            .draw(buf, bw, bh, x, y, w, 18, "Search / Swarm P2P", TEXT_PRIMARY, true);
        y += 28;

        // Live search input box + Search button.
        let btn_w = 88i32;
        let input_r = Rect::new(x, y, (w - btn_w - 8).max(120), 34);
        let btn_r = Rect::new(input_r.x + input_r.w + 8, y, btn_w, 34);
        draw_card(
            buf,
            bw,
            bh,
            input_r,
            8,
            BG_ELEVATED,
            if self.search_focused {
                BORDER_ACTIVE
            } else {
                BORDER_DEFAULT
            },
        );
        let display_q = if self.search_query.is_empty() {
            if self.search_focused {
                "Type query and press Enter..."
            } else {
                "Click to search Cinemeta & Swarm indexers..."
            }
        } else {
            &self.search_query
        };
        let q_col = if self.search_query.is_empty() {
            TEXT_MUTED
        } else {
            TEXT_PRIMARY
        };
        self.tp.draw(
            buf,
            bw,
            bh,
            input_r.x + 12,
            input_r.y + 9,
            input_r.w - 24,
            13,
            display_q,
            q_col,
            false,
        );
        self.hits.push(HitZone {
            rect: input_r,
            action: HitAction::FocusSearch,
        });

        let btn_hov = self.detail.is_none() && btn_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(buf, bw, bh, &mut self.tp, btn_r, "Search", true, btn_hov);
        self.hits.push(HitZone {
            rect: btn_r,
            action: HitAction::RunSearch,
        });

        y += 42;
        self.tp
            .draw(buf, bw, bh, x, y, w, 12, &self.search_status, TEXT_MUTED, false);
        y += 22;

        // Catalog search results strip (if any).
        if !self.search_results.is_empty() {
            let cols = (w / 136).clamp(3, 6);
            let gap = 10i32;
            let cw = (w - gap * (cols - 1)) / cols;
            let ch = (cw * 3) / 2 + 46;
            for i in 0..(cols as usize).min(self.search_results.len()) {
                let cr = Rect::new(x + (i as i32) * (cw + gap), y, cw, ch);
                self.render_poster_card(buf, bw, bh, cr, i, true);
            }
            y += ch + 18;
        }

        // Configured Swarm Indexers from provider_pack.json.
        self.tp.draw(
            buf,
            bw,
            bh,
            x,
            y,
            w,
            14,
            "Configured Swarm Indexers",
            ACCENT_CYAN,
            true,
        );
        y += 24;

        let bottom = area.y + area.h - 8;
        if self.pack.swarm_indexers.is_empty() {
            let empty = Rect::new(x, y, w, 52);
            draw_card(buf, bw, bh, empty, 8, BG_CARD, BORDER_DEFAULT);
            self.tp.draw(
                buf,
                bw,
                bh,
                empty.x + 14,
                empty.y + 18,
                empty.w - 28,
                12,
                "No Swarm indexers in provider_pack.json. Add `swarmIndexers` to enable P2P search links.",
                TEXT_SECONDARY,
                false,
            );
            y += 64;
        } else {
            let q = if self.search_query.trim().is_empty() {
                "1080p"
            } else {
                self.search_query.trim()
            };
            for (i, ix) in self.pack.swarm_indexers.iter().enumerate() {
                let ry = y - self.swarm_scroll_y;
                if ry + 48 > bottom {
                    break;
                }
                let rr = Rect::new(x, ry, w, 46);
                let hovered = self.detail.is_none() && rr.contains(self.mouse_x, self.mouse_y);
                draw_card(
                    buf,
                    bw,
                    bh,
                    rr,
                    8,
                    if hovered { BG_HOVER } else { BG_CARD },
                    if hovered { BORDER_ACTIVE } else { BORDER_DEFAULT },
                );
                let kind_lbl = if ix.kind.is_empty() { "SWARM" } else { &ix.kind };
                let bw_badge = draw_badge(
                    buf,
                    bw,
                    bh,
                    &mut self.tp,
                    rr.x + 10,
                    rr.y + 12,
                    kind_lbl,
                    ACCENT_CYAN_DIM,
                    ACCENT_CYAN,
                );
                self.tp.draw(
                    buf,
                    bw,
                    bh,
                    rr.x + 18 + bw_badge,
                    rr.y + 7,
                    rr.w - bw_badge - 30,
                    13,
                    &ix.name,
                    TEXT_PRIMARY,
                    true,
                );
                let preview_url = ix.build_query_url(q);
                self.tp.draw(
                    buf,
                    bw,
                    bh,
                    rr.x + 18 + bw_badge,
                    rr.y + 25,
                    rr.w - bw_badge - 30,
                    11,
                    &preview_url,
                    TEXT_MUTED,
                    false,
                );
                self.hits.push(HitZone {
                    rect: rr,
                    action: HitAction::LaunchSwarmIndexer(i),
                });
                y += 54;
            }
        }

        // Featured Feeds from provider_pack.json.
        if y + 40 < bottom {
            y += 8;
            self.tp.draw(
                buf,
                bw,
                bh,
                x,
                y,
                w,
                14,
                "Featured Feeds",
                ACCENT_EMERALD,
                true,
            );
            y += 24;
            if self.pack.featured_feeds.is_empty() {
                self.tp.draw(
                    buf,
                    bw,
                    bh,
                    x,
                    y,
                    w,
                    12,
                    "No featured feeds configured in provider_pack.json.",
                    TEXT_MUTED,
                    false,
                );
            } else {
                for (i, feed) in self.pack.featured_feeds.iter().enumerate() {
                    if y + 42 > bottom {
                        break;
                    }
                    let rr = Rect::new(x, y, w, 40);
                    let hovered = self.detail.is_none() && rr.contains(self.mouse_x, self.mouse_y);
                    draw_card(
                        buf,
                        bw,
                        bh,
                        rr,
                        8,
                        if hovered { BG_HOVER } else { BG_CARD },
                        if hovered { BORDER_ACTIVE } else { BORDER_DEFAULT },
                    );
                    let bw_badge = draw_badge(
                        buf,
                        bw,
                        bh,
                        &mut self.tp,
                        rr.x + 10,
                        rr.y + 10,
                        "FEED",
                        ACCENT_EMERALD_DIM,
                        ACCENT_EMERALD,
                    );
                    self.tp.draw(
                        buf,
                        bw,
                        bh,
                        rr.x + 18 + bw_badge,
                        rr.y + 12,
                        rr.w / 2,
                        13,
                        &feed.label,
                        TEXT_PRIMARY,
                        true,
                    );
                    let short_u = truncate(&feed.url, 42);
                    let uw = self.tp.measure(&short_u, 11, false);
                    self.tp.draw(
                        buf,
                        bw,
                        bh,
                        rr.x + rr.w - uw - 12,
                        rr.y + 13,
                        uw + 6,
                        11,
                        &short_u,
                        TEXT_MUTED,
                        false,
                    );
                    self.hits.push(HitZone {
                        rect: rr,
                        action: HitAction::LaunchFeed(i),
                    });
                    y += 48;
                }
            }
        }
    }

    fn render_kodi(&mut self, buf: &mut [u32], bw: usize, bh: usize, area: Rect) {
        let pad = 18i32;
        let x = area.x + pad;
        let mut y = area.y + 16;
        let w = (area.w - pad * 2).max(200);

        self.tp
            .draw(buf, bw, bh, x, y, w - 220, 18, "Kodi Add-ons", TEXT_PRIMARY, true);

        let ref_r = Rect::new(x + w - 210, y - 2, 96, 28);
        let ref_hov = self.detail.is_none() && ref_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(buf, bw, bh, &mut self.tp, ref_r, "↻ Refresh", false, ref_hov);
        self.hits.push(HitZone {
            rect: ref_r,
            action: HitAction::RefreshKodi,
        });

        let open_r = Rect::new(x + w - 106, y - 2, 106, 28);
        let open_hov = self.detail.is_none() && open_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(buf, bw, bh, &mut self.tp, open_r, "Manage...", true, open_hov);
        self.hits.push(HitZone {
            rect: open_r,
            action: HitAction::LaunchKodiUi,
        });

        y += 30;
        self.tp
            .draw(buf, bw, bh, x, y, w, 12, &self.kodi_status, TEXT_MUTED, false);
        y += 24;

        let bottom = area.y + area.h - 10;
        if self.kodi.installed.is_empty() {
            let card = Rect::new(x, y, w, 86);
            draw_card(buf, bw, bh, card, 10, BG_CARD, BORDER_DEFAULT);
            self.tp.draw(
                buf,
                bw,
                bh,
                card.x + 16,
                card.y + 16,
                card.w - 32,
                14,
                "No Kodi add-ons installed in kodi/",
                TEXT_PRIMARY,
                true,
            );
            self.tp.draw(
                buf,
                bw,
                bh,
                card.x + 16,
                card.y + 42,
                card.w - 32,
                12,
                "Install a Kodi repository zip via `run +net qupid --add-repo <URL>` or click Manage... above.",
                TEXT_SECONDARY,
                false,
            );
            return;
        }

        for (id, ver) in &self.kodi.installed {
            if y + 44 > bottom {
                break;
            }
            let rr = Rect::new(x, y, w, 42);
            draw_card(buf, bw, bh, rr, 8, BG_CARD, BORDER_DEFAULT);
            let bw_badge = draw_badge(
                buf,
                bw,
                bh,
                &mut self.tp,
                rr.x + 10,
                rr.y + 11,
                "KODI",
                ACCENT_CYAN_DIM,
                ACCENT_CYAN,
            );
            self.tp.draw(
                buf,
                bw,
                bh,
                rr.x + 18 + bw_badge,
                rr.y + 13,
                rr.w - 140,
                13,
                id,
                TEXT_PRIMARY,
                true,
            );
            let v_lbl = format!("v{}", ver);
            let vw = self.tp.measure(&v_lbl, 11, false);
            self.tp.draw(
                buf,
                bw,
                bh,
                rr.x + rr.w - vw - 14,
                rr.y + 14,
                vw + 6,
                11,
                &v_lbl,
                ACCENT_EMERALD,
                false,
            );
            y += 48;
        }
    }

    fn render_library(&mut self, buf: &mut [u32], bw: usize, bh: usize, area: Rect) {
        let pad = 18i32;
        let x = area.x + pad;
        let mut y = area.y + 16;
        let w = (area.w - pad * 2).max(200);

        self.tp
            .draw(buf, bw, bh, x, y, w - 110, 18, "Local Media Library", TEXT_PRIMARY, true);
        let scan_r = Rect::new(x + w - 96, y - 2, 96, 28);
        let scan_hov = self.detail.is_none() && scan_r.contains(self.mouse_x, self.mouse_y);
        draw_pill(buf, bw, bh, &mut self.tp, scan_r, "↻ Rescan", false, scan_hov);
        self.hits.push(HitZone {
            rect: scan_r,
            action: HitAction::RescanLibrary,
        });

        y += 30;
        let sub = format!(
            "{} local media file(s) across /media/Ventoy, /data/media, /data/music, /Movies",
            self.library_files.len()
        );
        self.tp.draw(buf, bw, bh, x, y, w, 12, &sub, TEXT_MUTED, false);
        y += 24;

        let bottom = area.y + area.h - 10;
        if self.library_files.is_empty() {
            let card = Rect::new(x, y, w, 72);
            draw_card(buf, bw, bh, card, 10, BG_CARD, BORDER_DEFAULT);
            self.tp.draw(
                buf,
                bw,
                bh,
                card.x + 16,
                card.y + 26,
                card.w - 32,
                13,
                "No local video/audio files found. Mount Ventoy or place media in /data/media.",
                TEXT_SECONDARY,
                false,
            );
            return;
        }

        for (i, media) in self.library_files.iter().enumerate() {
            if y + 44 > bottom {
                break;
            }
            let rr = Rect::new(x, y, w, 42);
            let hovered = self.detail.is_none() && rr.contains(self.mouse_x, self.mouse_y);
            draw_card(
                buf,
                bw,
                bh,
                rr,
                8,
                if hovered { BG_HOVER } else { BG_CARD },
                if hovered { BORDER_ACTIVE } else { BORDER_DEFAULT },
            );
            let (lbl, bg, fg) = if media.is_video {
                ("VIDEO", ACCENT_CYAN_DIM, ACCENT_CYAN)
            } else {
                ("AUDIO", ACCENT_EMERALD_DIM, ACCENT_EMERALD)
            };
            let bw_badge = draw_badge(buf, bw, bh, &mut self.tp, rr.x + 10, rr.y + 11, lbl, bg, fg);
            self.tp.draw(
                buf,
                bw,
                bh,
                rr.x + 18 + bw_badge,
                rr.y + 13,
                rr.w - 170,
                13,
                &media.name,
                TEXT_PRIMARY,
                true,
            );
            let sz = format_size(media.size);
            let sw = self.tp.measure(&sz, 11, false);
            self.tp.draw(
                buf,
                bw,
                bh,
                rr.x + rr.w - sw - 14,
                rr.y + 14,
                sw + 6,
                11,
                &sz,
                TEXT_MUTED,
                false,
            );
            self.hits.push(HitZone {
                rect: rr,
                action: HitAction::PlayLibraryFile(i),
            });
            y += 48;
        }
    }
}

fn classify_media_ext(name: &str) -> Option<bool> {
    let lower = name.to_ascii_lowercase();
    for ext in [".mp4", ".mkv", ".webm", ".mov", ".avi", ".m4v", ".ts"] {
        if lower.ends_with(ext) {
            return Some(true);
        }
    }
    for ext in [".mp3", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".wav"] {
        if lower.ends_with(ext) {
            return Some(false);
        }
    }
    None
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{}.{} GiB", bytes >> 30, ((bytes >> 20) % 1024) * 10 / 1024)
    } else if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes >> 20)
    } else if bytes >= 1024 {
        format!("{} KiB", bytes >> 10)
    } else {
        format!("{} B", bytes)
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars || max_chars < 4 {
        return String::from(s);
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

extern "C" fn app_main() -> ! {
    // Reference sex-media crate so media capabilities remain linked cleanly.
    let _ = core::mem::size_of::<sex_media::Info>();

    let mut window = Window::open();
    window.title(b"BrootLite");
    sex_pdx::serial_record!(
        "[brootlite.ready] target={} task={}",
        window.target,
        window.task
    );

    let mut app = App::new(&mut window);
    app.render(&mut window);
    app.reload_discover(&mut window);

    loop {
        while let Some(ev) = window.poll() {
            match ev {
                Event::Close => {
                    sex_pdx::serial_record!(
                        "[brootlite.closed] target={} task={}",
                        window.target,
                        window.task
                    );
                    window.close();
                }
                Event::Resize { .. } => {
                    app.dirty = true;
                }
                Event::Pointer(p) => {
                    let prev = (app.mouse_x, app.mouse_y);
                    app.mouse_x = p.x as i32;
                    app.mouse_y = p.y as i32;
                    match p.kind {
                        PtrKind::Move => {
                            if prev != (app.mouse_x, app.mouse_y) {
                                app.dirty = true;
                            }
                        }
                        PtrKind::Down { button: 0 } => {
                            app.handle_click(&mut window, app.mouse_x, app.mouse_y);
                        }
                        PtrKind::Wheel { steps } => {
                            app.handle_wheel(steps);
                        }
                        PtrKind::Leave => {
                            app.mouse_x = -1;
                            app.mouse_y = -1;
                            app.dirty = true;
                        }
                        _ => {}
                    }
                }
                Event::Key(k) => {
                    app.handle_key(&mut window, k);
                }
                _ => {}
            }
        }

        if app.dirty {
            app.render(&mut window);
        }

        // Stream in 1 visible uncached poster thumbnail per idle tick.
        if !app.pump_one_poster(&mut window) {
            let _ = sex_pdx::sys_wait_self_ticks(16);
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_app::with_stack(512 * 1024, app_main)
}
