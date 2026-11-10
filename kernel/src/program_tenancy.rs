// SPDX-License-Identifier: GPL-3.0-only
//! Windowed program rows past the first few share one hardware key, as there are only
//! sixteen. They take turns: only the row about to run can reach its memory, so a row
//! can never reach another's. All of them run on one core, one at a time, which is what
//! makes one set of page tables enough in a single address space.
//!
//! A turn is taken by span, not by page. Every page a shared row holds lies in a 2 MiB
//! span no one else maps into (its arena, and its regions, which BOOT places on whole
//! spans), so the user bit of the span's page-directory entry opens or closes 512 pages
//! at once. A switch costs a few entries and one local TLB flush, whatever the
//! programs' size. Leaves keep their values throughout, except for the few pages BOOT
//! placed for the row before it had an arena (its first stack): those share spans with
//! other rows, so they take turns by their own present bits, as every page once did.
use crate::app_resource_state::PrivatePage;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::app_catalog::{App, INCARNATIONS};
use spin::Mutex;

pub const SPAN: u64 = 2 * 1024 * 1024;
const USER: u64 = 1 << 2;
const PRESENT: u64 = 1;
const ROWS: usize = shared_rows();
const fn shared_rows() -> usize {
    let mut n = 0; let mut i = 0;
    while i < INCARNATIONS.len() {
        let row = &INCARNATIONS[i];
        if row.app as u8 == App::Program as u8 && sex_pdx::organs::program_key(row.ordinal) == sex_pdx::organs::SHARED_PROGRAM_KEY { n += 1; }
        i += 1;
    }
    n
}
/// `spans`: the page-directory entries of every span the row has used, sorted; tables
/// are never taken back, so a span stays the row's. `owned`: where its spans may lie.
/// `loose`: BOOT's pages for the row, outside any span it owns.
struct Row { target: u32, spans: Vec<u64>, owned: Vec<(u64, u64)>, loose: Vec<PrivatePage> }
struct Pool { rows: [Row; ROWS], tenant: u32 }
static POOL: Mutex<Pool> = Mutex::new(Pool { rows: [const { Row { target: 0, spans: Vec::new(), owned: Vec::new(), loose: Vec::new() } }; ROWS], tenant: 0 });
pub static SWITCHES: AtomicU64 = AtomicU64::new(0);
static SWITCH_CYCLES: AtomicU64 = AtomicU64::new(0);

