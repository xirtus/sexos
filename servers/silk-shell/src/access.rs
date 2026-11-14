// SPDX-License-Identifier: GPL-3.0-only
//! Accessibility in the Shell (slice S8): magnifier and colour filter keys,
//! and the keyboard aids sticky keys, slow keys and bounce keys.
//!
//! Every key event passes through [`Access::key`] before anything else in the
//! Shell sees it. The aids can hold an event back (slow keys), drop it
//! (bounce keys) or add modifier presses around it (sticky keys); what comes
//! out is a short list of events the Shell then handles as if typed.
//!
//! Keys (GNOME's and Windows' conventions where they exist):
//! * Super+Alt+8 magnifier on/off, Super+Alt+= / Super+Alt+- zoom in/out
//! * Super+Alt+C next colour filter, Super+Alt+I inverted colours on/off
//! * Super+Alt+S sticky keys, Super+Alt+K slow keys, Super+Alt+B bounce keys
//! * Shift pressed five times in a row: sticky keys on/off
//!
//! Display draws the magnifier and filters (`sex_pdx::access_view`); this
//! module only keeps the choice. It builds alone for host tests.

/// PS/2 set 1 codes, as the Shell receives them.
pub mod code {
    pub const SHIFT_LEFT: u8 = 0x2A;
    pub const SHIFT_RIGHT: u8 = 0x36;
    pub const CTRL_LEFT: u8 = 0x1D;
    pub const CTRL_RIGHT: u8 = 0x61;
    pub const ALT_LEFT: u8 = 0x38;
    pub const ALT_GR: u8 = 0x64;
    pub const SUPER_LEFT: u8 = 0x5B;
    pub const SUPER_RIGHT: u8 = 0x5C;
    pub const DIGIT_8: u8 = 0x09;
    pub const MINUS: u8 = 0x0C;
    pub const EQUAL: u8 = 0x0D;
    pub const KEYPAD_MINUS: u8 = 0x4A;
    pub const KEYPAD_PLUS: u8 = 0x4E;
    pub const C: u8 = 0x2E;
    pub const I: u8 = 0x17;
    pub const S: u8 = 0x1F;
    pub const K: u8 = 0x25;
    pub const B: u8 = 0x30;
}

/// Zoom steps and filters mirror `sex_pdx::access_view` (kept here so this
/// file builds alone; the access_view host test checks they agree).
pub const ZOOM_ONE: u8 = 4;
pub const ZOOM_MAX: u8 = 32;
pub const FILTER_COUNT: u8 = 6;
pub const FILTER_INVERTED: u8 = 2;
pub const FILTER_NAMES: [&str; FILTER_COUNT as usize] =
    ["Off", "High contrast", "Inverted", "Grayscale", "Red-green aid", "Blue-yellow aid"];

pub fn zoom_step(zoom: u8, up: bool) -> u8 {
    let step = if zoom < 8 || (zoom == 8 && !up) { 1 } else if zoom < 16 || (zoom == 16 && !up) { 2 } else { 4 };
    if up { zoom.saturating_add(step).min(ZOOM_MAX) } else { zoom.saturating_sub(step).max(ZOOM_ONE) }
}

/// Slow keys: a press counts once the key has been held this long.
pub const SLOW_MS: u64 = 300;
/// Bounce keys: a second press of the same key this soon after its release is ignored.
pub const BOUNCE_MS: u64 = 300;
/// Shift presses in a row that toggle sticky keys.
pub const SHIFT_TAPS: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The magnifier or filter changed: send (zoom, filter) to Display.
    View { zoom: u8, filter: u8 },
    StickyKeys(bool),
    SlowKeys(bool),
    BounceKeys(bool),
}

/// What [`Access::key`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Handle the event as usual.
    Pass,
    /// Handle the events in `Events` instead (possibly none).
    Replace,
    /// An accessibility chord: apply the change. The key event itself should
    /// still reach the window manager's chord tracker, so Super's own tap
    /// (the launcher) knows a chord used it.
    Chord(Change),
}

/// A few key events to handle in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Events { list: [(u8, u64); 8], len: usize }
impl Events {
    pub const fn new() -> Self { Self { list: [(0, 0); 8], len: 0 } }
    fn push(&mut self, code: u8, value: u64) { if self.len < self.list.len() { self.list[self.len] = (code, value); self.len += 1; } }
    pub fn as_slice(&self) -> &[(u8, u64)] { &self.list[..self.len] }
}

