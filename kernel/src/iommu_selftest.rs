// SPDX-License-Identifier: GPL-3.0-only
//! BOOT proof that DMA isolation holds, using QEMU's `edu` teaching device
//! (1234:11e8), which copies between its own 4 KiB buffer and memory by DMA.
//! Runs only when that device is present, i.e. in the IOMMU gate.
//!
//! 1. Unleased: the device has no domain, so its write must not land.
//! 2. Translated: device address 1 MiB reads a page that sits elsewhere in RAM,
//!    and 2 MiB writes another; the data must arrive through the domain.
//! 3. Read-only: a page lent for reading must not be written.
//! 4. Unmapped: an address outside the domain must not be reached.
use sex_iommu::table::{READ, WRITE};
use sex_iommu::Bdf;

use crate::serial_println;

const VENDOR: u16 = 0x1234;
const DEVICE: u16 = 0x11e8;
const BUFFER: u64 = 0x40000;
const SRC: usize = 0x80;
const DST: usize = 0x88;
const CNT: usize = 0x90;
const CMD: usize = 0x98;
const START: u64 = 1;
const TO_RAM: u64 = 2;
const PATTERN: u8 = 0x5a;
const SENTINEL: u8 = 0xa5;

struct Edu { va: u64 }

impl Edu {
    fn w(&self, off: usize, v: u64) { unsafe { core::ptr::write_volatile((self.va + off as u64) as *mut u64, v) } }
    fn r(&self, off: usize) -> u64 { unsafe { core::ptr::read_volatile((self.va + off as u64) as *const u64) } }
    /// One DMA of a page; true when the device reports it finished.
    fn copy(&self, src: u64, dst: u64, to_ram: bool) -> bool {
        self.w(SRC, src);
        self.w(DST, dst);
        self.w(CNT, 4096);
        self.w(CMD, START | if to_ram { TO_RAM } else { 0 });
        // edu runs its DMA from a 100 ms virtual-clock timer.
        let start = unsafe { core::arch::x86_64::_rdtsc() };
        while self.r(CMD) & START != 0 {
            if unsafe { core::arch::x86_64::_rdtsc() } - start > 20_000_000_000 { return false; }
            core::hint::spin_loop();
        }
        true
    }
}

fn page(fill: u8) -> u64 {
    let frame = crate::boot_memory::backing(4096);
    let hhdm = crate::HHDM_REQUEST.response().unwrap().offset;
    unsafe { core::ptr::write_bytes((frame + hhdm) as *mut u8, fill, 4096); }
    frame
}

fn all(frame: u64, value: u8) -> bool {
    let hhdm = crate::HHDM_REQUEST.response().unwrap().offset;
    (0..4096).all(|i| unsafe { core::ptr::read_volatile((frame + hhdm + i) as *const u8) } == value)
}

pub fn run() {
    let Some(dev) = crate::hal::pci::enumerate_bus().into_iter().find(|d| d.vendor_id == VENDOR && d.device_id == DEVICE) else { return };
    let bdf = Bdf::new(dev.bus, dev.dev, dev.func);
    let Some((bar, _)) = dev.probe_boot_bar0() else { serial_println!("[iommu.selftest] ok=0 reason=edu_bar"); return };
    let va = {
        use x86_64::structures::paging::PageTableFlags as F;
        let va = crate::memory::va_allocator::allocate_va(4096).expect("IOMMU_TEST_VA");
        crate::memory::manager::GLOBAL_VAS.lock().as_mut().unwrap()
            .map_physical_range(x86_64::VirtAddr::new(va), bar, 4096, F::PRESENT | F::WRITABLE | F::NO_CACHE | F::NO_EXECUTE, 0)
            .expect("IOMMU_TEST_MAP");
        va
    };
    let edu = Edu { va };
    let command = unsafe { crate::hal::pci::pci_config_read(dev.bus, dev.dev, dev.func, 0x04) } & 0xffff;
    unsafe { crate::hal::pci::pci_config_write(dev.bus, dev.dev, dev.func, 0x04, command | 0x6); }

    let source = page(PATTERN);
    let zeros = page(0);
    let target = page(0);
    let readonly = page(SENTINEL);
    let outside = page(SENTINEL);
    let result = crate::iommu::with_unit(bdf, |t| {
        // 1. No domain yet: the device reaches nothing, even "its own" address.
        let wrote = edu.copy(BUFFER, outside & 0x0fff_ffff, true);
        let unleased = t.faults_from(bdf).len();
        let blocked_unleased = wrote && unleased > 0 && all(outside, SENTINEL);

        let Some(table) = t.table() else { return None };
        let mapped = t.map(&table, 0x10_0000, source, READ) && t.map(&table, 0x20_0000, target, READ | WRITE)
            && t.map(&table, 0x30_0000, zeros, READ) && t.map(&table, 0x40_0000, readonly, READ);
        if !mapped || !t.attach(bdf, Some(&table)) { return None; }
        // 2. Read through the domain, then write through it.
        let translated = edu.copy(0x10_0000, BUFFER, false) && edu.copy(BUFFER, 0x20_0000, true) && all(target, PATTERN);
        let clean = t.faults_from(bdf).is_empty();
        // 3. Device buffer back to zeros, then try writing a read-only page.
        let _ = edu.copy(0x30_0000, BUFFER, false);
        let _ = edu.copy(BUFFER, 0x40_0000, true);
        let readonly_faults = t.faults_from(bdf);
        let read_only_held = all(readonly, SENTINEL) && readonly_faults.iter().any(|f| f.write);
        // 4. An address the domain does not map.
        let _ = edu.copy(BUFFER, 0x50_0000, true);
        let unmapped_blocked = !t.faults_from(bdf).is_empty();
        // Back to no domain: the device is unleased again.
        t.attach(bdf, None);
        Some((blocked_unleased, unleased, translated && clean, read_only_held, unmapped_blocked))
    }).flatten();
    unsafe { crate::hal::pci::pci_config_write(dev.bus, dev.dev, dev.func, 0x04, command & !0x4); }
    match result {
        Some((a, n, b, c, d)) => { serial_println!(
            "[iommu.selftest] device={:02x}:{:02x}.{} blocked_unleased={} unleased_faults={} translated={} read_only_held={} unmapped_blocked={} ok={}",
            dev.bus, dev.dev, dev.func, a as u8, n, b as u8, c as u8, d as u8, (a && b && c && d) as u8); }
        None => { serial_println!("[iommu.selftest] device={:02x}:{:02x}.{} ok=0 reason=setup", dev.bus, dev.dev, dev.func); }
    }
}
