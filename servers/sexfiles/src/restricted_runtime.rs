//! Restricted row deny fence, initialized before replay. Positive immutable job
//! admission is deliberately unavailable until the job snapshot layer is wired.
use sex_pdx::restricted_helper::{Subject, OP_FETCH, OP_POLICY};
use sex_pdx::restricted_helper_journal::Journal;
use sex_pdx::restricted_helper_provider::{Effect, Message, Platform, Provider};
use sex_pdx::{app_catalog as catalog, app_lifecycle as life, fs as f};
static JOURNAL: Journal = Journal::new();
static mut STATE: Provider = Provider::new();
use core::sync::atomic::{AtomicBool, Ordering};
static ENTERED: AtomicBool = AtomicBool::new(false);
struct Turn;
impl Turn {
    fn enter() -> Self {
        assert!(
            !ENTERED.swap(true, Ordering::Acquire),
            "RESTRICTED_FILES_REENTRY"
        );
        Self
    }
    fn state(&mut self) -> &mut Provider {
        unsafe { &mut *core::ptr::addr_of_mut!(STATE) }
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        ENTERED.store(false, Ordering::Release)
    }
}
struct Kernel;
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
pub fn initialize() {
    Turn::enter().state().initialize(&JOURNAL, &mut Kernel)
}
pub fn pending() -> bool {
    Turn::enter().state().pending()
}
pub fn denied(pd: u32) -> bool {
    if catalog::by_target(pd).is_none_or(|r| r.app != catalog::App::Program) {
        return false;
    }
    Turn::enter().state().denied(pd)
}
pub fn message(opcode: u64, caller: u32, task: u32, words: [u64; 3]) -> bool {
    if opcode != OP_POLICY && opcode != OP_FETCH {
        return false;
    }
    Turn::enter().state().message(
        Message {
            opcode,
            caller: Subject { pd: caller, task },
            words,
        },
        &JOURNAL,
        &mut Kernel,
    )
}
pub fn poll() {
    Turn::enter()
        .state()
        .poll(&mut Kernel, |effect| match effect {
            Effect::Scrub(subject) => scrub(subject),
            // No ACK/ready can activate an unimplemented positive rights path.
            Effect::Install(_) => false,
            Effect::Expire(_) => true,
        })
}
fn scrub(subject: Subject) -> bool {
    if Kernel.catalog(subject.pd) != Some(subject) {
        return false;
    }
    if !crate::fs_service::quarantine_program(subject.pd) {
        return false;
    }
    let Some(index) = catalog::INCARNATIONS
        .iter()
        .filter(|r| r.app == catalog::App::Program)
        .position(|r| r.target == subject.pd)
    else {
        return false;
    };
    let index = f::PROGRAM_INDEX_BASE + index;
    let slot = f::PROVIDER_SLOT_BASE + index as u64;
    let Ok(info) = sex_pdx::memory_region_info(slot) else {
        return false;
    };
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    let Ok(view) = sex_pdx::memory_region_view(slot, 0, info.length, QUERY_VIEW | READ | WRITE)
    else {
        return false;
    };
    if !scrub_shape(subject, index, &info, &view) {
        return false;
    }
    // Entire grant, including stat cache and all mapped page padding. Never round
    // beyond the immutable capability extent into an adjacent region.
    unsafe {
        erase_words(view.virtual_address as *mut u64, view.length);
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    true
}

fn scrub_shape(
    subject: Subject,
    index: usize,
    info: &sex_pdx::memory_region::MemoryRegionInfo,
    view: &sex_pdx::memory_region::RegionView,
) -> bool {
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    info.region_id == f::REGION_ID_BASE + index as u32
        && info.peer_pd == Some(subject.pd)
        && info.length == f::REGION_BYTES as u64
        && info.length % 4096 == 0
        && info.rights & (QUERY_VIEW | READ | WRITE) == (QUERY_VIEW | READ | WRITE)
        && view.region_id == info.region_id
        && view.length == info.length
        && view.virtual_address != 0
        && view.virtual_address % 4096 == 0
}
unsafe fn erase_words(base: *mut u64, length: u64) {
    for i in 0..length as usize / 8 {
        unsafe { core::ptr::write_volatile(base.add(i), 0) }
    }
}
