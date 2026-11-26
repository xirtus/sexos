// SPDX-License-Identifier: GPL-3.0-only
//! Alienware Command Center (AWCC) over WMI: fans, temperatures and thermal profiles.
//!
//! Alienware laptops (the x17 R1 among them) expose their fan controller through the
//! `WMAX` method of WMI GUID `A70591CE-...` (see [`crate::wmi`]). Every call takes a
//! 4-byte buffer `[operation, arg1, arg2, arg3]` and answers one 32-bit word, with
//! `0xFFFFFFFF`/`0xFFFFFFFE` meaning "refused". Method 0x14 reads (system description,
//! resource ids, temperatures, fan speeds, the current profile), method 0x15 writes
//! (activate a profile, set a fan's boost) and method 0x13 tells which sensors drive a fan.
//!
//! The system description is four counts in one word: fans, temperature sensors, unknown
//! resources and thermal profiles. Resource ids are then read by index in that order. The
//! protocol matches Linux's `alienware-wmi-wmax` (which lists the "Alienware x17" with
//! fan monitoring and platform profiles); nothing here is Linux code.
//!
//! Everything goes through [`Firmware`], so the logic is tested on the host and
//! `servers/sexacpi` supplies the AML call.

use crate::profile::Profile;

pub use crate::wmi::guids::ALIENWARE_WMAX as GUID;

pub const METHOD_FAN_SENSORS: u32 = 0x13;
pub const METHOD_THERMAL_INFORMATION: u32 = 0x14;
pub const METHOD_THERMAL_CONTROL: u32 = 0x15;
pub const METHOD_GAME_SHIFT: u32 = 0x25;

// Method 0x13
pub const OP_FAN_TEMP_COUNT: u8 = 0x01;
pub const OP_FAN_TEMP_ID: u8 = 0x02;
// Method 0x14
pub const OP_SYSTEM_DESCRIPTION: u8 = 0x02;
pub const OP_RESOURCE_ID: u8 = 0x03;
pub const OP_TEMPERATURE: u8 = 0x04;
pub const OP_FAN_RPM: u8 = 0x05;
pub const OP_FAN_MIN_RPM: u8 = 0x08;
pub const OP_FAN_MAX_RPM: u8 = 0x09;
pub const OP_CURRENT_PROFILE: u8 = 0x0B;
pub const OP_FAN_BOOST: u8 = 0x0C;
// Method 0x15
pub const OP_ACTIVATE_PROFILE: u8 = 0x01;
pub const OP_SET_FAN_BOOST: u8 = 0x02;
// Method 0x25
pub const OP_TOGGLE_GAME_SHIFT: u8 = 0x01;
pub const OP_GAME_SHIFT_STATUS: u8 = 0x02;

pub const FAILURE: u32 = 0xFFFF_FFFF;
pub const FAILURE_2: u32 = 0xFFFF_FFFE;
/// Firmware never lists more than this many of one resource kind; more means garbage.
pub const MAX_RESOURCES: usize = 16;

/// The 4-byte input buffer as a little-endian word.
pub const fn args(operation: u8, arg1: u8, arg2: u8, arg3: u8) -> u32 {
    operation as u32 | (arg1 as u32) << 8 | (arg2 as u32) << 16 | (arg3 as u32) << 24
}

/// Where `WMAX` is: `servers/sexacpi` evaluates `WMAX(0, method, buffer)`.
pub trait Firmware {
    /// The answer word, or `None` when the method failed or returned nothing usable.
    fn call(&mut self, method: u32, args: u32) -> Option<u32>;
}

