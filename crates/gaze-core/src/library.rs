// SPDX-License-Identifier: GPL-3.0-only
//! Gaze Photo Library catalog, user albums, smart place/date aggregations,
//! and bounded deterministic `library.gaze` persistence.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::edits::EditRecipe;
use crate::metadata::{PhotoDate, PhotoMeta};
use crate::search::SmartQuery;

const MAGIC_HEADER: &str = "GAZE_LIBRARY_V1";
const MAX_SERIALIZED_LINES: usize = 16_384;
const MAX_ALBUMS: usize = 256;
const MAX_PHOTOS: usize = 8_192;

/// FNV-1a 64-bit deterministic hash for photo paths.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Catalog entry for a single photo in the Gaze library.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoRecord {
    pub id: u64,
    pub path: String,
    pub name: String,
    pub favorite: bool,
    pub meta: PhotoMeta,
    pub recipe: EditRecipe,
}

impl PhotoRecord {
    pub fn new(path: &str, meta: PhotoMeta) -> Self {
        let name = basename(path).to_string();
        Self {
            id: fnv1a_64(path.as_bytes()),
            path: path.to_string(),
            name,
            favorite: false,
            meta,
            recipe: EditRecipe::default(),
        }
    }
}

/// Named user album referencing photo paths non-destructively.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Album {
    pub name: String,
    pub photo_paths: Vec<String>,
}

impl Album {
    pub fn contains(&self, path: &str) -> bool {
        self.photo_paths.iter().any(|p| p == path)
    }
}

/// Active collection filter in the Gaze Library sidebar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SidebarCollection {
    #[default]
    AllPhotos,
    Favorites,
    Edited,
    PlaceFilter(String),
    YearFilter(u16),
    UserAlbum(usize),
}

impl SidebarCollection {
    pub fn label(&self, albums: &[Album]) -> String {
        match self {
            SidebarCollection::AllPhotos => "All Photos".to_string(),
            SidebarCollection::Favorites => "Favorites".to_string(),
            SidebarCollection::Edited => "Edited".to_string(),
            SidebarCollection::PlaceFilter(p) => format!("Place: {}", p),
            SidebarCollection::YearFilter(y) => format!("Year: {}", y),
            SidebarCollection::UserAlbum(idx) => albums
                .get(*idx)
                .map(|a| format!("Album: {}", a.name))
                .unwrap_or_else(|| "Album".to_string()),
        }
    }
}

/// Faces-free persistent Gaze photo catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhotoLibrary {
    pub photos: Vec<PhotoRecord>,
    pub albums: Vec<Album>,
}

