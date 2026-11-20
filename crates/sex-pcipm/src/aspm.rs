// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Link power: ASPM L0s/L1, L1 PM Substates, clock PM and LTR, for every PCIe link.
//!
//! Policy (Linux's `powersupersave` with its safety checks):
//! * A link gets only the states both ends support, and only those whose exit latency
//!   every endpoint below it accepts (Device Capabilities "acceptable latency"; each
//!   switch on the way adds 1 us to L1).
//! * Pre-1.1 devices (no Role-Based Error Reporting) and PCIe-to-PCI bridges keep what
//!   firmware set. So do links named in [`quirk`].
//! * What firmware enabled stays enabled: this only adds states. L1 substates firmware
//!   already set up keep firmware's timing.
//! * LTR is turned on from the root port down; ASPM L1.2 needs it on the device (its
//!   latency messages are what keep L1.2 from hurting a busy device).
//! * Order (PCIe 5.5.4, 7.8.3.3): L1 off while L1 substates change, substates on in
//!   the upstream port first, then ASPM enables upstream first.
use crate::{find_ext_cap, pcie, reg, update16, Bdf, Config};

pub const L0S: u8 = 1 << 0;
pub const L1: u8 = 1 << 1;
pub const ASPM_L1_1: u8 = 1 << 2;
pub const ASPM_L1_2: u8 = 1 << 3;
pub const PCIPM_L1_1: u8 = 1 << 4;
pub const PCIPM_L1_2: u8 = 1 << 5;
pub const CLKPM: u8 = 1 << 6;
pub const L1SS: u8 = ASPM_L1_1 | ASPM_L1_2 | PCIPM_L1_1 | PCIPM_L1_2;
pub const ALL: u8 = L0S | L1 | L1SS | CLKPM;

pub const MAX_LINKS: usize = 64;
pub const MAX_CHILDREN: usize = 8;
const MAX_FUNCTIONS: usize = 256;

/// Which states may be added. `ltr` turns Latency Tolerance Reporting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy { pub states: u8, pub ltr: bool }
impl Policy {
    /// Laptops and everything by default: every state the hardware can take.
    pub const POWERSAVE: Policy = Policy { states: ALL, ltr: true };
    /// Firmware's settings, untouched.
    pub const FIRMWARE: Policy = Policy { states: 0, ltr: false };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// The FADT forbids the OS from changing ASPM.
    Forbidden,
    /// A function below lacks Role-Based Error Reporting (PCIe 1.0a or older).
    PreSpec11,
    /// The device below is a PCIe-to-PCI bridge.
    PciBridge,
    /// No function on the secondary bus (empty slot).
    Empty,
}
impl Skip {
    pub fn name(self) -> &'static str {
        match self { Skip::Forbidden => "fadt_no_aspm", Skip::PreSpec11 => "pre_1_1", Skip::PciBridge => "pci_bridge", Skip::Empty => "empty" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    /// The downstream port (root port or switch downstream port).
    pub port: Bdf,
    pub children: [Bdf; MAX_CHILDREN],
    pub count: u8,
    /// Index of the link above this one, for switches.
    pub parent: Option<u8>,
    /// Device id of the first function below (vendor | device << 16).
    pub id: u32,
    /// States both ends support (before latency and quirks).
    pub support: u8,
    /// States that passed the latency check and quirks.
    pub capable: u8,
    /// States enabled as found, and after this pass.
    pub before: u8,
    pub after: u8,
    pub skip: Option<Skip>,
    /// Each function's own ASPM control bits as found (port first).
    own: [(Bdf, u8); MAX_CHILDREN + 1],
    /// L1.2 entry threshold programmed, in microseconds (0 = not programmed).
    pub threshold_us: u32,
    port_exp: u16,
    child_exp: [u16; MAX_CHILDREN],
    port_l1ss: u16,
    child_l1ss: u16,
    l0s_up_ns: u32,
    l0s_dw_ns: u32,
    l1_up_ns: u32,
    l1_dw_ns: u32,
}
impl Link {
    pub fn children(&self) -> &[Bdf] { &self.children[..self.count as usize] }
    pub fn vendor(&self) -> u16 { self.id as u16 }
    pub fn device(&self) -> u16 { (self.id >> 16) as u16 }
}

/// Everything one pass did.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub links: [Option<Link>; MAX_LINKS],
    pub count: usize,
    /// Functions whose LTR this pass turned on, and that had it already.
    pub ltr_enabled: u16,
    pub ltr_already: u16,
    /// Links past MAX_LINKS, left alone.
    pub dropped: u16,
}
impl Report {
    pub fn iter(&self) -> impl Iterator<Item = &Link> { self.links[..self.count].iter().flatten() }
}

