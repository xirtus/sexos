// SPDX-License-Identifier: GPL-3.0-only
//! Gaze Photo Library & Non-Destructive Shared-Batik Edit Studio UI.

#![forbid(unsafe_code)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use batik_core::encode_rgba_png;
use gaze_core::{
    apply_edits, extract_metadata, CropAspect, CropRect, EditRecipe, FilterNode, FilterSpec,
    PhotoLibrary, SharedFilterPreset, SidebarCollection, SmartQuery,
};
use sex_app::{files, Nav, Window};
use sex_image::{Image, Limits};
use sex_pdx::app_key::Key;
use sex_pdx::app_pointer::{Kind, Pointer};

use crate::canvas::{
    blit_image_fitted, draw_luma_histogram, fill_rect, fill_round_rect, stroke_rect,
    stroke_round_rect, FontPainter, HitControl, UiRect,
};

const CATALOG_FILE: &[u8] = b"library.gaze";
const MAX_READ_BYTES: usize = 8 * 1024 * 1024;
const THUMB_MAX_W: usize = 360;
const THUMB_MAX_H: usize = 240;

const PHOTO_LIMITS: Limits = Limits {
    max_width: 4096,
    max_height: 4096,
    max_pixels: 16_000_000,
    max_input_bytes: MAX_READ_BYTES,
};

const ENTER_CODE: u8 = 0x1C;
const BACKSPACE_CODE: u8 = 0x0E;
const ESC_CODE: u8 = 0x01;
const TAB_CODE: u8 = 0x0F;

