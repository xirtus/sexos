// SPDX-License-Identifier: GPL-3.0-only
//! Windowed program rows past the first few share one hardware key, as there are only
//! sixteen. Each of them runs in a protection view of its own (`views`), so rows run
//! side by side on any core and a row can never reach another's memory.
//!
//! Views separate rows by span, not by page. Every page a shared row holds lies in a
//! 2 MiB span no one else maps into: its arena, its regions (BOOT places them on whole
//! spans), and the spans of its boot image and first stack, which BOOT lays out 16 MiB
//! apart per domain (checked here at BOOT). The user bit of a span's page-directory
//! entry is clear in the canonical tables and in every other view, and set in the
//! row's own view, so 512 pages open or close by one entry, and leaves never change for
//! a switch. A switch is one CR3 write with no flush.
use crate::app_resource_state::PrivatePage;
use alloc::vec::Vec;
use sex_pdx::app_catalog::{App, INCARNATIONS};
use spin::Mutex;

pub const SPAN: u64 = 2 * 1024 * 1024;
pub const ROWS: usize = shared_rows();
const fn is_shared(i: usize) -> bool {
    let row = &INCARNATIONS[i];
    row.app as u8 == App::Program as u8 && sex_pdx::organs::program_key(row.ordinal) == sex_pdx::organs::SHARED_PROGRAM_KEY
}
const fn shared_rows() -> usize {
    let mut n = 0; let mut i = 0;
    while i < INCARNATIONS.len() { if is_shared(i) { n += 1; } i += 1; }
    n
}
/// `spans`: every span the row has used, sorted; tables are never taken back, so a span
/// stays the row's. `owned`: where its spans may lie. `boot`: BOOT's pages for the row
/// before it had an arena, whose spans join at `claim_regions`.
struct Row { spans: Vec<u64>, owned: Vec<(u64, u64)>, boot: Vec<PrivatePage> }
static POOL: Mutex<[Row; ROWS]> = Mutex::new([const { Row { spans: Vec::new(), owned: Vec::new(), boot: Vec::new() } }; ROWS]);

