// SPDX-License-Identifier: GPL-3.0-only
//! Bounded public-data relay between one Vellum run and one separately launched
//! Shibari signer. BOOT already gives Shell a route to every program row. Every
//! request is bound to the kernel-stamped current run task, not just the row.
use sex_pdx::{pdf_sign as p, PdxMessage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run { row: u32, task: u32 }
#[derive(Clone, Copy)]
struct Signer { run: Run, certificate_bytes: usize }
#[derive(Clone, Copy)]
struct Request { client: Run, signer: Run, len: usize, next: usize, finished: bool }
static mut SIGNER: Option<Signer> = None;
static mut REQUEST: Option<Request> = None;

fn live(run: Run) -> bool {
    crate::app_catalog::by_target(run.row).is_some_and(|r| r.app == crate::app_catalog::App::Program)
        && crate::running_task(run.row) == Some(run.task)
}
fn route(run: Run) -> Option<u64> {
    if !live(run) { return None; }
    Some(crate::app_catalog::by_target(run.row)?.route_slot)
}
fn forward(run: Run, op: u64, a: u64, b: u64, c: u64) -> Option<u64> {
    let (status, value) = crate::pdx_call(route(run)?, op, a, b, c);
    (status == 0).then_some(value)
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

pub unsafe fn receive(msg: &PdxMessage) -> bool {
    let op = msg.type_id;
    if !(p::REGISTER..=p::END).contains(&op) { return false; }
    let answer = match op {
        p::REGISTER => {
            let run = client(msg);
            let len = msg.arg0 as usize;
            if msg.arg0 > p::MAX_CERT as u64 || len == 0 || run.is_none()
                || current_signer().is_some_and(|s| Some(s.run) != run) { 1 }
            else {
                SIGNER = Some(Signer { run: run.unwrap(), certificate_bytes: len });
                sex_pdx::serial_record!("[pdf.sign.signer] row={} task={} certificate_bytes={}", run.unwrap().row, run.unwrap().task, len);
                0
            }
        }
        p::LIST => current_signer().map_or(0, |s| (s.run.row as u64) << 32 | s.certificate_bytes as u64),
        p::CERT | p::POINT => match selected(msg) {
            Some((_, signer)) if (op == p::CERT && msg.arg1 < signer.certificate_bytes.div_ceil(8) as u64)
                || (op == p::POINT && msg.arg1 < p::POINT_BYTES.div_ceil(8) as u64) =>
                forward(signer.run, op, msg.arg1, 0, 0).unwrap_or(0),
            _ => 0,
        },
        p::BEGIN => match selected(msg) {
            Some((caller, signer)) if msg.arg1 > 0 && msg.arg1 <= p::MAX_ATTRS as u64
                && REQUEST.is_none_or(|r| !live(r.client) || !live(r.signer)) => {
                let result = forward(signer.run, op, caller.row as u64, msg.arg1, caller.task as u64);
                if result == Some(0) {
                    REQUEST = Some(Request { client: caller, signer: signer.run, len: msg.arg1 as usize, next: 0, finished: false });
                    sex_pdx::serial_record!("[pdf.sign.request] client={} signer={} attrs={}", caller.row, signer.run.row, msg.arg1);
                    0
                } else { 1 }
            }
            _ => 1,
        },
        p::CHUNK => match (selected(msg), REQUEST) {
            (Some((caller, signer)), Some(mut request)) if request.client == caller && request.signer == signer.run
                && !request.finished && msg.arg1 as usize == request.next && request.next * 8 < request.len => {
                let result = forward(signer.run, op, msg.arg1, msg.arg2, 0);
                if result == Some(0) { request.next += 1; REQUEST = Some(request); 0 } else { 1 }
            }
            _ => 1,
        },
        p::FINISH => match (selected(msg), REQUEST) {
            (Some((caller, signer)), Some(mut request)) if request.client == caller && request.signer == signer.run
                && !request.finished && request.next == request.len.div_ceil(8) => {
                let result = forward(signer.run, op, 0, 0, 0);
                if result == Some(0) { request.finished = true; REQUEST = Some(request); 0 } else { 1 }
            }
            _ => 1,
        },
        p::STATUS => match (selected(msg), REQUEST) {
            (Some((caller, signer)), Some(request)) if request.client == caller && request.signer == signer.run && request.finished =>
                forward(signer.run, op, 0, 0, 0).unwrap_or(p::REFUSED),
            _ => p::REFUSED,
        },
        p::SIGNATURE => match (selected(msg), REQUEST) {
            (Some((caller, signer)), Some(request)) if request.client == caller && request.signer == signer.run
                && request.finished && msg.arg1 < p::MAX_SIGNATURE.div_ceil(8) as u64 =>
                forward(signer.run, op, msg.arg1, 0, 0).unwrap_or(0),
            _ => 0,
        },
        p::END => {
            if let (Some((caller, signer)), Some(request)) = (selected(msg), REQUEST) {
                if request.client == caller && request.signer == signer.run {
                    let _ = forward(signer.run, op, 0, 0, 0);
                    REQUEST = None;
                }
            }
            0
        }
        _ => unreachable!(),
    };
    sex_pdx::pdx_reply(msg.caller_pd, answer);
    true
}
