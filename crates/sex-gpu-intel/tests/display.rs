// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Drives the Gen12 display logic against simulated registers: a GMBUS that
//! serves an HDMI EDID, an AUX channel with an eDP panel behind it, a frame
//! counter that ticks, and planes that latch at vblank.
use std::collections::HashMap;
use sex_gpu_intel::regs::*;
use sex_gpu_intel::{aux, backlight, gmbus, hotplug, plane, readout, Error, Mmio};
use sex_kms::commit::{self, Change, Commit};
use sex_kms::edid::Edid;
use sex_kms::model::{ConnectorKind, PlaneKind, Rect, Status};

#[path = "../../sex-kms/tests/fixtures/edid.rs"]
mod fixtures;
use fixtures::{panel_edid, tv_edid};

struct Sink { dpcd: Vec<u8>, edid: Vec<u8>, i2c_off: usize, defers: u32 }

#[derive(Default)]
struct Fake {
    regs: HashMap<u32, u32>,
    log: Vec<(u32, u32)>,
    ddc: HashMap<u32, Vec<u8>>,
    sinks: HashMap<u32, Sink>,
    gm: Option<(Vec<u8>, usize, usize)>,
    gm_nak: bool,
    frame_reads: u32,
    frozen: bool,
}

impl Fake {
    fn set(&mut self, r: u32, v: u32) { self.regs.insert(r, v); }
    fn get(&self, r: u32) -> u32 { *self.regs.get(&r).unwrap_or(&0) }
    fn vblank(&mut self) {
        for p in 0..PIPES {
            let c = self.get(pipe(p, FRMCOUNT));
            self.set(pipe(p, FRMCOUNT), c.wrapping_add(1));
            let s = self.get(plane(p, 1, PLANE_SURF));
            self.set(plane(p, 1, PLANE_SURFLIVE), s);
        }
    }
    fn aux(&mut self, port: u32, ctl: u32) {
        let n = ((ctl & AUX_SIZE_MASK) >> AUX_SIZE_SHIFT) as usize;
        let mut msg = [0u8; 20];
        for i in 0..5 { aux::unpack(self.get(aux_data(port, i)), &mut msg[i as usize * 4..i as usize * 4 + 4]); }
        let Some(s) = self.sinks.get_mut(&port) else { self.set(aux_ctl(port), AUX_DONE | AUX_TIME_OUT_ERROR); return };
        let mut reply = vec![0u8];
        if s.defers > 0 { s.defers -= 1; reply[0] = 0x20; } else {
            let cmd = msg[0] >> 4;
            let addr = ((msg[0] & 0xf) as usize) << 16 | (msg[1] as usize) << 8 | msg[2] as usize;
            let len = if n > 3 { msg[3] as usize + 1 } else { 0 };
            match cmd {
                aux::NATIVE_READ => reply.extend_from_slice(&s.dpcd[addr..addr + len]),
                aux::NATIVE_WRITE => s.dpcd[addr..addr + len].copy_from_slice(&msg[4..4 + len]),
                c if c & 3 == aux::I2C_WRITE && addr == 0x50 => if len > 0 { s.i2c_off = msg[4] as usize; },
                c if c & 3 == aux::I2C_READ && addr == 0x50 => {
                    reply.extend_from_slice(&s.edid[s.i2c_off..s.i2c_off + len]);
                    s.i2c_off += len;
                }
                _ => reply[0] = 0x40, // I2C NAK
            }
        }
        for (i, c) in reply.chunks(4).enumerate() { self.set(aux_data(port, i as u32), aux::pack(c)); }
        self.set(aux_ctl(port), AUX_DONE | (reply.len() as u32) << AUX_SIZE_SHIFT);
    }
}