/// The modifiers sticky keys latches. Super stays out: its tap opens the launcher.
const STICKY: [u8; 6] = [code::SHIFT_LEFT, code::SHIFT_RIGHT, code::CTRL_LEFT, code::CTRL_RIGHT, code::ALT_LEFT, code::ALT_GR];
fn sticky_index(c: u8) -> Option<usize> { STICKY.iter().position(|&k| k == c) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Latch { Off, Once, Locked }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub zoom: u8,
    /// The zoom Super+Alt+8 returns to.
    last_zoom: u8,
    pub filter: u8,
    /// The filter Super+Alt+I returns to.
    last_filter: u8,
    pub sticky: bool,
    pub slow: bool,
    pub bounce: bool,
    super_down: u8,
    alt_down: u8,
    /// Sticky keys: each latchable modifier's state, and whether the
    /// physical key is down now, and whether another key came while it was.
    latch: [Latch; 6],
    down: [bool; 6],
    chorded: [bool; 6],
    /// Keys the latched modifiers were applied to, released with them.
    applied: Option<(u8, [bool; 6])>,
    shift_taps: u8,
    /// Slow keys: the key being held and when it went down.
    pending: Option<(u8, u64)>,
    /// A slow key that was accepted and still has to be released.
    accepted: Option<u8>,
    /// Bounce keys: the last released key and when.
    released: Option<(u8, u64)>,
    /// Bounce keys: a press that was ignored, whose release is ignored too.
    bounced: Option<u8>,
}

impl Access {
    pub const fn new() -> Self {
        Self {
            zoom: ZOOM_ONE, last_zoom: 8, filter: 0, last_filter: 1,
            sticky: false, slow: false, bounce: false,
            super_down: 0, alt_down: 0,
            latch: [Latch::Off; 6], down: [false; 6], chorded: [false; 6], applied: None, shift_taps: 0,
            pending: None, accepted: None, released: None, bounced: None,
        }
    }

    pub fn filter_name(&self) -> &'static str { FILTER_NAMES[(self.filter % FILTER_COUNT) as usize] }

    /// One key event at `now` ms. Events to handle instead go into `out`.
    pub fn key(&mut self, c: u8, value: u64, now: u64, out: &mut Events) -> Verdict {
        // Super and Alt are tracked from the raw keys for the chords.
        match c {
            code::SUPER_LEFT | code::SUPER_RIGHT => track(&mut self.super_down, c == code::SUPER_LEFT, value),
            code::ALT_LEFT | code::ALT_GR => track(&mut self.alt_down, c == code::ALT_LEFT, value),
            _ => {}
        }
        if value == 1 && self.super_down != 0 && self.alt_down != 0 {
            if let Some(change) = self.chord(c) { self.chorded = [true; 6]; return Verdict::Chord(change); }
        }
        // Five Shift taps in a row toggle sticky keys.
        if value == 1 {
            if c == code::SHIFT_LEFT || c == code::SHIFT_RIGHT {
                self.shift_taps += 1;
                if self.shift_taps >= SHIFT_TAPS {
                    self.shift_taps = 0;
                    return Verdict::Chord(self.set_sticky(!self.sticky));
                }
            } else {
                self.shift_taps = 0;
            }
        }
        if !self.sticky && !self.slow && !self.bounce { return Verdict::Pass; }
        if self.bounce && !self.bounce_ok(c, value, now) { return Verdict::Replace; }
        if self.slow {
            match value {
                1 | 2 => {
                    if self.accepted == Some(c) { if value == 2 { return self.sticky_or_pass(c, value, out); } return Verdict::Replace; }
                    if !matches!(self.pending, Some((p, _)) if p == c) { self.pending = Some((c, now)); }
                    return Verdict::Replace;
                }
                _ => {
                    if matches!(self.pending, Some((p, _)) if p == c) { self.pending = None; return Verdict::Replace; }
                    if self.accepted == Some(c) { self.accepted = None; }
                }
            }
        }
        self.sticky_or_pass(c, value, out)
    }

    /// Slow keys: the held key once its time has come. Call often.
    pub fn poll(&mut self, now: u64, out: &mut Events) -> bool {
        let Some((c, since)) = self.pending else { return false };
        if now.saturating_sub(since) < SLOW_MS { return false; }
        self.pending = None;
        self.accepted = Some(c);
        if self.sticky_or_pass(c, 1, out) == Verdict::Pass { out.push(c, 1); }
        true
    }

    fn bounce_ok(&mut self, c: u8, value: u64, now: u64) -> bool {
        match value {
            1 => {
                if let Some((r, at)) = self.released {
                    if r == c && now.saturating_sub(at) < BOUNCE_MS { self.bounced = Some(c); return false; }
                }
                true
            }
            0 => {
                if self.bounced == Some(c) { self.bounced = None; return false; }
                self.released = Some((c, now));
                true
            }
            _ => self.bounced != Some(c),
        }
    }

