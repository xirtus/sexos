// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The bar clock's time zone: the Shell reads it from settings and tells Display,
//! which shows local time from the same UTC sample. arg0 = zone index in
//! `sex_config::ZONES` order (Display falls back to UTC for one it does not know).

use core::sync::atomic::{AtomicU16, Ordering};

/// Shell -> Display.
pub const OP_CLOCK_ZONE: u64 = 0x2F1;

static ZONE: AtomicU16 = AtomicU16::new(0);
/// Display keeps the zone here; true when it changed.
pub fn set(zone: u64) -> bool {
    let zone = zone.min(u16::MAX as u64) as u16;
    ZONE.swap(zone, Ordering::Relaxed) != zone
}
pub fn zone() -> usize { ZONE.load(Ordering::Relaxed) as usize }