impl PhotoLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates metadata for `path`, preserving existing `favorite` and `recipe`!
    pub fn upsert_photo(&mut self, path: &str, meta: PhotoMeta) -> usize {
        if let Some(idx) = self.photos.iter().position(|p| p.path == path) {
            let rec = &mut self.photos[idx];
            if meta.width > 0 {
                rec.meta.width = meta.width;
            }
            if meta.height > 0 {
                rec.meta.height = meta.height;
            }
            if !meta.format_name.is_empty() {
                rec.meta.format_name = meta.format_name;
            }
            if meta.date.is_some() {
                rec.meta.date = meta.date;
            }
            if meta.gps.is_some() {
                rec.meta.gps = meta.gps;
            }
            if meta.place.is_some() {
                rec.meta.place = meta.place;
            }
            if meta.place_region.is_some() {
                rec.meta.place_region = meta.place_region;
            }
            if meta.camera.is_some() {
                rec.meta.camera = meta.camera;
            }
            if meta.lens.is_some() {
                rec.meta.lens = meta.lens;
            }
            if meta.exposure.is_some() {
                rec.meta.exposure = meta.exposure;
            }
            return idx;
        }
        if self.photos.len() >= MAX_PHOTOS {
            return self.photos.len() - 1;
        }
        let idx = self.photos.len();
        self.photos.push(PhotoRecord::new(path, meta));
        idx
    }

    pub fn photo_by_path(&self, path: &str) -> Option<&PhotoRecord> {
        self.photos.iter().find(|p| p.path == path)
    }

    pub fn photo_by_path_mut(&mut self, path: &str) -> Option<&mut PhotoRecord> {
        self.photos.iter_mut().find(|p| p.path == path)
    }

    pub fn toggle_favorite(&mut self, path: &str) -> bool {
        if let Some(rec) = self.photo_by_path_mut(path) {
            rec.favorite = !rec.favorite;
            rec.favorite
        } else {
            false
        }
    }

    pub fn set_recipe(&mut self, path: &str, recipe: EditRecipe) -> bool {
        if let Some(rec) = self.photo_by_path_mut(path) {
            rec.recipe = recipe;
            true
        } else {
            false
        }
    }

    pub fn reset_recipe(&mut self, path: &str) -> bool {
        self.set_recipe(path, EditRecipe::default())
    }

    pub fn create_album(&mut self, raw_name: &str) -> usize {
        let trimmed = raw_name.trim();
        let base_name = if trimmed.is_empty() { "New Album" } else { trimmed };
        if let Some(idx) = self
            .albums
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(base_name))
        {
            return idx;
        }
        if self.albums.len() >= MAX_ALBUMS {
            return self.albums.len() - 1;
        }
        let idx = self.albums.len();
        self.albums.push(Album {
            name: base_name.to_string(),
            photo_paths: Vec::new(),
        });
        idx
    }

    pub fn rename_album(&mut self, idx: usize, new_name: &str) -> bool {
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Some(album) = self.albums.get_mut(idx) {
            album.name = trimmed.to_string();
            true
        } else {
            false
        }
    }

    pub fn delete_album(&mut self, idx: usize) -> bool {
        if idx < self.albums.len() {
            self.albums.remove(idx);
            true
        } else {
            false
        }
    }

    /// Toggles membership of `path` inside `album_idx`. Returns `true` if `path` is now in the album.
    pub fn toggle_photo_in_album(&mut self, album_idx: usize, path: &str) -> bool {
        let Some(album) = self.albums.get_mut(album_idx) else {
            return false;
        };
        if let Some(pos) = album.photo_paths.iter().position(|p| p == path) {
            album.photo_paths.remove(pos);
            false
        } else {
            album.photo_paths.push(path.to_string());
            true
        }
    }

    pub fn add_photo_to_album(&mut self, album_idx: usize, path: &str) -> bool {
        let Some(album) = self.albums.get_mut(album_idx) else {
            return false;
        };
        if !album.contains(path) {
            album.photo_paths.push(path.to_string());
        }
        true
    }

    pub fn albums_for_photo(&self, path: &str) -> Vec<&str> {
        self.albums
            .iter()
            .filter(|a| a.contains(path))
            .map(|a| a.name.as_str())
            .collect()
    }

    pub fn favorites_count(&self) -> usize {
        self.photos.iter().filter(|p| p.favorite).count()
    }

    pub fn edited_count(&self) -> usize {
        self.photos.iter().filter(|p| !p.recipe.is_identity()).count()
    }

    /// Returns distinct resolved places sorted by photo count descending, then alphabetically.
    pub fn top_places(&self, limit: usize) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for p in &self.photos {
            if let Some(place) = p.meta.place.as_deref().filter(|s| !s.is_empty()) {
                if let Some(entry) = counts.iter_mut().find(|(k, _)| k == place) {
                    entry.1 += 1;
                } else {
                    counts.push((place.to_string(), 1));
                }
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts.truncate(limit);
        counts
    }

    /// Returns distinct years sorted newest first.
    pub fn top_years(&self, limit: usize) -> Vec<(u16, usize)> {
        let mut counts: Vec<(u16, usize)> = Vec::new();
        for p in &self.photos {
            if let Some(d) = p.meta.date {
                if let Some(entry) = counts.iter_mut().find(|(y, _)| *y == d.year) {
                    entry.1 += 1;
                } else {
                    counts.push((d.year, 1));
                }
            }
        }
        counts.sort_by(|a, b| b.0.cmp(&a.0));
        counts.truncate(limit);
        counts
    }

    /// Computes filtered photo indices matching both `collection` and `query`,
    /// sorted deterministically (newest date first, tie-broken by filename).
    pub fn visible_indices(&self, collection: &SidebarCollection, query: &SmartQuery) -> Vec<usize> {
        let mut indices: Vec<usize> = Vec::new();
        for (idx, rec) in self.photos.iter().enumerate() {
            let in_collection = match collection {
                SidebarCollection::AllPhotos => true,
                SidebarCollection::Favorites => rec.favorite,
                SidebarCollection::Edited => !rec.recipe.is_identity(),
                SidebarCollection::PlaceFilter(place) => {
                    rec.meta.place.as_deref() == Some(place.as_str())
                }
                SidebarCollection::YearFilter(year) => {
                    rec.meta.date.map_or(false, |d| d.year == *year)
                }
                SidebarCollection::UserAlbum(a_idx) => self
                    .albums
                    .get(*a_idx)
                    .map_or(false, |a| a.contains(&rec.path)),
            };
            if !in_collection {
                continue;
            }
            let album_names = self.albums_for_photo(&rec.path);
            if query.matches(rec, &album_names) {
                indices.push(idx);
            }
        }

        indices.sort_by(|&ia, &ib| {
            let a = &self.photos[ia];
            let b = &self.photos[ib];
            let ka = a.meta.date.map(|d| d.sort_key()).unwrap_or(0);
            let kb = b.meta.date.map(|d| d.sort_key()).unwrap_or(0);
            kb.cmp(&ka)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.path.cmp(&b.path))
        });
        indices
    }

    /// Serializes the library catalog, albums, favorites, metadata, and non-destructive recipes
    /// to a deterministic line-oriented text format (`library.gaze`).
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str(MAGIC_HEADER);
        out.push('\n');

        for (idx, album) in self.albums.iter().enumerate() {
            out.push_str("ALBUM\t");
            out.push_str(&escape_field(&album.name));
            out.push('\n');
            for path in &album.photo_paths {
                out.push_str("ALBUM_ITEM\t");
                out.push_str(&format!("{}\t{}", idx, escape_field(path)));
                out.push('\n');
            }
        }

        for p in &self.photos {
            let date_s = p
                .meta
                .date
                .map(|d| d.format_datetime())
                .unwrap_or_else(|| "-".to_string());
            let (lat_s, lon_s) = match p.meta.gps {
                Some((lat, lon)) => (format!("{:.6}", lat), format!("{:.6}", lon)),
                None => ("-".to_string(), "-".to_string()),
            };
            let place_s = p
                .meta
                .place
                .as_deref()
                .map(escape_field)
                .unwrap_or_else(|| "-".to_string());
            let region_s = p
                .meta
                .place_region
                .as_deref()
                .map(escape_field)
                .unwrap_or_else(|| "-".to_string());
            let cam_s = p
                .meta
                .camera
                .as_deref()
                .map(escape_field)
                .unwrap_or_else(|| "-".to_string());
            let fmt_s = if p.meta.format_name.is_empty() {
                "-".to_string()
            } else {
                escape_field(&p.meta.format_name)
            };

            let opt = |v: &Option<String>| v.as_deref().map(escape_field).unwrap_or_else(|| "-".to_string());
            // Lens and exposure come after the recipe, so older catalogs (13 columns) still load.
            out.push_str(&format!(
                "PHOTO\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                escape_field(&p.path),
                p.favorite as u8,
                p.meta.width,
                p.meta.height,
                fmt_s,
                date_s,
                lat_s,
                lon_s,
                place_s,
                region_s,
                cam_s,
                p.recipe.encode_compact(),
                opt(&p.meta.lens),
                opt(&p.meta.exposure)
            ));
        }

        out
    }

    /// Parses a `library.gaze` catalog safely with bounded line and collection counts.
    pub fn deserialize(text: &str) -> Self {
        let mut lib = Self::new();
        let mut lines = text.lines();
        let Some(first) = lines.next() else {
            return lib;
        };
        if first.trim() != MAGIC_HEADER {
            return lib;
        }

        for line in lines.take(MAX_SERIALIZED_LINES) {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            match cols.first().copied() {
                Some("ALBUM") if cols.len() >= 2 && lib.albums.len() < MAX_ALBUMS => {
                    let name = unescape_field(cols[1]);
                    if !name.is_empty() {
                        lib.albums.push(Album {
                            name,
                            photo_paths: Vec::new(),
                        });
                    }
                }
                Some("ALBUM_ITEM") if cols.len() >= 3 => {
                    if let Ok(idx) = cols[1].parse::<usize>() {
                        let path = unescape_field(cols[2]);
                        if let Some(album) = lib.albums.get_mut(idx) {
                            if !path.is_empty() && !album.contains(&path) && album.photo_paths.len() < MAX_PHOTOS {
                                album.photo_paths.push(path);
                            }
                        }
                    }
                }
                Some("PHOTO") if cols.len() >= 13 && lib.photos.len() < MAX_PHOTOS => {
                    let path = unescape_field(cols[1]);
                    if path.is_empty() {
                        continue;
                    }
                    let favorite = cols[2] == "1";
                    let width = cols[3].parse::<u32>().unwrap_or(0);
                    let height = cols[4].parse::<u32>().unwrap_or(0);
                    let format_name = if cols[5] == "-" {
                        String::new()
                    } else {
                        unescape_field(cols[5])
                    };
                    let date = if cols[6] == "-" {
                        None
                    } else {
                        PhotoDate::parse_flexible(cols[6])
                    };
                    let gps = match (cols[7], cols[8]) {
                        ("-", _) | (_, "-") => None,
                        (la, lo) => match (la.parse::<f64>(), lo.parse::<f64>()) {
                            (Ok(lat), Ok(lon))
                                if lat.is_finite()
                                    && lon.is_finite()
                                    && (-90.0..=90.0).contains(&lat)
                                    && (-180.0..=180.0).contains(&lon) =>
                            {
                                Some((lat, lon))
                            }
                            _ => None,
                        },
                    };
                    let place = if cols[9] == "-" {
                        None
                    } else {
                        Some(unescape_field(cols[9]))
                    };
                    let place_region = if cols[10] == "-" {
                        None
                    } else {
                        Some(unescape_field(cols[10]))
                    };
                    let camera = if cols[11] == "-" {
                        None
                    } else {
                        Some(unescape_field(cols[11]))
                    };
                    let recipe = EditRecipe::decode_compact(cols[12]);
                    let extra = |i: usize| cols.get(i).filter(|c| **c != "-" && !c.is_empty()).map(|c| unescape_field(c));
                    let (lens, exposure) = (extra(13), extra(14));

                    let mut rec = PhotoRecord::new(
                        &path,
                        PhotoMeta {
                            width,
                            height,
                            format_name,
                            date,
                            gps,
                            place,
                            place_region,
                            camera,
                            lens,
                            exposure,
                        },
                    );
                    rec.favorite = favorite;
                    rec.recipe = recipe;
                    lib.photos.push(rec);
                }
                _ => {}
            }
        }
        lib
    }
}

pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed.strip_prefix('@').unwrap_or(trimmed),
    }
}

fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}
