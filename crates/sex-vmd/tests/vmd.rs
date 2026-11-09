//! VMD bus driver logic against a simulated VMD domain: two PCIe root ports,
//! each with an NVMe drive behind it, the way a Tiger Lake laptop in "RAID On"
//! mode looks. Config reads to a bus only reach a device once the bridge above
//! it has been given that bus number, like real hardware.

use sex_vmd::*;

#[derive(Clone)]
struct Func {
    dev: u8,
    func: u8,
    /// Index of the bridge this function sits behind; None = root bus.
    parent: Option<usize>,
    regs: [u32; 64],
    /// (length, low flag bits) per BAR slot; length 0 = no BAR.
    bars: [(u64, u32); 6],
    header: u8,
}

/// `first`: the domain's first bus; bridges hold absolute bus numbers.
struct Sim { funcs: Vec<Func>, first: u8 }

impl Sim {
    fn func(id: u32, class: u32, header: u8, multi: bool, dev: u8, func: u8, parent: Option<usize>) -> Func {
        let mut regs = [0u32; 64];
        regs[0] = id;
        regs[2] = class << 8;
        regs[3] = ((header as u32) | if multi { 0x80 } else { 0 }) << 16;
        regs[1] = 0x0010_0007; // firmware left decoding on, with a status bit set
        Func { dev, func, parent, regs, bars: [(0, 0); 6], header }
    }
    fn secondary(&self, i: usize) -> u8 { (self.funcs[i].regs[6] >> 8) as u8 }
    /// Absolute bus a function answers on, once the bridges route to it.
    fn reachable(&self, i: usize) -> Option<u8> {
        match self.funcs[i].parent {
            None => Some(self.first),
            Some(p) => { self.reachable(p)?; let s = self.secondary(p); if s <= self.first { None } else { Some(s) } }
        }
    }
    /// `bus` is relative, as config accesses through CFGBAR are.
    fn find(&self, bus: u8, dev: u8, func: u8) -> Option<usize> {
        let abs = self.first.checked_add(bus)?;
        (0..self.funcs.len()).find(|&i| {
            let f = &self.funcs[i];
            f.dev == dev && f.func == func && self.reachable(i) == Some(abs)
        })
    }
}

impl ConfigSpace for Sim {
    fn read(&mut self, bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
        let Some(i) = self.find(bus, dev, func) else { return u32::MAX };
        self.funcs[i].regs[offset as usize / 4]
    }
    fn write(&mut self, bus: u8, dev: u8, func: u8, offset: u16, value: u32) {
        let Some(i) = self.find(bus, dev, func) else { return };
        let f = &mut self.funcs[i];
        let reg = offset as usize / 4;
        let bar_slots = if f.header == 0 { 6 } else { 2 };
        if (4..4 + bar_slots).contains(&reg) {
            let slot = reg - 4;
            // Upper half of a 64-bit BAR.
            if slot > 0 && f.bars[slot].0 == 0 && f.bars[slot - 1].1 & 6 == 4 {
                let len = f.bars[slot - 1].0;
                f.regs[reg] = value & (!(len - 1) >> 32) as u32;
                return;
            }
            let (len, flags) = f.bars[slot];
            f.regs[reg] = if len == 0 { 0 } else { (value & !(len as u32).wrapping_sub(1) & !0xF) | flags };
            return;
        }
        if reg == 1 { f.regs[1] = (f.regs[1] & 0xFFFF_0000) | (value & 0xFFFF); return; }
        if reg == 0 || reg == 2 || reg == 3 { return; }
        f.regs[reg] = value;
    }
}

const NVME: u32 = 0x01_08_02;
const BRIDGE: u32 = 0x06_04_00;

