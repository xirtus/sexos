// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! sex-config: the typed settings every part of SexOS shares.
//!
//! One schema, [`KEYS`], names every setting: its stable number, its TOML name, its
//! kind, its range and its default. A [`Config`] holds one value per key, always
//! valid: a value outside its key's range is refused, never clamped behind the
//! writer's back. Files keeps the one live `Config` and its text form on disk
//! (`/system/settings.toml`); Settings changes it, the Shell and other servers ask
//! Files for changes with a generation number (see `sex_pdx::config`).
//!
//! Key numbers are append-only: a number is never reused or renumbered, and a
//! choice list only grows at its end, so a number or index written by an older
//! system still means the same thing. The file stores names, not numbers.
//!
//! The text format is a small TOML subset: `[section]` headers, `name = value`
//! lines, `#` comments, and values that are `true`/`false`, integers or
//! double-quoted strings. Unknown names and bad values are skipped and counted, so
//! a file written by a newer system or edited by hand never stops the machine.
#![cfg_attr(not(test), no_std)]

pub mod wire;
pub mod secret;

/// What values a key takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// An integer in `min..=max`.
    Int { min: i32, max: i32 },
    /// An index into a list of names; the file holds the name.
    Choice(&'static [&'static str]),
    /// `0xRRGGBB`; the file holds `"#rrggbb"`.
    Color,
}

/// One setting.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// Stable number, used on the wire. Never reused.
    pub id: u16,
    /// `section.name`, used in the file.
    pub name: &'static str,
    /// Words for people (Settings shows them).
    pub label: &'static str,
    pub kind: Kind,
    pub default: i32,
}

impl Key {
    pub fn section(&self) -> &'static str { self.name.rsplit_once('.').map_or("", |(s, _)| s) }
    pub fn short(&self) -> &'static str { self.name.rsplit_once('.').map_or(self.name, |(_, n)| n) }
    /// True when `value` is one this key can hold.
    pub fn accepts(&self, value: i32) -> bool {
        match self.kind {
            Kind::Bool => value == 0 || value == 1,
            Kind::Int { min, max } => (min..=max).contains(&value),
            Kind::Choice(names) => value >= 0 && (value as usize) < names.len(),
            Kind::Color => (0..=0xFF_FFFF).contains(&value),
        }
    }
}

/// Wallpapers, in `sex_pdx::desktop_look::WALLPAPERS` order.
pub const WALLPAPERS: &[&str] = &["theme", "dusk", "forest", "ocean", "slate"];
/// Keyboard layouts. Workstream A appends its layouts here.
pub const LAYOUTS: &[&str] = &["us"];
/// Time zones by IANA name. Append-only; `sex-time` holds their rules.
pub const ZONES: &[&str] = &[
    "UTC",
    "Europe/London", "Europe/Dublin", "Europe/Lisbon", "Europe/Paris", "Europe/Berlin",
    "Europe/Madrid", "Europe/Rome", "Europe/Amsterdam", "Europe/Stockholm", "Europe/Oslo",
    "Europe/Copenhagen", "Europe/Helsinki", "Europe/Warsaw", "Europe/Prague", "Europe/Vienna",
    "Europe/Zurich", "Europe/Athens", "Europe/Istanbul", "Europe/Kyiv", "Europe/Moscow",
    "America/New_York", "America/Chicago", "America/Denver", "America/Phoenix",
    "America/Los_Angeles", "America/Anchorage", "Pacific/Honolulu", "America/Toronto",
    "America/Vancouver", "America/Mexico_City", "America/Sao_Paulo", "America/Buenos_Aires",
    "America/Bogota", "America/Santiago",
    "Asia/Tokyo", "Asia/Seoul", "Asia/Shanghai", "Asia/Hong_Kong", "Asia/Taipei",
    "Asia/Singapore", "Asia/Kolkata", "Asia/Dubai", "Asia/Bangkok", "Asia/Jakarta",
    "Asia/Manila", "Asia/Karachi", "Asia/Tehran", "Asia/Jerusalem",
    "Africa/Cairo", "Africa/Johannesburg", "Africa/Lagos", "Africa/Nairobi",
    "Australia/Sydney", "Australia/Melbourne", "Australia/Brisbane", "Australia/Perth",
    "Australia/Adelaide", "Pacific/Auckland",
];

