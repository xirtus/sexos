// SPDX-License-Identifier: GPL-3.0-only
//! ACPI power: shut down (S5), restart, the power button, power sensors and CPU idle and
//! performance states.
//!
//! Fixed hardware comes straight from the FADT and is driven by small, bounds-checked code
//! here, so shutting down and restarting never depend on AML. The kernel never runs AML:
//! the ACPI service (`servers/sexacpi`, ring 3) runs the interpreter for what only AML can
//! tell (`\_S5` when it differs from the table bytes, `\_PTS`, batteries, lids, thermal
//! zones, `_CST`, I2C HID devices). It reads the tables and reaches the hardware the
//! firmware names through the firmware window below, which refuses RAM, the kernel's own
//! ports and PCI BARs, and it reports back what it found. If it crashes or hangs, only it
//! stops; shutting down, restarting and the power button keep working.
//!
//! The authority to shut down is a `Power` capability (`sex_pdx::power`); see `invoke`.

use crate::serial_println;
use acpi::address::{AddressSpace, GenericAddress};
use acpi::sdt::fadt::Fadt;
use acpi::{AcpiTables, Handle, PciAddress, PhysicalMapping};
use alloc::vec::Vec;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use sex_pdx::power as w;
use x86_64::instructions::port::Port;

/// The IDT vector the SCI is routed to.
pub const SCI_VECTOR: u8 = 0x29;

static HHDM: AtomicU64 = AtomicU64::new(0);

// ── Hardware access for the tables ──────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct Hw;

fn phys<T>(address: usize) -> *mut T { (HHDM.load(Ordering::Relaxed) + address as u64) as *mut T }

fn pci_ecam(address: PciAddress, offset: u16) -> Option<usize> {
    let regions = crate::apic::PCI_REGIONS.try_lock()?;
    let r = regions.iter().find(|r| r.segment == address.segment() && (r.bus_start..=r.bus_end).contains(&address.bus()))?;
    Some(r.phys_addr + (((address.bus() - r.bus_start) as usize) << 20 | (address.device() as usize) << 15 | (address.function() as usize) << 12) + offset as usize)
}
fn pci_read(address: PciAddress, offset: u16, bytes: u8) -> u32 {
    if let Some(at) = pci_ecam(address, offset) {
        return unsafe { match bytes { 1 => phys::<u8>(at).read_volatile() as u32, 2 => phys::<u16>(at).read_volatile() as u32, _ => phys::<u32>(at).read_volatile() } };
    }
    if address.segment() != 0 || offset >= 256 { return u32::MAX; }
    // Legacy config ports only through the HAL's locked accessor.
    let word = unsafe { crate::hal::pci::pci_config_read(address.bus(), address.device(), address.function(), (offset & 0xFC) as u8) };
    let shift = (offset & 3) * 8;
    match bytes { 1 => (word >> shift) & 0xFF, 2 => (word >> shift) & 0xFFFF, _ => word }
}
fn pci_write(address: PciAddress, offset: u16, bytes: u8, value: u32) {
    if let Some(at) = pci_ecam(address, offset) {
        unsafe { match bytes { 1 => phys::<u8>(at).write_volatile(value as u8), 2 => phys::<u16>(at).write_volatile(value as u16), _ => phys::<u32>(at).write_volatile(value) } }
        return;
    }
    if address.segment() != 0 || offset >= 256 { return; }
    let (b, d, f, at) = (address.bus(), address.device(), address.function(), (offset & 0xFC) as u8);
    let shift = (offset & 3) * 8;
    let mask = match bytes { 1 => 0xFFu32, 2 => 0xFFFF, _ => u32::MAX };
    unsafe {
        let old = if bytes == 4 { 0 } else { crate::hal::pci::pci_config_read(b, d, f, at) };
        crate::hal::pci::pci_config_write(b, d, f, at, (old & !(mask << shift)) | ((value & mask) << shift));
    }
}
/// About a microsecond: a write to the POST port is an ISA bus cycle.
pub(crate) fn stall_us(us: u64) { for _ in 0..us { unsafe { Port::<u8>::new(0x80).write(0) } } }


