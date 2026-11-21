// SPDX-License-Identifier: GPL-3.0-only
//! The Shell's view of the settings Files keeps (`sex_pdx::config`).
//!
//! Whenever the Shell is awake and has not asked for a while, it asks Files for the
//! settings generation; when it grew, it walks the changed keys, keeps them in its
//! own copy and applies the ones it owns (wallpaper, night light, theme, clock zone, volume).
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
enum Asked { Nothing, Generation, Next, Set, Program }
/// Program rows whose program the Shell still has to ask Files about.
const ROWS: usize = 8;

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
    /// The bar clock's time zone still has to reach Display.
    zone_owed: bool,
    /// The volume from settings still has to reach Bell.
    volume_owed: bool,
    /// Network time on/off still has to reach sexnet.
    time_owed: bool,
    /// A whole walk has finished: `copy` holds the person's settings, not defaults.
    walked: bool,
    /// Rows just bound to a run, to ask Files which program it is (`c::PROGRAM`).
    rows: [u32; ROWS],
    row_count: usize,
}

static mut WATCH: Watch = Watch {
    copy: Config::new(), seen: None, target: 0, asked: Asked::Nothing, asked_at: 0, last_poll: 0,
    queue: [(0, 0); QUEUE], queued: 0, look_owed: false, zone_owed: false, volume_owed: false, time_owed: false,
    walked: false, rows: [0; ROWS], row_count: 0,
};

fn watch() -> &'static mut Watch { unsafe { &mut *core::ptr::addr_of_mut!(WATCH) } }

/// The setting's value as the Shell last heard it (its default before that).
pub fn value(key: u16) -> i32 { watch().copy.value(key) }
/// True once the Shell holds the person's settings (the first walk ended).
pub fn current() -> bool { watch().walked }
/// True when the set `key` holds item `index`.
pub fn holds(key: u16, index: usize) -> bool { watch().copy.holds(key, index) }
/// A program row now runs something new: ask Files which program (`program_notify`).
pub fn ask_program(row: u32) {
    let w = watch();
    if w.rows[..w.row_count].contains(&row) { return; }
    if w.row_count < ROWS { w.rows[w.row_count] = row; w.row_count += 1; }
    poll();
}

fn ask(w: &mut Watch, asked: Asked, verb: u8, key: u16, arg1: u64) {
    let (status, _) = sex_pdx::pdx_call(sex_pdx::program_store::SHELL_ROUTE_SLOT, c::OP, c::request(verb, key), arg1, 0);
    if status == 0 { w.asked = asked; w.asked_at = sex_pdx::get_ticks(); }
}

/// Called each time the Shell wakes. Sends at most one request.
pub fn poll() {
    let w = watch();
    let now = sex_pdx::get_ticks();
    if w.look_owed { show_look(w); }
    if w.zone_owed { show_zone(w); }
    if w.volume_owed { give_volume(w); }
    if w.time_owed { give_network_time(w); }
    // Files restarted or lost the request: ask again after a while.
    if w.asked != Asked::Nothing && now.saturating_sub(w.asked_at) > 5 * POLL_TICKS { w.asked = Asked::Nothing; }
    if w.asked != Asked::Nothing { return; }
    // A lock-screen password check is waiting: its reply must not be overwritten.
    if crate::lock::busy() { return; }
    if w.queued > 0 {
        let (key, value) = w.queue[0];
        ask(w, Asked::Set, c::SET, key, value as u32 as u64);
        return;
    }
    if w.row_count > 0 {
        let row = w.rows[0];
        ask(w, Asked::Program, c::PROGRAM, row as u16, 0);
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
                // A walk from the start (the first, or Files restarted): the keyboard
                // settings hold even where none changed from their defaults.
                if w.seen == Some(0) { keyboard(w); }
                // The first walk: Display may hold another theme than the
                // person's (the Shell restarted), so show the look once.
                if !w.walked { w.look_owed = true; }
                w.seen = Some(w.target);
                w.walked = true;
                sex_pdx::serial_record!("[shell.config.current] generation={}", w.target);
            }
        }
        Asked::Program => {
            if w.row_count > 0 && w.rows[0] as u16 == r.key { w.rows.copy_within(1..w.row_count, 0); w.row_count -= 1; }
            crate::program_notify::learned(r.key as u32, if r.status == c::OK { r.value } else { -1 });
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
        // The theme rides with the look (`theme_glue`).
        id::WALLPAPER | id::NIGHT_LIGHT | id::THEME | id::ACCENT => show_look(w),
        id::ZONE => show_zone(w),
        id::VOLUME => give_volume(w),
        id::NTP => give_network_time(w),
        id::REPEAT_DELAY | id::REPEAT_RATE | id::CAPS_AS_CTRL => keyboard(w),
        id::WIDGETS => unsafe { crate::widgets_glue::set(value != 0) },
        id::WEATHER => unsafe { crate::widgets_glue::weather(value != 0) },
        id::MAGNIFIER_ZOOM | id::COLOUR_FILTER | id::STICKY_KEYS | id::SLOW_KEYS | id::BOUNCE_KEYS | id::SCREEN_READER => unsafe { crate::access_glue::setting(key, value) },
        _ => {}
    }
}

