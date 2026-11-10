// SPDX-License-Identifier: GPL-3.0-only
//! DMA isolation through the IOMMU (Intel VT-d and AMD-Vi).
//!
//! Each driver service that holds a device lease gets a DMA domain of its own.
//! Its devices reach exactly: the service's BOOT DMA pool (at its physical
//! address, so drivers keep programming the addresses they always did), the
//! firmware-reserved memory (RMRR) of those devices, and whatever the service
//! lends later through syscall 90. Every other device on a translated unit,
//! leased or not, cannot reach memory at all. Stray DMA becomes a logged
//! fault instead of a write into someone else's memory.
//!
//! BOOT builds every table and the lending window's page tables before the
//! capability graph is sealed; at runtime lending only fills prepared leaves.
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use sex_iommu::amdvi;
use sex_iommu::dmar::Dmar;
use sex_iommu::ivrs::Ivrs;
use sex_iommu::table::{Format, Frames, Table, READ, WRITE};
use sex_iommu::vtd::{self, Cap, Ecap};
use sex_iommu::Fault;
use sex_iommu::window::{self, Window};
use sex_iommu::{Bdf, PAGE};
use sex_pdx::dma_lend as abi;
use spin::Mutex;

use crate::capability::CapabilityData;
use crate::serial_println;

/// Physical RSDP, recorded by early boot.
pub static RSDP: AtomicU64 = AtomicU64::new(0);
static TICKS: AtomicU32 = AtomicU32::new(0);
static FAULTS_LOGGED: AtomicU32 = AtomicU32::new(0);
const FAULT_LOG_LIMIT: u32 = 64;
/// 32768 pages: the 128 MiB lending window of each domain.
const WINDOW_WORDS: usize = (abi::WINDOW_BYTES / PAGE / 64) as usize;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Vendor { Intel, Amd }

impl Vendor {
    fn name(self) -> &'static str { match self { Vendor::Intel => "intel", Vendor::Amd => "amd" } }
    fn format(self) -> Format { match self { Vendor::Intel => Format::Vtd, Vendor::Amd => Format::AmdVi } }
}

struct Unit {
    phys: u64,
    va: u64,
    hhdm: u64,
    enabled: bool,
    hw: Hw,
}

enum Hw {
    Vtd { cap: Cap, ecap: Ecap, root: u64, contexts: [u64; 256] },
    /// `devices` is the device table every AMD unit shares; `blocked` an empty
    /// page table that every device nobody leased points at (domain 0), so its
    /// DMA faults and is logged. `commands` is the ring page, followed by the
    /// completion-wait semaphore page.
    Amd { features: u64, devices: u64, blocked: u64, commands: u64, tail: u64, events: u64 },
}
struct Loan { address: u64, pages: u64 }

struct Domain {
    pd: u32,
    did: u16,
    table: Table,
    window: Window<WINDOW_WORDS>,
    loans: Vec<Loan>,
    faults: u64,
    /// Every unit translating this service's devices is on. Only then do
    /// device addresses from loans mean anything; otherwise LEND is refused.
    active: bool,
    /// Pages whose IOTLB flush did not complete: never handed out again.
    stuck: u64,
}

struct Device { bdf: Bdf, pd: u32, unit: usize }

struct State {
    vendor: Vendor,
    units: Vec<Unit>,
    domains: Vec<Domain>,
    devices: Vec<Device>,
    levels: u8,
    frames: KernelFrames,
    /// Next free domain id (services, firmware-reserved devices, self-test).
    next_did: u16,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Table frames come from one BOOT reservation, sized for the domains found.
struct KernelFrames { hhdm: u64, coherent: bool, base: u64, next: u64, count: u64 }

impl Frames for KernelFrames {
    fn alloc(&mut self) -> Option<u64> {
        // Tables are BOOT memory; the runtime path only fills prepared leaves.
        if crate::ipc::BOOT_PHASE.load(Ordering::Acquire) != crate::ipc::BootPhase::RegistryBuild as u8 { return None; }
        if self.next >= self.count { return None; }
        self.next += 1;
        Some(self.base + (self.next - 1) * PAGE)
    }
    fn read(&self, table: u64, index: usize) -> u64 {
        unsafe { core::ptr::read_volatile((self.hhdm + table + index as u64 * 8) as *const u64) }
    }
    fn write(&mut self, table: u64, index: usize, value: u64) {
        let p = (self.hhdm + table + index as u64 * 8) as *mut u64;
        unsafe { core::ptr::write_volatile(p, value); }
        if !self.coherent { flush(p as *const u8); }
    }
}

fn flush(p: *const u8) {
    unsafe { core::arch::x86_64::_mm_clflush(p); core::arch::x86_64::_mm_mfence(); }
}

impl Unit {
    fn r32(&self, off: usize) -> u32 { unsafe { core::ptr::read_volatile((self.va + off as u64) as *const u32) } }
    fn w32(&self, off: usize, v: u32) { unsafe { core::ptr::write_volatile((self.va + off as u64) as *mut u32, v) } }
    fn r64(&self, off: usize) -> u64 { unsafe { core::ptr::read_volatile((self.va + off as u64) as *const u64) } }
    fn w64(&self, off: usize, v: u64) { unsafe { core::ptr::write_volatile((self.va + off as u64) as *mut u64, v) } }
    fn mem(&self, pa: u64) -> *mut u64 { (self.hhdm + pa) as *mut u64 }

    fn wait(&self, off: usize, mask: u64, set: bool, wide: bool) -> bool {
        for _ in 0..50_000_000u32 {
            let v = if wide { self.r64(off) } else { self.r32(off) as u64 };
            if (v & mask != 0) == set { return true; }
            core::hint::spin_loop();
        }
        false
    }

