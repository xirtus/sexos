// SPDX-License-Identifier: GPL-3.0-only
//! Stage 6b: a call or reply that waits hands its caller the first event of its own
//! inbox in registers (syscalls 102 and 103), so a round trip needs no listen after
//! the wake. The event is taken on the waiter's own core when it next runs, while
//! that core holds its custody, and written into its saved frame.
use crate::capability::ProtectionDomain;
use crate::ipc::messages::{IpcReply, MessageType};
use crate::scheduler::Task;
use core::sync::atomic::Ordering;

/// Syscall 28's view of one inbox message: [type, caller_pd, arg0, arg1, arg2, caller_task].
pub fn event_words(msg: MessageType) -> [u64; 6] {
    match msg {
        MessageType::IpcCall { func_id, arg0, arg1, arg2, caller_pd, caller_task } =>
            [func_id, caller_pd as u64, arg0, arg1, arg2, caller_task as u64],
        MessageType::DisplayPrimaryFramebuffer { virt_addr, width, height, pitch } =>
            [0x11, 1, virt_addr, width as u64 | (height as u64) << 32, pitch as u64, 0],
        MessageType::RawInput(scancode) => [0x201, 1, scancode as u64, 0, 0, 0],
        MessageType::WindowCreate => [0xDE, 1, 0, 0, 0, 0],
        MessageType::CompositorCommit => [0xDD, 1, 0, 0, 0, 0],
        MessageType::SetWindowRoundness => [0xDF, 1, 0, 0, 0, 0],
        MessageType::SetWindowBlur => [0xE0, 1, 0, 0, 0, 0],
        MessageType::GetDisplayInfo => [0xE3, 1, 0, 0, 0, 0],
        _ => [0xFF, 1, 0, 0, 0, 0],
    }
}

/// The first event of a domain's own inbox, a reply ahead of any message, as syscall
/// 28 on slot 0 takes it. Type 0: nothing waiting.
pub fn take_event(pd: &ProtectionDomain) -> [u64; 6] {
    if let Some(reply) = IpcReply::take(&mut pd.incoming_replies.lock()) {
        return [1, reply.caller_pd as u64, reply.value, 0, 0, reply.caller_task as u64];
    }
    match unsafe { (*pd.message_ring).dequeue() } { Some(msg) => event_words(msg), None => [0; 6] }
}

/// Registers of a 102/103 return: rax status, rdi type, rsi caller_pd << 32 | caller_task,
/// rdx/r10/r8 the arguments.
fn event_regs(e: [u64; 6]) -> [u64; 6] { [0, e[0], e[1] << 32 | (e[5] & 0xFFFF_FFFF), e[2], e[3], e[4]] }

/// After a delivered call or reply: waits like syscall 53, then returns the first inbox
/// event in registers. A task that does not sleep (a worker, or one with something
/// already waiting) gets it at once; a sleeping one gets it when it next runs (`deliver`).
pub unsafe fn wait_and_take(regs: &mut crate::interrupts::SyscallRegs) -> u64 { wait_and_take_from(regs, None) }

/// Cross-core counters: answers caught spinning, spins that ran out.
static SPINS: [core::sync::atomic::AtomicU64; 2] = [const { core::sync::atomic::AtomicU64::new(0) }; 2];
/// How long a call or reply whose partner runs on another core waits for the answer
/// still running. A real CPU answers in a few microseconds; parking, switching away and
/// the wake interrupt back cost more than that, most of all under a hypervisor, where
/// the interrupt and the halt are each a trip out of the guest.
const SPIN_CYCLES: u64 = 40_000;

/// True when domain `pd`'s main task lives on another core. Tasks stay on their home
/// core, so this is where it waits and where it answers from.
fn away(pd: u32) -> bool {
    let Some(domain) = crate::ipc::DOMAIN_REGISTRY.get(pd) else { return false };
    let task = domain.main_task.load(Ordering::Acquire);
    if task.is_null() { return false; }
    let home = match unsafe { (*task).home.load(Ordering::Relaxed) } { u32::MAX => crate::smp::home_core(pd as u64), core => core };
    home != crate::core_local::CoreLocal::get().core_id
}

