// SPDX-License-Identifier: GPL-3.0-only
//! Device leases and their message-signalled interrupts (stage D3).
//!
//! BOOT records every device lease devmgr makes, and for a lease whose manifest wants
//! interrupts it programs the device's MSI-X table (or MSI capability) with vectors from
//! `FIRST_VECTOR..FIRST_VECTOR + VECTORS`, aimed at the boot core, every vector masked.
//! The device's legacy INTx line is turned off at the same time.
//!
//! An interrupt sets one bit in the owning service's pending word and wakes its main
//! task; nothing is queued, so a storm costs a bit, not ring space. The driver unmasks
//! the vectors it is ready for and takes the bits through syscall 86 (`SYS_DEVICE`).
//! Nothing here is created after BOOT: the syscall only reads records, flips mask
//! bits of vectors BOOT already gave the caller, and clears the caller's own bits.
//!
//! Each vector is aimed at the core of the task that unmasks it, so a driver homed
//! away from core 0 takes its own interrupts without an IPI hop; `OP_STEER` aims one
//! vector at any core (one queue per core). `OP_BIND` makes a vector signal an IPC v2
//! notification the caller holds instead of the main task, so each per-core worker
//! sleeps on its own queue's interrupt.
//!
//! Lines are the same bits fed by an I/O APIC input instead of a message: BOOT reserves
//! the vector and bit (`sex_hwdb::manifest::LINES`), the service binds the input firmware
//! named once, and a level line is masked here each time it fires (the driver unmasks
//! it after quieting the device), so a wire stuck asserted cannot storm.
//!
//! Without interrupt remapping (D4) a driver that can DMA can also forge an MSI
//! write; that is the same trust a DMA-capable driver already has before the IOMMU.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use sex_pdx::device::{Lease, Mode, MAX_LEASES, MAX_LINES, MAX_VECTORS_PER_SERVICE, ERR_NONE, OP_BIND, OP_LINE_BIND, OP_LINE_MASK, OP_LIST, OP_MASK, OP_STEER, OP_TAKE, OP_UNMASK, STEER_HERE, LINE_ACTIVE_LOW, LINE_EDGE};
use x86_64::structures::idt::InterruptStackFrame;

pub const FIRST_VECTOR: u8 = 0x50;
pub const VECTORS: usize = 64;

const CAP_LIST: u32 = 1 << 20;
const CAP_MSI: u8 = 0x05;
const CAP_MSIX: u8 = 0x11;
const INTX_DISABLE: u32 = 1 << 10;

/// One service's pending bits, by domain id.
static PENDING: [AtomicU64; crate::ipc::MAX_DOMAINS] = [const { AtomicU64::new(0) }; crate::ipc::MAX_DOMAINS];
/// Next free pending bit, by domain id.
static NEXT_BIT: [AtomicU32; crate::ipc::MAX_DOMAINS] = [const { AtomicU32::new(0) }; crate::ipc::MAX_DOMAINS];
/// vector index → valid << 40 | bit << 32 | domain id.
static ROUTE: [AtomicU64; VECTORS] = [const { AtomicU64::new(0) }; VECTORS];
/// Interrupts delivered, by vector index.
static FIRED: [AtomicU64; VECTORS] = [const { AtomicU64::new(0) }; VECTORS];
static NEXT_VECTOR: AtomicU32 = AtomicU32::new(0);
/// vector index → 1 << 63 | notification bit << 48 | pool entry << 32 | object id (OP_BIND).
static BOUND: [AtomicU64; VECTORS] = [const { AtomicU64::new(0) }; VECTORS];
/// The bound notification's object generation (BOUND names its id).
static BOUND_GENERATION: [AtomicU32; VECTORS] = [const { AtomicU32::new(0) }; VECTORS];

/// A line BOOT reserved: owner, vector index, pending bit, target core; the bound input.
#[derive(Clone, Copy)]
struct Line { pd: u32, vector: usize, bit: u8, apic: u32, gsi: Option<u32> }
static LINES: spin::Mutex<[Option<Line>; 8]> = spin::Mutex::new([None; 8]);
/// vector index → bound << 63 | level << 62 | redirection entry << 32 | I/O APIC address.
static LINE_RTE: [AtomicU64; VECTORS] = [const { AtomicU64::new(0) }; VECTORS];
/// The I/O APIC's index/data pair is one shared window; interrupts are off while held.
static IOAPIC: spin::Mutex<()> = spin::Mutex::new(());

