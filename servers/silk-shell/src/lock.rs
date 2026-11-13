// SPDX-License-Identifier: GPL-3.0-only
//! The session lock and the idle policy (workstream B6/B7).
//!
//! Super+Esc locks. While locked, every key, click and pointer move stays here: no
//! program sees input, and Display paints only the lock surface. Typed characters go
//! to a buffer this module wipes after each try; Enter sends them to Files, which
//! answers whether they match the stored Argon2id hash (`sex_config::secret`). With no
//! password set, Enter unlocks.
//!
//! Idle: with no input for the times in settings (`session.*`), the screen dims, then
//! locks (if `session.lock_on_idle`), then goes off. Any input brings it back; the
//! input that wakes a dimmed or dark screen does nothing else.
use sex_config::{id, secret as p};
use sex_pdx::screen_state::{self as s, Lock};

const FILES_PD: u32 = 11;
const MINUTE: u64 = 60_000;
const ENTER: u8 = 0x1C;
const ENTER_PAD: u8 = 0x9C;
const BACKSPACE: u8 = 0x0E;
const ESC: u8 = 0x01;
const SHIFT_L: u8 = 0x2A;
const SHIFT_R: u8 = 0x36;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Asked { Nothing, State, Check { left: u8 } }

struct Session {
    lock: Lock,
    typed: [u8; p::MAX],
    asked: Asked,
    shift: bool,
    mode: u8,
    last_input: u64,
    /// (mode, lock word) Display last took; None until it took one.
    shown: Option<(u8, u64)>,
}

static mut SESSION: Session = Session {
    lock: Lock { locked: false, typed: 0, say: s::SAY_PROMPT },
    typed: [0; p::MAX], asked: Asked::Nothing, shift: false, mode: s::NORMAL, last_input: 0, shown: None,
};
fn session() -> &'static mut Session { unsafe { &mut *core::ptr::addr_of_mut!(SESSION) } }

pub fn locked() -> bool { session().lock.locked }

fn show(se: &mut Session) {
    let want = (se.mode, se.lock.word());
    if se.shown == Some(want) { return; }
    let ok = sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY, s::OP_SCREEN_STATE, want.0 as u64, want.1, 0).0 == 0;
    if ok { se.shown = Some(want); }
    sex_pdx::serial_record!("[shell.screen] mode={} locked={} typed={} say={} ok={}", se.mode, se.lock.locked as u8, se.lock.typed, se.lock.say, ok as u8);
}

fn wipe(se: &mut Session) {
    for b in se.typed.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) }; }
    se.lock.typed = 0;
}

fn files(arg0: u64, arg1: u64) -> bool {
    sex_pdx::pdx_call(sex_pdx::program_store::SHELL_ROUTE_SLOT, p::OP, arg0, arg1, 0).0 == 0
}

/// Locks the session now (Super+Esc, or idle).
pub fn lock() -> bool {
    let se = session();
    if !se.lock.locked {
        se.lock = Lock { locked: true, typed: 0, say: s::SAY_PROMPT };
        wipe(se);
        // Ask whether a password is set, to say so on the lock surface.
        if se.asked == Asked::Nothing && files(sex_config::wire::request(p::STATE, 0), 0) { se.asked = Asked::State; }
        sex_pdx::serial_record!("[shell.lock] locked=1");
    }
    show(se);
    true
}

fn unlock(se: &mut Session) {
    se.lock = Lock { locked: false, typed: 0, say: s::SAY_PROMPT };
    wipe(se);
    se.last_input = sex_pdx::get_ticks();
    sex_pdx::serial_record!("[shell.lock] locked=0");
    show(se);
}

/// Files' answer to a password request; false for any other message.
pub fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != 1 || msg.caller_pd != FILES_PD { return false; }
    let Some(status) = p::decode_reply(msg.arg0) else { return false };
    let se = session();
    match se.asked {
        Asked::State => {
            se.asked = Asked::Nothing;
            if se.lock.locked && status == p::NO_PASSWORD { se.lock.say = s::SAY_NO_PASSWORD; show(se); }
        }
        Asked::Check { left } if left > 1 => se.asked = Asked::Check { left: left - 1 },
        Asked::Check { .. } => {
            se.asked = Asked::Nothing;
            sex_pdx::serial_record!("[shell.lock.check] status={}", status);
            match status {
                p::OK | p::NO_PASSWORD => unlock(se),
                _ => { se.lock.say = s::SAY_WRONG; show(se); }
            }
        }
        Asked::Nothing => {}
    }
    true
}