/// Stable key numbers.
pub mod id {
    pub const ACCENT: u16 = 1;
    pub const WALLPAPER: u16 = 2;
    pub const NIGHT_LIGHT: u16 = 3;
    pub const ANIMATIONS: u16 = 4;
    pub const SCALE: u16 = 5;
    pub const LAYOUT: u16 = 6;
    pub const REPEAT_DELAY: u16 = 7;
    pub const REPEAT_RATE: u16 = 8;
    pub const CAPS_AS_CTRL: u16 = 9;
    pub const VOLUME: u16 = 10;
    pub const ZONE: u16 = 11;
    pub const NTP: u16 = 12;
    pub const HOUR24: u16 = 13;
    pub const DIM_AFTER: u16 = 14;
    pub const LOCK_AFTER: u16 = 15;
    pub const BLANK_AFTER: u16 = 16;
    pub const LOCK_ON_IDLE: u16 = 17;
    pub const SETUP_DONE: u16 = 18;
}

const fn int(min: i32, max: i32) -> Kind { Kind::Int { min, max } }

/// Every setting, in the order Settings and the file show them. Keys of one
/// section stay together. Idle times default to never: first-start setup offers
/// times, so a machine nobody set up (or a test run) never dims on its own.
pub const KEYS: &[Key] = &[
    Key { id: id::ACCENT, name: "appearance.accent", label: "Accent colour", kind: Kind::Color, default: 0x3D7EFF },
    Key { id: id::ANIMATIONS, name: "appearance.animations", label: "Animations", kind: Kind::Bool, default: 1 },
    Key { id: id::WALLPAPER, name: "desktop.wallpaper", label: "Wallpaper", kind: Kind::Choice(WALLPAPERS), default: 0 },
    Key { id: id::NIGHT_LIGHT, name: "desktop.night_light", label: "Night light (0 off to 3 warmest)", kind: int(0, 3), default: 0 },
    Key { id: id::SCALE, name: "display.scale", label: "Scale (0 automatic)", kind: int(0, 3), default: 0 },
    Key { id: id::LAYOUT, name: "keyboard.layout", label: "Keyboard layout", kind: Kind::Choice(LAYOUTS), default: 0 },
    Key { id: id::REPEAT_DELAY, name: "keyboard.repeat_delay_ms", label: "Repeat delay (ms)", kind: int(150, 1000), default: 400 },
    Key { id: id::REPEAT_RATE, name: "keyboard.repeat_rate", label: "Repeat rate (per second)", kind: int(2, 60), default: 25 },
    Key { id: id::CAPS_AS_CTRL, name: "keyboard.caps_as_ctrl", label: "Caps Lock is Ctrl", kind: Kind::Bool, default: 0 },
    Key { id: id::VOLUME, name: "sound.volume", label: "Volume (%)", kind: int(0, 100), default: 70 },
    Key { id: id::ZONE, name: "time.zone", label: "Time zone", kind: Kind::Choice(ZONES), default: 0 },
    Key { id: id::NTP, name: "time.network_time", label: "Set time from the network", kind: Kind::Bool, default: 1 },
    Key { id: id::HOUR24, name: "time.hour24", label: "24-hour clock", kind: Kind::Bool, default: 1 },
    Key { id: id::DIM_AFTER, name: "session.dim_after_min", label: "Dim after (minutes, 0 never)", kind: int(0, 120), default: 0 },
    Key { id: id::LOCK_AFTER, name: "session.lock_after_min", label: "Lock after (minutes, 0 never)", kind: int(0, 240), default: 0 },
    Key { id: id::BLANK_AFTER, name: "session.blank_after_min", label: "Screen off after (minutes, 0 never)", kind: int(0, 240), default: 0 },
    Key { id: id::LOCK_ON_IDLE, name: "session.lock_on_idle", label: "Lock when idle", kind: Kind::Bool, default: 1 },
    Key { id: id::SETUP_DONE, name: "setup.done", label: "First-start setup finished", kind: Kind::Bool, default: 0 },
];
pub const COUNT: usize = KEYS.len();

/// The key with number `id`.
pub fn key(id: u16) -> Option<&'static Key> { KEYS.iter().find(|k| k.id == id) }
fn slot(id: u16) -> Option<usize> { KEYS.iter().position(|k| k.id == id) }
/// The key named `section.name`.
pub fn named(name: &str) -> Option<&'static Key> { KEYS.iter().find(|k| k.name == name) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetError { UnknownKey, OutOfRange }

