// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for SexOS fault recovery & automatic client reconnect:
//! 1. Stale handoff purge & custody guard against dead/exited/restarting tasks.
//! 2. IPC v2 endpoint detach & waiter wakeup with E_RESTARTED on server restart.
//! 3. Full service_restart notification (answers_every_call, MAX_TOLD >= 32) and hang_limit coverage.
//! 4. Automatic client reconnect (retry_on_peer_restart_with) on PEER_RESTARTED / ERR_SERVICE_NOT_READY.
#![allow(dead_code)]

extern crate self as spin;
extern crate self as sex_pdx;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

pub struct Mutex<T>(std::sync::Mutex<T>);
impl<T> Mutex<T> {
    pub const fn new(v: T) -> Self { Self(std::sync::Mutex::new(v)) }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> { self.0.lock().unwrap() }
    pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> { self.0.try_lock().ok() }
}

pub const SEXFS_V0_META_END_LBA: u64 = 47;
pub const SEXDRIVE_AP4_MULTI_BASE_LBA: u64 = 128;
pub const ERR_SERVICE_NOT_READY: u64 = 0xFFFF_FFFF_FFFF_FFFE;
// x86_64 hosts compile ipc_recv's live wrappers too; they reach these through `crate::`.
pub fn get_ticks() -> u64 { 0 }
pub fn sys_yield() {}
pub mod program_store { pub const DIRECTORY_LBA: u64 = 2048; }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdxMessage {
    pub type_id: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub caller_pd: u32,
    pub caller_task: u32,
}

#[path = "../crates/sex-pdx/src/crash_log.rs"]
mod wire_crash_log;
pub mod crash_log {
    pub use crate::wire_crash_log::*;
    pub fn poll() {}
    pub fn service(_: u32, _: u32, _: bool) {}
    pub fn fault(_: u32, _: u64, _: u64, _: u64, _: u64, _: &str) {}
}

#[path = "../crates/sex-pdx/src/endpoint.rs"]
pub mod endpoint;

#[path = "../crates/sex-pdx/src/ipc_recv.rs"]
pub mod ipc_recv;

#[path = "../kernel/src/capability_slot.rs"]
mod capability_slot;

#[path = "../kernel/src/task_ownership.rs"]
pub mod task_ownership;

#[path = "../kernel/src/ipc_v2"]
pub mod ipc_v2_objs {
    pub type TaskId = u64;
    pub mod endpoint;
    pub mod notification;
    pub mod reply;
}

#[macro_export]
macro_rules! serial_println {
    ($($t:tt)*) => {{ let _ = format_args!($($t)*); }};
}

static PURGE_PD_CALLS: AtomicUsize = AtomicUsize::new(0);
static PURGE_TASK_CALLS: AtomicUsize = AtomicUsize::new(0);
static V2_TEARDOWN_CALLS: AtomicUsize = AtomicUsize::new(0);
static V2_TEARDOWN_LAST_STATUS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

mod ipc_v2 {
    pub mod syscall {
        use std::sync::atomic::Ordering;
        pub fn teardown_pd(_pd: u32, status: u64) -> usize {
            crate::V2_TEARDOWN_CALLS.fetch_add(1, Ordering::AcqRel);
            crate::V2_TEARDOWN_LAST_STATUS.store(status, Ordering::Release);
            1
        }
    }
}

