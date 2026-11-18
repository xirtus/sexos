//! IPC v2 system calls (kernel plan phase 2): 105 SYS_ENDPOINT and 106 SYS_NOTIFY,
//! over endpoint and notification capabilities (authority v2 object kinds).
//!
//! Registers in: rdi = op | slot << 8, rsi = message info, rdx r10 r8 = payload.
//! Registers out: rax status, rdi badge, rsi info (or bits, or a new slot), rdx r10 r8.
//! A task that waits gets its result written into its saved frame by `deliver` when
//! it next runs, in the slots `ipc::recv::deliver` uses. A call that finds its server
//! parked wakes it as a #402 handoff, so the server runs next on this core.
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering};
use crate::capability::CapabilityData;
use crate::capability_table::{rights, Generations, ObjectCap, MAX_OBJECTS, OBJECTS};
use crate::interrupts::SyscallRegs;
use crate::scheduler::{Task, SCHEDULERS};
use sex_pdx::endpoint::*;
use super::endpoint::{CallKind, Endpoint, RecvOutcome, Released, SendOutcome, Sender};
use super::notification::{Notification, SignalOutcome, WaitOutcome};
use super::reply::{Incarnations, ReplyCap, ReplySlot};
use super::TaskId;

const POOL: usize = 128;
static ENDPOINTS: [Endpoint; POOL] = [const { Endpoint::new() }; POOL];
static NOTIFICATIONS: [Notification; POOL] = [const { Notification::new() }; POOL];
/// Object id + 1 using each pool entry (0: free), per kind.
static ENDPOINT_OBJECT: [AtomicU32; POOL] = [const { AtomicU32::new(0) }; POOL];
static NOTIFICATION_OBJECT: [AtomicU32; POOL] = [const { AtomicU32::new(0) }; POOL];
/// Pool entry + 1 for each object id (0: none).
static POOL_OF: [AtomicU16; MAX_OBJECTS] = [const { AtomicU16::new(0) }; MAX_OBJECTS];

/// Counters for `OP_STATS`: rendezvous, queued, waits that slept, E_AGAIN returns.
static COUNTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
fn count(i: usize) { COUNTS[i].fetch_add(1, Ordering::Relaxed); }

/// Per-task IPC v2 state, in `Task`.
pub struct TaskIpc {
    /// The result its wait returns: status, badge, info, three payload words.
    result: [AtomicU64; 6],
    ready: AtomicBool,
    /// It slept in a wait: `deliver` writes `result` into its frame.
    parked: AtomicBool,
    /// Its own message while it is queued on an endpoint: info, payload.
    outbox: [AtomicU64; 4],
    reply: ReplySlot,
}

impl TaskIpc {
    pub const fn new() -> Self {
        Self { result: [const { AtomicU64::new(0) }; 6], ready: AtomicBool::new(false), parked: AtomicBool::new(false),
               outbox: [const { AtomicU64::new(0) }; 4], reply: ReplySlot::new() }
    }
    /// It sleeps in an IPC v2 wait: an idle core spins briefly for its answer.
    pub fn waiting(&self) -> bool { self.parked.load(Ordering::Relaxed) && !self.ready.load(Ordering::Relaxed) }
}

impl Incarnations for Generations {
    fn is_current(&self, task: TaskId, generation: u32) -> bool { self.caller_live(pd_of(task), generation) }
}

fn ipc(task: TaskId) -> &'static TaskIpc { unsafe { &(*(task as *mut Task)).ipc_v2 } }
fn pd_of(task: TaskId) -> u32 { unsafe { (*(task as *mut Task)).context.pd_id as u32 } }
fn current() -> TaskId {
    SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize].current_task.load(Ordering::Acquire) as TaskId
}

/// Hands `task` its result and wakes it; `handoff` makes it run next on this core.
fn complete(task: TaskId, words: [u64; 6], handoff: bool) {
    let state = ipc(task);
    for (slot, word) in state.result.iter().zip(words) { slot.store(word, Ordering::Relaxed); }
    state.ready.store(true, Ordering::Release);
    let sched = &SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize];
    if handoff { sched.arm_handoff(true); }
    crate::scheduler::unpark_thread(task as *mut Task);
    if handoff { sched.arm_handoff(false); }
}

fn finish(regs: &mut SyscallRegs, state: &TaskIpc) -> u64 {
    let r = state.result.each_ref().map(|w| w.load(Ordering::Relaxed));
    regs.rdi = r[1]; regs.rsi = r[2]; regs.rdx = r[3]; regs.r10 = r[4]; regs.r8 = r[5];
    r[0]
}

