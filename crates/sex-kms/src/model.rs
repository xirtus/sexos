// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The objects a display driver exposes and the state they are in.
//!
//! Connector: a physical (or virtual) output and the modes it accepts.
//! CRTC: a scanout engine (Intel pipe+transcoder, virtio-gpu scanout) running one mode.
//! Plane: one layer a CRTC blends: primary picture, cursor or overlay (video).
//! Framebuffer: pixels a plane reads, registered with the driver beforehand.

use alloc::string::String;
use alloc::vec::Vec;
use crate::mode::Mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectorKind { Virtual = 0, Edp = 1, DisplayPort = 2, Hdmi = 3, Dvi = 4, Vga = 5, Lvds = 6, Dsi = 7 }
impl ConnectorKind {
    pub fn from(v: u8) -> Self {
        match v { 1 => Self::Edp, 2 => Self::DisplayPort, 3 => Self::Hdmi, 4 => Self::Dvi, 5 => Self::Vga, 6 => Self::Lvds, 7 => Self::Dsi, _ => Self::Virtual }
    }
    pub fn name(self) -> &'static str {
        match self { Self::Virtual => "Virtual", Self::Edp => "eDP", Self::DisplayPort => "DP", Self::Hdmi => "HDMI",
            Self::Dvi => "DVI", Self::Vga => "VGA", Self::Lvds => "LVDS", Self::Dsi => "DSI" }
    }
    /// Built into the machine (a laptop panel), so it is the first screen.
    pub fn internal(self) -> bool { matches!(self, Self::Edp | Self::Lvds | Self::Dsi) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Status { Disconnected = 0, Connected = 1, Unknown = 2 }
impl Status { pub fn from(v: u8) -> Self { match v { 0 => Self::Disconnected, 1 => Self::Connected, _ => Self::Unknown } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PlaneKind { Primary = 0, Cursor = 1, Overlay = 2 }

/// Pixel formats, as bits in a plane's `formats` mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Format { Xrgb8888 = 0, Argb8888 = 1, Xbgr8888 = 2, Rgb565 = 3, Nv12 = 4, Xrgb2101010 = 5 }
impl Format {
    pub const fn bit(self) -> u32 { 1 << self as u8 }
    pub fn from(v: u8) -> Option<Self> {
        Some(match v { 0 => Self::Xrgb8888, 1 => Self::Argb8888, 2 => Self::Xbgr8888, 3 => Self::Rgb565, 4 => Self::Nv12, 5 => Self::Xrgb2101010, _ => return None })
    }
    /// Bytes per pixel of the first (luma) plane.
    pub fn bytes(self) -> u32 { match self { Self::Rgb565 => 2, Self::Nv12 => 1, _ => 4 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }
impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self { Self { x, y, w, h } }
    pub fn right(&self) -> i64 { self.x as i64 + self.w as i64 }
    pub fn bottom(&self) -> i64 { self.y as i64 + self.h as i64 }
    pub fn intersects(&self, o: &Rect) -> bool {
        (self.x as i64) < o.right() && (o.x as i64) < self.right() && (self.y as i64) < o.bottom() && (o.y as i64) < self.bottom()
    }
    pub fn contains(&self, o: &Rect) -> bool {
        self.x <= o.x && self.y <= o.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && (x as i64) < self.right() && (y as i64) < self.bottom()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connector {
    pub kind: ConnectorKind,
    /// 1-based number among connectors of the same kind ("HDMI-2").
    pub number: u8,
    pub status: Status,
    pub modes: Vec<Mode>,
    pub width_mm: u16,
    pub height_mm: u16,
    /// Monitor name from EDID, empty when unknown.
    pub monitor: String,
    /// Bit `n` set: CRTC `n` can drive this connector.
    pub possible_crtcs: u32,
    /// Virtual outputs (virtio-gpu) accept any valid mode, not only listed ones.
    pub any_mode: bool,
}
impl Connector {
    pub fn new(kind: ConnectorKind, number: u8, possible_crtcs: u32) -> Self {
        Self { kind, number, status: Status::Unknown, modes: Vec::new(), width_mm: 0, height_mm: 0,
            monitor: String::new(), possible_crtcs, any_mode: kind == ConnectorKind::Virtual }
    }
    pub fn accepts(&self, m: &Mode) -> bool {
        m.valid() && (self.any_mode || self.modes.iter().any(|o| o.same_timing(m)))
    }
    pub fn preferred(&self) -> Option<Mode> {
        self.modes.iter().copied().find(|m| m.preferred()).or_else(|| self.modes.first().copied())
    }
    /// Fill status, size, name and modes from a parsed EDID.
    pub fn apply_edid(&mut self, e: &crate::edid::Edid) {
        self.status = Status::Connected;
        self.width_mm = e.width_mm;
        self.height_mm = e.height_mm;
        self.monitor = e.name.clone();
        self.modes = e.modes.clone();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    pub kind: PlaneKind,
    pub possible_crtcs: u32,
    pub formats: u32,
    pub max_width: u32,
    pub max_height: u32,
    /// The plane can scale (src size differs from dst size).
    pub scaling: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framebuffer { pub width: u32, pub height: u32, pub stride: u32, pub format: Format }
impl Framebuffer {
    pub fn valid(&self) -> bool {
        self.width > 0 && self.height > 0 && self.stride >= self.width * self.format.bytes()
    }
}

/// What the hardware has: fixed for the life of the driver, apart from connector status and modes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Device {
    pub connectors: Vec<Connector>,
    /// Number of CRTCs.
    pub crtcs: u32,
    pub planes: Vec<Plane>,
}
impl Device {
    pub fn primary_plane(&self, crtc: u32) -> Option<usize> {
        self.planes.iter().position(|p| p.kind == PlaneKind::Primary && p.possible_crtcs & (1 << crtc) != 0)
    }
    pub fn plane_of(&self, crtc: u32, kind: PlaneKind) -> Option<usize> {
        self.planes.iter().position(|p| p.kind == kind && p.possible_crtcs & (1 << crtc) != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Crtc { pub active: bool, pub mode: Option<Mode> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PlaneState { pub crtc: Option<u32>, pub fb: Option<u32>, pub src: Rect, pub dst: Rect }

/// The configuration the hardware is in, or would be in after a commit.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    pub crtcs: Vec<Crtc>,
    /// The CRTC each connector is bound to.
    pub connectors: Vec<Option<u32>>,
    pub planes: Vec<PlaneState>,
    /// Registered framebuffers by id (index); `None` once removed.
    pub fbs: Vec<Option<Framebuffer>>,
}
impl State {
    /// Everything off: the state a driver starts from when it cannot read firmware's.
    pub fn off(dev: &Device) -> Self {
        Self { crtcs: alloc::vec![Crtc::default(); dev.crtcs as usize],
            connectors: alloc::vec![None; dev.connectors.len()],
            planes: alloc::vec![PlaneState::default(); dev.planes.len()], fbs: Vec::new() }
    }
    pub fn add_fb(&mut self, fb: Framebuffer) -> Option<u32> {
        if !fb.valid() { return None; }
        if let Some(i) = self.fbs.iter().position(|f| f.is_none()) { self.fbs[i] = Some(fb); return Some(i as u32); }
        self.fbs.push(Some(fb));
        Some(self.fbs.len() as u32 - 1)
    }
    /// A framebuffer still shown by a plane cannot be removed.
    pub fn remove_fb(&mut self, id: u32) -> bool {
        if self.planes.iter().any(|p| p.fb == Some(id)) { return false; }
        match self.fbs.get_mut(id as usize) { Some(f @ Some(_)) => { *f = None; true } _ => false }
    }
    pub fn fb(&self, id: u32) -> Option<Framebuffer> { self.fbs.get(id as usize).copied().flatten() }
    pub fn crtc_connectors(&self, crtc: u32) -> impl Iterator<Item = usize> + '_ {
        self.connectors.iter().enumerate().filter(move |(_, c)| **c == Some(crtc)).map(|(i, _)| i)
    }
}