impl Mmio for Fake {
    fn read(&mut self, r: u32) -> u32 {
        if r == GMBUS2 {
            if self.gm_nak { return GMBUS_SATOER | GMBUS_INUSE; }
            return match &self.gm { Some((_, _, left)) if *left > 0 => GMBUS_HW_RDY | GMBUS_ACTIVE,
                Some(_) => GMBUS_HW_WAIT_PHASE | GMBUS_ACTIVE, None => 0 };
        }
        if r == GMBUS3 {
            let mut v = [0u8; 4];
            if let Some((data, pos, left)) = &mut self.gm {
                for b in v.iter_mut() { if *left > 0 { *b = data[*pos]; *pos += 1; *left -= 1; } }
            }
            return u32::from_le_bytes(v);
        }
        if (0..PIPES).any(|p| r == pipe(p, FRMCOUNT)) {
            self.frame_reads += 1;
            if !self.frozen && self.frame_reads % 3 == 0 { self.vblank(); }
        }
        self.get(r)
    }
    fn write(&mut self, r: u32, v: u32) {
        self.log.push((r, v));
        if r == GMBUS1 {
            if v & GMBUS_SW_CLR_INT != 0 { self.gm = None; self.gm_nak = false; }
            else if v & GMBUS_SW_RDY != 0 && v & GMBUS_CYCLE_STOP == GMBUS_CYCLE_STOP { self.gm = None; }
            else if v & GMBUS_SW_RDY != 0 {
                let pin = self.get(GMBUS0) & 0x1f;
                match self.ddc.get(&pin) {
                    Some(e) if (v >> 1) & 0x7f == 0x50 => {
                        let off = ((v >> GMBUS_INDEX_SHIFT) & 0xff) as usize;
                        let len = ((v >> GMBUS_BYTE_COUNT_SHIFT) & 0x1ff) as usize;
                        self.gm = Some((e.clone(), off, len));
                    }
                    _ => self.gm_nak = true,
                }
            }
            return;
        }
        for port in 0..9 { if r == aux_ctl(port) && v & AUX_SEND_BUSY != 0 { self.aux(port, v); return; } }
        self.regs.insert(r, v);
    }
}

fn dpcd_edp() -> Vec<u8> {
    let mut d = vec![0u8; 0x800];
    d[0] = 0x12; d[1] = 0x14; d[2] = 0x84; d[0x70] = 0x03; d[0x700] = 0x04;
    d
}

/// Registers as UEFI GOP leaves an x17-class laptop: pipe A on DDI A (eDP),
/// 1920x1080 at 165 Hz, linear XRGB framebuffer at GGTT 0x10_0000.
fn gop_lit() -> Fake {
    let mut f = Fake::default();
    let w = |a: u16, b: u16| ((b as u32 - 1) << 16) | (a as u32 - 1);
    f.set(pipe(0, CONF), CONF_ENABLE | CONF_STATE);
    f.set(trans(0, DDI_FUNC_CTL), FUNC_ENABLE | 1 << FUNC_PORT_SHIFT | FUNC_MODE_DP_SST << FUNC_MODE_SHIFT | FUNC_PHSYNC);
    f.set(trans(0, HTOTAL), w(1920, 2080)); f.set(trans(0, HSYNC), w(1968, 2000));
    f.set(trans(0, VTOTAL), w(1080, 1101)); f.set(trans(0, VSYNC), w(1083, 1088));
    f.set(trans(0, PIPESRC), (1919 << 16) | 1079);
    f.set(plane(0, 1, PLANE_CTL), PLANE_ENABLE | PLANE_FORMAT_XRGB8888 | 1 << 13);
    f.set(plane(0, 1, PLANE_STRIDE), 7680 / 64);
    f.set(plane(0, 1, PLANE_SIZE), (1079 << 16) | 1919);
    f.set(plane(0, 1, PLANE_SURF), 0x10_0000);
    f.set(trans(0, PSR_CTL), PSR_ENABLE);
    f.set(SDEISR, sde_hotplug_bit(0));
    f.sinks.insert(0, Sink { dpcd: dpcd_edp(), edid: panel_edid(), i2c_off: 0, defers: 0 });
    f
}

#[test]
fn gmbus_reads_hdmi_edid_and_survives_nak() {
    let mut f = Fake::default();
    f.ddc.insert(gmbus_pin(3).unwrap(), tv_edid());
    let mut buf = [0u8; 256];
    assert_eq!(gmbus::read_edid(&mut f, 3, &mut buf), Ok(256));
    assert_eq!(&buf[..], &tv_edid()[..]);
    assert_eq!(f.get(GMBUS0), 0, "bus released");
    assert_eq!(gmbus::read_edid(&mut f, 4, &mut buf), Err(Error::Nak));
    assert!(f.log.iter().any(|&(r, v)| r == GMBUS1 && v == GMBUS_SW_CLR_INT));
    assert_eq!(f.get(GMBUS0), 0);
    assert_eq!(gmbus::read_edid(&mut f, 2, &mut buf), Err(Error::NotSupported));
    assert_eq!(gmbus_pin(0), Some(1));
    assert_eq!(gmbus_pin(8), Some(14));
}