/// Waits for this task's result. A wake with no result returns E_AGAIN; the caller
/// then asks again with OP_WAIT (or RECV), never by resending.
unsafe fn wait(regs: &mut SyscallRegs, me: TaskId) -> u64 {
    let state = ipc(me);
    regs.rax = E_AGAIN;
    state.parked.store(true, Ordering::Release);
    count(2);
    crate::scheduler::park_current_unless(regs as *const _, || state.ready.load(Ordering::Acquire));
    state.parked.store(false, Ordering::Relaxed);
    if state.ready.swap(false, Ordering::AcqRel) { finish(regs, state) } else { count(3); E_AGAIN }
}

/// Called as `task` is about to run (from `ipc::recv::deliver`): a task that slept in
/// `wait` finds its result in its saved frame (rax rbx rcx rdx rbp rsi rdi r8 r9 r10).
pub unsafe fn deliver(task: *mut Task) {
    let state = &(*task).ipc_v2;
    if !state.parked.swap(false, Ordering::AcqRel) || !state.ready.swap(false, Ordering::AcqRel) { return; }
    let r = state.result.each_ref().map(|w| w.load(Ordering::Relaxed));
    let frame = (*task).context.kstack_top as *mut u64;
    *frame = r[0]; *frame.add(6) = r[1]; *frame.add(5) = r[2]; *frame.add(3) = r[3]; *frame.add(9) = r[4]; *frame.add(7) = r[5];
}

fn table() -> &'static crate::capability_table::CapabilityTable {
    unsafe { &*crate::core_local::CoreLocal::get().current_pd_ref().cap_table }
}

/// The live object capability of `kind` in `slot` with `needed` rights, and its pool entry.
fn object(slot: u32, needed: u32, endpoint: bool) -> Result<(ObjectCap, usize), u64> {
    let cap = match table().find(slot).map(|c| c.data) {
        Some(CapabilityData::Endpoint(cap)) if endpoint => cap,
        Some(CapabilityData::Notification(cap)) if !endpoint => cap,
        _ => return Err(sex_pdx::ERR_CAP_INVALID),
    };
    if !OBJECTS.live(&cap) { return Err(E_CLOSED); }
    if !cap.allows(needed) { return Err(E_RIGHTS); }
    match POOL_OF[cap.object as usize % MAX_OBJECTS].load(Ordering::Acquire) {
        0 => Err(E_CLOSED),
        entry => Ok((cap, entry as usize - 1)),
    }
}

fn release(released: Released) {
    let task = match released { Released::Receiver(t) => t, Released::Caller(s) => s.task };
    complete(task, [E_CLOSED, 0, 0, 0, 0, 0], false);
}

/// Frees pool entries whose object died (revoked root destroyed, or its owner's
/// incarnation ended), waking everyone waiting on them with E_CLOSED.
pub fn release_dead() {
    for entry in 0..POOL {
        for (objects, endpoint) in [(&ENDPOINT_OBJECT, true), (&NOTIFICATION_OBJECT, false)] {
            let object = objects[entry].load(Ordering::Acquire);
            if object == 0 || OBJECTS.owner(object - 1).is_some() { continue; }
            free(entry, object - 1, endpoint);
        }
    }
}

fn free(entry: usize, object: u32, endpoint: bool) {
    POOL_OF[object as usize % MAX_OBJECTS].store(0, Ordering::Release);
    if endpoint {
        ENDPOINTS[entry].close(release);
        ENDPOINT_OBJECT[entry].store(0, Ordering::Release);
    } else {
        if let Some(t) = NOTIFICATIONS[entry].waiter() { if NOTIFICATIONS[entry].cancel_wait(t) { release(Released::Receiver(t)); } }
        NOTIFICATIONS[entry].poll();
        NOTIFICATION_OBJECT[entry].store(0, Ordering::Release);
    }
}

