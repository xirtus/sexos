// SPDX-License-Identifier: GPL-3.0-only
//! Dell's SMBIOS calling interface over WMI: tokens, and the battery charge modes built on
//! them (Dell and Alienware laptops, the x17 R1 among them).
//!
//! Dell firmware settings are "tokens": SMBIOS type 0xDA lists each token's id, location
//! and default value. A call is a 36-byte buffer `{class: u16, select: u16, input: [u32; 4],
//! output: [u32; 4]}` passed to WMI method 1 of GUID `A80593CE-...`, padded to the size the
//! Dell descriptor block (`8D9DDCBC-...`) names. Class 0 reads a token (input[0] =
//! location, answer in output[1]); class 1 writes one (input[1] = value). output[0] is
//! 0 on success, -1 error, -2 unsupported.
//!
//! Charge modes are tokens too: standard, express, adaptive, primarily-AC and custom, with
//! custom start/stop percentages in tokens 0x0349/0x034A. The rules follow Linux's
//! `dell-laptop` (start 50..=95, stop 55..=100, at least 5 apart).

use crate::smbios::Structure;

pub const CLASS_TOKEN_READ: u16 = 0;
pub const CLASS_TOKEN_WRITE: u16 = 1;
pub const SELECT_TOKEN_STD: u16 = 0;

pub const TOKEN_PRIMARILY_AC: u16 = 0x0341;
pub const TOKEN_ADAPTIVE: u16 = 0x0342;
pub const TOKEN_CUSTOM: u16 = 0x0343;
pub const TOKEN_STANDARD: u16 = 0x0346;
pub const TOKEN_EXPRESS: u16 = 0x0347;
pub const TOKEN_CUSTOM_START: u16 = 0x0349;
pub const TOKEN_CUSTOM_END: u16 = 0x034A;

pub const START_MIN: u8 = 50;
pub const START_MAX: u8 = 95;
pub const END_MIN: u8 = 55;
pub const END_MAX: u8 = 100;
pub const MIN_GAP: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token { pub id: u16, pub location: u16, pub value: u16 }

/// Tokens from one type 0xDA structure (several may exist; callers concatenate).
pub fn tokens<'a>(s: &Structure<'a>) -> impl Iterator<Item = Token> + 'a {
    let body: &'a [u8] = if s.kind == 0xDA && s.data.len() >= 11 { &s.data[11..] } else { &[] };
    body.chunks_exact(6)
        .map(|c| Token { id: u16::from_le_bytes([c[0], c[1]]), location: u16::from_le_bytes([c[2], c[3]]), value: u16::from_le_bytes([c[4], c[5]]) })
        .take_while(|t| t.id != 0xFFFF)
        .filter(|t| t.id != 0)
}

/// Supported command classes (bit n = class n), from the first 0xDA structure.
pub fn supported_classes(s: &Structure) -> u32 {
    if s.kind == 0xDA && s.data.len() >= 11 { u32::from_le_bytes([s.data[7], s.data[8], s.data[9], s.data[10]]) } else { 0 }
}

/// The Dell descriptor block: interface version and the buffer size WMI calls take.
pub fn descriptor(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 16 || &b[..4] != b"DELL" || &b[4..8] != b" WMI" { return None; }
    let version = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
    let size = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
    (36..=32 * 1024).contains(&size).then_some((version, size))
}

/// One call, as bytes `size` long (the descriptor's size minus the 8-byte length Linux
/// keeps in front; firmware sees only `std` and the zeroed extension).
pub fn request(class: u16, select: u16, input: [u32; 4], size: usize, out: &mut [u8]) -> usize {
    let n = size.clamp(36, out.len());
    out[..n].fill(0);
    out[0..2].copy_from_slice(&class.to_le_bytes());
    out[2..4].copy_from_slice(&select.to_le_bytes());
    for (i, v) in input.iter().enumerate() { out[4 + 4 * i..8 + 4 * i].copy_from_slice(&v.to_le_bytes()); }
    n
}

/// output[0..4] from a returned buffer.
pub fn answer(b: &[u8]) -> Option<[u32; 4]> {
    if b.len() < 36 { return None; }
    let w = |i: usize| u32::from_le_bytes([b[20 + 4 * i], b[21 + 4 * i], b[22 + 4 * i], b[23 + 4 * i]]);
    Some([w(0), w(1), w(2), w(3)])
}

