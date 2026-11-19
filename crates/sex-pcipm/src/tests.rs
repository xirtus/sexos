// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
extern crate std;
use std::collections::BTreeMap;
use std::vec;
use std::vec::Vec;

use crate::aspm::{self, Policy, Skip};
use crate::{pm, reg, Bdf, Config};

const PM_AT: u16 = 0x40;
const EXP_AT: u16 = 0x50;
const MSI_AT: u16 = 0x90;
const L1SS_AT: u16 = 0x100;
const LTR_AT: u16 = 0x110;

#[derive(Default)]
struct Sim { space: BTreeMap<Bdf, Vec<u32>>, delayed_us: u64, soft_reset: Vec<Bdf>, pmcsr_written: u32 }

impl Config for Sim {
    fn read(&mut self, f: Bdf, at: u16) -> u32 {
        self.space.get(&f).map_or(u32::MAX, |s| s[(at / 4) as usize])
    }
    fn write(&mut self, f: Bdf, at: u16, value: u32) {
        let reset = self.soft_reset.contains(&f);
        let Some(s) = self.space.get_mut(&f) else { return };
        let s: &mut Vec<u32> = s;
        let i = (at / 4) as usize;
        if at == PM_AT + 4 {
            self.pmcsr_written = value;
            let (from, to) = (s[i] & 3, value & 3);
            s[i] = (s[i] & !3) | to;
            // Coming out of D3hot without No_Soft_Reset: the function forgets its setup.
            if from == 3 && to == 0 && reset {
                for w in &mut s[1..16] { if *w != 0 { *w &= 0x00FF_0000; } }
                s[(MSI_AT / 4) as usize] &= 0xFFFF;
                s[(MSI_AT / 4 + 1) as usize] = 0;
                s[(MSI_AT / 4 + 2) as usize] = 0;
                s[((EXP_AT + reg::EXP_LNKCTL) / 4) as usize] = 0;
            }
            return;
        }
        if at == MSI_AT {
            // Capability id and next pointer are read-only.
            s[i] = (s[i] & 0xFFFF) | (value & 0xFFFF_0000);
            return;
        }
        if at == reg::COMMAND {
            // Status half is RW1C; the model keeps the capability-list bit.
            s[i] = (s[i] & 0xFFFF_0000) | (value & 0xFFFF);
            return;
        }
        if at >= EXP_AT && at < EXP_AT + 0x3C && (at - EXP_AT) % 8 == 0 && at != EXP_AT {
            // DEVCTL/LNKCTL/DEVCTL2/LNKCTL2 share their dword with a status half.
            s[i] = (s[i] & 0xFFFF_0000) | (value & 0xFFFF);
            return;
        }
        s[i] = value;
    }
    fn delay_us(&mut self, us: u32) { self.delayed_us += us as u64; }
}

#[derive(Clone, Copy)]
struct Fun {
    id: u32,
    class: u32,
    kind: u8,
    secondary: Option<u8>,
    devcap: u32,
    lnkcap: u32,
    devcap2: u32,
    l1ss: Option<u32>,
}
/// L1 exit 4-8 us (enc 3), L0s exit 512 ns-1 us (enc 3), clock PM, both ASPM states.
const LNKCAP: u32 = reg::LNKCAP_ASPM_L0S | reg::LNKCAP_ASPM_L1 | reg::LNKCAP_CLKPM | 3 << 12 | 3 << 15;
/// RBER, acceptable L0s 4 us (enc 6), acceptable L1 64 us (enc 6).
const DEVCAP: u32 = reg::DEVCAP_RBER | 6 << 6 | 6 << 9;
/// Every substate, Common_Mode_Restore 10 us, T_POWER_ON 6 x 10 us = 60 us.
const L1SS_ALL: u32 = 0x1F | 10 << 8 | 1 << 16 | 6 << 19;