    fn cap(&self) -> Cap { match self.hw { Hw::Vtd { cap, .. } => cap, Hw::Amd { .. } => Cap(0) } }
    fn ecap(&self) -> Ecap { match self.hw { Hw::Vtd { ecap, .. } => ecap, Hw::Amd { .. } => Ecap(0) } }

    /// Issue a VT-d global command bit and wait for the hardware to show it.
    fn command(&self, bit: u32, on: bool) -> bool {
        let status = self.r32(vtd::GSTS);
        self.w32(vtd::GCMD, vtd::command(status, bit, on));
        self.wait(vtd::GSTS, bit as u64, on, false)
    }

    fn flush_write_buffer(&self) {
        if !self.cap().required_write_buffer_flush() { return; }
        let status = self.r32(vtd::GSTS);
        self.w32(vtd::GCMD, (status & vtd::GSTS_PERSISTENT) | vtd::WBF);
        self.wait(vtd::GSTS, vtd::WBF as u64, false, false);
    }

    fn invalidate_context(&self) -> bool {
        self.flush_write_buffer();
        self.w64(vtd::CCMD, vtd::CCMD_ICC | vtd::CCMD_GLOBAL);
        self.wait(vtd::CCMD, vtd::CCMD_ICC, false, true)
    }

    fn invalidate_iotlb(&self, did: Option<u16>) -> bool {
        self.flush_write_buffer();
        let reg = self.ecap().iotlb_offset() + 8;
        self.w64(reg, match did { Some(d) => vtd::iotlb_domain(d), None => vtd::IOTLB_IVT | vtd::IOTLB_GLOBAL | vtd::IOTLB_DRAIN });
        self.wait(reg, vtd::IOTLB_IVT, false, true)
    }

    /// Queue one AMD-Vi command, waiting while the ring is full.
    fn submit(&mut self, cmd: [u64; 2]) -> bool {
        let Hw::Amd { commands, tail, .. } = &mut self.hw else { return false };
        let (ring, at) = (*commands, *tail);
        let next = (at + 16) % amdvi::RING_BYTES;
        let va = self.va;
        let head = || unsafe { core::ptr::read_volatile((va + amdvi::COMMAND_HEAD as u64) as *const u64) } & 0x7fff0;
        let mut spins = 0u32;
        while head() == next {
            spins += 1;
            if spins > 50_000_000 { return false; }
            core::hint::spin_loop();
        }
        let slot = (self.hhdm + ring + at) as *mut u64;
        unsafe { core::ptr::write_volatile(slot, cmd[0]); core::ptr::write_volatile(slot.add(1), cmd[1]); }
        core::sync::atomic::fence(Ordering::SeqCst);
        *tail = next;
        self.w64(amdvi::COMMAND_TAIL, next);
        true
    }

    /// Every AMD-Vi command queued so far has finished.
    fn sync(&mut self) -> bool {
        let Hw::Amd { commands, .. } = self.hw else { return true };
        let semaphore = commands + amdvi::RING_BYTES;
        unsafe { core::ptr::write_volatile(self.mem(semaphore), 0); }
        if !self.submit(amdvi::completion_wait(semaphore, 1)) { return false; }
        for _ in 0..50_000_000u32 {
            if unsafe { core::ptr::read_volatile(self.mem(semaphore)) } == 1 { return true; }
            core::hint::spin_loop();
        }
        false
    }

    /// Point `bdf` at a domain's page table, or at nothing. Takes effect at
    /// once on a running unit; at BOOT the enable step flushes everything.
    fn attach(&mut self, frames: &mut KernelFrames, bdf: Bdf, entry: Option<(u64, u16)>, levels: u8) -> bool {
        match &mut self.hw {
            Hw::Vtd { root, contexts, .. } => {
                let bus = bdf.bus as usize;
                if contexts[bus] == 0 {
                    let Some(table) = frames.alloc() else { return false };
                    contexts[bus] = table;
                    frames.write(*root, bus * 2, vtd::root_entry(table));
                }
                let at = bdf.devfn() as usize * 2;
                let aw = if levels == 4 { 2 } else { 1 };
                let (lo, hi) = entry.map(|(t, did)| vtd::context_entry(t, aw, did)).unwrap_or((0, 0));
                // Clear present first, fill the high word, then publish the low word.
                let table = contexts[bus];
                frames.write(table, at, 0);
                frames.write(table, at + 1, hi);
                frames.write(table, at, lo);
                !self.enabled || (self.invalidate_context() && self.invalidate_iotlb(None))
            }
            Hw::Amd { devices, blocked, .. } => {
                let (table, did) = entry.unwrap_or((*blocked, 0));
                let words = amdvi::dte(Some(table), levels, did);
                let dte = (self.hhdm + *devices + bdf.sid() as u64 * 32) as *mut u64;
                // The valid word last, so the IOMMU never sees half an entry.
                unsafe {
                    for w in (1..4).rev() { core::ptr::write_volatile(dte.add(w), words[w]); }
                    core::ptr::write_volatile(dte, words[0]);
                }
                core::sync::atomic::fence(Ordering::SeqCst);
                !self.enabled || (self.submit(amdvi::invalidate_device(bdf.sid())) && self.submit(amdvi::invalidate_domain(did)) && self.sync())
            }
        }
    }

    /// Forget cached translations of one domain; false if the unit did not confirm.
    fn invalidate_domain(&mut self, did: u16) -> bool {
        match self.hw {
            Hw::Vtd { .. } => self.invalidate_iotlb(Some(did)),
            Hw::Amd { .. } => self.submit(amdvi::invalidate_domain(did)) && self.sync(),
        }
    }

    /// Does a new mapping need a flush (the unit may cache "not present")?
    fn flush_after_map(&self) -> bool {
        match self.hw { Hw::Vtd { cap, .. } => cap.caching_mode(), Hw::Amd { .. } => true }
    }

