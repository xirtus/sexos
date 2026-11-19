// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Drives the DCN display logic against simulated registers: OTG timing as
//! firmware programs it, a HUBP scanning a linear surface, an AUX engine with
//! an eDP panel behind it (DPCD + EDID over I2C-over-AUX), a frame counter
//! that ticks, and flips that latch at vblank.
use std::collections::HashMap;
use sex_gpu_amd::regs::*;
use sex_gpu_amd::{aux, identify, plane, readout, Asic, Error, Layout, Mmio};
use sex_kms::edid::Edid;
use sex_kms::mode::{cvt_rb, cta_vic, Mode, FLAG_HSYNC_POSITIVE, FLAG_VSYNC_POSITIVE};
use sex_kms::model::{ConnectorKind, PlaneKind, Status};

#[path = "../../sex-kms/tests/fixtures/edid.rs"]
mod fixtures;
use fixtures::panel_edid;

struct Sink { dpcd: Vec<u8>, edid: Vec<u8>, offset: usize, defers: u32 }

#[derive(Default)]
struct Fake {
    regs: HashMap<u32, u32>,
    sinks: HashMap<u32, Sink>,
    /// Per AUX channel: request bytes, reply bytes, data pointer, read mode.
    tx: HashMap<u32, (Vec<u8>, Vec<u8>, usize, bool)>,
    dmcu_owns: bool,
    frame_reads: u32,
    pending_reads: u32,
    log: Vec<(u32, u32)>,
}

fn channel_of(off: u32) -> Option<(u32, u32)> {
    let base = at(AUX_CONTROL);
    if off < base || off >= base + 6 * AUX_STRIDE * 4 { return None; }
    let rel = (off - base) / 4;
    Some((rel / AUX_STRIDE, AUX_CONTROL + rel % AUX_STRIDE))
}

impl Fake {
    fn set(&mut self, off: u32, v: u32) { self.regs.insert(off, v); }
    fn get(&self, off: u32) -> u32 { *self.regs.get(&off).unwrap_or(&0) }

    fn run_aux(&mut self, ch: u32) {
        let (req, _, _, _) = self.tx.get(&ch).cloned().unwrap_or_default();
        let mut reply = Vec::new();
        let status_off = aux(ch, AUX_SW_STATUS);
        let Some(sink) = self.sinks.get_mut(&ch) else {
            self.set(status_off, AUX_SW_DONE | AUX_SW_HPD_DISCON);
            return;
        };
        let cmd = req[0] >> 4;
        let addr = ((req[0] as u32 & 0xF) << 16) | (req[1] as u32) << 8 | req[2] as u32;
        let len = if req.len() > 3 { req[3] as usize + 1 } else { 0 };
        if sink.defers > 0 { sink.defers -= 1; reply.push(0x20); }
        else if cmd == aux::NATIVE_READ {
            reply.push(0);
            for i in 0..len { reply.push(*sink.dpcd.get(addr as usize + i).unwrap_or(&0)); }
        } else if cmd == aux::NATIVE_WRITE {
            for (i, b) in req[4..].iter().enumerate() { let a = addr as usize + i; if a < sink.dpcd.len() { sink.dpcd[a] = *b; } }
            reply.push(0);
        } else if cmd & 3 == aux::I2C_WRITE {
            if addr != aux::EDID_ADDR as u32 { reply.push(0x40); } else {
                if len > 0 { sink.offset = req[4] as usize; }
                reply.push(0);
            }
        } else if cmd & 3 == aux::I2C_READ {
            if addr != aux::EDID_ADDR as u32 { reply.push(0x40); } else {
                reply.push(0);
                for _ in 0..len { reply.push(*sink.edid.get(sink.offset).unwrap_or(&0xFF)); sink.offset += 1; }
            }
        }
        self.set(status_off, AUX_SW_DONE | (reply.len() as u32) << AUX_SW_REPLY_BYTE_COUNT_SHIFT);
        self.tx.get_mut(&ch).unwrap().1 = reply;
    }
}

