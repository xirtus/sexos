// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Settings: Files keeps the one live table and its text on disk.
//!
//! The table is read from `/system/settings.toml` at the first request and written
//! back whole on every change. The protocol and the rules are in `sex_pdx::config`;
//! who may change settings is decided here from the kernel-named caller.
use sex_pdx::config::{self as c, schema::{Config, TEXT_MAX}};

pub const PATH: &[u8] = b"/system/settings.toml";
/// The Shell's domain (it calls over its program route).
const SHELL: u32 = 3;

static mut TABLE: Option<Config> = None;

fn table() -> &'static mut Config {
    let table = unsafe { &mut *core::ptr::addr_of_mut!(TABLE) };
    table.get_or_insert_with(|| {
        let mut config = Config::new();
        let mut text = [0u8; TEXT_MAX];
        match crate::fs_service::read_own(PATH, &mut text) {
            Some(n) => {
                let r = config.read(&text[..n]);
                sex_pdx::serial_record!("[files.config.loaded] bytes={} taken={} unknown={} invalid={}", n, r.taken, r.unknown, r.invalid);
            }
            None => sex_pdx::serial_record!("[files.config.defaults]"),
        }
        config
    })
}

fn writer(caller: u32) -> bool {
    caller == SHELL
        || sex_pdx::app_catalog::SPINDLE_TARGETS.contains(&caller) && crate::spindle_history::live(caller)
        || crate::fs_service::program_is(caller, b"tatami")
}

/// One request; the word to reply with.
pub fn handle(caller: u32, arg0: u64, arg1: u64) -> u64 {
    // The Shell asks which listed program a row runs (`c::PROGRAM`), for per-program settings.
    if let Some((c::PROGRAM, row)) = c::decode_request(arg0) {
        if caller != SHELL { return c::Reply { status: c::DENIED, key: row, value: 0 }.word(); }
        let index = crate::fs_service::program_name(row as u32, sex_pdx::config::schema::program_index).flatten().map_or(-1, |i| i as i32);
        sex_pdx::serial_record!("[files.config.program] row={} program={}", row, index);
        return c::Reply { status: c::OK, key: row, value: index }.word();
    }
    let reply = c::serve(table(), writer(caller), arg0, arg1, |config| {
        let mut text = [0u8; TEXT_MAX];
        config.write(&mut text).is_some_and(|n| crate::fs_service::replace_own(PATH, &text[..n]))
    });
    if c::decode_request(arg0).is_some_and(|(verb, _)| verb == c::SET) {
        sex_pdx::serial_record!("[files.config.set] caller={} key={} value={} status={}", caller, reply.key, reply.value, reply.status);
    }
    reply.word()
}

/// One value, for Files' own use, without loading the table: until someone has asked
/// for settings (the Shell does at start) the key's default stands. Loading here could
/// re-enter the filesystem in the middle of a request.
pub fn value(id: u16) -> i32 {
    match unsafe { &*core::ptr::addr_of!(TABLE) } {
        Some(t) => t.value(id),
        None => sex_pdx::config::schema::key(id).map_or(0, |k| k.default),
    }
}
