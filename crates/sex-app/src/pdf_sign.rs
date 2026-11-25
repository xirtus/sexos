// SPDX-License-Identifier: GPL-3.0-only
//! Serialized Vellum client for Shell's Shibari relay. The PDX call reports
//! queue admission; the answer arrives later as a stamped reply message.
use super::mailbox;
use sex_pdx::{self, pdf_sign as p};

pub use p::{Signer, POINT_BYTES, MAX_SIGNATURE};

fn mine(m: &sex_pdx::PdxMessage) -> bool {
    m.type_id == 1 && m.caller_pd == 3 && m.caller_task == 3
}

fn discard_stale_shell_replies() {
    // Window::open's registration reply can be queued before Vellum starts
    // signing. Window::title and normal window actions do not produce replies.
    // selected() is the first signing operation, so no signing reply is due yet.
    while mailbox::take(mine).is_some() {}
    while let Some(message) = sex_pdx::pdx_try_listen_raw(0) {
        if !mine(&message) { mailbox::put(message); }
    }
}

fn call(op: u64, a: u64, b: u64, c: u64) -> Result<u64, u64> {
    let admission_deadline = sex_pdx::get_ticks().saturating_add(5_000);
    loop {
        match sex_pdx::pdx_call(sex_pdx::SLOT_SHELL, op, a, b, c).0 {
            0 => break,
            sex_pdx::ERR_SERVICE_NOT_READY if sex_pdx::get_ticks() < admission_deadline => sex_pdx::sys_yield(),
            error => return Err(error),
        }
    }
    let reply_deadline = sex_pdx::get_ticks().saturating_add(5_000);
    loop {
        let message = mailbox::take(mine).or_else(|| sex_pdx::pdx_try_listen_raw(0));
        if let Some(message) = message {
            if mine(&message) { return Ok(message.arg0); }
            mailbox::put(message);
        } else if sex_pdx::get_ticks() >= reply_deadline { return Err(p::REFUSED); }
        else { let _ = sex_pdx::sys_wait_self_ticks(2); }
    }
}

/// Shibari announces a checked public certificate; Shell binds its current run.
pub fn register(certificate_bytes: usize) -> Result<(), u64> {
    if certificate_bytes == 0 || certificate_bytes > p::MAX_CERT { return Err(p::REFUSED); }
    if call(p::REGISTER, certificate_bytes as u64, 0, 0)? == 0 { Ok(()) } else { Err(p::REFUSED) }
}

/// The sole active Shibari certificate. Its identity is displayed before use.
pub fn selected() -> Result<Option<Signer>, u64> {
    discard_stale_shell_replies();
    let value = call(p::LIST, 0, 0, 0)?;
    if value == 0 { return Ok(None); }
    let signer = Signer { row: (value >> 32) as u32, certificate_bytes: value as u32 as usize };
    if signer.row == 0 || signer.certificate_bytes == 0 || signer.certificate_bytes > p::MAX_CERT { return Err(p::REFUSED); }
    Ok(Some(signer))
}

pub fn certificate(signer: Signer, into: &mut [u8]) -> Result<(), u64> {
    if into.len() != signer.certificate_bytes || into.len() > p::MAX_CERT { return Err(p::REFUSED); }
    for (i, chunk) in into.chunks_mut(8).enumerate() {
        p::unpack(call(p::CERT, signer.row as u64, i as u64, 0)?, chunk);
    }
    Ok(())
}

pub fn public_point(signer: Signer, into: &mut [u8; POINT_BYTES]) -> Result<(), u64> {
    for (i, chunk) in into.chunks_mut(8).enumerate() {
        p::unpack(call(p::POINT, signer.row as u64, i as u64, 0)?, chunk);
    }
    Ok(())
}

pub fn request(signer: Signer, attrs: &[u8]) -> Result<(), u64> {
    if attrs.is_empty() || attrs.len() > p::MAX_ATTRS { return Err(p::REFUSED); }
    if call(p::BEGIN, signer.row as u64, attrs.len() as u64, 0)? != 0 { return Err(p::REFUSED); }
    for (i, chunk) in attrs.chunks(8).enumerate() {
        if call(p::CHUNK, signer.row as u64, i as u64, p::pack(chunk))? != 0 { cancel(signer); return Err(p::REFUSED); }
    }
    if call(p::FINISH, signer.row as u64, 0, 0)? != 0 { cancel(signer); return Err(p::REFUSED); }
    Ok(())
}

/// `None` until the Shibari window's owner approves or refuses the request.
pub fn poll(signer: Signer, into: &mut [u8; MAX_SIGNATURE]) -> Result<Option<usize>, u64> {
    let len = call(p::STATUS, signer.row as u64, 0, 0)?;
    if len == 0 { return Ok(None); }
    if len == p::REFUSED || len > MAX_SIGNATURE as u64 { cancel(signer); return Err(p::REFUSED); }
    for (i, chunk) in into[..len as usize].chunks_mut(8).enumerate() {
        p::unpack(call(p::SIGNATURE, signer.row as u64, i as u64, 0)?, chunk);
    }
    cancel(signer);
    Ok(Some(len as usize))
}

pub fn cancel(signer: Signer) { let _ = call(p::END, signer.row as u64, 0, 0); }
