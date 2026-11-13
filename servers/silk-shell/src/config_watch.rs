// SPDX-License-Identifier: GPL-3.0-only
//! The Shell's view of the settings Files keeps (`sex_pdx::config`).
//!
//! Whenever the Shell is awake and has not asked for a while, it asks Files for the
//! settings generation; when it grew, it walks the changed keys, keeps them in its
//! own copy and applies the ones it owns (wallpaper and night light so far).
//! Changes the person makes with Shell keys go the other way, to Files, so they
//! last across restarts. Answers come back as tagged replies, one request in
//! flight at a time.
use sex_pdx::config::{self as c, schema::{id, Config}};

/// Files' domain: its replies arrive as `type_id` 1 from here.
const FILES_PD: u32 = 11;
/// Ask at most this often (ms ticks) when nothing is waiting.
const POLL_TICKS: u64 = 500;
/// Changes from Shell keys waiting to be sent.
const QUEUE: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Asked { Nothing, Generation, Next, Set }

struct Watch {
    copy: Config,
    /// The generation whose changes `copy` holds (None: none walked yet).
    seen: Option<u64>,
    /// The generation Files reported, being walked toward.
    target: u64,
    asked: Asked,
    asked_at: u64,
    last_poll: u64,
    queue: [(u16, i32); QUEUE],
    queued: usize,
    /// The look from settings still has to reach Display.
    look_owed: bool,
}

static mut WATCH: Watch = Watch {
    copy: Config::new(), seen: None, target: 0, asked: Asked::Nothing, asked_at: 0, last_poll: 0,
    queue: [(0, 0); QUEUE], queued: 0, look_owed: false,
};

fn watch() -> &'static mut Watch { unsafe { &mut *core::ptr::addr_of_mut!(WATCH) } }

/// The setting's value as the Shell last heard it (its default before that).
pub fn value(key: u16) -> i32 { watch().copy.value(key) }

fn ask(w: &mut Watch, asked: Asked, verb: u8, key: u16, arg1: u64) {
    let (status, _) = sex_pdx::pdx_call(sex_pdx::program_store::SHELL_ROUTE_SLOT, c::OP, c::request(verb, key), arg1, 0);
    if status == 0 { w.asked = asked; w.asked_at = sex_pdx::get_ticks(); }
}

/// Called each time the Shell wakes. Sends at most one request.
pub fn poll() {
    let w = watch();
    let now = sex_pdx::get_ticks();
    if w.look_owed { show_look(w); }
    // Files restarted or lost the request: ask again after a while.
    if w.asked != Asked::Nothing && now.saturating_sub(w.asked_at) > 5 * POLL_TICKS { w.asked = Asked::Nothing; }
    if w.asked != Asked::Nothing { return; }
    if w.queued > 0 {
        let (key, value) = w.queue[0];
        ask(w, Asked::Set, c::SET, key, value as u32 as u64);
        return;
    }
    if now.saturating_sub(w.last_poll) < POLL_TICKS { return; }
    w.last_poll = now;
    ask(w, Asked::Generation, c::GENERATION, 0, 0);
}

/// Takes Files' answer to a settings request; false for any other message.
pub fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != 1 || msg.caller_pd != FILES_PD { return false; }
    let Some(r) = c::Reply::decode(msg.arg0) else { return false };
    let w = watch();
    match core::mem::replace(&mut w.asked, Asked::Nothing) {
        Asked::Generation if r.status == c::OK => {
            let generation = r.value as u32 as u64;
            // A smaller generation means Files started again: walk everything.
            let since = match w.seen { Some(s) if s <= generation => s, _ => 0 };
            if w.seen != Some(generation) {
                w.target = generation;
                w.seen = Some(since);
                ask(w, Asked::Next, c::NEXT, 0, since);
            }
        }
        Asked::Next => {
            if r.status == c::OK {
                take(w, r.key, r.value);
                ask(w, Asked::Next, c::NEXT, r.key, w.seen.unwrap_or(0));
            } else {
                w.seen = Some(w.target);
                sex_pdx::serial_record!("[shell.config.current] generation={}", w.target);
            }
        }
        Asked::Set => {
            sex_pdx::serial_record!("[shell.config.saved] key={} value={} status={}", r.key, r.value, r.status);
            if w.queued > 0 { w.queue.copy_within(1..w.queued, 0); w.queued -= 1; }
            if r.status == c::OK { let _ = w.copy.set(r.key, r.value); }
            // The change came from here; the next poll picks up its generation.
            w.last_poll = 0;
        }
        _ => {}
    }
    true
}

/// One changed key from Files: keep it, and apply it if the Shell owns it.
fn take(w: &mut Watch, key: u16, value: i32) {
    // A change made here and not yet saved wins over the older value Files still has.
    if w.queue[..w.queued].iter().any(|(k, _)| *k == key) { return; }
    let before = w.copy.value(key);
    if w.copy.set(key, value).is_err() { return; }
    sex_pdx::serial_record!("[shell.config.changed] key={} value={}", key, value);
    if before == value { return; }
    match key {
        id::WALLPAPER | id::NIGHT_LIGHT => show_look(w),
        _ => {}
    }
}

fn show_look(w: &mut Watch) {
    w.look_owed = !unsafe { crate::desktop_look_apply(w.copy.value(id::WALLPAPER) as u8, w.copy.value(id::NIGHT_LIGHT) as u8) };
}

/// Display started again and knows nothing: show the look from settings again.
pub fn display_restarted() { watch().look_owed = true; }

/// The person changed a setting with a Shell key: tell Files, so it lasts.
pub fn changed_here(key: u16, value: i32) {
    let w = watch();
    if w.copy.value(key) == value && w.seen.is_some() { return; }
    let _ = w.copy.set(key, value);
    if let Some(slot) = w.queue[..w.queued].iter_mut().find(|(k, _)| *k == key) { slot.1 = value; return; }
    if w.queued < QUEUE { w.queue[w.queued] = (key, value); w.queued += 1; }
    poll();
}
