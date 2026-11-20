// SPDX-License-Identifier: GPL-3.0-only
//! Gaze Photo Library & Non-Destructive Shared-Batik Edit Studio UI.

#![forbid(unsafe_code)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use batik_core::encode_rgba_png;
use gaze_core::{
    analyze_scope_stats, apply_full_develop, builtin_lut_preset, export_contact_sheet_png,
    extract_metadata, film_stock_from_slug, film_stock_slug, next_film_stock, render_scope_into,
    sidecar_path_for, BrushDab, BrushMask, ColorLabel, ColorRangeMask, ContactSheetEntry,
    ContactSheetSpec, CropAspect, CropRect, CullFlag, DenoiseParams, DevelopRecipe, EditRecipe,
    FilmStock, FilterNode, FilterSpec, HealPatch, InterpMode, LinearGradientMask, LocalMask,
    LuminanceRangeMask, MaskAdjustments, PhotoLibrary, PhotoSidecar, RadialMask, ScopeKind,
    ScopeStandard, ScopeSurface, SharedFilterPreset, SidebarCollection, SmartQuery,
};
use sex_app::{files, Nav, Window};
use sex_image::{Image, Limits};
use sex_pdx::app_key::Key;
use sex_pdx::app_pointer::{Kind, Pointer};

use crate::canvas::{
    blit_image_fitted, blit_scope_surface, draw_luma_histogram, draw_star_row, fill_rect,
    fill_round_rect, stroke_rect, stroke_round_rect, FontPainter, HitControl, UiRect,
};

const CATALOG_FILE: &[u8] = b"library.gaze";
const MAX_READ_BYTES: usize = 8 * 1024 * 1024;
const MAX_SIDECAR_BYTES: usize = 512 * 1024;
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
pub enum DevelopTab {
    ToneColor,
    FilmLut,
    MasksDenoise,
}

impl DevelopTab {
    pub const fn next(self) -> Self {
        match self {
            Self::ToneColor => Self::FilmLut,
            Self::FilmLut => Self::MasksDenoise,
            Self::MasksDenoise => Self::ToneColor,
        }
    }

    pub const fn prev(self) -> Self {
        match self {
            Self::ToneColor => Self::MasksDenoise,
            Self::FilmLut => Self::ToneColor,
            Self::MasksDenoise => Self::FilmLut,
        }
    }
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
    pub develop_tab: DevelopTab,
    pub scope_kind: ScopeKind,
    pub cull_mode: bool,
    pub active_mask_idx: usize,
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
    scope_surface: ScopeSurface,
    io_buf: Vec<u8>,
}

