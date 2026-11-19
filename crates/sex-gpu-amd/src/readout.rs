// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Read what firmware (UEFI GOP) left running, so SexOS can take the panel
//! over without a modeset: same timings, same OTG, new framebuffers.
//!
//! DCN counts a line from the start of sync: sync runs 0..H_SYNC_A_END, active
//! video H_BLANK_END..H_BLANK_START, front porch H_BLANK_START..H_TOTAL+1.
//! The usual display-start form follows from that (DC's optc1_get_hw_timing).

use crate::asic::Asic;
use crate::regs::*;
use crate::Mmio;

#[cfg(feature = "kms")]
pub use kms::*;

/// Sync polarity bits, as `sex_kms::mode` numbers them.
pub const FLAG_HSYNC_POSITIVE: u8 = 1 << 0;
pub const FLAG_VSYNC_POSITIVE: u8 = 1 << 1;

/// One axis of DCN timing, as the registers hold it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Axis { pub total_minus_1: u32, pub blank_start: u32, pub blank_end: u32, pub sync_start: u32, pub sync_end: u32 }
impl Axis {
    /// (active, sync start, sync end, total) measured from the first active pixel,
    /// or None when the registers do not describe a sane line or frame.
    pub fn display_form(&self) -> Option<[u16; 4]> {
        let total = self.total_minus_1 + 1;
        if self.blank_end >= self.blank_start || self.blank_start > total || self.sync_end <= self.sync_start || total > 0x7FFF { return None; }
        let active = self.blank_start - self.blank_end;
        let front = total - self.blank_start;
        let width = self.sync_end - self.sync_start;
        let sync_start = active + front;
        if sync_start + width > total { return None; }
        Some([active as u16, sync_start as u16, (sync_start + width) as u16, total as u16])
    }
}