// Studio color palette (Obsidian Slate + Electric Amber + Indigo + Emerald)
const COL_BG: u32 = 0x0B101A;
const COL_TOPBAR: u32 = 0x111927;
const COL_SIDEBAR: u32 = 0x0F1623;
const COL_CARD: u32 = 0x162032;
const COL_CARD_SEL: u32 = 0x1E2D47;
const COL_BORDER: u32 = 0x24324B;
const COL_TEXT: u32 = 0xF1F5F9;
const COL_MUTED: u32 = 0x94A3B8;
const COL_DIM: u32 = 0x64748B;
const COL_AMBER: u32 = 0xF59E0B;
const COL_AMBER_DARK: u32 = 0x422B08;
const COL_INDIGO: u32 = 0x6366F1;
const COL_INDIGO_DARK: u32 = 0x1E2258;
const COL_EMERALD: u32 = 0x10B981;
const COL_EMERALD_DARK: u32 = 0x08382B;
const COL_ROSE: u32 = 0xF43F5E;
const COL_ROSE_DARK: u32 = 0x451222;
const COL_BTN: u32 = 0x1B273D;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioScreen {
    Library,
    Edit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    None,
    Search,
    NewAlbum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOutcome {
    Stay,
    OpenViewer,
}

struct CachedThumb {
    id: u64,
    raw_thumb: Image,
    edited_thumb: Image,
}

pub struct GazeStudio {
    pub screen: StudioScreen,
    pub library: PhotoLibrary,
    pub collection: SidebarCollection,
    pub search_text: String,
    pub query: SmartQuery,
    pub input_mode: InputMode,
    pub album_draft: String,
    pub selected_idx: Option<usize>,
    pub active_album_idx: Option<usize>,
    pub compare_original: bool,
    pub status: String,
    pub frames: u64,
    pub painter: FontPainter,
    pub controls: Vec<HitControl>,
    thumbs: Vec<CachedThumb>,
    edit_source: Option<(String, Image)>,
    edit_preview: Option<Image>,
    io_buf: Vec<u8>,
}

impl GazeStudio {
    pub fn new() -> Self {
        Self {
            screen: StudioScreen::Library,
            library: PhotoLibrary::new(),
            collection: SidebarCollection::AllPhotos,
            search_text: String::new(),
            query: SmartQuery::default(),
            input_mode: InputMode::None,
            album_draft: String::new(),
            selected_idx: None,
            active_album_idx: None,
            compare_original: false,
            status: "Faces-Free Smart Library ready.".to_string(),
            frames: 0,
            painter: FontPainter::new(),
            controls: Vec::new(),
            thumbs: Vec::new(),
            edit_source: None,
            edit_preview: None,
            io_buf: Vec::new(),
        }
    }

    fn ensure_io_buf(&mut self, cap: usize) -> bool {
        if self.io_buf.len() < cap {
            if self.io_buf.try_reserve_exact(cap - self.io_buf.len()).is_err() {
                return false;
            }
            self.io_buf.resize(cap, 0);
        }
        true
    }

    /// Loads `library.gaze` from Gaze's private `/data/gaze/` folder if present.
    pub fn load_catalog(&mut self, window: &mut Window) {
        let Ok(st) = files::stat(window, CATALOG_FILE) else {
            return;
        };
        if st.dir || st.size == 0 || st.size as usize > 2 * 1024 * 1024 {
            return;
        }
        let need = st.size as usize;
        if !self.ensure_io_buf(need) {
            return;
        }
        if let Ok(n) = files::read(window, CATALOG_FILE, &mut self.io_buf[..need]) {
            if let Ok(text) = core::str::from_utf8(&self.io_buf[..n]) {
                self.library = PhotoLibrary::deserialize(text);
                if !self.library.albums.is_empty() && self.active_album_idx.is_none() {
                    self.active_album_idx = Some(0);
                }
            }
        }
    }

    /// Saves `library.gaze` deterministically to `/data/gaze/library.gaze`.
    pub fn persist_catalog(&self, window: &mut Window) {
        let text = self.library.serialize();
        if files::write(window, CATALOG_FILE, text.as_bytes()).is_ok() {
            let _ = files::sync(window);
            sex_pdx::serial_record!(
                "[gaze.persisted] bytes={} photos={} albums={}",
                text.len(),
                self.library.photos.len(),
                self.library.albums.len()
            );
        }
    }

    /// Scans granted folders/files plus Gaze's private `/data/gaze` & `/data/gaze/photos`.
    pub fn scan_sources(&mut self, window: &mut Window, extra_paths: &[Vec<u8>]) {
        let _ = files::mkdir(window, b"photos");
        let _ = files::mkdir(window, b"exports");

        for p in extra_paths {
            self.ingest_path(window, p);
        }
        self.ingest_folder(window, b"photos");
        self.ingest_folder(window, b"");

        let vis = self.library.visible_indices(&self.collection, &self.query);
        if self.selected_idx.is_none() || self.selected_idx.map_or(false, |i| i >= self.library.photos.len()) {
            self.selected_idx = vis.first().copied();
        }
        if !self.library.albums.is_empty() && self.active_album_idx.is_none() {
            self.active_album_idx = Some(0);
        }
    }

    pub fn ingest_path(&mut self, window: &mut Window, path: &[u8]) {
        let Ok(st) = files::stat(window, path) else {
            return;
        };
        if st.dir {
            self.ingest_folder(window, path);
        } else if sex_image::view::is_picture_name(path) {
            self.ingest_file(window, path, st.size, st.mtime);
        }
    }

    fn ingest_folder(&mut self, window: &mut Window, folder: &[u8]) {
        let Ok(mut entries) = files::list(window, folder) else {
            return;
        };
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            if entry.dir || !sex_image::view::is_picture_name(&entry.name) {
                continue;
            }
            let full = join_path(folder, &entry.name);
            let mtime = files::stat(window, &full).map(|s| s.mtime).unwrap_or(0);
            self.ingest_file(window, &full, entry.size, mtime);
        }
    }

    fn ingest_file(&mut self, window: &mut Window, path: &[u8], size: u64, mtime: u64) {
        if size == 0 || size as usize > MAX_READ_BYTES {
            return;
        }
        let Ok(path_str) = core::str::from_utf8(path) else {
            return;
        };
        let path_owned = path_str.to_string();
        let need = size as usize;
        if !self.ensure_io_buf(need) {
            return;
        }
        let Ok(n) = files::read(window, path, &mut self.io_buf[..need]) else {
            return;
        };
        let bytes = &self.io_buf[..n];
        let meta = extract_metadata(bytes, (mtime > 0).then_some(mtime));
        let decoded = gaze_codecs::decode(bytes, &PHOTO_LIMITS)
            .or_else(|_| sex_image::decode(bytes, &PHOTO_LIMITS).map_err(gaze_codecs::Error::Image));
        let Ok(full_img) = decoded else {
            return;
        };
        let raw_thumb = full_img
            .fit_within(THUMB_MAX_W, THUMB_MAX_H)
            .unwrap_or(Image {
                width: 1,
                height: 1,
                pixels: alloc::vec![0xFF000000],
            });

        let idx = self.library.upsert_photo(&path_owned, meta);
        let id = self.library.photos[idx].id;
        let recipe = self.library.photos[idx].recipe.clone();
        let edited_thumb = apply_edits(&raw_thumb, &recipe).unwrap_or_else(|_| raw_thumb.clone());

        if let Some(slot) = self.thumbs.iter_mut().find(|t| t.id == id) {
            slot.raw_thumb = raw_thumb;
            slot.edited_thumb = edited_thumb;
        } else {
            self.thumbs.push(CachedThumb {
                id,
                raw_thumb,
                edited_thumb,
            });
        }
    }

    pub fn selected_path(&self) -> Option<&str> {
        let idx = self.selected_idx?;
        self.library.photos.get(idx).map(|p| p.path.as_str())
    }

    pub fn recipe_for_path(&self, path: &[u8]) -> Option<&EditRecipe> {
        let s = core::str::from_utf8(path).ok()?;
        self.library.photo_by_path(s).map(|r| &r.recipe)
    }

    fn refresh_thumb_for(&mut self, idx: usize) {
        let Some(rec) = self.library.photos.get(idx) else {
            return;
        };
        let id = rec.id;
        let recipe = rec.recipe.clone();
        if let Some(slot) = self.thumbs.iter_mut().find(|t| t.id == id) {
            if let Ok(edited) = apply_edits(&slot.raw_thumb, &recipe) {
                slot.edited_thumb = edited;
            }
        }
    }

    fn thumb_for(&self, id: u64) -> Option<&Image> {
        self.thumbs.iter().find(|t| t.id == id).map(|t| &t.edited_thumb)
    }

    pub fn select_by_path(&mut self, path: &[u8]) {
        if let Ok(s) = core::str::from_utf8(path) {
            if let Some(pos) = self.library.photos.iter().position(|p| p.path == s) {
                self.selected_idx = Some(pos);
            }
        }
    }

    /// Opens the Non-Destructive Edit Studio on the currently selected photo.
    pub fn open_edit_studio(&mut self, window: &mut Window) -> bool {
        let Some(idx) = self.selected_idx else {
            return false;
        };
        let Some(path) = self.library.photos.get(idx).map(|p| p.path.clone()) else {
            return false;
        };
        let Ok(st) = files::stat(window, path.as_bytes()) else {
            return false;
        };
        if st.dir || st.size == 0 || st.size as usize > MAX_READ_BYTES {
            return false;
        }
        let need = st.size as usize;
        if !self.ensure_io_buf(need) {
            return false;
        }
        let Ok(n) = files::read(window, path.as_bytes(), &mut self.io_buf[..need]) else {
            return false;
        };
        let bytes = &self.io_buf[..n];
        let Ok(decoded) = gaze_codecs::decode(bytes, &PHOTO_LIMITS)
            .or_else(|_| sex_image::decode(bytes, &PHOTO_LIMITS).map_err(gaze_codecs::Error::Image))
        else {
            return false;
        };
        // Bound studio preview source to 1280x960 for instantaneous interactive slider response
        let studio_src = decoded.fit_within(1280, 960).unwrap_or_else(|_| Image {
            width: 1,
            height: 1,
            pixels: alloc::vec![0xFF000000],
        });
        let recipe = self.library.photos[idx].recipe.clone();
        let preview = apply_edits(&studio_src, &recipe).unwrap_or_else(|_| studio_src.clone());
        self.edit_source = Some((path.clone(), studio_src));
        self.edit_preview = Some(preview);
        self.compare_original = false;
        self.screen = StudioScreen::Edit;
        self.input_mode = InputMode::None;
        self.status = format!("Editing {} non-destructively (original untouched).", self.library.photos[idx].name);
        sex_pdx::serial_record!("[gaze.edit.opened] path={}", path);
        true
    }

    fn apply_recipe_change(&mut self, window: &mut Window, f: impl FnOnce(&mut EditRecipe, usize, usize)) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let (src_w, src_h) = self
            .edit_source
            .as_ref()
            .map(|(_, im)| (im.width, im.height))
            .unwrap_or_else(|| {
                let m = &self.library.photos[idx].meta;
                (m.width.max(1) as usize, m.height.max(1) as usize)
            });

        let mut recipe = self.library.photos[idx].recipe.clone();
        f(&mut recipe, src_w, src_h);
        let path = self.library.photos[idx].path.clone();
        self.library.set_recipe(&path, recipe.clone());
        self.refresh_thumb_for(idx);

        if let Some((ref src_path, ref src_img)) = self.edit_source {
            if *src_path == path {
                if let Ok(updated) = apply_edits(src_img, &recipe) {
                    self.edit_preview = Some(updated);
                }
            }
        }

        let enabled_stack = recipe.filter_stack.iter().filter(|n| n.enabled).count();
        sex_pdx::serial_record!(
            "[gaze.edit.updated] path={} crop={}x{} ev_x100={} filter={} stack={}",
            path,
            recipe.crop.w_permille,
            recipe.crop.h_permille,
            recipe.exposure_ev_x100,
            recipe.preset.slug(),
            enabled_stack
        );
        self.status = format!("Updated: {}", recipe.summary());
        self.persist_catalog(window);
    }

    fn reset_selected_edits(&mut self, window: &mut Window) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let path = self.library.photos[idx].path.clone();
        self.library.reset_recipe(&path);
        self.refresh_thumb_for(idx);
        if let Some((ref src_path, ref src_img)) = self.edit_source {
            if *src_path == path {
                self.edit_preview = Some(src_img.clone());
            }
        }
        self.status = "Reverted to original unedited photo.".to_string();
        sex_pdx::serial_record!("[gaze.edit.reset] path={}", path);
        self.persist_catalog(window);
    }

    fn export_edited_copy(&mut self, window: &mut Window) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let Some(ref preview) = self.edit_preview else {
            return;
        };
        let _ = files::mkdir(window, b"exports");
        let rec = &self.library.photos[idx];
        let stem = rec
            .name
            .rfind('.')
            .map(|d| &rec.name[..d])
            .unwrap_or(&rec.name);
        let export_rel = format!("exports/{}-edited.png", stem);
        if let Ok(png_bytes) = encode_rgba_png(&preview.pixels, preview.width, preview.height) {
            if files::write(window, export_rel.as_bytes(), &png_bytes).is_ok() {
                let _ = files::sync(window);
                self.status = format!("Exported flattened PNG copy to /data/gaze/{}", export_rel);
                sex_pdx::serial_record!(
                    "[gaze.export.saved] path=/data/gaze/{} bytes={}",
                    export_rel,
                    png_bytes.len()
                );
            }
        }
    }

    fn sync_selection_with_visible(&mut self) {
        let vis = self.library.visible_indices(&self.collection, &self.query);
        if let Some(cur) = self.selected_idx {
            if vis.contains(&cur) {
                return;
            }
        }
        self.selected_idx = vis.first().copied();
    }

    fn emit_search_telemetry(&self) {
        let vis = self.library.visible_indices(&self.collection, &self.query);
        sex_pdx::serial_record!(
            "[gaze.search] query=\"{}\" visible={} total={} biometric_refused={}",
            self.search_text,
            vis.len(),
            self.library.photos.len(),
            self.query.rejected_biometric as u8
        );
    }

    fn create_album_commit(&mut self, window: &mut Window, name: &str) {
        let clean = if name.trim().is_empty() {
            format!("Album {}", self.library.albums.len() + 1)
        } else {
            name.trim().to_string()
        };
        let idx = self.library.create_album(&clean);
        let actual_name = self.library.albums[idx].name.clone();
        self.active_album_idx = Some(idx);
        self.status = format!("Created album \"{}\" (press M to add/remove photos).", actual_name);
        sex_pdx::serial_record!("[gaze.album.created] idx={} name={}", idx, actual_name);
        self.persist_catalog(window);
    }

    fn toggle_selected_in_active_album(&mut self, window: &mut Window) {
        let Some(photo_idx) = self.selected_idx else {
            return;
        };
        if self.library.albums.is_empty() {
            self.create_album_commit(window, "Favorites Album");
        }
        let album_idx = match &self.collection {
            SidebarCollection::UserAlbum(i) => *i,
            _ => self.active_album_idx.unwrap_or(0),
        };
        if album_idx >= self.library.albums.len() {
            return;
        }
        let path = self.library.photos[photo_idx].path.clone();
        let photo_name = self.library.photos[photo_idx].name.clone();
        let album_name = self.library.albums[album_idx].name.clone();
        let now_member = self.library.toggle_photo_in_album(album_idx, &path);
        self.status = if now_member {
            format!("Added {} to album \"{}\".", photo_name, album_name)
        } else {
            format!("Removed {} from album \"{}\".", photo_name, album_name)
        };
        sex_pdx::serial_record!(
            "[gaze.album.toggled] album={} path={} member={}",
            album_name,
            path,
            now_member as u8
        );
        self.sync_selection_with_visible();
        self.persist_catalog(window);
    }

    pub fn handle_key(&mut self, window: &mut Window, key: Key) -> StudioOutcome {
        // 1. Handle inline text input modes (Search / NewAlbum)
        match self.input_mode {
            InputMode::Search => {
                if key.code == ESC_CODE {
                    self.input_mode = InputMode::None;
                    if !self.search_text.is_empty() {
                        self.search_text.clear();
                        self.query = SmartQuery::default();
                        self.sync_selection_with_visible();
                        self.status = "Cleared smart search.".to_string();
                        self.emit_search_telemetry();
                    }
                    return StudioOutcome::Stay;
                }
                if key.code == ENTER_CODE {
                    self.input_mode = InputMode::None;
                    self.emit_search_telemetry();
                    return StudioOutcome::Stay;
                }
                if key.code == BACKSPACE_CODE {
                    self.search_text.pop();
                    self.query = SmartQuery::parse(&self.search_text);
                    self.sync_selection_with_visible();
                    self.emit_search_telemetry();
                    return StudioOutcome::Stay;
                }
                if let Some(ch) = sex_app::text(key) {
                    if !ch.is_control() && self.search_text.len() < 64 {
                        self.search_text.push(ch);
                        self.query = SmartQuery::parse(&self.search_text);
                        self.sync_selection_with_visible();
                        self.status = if self.query.rejected_biometric {
                            "Faces-Free Privacy: biometric face/person search is disabled.".to_string()
                        } else {
                            format!("Smart search: \"{}\"", self.search_text)
                        };
                        self.emit_search_telemetry();
                    }
                }
                return StudioOutcome::Stay;
            }
            InputMode::NewAlbum => {
                if key.code == ESC_CODE {
                    self.input_mode = InputMode::None;
                    self.album_draft.clear();
                    return StudioOutcome::Stay;
                }
                if key.code == ENTER_CODE {
                    let draft = self.album_draft.clone();
                    self.album_draft.clear();
                    self.input_mode = InputMode::None;
                    self.create_album_commit(window, &draft);
                    return StudioOutcome::Stay;
                }
                if key.code == BACKSPACE_CODE {
                    self.album_draft.pop();
                    return StudioOutcome::Stay;
                }
                if let Some(ch) = sex_app::text(key) {
                    if !ch.is_control() && self.album_draft.len() < 32 {
                        self.album_draft.push(ch);
                    }
                }
                return StudioOutcome::Stay;
            }
            InputMode::None => {}
        }

        // 2. Edit Studio shortcuts
        if self.screen == StudioScreen::Edit {
            if key.code == ESC_CODE || key.code == TAB_CODE {
                self.screen = StudioScreen::Library;
                self.compare_original = false;
                return StudioOutcome::Stay;
            }
            match sex_app::typed(key) {
                Some(b'l' | b'L') => {
                    self.screen = StudioScreen::Library;
                    self.compare_original = false;
                }
                Some(b'\\') => {
                    self.compare_original = !self.compare_original;
                }
                Some(b'0') => {
                    self.reset_selected_edits(window);
                }
                Some(b's' | b'S') => {
                    self.export_edited_copy(window);
                }
                Some(b']') => {
                    self.apply_recipe_change(window, |r, _, _| {
                        r.exposure_ev_x100 = (r.exposure_ev_x100 + 25).clamp(-200, 200);
                    });
                }
                Some(b'[') => {
                    self.apply_recipe_change(window, |r, _, _| {
                        r.exposure_ev_x100 = (r.exposure_ev_x100 - 25).clamp(-200, 200);
                    });
                }
                Some(b'w' | b'W') => {
                    self.apply_recipe_change(window, |r, w, h| {
                        r.aspect = CropAspect::SixteenNine;
                        r.crop = CropRect::from_aspect(CropAspect::SixteenNine, w, h, 0);
                    });
                }
                Some(b'c' | b'C') => {
                    self.apply_recipe_change(window, |r, _, _| {
                        r.crop = r.crop.zoom_in(80);
                    });
                }
                Some(b'r' | b'R') => {
                    let delta = if key.shift { 3 } else { 1 };
                    self.apply_recipe_change(window, |r, _, _| {
                        r.quarter_turns = (r.quarter_turns + delta) % 4;
                    });
                }
                Some(b'h' | b'H') => {
                    self.apply_recipe_change(window, |r, _, _| {
                        r.flip_h = !r.flip_h;
                    });
                }
                Some(b'f' | b'F') => {
                    self.apply_recipe_change(window, |r, _, _| {
                        r.preset = r.preset.next();
                    });
                }
                Some(b'+' | b'=') => {
                    self.apply_recipe_change(window, |r, _, _| {
                        if r.filter_stack.len() < 8 {
                            r.filter_stack
                                .push(FilterNode::new(FilterSpec::Sharpen { amount_pct: 75 }));
                        }
                    });
                }
                _ => {}
            }
            return StudioOutcome::Stay;
        }

        // 3. Library Mode navigation & shortcuts
        if key.code == ESC_CODE {
            if !self.search_text.is_empty() || self.collection != SidebarCollection::AllPhotos {
                self.search_text.clear();
                self.query = SmartQuery::default();
                self.collection = SidebarCollection::AllPhotos;
                self.sync_selection_with_visible();
                self.status = "Showing all photos.".to_string();
                self.emit_search_telemetry();
            }
            return StudioOutcome::Stay;
        }

        if key.code == ENTER_CODE {
            if self.selected_idx.is_some() {
                return StudioOutcome::OpenViewer;
            }
        }

        let vis = self.library.visible_indices(&self.collection, &self.query);
        if let Some(nav) = sex_app::nav(key) {
            if !vis.is_empty() {
                let pos = self
                    .selected_idx
                    .and_then(|s| vis.iter().position(|&v| v == s))
                    .unwrap_or(0);
                let cols = if vis.len() <= 4 { 2 } else { 3 };
                let next_pos = match nav {
                    Nav::Left => pos.saturating_sub(1),
                    Nav::Right => (pos + 1).min(vis.len() - 1),
                    Nav::Up => pos.saturating_sub(cols),
                    Nav::Down => (pos + cols).min(vis.len() - 1),
                    Nav::Home => 0,
                    Nav::End => vis.len() - 1,
                    Nav::Delete => pos,
                };
                let chosen = vis[next_pos];
                self.selected_idx = Some(chosen);
                sex_pdx::serial_record!(
                    "[gaze.selected] idx={} name={} path={}",
                    chosen,
                    self.library.photos[chosen].name,
                    self.library.photos[chosen].path
                );
            }
            return StudioOutcome::Stay;
        }

        match sex_app::typed(key) {
            Some(b'/') => {
                self.input_mode = InputMode::Search;
                self.status = "Type smart search (e.g. 2026-05, Paris, Japan, Europe) + Enter".to_string();
            }
            Some(b'a' | b'A') => {
                self.input_mode = InputMode::NewAlbum;
                self.album_draft.clear();
                self.status = "Type new album name and press Enter.".to_string();
            }
            Some(b'm' | b'M') => {
                self.toggle_selected_in_active_album(window);
            }
            Some(b'f' | b'F') => {
                if let Some(idx) = self.selected_idx {
                    let path = self.library.photos[idx].path.clone();
                    let fav = self.library.toggle_favorite(&path);
                    self.status = if fav {
                        format!("Favorited {}.", self.library.photos[idx].name)
                    } else {
                        format!("Unfavorited {}.", self.library.photos[idx].name)
                    };
                    self.sync_selection_with_visible();
                    self.persist_catalog(window);
                }
            }
            Some(b'e' | b'E') => {
                let _ = self.open_edit_studio(window);
            }
            Some(b'v' | b'V') => {
                if self.selected_idx.is_some() {
                    return StudioOutcome::OpenViewer;
                }
            }
            Some(b'1'..=b'9') => {
                let n = (sex_app::typed(key).unwrap() - b'1') as usize;
                if let Some(album) = self.library.albums.get(n) {
                    let a_name = album.name.clone();
                    self.collection = SidebarCollection::UserAlbum(n);
                    self.active_album_idx = Some(n);
                    self.sync_selection_with_visible();
                    let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                    self.status = format!("Album \"{}\" ({} photos)", a_name, vis_n);
                    sex_pdx::serial_record!(
                        "[gaze.album.selected] name={} visible={}",
                        a_name,
                        vis_n
                    );
                }
            }
            Some(b'0') => {
                self.collection = SidebarCollection::AllPhotos;
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                sex_pdx::serial_record!(
                    "[gaze.album.selected] name=AllPhotos visible={}",
                    vis_n
                );
            }
            _ => {}
        }

        StudioOutcome::Stay
    }

    pub fn handle_pointer(&mut self, window: &mut Window, p: Pointer) -> StudioOutcome {
        let Kind::Down { button: 0 } = p.kind else {
            return StudioOutcome::Stay;
        };
        let px = p.x as i32;
        let py = p.y as i32;

        // Search in reverse so inner buttons (e.g. Clear inside search box, Delete inside album row) win
        let hit = self
            .controls
            .iter()
            .rev()
            .find(|c| c.rect.contains(px, py))
            .map(|c| c.name.clone());

        let Some(action) = hit else {
            if self.input_mode != InputMode::None {
                self.input_mode = InputMode::None;
            }
            return StudioOutcome::Stay;
        };

        self.dispatch_control(window, &action)
    }

    fn dispatch_control(&mut self, window: &mut Window, name: &str) -> StudioOutcome {
        match name {
            "search_box" => {
                self.input_mode = InputMode::Search;
                self.status = "Type smart search (date, city, country, album) and press Enter.".to_string();
            }
            "btn_clear_search" => {
                self.search_text.clear();
                self.query = SmartQuery::default();
                self.input_mode = InputMode::None;
                self.sync_selection_with_visible();
                self.status = "Cleared smart search.".to_string();
                self.emit_search_telemetry();
            }
            "btn_new_album" => {
                self.input_mode = InputMode::NewAlbum;
                self.album_draft.clear();
                self.status = "Type new album name and press Enter.".to_string();
            }
            "btn_toggle_album" => {
                self.toggle_selected_in_active_album(window);
            }
            "btn_fav" => {
                if let Some(idx) = self.selected_idx {
                    let path = self.library.photos[idx].path.clone();
                    let _ = self.library.toggle_favorite(&path);
                    self.sync_selection_with_visible();
                    self.persist_catalog(window);
                }
            }
            "btn_edit" => {
                let _ = self.open_edit_studio(window);
            }
            "btn_viewer" => {
                if self.selected_idx.is_some() {
                    return StudioOutcome::OpenViewer;
                }
            }
            "nav_all" => {
                self.collection = SidebarCollection::AllPhotos;
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                sex_pdx::serial_record!("[gaze.album.selected] name=AllPhotos visible={}", vis_n);
            }
            "nav_favorites" => {
                self.collection = SidebarCollection::Favorites;
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                sex_pdx::serial_record!("[gaze.album.selected] name=Favorites visible={}", vis_n);
            }
            "nav_edited" => {
                self.collection = SidebarCollection::Edited;
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                sex_pdx::serial_record!("[gaze.album.selected] name=Edited visible={}", vis_n);
            }
            // Edit Studio controls
            "btn_back_library" => {
                self.screen = StudioScreen::Library;
                self.compare_original = false;
            }
            "btn_compare" => {
                self.compare_original = !self.compare_original;
            }
            "btn_reset_edits" => {
                self.reset_selected_edits(window);
            }
            "btn_export_copy" => {
                self.export_edited_copy(window);
            }
            "crop_free" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.aspect = CropAspect::Free;
                    r.crop = CropRect::FULL;
                });
            }
            "crop_1x1" => {
                self.apply_recipe_change(window, |r, w, h| {
                    r.aspect = CropAspect::Square1x1;
                    r.crop = CropRect::from_aspect(CropAspect::Square1x1, w, h, 0);
                });
            }
            "crop_4x3" => {
                self.apply_recipe_change(window, |r, w, h| {
                    r.aspect = CropAspect::FourThree;
                    r.crop = CropRect::from_aspect(CropAspect::FourThree, w, h, 0);
                });
            }
            "crop_16x9" => {
                self.apply_recipe_change(window, |r, w, h| {
                    r.aspect = CropAspect::SixteenNine;
                    r.crop = CropRect::from_aspect(CropAspect::SixteenNine, w, h, 0);
                });
            }
            "crop_zoom_in" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.crop = r.crop.zoom_in(80);
                });
            }
            "crop_reset" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.aspect = CropAspect::Free;
                    r.crop = CropRect::FULL;
                });
            }
            "rot_cw" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.quarter_turns = (r.quarter_turns + 1) % 4;
                });
            }
            "flip_h" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.flip_h = !r.flip_h;
                });
            }
            "ev_dec" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.exposure_ev_x100 = (r.exposure_ev_x100 - 25).clamp(-200, 200);
                });
            }
            "ev_inc" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.exposure_ev_x100 = (r.exposure_ev_x100 + 25).clamp(-200, 200);
                });
            }
            "con_dec" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.contrast_pct = (r.contrast_pct - 15).clamp(-90, 90);
                });
            }
            "con_inc" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.contrast_pct = (r.contrast_pct + 15).clamp(-90, 90);
                });
            }
            "warm_dec" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.warmth_pct = (r.warmth_pct - 15).clamp(-90, 90);
                });
            }
            "warm_inc" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.warmth_pct = (r.warmth_pct + 15).clamp(-90, 90);
                });
            }
            "sat_dec" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.saturation_pct = (r.saturation_pct - 15).clamp(-90, 90);
                });
            }
            "sat_inc" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.saturation_pct = (r.saturation_pct + 15).clamp(-90, 90);
                });
            }
            "stack_add_sharpen" => {
                self.apply_recipe_change(window, |r, _, _| {
                    if r.filter_stack.len() < 8 {
                        r.filter_stack
                            .push(FilterNode::new(FilterSpec::Sharpen { amount_pct: 75 }));
                    }
                });
            }
            "stack_clear" => {
                self.apply_recipe_change(window, |r, _, _| {
                    r.filter_stack.clear();
                });
            }
            other => {
                if let Some(slug) = other.strip_prefix("preset_") {
                    let preset = SharedFilterPreset::from_slug(slug);
                    self.apply_recipe_change(window, |r, _, _| {
                        r.preset = preset;
                    });
                } else if let Some(idx_s) = other.strip_prefix("album_") {
                    if let Ok(idx) = idx_s.parse::<usize>() {
                        if let Some(album) = self.library.albums.get(idx) {
                            let a_name = album.name.clone();
                            self.collection = SidebarCollection::UserAlbum(idx);
                            self.active_album_idx = Some(idx);
                            self.sync_selection_with_visible();
                            let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                            sex_pdx::serial_record!(
                                "[gaze.album.selected] name={} visible={}",
                                a_name,
                                vis_n
                            );
                        }
                    }
                } else if let Some(idx_s) = other.strip_prefix("del_album_") {
                    if let Ok(idx) = idx_s.parse::<usize>() {
                        if self.library.delete_album(idx) {
                            self.collection = SidebarCollection::AllPhotos;
                            self.active_album_idx = (!self.library.albums.is_empty()).then_some(0);
                            self.sync_selection_with_visible();
                            self.persist_catalog(window);
                        }
                    }
                } else if let Some(place) = other.strip_prefix("place:") {
                    self.collection = SidebarCollection::PlaceFilter(place.to_string());
                    self.sync_selection_with_visible();
                    let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                    sex_pdx::serial_record!("[gaze.album.selected] name=Place:{} visible={}", place, vis_n);
                } else if let Some(year_s) = other.strip_prefix("year_") {
                    if let Ok(y) = year_s.parse::<u16>() {
                        self.collection = SidebarCollection::YearFilter(y);
                        self.sync_selection_with_visible();
                        let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                        sex_pdx::serial_record!("[gaze.album.selected] name=Year:{} visible={}", y, vis_n);
                    }
                } else if let Some(pidx_s) = other.strip_prefix("photo_idx_") {
                    if let Ok(pidx) = pidx_s.parse::<usize>() {
                        if pidx < self.library.photos.len() {
                            self.selected_idx = Some(pidx);
                            sex_pdx::serial_record!(
                                "[gaze.selected] idx={} name={} path={}",
                                pidx,
                                self.library.photos[pidx].name,
                                self.library.photos[pidx].path
                            );
                        }
                    }
                }
            }
        }
        StudioOutcome::Stay
    }

    fn register_control(&mut self, name: &str, rect: UiRect) {
        self.controls.push(HitControl {
            name: name.to_string(),
            rect,
        });
    }

    pub fn render(&mut self, window: &mut Window) {
        let (aw, ah) = window.size();
        let (w, h) = (aw.max(1), ah.max(1));
        let fb = window.pixels();
        if fb.len() < w * h {
            return;
        }
        self.controls.clear();

        match self.screen {
            StudioScreen::Library => self.render_library(fb, w, h),
            StudioScreen::Edit => self.render_edit_studio(fb, w, h),
        }

        window.show_pixels(w, h);
        self.frames += 1;

        for c in &self.controls {
            let (cx, cy) = c.rect.center();
            sex_pdx::serial_record!("[gaze.control] name={} x={} y={}", c.name, cx, cy);
        }
        let mode_str = match self.screen {
            StudioScreen::Library => "library",
            StudioScreen::Edit => "edit",
        };
        sex_pdx::serial_record!("[gaze.frame] n={} mode={}", self.frames, mode_str);
    }

    fn draw_button(
        &mut self,
        fb: &mut [u32],
        w: usize,
        h: usize,
        id: &str,
        rect: UiRect,
        label: &str,
        bg: u32,
        border: u32,
        fg: u32,
        bold: bool,
    ) {
        stroke_round_rect(fb, w, h, rect, 6, 1, border, bg);
        let tw = self.painter.measure_text(label, 12, false);
        let tx = (rect.x + (rect.w - tw) / 2).max(rect.x + 5);
        let ty = rect.y + (rect.h - 13) / 2;
        self.painter
            .draw_text_clipped(fb, w, h, tx, ty, label, 12, fg, false, bold, rect.inset(4, 2));
        self.register_control(id, rect);
    }

    fn render_library(&mut self, fb: &mut [u32], w: usize, h: usize) {
        fill_rect(fb, w, h, UiRect::new(0, 0, w as i32, h as i32), COL_BG);

        let top_h = 50i32;
        fill_rect(fb, w, h, UiRect::new(0, 0, w as i32, top_h), COL_TOPBAR);
        fill_rect(fb, w, h, UiRect::new(0, top_h - 1, w as i32, 1), COL_BORDER);

        // Brand + Faces-Free badge
        let brand_clip = UiRect::new(14, 8, 160, 34);
        let nx = self.painter.draw_text_clipped(
            fb,
            w,
            h,
            14,
            15,
            "GAZE",
            16,
            COL_AMBER,
            false,
            true,
            brand_clip,
        );
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            nx + 8,
            17,
            "LIBRARY",
            12,
            COL_MUTED,
            false,
            true,
            UiRect::new(nx + 6, 10, 80, 28),
        );

        let ff_rect = UiRect::new(142, 13, 86, 22);
        stroke_round_rect(fb, w, h, ff_rect, 10, 1, COL_EMERALD, COL_EMERALD_DARK);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            ff_rect.x + 8,
            ff_rect.y + 5,
            "FACES-FREE",
            10,
            COL_EMERALD,
            false,
            true,
            ff_rect.inset(4, 2),
        );

        // Right toolbar buttons
        let mut bx = w as i32 - 12;
        let btn_y = 10;
        let btn_h = 28;

        bx -= 58;
        self.draw_button(
            fb,
            w,
            h,
            "btn_viewer",
            UiRect::new(bx, btn_y, 58, btn_h),
            "View",
            COL_BTN,
            COL_BORDER,
            COL_TEXT,
            false,
        );
        bx -= 80;
        self.draw_button(
            fb,
            w,
            h,
            "btn_edit",
            UiRect::new(bx, btn_y, 74, btn_h),
            "Edit [E]",
            COL_INDIGO_DARK,
            COL_INDIGO,
            COL_TEXT,
            true,
        );
        bx -= 68;
        let is_fav = self
            .selected_idx
            .and_then(|i| self.library.photos.get(i))
            .map_or(false, |p| p.favorite);
        self.draw_button(
            fb,
            w,
            h,
            "btn_fav",
            UiRect::new(bx, btn_y, 62, btn_h),
            if is_fav { "Fav *" } else { "Fav [F]" },
            if is_fav { COL_ROSE_DARK } else { COL_BTN },
            if is_fav { COL_ROSE } else { COL_BORDER },
            if is_fav { COL_ROSE } else { COL_TEXT },
            is_fav,
        );
        bx -= 92;
        self.draw_button(
            fb,
            w,
            h,
            "btn_toggle_album",
            UiRect::new(bx, btn_y, 86, btn_h),
            "In Album [M]",
            COL_BTN,
            COL_BORDER,
            COL_TEXT,
            false,
        );
        bx -= 86;
        self.draw_button(
            fb,
            w,
            h,
            "btn_new_album",
            UiRect::new(bx, btn_y, 80, btn_h),
            "+ Album [A]",
            COL_AMBER_DARK,
            COL_AMBER,
            COL_AMBER,
            true,
        );

        // Center Smart Search Bar
        let search_x = 238;
        let search_w = (bx - 12 - search_x).max(140);
        let search_rect = UiRect::new(search_x, 9, search_w, 30);
        let search_focused = self.input_mode == InputMode::Search;
        let s_border = if self.query.rejected_biometric {
            COL_ROSE
        } else if search_focused {
            COL_AMBER
        } else {
            COL_BORDER
        };
        stroke_round_rect(fb, w, h, search_rect, 7, 1, s_border, 0x0A0F18);
        self.register_control("search_box", search_rect);

        let vis = self.library.visible_indices(&self.collection, &self.query);
        let count_tag = format!("{}/{}", vis.len(), self.library.photos.len());
        let count_w = self.painter.measure_text(&count_tag, 11, true) + 14;
        let clear_space = if !self.search_text.is_empty() { 48 } else { 0 };

        let text_clip = UiRect::new(
            search_rect.x + 10,
            search_rect.y + 4,
            (search_rect.w - count_w - clear_space - 18).max(20),
            22,
        );
        if self.search_text.is_empty() && !search_focused {
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                search_rect.x + 10,
                search_rect.y + 8,
                "[/] Search date or place (2026-05, Paris, Tokyo, Europe)...",
                12,
                COL_DIM,
                false,
                false,
                text_clip,
            );
        } else {
            let shown = if search_focused {
                format!("{}|", self.search_text)
            } else {
                self.search_text.clone()
            };
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                search_rect.x + 10,
                search_rect.y + 8,
                &shown,
                12,
                COL_TEXT,
                false,
                false,
                text_clip,
            );
        }

        if !self.search_text.is_empty() {
            let clr_rect = UiRect::new(search_rect.x + search_rect.w - count_w - 46, search_rect.y + 4, 40, 22);
            self.draw_button(
                fb,
                w,
                h,
                "btn_clear_search",
                clr_rect,
                "Clear",
                0x1F293D,
                COL_BORDER,
                COL_MUTED,
                false,
            );
        }

        let badge_rect = UiRect::new(
            search_rect.x + search_rect.w - count_w - 4,
            search_rect.y + 5,
            count_w,
            20,
        );
        fill_round_rect(fb, w, h, badge_rect, 5, 0x182234);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            badge_rect.x + 7,
            badge_rect.y + 4,
            &count_tag,
            11,
            COL_MUTED,
            true,
            false,
            badge_rect,
        );

        // Main 3-column body: Left Sidebar | Center Contact Grid | Right Inspector
        let body_y = top_h;
        let body_h = (h as i32 - body_y).max(1);
        let side_w = if w >= 720 { 216i32 } else { 164i32 };
        let insp_w = if w >= 920 { 258i32 } else { 0i32 };

        // Left Sidebar
        let side_rect = UiRect::new(0, body_y, side_w, body_h);
        fill_rect(fb, w, h, side_rect, COL_SIDEBAR);
        fill_rect(fb, w, h, UiRect::new(side_w - 1, body_y, 1, body_h), COL_BORDER);
        self.render_sidebar(fb, w, h, side_rect);

        // Right Inspector
        if insp_w > 0 {
            let insp_rect = UiRect::new(w as i32 - insp_w, body_y, insp_w, body_h);
            fill_rect(fb, w, h, insp_rect, COL_SIDEBAR);
            fill_rect(fb, w, h, UiRect::new(insp_rect.x, body_y, 1, body_h), COL_BORDER);
            self.render_inspector(fb, w, h, insp_rect);
        }

        // Center Contact Grid
        let grid_rect = UiRect::new(side_w, body_y, (w as i32 - side_w - insp_w).max(80), body_h);
        self.render_photo_grid(fb, w, h, grid_rect, &vis);
    }

    fn render_sidebar(&mut self, fb: &mut [u32], w: usize, h: usize, area: UiRect) {
        let mut y = area.y + 12;
        let row_w = area.w - 20;
        let x = area.x + 10;

        let draw_section_hdr = |painter: &mut FontPainter, fb: &mut [u32], yy: i32, title: &str| {
            painter.draw_text_clipped(
                fb,
                w,
                h,
                x + 4,
                yy,
                title,
                10,
                COL_DIM,
                false,
                true,
                UiRect::new(x, yy - 2, row_w, 16),
            );
        };

        draw_section_hdr(&mut self.painter, fb, y, "LIBRARY");
        y += 18;

        let nav_items = [
            (
                "nav_all",
                "All Photos",
                self.library.photos.len(),
                self.collection == SidebarCollection::AllPhotos,
            ),
            (
                "nav_favorites",
                "Favorites",
                self.library.favorites_count(),
                self.collection == SidebarCollection::Favorites,
            ),
            (
                "nav_edited",
                "Edited Non-Destructive",
                self.library.edited_count(),
                self.collection == SidebarCollection::Edited,
            ),
        ];

        for (id, title, cnt, active) in nav_items {
            let r = UiRect::new(x, y, row_w, 26);
            if active {
                stroke_round_rect(fb, w, h, r, 6, 1, COL_AMBER, COL_CARD_SEL);
            }
            let fg = if active { COL_AMBER } else { COL_TEXT };
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                r.x + 10,
                r.y + 6,
                title,
                12,
                fg,
                false,
                active,
                UiRect::new(r.x + 8, r.y + 2, r.w - 42, 22),
            );
            let cnt_s = format!("{}", cnt);
            let cw = self.painter.measure_text(&cnt_s, 11, true);
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                r.x + r.w - cw - 8,
                r.y + 7,
                &cnt_s,
                11,
                COL_MUTED,
                true,
                false,
                r,
            );
            self.register_control(id, r);
            y += 29;
        }

        // SMART PLACES
        y += 10;
        draw_section_hdr(&mut self.painter, fb, y, "SMART PLACES (OFFLINE)");
        y += 18;
        let places = self.library.top_places(4);
        if places.is_empty() {
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                x + 8,
                y,
                "No GPS tags yet",
                11,
                COL_DIM,
                false,
                false,
                UiRect::new(x, y, row_w, 18),
            );
            y += 22;
        } else {
            for (place, cnt) in places {
                let active = self.collection == SidebarCollection::PlaceFilter(place.clone());
                let r = UiRect::new(x, y, row_w, 24);
                if active {
                    stroke_round_rect(fb, w, h, r, 5, 1, COL_INDIGO, COL_INDIGO_DARK);
                }
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    r.x + 10,
                    r.y + 5,
                    &place,
                    11,
                    if active { COL_TEXT } else { COL_MUTED },
                    false,
                    active,
                    UiRect::new(r.x + 8, r.y + 2, r.w - 34, 20),
                );
                let cnt_s = format!("{}", cnt);
                let cw = self.painter.measure_text(&cnt_s, 11, true);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    r.x + r.w - cw - 8,
                    r.y + 6,
                    &cnt_s,
                    11,
                    COL_DIM,
                    true,
                    false,
                    r,
                );
                let ctrl_name = format!("place:{}", place);
                self.register_control(&ctrl_name, r);
                y += 26;
            }
        }

        // SMART DATES (Year pills)
        y += 8;
        draw_section_hdr(&mut self.painter, fb, y, "SMART DATES");
        y += 18;
        let years = self.library.top_years(3);
        if !years.is_empty() {
            let mut px = x;
            for (yr, cnt) in years {
                let active = self.collection == SidebarCollection::YearFilter(yr);
                let pill_w = (row_w - 8) / 2;
                if px + pill_w > x + row_w {
                    px = x;
                    y += 26;
                }
                let r = UiRect::new(px, y, pill_w, 22);
                stroke_round_rect(
                    fb,
                    w,
                    h,
                    r,
                    5,
                    1,
                    if active { COL_AMBER } else { COL_BORDER },
                    if active { COL_AMBER_DARK } else { COL_CARD },
                );
                let lbl = format!("{} ({})", yr, cnt);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    r.x + 8,
                    r.y + 5,
                    &lbl,
                    11,
                    if active { COL_AMBER } else { COL_TEXT },
                    true,
                    active,
                    r.inset(4, 2),
                );
                self.register_control(&format!("year_{}", yr), r);
                px += pill_w + 6;
            }
            y += 30;
        }

        // USER ALBUMS
        y += 6;
        draw_section_hdr(&mut self.painter, fb, y, "ALBUMS");
        y += 18;

        if self.input_mode == InputMode::NewAlbum {
            let inp_r = UiRect::new(x, y, row_w, 26);
            stroke_round_rect(fb, w, h, inp_r, 5, 1, COL_AMBER, 0x0A0F18);
            let txt = format!("Name: {}|", self.album_draft);
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                inp_r.x + 8,
                inp_r.y + 6,
                &txt,
                11,
                COL_AMBER,
                false,
                true,
                inp_r.inset(4, 2),
            );
            y += 30;
        }

        if self.library.albums.is_empty() && self.input_mode != InputMode::NewAlbum {
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                x + 8,
                y,
                "Press [A] or [+ Album]",
                11,
                COL_DIM,
                false,
                false,
                UiRect::new(x, y, row_w, 20),
            );
        } else {
            let albums_snapshot: Vec<(usize, String, usize)> = self
                .library
                .albums
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.name.clone(), a.photo_paths.len()))
                .collect();

            for (idx, a_name, cnt) in albums_snapshot {
                if y + 26 > area.y + area.h - 24 {
                    break;
                }
                let active = self.collection == SidebarCollection::UserAlbum(idx);
                let r = UiRect::new(x, y, row_w, 26);
                stroke_round_rect(
                    fb,
                    w,
                    h,
                    r,
                    6,
                    1,
                    if active { COL_AMBER } else { 0x1C273B },
                    if active { COL_CARD_SEL } else { 0x131C2D },
                );
                let lbl = format!("{} ({})", a_name, cnt);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    r.x + 10,
                    r.y + 6,
                    &lbl,
                    12,
                    if active { COL_AMBER } else { COL_TEXT },
                    false,
                    active,
                    UiRect::new(r.x + 8, r.y + 2, r.w - 34, 22),
                );
                self.register_control(&format!("album_{}", idx), r);

                let del_r = UiRect::new(r.x + r.w - 22, r.y + 4, 18, 18);
                fill_round_rect(fb, w, h, del_r, 4, 0x1F2B40);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    del_r.x + 5,
                    del_r.y + 2,
                    "x",
                    11,
                    COL_DIM,
                    true,
                    false,
                    del_r,
                );
                self.register_control(&format!("del_album_{}", idx), del_r);
                y += 30;
            }
        }
    }

    fn render_photo_grid(&mut self, fb: &mut [u32], w: usize, h: usize, area: UiRect, vis: &[usize]) {
        // Subheader bar
        let hdr_h = 34i32;
        fill_rect(fb, w, h, UiRect::new(area.x, area.y, area.w, hdr_h), 0x0E1522);
        fill_rect(
            fb,
            w,
            h,
            UiRect::new(area.x, area.y + hdr_h - 1, area.w, 1),
            COL_BORDER,
        );

        let col_title = self.collection.label(&self.library.albums);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            area.x + 16,
            area.y + 9,
            &col_title,
            13,
            COL_TEXT,
            false,
            true,
            UiRect::new(area.x + 14, area.y + 4, area.w / 2, 24),
        );

        let right_info = if self.query.rejected_biometric {
            "Faces-Free Policy: 0 biometric matches".to_string()
        } else {
            self.status.clone()
        };
        let rw = self.painter.measure_text(&right_info, 11, false);
        let rx = (area.x + area.w - rw - 16).max(area.x + area.w / 2);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            rx,
            area.y + 10,
            &right_info,
            11,
            if self.query.rejected_biometric {
                COL_ROSE
            } else {
                COL_MUTED
            },
            false,
            false,
            UiRect::new(area.x + area.w / 3, area.y + 4, (area.w * 2) / 3 - 14, 24),
        );

        let content = UiRect::new(
            area.x + 14,
            area.y + hdr_h + 12,
            (area.w - 28).max(40),
            (area.h - hdr_h - 24).max(40),
        );

        if vis.is_empty() {
            let box_r = UiRect::new(
                content.x + content.w / 6,
                content.y + content.h / 3,
                (content.w * 2) / 3,
                96,
            );
            stroke_round_rect(fb, w, h, box_r, 10, 1, COL_BORDER, COL_CARD);
            let msg1 = if self.query.rejected_biometric {
                "Biometric / Face Search Refused"
            } else {
                "No Photos Match Current Filter"
            };
            let msg2 = if self.query.rejected_biometric {
                "Gaze indexes dates & offline places only — never faces or people."
            } else {
                "Try searching by year (2026), month (2026-05), or place (Paris, Japan)."
            };
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                box_r.x + 20,
                box_r.y + 24,
                msg1,
                14,
                COL_AMBER,
                false,
                true,
                box_r.inset(14, 8),
            );
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                box_r.x + 20,
                box_r.y + 52,
                msg2,
                12,
                COL_MUTED,
                false,
                false,
                box_r.inset(14, 8),
            );
            return;
        }

        let cols = if vis.len() <= 4 && content.w >= 460 {
            2
        } else if content.w >= 640 {
            3
        } else {
            2
        };
        let gap = 14i32;
        let card_w = ((content.w - gap * (cols - 1)) / cols).max(120);
        let rows_needed = ((vis.len() as i32 + cols - 1) / cols).max(1);
        let max_card_h = if cols == 2 && rows_needed <= 2 {
            ((content.h - gap) / 2).clamp(180, 290)
        } else {
            218
        };

        for (slot_i, &photo_idx) in vis.iter().enumerate() {
            let col = (slot_i as i32) % cols;
            let row = (slot_i as i32) / cols;
            let cx = content.x + col * (card_w + gap);
            let cy = content.y + row * (max_card_h + gap);
            if cy + 80 > area.y + area.h {
                break;
            }
            let card_r = UiRect::new(cx, cy, card_w, max_card_h);
            let selected = self.selected_idx == Some(photo_idx);

            if selected {
                stroke_round_rect(fb, w, h, card_r, 8, 2, COL_AMBER, COL_CARD_SEL);
            } else {
                stroke_round_rect(fb, w, h, card_r, 8, 1, COL_BORDER, COL_CARD);
            }

            let rec = self.library.photos[photo_idx].clone();
            let thumb_h = (card_r.h - 62).max(70);
            let thumb_box = UiRect::new(card_r.x + 8, card_r.y + 8, card_r.w - 16, thumb_h);

            if let Some(t_img) = self.thumb_for(rec.id) {
                let t_clone = t_img.clone();
                blit_image_fitted(fb, w, h, &t_clone, thumb_box, 0x0A0E17);
            } else {
                fill_rect(fb, w, h, thumb_box, 0x0A0E17);
            }
            stroke_rect(fb, w, h, thumb_box, 1, 0x223049);

            // Overlay badges on thumbnail
            let mut badge_x = thumb_box.x + thumb_box.w - 8;
            if !rec.recipe.is_identity() {
                badge_x -= 56;
                let ed_r = UiRect::new(badge_x, thumb_box.y + 6, 56, 18);
                stroke_round_rect(fb, w, h, ed_r, 4, 1, COL_AMBER, 0x2C1D07);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    ed_r.x + 6,
                    ed_r.y + 3,
                    "EDITED",
                    10,
                    COL_AMBER,
                    false,
                    true,
                    ed_r,
                );
                badge_x -= 6;
            }
            if rec.favorite {
                badge_x -= 36;
                let fv_r = UiRect::new(badge_x, thumb_box.y + 6, 36, 18);
                stroke_round_rect(fb, w, h, fv_r, 4, 1, COL_ROSE, COL_ROSE_DARK);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    fv_r.x + 6,
                    fv_r.y + 3,
                    "FAV",
                    10,
                    COL_ROSE,
                    false,
                    true,
                    fv_r,
                );
            }

            // Card footer line 1: Filename + Dimensions
            let fy1 = thumb_box.y + thumb_box.h + 8;
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                card_r.x + 10,
                fy1,
                &rec.name,
                13,
                COL_TEXT,
                false,
                true,
                UiRect::new(card_r.x + 8, fy1 - 2, card_r.w / 2 + 24, 18),
            );
            let dim_txt = format!("{}x{}", rec.meta.width, rec.meta.height);
            let dw = self.painter.measure_text(&dim_txt, 11, true);
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                card_r.x + card_r.w - dw - 10,
                fy1 + 1,
                &dim_txt,
                11,
                COL_DIM,
                true,
                false,
                UiRect::new(card_r.x + card_r.w / 2, fy1 - 2, card_r.w / 2 - 8, 18),
            );

            // Card footer line 2: Date pill + Offline Place pill
            let fy2 = fy1 + 22;
            let date_txt = rec
                .meta
                .date
                .map(|d| d.format_ymd())
                .unwrap_or_else(|| "No Date".to_string());
            let date_w = (self.painter.measure_text(&date_txt, 11, true) + 12).min(96);
            let date_pill = UiRect::new(card_r.x + 10, fy2, date_w, 19);
            fill_round_rect(fb, w, h, date_pill, 4, 0x101827);
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                date_pill.x + 6,
                date_pill.y + 3,
                &date_txt,
                11,
                COL_MUTED,
                true,
                false,
                date_pill,
            );

            if let Some(place) = rec.meta.place.as_deref() {
                let px = date_pill.x + date_pill.w + 6;
                let max_pw = (card_r.x + card_r.w - 10 - px).max(0);
                if max_pw >= 40 {
                    let pw = (self.painter.measure_text(place, 11, false) + 14).min(max_pw);
                    let place_pill = UiRect::new(px, fy2, pw, 19);
                    stroke_round_rect(fb, w, h, place_pill, 4, 1, 0x334175, COL_INDIGO_DARK);
                    self.painter.draw_text_clipped(
                        fb,
                        w,
                        h,
                        place_pill.x + 7,
                        place_pill.y + 3,
                        place,
                        11,
                        0xC7D2FE,
                        false,
                        false,
                        place_pill.inset(4, 1),
                    );
                }
            }

            self.register_control(&format!("photo_idx_{}", photo_idx), card_r);
        }
    }

    fn render_inspector(&mut self, fb: &mut [u32], w: usize, h: usize, area: UiRect) {
        let mut y = area.y + 12;
        let x = area.x + 12;
        let iw = area.w - 24;

        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            x,
            y,
            "PHOTO INSPECTOR",
            10,
            COL_DIM,
            false,
            true,
            UiRect::new(x, y - 2, iw, 16),
        );
        y += 18;

        let Some(idx) = self.selected_idx else {
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                x,
                y + 12,
                "Select a photo to inspect.",
                12,
                COL_MUTED,
                false,
                false,
                UiRect::new(x, y, iw, 24),
            );
            return;
        };
        let Some(rec) = self.library.photos.get(idx).cloned() else {
            return;
        };

        // Mini preview
        let prev_r = UiRect::new(x, y, iw, 132);
        if let Some(t_img) = self.thumb_for(rec.id).cloned() {
            blit_image_fitted(fb, w, h, &t_img, prev_r, 0x090D16);
            stroke_rect(fb, w, h, prev_r, 1, COL_BORDER);
            y += 140;

            // Live RGB Histogram
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                x,
                y,
                "RGB HISTOGRAM",
                10,
                COL_DIM,
                false,
                true,
                UiRect::new(x, y - 2, iw, 14),
            );
            y += 15;
            let hist_r = UiRect::new(x, y, iw, 58);
            draw_luma_histogram(fb, w, h, &t_img, hist_r);
            y += 68;
        } else {
            y += 12;
        }

        // Metadata key/value table
        let date_val = rec
            .meta
            .date
            .map(|d| d.format_short_datetime())
            .unwrap_or_else(|| "Unknown".to_string());
        let place_val = rec
            .meta
            .place
            .clone()
            .unwrap_or_else(|| "Unlocated".to_string());
        let gps_val = match rec.meta.gps {
            Some((la, lo)) => format!("{:.4}, {:.4}", la, lo),
            None => "None".to_string(),
        };
        let cam_val = rec
            .meta
            .camera
            .clone()
            .unwrap_or_else(|| "Standard Raster".to_string());
        let albums_list = self.library.albums_for_photo(&rec.path);
        let albums_val = if albums_list.is_empty() {
            "None".to_string()
        } else {
            albums_list.join(", ")
        };
        let dims_val = format!(
            "{} x {} ({})",
            rec.meta.width,
            rec.meta.height,
            if rec.meta.format_name.is_empty() {
                "IMG"
            } else {
                &rec.meta.format_name
            }
        );

        let rows = [
            ("File", rec.name.as_str()),
            ("Dimensions", dims_val.as_str()),
            ("Date Taken", date_val.as_str()),
            ("Place", place_val.as_str()),
            ("GPS", gps_val.as_str()),
            ("Camera", cam_val.as_str()),
            ("Albums", albums_val.as_str()),
        ];

        for (k, v) in rows {
            if y + 20 > area.y + area.h - 90 {
                break;
            }
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                x,
                y,
                k,
                11,
                COL_DIM,
                false,
                false,
                UiRect::new(x, y - 1, 74, 16),
            );
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                x + 76,
                y,
                v,
                11,
                COL_TEXT,
                false,
                true,
                UiRect::new(x + 76, y - 1, iw - 76, 16),
            );
            y += 21;
        }

        // Non-destructive recipe status card
        y += 8;
        if y + 66 <= area.y + area.h - 8 {
            let box_r = UiRect::new(x, y, iw, 68);
            let edited = !rec.recipe.is_identity();
            stroke_round_rect(
                fb,
                w,
                h,
                box_r,
                7,
                1,
                if edited { COL_AMBER } else { COL_EMERALD },
                if edited { 0x22190B } else { COL_EMERALD_DARK },
            );
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                box_r.x + 10,
                box_r.y + 8,
                if edited {
                    "NON-DESTRUCTIVE RECIPE"
                } else {
                    "ORIGINAL UNTOUCHED"
                },
                10,
                if edited { COL_AMBER } else { COL_EMERALD },
                false,
                true,
                box_r.inset(8, 4),
            );
            let sum = rec.recipe.summary();
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                box_r.x + 10,
                box_r.y + 26,
                &sum,
                11,
                COL_TEXT,
                false,
                false,
                UiRect::new(box_r.x + 8, box_r.y + 24, box_r.w - 16, 18),
            );
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                box_r.x + 10,
                box_r.y + 46,
                "Source file bytes never modified",
                10,
                COL_MUTED,
                false,
                false,
                UiRect::new(box_r.x + 8, box_r.y + 44, box_r.w - 16, 18),
            );
        }
    }

    fn render_edit_studio(&mut self, fb: &mut [u32], w: usize, h: usize) {
        fill_rect(fb, w, h, UiRect::new(0, 0, w as i32, h as i32), 0x080C14);

        let top_h = 50i32;
        fill_rect(fb, w, h, UiRect::new(0, 0, w as i32, top_h), COL_TOPBAR);
        fill_rect(fb, w, h, UiRect::new(0, top_h - 1, w as i32, 1), COL_BORDER);

        self.draw_button(
            fb,
            w,
            h,
            "btn_back_library",
            UiRect::new(12, 10, 96, 28),
            "< Library",
            COL_BTN,
            COL_BORDER,
            COL_TEXT,
            true,
        );

        let (photo_name, recipe) = self
            .selected_idx
            .and_then(|i| self.library.photos.get(i))
            .map(|p| (p.name.clone(), p.recipe.clone()))
            .unwrap_or_else(|| ("Photo".to_string(), EditRecipe::default()));

        let title_end = self.painter.draw_text_clipped(
            fb,
            w,
            h,
            122,
            16,
            &photo_name,
            15,
            COL_TEXT,
            false,
            true,
            UiRect::new(120, 10, 240, 30),
        );

        let safe_pill = UiRect::new(title_end + 12, 13, 212, 22);
        stroke_round_rect(fb, w, h, safe_pill, 10, 1, COL_EMERALD, COL_EMERALD_DARK);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            safe_pill.x + 10,
            safe_pill.y + 5,
            "NON-DESTRUCTIVE · ORIGINAL SAFE",
            10,
            COL_EMERALD,
            false,
            true,
            safe_pill.inset(6, 2),
        );

        let mut rx = w as i32 - 12;
        rx -= 98;
        self.draw_button(
            fb,
            w,
            h,
            "btn_export_copy",
            UiRect::new(rx, 10, 98, 28),
            "Save Copy [S]",
            COL_EMERALD_DARK,
            COL_EMERALD,
            COL_EMERALD,
            true,
        );
        rx -= 82;
        self.draw_button(
            fb,
            w,
            h,
            "btn_reset_edits",
            UiRect::new(rx, 10, 76, 28),
            "Reset [0]",
            COL_BTN,
            COL_BORDER,
            COL_TEXT,
            false,
        );
        rx -= 94;
        self.draw_button(
            fb,
            w,
            h,
            "btn_compare",
            UiRect::new(rx, 10, 88, 28),
            if self.compare_original {
                "Original [\\]"
            } else {
                "Compare [\\]"
            },
            if self.compare_original {
                COL_AMBER_DARK
            } else {
                COL_BTN
            },
            if self.compare_original {
                COL_AMBER
            } else {
                COL_BORDER
            },
            if self.compare_original {
                COL_AMBER
            } else {
                COL_TEXT
            },
            self.compare_original,
        );

        // Right Studio Controls Panel
        let panel_w = 324i32.min((w as i32) / 2);
        let body_y = top_h;
        let body_h = (h as i32 - body_y).max(1);
        let panel_r = UiRect::new(w as i32 - panel_w, body_y, panel_w, body_h);
        fill_rect(fb, w, h, panel_r, COL_SIDEBAR);
        fill_rect(fb, w, h, UiRect::new(panel_r.x, body_y, 1, body_h), COL_BORDER);

        // Left/Center Live Canvas
        let canvas_r = UiRect::new(0, body_y, (w as i32 - panel_w).max(60), body_h);
        let stage = UiRect::new(
            canvas_r.x + 26,
            canvas_r.y + 22,
            (canvas_r.w - 52).max(20),
            (canvas_r.h - 68).max(20),
        );

        let shown_img = if self.compare_original {
            self.edit_source.as_ref().map(|(_, im)| im.clone())
        } else {
            self.edit_preview.clone()
        };

        if let Some(img) = shown_img {
            let drawn = blit_image_fitted(fb, w, h, &img, stage, 0x080C14);
            stroke_rect(fb, w, h, drawn.inset(-1, -1), 1, 0x2D3E5E);

            // Subtle rule-of-thirds composition lines when cropped
            if !recipe.crop.is_full() && !self.compare_original && drawn.w > 40 && drawn.h > 40 {
                for third in [1, 2] {
                    let gx = drawn.x + (drawn.w * third) / 3;
                    let gy = drawn.y + (drawn.h * third) / 3;
                    fill_rect(fb, w, h, UiRect::new(gx, drawn.y, 1, drawn.h), 0x44587A);
                    fill_rect(fb, w, h, UiRect::new(drawn.x, gy, drawn.w, 1), 0x44587A);
                }
                stroke_rect(fb, w, h, drawn, 2, COL_AMBER);
            }

            // Bottom Studio HUD pill
            let hud_r = UiRect::new(
                canvas_r.x + 24,
                canvas_r.y + canvas_r.h - 36,
                (canvas_r.w - 48).max(40),
                26,
            );
            stroke_round_rect(fb, w, h, hud_r, 6, 1, COL_BORDER, 0x101827);
            let hud_txt = if self.compare_original {
                format!("ORIGINAL SOURCE PREVIEW  ·  {}x{}", img.width, img.height)
            } else {
                format!(
                    "LIVE OUTPUT: {}x{}  ·  {}",
                    img.width,
                    img.height,
                    recipe.summary()
                )
            };
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                hud_r.x + 12,
                hud_r.y + 6,
                &hud_txt,
                11,
                if self.compare_original {
                    COL_AMBER
                } else {
                    COL_MUTED
                },
                false,
                true,
                hud_r.inset(8, 2),
            );
        }

        // Populate Right Studio Controls
        let px = panel_r.x + 14;
        let pw = panel_r.w - 28;
        let mut y = panel_r.y + 12;

        // 1. CROP & GEOMETRY
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            px,
            y,
            "CROP & GEOMETRY",
            10,
            COL_DIM,
            false,
            true,
            UiRect::new(px, y - 2, pw, 14),
        );
        y += 17;

        let gap = 6;
        let cw4 = (pw - gap * 3) / 4;
        let aspects = [
            ("crop_free", "Free", recipe.aspect == CropAspect::Free && recipe.crop.is_full()),
            ("crop_1x1", "1:1", recipe.aspect == CropAspect::Square1x1),
            ("crop_4x3", "4:3", recipe.aspect == CropAspect::FourThree),
            ("crop_16x9", "16:9", recipe.aspect == CropAspect::SixteenNine),
        ];
        for (i, (cid, lbl, active)) in aspects.into_iter().enumerate() {
            let r = UiRect::new(px + (i as i32) * (cw4 + gap), y, cw4, 25);
            self.draw_button(
                fb,
                w,
                h,
                cid,
                r,
                lbl,
                if active { COL_AMBER_DARK } else { COL_CARD },
                if active { COL_AMBER } else { COL_BORDER },
                if active { COL_AMBER } else { COL_TEXT },
                active,
            );
        }
        y += 30;

        let geo_btns = [
            ("crop_zoom_in", "Zoom +", !recipe.crop.is_full()),
            ("crop_reset", "Full", false),
            ("rot_cw", "Rot 90", recipe.quarter_turns % 4 != 0),
            ("flip_h", "Flip H", recipe.flip_h),
        ];
        for (i, (cid, lbl, active)) in geo_btns.into_iter().enumerate() {
            let r = UiRect::new(px + (i as i32) * (cw4 + gap), y, cw4, 25);
            self.draw_button(
                fb,
                w,
                h,
                cid,
                r,
                lbl,
                if active { COL_INDIGO_DARK } else { COL_CARD },
                if active { COL_INDIGO } else { COL_BORDER },
                COL_TEXT,
                active,
            );
        }
        y += 36;

        // 2. LIGHT & COLOR SLIDERS
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            px,
            y,
            "LIGHT & COLOR (FILMIC LUT + HSL)",
            10,
            COL_DIM,
            false,
            true,
            UiRect::new(px, y - 2, pw, 14),
        );
        y += 17;

        let ev_sign = if recipe.exposure_ev_x100 > 0 {
            "+"
        } else if recipe.exposure_ev_x100 < 0 {
            "-"
        } else {
            ""
        };
        let ev_abs = recipe.exposure_ev_x100.unsigned_abs();
        let ev_txt = format!("{}{}.{:02} EV", ev_sign, ev_abs / 100, ev_abs % 100);
        let con_txt = format!("{:+}%", recipe.contrast_pct);
        let warm_txt = format!("{:+}%", recipe.warmth_pct);
        let sat_txt = format!("{:+}%", recipe.saturation_pct);

        let sliders = [
            ("Exposure", ev_txt.as_str(), recipe.exposure_ev_x100 as i32, 200, "ev_dec", "ev_inc"),
            ("Contrast", con_txt.as_str(), recipe.contrast_pct as i32, 90, "con_dec", "con_inc"),
            ("Warmth", warm_txt.as_str(), recipe.warmth_pct as i32, 90, "warm_dec", "warm_inc"),
            ("Saturation", sat_txt.as_str(), recipe.saturation_pct as i32, 90, "sat_dec", "sat_inc"),
        ];

        for (title, val_s, val, max_abs, dec_id, inc_id) in sliders {
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                px,
                y,
                title,
                11,
                COL_TEXT,
                false,
                true,
                UiRect::new(px, y - 2, 90, 16),
            );
            let vw = self.painter.measure_text(val_s, 11, true);
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                px + pw - vw,
                y,
                val_s,
                11,
                if val != 0 { COL_AMBER } else { COL_MUTED },
                true,
                val != 0,
                UiRect::new(px + 90, y - 2, pw - 90, 16),
            );
            y += 16;

            let dec_r = UiRect::new(px, y, 26, 20);
            let inc_r = UiRect::new(px + pw - 26, y, 26, 20);
            self.draw_button(fb, w, h, dec_id, dec_r, "-", COL_CARD, COL_BORDER, COL_TEXT, true);
            self.draw_button(fb, w, h, inc_id, inc_r, "+", COL_CARD, COL_BORDER, COL_TEXT, true);

            // Bipolar track bar
            let track = UiRect::new(px + 32, y + 6, pw - 64, 8);
            fill_round_rect(fb, w, h, track, 4, 0x090E17);
            stroke_rect(fb, w, h, track, 1, COL_BORDER);
            let mid = track.x + track.w / 2;
            fill_rect(fb, w, h, UiRect::new(mid, track.y - 2, 2, track.h + 4), COL_MUTED);
            if val != 0 {
                let span = ((val * (track.w / 2 - 2)) / max_abs).clamp(-(track.w / 2), track.w / 2);
                let (bx0, bw0) = if span >= 0 {
                    (mid, span)
                } else {
                    (mid + span, -span)
                };
                fill_rect(fb, w, h, UiRect::new(bx0, track.y + 1, bw0.max(2), track.h - 2), COL_AMBER);
            }
            y += 26;
        }

        // 3. SHARED BATIK FILTERS (batik-core)
        y += 6;
        let hdr_end = self.painter.draw_text_clipped(
            fb,
            w,
            h,
            px,
            y,
            "SHARED BATIK FILTERS",
            10,
            COL_DIM,
            false,
            true,
            UiRect::new(px, y - 2, 160, 16),
        );
        let b_pill = UiRect::new(hdr_end + 8, y - 2, 76, 16);
        stroke_round_rect(fb, w, h, b_pill, 4, 1, COL_INDIGO, COL_INDIGO_DARK);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            b_pill.x + 6,
            b_pill.y + 2,
            "batik-core",
            10,
            0xC7D2FE,
            true,
            false,
            b_pill,
        );
        y += 19;

        let col_w = (pw - 8) / 2;
        for (i, preset) in SharedFilterPreset::ALL.iter().copied().enumerate() {
            let col = (i as i32) % 2;
            let row = (i as i32) / 2;
            let ry = y + row * 28;
            if ry + 24 > panel_r.y + panel_r.h - 36 {
                break;
            }
            let rx = px + col * (col_w + 8);
            let r = UiRect::new(rx, ry, col_w, 24);
            let active = recipe.preset == preset;
            let ctrl = format!("preset_{}", preset.slug());
            self.draw_button(
                fb,
                w,
                h,
                &ctrl,
                r,
                preset.label(),
                if active { COL_INDIGO_DARK } else { COL_CARD },
                if active { COL_AMBER } else { COL_BORDER },
                if active { COL_AMBER } else { COL_TEXT },
                active,
            );
        }

        // Bottom FilterStack bar
        let sy = (panel_r.y + panel_r.h - 34).max(y + 6 * 28 + 4);
        if sy + 26 <= panel_r.y + panel_r.h {
            let stack_cnt = recipe.filter_stack.iter().filter(|n| n.enabled).count();
            let add_w = (pw * 3) / 5 - 4;
            let clr_w = pw - add_w - 8;
            self.draw_button(
                fb,
                w,
                h,
                "stack_add_sharpen",
                UiRect::new(px, sy, add_w, 25),
                "+ Stack Sharpen",
                COL_BTN,
                COL_INDIGO,
                COL_TEXT,
                true,
            );
            let clr_lbl = format!("Clear FX ({})", stack_cnt);
            self.draw_button(
                fb,
                w,
                h,
                "stack_clear",
                UiRect::new(px + add_w + 8, sy, clr_w, 25),
                &clr_lbl,
                if stack_cnt > 0 { COL_AMBER_DARK } else { COL_CARD },
                if stack_cnt > 0 { COL_AMBER } else { COL_BORDER },
                if stack_cnt > 0 { COL_AMBER } else { COL_MUTED },
                false,
            );
        }
    }
}

pub fn join_path(folder: &[u8], name: &[u8]) -> Vec<u8> {
    if folder.is_empty() {
        return name.to_vec();
    }
    let mut trimmed = folder;
    while trimmed.len() > 2 && trimmed.ends_with(b"/") {
        trimmed = &trimmed[..trimmed.len() - 1];
    }
    let mut p = Vec::with_capacity(trimmed.len() + 1 + name.len());
    p.extend_from_slice(trimmed);
    if !p.ends_with(b"/") {
        p.push(b'/');
    }
    p.extend_from_slice(name);
    p
}