/// States a device must never be given, by vendor/device id (and class for families).
/// From Linux's PCI quirks and NIC/Wi-Fi drivers' own ASPM limits.
pub fn quirk(vendor: u16, device: u16, class: u32) -> u8 {
    match (vendor, device) {
        // Intel 82575/82598/82599 and 82574 gigabit: L0s errata.
        (0x8086, 0x10a7 | 0x10a9 | 0x10b6 | 0x10c6 | 0x10c7 | 0x10c8 | 0x10d6 | 0x10db | 0x10dd
            | 0x10e1 | 0x10ec | 0x10f1 | 0x10f4 | 0x1508 | 0x10d3) => L0S,
        // ASMedia ASM1080 PCIe-to-PCI bridge.
        (0x1b21, 0x1080) => L0S | L1 | L1SS,
        // Realtek Ethernet (r8168/r8125 family): L0s and L1.2 hang some chips.
        (0x10ec, _) if class >> 8 == 0x0200 => L0S | ASPM_L1_2 | PCIPM_L1_2,
        // Intel Wi-Fi: its driver keeps L0s off whenever L1 is on.
        (0x8086, _) if class >> 8 == 0x0280 => L0S,
        _ => 0,
    }
}

fn l0s_exit_ns(lnkcap: u32) -> u32 { match (lnkcap >> 12) & 7 { 7 => 5_000, e => 64 << e } }
fn l1_exit_ns(lnkcap: u32) -> u32 { match (lnkcap >> 15) & 7 { 7 => 65_000, e => 1_000 << e } }
fn l0s_acceptable_ns(devcap: u32) -> u32 { match (devcap >> 6) & 7 { 7 => u32::MAX, e => 64 << e } }
fn l1_acceptable_ns(devcap: u32) -> u32 { match (devcap >> 9) & 7 { 7 => u32::MAX, e => 1_000 << e } }

/// T_POWER_ON in microseconds from a scale (2/10/100 us) and value.
pub fn power_on_us(scale: u32, value: u32) -> u32 {
    match scale { 0 => value * 2, 1 => value * 10, 2 => value * 100, _ => 0 }
}

/// LTR_L1.2_THRESHOLD as (scale, value) for a time in microseconds.
/// The smallest scale whose 10-bit value holds the time, rounded up, so the encoded
/// threshold is never shorter than the transition it guards (as Linux encodes it).
pub fn encode_threshold(us: u32) -> (u32, u32) {
    let ns = us as u64 * 1_000;
    for scale in 0..6u32 {
        let unit = 1u64 << (5 * scale);
        if ns <= unit * 0x3FF { return (scale, ns.div_ceil(unit) as u32); }
    }
    (5, 0x3FF)
}

/// The CTL1 timing fields and CTL2 for L1.2 on a link, and the threshold in us
/// (PCIe 5.5.3.3.1: T_POWER_OFF at most 2 us, T_L1.2 at least 4 us).
pub fn l12_timing(port_cap: u32, child_cap: u32) -> (u32, u32, u32) {
    let common_mode = ((port_cap >> 8) & 0xFF).max((child_cap >> 8) & 0xFF);
    let (ps, pv) = ((port_cap >> 16) & 3, (port_cap >> 19) & 0x1F);
    let (cs, cv) = ((child_cap >> 16) & 3, (child_cap >> 19) & 0x1F);
    let (scale, value) = if power_on_us(ps, pv) > power_on_us(cs, cv) { (ps, pv) } else { (cs, cv) };
    let power_on = power_on_us(scale, value);
    let threshold = 2 + 4 + common_mode + power_on;
    let (ts, tv) = encode_threshold(threshold);
    (common_mode << 8 | tv << 16 | ts << 29, scale | value << 3, threshold)
}

