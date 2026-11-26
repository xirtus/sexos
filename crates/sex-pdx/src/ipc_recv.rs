// SPDX-License-Identifier: GPL-3.0-only
//! A call that waits for its answer, and a reply that waits for the next request, with
//! the event coming back in registers (system calls 102 and 103): a round trip is two
//! kernel entries, and on one core the partner runs next without a queue. The event is
//! the first of this domain's own inbox, a reply ahead of any message, exactly as
//! `pdx_listen_raw(0)` would take it; it need not be the answer to this call (another
//! answer or a message may be ahead of it), so callers handle it as they handle a listen.
use crate::PdxMessage;

#[cfg(target_arch = "x86_64")]
fn event(t: u64, from: u64, a0: u64, a1: u64, a2: u64) -> Option<PdxMessage> {
    (t != 0).then(|| PdxMessage { type_id: t, arg0: a0, arg1: a1, arg2: a2, caller_pd: (from >> 32) as u32, caller_task: from as u32 })
}

/// `pdx_call_wait`, and the first inbox event after the wake in registers. `Err` is the
/// call's refusal (nothing was sent); `Ok(None)` means woken with nothing waiting yet
/// (listen as usual).
#[cfg(target_arch = "x86_64")]
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
#[cfg(target_arch = "x86_64")]
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

/// True when `msg` is a kernel-synthesized `PEER_RESTARTED` reply from `expected_pd`.
#[inline]
pub const fn is_peer_restarted(msg: &PdxMessage, expected_pd: u32) -> bool {
    msg.type_id == 1
        && msg.caller_pd == expected_pd
        && msg.arg0 == crate::crash_log::PEER_RESTARTED
}

/// Outcome of one attempt inside [`retry_on_peer_restart_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptOutcome<T, E> {
    /// Request completed with a final answer.
    Done(T),
    /// Send refused with `ERR_SERVICE_NOT_READY`: either transient backpressure
    /// while the peer is `Live` at the same epoch (yields without burning
    /// `max_retries`) or an actual service restart / crash-loop give-up.
    NotReady,
    /// Service restarted mid-request (`PEER_RESTARTED` / `E_RESTARTED`): wait
    /// for the peer to come back `Live` and re-issue if restart retries remain.
    PeerRestarted,
    /// Permanent error (do not retry).
    Failed(E),
}

/// Reason [`retry_on_peer_restart_with`] stopped without a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconnectError<E> {
    /// Supervision gave up on the peer (crash-loop limit reached).
    GaveUp,
    /// Exhausted `max_retries` or `timeout_ticks` waiting for the peer to return.
    Exhausted,
    /// The request failed with a non-retryable error.
    Failed(E),
}

/// Default bounded retry count for automatic client reconnect.
pub const DEFAULT_RECONNECT_RETRIES: u32 = 8;
/// Default bounded wait (in ticks) for a restarting service to become `Live`.
pub const DEFAULT_RECONNECT_TIMEOUT_TICKS: u64 = 15_000;