fn pair(v: u32) -> (u32, u32) { (v & FIELD15, (v >> 16) & FIELD15) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct OtgReadout {
    pub otg: u32,
    pub enabled: bool,
    pub h: Axis,
    pub v: Axis,
    pub flags: u8,
    pub frame_count: u32,
}
impl OtgReadout {
    pub fn h(&self) -> Option<[u16; 4]> { self.h.display_form() }
    pub fn v(&self) -> Option<[u16; 4]> { self.v.display_form() }
}

pub fn read_otg(m: &mut impl Mmio, asic: &Asic, i: u32) -> OtgReadout {
    let late = otg_late(asic.layout);
    let mut r = OtgReadout { otg: i, ..Default::default() };
    let control = m.read(otg(i, late.control));
    r.enabled = control & OTG_MASTER_EN != 0;
    if !r.enabled { return r; }
    let (hbs, hbe) = pair(m.read(otg(i, OTG_H_BLANK_START_END)));
    let (hss, hse) = pair(m.read(otg(i, OTG_H_SYNC_A)));
    let (vbs, vbe) = pair(m.read(otg(i, late.v_blank_start_end)));
    let (vss, vse) = pair(m.read(otg(i, late.v_sync_a)));
    r.h = Axis { total_minus_1: m.read(otg(i, OTG_H_TOTAL)) & FIELD15, blank_start: hbs, blank_end: hbe, sync_start: hss, sync_end: hse };
    r.v = Axis { total_minus_1: m.read(otg(i, OTG_V_TOTAL)) & FIELD15, blank_start: vbs, blank_end: vbe, sync_start: vss, sync_end: vse };
    if m.read(otg(i, OTG_H_SYNC_A_CNTL)) & SYNC_POL_NEGATIVE == 0 { r.flags |= FLAG_HSYNC_POSITIVE; }
    if m.read(otg(i, late.v_sync_a_cntl)) & SYNC_POL_NEGATIVE == 0 { r.flags |= FLAG_VSYNC_POSITIVE; }
    r.frame_count = m.read(otg(i, late.frame_count)) & FRAME_COUNT_MASK;
    r
}

/// Pixel formats SexOS scans out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat { Rgb565, Xrgb8888, Xrgb2101010, Other(u32) }
impl PixelFormat {
    pub fn from_hw(v: u32) -> Self {
        match v & PIXEL_FORMAT_MASK { PIXEL_FORMAT_RGB565 => Self::Rgb565, PIXEL_FORMAT_ARGB8888 => Self::Xrgb8888,
            PIXEL_FORMAT_ARGB2101010 => Self::Xrgb2101010, o => Self::Other(o) }
    }
    pub fn bytes(self) -> u32 { match self { Self::Rgb565 => 2, Self::Other(_) => 0, _ => 4 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HubpReadout {
    pub pipe: u32,
    /// HUBP fetching (not blanked).
    pub enabled: bool,
    /// The OTG this pipe feeds (HUBP_VTG_SEL).
    pub otg: u32,
    pub format: PixelFormat,
    pub linear: bool,
    pub pitch_pixels: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Address the pipe scans from, in the display's address space (48 bits).
    pub address: u64,
}
impl HubpReadout {
    pub fn stride_bytes(&self) -> u32 { self.pitch_pixels * self.format.bytes() }
    /// A surface SexOS can draw into directly.
    pub fn linear_xrgb(&self) -> bool { self.linear && self.format == PixelFormat::Xrgb8888 }
}

pub fn read_hubp(m: &mut impl Mmio, i: u32) -> HubpReadout {
    let cntl = m.read(hubp(i, HUBP_CNTL));
    let start = m.read(hubp(i, HUBP_PRI_VIEWPORT_START));
    let dim = m.read(hubp(i, HUBP_PRI_VIEWPORT_DIMENSION));
    let high = m.read(hubp(i, HUBPREQ_PRIMARY_SURFACE_ADDRESS_HIGH)) & ADDRESS_HIGH_MASK;
    let low = m.read(hubp(i, HUBPREQ_PRIMARY_SURFACE_ADDRESS));
    HubpReadout {
        pipe: i,
        enabled: cntl & HUBP_BLANK_EN == 0,
        otg: (cntl & HUBP_VTG_SEL_MASK) >> HUBP_VTG_SEL_SHIFT,
        format: PixelFormat::from_hw(m.read(hubp(i, HUBP_SURFACE_CONFIG))),
        linear: m.read(hubp(i, HUBP_TILING_CONFIG)) & SW_MODE_MASK == SW_MODE_LINEAR,
        pitch_pixels: (m.read(hubp(i, HUBPREQ_SURFACE_PITCH)) & FIELD14) + 1,
        x: start & FIELD14, y: (start >> 16) & FIELD14,
        width: dim & FIELD14, height: (dim >> 16) & FIELD14,
        address: (high as u64) << 32 | low as u64,
    }
}

/// Where VRAM (on an APU: the carve-out) sits in the display's address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FbAperture { pub base: u64, pub top: u64, pub offset: u64 }
impl FbAperture {
    /// Byte offset into VRAM (BAR 0 / the carve-out) of a display address, if inside.
    pub fn vram_offset(&self, address: u64) -> Option<u64> {
        (address >= self.base && address < self.top + (1 << FB_UNIT_SHIFT)).then(|| address - self.base + self.offset)
    }
}
pub fn read_aperture(m: &mut impl Mmio, asic: &Asic) -> FbAperture {
    let f = |v: u32| ((v & FB_FIELD) as u64) << FB_UNIT_SHIFT;
    FbAperture { base: f(m.read(fb_location_base(asic.layout))), top: f(m.read(fb_location_top(asic.layout))), offset: f(m.read(fb_offset(asic.layout))) }
}

/// Everything firmware left lit: each enabled OTG with the unblanked pipe that feeds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head { pub otg: OtgReadout, pub hubp: Option<HubpReadout> }

/// Calls `f` for each lit head, OTG order. No allocation.
pub fn for_each_head(m: &mut impl Mmio, asic: &Asic, mut f: impl FnMut(Head)) {
    for i in 0..asic.otgs {
        let o = read_otg(m, asic, i);
        if !o.enabled { continue; }
        let mut found = None;
        for p in 0..asic.pipes {
            let h = read_hubp(m, p);
            if h.enabled && h.otg == i { found = Some(h); break; }
        }
        f(Head { otg: o, hubp: found });
    }
}

#[cfg(feature = "kms")]
mod kms {
use alloc::vec::Vec;
use sex_kms::edid::Edid;
use sex_kms::mode::Mode;
use sex_kms::model::{Connector, ConnectorKind, Crtc, Device, Format, Framebuffer, Plane, PlaneKind, PlaneState, Rect, State, Status};
use crate::asic::Asic;
use crate::Mmio;
use super::{for_each_head, Head, OtgReadout};

pub fn read_heads(m: &mut impl Mmio, asic: &Asic) -> Vec<Head> {
    let mut v = Vec::new();
    for_each_head(m, asic, |h| v.push(h));
    v
}

/// The running mode of an OTG: the EDID mode with identical timings when the
/// monitor lists one (exact clock), otherwise one built from a measured refresh.
pub fn running_mode(o: &OtgReadout, edid: Option<&Edid>, measured_mhz: u32) -> Option<Mode> {
    let (h, v) = (o.h()?, o.v()?);
    let same = |m: &&Mode| [m.hdisplay, m.hsync_start, m.hsync_end, m.htotal] == h && [m.vdisplay, m.vsync_start, m.vsync_end, m.vtotal] == v;
    if let Some(m) = edid.and_then(|e| e.modes.iter().find(same)) { return Some(*m); }
    if measured_mhz == 0 { return None; }
    Some(Mode::from_measured(h, v, o.flags, measured_mhz))
}

/// Frames counted over a window of microseconds, as millihertz.
pub fn refresh_mhz(frames: u64, elapsed_us: u64) -> u32 {
    if elapsed_us == 0 { 0 } else { (frames * 1_000_000_000 / elapsed_us) as u32 }
}

/// One output SexOS found: its kind, the OTG lighting it (if any), and its EDID.
#[derive(Clone, Debug)]
pub struct Port { pub kind: ConnectorKind, pub otg: Option<u32>, pub live: bool, pub edid: Option<Edid> }

/// Plane layout per OTG: primary, cursor.
pub const PLANES_PER_CRTC: u32 = 2;
pub fn plane_index(crtc: u32, kind: PlaneKind) -> u32 { crtc * PLANES_PER_CRTC + if kind == PlaneKind::Cursor { 1 } else { 0 } }

/// Build the KMS device and its current state. Each inherited primary plane
/// registers the firmware framebuffer as fb id `otg`.
pub fn to_kms(asic: &Asic, heads: &[Head], ports: &[Port], measured_mhz: &[u32]) -> (Device, State) {
    let crtcs = asic.otgs;
    let mut numbers = [0u8; 8];
    let mut connectors = Vec::new();
    for p in ports {
        numbers[p.kind as usize] += 1;
        let mut c = Connector::new(p.kind, numbers[p.kind as usize], (1 << crtcs) - 1);
        match &p.edid {
            Some(e) => c.apply_edid(e),
            None => c.status = if p.live { Status::Unknown } else { Status::Disconnected },
        }
        connectors.push(c);
    }
    let mut planes = Vec::new();
    for crtc in 0..crtcs {
        let on = 1 << crtc;
        planes.push(Plane { kind: PlaneKind::Primary, possible_crtcs: on, formats: Format::Xrgb8888.bit() | Format::Argb8888.bit() | Format::Xrgb2101010.bit(), max_width: 8192, max_height: 8192, scaling: true });
        planes.push(Plane { kind: PlaneKind::Cursor, possible_crtcs: on, formats: Format::Argb8888.bit(), max_width: 256, max_height: 256, scaling: false });
    }
    let dev = Device { connectors, crtcs, planes };
    let mut st = State::off(&dev);
    for h in heads {
        let o = h.otg.otg;
        let Some(ci) = ports.iter().position(|p| p.otg == Some(o)) else { continue };
        let mode = running_mode(&h.otg, ports[ci].edid.as_ref(), measured_mhz.get(o as usize).copied().unwrap_or(0));
        st.connectors[ci] = Some(o);
        st.crtcs[o as usize] = Crtc { active: mode.is_some(), mode };
        if let Some(p) = h.hubp.filter(|p| p.linear_xrgb()) {
            while st.fbs.len() <= o as usize { st.fbs.push(None); }
            st.fbs[o as usize] = Some(Framebuffer { width: p.width, height: p.height, stride: p.stride_bytes(), format: Format::Xrgb8888 });
            st.planes[plane_index(o, PlaneKind::Primary) as usize] = PlaneState {
                crtc: Some(o), fb: Some(o),
                src: Rect::new(p.x as i32, p.y as i32, p.width, p.height),
                dst: Rect::new(0, 0, p.width, p.height),
            };
        }
    }
    (dev, st)
}
}
