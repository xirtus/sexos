// SPDX-License-Identifier: GPL-3.0-only
//! Byte-budgeted frame cache with low-memory mode for Splicy.
//!
//! Inspired by Rook's engine frame cache: tracks pixel byte usage,
//! enforces dual limits (max_entries and max_bytes), evicts LRU frames
//! on insert and budget reduction, and supports tolerance-based lookup.

use alloc::vec::Vec;
use sex_splice::Micros;

/// Configuration for the frame cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl CacheConfig {
    /// Standard budget: 256 entries, 128 MB.
    pub const fn standard() -> Self {
        Self {
            max_entries: 256,
            max_bytes: 128 * 1024 * 1024,
        }
    }

    /// Low memory budget tailored for tight guest RAM (e.g. SexOS): 64 entries, 16 MB.
    pub const fn low_memory() -> Self {
        Self {
            max_entries: 64,
            max_bytes: 16 * 1024 * 1024,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// A cached video frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedFrame {
    pub media: u32,
    pub source_us: Micros,
    pub width: usize,
    pub height: usize,
    pub quarter_res: bool,
    pub pixels: Vec<u32>,
}

impl CachedFrame {
    pub fn new(
        media: u32,
        source_us: Micros,
        width: usize,
        height: usize,
        quarter_res: bool,
        pixels: Vec<u32>,
    ) -> Self {
        Self {
            media,
            source_us,
            width,
            height,
            quarter_res,
            pixels,
        }
    }

    /// Size in bytes of the pixel buffer (4 bytes per u32 pixel).
    pub fn byte_len(&self) -> usize {
        self.pixels.len() * 4
    }
}

#[derive(Clone, Debug)]
struct CacheEntry {
    frame: CachedFrame,
    last_access: u64,
}

/// LRU frame cache with dual limits: maximum entry count and maximum total bytes.
#[derive(Clone, Debug)]
pub struct FrameCache {
    pub config: CacheConfig,
    pub total_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    tick: u64,
    entries: Vec<CacheEntry>,
}

impl Default for FrameCache {
    fn default() -> Self {
        Self::new(CacheConfig::standard())
    }
}

impl FrameCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            total_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            tick: 0,
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    /// Look up a cached frame within `tolerance_us` of `source_us`.
    pub fn get(
        &mut self,
        media: u32,
        source_us: Micros,
        tolerance_us: Micros,
        quarter_res: bool,
    ) -> Option<&CachedFrame> {
        let mut best_idx = None;
        let mut best_diff = i64::MAX;

        for (i, entry) in self.entries.iter().enumerate() {
            if entry.frame.media == media && entry.frame.quarter_res == quarter_res {
                let diff = (entry.frame.source_us - source_us).abs();
                if diff <= tolerance_us && diff < best_diff {
                    best_diff = diff;
                    best_idx = Some(i);
                }
            }
        }

        if let Some(idx) = best_idx {
            self.hits += 1;
            let tick = self.next_tick();
            self.entries[idx].last_access = tick;
            Some(&self.entries[idx].frame)
        } else {
            self.misses += 1;
            None
        }
    }

    /// Insert a frame into the cache. Evicts LRU entries until within budget.
    /// If the single frame exceeds `max_bytes`, it is cleanly refused without panicking.
    pub fn insert(&mut self, frame: CachedFrame) {
        let frame_bytes = frame.byte_len();
        if frame_bytes > self.config.max_bytes || self.config.max_entries == 0 {
            return;
        }

        // If an exact entry already exists, remove it first
        if let Some(pos) = self.entries.iter().position(|e| {
            e.frame.media == frame.media
                && e.frame.source_us == frame.source_us
                && e.frame.quarter_res == frame.quarter_res
        }) {
            let old = self.entries.remove(pos);
            self.total_bytes = self.total_bytes.saturating_sub(old.frame.byte_len());
        }

        // Evict LRU entries until BOTH count < max_entries AND total_bytes + frame_bytes <= max_bytes
        while !self.entries.is_empty()
            && (self.entries.len() >= self.config.max_entries
                || self.total_bytes + frame_bytes > self.config.max_bytes)
        {
            self.evict_lru();
        }

        let tick = self.next_tick();
        self.total_bytes += frame_bytes;
        self.entries.push(CacheEntry {
            frame,
            last_access: tick,
        });
    }

    fn evict_lru(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let lru_idx = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.last_access)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let evicted = self.entries.remove(lru_idx);
        self.total_bytes = self.total_bytes.saturating_sub(evicted.frame.byte_len());
        self.evictions += 1;
    }

    /// Resize budget at runtime. Immediately evicts entries exceeding the new limits.
    pub fn resize(&mut self, config: CacheConfig) {
        self.config = config;
        while !self.entries.is_empty()
            && (self.entries.len() > self.config.max_entries
                || self.total_bytes > self.config.max_bytes)
        {
            self.evict_lru();
        }
    }

    /// Toggle low memory mode.
    pub fn set_low_memory_mode(&mut self, enabled: bool) {
        let cfg = if enabled {
            CacheConfig::low_memory()
        } else {
            CacheConfig::standard()
        };
        self.resize(cfg);
    }

    /// Purge all cached frames for a closed media clip.
    pub fn evict_media(&mut self, media: u32) {
        let mut i = 0;
        while i < self.entries.len() {
            if self.entries[i].frame.media == media {
                let evicted = self.entries.remove(i);
                self.total_bytes = self.total_bytes.saturating_sub(evicted.frame.byte_len());
                self.evictions += 1;
            } else {
                i += 1;
            }
        }
    }
}