/// Two root ports on the root bus (one multi-function), an NVMe behind each.
fn laptop() -> Sim {
    let mut port_a = Sim::func(0x9a09_8086, BRIDGE, 1, false, 6, 0, None);
    port_a.bars = [(0, 0); 6];
    let port_b = Sim::func(0x9a0b_8086 ^ 0x0002_0000, BRIDGE, 1, true, 0x1d, 0, None);
    let other_fn = Sim::func(0x43ef_8086, 0x08_80_00, 0, false, 0x1d, 3, None);
    let mut samsung = Sim::func(0xa80a_144d, NVME, 0, false, 0, 0, Some(0));
    samsung.bars[0] = (16 * 1024, 0x4); // 64-bit, non-prefetchable
    samsung.bars[4] = (0x100, 0x1);     // an I/O BAR, left closed
    let mut wd = Sim::func(0x5009_15b7, NVME, 0, false, 0, 0, Some(1));
    wd.bars[0] = (16 * 1024, 0x4);
    wd.bars[2] = (8 * 1024, 0x0);       // 32-bit memory BAR
    let mut funcs = vec![port_a, port_b, other_fn, samsung, wd];
    // A BAR always reads back its type bits.
    for f in &mut funcs { for (slot, &(len, flags)) in f.bars.iter().enumerate() { if len != 0 { f.regs[4 + slot] = flags; } } }
    Sim { funcs, first: 0 }
}

fn inside(w: Window, address: u64, length: u64) -> bool { address >= w.base && address + length <= w.base + w.length }

fn window(sim: &Sim, bridge: usize) -> (u64, u64) {
    let r = sim.funcs[bridge].regs[8];
    (((r & 0xFFF0) as u64) << 16, (((r >> 16) & 0xFFF0) as u64) << 16 | 0xFFFFF)
}

#[test]
fn finds_both_drives_and_places_their_registers_in_membar1() {
    let mut sim = laptop();
    let membar1 = Window { base: 0x8200_0000, length: 32 << 20 };
    let membar2 = Window { base: 0x40_0000_0000, length: 1 << 20 };
    let mut out = [Device::default(); 16];
    let n = configure(&mut sim, 0, 32, membar1, membar2, &mut out).unwrap();
    let drives: Vec<&Device> = out[..n].iter().filter(|d| (d.class, d.subclass) == (1, 8)).collect();
    assert_eq!(drives.len(), 2, "{:#?}", &out[..n]);
    assert_eq!(out[..n].iter().filter(|d| d.bridge).count(), 2);
    assert!(out[..n].iter().any(|d| (d.dev, d.func, d.vendor) == (0x1d, 3, 0x8086)), "multi-function port seen");
    let mut seen = Vec::new();
    for d in &drives {
        let bar = d.bar(0).unwrap();
        assert!(bar.wide && bar.length == 16384 && bar.address % 16384 == 0);
        assert!(inside(membar1, bar.address, bar.length), "{:#x}", bar.address);
        assert!(d.bar(4).is_none(), "I/O BARs stay closed");
        assert!(d.bus >= 1 && d.bus < 32);
        seen.push((d.vendor, d.bus));
    }
    assert_ne!(seen[0].1, seen[1].1, "each port gets its own bus");
    let wd = drives.iter().find(|d| d.vendor == 0x15b7).unwrap();
    let small = wd.bar(2).unwrap();
    assert!(!small.wide && inside(membar1, small.address, 8192));
    // The device registers hold what we said, decoding and DMA are on, INTx off.
    for (i, f) in sim.funcs.iter().enumerate().skip(3) {
        let d = drives.iter().find(|d| d.vendor == f.regs[0] as u16).unwrap();
        assert_eq!(f.regs[4] as u64 & !0xF | (f.regs[5] as u64) << 32, d.bar(0).unwrap().address, "func {}", i);
        assert_eq!(f.regs[1] & 0x0407, 0x0406);
        assert_eq!(f.regs[1] >> 16, 0x0010, "status untouched");
        // The bridge above opens exactly a window that covers it.
        let (lo, hi) = window(&sim, f.parent.unwrap());
        assert!(d.bars().iter().all(|b| b.address >= lo && b.address + b.length - 1 <= hi));
        assert!(inside(membar1, lo, hi - lo + 1));
    }
    let (a, b) = (window(&sim, 0), window(&sim, 1));
    assert!(a.1 < b.0 || b.1 < a.0, "bridge windows do not overlap");
}