impl acpi::Handler for Hw {
    unsafe fn map_physical_region<T>(&self, physical_address: usize, size: usize) -> PhysicalMapping<Self, T> {
        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: NonNull::new(phys::<T>(physical_address)).unwrap(),
            region_length: size,
            mapped_length: size,
            handler: *self,
        }
    }
    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
    fn read_u8(&self, a: usize) -> u8 { unsafe { phys::<u8>(a).read_volatile() } }
    fn read_u16(&self, a: usize) -> u16 { unsafe { phys::<u16>(a).read_volatile() } }
    fn read_u32(&self, a: usize) -> u32 { unsafe { phys::<u32>(a).read_volatile() } }
    fn read_u64(&self, a: usize) -> u64 { unsafe { phys::<u64>(a).read_volatile() } }
    fn write_u8(&self, a: usize, v: u8) { unsafe { phys::<u8>(a).write_volatile(v) } }
    fn write_u16(&self, a: usize, v: u16) { unsafe { phys::<u16>(a).write_volatile(v) } }
    fn write_u32(&self, a: usize, v: u32) { unsafe { phys::<u32>(a).write_volatile(v) } }
    fn write_u64(&self, a: usize, v: u64) { unsafe { phys::<u64>(a).write_volatile(v) } }
    fn read_io_u8(&self, p: u16) -> u8 { unsafe { Port::new(p).read() } }
    fn read_io_u16(&self, p: u16) -> u16 { unsafe { Port::new(p).read() } }
    fn read_io_u32(&self, p: u16) -> u32 { unsafe { Port::new(p).read() } }
    fn write_io_u8(&self, p: u16, v: u8) { unsafe { Port::new(p).write(v) } }
    fn write_io_u16(&self, p: u16, v: u16) { unsafe { Port::new(p).write(v) } }
    fn write_io_u32(&self, p: u16, v: u32) { unsafe { Port::new(p).write(v) } }
    fn read_pci_u8(&self, a: PciAddress, o: u16) -> u8 { pci_read(a, o, 1) as u8 }
    fn read_pci_u16(&self, a: PciAddress, o: u16) -> u16 { pci_read(a, o, 2) as u16 }
    fn read_pci_u32(&self, a: PciAddress, o: u16) -> u32 { pci_read(a, o, 4) }
    fn write_pci_u8(&self, a: PciAddress, o: u16, v: u8) { pci_write(a, o, 1, v as u32) }
    fn write_pci_u16(&self, a: PciAddress, o: u16, v: u16) { pci_write(a, o, 2, v as u32) }
    fn write_pci_u32(&self, a: PciAddress, o: u16, v: u32) { pci_write(a, o, 4, v) }
    fn nanos_since_boot(&self) -> u64 { crate::interrupts::TICKS.load(Ordering::Relaxed).saturating_mul(1_000_000) }
    fn stall(&self, us: u64) { stall_us(us.min(100_000)) }
    fn sleep(&self, ms: u64) { stall_us(ms.min(2_000) * 1000) }
    // Table parsing only. acpi 6.1.1 needs its "aml" feature to build with "alloc", so these
    // exist, but the kernel never constructs an interpreter: sexacpi runs AML in ring 3.
    fn create_mutex(&self) -> Handle { Handle(0) }
    fn acquire(&self, _: Handle, _: u16) -> Result<(), acpi::aml::AmlError> { Ok(()) }
    fn release(&self, _: Handle) {}
}

// ── Generic address registers ───────────────────────────────────────────────

fn gas_bits(g: &GenericAddress) -> u8 {
    match g.access_size { 1 => 8, 2 => 16, 3 => 32, 4 => 64, _ => match g.bit_width { 0..=8 => 8, 9..=16 => 16, 17..=32 => 32, _ => 64 } }
}
fn gas_read(g: &GenericAddress) -> Option<u64> {
    if g.address == 0 { return None; }
    let bits = gas_bits(g);
    Some(unsafe {
        match g.address_space {
            AddressSpace::SystemIo => match bits { 8 => Port::<u8>::new(g.address as u16).read() as u64, 16 => Port::<u16>::new(g.address as u16).read() as u64, _ => Port::<u32>::new(g.address as u16).read() as u64 },
            AddressSpace::SystemMemory => match bits { 8 => phys::<u8>(g.address as usize).read_volatile() as u64, 16 => phys::<u16>(g.address as usize).read_volatile() as u64, 32 => phys::<u32>(g.address as usize).read_volatile() as u64, _ => phys::<u64>(g.address as usize).read_volatile() },
            AddressSpace::PciConfigSpace => {
                let a = PciAddress::new(0, 0, (g.address >> 32) as u8, (g.address >> 16) as u8);
                pci_read(a, g.address as u16, (bits / 8).min(4)) as u64
            }
            _ => return None,
        }
    })
}
fn gas_write(g: &GenericAddress, v: u64) -> bool {
    if g.address == 0 { return false; }
    let bits = gas_bits(g);
    unsafe {
        match g.address_space {
            AddressSpace::SystemIo => match bits { 8 => Port::<u8>::new(g.address as u16).write(v as u8), 16 => Port::<u16>::new(g.address as u16).write(v as u16), _ => Port::<u32>::new(g.address as u16).write(v as u32) },
            AddressSpace::SystemMemory => match bits { 8 => phys::<u8>(g.address as usize).write_volatile(v as u8), 16 => phys::<u16>(g.address as usize).write_volatile(v as u16), 32 => phys::<u32>(g.address as usize).write_volatile(v as u32), _ => phys::<u64>(g.address as usize).write_volatile(v) },
            AddressSpace::PciConfigSpace => {
                let a = PciAddress::new(0, 0, (g.address >> 32) as u8, (g.address >> 16) as u8);
                pci_write(a, g.address as u16, (bits / 8).min(4), v as u32);
            }
            _ => return false,
        }
    }
    true
}
/// A sub-register of a block: `bytes` wide at `offset`.
fn gas_part(g: &GenericAddress, offset: u64, bytes: u8) -> GenericAddress {
    GenericAddress { address_space: g.address_space, bit_width: bytes * 8, bit_offset: 0, access_size: match bytes { 1 => 1, 2 => 2, _ => 3 }, address: g.address + offset }
}
fn gas_label(g: &Option<GenericAddress>) -> (&'static str, u64) {
    match g {
        None => ("none", 0),
        Some(g) => (match g.address_space { AddressSpace::SystemIo => "io", AddressSpace::SystemMemory => "mmio", AddressSpace::PciConfigSpace => "pci", _ => "other" }, g.address),
    }
}