fn l1ss_bits_from_ctl(v: u32) -> u8 {
    let mut s = 0;
    if v & reg::L1SS_ASPM_L1_1 != 0 { s |= ASPM_L1_1; }
    if v & reg::L1SS_ASPM_L1_2 != 0 { s |= ASPM_L1_2; }
    if v & reg::L1SS_PCIPM_L1_1 != 0 { s |= PCIPM_L1_1; }
    if v & reg::L1SS_PCIPM_L1_2 != 0 { s |= PCIPM_L1_2; }
    s
}
fn ctl_from_l1ss_bits(s: u8) -> u32 {
    let mut v = 0;
    if s & ASPM_L1_1 != 0 { v |= reg::L1SS_ASPM_L1_1; }
    if s & ASPM_L1_2 != 0 { v |= reg::L1SS_ASPM_L1_2; }
    if s & PCIPM_L1_1 != 0 { v |= reg::L1SS_PCIPM_L1_1; }
    if s & PCIPM_L1_2 != 0 { v |= reg::L1SS_PCIPM_L1_2; }
    v
}

/// Functions with LTR on all the way from their root port.
struct LtrPath { on: [Bdf; MAX_FUNCTIONS], n: usize }
impl LtrPath {
    fn has(&self, f: Bdf) -> bool { self.on[..self.n].contains(&f) }
    fn add(&mut self, f: Bdf) { if self.n < MAX_FUNCTIONS && !self.has(f) { self.on[self.n] = f; self.n += 1; } }
}

/// Turns LTR on in `f` if it supports it and `upstream_on` (always for a root port).
fn ltr<C: Config + ?Sized>(c: &mut C, f: Bdf, upstream_on: bool, path: &mut LtrPath, r: &mut Report) {
    let Some((at, _)) = pcie(c, f) else { return };
    if c.read(f, at + reg::EXP_DEVCAP2) & reg::DEVCAP2_LTR == 0 || !upstream_on { return; }
    let (before, _) = update16(c, f, at + reg::EXP_DEVCTL2, 0, reg::DEVCTL2_LTR);
    if before & reg::DEVCTL2_LTR != 0 { r.ltr_already += 1 } else { r.ltr_enabled += 1 }
    path.add(f);
}

/// The bridge whose secondary bus is `bus` (the upstream bridge of anything on it).
fn bridge_to(c: &mut impl Config, functions: &[Bdf], bus: u8) -> Option<Bdf> {
    functions.iter().copied().find(|b| c.read(*b, reg::HEADER) >> 16 & 0x7F == 1 && (c.read(*b, reg::SECONDARY_BUS) >> 8) as u8 == bus)
}

/// Finds every link among `functions` (all of segment 0's functions, in bus order).
pub fn discover<C: Config>(c: &mut C, functions: &[Bdf], r: &mut Report) {
    for &port in functions {
        let Some((port_exp, kind)) = pcie(c, port) else { continue };
        if kind != reg::TYPE_ROOT_PORT && kind != reg::TYPE_DOWNSTREAM { continue; }
        if c.read(port, reg::HEADER) >> 16 & 0x7F != 1 { continue; }
        let secondary = (c.read(port, reg::SECONDARY_BUS) >> 8) as u8;
        if secondary == 0 || secondary <= port.bus { continue; }
        let mut link = Link {
            port, children: [Bdf::new(0, 0, 0); MAX_CHILDREN], count: 0, parent: None, id: 0, support: 0, capable: 0,
            before: 0, after: 0, skip: None, own: [(Bdf::new(0, 0, 0), 0); MAX_CHILDREN + 1], threshold_us: 0, port_exp, child_exp: [0; MAX_CHILDREN],
            port_l1ss: 0, child_l1ss: 0, l0s_up_ns: 0, l0s_dw_ns: 0, l1_up_ns: 0, l1_dw_ns: 0,
        };
        for &f in functions.iter().filter(|f| f.bus == secondary && f.dev == 0) {
            if (link.count as usize) < MAX_CHILDREN { link.children[link.count as usize] = f; link.count += 1; }
        }
        if r.count >= MAX_LINKS { r.dropped += 1; continue; }
        r.links[r.count] = Some(link);
        r.count += 1;
    }
    // Parents: a switch downstream port sits behind an upstream port that is the
    // child of another link.
    for i in 0..r.count {
        let Some(l) = r.links[i] else { continue };
        let Some(up) = bridge_to(c, functions, l.port.bus) else { continue };
        let parent = (0..r.count).find(|&j| r.links[j].is_some_and(|p| p.children().contains(&up)));
        if let Some(link) = r.links[i].as_mut() { link.parent = parent.map(|p| p as u8); }
    }
}