impl Mmio for Fake {
    fn read(&mut self, off: u32) -> u32 {
        if let Some((ch, reg)) = channel_of(off) {
            match reg {
                AUX_ARB_CONTROL => {
                    let v = self.get(off) & !AUX_REG_RW_CNTL_STATUS_MASK;
                    let s = if self.dmcu_owns { ARB_DMCU } else if v & AUX_SW_USE_AUX_REG_REQ != 0 { ARB_SW } else { 0 };
                    return v | s << AUX_REG_RW_CNTL_STATUS_SHIFT;
                }
                AUX_SW_DATA => {
                    let t = self.tx.entry(ch).or_default();
                    let b = *t.1.get(t.2).unwrap_or(&0);
                    t.2 += 1;
                    return (b as u32) << AUX_SW_DATA_SHIFT;
                }
                _ => {}
            }
        }
        for o in 0..6 {
            for l in [Layout::Dcn20, Layout::Dcn35] {
                if off == otg(o, otg_late(l).frame_count) { self.frame_reads += 1; return self.get(off) + self.frame_reads / 3; }
            }
        }
        if (0..6).any(|p| off == hubp(p, HUBPREQ_FLIP_CONTROL)) && self.get(off) & SURFACE_FLIP_PENDING != 0 {
            self.pending_reads += 1;
            if self.pending_reads >= 3 { let v = self.get(off) & !SURFACE_FLIP_PENDING; self.set(off, v); }
        }
        self.get(off)
    }
    fn write(&mut self, off: u32, v: u32) {
        self.log.push((off, v));
        if let Some((ch, reg)) = channel_of(off) {
            match reg {
                AUX_CONTROL => {
                    let done = if v & AUX_RESET != 0 { AUX_RESET_DONE } else { 0 };
                    self.set(off, (v & !AUX_RESET_DONE) | done);
                    return;
                }
                AUX_INTERRUPT_CONTROL if v & AUX_SW_DONE_ACK != 0 => {
                    let s = aux(ch, AUX_SW_STATUS);
                    self.set(s, 0);
                    return;
                }
                AUX_SW_DATA => {
                    let t = self.tx.entry(ch).or_default();
                    if v & AUX_SW_DATA_RW != 0 { t.2 = (v >> AUX_SW_INDEX_SHIFT & 0x1F) as usize; t.3 = true; return; }
                    if v & AUX_SW_AUTOINCREMENT_DISABLE != 0 { t.0.clear(); t.3 = false; }
                    t.0.push((v >> AUX_SW_DATA_SHIFT) as u8);
                    return;
                }
                AUX_SW_CONTROL if v & AUX_SW_GO != 0 => {
                    let n = ((v & AUX_SW_WR_BYTES_MASK) >> AUX_SW_WR_BYTES_SHIFT) as usize;
                    assert_eq!(self.tx[&ch].0.len(), n, "AUX_SW_WR_BYTES must match the bytes written");
                    assert!(self.get(aux(ch, AUX_ARB_CONTROL)) & AUX_SW_USE_AUX_REG_REQ != 0, "GO without owning the engine");
                    self.set(off, v & !AUX_SW_GO);
                    self.run_aux(ch);
                    return;
                }
                _ => {}
            }
        }
        if (0..6).any(|p| off == hubp(p, HUBPREQ_PRIMARY_SURFACE_ADDRESS)) {
            let fc = off - at(HUBPREQ_PRIMARY_SURFACE_ADDRESS) + at(HUBPREQ_FLIP_CONTROL);
            let f = self.get(fc) | SURFACE_FLIP_PENDING;
            self.set(fc, f);
            self.pending_reads = 0;
        }
        self.set(off, v);
    }
}

/// Program an OTG the way DC's optc1_program_timing does.
fn program_otg(f: &mut Fake, asic: &Asic, i: u32, m: &Mode) {
    let late = otg_late(asic.layout);
    let (hfp, hsw) = ((m.hsync_start - m.hdisplay) as u32, (m.hsync_end - m.hsync_start) as u32);
    let (vfp, vsw) = ((m.vsync_start - m.vdisplay) as u32, (m.vsync_end - m.vsync_start) as u32);
    let (ht, vt) = (m.htotal as u32, m.vtotal as u32);
    f.set(otg(i, OTG_H_TOTAL), ht - 1);
    f.set(otg(i, OTG_H_SYNC_A), hsw << 16);
    let hbs = ht - hfp;
    f.set(otg(i, OTG_H_BLANK_START_END), hbs | (hbs - m.hdisplay as u32) << 16);
    f.set(otg(i, OTG_H_SYNC_A_CNTL), (m.flags & FLAG_HSYNC_POSITIVE == 0) as u32);
    f.set(otg(i, OTG_V_TOTAL), vt - 1);
    f.set(otg(i, late.v_sync_a), vsw << 16);
    let vbs = vt - vfp;
    f.set(otg(i, late.v_blank_start_end), vbs | (vbs - m.vdisplay as u32) << 16);
    f.set(otg(i, late.v_sync_a_cntl), (m.flags & FLAG_VSYNC_POSITIVE == 0) as u32);
    f.set(otg(i, late.control), OTG_MASTER_EN | OTG_CURRENT_MASTER_EN_STATE);
    f.set(otg(i, late.frame_count), 1000);
}