fn ioapic(address: u32, register: u32, write: Option<u32>) -> u32 {
    let base = crate::HHDM_REQUEST.response().map_or(0, |h| h.offset) + address as u64;
    unsafe {
        (base as *mut u32).write_volatile(register);
        let window = (base + 0x10) as *mut u32;
        match write { Some(v) => { window.write_volatile(v); v } None => window.read_volatile() }
    }
}

/// The I/O APIC address and redirection entry for `gsi`, by each one's own entry count.
/// Caller holds `IOAPIC` with interrupts off.
fn line_entry(gsi: u32) -> Option<(u32, u32)> {
    crate::apic::IO_APICS.lock().iter().find_map(|io| {
        let entries = (ioapic(io.address, 1, None) >> 16 & 0xff) + 1;
        let i = gsi.checked_sub(io.global_system_interrupt_base)?;
        (i < entries).then_some((io.address, i))
    })
}

fn set_line_mask(rte: u64, masked: bool) {
    let (address, low) = (rte as u32, 0x10 + 2 * (rte >> 32 & 0xff) as u32);
    let _held = IOAPIC.lock();
    let v = ioapic(address, low, None);
    ioapic(address, low, Some(if masked { v | 1 << 16 } else { v & !(1 << 16) }));
}

/// BOOT: reserve `n` lines for a service holding a device that may need one.
pub fn reserve_lines(pd: u32, n: u8) {
    let mut lines = LINES.lock();
    for _ in 0..(n as usize).min(MAX_LINES) {
        let (Some(free), Some((vector, bit))) = (lines.iter_mut().find(|l| l.is_none()), allocate(pd, 1)) else { break };
        *free = Some(Line { pd, vector, bit, apic: crate::apic::local_id(), gsi: None });
        crate::serial_println!("[device.irq.line.reserved] pd={} vector={:#x} bit={}", pd, FIRST_VECTOR as usize + vector, bit);
    }
}

fn line_call(pd: u32, op: u64, index: u64, arg: u64) -> u64 {
    let mut lines = LINES.lock();
    let gsi = (arg & 0xffff) as u32;
    let taken = lines.iter().flatten().any(|l| l.gsi == Some(gsi));
    let Some(line) = lines.iter_mut().flatten().filter(|l| l.pd == pd).nth(index as usize) else { return sex_pdx::ERR_CAP_INVALID };
    let rte = LINE_RTE[line.vector].load(Ordering::Acquire);
    if op == OP_LINE_MASK {
        if rte >> 63 == 0 { return sex_pdx::ERR_CAP_INVALID; }
        x86_64::instructions::interrupts::without_interrupts(|| set_line_mask(rte, arg & 1 != 0));
        return 0;
    }
    // ISA inputs (timer, keyboard, SCI) are the kernel's; one owner per input.
    if line.gsi.is_some() || gsi < 16 || taken { return sex_pdx::ERR_CAP_INVALID; }
    let level = arg & LINE_EDGE == 0;
    let low = (FIRST_VECTOR as u32 + line.vector as u32) | (level as u32) << 15 | ((arg & LINE_ACTIVE_LOW != 0) as u32) << 13 | 1 << 16;
    let found = x86_64::instructions::interrupts::without_interrupts(|| {
        let _held = IOAPIC.lock();
        let (address, entry) = line_entry(gsi)?;
        ioapic(address, 0x10 + 2 * entry + 1, Some((line.apic & 0xff) << 24));
        ioapic(address, 0x10 + 2 * entry, Some(low));
        Some((address, entry))
    });
    let Some((address, entry)) = found else { return sex_pdx::ERR_CAP_INVALID };
    line.gsi = Some(gsi);
    LINE_RTE[line.vector].store(1 << 63 | (level as u64) << 62 | (entry as u64) << 32 | address as u64, Ordering::Release);
    crate::serial_println!("[device.irq.line.bound] pd={} gsi={} level={} active_low={} vector={:#x} bit={} masked=1",
        pd, gsi, level as u8, (arg & LINE_ACTIVE_LOW != 0) as u8, FIRST_VECTOR as usize + line.vector, line.bit);
    line.bit as u64
}

