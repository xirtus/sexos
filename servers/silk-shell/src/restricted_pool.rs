//! Deny-only durable reservation. A former helper row never silently returns to
//! the ordinary allocator. No positive job or pointer survives Shell restart.
use sex_pdx::restricted_helper::{Subject, MAX_GENERATION};
use sex_pdx::restricted_helper_journal::{Journal, Ledger};
pub struct Pool {
    ledger: Ledger,
    ready: bool,
    recovering: bool,
}
impl Pool {
    pub const fn new() -> Self {
        Self {
            ledger: Ledger::quarantine_all(),
            ready: false,
            recovering: true,
        }
    }
    pub fn initialize(
        &mut self,
        journal: &Journal,
        epoch: Option<u64>,
        kept: bool,
        programs: &[u32],
    ) {
        self.ledger = journal.load(epoch, kept);
        if self.ledger.subjects().count() > 2
            || self.ledger.subjects().any(|s| !programs.contains(&s.pd))
        {
            self.ledger = Ledger::quarantine_all();
        }
        self.ready = kept && !self.ledger.all_denied();
        self.recovering = epoch != Some(1) && programs.iter().any(|pd| self.ledger.denied(*pd));
        if kept {
            journal.persist(&self.ledger);
        }
    }
    pub fn reserved(&self, pd: u32) -> bool {
        self.ledger.denied(pd)
    }
    pub fn normal(&self) -> bool {
        self.ready && !self.recovering
    }
    pub fn recovering(&self) -> bool {
        self.recovering
    }
    pub fn recovered(&mut self) {
        if self.ready {
            self.recovering = false;
        }
    }
    /// Caller supplies the exact owned READY/REUSABLE catalog subject, never a
    /// parent-selected target. Reservation is persisted before any remote Q.
    pub fn reserve(
        &mut self,
        journal: &Journal,
        subject: Subject,
        programs: &[u32],
    ) -> Option<u64> {
        if !self.normal() || !programs.contains(&subject.pd) {
            return None;
        }
        let count = programs
            .iter()
            .filter(|pd| self.ledger.denied(**pd))
            .count();
        if !self.ledger.denied(subject.pd) && count >= 2 {
            return None;
        }
        let generation = self
            .ledger
            .maximum_generation()
            .checked_add(1)
            .filter(|g| *g <= MAX_GENERATION)?;
        if !self.ledger.quarantine(subject, generation) {
            return None;
        }
        journal.persist(&self.ledger);
        Some(generation)
    }
    pub fn bound(
        &mut self,
        journal: &Journal,
        previous: Subject,
        child: Subject,
        generation: u64,
    ) -> bool {
        if !self.ready || !self.ledger.bind_task(previous, child, generation) {
            return false;
        }
        journal.persist(&self.ledger);
        true
    }
}
#[cfg(target_os = "none")]
mod native {
    use super::*;
    use sex_pdx::{app_catalog as c, app_lifecycle as a};
    static JOURNAL: Journal = Journal::new();
    static mut POOL: Pool = Pool::new();
    fn programs() -> [u32; c::count(c::App::Program)] {
        let mut out = [0; c::count(c::App::Program)];
        for (i, row) in c::INCARNATIONS
            .iter()
            .filter(|r| r.app == c::App::Program && r.activation_slot.is_some())
            .enumerate()
        {
            out[i] = row.target;
        }
        out
    }
    pub fn initialize() {
        let epoch = sex_pdx::crash_log::self_epoch().ok();
        let kept = sex_pdx::crash_log::keep(JOURNAL.address(), 1);
        unsafe {
            (&mut *core::ptr::addr_of_mut!(POOL)).initialize(&JOURNAL, epoch, kept, &programs())
        }
    }
    pub fn reserved(pd: u32) -> bool {
        c::by_target(pd).is_some_and(|r| r.app == c::App::Program)
            && unsafe { (&*core::ptr::addr_of!(POOL)).reserved(pd) }
    }
    pub fn blocks(msg: &sex_pdx::PdxMessage) -> bool {
        reserved(msg.caller_pd) && msg.type_id != sex_pdx::launch::OP_EXIT
    }
    pub fn normal() -> bool {
        unsafe { (&*core::ptr::addr_of!(POOL)).normal() }
    }
    pub fn reserve(target: u32) -> Option<(Subject, usize, u64, u64)> {
        let row = c::by_target(target).filter(|r| r.app == c::App::Program)?;
        let slot = row.activation_slot?;
        let edge = a::activation(slot, false).ok()?;
        if !matches!(edge.state, a::ACTIVATION_READY | a::ACTIVATION_REUSABLE)
            || (edge.successor, edge.surface) != (row.target, row.surface)
        {
            return None;
        }
        let own = a::catalog(row.lifecycle_slot).ok()?;
        if !own.is_subject(row.target, row.surface, 3) || own.task_id != edge.task {
            return None;
        }
        let subject = Subject {
            pd: own.target_pd,
            task: own.task_id,
        };
        let generation = unsafe {
            (&mut *core::ptr::addr_of_mut!(POOL)).reserve(&JOURNAL, subject, &programs())
        }?;
        Some((subject, slot, edge.state, generation))
    }
    pub fn bound(previous: Subject, child: Subject, generation: u64) -> bool {
        unsafe {
            (&mut *core::ptr::addr_of_mut!(POOL)).bound(&JOURNAL, previous, child, generation)
        }
    }
    pub fn recover() {
        if !unsafe { (&*core::ptr::addr_of!(POOL)).recovering() } {
            return;
        }
        let mut done = true;
        for row in c::INCARNATIONS
            .iter()
            .filter(|r| r.app == c::App::Program && reserved(r.target))
        {
            let Some(slot) = row.activation_slot else {
                done = false;
                continue;
            };
            match a::activation(slot, false) {
                Ok(edge) if matches!(edge.state, a::ACTIVATION_READY | a::ACTIVATION_REUSABLE) => {}
                Ok(edge) if edge.state == a::ACTIVATION_ACTIVE => {
                    let _ = a::force_end(row.lifecycle_slot);
                    done = false;
                }
                _ => done = false, // uncertain dormant/rebirthing rows remain quarantined
            }
        }
        if done {
            unsafe {
                (&mut *core::ptr::addr_of_mut!(POOL)).recovered();
            }
        }
    }
}
#[cfg(target_os = "none")]
pub use native::*;