mod scheduler {
    pub use crate::task_ownership;
    use std::sync::atomic::{AtomicPtr, Ordering};
    pub struct Context {
        pub pd_id: u64,
        pub pd_ptr: *const crate::capability::ProtectionDomain,
        pub rip: u64,
        pub rsp: u64,
    }
    pub struct Task {
        pub id: u32,
        pub context: Context,
        pub state: task_ownership::TaskOwnership,
    }
    impl Task {
        pub fn rearm(&mut self, id: u32, entry: u64, stack: u64, pd: &crate::capability::ProtectionDomain) {
            self.id = id;
            self.context = Context { pd_id: pd.id as u64, pd_ptr: pd, rip: entry, rsp: stack };
            self.state = task_ownership::TaskOwnership::new();
            let _ = purge_handoff_for_task(self as *mut Task);
        }
    }
    pub struct Scheduler {
        pub core_id: u32,
        pub handoff: AtomicPtr<Task>,
    }
    pub static SCHEDULERS: [Scheduler; 4] = [
        Scheduler { core_id: 0, handoff: AtomicPtr::new(std::ptr::null_mut()) },
        Scheduler { core_id: 1, handoff: AtomicPtr::new(std::ptr::null_mut()) },
        Scheduler { core_id: 2, handoff: AtomicPtr::new(std::ptr::null_mut()) },
        Scheduler { core_id: 3, handoff: AtomicPtr::new(std::ptr::null_mut()) },
    ];
    impl Scheduler {
        pub fn activate_task(&self, t: *mut Task) -> bool { unsafe { (*t).state.activate() } }
    }
    pub fn unpark_thread(_: *mut Task) {}
    pub fn purge_handoff_for_task(task: *mut Task) -> usize {
        crate::PURGE_TASK_CALLS.fetch_add(1, Ordering::AcqRel);
        if task.is_null() { return 0; }
        let mut purged = 0;
        for sched in SCHEDULERS.iter() {
            if sched.handoff.compare_exchange(task, std::ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                purged += 1;
                let state = unsafe { (*task).state.snapshot() };
                if state.exited && state.phase == task_ownership::Phase::Queued {
                    let _ = unsafe { (*task).state.begin_dispatch(sched.core_id) };
                }
            }
        }
        purged
    }
    pub fn purge_handoff_for_pd(pd_id: u32) -> usize {
        crate::PURGE_PD_CALLS.fetch_add(1, Ordering::AcqRel);
        if pd_id == 0 { return 0; }
        let mut purged = 0;
        for sched in SCHEDULERS.iter() {
            let task = sched.handoff.load(Ordering::Acquire);
            if task.is_null() { continue; }
            if unsafe { (*task).context.pd_id } == pd_id as u64 {
                if sched.handoff.compare_exchange(task, std::ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    purged += 1;
                    let state = unsafe { (*task).state.snapshot() };
                    if state.exited && state.phase == task_ownership::Phase::Queued {
                        let _ = unsafe { (*task).state.begin_dispatch(sched.core_id) };
                    }
                }
            }
        }
        purged
    }
}

static REGISTRY: [AtomicPtr<capability::ProtectionDomain>; 1024] = [const { AtomicPtr::new(std::ptr::null_mut()) }; 1024];

mod capability {
    use crate::*;
    #[derive(Clone, Copy)]
    pub enum CapabilityData { Domain(u32), Memory }
    pub struct Capability { pub data: CapabilityData }
    pub struct ProtectionDomain {
        pub id: u32,
        pub pku_key: u8,
        pub main_task: AtomicPtr<scheduler::Task>,
        pub message_ring: *mut Queue,
        pub signal_ring: *mut Queue,
        pub incoming_replies: Mutex<VecDeque<ipc::messages::IpcReply>>,
        pub reply_credits: Mutex<Credits>,
        pub caps: Mutex<Vec<(u32, CapabilityData)>>,
    }
    pub struct Queue;
    impl Queue {
        pub fn dequeue(&self) -> Option<u8> { None }
        pub fn len(&self) -> usize { 0 }
    }
    #[derive(Default)]
    pub struct Credits(pub Vec<u32>);
    impl Credits {
        pub fn push(&mut self, c: u32) { self.0.push(c) }
        pub fn is_empty(&self) -> bool { self.0.is_empty() }
        pub fn owed(&self) -> impl Iterator<Item = u32> + '_ { self.0.iter().copied() }
        pub fn clear(&mut self) { self.0.clear() }
    }
    impl ProtectionDomain {
        pub fn new(id: u32, task: u32) -> &'static Self {
            let main = Box::into_raw(Box::new(scheduler::Task {
                id: task,
                context: scheduler::Context { pd_id: id as u64, pd_ptr: std::ptr::null(), rip: 0x4000, rsp: 0x9000 },
                state: task_ownership::TaskOwnership::new(),
            }));
            let pd = Box::leak(Box::new(Self {
                id,
                pku_key: 1,
                main_task: AtomicPtr::new(main),
                message_ring: Box::into_raw(Box::new(Queue)),
                signal_ring: Box::into_raw(Box::new(Queue)),
                incoming_replies: Mutex::new(VecDeque::with_capacity(8)),
                reply_credits: Mutex::new(Credits::default()),
                caps: Mutex::new(Vec::new()),
            }));
            unsafe { (*main).context.pd_ptr = pd; }
            REGISTRY[id as usize].store(pd, Ordering::Release);
            pd
        }
        pub fn find_capability(&self, slot: u32) -> Option<Capability> {
            self.caps.lock().iter().find(|c| c.0 == slot).map(|c| Capability { data: c.1 })
        }
    }
}

