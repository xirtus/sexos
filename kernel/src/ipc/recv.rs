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
pub unsafe fn wait_and_take(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let core = crate::core_local::CoreLocal::get();
    let task = crate::scheduler::SCHEDULERS[core.core_id as usize].current_task.load(Ordering::Acquire);
    let pd = core.current_pd_ref();
    if !task.is_null() && task == pd.main_task.load(Ordering::Acquire) {
        regs.rax = 0;
        (*task).deliver.store(true, Ordering::Relaxed);
        crate::scheduler::wait_for_message(regs as *const _);
        // Still here: it did not sleep.
        (*task).deliver.store(false, Ordering::Relaxed);
    }
    let [_, t, from, a0, a1, a2] = event_regs(take_event(pd));
    regs.rdi = t; regs.rsi = from; regs.rdx = a0; regs.r10 = a1; regs.r8 = a2;
    0
}

/// Called with the task's custody as it is about to run: a task that slept in
/// `wait_and_take` finds its event in its saved frame (rax rbx rcx rdx rbp rsi rdi r8
/// r9 r10 ..., as `yield_and_switch` saved it).
pub unsafe fn deliver(task: *mut Task) {
    if !(*task).deliver.swap(false, Ordering::Relaxed) { return; }
    let pd = &*(*task).context.pd_ptr;
    let [status, t, from, a0, a1, a2] = event_regs(take_event(pd));
    let frame = (*task).context.kstack_top as *mut u64;
    *frame = status; *frame.add(6) = t; *frame.add(5) = from; *frame.add(3) = a0; *frame.add(9) = a1; *frame.add(7) = a2;
}

/// Syscall 102, PDX_CALL_RECV: syscall 100, and the answer comes back in registers.
pub unsafe fn call_recv(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let sched = &crate::scheduler::SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize];
    sched.arm_handoff(true);
    let called = if regs.rdi == 0 { Err(sex_pdx::ERR_CAP_INVALID) } else { super::safe_pdx_call(regs.rdi as u32, regs.rsi, regs.rdx, regs.r10, regs.r8) };
    sched.arm_handoff(false);
    match called { Ok(_) => wait_and_take(regs), Err(e) => { regs.rdi = 0; e } }
}

/// Syscall 103, PDX_REPLY_RECV: syscall 101, and the next request comes in registers.
/// A refused reply returns 1 at once, as 101 does.
pub unsafe fn reply_recv(regs: &mut crate::interrupts::SyscallRegs) -> u64 {
    let sched = &crate::scheduler::SCHEDULERS[crate::core_local::CoreLocal::get().core_id as usize];
    sched.arm_handoff(true);
    let sent = super::router::send_reply(regs.rdi as u32, regs.rsi).is_ok();
    sched.arm_handoff(false);
    if sent { wait_and_take(regs) } else { regs.rdi = 0; 1 }
}