fn call(fw: &mut impl Firmware, method: u32, a: u32) -> Option<u32> {
    fw.call(method, a).filter(|v| *v != FAILURE && *v != FAILURE_2)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Description { pub fans: u8, pub temps: u8, pub unknown: u8, pub profiles: u8 }

impl Description {
    pub fn from_word(w: u32) -> Option<Self> {
        let d = Self { fans: w as u8, temps: (w >> 8) as u8, unknown: (w >> 16) as u8, profiles: (w >> 24) as u8 };
        [d.fans, d.temps, d.unknown, d.profiles].iter().all(|c| (*c as usize) <= MAX_RESOURCES).then_some(d)
    }
    pub fn word(&self) -> u32 { args(self.fans, self.temps, self.unknown, self.profiles) }
}

/// What a thermal profile id means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Custom, Quiet, Balanced, BalancedPerformance, Performance, Cool, LowPower, GMode }

pub const PROFILE_CUSTOM: u8 = 0x00;
pub const PROFILE_LEGACY_QUIET: u8 = 0x96;
pub const PROFILE_LEGACY_BALANCED: u8 = 0x97;
pub const PROFILE_LEGACY_BALANCED_PERFORMANCE: u8 = 0x98;
pub const PROFILE_LEGACY_PERFORMANCE: u8 = 0x99;
pub const PROFILE_USTT_BALANCED: u8 = 0xA0;
pub const PROFILE_USTT_BALANCED_PERFORMANCE: u8 = 0xA1;
pub const PROFILE_USTT_COOL: u8 = 0xA2;
pub const PROFILE_USTT_QUIET: u8 = 0xA3;
pub const PROFILE_USTT_PERFORMANCE: u8 = 0xA4;
pub const PROFILE_USTT_LOW_POWER: u8 = 0xA5;
pub const PROFILE_GMODE: u8 = 0xAB;

pub fn kind(id: u8) -> Option<Kind> {
    Some(match id {
        PROFILE_CUSTOM => Kind::Custom,
        PROFILE_LEGACY_QUIET | PROFILE_USTT_QUIET => Kind::Quiet,
        PROFILE_LEGACY_BALANCED | PROFILE_USTT_BALANCED => Kind::Balanced,
        PROFILE_LEGACY_BALANCED_PERFORMANCE | PROFILE_USTT_BALANCED_PERFORMANCE => Kind::BalancedPerformance,
        PROFILE_LEGACY_PERFORMANCE | PROFILE_USTT_PERFORMANCE => Kind::Performance,
        PROFILE_USTT_COOL => Kind::Cool,
        PROFILE_USTT_LOW_POWER => Kind::LowPower,
        PROFILE_GMODE => Kind::GMode,
        _ => return None,
    })
}

pub fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Custom => "custom",
        Kind::Quiet => "quiet",
        Kind::Balanced => "balanced",
        Kind::BalancedPerformance => "balanced_performance",
        Kind::Performance => "performance",
        Kind::Cool => "cool",
        Kind::LowPower => "low_power",
        Kind::GMode => "gmode",
    }
}

/// The firmware profile that matches the desktop's power profile, from the ones this
/// machine lists, in order of preference. G-Mode is never chosen on its own (it is a
/// separate toggle that runs the fans flat out).
pub fn for_profile(p: Profile, listed: &[u8]) -> Option<u8> {
    let wanted: &[Kind] = match p {
        Profile::Performance => &[Kind::Performance, Kind::BalancedPerformance, Kind::Balanced],
        Profile::Balanced => &[Kind::Balanced, Kind::BalancedPerformance],
        Profile::Saver => &[Kind::Quiet, Kind::LowPower, Kind::Cool, Kind::Balanced],
    };
    wanted.iter().find_map(|w| listed.iter().copied().find(|id| kind(*id) == Some(*w)))
}

/// Fan names by resource id.
pub fn fan_name(id: u8) -> &'static str {
    match id {
        0x32 => "cpu", 0x33 => "gpu", 0x34 => "pci", 0x35 => "mid", 0x36 => "top",
        0x37 => "side", 0x38 | 0x39 => "u2", 0x3A | 0x3F => "front", 0x3B => "cpu2",
        0x3C => "gpu2", 0x3D | 0x3E => "top", 0x40 | 0x41 => "bottom", _ => "fan",
    }
}

/// Temperature sensor names by resource id.
pub fn sensor_name(id: u8) -> &'static str {
    match id { 0x01 => "cpu", 0x03 => "front", 0x06 => "gpu", _ => "sensor" }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fan { pub id: u8, pub min_rpm: u16, pub max_rpm: u16 }

/// What the firmware lists, read once at start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub description: Description,
    pub fans: [Fan; MAX_RESOURCES],
    pub fan_count: usize,
    pub sensors: [u8; MAX_RESOURCES],
    pub sensor_count: usize,
    pub profiles: [u8; MAX_RESOURCES],
    pub profile_count: usize,
}