#[test]
fn aux_reads_panel_edid_dpcd_and_retries_defer() {
    let mut f = gop_lit();
    f.sinks.get_mut(&0).unwrap().defers = 3;
    let mut buf = [0u8; 256];
    assert_eq!(aux::read_edid(&mut f, 0, &mut buf), Ok(128));
    assert_eq!(&buf[..128], &panel_edid()[..]);
    let c = aux::caps(&mut f, 0, true).unwrap();
    assert_eq!((c.rev, c.max_link_khz, c.max_lanes, c.psr, c.edp_rev), (0x12, 5_400_000, 4, 3, 4));
    aux::dpcd_write(&mut f, 0, aux::dpcd::SET_POWER, &[1]).unwrap();
    assert_eq!(f.sinks[&0].dpcd[0x600], 1);
    f.sinks.get_mut(&0).unwrap().defers = 99;
    assert_eq!(aux::dpcd_read(&mut f, 0, 0, &mut [0u8; 1]), Err(Error::Defer));
    let mut tv = Fake::default();
    tv.sinks.insert(3, Sink { dpcd: dpcd_edp(), edid: tv_edid(), i2c_off: 0, defers: 0 });
    assert_eq!(aux::read_edid(&mut tv, 3, &mut buf), Ok(256));
    assert_eq!(&buf[..], &tv_edid()[..]);
    assert_eq!(aux::read_edid(&mut tv, 4, &mut buf), Err(Error::Nak));
    assert_eq!(aux::pack(&[1, 2, 3, 4]), 0x0102_0304);
}

#[test]
fn readout_inherits_gop_panel_without_modeset() {
    let mut f = gop_lit();
    let pipes = readout::read_all(&mut f);
    let a = pipes[0];
    assert!(a.enabled && a.psr && !a.psr2);
    assert_eq!((a.port, a.bpc, a.src_w, a.src_h), (Some(0), 8, 1920, 1080));
    assert!(a.plane.linear_xrgb());
    assert_eq!((a.plane.stride_bytes, a.plane.surf), (7680, 0x10_0000));
    assert!(!pipes[1].enabled);

    let mut buf = [0u8; 256];
    let n = aux::read_edid(&mut f, 0, &mut buf).unwrap();
    let edid = Edid::parse(&buf[..n]).unwrap();
    let ports = vec![
        readout::Port { port: 0, kind: readout::kind(0, a.mode_select, Some(0)), live: true, edid: Some(edid.clone()) },
        readout::Port { port: 3, kind: ConnectorKind::Hdmi, live: hotplug::live(&mut f, 3), edid: None },
    ];
    let (dev, st) = readout::to_kms(&pipes, &ports, &[0; 4]);
    assert_eq!(dev.connectors[0].kind, ConnectorKind::Edp);
    assert_eq!(dev.connectors[1].status, Status::Disconnected);
    let mode = st.crtcs[0].mode.unwrap();
    assert_eq!(mode, edid.preferred().unwrap(), "exact EDID mode, clock included");
    assert_eq!(mode.refresh_hz(), 165);

    // Our own framebuffer replaces firmware's with a flip: no modeset needed.
    let mut st = st;
    let ours = st.add_fb(sex_kms::model::Framebuffer { width: 1920, height: 1080, stride: 7680, format: sex_kms::model::Format::Xrgb8888 }).unwrap();
    let primary = readout::plane_index(0, PlaneKind::Primary);
    let c = commit::check(&dev, &st, Commit { changes: &[Change::Flip { plane: primary, fb: ours }], flags: 0 }).unwrap();
    assert_eq!(c.modeset, 0);
    // Moving the panel to pipe B would be a modeset and is refused without permission.
    let mv = [Change::Route { connector: 0, crtc: Some(1) }];
    assert!(commit::check(&dev, &st, Commit { changes: &mv, flags: 0 }).is_err());

    // Without EDID, the measured refresh rebuilds the mode.
    let measured = readout::refresh_mhz(165 * 4, 4_000_000);
    let m = readout::running_mode(&a, None, measured).unwrap();
    assert_eq!(m.refresh_hz(), 165);
    assert!(m.clock_khz.abs_diff(377_000) < 1_000);
}