/// The ASCII a set-1 key types, if any.
fn typed(code: u8, shift: bool) -> Option<u8> {
    let plain: &[u8; 58] = b"\0\x001234567890-=\0\0qwertyuiop[]\0\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
    let upper: &[u8; 58] = b"\0\0!@#$%^&*()_+\0\0QWERTYUIOP{}\0\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";
    let c = *if shift { upper } else { plain }.get(code as usize)?;
    (c != 0).then_some(c)
}

fn key(se: &mut Session, code: u8, value: u64) {
    if matches!(code, SHIFT_L | SHIFT_R) { se.shift = value != 0; return; }
    // Desktop chords must see releases, or Super would stay held after unlocking.
    if value == 0 { unsafe { let _ = (*core::ptr::addr_of_mut!(crate::WM_CHORDS)).event(code, 0); } return; }
    if se.asked != Asked::Nothing && !matches!(se.asked, Asked::State) { return; }
    match code {
        ENTER | ENTER_PAD => {
            let n = se.lock.typed as usize;
            let mut sent = 0u8;
            let mut all = true;
            for (a, b) in p::requests(&se.typed[..n], p::CHECK) { all &= files(a, b); sent += 1; }
            wipe(se);
            if all { se.asked = Asked::Check { left: sent }; se.lock.say = s::SAY_CHECKING; } else { se.lock.say = s::SAY_WRONG; }
        }
        BACKSPACE => { if se.lock.typed > 0 { se.lock.typed -= 1; se.typed[se.lock.typed as usize] = 0; } }
        ESC => wipe(se),
        _ => if let Some(c) = typed(code, se.shift) {
            if (se.lock.typed as usize) < p::MAX { se.typed[se.lock.typed as usize] = c; se.lock.typed += 1; }
            if se.lock.say == s::SAY_WRONG { se.lock.say = s::SAY_PROMPT; }
        },
    }
    show(se);
}

/// Every HID event passes here first. True when it is the session's (locked, or the
/// key or move that wakes a dimmed screen) and must go no further.
pub fn input(class: u64, code: u8, value: u64) -> bool {
    let se = session();
    se.last_input = sex_pdx::get_ticks();
    let woke = se.mode != s::NORMAL;
    if woke {
        se.mode = s::NORMAL;
        sex_pdx::serial_record!("[shell.idle] state=awake");
        show(se);
    }
    if se.lock.locked {
        if class == sex_pdx::EV_KEY { key(se, code, value); }
        return true;
    }
    // A key release after waking still goes on, so nothing stays held.
    woke && !(class == sex_pdx::EV_KEY && value == 0)
}

fn minutes(key: u16) -> u64 { crate::config_watch::value(key).max(0) as u64 * MINUTE }

/// Steps the idle policy. Called whenever the Shell wakes.
pub fn poll() {
    let se = session();
    let now = sex_pdx::get_ticks();
    if se.last_input == 0 { se.last_input = now; }
    let idle = now.saturating_sub(se.last_input);
    let (dim, lock_at, off) = (minutes(id::DIM_AFTER), minutes(id::LOCK_AFTER), minutes(id::BLANK_AFTER));
    if lock_at != 0 && idle >= lock_at && !se.lock.locked && crate::config_watch::value(id::LOCK_ON_IDLE) != 0 { lock(); }
    let mode = if off != 0 && idle >= off { s::OFF } else if dim != 0 && idle >= dim { s::DIM } else { se.mode };
    if mode != se.mode {
        se.mode = mode;
        sex_pdx::serial_record!("[shell.idle] state={} idle_ms={}", if mode == s::OFF { "off" } else { "dim" }, idle);
    }
    // Display may have refused or restarted: keep offering what it should show.
    if se.shown != Some((se.mode, se.lock.word())) { show(se); }
}

/// Ticks until the idle policy next has something to do, for the Shell's wait.
pub fn next_wake() -> Option<u64> {
    let se = session();
    let idle = sex_pdx::get_ticks().saturating_sub(se.last_input);
    [minutes(id::DIM_AFTER), minutes(id::LOCK_AFTER), minutes(id::BLANK_AFTER)].iter()
        .filter(|t| **t != 0 && **t > idle).map(|t| t - idle).min()
}

/// Display started again: show it the lock and screen state.
pub fn display_restarted() { session().shown = None; }
