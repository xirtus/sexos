//! Intel Volume Management Device (VMD): the PCI bus that Intel RST "RAID On"
//! puts NVMe drives behind. With VMD on, the drives are not on the normal PCI
//! bus at all. The VMD controller (class 01:04, e.g. 8086:9a0b on Tiger Lake)
//! has three BARs:
//!
//! * BAR0 `CFGBAR`: an ECAM window for its own PCI domain, 1 MiB per bus,
//!   starting at bus 0, 128 or 224 (VMCONFIG bits 9:8 when VMCAP bit 0 is set).
//! * BAR2 `MEMBAR1`: non-prefetchable memory for the devices behind it.
//! * BAR4 `MEMBAR2`: prefetchable memory; its first 8 KiB holds VMD's own
//!   MSI-X table and is never handed out.
//!
//! This crate is the bus driver's logic, free of hardware access so it runs on
//! the host: number the buses behind each root port, give every memory BAR an
//! address inside MEMBAR1/MEMBAR2, open the bridge windows to match, and turn on
//! memory decoding and bus mastering. The kernel then hands each device found
//! (an NVMe drive, normally) to its driver like any other PCI device.
//!
//! Firmware leaves its own setup behind, which may be partial (the RST UEFI
//! driver only sets up what it booted from), so everything is assigned afresh,
//! as Linux's vmd driver also does. Facts used: the PCI and PCIe base specs and
//! the VMD register meanings documented in Linux's drivers/pci/controller/vmd.c
//! (read only).
#![no_std]

/// Intel VMD controllers, by device id. All are vendor 8086, class 01:04.
pub const DEVICE_IDS: &[u16] = &[
    0x201d, // Skylake-SP / Cascade Lake-SP
    0x28c0, // VMD in a guest (shadow registers)
    0x467f, // Alder Lake
    0x4c3d, // Rocket Lake
    0x7d0b, // Meteor Lake
    0x9a0b, // Tiger Lake (Alienware x17 R1 and most 2021 laptops)
    0xa77f, // Raptor Lake
    0xad0b, // Arrow Lake
    0xb60b, // Lunar Lake
    0xb06f, // Panther Lake
];

pub fn is_vmd(vendor: u16, device: u16) -> bool { vendor == 0x8086 && DEVICE_IDS.contains(&device) }

/// VMD capability (config 0x40) and configuration (0x44) registers.
pub const VMCAP: u16 = 0x40;
pub const VMCONFIG: u16 = 0x44;
/// The first bus number of the VMD domain.
pub fn bus_start(vmcap: u32, vmconfig: u32) -> u8 {
    if vmcap & 1 == 0 { return 0; }
    match (vmconfig >> 8) & 3 { 1 => 128, 2 => 224, _ => 0 }
}

/// MEMBAR2 begins with VMD's own MSI-X table.
pub const MEMBAR2_RESERVED: u64 = 0x2000;

/// Config space of the VMD domain, addressed by bus number relative to the
/// domain's first bus (the CFGBAR offset). Bridge registers, on the other hand,
/// hold absolute bus numbers (first bus + relative); `configure` writes those.
pub trait ConfigSpace {
    fn read(&mut self, bus: u8, dev: u8, func: u8, offset: u16) -> u32;
    fn write(&mut self, bus: u8, dev: u8, func: u8, offset: u16, value: u32);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window { pub base: u64, pub length: u64 }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar { pub index: u8, pub address: u64, pub length: u64, pub prefetchable: bool, pub wide: bool }

/// A device found behind VMD, with its BARs as assigned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub bus: u8, pub dev: u8, pub func: u8,
    pub vendor: u16, pub device: u16,
    pub class: u8, pub subclass: u8, pub prog_if: u8,
    pub bridge: bool,
    pub bars: [Bar; 6],
    pub bar_count: u8,
}

impl Device {
    pub fn bars(&self) -> &[Bar] { &self.bars[..self.bar_count as usize] }
    pub fn bar(&self, index: u8) -> Option<Bar> { self.bars().iter().copied().find(|b| b.index == index) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// More buses are needed than the CFGBAR covers.
    OutOfBuses,
    /// A BAR does not fit in what is left of MEMBAR1/MEMBAR2.
    OutOfMemory { bus: u8, dev: u8, func: u8, bar: u8, length: u64 },
    /// More devices than the caller's list holds.
    TooManyDevices,
}

const BRIDGE_ALIGN: u64 = 1 << 20;

struct Cursor { next: u64, end: u64 }
impl Cursor {
    fn take(&mut self, length: u64, align: u64) -> Option<u64> {
        let at = self.next.checked_add(align - 1)? & !(align - 1);
        let end = at.checked_add(length)?;
        if end > self.end { return None; }
        self.next = end;
        Some(at)
    }
    fn align(&mut self, align: u64) { self.next = self.next.saturating_add(align - 1) & !(align - 1); }
}

struct Walk<'a, C: ConfigSpace> {
    cfg: &'a mut C,
    buses: u16,
    next_bus: u16,
    /// Absolute number of relative bus 0 (VMD's bus start: 0, 128 or 224).
    first: u16,
    mem: Cursor,
    pref: Cursor,
    /// Non-prefetchable bridge windows are 32-bit; if MEMBAR1 lies above 4 GiB
    /// 64-bit BARs go to the prefetchable window instead (allowed on PCIe).
    mem_32bit: bool,
    out: &'a mut [Device],
    found: usize,
}

