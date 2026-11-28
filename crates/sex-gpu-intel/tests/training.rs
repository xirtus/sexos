// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! DP link training against a simulated sink that needs some swing to lock
//! its clock and some pre-emphasis to equalise.
use sex_gpu_intel::training::{self, dpcd, Config, Failure, Pattern, Source};
use sex_gpu_intel::Error;

struct Sink {
    dpcd: Vec<u8>,
    levels: [(u8, u8); 4],
    pattern: Pattern,
    need_swing: u8,
    need_pre: u8,
    /// Lanes that never lock (a bad cable).
    dead_lanes: u8,
    waited_us: u64,
    writes: Vec<(u32, Vec<u8>)>,
}
impl Sink {
    fn new(need_swing: u8, need_pre: u8) -> Sink {
        Sink { dpcd: vec![0; 0x300], levels: [(0, 0); 4], pattern: Pattern::Off, need_swing, need_pre, dead_lanes: 0, waited_us: 0, writes: vec![] }
    }
    fn update(&mut self) {
        let lanes = (self.dpcd[dpcd::LANE_COUNT_SET as usize] & 0x1f) as usize;
        let tp = self.dpcd[dpcd::TRAINING_PATTERN_SET as usize] & 0xf;
        let (s, p) = self.levels[0];
        let cr = s >= self.need_swing && tp != 0 && self.pattern != Pattern::Off;
        let eq = cr && p >= self.need_pre && tp != 1 && self.pattern != Pattern::Tps1;
        let mut st = [0u8; 2];
        for ln in 0..lanes {
            let dead = self.dead_lanes & (1 << ln) != 0;
            let bits = if dead { 0 } else { (cr as u8) | (eq as u8) * 6 };
            st[ln / 2] |= bits << ((ln % 2) * 4);
        }
        self.dpcd[0x202] = st[0]; self.dpcd[0x203] = st[1];
        self.dpcd[0x204] = eq as u8;
        // Ask for one more step of whatever is missing.
        let want_s = if cr { s } else { (s + 1).min(3) };
        let want_p = if cr && !eq { (p + 1).min(3) } else { p };
        let adj = want_s | want_p << 2;
        self.dpcd[0x206] = adj | adj << 4; self.dpcd[0x207] = adj | adj << 4;
    }
}
impl Source for Sink {
    fn set_pattern(&mut self, p: Pattern) { self.pattern = p; }
    fn set_levels(&mut self, l: &[(u8, u8); 4]) { self.levels = *l; }
    fn dpcd_read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), Error> {
        self.update();
        buf.copy_from_slice(&self.dpcd[addr as usize..addr as usize + buf.len()]);
        Ok(())
    }
    fn dpcd_write(&mut self, addr: u32, data: &[u8]) -> Result<(), Error> {
        self.writes.push((addr, data.to_vec()));
        self.dpcd[addr as usize..addr as usize + data.len()].copy_from_slice(data);
        Ok(())
    }
    fn delay_us(&mut self, us: u32) { self.waited_us += us as u64; }
}

fn hbr2x4() -> Config { Config { rate_khz: 540_000, lanes: 4, rate_index: None, tps3: true, tps4: false } }

#[test]
fn trains_by_raising_swing_then_pre_emphasis() {
    let mut s = Sink::new(2, 1);
    let t = training::train(&mut s, &hbr2x4()).unwrap();
    assert_eq!(t.levels[0], (2, 1));
    assert_eq!(t.pattern, Pattern::Tps3);
    assert_eq!(s.dpcd[dpcd::LINK_BW_SET as usize], 0x14);
    assert_eq!(s.dpcd[dpcd::LANE_COUNT_SET as usize], 0x84);
    assert_eq!(s.dpcd[dpcd::TRAINING_PATTERN_SET as usize], 0x23);
    training::finish(&mut s).unwrap();
    assert_eq!(s.dpcd[dpcd::TRAINING_PATTERN_SET as usize], 0);
    // The lane-set bytes the sink saw carry max flags only at the limits.
    assert_eq!(s.dpcd[0x103], 2 | 1 << 3 | training::MAX_PRE_REACHED);
}