/// By target: the row's view, 1 + its place among the shared rows, or 0.
static VIEW_OF: [u8; 256] = view_table();
const fn view_table() -> [u8; 256] {
    let mut table = [0; 256]; let (mut i, mut n) = (0, 0);
    while i < INCARNATIONS.len() {
        if is_shared(i) { n += 1; table[INCARNATIONS[i].target as usize] = n; }
        i += 1;
    }
    table
}
pub fn shared(target: u32) -> bool { view_of(target).is_some() }
pub fn view_of(target: u32) -> Option<usize> {
    match VIEW_OF.get(target as usize) { Some(&v) if v != 0 => Some(v as usize), _ => None }
}
/// How a check sees a leaf.
pub fn read(_target: u32, leaf: u64) -> u64 { unsafe { core::ptr::read_volatile(leaf as *const u64) } }
/// What a restore writes.
pub fn restored(_target: u32, _leaf: u64, value: u64) -> u64 { value }
/// The span over `va` joins the row's view.
fn note(row: &mut Row, view: usize, va: u64) {
    assert!(row.owned.iter().any(|&(base, end)| va >= base && va < end), "PROGRAM_POOL_SPAN_NOT_OWNED");
    let span = va & !(SPAN - 1);
    if let Err(i) = row.spans.binary_search(&span) {
        row.spans.insert(i, span);
        crate::views::open(view, span);
    }
}
fn with_row(target: u32, f: impl FnOnce(&mut Row, usize)) {
    let Some(view) = view_of(target) else { return };
    f(&mut POOL.lock()[view - 1], view);
}
/// At BOOT: the row's arena, which no one else maps into.
pub fn arena(target: u32, base: u64, bytes: u64) {
    assert!(base % SPAN == 0 && bytes % SPAN == 0, "PROGRAM_POOL_ARENA_SPAN");
    with_row(target, |row, _| row.owned.push((base, base + bytes)));
}
/// At BOOT: the pages BOOT made for the row before its arena. Their spans join the row's
/// view at once if views exist (BOOT reserves successors after `claim_regions`).
pub fn boot_pages(target: u32, pages: &[PrivatePage]) {
    let Some(view) = view_of(target) else { return };
    let mut pool = POOL.lock();
    pool[view - 1].boot.extend_from_slice(pages);
    if crate::views::ready() { claim_boot(&mut pool, view); }
}
/// The spans of the row's BOOT pages not yet its own become its own: closed everywhere
/// but in its view. Returns how many.
fn claim_boot(pool: &mut [Row; ROWS], view: usize) -> usize {
    let mut boot: Vec<u64> = pool[view - 1].boot.iter().map(|p| p.va & !(SPAN - 1)).collect();
    boot.sort_unstable(); boot.dedup();
    boot.retain(|&span| !pool[view - 1].owned.iter().any(|&(b, e)| span >= b && span < e));
    for &span in &boot {
        assert!(boot_span_alone(&*pool, view - 1, span), "PROGRAM_POOL_BOOT_SPAN_SHARED");
        let row = &mut pool[view - 1];
        row.owned.push((span, span + SPAN));
        crate::views::close(view, span);
        note(row, view, span);
    }
    boot.len()
}
/// A span of BOOT's pages for a row is the row's alone: every page in it carries the
/// shared key, and no other row's BOOT page lies in it.
fn boot_span_alone(pool: &[Row], me: usize, span: u64) -> bool {
    let Some(leaf) = crate::boot_memory::existing_leaf_address(span) else { return false };
    let table = leaf & !0xFFF;
    let others = |va: u64| pool.iter().enumerate().any(|(i, r)| i != me && r.boot.iter().any(|p| p.va & !(SPAN - 1) == va));
    (0..512u64).all(|i| {
        let v = unsafe { core::ptr::read_volatile((table + i * 8) as *const u64) };
        v == 0 || (v >> 59) & 15 == sex_pdx::organs::SHARED_PROGRAM_KEY as u64
    }) && !others(span)
}
/// At BOOT, before anything runs: views exist, and every span a shared row holds is
/// closed everywhere but in its own view.
pub fn claim_regions() {
    crate::views::init();
    let mut pool = POOL.lock();
    for (target, view) in INCARNATIONS.iter().filter_map(|r| view_of(r.target).map(|v| (r.target, v))) {
        let pd = crate::ipc::DOMAIN_REGISTRY.get(target).expect("PROGRAM_POOL_DOMAIN");
        let mut pages = 0;
        let mut owned = Vec::new();
        for slot in 0..1024u32 {
            let Some(crate::capability::CapabilityData::Region(region)) = pd.find_capability(slot).map(|c| c.data) else { continue };
            assert!(region.virtual_base % SPAN == 0, "PROGRAM_POOL_REGION_SPAN");
            owned.push((region.virtual_base, region.virtual_base + region.length.next_multiple_of(SPAN)));
            pages += region.length / 4096;
        }
        let boot = claim_boot(&mut pool, view);
        let row = &mut pool[view - 1];
        row.owned.extend_from_slice(&owned);
        for &(base, end) in &owned {
            for va in (base..end).step_by(SPAN as usize) { crate::views::close(view, va); note(row, view, va); }
        }
        crate::serial_println!("[boot.program.pool.regions] target={} pages={} spans={} view={} boot_spans={}", target, pages, row.spans.len(), view, boot);
    }
    crate::views::resync();
}
/// The row's pages, from the restore that just wrote them. A run starts here, so views
/// also take any table the canonical root gained outside `boot_memory::arena_leaf`.
pub fn adopt(target: u32, pages: &[PrivatePage]) {
    if !shared(target) { return; }
    crate::views::resync();
    with_row(target, |row, view| for page in pages { note(row, view, page.va); });
}
/// A row was given one more page: its leaf is set, and its span joins its view.
pub fn grant(target: u32, page: PrivatePage) {
    with_row(target, |row, view| note(row, view, page.va));
    unsafe { core::ptr::write_volatile(page.leaf as *mut u64, page.expected) };
}
/// Before a retire clears the leaves. Spans stay the row's.
pub fn release(_target: u32) {}
/// Called before any task runs: its view.
pub fn before_dispatch(pd: u64) {
    crate::views::enter(u32::try_from(pd).ok().and_then(view_of).unwrap_or(0));
}
