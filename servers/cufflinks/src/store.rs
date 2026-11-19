// SPDX-License-Identifier: GPL-3.0-only
//! Bonds kept on disk by Files (`sex_pdx::bt_bonds`, `/system/bluetooth.bonds`), in the
//! `sex_bt::bonds` format. Read once when a controller is ready, written whenever the
//! host stack says the bonds changed.

use alloc::vec::Vec;
use sex_bt::bonds::Bonds;
use sex_pdx::bt_bonds as b;
use sex_pdx::serial_record as log;

/// Files' domain (BOOT spawn order).
const FILES: u32 = 11;
/// How long to wait for one answer (ticks).
const PATIENCE: u64 = 3_000;

fn send(arg0: u64, arg1: u64, answered: bool) -> bool {
    for _ in 0..64 {
        let call = if answered { sex_pdx::pdx_call_wait } else { sex_pdx::pdx_call };
        match call(sex_pdx::SLOT_STORAGE, b::OP, arg0, arg1, 0) {
            (0, _) => return true,
            (sex_pdx::ERR_SERVICE_NOT_READY, _) => sex_pdx::sys_yield(),
            _ => return false,
        }
    }
    false
}

/// Files' answer; other messages are answered on the way (SexUSB's link notes).
fn answer() -> Option<u64> {
    let give_up = sex_pdx::get_ticks().saturating_add(PATIENCE);
    loop {
        match sex_pdx::pdx_try_listen_raw(0) {
            Some(m) if m.type_id == 1 && m.caller_pd == FILES => { if let Some(v) = b::decode_reply(m.arg0) { return Some(v); } }
            Some(m) if m.type_id > 1 => { let _ = sex_pdx::pdx_reply(m.caller_pd, if m.type_id == sex_pdx::bt_hci::OP_LINK { 0 } else { u64::MAX }); }
            Some(_) => {}
            None => {
                if sex_pdx::get_ticks() > give_up { return None; }
                let _ = sex_pdx::sys_wait_self_ticks(2);
            }
        }
    }
}

fn call(verb: u8, chunk: u16) -> Option<u64> {
    if !send(b::request(verb, chunk), 0, true) { return None; }
    answer()
}

/// The stored bonds (empty when there are none or Files cannot be reached).
pub fn load() -> Bonds {
    let Some(head) = call(b::LOAD, 0) else { log!("[cufflinks.bonds] loaded=0 reason=no_answer"); return Bonds::default() };
    if head & 0xff != b::OK as u64 { log!("[cufflinks.bonds] loaded=0 status={}", head & 0xff); return Bonds::default(); }
    let len = (head >> 8) as usize;
    let mut bytes = Vec::with_capacity(len + 7);
    for chunk in 0..len.div_ceil(7) {
        let Some(w) = call(b::GET, chunk as u16) else { log!("[cufflinks.bonds] loaded=0 reason=short"); return Bonds::default() };
        bytes.extend_from_slice(&w.to_le_bytes()[..7]);
    }
    bytes.truncate(len);
    match Bonds::decode(&bytes) {
        Some(bonds) => { log!("[cufflinks.bonds] loaded={} le={} classic={}", bonds.len(), bonds.le.len(), bonds.classic.len()); bonds }
        None => { log!("[cufflinks.bonds] loaded=0 reason=damaged bytes={}", len); Bonds::default() }
    }
}

/// Write the bonds; false if Files did not take them.
pub fn save(bonds: &Bonds) -> bool {
    let bytes = bonds.encode();
    let mut ok = true;
    let mut status = None;
    for (arg0, arg1) in b::save_requests(&bytes) {
        let last = b::decode_request(arg0).0 == b::SAVE;
        ok &= send(arg0, arg1, last);
        if last && ok { status = answer(); }
    }
    let saved = ok && status == Some(b::OK as u64);
    log!("[cufflinks.bonds] saved={} count={} bytes={}", saved as u8, bonds.len(), bytes.len());
    saved
}