fn program_hubp(f: &mut Fake, p: u32, otg_index: u32, w: u32, h: u32, pitch: u32, address: u64) {
    f.set(hubp(p, HUBP_CNTL), otg_index << HUBP_VTG_SEL_SHIFT);
    f.set(hubp(p, HUBP_SURFACE_CONFIG), PIXEL_FORMAT_ARGB8888);
    f.set(hubp(p, HUBP_TILING_CONFIG), SW_MODE_LINEAR);
    f.set(hubp(p, HUBPREQ_SURFACE_PITCH), pitch - 1);
    f.set(hubp(p, HUBP_PRI_VIEWPORT_DIMENSION), w | h << 16);
    f.set(hubp(p, HUBPREQ_PRIMARY_SURFACE_ADDRESS_HIGH), (address >> 32) as u32);
    f.set(hubp(p, HUBPREQ_PRIMARY_SURFACE_ADDRESS), address as u32);
}

fn panel_mode() -> Mode { Edid::parse(&panel_edid()).unwrap().modes.into_iter().find(|m| m.preferred()).unwrap() }

fn phoenix() -> Asic { identify(0x1002, 0x15BF).unwrap() }

/// A Phoenix laptop as GOP leaves it: eDP panel on OTG0 / HUBP0 / AUX0.
fn laptop() -> Fake {
    let asic = phoenix();
    let mut f = Fake::default();
    program_otg(&mut f, &asic, 0, &panel_mode());
    program_hubp(&mut f, 0, 0, 1920, 1080, 2048, 0xF4_0010_0000);
    for p in 1..4 { f.set(hubp(p, HUBP_CNTL), HUBP_BLANK_EN); }
    f.set(fb_location_base(asic.layout), 0xF400);
    f.set(fb_location_top(asic.layout), 0xF41F);
    let mut dpcd = vec![0u8; 0x800];
    dpcd[0] = 0x14; dpcd[1] = 0x1E; dpcd[2] = 0x84; dpcd[0x70] = 0x03; dpcd[0x700] = 0x04;
    f.sinks.insert(0, Sink { dpcd, edid: panel_edid(), offset: 0, defers: 0 });
    f
}

#[test]
fn chip_table() {
    let p = phoenix();
    assert_eq!((p.name, p.dcn, p.layout, p.apu), ("Phoenix", (3, 1, 4), Layout::Dcn31, true));
    assert_eq!(identify(0x1002, 0x1638).unwrap().layout, Layout::Dcn20, "Cezanne is DCN 2.1");
    assert_eq!(identify(0x1002, 0x1681).unwrap().layout, Layout::Dcn31, "Rembrandt is DCN 3.1.2");
    assert_eq!(identify(0x1002, 0x150E).unwrap().layout, Layout::Dcn35, "Strix is DCN 3.5");
    assert_eq!(identify(0x1002, 0x73BF).unwrap().otgs, 6);
    assert!(identify(0x10de, 0x249d).is_none() && identify(0x1002, 0x0001).is_none());
    for a in sex_gpu_amd::asic::TABLE {
        assert!(a.pipes <= a.otgs && a.otgs <= 6 && a.pipes <= 6 && a.auxes <= 6, "{}", a.name);
        assert_eq!(sex_gpu_amd::asic::TABLE.iter().filter(|b| b.device == a.device).count(), 1, "duplicate {:#x}", a.device);
    }
}

#[test]
fn register_offsets_match_amd_headers() {
    // Spot checks against dcn_3_1_4_offset.h (segment 2 base 0x34C0).
    assert_eq!(otg(1, otg_late(Layout::Dcn31).control), (0x34C0 + 0x1BC1) * 4);
    assert_eq!(otg(3, otg_late(Layout::Dcn35).control), (0x34C0 + 0x1CC3) * 4, "dcn_3_5_0 OTG3_OTG_CONTROL");
    assert_eq!(hubp(5, HUBP_CNTL), (0x34C0 + 0x0A3F) * 4, "dcn_3_0_0 HUBP5_DCHUBP_CNTL");
    assert_eq!(hubp(1, HUBPREQ_PRIMARY_SURFACE_ADDRESS), (0x34C0 + 0x06E6) * 4);
    assert_eq!(aux(4, AUX_CONTROL), (0x34C0 + 0x1FC0) * 4);
    assert_eq!(fb_location_base(Layout::Dcn20), (0x34C0 + 0x0493) * 4);
    assert_eq!(fb_location_base(Layout::Dcn31), (0x34C0 + 0x0475) * 4);
}