/// Set up everything behind the VMD controller and list what is there.
/// `first_bus` is the domain's first bus number (`bus_start`); `buses` is how
/// many buses the CFGBAR covers (its length / 1 MiB). Returns how many entries
/// of `out` were filled; bridges are listed too (with `bridge` set). Device
/// bus numbers in `out` are relative, like `ConfigSpace`'s.
pub fn configure<C: ConfigSpace>(cfg: &mut C, first_bus: u8, buses: u16, membar1: Window, membar2: Window, out: &mut [Device]) -> Result<usize, Error> {
    let pref_base = membar2.base + MEMBAR2_RESERVED.min(membar2.length);
    let mut walk = Walk {
        cfg, buses: buses.min(256 - first_bus as u16), next_bus: 1, first: first_bus as u16,
        mem: Cursor { next: membar1.base, end: membar1.base + membar1.length },
        pref: Cursor { next: pref_base, end: membar2.base + membar2.length },
        mem_32bit: membar1.base + membar1.length <= 1 << 32,
        out, found: 0,
    };
    if walk.buses == 0 { return Err(Error::OutOfBuses); }
    walk.bus(0)?;
    Ok(walk.found)
}

const COMMAND: u16 = 0x04;
const MEMORY: u32 = 1 << 1;
const MASTER: u32 = 1 << 2;
const IO: u32 = 1 << 0;
const INTX_OFF: u32 = 1 << 10;

