// SPDX-License-Identifier: GPL-3.0-only
//! Bounded LRU poster thumbnail cache backed by `sex-image`.
//!
//! Keeps RAM tiny on SexOS by capping cached posters at `MAX_POSTERS` (32) and
//! downscaling every decoded poster to at most `THUMB_W x THUMB_H` (140x210,
//! ~117 KiB per poster -> ~3.7 MiB worst-case memory footprint).

use alloc::string::String;
use alloc::vec::Vec;
use sex_app::Window;
use sex_image::{Image, Limits};

use crate::stremio::http_get;

pub const MAX_POSTERS: usize = 32;
pub const THUMB_W: usize = 140;
pub const THUMB_H: usize = 210;

const MAX_FAILED: usize = 64;
const POSTER_LIMITS: Limits = Limits {
    max_width: 2048,
    max_height: 3072,
    max_pixels: 4_000_000,
    max_input_bytes: 2 << 20, // 2 MiB max download per poster
};

struct Entry {
    url: String,
    image: Image,
    last_used: u64,
}

pub struct PosterCache {
    entries: Vec<Entry>,
    failed: Vec<String>,
    clock: u64,
}

impl PosterCache {
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(MAX_POSTERS),
            failed: Vec::new(),
            clock: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn clear_failed(&mut self) {
        self.failed.clear();
    }

    /// Looks up a decoded poster thumbnail without triggering network I/O.
    pub fn get(&mut self, url: &str) -> Option<&Image> {
        if url.is_empty() {
            return None;
        }
        self.clock += 1;
        let now = self.clock;
        if let Some(pos) = self.entries.iter().position(|e| e.url == url) {
            self.entries[pos].last_used = now;
            return Some(&self.entries[pos].image);
        }
        None
    }

    /// Returns true if `url` is neither cached nor previously failed.
    pub fn needs_fetch(&self, url: &str) -> bool {
        if url.is_empty() || (!url.starts_with("http://") && !url.starts_with("https://")) {
            return false;
        }
        if self.entries.iter().any(|e| e.url == url) {
            return false;
        }
        if self.failed.iter().any(|f| f == url) {
            return false;
        }
        true
    }

    /// Fetches and decodes a single poster URL into the bounded LRU cache.
    /// Returns `true` if a new thumbnail was decoded and inserted.
    pub fn fetch_one(&mut self, window: &mut Window, url: &str) -> bool {
        if !self.needs_fetch(url) {
            return false;
        }
        let bytes = match http_get(window, url, POSTER_LIMITS.max_input_bytes) {
            Ok(b) => b,
            Err(_) => {
                self.record_failed(url);
                return false;
            }
        };
        let decoded = match sex_image::decode(&bytes, &POSTER_LIMITS) {
            Ok(img) => img,
            Err(_) => {
                self.record_failed(url);
                return false;
            }
        };
        let thumb = match decoded.fit_within(THUMB_W, THUMB_H) {
            Ok(img) => img,
            Err(_) => {
                self.record_failed(url);
                return false;
            }
        };

        if self.entries.len() >= MAX_POSTERS {
            // Evict least-recently-used poster.
            if let Some((lru_idx, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used)
            {
                self.entries.swap_remove(lru_idx);
            }
        }
        self.clock += 1;
        self.entries.push(Entry {
            url: String::from(url),
            image: thumb,
            last_used: self.clock,
        });
        true
    }

    fn record_failed(&mut self, url: &str) {
        if self.failed.len() >= MAX_FAILED {
            self.failed.remove(0);
        }
        self.failed.push(String::from(url));
    }
}
