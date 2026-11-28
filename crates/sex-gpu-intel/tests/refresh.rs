// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Seamless refresh changes: EDID timings without a heap, M/N scaling, and
//! the refusals that keep a running panel lit.
use std::collections::HashMap;
use sex_gpu_intel::refresh::{self, Boot, Refusal, MN};
use sex_gpu_intel::watermarks::{self as wm, Which};
use sex_gpu_intel::regs::*;
use sex_gpu_intel::timing::{self, Timing};
use sex_gpu_intel::Mmio;

#[derive(Default)]
struct Regs(HashMap<u32, u32>, Vec<(u32, u32)>);
impl Mmio for Regs {
    fn read(&mut self, r: u32) -> u32 {
        let v = *self.0.get(&r).unwrap_or(&0);
        // The frame counter ticks every few reads, like a running pipe.
        if r == pipe(0, FRMCOUNT) { self.0.insert(r, v + 1); }
        v
    }
    fn write(&mut self, r: u32, v: u32) { self.1.push((r, v)); self.0.insert(r, v); }
}

/// An 18-byte DTD (digital separate sync, +h +v).
fn dtd(clock_khz: u32, ha: u16, hb: u16, hso: u16, hsw: u16, va: u16, vb: u16, vso: u16, vsw: u16) -> [u8; 18] {
    let c = (clock_khz / 10) as u16;
    [c as u8, (c >> 8) as u8, ha as u8, hb as u8, ((ha >> 8) as u8) << 4 | (hb >> 8) as u8,
     va as u8, vb as u8, ((va >> 8) as u8) << 4 | (vb >> 8) as u8,
     hso as u8, hsw as u8, ((vso & 0xf) as u8) << 4 | (vsw & 0xf) as u8,
     (((hso >> 8) & 3) as u8) << 6 | (((hsw >> 8) & 3) as u8) << 4 | (((vso >> 4) & 3) as u8) << 2 | ((vsw >> 4) & 3) as u8,
     0, 0, 0, 0, 0, 0x1e]
}

/// A 165 Hz laptop panel EDID: 1920x1080 at 165 and at 60 on the same raster.
fn panel() -> Vec<u8> {
    let mut e = vec![0u8; 128];
    e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    // total 2080 x 1144: 165 Hz = 392.6 MHz, 60 Hz = 142.8 MHz
    e[54..72].copy_from_slice(&dtd(392_640, 1920, 160, 48, 32, 1080, 64, 3, 5));
    e[72..90].copy_from_slice(&dtd(142_770, 1920, 160, 48, 32, 1080, 64, 3, 5));
    e[90..95].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
    e
}

fn lit(t: &Timing) -> Regs {
    let mut r = Regs::default();
    let w = |a: u16, b: u16| ((b as u32 - 1) << 16) | (a as u32 - 1);
    r.0.insert(pipe(0, CONF), CONF_ENABLE | CONF_STATE);
    r.0.insert(trans(0, DDI_FUNC_CTL), FUNC_ENABLE | 1 << FUNC_PORT_SHIFT | FUNC_MODE_DP_SST << FUNC_MODE_SHIFT | FUNC_PHSYNC | FUNC_PVSYNC);
    r.0.insert(trans(0, HTOTAL), w(t.h[0], t.h[3])); r.0.insert(trans(0, HSYNC), w(t.h[1], t.h[2]));
    r.0.insert(trans(0, VTOTAL), w(t.v[0], t.v[3])); r.0.insert(trans(0, VSYNC), w(t.v[1], t.v[2]));
    // HBR2 x4 at 8 bpc: data M/N = 392620*24 / (540000*4*8) ≈ 0.545, N fixed.
    r.0.insert(trans(0, DATA_M1), 63 << 25 | 0x45ca78);
    r.0.insert(trans(0, DATA_N1), 0x800000);
    r.0.insert(trans(0, LINK_M1), 0x5d0ce);
    r.0.insert(trans(0, LINK_N1), 0x80000);
    r.0.insert(CDCLK_CTL, (652_800 - 1000) / 500);
    r
}

#[test]
fn edid_timings_decode_both_rates_on_one_raster() {
    let mut out = [Timing::default(); 8];
    let n = timing::edid_timings(&panel(), &mut out);
    assert_eq!(n, 2);
    assert_eq!(out[0].h, [1920, 1968, 2000, 2080]);
    assert_eq!(out[0].v, [1080, 1083, 1088, 1144]);
    assert_eq!(out[0].flags, 3);
    assert_eq!(out[0].refresh_mhz() / 1000, 165);
    assert_eq!(out[1].refresh_mhz() / 1000, 59);
    assert!(out[0].same_raster(&out[1]));
}

#[test]
fn m_n_scales_with_the_pixel_clock_and_keeps_n() {
    let mn = MN { tu: 64, data_m: 0x45ca78, data_n: 0x800000, link_m: 0x5d0ce, link_n: 0x80000 };
    let down = mn.scaled(392_640, 142_770).unwrap();
    assert_eq!((down.data_n, down.link_n, down.tu), (mn.data_n, mn.link_n, 64));
    assert!((down.data_m as i64 - (0x45ca78i64 * 142_770 / 392_640)).abs() <= 1);
    // Twice the rate would need 109 % of the link: refused.
    assert_eq!(mn.scaled(392_640, 785_240), None);
}