pub fn shared(target: u32) -> bool {
    sex_pdx::app_catalog::by_target(target).is_some_and(|row| row.app == App::Program && sex_pdx::organs::program_key(row.ordinal) == sex_pdx::organs::SHARED_PROGRAM_KEY)
}
fn entry(at: u64) -> &'static AtomicU64 { unsafe { &*(at as *const AtomicU64) } }
fn open(spans: &[u64], on: bool) {
    for &at in spans {
        let _ = entry(at).fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| (v & 1 != 0).then_some(if on { v | USER } else { v & !USER }));
    }
}
fn leaves(pages: &[PrivatePage], on: bool) {
    for page in pages {
        // A leaf cleared since it was written stays cleared.
        let _ = entry(page.leaf).fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| (v != 0).then_some(if on { v | PRESENT } else { v & !PRESENT }));
    }
}
fn turn(row: &Row, on: bool) { open(&row.spans, on); leaves(&row.loose, on); }
fn loose(target: u32, leaf: u64) -> bool {
    shared(target) && POOL.lock().rows.iter().any(|r| r.target == target && r.loose.iter().any(|p| p.leaf == leaf))
}
/// How a check sees a leaf: a loose page out of its turn still counts as the row's own.
pub fn read(target: u32, leaf: u64) -> u64 {
    let v = unsafe { core::ptr::read_volatile(leaf as *const u64) };
    if v != 0 && loose(target, leaf) { v | PRESENT } else { v }
}
/// What a restore writes: a loose page waits, absent, for the row's turn.
pub fn restored(target: u32, leaf: u64, value: u64) -> u64 { if loose(target, leaf) { value & !PRESENT } else { value } }
fn row_mut(pool: &mut Pool, target: u32) -> &mut Row {
    let index = match pool.rows.iter().position(|r| r.target == target) { Some(i) => i, None => pool.rows.iter().position(|r| r.target == 0).expect("PROGRAM_POOL_ROWS") };
    let row = &mut pool.rows[index];
    row.target = target;
    row
}
/// The span over `va` joins the row's turns. Returns its entry.
fn note(row: &mut Row, va: u64) -> u64 {
    assert!(row.owned.iter().any(|&(base, end)| va >= base && va < end), "PROGRAM_POOL_SPAN_NOT_OWNED");
    let at = crate::boot_memory::span_entry(va).expect("PROGRAM_POOL_SPAN_ENTRY");
    if let Err(i) = row.spans.binary_search(&at) { row.spans.insert(i, at); }
    at
}
/// A row stepping out of its turn closes its spans.
fn step_out(pool: &mut Pool, target: u32) {
    if pool.tenant != target { return; }
    if let Some(row) = pool.rows.iter().find(|r| r.target == target) { turn(row, false); }
    pool.tenant = 0;
}
/// At BOOT: the row's arena, which no one else maps into.
pub fn arena(target: u32, base: u64, bytes: u64) {
    if !shared(target) { return; }
    assert!(base % SPAN == 0 && bytes % SPAN == 0, "PROGRAM_POOL_ARENA_SPAN");
    row_mut(&mut POOL.lock(), target).owned.push((base, base + bytes));
}
/// At BOOT: the pages BOOT made for the row before its arena.
pub fn boot_pages(target: u32, pages: &[PrivatePage]) {
    if !shared(target) { return; }
    row_mut(&mut POOL.lock(), target).loose.extend_from_slice(pages);
}
/// At BOOT, before anything runs: every region a shared row holds joins its turns.
pub fn claim_regions() {
    for row in INCARNATIONS.iter().filter(|r| shared(r.target)) {
        let pd = crate::ipc::DOMAIN_REGISTRY.get(row.target).expect("PROGRAM_POOL_DOMAIN");
        let mut pool = POOL.lock();
        let entry = row_mut(&mut pool, row.target);
        let mut pages = 0;
        for slot in 0..1024u32 {
            let Some(crate::capability::CapabilityData::Region(region)) = pd.find_capability(slot).map(|c| c.data) else { continue };
            assert!(region.virtual_base % SPAN == 0, "PROGRAM_POOL_REGION_SPAN");
            let end = region.virtual_base + region.length.next_multiple_of(SPAN);
            entry.owned.push((region.virtual_base, end));
            for va in (region.virtual_base..end).step_by(SPAN as usize) { note(entry, va); }
            pages += region.length / 4096;
        }
        open(&entry.spans, false);
        crate::serial_println!("[boot.program.pool.regions] target={} pages={} spans={}", row.target, pages, entry.spans.len());
    }
    x86_64::instructions::tlb::flush_all();
}
/// The row's pages, from the restore that just wrote them. Their spans stay closed
/// until the row's turn.
pub fn adopt(target: u32, pages: &[PrivatePage]) {
    if !shared(target) { return; }
    let mut pool = POOL.lock();
    let pool = &mut *pool;
    step_out(pool, target);
    let row = row_mut(pool, target);
    let mut last = u64::MAX;
    for page in pages {
        if row.loose.iter().any(|p| p.leaf == page.leaf) { continue; }
        if page.va / SPAN != last { note(row, page.va); last = page.va / SPAN; }
    }
}
/// A running row was given one more page: its leaf is set, and its span opens now only
/// if the row holds the turn.
pub fn grant(target: u32, page: PrivatePage) {
    if !shared(target) {
        unsafe { core::ptr::write_volatile(page.leaf as *mut u64, page.expected) };
        return;
    }
    let mut pool = POOL.lock();
    let pool = &mut *pool;
    let tenant = pool.tenant == target;
    let row = row_mut(pool, target);
    let at = note(row, page.va);
    unsafe { core::ptr::write_volatile(page.leaf as *mut u64, page.expected) };
    if tenant {
        open(&[at], true);
        x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(page.va));
    }
}
/// Before a retire clears the leaves: the row stops taking turns.
pub fn release(target: u32) {
    if !shared(target) { return; }
    step_out(&mut POOL.lock(), target);
}
/// Called before any task runs. Only the shared row about to run has its spans open.
pub fn before_dispatch(pd: u64) {
    let Ok(target) = u32::try_from(pd) else { return };
    if !shared(target) { return; }
    assert_eq!(crate::core_local::CoreLocal::get().core_id, crate::smp::home_core(pd), "PROGRAM_POOL_CORE");
    let mut pool = POOL.lock();
    if pool.tenant == target { return; }
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let old = pool.tenant;
    if let Some(row) = pool.rows.iter().find(|r| r.target == old && old != 0) { turn(row, false); }
    let spans = pool.rows.iter().find(|r| r.target == target).map_or(0, |row| { turn(row, true); row.spans.len() });
    // Shared rows run on this core only, so only its translations can be stale.
    x86_64::instructions::tlb::flush_all();
    pool.tenant = target;
    let spent = unsafe { core::arch::x86_64::_rdtsc() } - start;
    let cycles = SWITCH_CYCLES.fetch_add(spent, Ordering::Relaxed) + spent;
    let n = SWITCHES.fetch_add(1, Ordering::Relaxed) + 1;
    if n.is_power_of_two() {
        crate::serial_println!("[program.pool.switch] switches={} target={} spans={} avg_cycles={}", n, target, spans, cycles / n);
    }
}