#[test]
fn tps4_is_scrambled_and_edp_rate_index_uses_link_rate_set() {
    let mut s = Sink::new(0, 0);
    let cfg = Config { rate_khz: 432_000, lanes: 2, rate_index: Some(3), tps3: true, tps4: true };
    let t = training::train(&mut s, &cfg).unwrap();
    assert_eq!(t.pattern, Pattern::Tps4);
    assert_eq!(s.dpcd[dpcd::TRAINING_PATTERN_SET as usize], 7);
    assert_eq!(s.dpcd[dpcd::LINK_BW_SET as usize], 0);
    assert_eq!(s.dpcd[dpcd::LINK_RATE_SET as usize], 3);
}

#[test]
fn a_dead_lane_fails_clock_recovery_in_bounded_tries() {
    let mut s = Sink::new(0, 0);
    s.dead_lanes = 1 << 3;
    assert_eq!(training::train(&mut s, &hbr2x4()), Err(Failure::ClockRecovery));
    assert!(s.waited_us < 10_000);
}

#[test]
fn equalisation_gives_up_after_five_tries() {
    let mut s = Sink::new(0, 4); // never satisfied
    assert_eq!(training::train(&mut s, &hbr2x4()), Err(Failure::Equalisation));
}

#[test]
fn combo_phy_levels_program_every_lane_and_latch_last() {
    use sex_gpu_intel::phy;
    use std::collections::HashMap;
    #[derive(Default)]
    struct R(HashMap<u32, u32>, Vec<u32>);
    impl sex_gpu_intel::Mmio for R {
        fn read(&mut self, r: u32) -> u32 { *self.0.get(&r).unwrap_or(&0) }
        fn write(&mut self, r: u32, v: u32) { self.1.push(r); self.0.insert(r, v); }
    }
    let mut r = R::default();
    phy::set_levels(&mut r, 0, 540_000, 4, true, &[(2, 1); 4]);
    let e = phy::DP_HBR2[phy::row(2, 1)];
    for ln in 0..4 {
        assert_eq!(r.0[&phy::tx_ln(0, ln, 7)] >> 24, e.n_scalar as u32);
        assert_eq!(r.0[&phy::tx_ln(0, ln, 4)] & 0x3f, e.cursor as u32);
        assert_eq!(r.0[&phy::tx_ln(0, ln, 4)] & phy::LOADGEN_SELECT != 0, ln >= 1);
    }
    assert_eq!(*r.1.last().unwrap(), phy::tx_grp(0, 5));
    assert!(r.0[&phy::tx_grp(0, 5)] & phy::TX_TRAINING_EN != 0);
    assert_eq!(phy::row(3, 3), 9);
    assert_eq!(phy::row(1, 2), 6);
}

#[test]
fn wm_latency_adjust_and_compute() {
    use sex_gpu_intel::{pcode, watermarks as wm};
    // Level 3 reported 0: it and deeper go off; level 0 = 0 gets +3 µs read latency.
    assert_eq!(pcode::adjust([0, 4, 6, 0, 40, 50, 60, 70]), [3, 7, 9, 0, 0, 0, 0, 0]);
    assert_eq!(pcode::adjust([2, 1, 30, 30, 60, 80, 100, 120]), [2, 2, 30, 30, 60, 80, 100, 120]);
    let lat = [2u16, 4, 12, 22, 40, 52, 70, 100];
    // 1920x1080 at 165 Hz (htotal 2080, 392.64 MHz), 4 bpp.
    let fast = wm::compute(&lat, 10, 392_640, 2080, 1920, 4, 300).unwrap();
    let slow = wm::compute(&lat, 10, 142_770, 2080, 1920, 4, 300).unwrap();
    // 100 µs at 165 Hz = 19 lines of 16 blocks; it fits a whole slice.
    assert_eq!(wm::compute(&lat, 10, 392_640, 2080, 1920, 4, 860).unwrap()[7] & wm::WM_BLOCKS_MASK, 305);
    for l in 0..8 {
        if fast[l] & wm::WM_ENABLE == 0 { assert_eq!(fast[l], 0); continue; }
        assert!(fast[l] & wm::WM_BLOCKS_MASK >= slow[l] & wm::WM_BLOCKS_MASK, "faster needs more, level {l}");
        if l > 0 { assert!(fast[l] & wm::WM_BLOCKS_MASK >= fast[l - 1] & wm::WM_BLOCKS_MASK); }
    }
    assert!(fast.iter().filter(|v| *v & wm::WM_ENABLE != 0).count() < 8, "the deepest levels do not fit a 300-block share");
    // Level 0 too large for a tiny share: refused.
    assert_eq!(wm::compute(&lat, 10, 392_640, 2080, 1920, 4, 20), None);
}
