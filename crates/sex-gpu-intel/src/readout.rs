// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Read what firmware (UEFI GOP) left running, so SexOS can take the panel
//! over without a modeset: same timings, same pipe, new framebuffers.

#[cfg(feature = "kms")]
pub use kms::*;
use crate::regs::*;

/// Sync polarity bits, as `sex_kms::mode` numbers them.
const FLAG_HSYNC_POSITIVE: u8 = 1 << 0;
const FLAG_VSYNC_POSITIVE: u8 = 1 << 1;
use crate::Mmio;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PlaneReadout {
    pub enabled: bool,
    pub ctl: u32,
    pub stride_bytes: u32,
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    /// GGTT address the plane scans from.
    pub surf: u32,
}
impl PlaneReadout {
    pub fn linear_xrgb(&self) -> bool {
        self.ctl & PLANE_TILED_MASK == PLANE_TILED_LINEAR && self.ctl & PLANE_FORMAT_MASK == PLANE_FORMAT_XRGB8888
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PipeReadout {
    pub pipe: u32,
    pub enabled: bool,
    pub port: Option<u32>,
    /// FUNC_MODE_* of the transcoder.
    pub mode_select: u32,
    pub bpc: u8,
    pub h: [u16; 4],
    pub v: [u16; 4],
    pub flags: u8,
    pub src_w: u32,
    pub src_h: u32,
    pub plane: PlaneReadout,
    pub psr: bool,
    pub psr2: bool,
}

fn pair(v: u32) -> (u16, u16) { ((v & 0xffff) as u16 + 1, (v >> 16) as u16 + 1) }

pub fn read_pipe(m: &mut impl Mmio, p: u32) -> PipeReadout {
    let mut r = PipeReadout { pipe: p, ..Default::default() };
    let conf = m.read(pipe(p, CONF));
    let func = m.read(trans(p, DDI_FUNC_CTL));
    r.enabled = conf & CONF_ENABLE != 0 && func & FUNC_ENABLE != 0;
    if !r.enabled { return r; }
    let port = (func & FUNC_PORT_MASK) >> FUNC_PORT_SHIFT;
    r.port = port.checked_sub(1);
    r.mode_select = (func >> FUNC_MODE_SHIFT) & 7;
    r.bpc = bpc(func);
    if func & FUNC_PHSYNC != 0 { r.flags |= FLAG_HSYNC_POSITIVE; }
    if func & FUNC_PVSYNC != 0 { r.flags |= FLAG_VSYNC_POSITIVE; }
    let (ha, ht) = pair(m.read(trans(p, HTOTAL)));
    let (hs, he) = pair(m.read(trans(p, HSYNC)));
    let (va, vt) = pair(m.read(trans(p, VTOTAL)));
    let (vs, ve) = pair(m.read(trans(p, VSYNC)));
    r.h = [ha, hs, he, ht];
    r.v = [va, vs, ve, vt];
    let src = m.read(trans(p, PIPESRC));
    r.src_w = (src >> 16) + 1;
    r.src_h = (src & 0xffff) + 1;
    let ctl = m.read(plane(p, 1, PLANE_CTL));
    if ctl & PLANE_ENABLE != 0 {
        let size = m.read(plane(p, 1, PLANE_SIZE));
        let pos = m.read(plane(p, 1, PLANE_POS));
        r.plane = PlaneReadout {
            enabled: true, ctl,
            stride_bytes: (m.read(plane(p, 1, PLANE_STRIDE)) & 0x7ff) * STRIDE_UNIT,
            width: (size & 0xffff) + 1, height: (size >> 16) + 1,
            x: pos & 0xffff, y: pos >> 16,
            surf: m.read(plane(p, 1, PLANE_SURF)) & !(SURF_ALIGN - 1),
        };
    }
    r.psr = m.read(trans(p, PSR_CTL)) & PSR_ENABLE != 0;
    r.psr2 = m.read(trans(p, PSR2_CTL)) & PSR2_ENABLE != 0;
    r
}


#[cfg(feature = "kms")]
mod kms {
use alloc::vec::Vec;
use sex_kms::edid::Edid;
use sex_kms::mode::Mode;
use sex_kms::model::{Connector, ConnectorKind, Device, Format, Framebuffer, Plane, PlaneKind, PlaneState, Rect, State, Status};
use crate::regs::*;
use crate::Mmio;
use super::{read_pipe, PipeReadout};

pub fn read_all(m: &mut impl Mmio) -> Vec<PipeReadout> { (0..PIPES).map(|p| read_pipe(m, p)).collect() }

/// The connector type a port drives. Port A in DP mode is the eDP panel on
/// laptops (the VBT confirms this on hardware; see the L plan).
pub fn kind(port: u32, mode_select: u32, edp_port: Option<u32>) -> ConnectorKind {
    if Some(port) == edp_port { return ConnectorKind::Edp; }
    match mode_select { FUNC_MODE_HDMI => ConnectorKind::Hdmi, FUNC_MODE_DVI => ConnectorKind::Dvi, _ => ConnectorKind::DisplayPort }
}

/// The running mode of a pipe: the EDID mode with identical timings when the
/// monitor lists one (exact clock), otherwise one built from a measured refresh.
pub fn running_mode(r: &PipeReadout, edid: Option<&Edid>, measured_mhz: u32) -> Option<Mode> {
    if !r.enabled { return None; }
    let same = |m: &&Mode| [m.hdisplay, m.hsync_start, m.hsync_end, m.htotal] == r.h && [m.vdisplay, m.vsync_start, m.vsync_end, m.vtotal] == r.v;
    if let Some(m) = edid.and_then(|e| e.modes.iter().find(same)) { return Some(*m); }
    if measured_mhz == 0 { return None; }
    Some(Mode::from_measured(r.h, r.v, r.flags, measured_mhz))
}

/// Frames counted over a window of microseconds, as millihertz.
pub fn refresh_mhz(frames: u64, elapsed_us: u64) -> u32 {
    if elapsed_us == 0 { 0 } else { (frames * 1_000_000_000 / elapsed_us) as u32 }
}

/// One port SexOS found: its kind, whether a monitor is there, and its EDID.
#[derive(Clone, Debug)]
pub struct Port { pub port: u32, pub kind: ConnectorKind, pub live: bool, pub edid: Option<Edid> }

/// Plane layout per pipe: primary (universal plane 1), cursor, overlay (plane 2).
pub const PLANES_PER_PIPE: u32 = 3;
pub fn plane_index(pipe: u32, kind: PlaneKind) -> u32 { pipe * PLANES_PER_PIPE + kind as u32 }

/// Build the KMS device and its current state from the readout. Each inherited
/// primary plane registers the firmware framebuffer as fb id `pipe`.
pub fn to_kms(pipes: &[PipeReadout], ports: &[Port], measured_mhz: &[u32]) -> (Device, State) {
    let mut numbers = [0u8; 8];
    let mut connectors = Vec::new();
    for p in ports {
        numbers[p.kind as usize] += 1;
        let mut c = Connector::new(p.kind, numbers[p.kind as usize], (1 << PIPES) - 1);
        match &p.edid {
            Some(e) => c.apply_edid(e),
            None => c.status = if p.live { Status::Unknown } else { Status::Disconnected },
        }
        connectors.push(c);
    }
    let mut planes = Vec::new();
    for pipe in 0..PIPES {
        let on = 1 << pipe;
        planes.push(Plane { kind: PlaneKind::Primary, possible_crtcs: on, formats: Format::Xrgb8888.bit() | Format::Argb8888.bit() | Format::Xrgb2101010.bit(), max_width: 5120, max_height: 4096, scaling: false });
        planes.push(Plane { kind: PlaneKind::Cursor, possible_crtcs: on, formats: Format::Argb8888.bit(), max_width: 256, max_height: 256, scaling: false });
        planes.push(Plane { kind: PlaneKind::Overlay, possible_crtcs: on, formats: Format::Xrgb8888.bit() | Format::Nv12.bit(), max_width: 5120, max_height: 4096, scaling: true });
    }
    let dev = Device { connectors, crtcs: PIPES, planes };
    let mut st = State::off(&dev);
    for r in pipes.iter().filter(|r| r.enabled) {
        let Some(ci) = ports.iter().position(|p| Some(p.port) == r.port) else { continue };
        let mode = running_mode(r, ports[ci].edid.as_ref(), measured_mhz.get(r.pipe as usize).copied().unwrap_or(0));
        st.connectors[ci] = Some(r.pipe);
        st.crtcs[r.pipe as usize] = sex_kms::model::Crtc { active: mode.is_some(), mode };
        if r.plane.enabled && r.plane.linear_xrgb() {
            while st.fbs.len() <= r.pipe as usize { st.fbs.push(None); }
            st.fbs[r.pipe as usize] = Some(Framebuffer { width: r.plane.width, height: r.plane.height, stride: r.plane.stride_bytes, format: Format::Xrgb8888 });
            st.planes[plane_index(r.pipe, PlaneKind::Primary) as usize] = PlaneState {
                crtc: Some(r.pipe), fb: Some(r.pipe),
                src: Rect::new(0, 0, r.plane.width, r.plane.height),
                dst: Rect::new(r.plane.x as i32, r.plane.y as i32, r.plane.width, r.plane.height),
            };
        }
    }
    (dev, st)
}
}