#[derive(Clone, Copy)]
struct Record {
    pd: u32,
    lease: Lease,
    /// Index of the device's first vector in `ROUTE`.
    vector: usize,
    /// Kernel-only mapping of the MSI-X table (Mode::MsiX).
    table: u64,
    /// MSI capability offset, its 64-bit flag and per-vector masking (Mode::Msi).
    msi: u8,
    msi_wide: bool,
    msi_masking: bool,
}

static RECORDS: spin::Mutex<[Option<Record>; MAX_LEASES]> = spin::Mutex::new([None; MAX_LEASES]);

fn byte(dev: &crate::hal::pci::PciDevice, at: u8) -> u8 { (dev.read_u32(at & !3) >> ((at & 3) * 8)) as u8 }

/// Offsets of the MSI and MSI-X capabilities, if the device has them.
pub(crate) fn capabilities(dev: &crate::hal::pci::PciDevice) -> (Option<u8>, Option<u8>) {
    let (mut msi, mut msix) = (None, None);
    if dev.read_u32(0x04) & CAP_LIST == 0 { return (None, None); }
    let mut at = byte(dev, 0x34) & !3;
    // At most 48 capabilities fit in config space; a longer chain is a loop.
    for _ in 0..48 {
        if at < 0x40 { break; }
        match byte(dev, at) { CAP_MSI => msi = msi.or(Some(at)), CAP_MSIX => msix = msix.or(Some(at)), _ => {} }
        at = byte(dev, at + 1) & !3;
    }
    (msi, msix)
}

/// MSI address for fixed delivery to one local APIC, and the data word for a vector.
pub(crate) fn address(apic: u32) -> u32 { 0xFEE0_0000 | (apic & 0xff) << 12 }

fn allocate(pd: u32, want: u32) -> Option<(usize, u8)> {
    let bit = NEXT_BIT[pd as usize].load(Ordering::Relaxed);
    let vector = NEXT_VECTOR.load(Ordering::Relaxed) as usize;
    if want == 0 || bit + want > MAX_VECTORS_PER_SERVICE || vector + want as usize > VECTORS { return None; }
    NEXT_BIT[pd as usize].store(bit + want, Ordering::Relaxed);
    NEXT_VECTOR.store((vector + want as usize) as u32, Ordering::Relaxed);
    for i in 0..want as usize {
        ROUTE[vector + i].store(1 << 40 | ((bit as u64 + i as u64) << 32) | pd as u64, Ordering::Release);
    }
    Some((vector, bit as u8))
}