fn show_look(w: &mut Watch) {
    w.look_owed = !unsafe { crate::desktop_look_apply(w.copy.value(id::WALLPAPER) as u8, w.copy.value(id::NIGHT_LIGHT) as u8) };
}

fn show_zone(w: &mut Watch) {
    let zone = w.copy.value(id::ZONE) as u64;
    w.zone_owed = sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY, sex_pdx::clock_zone::OP_CLOCK_ZONE, zone, 0, 0).0 != 0;
    sex_pdx::serial_record!("[shell.clock.zone] zone={} ok={}", zone, !w.zone_owed as u8);
    unsafe { crate::weather_glue::zone(zone); }
}

fn keyboard(w: &mut Watch) {
    let v = |k| w.copy.value(k).max(0) as u64;
    unsafe { crate::keyboard_settings(v(id::REPEAT_DELAY), v(id::REPEAT_RATE), v(id::CAPS_AS_CTRL) != 0) };
}

/// sexnet sets the clock from the network unless the person turned it off.
fn give_network_time(w: &mut Watch) {
    let on = w.copy.value(id::NTP) != 0;
    // sexnet answers in Shell's one reply slot: wait while Bell or sexnet still owe one.
    if unsafe { crate::answer_owed() } { w.time_owed = true; return; }
    let r = sex_pdx::pdx_call(sex_pdx::SLOT_NET, sex_pdx::network_time::OP, sex_pdx::network_time::SET_ENABLED, on as u64, 0).0;
    w.time_owed = r != 0;
    if r == 0 { unsafe { crate::NET_TIME_ASKED = Some(sex_pdx::get_ticks()); } }
    sex_pdx::serial_record!("[shell.network_time] on={} ok={}", on as u8, (r == 0) as u8);
}

fn give_volume(w: &mut Watch) {
    let volume = w.copy.value(id::VOLUME).clamp(0, 100) as u8;
    w.volume_owed = !unsafe { crate::volume_restore(volume) };
    sex_pdx::serial_record!("[shell.volume.restore] volume={} ok={}", volume, !w.volume_owed as u8);
}

/// Display started again and knows nothing: show the look and the clock's zone again.
pub fn display_restarted() { let w = watch(); w.look_owed = true; w.zone_owed = w.copy.value(id::ZONE) != 0; crate::theme_glue::display_restarted(); }

/// The person changed a setting with a Shell key: tell Files, so it lasts.
pub fn changed_here(key: u16, value: i32) {
    let w = watch();
    if w.copy.value(key) == value && w.seen.is_some() { return; }
    let _ = w.copy.set(key, value);
    if let Some(slot) = w.queue[..w.queued].iter_mut().find(|(k, _)| *k == key) { slot.1 = value; return; }
    if w.queued < QUEUE { w.queue[w.queued] = (key, value); w.queued += 1; }
    poll();
}