#[test]
fn down_switch_writes_m_n_only_and_up_returns() {
    let mut out = [Timing::default(); 8];
    timing::edid_timings(&panel(), &mut out);
    let (fast, slow) = (out[0], out[1]);
    let mut r = lit(&fast);
    let before = refresh::read(&mut r, 0);
    assert_eq!(before.tu, 64);
    let boot = Boot::read(&mut r, 0, fast.clock_khz, 1920);
    let p = refresh::plan(&mut r, 0, &fast, &slow, &boot).unwrap();
    assert_eq!(p.wm, None);
    refresh::apply(&mut r, &p, 16).unwrap();
    // Exactly the four M/N registers, nothing that could blank the pipe.
    let written: Vec<u32> = r.1.iter().map(|w| w.0).collect();
    assert_eq!(written, vec![trans(0, DATA_M1), trans(0, DATA_N1), trans(0, LINK_M1), trans(0, LINK_N1)]);
    assert_eq!(refresh::read(&mut r, 0), p.after);
    // And back up to firmware's rate restores firmware's values (within rounding).
    let back = refresh::plan(&mut r, 0, &slow, &fast, &boot).unwrap();
    assert_eq!(back.wm, None, "back to firmware's own rate keeps its watermarks");
    assert!((back.after.data_m as i64 - before.data_m as i64).abs() <= 2);
}

#[test]
fn refusals_leave_the_pipe_alone() {
    let mut out = [Timing::default(); 8];
    timing::edid_timings(&panel(), &mut out);
    let (fast, slow) = (out[0], out[1]);
    let mut r = lit(&slow);
    let boot = Boot::read(&mut r, 0, slow.clock_khz, 1920);
    // A 60 Hz boot whose 8 MiB-class link M/N leaves no room: 165 Hz is refused.
    r.0.insert(trans(0, DATA_M1), 63 << 25 | 0x400000);
    assert_eq!(refresh::plan(&mut r, 0, &slow, &fast, &boot), Err(Refusal::Link));
    let other = Timing { v: [1080, 1083, 1088, 1120], ..slow };
    assert_eq!(refresh::plan(&mut r, 0, &slow, &other, &boot), Err(Refusal::Raster));
    r.0.insert(CDCLK_CTL, (172_800 - 1000) / 500);
    assert_eq!(refresh::plan(&mut r, 0, &fast, &fast, &boot), Err(Refusal::Cdclk));
    r.0.insert(trans(0, DDI_FUNC_CTL), FUNC_ENABLE | FUNC_MODE_HDMI << FUNC_MODE_SHIFT);
    assert_eq!(refresh::plan(&mut r, 0, &slow, &slow, &boot), Err(Refusal::NotDp));
    assert!(r.1.is_empty());
}

#[test]
fn m_n_write_waits_for_the_first_half_of_the_frame() {
    let mut out = [Timing::default(); 8];
    timing::edid_timings(&panel(), &mut out);
    let mut r = lit(&out[0]);
    r.0.insert(pipe(0, PIPEDSL), 1000);
    let boot = Boot::read(&mut r, 0, out[0].clock_khz, 1920);
    let p = refresh::plan(&mut r, 0, &out[0], &out[1], &boot).unwrap();
    assert!(matches!(refresh::apply(&mut r, &p, 8), Err(Refusal::Device(_))));
    assert!(r.1.is_empty());
}

/// Firmware's 60 Hz watermarks for a 1920-wide linear plane with 860 DBUF blocks.
fn gop_60hz_wm(r: &mut Regs) {
    let lvl = |lines: u32, blocks: u32| wm::WM_ENABLE | lines << wm::WM_LINES_SHIFT | blocks;
    let primary = [lvl(1, 16), lvl(2, 31), lvl(3, 46), lvl(4, 61), lvl(8, 121), lvl(10, 151), lvl(16, 241), lvl(24, 361)];
    for (i, v) in primary.iter().enumerate() { r.0.insert(wm::plane_wm(0, 1, i as u32), *v); }
    r.0.insert(wm::plane_wm_trans(0, 1), lvl(0, 40));
    r.0.insert(wm::plane_buf_cfg(0, 1), 891 << wm::BUF_END_SHIFT | 32);
    for i in 0..8 { r.0.insert(wm::cur_wm(0, i), lvl(1 + i, 4 + 4 * i)); }
    r.0.insert(wm::cur_buf_cfg(0), 31 << wm::BUF_END_SHIFT);
}