mod ipc {
    pub const MAX_DOMAINS: usize = 1024;
    pub struct Registry;
    pub static DOMAIN_REGISTRY: Registry = Registry;
    impl Registry {
        pub fn get(&self, id: u32) -> Option<&'static crate::capability::ProtectionDomain> {
            let p = crate::REGISTRY.get(id as usize)?.load(crate::Ordering::Acquire);
            unsafe { p.as_ref() }
        }
    }
    pub mod messages {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct IpcReply { pub value: u64, pub caller_pd: u32, pub caller_task: u32 }
        impl IpcReply {
            pub fn erase_queue(q: &mut std::collections::VecDeque<Self>) { q.clear() }
        }
    }
}

mod boot_memory {
    pub fn existing_leaf_address(_: u64) -> Option<u64> { None }
    pub fn arena_leaf(_: u64, _: u8) -> Option<u64> { None }
    pub fn backing(_: u64) -> u64 { 0 }
    pub fn copy_frame(_: u64, _: u64) {}
    pub fn write_frames(_: u64, _: &[u8]) {}
}
mod program_frames { pub fn take() -> Option<u64> { None } pub fn give(_: u64) {} }
#[path = "../crates/sex-pdx/src/program_image.rs"]
mod program_image;
#[path = "../crates/sex-pdx/src/program_code.rs"]
pub mod program_code;
pub struct HhdmResp { pub offset: u64 }
pub struct HhdmReq;
impl HhdmReq { pub fn response(&self) -> Option<HhdmResp> { Some(HhdmResp { offset: 0 }) } }
pub static HHDM_REQUEST: HhdmReq = HhdmReq;
mod smp { pub mod tlb { pub fn flush_pages(_: usize, _: impl Iterator<Item = u64>) {} } }
mod pci_command { pub fn quiesce(_: &crate::capability::ProtectionDomain) {} }
mod iommu { pub fn revoke(_: u32) {} }
mod device_irq {
    pub fn snapshot(_: u32) {}
    pub fn reset_begin(_: u32) -> bool { false }
    pub fn reset_end(_: u32) {}
}
mod program_workers {
    pub fn end(_: u32) {}
    pub fn all_dead(_: u32) -> bool { true }
}
mod console { pub fn stop(_: std::fmt::Arguments) -> ! { panic!("unexpected desktop stop") } }
mod interrupts { pub static TICKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(100); }
extern crate alloc;

#[path = "../kernel/src/service_restart.rs"]
mod service_restart;

#[test]
fn stale_handoff_purge_retires_queued_exited_task_to_dead() {
    use task_ownership::{Admission, Phase};
    let pd = capability::ProtectionDomain::new(50, 500);
    let task = pd.main_task.load(Ordering::Acquire);
    unsafe {
        assert!((*task).state.activate());
        assert_eq!((*task).state.try_admit::<()>(|| {
            scheduler::SCHEDULERS[2].handoff.store(task, Ordering::Release);
            Ok(())
        }), Admission::Published);
        assert_eq!((*task).state.snapshot().phase, Phase::Queued);
        // Task exits while sitting in core 2's handoff slot.
        assert!((*task).state.exit());
        assert_eq!((*task).state.snapshot().phase, Phase::Queued);
    }
    assert_eq!(scheduler::purge_handoff_for_task(task), 1);
    assert!(scheduler::SCHEDULERS[2].handoff.load(Ordering::Acquire).is_null());
    // Custody MUST advance to Dead so service_restart never wedges!
    let snap = unsafe { (*task).state.snapshot() };
    assert!(snap.exited);
    assert_eq!(snap.phase, Phase::Dead);
    assert_eq!(snap.owner_core, None);
}