impl Fun {
    fn port(kind: u8, secondary: u8) -> Fun {
        Fun { id: 0x43b8_8086, class: 0x0604_00, kind, secondary: Some(secondary), devcap: DEVCAP, lnkcap: LNKCAP,
            devcap2: reg::DEVCAP2_LTR, l1ss: Some(L1SS_ALL) }
    }
    fn nvme() -> Fun {
        Fun { id: 0xa809_144d, class: 0x0108_02, kind: reg::TYPE_ENDPOINT, secondary: None, devcap: DEVCAP, lnkcap: LNKCAP,
            devcap2: reg::DEVCAP2_LTR, l1ss: Some(0x1F | 40 << 8 | 0 << 16 | 5 << 19) }
    }
}

impl Sim {
    fn add(&mut self, b: Bdf, f: Fun) {
        let mut s = vec![0u32; 1024];
        s[0] = f.id;
        s[1] = reg::STATUS_CAP_LIST;
        s[2] = f.class << 8;
        if let Some(sec) = f.secondary { s[3] = 1 << 16; s[6] = (sec as u32) << 8 | (sec as u32) << 16 | b.bus as u32; }
        s[(reg::CAP_PTR / 4) as usize] = PM_AT as u32;
        s[(PM_AT / 4) as usize] = reg::CAP_PM as u32 | (EXP_AT as u32) << 8;
        s[(EXP_AT / 4) as usize] = reg::CAP_EXP as u32 | (MSI_AT as u32) << 8 | 2 << 16 | (f.kind as u32) << 20;
        s[((EXP_AT + reg::EXP_DEVCAP) / 4) as usize] = f.devcap;
        s[((EXP_AT + reg::EXP_LNKCAP) / 4) as usize] = f.lnkcap;
        s[((EXP_AT + reg::EXP_DEVCAP2) / 4) as usize] = f.devcap2;
        // 64-bit MSI with per-vector masking.
        s[(MSI_AT / 4) as usize] = reg::CAP_MSI as u32 | (1 << 7 | 1 << 8) << 16;
        if let Some(cap) = f.l1ss {
            s[(L1SS_AT / 4) as usize] = reg::EXT_L1SS as u32 | 1 << 16 | (LTR_AT as u32) << 20;
            s[((L1SS_AT + 4) / 4) as usize] = cap;
            s[(LTR_AT / 4) as usize] = reg::EXT_LTR as u32 | 1 << 16;
        }
        self.space.insert(b, s);
    }
    fn word(&self, b: Bdf, at: u16) -> u32 { self.space[&b][(at / 4) as usize] }
    fn lnkctl(&self, b: Bdf) -> u32 { self.word(b, EXP_AT + reg::EXP_LNKCTL) & 0xFFFF }
    fn ltr_on(&self, b: Bdf) -> bool { self.word(b, EXP_AT + reg::EXP_DEVCTL2) & reg::DEVCTL2_LTR != 0 }
    fn l1ss_ctl1(&self, b: Bdf) -> u32 { self.word(b, L1SS_AT + reg::L1SS_CTL1) }
    fn functions(&self) -> Vec<Bdf> { self.space.keys().copied().collect() }
}

fn run(s: &mut Sim, policy: Policy, forbidden: bool) -> aspm::Report {
    let functions = s.functions();
    aspm::configure(s, &functions, policy, forbidden)
}

const RP: Bdf = Bdf::new(0, 0x1d, 0);
const SSD: Bdf = Bdf::new(1, 0, 0);

fn laptop() -> Sim {
    let mut s = Sim::default();
    s.add(RP, Fun::port(reg::TYPE_ROOT_PORT, 1));
    s.add(SSD, Fun::nvme());
    s
}

