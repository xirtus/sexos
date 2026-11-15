//! Held-capability bridge for restricted offline jobs. No filename, parent hash,
//! requested target or claimed task supplies authority. Sole Shell main turn.
use crate::restricted_coordinator::{Activation, Coordinator, Platform, Reservation, Service};
use core::sync::atomic::{AtomicBool, Ordering};
use sex_pdx::{
    app_catalog as catalog, app_lifecycle as life,
    restricted_helper::{self as wire, Subject},
    restricted_job as job, PdxMessage,
};
static mut CORE: Coordinator = Coordinator::new();
static ENTERED: AtomicBool = AtomicBool::new(false);
struct Turn;
impl Turn {
    fn enter() -> Self {
        assert!(
            !ENTERED.swap(true, Ordering::Acquire),
            "RESTRICTED_SHELL_REENTRY"
        );
        Self
    }
    fn core(&mut self) -> &mut Coordinator {
        unsafe { &mut *core::ptr::addr_of_mut!(CORE) }
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        ENTERED.store(false, Ordering::Release);
    }
}
const NOTICES: usize = catalog::count(catalog::App::Program) * 2;
#[derive(Clone, Copy)]
struct Notice {
    parent: Subject,
    event: sex_pdx::launch::Event,
}
static mut OUTBOX: [Option<Notice>; NOTICES] = [None; NOTICES];
#[derive(Clone, Copy)]
struct StatusNotice {
    parent: Subject,
    value: job::Observation,
}
static mut STATUSBOX: [Option<StatusNotice>; NOTICES] = [None; NOTICES];
fn statuses() {
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(STATUSBOX) } {
        if let Some(n) = *slot {
            if !live(n.parent) || sex_pdx::crash_log::self_epoch().ok() != Some(n.value.shell_epoch)
            {
                *slot = None;
                continue;
            }
            let Some(row) = catalog::by_target(n.parent.pd) else {
                *slot = None;
                continue;
            };
            let Some(w) = n.value.words() else {
                *slot = None;
                continue;
            };
            if sex_pdx::pdx_call(row.route_slot, job::OP_STATUS_EVENT, w[0], w[1], w[2]).0 == 0 {
                *slot = None;
            }
        }
    }
}
fn status(actor: Subject, words: [u64; 3]) {
    if !live(actor) || crate::restricted_pool::reserved(actor.pd) {
        return;
    }
    let Some((cookie, ticket, known, epoch)) = job::status_decode(words) else {
        return;
    };
    if sex_pdx::crash_log::self_epoch().ok() != Some(epoch) {
        return;
    }
    let mut value = job::Observation {
        cookie,
        ticket,
        state: job::STATUS_UNKNOWN,
        recovery_complete: crate::restricted_pool::normal(),
        current: None,
        shell_epoch: epoch,
    };
    if let Some(known) = known {
        if known.pd == actor.pd || !crate::restricted_pool::reserved(known.pd) {
            return;
        }
        let Some(row) = catalog::by_target(known.pd).filter(|r| r.app == catalog::App::Program)
        else {
            return;
        };
        let Some(slot) = row.activation_slot else {
            return;
        };
        let Ok(own) = life::catalog(row.lifecycle_slot) else {
            return;
        };
        let Ok(a) = life::activation(slot, false) else {
            return;
        };
        if !own.is_subject(row.target, row.surface, 3)
            || own.task_id == 0
            || a.task != own.task_id
            || a.successor != row.target
            || a.surface != row.surface
        {
            return;
        }
        value.current = Some(Subject {
            pd: row.target,
            task: own.task_id,
        });
        value.state = if own.task_id != known.task || a.state == life::ACTIVATION_REUSABLE {
            job::STATUS_RETIRED
        } else if a.state == life::ACTIVATION_ACTIVE {
            job::STATUS_LIVE
        } else {
            job::STATUS_UNAVAILABLE
        };
    }
    let Some(i) = catalog::INCARNATIONS
        .iter()
        .filter(|r| r.app == catalog::App::Program)
        .position(|r| r.target == actor.pd)
    else {
        return;
    };
    // A full reply queue never grants anything and never overwrites another cookie.
    let slots = unsafe { &mut *core::ptr::addr_of_mut!(STATUSBOX) };
    if let Some(slot) = slots[i * 2..i * 2 + 2].iter_mut().find(|s| {
        s.is_none()
            || s.as_ref().is_some_and(|s| {
                s.parent == actor && s.value.cookie == cookie && s.value.ticket == ticket
            })
    }) {
        *slot = Some(StatusNotice {
            parent: actor,
            value,
        });
    }
}