impl Inventory {
    pub fn fans(&self) -> &[Fan] { &self.fans[..self.fan_count] }
    pub fn sensors(&self) -> &[u8] { &self.sensors[..self.sensor_count] }
    pub fn profiles(&self) -> &[u8] { &self.profiles[..self.profile_count] }
}

fn rpm(v: u32) -> u16 { v.min(u16::MAX as u32) as u16 }

/// Reads the description and every resource id. `None` when `WMAX` does not answer like
/// an AWCC controller.
pub fn probe(fw: &mut impl Firmware) -> Option<Inventory> {
    let d = Description::from_word(call(fw, METHOD_THERMAL_INFORMATION, args(OP_SYSTEM_DESCRIPTION, 0, 0, 0))?)?;
    let mut inv = Inventory { description: d, ..Inventory::default() };
    let id_at = |fw: &mut _, i: usize| call(fw, METHOD_THERMAL_INFORMATION, args(OP_RESOURCE_ID, i as u8, 0, 0)).map(|v| v as u8);
    for i in 0..d.fans as usize {
        let Some(id) = id_at(fw, i) else { break };
        let min = call(fw, METHOD_THERMAL_INFORMATION, args(OP_FAN_MIN_RPM, id, 0, 0)).map_or(0, rpm);
        let max = call(fw, METHOD_THERMAL_INFORMATION, args(OP_FAN_MAX_RPM, id, 0, 0)).map_or(0, rpm);
        inv.fans[inv.fan_count] = Fan { id, min_rpm: min, max_rpm: max };
        inv.fan_count += 1;
    }
    let base = d.fans as usize;
    for i in 0..d.temps as usize {
        let Some(id) = id_at(fw, base + i) else { break };
        inv.sensors[inv.sensor_count] = id;
        inv.sensor_count += 1;
    }
    // Profiles come last; some machines report more than they list, so stop at a refusal.
    let base = base + d.temps as usize + d.unknown as usize;
    for i in 0..d.profiles as usize {
        let Some(id) = id_at(fw, base + i) else { break };
        if kind(id).is_none() || id == PROFILE_GMODE { continue; }
        inv.profiles[inv.profile_count] = id;
        inv.profile_count += 1;
    }
    Some(inv)
}

/// Plausible degrees Celsius only (firmware returns 0, 2 or 255 for a missing or D3cold sensor).
pub fn plausible_celsius(v: u32) -> Option<u8> { (10..=125).contains(&v).then_some(v as u8) }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub rpm: [Option<u16>; MAX_RESOURCES],
    pub celsius: [Option<u8>; MAX_RESOURCES],
    pub profile: Option<u8>,
}

impl Reading {
    /// The hottest sensor, in tenths of a kelvin (the unit ACPI thermal zones use).
    pub fn hottest_decikelvin(&self) -> Option<u32> {
        self.celsius.iter().flatten().max().map(|c| *c as u32 * 10 + 2732)
    }
}

pub fn read(fw: &mut impl Firmware, inv: &Inventory) -> Reading {
    let mut r = Reading::default();
    for (i, f) in inv.fans().iter().enumerate() {
        r.rpm[i] = call(fw, METHOD_THERMAL_INFORMATION, args(OP_FAN_RPM, f.id, 0, 0)).map(rpm);
    }
    for (i, s) in inv.sensors().iter().enumerate() {
        r.celsius[i] = call(fw, METHOD_THERMAL_INFORMATION, args(OP_TEMPERATURE, *s, 0, 0)).and_then(plausible_celsius);
    }
    r.profile = call(fw, METHOD_THERMAL_INFORMATION, args(OP_CURRENT_PROFILE, 0, 0, 0)).map(|v| v as u8);
    r
}

/// Activates a listed profile (or custom, for manual fan boost). Unlisted ids are refused
/// without calling the firmware.
pub fn activate(fw: &mut impl Firmware, inv: &Inventory, id: u8) -> bool {
    if id != PROFILE_CUSTOM && !inv.profiles().contains(&id) { return false; }
    call(fw, METHOD_THERMAL_CONTROL, args(OP_ACTIVATE_PROFILE, id, 0, 0)).is_some()
}