/// OP_CREATE: a new object owned by the caller's domain; its root capability goes in
/// the caller's table (slot in rsi).
fn create(regs: &mut SyscallRegs, endpoint: bool) -> u64 {
    let pd = pd_of(current());
    let Some(cap) = OBJECTS.create(pd) else { return E_FULL };
    let objects = if endpoint { &ENDPOINT_OBJECT } else { &NOTIFICATION_OBJECT };
    let claim = || objects.iter().position(|o| o.compare_exchange(0, cap.object + 1, Ordering::AcqRel, Ordering::Relaxed).is_ok());
    let Some(entry) = claim().or_else(|| { release_dead(); claim() }) else { OBJECTS.destroy(cap.object); return E_FULL };
    if endpoint { ENDPOINTS[entry].reopen(); }
    POOL_OF[cap.object as usize % MAX_OBJECTS].store(entry as u16 + 1, Ordering::Release);
    let data = if endpoint { CapabilityData::Endpoint(cap) } else { CapabilityData::Notification(cap) };
    match table().install(data) {
        Ok(slot) => { regs.rsi = slot as u64; 0 }
        Err(_) => { free(entry, cap.object, endpoint); OBJECTS.destroy(cap.object); E_FULL }
    }
}

/// OP_GRANT / OP_DERIVE: derive from `slot` (rights rdx, badge r10) into the domain
/// named by the Domain capability in rsi, or into the caller's own table; the new
/// slot comes back in rsi.
fn grant(regs: &mut SyscallRegs, slot: u32, own: bool) -> u64 {
    let target = if own { table() } else {
        let Some(CapabilityData::Domain(target)) = table().find(regs.rsi as u32).map(|c| c.data) else { return sex_pdx::ERR_CAP_INVALID };
        let Some(target) = crate::ipc::DOMAIN_REGISTRY.get(target) else { return sex_pdx::ERR_CAP_INVALID };
        unsafe { &*target.cap_table }
    };
    let data = match table().derive(slot, regs.rdx as u32, regs.r10, &OBJECTS) { Ok(d) => d, Err(_) => return E_RIGHTS };
    match target.install(data) { Ok(s) => { regs.rsi = s as u64; 0 } Err(_) => E_FULL }
}

/// OP_DESTROY: the owner ends the object; every capability to it dies.
fn destroy(slot: u32, endpoint: bool) -> u64 {
    let (cap, entry) = match object(slot, rights::ALL, endpoint) { Ok(o) => o, Err(e) => return e };
    if OBJECTS.owner(cap.object) != Some(pd_of(current())) { return E_RIGHTS; }
    OBJECTS.destroy(cap.object);
    free(entry, cap.object, endpoint);
    table().revoke(slot);
    0
}

fn message(regs: &SyscallRegs) -> Result<[u64; 4], u64> {
    match MessageInfo::from_raw(regs.rsi) {
        Ok(info) if info.registers() <= REGISTER_PAYLOAD && info.buffer_len() == 0 => Ok([regs.rsi, regs.rdx, regs.r10, regs.r8]),
        _ => Err(E_BADMSG),
    }
}

/// A server is handed a caller's message: a reply capability for a call, and the
/// words as its result.
fn hand_over(server: TaskId, sender: &Sender, words: [u64; 4], handoff: bool) {
    if let Some(cap) = ReplyCap::for_sender(sender) { let _ = ipc(server).reply.install(cap); }
    complete(server, [0, sender.badge, words[0], words[1], words[2], words[3]], handoff);
}

unsafe fn send(regs: &mut SyscallRegs, slot: u32, kind: CallKind) -> u64 {
    let (cap, entry) = match object(slot, rights::CALL, true) { Ok(o) => o, Err(e) => return e };
    let words = match message(regs) { Ok(w) => w, Err(e) => return e };
    let me = current();
    let sender = Sender { task: me, badge: cap.badge, generation: OBJECTS.incarnation(pd_of(me)), kind };
    for (slot, word) in ipc(me).outbox.iter().zip(words) { slot.store(word, Ordering::Relaxed); }
    match ENDPOINTS[entry].send(sender) {
        SendOutcome::Rendezvous { receiver } if kind == CallKind::Call => { count(0); hand_over(receiver, &sender, words, true); wait(regs, me) }
        SendOutcome::Rendezvous { receiver } => { hand_over(receiver, &sender, words, false); 0 }
        SendOutcome::Queued => { count(1); wait(regs, me) }
        SendOutcome::Full => E_FULL,
        SendOutcome::Closed => E_CLOSED,
    }
}

/// Answers the call this task holds, if any (OP_REPLY, and the first half of REPLY_RECV).
fn reply(regs: &SyscallRegs, me: TaskId) -> u64 {
    let Ok(words) = message(regs) else { return E_BADMSG };
    match ipc(me).reply.take(&OBJECTS) {
        Ok(cap) => { complete(cap.caller, [0, 0, words[0], words[1], words[2], words[3]], true); 0 }
        Err(_) => E_GONE,
    }
}