    /// Program the root of translation and turn translation on.
    fn enable(&mut self, max_bus: u8, dids: u16) -> (bool, usize) {
        let mut stale = Vec::new();
        match self.hw {
            Hw::Vtd { root, .. } => {
                self.w32(vtd::FECTL, vtd::FECTL_IM);
                self.w64(vtd::RTADDR, root);
                let ok = self.command(vtd::SRTP, true) && self.invalidate_context() && self.invalidate_iotlb(None);
                self.take_faults(&mut stale);
                let ok = ok && self.command(vtd::TE, true);
                if ok && self.r32(vtd::PMEN) & (1 << 31) != 0 {
                    // Firmware's boot-time DMA protection; our tables replace it.
                    self.w32(vtd::PMEN, 0);
                    self.wait(vtd::PMEN, 1, false, false);
                }
                self.enabled = ok;
            }
            Hw::Amd { features, devices, commands, events, .. } => {
                // Firmware may have left it on; base registers change only while it is off.
                self.w64(amdvi::CONTROL, 0);
                self.w64(amdvi::DEVICE_TABLE, amdvi::device_table(devices, amdvi::DEVICE_TABLE_BYTES));
                self.w64(amdvi::COMMAND_BUFFER, amdvi::ring(commands));
                self.w64(amdvi::EVENT_LOG, amdvi::ring(events));
                for r in [amdvi::COMMAND_HEAD, amdvi::COMMAND_TAIL, amdvi::EVENT_HEAD, amdvi::EVENT_TAIL] { self.w64(r, 0); }
                if let Hw::Amd { tail, .. } = &mut self.hw { *tail = 0; }
                self.w64(amdvi::EXCLUSION_BASE, 0);
                self.w64(amdvi::STATUS, amdvi::STATUS_EVENT_OVERFLOW | 0b110);
                let control = amdvi::CONTROL_COMMAND_BUFFER | amdvi::CONTROL_EVENT_LOG | amdvi::CONTROL_COHERENT | amdvi::CONTROL_TIMEOUT_1MS;
                self.w64(amdvi::CONTROL, control);
                // Some implementations (QEMU) start the command ring only once translation is on.
                self.w64(amdvi::CONTROL, control | amdvi::CONTROL_IOMMU);
                let running = self.wait(amdvi::STATUS, amdvi::STATUS_COMMAND_RUN, true, true);
                self.enabled = true;
                // Nothing cached from firmware's tables may survive.
                let mut ok = running;
                if features & amdvi::FEATURE_INVALIDATE_ALL != 0 { ok &= self.submit(amdvi::invalidate_all()); } else {
                    for sid in 0..=((max_bus as u32) << 8 | 0xff) { ok &= self.submit(amdvi::invalidate_device(sid as u16)); }
                    for did in 0..dids { ok &= self.submit(amdvi::invalidate_domain(did)); }
                }
                ok = ok && self.sync();
                self.take_faults(&mut stale);
                if !ok { self.w64(amdvi::CONTROL, 0); }
                self.enabled = ok;
            }
        }
        (self.enabled, stale.len())
    }

    fn take_faults(&mut self, out: &mut Vec<Fault>) {
        match self.hw {
            Hw::Vtd { cap, .. } => {
                let status = self.r32(vtd::FSTS);
                if status & (vtd::FSTS_PPF | vtd::FSTS_PFO) == 0 { return; }
                let records = cap.fault_records();
                let first = ((status >> 8) & 0xff) as usize;
                for n in 0..records {
                    let at = cap.fault_offset() + ((first + n) % records) * 16;
                    let high = self.r64(at + 8);
                    let Some(fault) = vtd::decode_fault(self.r64(at), high) else { continue };
                    out.push(fault);
                    // F is write-one-to-clear, in the record's top dword.
                    self.w32(at + 12, 1 << 31);
                }
                if status & vtd::FSTS_PFO != 0 { self.w32(vtd::FSTS, vtd::FSTS_PFO); }
            }
            Hw::Amd { events, .. } => {
                let tail = self.r64(amdvi::EVENT_TAIL) & 0x7fff0;
                let mut head = self.r64(amdvi::EVENT_HEAD) & 0x7fff0;
                while head != tail {
                    let e = self.mem(events + head);
                    let (lo, hi) = unsafe { (core::ptr::read_volatile(e), core::ptr::read_volatile(e.add(1))) };
                    unsafe { core::ptr::write_volatile(e, 0); core::ptr::write_volatile(e.add(1), 0); }
                    if let Some(f) = amdvi::decode_event(lo, hi) { out.push(f); }
                    head = (head + 16) % amdvi::RING_BYTES;
                }
                self.w64(amdvi::EVENT_HEAD, head);
                if self.r64(amdvi::STATUS) & amdvi::STATUS_EVENT_OVERFLOW != 0 {
                    // An overflowed log stops; clear and restart it.
                    let control = self.r64(amdvi::CONTROL);
                    self.w64(amdvi::CONTROL, control & !amdvi::CONTROL_EVENT_LOG);
                    self.w64(amdvi::STATUS, amdvi::STATUS_EVENT_OVERFLOW);
                    self.w64(amdvi::EVENT_HEAD, 0);
                    self.w64(amdvi::EVENT_TAIL, 0);
                    self.w64(amdvi::CONTROL, control);
                }
            }
        }
    }
}

fn find_table(signature: &[u8; 4]) -> Option<&'static [u8]> {
    let rsdp = RSDP.load(Ordering::Acquire);
    if rsdp == 0 { return None; }
    let hhdm = crate::HHDM_REQUEST.response()?.offset;
    let at = |pa: u64| (hhdm + pa) as *const u8;
    let read = |pa: u64, len: usize| unsafe { core::slice::from_raw_parts(at(pa), len) };
    let r = read(rsdp, 36);
    let (sdt, wide) = if r[15] >= 2 && u64::from_le_bytes(r[24..32].try_into().ok()?) != 0 {
        (u64::from_le_bytes(r[24..32].try_into().ok()?), true)
    } else {
        (u32::from_le_bytes(r[16..20].try_into().ok()?) as u64, false)
    };
    let len = u32::from_le_bytes(read(sdt, 8)[4..8].try_into().ok()?) as usize;
    let entries = read(sdt, len.max(36));
    let step = if wide { 8 } else { 4 };
    for e in entries[36..].chunks_exact(step) {
        let pa = if wide { u64::from_le_bytes(e.try_into().ok()?) } else { u32::from_le_bytes(e.try_into().ok()?) as u64 };
        let head = read(pa, 8);
        if &head[0..4] == signature {
            let n = u32::from_le_bytes(head[4..8].try_into().ok()?) as usize;
            let table = read(pa, n);
            return sex_iommu::checksum_ok(table).then_some(table);
        }
    }
    None
}