/// BOOT: remember a lease devmgr just granted, and give it up to `want` vectors.
pub fn record(pd: u32, slot: u64, dev: &crate::hal::pci::PciDevice, want: u8) {
    // Behind Intel VMD, MSI-X goes through VMD's own table (`vmd::route`); plain MSI
    // is not routed there, so such a device polls.
    let behind_vmd = dev.ecam != 0;
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire), crate::ipc::BootPhase::RegistryBuild as u8, "DEVICE_IRQ_AFTER_GENESIS");
    let mut lease = Lease { slot, vendor: dev.vendor_id, device: dev.device_id,
        class: (dev.class_id as u32) << 16 | (dev.subclass_id as u32) << 8 | dev.prog_if as u32,
        bus: dev.bus, dev: dev.dev, func: dev.func, mode: Mode::None, vectors: 0, first_bit: 0 };
    let mut record = Record { pd, lease, vector: 0, table: 0, msi: 0, msi_wide: false, msi_masking: false };
    let (msi, msix) = if want == 0 { (None, None) } else { capabilities(dev) };
    let msi = if behind_vmd { None } else { msi };
    let apic = crate::apic::local_id();
    if let Some(cap) = msix {
        let control = dev.read_u32(cap) >> 16;
        let size = (control & 0x7ff) + 1;
        let place = dev.read_u32(cap + 4);
        let (bir, offset) = ((place & 7) as u8, (place & !7) as u64);
        let bar = dev.probe_boot_bar(bir);
        match (bar, allocate(pd, (want as u32).min(size))) {
            (Some((physical, length)), Some((vector, bit))) if offset + size as u64 * 16 <= length => {
                let n = (want as u32).min(size);
                // Behind VMD the entries name VMD vectors, which point at ours.
                let via = if behind_vmd {
                    match crate::vmd::route(dev, n as u16, FIRST_VECTOR + vector as u8, apic) {
                        Some(first) => Some(first),
                        None => { crate::serial_println!("[device.irq.skip] pd={} slot={} reason=vmd_vectors polling=1", pd, slot); return keep(record); }
                    }
                } else { None };
                // Function-masked while the table is written; enabled first so INTx stays off.
                dev.write_u32(cap, dev.read_u32(cap) | 0xC000 << 16);
                let table_pa = physical + offset;
                let page = table_pa & !0xfff;
                let bytes = ((table_pa - page) + size as u64 * 16 + 0xfff) & !0xfff;
                let table = crate::boot_memory::kernel_mmio(page, bytes) + (table_pa - page);
                for i in 0..size as u64 {
                    let entry = (table + i * 16) as *mut u32;
                    unsafe {
                        entry.add(3).write_volatile(1);
                        if i < n as u64 {
                            let (addr, data) = match via {
                                Some(first) => sex_vmd::child_msi(first + i as u16),
                                None => (address(apic), (FIRST_VECTOR as usize + vector + i as usize) as u32),
                            };
                            entry.write_volatile(addr);
                            entry.add(1).write_volatile(0);
                            entry.add(2).write_volatile(data);
                        }
                    }
                }
                dev.write_u32(cap, (dev.read_u32(cap) | 0x8000 << 16) & !(0x4000 << 16));
                dev.write_u32(0x04, (dev.read_u32(0x04) & 0xffff) | INTX_DISABLE);
                record.table = table; record.vector = vector;
                lease.mode = Mode::MsiX; lease.vectors = n as u8; lease.first_bit = bit;
                crate::serial_println!("[device.irq.msix] pd={} slot={} {:02x}:{:02x}.{} table_size={} vectors={} first_vector={:#x} first_bit={} apic={} masked=1 vmd_vector={}",
                    pd, slot, dev.bus, dev.dev, dev.func, size, n, FIRST_VECTOR as usize + vector, bit, apic, via.map(|v| v as i32).unwrap_or(-1));
            }
            _ => { crate::serial_println!("[device.irq.skip] pd={} slot={} reason=msix_table_or_vectors bir={} offset={:#x}", pd, slot, bir, offset); }
        }
    } else if let Some(cap) = msi {
        if let Some((vector, bit)) = allocate(pd, 1) {
            let control = dev.read_u32(cap) >> 16;
            let (wide, masking) = (control & 0x80 != 0, control & 0x100 != 0);
            // One vector (MME=0), disabled while written.
            dev.write_u32(cap, dev.read_u32(cap) & !(0x71 << 16));
            dev.write_u32(cap + 4, address(apic));
            let data_at = if wide { dev.write_u32(cap + 8, 0); cap + 12 } else { cap + 8 };
            dev.write_u32(data_at, FIRST_VECTOR as u32 + vector as u32);
            if masking {
                dev.write_u32(data_at + 4, dev.read_u32(data_at + 4) | 1);
                dev.write_u32(cap, dev.read_u32(cap) | 1 << 16);
                dev.write_u32(0x04, (dev.read_u32(0x04) & 0xffff) | INTX_DISABLE);
            }
            record.msi = cap; record.msi_wide = wide; record.msi_masking = masking; record.vector = vector;
            lease.mode = Mode::Msi; lease.vectors = 1; lease.first_bit = bit;
            crate::serial_println!("[device.irq.msi] pd={} slot={} {:02x}:{:02x}.{} vector={:#x} bit={} wide={} masking={} apic={} masked=1",
                pd, slot, dev.bus, dev.dev, dev.func, FIRST_VECTOR as usize + vector, bit, wide as u8, masking as u8, apic);
        }
    } else if want != 0 {
        crate::serial_println!("[device.irq.skip] pd={} slot={} reason=no_msi_capability polling=1", pd, slot);
    }
    record.lease = lease;
    keep(record);
}

fn keep(record: Record) {
    let (pd, slot) = (record.pd, record.lease.slot);
    let mut records = RECORDS.lock();
    match records.iter_mut().find(|r| r.is_none()) {
        Some(free) => *free = Some(record),
        None => { crate::serial_println!("[device.lease.unrecorded] pd={} slot={} reason=table_full", pd, slot); }
    }
}

/// Whether domain `pd` holds any device lease: a driver service.
pub fn holds_lease(pd: u32) -> bool { RECORDS.lock().iter().flatten().any(|r| r.pd == pd) }

fn find(pd: u32, slot: u64) -> Option<Record> {
    RECORDS.lock().iter().flatten().find(|r| r.pd == pd && r.lease.slot == slot).copied()
}

