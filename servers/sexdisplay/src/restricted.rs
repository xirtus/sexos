//! Deny-only display role. Shared text/pixel grants are read-only here; the
//! attested bootstrap must erase its own complete grants before plugin entry.
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

pub fn quiet(subject: Subject) -> bool {
    if Kernel.catalog(subject.pd) != Some(subject) {
        return false;
    }
    let Some(row) = catalog::by_target(subject.pd) else {
        return false;
    };
    unsafe {
        if (*core::ptr::addr_of!(crate::RENDER_SCENE)).is_valid() {
            return false;
        }
        if (&*core::ptr::addr_of!(crate::SURFACES))
            .iter()
            .any(|surface| {
                row_busy(
                    subject,
                    row.surface,
                    row.frame(),
                    surface.owner_pd,
                    surface.surface_id,
                    surface.active,
                )
            })
        {
            return false;
        }
        // All of this row's old transactions are narrowing-only cleanup; no live
        // scene may borrow them. Other producers and their surfaces are untouched.
        for stage in &mut *core::ptr::addr_of_mut!(crate::TEXT_STAGES) {
            if stage.diagnostic().0 == subject.pd {
                stage.reset();
            }
        }
        if crate::TOKEN_BUF_OWNER == subject.pd {
            crate::TOKEN_BUF_OWNER = 0;
            crate::TOKEN_BUF_CALL1_RECEIVED = false;
            core::ptr::write_volatile(core::ptr::addr_of_mut!(crate::TOKEN_BUF_ARG0), 0);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(crate::TOKEN_BUF_ARG1), 0);
        }
    }
    true
}
fn row_busy(
    subject: Subject,
    content: u64,
    frame: u64,
    owner: u32,
    sid: u64,
    active: bool,
) -> bool {
    active && (owner == subject.pd || sid == content || sid == frame)
}