// ── What the tables say ────────────────────────────────────────────────────

struct Fixed {
    pm1a_evt: Option<GenericAddress>,
    pm1b_evt: Option<GenericAddress>,
    /// Bytes in each half (status, enable) of a PM1 event block.
    pm1_half: u8,
    pm1a_cnt: Option<GenericAddress>,
    pm1b_cnt: Option<GenericAddress>,
    sleep_cnt: Option<GenericAddress>,
    reset: Option<(GenericAddress, u8)>,
    gpe: [(Option<GenericAddress>, u8); 2],
    sci: u16,
    hw_reduced: bool,
    power_button_fixed: bool,
}
static FIXED: spin::Once<Fixed> = spin::Once::new();
/// SLP_TYPa / SLP_TYPb for S5: from the DSDT bytes, or as the ACPI service read `\_S5`.
static S5: AtomicU32 = AtomicU32::new(u32::MAX);
static INFO: spin::Mutex<w::Info> = spin::Mutex::new(w::Info { flags: 0, idle_state: 1, idle_latency_us: 1, batteries: 0, lids: 0, thermal_zones: 0, i2c_hid: 0 });

const PM1_PWRBTN: u64 = 1 << 8;
const PM1_WAK: u64 = 1 << 15;
const PM1_SCI_EN: u64 = 1;
const PM1_SLP_EN: u64 = 1 << 13;

fn pm1_status_read(f: &Fixed) -> u64 {
    let mut v = 0;
    for g in [f.pm1a_evt, f.pm1b_evt].iter().flatten() { v |= gas_read(&gas_part(g, 0, f.pm1_half)).unwrap_or(0); }
    v
}
fn pm1_status_clear(f: &Fixed, bits: u64) {
    for g in [f.pm1a_evt, f.pm1b_evt].iter().flatten() { gas_write(&gas_part(g, 0, f.pm1_half), bits); }
}
fn pm1_enable(f: &Fixed, bits: u64) {
    for g in [f.pm1a_evt, f.pm1b_evt].iter().flatten() {
        let en = gas_part(g, f.pm1_half as u64, f.pm1_half);
        let old = gas_read(&en).unwrap_or(0);
        gas_write(&en, old | bits);
    }
}

// ── The firmware window (the ACPI service) ─────────────────────────────────

/// The domain holding the `FIRMWARE` right; told `OP_PREPARE` so it can run `\_PTS`.
static FIRMWARE_PD: AtomicU32 = AtomicU32::new(0);
pub fn set_firmware(pd: u32) { FIRMWARE_PD.store(pd, Ordering::Release); }
/// FADT, DSDT, then each SSDT: (physical address, length, signature).
static TABLES: spin::Once<Vec<(u64, u32, u32)>> = spin::Once::new();
/// Words 1..=3 of the last `Sensors` the service reported.
static SENSOR_WORDS: spin::Mutex<Option<[u64; 3]>> = spin::Mutex::new(None);

/// Limine memory map types the window never touches: usable RAM, bootloader-reclaimable,
/// kernel and modules, framebuffer. ACPI-reclaimable (the tables) is read only.
fn memory_refused(address: u64, length: u64, write: bool) -> bool {
    let Some(map) = crate::MEMMAP_REQUEST.response() else { return true };
    let end = match address.checked_add(length) { Some(e) => e, None => return true };
    map.entries().iter().any(|e| {
        let t = e.type_ as u64;
        let refused = matches!(t, 0 | 5 | 6 | 7) || (write && t == 2);
        refused && address < e.base + e.length && end > e.base
    })
}
/// Whether the HHDM maps `address` (4 KiB, 2 MiB or 1 GiB leaf).
/// Whether HHDM + `address` is mapped *to physical `address`*. The HHDM is a fixed offset,
/// so any other result means `address` aliases a non-HHDM kernel mapping (code, heap,
/// stacks) and must be refused.
fn mapped(address: u64) -> bool {
    use x86_64::registers::control::Cr3;
    if address >= 1 << 52 { return false; }
    let hhdm = HHDM.load(Ordering::Relaxed);
    let Some(va) = hhdm.checked_add(address) else { return false };
    let mut table = Cr3::read().0.start_address().as_u64();
    for shift in [39u64, 30, 21, 12] {
        let entry = unsafe { core::ptr::read_volatile((hhdm + table + ((va >> shift) & 511) * 8) as *const u64) };
        if entry & 1 == 0 { return false; }
        if shift == 12 || (shift != 39 && entry & (1 << 7) != 0) {
            let size = 1u64 << shift;
            let frame = entry & 0x000f_ffff_ffff_f000 & !(size - 1);
            return frame + (va & (size - 1)) == address;
        }
        table = entry & 0x000f_ffff_ffff_f000;
    }
    false
}
fn width_ok(width: u64) -> Option<u8> { matches!(width, 1 | 2 | 4 | 8).then_some(width as u8) }