/// One value per key, plus a generation that grows with every change.
#[derive(Clone, Debug)]
pub struct Config {
    values: [i32; COUNT],
    /// The generation at which each key last changed (0: never since start).
    changed: [u64; COUNT],
    generation: u64,
}

impl Default for Config { fn default() -> Self { Self::new() } }

/// What reading a file found besides the values it took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report { pub taken: u16, pub unknown: u16, pub invalid: u16 }

impl Config {
    pub const fn new() -> Self {
        let mut values = [0; COUNT];
        let mut i = 0;
        while i < COUNT { values[i] = KEYS[i].default; i += 1; }
        Config { values, changed: [0; COUNT], generation: 0 }
    }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn get(&self, id: u16) -> Option<i32> { slot(id).map(|i| self.values[i]) }
    /// The value, or the key's default for an unknown number (never panics).
    pub fn value(&self, id: u16) -> i32 { self.get(id).or(key(id).map(|k| k.default)).unwrap_or(0) }
    pub fn flag(&self, id: u16) -> bool { self.value(id) != 0 }
    /// Sets one value. `Ok(true)` when it changed (and the generation grew).
    pub fn set(&mut self, id: u16, value: i32) -> Result<bool, SetError> {
        let i = slot(id).ok_or(SetError::UnknownKey)?;
        if !KEYS[i].accepts(value) { return Err(SetError::OutOfRange); }
        if self.values[i] == value { return Ok(false); }
        self.values[i] = value;
        self.generation += 1;
        self.changed[i] = self.generation;
        Ok(true)
    }
    /// The first key, in schema order after `after` (an id, 0 for the start), that
    /// changed after generation `since`. Watchers walk the changes with this.
    pub fn changed_since(&self, since: u64, after: u16) -> Option<(u16, i32)> {
        let start = if after == 0 { 0 } else { slot(after)? + 1 };
        (start..COUNT).find(|&i| self.changed[i] > since).map(|i| (KEYS[i].id, self.values[i]))
    }

    /// Reads settings text over the current values. Lines it cannot use are counted
    /// and skipped. Each value taken counts as a change.
    pub fn read(&mut self, text: &[u8]) -> Report {
        let mut report = Report::default();
        let mut section: &[u8] = b"";
        for raw in text.split(|b| *b == b'\n') {
            let line = trim(strip_comment(raw));
            if line.is_empty() { continue; }
            if line[0] == b'[' {
                section = if line.last() == Some(&b']') { trim(&line[1..line.len() - 1]) } else { b"" };
                continue;
            }
            let Some(eq) = line.iter().position(|b| *b == b'=') else { report.invalid += 1; continue };
            let (name, value) = (trim(&line[..eq]), trim(&line[eq + 1..]));
            let Some(k) = KEYS.iter().find(|k| k.section().as_bytes() == section && k.short().as_bytes() == name) else {
                report.unknown += 1;
                continue;
            };
            match parse_value(k, value).map(|v| self.set(k.id, v)) {
                Some(Ok(_)) => report.taken += 1,
                _ => report.invalid += 1,
            }
        }
        report
    }

    /// Writes the settings as text. Returns the bytes written, or None when `out` is
    /// too small (nothing useful is left in it then).
    pub fn write(&self, out: &mut [u8]) -> Option<usize> {
        let mut w = Writer { out, at: 0 };
        w.put(b"# SexOS settings. The Settings program writes this file.\n")?;
        w.put(b"# Unknown names and values out of range are skipped when it is read.\n")?;
        let mut section = "";
        for (i, k) in KEYS.iter().enumerate() {
            if k.section() != section {
                section = k.section();
                w.put(b"\n[")?; w.put(section.as_bytes())?; w.put(b"]\n")?;
            }
            w.put(k.short().as_bytes())?;
            w.put(b" = ")?;
            let v = self.values[i];
            match k.kind {
                Kind::Bool => w.put(if v != 0 { b"true" } else { b"false" })?,
                Kind::Int { .. } => w.int(v)?,
                Kind::Choice(names) => { w.put(b"\"")?; w.put(names[v as usize].as_bytes())?; w.put(b"\"")?; }
                Kind::Color => {
                    w.put(b"\"#")?;
                    for shift in [20, 16, 12, 8, 4, 0] { w.put(&[b"0123456789abcdef"[(v >> shift & 0xF) as usize]])?; }
                    w.put(b"\"")?;
                }
            }
            w.put(b"\n")?;
        }
        Some(w.at)
    }
}