#[test]
fn flips_latch_at_vblank_with_surface_written_last() {
    let mut f = gop_lit();
    let s = plane::Surface { ggtt: 0x80_0000, stride_bytes: 7680, width: 1920, height: 1080 };
    f.log.clear();
    plane::set_primary(&mut f, 0, &s, 0, 0).unwrap();
    assert_eq!(f.log.last(), Some(&(plane(0, 1, PLANE_SURF), 0x80_0000)));
    let ctl = f.get(plane(0, 1, PLANE_CTL));
    assert!(ctl & PLANE_ENABLE != 0 && ctl & (1 << 13) != 0, "firmware's other bits kept");
    assert!(!plane::flip_done(&mut f, 0, 0x80_0000));
    plane::wait_vblank(&mut f, 0, 10).unwrap();
    assert!(plane::flip_done(&mut f, 0, 0x80_0000));
    plane::flip(&mut f, 0, 0x100_0000).unwrap();
    assert_eq!(plane::flip(&mut f, 0, 0x100_0100), Err(Error::BadArgument));
    plane::wait_vblank(&mut f, 0, 10).unwrap();
    assert!(plane::flip_done(&mut f, 0, 0x100_0000));
    assert!(plane::set_primary(&mut f, 0, &plane::Surface { stride_bytes: 7000, ..s }, 0, 0).is_err());
    let mut dead = Fake::default();
    dead.frozen = true; // a pipe that is off never counts frames
    assert!(plane::wait_vblank(&mut dead, 1, 100).is_err());
}

#[test]
fn hardware_cursor_moves_without_pixels() {
    let mut f = gop_lit();
    plane::cursor_set(&mut f, 0, 64, 0x20_0000, -10, 5).unwrap();
    assert_eq!(f.get(cursor(0, CUR_CTL)) & CUR_MODE_MASK, CUR_MODE_64_ARGB);
    assert_eq!(f.get(cursor(0, CUR_POS)), 5 << 16 | CUR_POS_SIGN | 10);
    f.log.clear();
    plane::cursor_move(&mut f, 0, 300, -2);
    assert_eq!(f.log, vec![(cursor(0, CUR_POS), (CUR_POS_SIGN | 2) << 16 | 300), (cursor(0, CUR_BASE), 0x20_0000)]);
    assert_eq!(plane::cursor_set(&mut f, 0, 100, 0x20_0000, 0, 0), Err(Error::BadArgument));
    plane::cursor_off(&mut f, 0);
    assert_eq!(f.get(cursor(0, CUR_CTL)) & CUR_MODE_MASK, 0);
}

#[test]
fn backlight_percent_never_zero() {
    let mut f = Fake::default();
    assert_eq!(backlight::set_percent(&mut f, 50), None);
    f.set(BLC_PWM_CTL, BLC_PWM_ENABLE); f.set(BLC_PWM_FREQ, 12_000); f.set(BLC_PWM_DUTY, 6_000);
    assert_eq!(backlight::read(&mut f).percent(), 50);
    assert_eq!(backlight::set_percent(&mut f, 0).unwrap().duty, 120);
    assert_eq!(backlight::set_percent(&mut f, 250).unwrap().duty, 12_000);
}

#[test]
fn hotplug_scan_and_diff() {
    let mut f = Fake::default();
    f.set(SDEISR, sde_hotplug_bit(0));
    assert_eq!(hotplug::scan(&mut f), 0b1);
    f.set(DE_HPD_ISR, tc_hotplug_bit(4));
    let now = hotplug::scan(&mut f);
    assert_eq!(now, 0b1001);
    assert_eq!(hotplug::diff(0b1, now), (0b1000, 0));
    assert_eq!(port_name(4), "TC2");
}

#[test]
fn ggtt_and_ids() {
    assert_eq!(ggtt_pte(0x1_2345_6789), 0x1_2345_6001);
    assert!(sex_gpu_intel::supported(0x8086, 0x9a60));
    assert!(!sex_gpu_intel::supported(0x10de, 0x249d));
    let _ = Rect::default();
}