fn map_registers(phys: u64, bytes: u64) -> u64 {
    use x86_64::structures::paging::PageTableFlags as F;
    let va = crate::memory::va_allocator::allocate_va(bytes as usize).expect("IOMMU_REGISTER_VA");
    crate::memory::manager::GLOBAL_VAS.lock().as_mut().expect("IOMMU_REGISTER_VAS")
        .map_physical_range(x86_64::VirtAddr::new(va), phys, bytes,
            F::PRESENT | F::WRITABLE | F::NO_CACHE | F::WRITE_THROUGH | F::NO_EXECUTE, 0)
        .expect("IOMMU_REGISTER_MAP");
    va
}

/// A bridge's (secondary, subordinate) bus numbers.
fn secondary_bus(b: Bdf) -> Option<(u8, u8)> {
    let header = unsafe { crate::hal::pci::pci_config_read(b.bus, b.dev, b.func, 0x0c) } >> 16 & 0x7f;
    if header != 1 { return None; }
    let buses = unsafe { crate::hal::pci::pci_config_read(b.bus, b.dev, b.func, 0x18) };
    Some(((buses >> 8) as u8, (buses >> 16) as u8))
}

fn fmt(b: Bdf) -> (u8, u8, u8) { (b.bus, b.dev, b.func) }

/// Who leases what: every device lease with registers, and every BOOT DMA pool.
fn leases() -> (Vec<(Bdf, u32)>, Vec<(u32, u64, u64)>) {
    let (mut devices, mut pools) = (Vec::new(), Vec::new());
    for id in 1..crate::ipc::MAX_DOMAINS as u32 {
        let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(id) else { continue };
        for slot in 1..=1024u32 {
            match pd.find_capability(slot).map(|c| c.data) {
                Some(CapabilityData::Pci(d)) if d.bar0_view.is_some() =>
                    devices.push((Bdf::new(d.bus, d.dev, d.func), id)),
                Some(CapabilityData::Region(g)) => if let Some(base) = g.dma_base {
                    pools.push((id, base, g.length));
                },
                _ => {}
            }
        }
    }
    (devices, pools)
}

/// Memory firmware keeps using for DMA by requester ids first..=last.
struct Reserved { start: u64, end: u64, first: u16, last: u16, rights: u8 }

/// What the firmware tables say: units, which unit translates a device, reserved memory.
enum Firmware { Intel(Dmar<'static>), Amd(Ivrs<'static>) }

impl Firmware {
    fn find() -> Result<Self, &'static str> {
        if let Some(t) = find_table(sex_iommu::dmar::SIGNATURE) {
            return Dmar::parse(t).map(Firmware::Intel).map_err(|_| "dmar_malformed");
        }
        match find_table(sex_iommu::ivrs::SIGNATURE) {
            Some(t) => Ivrs::parse(t).map(Firmware::Amd).map_err(|_| "ivrs_malformed"),
            None => Err("no_dmar_or_ivrs"),
        }
    }

    /// Register base of the unit translating `bdf`.
    fn unit_base(&self, bdf: Bdf) -> Option<u64> {
        match self {
            Firmware::Intel(d) => d.unit_for(0, bdf, &secondary_bus).map(|u| u.register_base),
            Firmware::Amd(i) => i.unit_for(bdf.sid()).map(|u| u.register_base),
        }
    }

    /// The requester id a device's DMA arrives with, if firmware says it differs.
    fn alias(&self, bdf: Bdf) -> Option<Bdf> {
        match self {
            Firmware::Intel(_) => None,
            Firmware::Amd(i) => i.unit_for(bdf.sid())?.alias(bdf.sid()).map(Bdf::from_sid).filter(|a| *a != bdf),
        }
    }

    fn reserved(&self) -> Vec<Reserved> {
        let mut out = Vec::new();
        match self {
            Firmware::Intel(d) => for r in d.reserved() {
                let Some((start, end)) = r.span() else { continue };
                for t in r.scopes().filter_map(|s| s.resolve(&secondary_bus)) {
                    out.push(Reserved { start, end, first: t.sid(), last: t.sid(), rights: READ | WRITE });
                }
            },
            Firmware::Amd(i) => for r in i.reserved() {
                let rights = if r.read { READ } else { 0 } | if r.write { WRITE } else { 0 };
                if rights != 0 { out.push(Reserved { start: r.start, end: r.end, first: r.first, last: r.last, rights }); }
            },
        }
        out
    }
}

fn unit_index(units: &[Unit], firmware: &Firmware, bdf: Bdf) -> Option<usize> {
    let base = firmware.unit_base(bdf)?;
    units.iter().position(|u| u.phys == base)
}