/// The value text means for key `k`, if it is one the key can hold.
pub fn parse_value(k: &Key, text: &[u8]) -> Option<i32> {
    let v = match k.kind {
        Kind::Bool => match text { b"true" => 1, b"false" => 0, _ => return None },
        Kind::Int { .. } => parse_int(text)?,
        Kind::Choice(names) => {
            let s = unquote(text)?;
            names.iter().position(|n| n.as_bytes().eq_ignore_ascii_case(s))? as i32
        }
        Kind::Color => {
            let s = unquote(text)?;
            let hex = s.strip_prefix(b"#")?;
            if hex.len() != 6 { return None; }
            let mut v = 0i32;
            for b in hex { v = v << 4 | (*b as char).to_digit(16)? as i32; }
            v
        }
    };
    k.accepts(v).then_some(v)
}

fn parse_int(text: &[u8]) -> Option<i32> {
    let (neg, digits) = match text.first()? { b'-' => (true, &text[1..]), b'+' => (false, &text[1..]), _ => (false, text) };
    if digits.is_empty() || digits[0] == b'_' { return None; }
    let mut v: i64 = 0;
    for b in digits {
        match b {
            b'0'..=b'9' => v = v * 10 + (b - b'0') as i64,
            b'_' => continue,
            _ => return None,
        }
        if v > i32::MAX as i64 + 1 { return None; }
    }
    i32::try_from(if neg { -v } else { v }).ok()
}
fn unquote(text: &[u8]) -> Option<&[u8]> {
    let inner = text.strip_prefix(b"\"")?.strip_suffix(b"\"")?;
    (!inner.contains(&b'"') && !inner.contains(&b'\\')).then_some(inner)
}
/// Drops a `#` comment, unless the `#` is inside a string.
fn strip_comment(line: &[u8]) -> &[u8] {
    let mut quoted = false;
    for (i, b) in line.iter().enumerate() {
        match b { b'"' => quoted = !quoted, b'#' if !quoted => return &line[..i], _ => {} }
    }
    line
}
fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t' | b'\r', rest @ ..] = s { s = rest; }
    while let [rest @ .., b' ' | b'\t' | b'\r'] = s { s = rest; }
    s
}

struct Writer<'a> { out: &'a mut [u8], at: usize }
impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.at.checked_add(bytes.len()).filter(|e| *e <= self.out.len())?;
        self.out[self.at..end].copy_from_slice(bytes);
        self.at = end;
        Some(())
    }
    fn int(&mut self, v: i32) -> Option<()> {
        let mut digits = [0u8; 11];
        let mut n = (v as i64).unsigned_abs();
        let mut i = digits.len();
        loop { i -= 1; digits[i] = b'0' + (n % 10) as u8; n /= 10; if n == 0 { break; } }
        if v < 0 { i -= 1; digits[i] = b'-'; }
        self.put(&digits[i..])
    }
}