fn config(l: &Lease) -> crate::hal::pci::PciDevice {
    crate::hal::pci::PciDevice { bus: l.bus, dev: l.dev, func: l.func, vendor_id: l.vendor, device_id: l.device,
        class_id: (l.class >> 16) as u8, subclass_id: (l.class >> 8) as u8, prog_if: l.class as u8, ecam: 0 }
}

/// Points vector `index` at local APIC `apic`. Only entries that name our own vector
/// directly are moved: one routed through another table (Intel VMD) stays put.
fn aim(record: &Record, index: u64, apic: u32) -> bool {
    if index >= record.lease.vectors as u64 { return false; }
    let ours = (FIRST_VECTOR as usize + record.vector + index as usize) as u32;
    match record.lease.mode {
        Mode::MsiX => unsafe {
            let entry = (record.table + index * 16) as *mut u32;
            if entry.add(2).read_volatile() != ours { return false; }
            let masked = entry.add(3).read_volatile();
            // Masked while the address changes, so no message goes out half-written.
            entry.add(3).write_volatile(masked | 1);
            entry.write_volatile(address(apic));
            entry.add(3).write_volatile(masked);
        },
        Mode::Msi => {
            let dev = config(&record.lease);
            let data_at = if record.msi_wide { record.msi + 12 } else { record.msi + 8 };
            if dev.read_u32(data_at) & 0xffff != ours { return false; }
            dev.write_u32(record.msi + 4, address(apic));
        }
        Mode::None => return false,
    }
    true
}

fn set_mask(record: &Record, index: u64, masked: bool) -> bool {
    if index >= record.lease.vectors as u64 { return false; }
    let l = &record.lease;
    match l.mode {
        Mode::MsiX => unsafe { ((record.table + index * 16 + 12) as *mut u32).write_volatile(masked as u32); },
        Mode::Msi => {
            let dev = config(l);
            let cap = record.msi;
            if record.msi_masking {
                let at = if record.msi_wide { cap + 16 } else { cap + 12 };
                let bits = dev.read_u32(at);
                dev.write_u32(at, if masked { bits | 1 } else { bits & !1 });
            } else {
                // No per-vector mask: the enable bit is the mask, and INTx goes off with it.
                let control = dev.read_u32(cap);
                dev.write_u32(cap, if masked { control & !(1 << 16) } else { control | 1 << 16 });
                if !masked { dev.write_u32(0x04, (dev.read_u32(0x04) & 0xffff) | INTX_DISABLE); }
            }
        }
        Mode::None => return false,
    }
    true
}

