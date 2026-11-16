// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `reader.rs`, Whisper the screen reader (slice S7): starts the voice
//! program when the reader turns on and ends it when it turns off, sends it speech
//! from a poll (a full voice queue never stalls the Shell), looks at the desktop for
//! what to say, and keeps the accessibility trees programs send (`sex_pdx::a11y`).
//! Kept out of main.rs, which holds only hook lines.
//!
//! Serial record: `[shell.reader] on=0|1`, `[shell.reader.voice] state=.. target=..
//! task=..`, `[shell.reader.say] text=TEXT`, `[shell.reader.asked] pd=.. task=..`,
//! `[shell.reader.tree] pd=.. nodes=.. focus=..`.
use alloc::vec::Vec;
use sex_pdx::{a11y, app_catalog, program_store as store, speech_voice as voice};

use crate::reader::{self, Applied, Say, Snapshot, Speech, Tree, Watch};

/// The voice program's name in the program store.
const VOICE: &[u8] = b"whisper-voice";
/// How long a start may take before the reader asks again, and how many tries.
const START_TICKS: u64 = 15_000;
const START_TRIES: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Voice {
    /// Not running; `tries` starts failed in a row.
    Off { tries: u8 },
    /// Asked Files for the program at `since`.
    Starting { since: u64, tries: u8 },
    Running { target: u32, task: u32 },
}

static mut ON: bool = false;
static mut VOICE_STATE: Voice = Voice::Off { tries: 0 };
static mut SPEECH: Speech = Speech::new();
static mut WATCH: Watch = Watch::new();
/// Programs that asked whether a reader is on: (pd, task, what they were last told).
static mut ASKERS: [Option<(u32, u32, Option<bool>)>; 16] = [None; 16];
/// Trees of programs that send them, by (pd, task).
static mut TREES: Vec<(u32, u32, Tree)> = Vec::new();


/// Super+Alt+R: the reader on or off.
pub unsafe fn set(on: bool) {
    if ON == on { return; }
    ON = on;
    sex_pdx::serial_record!("[shell.reader] on={}", on as u8);
    if on {
        (*core::ptr::addr_of_mut!(WATCH)).reset();
        say(b"Screen reader on");
        if !matches!(VOICE_STATE, Voice::Running { .. } | Voice::Starting { .. }) { VOICE_STATE = Voice::Off { tries: 0 }; }
    } else {
        // Stop talking now; then the voice program ends (an ordinary Close).
        let speech = &mut *core::ptr::addr_of_mut!(SPEECH);
        speech.hush();
        send_speech();
        speech.clear();
        // A voice still starting is ended once it runs (`voice_started`).
        if let Voice::Running { target, task } = VOICE_STATE { set_off_voice(target, task); }
        if !matches!(VOICE_STATE, Voice::Starting { .. }) { VOICE_STATE = Voice::Off { tries: 0 }; }
        (*core::ptr::addr_of_mut!(TREES)).clear();
    }
}

/// Queues `text`, interrupting what is being said.
pub unsafe fn say(text: &[u8]) {
    if !ON || text.is_empty() { return; }
    sex_pdx::serial_record!("[shell.reader.say] text={}", core::str::from_utf8(text).unwrap_or("?"));
    (*core::ptr::addr_of_mut!(SPEECH)).say(text);
}

/// The Shell must keep running (not wait for a message) while this is true.
pub fn busy() -> bool {
    unsafe {
        if !ON { return false; }
        let speech = &*core::ptr::addr_of!(SPEECH);
        let tell = ASKERS.iter().flatten().any(|a| a.2 != Some(ON));
        (speech.pending() && matches!(VOICE_STATE, Voice::Running { .. })) || matches!(VOICE_STATE, Voice::Off { tries } if tries < START_TRIES) || tell
    }
}

/// Called often from the Shell's loop.
pub unsafe fn poll() {
    tell_askers();
    if !ON { return; }
    voice_poll();
    look();
    send_speech();
}

unsafe fn voice_poll() {
    match VOICE_STATE {
        Voice::Off { tries } if tries < START_TRIES => {
            if voice_start() { VOICE_STATE = Voice::Starting { since: sex_pdx::get_ticks(), tries }; }
        }
        Voice::Starting { since, tries } => {
            if sex_pdx::get_ticks().saturating_sub(since) > START_TICKS && !crate::PROGRAM_READING && crate::PROGRAM_REBIRTH.is_none() {
                sex_pdx::serial_record!("[shell.reader.voice] state=timeout tries={}", tries + 1);
                VOICE_STATE = Voice::Off { tries: tries + 1 };
            }
        }
        Voice::Running { target, task } => {
            if crate::program_run(target).is_none_or(|(_, t)| t != task) {
                sex_pdx::serial_record!("[shell.reader.voice] state=ended target={} task={}", target, task);
                VOICE_STATE = Voice::Off { tries: 0 };
            }
        }
        _ => {}
    }
}