/// Pure, host-testable automatic reconnect loop. Runs `attempt()`; on
/// [`AttemptOutcome::NotReady`], distinguishes transient busy backpressure
/// (`PeerLife::Live` at an unchanged epoch, which yields without consuming
/// `max_retries`) from an actual restart or [`crate::crash_log::PeerLife::GaveUp`].
/// On [`AttemptOutcome::PeerRestarted`], observes the peer via `query_peer()` until
/// it reports [`crate::crash_log::PeerLife::Live`] (tracking epoch advances),
/// stops immediately on [`crate::crash_log::PeerLife::GaveUp`] or `timeout_ticks`,
/// and re-issues the call up to `max_retries` times.
pub fn retry_on_peer_restart_with<T, E>(
    max_retries: u32,
    timeout_ticks: u64,
    mut query_peer: impl FnMut() -> Result<crate::crash_log::PeerState, u64>,
    mut now_ticks: impl FnMut() -> u64,
    mut pause: impl FnMut(),
    mut attempt: impl FnMut() -> AttemptOutcome<T, E>,
) -> Result<T, ReconnectError<E>> {
    use crate::crash_log::{PeerLife, DENIED, INVALID, NOT_FOUND};
    let start = now_ticks();
    let mut retries = 0u32;
    let mut last_epoch = query_peer().ok().map(|p| p.epoch).unwrap_or(0);
    loop {
        match attempt() {
            AttemptOutcome::Done(v) => return Ok(v),
            AttemptOutcome::Failed(e) => return Err(ReconnectError::Failed(e)),
            AttemptOutcome::NotReady => {
                if now_ticks().saturating_sub(start) >= timeout_ticks {
                    return Err(ReconnectError::Exhausted);
                }
                match query_peer() {
                    Ok(p) if p.life == PeerLife::GaveUp => return Err(ReconnectError::GaveUp),
                    Ok(p) if p.live() => {
                        if last_epoch != 0 && p.epoch > last_epoch {
                            // Peer restarted between calls (epoch advanced).
                            if retries >= max_retries {
                                return Err(ReconnectError::Exhausted);
                            }
                            retries += 1;
                            last_epoch = p.epoch;
                        } else {
                            // Peer is Live at the same epoch: transient full-inbox
                            // backpressure. Yield without burning the restart budget.
                            if last_epoch == 0 {
                                last_epoch = p.epoch;
                            }
                            pause();
                        }
                    }
                    Err(NOT_FOUND | DENIED | INVALID) => {
                        // Peer is not a restart-supervised slot; treat NotReady as
                        // transient busy backpressure bounded by timeout_ticks.
                        pause();
                    }
                    Err(_) => {
                        // Contended snapshot (e.g. RETRY): yield and re-issue without
                        // burning the restart budget.
                        pause();
                    }
                    Ok(_) => {
                        // Peer is actually down / restarting (Starting, Ending, Exited,
                        // Dead, FaultBlocked): counts as one restart retry.
                        if retries >= max_retries {
                            return Err(ReconnectError::Exhausted);
                        }
                        retries += 1;
                        loop {
                            if now_ticks().saturating_sub(start) >= timeout_ticks {
                                return Err(ReconnectError::Exhausted);
                            }
                            match query_peer() {
                                Ok(p) if p.life == PeerLife::GaveUp => return Err(ReconnectError::GaveUp),
                                Ok(p) if p.live() => {
                                    last_epoch = p.epoch;
                                    break;
                                }
                                Err(NOT_FOUND | DENIED | INVALID) => {
                                    pause();
                                    break;
                                }
                                _ => pause(),
                            }
                        }
                    }
                }
            }
            AttemptOutcome::PeerRestarted => {
                if retries >= max_retries {
                    return Err(ReconnectError::Exhausted);
                }
                retries += 1;
                loop {
                    if now_ticks().saturating_sub(start) >= timeout_ticks {
                        return Err(ReconnectError::Exhausted);
                    }
                    match query_peer() {
                        Ok(p) if p.life == PeerLife::GaveUp => return Err(ReconnectError::GaveUp),
                        Ok(p) if p.live() => {
                            if last_epoch == 0 || p.epoch > last_epoch {
                                last_epoch = p.epoch;
                            } else {
                                pause();
                            }
                            break;
                        }
                        Err(NOT_FOUND | DENIED | INVALID) => {
                            pause();
                            break;
                        }
                        _ => pause(),
                    }
                }
            }
        }
    }
}

/// Retries an idempotent operation targeting the restartable service at `slot`
/// whenever it reports [`AttemptOutcome::NotReady`] or [`AttemptOutcome::PeerRestarted`].
#[cfg(target_arch = "x86_64")]
pub fn retry_on_peer_restart<T, E>(
    slot: u64,
    max_retries: u32,
    timeout_ticks: u64,
    attempt: impl FnMut() -> AttemptOutcome<T, E>,
) -> Result<T, ReconnectError<E>> {
    retry_on_peer_restart_with(
        max_retries,
        timeout_ticks,
        || crate::crash_log::peer_state(slot),
        crate::get_ticks,
        crate::sys_yield,
        attempt,
    )
}

/// Issues an idempotent `pdx_call_recv` to `slot` (`expected_pd`), automatically
/// yielding on transient busy `ERR_SERVICE_NOT_READY` and waiting for the peer
/// to return `Live` and retrying on actual restarts or a `PEER_RESTARTED` reply.
#[cfg(target_arch = "x86_64")]
pub fn call_with_reconnect(
    slot: u64,
    expected_pd: u32,
    opcode: u64,
    arg0: u64,
    arg1: u64,
    arg2: u64,
    max_retries: u32,
    timeout_ticks: u64,
) -> Result<Option<PdxMessage>, ReconnectError<u64>> {
    retry_on_peer_restart(slot, max_retries, timeout_ticks, || {
        match pdx_call_recv(slot, opcode, arg0, arg1, arg2) {
            Ok(Some(msg)) if is_peer_restarted(&msg, expected_pd) => AttemptOutcome::PeerRestarted,
            Ok(msg) => AttemptOutcome::Done(msg),
            Err(crate::ERR_SERVICE_NOT_READY) => AttemptOutcome::NotReady,
            Err(e) => AttemptOutcome::Failed(e),
        }
    })
}