fn fw_table(index: u64, offset: u64) -> [u64; 5] {
    let Some(&(address, length, signature)) = TABLES.get().and_then(|t| t.get(index as usize)) else { return [w::INVALID, 0, 0, 0, 0] };
    let mut words = [0u64; 3];
    for (i, word) in words.iter_mut().enumerate() {
        let mut bytes = [0u8; 8];
        for (j, b) in bytes.iter_mut().enumerate() {
            let at = offset + (i * 8 + j) as u64;
            if at < length as u64 { *b = unsafe { phys::<u8>((address + at) as usize).read_volatile() }; }
        }
        *word = u64::from_le_bytes(bytes);
    }
    [w::OK, length as u64 | (signature as u64) << 32, words[0], words[1], words[2]]
}
fn fw_io(arg: u64, value: u64) -> [u64; 5] {
    let (port, write) = (arg as u16, (arg >> 24) & 1 != 0);
    let Some(width) = width_ok((arg >> 16) & 0xFF).filter(|w| *w <= 4) else { return [w::INVALID, 0, 0, 0, 0] };
    if w::port_reserved(port, width) { return [w::DENIED, 0, 0, 0, 0]; }
    let v = unsafe {
        match (width, write) {
            (1, false) => Port::<u8>::new(port).read() as u64,
            (2, false) => Port::<u16>::new(port).read() as u64,
            (4, false) => Port::<u32>::new(port).read() as u64,
            (1, true) => { Port::<u8>::new(port).write(value as u8); 0 }
            (2, true) => { Port::<u16>::new(port).write(value as u16); 0 }
            _ => { Port::<u32>::new(port).write(value as u32); 0 }
        }
    };
    [w::OK, v, 0, 0, 0]
}
fn fw_mem(address: u64, mode: u64, value: u64) -> [u64; 5] {
    let write = (mode >> 8) & 1 != 0;
    let Some(width) = width_ok(mode & 0xFF) else { return [w::INVALID, 0, 0, 0, 0] };
    if address % width as u64 != 0 { return [w::INVALID, 0, 0, 0, 0]; }
    if memory_refused(address, width as u64, write) || !mapped(address) || !mapped(address + width as u64 - 1) { return [w::DENIED, 0, 0, 0, 0]; }
    let a = address as usize;
    let v = unsafe {
        match (width, write) {
            (1, false) => phys::<u8>(a).read_volatile() as u64,
            (2, false) => phys::<u16>(a).read_volatile() as u64,
            (4, false) => phys::<u32>(a).read_volatile() as u64,
            (8, false) => phys::<u64>(a).read_volatile(),
            (1, true) => { phys::<u8>(a).write_volatile(value as u8); 0 }
            (2, true) => { phys::<u16>(a).write_volatile(value as u16); 0 }
            (4, true) => { phys::<u32>(a).write_volatile(value as u32); 0 }
            _ => { phys::<u64>(a).write_volatile(value); 0 }
        }
    };
    [w::OK, v, 0, 0, 0]
}
fn fw_pci(arg: u64, mode: u64, value: u64) -> [u64; 5] {
    let write = (mode >> 8) & 1 != 0;
    let Some(width) = width_ok(mode & 0xFF).filter(|w| *w <= 4) else { return [w::INVALID, 0, 0, 0, 0] };
    let offset = (arg & 0xFFF) as u16;
    if offset as u32 + width as u32 > 4096 || offset % width as u16 != 0 { return [w::INVALID, 0, 0, 0, 0]; }
    if write && w::pci_write_reserved(offset, width) { return [w::DENIED, 0, 0, 0, 0]; }
    let a = PciAddress::new((arg >> 32) as u16, (arg >> 24) as u8, ((arg >> 19) & 31) as u8, ((arg >> 16) & 7) as u8);
    if write { pci_write(a, offset, width, value as u32); [w::OK, 0, 0, 0, 0] } else { [w::OK, pci_read(a, offset, width) as u64, 0, 0, 0] }
}
/// The service's inventory: counts for `INFO`, `\_S5` as AML reads it.
fn fw_inventory(counts: u64, s5: u64, failed: u64) -> [u64; 5] {
    let mut info = INFO.lock();
    if failed != 0 {
        info.flags = (info.flags & !w::AML) | w::AML_QUARANTINED;
        serial_println!("[acpi.firmware.inventory] aml=0 quarantined=1");
        return [w::OK, 0, 0, 0, 0];
    }
    info.batteries = counts as u8; info.lids = (counts >> 8) as u8;
    info.thermal_zones = (counts >> 16) as u8; info.i2c_hid = (counts >> 24) as u8;
    if (counts >> 32) & 1 != 0 { info.flags |= w::AML; }
    if s5 != u64::MAX { S5.store((s5 & 7) as u32 | (((s5 >> 8) & 7) as u32) << 8, Ordering::Release); }
    let can_off = S5.load(Ordering::Acquire) != u32::MAX && FIXED.get().map_or(false, |f| f.pm1a_cnt.is_some() || f.sleep_cnt.is_some());
    if can_off { info.flags |= w::CAN_OFF; }
    serial_println!("[acpi.firmware.inventory] aml={} batteries={} lids={} thermal={} i2c_hid={} s5={:#x}",
        (info.flags & w::AML != 0) as u8, info.batteries, info.lids, info.thermal_zones, info.i2c_hid, S5.load(Ordering::Acquire));
    [w::OK, 0, 0, 0, 0]
}
/// The C-state the service chose from `_CST`: used only where MONITOR/MWAIT exist and the
/// local APIC timer keeps running in deep states (ARAT), and only C2 and deeper.
fn fw_idle(arg: u64) -> [u64; 5] {
    let (kind, latency, hint) = (arg as u8, (arg >> 8) as u16, (arg >> 32) as u32);
    let monitor = cpuid(1, 0).ecx & (1 << 3) != 0;
    let arat = max_leaf() >= 6 && cpuid(6, 0).eax & (1 << 2) != 0;
    if !(monitor && arat && kind >= 2 && latency <= 200 && hint <= 0xFF) { return [w::INVALID, 0, 0, 0, 0]; }
    MWAIT_HINT.store(hint, Ordering::Release);
    let mut info = INFO.lock();
    info.idle_state = kind; info.idle_latency_us = latency; info.flags |= w::MWAIT_IDLE;
    serial_println!("[acpi.firmware.idle] c={} latency_us={} hint={:#x}", kind, latency, hint);
    [w::OK, 0, 0, 0, 0]
}