/// Asks Files for the voice program, as `run whisper-voice` does. False if another run
/// is being read or started now; the poll tries again.
unsafe fn voice_start() -> bool {
    if crate::PROGRAM_READING || crate::PROGRAM_REBIRTH.is_some() || crate::RECORD_SPAWN_LEFT != 0 || crate::program_free_row().is_none() { return false; }
    let Some(name) = store::pack_name(VOICE) else { return false };
    let (status, _) = crate::pdx_call(store::SHELL_ROUTE_SLOT, store::OP, name[0], name[1], 0);
    if status != 0 { return false; }
    crate::PROGRAM_READING = true;
    crate::PROGRAM_NETWORK = false;
    crate::PROGRAM_LAUNCHER = None;
    crate::PROGRAM_TICKET = 0;
    crate::PROGRAM_SCENE = None;
    sex_pdx::serial_record!("[shell.reader.voice] state=starting");
    true
}

/// Hook in `launch_started`: true when the run that just started is the voice. It runs
/// hidden: no window on any workspace, nothing in the session.
pub unsafe fn voice_started(target: u32, task: u32) -> bool {
    if !matches!(VOICE_STATE, Voice::Starting { .. }) || crate::LAUNCH_ASKER.is_some() { return false; }
    VOICE_STATE = Voice::Running { target, task };
    if let Some(slot) = crate::HIDDEN_RUNS.iter_mut().find(|s| s.is_none()) { *slot = Some((target, task)); }
    sex_pdx::serial_record!("[shell.reader.voice] state=running target={} task={}", target, task);
    // Turned off while it started: end it again.
    if !ON { set_off_voice(target, task); }
    true
}

unsafe fn set_off_voice(target: u32, task: u32) {
    if let Some(e) = crate::APP_CUSTODY.iter_mut().flatten().find(|e| e.target == target && e.task == task) { e.request_close(); }
    VOICE_STATE = Voice::Off { tries: 0 };
}

/// Hook in `program_refused`: true when the refused run was the voice (Spindle did not
/// ask for it, so it is not told).
pub unsafe fn voice_refused(code: u32) -> bool {
    let Voice::Starting { tries, .. } = VOICE_STATE else { return false };
    if crate::LAUNCH_ASKER.is_some() { return false; }
    VOICE_STATE = Voice::Off { tries: tries + 1 };
    sex_pdx::serial_record!("[shell.reader.voice] state=refused code={} tries={}", code, tries + 1);
    if tries + 1 >= START_TRIES { crate::notify_panel(b"Screen reader: the voice did not start"); }
    true
}

unsafe fn send_speech() {
    let Voice::Running { target, .. } = VOICE_STATE else { return };
    let Some(row) = app_catalog::by_target(target) else { return };
    let speech = &mut *core::ptr::addr_of_mut!(SPEECH);
    // A few pieces a pass: the rest go on the next.
    for _ in 0..8 {
        let Some([a, b, c]) = speech.front() else { return };
        let (status, _) = crate::pdx_call(row.route_slot, voice::OP_SPEAK, a, b, c);
        if status == sex_pdx::ERR_SERVICE_NOT_READY { return; }
        if status != 0 { speech.clear(); return; }
        speech.sent();
    }
}

/// The name a window shows in its title bar (Display's `surface_title`).
fn title(sid: u64, buf: &mut [u8; 16]) -> Option<usize> {
    if sid == 200 { return Some(reader::join(&[b"Linen"], buf)); }
    let row = app_catalog::INCARNATIONS.iter().find(|r| r.surface == sid || r.shell_surface == Some(sid))?;
    let name: &[u8] = match row.app {
        app_catalog::App::Quil => b"Quil",
        app_catalog::App::Spindle => b"Spindle",
        app_catalog::App::Browser => b"Browser",
        app_catalog::App::Program => {
            let own = sex_pdx::system_stats::name(row.target);
            let n = own.iter().position(|&c| c == 0).unwrap_or(16);
            if n > 0 && own[..n].iter().all(|b| (32..127).contains(b)) { buf[..n].copy_from_slice(&own[..n]); return Some(n); }
            b"Program"
        }
    };
    Some(reader::join(&[name], buf))
}

