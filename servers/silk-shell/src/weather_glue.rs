// SPDX-License-Identifier: GPL-3.0-only
//! The weather card (slice S5c). While the desktop widgets show and the
//! `desktop.weather` setting is on, the Shell runs `steamy`, a hidden helper
//! with the network right that asks Open-Meteo about the time zone's city
//! (`sex_pdx::desktop_widgets::PLACES`). The Shell tells it the zone
//! (`OP_WEATHER_PLACE`), takes its reports (`OP_WEATHER`) from that run only,
//! and passes them on to Display. A zone with no city (UTC) needs no helper:
//! the Shell tells Display so itself. When the cards go, the helper ends.
//!
//! Serial record: `[shell.weather] state=starting|running|refused|timeout|
//! ended|stopped ...`, `[shell.weather.place] zone=.. ok=..`,
//! `[shell.weather.report] zone=.. report=.. ok=..`.
use sex_pdx::{app_catalog, desktop_widgets as dw, program_store as store};

/// The helper's name in the program store.
const HELPER: &[u8] = b"steamy";
const START_TICKS: u64 = 15_000;
const START_TRIES: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Helper {
    Off { tries: u8 },
    Starting { since: u64, tries: u8 },
    Running { target: u32, task: u32 },
}

static mut HELPER_STATE: Helper = Helper::Off { tries: 0 };
/// The cards show (the Shell's widgets are on).
static mut CARDS: bool = false;
/// `desktop.weather` (default on).
static mut SETTING: bool = true;
static mut ZONE: u64 = 0;
/// The zone the running helper was told about.
static mut TOLD: Option<u64> = None;
/// The latest report and its zone, and whether Display still lacks it.
static mut LAST: Option<(u64, u64)> = None;
static mut DISPLAY_OWED: bool = false;

/// Whether the weather card shows with the other cards.
pub fn on() -> bool { unsafe { SETTING } }

/// The cards came or went. Coming again, a helper that would not start gets new tries.
pub unsafe fn cards(on: bool) {
    CARDS = on;
    if on && matches!(HELPER_STATE, Helper::Off { .. }) { HELPER_STATE = Helper::Off { tries: 0 }; }
}

/// `desktop.weather` changed.
pub unsafe fn setting(on: bool) { SETTING = on; }

/// `time.zone` changed.
pub unsafe fn zone(z: u64) {
    if ZONE == z { return; }
    ZONE = z;
    TOLD = None;
}

fn zone_name(z: u64) -> &'static str { sex_config::ZONES.get(z as usize).copied().unwrap_or("UTC") }

fn wanted() -> bool { unsafe { CARDS && SETTING && dw::place(zone_name(ZONE)).is_some() } }

/// Display started again: it has to be told the weather again.
pub fn display_restarted() { unsafe { DISPLAY_OWED = LAST.is_some(); } }

/// Main loop.
pub unsafe fn poll() {
    if CARDS && SETTING && !wanted() && LAST != Some((dw::Report::Unavailable(dw::NO_PLACE).pack(), ZONE)) {
        // No city for this zone: nothing to ask, and the card says so.
        LAST = Some((dw::Report::Unavailable(dw::NO_PLACE).pack(), ZONE));
        DISPLAY_OWED = true;
    }
    match HELPER_STATE {
        Helper::Off { tries } if tries < START_TRIES && wanted() => {
            if start() { HELPER_STATE = Helper::Starting { since: sex_pdx::get_ticks(), tries }; }
        }
        Helper::Starting { since, tries } => {
            if sex_pdx::get_ticks().saturating_sub(since) > START_TICKS && !crate::PROGRAM_READING && crate::PROGRAM_REBIRTH.is_none() {
                sex_pdx::serial_record!("[shell.weather] state=timeout tries={}", tries + 1);
                HELPER_STATE = Helper::Off { tries: tries + 1 };
            }
        }
        Helper::Running { target, task } => {
            if crate::program_run(target).is_none_or(|(_, t)| t != task) {
                sex_pdx::serial_record!("[shell.weather] state=ended target={} task={}", target, task);
                HELPER_STATE = Helper::Off { tries: 0 };
                TOLD = None;
            } else if !wanted() {
                stop(target, task);
            } else if TOLD != Some(ZONE) {
                tell_place(target);
            }
        }
        _ => {}
    }
    if DISPLAY_OWED { tell_display(); }
}