    fn sticky_or_pass(&mut self, c: u8, value: u64, out: &mut Events) -> Verdict {
        if !self.sticky { return Verdict::Pass; }
        if let Some(i) = sticky_index(c) {
            match value {
                1 => { if !self.down[i] { self.down[i] = true; self.chorded[i] = false; } }
                0 => {
                    // Only a tap sticky keys saw from its press latches (not
                    // the release of a key held through a chord).
                    let tapped = core::mem::replace(&mut self.down[i], false) && !self.chorded[i];
                    if tapped {
                        // A modifier tapped alone: latch, lock, then off.
                        self.latch[i] = match self.latch[i] { Latch::Off => Latch::Once, Latch::Once => Latch::Locked, Latch::Locked => Latch::Off };
                        // The modifier itself is held logically while latched.
                        if self.latch[i] != Latch::Off { return Verdict::Replace; }
                    }
                }
                _ => {}
            }
            return Verdict::Pass;
        }
        for i in 0..STICKY.len() { if self.down[i] { self.chorded[i] = true; } }
        match value {
            1 => {
                let mut held = [false; 6];
                for i in 0..STICKY.len() {
                    if self.latch[i] != Latch::Off && !self.down[i] { held[i] = true; out.push(STICKY[i], 1); }
                }
                if !held.iter().any(|&h| h) { return Verdict::Pass; }
                out.push(c, 1);
                self.applied = Some((c, held));
                // Once-latches are used up; locks stay.
                for i in 0..STICKY.len() { if self.latch[i] == Latch::Once { self.latch[i] = Latch::Off; } }
                Verdict::Replace
            }
            0 => match self.applied {
                Some((k, held)) if k == c => {
                    out.push(c, 0);
                    for i in 0..STICKY.len() { if held[i] { out.push(STICKY[i], 0); } }
                    self.applied = None;
                    Verdict::Replace
                }
                _ => Verdict::Pass,
            },
            _ => Verdict::Pass,
        }
    }

    fn set_sticky(&mut self, on: bool) -> Change {
        self.sticky = on;
        self.latch = [Latch::Off; 6];
        self.applied = None;
        Change::StickyKeys(on)
    }

    fn chord(&mut self, c: u8) -> Option<Change> {
        let view = |s: &Self| Change::View { zoom: s.zoom, filter: s.filter };
        Some(match c {
            code::DIGIT_8 => {
                if self.zoom == ZOOM_ONE { self.zoom = self.last_zoom.max(ZOOM_ONE + 1); } else { self.last_zoom = self.zoom; self.zoom = ZOOM_ONE; }
                view(self)
            }
            code::EQUAL | code::KEYPAD_PLUS => { self.zoom = zoom_step(self.zoom, true); self.last_zoom = self.zoom; view(self) }
            code::MINUS | code::KEYPAD_MINUS => { self.zoom = zoom_step(self.zoom, false); if self.zoom != ZOOM_ONE { self.last_zoom = self.zoom; } view(self) }
            code::C => { self.filter = (self.filter + 1) % FILTER_COUNT; if self.filter != 0 { self.last_filter = self.filter; } view(self) }
            code::I => { self.filter = if self.filter == FILTER_INVERTED { 0 } else { FILTER_INVERTED }; view(self) }
            code::S => self.set_sticky(!self.sticky),
            code::K => { self.slow = !self.slow; self.pending = None; self.accepted = None; Change::SlowKeys(self.slow) }
            code::B => { self.bounce = !self.bounce; self.released = None; self.bounced = None; Change::BounceKeys(self.bounce) }
            _ => return None,
        })
    }

    /// Panel text for a change.
    pub fn notice(&self, change: Change, buf: &mut [u8]) -> usize {
        let mut n = 0;
        let mut put = |s: &[u8]| { let t = s.len().min(buf.len() - n); buf[n..n + t].copy_from_slice(&s[..t]); n += t; };
        match change {
            Change::View { zoom, filter } => {
                put(b"Magnifier ");
                if zoom == ZOOM_ONE { put(b"off"); } else {
                    let mut d = [0u8; 8];
                    put(fmt_zoom(zoom, &mut d));
                }
                put(b"  Colours: ");
                put(FILTER_NAMES[(filter % FILTER_COUNT) as usize].as_bytes());
            }
            Change::StickyKeys(on) => { put(b"Sticky keys "); put(if on { b"on" } else { b"off" }); }
            Change::SlowKeys(on) => { put(b"Slow keys "); put(if on { b"on" } else { b"off" }); }
            Change::BounceKeys(on) => { put(b"Bounce keys "); put(if on { b"on" } else { b"off" }); }
        }
        n
    }
}

fn track(bits: &mut u8, left: bool, value: u64) {
    let bit = if left { 1 } else { 2 };
    match value { 1 => *bits |= bit, 0 => *bits &= !bit, _ => {} }
}

/// "2x", "2.5x", "1.25x".
fn fmt_zoom(zoom: u8, d: &mut [u8; 8]) -> &[u8] {
    let whole = zoom / 4;
    let mut n = 0;
    if whole >= 10 { d[n] = b'0' + whole / 10; n += 1; }
    d[n] = b'0' + whole % 10; n += 1;
    match zoom % 4 { 1 => { d[n..n + 3].copy_from_slice(b".25"); n += 3 } 2 => { d[n..n + 2].copy_from_slice(b".5"); n += 2 } 3 => { d[n..n + 3].copy_from_slice(b".75"); n += 3 } _ => {} }
    d[n] = b'x';
    &d[..n + 1]
}
