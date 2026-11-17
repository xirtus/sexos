// SPDX-License-Identifier: GPL-3.0-only
//! Protection views (plan decision D2): one translation, more than one root table.
//!
//! Every view maps every address to the same frame (still one address space); views
//! differ only in which 2 MiB spans user mode may reach. View 0 is the canonical tables
//! every mapping is made in. Each shared-key program row has a view of its own where its
//! spans are open; they are closed in the canonical tables and in every other view. A
//! view holds private copies of only the root and the tables above its spans
//! (`views_rules`); page tables are shared, so a leaf written once is seen by all views.
//!
//! Each view is tagged with a PCID, its index, so entering one is a CR3 write that keeps
//! the TLB. INVLPG and a CR3 reload reach only the current PCID, so every invalidation
//! marks this core's other views stale (`invalidate`), and a stale view is flushed, alone,
//! when this core next enters it. Without PCIDs every entry flushes, which stays correct.
use crate::views_rules::{self as rules, Tables, FRAME, PRESENT, USER};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;

pub const COUNT: usize = 1 + crate::program_tenancy::ROWS;
const _: () = assert!(COUNT <= 64);
const CORES: usize = 256;
const WRITABLE: u64 = 1 << 1;

/// `copies`: (canonical, private, level, an address the table covers).
struct View { copies: Vec<(u64, u64, u32, u64)>, spans: Vec<u64> }
static VIEWS: Mutex<[View; COUNT]> = Mutex::new([const { View { copies: Vec::new(), spans: Vec::new() } }; COUNT]);
static ROOT: [AtomicU64; COUNT] = [const { AtomicU64::new(0) }; COUNT];
static PCID: AtomicBool = AtomicBool::new(false);
/// Per local APIC id: the views that core must flush when it next enters them.
static STALE: [AtomicU64; CORES] = [const { AtomicU64::new(!0) }; CORES];
/// Per local APIC id: the views that core has entered, for one line each.
static ENTERED: [AtomicU64; CORES] = [const { AtomicU64::new(0) }; CORES];
static SWITCHES: AtomicU64 = AtomicU64::new(0);
static FLUSHED: AtomicU64 = AtomicU64::new(0);

struct Hw(u64);
impl Tables for Hw {
    fn read(&self, table: u64, index: usize) -> u64 { unsafe { core::ptr::read_volatile((self.0 + table + index as u64 * 8) as *const u64) } }
    fn write(&mut self, table: u64, index: usize, value: u64) { unsafe { core::ptr::write_volatile((self.0 + table + index as u64 * 8) as *mut u64, value) } }
    fn copy(&mut self, from: u64) -> Option<u64> {
        let frame = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0)?;
        crate::boot_memory::copy_frame(from, frame);
        Some(frame)
    }
}
fn hw() -> Hw { Hw(crate::HHDM_REQUEST.response().expect("VIEWS_NO_HHDM").offset) }
fn cr3() -> u64 { let v: u64; unsafe { core::arch::asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)); } v }
unsafe fn set_cr3(v: u64) { unsafe { core::arch::asm!("mov cr3, {}", in(reg) v, options(nostack, preserves_flags)); } }
fn tagged() -> bool {
    use x86_64::registers::control::{Cr4, Cr4Flags};
    PCID.load(Ordering::Relaxed) && Cr4::read().contains(Cr4Flags::PCID)
}

/// The tables every mapping is made in: the canonical root once views exist.
pub fn root_table() -> u64 {
    match ROOT[0].load(Ordering::Acquire) { 0 => cr3() & FRAME, root => root }
}

/// BOOT, before anything runs. The kernel half of the root is filled first, so a kernel
/// mapping made later lands in tables every view shares.
pub fn init() {
    let mut t = hw();
    let root = cr3() & FRAME;
    assert_eq!(cr3() & 0xFFF, 0, "VIEWS_ROOT_FLAGS");
    for i in 256..512 {
        if t.read(root, i) & PRESENT != 0 { continue; }
        let frame = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0).expect("VIEWS_KERNEL_TABLE");
        crate::boot_memory::zero_frame(frame);
        t.write(root, i, frame | PRESENT | WRITABLE);
    }
    for (v, slot) in ROOT.iter().enumerate().skip(1) {
        slot.store(t.copy(root).expect("VIEWS_ROOT_COPY"), Ordering::Release);
        VIEWS.lock()[v].copies.push((root, slot.load(Ordering::Relaxed), 3, 0));
    }
    ROOT[0].store(root, Ordering::Release);
    let pcid = core::arch::x86_64::__cpuid(1).ecx & (1 << 17) != 0;
    PCID.store(pcid, Ordering::Release);
    crate::serial_println!("[boot.views] views={} pcid={} root={:#x}", COUNT, pcid as u8, root);
}

/// `va`'s span opens in `view`, which gets private tables down to it as needed.
pub fn open(view: usize, va: u64) {
    assert!(view != 0 && view < COUNT, "VIEWS_INDEX");
    let span = va & !(rules::SPAN - 1);
    let mut views = VIEWS.lock();
    let entry = &mut views[view];
    let mut made = Vec::new();
    let closed = rules::open(&mut hw(), root_table(), ROOT[view].load(Ordering::Acquire), span, &mut |level, c, p| made.push((c, p, level, span))).expect("VIEWS_SPAN_UNMAPPED");
    entry.copies.extend(made);
    if let Err(i) = entry.spans.binary_search(&span) { entry.spans.insert(i, span); }
    drop(views);
    if closed { changed(view); }
}