#[test]
fn root_port_and_ssd_get_every_state() {
    let mut s = laptop();
    let r = run(&mut s, Policy::POWERSAVE, false);
    assert_eq!(r.count, 1);
    let l = r.iter().next().unwrap();
    assert_eq!((l.port, l.children()), (RP, &[SSD][..]));
    assert_eq!(l.before, 0);
    assert_eq!(l.after, aspm::ALL);
    assert_eq!(s.lnkctl(RP) & 3, 3);
    assert_eq!(s.lnkctl(SSD) & 3, 3);
    assert_ne!(s.lnkctl(SSD) & reg::LNKCTL_CLKPM, 0);
    assert!(s.ltr_on(RP) && s.ltr_on(SSD));
    assert_eq!((r.ltr_enabled, r.ltr_already), (2, 0));
    assert_eq!(s.l1ss_ctl1(RP) & 0xF, 0xF);
    assert_eq!(s.l1ss_ctl1(SSD) & 0xF, 0xF);
    // T_POWER_ON: port 60 us beats the SSD's 5 x 2 us; common mode: SSD's 40 us.
    // Threshold 2 + 4 + 40 + 60 = 106 us = 106000 ns -> scale 2 (1024 ns units), value 104 (rounded up).
    assert_eq!(l.threshold_us, 106);
    assert_eq!(s.word(RP, L1SS_AT + reg::L1SS_CTL2), 1 | 6 << 3);
    assert_eq!(s.word(SSD, L1SS_AT + reg::L1SS_CTL2), 1 | 6 << 3);
    assert_eq!(s.l1ss_ctl1(RP) >> 8 & 0xFF, 40, "common mode restore in the upstream port");
    assert_eq!(s.l1ss_ctl1(SSD) >> 8 & 0xFF, 0, "and only there");
    for b in [RP, SSD] {
        assert_eq!(s.l1ss_ctl1(b) >> 16 & 0x3FF, 104);
        assert_eq!(s.l1ss_ctl1(b) >> 29, 2);
    }
}

#[test]
fn second_pass_changes_nothing() {
    let mut s = laptop();
    run(&mut s, Policy::POWERSAVE, false);
    let snapshot = s.space.clone();
    let r = run(&mut s, Policy::POWERSAVE, false);
    assert_eq!(s.space, snapshot);
    let l = r.iter().next().unwrap();
    assert_eq!(l.before, l.after);
    assert_eq!((r.ltr_enabled, r.ltr_already), (0, 2));
}

#[test]
fn fadt_flag_and_firmware_policy_touch_nothing() {
    for (policy, forbidden) in [(Policy::POWERSAVE, true), (Policy::FIRMWARE, false)] {
        let mut s = laptop();
        let snapshot = s.space.clone();
        let r = run(&mut s, policy, forbidden);
        assert_eq!(s.space, snapshot);
        if forbidden { assert_eq!(r.iter().next().unwrap().skip, Some(Skip::Forbidden)); }
    }
}

#[test]
fn endpoint_that_cannot_wait_keeps_l1_off() {
    let mut s = laptop();
    // Acceptable L1 latency 1 us; the link needs 8 us. Acceptable L0s 64 ns; needs 1 us.
    let at = EXP_AT + reg::EXP_DEVCAP;
    s.space.get_mut(&SSD).unwrap()[(at / 4) as usize] = reg::DEVCAP_RBER;
    let r = run(&mut s, Policy::POWERSAVE, false);
    let l = r.iter().next().unwrap();
    assert_eq!(l.after, aspm::CLKPM);
    assert_eq!(s.lnkctl(RP) & 3, 0);
    assert_eq!(s.l1ss_ctl1(SSD) & 0xF, 0);
}

#[test]
fn firmware_states_stay_and_firmware_substate_timing_is_kept() {
    let mut s = laptop();
    let ctl1 = EXP_AT + reg::EXP_LNKCTL;
    for b in [RP, SSD] {
        s.space.get_mut(&b).unwrap()[(ctl1 / 4) as usize] = reg::LNKCTL_ASPM_L1;
        s.space.get_mut(&b).unwrap()[((L1SS_AT + reg::L1SS_CTL1) / 4) as usize] = 0x0A | 7 << 16 | 2 << 29;
    }
    let r = run(&mut s, Policy::POWERSAVE, false);
    let l = r.iter().next().unwrap();
    assert_eq!(l.before, aspm::L1 | aspm::ASPM_L1_1 | aspm::PCIPM_L1_1);
    assert_eq!(s.l1ss_ctl1(RP), 0x0A | 7 << 16 | 2 << 29, "firmware's substate setup untouched");
    assert_eq!(l.after & aspm::L1SS, l.before & aspm::L1SS);
    assert_eq!(s.lnkctl(SSD) & 3, 3, "L0s added");
}

