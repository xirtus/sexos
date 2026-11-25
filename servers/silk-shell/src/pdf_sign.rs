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
struct Request { client: Run, signer: Run, len: usize, next: usize, finished: bool }
#[derive(Clone, Copy)]
struct Pending { client: Run, signer: Run, op: u64, arg: usize }
static mut SIGNER: Option<Signer> = None;
static mut REQUEST: Option<Request> = None;
static mut PENDING: Option<Pending> = None;

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
    PENDING = Some(Pending { client, signer, op, arg });
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
    let value = msg.arg0;
    if value == 0 {
        match pending.op {
            p::BEGIN => REQUEST = Some(Request { client: pending.client, signer: pending.signer, len: pending.arg, next: 0, finished: false }),
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
    if reply(msg) { return true; }
    let op = msg.type_id;
    if !(p::REGISTER..=p::END).contains(&op) { return false; }
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
    if op == p::BEGIN && REQUEST.is_some_and(|r| !live(r.client) || !live(r.signer)) { REQUEST = None; }
    let queued = match op {
        p::CERT if msg.arg1 < signer.certificate_bytes.div_ceil(8) as u64 =>
            send(caller, signer.run, op, 0, msg.arg1, 0, 0),
        p::POINT if msg.arg1 < p::POINT_BYTES.div_ceil(8) as u64 =>
            send(caller, signer.run, op, 0, msg.arg1, 0, 0),
        p::BEGIN if msg.arg1 > 0 && msg.arg1 <= p::MAX_ATTRS as u64 && REQUEST.is_none() => {
            let ok = send(caller, signer.run, op, msg.arg1 as usize, caller.row as u64, msg.arg1, caller.task as u64);
            if ok { sex_pdx::serial_record!("[pdf.sign.request] client={} signer={} attrs={}", caller.row, signer.run.row, msg.arg1); }
            ok
        }
        p::CHUNK if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && !r.finished
            && r.next == msg.arg1 as usize && r.next * 8 < r.len) =>
            send(caller, signer.run, op, msg.arg1 as usize, msg.arg1, msg.arg2, 0),
        p::FINISH if REQUEST.is_some_and(|r| r.client == caller && r.signer == signer.run && !r.finished && r.next == r.len.div_ceil(8)) =>
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