/// The largest text `Config::write` makes, with room to spare.
pub const TEXT_MAX: usize = 4096;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_sound() {
        let mut ids: Vec<u16> = KEYS.iter().map(|k| k.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), COUNT, "key numbers are unique");
        for k in KEYS {
            assert!(k.id != 0, "0 means 'from the start' on the wire");
            assert!(k.accepts(k.default), "{} default in range", k.name);
            assert!(k.name.contains('.'), "{} has a section", k.name);
        }
        // Keys of one section stay together, so the file has each section once.
        let mut seen: Vec<&str> = Vec::new();
        for k in KEYS {
            if seen.last() != Some(&k.section()) {
                assert!(!seen.contains(&k.section()), "section {} split", k.section());
                seen.push(k.section());
            }
        }
        assert_eq!(WALLPAPERS.len(), 5);
    }

    #[test]
    fn defaults_round_trip() {
        let c = Config::new();
        let mut buf = [0u8; TEXT_MAX];
        let n = c.write(&mut buf).unwrap();
        let mut d = Config::new();
        let r = d.read(&buf[..n]);
        assert_eq!(r, Report { taken: COUNT as u16, unknown: 0, invalid: 0 });
        for k in KEYS { assert_eq!(d.get(k.id), c.get(k.id)); }
    }

    #[test]
    fn changed_values_round_trip() {
        let mut c = Config::new();
        c.set(id::ACCENT, 0xA0B1C2).unwrap();
        c.set(id::WALLPAPER, 3).unwrap();
        c.set(id::ZONE, ZONES.iter().position(|z| *z == "Europe/Berlin").unwrap() as i32).unwrap();
        c.set(id::REPEAT_DELAY, 250).unwrap();
        c.set(id::HOUR24, 0).unwrap();
        let mut buf = [0u8; TEXT_MAX];
        let n = c.write(&mut buf).unwrap();
        let text = core::str::from_utf8(&buf[..n]).unwrap();
        assert!(text.contains("accent = \"#a0b1c2\""), "{text}");
        assert!(text.contains("wallpaper = \"ocean\""));
        assert!(text.contains("zone = \"Europe/Berlin\""));
        assert!(text.contains("repeat_delay_ms = 250"));
        assert!(text.contains("hour24 = false"));
        let mut d = Config::new();
        d.read(text.as_bytes());
        for k in KEYS { assert_eq!(d.get(k.id), c.get(k.id), "{}", k.name); }
    }

    #[test]
    fn set_refuses_bad_values_and_counts_changes() {
        let mut c = Config::new();
        assert_eq!(c.set(id::NIGHT_LIGHT, 4), Err(SetError::OutOfRange));
        assert_eq!(c.set(id::WALLPAPER, -1), Err(SetError::OutOfRange));
        assert_eq!(c.set(999, 1), Err(SetError::UnknownKey));
        assert_eq!(c.generation(), 0);
        assert_eq!(c.set(id::VOLUME, 70), Ok(false), "same value is no change");
        assert_eq!(c.set(id::VOLUME, 30), Ok(true));
        assert_eq!(c.generation(), 1);
        assert_eq!(c.value(id::VOLUME), 30);
    }

    #[test]
    fn watchers_see_each_change_once() {
        let mut c = Config::new();
        let seen = c.generation();
        c.set(id::WALLPAPER, 1).unwrap();
        c.set(id::SCALE, 2).unwrap();
        c.set(id::WALLPAPER, 2).unwrap();
        let mut got = Vec::new();
        let mut after = 0;
        while let Some((k, v)) = c.changed_since(seen, after) { got.push((k, v)); after = k; }
        // Schema order, latest value, each key once.
        assert_eq!(got, vec![(id::WALLPAPER, 2), (id::SCALE, 2)]);
        let now = c.generation();
        assert_eq!(c.changed_since(now, 0), None);
        c.set(id::NTP, 0).unwrap();
        assert_eq!(c.changed_since(now, 0), Some((id::NTP, 0)));
    }

    #[test]
    fn hand_edited_files_are_forgiving() {
        let text = b"# mine\n[desktop]\nwallpaper = \"Forest\" # comment\nnight_light=9\nmystery = 1\n\n[keyboard]\n  repeat_rate = 3_0\nrepeat_delay_ms = \"x\"\n[future]\nthing = true\nnot a line\n[time]\nzone = \"Asia/Tokyo\"\nhour24 = false\n";
        let mut c = Config::new();
        let r = c.read(text);
        assert_eq!(r, Report { taken: 4, unknown: 2, invalid: 3 });
        assert_eq!(c.value(id::WALLPAPER), 2);
        assert_eq!(c.value(id::NIGHT_LIGHT), 0, "out of range keeps default");
        assert_eq!(c.value(id::REPEAT_RATE), 30);
        assert_eq!(ZONES[c.value(id::ZONE) as usize], "Asia/Tokyo");
        assert!(!c.flag(id::HOUR24));
    }

    #[test]
    fn small_buffer_fails_cleanly() {
        let c = Config::new();
        let mut small = [0u8; 64];
        assert_eq!(c.write(&mut small), None);
    }

    #[test]
    fn ints_parse_strictly() {
        assert_eq!(parse_int(b"-5"), Some(-5));
        assert_eq!(parse_int(b"2147483647"), Some(i32::MAX));
        assert_eq!(parse_int(b"-2147483648"), Some(i32::MIN));
        assert_eq!(parse_int(b"2147483648"), None);
        assert_eq!(parse_int(b"99999999999999999999"), None);
        assert_eq!(parse_int(b""), None);
        assert_eq!(parse_int(b"1x"), None);
    }
}