#[test]
fn faster_than_firmware_latches_scaled_watermarks_first() {
    let mut out = [Timing::default(); 8];
    timing::edid_timings(&panel(), &mut out);
    let (fast, slow) = (out[0], out[1]);
    let mut r = lit(&slow);
    // 60 Hz on HBR2 x4: plenty of link left for 165 Hz.
    r.0.insert(trans(0, DATA_M1), 63 << 25 | 0x196e4f);
    r.0.insert(trans(0, LINK_M1), 0x21d5c);
    gop_60hz_wm(&mut r);
    let boot = Boot::read(&mut r, 0, slow.clock_khz, 1920);
    let p = refresh::plan(&mut r, 0, &slow, &fast, &boot).unwrap();
    let (primary, cursor) = p.wm.expect("faster than boot needs deeper watermarks");
    // Every enabled level grows by at least the rate ratio (2.75x).
    for i in 0..wm::LEVELS {
        let (b, a) = (boot.primary.levels[i], primary.levels[i]);
        if a & wm::WM_ENABLE == 0 { continue; }
        assert!((a & wm::WM_BLOCKS_MASK) * 100 >= (b & wm::WM_BLOCKS_MASK) * 275, "level {i}");
    }
    assert!(primary.enabled_levels() >= 4 && primary.enabled_levels() < 8, "deep levels that no longer fit 860 blocks turn off");
    assert!(primary.levels.iter().skip(primary.enabled_levels()).all(|l| l & wm::WM_ENABLE == 0), "monotonic");
    assert!(cursor.levels[0] & wm::WM_ENABLE != 0);
    // Cursor's 32 blocks fit only the shallow levels.
    assert!(cursor.enabled_levels() < 8);
    r.1.clear();
    refresh::apply(&mut r, &p, 64).unwrap();
    let order: Vec<u32> = r.1.iter().map(|w| w.0).collect();
    let surf = order.iter().position(|&a| a == plane(0, 1, PLANE_SURF)).unwrap();
    let mn = order.iter().position(|&a| a == trans(0, DATA_M1)).unwrap();
    assert!(order[..surf].contains(&wm::plane_wm(0, 1, 0)), "watermarks written before the arming SURF write");
    assert!(surf < mn, "M/N only after the watermarks were armed");
    assert_eq!(wm::read(&mut r, 0, Which::Plane(1)).levels, primary.levels);
}

#[test]
fn level_zero_that_cannot_fit_refuses() {
    let mut out = [Timing::default(); 8];
    timing::edid_timings(&panel(), &mut out);
    let (fast, slow) = (out[0], out[1]);
    let mut r = lit(&slow);
    r.0.insert(trans(0, DATA_M1), 63 << 25 | 0x196e4f);
    gop_60hz_wm(&mut r);
    r.0.insert(wm::plane_buf_cfg(0, 1), 60 << wm::BUF_END_SHIFT | 32);
    let boot = Boot::read(&mut r, 0, slow.clock_khz, 1920);
    assert_eq!(refresh::plan(&mut r, 0, &slow, &fast, &boot), Err(Refusal::Watermarks));
}

#[test]
fn combo_pll_table_round_trips_every_dp_rate() {
    use sex_gpu_intel::dpll;
    for refk in [19_200, 24_000, 38_400] {
        for &rate in &dpll::DP_RATES {
            let p = dpll::dp_params(refk, rate).unwrap();
            let (c0, c1) = dpll::encode(refk, &p);
            let got = dpll::port_clock_khz(refk, c0, c1);
            assert!(got.abs_diff(rate) <= rate / 1000, "ref {refk} rate {rate} got {got}");
        }
    }
    assert_eq!(dpll::dp_params(38_400, 123_000), None);
}

#[test]
fn combo_pll_readout_and_enable_sequence() {
    use sex_gpu_intel::dpll;
    let mut r = Regs::default();
    r.0.insert(dpll::DSSM, 2 << 29);
    let (c0, c1) = dpll::encode(38_400, &dpll::dp_params(38_400, 540_000).unwrap());
    // Firmware: DPLL0 locked at HBR2, routed to DDI A; DDI B gated.
    r.0.insert(dpll::enable_reg(0), dpll::PLL_ENABLE | dpll::PLL_LOCK | dpll::PLL_POWER_ENABLE | dpll::PLL_POWER_STATE);
    r.0.insert(dpll::cfgcr0(0), c0); r.0.insert(dpll::cfgcr1(0), c1);
    r.0.insert(dpll::DPCLKA_CFGCR0, 1 << 11);
    assert_eq!(dpll::port_pll(&mut r, 0), Some(0));
    assert_eq!(dpll::port_pll(&mut r, 1), None);
    assert!(dpll::port_link_khz(&mut r, 0).unwrap().abs_diff(540_000) < 540);
    // Enabling DPLL1 for DDI B: power, dividers, enable; a fake that never
    // reports power times out instead of hanging.
    assert!(dpll::enable(&mut r, 1, c0, c1, 8).is_err());
    r.0.insert(dpll::enable_reg(1), dpll::PLL_POWER_STATE | dpll::PLL_LOCK);
    dpll::enable(&mut r, 1, c0, c1, 8).unwrap();
    dpll::map_port(&mut r, 1, 1).unwrap();
    assert_eq!(dpll::port_pll(&mut r, 1), Some(1));
}
