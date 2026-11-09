// SPDX-License-Identifier: GPL-3.0-only
//! DMA isolation through the IOMMU (Intel VT-d now; AMD-Vi is detected).
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
use sex_iommu::dmar::Dmar;
use sex_iommu::table::{Format, Frames, Table, READ, WRITE};
use sex_iommu::vtd::{self, Cap, Ecap, Fault};
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

struct Unit {
    phys: u64,
    va: u64,
    cap: Cap,
    ecap: Ecap,
    include_all: bool,
    root: u64,
    contexts: [u64; 256],
    enabled: bool,
}

struct Loan { address: u64, pages: u64 }

struct Domain {
    pd: u32,
    did: u16,
    table: Table,
    window: Window<WINDOW_WORDS>,
    loans: Vec<Loan>,
    faults: u64,
}

struct Device { bdf: Bdf, pd: u32, unit: usize }

struct State {
    units: Vec<Unit>,
    domains: Vec<Domain>,
    devices: Vec<Device>,
    levels: u8,
    frames: KernelFrames,
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

    fn wait(&self, off: usize, mask: u64, set: bool, wide: bool) -> bool {
        for _ in 0..50_000_000u32 {
            let v = if wide { self.r64(off) } else { self.r32(off) as u64 };
            if (v & mask != 0) == set { return true; }
            core::hint::spin_loop();
        }
        false
    }

    /// Issue a global command bit and wait for the hardware to show it.
    fn command(&self, bit: u32, on: bool) -> bool {
        let status = self.r32(vtd::GSTS);
        self.w32(vtd::GCMD, vtd::command(status, bit, on));
        self.wait(vtd::GSTS, bit as u64, on, false)
    }

    fn flush_write_buffer(&self) {
        if !self.cap.required_write_buffer_flush() { return; }
        let status = self.r32(vtd::GSTS);
        self.w32(vtd::GCMD, (status & vtd::GSTS_PERSISTENT) | vtd::WBF);
        self.wait(vtd::GSTS, vtd::WBF as u64, false, false);
    }

    fn invalidate_context(&self) -> bool {
        self.flush_write_buffer();
        self.w64(vtd::CCMD, vtd::CCMD_ICC | vtd::CCMD_GLOBAL);
        self.wait(vtd::CCMD, vtd::CCMD_ICC, false, true)
    }

    /// Flush cached translations of one domain (all of them when did is None).
    fn invalidate_iotlb(&self, did: Option<u16>) -> bool {
        self.flush_write_buffer();
        let reg = self.ecap.iotlb_offset() + 8;
        self.w64(reg, match did { Some(d) => vtd::iotlb_domain(d), None => vtd::IOTLB_IVT | vtd::IOTLB_GLOBAL | vtd::IOTLB_DRAIN });
        self.wait(reg, vtd::IOTLB_IVT, false, true)
    }

    fn set_context(&mut self, frames: &mut KernelFrames, bdf: Bdf, entry: Option<(u64, u64)>) -> bool {
        let bus = bdf.bus as usize;
        if self.contexts[bus] == 0 {
            let Some(table) = frames.alloc() else { return false };
            self.contexts[bus] = table;
            frames.write(self.root, bus * 2, vtd::root_entry(table));
        }
        let at = bdf.devfn() as usize * 2;
        let (lo, hi) = entry.unwrap_or((0, 0));
        // Clear present first, fill the high word, then publish the low word.
        frames.write(self.contexts[bus], at, 0);
        frames.write(self.contexts[bus], at + 1, hi);
        frames.write(self.contexts[bus], at, lo);
        true
    }