/// After a call or reply to a task on another core: keep this core until the answer is
/// in the inbox or `SPIN_CYCLES` pass, then park as usual. The task stays running, so
/// the partner's wake only sets its pending bit: no interrupt, no switch either way.
/// Interrupts stay off (this is a system call); TLB shootdowns are answered meanwhile,
/// as `smp::tlb` does while it waits. True when it took a wake: the caller then returns
/// without parking, as a park that finds the bit would, even when the inbox is empty
/// (a worker may have moved the answer to the program's mailbox).
unsafe fn spin_for_partner(task: *mut Task, pd: &ProtectionDomain) -> bool {
    let core = crate::core_local::CoreLocal::get().core_id;
    let until = core::arch::x86_64::_rdtsc() + SPIN_CYCLES;
    loop {
        // Only a pending bit is taken (a park without one would block a running task).
        // A request already in the ring has its wake on the way: that bit stays, and
        // costs at most one early return later.
        let pending = (*task).state.snapshot().pending;
        if pending || (*pd.message_ring).len() != 0 {
            if pending { let _ = (*task).state.park(core); }
            note_spin(0);
            return true;
        }
        if core::arch::x86_64::_rdtsc() >= until { note_spin(1); return false; }
        crate::smp::tlb::service();
        core::hint::spin_loop();
    }
}
fn note_spin(i: usize) {
    let n = SPINS[i].fetch_add(1, Ordering::Relaxed) + 1;
    if n & (n - 1) == 0 && n >= 1024 {
        crate::serial_println!("[ipc.xcore] spin_hits={} spin_misses={} remote_resumes={}", SPINS[0].load(Ordering::Relaxed),
            SPINS[1].load(Ordering::Relaxed), crate::scheduler::REMOTE_RESUMES.load(Ordering::Relaxed));
    }
}

/// `wait_and_take`, after a call or reply to domain `partner`.
unsafe fn wait_and_take_from(regs: &mut crate::interrupts::SyscallRegs, partner: Option<u32>) -> u64 {
    let core = crate::core_local::CoreLocal::get();
    let task = crate::scheduler::SCHEDULERS[core.core_id as usize].current_task.load(Ordering::Acquire);
    let pd = core.current_pd_ref();
    if !task.is_null() && task == pd.main_task.load(Ordering::Acquire) {
        regs.rax = 0;
        if !(partner.is_some_and(away) && spin_for_partner(task, pd)) {
            (*task).deliver.store(true, Ordering::Relaxed);
            crate::scheduler::wait_for_message(regs as *const _);
            // Still here: it did not sleep.
            (*task).deliver.store(false, Ordering::Relaxed);
        }
    }
    let [_, t, from, a0, a1, a2] = event_regs(take_event(pd));
    regs.rdi = t; regs.rsi = from; regs.rdx = a0; regs.r10 = a1; regs.r8 = a2;
    0
}

/// Called with the task's custody as it is about to run: a task that slept in
/// `wait_and_take` finds its event in its saved frame (rax rbx rcx rdx rbp rsi rdi r8
/// r9 r10 ..., as `yield_and_switch` saved it).
pub unsafe fn deliver(task: *mut Task) {
    crate::ipc_v2::syscall::deliver(task);
    if !(*task).deliver.swap(false, Ordering::Relaxed) { return; }
    let pd = &*(*task).context.pd_ptr;
    let [status, ty, from, a0, a1, a2] = event_regs(take_event(pd));
    let frame = (*task).context.kstack_top as *mut u64;
    *frame = status; *frame.add(6) = ty; *frame.add(5) = from; *frame.add(3) = a0; *frame.add(9) = a1; *frame.add(7) = a2;
}

/// Syscall 102, PDX_CALL_RECV: syscall 100, and the answer comes back in registers.
pub unsafe fn call_recv(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let sched = &crate::scheduler::SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize];
    sched.arm_handoff(true);
    let called = if regs.rdi == 0 { Err(sex_pdx::ERR_CAP_INVALID) } else { super::safe_pdx_call(regs.rdi as u32, regs.rsi, regs.rdx, regs.r10, regs.r8) };
    sched.arm_handoff(false);
    let partner = || match (*crate::core_local::CoreLocal::get().current_pd_ref().cap_table).find(regs.rdi as u32)?.data {
        crate::capability::CapabilityData::Domain(id) => Some(id), _ => None };
    match called { Ok(_) => { let partner = partner(); wait_and_take_from(regs, partner) } Err(e) => { regs.rdi = 0; e } }
}

/// Syscall 103, PDX_REPLY_RECV: syscall 101, and the next request comes in registers.
/// A refused reply returns 1 at once, as 101 does.
pub unsafe fn reply_recv(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let sched = &crate::scheduler::SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize];
    sched.arm_handoff(true);
    let sent = super::router::send_reply(regs.rdi as u32, regs.rsi).is_ok();
    sched.arm_handoff(false);
    if sent { wait_and_take_from(regs, Some(regs.rdi as u32)) } else { regs.rdi = 0; 1 }
}
