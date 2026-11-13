// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Atomic commits: a list of changes checked as a whole against the device.
//! [`check`] returns the complete next state, or why it cannot exist. Drivers
//! program hardware only from a checked state, so a refused commit leaves the
//! screen exactly as it was.

use alloc::vec::Vec;
use crate::mode::Mode;
use crate::model::{Device, PlaneKind, PlaneState, Rect, State, Status};

/// The commit may change modes (blanks the screen briefly). Without it only
/// plane changes (flips, cursor moves) pass.
pub const ALLOW_MODESET: u32 = 1 << 0;
/// Check only; never touch hardware.
pub const TEST_ONLY: u32 = 1 << 1;
/// Send a [`FlipEvent`] for each touched CRTC when the change reaches the screen.
pub const PAGE_FLIP_EVENT: u32 = 1 << 2;
/// Return before vblank; the event reports completion.
pub const NONBLOCK: u32 = 1 << 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Bind a connector to a CRTC, or unbind it.
    Route { connector: u32, crtc: Option<u32> },
    Active { crtc: u32, on: bool },
    Mode { crtc: u32, mode: Option<Mode> },
    /// Full plane setup: which CRTC, which framebuffer, which part of it, where.
    Plane { plane: u32, crtc: Option<u32>, fb: Option<u32>, src: Rect, dst: Rect },
    /// Page flip: same geometry, new framebuffer.
    Flip { plane: u32, fb: u32 },
    /// Move a plane (the cursor) without changing what it shows.
    Move { plane: u32, x: i32, y: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Commit<'a> { pub changes: &'a [Change], pub flags: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitError {
    NoSuchConnector(u32),
    NoSuchCrtc(u32),
    NoSuchPlane(u32),
    NoSuchFramebuffer(u32),
    /// The connector cannot be wired to that CRTC.
    CrtcNotPossible { connector: u32, crtc: u32 },
    PlaneNotPossible { plane: u32, crtc: u32 },
    ConnectorDisconnected(u32),
    /// Active CRTC with no mode, or no connector.
    CrtcIncomplete(u32),
    /// A bound connector does not accept the CRTC's mode.
    ModeRejected { connector: u32, crtc: u32 },
    /// Active CRTC without a primary plane showing something.
    NoPrimary(u32),
    PlaneOnInactiveCrtc(u32),
    FormatUnsupported(u32),
    /// Source rectangle empty or outside its framebuffer.
    BadSource(u32),
    /// Plane cannot scale, or exceeds its size limit.
    BadSize(u32),
    /// Plane lies entirely outside its CRTC's picture.
    Offscreen(u32),
    /// Flip or move on a disabled plane.
    PlaneDisabled(u32),
    /// The commit needs a modeset but did not allow it.
    ModesetNotAllowed,
}

/// The result of a successful check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub state: State,
    /// CRTCs whose timing, activity or routing changed: the driver must modeset them.
    pub modeset: u32,
    /// CRTCs with any change, modeset or not: these get flip events.
    pub touched: u32,
}

/// Reported at vblank after a flip reaches the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlipEvent { pub crtc: u32, pub sequence: u64, pub time_us: u64 }