#[test]
fn purge_handoff_for_pd_clears_only_target_domain() {
    let pd_a = capability::ProtectionDomain::new(51, 501);
    let pd_b = capability::ProtectionDomain::new(52, 502);
    let task_a = pd_a.main_task.load(Ordering::Acquire);
    let task_b = pd_b.main_task.load(Ordering::Acquire);
    scheduler::SCHEDULERS[0].handoff.store(task_a, Ordering::Release);
    scheduler::SCHEDULERS[1].handoff.store(task_b, Ordering::Release);
    scheduler::SCHEDULERS[3].handoff.store(task_a, Ordering::Release);

    assert_eq!(scheduler::purge_handoff_for_pd(51), 2);
    assert!(scheduler::SCHEDULERS[0].handoff.load(Ordering::Acquire).is_null());
    assert!(scheduler::SCHEDULERS[3].handoff.load(Ordering::Acquire).is_null());
    assert_eq!(scheduler::SCHEDULERS[1].handoff.load(Ordering::Acquire), task_b);
    scheduler::SCHEDULERS[1].handoff.store(std::ptr::null_mut(), Ordering::Release);
}

#[test]
fn ipc_v2_endpoint_teardown_detaches_server_and_wakes_clients_with_e_restarted() {
    use ipc_v2_objs::endpoint::*;
    use ipc_v2_objs::reply::*;

    struct TestGen { live_gen: u32 }
    impl Incarnations for TestGen {
        fn is_current(&self, _task: ipc_v2_objs::TaskId, gen: u32) -> bool { gen == self.live_gen }
    }

    // 1. Server parked on a foreign endpoint + queued on another: detach_task removes both.
    let foreign_ep = Endpoint::new();
    assert_eq!(foreign_ep.recv(42), RecvOutcome::Parked);
    let (was_recv, removed_send) = foreign_ep.detach_task(42);
    assert!(was_recv);
    assert_eq!(removed_send, None);
    assert_eq!(foreign_ep.waiter(), None);

    // 2. Multiple PDs queued on an endpoint; cancel_send_if removes crashed PD (tasks 100..102)
    //    while preserving other PDs in exact FIFO order.
    let other_ep = Endpoint::new();
    for t in [10u64, 100, 20, 101, 30] {
        assert_eq!(other_ep.send(Sender { task: t, badge: t, generation: 1, kind: CallKind::Call }), SendOutcome::Queued);
    }
    assert_eq!(other_ep.cancel_send_if(|s| s.task >= 100), 2);
    let remaining: Vec<u64> = (0..3)
        .map(|_| match other_ep.recv(1) { RecvOutcome::Message(s) => s.task, o => panic!("{o:?}") })
        .collect();
    assert_eq!(remaining, vec![10, 20, 30]);

    // 3. Server's own endpoint has queued clients: close wakes every client with E_RESTARTED.
    let server_ep = Endpoint::new();
    for client in 200u64..206 {
        assert_eq!(
            server_ep.send(Sender { task: client, badge: client, generation: 1, kind: CallKind::Call }),
            SendOutcome::Queued
        );
    }
    let mut woken = Vec::new();
    let released = server_ep.close(|r| {
        if let Released::Caller(s) = r {
            woken.push((s.task, endpoint::E_RESTARTED));
        }
    });
    assert_eq!(released, 6);
    assert_eq!(woken.len(), 6);
    assert!(woken.iter().all(|(_, status)| *status == endpoint::E_RESTARTED));
    assert_eq!(server_ep.queued(), 0);
    assert_eq!(server_ep.waiter(), None);

    // 4. Unanswered reply capability held by crashed server wakes its client before incarnation ends.
    let reply_slot = ReplySlot::new();
    let cap = ReplyCap::for_sender(&Sender { task: 300, badge: 1, generation: 5, kind: CallKind::Call }).unwrap();
    assert!(reply_slot.install(cap).is_ok());
    let taken = reply_slot.take(&TestGen { live_gen: 5 }).expect("unanswered reply cap must be taken");
    assert_eq!(taken.caller, 300);
}