    fn take_faults(&self, out: &mut Vec<Fault>) {
        let status = self.r32(vtd::FSTS);
        if status & (vtd::FSTS_PPF | vtd::FSTS_PFO) == 0 { return; }
        let records = self.cap.fault_records();
        let first = ((status >> 8) & 0xff) as usize;
        for n in 0..records {
            let at = self.cap.fault_offset() + ((first + n) % records) * 16;
            let high = self.r64(at + 8);
            let Some(fault) = Fault::decode(self.r64(at), high) else { continue };
            out.push(fault);
            // F is write-one-to-clear, in the record's top dword.
            self.w32(at + 12, 1 << 31);
        }
        if status & vtd::FSTS_PFO != 0 { self.w32(vtd::FSTS, vtd::FSTS_PFO); }
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

fn secondary_bus(b: Bdf) -> Option<u8> {
    let header = unsafe { crate::hal::pci::pci_config_read(b.bus, b.dev, b.func, 0x0c) } >> 16 & 0x7f;
    if header != 1 { return None; }
    Some((unsafe { crate::hal::pci::pci_config_read(b.bus, b.dev, b.func, 0x18) } >> 8) as u8)
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

/// BOOT, after every lease and DMA pool exists and before the seal.
pub fn init_boot() {
    let Some(table) = find_table(sex_iommu::dmar::SIGNATURE) else {
        match find_table(sex_iommu::ivrs::SIGNATURE).and_then(|t| sex_iommu::ivrs::Ivrs::parse(t).ok()) {
            Some(ivrs) => {
                for u in ivrs.units() {
                    serial_println!("[iommu.amd.unit] kind={:#x} base={:#x} device={:04x} segment={} va_bits={}",
                        u.kind, u.register_base, u.device_id, u.segment, ivrs.virtual_address_bits());
                }
                serial_println!("[iommu.absent] vendor=amd units={} translation=off reason=amd_vi_not_yet dma=boot_pools", ivrs.units().count());
            }
            None => { serial_println!("[iommu.absent] reason=no_dmar_or_ivrs translation=off dma=boot_pools"); }
        }
        return;
    };
    let Ok(dmar) = Dmar::parse(table) else {
        serial_println!("[iommu.absent] reason=dmar_malformed translation=off dma=boot_pools");
        return;
    };
    let hhdm = crate::HHDM_REQUEST.response().expect("IOMMU_HHDM").offset;
    let mut units = Vec::new();
    for d in dmar.units() {
        if d.segment != 0 { serial_println!("[iommu.unit.skip] base={:#x} segment={} reason=segment", d.register_base, d.segment); continue; }
        let va = map_registers(d.register_base, d.register_bytes.max(PAGE));
        let mut u = Unit { phys: d.register_base, va, cap: Cap(0), ecap: Ecap(0), include_all: d.include_all,
            root: 0, contexts: [0; 256], enabled: false };
        u.cap = Cap(u.r64(vtd::CAP));
        u.ecap = Ecap(u.r64(vtd::ECAP));
        serial_println!("[iommu.unit] index={} base={:#x} version={:#x} include_all={} sagaw={:#x} mgaw={} domains={} coherent={} pass_through={} queued_invalidation={} interrupt_remap={} caching_mode={} firmware_te={}",
            units.len(), u.phys, u.r32(vtd::VER), u.include_all as u8, u.cap.sagaw(), u.cap.mgaw(), u.cap.domains(),
            u.ecap.coherent() as u8, u.ecap.pass_through() as u8, u.ecap.queued_invalidation() as u8,
            u.ecap.interrupt_remapping() as u8, u.cap.caching_mode() as u8, (u.r32(vtd::GSTS) & vtd::TE != 0) as u8);
        units.push(u);
    }
    let levels = if units.iter().all(|u| u.cap.sagaw() & 4 != 0) { 4 }
        else if units.iter().all(|u| u.cap.sagaw() & 2 != 0) { 3 } else { 0 };
    if units.is_empty() || levels == 0 {
        serial_println!("[iommu.absent] reason=no_usable_unit units={} translation=off dma=boot_pools", units.len());
        return;
    }
    let (leased, pools) = leases();
    // Per service: its root, pool and reserved-memory tables, and the 64 leaf
    // tables (plus parents) of its 128 MiB window. Then context tables, the
    // firmware domain and the self-test.
    let services = { let mut p: Vec<u32> = leased.iter().map(|l| l.1).collect(); p.sort(); p.dedup(); p.len() as u64 };
    let count = (64 + services * 96).next_power_of_two();
    let base = crate::boot_memory::backing(count * PAGE);
    let mut frames = KernelFrames { hhdm, coherent: units.iter().all(|u| u.ecap.coherent()), base, next: 0, count };
    for u in units.iter_mut() { u.root = frames.alloc().expect("IOMMU_ROOT"); }
    let max_did = units.iter().map(|u| u.cap.domains()).min().unwrap_or(16);
    let mut state = State { units, domains: Vec::new(), devices: Vec::new(), levels, frames };
    let aw = if levels == 4 { 2 } else { 1 };
    let unit_of = |b: Bdf, units: &[Unit]| dmar.unit_for(0, b, &secondary_bus)
        .and_then(|d| units.iter().position(|u| u.phys == d.register_base));

    // One domain per service with at least one translated device.
    for &(bdf, pd) in &leased {
        let (b, d, f) = fmt(bdf);
        let Some(unit) = unit_of(bdf, &state.units) else {
            serial_println!("[iommu.device] {:02x}:{:02x}.{} pd={} unit=none translation=off", b, d, f, pd);
            continue;
        };
        if state.domains.iter().all(|x| x.pd != pd) {
            let did = state.domains.len() as u16 + 1;
            if did as u32 >= max_did { serial_println!("[iommu.domain.skip] pd={} reason=domain_ids", pd); continue; }
            let table = Table::new(&mut state.frames, levels, Format::Vtd).expect("IOMMU_DOMAIN_TABLE");
            let mut busy: Vec<(u64, u64)> = Vec::new();
            for &(owner, base, length) in pools.iter().filter(|p| p.0 == pd) {
                table.map_range(&mut state.frames, base, base, length, READ | WRITE).expect("IOMMU_POOL_MAP");
                busy.push((base, base + length));
                serial_println!("[iommu.domain.pool] pd={} did={} base={:#x} bytes={} identity=1", owner, did, base, length);
            }
            for r in dmar.reserved() {
                let Some((start, end)) = r.span() else { continue };
                let ours = r.scopes().filter_map(|s| s.resolve(&secondary_bus))
                    .any(|t| leased.iter().any(|&(lb, lp)| lp == pd && lb == t));
                if !ours { continue; }
                if table.map_range(&mut state.frames, start, start, end - start, READ | WRITE).is_ok() {
                    busy.push((start, end));
                    serial_println!("[iommu.domain.rmrr] pd={} did={} base={:#x} bytes={}", pd, did, start, end - start);
                }
            }
            let limit = table.limit();
            let base = window::place(&busy, abi::WINDOW_BYTES, 1 << 30, 1 << 30, limit.min(1 << 32))
                .or_else(|| window::place(&busy, abi::WINDOW_BYTES, 1 << 30, 1 << 32, limit))
                .expect("IOMMU_WINDOW_PLACE");
            table.prepare(&mut state.frames, base, abi::WINDOW_BYTES).expect("IOMMU_WINDOW_TABLES");
            serial_println!("[iommu.domain] pd={} did={} levels={} lend_window={:#x} lend_bytes={}", pd, did, levels, base, abi::WINDOW_BYTES);
            state.domains.push(Domain { pd, did, table, window: Window::new(base), loans: Vec::new(), faults: 0 });
        }
        let Some(dom) = state.domains.iter().find(|x| x.pd == pd) else { continue };
        let entry = vtd::context_entry(dom.table.root, aw, dom.did);
        let did = dom.did;
        assert!(state.units[unit].set_context(&mut state.frames, bdf, Some(entry)), "IOMMU_CONTEXT");
        state.devices.push(Device { bdf, pd, unit });
        serial_println!("[iommu.device] {:02x}:{:02x}.{} pd={} did={} unit={} translation=domain", b, d, f, pd, did, unit);
    }

    // Reserved memory of devices nobody leased stays reachable by them alone.
    let mut firmware: Option<(u16, Table)> = None;
    for r in dmar.reserved() {
        let Some((start, end)) = r.span() else { continue };
        for target in r.scopes().filter_map(|s| s.resolve(&secondary_bus)) {
            if leased.iter().any(|&(b, _)| b == target) { continue; }
            let Some(unit) = unit_of(target, &state.units) else { continue };
            if firmware.is_none() {
                let did = state.domains.len() as u16 + 1;
                firmware = Some((did, Table::new(&mut state.frames, levels, Format::Vtd).expect("IOMMU_FIRMWARE_TABLE")));
            }
            let (did, table) = firmware.as_ref().unwrap();
            let _ = table.map_range(&mut state.frames, start, start, end - start, READ | WRITE);
            let entry = vtd::context_entry(table.root, aw, *did);
            state.units[unit].set_context(&mut state.frames, target, Some(entry));
            if !state.devices.iter().any(|d| d.bdf == target) { state.devices.push(Device { bdf: target, pd: 0, unit }); }
            let (b, d, fu) = fmt(target);
            serial_println!("[iommu.device] {:02x}:{:02x}.{} pd=0 did={} translation=firmware_reserved base={:#x} bytes={}", b, d, fu, did, start, end - start);
        }
    }

    // Turn on the units that translate something; leave the rest as firmware set them.
    let mut enabled = 0;
    for (i, u) in state.units.iter_mut().enumerate() {
        if !state.devices.iter().any(|d| d.unit == i) {
            serial_println!("[iommu.unit.idle] index={} base={:#x} translation=off reason=no_leased_devices", i, u.phys);
            continue;
        }
        u.w32(vtd::FECTL, vtd::FECTL_IM);
        u.w64(vtd::RTADDR, u.root);
        let ok = u.command(vtd::SRTP, true) && u.invalidate_context() && u.invalidate_iotlb(None);
        let mut stale = Vec::new();
        u.take_faults(&mut stale);
        let ok = ok && u.command(vtd::TE, true);
        if ok && u.r32(vtd::PMEN) & (1 << 31) != 0 {
            // Firmware's boot-time DMA protection; our tables replace it.
            u.w32(vtd::PMEN, 0);
            u.wait(vtd::PMEN, 1, false, false);
        }
        u.enabled = ok;
        if ok { enabled += 1; }
        serial_println!("[iommu.unit.enable] index={} base={:#x} root={:#x} translation={} stale_faults={}", i, u.phys, u.root, if ok { "on" } else { "failed" }, stale.len());
    }
    serial_println!("[iommu.enabled] vendor=intel units={} enabled={} domains={} devices={} levels={} coherent={} table_frames={}/{} unleased=blocked lend_syscall={}",
        state.units.len(), enabled, state.domains.len(), state.devices.len(), state.levels, state.frames.coherent as u8,
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
    for u in state.units.iter().filter(|u| u.enabled) { u.take_faults(&mut faults); }
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
                (Some(s), Some(i)) => {
                    let x = &s.domains[i];
                    [0, 1, x.did as u64, x.faults, x.window.pages_used()]
                }
                _ => [0, 0, 0, 0, 0],
            }
        }
        abi::OP_LEND => {
            let (Some(s), Some(i)) = (state, domain) else { return [abi::NO_IOMMU, 0, 0, 0, 0] };
            lend(s, i, pd, a, b, c, d)
        }
        abi::OP_RETURN => {
            let (Some(s), Some(i)) = (state, domain) else { return [abi::NO_IOMMU, 0, 0, 0, 0] };
            if b | c | d != 0 { return deny; }
            match end_loan(s, i, a) { Some(pages) => [0, pages, 0, 0, 0], None => deny }
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
    for u in s.units.iter().filter(|u| u.enabled && u.cap.caching_mode()) { u.invalidate_iotlb(Some(did)); }
    [0, address, pages, 0, 0]
}

fn end_loan(s: &mut State, i: usize, address: u64) -> Option<u64> {
    let dom = &mut s.domains[i];
    let at = dom.loans.iter().position(|l| l.address == address)?;
    let loan = dom.loans.swap_remove(at);
    for n in 0..loan.pages { dom.table.unmap(&mut s.frames, loan.address + n * PAGE); }
    let did = dom.did;
    // The memory is the service's again only after no unit can still use the old entries.
    for u in s.units.iter().filter(|u| u.enabled) { u.invalidate_iotlb(Some(did)); }
    s.domains[i].window.give(loan.address, loan.pages);
    Some(loan.pages)
}

/// A restarted service's old loans end with its old run (after its devices are quiesced).
pub fn revoke(pd_id: u32) {
    let mut guard = STATE.lock();
    let Some(s) = guard.as_mut() else { return };
    let Some(i) = s.domains.iter().position(|d| d.pd == pd_id) else { return };
    let mut ended = 0;
    while let Some(address) = s.domains[i].loans.first().map(|l| l.address) {
        ended += end_loan(s, i, address).unwrap_or(0);
    }
    serial_println!("[iommu.loans.revoked] pd={} pages={}", pd_id, ended);
}

/// For the self-test: run `f` with the unit translating `bdf`, its frames and a fresh domain id.
pub(crate) fn with_unit<R>(bdf: Bdf, f: impl FnOnce(&mut TestAccess) -> R) -> Option<R> {
    let mut guard = STATE.lock();
    let s = guard.as_mut()?;
    let dmar = Dmar::parse(find_table(sex_iommu::dmar::SIGNATURE)?).ok()?;
    let base = dmar.unit_for(0, bdf, &secondary_bus)?.register_base;
    let unit = s.units.iter().position(|u| u.phys == base && u.enabled)?;
    let did = s.domains.len() as u16 + 2;
    let mut access = TestAccess { state: s, unit, did };
    Some(f(&mut access))
}

pub(crate) struct TestAccess<'a> { state: &'a mut State, unit: usize, did: u16 }

impl TestAccess<'_> {
    pub fn table(&mut self) -> Option<Table> { Table::new(&mut self.state.frames, self.state.levels, Format::Vtd) }
    pub fn map(&mut self, t: &Table, iova: u64, frame: u64, rights: u8) -> bool { t.map(&mut self.state.frames, iova, frame, rights, true).is_ok() }
    pub fn attach(&mut self, bdf: Bdf, t: Option<&Table>) -> bool {
        let aw = if self.state.levels == 4 { 2 } else { 1 };
        let entry = t.map(|t| vtd::context_entry(t.root, aw, self.did));
        let u = &mut self.state.units[self.unit];
        u.set_context(&mut self.state.frames, bdf, entry) && u.invalidate_context() && u.invalidate_iotlb(None)
    }
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