impl<C: ConfigSpace> Walk<'_, C> {
    fn bus(&mut self, bus: u8) -> Result<(), Error> {
        for dev in 0..32u8 {
            let id = self.cfg.read(bus, dev, 0, 0);
            if id & 0xFFFF == 0xFFFF || id == 0 { continue; }
            let multi = (self.cfg.read(bus, dev, 0, 0x0C) >> 16) & 0x80 != 0;
            for func in 0..if multi { 8 } else { 1 } {
                let id = self.cfg.read(bus, dev, func, 0);
                if id & 0xFFFF == 0xFFFF || id == 0 { continue; }
                self.function(bus, dev, func, id)?;
            }
        }
        Ok(())
    }

    fn function(&mut self, bus: u8, dev: u8, func: u8, id: u32) -> Result<(), Error> {
        let class = self.cfg.read(bus, dev, func, 0x08);
        let header = (self.cfg.read(bus, dev, func, 0x0C) >> 16) as u8 & 0x7F;
        let command = self.cfg.read(bus, dev, func, COMMAND) & 0xFFFF;
        // Decoding off while BARs move.
        self.cfg.write(bus, dev, func, COMMAND, command & !(MEMORY | IO | MASTER));
        let slot = self.found;
        if slot >= self.out.len() { return Err(Error::TooManyDevices); }
        self.found += 1;
        let mut d = Device { bus, dev, func, vendor: id as u16, device: (id >> 16) as u16,
            class: (class >> 24) as u8, subclass: (class >> 16) as u8, prog_if: (class >> 8) as u8,
            bridge: header == 1, ..Default::default() };
        let bar_slots = match header { 0 => 6, 1 => 2, _ => 0 };
        let mut index = 0u8;
        while index < bar_slots {
            let wide = self.bar(bus, dev, func, index, &mut d)?;
            index += if wide { 2 } else { 1 };
        }
        if d.bridge { self.bridge(bus, dev, func)?; }
        self.cfg.write(bus, dev, func, COMMAND, (command & !IO) | MEMORY | MASTER | INTX_OFF);
        self.out[slot] = d;
        Ok(())
    }

    /// Size one BAR and place it. Returns whether it took two slots.
    fn bar(&mut self, bus: u8, dev: u8, func: u8, index: u8, d: &mut Device) -> Result<bool, Error> {
        let at = 0x10 + 4 * index as u16;
        let low = self.cfg.read(bus, dev, func, at);
        if low & 1 != 0 {
            // I/O BARs are not decoded behind VMD; leave them closed.
            return Ok(false);
        }
        let wide = (low >> 1) & 3 == 2;
        let prefetchable = low & 8 != 0;
        self.cfg.write(bus, dev, func, at, u32::MAX);
        if wide { self.cfg.write(bus, dev, func, at + 4, u32::MAX); }
        let mask = (self.cfg.read(bus, dev, func, at) & !0xF) as u64
            | if wide { (self.cfg.read(bus, dev, func, at + 4) as u64) << 32 } else { 0xFFFF_FFFF_0000_0000 };
        let length = (!mask).wrapping_add(1);
        if mask == 0xFFFF_FFFF_0000_0000 || length == 0 || !length.is_power_of_two() {
            self.cfg.write(bus, dev, func, at, 0);
            if wide { self.cfg.write(bus, dev, func, at + 4, 0); }
            return Ok(wide);
        }
        let align = length.max(4096);
        let use_pref = wide && (prefetchable || !self.mem_32bit);
        let address = if use_pref { self.pref.take(length, align) } else { self.mem.take(length, align) }
            .or_else(|| if wide && !use_pref { self.pref.take(length, align) } else { None })
            .ok_or(Error::OutOfMemory { bus, dev, func, bar: index, length })?;
        if !wide && address >> 32 != 0 { return Err(Error::OutOfMemory { bus, dev, func, bar: index, length }); }
        self.cfg.write(bus, dev, func, at, (address as u32 & !0xF) | (low & 0xF));
        if wide { self.cfg.write(bus, dev, func, at + 4, (address >> 32) as u32); }
        let n = d.bar_count as usize;
        d.bars[n] = Bar { index, address, length, prefetchable, wide };
        d.bar_count += 1;
        Ok(wide)
    }

    fn bridge(&mut self, bus: u8, dev: u8, func: u8) -> Result<(), Error> {
        if self.next_bus >= self.buses { return Err(Error::OutOfBuses); }
        let secondary = self.next_bus as u8;
        self.next_bus += 1;
        // Primary, secondary, subordinate (open to the end while we walk).
        let reg = self.cfg.read(bus, dev, func, 0x18) & 0xFF00_0000;
        let abs = |b: u16| (self.first + b) as u32;
        self.cfg.write(bus, dev, func, 0x18, reg | abs(bus as u16) | abs(secondary as u16) << 8 | abs(self.buses - 1) << 16);
        self.mem.align(BRIDGE_ALIGN);
        self.pref.align(BRIDGE_ALIGN);
        let (mem_start, pref_start) = (self.mem.next, self.pref.next);
        self.bus(secondary)?;
        self.mem.align(BRIDGE_ALIGN);
        self.pref.align(BRIDGE_ALIGN);
        let subordinate = (self.next_bus - 1) as u8;
        let abs = |b: u16| (self.first + b) as u32;
        self.cfg.write(bus, dev, func, 0x18, reg | abs(bus as u16) | abs(secondary as u16) << 8 | abs(subordinate as u16) << 16);
        // I/O window closed (base above limit).
        let io = self.cfg.read(bus, dev, func, 0x1C) & 0xFFFF_0000;
        self.cfg.write(bus, dev, func, 0x1C, io | 0x00F0);
        self.cfg.write(bus, dev, func, 0x30, 0x0000_FFFF);
        // Memory window: 1 MiB units in bits 31:20 of base and limit.
        let (mem_end, pref_end) = (self.mem.next, self.pref.next);
        let mem = if self.mem_32bit { window_word(mem_start, mem_end) } else { window_word(0, 0) };
        self.cfg.write(bus, dev, func, 0x20, mem);
        // Prefetchable window, 64-bit.
        self.cfg.write(bus, dev, func, 0x24, window_word(pref_start, pref_end) | 0x0001_0001);
        if pref_end > pref_start {
            self.cfg.write(bus, dev, func, 0x28, (pref_start >> 32) as u32);
            self.cfg.write(bus, dev, func, 0x2C, ((pref_end - 1) >> 32) as u32);
        } else {
            self.cfg.write(bus, dev, func, 0x28, 0xFFFF_FFFF);
            self.cfg.write(bus, dev, func, 0x2C, 0);
        }
        Ok(())
    }
}

/// Base/limit register word for [start, end); closed (base > limit) if empty.
fn window_word(start: u64, end: u64) -> u32 {
    if end <= start { return 0x0000_FFF0; }
    let base = ((start >> 16) as u32) & 0xFFF0;
    let limit = (((end - 1) >> 16) as u32) & 0xFFF0;
    base | limit << 16
}