fn depth(r: &Report, i: usize) -> usize {
    let mut d = 0;
    let mut at = r.links[i].and_then(|l| l.parent);
    while let Some(p) = at { d += 1; if d > MAX_LINKS { break; } at = r.links[p as usize].and_then(|l| l.parent); }
    d
}

/// Reads each link's support, current state and latencies; marks links to leave alone.
fn survey<C: Config>(c: &mut C, r: &mut Report, forbidden: bool) {
    for i in 0..r.count {
        let Some(mut l) = r.links[i] else { continue };
        if l.count == 0 { l.skip = Some(Skip::Empty); r.links[i] = Some(l); continue; }
        let port_cap = c.read(l.port, l.port_exp + reg::EXP_LNKCAP);
        let port_ctl = c.read(l.port, l.port_exp + reg::EXP_LNKCTL);
        l.own[0] = (l.port, (port_ctl & 3) as u8);
        l.id = c.read(l.children[0], 0);
        let mut support = ((port_cap >> 10) & 3) as u8;
        let mut clkpm_support = true;
        let mut clkpm_on = true;
        let mut children_aspm = 0xFFu8;
        for k in 0..l.count as usize {
            let f = l.children[k];
            let Some((at, kind)) = pcie(c, f) else { l.skip = Some(Skip::PciBridge); break };
            l.child_exp[k] = at;
            if kind == reg::TYPE_PCIE_TO_PCI { l.skip = Some(Skip::PciBridge); }
            if c.read(f, at + reg::EXP_DEVCAP) & reg::DEVCAP_RBER == 0 && l.skip.is_none() { l.skip = Some(Skip::PreSpec11); }
            let cap = c.read(f, at + reg::EXP_LNKCAP);
            let ctl = c.read(f, at + reg::EXP_LNKCTL);
            support &= ((cap >> 10) & 3) as u8;
            clkpm_support &= cap & reg::LNKCAP_CLKPM != 0;
            clkpm_on &= ctl & reg::LNKCTL_CLKPM != 0;
            children_aspm &= (ctl & 3) as u8;
            l.own[k + 1] = (f, (ctl & 3) as u8);
            if k == 0 {
                l.l0s_up_ns = l0s_exit_ns(cap);
                l.l1_up_ns = l1_exit_ns(cap);
            }
        }
        l.l0s_dw_ns = l0s_exit_ns(port_cap);
        l.l1_dw_ns = l1_exit_ns(port_cap);
        if clkpm_support { support |= CLKPM; }
        l.before = (port_ctl as u8 & 3) & children_aspm | if clkpm_on { CLKPM } else { 0 };
        // L1 substates: both ends, through ECAM.
        if let (Some(pa), Some(ca)) = (find_ext_cap(c, l.port, reg::EXT_L1SS), find_ext_cap(c, l.children[0], reg::EXT_L1SS)) {
            let (pc, cc) = (c.read(l.port, pa + reg::L1SS_CAP), c.read(l.children[0], ca + reg::L1SS_CAP));
            if pc & cc & reg::L1SS_CAP_SUBSTATES != 0 && support & L1 != 0 {
                support |= l1ss_bits_from_ctl(pc & cc);
            }
            l.port_l1ss = pa;
            l.child_l1ss = ca;
            l.before |= l1ss_bits_from_ctl(c.read(l.port, pa + reg::L1SS_CTL1) & c.read(l.children[0], ca + reg::L1SS_CTL1));
        }
        if forbidden { l.skip = Some(Skip::Forbidden); }
        let class = c.read(l.children[0], 0x08) >> 8;
        l.support = support;
        l.capable = support & !quirk(l.vendor(), l.device(), class);
        r.links[i] = Some(l);
    }
}

