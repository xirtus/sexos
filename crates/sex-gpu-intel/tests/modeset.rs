// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A whole DP modeset on combo port A against simulated hardware: a PLL that
//! powers and locks, a DDI buffer that leaves idle, a pipe that reports its
//! state, and a sink behind the AUX channel that trains once it sees swing 1.
use std::collections::HashMap;
use sex_gpu_intel::modeset::{self, Link, Plan, Step};
use sex_gpu_intel::regs::*;
use sex_gpu_intel::timing::Timing;
use sex_gpu_intel::{aux, dpll, phy, refresh, Mmio};

#[derive(Default)]
struct Hw { r: HashMap<u32, u32>, dpcd: Vec<u8>, log: Vec<u32>, dead_pll: bool }
impl Hw {
    fn get(&self, a: u32) -> u32 { *self.r.get(&a).unwrap_or(&0) }
    fn sink_status(&mut self) {
        let lanes = (self.dpcd[0x101] & 0x1f) as usize;
        let tp = self.dpcd[0x102] & 0xf;
        // Lane 0's N scalar tells the levels apart: 0x35 is swing 0 / pre 0 at HBR2.
        let n_scalar = self.get(phy::tx_ln(0, 0, 7)) >> 24;
        let pat = self.get(modeset::dp_tp_ctl(0)) & modeset::TP_TRAIN_MASK;
        let cr = tp != 0 && n_scalar != 0x35;
        let eq = cr && tp != 1 && pat != modeset::TP_PAT1;
        let mut st = [0u8; 2];
        for ln in 0..lanes { st[ln / 2] |= ((cr as u8) | (eq as u8) * 6) << ((ln % 2) * 4); }
        self.dpcd[0x202] = st[0]; self.dpcd[0x203] = st[1]; self.dpcd[0x204] = eq as u8;
        let adj: u8 = if cr { 1 } else { 1 };
        self.dpcd[0x206] = adj | adj << 4; self.dpcd[0x207] = adj | adj << 4;
    }
    fn aux(&mut self, ctl: u32) {
        let n = ((ctl & AUX_SIZE_MASK) >> AUX_SIZE_SHIFT) as usize;
        let mut msg = [0u8; 20];
        for i in 0..5 { aux::unpack(self.get(aux_data(0, i)), &mut msg[i as usize * 4..i as usize * 4 + 4]); }
        let cmd = msg[0] >> 4;
        let addr = ((msg[0] & 0xf) as usize) << 16 | (msg[1] as usize) << 8 | msg[2] as usize;
        let len = msg[3] as usize + 1;
        let mut reply = vec![0u8];
        if cmd == aux::NATIVE_READ { self.sink_status(); reply.extend_from_slice(&self.dpcd[addr..addr + len]); }
        else if cmd == aux::NATIVE_WRITE && n > 4 { self.dpcd[addr..addr + len].copy_from_slice(&msg[4..4 + len]); }
        for (i, c) in reply.chunks(4).enumerate() { self.r.insert(aux_data(0, i as u32), aux::pack(c)); }
        self.r.insert(aux_ctl(0), AUX_DONE | (reply.len() as u32) << AUX_SIZE_SHIFT);
    }
}
impl Mmio for Hw {
    fn read(&mut self, a: u32) -> u32 { self.get(a) }
    fn write(&mut self, a: u32, v: u32) {
        self.log.push(a);
        if a == aux_ctl(0) && v & AUX_SEND_BUSY != 0 { return self.aux(v); }
        let mut v = v;
        if a == dpll::enable_reg(0) && !self.dead_pll {
            if v & dpll::PLL_POWER_ENABLE != 0 { v |= dpll::PLL_POWER_STATE; } else { v &= !dpll::PLL_POWER_STATE; }
            if v & dpll::PLL_ENABLE != 0 { v |= dpll::PLL_LOCK; } else { v &= !dpll::PLL_LOCK; }
        }
        if a == ddi_buf_ctl(0) { if v & DDI_BUF_ENABLE != 0 { v &= !DDI_BUF_IDLE; } else { v |= DDI_BUF_IDLE; } }
        if a == pipe(0, CONF) { if v & CONF_ENABLE != 0 { v |= CONF_STATE; } else { v &= !CONF_STATE; } }
        self.r.insert(a, v);
    }
}

fn hw() -> Hw {
    let mut h = Hw { dpcd: vec![0; 0x800], ..Hw::default() };
    h.r.insert(dpll::DSSM, 2 << 29);
    h.r.insert(ddi_buf_ctl(0), DDI_BUF_IDLE);
    h.r.insert(dpll::DPCLKA_CFGCR0, 3 << 10);
    h.r.insert(phy::tx_ln(0, 0, 5), 0);
    h
}