/// The program row whose window `sid` is.
fn owner(sid: u64) -> Option<u32> {
    app_catalog::INCARNATIONS.iter().find(|r| r.surface == sid || r.shell_surface == Some(sid)).map(|r| r.target)
}

unsafe fn look() {
    let focus = crate::FOCUSED_SURFACE_ID;
    let mut tb = [0u8; 16];
    let title_len = title(focus, &mut tb);
    let launcher = if crate::COMMAND_PALETTE_OPEN {
        let i = (crate::COMMAND_PALETTE_SELECTED as usize).min(crate::COMMAND_LIST.len() - 1);
        let item = &crate::COMMAND_LIST[i];
        Some((i as u8, crate::launcher_row_title(item.command, item.name).as_bytes()))
    } else { None };
    let snapshot = Snapshot { focus, title: title_len.map(|n| &tb[..n]), launcher, notices: crate::NOTIFY_TOTAL };
    let mut out = [None, None];
    (*core::ptr::addr_of_mut!(WATCH)).look(&snapshot, &mut out);
    let mut text = [0u8; voice::MAX_TEXT];
    for s in out.into_iter().flatten() {
        let n = match s {
            Say::Title(t) | Say::Launcher(t) => reader::join(&[t], &mut text),
            Say::Notice(_) => {
                let ring = &*core::ptr::addr_of!(crate::NOTIFY_RING);
                let newest = &ring[(crate::NOTIFY_TOTAL + crate::NOTIFY_HISTORY - 1) % crate::NOTIFY_HISTORY];
                reader::notice_text(newest.summary(), newest.body(), &mut text)
            }
        };
        say(&text[..n]);
    }
}

/// Tells each program that asked whether the reader is on, when it changed.
unsafe fn tell_askers() {
    let askers = &mut *core::ptr::addr_of_mut!(ASKERS);
    for slot in askers.iter_mut() {
        let Some((pd, task, told)) = *slot else { continue };
        if crate::running_task(pd) != Some(task) { *slot = None; continue; }
        if told == Some(ON) { continue; }
        let Some(row) = app_catalog::by_target(pd) else { *slot = None; continue };
        let [a, b, c] = a11y::state(ON);
        let (status, _) = crate::pdx_call(row.route_slot, a11y::OP_A11Y, a, b, c);
        if status == sex_pdx::ERR_SERVICE_NOT_READY { continue; }
        *slot = Some((pd, task, Some(ON)));
    }
}

/// A program's `OP_A11Y`. True when it was one.
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != a11y::OP_A11Y { return false; }
    let (pd, task) = (msg.caller_pd, msg.caller_task);
    if app_catalog::by_target(pd).is_none() || crate::running_task(pd) != Some(task) { return true; }
    let askers = &mut *core::ptr::addr_of_mut!(ASKERS);
    match a11y::kind(msg.arg0) {
        a11y::ASK => {
            // The program hears the answer from the poll, now and on each change.
            match askers.iter().position(|a| a.is_some_and(|a| a.0 == pd)) {
                Some(i) => askers[i] = Some((pd, task, None)),
                None => if let Some(s) = askers.iter_mut().find(|s| s.is_none()) { *s = Some((pd, task, None)); },
            }
            sex_pdx::serial_record!("[shell.reader.asked] pd={} task={} on={}", pd, task, ON as u8);
        }
        a11y::TREE => {
            // Only from a program told that a reader is on.
            if !ON || !askers.iter().flatten().any(|a| a.0 == pd && a.1 == task && a.2 == Some(true)) { return true; }
            let Some(piece) = a11y::Piece::decode(msg.arg0, msg.arg1, msg.arg2) else { return true };
            let trees = &mut *core::ptr::addr_of_mut!(TREES);
            trees.retain(|t| crate::running_task(t.0) == Some(t.1));
            let i = match trees.iter().position(|t| t.0 == pd && t.1 == task) {
                Some(i) => i,
                None => { if trees.len() >= 16 { return true; } trees.push((pd, task, Tree::new())); trees.len() - 1 }
            };
            let tree = &mut trees[i].2;
            let Some(applied) = tree.piece(&piece) else { return true };
            sex_pdx::serial_record!("[shell.reader.tree] pd={} applied={:?} nodes={} focus={}", pd, applied, tree.len(), tree.focus());
            if applied == Applied::FocusMoved && owner(crate::FOCUSED_SURFACE_ID) == Some(pd) {
                let mut text = [0u8; 256];
                let n = tree.focused_text(&mut text);
                say(&text[..n]);
            }
        }
        _ => {}
    }
    true
}