fn route(service: Service) -> u64 {
    match service {
        Service::Files => sex_pdx::program_store::SHELL_ROUTE_SLOT,
        Service::Display => sex_pdx::SLOT_DISPLAY,
        Service::Net => sex_pdx::web::SHELL_SLOT,
        Service::Bell => sex_pdx::SLOT_BELL,
    }
}
fn catalog_subject(subject: Subject) -> bool {
    let Some(row) = catalog::by_target(subject.pd).filter(|r| r.app == catalog::App::Program)
    else {
        return false;
    };
    matches!(life::catalog(row.lifecycle_slot),Ok(c)if c.is_subject(row.target,row.surface,3)&&c.task_id==subject.task)
}
fn live(subject: Subject) -> bool {
    let Some(row) = catalog::by_target(subject.pd).filter(|r| r.app == catalog::App::Program)
    else {
        return false;
    };
    if !catalog_subject(subject) {
        return false;
    }
    matches!(row.activation_slot.and_then(|slot|life::activation(slot,false).ok()),Some(a)if a.state==life::ACTIVATION_ACTIVE&&a.task==subject.task&&a.successor==subject.pd)
}
pub fn send_event(parent: Subject, event: sex_pdx::launch::Event) -> bool {
    if !live(parent) {
        return false;
    }
    let Some(row) = catalog::by_target(parent.pd) else {
        return false;
    };
    let words = event.wire();
    sex_pdx::pdx_call(
        row.route_slot,
        sex_pdx::launch::OP_EVENT,
        words[0],
        words[1],
        words[2],
    )
    .0 == 0
}
fn refuse(parent: Subject, cookie: u32) {
    if !live(parent) {
        return;
    }
    let Some(i) = catalog::INCARNATIONS
        .iter()
        .filter(|r| r.app == catalog::App::Program)
        .position(|r| r.target == parent.pd)
    else {
        return;
    };
    let slots = unsafe { &mut *core::ptr::addr_of_mut!(OUTBOX) };
    if let Some(slot) = slots[i * 2..i * 2 + 2].iter_mut().find(|s| {
        s.as_ref()
            .is_some_and(|n| n.parent == parent && n.event.cookie() == cookie)
            || s.is_none()
    }) {
        *slot = Some(Notice {
            parent,
            event: sex_pdx::launch::Event::Refused {
                cookie,
                code: sex_pdx::program_store::NOT_ALLOWED,
            },
        });
    }
}
fn notices() {
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(OUTBOX) } {
        if let Some(n) = *slot {
            if !live(n.parent) || send_event(n.parent, n.event) {
                *slot = None;
            }
        }
    }
}
struct Kernel;
impl Platform for Kernel {
    fn now(&mut self) -> u64 {
        sex_pdx::get_ticks()
    }
    fn self_epoch(&mut self) -> Option<u64> {
        sex_pdx::crash_log::self_epoch().ok().filter(|e| *e != 0)
    }
    fn peer_epoch(&mut self, service: Service) -> Option<u64> {
        let p = sex_pdx::crash_log::peer_state(route(service)).ok()?;
        p.live().then_some(p.epoch)
    }
    fn live(&mut self, subject: Subject) -> bool {
        live(subject)
            && unsafe {
                crate::APP_CUSTODY
                    .iter()
                    .flatten()
                    .find(|e| e.target == subject.pd && e.task == subject.task)
                    .is_some_and(|e| !e.dead && !e.close_pending)
            }
    }
    fn reserve(&mut self) -> Option<Reservation> {
        if unsafe {
            crate::PROGRAM_READING
                || crate::PROGRAM_REBIRTH.is_some()
                || crate::RECORD_SPAWN_LEFT != 0
        } {
            return None;
        }
        for pass in 0..4 {
            for row in catalog::INCARNATIONS
                .iter()
                .filter(|r| r.app == catalog::App::Program && r.activation_slot.is_some())
            {
                if unsafe {
                    crate::LAUNCH_RUNS
                        .iter()
                        .flatten()
                        .any(|r| r.child == row.target)
                } {
                    continue;
                }
                let edge = life::activation(row.activation_slot?, false).ok()?;
                let reusable = edge.state == life::ACTIVATION_REUSABLE;
                let fresh = edge.state == life::ACTIVATION_READY;
                let reserved = crate::restricted_pool::reserved(row.target);
                let chosen = match pass {
                    0 => reserved && reusable,
                    1 => !reserved && reusable,
                    2 => reserved && fresh,
                    _ => !reserved && fresh,
                };
                if !chosen {
                    continue;
                }
                if let Some((previous, slot, state, generation)) =
                    crate::restricted_pool::reserve(row.target)
                {
                    sex_pdx::serial_record!(
                        "[moan.role.reserved] pd={} task={} generation={}",
                        previous.pd,
                        previous.task,
                        generation
                    );
                    return Some(Reservation {
                        previous,
                        slot,
                        fresh: state == life::ACTIVATION_READY,
                        generation,
                    });
                }
            }
        }
        None
    }
    fn bound(&mut self, r: Reservation, child: Subject) -> bool {
        let accepted = crate::restricted_pool::bound(r.previous, child, r.generation);
        if accepted {
            sex_pdx::serial_record!(
                "[moan.role.copied] pd={} task={} generation={}",
                child.pd,
                child.task,
                r.generation
            );
        }
        accepted
    }
    fn send(&mut self, service: Service, opcode: u64, words: [u64; 3]) -> bool {
        sex_pdx::pdx_call(route(service), opcode, words[0], words[1], words[2]).0 == 0
    }
    fn copy(&mut self, r: Reservation, length: u32) -> Option<Subject> {
        if !crate::restricted_pool::reserved(r.previous.pd) {
            return None;
        }
        let row = catalog::by_target(r.previous.pd)?;
        let loaded = life::load_program(r.slot, length).ok()?;
        if loaded.state
            != if r.fresh {
                life::ACTIVATION_READY
            } else {
                life::ACTIVATION_REBIRTHING
            }
            || (loaded.successor, loaded.surface) != (row.target, row.surface)
        {
            return None;
        }
        let own = life::catalog(row.lifecycle_slot).ok()?;
        (own.is_subject(row.target, row.surface, 3) && own.task_id == loaded.task).then_some(
            Subject {
                pd: own.target_pd,
                task: own.task_id,
            },
        )
    }
    fn activate(&mut self, r: Reservation, child: Subject) -> Activation {
        if !crate::restricted_pool::reserved(child.pd) || !catalog_subject(child) {
            return Activation::Failed;
        }
        let reply = if r.fresh {
            life::activation(r.slot, true)
        } else {
            life::rebirth(r.slot)
        };
        match reply {
            Ok(a)
                if a.state == life::ACTIVATION_ACTIVE
                    && a.successor == child.pd
                    && a.task == child.task
                    && catalog::by_target(child.pd).is_some_and(|row| row.surface == a.surface) =>
            {
                sex_pdx::serial_record!(
                    "[moan.role.active] pd={} task={} generation={}",
                    child.pd,
                    child.task,
                    r.generation
                );
                Activation::Active
            }
            Ok(a) if a.state == life::ACTIVATION_REBIRTHING => Activation::Pending,
            Err(life::RETRY) => Activation::Pending,
            _ => Activation::Failed,
        }
    }
    fn started(&mut self, parent: Subject, cookie: u32, child: Subject) -> bool {
        if !live(parent) || !crate::restricted_pool::reserved(child.pd) || parent.pd == child.pd {
            return false;
        }
        unsafe {
            if !crate::LAUNCH_RUNS.iter().flatten().any(|r| {
                r.parent == parent.pd
                    && r.parent_task == parent.task
                    && r.cookie == cookie
                    && r.child == child.pd
                    && r.child_task == child.task
            }) {
                let Some(slot) = crate::LAUNCH_RUNS.iter_mut().find(|r| r.is_none()) else {
                    return false;
                };
                *slot = Some(crate::LaunchRun {
                    parent: parent.pd,
                    parent_task: parent.task,
                    cookie,
                    child: child.pd,
                    child_task: child.task,
                    status: None,
                    ended: None,
                });
            }
            if !crate::HIDDEN_RUNS
                .iter()
                .any(|r| *r == Some((child.pd, child.task)))
            {
                let Some(slot) = crate::HIDDEN_RUNS.iter_mut().find(|r| r.is_none()) else {
                    return false;
                };
                *slot = Some((child.pd, child.task));
            }
        }
        let delivered = send_event(
            parent,
            sex_pdx::launch::Event::Started {
                cookie,
                row: child.pd,
                task: child.task,
            },
        );
        if delivered {
            sex_pdx::serial_record!(
                "[moan.role.started] parent={} parent_task={} cookie={} pd={} task={}",
                parent.pd,
                parent.task,
                cookie,
                child.pd,
                child.task
            );
        }
        delivered
    }
    fn refused(&mut self, parent: Subject, cookie: u32) {
        refuse(parent, cookie)
    }
    fn ready(&mut self, child: Subject, value: u64) -> bool {
        let delivered = crate::restricted_pool::reserved(child.pd)
            && live(child)
            && sex_pdx::pdx_reply(child.pd, value) == 0;
        if delivered {
            sex_pdx::serial_record!("[moan.role.ready] pd={} task={}", child.pd, child.task);
        }
        delivered
    }
    fn policy_acked(&mut self, service: Service, phase: u8, child: Subject, generation: u64) {
        sex_pdx::serial_record!(
            "[moan.role.policy] service={} phase={} pd={} task={} generation={} complete=1",
            service.pd(),
            phase,
            child.pd,
            child.task,
            generation
        );
    }
    fn kill(&mut self, child: Subject) {
        if !crate::restricted_pool::reserved(child.pd) || !catalog_subject(child) {
            return;
        }
        let Some(row) = catalog::by_target(child.pd) else {
            return;
        };
        let _ = life::force_end(row.lifecycle_slot);
    }
    fn retired(&mut self, child: Subject) -> bool {
        retired(child)
    }
}
pub fn retired(child: Subject) -> bool {
    let Some(row) = catalog::by_target(child.pd) else {
        return false;
    };
    catalog_subject(child)
        && matches!(row.activation_slot.and_then(|slot|life::activation(slot,false).ok()),Some(a)if a.state==life::ACTIVATION_REUSABLE&&a.task==child.task)
}
pub fn busy() -> bool {
    Turn::enter().core().busy()
}
pub fn pending() -> bool {
    Turn::enter().core().pending()
        || unsafe {
            (*core::ptr::addr_of!(OUTBOX)).iter().any(Option::is_some)
                || (*core::ptr::addr_of!(STATUSBOX))
                    .iter()
                    .any(Option::is_some)
        }
}
pub fn holds_done(child: Subject) -> bool {
    Turn::enter().core().holds_done(child)
}
pub fn poll() {
    Turn::enter().core().poll(&mut Kernel);
    notices();
    statuses();
}
pub fn retire(actor: Subject) {
    Turn::enter().core().retire(actor, &mut Kernel);
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(STATUSBOX) } {
        if slot.as_ref().is_some_and(|n| n.parent == actor) {
            *slot = None;
        }
    }
    for slot in unsafe { &mut *core::ptr::addr_of_mut!(OUTBOX) } {
        if slot.as_ref().is_some_and(|n| n.parent == actor) {
            *slot = None;
        }
    }
}
fn fixed_reply(value: u64) -> bool {
    let ticket = (value >> 32) & 0xff_ffff;
    if ticket == 0 {
        return false;
    }
    match value >> 56 {
        0xd1 | 0xd3 => value as u32 == 0,
        0xd2 => (1..=32 * 1024 * 1024).contains(&(value as u32)),
        _ => false,
    }
}
fn provider(actor: Subject) -> bool {
    [11, 1, 13, 10].contains(&actor.pd) && actor.task == actor.pd
}
pub fn receive(msg: &PdxMessage) -> bool {
    let actor = Subject {
        pd: msg.caller_pd,
        task: msg.caller_task,
    };
    let words = [msg.arg0, msg.arg1, msg.arg2];
    if msg.type_id == job::OP_STATUS {
        status(actor, words);
        return true;
    }
    if msg.type_id == 1 {
        let tagged = wire::is_chunk(msg.arg0);
        let fixed = actor == (Subject { pd: 11, task: 11 }) && fixed_reply(msg.arg0);
        if !tagged && !fixed {
            return false;
        }
        if provider(actor) && Turn::enter().core().reply(actor, msg.arg0, &mut Kernel) {
            return true;
        }
        // Reserved private shapes never reach a legacy decoder, including late
        // or foreign chunks. Normal Display/Bell/Files receipts remain untouched.
        return true;
    }
    if msg.type_id == wire::OP_READY {
        if crate::restricted_pool::reserved(actor.pd) && live(actor) {
            Turn::enter().core().ready(actor, words, &mut Kernel);
        }
        return true;
    }
    if !matches!(msg.type_id, job::OP_START | job::OP_CANCEL) {
        return false;
    }
    if crate::restricted_pool::reserved(actor.pd) || !live(actor) {
        return true;
    }
    let Some((ticket, cookie, files_epoch, shell_epoch)) = job::start_decode(words) else {
        return true;
    };
    if sex_pdx::crash_log::self_epoch().ok() != Some(shell_epoch) {
        return true;
    }
    if msg.type_id == job::OP_CANCEL {
        Turn::enter()
            .core()
            .cancel(actor, ticket, cookie, files_epoch, &mut Kernel);
        return true;
    }
    // A conflicting/repeated request cannot manufacture Refused for a job
    // whose private child still needs exact retirement. A retained pre-copy
    // refusal also prevents retry admission before that notice is delivered.
    if Turn::enter().core().copied_cookie(actor, cookie)
        || unsafe {
            (*core::ptr::addr_of!(OUTBOX))
                .iter()
                .flatten()
                .any(|n| n.parent == actor && n.event.cookie() == cookie)
        }
    {
        return true;
    }
    let admissible = crate::restricted_pool::normal()
        && unsafe {
            !crate::PROGRAM_READING
                && crate::PROGRAM_REBIRTH.is_none()
                && crate::RECORD_SPAWN_LEFT == 0
                && crate::LAUNCH_RUNS.iter().any(Option::is_none)
                && crate::HIDDEN_RUNS.iter().any(Option::is_none)
        };
    if !admissible
        || !Turn::enter()
            .core()
            .begin(actor, ticket, cookie, files_epoch, &mut Kernel)
    {
        refuse(actor, cookie);
    }
    true
}