fn mode_1080p165() -> Timing { Timing { clock_khz: 392_640, h: [1920, 1968, 2000, 2080], v: [1080, 1083, 1088, 1144], flags: 3 } }

#[test]
fn link_choice_and_m_n() {
    let rates = [162_000, 270_000, 540_000, 810_000];
    // 392.64 MHz x 24 bpp = 9.42 Gbit/s of data: HBR2 x 4 is not the cheapest that fits... HBR3 x2 is.
    let l = modeset::choose_link(392_640, 24, &rates, 4, 1).unwrap();
    assert_eq!(l, Link { rate_khz: 810_000, lanes: 2 });
    // A panel wired for 4 lanes keeps them: HBR2 x4.
    assert_eq!(modeset::choose_link(392_640, 24, &rates, 4, 4).unwrap(), Link { rate_khz: 540_000, lanes: 4 });
    assert_eq!(modeset::choose_link(2_000_000, 24, &rates, 4, 1), None);
    let mn = modeset::mn(392_640, 24, &Link { rate_khz: 540_000, lanes: 4 });
    assert!(mn.link_use_bp() < 5_500 && mn.link_use_bp() > 5_400);
    assert_eq!(refresh::pixel_khz(&mn, 540_000), 392_639);
}

#[test]
fn enable_trains_and_lights_the_pipe_in_prm_order() {
    let mut h = hw();
    let plan = Plan { pipe: 0, port: 0, pll: 0, timing: mode_1080p165(), link: Link { rate_khz: 540_000, lanes: 4 }, edp: true, tps3: true, tps4: false, rate_index: None };
    let mut waited = 0u32;
    let t = modeset::enable(&mut h, &plan, 64, |us| waited += us).unwrap();
    assert!(t.levels[0].0 >= 1, "trained at swing {:?}", t.levels[0]);
    assert!(h.get(pipe(0, CONF)) & CONF_STATE != 0);
    assert_eq!(h.get(modeset::dp_tp_ctl(0)) & modeset::TP_TRAIN_MASK, modeset::TP_NORMAL);
    assert_eq!(h.dpcd[0x102], 0, "sink out of training");
    assert_eq!(h.dpcd[0x100], 0x14);
    assert_eq!(dpll::port_pll(&mut h, 0), Some(0));
    assert!(dpll::port_link_khz(&mut h, 0).unwrap().abs_diff(540_000) < 540);
    let pos = |h: &Hw, a: u32| h.log.iter().position(|&x| x == a).unwrap();
    assert!(pos(&h, dpll::cfgcr0(0)) < pos(&h, modeset::trans_clk_sel(0)));
    assert!(pos(&h, modeset::trans_clk_sel(0)) < pos(&h, ddi_buf_ctl(0)));
    assert!(pos(&h, ddi_buf_ctl(0)) < pos(&h, trans(0, HTOTAL)));
    assert!(pos(&h, trans(0, HTOTAL)) < pos(&h, pipe(0, CONF)));
    assert_eq!(h.get(trans(0, HTOTAL)), (2079 << 16) | 1919);
    assert_eq!(h.get(trans(0, PIPESRC)), (1919 << 16) | 1079);
    let r = sex_gpu_intel::readout::read_pipe(&mut h, 0);
    assert_eq!((r.h, r.v, r.port), ([1920, 1968, 2000, 2080], [1080, 1083, 1088, 1144], Some(0)));
    // And off again, leaving the eDP panel powered.
    modeset::disable(&mut h, 0, 0, Some(0), true, 64).unwrap();
    assert_eq!(h.get(pipe(0, CONF)) & CONF_STATE, 0);
    assert!(h.get(ddi_buf_ctl(0)) & DDI_BUF_IDLE != 0);
    assert_eq!(dpll::port_pll(&mut h, 0), None);
    assert_eq!(h.dpcd[0x600], 1, "panel kept awake");
}

#[test]
fn a_pll_that_never_locks_stops_before_the_port() {
    let mut h = hw();
    h.dead_pll = true;
    let plan = Plan { pipe: 0, port: 0, pll: 0, timing: mode_1080p165(), link: Link { rate_khz: 540_000, lanes: 4 }, edp: true, tps3: false, tps4: false, rate_index: None };
    assert_eq!(modeset::enable(&mut h, &plan, 16, |_| {}), Err(Step::Pll));
    assert!(!h.log.contains(&ddi_buf_ctl(0)));
}