/// Read and map every unit the firmware lists; None (after logging) when none is usable.
fn find_units(firmware: &Firmware, hhdm: u64) -> Option<(Vec<Unit>, u8, u32, bool)> {
    let mut units = Vec::new();
    match firmware {
        Firmware::Intel(dmar) => {
            for d in dmar.units() {
                if d.segment != 0 { serial_println!("[iommu.unit.skip] base={:#x} segment={} reason=segment", d.register_base, d.segment); continue; }
                let va = map_registers(d.register_base, d.register_bytes.max(PAGE));
                let mut u = Unit { phys: d.register_base, va, hhdm, enabled: false,
                    hw: Hw::Vtd { cap: Cap(0), ecap: Ecap(0), root: 0, contexts: [0; 256] } };
                let (cap, ecap) = (Cap(u.r64(vtd::CAP)), Ecap(u.r64(vtd::ECAP)));
                u.hw = Hw::Vtd { cap, ecap, root: 0, contexts: [0; 256] };
                serial_println!("[iommu.unit] index={} vendor=intel base={:#x} version={:#x} include_all={} sagaw={:#x} mgaw={} domains={} coherent={} pass_through={} queued_invalidation={} interrupt_remap={} caching_mode={} firmware_te={}",
                    units.len(), u.phys, u.r32(vtd::VER), d.include_all as u8, cap.sagaw(), cap.mgaw(), cap.domains(),
                    ecap.coherent() as u8, ecap.pass_through() as u8, ecap.queued_invalidation() as u8,
                    ecap.interrupt_remapping() as u8, cap.caching_mode() as u8, (u.r32(vtd::GSTS) & vtd::TE != 0) as u8);
                units.push(u);
            }
            let levels = if units.iter().all(|u| u.cap().sagaw() & 4 != 0) { 4 }
                else if units.iter().all(|u| u.cap().sagaw() & 2 != 0) { 3 } else { 0 };
            let dids = units.iter().map(|u| u.cap().domains()).min().unwrap_or(16);
            let coherent = units.iter().all(|u| u.ecap().coherent());
            if units.is_empty() || levels == 0 {
                serial_println!("[iommu.absent] vendor=intel reason=no_usable_unit units={} translation=off dma=boot_pools", units.len());
                return None;
            }
            Some((units, levels, dids, coherent))
        }
        Firmware::Amd(ivrs) => {
            for d in ivrs.units() {
                if d.segment != 0 { serial_println!("[iommu.unit.skip] base={:#x} segment={} reason=segment", d.register_base, d.segment); continue; }
                let va = map_registers(d.register_base, 0x4000);
                let mut u = Unit { phys: d.register_base, va, hhdm, enabled: false,
                    hw: Hw::Amd { features: 0, devices: 0, blocked: 0, commands: 0, tail: 0, events: 0 } };
                let features = u.r64(amdvi::EXTENDED_FEATURES);
                if let Hw::Amd { features: f, .. } = &mut u.hw { *f = features; }
                serial_println!("[iommu.unit] index={} vendor=amd base={:#x} ivhd={:#x} device={:04x} features={:#x} invalidate_all={} va_bits={} firmware_on={}",
                    units.len(), u.phys, d.kind, d.device_id, features, (features & amdvi::FEATURE_INVALIDATE_ALL != 0) as u8,
                    ivrs.virtual_address_bits(), (u.r64(amdvi::CONTROL) & amdvi::CONTROL_IOMMU) as u8);
                units.push(u);
            }
            // Host page tables: 4 levels (48-bit device addresses) unless the table says fewer bits.
            let levels = match ivrs.virtual_address_bits() { 1..=47 => 3, _ => 4 };
            if units.is_empty() {
                serial_println!("[iommu.absent] vendor=amd reason=no_usable_unit translation=off dma=boot_pools");
                return None;
            }
            Some((units, levels, 65536, true))
        }
    }
}