#[test]
fn each_end_keeps_its_own_firmware_bits() {
    // Firmware: L0s+L1 on the port, L1 only on the SSD. Adding clock PM must not drop
    // the port's L0s (the intersection of both ends is only L1).
    let mut s = laptop();
    let at = EXP_AT + reg::EXP_LNKCTL;
    s.space.get_mut(&RP).unwrap()[(at / 4) as usize] = 3;
    s.space.get_mut(&SSD).unwrap()[(at / 4) as usize] = reg::LNKCTL_ASPM_L1;
    // The SSD cannot take L0s, so L0s is not added on the link.
    s.space.get_mut(&SSD).unwrap()[((EXP_AT + reg::EXP_DEVCAP) / 4) as usize] = reg::DEVCAP_RBER | 6 << 9;
    let r = run(&mut s, Policy::POWERSAVE, false);
    let l = r.iter().next().unwrap();
    assert_eq!(l.before & (aspm::L0S | aspm::L1), aspm::L1);
    assert_ne!(l.after & aspm::CLKPM, 0);
    assert_eq!(s.lnkctl(RP) & 3, 3, "the port's firmware L0s stays");
    assert_eq!(s.lnkctl(SSD) & 3, 2);
}

#[test]
fn realtek_nic_gets_l1_and_l1_1_only() {
    let mut s = Sim::default();
    s.add(RP, Fun::port(reg::TYPE_ROOT_PORT, 1));
    s.add(SSD, Fun { id: 0x3000_10ec, class: 0x0200_00, ..Fun::nvme() });
    let r = run(&mut s, Policy::POWERSAVE, false);
    let l = r.iter().next().unwrap();
    assert_eq!(l.after, aspm::L1 | aspm::ASPM_L1_1 | aspm::PCIPM_L1_1 | aspm::CLKPM);
    assert_eq!(l.threshold_us, 0, "no L1.2, no threshold");
    assert_eq!(s.l1ss_ctl1(SSD) & 0xF, (reg::L1SS_ASPM_L1_1 | reg::L1SS_PCIPM_L1_1));
}

#[test]
fn old_devices_and_bridges_are_skipped() {
    let mut s = laptop();
    let at = EXP_AT + reg::EXP_DEVCAP;
    s.space.get_mut(&SSD).unwrap()[(at / 4) as usize] = 0;
    let snapshot = s.space.clone();
    let r = run(&mut s, Policy { states: aspm::ALL, ltr: false }, false);
    assert_eq!(r.iter().next().unwrap().skip, Some(Skip::PreSpec11));
    assert_eq!(s.space, snapshot);

    let mut s = Sim::default();
    s.add(RP, Fun::port(reg::TYPE_ROOT_PORT, 1));
    s.add(SSD, Fun { kind: reg::TYPE_PCIE_TO_PCI, secondary: Some(2), ..Fun::nvme() });
    let r = run(&mut s, Policy::POWERSAVE, false);
    assert_eq!(r.iter().next().unwrap().skip, Some(Skip::PciBridge));
}

#[test]
fn empty_slot_is_reported_not_touched() {
    let mut s = Sim::default();
    s.add(RP, Fun::port(reg::TYPE_ROOT_PORT, 1));
    let r = run(&mut s, Policy::POWERSAVE, false);
    assert_eq!(r.iter().next().unwrap().skip, Some(Skip::Empty));
    assert_eq!(s.lnkctl(RP), 0);
}

#[test]
fn no_ltr_means_no_aspm_l1_2() {
    let mut s = laptop();
    let at = EXP_AT + reg::EXP_DEVCAP2;
    s.space.get_mut(&SSD).unwrap()[(at / 4) as usize] = 0;
    let r = run(&mut s, Policy::POWERSAVE, false);
    let l = r.iter().next().unwrap();
    assert_eq!(l.after & aspm::ASPM_L1_2, 0);
    assert_ne!(l.after & aspm::PCIPM_L1_2, 0, "PCI-PM L1.2 does not depend on LTR");
}

