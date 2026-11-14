// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! SCL clock counts from the controller's input clock, for firmware that gives none.
//!
//! Firmware usually names exact counts (`SSCN`, `FMCN` on the controller's ACPI
//! device); use those when present. Otherwise these counts keep every I2C timing
//! minimum (tLOW 4.7/1.3 µs, tHIGH 4.0/0.6 µs) and never run the bus above its rate,
//! whatever the rise time.
use crate::{Counts, Speed};

/// Input clock of Intel Serial IO I2C from Ice Lake/Tiger Lake/Alder Lake on (100 MHz).
pub const INTEL_TGL_HZ: u32 = 100_000_000;
/// Cannon Lake and Comet Lake run it at 216 MHz.
pub const INTEL_CNL_HZ: u32 = 216_000_000;

/// The input clock of an Intel Serial IO I2C function, by PCI product id.
pub fn intel_clock(device: u16) -> u32 {
    match device {
        // Cannon Lake and Comet Lake PCH (LP and H).
        0x9dc5 | 0x9dc6 | 0x9de8..=0x9deb | 0xa368 | 0xa369 | 0xa36a | 0xa36b
        | 0x02c5 | 0x02c6 | 0x02e8..=0x02eb | 0x06e8..=0x06eb | 0xa3e0..=0xa3e3 => INTEL_CNL_HZ,
        // Sunrise Point / Kaby Lake PCH.
        0x9d60..=0x9d65 | 0xa160..=0xa166 => 120_000_000,
        _ => INTEL_TGL_HZ,
    }
}

fn cycles(clock_hz: u32, ns: u32) -> u32 { ((clock_hz as u64 * ns as u64 + 999_999_999) / 1_000_000_000) as u32 }

/// Counts for `speed` at `clock_hz`, with an SDA hold of `hold_ns`.
pub fn counts(clock_hz: u32, speed: Speed, hold_ns: u32) -> Counts {
    let (rate, low_min, high_min) = match speed { Speed::Standard => (100_000, 4700, 4000), Speed::Fast => (400_000, 1300, 600) };
    let period = clock_hz.div_ceil(rate);
    // The controller adds about 8 clocks of its own to each SCL period.
    let low = cycles(clock_hz, low_min).max(period * 55 / 100);
    let high = period.saturating_sub(low + 8).max(cycles(clock_hz, high_min));
    Counts { high: high.min(0xffff) as u16, low: low.min(0xffff) as u16, sda_hold: cycles(clock_hz, hold_ns).min(0xffff) as u16 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(clock: u32, c: Counts) -> u32 { clock / (c.high as u32 + c.low as u32 + 8) }

    #[test]
    fn fast_mode_never_exceeds_400_khz_and_keeps_minimums() {
        for clock in [INTEL_TGL_HZ, INTEL_CNL_HZ, 120_000_000, 133_000_000] {
            let c = counts(clock, Speed::Fast, 300);
            assert!(rate(clock, c) <= 400_000, "{clock}: {c:?}");
            assert!(c.low as u64 * 1_000_000_000 / clock as u64 >= 1300);
            assert!(c.high as u64 * 1_000_000_000 / clock as u64 >= 600);
        }
        assert_eq!(counts(INTEL_TGL_HZ, Speed::Fast, 300), Counts { high: 105, low: 137, sda_hold: 30 });
    }

    #[test]
    fn standard_mode_never_exceeds_100_khz() {
        for clock in [INTEL_TGL_HZ, INTEL_CNL_HZ] {
            let c = counts(clock, Speed::Standard, 300);
            assert!(rate(clock, c) <= 100_000);
            assert!(c.low as u64 * 1_000_000_000 / clock as u64 >= 4700);
        }
    }

    #[test]
    fn knows_the_clock_of_comet_and_tiger_lake() {
        assert_eq!(intel_clock(0x02e8), INTEL_CNL_HZ);
        assert_eq!(intel_clock(0x43e8), INTEL_TGL_HZ);
        assert_eq!(intel_clock(0xa0e8), INTEL_TGL_HZ);
    }
}