#[test]
fn service_restart_notifies_24_concurrent_callers_and_keeps_watchdog_scope() {
    // Only services that answer every call they take are owed-reply services; Files,
    // Display and Shell defer replies and are not watched for holding a message.
    for pd in [2u32, 5] {
        assert!(service_restart::answers_every_call(pd), "pd={pd} must answer every call");
    }
    for pd in [1u32, 3, 11] {
        assert!(!service_restart::answers_every_call(pd), "pd={pd} defers replies");
        assert_eq!(service_restart::hang_limit(pd), 0, "pd={pd} is not watched");
    }
    assert_eq!(service_restart::hang_limit(2), 45_000);
    assert_eq!(service_restart::hang_limit(13), 10_000);
    // Desk services are watched for being stuck, not for holding a message.
    for pd in [1u32, 3, 11] { assert_eq!(service_restart::desk_limit(pd), 60_000, "pd={pd} desk watch"); }
    for pd in [2u32, 4, 5, 10, 13, 54] { assert_eq!(service_restart::desk_limit(pd), 0, "pd={pd} is not a desk service"); }

    // Register USB (PD 5) and 24 concurrent callers (PDs 100..124).
    let files = capability::ProtectionDomain::new(5, 50);
    service_restart::capture(5, 5);
    let mut callers = Vec::new();
    for c in 100u32..124 {
        callers.push(capability::ProtectionDomain::new(c, 1000 + c));
        files.reply_credits.lock().push(c);
    }

    // Simulate the service crashing (exited + Dead).
    let main = files.main_task.load(Ordering::Acquire);
    unsafe {
        assert!((*main).state.exit());
        assert_eq!((*main).state.snapshot().phase, task_ownership::Phase::Dead);
    }

    let before_purge = PURGE_PD_CALLS.load(Ordering::Acquire);
    let before_v2 = V2_TEARDOWN_CALLS.load(Ordering::Acquire);

    // Poll 1: enters reset/quiesce, purges handoffs, tears down IPC v2 with E_RESTARTED.
    assert!(service_restart::poll());
    assert!(PURGE_PD_CALLS.load(Ordering::Acquire) > before_purge);
    assert!(V2_TEARDOWN_CALLS.load(Ordering::Acquire) > before_v2);
    assert_eq!(V2_TEARDOWN_LAST_STATUS.load(Ordering::Acquire), endpoint::E_RESTARTED);

    // Advance tick past reset settle window and poll 2: completes restart & notifies all 24 callers.
    interrupts::TICKS.fetch_add(10, Ordering::AcqRel);
    let _ = service_restart::poll();

    assert_eq!(service_restart::restarts_of(5), 1);
    assert!(!service_restart::down(5));
    for (idx, caller) in callers.iter().enumerate() {
        let mut q = caller.incoming_replies.lock();
        let reply = q.pop_front().unwrap_or_else(|| panic!("caller {idx} was stranded without PEER_RESTARTED"));
        assert_eq!(reply.caller_pd, 5);
        assert_eq!(reply.value, crash_log::PEER_RESTARTED);
    }
}

