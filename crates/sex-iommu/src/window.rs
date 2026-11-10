// SPDX-License-Identifier: GPL-3.0-only
//! A fixed device-address window a driver borrows pages into. One bit per
//! page; runs are found first-fit. The window and its tables are sized at
//! BOOT, so lending never allocates.
use crate::PAGE;

/// `WORDS * 64` pages of device addresses starting at `base`.
pub struct Window<const WORDS: usize> {
    pub base: u64,
    used: [u64; WORDS],
}

impl<const WORDS: usize> Window<WORDS> {
    pub const PAGES: u64 = WORDS as u64 * 64;

    pub const fn new(base: u64) -> Self { Self { base, used: [0; WORDS] } }

    pub fn bytes(&self) -> u64 { Self::PAGES * PAGE }

    fn taken(&self, page: u64) -> bool { self.used[(page / 64) as usize] & (1 << (page % 64)) != 0 }
    fn set(&mut self, page: u64, on: bool) {
        let word = &mut self.used[(page / 64) as usize];
        if on { *word |= 1 << (page % 64) } else { *word &= !(1 << (page % 64)) }
    }

    /// Reserve `pages` consecutive device pages; returns their address.
    pub fn take(&mut self, pages: u64) -> Option<u64> {
        if pages == 0 || pages > Self::PAGES { return None; }
        let mut start = 0;
        while start + pages <= Self::PAGES {
            match (start..start + pages).find(|&p| self.taken(p)) {
                Some(busy) => start = busy + 1,
                None => {
                    for p in start..start + pages { self.set(p, true); }
                    return Some(self.base + start * PAGE);
                }
            }
        }
        None
    }

    /// Give back a run `take` returned. Rejects anything else whole.
    pub fn give(&mut self, address: u64, pages: u64) -> bool {
        let Some(offset) = address.checked_sub(self.base) else { return false };
        if offset % PAGE != 0 || pages == 0 { return false; }
        let first = offset / PAGE;
        if first.checked_add(pages).map_or(true, |end| end > Self::PAGES) { return false; }
        if !(first..first + pages).all(|p| self.taken(p)) { return false; }
        for p in first..first + pages { self.set(p, false); }
        true
    }

    pub fn contains(&self, address: u64) -> bool {
        address >= self.base && address - self.base < Self::PAGES * PAGE
    }

    pub fn pages_used(&self) -> u64 { self.used.iter().map(|w| w.count_ones() as u64).sum() }
}

/// Lowest `align`-aligned base at or above `floor` where a window of `bytes`
/// overlaps none of `busy` ([start, end) spans) and stays under `limit`.
pub fn place(busy: &[(u64, u64)], bytes: u64, align: u64, floor: u64, limit: u64) -> Option<u64> {
    let mut base = floor.checked_add(align - 1)? & !(align - 1);
    loop {
        let end = base.checked_add(bytes)?;
        if end > limit { return None; }
        match busy.iter().filter(|(s, e)| *s < end && base < *e).map(|(_, e)| *e).max() {
            None => return Some(base),
            Some(past) => base = past.checked_add(align - 1)? & !(align - 1),
        }
    }
}