#[test]
fn switch_latency_adds_up_and_ltr_follows_the_path() {
    // Root port 00:1c.0 -> switch upstream 02:00.0 -> downstream 03:01.0 and 03:02.0 ->
    // endpoints 04:00.0 and 05:00.0. The endpoint under 03:01.0 accepts only 8 us of L1:
    // its own link (8 us) fits, the root link (8 us + 1 us for the switch) does not.
    let mut s = Sim::default();
    let root = Bdf::new(0, 0x1c, 0);
    let up = Bdf::new(2, 0, 0);
    let (d1, d2) = (Bdf::new(3, 1, 0), Bdf::new(3, 2, 0));
    let (e1, e2) = (Bdf::new(4, 0, 0), Bdf::new(5, 0, 0));
    s.add(root, Fun::port(reg::TYPE_ROOT_PORT, 2));
    s.add(up, Fun { kind: reg::TYPE_UPSTREAM, ..Fun::port(reg::TYPE_UPSTREAM, 3) });
    s.space.get_mut(&up).unwrap()[6] = 3 << 8 | 5 << 16 | 2;
    s.add(d1, Fun::port(reg::TYPE_DOWNSTREAM, 4));
    s.add(d2, Fun::port(reg::TYPE_DOWNSTREAM, 5));
    s.add(e1, Fun { devcap: reg::DEVCAP_RBER | 6 << 6 | 3 << 9, l1ss: None, ..Fun::nvme() });
    s.add(e2, Fun { l1ss: None, ..Fun::nvme() });
    let r = run(&mut s, Policy::POWERSAVE, false);
    assert_eq!(r.count, 3);
    let find = |p: Bdf| *r.iter().find(|l| l.port == p).unwrap();
    assert_eq!(find(root).after & aspm::L1, 0, "root link: 8 us + 1 us switch > 8 us");
    assert_ne!(find(d1).after & aspm::L1, 0);
    assert_ne!(find(d2).after & aspm::L1, 0);
    assert_eq!(find(d1).parent, find(d2).parent);
    assert!(find(d1).parent.is_some());
    for b in [root, up, d1, d2, e1, e2] { assert!(s.ltr_on(b), "LTR on {}", b); }
}

#[test]
fn ltr_stops_where_the_path_breaks() {
    let mut s = laptop();
    let at = EXP_AT + reg::EXP_DEVCAP2;
    s.space.get_mut(&RP).unwrap()[(at / 4) as usize] = 0;
    run(&mut s, Policy::POWERSAVE, false);
    assert!(!s.ltr_on(RP));
    assert!(!s.ltr_on(SSD), "a device under a port without LTR stays off");
}

#[test]
fn threshold_encoding_matches_linux() {
    assert_eq!(aspm::encode_threshold(0), (0, 0));
    // Smallest scale that fits 10 bits, rounded up: never shorter than asked.
    assert_eq!(aspm::encode_threshold(1), (0, 1000));
    assert_eq!(aspm::encode_threshold(30), (1, 938));
    assert_eq!(aspm::encode_threshold(106), (2, 104));
    assert_eq!(aspm::encode_threshold(2_000), (3, 62));
    assert_eq!(aspm::encode_threshold(100_000), (4, 96));
    assert_eq!(aspm::encode_threshold(u32::MAX), (5, 0x3FF));
    for us in [1u32, 7, 30, 106, 250, 2_000, 100_000] {
        let (scale, value) = aspm::encode_threshold(us);
        assert!(value as u64 * (1u64 << (5 * scale)) >= us as u64 * 1000, "{} us encoded short", us);
    }
    assert_eq!(aspm::power_on_us(3, 9), 0);
    assert_eq!(aspm::power_on_us(2, 3), 300);
}

