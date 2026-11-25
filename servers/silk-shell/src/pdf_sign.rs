// SPDX-License-Identifier: GPL-3.0-only
//! Asynchronous, task-bound relay between Vellum and a separately launched
//! Shibari signer. Calls enqueue; only the signer's later stamped reply is
//! returned to the waiting Vellum run. Shell stays free to deliver approval keys.
use sex_pdx::{pdf_sign as p, PdxMessage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run { row: u32, task: u32 }
#[derive(Clone, Copy)]
struct Signer { run: Run, certificate_bytes: usize }
#[derive(Clone, Copy)]
struct Request { client: Run, signer: Run, len: usize, next: usize, meta_len: usize, meta_next: usize, finished: bool }
#[derive(Clone, Copy)]
struct Pending { client: Run, signer: Run, op: u64, arg: usize, meta_len: usize, since: u64, canceled: bool }
static mut SIGNER: Option<Signer> = None;
static mut REQUEST: Option<Request> = None;
static mut PENDING: Option<Pending> = None;
/// Exact Vellum runs certified by Files at load time. A recycled row never inherits it.
struct ApprovedRuns([Option<Run>; 64]);
impl ApprovedRuns {
    const fn new() -> Self { Self([None; 64]) }
    fn bind(&mut self, run: Run, approved: bool) {
        for entry in &mut self.0 { if entry.is_some_and(|old| old.row == run.row) { *entry = None; } }
        if approved {
            if let Some(entry) = self.0.iter_mut().find(|entry| entry.is_none()) { *entry = Some(run); }
        }
    }
    fn retire(&mut self, run: Run) {
        for entry in &mut self.0 { if *entry == Some(run) { *entry = None; } }
    }
    fn contains(&self, run: Run) -> bool { self.0.contains(&Some(run)) }
}
static mut APPROVED: ApprovedRuns = ApprovedRuns::new();

pub unsafe fn bind_approved(row: u32, task: u32, approved: bool) {
    APPROVED.bind(Run { row, task }, approved);
    sex_pdx::serial_record!("[pdf.sign.image] row={} task={} approved={}", row, task, approved as u8);
}

pub unsafe fn retire(row: u32, task: u32) {
    APPROVED.retire(Run { row, task });
}

fn approved(run: Run) -> bool {
    unsafe { APPROVED.contains(run) && live(run) }
}

#[cfg(test)]
mod approved_tests {
    use super::*;
    #[test]
    fn reused_row_does_not_inherit_identity() {
        let mut runs = ApprovedRuns::new();
        let old = Run { row: 57, task: 101 };
        let new = Run { row: 57, task: 102 };
        runs.bind(old, true);
        assert!(runs.contains(old));
        runs.bind(new, false);
        assert!(!runs.contains(old));
        assert!(!runs.contains(new));
        runs.bind(new, true);
        runs.retire(old);
        assert!(runs.contains(new));
        runs.retire(new);
        assert!(!runs.contains(new));
    }
}

unsafe fn expire_pending(now: u64) {
    if let Some(mut pending) = PENDING {
        if now.saturating_sub(pending.since) >= 5_000 {
            pending.canceled = true;
            PENDING = Some(pending);
            REQUEST = None;
        }
    }
}

fn live(run: Run) -> bool {
    crate::app_catalog::by_target(run.row).is_some_and(|r| r.app == crate::app_catalog::App::Program)
        && crate::running_task(run.row) == Some(run.task)
}
fn route(run: Run) -> Option<u64> {
    if !live(run) { return None; }
    Some(crate::app_catalog::by_target(run.row)?.route_slot)
}
fn current_signer() -> Option<Signer> { unsafe { SIGNER.filter(|s| live(s.run)) } }
fn client(msg: &PdxMessage) -> Option<Run> {
    let run = Run { row: msg.caller_pd, task: msg.caller_task };
    live(run).then_some(run)
}
fn selected(msg: &PdxMessage) -> Option<(Run, Signer)> {
    let c = client(msg)?;
    let s = current_signer()?;
    (c != s.run && msg.arg0 == s.run.row as u64).then_some((c, s))
}

/// Enqueue the inner call, keeping its expected reply. Shell never waits for
/// an ordinary program to process the call.
unsafe fn send(client: Run, signer: Run, op: u64, arg: usize, a: u64, b: u64, c: u64) -> bool {
    if PENDING.is_some() { return false; }
    let Some(slot) = route(signer) else { return false };
    let (status, _) = crate::pdx_call(slot, op, a, b, c);
    if status != 0 { return false; }
    PENDING = Some(Pending { client, signer, op, arg, meta_len: 0, since: sex_pdx::get_ticks(), canceled: false });
    true
}

/// A signer reply is accepted only from the exact run that got the inner call.
unsafe fn reply(msg: &PdxMessage) -> bool {
    if msg.type_id != 1 { return false; }
    let Some(pending) = PENDING else {
        return current_signer().is_some_and(|s| s.run == Run { row: msg.caller_pd, task: msg.caller_task });
    };
    if pending.signer != (Run { row: msg.caller_pd, task: msg.caller_task }) { return false; }
    PENDING = None;
    if pending.canceled {
        REQUEST = None;
        // The old reply has now drained. Reset Shibari before allowing a new
        // request; no late word can be mistaken for the reset's reply.
        if pending.op != p::END && live(pending.signer) {
            if !send(pending.client, pending.signer, p::END, 0, 0, 0, 0) {
                // Do not risk reusing this run if its reset cannot be queued.
                SIGNER = None;
            } else if let Some(mut reset) = PENDING { reset.canceled = true; PENDING = Some(reset); }
        }
        return true;
    }
    let value = msg.arg0;
    if value == 0 {
        match pending.op {
            p::BEGIN => REQUEST = Some(Request { client: pending.client, signer: pending.signer, len: pending.arg, next: 0, meta_len: pending.meta_len, meta_next: 0, finished: false }),
            p::META => if let Some(mut request) = REQUEST { request.meta_next += 1; REQUEST = Some(request); },
            p::CHUNK => if let Some(mut request) = REQUEST { request.next += 1; REQUEST = Some(request); },
            p::FINISH => if let Some(mut request) = REQUEST { request.finished = true; REQUEST = Some(request); },
            _ => {},
        }
    }
    if pending.op == p::END { REQUEST = None; }
    if live(pending.client) { sex_pdx::pdx_reply(pending.client.row, value); }
    true
}

pub unsafe fn receive(msg: &PdxMessage) -> bool {
    expire_pending(sex_pdx::get_ticks());
    if reply(msg) { return true; }
    let op = msg.type_id;
    if !(p::REGISTER..=p::META).contains(&op) { return false; }
    // A dead client or signer cannot complete its queued call. Release the
    // broker reservation when the next protocol message reaches Shell.
    if PENDING.is_some_and(|r| !live(r.client) || !live(r.signer)) {
        PENDING = None;
        REQUEST = None;
    }
    if op == p::REGISTER {
        let run = client(msg);
        let len = msg.arg0 as usize;
        let answer = if msg.arg0 > p::MAX_CERT as u64 || len == 0 || run.is_none()
            || current_signer().is_some_and(|s| Some(s.run) != run) { 1 }
        else {
            SIGNER = Some(Signer { run: run.unwrap(), certificate_bytes: len });
            sex_pdx::serial_record!("[pdf.sign.signer] row={} task={} certificate_bytes={}", run.unwrap().row, run.unwrap().task, len);
            0
        };
        sex_pdx::pdx_reply(msg.caller_pd, answer);
        return true;
    }
    if op == p::LIST {
        let answer = if client(msg).is_some() { current_signer().map_or(0, |s| (s.run.row as u64) << 32 | s.certificate_bytes as u64) } else { 0 };
        sex_pdx::pdx_reply(msg.caller_pd, answer);
        return true;
    }
    let Some((caller, signer)) = selected(msg) else { sex_pdx::pdx_reply(msg.caller_pd, p::REFUSED); return true };
    if op == p::BEGIN && !approved(caller) {
        sex_pdx::serial_record!("[pdf.sign.denied] reason=unapproved_image row={} task={}", caller.row, caller.task);
        sex_pdx::pdx_reply(caller.row, p::REFUSED);
        return true;
    }
    if op == p::END {
        if let Some(mut pending) = PENDING {
            if pending.client == caller && pending.signer == signer.run {
                pending.canceled = true;
                PENDING = Some(pending);
                REQUEST = None;
                sex_pdx::pdx_reply(caller.row, 0);
                return true;
            }
        }
    }
    if op == p::BEGIN && REQUEST.is_some_and(|r| !live(r.client) || !live(r.signer)) { REQUEST = None; }
    let queued = match op {
        p::CERT if msg.arg1 < signer.certificate_bytes.div_ceil(8) as u64 =>
            send(caller, signer.run, op, 0, msg.arg1, 0, 0),
        p::POINT if msg.arg1 < p::POINT_BYTES.div_ceil(8) as u64 =>
            send(caller, signer.run, op, 0, msg.arg1, 0, 0),
        p::BEGIN if msg.arg1 > 0 && msg.arg1 <= p::MAX_ATTRS as u64 && REQUEST.is_none()
            && p::decode_metadata_lengths(msg.arg2).is_some() => {
            let ok = send(caller, signer.run, op, msg.arg1 as usize, caller.row as u64, msg.arg1, msg.arg2);
            if ok { if let Some(mut pending) = PENDING { pending.meta_len = p::decode_metadata_lengths(msg.arg2).unwrap().0 + p::decode_metadata_lengths(msg.arg2).unwrap().1; PENDING = Some(pending); } }
            if ok { sex_pdx::serial_record!("[pdf.sign.request] client={} signer={} attrs={}", caller.row, signer.run.row, msg.arg1); }
            ok
        }
        p::META if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && !r.finished
            && r.meta_next == msg.arg1 as usize && r.meta_next * 8 < r.meta_len && r.next == 0) =>
            send(caller, signer.run, op, msg.arg1 as usize, msg.arg1, msg.arg2, 0),
        p::CHUNK if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && !r.finished
            && r.meta_next == r.meta_len.div_ceil(8) && r.next == msg.arg1 as usize && r.next * 8 < r.len) =>
            send(caller, signer.run, op, msg.arg1 as usize, msg.arg1, msg.arg2, 0),
        p::FINISH if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && !r.finished
            && r.meta_next == r.meta_len.div_ceil(8) && r.next == r.len.div_ceil(8)) =>
            send(caller, signer.run, op, 0, 0, 0, 0),
        p::STATUS if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && r.finished) =>
            send(caller, signer.run, op, 0, 0, 0, 0),
        p::SIGNATURE if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && r.finished)
            && msg.arg1 < p::MAX_SIGNATURE.div_ceil(8) as u64 =>
            send(caller, signer.run, op, 0, msg.arg1, 0, 0),
        p::END if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run) =>
            send(caller, signer.run, op, 0, 0, 0, 0),
        _ => false,
    };
    if !queued { sex_pdx::pdx_reply(msg.caller_pd, p::REFUSED); }
    true
}
