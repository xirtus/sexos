// SPDX-License-Identifier: GPL-3.0-only
//! Windowed program rows past the first few share one hardware key, as there are only
//! sixteen. They take turns: only the row about to run has its pages present, so a row
//! can never reach another's memory. All of them run on core 0, one at a time, which
//! is what makes one set of present bits enough in a single address space.
use crate::app_resource_state::{PrivatePage, MAX_PAGES};
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::app_catalog::{App, INCARNATIONS};
use spin::Mutex;

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
struct Row { target: u32, pages: [PrivatePage; MAX_PAGES], count: usize }
struct Pool { rows: [Row; ROWS], tenant: u32 }
static POOL: Mutex<Pool> = Mutex::new(Pool { rows: [const { Row { target: 0, pages: [PrivatePage::EMPTY; MAX_PAGES], count: 0 } }; ROWS], tenant: 0 });
pub static SWITCHES: AtomicU64 = AtomicU64::new(0);

pub fn shared(target: u32) -> bool {
    sex_pdx::app_catalog::by_target(target).is_some_and(|row| row.app == App::Program && sex_pdx::organs::program_key(row.ordinal) == sex_pdx::organs::SHARED_PROGRAM_KEY)
}
fn leaf(at: u64) -> &'static AtomicU64 { unsafe { &*(at as *const AtomicU64) } }
fn present(pages: &[PrivatePage], on: bool) {
    for page in pages {
        // A leaf cleared since it was adopted stays cleared.
        let _ = leaf(page.leaf).fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| (v != 0).then_some(if on { v | PRESENT } else { v & !PRESENT }));
        x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(page.va));
    }
}
/// How a check sees a leaf: an evicted page still counts as the row's own.
pub fn read(target: u32, at: u64) -> u64 {
    let v = unsafe { core::ptr::read_volatile(at as *const u64) };
    if v != 0 && shared(target) { v | PRESENT } else { v }
}
/// What a restore writes: a shared row's pages wait, absent, for its turn.
pub fn restored(target: u32, value: u64) -> u64 { if shared(target) { value & !PRESENT } else { value } }
/// The row's pages, from the restore that just wrote them.
pub fn adopt(target: u32, pages: &[PrivatePage]) {
    if !shared(target) { return; }
    let mut pool = POOL.lock();
    let pool = &mut *pool;
    if pool.tenant == target { pool.tenant = 0; }
    let row = pool.rows.iter_mut().find(|r| r.target == target).or_else(|| None);
    let row = match row { Some(row) => row, None => pool.rows.iter_mut().find(|r| r.target == 0).expect("PROGRAM_POOL_ROWS") };
    row.target = target;
    row.pages[..pages.len()].copy_from_slice(pages);
    row.count = pages.len();
}
/// Before a retire clears the leaves: the row stops taking turns.
pub fn release(target: u32) {
    if !shared(target) { return; }
    let mut pool = POOL.lock();
    if pool.tenant == target { pool.tenant = 0; }
    if let Some(row) = pool.rows.iter_mut().find(|r| r.target == target) { row.count = 0; }
}
/// Called before any task runs. Only the shared row about to run has pages present.
pub fn before_dispatch(pd: u64) {
    let Ok(target) = u32::try_from(pd) else { return };
    if !shared(target) { return; }
    assert_eq!(crate::core_local::CoreLocal::get().core_id, 0, "PROGRAM_POOL_CORE");
    let mut pool = POOL.lock();
    if pool.tenant == target { return; }
    let old = pool.tenant;
    if let Some(row) = pool.rows.iter().find(|r| r.target == old && old != 0) { present(&row.pages[..row.count], false); }
    if let Some(row) = pool.rows.iter().find(|r| r.target == target) { present(&row.pages[..row.count], true); }
    pool.tenant = target;
    SWITCHES.fetch_add(1, Ordering::Relaxed);
}