#[test]
fn membar1_above_4g_puts_64bit_registers_in_membar2_past_the_msix_table() {
    let mut sim = laptop();
    sim.funcs[4].bars[2] = (0, 0); // no 32-bit BAR this time
    sim.funcs[4].regs[6] = 0;
    let membar1 = Window { base: 0x60_0000_0000, length: 32 << 20 };
    let membar2 = Window { base: 0x60_1000_0000, length: 32 << 20 };
    let mut out = [Device::default(); 16];
    let n = configure(&mut sim, 0, 32, membar1, membar2, &mut out).unwrap();
    for d in out[..n].iter().filter(|d| d.class == 1) {
        let bar = d.bar(0).unwrap();
        assert!(inside(membar2, bar.address, bar.length));
        assert!(bar.address >= membar2.base + MEMBAR2_RESERVED);
    }
    // Prefetchable window of the first port, upper halves included.
    let f = &sim.funcs[0];
    let base = ((f.regs[9] & 0xFFF0) as u64) << 16 | (f.regs[10] as u64) << 32;
    assert_eq!(base >> 32, 0x60);
}

#[test]
fn runs_out_cleanly() {
    let mut out = [Device::default(); 16];
    let tiny = Window { base: 0x8200_0000, length: 8 * 1024 };
    let none = Window { base: 0x40_0000_0000, length: 0x2000 };
    assert!(matches!(configure(&mut laptop(), 0, 32, tiny, none, &mut out), Err(Error::OutOfMemory { length: 16384, .. })));
    let big = Window { base: 0x8200_0000, length: 32 << 20 };
    assert_eq!(configure(&mut laptop(), 0, 2, big, none, &mut out), Err(Error::OutOfBuses));
    let mut few = [Device::default(); 2];
    assert_eq!(configure(&mut laptop(), 0, 32, big, none, &mut few), Err(Error::TooManyDevices));
}

#[test]
fn knows_the_controllers_and_the_bus_start() {
    assert!(is_vmd(0x8086, 0x9a0b));
    assert!(is_vmd(0x8086, 0x467f));
    assert!(!is_vmd(0x8086, 0x9a09));
    assert!(!is_vmd(0x10de, 0x9a0b));
    assert_eq!(bus_start(0, 0x200), 0);
    assert_eq!(bus_start(1, 0x000), 0);
    assert_eq!(bus_start(1, 0x100), 128);
    assert_eq!(bus_start(1, 0x200), 224);
}

#[test]
fn bridges_hold_absolute_bus_numbers_when_the_domain_starts_at_224() {
    // Tiger Lake laptops: VMCONFIG puts the domain at bus 224.
    let mut sim = laptop();
    sim.first = 224;
    let membar1 = Window { base: 0x8200_0000, length: 32 << 20 };
    let membar2 = Window { base: 0x40_0000_0000, length: 1 << 20 };
    let mut out = [Device::default(); 16];
    let n = configure(&mut sim, 224, 32, membar1, membar2, &mut out).unwrap();
    assert_eq!(out[..n].iter().filter(|d| d.class == 1).count(), 2, "both drives reached through the bridges");
    let r = sim.funcs[0].regs[6];
    assert_eq!((r & 0xFF, (r >> 8) & 0xFF), (224, 225), "primary and secondary are absolute");
    assert!(((r >> 16) & 0xFF) >= 225);
    // Only 32 buses fit from 224; asking for more is clipped, never wraps past 255.
    let mut sim = laptop();
    sim.first = 224;
    assert!(configure(&mut sim, 224, 64, membar1, membar2, &mut out).is_ok());
    assert_eq!((sim.funcs[1].regs[6] >> 16) & 0xFF, (sim.funcs[1].regs[6] >> 8) & 0xFF);
}