#[test]
fn describe_names_states() {
    let mut buf = [0u8; 48];
    assert_eq!(aspm::describe(0, &mut buf), "none");
    let mut buf = [0u8; 48];
    assert_eq!(aspm::describe(aspm::ALL, &mut buf), "l0s+l1+l1.1+l1.2+pm1.1+pm1.2+clkpm");
}

#[test]
fn quirks() {
    assert_eq!(aspm::quirk(0x10ec, 0x8125, 0x0200_00), aspm::L0S | aspm::ASPM_L1_2 | aspm::PCIPM_L1_2);
    assert_eq!(aspm::quirk(0x10ec, 0x5260, 0x0ff0_00), 0, "Realtek card readers are not NICs");
    assert_eq!(aspm::quirk(0x8086, 0x2725, 0x0280_00), aspm::L0S);
    assert_eq!(aspm::quirk(0x144d, 0xa809, 0x0108_02), 0);
}

// ── D-states ────────────────────────────────────────────────────────────────

fn programmed_ssd(s: &mut Sim) {
    let w = s.space.get_mut(&SSD).unwrap();
    w[1] = reg::STATUS_CAP_LIST | 0x0006;
    w[4] = 0xFE60_0004;
    w[5] = 0;
    w[15] = 0x0000_010B;
    w[(MSI_AT / 4) as usize] |= 1 << 16;
    w[(MSI_AT / 4 + 1) as usize] = 0xFEE0_0000;
    w[(MSI_AT / 4 + 3) as usize] = 0x4041;
    w[((EXP_AT + reg::EXP_LNKCTL) / 4) as usize] = 0x0142;
}

#[test]
fn d3hot_round_trip_restores_a_reset_function() {
    let mut s = laptop();
    s.soft_reset.push(SSD);
    programmed_ssd(&mut s);
    let before = s.space[&SSD].clone();
    let mut saved = None;
    let t = pm::set_state(&mut s, SSD, pm::D3HOT, &mut saved).unwrap();
    assert_eq!((t.from, t.to, t.restored), (0, 3, false));
    assert_eq!(pm::state(&mut s, SSD), Some(3));
    let t = pm::set_state(&mut s, SSD, pm::D0, &mut saved).unwrap();
    assert!(t.restored && !t.no_soft_reset);
    assert_eq!(s.space[&SSD], before, "everything the reset lost is back");
    assert_eq!(s.lnkctl(SSD), 0x0142, "link control (with its ASPM bits) restored last");
    assert_eq!(s.delayed_us, 2 * pm::D3HOT_DELAY_US as u64);
}

#[test]
fn no_soft_reset_skips_the_restore() {
    let mut s = laptop();
    programmed_ssd(&mut s);
    s.space.get_mut(&SSD).unwrap()[((PM_AT + 4) / 4) as usize] |= 1 << 3;
    let mut saved = None;
    pm::set_state(&mut s, SSD, pm::D3HOT, &mut saved).unwrap();
    let t = pm::set_state(&mut s, SSD, pm::D0, &mut saved).unwrap();
    assert!(t.no_soft_reset && !t.restored);
}

#[test]
fn pm_errors() {
    let mut s = laptop();
    let mut saved = None;
    assert_eq!(pm::set_state(&mut s, SSD, 2, &mut saved), Err(pm::Error::BadState));
    assert_eq!(pm::set_state(&mut s, Bdf::new(9, 0, 0), 3, &mut saved), Err(pm::Error::Absent));
    let same = pm::set_state(&mut s, SSD, pm::D0, &mut saved).unwrap();
    assert_eq!((same.from, same.to), (0, 0));
    assert_eq!(s.delayed_us, 0, "no transition, no wait");
    // PME status (RW1C) is never written back as one.
    s.space.get_mut(&SSD).unwrap()[((PM_AT + 4) / 4) as usize] |= 1 << 15;
    pm::set_state(&mut s, SSD, pm::D3HOT, &mut saved).unwrap();
    assert_eq!(s.pmcsr_written & (1 << 15), 0);
    assert_eq!(s.pmcsr_written & 3, 3);
}