fn this_core() -> u32 {
    if crate::core_local::INITIALIZED.load(Ordering::Acquire) { crate::core_local::CoreLocal::get().core_id } else { 0 }
}

// ── CPU performance and idle ───────────────────────────────────────────────

static MWAIT_HINT: AtomicU32 = AtomicU32::new(u32::MAX);
#[repr(align(64))]
struct Line([AtomicU64; 8]);
static MONITOR_LINES: [Line; 64] = [const { Line([const { AtomicU64::new(0) }; 8]) }; 64];

fn cpuid(leaf: u32, sub: u32) -> core::arch::x86_64::CpuidResult { unsafe { core::arch::x86_64::__cpuid_count(leaf, sub) } }
fn max_leaf() -> u32 { cpuid(0, 0).eax }
fn max_ext_leaf() -> u32 { cpuid(0x8000_0000, 0).eax }

/// Hands frequency choice to the CPU: Intel HWP or AMD CPPC, balanced bias. Per core.
pub fn cpu_init_local() -> u64 {
    use x86_64::registers::model_specific::Msr;
    let mut flags = 0;
    if max_leaf() >= 6 {
        let eax = cpuid(6, 0).eax;
        if eax & (1 << 7) != 0 {
            unsafe {
                Msr::new(0x770).write(1); // IA32_PM_ENABLE
                let caps = Msr::new(0x771).read();
                let (highest, lowest) = (caps & 0xFF, (caps >> 24) & 0xFF);
                let epp = if eax & (1 << 10) != 0 { 0x80u64 << 24 } else { 0 };
                Msr::new(0x774).write(lowest | highest << 8 | epp); // IA32_HWP_REQUEST, desired = auto
            }
            flags |= w::HWP;
        }
    }
    if flags == 0 && max_ext_leaf() >= 0x8000_0008 && cpuid(0x8000_0008, 0).ebx & (1 << 27) != 0 {
        unsafe {
            Msr::new(0xC001_02B1).write(1); // CPPC_ENABLE
            let caps = Msr::new(0xC001_02B0).read();
            let (highest, lowest) = ((caps >> 24) & 0xFF, caps & 0xFF);
            Msr::new(0xC001_02B3).write(highest | lowest << 8 | 0x80 << 24); // max, min, desired auto, EPP
        }
        flags |= w::CPPC;
    }
    flags
}

/// The scheduler's idle: MWAIT into the chosen C-state, or HLT (C1). Interrupts wake it.
pub fn idle() {
    let hint = MWAIT_HINT.load(Ordering::Relaxed);
    if hint == u32::MAX {
        x86_64::instructions::interrupts::enable_and_hlt();
        return;
    }
    let line = &MONITOR_LINES[(this_core() as usize) % 64].0[0] as *const AtomicU64;
    unsafe {
        core::arch::asm!("monitor", in("rax") line, in("ecx") 0, in("edx") 0, options(nostack));
        core::arch::asm!("sti", "mwait", in("eax") hint, in("ecx") 1, options(nostack));
    }
}

// ── Bring-up ──────────────────────────────────────────────────────────────