/// Syscall 86. rdi=op; see `sex_pdx::device`.
pub fn syscall(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let pd = crate::core_local::CoreLocal::get().current_pd_ref().id;
    let (op, a, b) = (regs.rdi, regs.rsi, regs.rdx);
    regs.rsi = 0; regs.rdx = 0; regs.r10 = 0;
    match op {
        OP_LIST => {
            let records = RECORDS.lock();
            let Some(r) = records.iter().flatten().filter(|r| r.pd == pd).nth(a.min(MAX_LEASES as u64) as usize) else { return ERR_NONE; };
            let w = r.lease.pack();
            regs.rsi = w[1]; regs.rdx = w[2]; regs.r10 = w[3];
            w[0]
        }
        OP_UNMASK | OP_MASK => match find(pd, a) {
            Some(r) if b < r.lease.vectors as u64 => {
                // The unmasking task is the one that will wait: its core takes the interrupt.
                let here = op == OP_UNMASK && aim(&r, b, crate::apic::local_id());
                if !set_mask(&r, b, op == OP_MASK) { return sex_pdx::ERR_CAP_INVALID; }
                if op == OP_UNMASK {
                    crate::serial_println!("[device.irq.unmask] pd={} slot={} vector_index={} mode={} core={} aimed={}",
                        pd, a, b, r.lease.mode.name(), crate::core_local::CoreLocal::get().core_id, here as u8);
                }
                0
            }
            _ => sex_pdx::ERR_CAP_INVALID,
        },
        // rsi=slot, rdx=vector index | core << 16 (STEER_HERE: the caller's core).
        OP_STEER => {
            let (index, core) = (b & 0xffff, (b >> 16) as u32);
            let core = if core == STEER_HERE { crate::core_local::CoreLocal::get().core_id } else { core };
            match (find(pd, a), crate::smp::local_apic_of(core)) {
                (Some(r), Some(apic)) if aim(&r, index, apic) => {
                    crate::serial_println!("[device.irq.steer] pd={} slot={} vector_index={} core={} apic={}", pd, a, index, core, apic);
                    0
                }
                _ => sex_pdx::ERR_CAP_INVALID,
            }
        }
        // rsi=slot, rdx=vector index | notification bit << 16 | notification slot << 32.
        // A notification slot of 0xffff_ffff unbinds: the vector wakes the main task again.
        OP_BIND => {
            let (index, bit, nslot) = (b & 0xffff, (b >> 16) & 0xff, (b >> 32) as u32);
            let Some(r) = find(pd, a) else { return sex_pdx::ERR_CAP_INVALID };
            if index >= r.lease.vectors as u64 || bit >= 64 { return sex_pdx::ERR_CAP_INVALID; }
            let at = r.vector + index as usize;
            if nslot == u32::MAX { BOUND[at].store(0, Ordering::Release); return 0; }
            match crate::ipc_v2::syscall::signal_target(nslot) {
                Ok((entry, object, generation)) => {
                    BOUND_GENERATION[at].store(generation, Ordering::Release);
                    BOUND[at].store(1 << 63 | bit << 48 | (entry as u64) << 32 | object as u64, Ordering::Release);
                    crate::serial_println!("[device.irq.bind] pd={} slot={} vector_index={} notification_slot={} bit={}", pd, a, index, nslot, bit);
                    0
                }
                Err(e) => e,
            }
        }
        OP_TAKE => PENDING[pd as usize].swap(0, Ordering::AcqRel),
        OP_LINE_BIND | OP_LINE_MASK => line_call(pd, op, a, b),
        _ => sex_pdx::ERR_CAP_INVALID,
    }
}

/// Whether this domain has interrupt bits it has not taken; a wait returns at once.
pub fn pending(pd: u32) -> bool {
    (pd as usize) < PENDING.len() && PENDING[pd as usize].load(Ordering::Acquire) != 0
}

/// How many interrupts BOOT-programmed vectors have delivered, for diagnostics.
pub fn fired(index: usize) -> u64 { FIRED[index].load(Ordering::Relaxed) }

fn fire(index: usize) {
    // A level line stays asserted until the driver quiets its device: hold it masked.
    let rte = LINE_RTE[index].load(Ordering::Acquire);
    if rte >> 62 == 3 { set_line_mask(rte, true); }
    let route = ROUTE[index].load(Ordering::Acquire);
    let bound = BOUND[index].load(Ordering::Acquire);
    if bound >> 63 != 0 {
        FIRED[index].fetch_add(1, Ordering::Relaxed);
        if crate::ipc_v2::syscall::signal_bound((bound >> 32) as u16 as usize, bound as u32, BOUND_GENERATION[index].load(Ordering::Acquire), 1 << (bound >> 48 & 63)) {
            unsafe { crate::interrupts::send_eoi(); }
            return;
        }
        // The notification is gone: the vector wakes the main task again.
        let _ = BOUND[index].compare_exchange(bound, 0, Ordering::AcqRel, Ordering::Relaxed);
    }
    if route >> 40 & 1 != 0 {
        let (pd, bit) = (route as u32, (route >> 32) as u8 & 63);
        PENDING[pd as usize].fetch_or(1 << bit, Ordering::AcqRel);
        let n = FIRED[index].fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 { crate::serial_println!("[device.irq.first] vector={:#x} pd={} bit={}", FIRST_VECTOR as usize + index, pd, bit); }
        if let Some(target) = crate::ipc::DOMAIN_REGISTRY.get(pd) {
            let main = target.main_task.load(Ordering::Acquire);
            if !main.is_null() { crate::scheduler::unpark_thread(main); }
        }
    }
    unsafe { crate::interrupts::send_eoi(); }
}

macro_rules! handlers {
    ($($n:literal)*) => {
        /// One entry per vector: an interrupt handler cannot ask which vector it is.
        pub static HANDLERS: [extern "x86-interrupt" fn(InterruptStackFrame); VECTORS] = [$({
            extern "x86-interrupt" fn h(_frame: InterruptStackFrame) { fire($n) }
            h
        }),*];
    };
}
handlers!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
          32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63);