/// Removes states whose exit latency an endpoint below does not accept.
fn check_latency<C: Config>(c: &mut C, r: &mut Report) {
    for i in 0..r.count {
        let Some(l) = r.links[i] else { continue };
        for k in 0..l.count as usize {
            let f = l.children[k];
            let Some((at, kind)) = pcie(c, f) else { continue };
            if kind != reg::TYPE_ENDPOINT && kind != reg::TYPE_LEGACY_ENDPOINT { continue; }
            let devcap = c.read(f, at + reg::EXP_DEVCAP);
            let (acc_l0s, acc_l1) = (l0s_acceptable_ns(devcap), l1_acceptable_ns(devcap));
            let mut switch_l1 = 0u32;
            let mut at_link = Some(i);
            let mut hops = 0;
            while let Some(j) = at_link {
                let Some(mut up) = r.links[j] else { break };
                if up.l0s_up_ns > acc_l0s || up.l0s_dw_ns > acc_l0s { up.capable &= !L0S; }
                if up.l1_up_ns.max(up.l1_dw_ns).saturating_add(switch_l1) > acc_l1 { up.capable &= !(L1 | L1SS); }
                switch_l1 += 1_000;
                r.links[j] = Some(up);
                at_link = up.parent.map(|p| p as usize);
                hops += 1;
                if hops > MAX_LINKS { break; }
            }
        }
    }
}

/// Programs one link to `l.after`, adding only. Returns the L1.2 threshold used.
fn apply<C: Config>(c: &mut C, l: &Link) -> u32 {
    let add = l.after & !l.before;
    if add == 0 { return 0; }
    let mut threshold = 0;
    if add & L1SS != 0 && l.port_l1ss != 0 {
        let (pa, ca, child) = (l.port_l1ss, l.child_l1ss, l.children[0]);
        // ASPM L1 off (downstream first) while the substates change.
        for k in 0..l.count as usize { update16(c, l.children[k], l.child_exp[k] + reg::EXP_LNKCTL, reg::LNKCTL_ASPM_L1, 0); }
        update16(c, l.port, l.port_exp + reg::EXP_LNKCTL, reg::LNKCTL_ASPM_L1, 0);
        if l.after & (ASPM_L1_2 | PCIPM_L1_2) != 0 {
            let (ctl1, ctl2, us) = l12_timing(c.read(l.port, pa + reg::L1SS_CAP), c.read(child, ca + reg::L1SS_CAP));
            threshold = us;
            for (f, at) in [(l.port, pa), (child, ca)] {
                let v = c.read(f, at + reg::L1SS_CTL1);
                c.write(f, at + reg::L1SS_CTL1, v & !reg::L1SS_ENABLES);
            }
            c.write(l.port, pa + reg::L1SS_CTL2, ctl2);
            c.write(child, ca + reg::L1SS_CTL2, ctl2);
            // Common mode restore time goes to the upstream port only; the threshold to both.
            let v = c.read(l.port, pa + reg::L1SS_CTL1);
            c.write(l.port, pa + reg::L1SS_CTL1, (v & !(reg::L1SS_CTL1_CM_RESTORE | reg::L1SS_CTL1_LTR_THRESHOLD)) | ctl1);
            let v = c.read(child, ca + reg::L1SS_CTL1);
            c.write(child, ca + reg::L1SS_CTL1, (v & !reg::L1SS_CTL1_LTR_THRESHOLD) | (ctl1 & reg::L1SS_CTL1_LTR_THRESHOLD));
        }
        let enables = ctl_from_l1ss_bits(l.after & L1SS);
        for (f, at) in [(l.port, pa), (child, ca)] {
            let v = c.read(f, at + reg::L1SS_CTL1);
            c.write(f, at + reg::L1SS_CTL1, (v & !reg::L1SS_ENABLES) | enables);
        }
    }
    if add & CLKPM != 0 {
        for k in 0..l.count as usize { update16(c, l.children[k], l.child_exp[k] + reg::EXP_LNKCTL, 0, reg::LNKCTL_CLKPM); }
    }
    // ASPM: the upstream port first, then every function below (all functions of a
    // device take the same setting).
    // Each function keeps its own firmware bits (an L0s firmware set on one end only),
    // plus the new states; L1 cleared above for the substates comes back here.
    let aspm = (l.after & (L0S | L1)) as u32;
    let own = |f: Bdf| (l.own.iter().find(|o| o.0 == f).map_or(0, |o| o.1)) as u32;
    update16(c, l.port, l.port_exp + reg::EXP_LNKCTL, 3, aspm | own(l.port));
    for k in 0..l.count as usize { let f = l.children[k]; update16(c, f, l.child_exp[k] + reg::EXP_LNKCTL, 3, aspm | own(f)); }
    threshold
}

