// SPDX-License-Identifier: GPL-3.0-only
//! A call that waits for its answer, and a reply that waits for the next request, with
//! the event coming back in registers (system calls 102 and 103): a round trip is two
//! kernel entries, and on one core the partner runs next without a queue. The event is
//! the first of this domain's own inbox, a reply ahead of any message, exactly as
//! `pdx_listen_raw(0)` would take it; it need not be the answer to this call (another
//! answer or a message may be ahead of it), so callers handle it as they handle a listen.
use crate::PdxMessage;

fn event(t: u64, from: u64, a0: u64, a1: u64, a2: u64) -> Option<PdxMessage> {
    (t != 0).then(|| PdxMessage { type_id: t, arg0: a0, arg1: a1, arg2: a2, caller_pd: (from >> 32) as u32, caller_task: from as u32 })
}

/// `pdx_call_wait`, and the first inbox event after the wake in registers. `Err` is the
/// call's refusal (nothing was sent); `Ok(None)` means woken with nothing waiting yet
/// (listen as usual).
#[inline]
pub fn pdx_call_recv(slot: u64, opcode: u64, arg0: u64, arg1: u64, arg2: u64) -> Result<Option<PdxMessage>, u64> {
    let (status, t, from, a0, a1, a2): (u64, u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 102u64 => status,
            inlateout("rdi") slot => t,
            inlateout("rsi") opcode => from,
            inlateout("rdx") arg0 => a0,
            inlateout("r10") arg1 => a1,
            inlateout("r8") arg2 => a2,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    if status != 0 { return Err(status); }
    Ok(event(t, from, a0, a1, a2))
}

/// `pdx_reply_wait`, and the next inbox event (usually the next request) in registers.
/// `Err(())`: the reply was refused and nothing waited, as `pdx_reply_wait` returning 1.
#[inline]
pub fn pdx_reply_recv(target_pd: u32, value: u64) -> Result<Option<PdxMessage>, ()> {
    let (status, t, from, a0, a1, a2): (u64, u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 103u64 => status,
            inlateout("rdi") target_pd as u64 => t,
            inlateout("rsi") value => from,
            lateout("rdx") a0,
            lateout("r10") a1,
            lateout("r8") a2,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    if status != 0 { return Err(()); }
    Ok(event(t, from, a0, a1, a2))
}