pub fn check(dev: &Device, cur: &State, commit: Commit<'_>) -> Result<Checked, CommitError> {
    let mut next = cur.clone();
    let mut touched = 0u32;
    let crtc_ok = |c: u32| if c < dev.crtcs { Ok(()) } else { Err(CommitError::NoSuchCrtc(c)) };
    let plane = |p: u32| dev.planes.get(p as usize).ok_or(CommitError::NoSuchPlane(p));
    for ch in commit.changes {
        match *ch {
            Change::Route { connector, crtc } => {
                let con = dev.connectors.get(connector as usize).ok_or(CommitError::NoSuchConnector(connector))?;
                if let Some(c) = crtc {
                    crtc_ok(c)?;
                    if con.possible_crtcs & (1 << c) == 0 { return Err(CommitError::CrtcNotPossible { connector, crtc: c }); }
                }
                if let Some(old) = next.connectors[connector as usize] { touched |= 1 << old; }
                if let Some(c) = crtc { touched |= 1 << c; }
                next.connectors[connector as usize] = crtc;
            }
            Change::Active { crtc, on } => { crtc_ok(crtc)?; next.crtcs[crtc as usize].active = on; touched |= 1 << crtc; }
            Change::Mode { crtc, mode } => { crtc_ok(crtc)?; next.crtcs[crtc as usize].mode = mode; touched |= 1 << crtc; }
            Change::Plane { plane: p, crtc, fb, src, dst } => {
                let pl = plane(p)?;
                if let Some(c) = crtc {
                    crtc_ok(c)?;
                    if pl.possible_crtcs & (1 << c) == 0 { return Err(CommitError::PlaneNotPossible { plane: p, crtc: c }); }
                    touched |= 1 << c;
                }
                if let Some(old) = next.planes[p as usize].crtc { touched |= 1 << old; }
                next.planes[p as usize] = PlaneState { crtc, fb, src, dst };
            }
            Change::Flip { plane: p, fb } => {
                plane(p)?;
                let st = &mut next.planes[p as usize];
                let c = st.crtc.filter(|_| st.fb.is_some()).ok_or(CommitError::PlaneDisabled(p))?;
                st.fb = Some(fb);
                touched |= 1 << c;
            }
            Change::Move { plane: p, x, y } => {
                plane(p)?;
                let st = &mut next.planes[p as usize];
                let c = st.crtc.ok_or(CommitError::PlaneDisabled(p))?;
                st.dst.x = x; st.dst.y = y;
                touched |= 1 << c;
            }
        }
    }

    let mut modeset = 0u32;
    for c in 0..dev.crtcs {
        let (a, b) = (&cur.crtcs[c as usize], &next.crtcs[c as usize]);
        let routing = |s: &State| s.crtc_connectors(c).fold(0u64, |m, i| m | 1 << i);
        if a.active != b.active || (b.active && a.mode != b.mode) || routing(cur) != routing(&next) { modeset |= 1 << c; }
    }
    if modeset != 0 && commit.flags & ALLOW_MODESET == 0 { return Err(CommitError::ModesetNotAllowed); }

    for (i, bound) in next.connectors.iter().enumerate() {
        if bound.is_some() && dev.connectors[i].status == Status::Disconnected && modeset & (1 << bound.unwrap()) != 0 {
            return Err(CommitError::ConnectorDisconnected(i as u32));
        }
    }
    for c in 0..dev.crtcs {
        let st = next.crtcs[c as usize];
        if !st.active { continue; }
        let mode = st.mode.filter(|m| m.valid()).ok_or(CommitError::CrtcIncomplete(c))?;
        let mut any = false;
        for i in next.crtc_connectors(c) {
            any = true;
            if !dev.connectors[i].accepts(&mode) { return Err(CommitError::ModeRejected { connector: i as u32, crtc: c }); }
        }
        if !any { return Err(CommitError::CrtcIncomplete(c)); }
        let primary_on = next.planes.iter().enumerate()
            .any(|(i, p)| p.crtc == Some(c) && p.fb.is_some() && dev.planes[i].kind == PlaneKind::Primary);
        if !primary_on { return Err(CommitError::NoPrimary(c)); }
    }
    for (i, st) in next.planes.iter().enumerate() {
        let p = i as u32;
        let (Some(c), Some(fbid)) = (st.crtc, st.fb) else { continue };
        let crtc = next.crtcs[c as usize];
        let Some(mode) = crtc.mode.filter(|_| crtc.active) else { return Err(CommitError::PlaneOnInactiveCrtc(p)) };
        let fb = next.fb(fbid).ok_or(CommitError::NoSuchFramebuffer(fbid))?;
        let pl = &dev.planes[i];
        if pl.formats & fb.format.bit() == 0 { return Err(CommitError::FormatUnsupported(p)); }
        let whole = Rect::new(0, 0, fb.width, fb.height);
        if st.src.w == 0 || st.src.h == 0 || !whole.contains(&st.src) { return Err(CommitError::BadSource(p)); }
        if st.dst.w == 0 || st.dst.h == 0 || st.dst.w > pl.max_width || st.dst.h > pl.max_height
            || (!pl.scaling && (st.src.w != st.dst.w || st.src.h != st.dst.h)) { return Err(CommitError::BadSize(p)); }
        let screen = Rect::new(0, 0, mode.hdisplay as u32, mode.vdisplay as u32);
        if !screen.intersects(&st.dst) { return Err(CommitError::Offscreen(p)); }
    }
    Ok(Checked { state: next, modeset, touched })
}

/// Extends a hardware frame counter (32-bit or narrower, wrapping) to a 64-bit
/// vblank sequence, so flip events never go backwards.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCounter { last: Option<u32>, seq: u64, bits: u32 }
impl FrameCounter {
    pub fn new(bits: u32) -> Self { Self { last: None, seq: 0, bits: bits.clamp(1, 32) } }
    pub fn update(&mut self, raw: u32) -> u64 {
        let mask = if self.bits == 32 { u32::MAX } else { (1u32 << self.bits) - 1 };
        let raw = raw & mask;
        if let Some(last) = self.last { self.seq += (raw.wrapping_sub(last) & mask) as u64; }
        self.last = Some(raw);
        self.seq
    }
    pub fn sequence(&self) -> u64 { self.seq }
}

/// Collects the changes for the usual "light this connector at its preferred
/// mode with this framebuffer full screen" commit.
pub fn light(dev: &Device, connector: u32, crtc: u32, fb: u32, fb_w: u32, fb_h: u32) -> Option<Vec<Change>> {
    let mode = dev.connectors.get(connector as usize)?.preferred()?;
    let primary = dev.primary_plane(crtc)? as u32;
    let (w, h) = (mode.hdisplay as u32, mode.vdisplay as u32);
    Some(alloc::vec![
        Change::Route { connector, crtc: Some(crtc) },
        Change::Mode { crtc, mode: Some(mode) },
        Change::Active { crtc, on: true },
        Change::Plane { plane: primary, crtc: Some(crtc), fb: Some(fb), src: Rect::new(0, 0, fb_w.min(w), fb_h.min(h)), dst: Rect::new(0, 0, fb_w.min(w), fb_h.min(h)) },
    ])
}