#[test]
fn timing_round_trips_for_many_modes() {
    for asic in [phoenix(), identify(0x1002, 0x1638).unwrap(), identify(0x1002, 0x150E).unwrap()] {
        let mut modes = vec![panel_mode()];
        modes.extend([16u8, 4, 19, 97].iter().filter_map(|v| cta_vic(*v)));
        modes.extend([(2560, 1600, 120), (2880, 1800, 90), (1366, 768, 60), (3840, 2160, 60)].iter().filter_map(|&(w, h, r)| cvt_rb(w, h, r)));
        for m in modes {
            let mut f = Fake::default();
            program_otg(&mut f, &asic, 2, &m);
            let o = readout::read_otg(&mut f, &asic, 2);
            assert!(o.enabled);
            assert_eq!(o.h().unwrap(), [m.hdisplay, m.hsync_start, m.hsync_end, m.htotal], "{} h {:?}", asic.name, m);
            assert_eq!(o.v().unwrap(), [m.vdisplay, m.vsync_start, m.vsync_end, m.vtotal], "{} v {:?}", asic.name, m);
            assert_eq!(o.flags, m.flags & (FLAG_HSYNC_POSITIVE | FLAG_VSYNC_POSITIVE));
        }
    }
}

#[test]
fn nonsense_timing_is_refused() {
    let a = readout::Axis { total_minus_1: 99, blank_start: 10, blank_end: 20, sync_start: 0, sync_end: 5 };
    assert_eq!(a.display_form(), None, "blank end after blank start");
    let a = readout::Axis { total_minus_1: 99, blank_start: 90, blank_end: 10, sync_start: 0, sync_end: 0 };
    assert_eq!(a.display_form(), None, "zero sync");
}

#[test]
fn firmware_scanout_readout() {
    let asic = phoenix();
    let mut f = laptop();
    let mut heads = Vec::new();
    readout::for_each_head(&mut f, &asic, |h| heads.push(h));
    assert_eq!(heads.len(), 1);
    let p = heads[0].hubp.unwrap();
    assert_eq!((p.pipe, p.otg, p.width, p.height, p.pitch_pixels, p.stride_bytes()), (0, 0, 1920, 1080, 2048, 8192));
    assert!(p.linear_xrgb());
    let ap = readout::read_aperture(&mut f, &asic);
    assert_eq!(ap.vram_offset(p.address), Some(0x10_0000), "firmware framebuffer 1 MiB into the carve-out");
    assert_eq!(ap.vram_offset(0x10_0000), None);
    // A disabled OTG reads as off without touching its timing.
    assert!(!readout::read_otg(&mut f, &asic, 1).enabled);
}

#[test]
fn edid_and_dpcd_over_the_aux_engine() {
    let mut f = laptop();
    let mut buf = [0u8; 256];
    assert_eq!(aux::read_edid(&mut f, 0, &mut buf), Ok(128));
    assert_eq!(&buf[..128], &panel_edid()[..]);
    let c = aux::caps(&mut f, 0).unwrap();
    assert_eq!((c.rev, c.max_link_khz, c.max_lanes, c.psr, c.edp_rev), (0x14, 0x1E * 270_000, 4, 3, 4));
    // The engine was handed back after every transaction.
    let arb = f.get(aux(0, AUX_ARB_CONTROL));
    assert_eq!(arb & AUX_SW_USE_AUX_REG_REQ, 0);
    assert_ne!(arb & AUX_SW_DONE_USING_AUX_REG, 0);
    // Writes reach the sink; a deferring sink is retried.
    f.sinks.get_mut(&0).unwrap().defers = 2;
    aux::dpcd_write(&mut f, 0, 0x600, &[1]).unwrap();
    assert_eq!(f.sinks[&0].dpcd[0x600], 1);
}