/// One pass over every link: survey, latency checks, LTR, then the new states.
/// `forbidden` is the FADT's "OSPM must not enable ASPM" flag: links are only reported.
pub fn configure<C: Config>(c: &mut C, functions: &[Bdf], policy: Policy, forbidden: bool) -> Report {
    let mut r = Report { links: [None; MAX_LINKS], count: 0, ltr_enabled: 0, ltr_already: 0, dropped: 0 };
    discover(c, functions, &mut r);
    survey(c, &mut r, forbidden);
    check_latency(c, &mut r);
    // LTR from the root ports down, before L1.2 (which needs it on the device).
    let mut order = [0u8; MAX_LINKS];
    for (i, o) in order.iter_mut().enumerate().take(r.count) { *o = i as u8; }
    order[..r.count].sort_unstable_by_key(|&i| depth(&r, i as usize));
    let mut path = LtrPath { on: [Bdf::new(0, 0, 0); MAX_FUNCTIONS], n: 0 };
    for &i in &order[..r.count] {
        let Some(mut l) = r.links[i as usize] else { continue };
        if policy.ltr && !forbidden {
            let upstream = match pcie(c, l.port) {
                Some((_, reg::TYPE_ROOT_PORT)) => true,
                _ => bridge_to(c, functions, l.port.bus).is_some_and(|b| path.has(b)),
            };
            ltr(c, l.port, upstream, &mut path, &mut r);
            let port_on = path.has(l.port);
            for k in 0..l.count as usize { ltr(c, l.children[k], port_on, &mut path, &mut r); }
        }
        // ASPM L1.2 needs LTR on the device below (PCI-PM L1.2 does not).
        if l.count > 0 && !path.has(l.children[0]) { l.capable &= !ASPM_L1_2; }
        l.after = l.before;
        if l.skip.is_none() {
            let mut want = l.capable & policy.states;
            if want & L1 == 0 && l.before & L1 == 0 { want &= !L1SS; }
            // Firmware's own substate timing stays as it is.
            if l.before & L1SS != 0 { want &= !L1SS; }
            l.after = l.before | want;
            l.threshold_us = apply(c, &l);
        }
        r.links[i as usize] = Some(l);
    }
    r
}

/// Short text for a set of state bits, for logs: "l0s+l1+l1.1+l1.2+pm1.1+pm1.2+clkpm" or "none".
pub fn describe(bits: u8, out: &mut [u8; 48]) -> &str {
    const NAMES: [(u8, &str); 7] = [(L0S, "l0s"), (L1, "l1"), (ASPM_L1_1, "l1.1"), (ASPM_L1_2, "l1.2"),
        (PCIPM_L1_1, "pm1.1"), (PCIPM_L1_2, "pm1.2"), (CLKPM, "clkpm")];
    let mut n = 0;
    for (bit, name) in NAMES {
        if bits & bit == 0 { continue; }
        if n > 0 { out[n] = b'+'; n += 1; }
        out[n..n + name.len()].copy_from_slice(name.as_bytes());
        n += name.len();
    }
    if n == 0 { out[..4].copy_from_slice(b"none"); n = 4; }
    core::str::from_utf8(&out[..n]).unwrap_or("?")
}
