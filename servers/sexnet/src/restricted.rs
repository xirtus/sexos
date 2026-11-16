//! Deny-only restricted program role. Net never receives job data authority.
use core::sync::atomic::{AtomicU64, Ordering};
use sex_pdx::restricted_helper_journal::Journal;
use sex_pdx::restricted_helper_provider::{Platform, Provider};
use sex_pdx::{app_catalog as catalog, app_lifecycle as life, restricted_helper::Subject};
pub static JOURNAL: Journal = Journal::new();
static DENIED: AtomicU64 = AtomicU64::new(u64::MAX);
const _: () = assert!(catalog::count(catalog::App::Program) <= 64);
fn index(target: u32) -> Option<usize> {
    catalog::INCARNATIONS
        .iter()
        .filter(|r| r.app == catalog::App::Program)
        .position(|r| r.target == target)
}
pub fn denied(target: u32) -> bool {
    index(target).is_some_and(|k| DENIED.load(Ordering::Acquire) & (1u64 << k) != 0)
}
pub fn publish(provider: &Provider) {
    let mut mask = 0;
    for (k, row) in catalog::INCARNATIONS
        .iter()
        .filter(|r| r.app == catalog::App::Program)
        .enumerate()
    {
        if provider.denied(row.target) {
            mask |= 1u64 << k
        }
    }
    DENIED.store(mask, Ordering::Release);
}
pub struct Kernel;
impl Platform for Kernel {
    fn epoch(&mut self) -> Option<u64> {
        sex_pdx::crash_log::self_epoch().ok()
    }
    fn keep(&mut self, address: u64) -> bool {
        sex_pdx::crash_log::keep(address, 1)
    }
    fn challenge(&mut self) -> Option<[u64; 2]> {
        sex_pdx::restricted_helper::fresh_challenge()
    }
    fn catalog(&mut self, target: u32) -> Option<Subject> {
        let row = catalog::by_target(target).filter(|row| row.app == catalog::App::Program)?;
        let c = life::catalog(row.lifecycle_slot).ok()?;
        c.is_subject(row.target, row.surface, 3).then_some(Subject {
            pd: c.target_pd,
            task: c.task_id,
        })
    }
    fn now(&mut self) -> u64 {
        sex_pdx::get_ticks()
    }
    fn reply(&mut self, value: u64) -> bool {
        sex_pdx::pdx_reply(3, value) == 0
    }
}
pub fn scrub(subject: Subject) -> bool {
    use sex_pdx::{
        memory_region::{QUERY_VIEW, READ, WRITE},
        web as w,
    };
    if Kernel.catalog(subject.pd) != Some(subject) {
        return false;
    }
    let Some(index) = index(subject.pd).map(|k| w::PROGRAM_INDEX_BASE + k) else {
        return false;
    };
    let slot = w::PROVIDER_SLOT_BASE + index as u64;
    let Ok(info) = sex_pdx::memory_region_info(slot) else {
        return false;
    };
    if info.region_id != w::REGION_ID_BASE + index as u32
        || info.peer_pd != Some(subject.pd)
        || info.length != w::REGION_BYTES as u64
        || info.length % 4096 != 0
        || info.rights & (QUERY_VIEW | READ | WRITE) != (QUERY_VIEW | READ | WRITE)
    {
        return false;
    }
    let Ok(view) = sex_pdx::memory_region_view(slot, 0, info.length, QUERY_VIEW | READ | WRITE)
    else {
        return false;
    };
    if view.region_id != info.region_id
        || view.length != info.length
        || view.virtual_address == 0
        || view.virtual_address % 4096 != 0
    {
        return false;
    }
    unsafe {
        let base = view.virtual_address as *mut u64;
        for n in 0..view.length as usize / 8 {
            core::ptr::write_volatile(base.add(n), 0)
        }
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    true
}