#[test]
fn aux_errors() {
    let mut f = laptop();
    let mut b = [0u8; 1];
    // No sink on AUX1: hot-plug low.
    assert_eq!(aux::dpcd_read(&mut f, 1, 0, &mut b), Err(Error::Nak));
    // The display microcontroller holds the engine.
    f.dmcu_owns = true;
    assert_eq!(aux::dpcd_read(&mut f, 0, 0, &mut b), Err(Error::Busy));
    f.dmcu_owns = false;
    f.sinks.get_mut(&0).unwrap().defers = 100;
    assert_eq!(aux::dpcd_read(&mut f, 0, 0, &mut b), Err(Error::Defer));
    assert_eq!(aux::dpcd_read(&mut f, 0, 0, &mut [0u8; 17]), Err(Error::BadArgument));
}

#[test]
fn aux_engine_is_enabled_through_reset() {
    let mut f = laptop();
    let mut b = [0u8; 1];
    aux::dpcd_read(&mut f, 0, 0, &mut b).unwrap();
    let ctl: Vec<u32> = f.log.iter().filter(|(o, _)| *o == aux(0, AUX_CONTROL)).map(|(_, v)| *v).collect();
    assert_eq!(ctl, vec![AUX_EN | AUX_RESET, AUX_EN], "enable with reset, then release reset");
    // Already enabled: no second reset.
    aux::dpcd_read(&mut f, 0, 0, &mut b).unwrap();
    assert_eq!(f.log.iter().filter(|(o, _)| *o == aux(0, AUX_CONTROL)).count(), 2);
}

#[test]
fn flips_latch_at_vblank() {
    let asic = phoenix();
    let mut f = laptop();
    plane::flip(&mut f, 0, 0xF4_0090_0000).unwrap();
    let hi = f.log.iter().position(|(o, _)| *o == hubp(0, HUBPREQ_PRIMARY_SURFACE_ADDRESS_HIGH)).unwrap();
    let lo = f.log.iter().position(|(o, _)| *o == hubp(0, HUBPREQ_PRIMARY_SURFACE_ADDRESS)).unwrap();
    assert!(hi < lo, "high half first, the low write arms the flip");
    assert_eq!(f.get(hubp(0, HUBPREQ_FLIP_CONTROL)) & SURFACE_FLIP_TYPE, 0, "vsync flip, not immediate");
    assert!(!plane::flip_done(&mut f, 0));
    plane::wait_vblank(&mut f, &asic, 0, 100).unwrap();
    let mut done = false;
    for _ in 0..5 { if plane::flip_done(&mut f, 0) { done = true; break; } }
    assert!(done);
    assert_eq!(readout::read_hubp(&mut f, 0).address, 0xF4_0090_0000);
    assert_eq!(plane::flip(&mut f, 0, 0x1234), Err(Error::BadArgument), "unaligned");
    assert_eq!(plane::flip(&mut f, 0, 1 << 48), Err(Error::BadArgument), "beyond 48 bits");
}

#[test]
fn kms_view_of_the_takeover() {
    let asic = phoenix();
    let mut f = laptop();
    let heads = readout::read_heads(&mut f, &asic);
    let mut buf = [0u8; 256];
    let n = aux::read_edid(&mut f, 0, &mut buf).unwrap();
    let edid = Edid::parse(&buf[..n]).unwrap();
    let ports = vec![
        readout::Port { kind: ConnectorKind::Edp, otg: Some(0), live: true, edid: Some(edid) },
        readout::Port { kind: ConnectorKind::Hdmi, otg: None, live: false, edid: None },
    ];
    let (dev, st) = readout::to_kms(&asic, &heads, &ports, &[]);
    assert_eq!(dev.crtcs, 4);
    assert_eq!(dev.connectors[0].status, Status::Connected);
    assert_eq!(dev.connectors[1].status, Status::Disconnected);
    assert_eq!(st.connectors[0], Some(0));
    let mode = st.crtcs[0].mode.unwrap();
    assert!(st.crtcs[0].active && mode.same_timing(&panel_mode()), "exact EDID mode, with its clock");
    let fb = st.fbs[0].unwrap();
    assert_eq!((fb.width, fb.height, fb.stride), (1920, 1080, 8192));
    assert_eq!(st.planes[readout::plane_index(0, PlaneKind::Primary) as usize].fb, Some(0));
    // Without an EDID the mode comes from the measured refresh.
    let o = &heads[0].otg;
    let m = readout::running_mode(o, None, readout::refresh_mhz(165, 1_000_000)).unwrap();
    assert_eq!((m.hdisplay, m.vdisplay, m.htotal, m.vtotal), (1920, 1080, 2080, 1101));
    assert!(readout::running_mode(o, None, 0).is_none());
}
