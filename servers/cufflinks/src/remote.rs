// SPDX-License-Identifier: GPL-3.0-only
//! A headset's buttons and volume (AVRCP, `sex_bt::avrcp`), on the desktop's own paths.
//!
//! Buttons: Cufflinks gives the headset a device number in the HID mailbox and sends its
//! buttons as keyboard snapshots, the same usages a USB keyboard's media keys send:
//! play/pause, stop, previous, next as the reserved usages 0xE8..0xEB, volume up/down
//! and mute as 0x80, 0x81, 0x7F. SexUSB and Input treat them as any keyboard's, and the
//! Shell's media keys do the rest (the player keys go to the program Bell names, the
//! volume keys step Bell's master volume and show the OSD). No new Shell protocol.
//! A press with no release is let go after `RELEASE_TICKS` (AVRCP's target rule).
//!
//! Volume, when the headset does absolute volume (it took our registration for
//! EVENT_VOLUME_CHANGED): Bell's master volume is the one volume.
//! - System to headset: Bell writes its master volume into the `bt_audio` region;
//!   whenever it differs from what we last sent, the headset gets SetAbsoluteVolume
//!   (0..=100 scaled to 0..=127), and Bell leaves the master volume out of the mix it
//!   sends the headset, so it is applied once, by the headset.
//! - Headset to system: a CHANGED from the headset (its own buttons) becomes one volume
//!   key press toward it (up or down, one Shell step), so the master volume moves as a
//!   keyboard's volume key would move it, with its OSD. The new master then goes back to
//!   the headset, which snaps it to the system's step.
//! - No loop: a headset volume that maps back to the master volume (our own setting
//!   echoed) is within half a step and presses nothing; a volume equal to what the
//!   headset has is never sent (`sex_bt` drops it).
//! A headset without absolute volume keeps its own volume; its VOL UP/DOWN buttons, if
//! it sends them, are volume keys like any other.

/// Keyboard usages the headset's buttons are sent as.
pub const PLAY_PAUSE: u8 = 0xE8;
pub const STOP: u8 = 0xE9;
pub const PREVIOUS: u8 = 0xEA;
pub const NEXT: u8 = 0xEB;
pub const VOLUME_UP: u8 = 0x80;
pub const VOLUME_DOWN: u8 = 0x81;
pub const MUTE: u8 = 0x7F;

/// The Shell's volume key step (`desktop_keys::VOLUME_STEP`), in percent.
pub const STEP: u8 = 5;
/// A held button with no release is let go after this long (ms).
pub const RELEASE_TICKS: u64 = 2_000;

/// Master volume (0..=100) to AVRCP absolute volume (0..=127), and back, both rounded
/// so that `to_master(to_headset(m)) == m` for every m.
pub fn to_headset(master: u8) -> u8 { ((master.min(100) as u32 * 127 + 50) / 100) as u8 }
pub fn to_master(volume: u8) -> u8 { ((volume.min(127) as u32 * 100 + 63) / 127) as u8 }

/// What to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    /// Send this keyboard snapshot (usages, low byte first) for the headset's device.
    Keys(u64),
    /// Press and let go of a volume key.
    Tap(u8),
    /// Send the headset this absolute volume.
    SetVolume(u8),
}

/// The one headset whose buttons and volume we follow (the first that is ready).
pub struct Remote {
    pub handle: Option<u16>,
    held: Option<(u8, u64)>,
    /// The headset does absolute volume.
    pub absolute: bool,
    /// The master volume last sent to it.
    sent: Option<u8>,
}

impl Remote {
    pub const fn new() -> Remote { Remote { handle: None, held: None, absolute: false, sent: None } }

    /// AVRCP is up on `handle`: true when it is the one followed now.
    pub fn ready(&mut self, handle: u16) -> bool {
        if self.handle.is_some_and(|h| h != handle) { return false; }
        *self = Remote::new();
        self.handle = Some(handle);
        true
    }
    fn ours(&self, handle: u16) -> bool { self.handle == Some(handle) }

    /// A button went down or up.
    pub fn key(&mut self, handle: u16, usage: u8, pressed: bool, now: u64) -> Option<Do> {
        if !self.ours(handle) { return None; }
        if pressed {
            self.held = Some((usage, now));
            return Some(Do::Keys(usage as u64));
        }
        // A release of a key that is not held (a lost press) lets everything go.
        self.held.take().map(|_| Do::Keys(0))
    }

    /// The headset's absolute volume.
    pub fn volume(&mut self, handle: u16, volume: u8, changed: bool, master: Option<u8>) -> Option<Do> {
        if !self.ours(handle) { return None; }
        self.absolute = true;
        let (true, Some(m)) = (changed, master) else { return None };
        let want = to_master(volume);
        if want > m.saturating_add(STEP / 2) { return Some(Do::Tap(VOLUME_UP)); }
        if want.saturating_add(STEP / 2) < m { return Some(Do::Tap(VOLUME_DOWN)); }
        // Less than half a step: the headset goes back to the system's volume.
        (to_headset(m) != volume).then_some(Do::SetVolume(to_headset(m)))
    }
    pub fn no_volume(&mut self, handle: u16) { if self.ours(handle) { self.absolute = false; } }

    /// Every pass: a held button's timeout, and the master volume to the headset.
    pub fn tick(&mut self, now: u64, master: Option<u8>) -> Option<Do> {
        if self.held.is_some_and(|(_, since)| now.saturating_sub(since) >= RELEASE_TICKS) {
            self.held = None;
            return Some(Do::Keys(0));
        }
        let m = master.filter(|_| self.absolute && self.handle.is_some())?;
        if self.sent == Some(m) { return None; }
        self.sent = Some(m);
        Some(Do::SetVolume(to_headset(m)))
    }

    /// AVRCP or the link went: true when it was ours (release what it held).
    pub fn gone(&mut self, handle: u16) -> bool {
        if !self.ours(handle) { return false; }
        *self = Remote::new();
        true
    }
}