#[test]
fn client_auto_reconnect_waits_for_epoch_advance_and_retries() {
    use crash_log::{PeerLife, PeerState, PEER_RESTARTED};
    use ipc_recv::{is_peer_restarted, retry_on_peer_restart_with, AttemptOutcome, ReconnectError};

    let msg_ok = PdxMessage { type_id: 1, caller_pd: 11, arg0: PEER_RESTARTED, ..Default::default() };
    let msg_wrong_pd = PdxMessage { type_id: 1, caller_pd: 13, arg0: PEER_RESTARTED, ..Default::default() };
    let msg_normal = PdxMessage { type_id: 1, caller_pd: 11, arg0: 42, ..Default::default() };
    assert!(is_peer_restarted(&msg_ok, 11));
    assert!(!is_peer_restarted(&msg_wrong_pd, 11));
    assert!(!is_peer_restarted(&msg_normal, 11));

    // Scenario 1: Server dies mid-call, transitions Ending -> Dead -> Starting -> Live(epoch=2),
    // client automatically retries and succeeds.
    let states = [
        PeerState { epoch: 1, life: PeerLife::Live },     // initial epoch read
        PeerState { epoch: 1, life: PeerLife::Ending },   // wait loop #1
        PeerState { epoch: 1, life: PeerLife::Dead },     // wait loop #2
        PeerState { epoch: 2, life: PeerLife::Starting }, // wait loop #3
        PeerState { epoch: 2, life: PeerLife::Live },     // wait loop #4 -> retry!
    ];
    let mut s_idx = 0usize;
    let mut ticks = 0u64;
    let mut attempts = 0u32;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        4,
        1_000,
        || { let s = states[s_idx.min(states.len() - 1)]; s_idx += 1; Ok(s) },
        || { ticks += 1; ticks },
        || {},
        || {
            attempts += 1;
            if attempts == 1 { AttemptOutcome::PeerRestarted } else { AttemptOutcome::Done(0xCAFEu64) }
        },
    );
    assert_eq!(res, Ok(0xCAFEu64));
    assert_eq!(attempts, 2);

    // Scenario 2: Server hits crash-loop limit (GaveUp) -> stops immediately.
    let mut attempts = 0u32;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        4,
        1_000,
        || Ok(PeerState { epoch: 9, life: PeerLife::GaveUp }),
        || 10,
        || {},
        || { attempts += 1; AttemptOutcome::PeerRestarted },
    );
    assert_eq!(res, Err(ReconnectError::GaveUp));
    assert_eq!(attempts, 1);

    // Scenario 3: Non-retryable error returns immediately without retrying.
    let mut attempts = 0u32;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        4,
        1_000,
        || Ok(PeerState { epoch: 1, life: PeerLife::Live }),
        || 10,
        || {},
        || { attempts += 1; AttemptOutcome::Failed(7u8) },
    );
    assert_eq!(res, Err(ReconnectError::Failed(7u8)));
    assert_eq!(attempts, 1);

    // Scenario 4: Exhausting max_retries returns Exhausted.
    let mut attempts = 0u32;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        2,
        1_000,
        || Ok(PeerState { epoch: 1, life: PeerLife::Live }),
        || 10,
        || {},
        || { attempts += 1; AttemptOutcome::PeerRestarted },
    );
    assert_eq!(res, Err(ReconnectError::Exhausted));
    assert_eq!(attempts, 3); // 1 initial + 2 retries
}