/// Extra fan speed on top of the firmware curve (0 = none, 255 = full). Only takes effect
/// in the custom profile.
pub fn set_boost(fw: &mut impl Firmware, inv: &Inventory, fan: u8, boost: u8) -> bool {
    if !inv.fans().iter().any(|f| f.id == fan) { return false; }
    call(fw, METHOD_THERMAL_CONTROL, args(OP_SET_FAN_BOOST, fan, boost, 0)).is_some()
}

#[cfg(test)]
pub mod fake {
    //! A pretend x17 R1 controller: two fans (CPU, GPU), three sensors, four profiles.
    use super::*;

    pub struct X17 { pub profile: u8, pub boost: [u8; 2], pub calls: usize, pub broken_profile_list: bool }
    impl Default for X17 { fn default() -> Self { Self { profile: PROFILE_USTT_BALANCED, boost: [0; 2], calls: 0, broken_profile_list: false } } }

    pub const RESOURCES: [u8; 9] = [0x32, 0x33, 0x01, 0x06, 0x03, PROFILE_USTT_BALANCED, PROFILE_USTT_PERFORMANCE, PROFILE_USTT_QUIET, PROFILE_GMODE];

    impl Firmware for X17 {
        fn call(&mut self, method: u32, a: u32) -> Option<u32> {
            self.calls += 1;
            let [op, a1, a2, _] = a.to_le_bytes();
            Some(match (method, op) {
                (METHOD_THERMAL_INFORMATION, OP_SYSTEM_DESCRIPTION) => args(2, 3, 0, if self.broken_profile_list { 6 } else { 4 }),
                (METHOD_THERMAL_INFORMATION, OP_RESOURCE_ID) => RESOURCES.get(a1 as usize).map_or(FAILURE, |v| 0x100 | *v as u32),
                (METHOD_THERMAL_INFORMATION, OP_FAN_MIN_RPM) => 0,
                (METHOD_THERMAL_INFORMATION, OP_FAN_MAX_RPM) => if a1 == 0x32 { 5600 } else { 5900 },
                (METHOD_THERMAL_INFORMATION, OP_FAN_RPM) => match a1 { 0x32 => 2400 + self.boost[0] as u32 * 10, 0x33 => 2100, _ => FAILURE },
                (METHOD_THERMAL_INFORMATION, OP_TEMPERATURE) => match a1 { 0x01 => 61, 0x06 => 48, 0x03 => 0, _ => FAILURE },
                (METHOD_THERMAL_INFORMATION, OP_CURRENT_PROFILE) => self.profile as u32,
                (METHOD_THERMAL_CONTROL, OP_ACTIVATE_PROFILE) => { self.profile = a1; 0 }
                (METHOD_THERMAL_CONTROL, OP_SET_FAN_BOOST) => { self.boost[(a1 - 0x32) as usize] = a2; 0 }
                _ => FAILURE,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding() {
        assert_eq!(args(OP_FAN_RPM, 0x32, 0, 0), 0x0000_3205);
        assert_eq!(args(1, 2, 3, 4).to_le_bytes(), [1, 2, 3, 4]);
        let d = Description::from_word(0x0400_0302).unwrap();
        assert_eq!(d, Description { fans: 2, temps: 3, unknown: 0, profiles: 4 });
        assert_eq!(d.word(), 0x0400_0302);
        assert_eq!(Description::from_word(0x0000_00FF), None);
    }

    #[test]
    fn probe_and_read_x17() {
        let mut fw = fake::X17::default();
        let inv = probe(&mut fw).unwrap();
        assert_eq!(inv.fans().iter().map(|f| f.id).collect::<Vec<_>>(), [0x32, 0x33]);
        assert_eq!(inv.fans()[0].max_rpm, 5600);
        assert_eq!(inv.sensors(), &[0x01, 0x06, 0x03]);
        // G-Mode is listed but never offered as an ordinary profile.
        assert_eq!(inv.profiles(), &[PROFILE_USTT_BALANCED, PROFILE_USTT_PERFORMANCE, PROFILE_USTT_QUIET]);
        let r = read(&mut fw, &inv);
        assert_eq!(&r.rpm[..2], &[Some(2400), Some(2100)]);
        assert_eq!(&r.celsius[..3], &[Some(61), Some(48), None]);
        assert_eq!(r.hottest_decikelvin(), Some(3342));
        assert_eq!(r.profile, Some(PROFILE_USTT_BALANCED));
        assert_eq!(fan_name(0x32), "cpu");
        assert_eq!(sensor_name(0x06), "gpu");
    }

    #[test]
    fn short_profile_list_stops_early() {
        let mut fw = fake::X17 { broken_profile_list: true, ..Default::default() };
        let inv = probe(&mut fw).unwrap();
        assert_eq!(inv.profile_count, 3);
    }

    #[test]
    fn profiles_follow_the_desktop() {
        let mut fw = fake::X17::default();
        let inv = probe(&mut fw).unwrap();
        let perf = for_profile(Profile::Performance, inv.profiles()).unwrap();
        assert_eq!(perf, PROFILE_USTT_PERFORMANCE);
        assert!(activate(&mut fw, &inv, perf));
        assert_eq!(fw.profile, PROFILE_USTT_PERFORMANCE);
        assert_eq!(for_profile(Profile::Saver, inv.profiles()), Some(PROFILE_USTT_QUIET));
        assert_eq!(for_profile(Profile::Balanced, inv.profiles()), Some(PROFILE_USTT_BALANCED));
        // Legacy-only machines map too; G-Mode alone is never picked.
        assert_eq!(for_profile(Profile::Saver, &[PROFILE_LEGACY_BALANCED, PROFILE_LEGACY_QUIET]), Some(PROFILE_LEGACY_QUIET));
        assert_eq!(for_profile(Profile::Performance, &[PROFILE_GMODE]), None);
        // Unlisted profiles never reach the firmware.
        let before = fw.calls;
        assert!(!activate(&mut fw, &inv, PROFILE_USTT_COOL));
        assert_eq!(fw.calls, before);
    }

    #[test]
    fn manual_boost() {
        let mut fw = fake::X17::default();
        let inv = probe(&mut fw).unwrap();
        assert!(activate(&mut fw, &inv, PROFILE_CUSTOM));
        assert!(set_boost(&mut fw, &inv, 0x32, 100));
        assert_eq!(read(&mut fw, &inv).rpm[0], Some(3400));
        assert!(!set_boost(&mut fw, &inv, 0x40, 100));
    }

    #[test]
    fn rpm_refusal_vs_zero_and_d3cold_temp_filtering() {
        struct Mixed;
        impl Firmware for Mixed {
            fn call(&mut self, method: u32, a: u32) -> Option<u32> {
                let [op, a1, _, _] = a.to_le_bytes();
                Some(match (method, op) {
                    (METHOD_THERMAL_INFORMATION, OP_SYSTEM_DESCRIPTION) => args(2, 2, 0, 0),
                    (METHOD_THERMAL_INFORMATION, OP_RESOURCE_ID) => [0x32, 0x33, 0x01, 0x06].get(a1 as usize).map_or(FAILURE, |v| *v as u32),
                    (METHOD_THERMAL_INFORMATION, OP_FAN_RPM) => match a1 { 0x32 => 0, 0x33 => FAILURE, _ => FAILURE },
                    (METHOD_THERMAL_INFORMATION, OP_TEMPERATURE) => match a1 { 0x01 => 45, 0x06 => 2, _ => FAILURE },
                    _ => FAILURE,
                })
            }
        }
        let inv = probe(&mut Mixed).unwrap();
        let r = read(&mut Mixed, &inv);
        // Stopped fan reports Some(0); firmware refusal reports None instead of masking as 0 RPM.
        assert_eq!(&r.rpm[..2], &[Some(0), None]);
        // Bogus 2 C reading from a D3cold dGPU is filtered out.
        assert_eq!(&r.celsius[..2], &[Some(45), None]);
        assert_eq!(plausible_celsius(2), None);
        assert_eq!(plausible_celsius(9), None);
        assert_eq!(plausible_celsius(10), Some(10));
        assert_eq!(plausible_celsius(125), Some(125));
        assert_eq!(plausible_celsius(126), None);
    }

    #[test]
    fn refusals() {
        struct Dead;
        impl Firmware for Dead { fn call(&mut self, _: u32, _: u32) -> Option<u32> { Some(FAILURE) } }
        assert!(probe(&mut Dead).is_none());
        assert_eq!(plausible_celsius(0), None);
        assert_eq!(plausible_celsius(255), None);
        assert_eq!(kind(0x42), None);
    }
}