/// `va`'s span closes in the canonical tables and in every view but `owner`'s.
pub fn close(owner: usize, va: u64) {
    let mut t = hw();
    let mut table = root_table();
    for level in [3, 2] { table = t.read(table, rules::index(va, level)) & FRAME; }
    let i = rules::index(va, 1);
    let before = t.read(table, i);
    if before & USER == 0 { return; }
    t.write(table, i, before & !USER);
    let views = VIEWS.lock();
    for (_, view) in views.iter().enumerate().filter(|(v, _)| *v != owner) {
        for &(_, private, _, _) in view.copies.iter().filter(|copy| copy.0 == table) {
            let e = t.read(private, i);
            t.write(private, i, e & !USER);
        }
    }
    drop(views);
    // Every core may hold the open entry, under any PCID.
    crate::smp::tlb::flush_pages(usize::MAX, core::iter::empty());
}

/// The canonical `table` changed at `index` (`boot_memory::arena_leaf`): views copying
/// that table follow.
pub fn mirror(table: u64, index: usize) {
    if ROOT[0].load(Ordering::Acquire) == 0 { return; }
    let mut t = hw();
    let c = t.read(table, index);
    for view in VIEWS.lock().iter() {
        for &(_, private, _, _) in view.copies.iter().filter(|copy| copy.0 == table) {
            let p = t.read(private, index);
            t.write(private, index, rules::merged(c, p));
        }
    }
}

/// A mapping was made in the canonical tables over [va, va + bytes) (`memory::manager`):
/// private tables above it take any table it created.
pub fn mapped(va: u64, bytes: u64) {
    if ROOT[0].load(Ordering::Acquire) == 0 { return; }
    let mut t = hw();
    let end = va.saturating_add(bytes);
    for view in VIEWS.lock().iter() {
        for &(canonical, private, level, base) in &view.copies {
            for i in rules::reaching(level, base, va, end) {
                let p = t.read(private, i);
                t.write(private, i, rules::merged(t.read(canonical, i), p));
            }
        }
    }
}

/// Every private table takes what the canonical ones gained since it was copied.
pub fn resync() {
    let mut t = hw();
    for view in VIEWS.lock().iter() {
        for &(canonical, private, _, _) in &view.copies {
            for i in 0..512 { let p = t.read(private, i); t.write(private, i, rules::merged(t.read(canonical, i), p)); }
        }
    }
}

/// Cores must flush `view` before using it again; this one at once if it is in it.
fn changed(view: usize) {
    for core in STALE.iter() { core.fetch_or(1 << view, Ordering::AcqRel); }
    x86_64::instructions::interrupts::without_interrupts(|| {
        let now = cr3();
        if now & FRAME == ROOT[view].load(Ordering::Acquire) { unsafe { set_cr3(now & !rules::NOFLUSH); } }
    });
}

/// Before a task runs: its view, entered with a CR3 write that keeps the TLB unless an
/// invalidation since this core last ran the view says otherwise.
pub fn enter(view: usize) {
    let root = ROOT[view].load(Ordering::Acquire);
    if root == 0 { return; }
    x86_64::instructions::interrupts::without_interrupts(|| {
        let now = cr3();
        let pcid = PCID.load(Ordering::Relaxed);
        if now == rules::cr3(root, view, pcid, true) { return; }
        let core = crate::apic::local_id() as usize % CORES;
        if pcid && !tagged() {
            // Tags start now; this core's PCID 0 entries are the canonical tables'.
            use x86_64::registers::control::{Cr4, Cr4Flags};
            assert_eq!(now, ROOT[0].load(Ordering::Relaxed), "VIEWS_PCID_ENABLE_ROOT");
            unsafe { Cr4::update(|f| f.insert(Cr4Flags::PCID)); }
            STALE[core].store(!1, Ordering::Release);
        }
        let stale = STALE[core].fetch_and(!(1 << view), Ordering::AcqRel) & (1 << view) != 0;
        unsafe { set_cr3(rules::cr3(root, view, pcid, stale)); }
        if ENTERED[core].fetch_or(1 << view, Ordering::Relaxed) & (1 << view) == 0 {
            crate::serial_println!("[views.first] view={} core={}", view, core);
        }
        let n = SWITCHES.fetch_add(1, Ordering::Relaxed) + 1;
        let flushed = FLUSHED.fetch_add((stale || !pcid) as u64, Ordering::Relaxed) + (stale || !pcid) as u64;
        if n.is_power_of_two() {
            crate::serial_println!("[views.switch] switches={} flushed={} view={} core={} pcid={}", n, flushed, view, core, pcid as u8);
        }
    });
}

/// This core drops `va` (everything for `u64::MAX`) from the current view now, and from
/// its other views when it next enters them. `smp::tlb` runs this on every core.
pub fn invalidate(va: u64) {
    if va == u64::MAX { let now = cr3(); unsafe { set_cr3(now & !rules::NOFLUSH); } }
    else { x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(va)); }
    if ROOT[0].load(Ordering::Relaxed) == 0 || !tagged() { return; }
    let current = (cr3() & 0xFFF) as usize;
    STALE[crate::apic::local_id() as usize % CORES].fetch_or(rules::stale_after_invalidate(current), Ordering::AcqRel);
}

pub fn census() -> (u64, u64) { (SWITCHES.load(Ordering::Relaxed), FLUSHED.load(Ordering::Relaxed)) }
