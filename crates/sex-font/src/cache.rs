// SPDX-License-Identifier: GPL-3.0-only
//! Glyph bitmaps drawn once and kept: a fixed open-addressed table over a byte arena.
//!
//! A lookup is a hash and a short probe, cheap enough for per-pixel callers. When the
//! arena or the table fills, everything is forgotten at once and redrawn on demand,
//! which keeps the cache a fixed size with no allocator and no eviction lists.

/// What identifies one drawn bitmap. `size` and `phase` are whatever the caller's
/// layout needs (pixel size, subpixel offset); `w` x `h` is the bitmap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    pub font: u8,
    pub scalar: u32,
    pub size: u16,
    pub phase: u8,
    pub w: u8,
    pub h: u8,
}

impl Key {
    fn pack(self) -> u64 {
        // Bit 63 marks an occupied slot, so an empty slot is 0.
        1 << 63 | (self.font as u64 & 0x7f) << 56 | (self.scalar as u64 & 0x1f_ffff) << 35
            | (self.size as u64 & 0x7ff) << 24 | (self.phase as u64) << 16 | (self.w as u64) << 8 | self.h as u64
    }
}

const ABSENT: u32 = u32::MAX;

pub struct GlyphCache<const SLOTS: usize, const BYTES: usize> {
    keys: [u64; SLOTS],
    offsets: [u32; SLOTS],
    arena: [u8; BYTES],
    used: usize,
    count: usize,
    /// How many times the cache has been emptied, for diagnostics.
    pub resets: u32,
}

impl<const SLOTS: usize, const BYTES: usize> GlyphCache<SLOTS, BYTES> {
    pub const fn new() -> Self {
        Self { keys: [0; SLOTS], offsets: [0; SLOTS], arena: [0; BYTES], used: 0, count: 0, resets: 0 }
    }

    fn slot(&self, packed: u64) -> (usize, bool) {
        // Fibonacci hashing spreads neighbouring scalars and sizes over the table.
        let mut i = (packed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize % SLOTS;
        loop {
            if self.keys[i] == packed { return (i, true); }
            if self.keys[i] == 0 { return (i, false); }
            i = (i + 1) % SLOTS;
        }
    }

    /// The bitmap for `key`, drawing it with `draw` on first use. `draw` fills the
    /// `w * h` bytes it is given and returns `false` when there is no such glyph,
    /// which is remembered too. `None` means no glyph (or a bitmap too big to keep).
    pub fn get(&mut self, key: Key, draw: impl FnOnce(&mut [u8]) -> bool) -> Option<&[u8]> {
        let packed = key.pack();
        let len = key.w as usize * key.h as usize;
        let (mut i, found) = self.slot(packed);
        if !found {
            if len > BYTES { return None; }
            // Keep the table at most three quarters full so probes stay short.
            if self.used + len > BYTES || (self.count + 1) * 4 > SLOTS * 3 {
                self.keys.fill(0);
                self.used = 0;
                self.count = 0;
                self.resets = self.resets.wrapping_add(1);
                i = self.slot(packed).0;
            }
            let start = self.used;
            let drawn = draw(&mut self.arena[start..start + len]);
            self.keys[i] = packed;
            self.count += 1;
            if drawn { self.offsets[i] = start as u32; self.used += len; } else { self.offsets[i] = ABSENT; }
        }
        let start = self.offsets[i];
        (start != ABSENT).then(|| &self.arena[start as usize..start as usize + len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(scalar: u32) -> Key { Key { font: 1, scalar, size: 18, phase: 0, w: 4, h: 4 } }

    #[test]
    fn draws_once_and_remembers_missing_glyphs() {
        let mut cache = GlyphCache::<16, 64>::new();
        let mut draws = 0;
        for _ in 0..3 {
            let got = cache.get(key(65), |out| { draws += 1; out.fill(7); true }).map(|b| b.to_vec_len());
            assert_eq!(got, Some(16 * 7));
        }
        assert_eq!(draws, 1);
        assert!(cache.get(key(66), |_| false).is_none());
        assert!(cache.get(key(66), |_| panic!("missing glyph redrawn")).is_none());
    }

    #[test]
    fn full_arena_resets_and_keeps_answering() {
        let mut cache = GlyphCache::<16, 64>::new();
        for scalar in 0..40u32 {
            let b = cache.get(key(scalar), |out| { out.fill(scalar as u8); true }).unwrap();
            assert!(b.iter().all(|v| *v == scalar as u8));
        }
        assert!(cache.resets > 0);
    }

    trait Sum { fn to_vec_len(&self) -> u32; }
    impl Sum for [u8] { fn to_vec_len(&self) -> u32 { self.iter().map(|v| *v as u32).sum() } }
}