unsafe fn recv(regs: &mut SyscallRegs, slot: u32, me: TaskId) -> u64 {
    let (_, entry) = match object(slot, rights::RECEIVE, true) { Ok(o) => o, Err(e) => return e };
    let state = ipc(me);
    if state.ready.swap(false, Ordering::AcqRel) { return finish(regs, state); }
    // A held call nobody answered: its caller hears E_GONE rather than waiting forever.
    if let Ok(cap) = state.reply.take(&OBJECTS) { complete(cap.caller, [E_GONE, 0, 0, 0, 0, 0], false); }
    loop {
        match ENDPOINTS[entry].recv(me) {
            RecvOutcome::Message(s) if !OBJECTS.caller_live(pd_of(s.task), s.generation) => continue,
            RecvOutcome::Message(s) => {
                let w = ipc(s.task).outbox.each_ref().map(|w| w.load(Ordering::Relaxed));
                if let Some(cap) = ReplyCap::for_sender(&s) { let _ = state.reply.install(cap); }
                else { complete(s.task, [0; 6], false); }
                regs.rdi = s.badge; regs.rsi = w[0]; regs.rdx = w[1]; regs.r10 = w[2]; regs.r8 = w[3];
                return 0;
            }
            RecvOutcome::Parked => return wait(regs, me),
            RecvOutcome::Busy => return E_BUSY,
            RecvOutcome::Closed => return E_CLOSED,
        }
    }
}

/// Syscall 105, SYS_ENDPOINT.
pub unsafe fn endpoint_syscall(regs: &mut SyscallRegs) -> u64 {
    let (op, slot) = (regs.rdi & 0xFF, (regs.rdi >> 8) as u32);
    let me = current();
    match op {
        OP_CREATE => create(regs, true),
        OP_SEND => send(regs, slot, CallKind::Send),
        OP_CALL => send(regs, slot, CallKind::Call),
        OP_RECV => recv(regs, slot, me),
        OP_REPLY => reply(regs, me),
        OP_REPLY_RECV => { let _ = reply(regs, me); recv(regs, slot, me) }
        OP_WAIT => { let s = ipc(me); if s.ready.swap(false, Ordering::AcqRel) { finish(regs, s) } else { wait(regs, me) } }
        OP_GRANT => grant(regs, slot, false),
        OP_DERIVE => grant(regs, slot, true),
        OP_DESTROY => destroy(slot, true),
        OP_STATS => {
            let c = COUNTS.each_ref().map(|c| c.load(Ordering::Relaxed));
            regs.rdi = crate::scheduler::HANDOFFS.load(Ordering::Relaxed);
            regs.rsi = c[0]; regs.rdx = c[1]; regs.r10 = c[2]; regs.r8 = c[3];
            0
        }
        _ => E_BADMSG,
    }
}

/// Syscall 106, SYS_NOTIFY.
pub unsafe fn notify_syscall(regs: &mut SyscallRegs) -> u64 {
    let (op, slot) = (regs.rdi & 0xFF, (regs.rdi >> 8) as u32);
    let me = current();
    let needed = match op { OP_SIGNAL => rights::WRITE, OP_NOTIFY_WAIT | OP_POLL => rights::READ, _ => 0 };
    let entry = match op {
        OP_SIGNAL | OP_NOTIFY_WAIT | OP_POLL => match object(slot, needed, false) { Ok((_, e)) => e, Err(e) => return e },
        _ => 0,
    };
    let n = &NOTIFICATIONS[entry];
    match op {
        OP_CREATE => create(regs, false),
        OP_SIGNAL => { if let SignalOutcome::Wake(t) = n.signal(regs.rsi) { complete(t, [0, 0, n.poll(), 0, 0, 0], false); } 0 }
        OP_NOTIFY_WAIT => {
            let s = ipc(me);
            if s.ready.swap(false, Ordering::AcqRel) { return finish(regs, s); }
            match n.wait(me) {
                WaitOutcome::Ready(bits) => { regs.rsi = bits; 0 }
                WaitOutcome::Parked => wait(regs, me),
                WaitOutcome::Busy => E_BUSY,
            }
        }
        OP_POLL => { regs.rsi = n.poll(); 0 }
        OP_GRANT => grant(regs, slot, false),
        OP_DERIVE => grant(regs, slot, true),
        OP_DESTROY => destroy(slot, false),
        _ => E_BADMSG,
    }
}