/// BOOT, after every lease and DMA pool exists and before the seal.
pub fn init_boot() {
    let firmware = match Firmware::find() {
        Ok(f) => f,
        Err(reason) => { serial_println!("[iommu.absent] reason={} translation=off dma=boot_pools", reason); return; }
    };
    let vendor = match firmware { Firmware::Intel(_) => Vendor::Intel, Firmware::Amd(_) => Vendor::Amd };
    let hhdm = crate::HHDM_REQUEST.response().expect("IOMMU_HHDM").offset;
    let Some((mut units, levels, max_did, coherent)) = find_units(&firmware, hhdm) else { return };
    let format = vendor.format();
    let (leased, pools) = leases();
    let reserved = firmware.reserved();
    let present: Vec<Bdf> = crate::hal::pci::enumerate_bus().iter().map(|d| Bdf::new(d.bus, d.dev, d.func)).collect();
    let max_bus = present.iter().map(|b| b.bus).max().unwrap_or(0);
    // Per service: its root, pool and reserved-memory tables, and the 64 leaf
    // tables (plus parents) of its 128 MiB window. Then context tables, the
    // firmware domains and the self-test.
    let services = { let mut p: Vec<u32> = leased.iter().map(|l| l.1).collect(); p.sort(); p.dedup(); p.len() as u64 };
    let count = (64 + services * 96).next_power_of_two();
    let base = crate::boot_memory::backing(count * PAGE);
    let mut frames = KernelFrames { hhdm, coherent, base, next: 0, count };
    if vendor == Vendor::Amd {
        // One device table for the segment, every entry blocked until a lease says otherwise.
        let devices = crate::boot_memory::backing(amdvi::DEVICE_TABLE_BYTES);
        let blocked = frames.alloc().expect("IOMMU_BLOCKED_TABLE");
        let entry = amdvi::dte(Some(blocked), levels, 0);
        for sid in 0..amdvi::DEVICE_TABLE_BYTES / 32 {
            let dte = (hhdm + devices + sid * 32) as *mut u64;
            for (w, v) in entry.iter().enumerate() { unsafe { core::ptr::write_volatile(dte.add(w), *v); } }
        }
        for u in units.iter_mut() {
            let commands = crate::boot_memory::backing(2 * PAGE);
            let events = crate::boot_memory::backing(PAGE);
            if let Hw::Amd { devices: d, blocked: b, commands: c, events: e, .. } = &mut u.hw { (*d, *b, *c, *e) = (devices, blocked, commands, events); }
        }
    } else {
        for u in units.iter_mut() {
            let table = frames.alloc().expect("IOMMU_ROOT");
            if let Hw::Vtd { root, .. } = &mut u.hw { *root = table; }
        }
    }
    let mut state = State { vendor, units, domains: Vec::new(), devices: Vec::new(), levels, frames, next_did: 1 };
    let covers = |r: &Reserved, b: Bdf| r.first <= b.sid() && b.sid() <= r.last;

    // One domain per service with at least one translated device.
    for &(bdf, pd) in &leased {
        let (b, d, f) = fmt(bdf);
        let Some(unit) = unit_index(&state.units, &firmware, bdf) else {
            serial_println!("[iommu.device] {:02x}:{:02x}.{} pd={} unit=none translation=off", b, d, f, pd);
            continue;
        };
        if state.domains.iter().all(|x| x.pd != pd) {
            let did = state.next_did;
            if did as u32 >= max_did { serial_println!("[iommu.domain.skip] pd={} reason=domain_ids", pd); continue; }
            let table = Table::new(&mut state.frames, levels, format).expect("IOMMU_DOMAIN_TABLE");
            let mut busy: Vec<(u64, u64)> = Vec::new();
            for &(owner, base, length) in pools.iter().filter(|p| p.0 == pd) {
                table.map_range(&mut state.frames, base, base, length, READ | WRITE).expect("IOMMU_POOL_MAP");
                busy.push((base, base + length));
                serial_println!("[iommu.domain.pool] pd={} did={} base={:#x} bytes={} identity=1", owner, did, base, length);
            }
            for r in &reserved {
                if !leased.iter().any(|&(lb, lp)| lp == pd && covers(r, lb)) || busy.iter().any(|&(s, e)| s < r.end && r.start < e) { continue; }
                if table.map_range(&mut state.frames, r.start, r.start, r.end - r.start, r.rights).is_ok() {
                    busy.push((r.start, r.end));
                    serial_println!("[iommu.domain.reserved] pd={} did={} base={:#x} bytes={}", pd, did, r.start, r.end - r.start);
                }
            }
            let limit = table.limit();
            let base = window::place(&busy, abi::WINDOW_BYTES, 1 << 30, 1 << 30, limit.min(1 << 32))
                .or_else(|| window::place(&busy, abi::WINDOW_BYTES, 1 << 30, 1 << 32, limit))
                .expect("IOMMU_WINDOW_PLACE");
            table.prepare(&mut state.frames, base, abi::WINDOW_BYTES).expect("IOMMU_WINDOW_TABLES");
            serial_println!("[iommu.domain] pd={} did={} levels={} lend_window={:#x} lend_bytes={}", pd, did, levels, base, abi::WINDOW_BYTES);
            state.next_did += 1;
            state.domains.push(Domain { pd, did, table, window: Window::new(base), loans: Vec::new(), faults: 0, active: false, stuck: 0 });
        }
        let Some(dom) = state.domains.iter().find(|x| x.pd == pd) else { continue };
        let (root, did) = (dom.table.root, dom.did);
        for source in core::iter::once(bdf).chain(firmware.alias(bdf)) {
            assert!(state.units[unit].attach(&mut state.frames, source, Some((root, did)), levels), "IOMMU_CONTEXT");
            state.devices.push(Device { bdf: source, pd, unit });
            let (b, d, f) = fmt(source);
            serial_println!("[iommu.device] {:02x}:{:02x}.{} pd={} did={} unit={} translation=domain{}", b, d, f, pd, did, unit,
                if source == bdf { "" } else { " alias=1" });
        }
    }

    // Reserved memory of devices nobody leased stays reachable by them alone:
    // each such device gets its own domain holding only its own reserved memory.
    let mut firmware_domains: Vec<(Bdf, u16, Table)> = Vec::new();
    for r in &reserved {
        let targets: Vec<Bdf> = if r.first == r.last { alloc::vec![Bdf::from_sid(r.first)] }
            else { present.iter().copied().filter(|b| covers(r, *b)).collect() };
        for target in targets {
            if leased.iter().any(|&(b, _)| b == target) || state.devices.iter().any(|d| d.bdf == target && d.pd != 0) { continue; }
            let Some(unit) = unit_index(&state.units, &firmware, target) else { continue };
            if !firmware_domains.iter().any(|f| f.0 == target) {
                let did = state.next_did;
                if did as u32 >= max_did { serial_println!("[iommu.domain.skip] firmware_device reason=domain_ids"); continue; }
                state.next_did += 1;
                firmware_domains.push((target, did, Table::new(&mut state.frames, levels, format).expect("IOMMU_FIRMWARE_TABLE")));
            }
            let (_, did, table) = firmware_domains.iter().find(|f| f.0 == target).unwrap();
            let _ = table.map_range(&mut state.frames, r.start, r.start, r.end - r.start, r.rights);
            state.units[unit].attach(&mut state.frames, target, Some((table.root, *did)), levels);
            if !state.devices.iter().any(|d| d.bdf == target) { state.devices.push(Device { bdf: target, pd: 0, unit }); }
            let (b, d, fu) = fmt(target);
            serial_println!("[iommu.device] {:02x}:{:02x}.{} pd=0 did={} translation=firmware_reserved base={:#x} bytes={}", b, d, fu, did, r.start, r.end - r.start);
        }
    }

    // Turn on the units that translate something; leave the rest as firmware set them.
    let mut enabled = 0;
    let dids = state.next_did;
    for i in 0..state.units.len() {
        let u = &mut state.units[i];
        if !state.devices.iter().any(|d| d.unit == i) {
            serial_println!("[iommu.unit.idle] index={} base={:#x} translation=off reason=no_leased_devices", i, u.phys);
            continue;
        }
        let (ok, stale) = u.enable(max_bus, dids);
        if ok { enabled += 1; }
        serial_println!("[iommu.unit.enable] index={} base={:#x} translation={} stale_faults={}", i, u.phys, if ok { "on" } else { "failed" }, stale);
    }
    for dom in state.domains.iter_mut() {
        let pd = dom.pd;
        dom.active = state.devices.iter().filter(|d| d.pd == pd).all(|d| state.units[d.unit].enabled);
        if !dom.active { serial_println!("[iommu.domain.inactive] pd={} did={} lending=refused reason=unit_not_enabled", pd, dom.did); }
    }
    serial_println!("[iommu.enabled] vendor={} units={} enabled={} domains={} devices={} levels={} coherent={} table_frames={}/{} unleased=blocked lend_syscall={}",
        vendor.name(), state.units.len(), enabled, state.domains.len(), state.devices.len(), state.levels, state.frames.coherent as u8,
        state.frames.next, state.frames.count, abi::SYS_DMA);
    *STATE.lock() = Some(state);
    if enabled > 0 { crate::iommu_selftest::run(); }
}