pub fn init(rsdp: u64, hhdm: u64) {
    HHDM.store(hhdm, Ordering::Release);
    let tables = match unsafe { AcpiTables::from_rsdp(Hw, rsdp as usize) } {
        Ok(t) => t,
        Err(e) => { serial_println!("[acpi.power] tables=0 error={:?}", e); return; }
    };
    let Some(fadt) = tables.find_table::<Fadt>() else { serial_println!("[acpi.power] fadt=0"); return };
    let flags = { fadt.flags };
    let reset = if flags.supports_system_reset_via_fadt() || fadt.header.revision >= 2 {
        fadt.reset_register().ok().filter(|g| g.address != 0).map(|g| (g, fadt.reset_value))
    } else { None };
    let fixed = Fixed {
        pm1a_evt: fadt.pm1a_event_block().ok().filter(|g| g.address != 0),
        pm1b_evt: fadt.pm1b_event_block().ok().flatten().filter(|g| g.address != 0),
        pm1_half: (fadt.pm1_event_length / 2).max(1),
        pm1a_cnt: fadt.pm1a_control_block().ok().filter(|g| g.address != 0),
        pm1b_cnt: fadt.pm1b_control_block().ok().flatten().filter(|g| g.address != 0),
        sleep_cnt: fadt.sleep_control_register().ok().flatten().filter(|g| g.address != 0),
        reset,
        gpe: [(fadt.gpe0_block().ok().flatten(), fadt.gpe0_block_length), (fadt.gpe1_block().ok().flatten(), fadt.gpe1_block_length)],
        sci: fadt.sci_interrupt,
        hw_reduced: flags.system_is_hw_reduced_acpi(),
        power_button_fixed: !flags.power_button_is_control_method(),
    };
    let (smi, enable) = (fadt.smi_cmd_port, fadt.acpi_enable);

    // \_S5 from the DSDT bytes: shut down works even if AML cannot run.
    if let Ok(dsdt) = tables.dsdt() {
        let header = core::mem::size_of::<acpi::sdt::SdtHeader>();
        if dsdt.length as usize > header {
            let bytes = unsafe { core::slice::from_raw_parts(phys::<u8>(dsdt.phys_address + header), dsdt.length as usize - header) };
            if let Some((a, b)) = w::s5_from_dsdt(bytes) { S5.store(a as u32 | (b as u32) << 8, Ordering::Release); }
        }
    }
    drop(fadt);

    // ACPI mode: firmware hands the fixed events to us and raises the SCI for them.
    if !fixed.hw_reduced {
        let sci_en = fixed.pm1a_cnt.as_ref().and_then(gas_read).map_or(false, |v| v & PM1_SCI_EN != 0);
        if !sci_en && smi != 0 && enable != 0 {
            unsafe { Port::<u8>::new(smi as u16).write(enable) };
            for _ in 0..3000 {
                if fixed.pm1a_cnt.as_ref().and_then(gas_read).map_or(false, |v| v & PM1_SCI_EN != 0) { break; }
                stall_us(100);
            }
        }
        // No general purpose events until there is code for them: an armed GPE with
        // no handler would hold the level-triggered SCI asserted.
        for (block, len) in fixed.gpe.iter() {
            let (Some(g), half) = (block, *len / 2) else { continue };
            if g.address_space != AddressSpace::SystemIo { continue; }
            for i in 0..half as u64 {
                gas_write(&gas_part(g, half as u64 + i, 1), 0);
                gas_write(&gas_part(g, i, 1), 0xFF);
            }
        }
        pm1_status_clear(&fixed, 0xFFFF);
        if fixed.power_button_fixed { pm1_enable(&fixed, PM1_PWRBTN); }
        // The SCI is level triggered, active low, unless the MADT says otherwise.
        if (fixed.sci as usize) < 256 {
            let mut iso = crate::apic::ISO_OVERRIDES.lock();
            if iso[fixed.sci as usize].is_none() {
                iso[fixed.sci as usize] = Some(crate::apic::IsoOverride { isa_source: fixed.sci as u8, gsi: fixed.sci as u32, active_low: true, level_triggered: true });
            }
        }
        unsafe { crate::apic::map_irq(fixed.sci as u8, SCI_VECTOR, 0, x86_64::VirtAddr::new(hhdm)); }
    }
    let sci_en = fixed.pm1a_cnt.as_ref().and_then(gas_read).map_or(0, |v| v & PM1_SCI_EN);
    let (rk, ra) = gas_label(&fixed.reset.map(|r| r.0));
    let (ck, ca) = gas_label(&fixed.pm1a_cnt);
    FIXED.call_once(|| fixed);

    // The AML tables, for the ACPI service: FADT, DSDT, then the SSDTs.
    let mut list = Vec::new();
    if let Some(fadt) = tables.find_table::<Fadt>() { list.push((fadt.physical_start as u64, fadt.region_length as u32, u32::from_le_bytes(*b"FACP"))); }
    if let Ok(dsdt) = tables.dsdt() { list.push((dsdt.phys_address as u64, dsdt.length, u32::from_le_bytes(*b"DSDT"))); }
    for ssdt in tables.ssdts().take(64) { list.push((ssdt.phys_address as u64, ssdt.length, u32::from_le_bytes(*b"SSDT"))); }
    let table_count = list.len();
    TABLES.call_once(|| list);
    let cpu_flags = cpu_init_local();
    let s5 = S5.load(Ordering::Acquire);
    let f = FIXED.get().unwrap();
    let mut info = INFO.lock();
    info.flags |= cpu_flags
        | if s5 != u32::MAX && (f.pm1a_cnt.is_some() || f.sleep_cnt.is_some()) { w::CAN_OFF } else { 0 }
        | w::CAN_RESTART
        | if f.power_button_fixed && !f.hw_reduced { w::POWER_BUTTON } else { 0 }
        | if f.hw_reduced { w::HW_REDUCED } else { 0 };
    serial_println!("[acpi.power] s5={} slp_typ_a={} slp_typ_b={} pm1_cnt={}:{:#x} reset={}:{:#x} sci={} sci_en={} power_button={} hw_reduced={} aml_tables={} aml_in_kernel=0 hwp={} cppc={}",
        (info.flags & w::CAN_OFF != 0) as u8, s5 & 0xFF, (s5 >> 8) & 0xFF, ck, ca, rk, ra, f.sci, sci_en,
        (info.flags & w::POWER_BUTTON != 0) as u8, f.hw_reduced as u8, table_count,
        (cpu_flags & w::HWP != 0) as u8, (cpu_flags & w::CPPC != 0) as u8);
}