/// Where the calls go: `servers/sexacpi` runs WMI method 1 with the request bytes.
pub trait Firmware {
    fn call(&mut self, class: u16, select: u16, input: [u32; 4]) -> Option<[u32; 4]>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Standard, Express, Adaptive, PrimarilyAc, Custom }

pub const MODES: [(Mode, u16); 5] = [(Mode::PrimarilyAc, TOKEN_PRIMARILY_AC), (Mode::Express, TOKEN_EXPRESS),
    (Mode::Standard, TOKEN_STANDARD), (Mode::Adaptive, TOKEN_ADAPTIVE), (Mode::Custom, TOKEN_CUSTOM)];

impl Mode {
    pub fn name(self) -> &'static str {
        match self { Mode::Standard => "standard", Mode::Express => "express", Mode::Adaptive => "adaptive", Mode::PrimarilyAc => "primarily_ac", Mode::Custom => "custom" }
    }
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v { 0 => Mode::Standard, 1 => Mode::Express, 2 => Mode::Adaptive, 3 => Mode::PrimarilyAc, 4 => Mode::Custom, _ => return None })
    }
    pub fn token(self) -> u16 { MODES.iter().find(|(m, _)| *m == self).map(|(_, t)| *t).unwrap_or(TOKEN_STANDARD) }
}

fn find(list: &[Token], id: u16) -> Option<Token> { list.iter().copied().find(|t| t.id == id) }

fn read(fw: &mut impl Firmware, t: Token) -> Option<u32> {
    let out = fw.call(CLASS_TOKEN_READ, SELECT_TOKEN_STD, [t.location as u32, 0, 0, 0])?;
    (out[0] == 0).then_some(out[1])
}

fn write(fw: &mut impl Firmware, t: Token, value: u32) -> bool {
    fw.call(CLASS_TOKEN_WRITE, SELECT_TOKEN_STD, [t.location as u32, value, 0, 0]).map_or(false, |o| o[0] == 0)
}

/// The battery's charging setup as the firmware has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Charge { pub supported: [bool; 5], pub mode: Option<Mode>, pub start: Option<u8>, pub end: Option<u8> }

impl Charge {
    pub fn any(&self) -> bool { self.supported.iter().any(|s| *s) }
    pub fn supports(&self, m: Mode) -> bool { MODES.iter().position(|(x, _)| *x == m).map_or(false, |i| self.supported[i]) }
}

/// A mode is active when reading its token returns the token's own value.
pub fn charge(fw: &mut impl Firmware, list: &[Token]) -> Charge {
    let mut c = Charge::default();
    for (i, (m, id)) in MODES.iter().enumerate() {
        let Some(t) = find(list, *id) else { continue };
        c.supported[i] = true;
        if c.mode.is_none() && read(fw, t) == Some(t.value as u32) { c.mode = Some(*m); }
    }
    let pct = |fw: &mut _, id| find(list, id).and_then(|t| read(fw, t)).filter(|v| *v <= 100).map(|v| v as u8);
    c.start = pct(fw, TOKEN_CUSTOM_START);
    c.end = pct(fw, TOKEN_CUSTOM_END);
    c
}

/// Custom thresholds clamped to the firmware's rules: (start, end).
pub fn clamp_thresholds(start: u8, end: u8) -> (u8, u8) {
    let end = end.clamp(END_MIN, END_MAX);
    let start = start.clamp(START_MIN, START_MAX).min(end - MIN_GAP);
    (start, end)
}