/// From the timer with IF masked: log and attribute faults, rate-limited.
pub fn poll() {
    if TICKS.fetch_add(1, Ordering::Relaxed) % 16 != 0 { return; }
    let Some(mut guard) = STATE.try_lock() else { return };
    let Some(state) = guard.as_mut() else { return };
    drain_faults(state);
}

fn drain_faults(state: &mut State) -> usize {
    let mut faults = Vec::new();
    for u in state.units.iter_mut().filter(|u| u.enabled) { u.take_faults(&mut faults); }
    for f in &faults {
        let owner = state.devices.iter().find(|d| d.bdf == f.source).map(|d| d.pd).unwrap_or(0);
        if let Some(dom) = state.domains.iter_mut().find(|d| d.pd == owner && owner != 0) { dom.faults += 1; }
        if FAULTS_LOGGED.fetch_add(1, Ordering::Relaxed) < FAULT_LOG_LIMIT {
            let (b, d, fu) = fmt(f.source);
            serial_println!("[iommu.fault] device={:02x}:{:02x}.{} owner_pd={} address={:#x} access={} reason={:#x} {} blocked=1",
                b, d, fu, owner, f.address, if f.write { "write" } else { "read" }, f.reason, f.reason_text());
        }
    }
    faults.len()
}

/// Syscall 90: lend memory a service holds to its own devices, or end a loan.
pub fn call(pd: &crate::capability::ProtectionDomain, op: u64, a: u64, b: u64, c: u64, d: u64) -> [u64; 5] {
    let deny = [abi::DENIED, 0, 0, 0, 0];
    let mut guard = STATE.lock();
    let state = guard.as_mut();
    let domain = state.as_ref().and_then(|s| s.domains.iter().position(|x| x.pd == pd.id));
    match op {
        abi::OP_INFO => {
            if a | b | c | d != 0 { return deny; }
            match (state, domain) {
                (Some(s), Some(i)) if s.domains[i].active => {
                    let x = &s.domains[i];
                    [0, 1, x.did as u64, x.faults, x.window.pages_used()]
                }
                _ => [0, 0, 0, 0, 0],
            }
        }
        abi::OP_LEND => {
            let (Some(s), Some(i)) = (state, domain) else { return [abi::NO_IOMMU, 0, 0, 0, 0] };
            if !s.domains[i].active { return [abi::NO_IOMMU, 0, 0, 0, 0]; }
            lend(s, i, pd, a, b, c, d)
        }
        abi::OP_RETURN => {
            let (Some(s), Some(i)) = (state, domain) else { return [abi::NO_IOMMU, 0, 0, 0, 0] };
            if b | c | d != 0 { return deny; }
            match end_loan(s, i, a) { Some(Ok(pages)) => [0, pages, 0, 0, 0], Some(Err(_)) => [abi::UNCERTAIN, 0, 0, 0, 0], None => deny }
        }
        _ => deny,
    }
}