/// How long the idle Shell may sleep while a start, a place or Display's report is owed.
pub unsafe fn wake() -> Option<u64> {
    match HELPER_STATE {
        Helper::Starting { .. } => Some(250),
        Helper::Running { .. } if TOLD != Some(ZONE) || !wanted() => Some(50),
        Helper::Off { tries } if tries < START_TRIES && wanted() => Some(250),
        _ if DISPLAY_OWED => Some(250),
        _ => None,
    }
}

/// Asks Files for the helper with the network right, as `run +net steamy` does.
/// False while another run is being read or started; the poll tries again.
unsafe fn start() -> bool {
    if crate::PROGRAM_READING || crate::PROGRAM_REBIRTH.is_some() || crate::RECORD_SPAWN_LEFT != 0 || crate::program_free_row().is_none() { return false; }
    let Some(name) = store::pack_name(HELPER) else { return false };
    let (status, _) = crate::pdx_call(store::SHELL_ROUTE_SLOT, store::OP, name[0], name[1], 0);
    if status != 0 { return false; }
    crate::PROGRAM_READING = true;
    crate::PROGRAM_NETWORK = true;
    crate::PROGRAM_LAUNCHER = None;
    crate::PROGRAM_TICKET = 0;
    crate::PROGRAM_SCENE = None;
    sex_pdx::serial_record!("[shell.weather] state=starting zone={}", ZONE);
    true
}

/// Hook in `launch_started`: true when the run that just started is the helper.
/// It runs hidden: no window on any workspace, nothing in the session.
pub unsafe fn started(target: u32, task: u32) -> bool {
    if !matches!(HELPER_STATE, Helper::Starting { .. }) || crate::LAUNCH_ASKER.is_some() { return false; }
    HELPER_STATE = Helper::Running { target, task };
    TOLD = None;
    if let Some(slot) = crate::HIDDEN_RUNS.iter_mut().find(|s| s.is_none()) { *slot = Some((target, task)); }
    sex_pdx::serial_record!("[shell.weather] state=running target={} task={}", target, task);
    true
}

/// Hook in `program_refused`: true when the refused run was the helper.
pub unsafe fn refused(code: u32) -> bool {
    let Helper::Starting { tries, .. } = HELPER_STATE else { return false };
    if crate::LAUNCH_ASKER.is_some() { return false; }
    HELPER_STATE = Helper::Off { tries: tries + 1 };
    sex_pdx::serial_record!("[shell.weather] state=refused code={} tries={}", code, tries + 1);
    true
}

unsafe fn stop(target: u32, task: u32) {
    if let Some(e) = crate::APP_CUSTODY.iter_mut().flatten().find(|e| e.target == target && e.task == task) { e.request_close(); }
    sex_pdx::serial_record!("[shell.weather] state=stopped target={} task={}", target, task);
    HELPER_STATE = Helper::Off { tries: 0 };
    TOLD = None;
}

unsafe fn tell_place(target: u32) {
    let Some(row) = app_catalog::by_target(target) else { return };
    let (status, _) = crate::pdx_call(row.route_slot, dw::OP_WEATHER_PLACE, ZONE, 0, 0);
    if status == sex_pdx::ERR_SERVICE_NOT_READY { return; }
    TOLD = Some(ZONE);
    sex_pdx::serial_record!("[shell.weather.place] zone={} ok={}", ZONE, (status == 0) as u8);
}

unsafe fn tell_display() {
    let Some((report, zone)) = LAST else { DISPLAY_OWED = false; return };
    DISPLAY_OWED = sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY, dw::OP_WEATHER, report, zone, 0).0 != 0;
}

/// A report from the helper's run. True when it was one (from anyone).
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != dw::OP_WEATHER { return false; }
    let ours = matches!(HELPER_STATE, Helper::Running { target, task } if target == msg.caller_pd && task == msg.caller_task);
    let report = dw::Report::unpack(msg.arg0);
    let ok = ours && report.is_some() && msg.arg1 == ZONE;
    sex_pdx::serial_record!("[shell.weather.report] pd={} zone={} report={:?} ok={}", msg.caller_pd, msg.arg1, report, ok as u8);
    if ok {
        LAST = Some((msg.arg0, msg.arg1));
        tell_display();
    }
    true
}
