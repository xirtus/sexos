// SPDX-License-Identifier: GPL-3.0-only
//! Settings kept by Files (`sex_pdx::config`): any program may read them; only the
//! program run from `/data/settings` (Settings itself) may change them.
use super::{is_answer, mailbox, FILES};
use sex_pdx::config as c;
pub use sex_pdx::config::schema::{self, id, Key, Kind, KEYS};

/// Why a request failed: a `sex_pdx::config` status, or `UNAVAILABLE`.
pub type Error = u8;
pub const UNAVAILABLE: Error = 0xFF;

fn mine(m: &sex_pdx::PdxMessage) -> bool {
    m.type_id == 1 && m.caller_pd == FILES && c::Reply::decode(m.arg0).is_some()
}

/// One request, answered before it returns. Other messages wait in the mailbox.
fn call(verb: u8, key: u16, arg1: u64) -> Result<c::Reply, Error> {
    loop {
        match sex_pdx::pdx_call_wait(sex_pdx::SLOT_STORAGE, c::OP, c::request(verb, key), arg1, 0) {
            (0, _) => break,
            (sex_pdx::ERR_SERVICE_NOT_READY, _) => sex_pdx::sys_yield(),
            _ => return Err(UNAVAILABLE),
        }
    }
    let give_up = sex_pdx::get_ticks().saturating_add(5_000);
    loop {
        let m = match mailbox::take(mine) {
            Some(m) => m,
            None => match sex_pdx::pdx_try_listen_raw(0) {
                Some(m) => m,
                None => {
                    if sex_pdx::get_ticks() > give_up { return Err(UNAVAILABLE); }
                    let _ = sex_pdx::sys_wait_self_ticks(2);
                    continue;
                }
            },
        };
        if mine(&m) { return c::Reply::decode(m.arg0).ok_or(UNAVAILABLE); }
        // Keys, closes and other tasks' answers: kept for whoever waits for them.
        if is_answer(&m) || m.caller_pd == super::SHELL { mailbox::put(m); }
    }
}
fn ok(r: c::Reply) -> Result<c::Reply, Error> { if r.status == c::OK { Ok(r) } else { Err(r.status) } }

/// The setting's current value.
pub fn get(key: u16) -> Result<i32, Error> { ok(call(c::GET, key, 0)?).map(|r| r.value) }
/// Changes a setting; Files checks the key, the range and that this program may.
/// `NOT_SAVED` means it holds until restart but could not be written.
pub fn set(key: u16, value: i32) -> Result<(), Error> { ok(call(c::SET, key, value as u32 as u64)?).map(|_| ()) }
/// The settings generation: it grows with every change anyone makes.
pub fn generation() -> Result<u64, Error> { ok(call(c::GENERATION, 0, 0)?).map(|r| r.value as u32 as u64) }
/// Every setting, in schema order, as Files holds it now.
pub fn all() -> Result<[i32; schema::COUNT], Error> {
    let mut out = [0; schema::COUNT];
    for (i, k) in KEYS.iter().enumerate() { out[i] = get(k.id)?; }
    Ok(out)
}
pub use sex_pdx::config::message;

fn secret_reply(m: &sex_pdx::PdxMessage) -> bool {
    m.type_id == 1 && m.caller_pd == FILES && sex_pdx::config::schema::secret::decode_reply(m.arg0).is_some()
}

/// Sends `password` to Files for `verb` (`secret::CHECK` or `secret::SET`) and
/// returns Files' status for it (`secret::OK`, `WRONG`, `NO_PASSWORD`, ...). Only
/// Settings may SET. The password never leaves Files once sent.
pub fn password(verb: u8, password: &[u8]) -> Result<u8, Error> {
    use sex_pdx::config::schema::secret as s;
    let mut sent = 0;
    for (a, b) in s::requests(password, verb) {
        loop {
            match sex_pdx::pdx_call_wait(sex_pdx::SLOT_STORAGE, s::OP, a, b, 0) {
                (0, _) => break,
                (sex_pdx::ERR_SERVICE_NOT_READY, _) => sex_pdx::sys_yield(),
                _ => return Err(UNAVAILABLE),
            }
        }
        sent += 1;
    }
    let give_up = sex_pdx::get_ticks().saturating_add(30_000);
    let mut last = UNAVAILABLE;
    while sent > 0 {
        let m = match mailbox::take(secret_reply).or_else(|| sex_pdx::pdx_try_listen_raw(0)) {
            Some(m) => m,
            None => {
                if sex_pdx::get_ticks() > give_up { return Err(UNAVAILABLE); }
                let _ = sex_pdx::sys_wait_self_ticks(2);
                continue;
            }
        };
        if secret_reply(&m) { last = s::decode_reply(m.arg0).unwrap_or(UNAVAILABLE); sent -= 1; continue; }
        if is_answer(&m) || m.caller_pd == super::SHELL { mailbox::put(m); }
    }
    Ok(last)
}
/// Whether a lock password is set.
pub fn password_set() -> Result<bool, Error> {
    use sex_pdx::config::schema::secret as s;
    password(s::STATE, b"").map(|status| status == s::OK)
}