fn lend(s: &mut State, i: usize, pd: &crate::capability::ProtectionDomain, slot: u64, offset: u64, length: u64, access: u64) -> [u64; 5] {
    use sex_pdx::memory_region as m;
    let deny = [abi::DENIED, 0, 0, 0, 0];
    if access == 0 || access & !(abi::DEVICE_READ | abi::DEVICE_WRITE) != 0 { return deny; }
    if offset % PAGE != 0 || length % PAGE != 0 || length == 0 || length > abi::WINDOW_BYTES { return deny; }
    let Some(slot) = crate::capability_slot::decode_capability_slot(slot) else { return deny };
    let Some(CapabilityData::Region(grant)) = pd.find_capability(slot).map(|c| c.data) else { return deny };
    // The device gets no more than the service: reading needs READ, writing WRITE.
    let rights = m::QUERY_VIEW | m::READ | if access & abi::DEVICE_WRITE != 0 { m::WRITE } else { 0 };
    let Ok(view) = m::resolve_view(Some(&grant), m::ViewRequest { offset, length, rights }) else { return deny };
    if s.domains[i].loans.len() >= abi::MAX_LOANS { return [abi::EXHAUSTED, 0, 0, 0, 0]; }
    let pages = length / PAGE;
    // Every page must be mapped for this service now; collect the frames first.
    let mut frames = Vec::with_capacity(pages as usize);
    for n in 0..pages {
        let Some(leaf) = crate::boot_memory::existing_leaf_address(view.virtual_address + n * PAGE) else { return deny };
        let pte = unsafe { core::ptr::read_volatile(leaf as *const u64) };
        let writable = pte & 2 != 0;
        if pte & 5 != 5 || ((pte >> 59) & 15) as u8 != pd.pku_key || (access & abi::DEVICE_WRITE != 0 && !writable) { return deny; }
        frames.push(pte & 0x000f_ffff_ffff_f000);
    }
    let dom = &mut s.domains[i];
    let Some(address) = dom.window.take(pages) else { return [abi::EXHAUSTED, 0, 0, 0, 0] };
    let r = (if access & abi::DEVICE_READ != 0 { READ } else { 0 }) | (if access & abi::DEVICE_WRITE != 0 { WRITE } else { 0 });
    for (n, frame) in frames.iter().enumerate() {
        if dom.table.map(&mut s.frames, address + n as u64 * PAGE, *frame, r, false).is_err() {
            for undo in 0..n as u64 { dom.table.unmap(&mut s.frames, address + undo * PAGE); }
            dom.window.give(address, pages);
            return deny;
        }
    }
    dom.loans.push(Loan { address, pages });
    // Units that cache not-present entries must forget them.
    let did = dom.did;
    for u in s.units.iter_mut().filter(|u| u.enabled && u.flush_after_map()) { u.invalidate_domain(did); }
    [0, address, pages, 0, 0]
}

/// Ok(pages) once no unit can still use the loan; Err(pages) when an IOTLB
/// flush did not complete: the device may still hold the old translation, so
/// its device addresses are never reused and the caller must not trust the memory.
fn end_loan(s: &mut State, i: usize, address: u64) -> Option<Result<u64, u64>> {
    let dom = &mut s.domains[i];
    let at = dom.loans.iter().position(|l| l.address == address)?;
    let loan = dom.loans.swap_remove(at);
    for n in 0..loan.pages { dom.table.unmap(&mut s.frames, loan.address + n * PAGE); }
    let did = dom.did;
    let mut flushed = true;
    for u in s.units.iter_mut().filter(|u| u.enabled) { flushed &= u.invalidate_domain(did); }
    if !flushed {
        s.domains[i].stuck += loan.pages;
        serial_println!("[iommu.loan.uncertain] pd={} did={} device_address={:#x} pages={} reason=iotlb_flush_timeout window_kept=1",
            s.domains[i].pd, did, loan.address, loan.pages);
        return Some(Err(loan.pages));
    }
    s.domains[i].window.give(loan.address, loan.pages);
    Some(Ok(loan.pages))
}

/// A restarted service's old loans end with its old run (after its devices are quiesced).
pub fn revoke(pd_id: u32) {
    let mut guard = STATE.lock();
    let Some(s) = guard.as_mut() else { return };
    let Some(i) = s.domains.iter().position(|d| d.pd == pd_id) else { return };
    let mut ended = 0;
    while let Some(address) = s.domains[i].loans.first().map(|l| l.address) {
        ended += match end_loan(s, i, address) { Some(Ok(p)) | Some(Err(p)) => p, None => 0 };
    }
    serial_println!("[iommu.loans.revoked] pd={} pages={}", pd_id, ended);
}

/// For the self-test: run `f` with the unit translating `bdf`, its frames and a fresh domain id.
pub(crate) fn with_unit<R>(bdf: Bdf, f: impl FnOnce(&mut TestAccess) -> R) -> Option<R> {
    let mut guard = STATE.lock();
    let s = guard.as_mut()?;
    let firmware = Firmware::find().ok()?;
    let unit = unit_index(&s.units, &firmware, bdf).filter(|&u| s.units[u].enabled)?;
    let did = s.next_did;
    let mut access = TestAccess { state: s, unit, did };
    Some(f(&mut access))
}

pub(crate) struct TestAccess<'a> { state: &'a mut State, unit: usize, did: u16 }

impl TestAccess<'_> {
    pub fn table(&mut self) -> Option<Table> { Table::new(&mut self.state.frames, self.state.levels, self.state.vendor.format()) }
    pub fn map(&mut self, t: &Table, iova: u64, frame: u64, rights: u8) -> bool { t.map(&mut self.state.frames, iova, frame, rights, true).is_ok() }
    pub fn attach(&mut self, bdf: Bdf, t: Option<&Table>) -> bool {
        let levels = self.state.levels;
        let entry = t.map(|t| (t.root, self.did));
        let u = &mut self.state.units[self.unit];
        u.attach(&mut self.state.frames, bdf, entry, levels) && (entry.is_none() || u.invalidate_domain(self.did))
    }
    /// Does this unit log every blocked DMA? QEMU's AMD-Vi model blocks
    /// without logging (empty entries, read-only pages), so there only the
    /// data proves isolation; real AMD hardware logs IO_PAGE_FAULT events.
    pub fn logs_every_fault(&self) -> bool { self.state.vendor == Vendor::Intel }
    pub fn faults_from(&mut self, bdf: Bdf) -> Vec<Fault> {
        let mut all = Vec::new();
        self.state.units[self.unit].take_faults(&mut all);
        for f in &all {
            let (b, d, fu) = fmt(f.source);
            serial_println!("[iommu.fault] device={:02x}:{:02x}.{} owner_pd=selftest address={:#x} access={} reason={:#x} {} blocked=1",
                b, d, fu, f.address, if f.write { "write" } else { "read" }, f.reason, f.reason_text());
        }
        all.into_iter().filter(|f| f.source == bdf).collect()
    }
}