#[test]
fn busy_not_ready_never_burns_restart_retry_budget() {
    use crash_log::{PeerLife, PeerState};
    use ipc_recv::{retry_on_peer_restart_with, AttemptOutcome, ReconnectError};

    // 1. 25 transient ERR_SERVICE_NOT_READY (busy) responses while peer is Live at
    //    unchanged epoch=1 must succeed even with max_retries=4.
    let mut attempts = 0u32;
    let mut yields = 0u32;
    let mut ticks = 0u64;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        4,
        1_000,
        || Ok(PeerState { epoch: 1, life: PeerLife::Live }),
        || { ticks += 1; ticks },
        || { yields += 1; },
        || {
            attempts += 1;
            if attempts <= 25 { AttemptOutcome::NotReady } else { AttemptOutcome::Done(0xBEEFu64) }
        },
    );
    assert_eq!(res, Ok(0xBEEFu64));
    assert_eq!(attempts, 26);
    assert_eq!(yields, 25);

    // 2. Mix of 15 busy NotReady + 1 genuine restart (Ending -> Starting -> Live epoch=2)
    //    + 15 busy NotReady at epoch=2 + 1 genuine PEER_RESTARTED (epoch=3) succeeds
    //    with max_retries=2 (only the 2 real restarts count).
    let mut attempts = 0u32;
    let phase = std::cell::Cell::new(0u32);
    let mut ticks = 0u64;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        2,
        1_000,
        || match phase.get() {
            0 => Ok(PeerState { epoch: 1, life: PeerLife::Live }),
            1 => { phase.set(2); Ok(PeerState { epoch: 1, life: PeerLife::Ending }) }
            2 => { phase.set(3); Ok(PeerState { epoch: 2, life: PeerLife::Starting }) }
            3 => Ok(PeerState { epoch: 2, life: PeerLife::Live }),
            4 => { phase.set(5); Ok(PeerState { epoch: 2, life: PeerLife::Dead }) }
            _ => Ok(PeerState { epoch: 3, life: PeerLife::Live }),
        },
        || { ticks += 1; ticks },
        || {},
        || {
            attempts += 1;
            match attempts {
                1..=15 => AttemptOutcome::NotReady,
                16 => { phase.set(1); AttemptOutcome::NotReady }
                17..=31 => AttemptOutcome::NotReady,
                32 => { phase.set(4); AttemptOutcome::PeerRestarted }
                _ => AttemptOutcome::Done(0x1234u64),
            }
        },
    );
    assert_eq!(res, Ok(0x1234u64));
    assert_eq!(attempts, 33);

    // 3. NotReady when peer has hit GaveUp fails immediately without retrying.
    let mut attempts = 0u32;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        8,
        1_000,
        || Ok(PeerState { epoch: 5, life: PeerLife::GaveUp }),
        || 10,
        || {},
        || { attempts += 1; AttemptOutcome::NotReady },
    );
    assert_eq!(res, Err(ReconnectError::GaveUp));
    assert_eq!(attempts, 1);

    // 4. Sustained NotReady while Live eventually honors timeout_ticks.
    let mut ticks = 0u64;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        8,
        50,
        || Ok(PeerState { epoch: 1, life: PeerLife::Live }),
        || { ticks += 10; ticks },
        || {},
        || AttemptOutcome::NotReady,
    );
    assert_eq!(res, Err(ReconnectError::Exhausted));

    // 5. NotReady where epoch advances between calls counts as a restart and bounds at max_retries.
    let epoch = std::cell::Cell::new(1u64);
    let mut attempts = 0u32;
    let res: Result<u64, ReconnectError<u8>> = retry_on_peer_restart_with(
        2,
        1_000,
        || Ok(PeerState { epoch: epoch.get(), life: PeerLife::Live }),
        || 10,
        || {},
        || { attempts += 1; epoch.set(epoch.get() + 1); AttemptOutcome::NotReady },
    );
    assert_eq!(res, Err(ReconnectError::Exhausted));
    assert_eq!(attempts, 3); // 1 initial + 2 epoch-advance restarts
}

#[test]
fn desk_watch_ends_only_a_parked_service_whose_code_stopped_running() {
    use service_restart::{desk_verdict, Desk};
    const L: u64 = 60_000;
    // Not holding long enough, or not watched: fine whatever else is true.
    assert_eq!(desk_verdict(L, 70_000, 20_000, 0, true), Desk::Fine);
    assert_eq!(desk_verdict(0, 900_000, 1, 0, true), Desk::Fine);
    assert_eq!(desk_verdict(L, 900_000, 0, 0, true), Desk::Fine, "never took a message");
    // Held past the limit but computing (on a core or queued): reported, never ended.
    assert_eq!(desk_verdict(L, 200_000, 100_000, 100_000, false), Desk::Slow);
    // Parked, but its code made a syscall within the limit (a sleep, a self-wait that
    // timed out and went round again): reported, never ended.
    assert_eq!(desk_verdict(L, 200_000, 100_000, 150_000, true), Desk::Slow);
    // Parked with no syscall for the whole limit while holding: stuck.
    assert_eq!(desk_verdict(L, 200_000, 100_000, 139_999, true), Desk::Hung);
    assert_eq!(desk_verdict(L, 160_001, 100_000, 100_000, true), Desk::Hung);
    assert_eq!(desk_verdict(L, 160_000, 100_000, 100_000, true), Desk::Fine, "exactly the limit is not past it");
}