impl GazeStudio {
    pub fn new() -> Self {
        Self {
            screen: StudioScreen::Library,
            develop_tab: DevelopTab::ToneColor,
            scope_kind: ScopeKind::Off,
            cull_mode: false,
            active_mask_idx: 0,
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
            scope_surface: ScopeSurface::new(260, 128),
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
        // Load folder-native `.gaze.toml` sidecar if present (original bytes remain untouched)
        let sidecar_path = sidecar_path_for(&path_owned);
        if let Ok(st_sc) = files::stat(window, sidecar_path.as_bytes()) {
            if !st_sc.dir && st_sc.size > 0 && (st_sc.size as usize) <= MAX_SIDECAR_BYTES {
                let sc_need = st_sc.size as usize;
                if self.ensure_io_buf(sc_need) {
                    if let Ok(sc_n) = files::read(window, sidecar_path.as_bytes(), &mut self.io_buf[..sc_need]) {
                        if let Ok(sc_str) = core::str::from_utf8(&self.io_buf[..sc_n]) {
                            if let Ok(sc) = PhotoSidecar::from_toml(sc_str) {
                                self.library.apply_sidecar(&path_owned, &sc);
                            }
                        }
                    }
                }
            }
        }

        let id = self.library.photos[idx].id;
        let recipe = self.library.photos[idx].recipe.clone();
        let develop = self.library.photos[idx].develop.clone();
        let edited_thumb =
            apply_full_develop(&raw_thumb, &recipe, &develop).unwrap_or_else(|_| raw_thumb.clone());

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

    #[allow(dead_code)]
    pub fn recipe_for_path(&self, path: &[u8]) -> Option<&EditRecipe> {
        let s = core::str::from_utf8(path).ok()?;
        self.library.photo_by_path(s).map(|r| &r.recipe)
    }

    pub fn recipes_for_path(&self, path: &[u8]) -> Option<(&EditRecipe, &DevelopRecipe)> {
        let s = core::str::from_utf8(path).ok()?;
        self.library.photo_by_path(s).map(|r| (&r.recipe, &r.develop))
    }

    fn persist_sidecar_for(&self, window: &mut Window, idx: usize) {
        let Some(rec) = self.library.photos.get(idx) else {
            return;
        };
        let sidecar_path = sidecar_path_for(&rec.path);
        let toml = rec.to_sidecar().to_toml();
        let _ = files::write(window, sidecar_path.as_bytes(), toml.as_bytes());
    }

    fn refresh_thumb_for(&mut self, idx: usize) {
        let Some(rec) = self.library.photos.get(idx) else {
            return;
        };
        let id = rec.id;
        let recipe = rec.recipe.clone();
        let develop = rec.develop.clone();
        if let Some(slot) = self.thumbs.iter_mut().find(|t| t.id == id) {
            if let Ok(edited) = apply_full_develop(&slot.raw_thumb, &recipe, &develop) {
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
        let develop = self.library.photos[idx].develop.clone();
        let preview =
            apply_full_develop(&studio_src, &recipe, &develop).unwrap_or_else(|_| studio_src.clone());
        self.edit_source = Some((path.clone(), studio_src));
        self.edit_preview = Some(preview);
        self.compare_original = false;
        self.screen = StudioScreen::Edit;
        self.input_mode = InputMode::None;
        let mask_cnt = develop.masks.len();
        if mask_cnt == 0 {
            self.active_mask_idx = 0;
        } else if self.active_mask_idx >= mask_cnt {
            self.active_mask_idx = mask_cnt - 1;
        }
        self.status = format!(
            "Editing {} non-destructively (original untouched).",
            self.library.photos[idx].name
        );
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
        let develop = self.library.photos[idx].develop.clone();
        let path = self.library.photos[idx].path.clone();
        self.library.set_recipe(&path, recipe.clone());
        self.refresh_thumb_for(idx);

        if let Some((ref src_path, ref src_img)) = self.edit_source {
            if *src_path == path {
                if let Ok(updated) = apply_full_develop(src_img, &recipe, &develop) {
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
        self.persist_sidecar_for(window, idx);
        self.persist_catalog(window);
    }

    fn apply_develop_change(&mut self, window: &mut Window, f: impl FnOnce(&mut DevelopRecipe)) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let recipe = self.library.photos[idx].recipe.clone();
        let mut develop = self.library.photos[idx].develop.clone();
        f(&mut develop);
        let path = self.library.photos[idx].path.clone();
        self.library.set_develop(&path, develop.clone());
        if develop.masks.is_empty() {
            self.active_mask_idx = 0;
        } else if self.active_mask_idx >= develop.masks.len() {
            self.active_mask_idx = develop.masks.len() - 1;
        }
        self.refresh_thumb_for(idx);

        if let Some((ref src_path, ref src_img)) = self.edit_source {
            if *src_path == path {
                if let Ok(updated) = apply_full_develop(src_img, &recipe, &develop) {
                    self.edit_preview = Some(updated);
                }
            }
        }

        let film_s = develop.film_stock.map(film_stock_slug).unwrap_or("none");
        sex_pdx::serial_record!(
            "[gaze.develop.updated] path={} film={} masks={} heals={} nr={:.2}/{:.2}",
            path,
            film_s,
            develop.masks.len(),
            develop.heal_spots.len(),
            develop.denoise.luma_strength,
            develop.denoise.chroma_strength
        );
        self.status = format!("Develop: {}", develop.summary());
        self.persist_sidecar_for(window, idx);
        self.persist_catalog(window);
    }

    fn set_selected_stars(&mut self, window: &mut Window, stars: u8) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let path = self.library.photos[idx].path.clone();
        let name = self.library.photos[idx].name.clone();
        let clamped = stars.min(5);
        self.library.set_stars(&path, clamped);
        self.status = if clamped == 0 {
            format!("Cleared rating on {}.", name)
        } else {
            format!("Rated {} {} star(s).", name, clamped)
        };
        sex_pdx::serial_record!(
            "[gaze.cull.updated] path={} stars={} flag={} label={}",
            path,
            clamped,
            self.library.photos[idx].flag.slug(),
            self.library.photos[idx].color_label.slug()
        );
        self.sync_selection_with_visible();
        self.persist_sidecar_for(window, idx);
        self.persist_catalog(window);
    }

    fn toggle_selected_flag(&mut self, window: &mut Window, target: CullFlag) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let path = self.library.photos[idx].path.clone();
        let name = self.library.photos[idx].name.clone();
        let cur = self.library.photos[idx].flag;
        let next = if cur == target && target != CullFlag::Unflagged {
            CullFlag::Unflagged
        } else {
            target
        };
        self.library.set_flag(&path, next);
        self.status = format!("{}: {}", next.label(), name);
        sex_pdx::serial_record!(
            "[gaze.cull.updated] path={} stars={} flag={} label={}",
            path,
            self.library.photos[idx].stars,
            next.slug(),
            self.library.photos[idx].color_label.slug()
        );
        self.sync_selection_with_visible();
        self.persist_sidecar_for(window, idx);
        self.persist_catalog(window);
    }

    fn cycle_selected_color_label(&mut self, window: &mut Window) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let path = self.library.photos[idx].path.clone();
        let next = self.library.photos[idx].color_label.next();
        self.library.set_color_label(&path, next);
        self.status = format!("Label: {}", next.label());
        sex_pdx::serial_record!(
            "[gaze.cull.updated] path={} stars={} flag={} label={}",
            path,
            self.library.photos[idx].stars,
            self.library.photos[idx].flag.slug(),
            next.slug()
        );
        self.sync_selection_with_visible();
        self.persist_sidecar_for(window, idx);
        self.persist_catalog(window);
    }

    fn reset_selected_edits(&mut self, window: &mut Window) {
        let Some(idx) = self.selected_idx else {
            return;
        };
        let path = self.library.photos[idx].path.clone();
        self.library.reset_recipe(&path);
        self.active_mask_idx = 0;
        self.refresh_thumb_for(idx);
        if let Some((ref src_path, ref src_img)) = self.edit_source {
            if *src_path == path {
                self.edit_preview = Some(src_img.clone());
            }
        }
        self.status = "Reverted to original unedited photo.".to_string();
        sex_pdx::serial_record!("[gaze.edit.reset] path={}", path);
        self.persist_sidecar_for(window, idx);
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

    fn export_contact_sheet(&mut self, window: &mut Window) {
        let vis = self.library.visible_indices(&self.collection, &self.query);
        let mut chosen: Vec<usize> = vis
            .iter()
            .copied()
            .filter(|&i| self.library.photos[i].is_keeper())
            .collect();
        if chosen.is_empty() {
            chosen = vis;
        }
        if chosen.is_empty() {
            self.status = "No photos available to export contact sheet.".to_string();
            return;
        }

        let mut entries: Vec<ContactSheetEntry<'_>> = Vec::new();
        for &idx in chosen.iter().take(9) {
            let rec = &self.library.photos[idx];
            if let Some(img) = self.thumb_for(rec.id) {
                entries.push(ContactSheetEntry::from_record(rec, img));
            }
        }
        if entries.is_empty() {
            return;
        }

        let exported_count = entries.len();
        let mut spec = ContactSheetSpec::dark_slate();
        spec.page_width = 1280;
        spec.page_height = 960;
        if exported_count <= 4 {
            spec.columns = 2;
            spec.rows = 2;
        }
        let png_res = export_contact_sheet_png(&entries, &spec);
        if let Ok(png_bytes) = png_res {
            let _ = files::mkdir(window, b"exports");
            let rel = "exports/contact-sheet.png";
            if files::write(window, rel.as_bytes(), &png_bytes).is_ok() {
                let _ = files::sync(window);
                self.status = format!(
                    "Saved {}x{} contact sheet ({} photos) to /data/gaze/{}",
                    spec.page_width,
                    spec.page_height,
                    exported_count,
                    rel
                );
                sex_pdx::serial_record!(
                    "[gaze.contact_sheet.saved] path=/data/gaze/{} count={} bytes={}",
                    rel,
                    exported_count,
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
                Some(b'1'..=b'5') => {
                    let s = sex_app::typed(key).unwrap() - b'0';
                    self.set_selected_stars(window, s);
                }
                Some(b'p' | b'P') => {
                    self.toggle_selected_flag(window, CullFlag::Pick);
                }
                Some(b'x' | b'X') => {
                    self.toggle_selected_flag(window, CullFlag::Reject);
                }
                Some(b's' | b'S') => {
                    self.export_edited_copy(window);
                }
                Some(b'g' | b'G') => {
                    self.export_contact_sheet(window);
                }
                Some(b'v' | b'V') => {
                    self.scope_kind = self.scope_kind.next();
                    self.status = format!("Broadcast Scope: {}", self.scope_kind.label());
                    sex_pdx::serial_record!("[gaze.scope.changed] mode={}", self.scope_kind.slug());
                }
                Some(b'm' | b'M') => {
                    self.apply_develop_change(window, |d| {
                        d.film_stock = next_film_stock(d.film_stock);
                    });
                }
                Some(b'd' | b'D') => {
                    self.apply_develop_change(window, |d| {
                        d.denoise = if !d.denoise.is_active() {
                            DenoiseParams::light()
                        } else if d.denoise.chroma_strength < 0.75 {
                            DenoiseParams::high_iso()
                        } else {
                            DenoiseParams::off()
                        };
                    });
                }
                Some(b',') => {
                    self.develop_tab = self.develop_tab.prev();
                }
                Some(b'.') => {
                    self.develop_tab = self.develop_tab.next();
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

        // Shift+1..5 or Ctrl+1..5 (PS/2 scancodes 0x02..=0x06) always apply star ratings directly
        if (key.shift || key.control) && !key.alt && (0x02..=0x06).contains(&key.code) {
            let stars = key.code - 0x01;
            self.set_selected_stars(window, stars);
            return StudioOutcome::Stay;
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
                self.status = "Type smart search (e.g. 2026-05, Paris, keepers, stars:4) + Enter".to_string();
            }
            Some(b'a' | b'A') => {
                self.input_mode = InputMode::NewAlbum;
                self.album_draft.clear();
                self.status = "Type new album name and press Enter.".to_string();
            }
            Some(b'c' | b'C') => {
                self.cull_mode = !self.cull_mode;
                self.status = if self.cull_mode {
                    "Cull Mode ON: press 1..5 for stars, 0 clear, P pick, X reject.".to_string()
                } else {
                    "Cull Mode OFF: 1..9 select albums, Shift+1..5 rate stars.".to_string()
                };
            }
            Some(b'p' | b'P') => {
                self.toggle_selected_flag(window, CullFlag::Pick);
            }
            Some(b'x' | b'X') => {
                self.toggle_selected_flag(window, CullFlag::Reject);
            }
            Some(b'u' | b'U') => {
                self.toggle_selected_flag(window, CullFlag::Unflagged);
            }
            Some(b'k' | b'K') => {
                if self.collection == SidebarCollection::Keepers {
                    self.collection = SidebarCollection::AllPhotos;
                } else {
                    self.collection = SidebarCollection::Keepers;
                }
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                let cname = self.collection.label(&self.library.albums);
                self.status = format!("{} ({} photos)", cname, vis_n);
                sex_pdx::serial_record!("[gaze.album.selected] name={} visible={}", cname, vis_n);
            }
            Some(b'g' | b'G') => {
                self.export_contact_sheet(window);
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
                let digit = sex_app::typed(key).unwrap() - b'0';
                let n = (digit - 1) as usize;
                if self.cull_mode && digit <= 5 {
                    self.set_selected_stars(window, digit);
                } else if let Some(album) = self.library.albums.get(n) {
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
                } else if digit <= 5 {
                    self.set_selected_stars(window, digit);
                }
            }
            Some(b'0') => {
                if self.cull_mode {
                    self.set_selected_stars(window, 0);
                } else {
                    self.collection = SidebarCollection::AllPhotos;
                    self.sync_selection_with_visible();
                    let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                    sex_pdx::serial_record!(
                        "[gaze.album.selected] name=AllPhotos visible={}",
                        vis_n
                    );
                }
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
                self.status = "Type smart search (date, city, country, album, stars:4, pick) + Enter.".to_string();
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
            "btn_cull_mode" => {
                self.cull_mode = !self.cull_mode;
                self.status = if self.cull_mode {
                    "Cull Mode ON (1..5 stars, P pick, X reject).".to_string()
                } else {
                    "Cull Mode OFF.".to_string()
                };
            }
            "cull_pick" => self.toggle_selected_flag(window, CullFlag::Pick),
            "cull_reject" => self.toggle_selected_flag(window, CullFlag::Reject),
            "cull_label_cycle" => self.cycle_selected_color_label(window),
            "btn_keepers_filter" => {
                if self.collection == SidebarCollection::Keepers {
                    self.collection = SidebarCollection::AllPhotos;
                } else {
                    self.collection = SidebarCollection::Keepers;
                }
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                let cname = self.collection.label(&self.library.albums);
                sex_pdx::serial_record!("[gaze.album.selected] name={} visible={}", cname, vis_n);
            }
            "btn_contact_sheet" => {
                self.export_contact_sheet(window);
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
            "nav_keepers" => {
                self.collection = SidebarCollection::Keepers;
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                sex_pdx::serial_record!("[gaze.album.selected] name=Keepers visible={}", vis_n);
            }
            "nav_picks" => {
                self.collection = SidebarCollection::Picks;
                self.sync_selection_with_visible();
                let vis_n = self.library.visible_indices(&self.collection, &self.query).len();
                sex_pdx::serial_record!("[gaze.album.selected] name=Picks visible={}", vis_n);
            }
            // Edit / Develop Studio controls
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
            "btn_scope_toggle" => {
                self.scope_kind = self.scope_kind.next();
                self.status = format!("Broadcast Scope: {}", self.scope_kind.label());
                sex_pdx::serial_record!("[gaze.scope.changed] mode={}", self.scope_kind.slug());
            }
            "scope_off" => self.scope_kind = ScopeKind::Off,
            "scope_waveform_luma" => self.scope_kind = ScopeKind::WaveformLuma,
            "scope_waveform_rgb" => self.scope_kind = ScopeKind::WaveformRgb,
            "scope_rgb_parade" => self.scope_kind = ScopeKind::RgbParade,
            "scope_vectorscope" => self.scope_kind = ScopeKind::Vectorscope,
            "scope_histogram" => self.scope_kind = ScopeKind::Histogram,
            "tab_tone" => self.develop_tab = DevelopTab::ToneColor,
            "tab_film" => self.develop_tab = DevelopTab::FilmLut,
            "tab_masks" => self.develop_tab = DevelopTab::MasksDenoise,
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
            // OmaRAW Tonal Recovery, Color Wheels, FilmStock & 3D LUT controls
            "hi_dec" => self.apply_develop_change(window, |d| d.highlights = (d.highlights - 0.15).clamp(-1.0, 1.0)),
            "hi_inc" => self.apply_develop_change(window, |d| d.highlights = (d.highlights + 0.15).clamp(-1.0, 1.0)),
            "sh_dec" => self.apply_develop_change(window, |d| d.shadows = (d.shadows - 0.15).clamp(-1.0, 1.0)),
            "sh_inc" => self.apply_develop_change(window, |d| d.shadows = (d.shadows + 0.15).clamp(-1.0, 1.0)),
            "vib_dec" => self.apply_develop_change(window, |d| d.grade.vibrance = (d.grade.vibrance - 0.15).clamp(-1.0, 1.5)),
            "vib_inc" => self.apply_develop_change(window, |d| d.grade.vibrance = (d.grade.vibrance + 0.15).clamp(-1.0, 1.5)),
            "sharp_dec" => self.apply_develop_change(window, |d| d.sharpness = (d.sharpness - 0.15).clamp(0.0, 1.0)),
            "sharp_inc" => self.apply_develop_change(window, |d| d.sharpness = (d.sharpness + 0.15).clamp(0.0, 1.0)),
            "wheel_neutral" => self.apply_develop_change(window, |d| {
                d.grade.lift = [0.0, 0.0, 0.0];
                d.grade.gamma = [1.0, 1.0, 1.0];
                d.grade.gain = [1.0, 1.0, 1.0];
            }),
            "wheel_teal_orange" => self.apply_develop_change(window, |d| {
                d.grade.lift = [-0.02, 0.005, 0.03];
                d.grade.gamma = [1.02, 1.0, 0.96];
                d.grade.gain = [1.08, 1.01, 0.90];
            }),
            "wheel_warm_skin" => self.apply_develop_change(window, |d| {
                d.grade.lift = [0.01, 0.0, -0.01];
                d.grade.gamma = [1.04, 1.01, 0.95];
                d.grade.gain = [1.05, 1.01, 0.96];
            }),
            "wheel_cool_noir" => self.apply_develop_change(window, |d| {
                d.grade.lift = [-0.02, 0.0, 0.025];
                d.grade.gamma = [0.95, 0.98, 1.04];
                d.grade.gain = [0.96, 1.0, 1.06];
            }),
            "film_none" => self.apply_develop_change(window, |d| d.film_stock = None),
            "film_cycle" => self.apply_develop_change(window, |d| d.film_stock = next_film_stock(d.film_stock)),
            "film_str_dec" => self.apply_develop_change(window, |d| d.film_strength = (d.film_strength - 0.15).clamp(0.0, 1.0)),
            "film_str_inc" => self.apply_develop_change(window, |d| d.film_strength = (d.film_strength + 0.15).clamp(0.0, 1.0)),
            "lut_none" => self.apply_develop_change(window, |d| d.clear_lut()),
            "lut_str_dec" => self.apply_develop_change(window, |d| d.lut_strength = (d.lut_strength - 0.15).clamp(0.0, 1.0)),
            "lut_str_inc" => self.apply_develop_change(window, |d| d.lut_strength = (d.lut_strength + 0.15).clamp(0.0, 1.0)),
            "lut_interp_toggle" => self.apply_develop_change(window, |d| {
                d.lut_interp = match d.lut_interp {
                    InterpMode::Tetrahedral => InterpMode::Trilinear,
                    InterpMode::Trilinear => InterpMode::Tetrahedral,
                };
            }),
            // Bilateral RAW Denoise, 5 Local Masks & Spot Heal controls
            "dn_off" => self.apply_develop_change(window, |d| d.denoise = DenoiseParams::off()),
            "dn_light" => self.apply_develop_change(window, |d| d.denoise = DenoiseParams::light()),
            "dn_high_iso" => self.apply_develop_change(window, |d| d.denoise = DenoiseParams::high_iso()),
            "dn_luma_dec" => self.apply_develop_change(window, |d| d.denoise.luma_strength = (d.denoise.luma_strength - 0.15).clamp(0.0, 1.0)),
            "dn_luma_inc" => self.apply_develop_change(window, |d| d.denoise.luma_strength = (d.denoise.luma_strength + 0.15).clamp(0.0, 1.0)),
            "dn_chroma_dec" => self.apply_develop_change(window, |d| d.denoise.chroma_strength = (d.denoise.chroma_strength - 0.15).clamp(0.0, 1.0)),
            "dn_chroma_inc" => self.apply_develop_change(window, |d| d.denoise.chroma_strength = (d.denoise.chroma_strength + 0.15).clamp(0.0, 1.0)),
            "mask_add_brush" => {
                self.apply_develop_change(window, |d| {
                    let adj = MaskAdjustments { exposure_ev: 0.45, ..MaskAdjustments::default() };
                    d.masks.push(LocalMask::brush(
                        BrushMask::new(alloc::vec![BrushDab::new(0.5, 0.5, 0.25, 0.5, 0.9)]),
                        adj,
                    ));
                });
                if let Some(idx) = self.selected_idx {
                    self.active_mask_idx = self.library.photos[idx].develop.masks.len().saturating_sub(1);
                }
            }
            "mask_add_linear" => {
                self.apply_develop_change(window, |d| {
                    let adj = MaskAdjustments { exposure_ev: -0.5, contrast: 0.15, ..MaskAdjustments::default() };
                    d.masks.push(LocalMask::linear_gradient(LinearGradientMask::default(), adj));
                });
                if let Some(idx) = self.selected_idx {
                    self.active_mask_idx = self.library.photos[idx].develop.masks.len().saturating_sub(1);
                }
            }
            "mask_add_radial" => {
                self.apply_develop_change(window, |d| {
                    let adj = MaskAdjustments { exposure_ev: 0.4, saturation: 0.15, ..MaskAdjustments::default() };
                    d.masks.push(LocalMask::radial(RadialMask::default(), adj));
                });
                if let Some(idx) = self.selected_idx {
                    self.active_mask_idx = self.library.photos[idx].develop.masks.len().saturating_sub(1);
                }
            }
            "mask_add_luma" => {
                self.apply_develop_change(window, |d| {
                    let adj = MaskAdjustments { highlights: -0.35, contrast: 0.15, ..MaskAdjustments::default() };
                    d.masks.push(LocalMask::luminance_range(LuminanceRangeMask::default(), adj));
                });
                if let Some(idx) = self.selected_idx {
                    self.active_mask_idx = self.library.photos[idx].develop.masks.len().saturating_sub(1);
                }
            }
            "mask_add_color" => {
                self.apply_develop_change(window, |d| {
                    let adj = MaskAdjustments { saturation: 0.35, ..MaskAdjustments::default() };
                    d.masks.push(LocalMask::color_range(ColorRangeMask::default(), adj));
                });
                if let Some(idx) = self.selected_idx {
                    self.active_mask_idx = self.library.photos[idx].develop.masks.len().saturating_sub(1);
                }
            }
            "mask_prev" => {
                if let Some(idx) = self.selected_idx {
                    let len = self.library.photos[idx].develop.masks.len();
                    if len > 0 {
                        self.active_mask_idx = if self.active_mask_idx == 0 { len - 1 } else { self.active_mask_idx - 1 };
                    }
                }
            }
            "mask_next" => {
                if let Some(idx) = self.selected_idx {
                    let len = self.library.photos[idx].develop.masks.len();
                    if len > 0 {
                        self.active_mask_idx = (self.active_mask_idx + 1) % len;
                    }
                }
            }
            "mask_toggle_enable" => {
                let m_idx = self.active_mask_idx;
                self.apply_develop_change(window, |d| {
                    if let Some(m) = d.masks.get_mut(m_idx) {
                        m.enabled = !m.enabled;
                    }
                });
            }
            "mask_toggle_invert" => {
                let m_idx = self.active_mask_idx;
                self.apply_develop_change(window, |d| {
                    if let Some(m) = d.masks.get_mut(m_idx) {
                        m.inverted = !m.inverted;
                    }
                });
            }
            "mask_delete" => {
                let m_idx = self.active_mask_idx;
                self.apply_develop_change(window, |d| {
                    if m_idx < d.masks.len() {
                        d.masks.remove(m_idx);
                    }
                });
            }
            "mask_ev_dec" | "mask_ev_inc" | "mask_con_dec" | "mask_con_inc" | "mask_hi_dec"
            | "mask_hi_inc" | "mask_sh_dec" | "mask_sh_inc" | "mask_temp_dec" | "mask_temp_inc"
            | "mask_sat_dec" | "mask_sat_inc" | "mask_sharp_dec" | "mask_sharp_inc" => {
                let m_idx = self.active_mask_idx;
                let action_name = name.to_string();
                self.apply_develop_change(window, |d| {
                    if let Some(m) = d.masks.get_mut(m_idx) {
                        match action_name.as_str() {
                            "mask_ev_dec" => m.adjustments.exposure_ev = (m.adjustments.exposure_ev - 0.25).clamp(-4.0, 4.0),
                            "mask_ev_inc" => m.adjustments.exposure_ev = (m.adjustments.exposure_ev + 0.25).clamp(-4.0, 4.0),
                            "mask_con_dec" => m.adjustments.contrast = (m.adjustments.contrast - 0.15).clamp(-1.0, 1.5),
                            "mask_con_inc" => m.adjustments.contrast = (m.adjustments.contrast + 0.15).clamp(-1.0, 1.5),
                            "mask_hi_dec" => m.adjustments.highlights = (m.adjustments.highlights - 0.15).clamp(-1.0, 1.0),
                            "mask_hi_inc" => m.adjustments.highlights = (m.adjustments.highlights + 0.15).clamp(-1.0, 1.0),
                            "mask_sh_dec" => m.adjustments.shadows = (m.adjustments.shadows - 0.15).clamp(-1.0, 1.0),
                            "mask_sh_inc" => m.adjustments.shadows = (m.adjustments.shadows + 0.15).clamp(-1.0, 1.0),
                            "mask_temp_dec" => m.adjustments.temperature = (m.adjustments.temperature - 0.15).clamp(-1.5, 1.5),
                            "mask_temp_inc" => m.adjustments.temperature = (m.adjustments.temperature + 0.15).clamp(-1.5, 1.5),
                            "mask_sat_dec" => m.adjustments.saturation = (m.adjustments.saturation - 0.15).clamp(-1.0, 1.5),
                            "mask_sat_inc" => m.adjustments.saturation = (m.adjustments.saturation + 0.15).clamp(-1.0, 1.5),
                            "mask_sharp_dec" => m.adjustments.sharpness = (m.adjustments.sharpness - 0.15).clamp(-1.0, 1.0),
                            "mask_sharp_inc" => m.adjustments.sharpness = (m.adjustments.sharpness + 0.15).clamp(-1.0, 1.0),
                            _ => {}
                        }
                    }
                });
            }
            "heal_add_center" => {
                self.apply_develop_change(window, |d| {
                    let offset = (d.heal_spots.len() as f32) * 0.04;
                    d.heal_spots.push(HealPatch::auto((0.5 + offset).min(0.85), 0.5, 0.04));
                });
            }
            "heal_clear" => {
                self.apply_develop_change(window, |d| d.heal_spots.clear());
            }
            other => {
                if let Some(star_s) = other.strip_prefix("cull_star_") {
                    if let Ok(stars) = star_s.parse::<u8>() {
                        self.set_selected_stars(window, stars);
                    }
                } else if let Some(slug) = other.strip_prefix("film_") {
                    let stock = film_stock_from_slug(slug);
                    self.apply_develop_change(window, |d| d.film_stock = stock);
                } else if let Some(slug) = other.strip_prefix("lut_") {
                    if let Some((title, lut)) = builtin_lut_preset(slug) {
                        self.apply_develop_change(window, |d| {
                            d.lut_name = Some(title);
                            d.lut = Some(lut);
                        });
                    }
                } else if let Some(slug) = other.strip_prefix("preset_") {
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
        let mut y = area.y + 10;
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
        y += 17;

        let nav_items = [
            (
                "nav_all",
                "All Photos",
                self.library.photos.len(),
                self.collection == SidebarCollection::AllPhotos,
            ),
            (
                "nav_keepers",
                "Keepers (Pick / >=3*)",
                self.library.keepers_count(),
                self.collection == SidebarCollection::Keepers,
            ),
            (
                "nav_picks",
                "Picks [P]",
                self.library.picks_count(),
                self.collection == SidebarCollection::Picks,
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
            let r = UiRect::new(x, y, row_w, 24);
            if active {
                stroke_round_rect(fb, w, h, r, 6, 1, COL_AMBER, COL_CARD_SEL);
            }
            let fg = if active { COL_AMBER } else { COL_TEXT };
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                r.x + 10,
                r.y + 5,
                title,
                11,
                fg,
                false,
                active,
                UiRect::new(r.x + 8, r.y + 2, r.w - 42, 20),
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
                COL_MUTED,
                true,
                false,
                r,
            );
            self.register_control(id, r);
            y += 26;
        }

        // SMART PLACES
        y += 8;
        draw_section_hdr(&mut self.painter, fb, y, "SMART PLACES (OFFLINE)");
        y += 17;
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
                let r = UiRect::new(x, y, row_w, 23);
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
                    UiRect::new(r.x + 8, r.y + 2, r.w - 34, 19),
                );
                let cnt_s = format!("{}", cnt);
                let cw = self.painter.measure_text(&cnt_s, 11, true);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    r.x + r.w - cw - 8,
                    r.y + 5,
                    &cnt_s,
                    11,
                    COL_DIM,
                    true,
                    false,
                    r,
                );
                let ctrl_name = format!("place:{}", place);
                self.register_control(&ctrl_name, r);
                y += 25;
            }
        }

        // SMART DATES (Year pills)
        y += 7;
        draw_section_hdr(&mut self.painter, fb, y, "SMART DATES");
        y += 17;
        let years = self.library.top_years(3);
        if !years.is_empty() {
            let mut px = x;
            for (yr, cnt) in years {
                let active = self.collection == SidebarCollection::YearFilter(yr);
                let pill_w = (row_w - 8) / 2;
                if px + pill_w > x + row_w {
                    px = x;
                    y += 25;
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
            y += 28;
        }

        // USER ALBUMS
        y += 6;
        draw_section_hdr(&mut self.painter, fb, y, "ALBUMS");
        y += 17;

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
                if y + 26 > area.y + area.h - 12 {
                    break;
                }
                let active = self.collection == SidebarCollection::UserAlbum(idx);
                let r = UiRect::new(x, y, row_w, 25);
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
                    UiRect::new(r.x + 8, r.y + 2, r.w - 34, 21),
                );
                self.register_control(&format!("album_{}", idx), r);

                let del_r = UiRect::new(r.x + r.w - 22, r.y + 4, 18, 17);
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
                y += 28;
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

        // Culling & Contact Sheet Strip
        let cull_h = 32i32;
        let cull_y = area.y + hdr_h;
        fill_rect(fb, w, h, UiRect::new(area.x, cull_y, area.w, cull_h), 0x0B121F);
        fill_rect(
            fb,
            w,
            h,
            UiRect::new(area.x, cull_y + cull_h - 1, area.w, 1),
            COL_BORDER,
        );

        let (sel_stars, sel_flag, sel_label) = self
            .selected_idx
            .and_then(|i| self.library.photos.get(i))
            .map(|p| (p.stars, p.flag, p.color_label))
            .unwrap_or((0, CullFlag::Unflagged, ColorLabel::None));

        let by = cull_y + 4;
        let bh = 23;
        let mut right_limit = area.x + area.w - 12;
        if area.w >= 260 {
            right_limit -= 76;
            self.draw_button(
                fb,
                w,
                h,
                "btn_contact_sheet",
                UiRect::new(right_limit, by, 76, bh),
                "Sheet [G]",
                COL_BTN,
                COL_BORDER,
                COL_TEXT,
                false,
            );
            right_limit -= 86;
            let keepers_on = self.collection == SidebarCollection::Keepers;
            self.draw_button(
                fb,
                w,
                h,
                "btn_keepers_filter",
                UiRect::new(right_limit, by, 80, bh),
                "Keepers [K]",
                if keepers_on { COL_EMERALD_DARK } else { COL_CARD },
                if keepers_on { COL_EMERALD } else { COL_BORDER },
                if keepers_on { COL_EMERALD } else { COL_TEXT },
                keepers_on,
            );
            right_limit -= 8;
        }

        let mut bx = area.x + 12;
        if bx + 64 <= right_limit {
            self.draw_button(
                fb,
                w,
                h,
                "btn_cull_mode",
                UiRect::new(bx, by, 62, bh),
                if self.cull_mode { "Cull ON" } else { "Cull [C]" },
                if self.cull_mode { COL_AMBER_DARK } else { COL_CARD },
                if self.cull_mode { COL_AMBER } else { COL_BORDER },
                if self.cull_mode { COL_AMBER } else { COL_TEXT },
                self.cull_mode,
            );
            bx += 68;
        }

        for star in 1u8..=5 {
            if bx + 26 > right_limit {
                break;
            }
            let lit = sel_stars >= star;
            let cid = format!("cull_star_{}", star);
            let lbl = format!("{}*", star);
            self.draw_button(
                fb,
                w,
                h,
                &cid,
                UiRect::new(bx, by, 25, bh),
                &lbl,
                if lit { COL_AMBER_DARK } else { COL_CARD },
                if lit { COL_AMBER } else { COL_BORDER },
                if lit { COL_AMBER } else { COL_MUTED },
                lit,
            );
            bx += 28;
        }
        bx += 4;

        if bx + 54 <= right_limit {
            let pick_on = sel_flag == CullFlag::Pick;
            self.draw_button(
                fb,
                w,
                h,
                "cull_pick",
                UiRect::new(bx, by, 54, bh),
                "Pick [P]",
                if pick_on { COL_EMERALD_DARK } else { COL_CARD },
                if pick_on { COL_EMERALD } else { COL_BORDER },
                if pick_on { COL_EMERALD } else { COL_TEXT },
                pick_on,
            );
            bx += 58;
        }
        if bx + 48 <= right_limit {
            let rej_on = sel_flag == CullFlag::Reject;
            self.draw_button(
                fb,
                w,
                h,
                "cull_reject",
                UiRect::new(bx, by, 48, bh),
                "Rej [X]",
                if rej_on { COL_ROSE_DARK } else { COL_CARD },
                if rej_on { COL_ROSE } else { COL_BORDER },
                if rej_on { COL_ROSE } else { COL_TEXT },
                rej_on,
            );
            bx += 52;
        }
        if bx + 66 <= right_limit {
            let lbl_col = sel_label.argb_u32() & 0x00FF_FFFF;
            let has_lbl = sel_label != ColorLabel::None;
            let lbl_txt = if has_lbl {
                sel_label.label().to_string()
            } else {
                "Label".to_string()
            };
            self.draw_button(
                fb,
                w,
                h,
                "cull_label_cycle",
                UiRect::new(bx, by, 62, bh),
                &lbl_txt,
                COL_CARD,
                if has_lbl { lbl_col } else { COL_BORDER },
                if has_lbl { lbl_col } else { COL_MUTED },
                has_lbl,
            );
        }

        let content = UiRect::new(
            area.x + 14,
            area.y + hdr_h + cull_h + 10,
            (area.w - 28).max(40),
            (area.h - hdr_h - cull_h - 20).max(40),
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
                "Try searching by year (2026), month (2026-05), place (Paris), or keepers / stars:4."
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

            // Top-left stars indicator on thumbnail
            if rec.stars > 0 {
                let st_r = UiRect::new(thumb_box.x + 6, thumb_box.y + 6, 58, 16);
                fill_round_rect(fb, w, h, st_r, 4, 0x0B101A);
                draw_star_row(
                    fb,
                    w,
                    h,
                    st_r.x + 5,
                    st_r.y + 4,
                    rec.stars,
                    COL_AMBER,
                    0x334155,
                );
            }

            // Top-right overlay badges on thumbnail
            let mut badge_x = thumb_box.x + thumb_box.w - 8;
            let min_badge_x = thumb_box.x + if rec.stars > 0 { 68 } else { 8 };

            if rec.flag != CullFlag::Unflagged && badge_x - 42 >= min_badge_x {
                let (fl_lbl, fl_fg, fl_bg) = if rec.flag == CullFlag::Pick {
                    ("PICK", COL_EMERALD, COL_EMERALD_DARK)
                } else {
                    ("REJ", COL_ROSE, COL_ROSE_DARK)
                };
                badge_x -= 40;
                let fl_r = UiRect::new(badge_x, thumb_box.y + 6, 40, 18);
                stroke_round_rect(fb, w, h, fl_r, 4, 1, fl_fg, fl_bg);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    fl_r.x + 6,
                    fl_r.y + 3,
                    fl_lbl,
                    10,
                    fl_fg,
                    false,
                    true,
                    fl_r,
                );
                badge_x -= 5;
            }
            if rec.is_edited() && badge_x - 56 >= min_badge_x {
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
                badge_x -= 5;
            }
            if rec.develop.film_stock.is_some()
                && rec.develop.film_strength > 1e-4
                && badge_x - 40 >= min_badge_x
            {
                badge_x -= 40;
                let fm_r = UiRect::new(badge_x, thumb_box.y + 6, 40, 18);
                stroke_round_rect(fb, w, h, fm_r, 4, 1, COL_INDIGO, COL_INDIGO_DARK);
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    fm_r.x + 6,
                    fm_r.y + 3,
                    "FILM",
                    10,
                    0xC7D2FE,
                    false,
                    true,
                    fm_r,
                );
                badge_x -= 5;
            }
            if rec.favorite && badge_x - 36 >= min_badge_x {
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

            // Card footer line 2: Color label swatch + Date pill + Offline Place pill
            let fy2 = fy1 + 22;
            let mut f2_x = card_r.x + 10;
            if rec.color_label != ColorLabel::None {
                let dot_r = UiRect::new(f2_x, fy2 + 4, 10, 10);
                fill_round_rect(fb, w, h, dot_r, 3, rec.color_label.argb_u32() & 0x00FF_FFFF);
                f2_x += 15;
            }
            let date_txt = rec
                .meta
                .date
                .map(|d| d.format_ymd())
                .unwrap_or_else(|| "No Date".to_string());
            let date_w = (self.painter.measure_text(&date_txt, 11, true) + 12).min(96);
            let date_pill = UiRect::new(f2_x, fy2, date_w, 19);
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
        let prev_r = UiRect::new(x, y, iw, 128);
        if let Some(t_img) = self.thumb_for(rec.id).cloned() {
            blit_image_fitted(fb, w, h, &t_img, prev_r, 0x090D16);
            stroke_rect(fb, w, h, prev_r, 1, COL_BORDER);
            y += 136;

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
            let hist_r = UiRect::new(x, y, iw, 54);
            draw_luma_histogram(fb, w, h, &t_img, hist_r);
            y += 62;
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
        let cull_val = format!(
            "{}* · {} · {}",
            rec.stars,
            rec.flag.label(),
            rec.color_label.label()
        );

        let rows = [
            ("File", rec.name.as_str()),
            ("Dimensions", dims_val.as_str()),
            ("Culling", cull_val.as_str()),
            ("Date Taken", date_val.as_str()),
            ("Place", place_val.as_str()),
            ("GPS", gps_val.as_str()),
            ("Camera", cam_val.as_str()),
            ("Lens", rec.meta.lens.as_deref().unwrap_or("Unknown")),
            ("Exposure", rec.meta.exposure.as_deref().unwrap_or("Unknown")),
            ("Albums", albums_val.as_str()),
        ];

        for (k, v) in rows {
            if y + 20 > area.y + area.h - 94 {
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
            y += 20;
        }

        // Non-destructive recipe & OmaRAW develop status card
        y += 6;
        let has_dev = !rec.develop.is_identity();
        let card_h = if has_dev { 84 } else { 68 };
        if y + card_h <= area.y + area.h - 6 {
            let box_r = UiRect::new(x, y, iw, card_h);
            let edited = rec.is_edited();
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
                    "NON-DESTRUCTIVE (.GAZE.TOML)"
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
            if has_dev {
                let dsum = rec.develop.summary();
                self.painter.draw_text_clipped(
                    fb,
                    w,
                    h,
                    box_r.x + 10,
                    box_r.y + 44,
                    &dsum,
                    10,
                    COL_AMBER,
                    false,
                    false,
                    UiRect::new(box_r.x + 8, box_r.y + 42, box_r.w - 16, 16),
                );
            }
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                box_r.x + 10,
                box_r.y + card_h - 20,
                "Source bytes untouched",
                10,
                COL_MUTED,
                false,
                false,
                UiRect::new(box_r.x + 8, box_r.y + card_h - 22, box_r.w - 16, 16),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_slider_row(
        &mut self,
        fb: &mut [u32],
        w: usize,
        h: usize,
        px: i32,
        mut y: i32,
        pw: i32,
        title: &str,
        val_s: &str,
        val: i32,
        max_abs: i32,
        bipolar: bool,
        dec_id: &str,
        inc_id: &str,
    ) -> i32 {
        let active = val != 0;
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
            UiRect::new(px, y - 2, 110, 15),
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
            if active { COL_AMBER } else { COL_MUTED },
            true,
            active,
            UiRect::new(px + 90, y - 2, (pw - 90).max(20), 15),
        );
        y += 14;

        let dec_r = UiRect::new(px, y, 24, 18);
        let inc_r = UiRect::new(px + pw - 24, y, 24, 18);
        self.draw_button(fb, w, h, dec_id, dec_r, "-", COL_CARD, COL_BORDER, COL_TEXT, true);
        self.draw_button(fb, w, h, inc_id, inc_r, "+", COL_CARD, COL_BORDER, COL_TEXT, true);

        let track = UiRect::new(px + 29, y + 5, (pw - 58).max(16), 8);
        fill_round_rect(fb, w, h, track, 4, 0x090E17);
        stroke_rect(fb, w, h, track, 1, COL_BORDER);
        let denom = max_abs.max(1);
        if bipolar {
            let mid = track.x + track.w / 2;
            fill_rect(fb, w, h, UiRect::new(mid, track.y - 2, 2, track.h + 4), COL_MUTED);
            if val != 0 {
                let span = ((val * (track.w / 2 - 2)) / denom).clamp(-(track.w / 2), track.w / 2);
                let (bx0, bw0) = if span >= 0 { (mid, span) } else { (mid + span, -span) };
                fill_rect(
                    fb,
                    w,
                    h,
                    UiRect::new(bx0, track.y + 1, bw0.max(2), track.h - 2),
                    COL_AMBER,
                );
            }
        } else if val > 0 {
            let span = ((val.clamp(0, denom) * (track.w - 2)) / denom).max(2);
            fill_rect(
                fb,
                w,
                h,
                UiRect::new(track.x + 1, track.y + 1, span, track.h - 2),
                COL_AMBER,
            );
        }

        y + 21
    }

    fn draw_section_label(&mut self, fb: &mut [u32], w: usize, h: usize, px: i32, y: i32, pw: i32, title: &str) {
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            px,
            y,
            title,
            10,
            COL_DIM,
            false,
            true,
            UiRect::new(px, y - 2, pw, 14),
        );
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
            UiRect::new(12, 10, 92, 28),
            "< Library",
            COL_BTN,
            COL_BORDER,
            COL_TEXT,
            true,
        );

        let (photo_name, recipe, develop, stars, flag) = self
            .selected_idx
            .and_then(|i| self.library.photos.get(i))
            .map(|p| (p.name.clone(), p.recipe.clone(), p.develop.clone(), p.stars, p.flag))
            .unwrap_or_else(|| {
                (
                    "Photo".to_string(),
                    EditRecipe::default(),
                    DevelopRecipe::default(),
                    0,
                    CullFlag::Unflagged,
                )
            });

        let title_end = self.painter.draw_text_clipped(
            fb,
            w,
            h,
            116,
            16,
            &photo_name,
            15,
            COL_TEXT,
            false,
            true,
            UiRect::new(114, 10, 210, 30),
        );

        let mut rx = w as i32 - 12;
        rx -= 96;
        self.draw_button(
            fb,
            w,
            h,
            "btn_export_copy",
            UiRect::new(rx, 10, 96, 28),
            "Save Copy [S]",
            COL_EMERALD_DARK,
            COL_EMERALD,
            COL_EMERALD,
            true,
        );
        rx -= 80;
        self.draw_button(
            fb,
            w,
            h,
            "btn_reset_edits",
            UiRect::new(rx, 10, 74, 28),
            "Reset [0]",
            COL_BTN,
            COL_BORDER,
            COL_TEXT,
            false,
        );
        rx -= 92;
        self.draw_button(
            fb,
            w,
            h,
            "btn_compare",
            UiRect::new(rx, 10, 86, 28),
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
        rx -= 104;
        let scope_on = self.scope_kind != ScopeKind::Off;
        let scope_lbl = format!("Scope:{} [V]", self.scope_kind.short_label());
        self.draw_button(
            fb,
            w,
            h,
            "btn_scope_toggle",
            UiRect::new(rx, 10, 98, 28),
            &scope_lbl,
            if scope_on { COL_INDIGO_DARK } else { COL_BTN },
            if scope_on { COL_INDIGO } else { COL_BORDER },
            if scope_on { 0xC7D2FE } else { COL_TEXT },
            scope_on,
        );

        if title_end + 170 < rx {
            let safe_pill = UiRect::new(title_end + 10, 13, 152, 22);
            stroke_round_rect(fb, w, h, safe_pill, 10, 1, COL_EMERALD, COL_EMERALD_DARK);
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                safe_pill.x + 10,
                safe_pill.y + 5,
                "NON-DESTRUCTIVE · SAFE",
                10,
                COL_EMERALD,
                false,
                true,
                safe_pill.inset(6, 2),
            );
        }

        // Right Studio Controls Panel
        let panel_w = if w >= 1100 {
            336i32.min((w as i32) / 2)
        } else {
            324i32.min((w as i32) / 2)
        };
        let body_y = top_h;
        let body_h = (h as i32 - body_y).max(1);
        let panel_r = UiRect::new(w as i32 - panel_w, body_y, panel_w, body_h);
        fill_rect(fb, w, h, panel_r, COL_SIDEBAR);
        fill_rect(fb, w, h, UiRect::new(panel_r.x, body_y, 1, body_h), COL_BORDER);

        // Left/Center Live Canvas + Optional Broadcast Scope Dock
        let canvas_r = UiRect::new(0, body_y, (w as i32 - panel_w).max(60), body_h);
        let scope_h = if self.scope_kind != ScopeKind::Off && canvas_r.h >= 340 {
            152i32
        } else {
            0i32
        };
        let stage = UiRect::new(
            canvas_r.x + 24,
            canvas_r.y + 18,
            (canvas_r.w - 48).max(20),
            (canvas_r.h - 62 - if scope_h > 0 { scope_h + 10 } else { 0 }).max(40),
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

            // Live Broadcast Video Scopes (`sex_color::scopes`)
            if scope_h > 0 {
                let scope_box = UiRect::new(
                    canvas_r.x + 24,
                    canvas_r.y + canvas_r.h - 42 - scope_h,
                    (canvas_r.w - 48).max(120),
                    scope_h,
                );
                stroke_round_rect(fb, w, h, scope_box, 7, 1, COL_BORDER, 0x090D16);

                let bar_y = scope_box.y + 4;
                let mut sx = scope_box.x + 8;
                let scope_tabs = [
                    ("scope_waveform_luma", "Wave", ScopeKind::WaveformLuma),
                    ("scope_waveform_rgb", "RGBW", ScopeKind::WaveformRgb),
                    ("scope_rgb_parade", "Parade", ScopeKind::RgbParade),
                    ("scope_vectorscope", "Vec", ScopeKind::Vectorscope),
                    ("scope_histogram", "Hist", ScopeKind::Histogram),
                ];
                for (sid, slbl, kind) in scope_tabs {
                    let bw = 54;
                    if sx + bw + 34 > scope_box.x + scope_box.w {
                        break;
                    }
                    let active = self.scope_kind == kind;
                    self.draw_button(
                        fb,
                        w,
                        h,
                        sid,
                        UiRect::new(sx, bar_y, bw, 20),
                        slbl,
                        if active { COL_INDIGO_DARK } else { COL_CARD },
                        if active { COL_AMBER } else { COL_BORDER },
                        if active { COL_AMBER } else { COL_MUTED },
                        active,
                    );
                    sx += bw + 4;
                }

                let close_r = UiRect::new(scope_box.x + scope_box.w - 28, bar_y, 20, 20);
                self.draw_button(fb, w, h, "scope_off", close_r, "x", COL_CARD, COL_BORDER, COL_MUTED, true);

                let stats = analyze_scope_stats(&img, ScopeStandard::Rec709);
                let stat_txt = format!(
                    "Mean {:.0}%  Shd {:.1}%  Hi {:.1}%",
                    stats.mean_ire, stats.shadow_clip_pct, stats.highlight_clip_pct
                );
                let st_w = self.painter.measure_text(&stat_txt, 10, true);
                let st_x = (close_r.x - st_w - 10).max(sx + 6);
                if st_x + 40 < close_r.x {
                    self.painter.draw_text_clipped(
                        fb,
                        w,
                        h,
                        st_x,
                        bar_y + 4,
                        &stat_txt,
                        10,
                        COL_MUTED,
                        true,
                        false,
                        UiRect::new(sx + 4, bar_y, (close_r.x - sx - 8).max(20), 18),
                    );
                }

                let plot_r = UiRect::new(
                    scope_box.x + 8,
                    scope_box.y + 28,
                    (scope_box.w - 16).max(20),
                    (scope_box.h - 34).max(20),
                );
                let surf_w = (plot_r.w as usize).clamp(200, 384);
                let surf_h = (plot_r.h as usize).clamp(96, 128);
                if self.scope_surface.width != surf_w || self.scope_surface.height != surf_h {
                    self.scope_surface = ScopeSurface::new(surf_w, surf_h);
                }
                render_scope_into(&img, self.scope_kind, &mut self.scope_surface, ScopeStandard::Rec709);
                blit_scope_surface(fb, w, h, &self.scope_surface, plot_r);
            }

            // Bottom Studio HUD pill
            let hud_r = UiRect::new(
                canvas_r.x + 24,
                canvas_r.y + canvas_r.h - 34,
                (canvas_r.w - 48).max(40),
                25,
            );
            stroke_round_rect(fb, w, h, hud_r, 6, 1, COL_BORDER, 0x101827);
            let cull_tag = format!("{}* {}", stars, flag.label());
            let hud_txt = if self.compare_original {
                format!("ORIGINAL SOURCE PREVIEW  ·  {}x{}  ·  {}", img.width, img.height, cull_tag)
            } else if develop.is_identity() {
                format!(
                    "LIVE: {}x{}  ·  {}  ·  {}",
                    img.width,
                    img.height,
                    recipe.summary(),
                    cull_tag
                )
            } else {
                format!(
                    "LIVE: {}x{}  ·  {}  ·  {}  ·  {}",
                    img.width,
                    img.height,
                    recipe.summary(),
                    develop.summary(),
                    cull_tag
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
                if self.compare_original { COL_AMBER } else { COL_MUTED },
                false,
                true,
                hud_r.inset(8, 2),
            );
        }

        // Right Studio Controls Header + 3-Tab Switcher
        let px = panel_r.x + 12;
        let pw = panel_r.w - 24;
        let mut y = panel_r.y + 10;

        let tab_gap = 6;
        let tab_w = (pw - tab_gap * 2) / 3;
        let tabs = [
            ("tab_tone", "Tone & FX", DevelopTab::ToneColor),
            ("tab_film", "Film & LUT", DevelopTab::FilmLut),
            ("tab_masks", "Masks & NR", DevelopTab::MasksDenoise),
        ];
        for (i, (tid, tlbl, kind)) in tabs.into_iter().enumerate() {
            let active = self.develop_tab == kind;
            let r = UiRect::new(px + (i as i32) * (tab_w + tab_gap), y, tab_w, 24);
            self.draw_button(
                fb,
                w,
                h,
                tid,
                r,
                tlbl,
                if active { COL_AMBER_DARK } else { COL_CARD },
                if active { COL_AMBER } else { COL_BORDER },
                if active { COL_AMBER } else { COL_TEXT },
                active,
            );
        }
        y += 30;

        match self.develop_tab {
            DevelopTab::ToneColor => {
                self.render_develop_tone_tab(fb, w, h, panel_r, px, y, pw, &recipe);
            }
            DevelopTab::FilmLut => {
                self.render_develop_film_tab(fb, w, h, panel_r, px, y, pw, &develop);
            }
            DevelopTab::MasksDenoise => {
                self.render_develop_masks_tab(fb, w, h, panel_r, px, y, pw, &develop);
            }
        }
    }

    fn render_develop_tone_tab(
        &mut self,
        fb: &mut [u32],
        w: usize,
        h: usize,
        panel_r: UiRect,
        px: i32,
        mut y: i32,
        pw: i32,
        recipe: &EditRecipe,
    ) {
        self.draw_section_label(fb, w, h, px, y, pw, "CROP & GEOMETRY");
        y += 16;

        let gap = 6;
        let cw4 = (pw - gap * 3) / 4;
        let aspects = [
            ("crop_free", "Free", recipe.aspect == CropAspect::Free && recipe.crop.is_full()),
            ("crop_1x1", "1:1", recipe.aspect == CropAspect::Square1x1),
            ("crop_4x3", "4:3", recipe.aspect == CropAspect::FourThree),
            ("crop_16x9", "16:9", recipe.aspect == CropAspect::SixteenNine),
        ];
        for (i, (cid, lbl, active)) in aspects.into_iter().enumerate() {
            let r = UiRect::new(px + (i as i32) * (cw4 + gap), y, cw4, 24);
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
        y += 28;

        let geo_btns = [
            ("crop_zoom_in", "Zoom +", !recipe.crop.is_full()),
            ("crop_reset", "Full", false),
            ("rot_cw", "Rot 90", recipe.quarter_turns % 4 != 0),
            ("flip_h", "Flip H", recipe.flip_h),
        ];
        for (i, (cid, lbl, active)) in geo_btns.into_iter().enumerate() {
            let r = UiRect::new(px + (i as i32) * (cw4 + gap), y, cw4, 24);
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
        y += 32;

        self.draw_section_label(fb, w, h, px, y, pw, "LIGHT & COLOR (FILMIC LUT + HSL)");
        y += 16;

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

        y = self.draw_slider_row(fb, w, h, px, y, pw, "Exposure", &ev_txt, recipe.exposure_ev_x100 as i32, 200, true, "ev_dec", "ev_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Contrast", &con_txt, recipe.contrast_pct as i32, 90, true, "con_dec", "con_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Warmth", &warm_txt, recipe.warmth_pct as i32, 90, true, "warm_dec", "warm_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Saturation", &sat_txt, recipe.saturation_pct as i32, 90, true, "sat_dec", "sat_inc");

        y += 4;
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
            UiRect::new(px, y - 2, 160, 15),
        );
        let b_pill = UiRect::new(hdr_end + 8, y - 2, 76, 15);
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
        y += 17;

        let col_w = (pw - 8) / 2;
        for (i, preset) in SharedFilterPreset::ALL.iter().copied().enumerate() {
            let col = (i as i32) % 2;
            let row = (i as i32) / 2;
            let ry = y + row * 25;
            if ry + 22 > panel_r.y + panel_r.h - 34 {
                break;
            }
            let rx = px + col * (col_w + 8);
            let r = UiRect::new(rx, ry, col_w, 22);
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

        let sy = (panel_r.y + panel_r.h - 32).max(y + 6 * 25 + 4);
        if sy + 25 <= panel_r.y + panel_r.h {
            let stack_cnt = recipe.filter_stack.iter().filter(|n| n.enabled).count();
            let add_w = (pw * 3) / 5 - 4;
            let clr_w = pw - add_w - 8;
            self.draw_button(
                fb,
                w,
                h,
                "stack_add_sharpen",
                UiRect::new(px, sy, add_w, 24),
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
                UiRect::new(px + add_w + 8, sy, clr_w, 24),
                &clr_lbl,
                if stack_cnt > 0 { COL_AMBER_DARK } else { COL_CARD },
                if stack_cnt > 0 { COL_AMBER } else { COL_BORDER },
                if stack_cnt > 0 { COL_AMBER } else { COL_MUTED },
                false,
            );
        }
    }

    fn render_develop_film_tab(
        &mut self,
        fb: &mut [u32],
        w: usize,
        h: usize,
        panel_r: UiRect,
        px: i32,
        mut y: i32,
        pw: i32,
        dev: &DevelopRecipe,
    ) {
        self.draw_section_label(fb, w, h, px, y, pw, "OMARAW TONE & VIBRANCE (SEX-COLOR)");
        y += 15;

        let to_pct = |v: f32| -> i32 {
            let s = v * 100.0;
            if s >= 0.0 { (s + 0.5) as i32 } else { (s - 0.5) as i32 }
        };

        let hi_p = to_pct(dev.highlights);
        let sh_p = to_pct(dev.shadows);
        let vib_p = to_pct(dev.grade.vibrance);
        let shp_p = to_pct(dev.sharpness);

        let hi_s = format!("{:+}%", hi_p);
        let sh_s = format!("{:+}%", sh_p);
        let vib_s = format!("{:+}%", vib_p);
        let shp_s = format!("{}%", shp_p);

        y = self.draw_slider_row(fb, w, h, px, y, pw, "Highlights", &hi_s, hi_p, 100, true, "hi_dec", "hi_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Shadows", &sh_s, sh_p, 100, true, "sh_dec", "sh_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Vibrance", &vib_s, vib_p, 150, true, "vib_dec", "vib_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Sharpness", &shp_s, shp_p, 100, false, "sharp_dec", "sharp_inc");

        y += 4;
        self.draw_section_label(fb, w, h, px, y, pw, "4-WAY COLOR WHEELS (LIFT/GAMMA/GAIN)");
        y += 15;

        let cw2 = (pw - 6) / 2;
        let wheel_neutral = to_pct(dev.grade.lift[0]) == 0
            && to_pct(dev.grade.gamma[0] - 1.0) == 0
            && to_pct(dev.grade.gain[0] - 1.0) == 0;
        let wheel_teal = to_pct(dev.grade.lift[2]) >= 2 && to_pct(dev.grade.gain[0]) >= 106;
        let wheel_skin = to_pct(dev.grade.gamma[0]) >= 103 && to_pct(dev.grade.lift[0]) > 0;
        let wheel_noir = to_pct(dev.grade.gamma[2]) >= 103 && to_pct(dev.grade.gain[2]) >= 104;

        let wheels = [
            ("wheel_neutral", "Neutral", wheel_neutral),
            ("wheel_teal_orange", "Teal / Orange", wheel_teal),
            ("wheel_warm_skin", "Warm Skin", wheel_skin),
            ("wheel_cool_noir", "Cool Noir", wheel_noir),
        ];
        for (i, (wid, wlbl, active)) in wheels.into_iter().enumerate() {
            let col = (i as i32) % 2;
            let row = (i as i32) / 2;
            let r = UiRect::new(px + col * (cw2 + 6), y + row * 24, cw2, 21);
            self.draw_button(
                fb,
                w,
                h,
                wid,
                r,
                wlbl,
                if active { COL_INDIGO_DARK } else { COL_CARD },
                if active { COL_AMBER } else { COL_BORDER },
                if active { COL_AMBER } else { COL_TEXT },
                active,
            );
        }
        y += 52;

        self.draw_section_label(fb, w, h, px, y, pw, "PHYSICAL FILM STOCK (12 PRESETS)");
        y += 15;

        let fstr_p = to_pct(dev.film_strength).clamp(0, 100);
        let no_film = dev.film_stock.is_none();
        self.draw_button(
            fb,
            w,
            h,
            "film_none",
            UiRect::new(px, y, 74, 21),
            "Off",
            if no_film { COL_AMBER_DARK } else { COL_CARD },
            if no_film { COL_AMBER } else { COL_BORDER },
            if no_film { COL_AMBER } else { COL_TEXT },
            no_film,
        );
        self.draw_button(fb, w, h, "film_str_dec", UiRect::new(px + 80, y, 24, 21), "-", COL_CARD, COL_BORDER, COL_TEXT, true);
        let f_lbl = format!("Str {}%", fstr_p);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            px + 110,
            y + 4,
            &f_lbl,
            11,
            if dev.film_stock.is_some() { COL_AMBER } else { COL_MUTED },
            true,
            false,
            UiRect::new(px + 108, y, pw - 140, 20),
        );
        self.draw_button(fb, w, h, "film_str_inc", UiRect::new(px + pw - 24, y, 24, 21), "+", COL_CARD, COL_BORDER, COL_TEXT, true);
        y += 25;

        for (i, stock) in FilmStock::ALL.iter().copied().enumerate() {
            let col = (i as i32) % 2;
            let row = (i as i32) / 2;
            let ry = y + row * 23;
            if ry + 21 > panel_r.y + panel_r.h - 78 {
                break;
            }
            let active = dev.film_stock == Some(stock);
            let cid = format!("film_{}", film_stock_slug(stock));
            let r = UiRect::new(px + col * (cw2 + 6), ry, cw2, 20);
            self.draw_button(
                fb,
                w,
                h,
                &cid,
                r,
                film_short_label(stock),
                if active { COL_INDIGO_DARK } else { COL_CARD },
                if active { COL_AMBER } else { COL_BORDER },
                if active { COL_AMBER } else { COL_TEXT },
                active,
            );
        }
        y += 6 * 23 + 4;

        if y + 72 <= panel_r.y + panel_r.h {
            self.draw_section_label(fb, w, h, px, y, pw, "3D .CUBE LUT (TETRAHEDRAL / TRILINEAR)");
            y += 15;

            let cur_lut = dev.lut_name.as_deref().unwrap_or("");
            let luts = [
                ("lut_none", "No LUT", dev.lut.is_none()),
                ("lut_teal_amber", "Teal/Amber", cur_lut == "Cine Teal & Amber"),
                ("lut_bleach_bypass", "Bleach", cur_lut == "Bleach Bypass"),
                ("lut_silver_mono", "Silver Mono", cur_lut == "Silver Halide Mono"),
            ];
            for (i, (lid, llbl, active)) in luts.into_iter().enumerate() {
                let col = (i as i32) % 2;
                let row = (i as i32) / 2;
                let r = UiRect::new(px + col * (cw2 + 6), y + row * 23, cw2, 20);
                self.draw_button(
                    fb,
                    w,
                    h,
                    lid,
                    r,
                    llbl,
                    if active { COL_AMBER_DARK } else { COL_CARD },
                    if active { COL_AMBER } else { COL_BORDER },
                    if active { COL_AMBER } else { COL_TEXT },
                    active,
                );
            }
            y += 48;

            let interp_lbl = match dev.lut_interp {
                InterpMode::Tetrahedral => "Interp: Tetra",
                InterpMode::Trilinear => "Interp: Trilin",
            };
            self.draw_button(fb, w, h, "lut_interp_toggle", UiRect::new(px, y, 108, 21), interp_lbl, COL_BTN, COL_BORDER, COL_TEXT, false);
            self.draw_button(fb, w, h, "lut_str_dec", UiRect::new(px + 114, y, 24, 21), "-", COL_CARD, COL_BORDER, COL_TEXT, true);
            let lstr_lbl = format!("Mix {}%", to_pct(dev.lut_strength).clamp(0, 100));
            self.painter.draw_text_clipped(
                fb,
                w,
                h,
                px + 144,
                y + 4,
                &lstr_lbl,
                11,
                if dev.lut.is_some() { COL_AMBER } else { COL_MUTED },
                true,
                false,
                UiRect::new(px + 142, y, pw - 172, 20),
            );
            self.draw_button(fb, w, h, "lut_str_inc", UiRect::new(px + pw - 24, y, 24, 21), "+", COL_CARD, COL_BORDER, COL_TEXT, true);
        }
    }

    fn render_develop_masks_tab(
        &mut self,
        fb: &mut [u32],
        w: usize,
        h: usize,
        panel_r: UiRect,
        px: i32,
        mut y: i32,
        pw: i32,
        dev: &DevelopRecipe,
    ) {
        let to_pct = |v: f32| -> i32 {
            let s = v * 100.0;
            if s >= 0.0 { (s + 0.5) as i32 } else { (s - 0.5) as i32 }
        };

        self.draw_section_label(fb, w, h, px, y, pw, "BILATERAL RAW DENOISE (OKLAB)");
        y += 15;

        let cw3 = (pw - 10) / 3;
        let dn_active = dev.denoise.is_active();
        let dn_hi = dev.denoise.chroma_strength >= 0.75;
        let dn_btns = [
            ("dn_off", "NR Off", !dn_active),
            ("dn_light", "Light NR", dn_active && !dn_hi),
            ("dn_high_iso", "High ISO", dn_active && dn_hi),
        ];
        for (i, (did, dlbl, active)) in dn_btns.into_iter().enumerate() {
            let r = UiRect::new(px + (i as i32) * (cw3 + 5), y, cw3, 22);
            self.draw_button(
                fb,
                w,
                h,
                did,
                r,
                dlbl,
                if active { COL_INDIGO_DARK } else { COL_CARD },
                if active { COL_AMBER } else { COL_BORDER },
                if active { COL_AMBER } else { COL_TEXT },
                active,
            );
        }
        y += 26;

        let luma_p = to_pct(dev.denoise.luma_strength).clamp(0, 100);
        let chroma_p = to_pct(dev.denoise.chroma_strength).clamp(0, 100);
        let luma_s = format!("{}%", luma_p);
        let chroma_s = format!("{}%", chroma_p);
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Luma NR", &luma_s, luma_p, 100, false, "dn_luma_dec", "dn_luma_inc");
        y = self.draw_slider_row(fb, w, h, px, y, pw, "Chroma NR", &chroma_s, chroma_p, 100, false, "dn_chroma_dec", "dn_chroma_inc");

        y += 4;
        self.draw_section_label(fb, w, h, px, y, pw, "LOCAL ADJUSTMENT MASKS (5 TYPES)");
        y += 15;

        let add1 = [
            ("mask_add_brush", "+ Brush"),
            ("mask_add_linear", "+ Linear"),
            ("mask_add_radial", "+ Radial"),
        ];
        for (i, (mid, mlbl)) in add1.into_iter().enumerate() {
            self.draw_button(
                fb,
                w,
                h,
                mid,
                UiRect::new(px + (i as i32) * (cw3 + 5), y, cw3, 21),
                mlbl,
                COL_BTN,
                COL_INDIGO,
                COL_TEXT,
                false,
            );
        }
        y += 24;

        let cw2 = (pw - 6) / 2;
        self.draw_button(fb, w, h, "mask_add_luma", UiRect::new(px, y, cw2, 21), "+ Luma Range", COL_BTN, COL_INDIGO, COL_TEXT, false);
        self.draw_button(fb, w, h, "mask_add_color", UiRect::new(px + cw2 + 6, y, cw2, 21), "+ Color Range", COL_BTN, COL_INDIGO, COL_TEXT, false);
        y += 25;

        let m_len = dev.masks.len();
        let active_mask = dev.masks.get(self.active_mask_idx);
        let m_label = if let Some(m) = active_mask {
            format!("{}/{} {}", self.active_mask_idx + 1, m_len, m.kind.label())
        } else {
            "No Masks (0)".to_string()
        };

        self.draw_button(fb, w, h, "mask_prev", UiRect::new(px, y, 22, 21), "<", COL_CARD, COL_BORDER, COL_TEXT, true);
        let status_r = UiRect::new(px + 25, y, (pw - 156).max(50), 21);
        stroke_round_rect(fb, w, h, status_r, 4, 1, COL_BORDER, 0x0A0F18);
        self.painter.draw_text_clipped(
            fb,
            w,
            h,
            status_r.x + 6,
            status_r.y + 4,
            &m_label,
            10,
            if active_mask.is_some() { COL_AMBER } else { COL_MUTED },
            false,
            true,
            status_r.inset(3, 2),
        );
        self.draw_button(fb, w, h, "mask_next", UiRect::new(status_r.x + status_r.w + 3, y, 22, 21), ">", COL_CARD, COL_BORDER, COL_TEXT, true);
        let en_on = active_mask.map_or(false, |m| m.enabled);
        let inv_on = active_mask.map_or(false, |m| m.inverted);
        let bx_r = status_r.x + status_r.w + 28;
        self.draw_button(
            fb,
            w,
            h,
            "mask_toggle_enable",
            UiRect::new(bx_r, y, 32, 21),
            if en_on { "On" } else { "Off" },
            if en_on { COL_EMERALD_DARK } else { COL_CARD },
            if en_on { COL_EMERALD } else { COL_BORDER },
            if en_on { COL_EMERALD } else { COL_MUTED },
            en_on,
        );
        self.draw_button(
            fb,
            w,
            h,
            "mask_toggle_invert",
            UiRect::new(bx_r + 35, y, 34, 21),
            "Inv",
            if inv_on { COL_INDIGO_DARK } else { COL_CARD },
            if inv_on { COL_INDIGO } else { COL_BORDER },
            if inv_on { 0xC7D2FE } else { COL_TEXT },
            inv_on,
        );
        self.draw_button(
            fb,
            w,
            h,
            "mask_delete",
            UiRect::new(px + pw - 34, y, 34, 21),
            "Del",
            COL_ROSE_DARK,
            COL_ROSE,
            COL_ROSE,
            false,
        );
        y += 26;

        let adj = active_mask.map(|m| m.adjustments).unwrap_or_default();
        let mev = to_pct(adj.exposure_ev);
        let mcon = to_pct(adj.contrast);
        let mhi = to_pct(adj.highlights);
        let msh = to_pct(adj.shadows);
        let mtmp = to_pct(adj.temperature);
        let msat = to_pct(adj.saturation);
        let mshp = to_pct(adj.sharpness);

        let mev_s = format!("{:+.2} EV", adj.exposure_ev);
        let mcon_s = format!("{:+}%", mcon);
        let mhi_s = format!("{:+}%", mhi);
        let msh_s = format!("{:+}%", msh);
        let mtmp_s = format!("{:+}%", mtmp);
        let msat_s = format!("{:+}%", msat);
        let mshp_s = format!("{:+}%", mshp);

        let mask_sliders = [
            ("Mask EV", mev_s.as_str(), mev, 400, "mask_ev_dec", "mask_ev_inc"),
            ("Mask Contrast", mcon_s.as_str(), mcon, 150, "mask_con_dec", "mask_con_inc"),
            ("Mask Highlights", mhi_s.as_str(), mhi, 100, "mask_hi_dec", "mask_hi_inc"),
            ("Mask Shadows", msh_s.as_str(), msh, 100, "mask_sh_dec", "mask_sh_inc"),
            ("Mask Temp", mtmp_s.as_str(), mtmp, 150, "mask_temp_dec", "mask_temp_inc"),
            ("Mask Sat", msat_s.as_str(), msat, 150, "mask_sat_dec", "mask_sat_inc"),
            ("Mask Sharp", mshp_s.as_str(), mshp, 100, "mask_sharp_dec", "mask_sharp_inc"),
        ];
        for (lbl, val_s, val, max_abs, dec_id, inc_id) in mask_sliders {
            if y + 34 > panel_r.y + panel_r.h - 42 {
                break;
            }
            y = self.draw_slider_row(fb, w, h, px, y, pw, lbl, val_s, val, max_abs, true, dec_id, inc_id);
        }

        y += 2;
        if y + 38 <= panel_r.y + panel_r.h {
            self.draw_section_label(fb, w, h, px, y, pw, "PATCH-MATCH SPOT HEAL");
            y += 15;
            let heal_n = dev.heal_spots.len();
            self.draw_button(
                fb,
                w,
                h,
                "heal_add_center",
                UiRect::new(px, y, cw2, 22),
                "+ Spot Heal",
                COL_BTN,
                COL_EMERALD,
                COL_EMERALD,
                false,
            );
            let hclr_lbl = format!("Clear Heals ({})", heal_n);
            self.draw_button(
                fb,
                w,
                h,
                "heal_clear",
                UiRect::new(px + cw2 + 6, y, cw2, 22),
                &hclr_lbl,
                if heal_n > 0 { COL_AMBER_DARK } else { COL_CARD },
                if heal_n > 0 { COL_AMBER } else { COL_BORDER },
                if heal_n > 0 { COL_AMBER } else { COL_MUTED },
                false,
            );
        }
    }
}

fn film_short_label(stock: FilmStock) -> &'static str {
    match stock {
        FilmStock::Kodak2383Print => "Kodak 2383",
        FilmStock::Kodak2393Print => "Kodak 2393",
        FilmStock::KodakVision3_5219 => "Vision3 5219",
        FilmStock::Portra160 => "Portra 160",
        FilmStock::Portra400 => "Portra 400",
        FilmStock::Portra800 => "Portra 800",
        FilmStock::Ektar100 => "Ektar 100",
        FilmStock::Gold200 => "Gold 200",
        FilmStock::Fuji3510Print => "Fuji 3510",
        FilmStock::Fuji3513Di => "Fuji 3513DI",
        FilmStock::AgfaCp30 => "Agfa CP30",
        FilmStock::CineonLog => "Cineon Log",
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
