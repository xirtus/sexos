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
//! Without interrupt remapping (D4) a driver that can DMA can also forge an MSI
//! write; that is the same trust a DMA-capable driver already has before the IOMMU.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use sex_pdx::device::{Lease, Mode, MAX_LEASES, MAX_VECTORS_PER_SERVICE, ERR_NONE, OP_LIST, OP_MASK, OP_TAKE, OP_UNMASK};
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

#[derive(Clone, Copy)]
struct Record {
    pd: u32,
    lease: Lease,
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
fn capabilities(dev: &crate::hal::pci::PciDevice) -> (Option<u8>, Option<u8>) {
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
fn address(apic: u32) -> u32 { 0xFEE0_0000 | (apic & 0xff) << 12 }

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
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire), crate::ipc::BootPhase::RegistryBuild as u8, "DEVICE_IRQ_AFTER_GENESIS");
    let mut lease = Lease { slot, vendor: dev.vendor_id, device: dev.device_id,
        class: (dev.class_id as u32) << 16 | (dev.subclass_id as u32) << 8 | dev.prog_if as u32,
        bus: dev.bus, dev: dev.dev, func: dev.func, mode: Mode::None, vectors: 0, first_bit: 0 };
    let mut record = Record { pd, lease, table: 0, msi: 0, msi_wide: false, msi_masking: false };
    let (msi, msix) = if want == 0 { (None, None) } else { capabilities(dev) };
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
                            entry.write_volatile(address(apic));
                            entry.add(1).write_volatile(0);
                            entry.add(2).write_volatile((FIRST_VECTOR as usize + vector + i as usize) as u32);
                        }
                    }
                }
                dev.write_u32(cap, (dev.read_u32(cap) | 0x8000 << 16) & !(0x4000 << 16));
                dev.write_u32(0x04, (dev.read_u32(0x04) & 0xffff) | INTX_DISABLE);
                record.table = table;
                lease.mode = Mode::MsiX; lease.vectors = n as u8; lease.first_bit = bit;
                crate::serial_println!("[device.irq.msix] pd={} slot={} {:02x}:{:02x}.{} table_size={} vectors={} first_vector={:#x} first_bit={} apic={} masked=1",
                    pd, slot, dev.bus, dev.dev, dev.func, size, n, FIRST_VECTOR as usize + vector, bit, apic);
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
            record.msi = cap; record.msi_wide = wide; record.msi_masking = masking;
            lease.mode = Mode::Msi; lease.vectors = 1; lease.first_bit = bit;
            crate::serial_println!("[device.irq.msi] pd={} slot={} {:02x}:{:02x}.{} vector={:#x} bit={} wide={} masking={} apic={} masked=1",
                pd, slot, dev.bus, dev.dev, dev.func, FIRST_VECTOR as usize + vector, bit, wide as u8, masking as u8, apic);
        }
    } else if want != 0 {
        crate::serial_println!("[device.irq.skip] pd={} slot={} reason=no_msi_capability polling=1", pd, slot);
    }
    record.lease = lease;
    let mut records = RECORDS.lock();
    match records.iter_mut().find(|r| r.is_none()) {
        Some(free) => *free = Some(record),
        None => { crate::serial_println!("[device.lease.unrecorded] pd={} slot={} reason=table_full", pd, slot); }
    }
}

fn find(pd: u32, slot: u64) -> Option<Record> {
    RECORDS.lock().iter().flatten().find(|r| r.pd == pd && r.lease.slot == slot).copied()
}

fn set_mask(record: &Record, index: u64, masked: bool) -> bool {
    if index >= record.lease.vectors as u64 { return false; }
    let l = &record.lease;
    match l.mode {
        Mode::MsiX => unsafe { ((record.table + index * 16 + 12) as *mut u32).write_volatile(masked as u32); },
        Mode::Msi => {
            let dev = crate::hal::pci::PciDevice { bus: l.bus, dev: l.dev, func: l.func, vendor_id: l.vendor, device_id: l.device,
                class_id: (l.class >> 16) as u8, subclass_id: (l.class >> 8) as u8, prog_if: l.class as u8 };
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
            Some(r) if set_mask(&r, b, op == OP_MASK) => {
                if op == OP_UNMASK {
                    crate::serial_println!("[device.irq.unmask] pd={} slot={} vector_index={} mode={}", pd, a, b, r.lease.mode.name());
                }
                0
            }
            _ => sex_pdx::ERR_CAP_INVALID,
        },
        OP_TAKE => PENDING[pd as usize].swap(0, Ordering::AcqRel),
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
    let route = ROUTE[index].load(Ordering::Acquire);
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