// ── Shutting down and restarting ───────────────────────────────────────────

/// 0 idle, 3 setting up a request, 1 waiting for Files, 2 going down.
static STATE: AtomicU8 = AtomicU8::new(0);
static KIND: AtomicU64 = AtomicU64::new(0);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
static FLUSHED: AtomicU8 = AtomicU8::new(0);
/// The domain told before power changes (Files), set by BOOT with its grant.
static FLUSHER: AtomicU32 = AtomicU32::new(0);

pub fn set_flusher(pd: u32) { FLUSHER.store(pd, Ordering::Release); }

/// Starts a shut down or restart. Files is told first; `poll` finishes it.
pub fn begin(kind: u64, why: &str) -> bool {
    // Claim first, publish "waiting" only once kind and deadline are set: another
    // core's `poll` must never see a zero deadline or a stale kind.
    if STATE.compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire).is_err() { return false; }
    KIND.store(kind, Ordering::Release);
    let now = crate::interrupts::TICKS.load(Ordering::Relaxed);
    DEADLINE.store(now + w::FLUSH_TICKS, Ordering::Release);
    STATE.store(1, Ordering::Release);
    let what = if kind == w::RESTART { "restart" } else { "off" };
    let files = FLUSHER.load(Ordering::Acquire);
    let told = files != 0 && crate::ipc::DOMAIN_REGISTRY.get(files).map_or(false, |pd| {
        let msg = crate::ipc::messages::MessageType::IpcCall { func_id: w::OP_PREPARE, arg0: kind, arg1: 0, arg2: 0, caller_pd: 0, caller_task: 0 };
        let ok = unsafe { (*pd.message_ring).enqueue(msg) }.is_ok();
        if ok { crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire)); }
        ok
    });
    let firmware = FIRMWARE_PD.load(Ordering::Acquire);
    let pts = firmware != 0 && crate::ipc::DOMAIN_REGISTRY.get(firmware).map_or(false, |pd| {
        let msg = crate::ipc::messages::MessageType::IpcCall { func_id: w::OP_PREPARE, arg0: kind, arg1: 0, arg2: 0, caller_pd: 0, caller_task: 0 };
        let ok = unsafe { (*pd.message_ring).enqueue(msg) }.is_ok();
        if ok { crate::scheduler::unpark_thread(pd.main_task.load(Ordering::Acquire)); }
        ok
    });
    serial_println!("[acpi.power.begin] kind={} by={} files={} told={} firmware_told={}", what, why, files, told as u8, pts as u8);
    if !told { FLUSHED.store(2, Ordering::Release); }
    true
}

/// Timer and syscall paths: finish once Files has answered or its time is up.
pub fn poll() {
    if STATE.load(Ordering::Acquire) != 1 { return; }
    let late = crate::interrupts::TICKS.load(Ordering::Relaxed) >= DEADLINE.load(Ordering::Acquire);
    if FLUSHED.load(Ordering::Acquire) == 0 && !late { return; }
    if STATE.compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire).is_err() { return; }
    let flushed = FLUSHED.load(Ordering::Acquire);
    serial_println!("[acpi.power.flush] files={} late={}", match flushed { 1 => "flushed", 2 => "absent", 3 => "failed", _ => "no_answer" }, late as u8);
    if KIND.load(Ordering::Acquire) == w::RESTART { restart() } else { power_off() }
}

/// The power button (SCI). Runs in the interrupt handler, before EOI.
pub fn sci() {
    let Some(f) = FIXED.get() else { return };
    let status = pm1_status_read(f);
    if status & PM1_PWRBTN != 0 {
        pm1_status_clear(f, PM1_PWRBTN);
        serial_println!("[acpi.power_button] pressed=1");
        begin(w::OFF, "power_button");
    }
    if status & PM1_WAK != 0 { pm1_status_clear(f, PM1_WAK); }
}

fn stop_everything() {
    x86_64::instructions::interrupts::disable();
}

pub fn power_off() -> ! {
    let f = FIXED.get();
    let s5 = S5.load(Ordering::Acquire);
    // \_PTS ran in the ACPI service when it was told OP_PREPARE.
    serial_println!("[acpi.power.off] s5={:#x} entering=1", s5);
    stop_everything();
    if let (Some(f), true) = (f, s5 != u32::MAX) {
        let (a, b) = ((s5 & 7) as u64, ((s5 >> 8) & 7) as u64);
        pm1_status_clear(f, PM1_WAK);
        if let Some(g) = &f.sleep_cnt {
            gas_write(g, (a << 2) | (1 << 5));
        } else {
            for (g, typ) in [(f.pm1a_cnt, a), (f.pm1b_cnt, b)] {
                let Some(g) = g else { continue };
                let v = gas_read(&g).unwrap_or(0) & !(7 << 10 | PM1_SLP_EN);
                gas_write(&g, v | typ << 10);
            }
            for (g, typ) in [(f.pm1a_cnt, a), (f.pm1b_cnt, b)] {
                let Some(g) = g else { continue };
                let v = gas_read(&g).unwrap_or(0) & !(7 << 10 | PM1_SLP_EN);
                gas_write(&g, v | typ << 10 | PM1_SLP_EN);
            }
        }
        stall_us(200_000);
    }
    serial_println!("[acpi.power.off] failed=1 halted=1 reason={}", if s5 == u32::MAX { "no_s5" } else { "still_running" });
    loop { x86_64::instructions::hlt(); }
}