/// Switches the charge mode; for `Custom`, sets the thresholds first.
pub fn set_mode(fw: &mut impl Firmware, list: &[Token], mode: Mode, thresholds: Option<(u8, u8)>) -> bool {
    let Some(t) = find(list, mode.token()) else { return false };
    if let (Mode::Custom, Some((s, e))) = (mode, thresholds) {
        let (s, e) = clamp_thresholds(s, e);
        let (Some(ts), Some(te)) = (find(list, TOKEN_CUSTOM_START), find(list, TOKEN_CUSTOM_END)) else { return false };
        // Raise the end first when moving up so start never passes it, and the reverse.
        let ok = if s >= read(fw, te).unwrap_or(100) as u8 { write(fw, te, e as u32) && write(fw, ts, s as u32) } else { write(fw, ts, s as u32) && write(fw, te, e as u32) };
        if !ok { return false; }
    }
    write(fw, t, t.value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smbios::{fixture, structures};

    /// Firmware that stores token values by location.
    struct Fake { values: std::collections::BTreeMap<u16, u32>, size_seen: usize }
    impl Firmware for Fake {
        fn call(&mut self, class: u16, select: u16, input: [u32; 4]) -> Option<[u32; 4]> {
            let mut buf = [0u8; 64];
            self.size_seen = request(class, select, input, 48, &mut buf);
            let loc = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as u16;
            let mut out = [0u32; 4];
            match class {
                CLASS_TOKEN_READ => out[1] = *self.values.get(&loc).unwrap_or(&0),
                CLASS_TOKEN_WRITE => {
                    let v = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
                    // Selecting a mode clears the other mode tokens.
                    if (0x0341..=0x0347).contains(&loc) { for l in 0x0341..=0x0347 { self.values.remove(&l); } }
                    self.values.insert(loc, v);
                }
                _ => out[0] = 0xFFFF_FFFE,
            }
            // Round-trip through the byte layout.
            buf[20..24].copy_from_slice(&out[0].to_le_bytes());
            buf[24..28].copy_from_slice(&out[1].to_le_bytes());
            answer(&buf)
        }
    }

    fn list() -> Vec<Token> {
        let t = fixture::table();
        structures(&t).filter(|s| s.kind == 0xDA).flat_map(|s| tokens(&s).collect::<Vec<_>>()).collect()
    }

    #[test]
    fn token_table() {
        let t = fixture::table();
        let da = structures(&t).find(|s| s.kind == 0xDA).unwrap();
        assert_eq!(supported_classes(&da) & 0b11, 0b11);
        let l = list();
        assert_eq!(l.len(), 6);
        assert_eq!(l[0], Token { id: 0x0341, location: 0x0341, value: 1 });
    }

    #[test]
    fn layout() {
        let mut b = [0xAAu8; 64];
        assert_eq!(request(1, 0, [0x0349, 80, 0, 0], 40, &mut b), 40);
        assert_eq!(&b[..12], &[1, 0, 0, 0, 0x49, 0x03, 0, 0, 80, 0, 0, 0]);
        assert!(b[12..40].iter().all(|v| *v == 0));
        assert_eq!(b[40], 0xAA);
        let mut d = [0u8; 16];
        d[..8].copy_from_slice(b"DELL WMI");
        d[8] = 1;
        d[12..16].copy_from_slice(&4096u32.to_le_bytes());
        assert_eq!(descriptor(&d), Some((1, 4096)));
        d[4] = b'X';
        assert_eq!(descriptor(&d), None);
    }

    #[test]
    fn charge_modes() {
        let l = list();
        let mut fw = Fake { values: [(TOKEN_STANDARD, 1u32), (TOKEN_CUSTOM_START, 50), (TOKEN_CUSTOM_END, 90)].into_iter().collect(), size_seen: 0 };
        let c = charge(&mut fw, &l);
        assert!(c.supports(Mode::Custom) && !c.supports(Mode::Express));
        assert_eq!(c.mode, Some(Mode::Standard));
        assert_eq!((c.start, c.end), (Some(50), Some(90)));
        assert!(set_mode(&mut fw, &l, Mode::Custom, Some((60, 80))));
        assert_eq!(fw.size_seen, 48);
        assert_eq!(fw.values.get(&TOKEN_CUSTOM_START), Some(&60));
        assert_eq!(fw.values.get(&TOKEN_CUSTOM_END), Some(&80));
        assert_eq!(fw.values.get(&TOKEN_CUSTOM), Some(&1));
        let c = charge(&mut fw, &l);
        assert_eq!(c.mode, Some(Mode::Custom));
        assert_eq!((c.start, c.end), (Some(60), Some(80)));
        assert!(!set_mode(&mut fw, &l, Mode::Express, None));
    }

    #[test]
    fn threshold_rules() {
        assert_eq!(clamp_thresholds(20, 40), (50, 55));
        assert_eq!(clamp_thresholds(99, 100), (95, 100));
        assert_eq!(clamp_thresholds(80, 82), (77, 82));
        assert_eq!(clamp_thresholds(60, 80), (60, 80));
    }
}
