// SPDX-License-Identifier: GPL-3.0-only
//! Thermal zones (ACPI 6.5 section 11): temperatures are tenths of a kelvin.

/// Plausible readings only: below -50 C or above 200 C is a firmware placeholder.
pub fn plausible(decikelvin: u64) -> bool { (2232..=4732).contains(&decikelvin) }

/// Whole degrees Celsius.
pub fn celsius(decikelvin: u32) -> i32 { (decikelvin as i32 - 2732 + if decikelvin >= 2732 { 5 } else { -5 }) / 10 }

/// A zone's reading against its critical trip point (`_CRT`). Firmware placeholders on
/// either side never trip it.
pub fn critical(temperature: Option<u64>, crt: Option<u64>) -> bool {
    match (temperature, crt) {
        (Some(t), Some(c)) => plausible(t) && plausible(c) && t >= c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_and_trip() {
        assert_eq!(celsius(3132), 40);
        assert_eq!(celsius(2732), 0);
        assert_eq!(celsius(2632), -10);
        assert!(critical(Some(3782), Some(3782)));
        assert!(!critical(Some(3781), Some(3782)));
        assert!(!critical(Some(0xFFFF_FFFF), Some(3782)));
        assert!(!critical(Some(3800), Some(0)));
        assert!(!critical(Some(3800), None));
    }
}