pub fn restart() -> ! {
    serial_println!("[acpi.power.restart] entering=1");
    stop_everything();
    if let Some((g, value)) = FIXED.get().and_then(|f| f.reset) {
        serial_println!("[acpi.power.restart] method=fadt_reset_register");
        gas_write(&g, value as u64);
        stall_us(50_000);
    }
    unsafe {
        serial_println!("[acpi.power.restart] method=port_cf9");
        Port::<u8>::new(0xCF9).write(0x02);
        stall_us(10);
        Port::<u8>::new(0xCF9).write(0x06);
        stall_us(50_000);
        serial_println!("[acpi.power.restart] method=keyboard_controller");
        for _ in 0..10_000 { if Port::<u8>::new(0x64).read() & 2 == 0 { break; } stall_us(10); }
        Port::<u8>::new(0x64).write(0xFE);
        stall_us(50_000);
        serial_println!("[acpi.power.restart] method=triple_fault");
        let empty = x86_64::structures::DescriptorTablePointer { limit: 0, base: x86_64::VirtAddr::new(0) };
        x86_64::instructions::tables::lidt(&empty);
        core::arch::asm!("int3");
    }
    loop { x86_64::instructions::hlt(); }
}

// ── Sensors ────────────────────────────────────────────────────────────────

fn read_sensors() -> [u64; 5] {
    match *SENSOR_WORDS.lock() {
        Some(words) => [w::OK, words[0], words[1], words[2], 0],
        None => w::Sensors::default().encode(),
    }
}

// ── The Power capability (syscall 84) ──────────────────────────────────────

pub fn invoke(pd: &crate::capability::ProtectionDomain, slot: u64, op: u64, arg: u64, arg2: u64, arg3: u64) -> [u64; 5] {
    let denied = [w::DENIED, 0, 0, 0, 0];
    let Some(slot) = crate::capability_slot::decode_capability_slot(slot) else { return denied };
    let Some(crate::capability::CapabilityData::Power(grant)) = pd.find_capability(slot).map(|c| c.data) else { return denied };
    if grant.holder_pd != pd.id { return denied; }
    let has = |right: u8| grant.rights & right != 0;
    match op {
        w::INFO => INFO.lock().encode(),
        w::OFF | w::RESTART if has(w::REQUEST) => {
            let info = INFO.lock().flags;
            if op == w::OFF && info & w::CAN_OFF == 0 { return [w::UNAVAILABLE, 0, 0, 0, 0]; }
            if begin(op, "shell") { [w::OK, 0, 0, 0, 0] } else { [w::BUSY, 0, 0, 0, 0] }
        }
        w::READY if has(w::FLUSH) => {
            if pd.id != FLUSHER.load(Ordering::Acquire) || STATE.load(Ordering::Acquire) != 1 { return [w::INVALID, 0, 0, 0, 0]; }
            FLUSHED.store(if arg != 0 { 1 } else { 3 }, Ordering::Release);
            poll();
            [w::OK, 0, 0, 0, 0]
        }
        w::SENSORS if has(w::SENSE) || has(w::REQUEST) => read_sensors(),
        w::FW_TABLE if has(w::FIRMWARE) => fw_table(arg, arg2),
        w::FW_IO if has(w::FIRMWARE) => fw_io(arg, arg2),
        w::FW_MEM if has(w::FIRMWARE) => fw_mem(arg, arg2, arg3),
        w::FW_PCI if has(w::FIRMWARE) => fw_pci(arg, arg2, arg3),
        w::FW_INVENTORY if has(w::FIRMWARE) => fw_inventory(arg, arg2, arg3),
        w::FW_IDLE if has(w::FIRMWARE) => fw_idle(arg),
        w::FW_DGPU if has(w::FIRMWARE) => super::dgpu::firmware_op(arg, arg2, arg3),
        w::FW_SENSORS if has(w::FIRMWARE) => {
            let first = SENSOR_WORDS.lock().replace([arg, arg2, arg3]).is_none();
            if first { serial_println!("[acpi.firmware.sensors] first=1 words={:#x},{:#x},{:#x}", arg, arg2, arg3); }
            [w::OK, 0, 0, 0, 0]
        }
        w::OFF | w::RESTART | w::READY | w::SENSORS => denied,
        w::FW_TABLE | w::FW_IO | w::FW_MEM | w::FW_PCI | w::FW_INVENTORY | w::FW_IDLE | w::FW_SENSORS | w::FW_DGPU => denied,
        _ => [w::INVALID, 0, 0, 0, 0],
    }
}
