// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `layout_plugin.rs` (slice S4 follow-up): Wasm layout plugins
//! for `silk layout WS plugin:FILE`.
//!
//! The Shell never links an interpreter and never waits on a plugin. Spindle
//! has Files prepare a run of `/bin/wasm layout @FILE` and sends the ticket
//! (`silk_ctl_glue` hands it here); the Shell starts that run hidden, as it
//! starts the weather helper, or reuses the run already there for the same
//! plugin. When a workspace using it retiles, the hook in
//! `tile_active_scene_frames` keeps the workspace's previous layout and sends
//! the run the tile count and work area (`sex_pdx::wasm_layout::OP_LAYOUT_ASK`)
//! as a plain call. The run's rectangles arrive as its own calls; once a whole
//! answer is in and checked, the workspace tiles again with it. A refused
//! answer, a fuel-exhausted or trapped guest, or a run that ended leaves the
//! previous layout in place. A run no workspace uses is closed.
//!
//! Serial record: `[shell.layout.plugin] ws=.. key=.. gen=.. n=.. ok=0|1
//! reason=..`, `[shell.layout.plugin.rect] ws=.. gen=.. i=.. x=.. y=.. w=.. h=..`,
//! `[shell.layout.plugin.run] key=.. state=starting|running|refused|timeout|
//! ended|stopping|stopped ...`, `[shell.layout.plugin.ask] ws=.. gen=.. n=..
//! w=.. h=.. sent=0|1`.

use crate::layout_plugin::{self as lp, Got, Plugins, Raw, Run, Tile};
use crate::wm::{self, Layout};
use crate::ATLAS_MAX_SCENES;
use sex_pdx::app_catalog;
use sex_pdx::program_store as store;
use sex_pdx::wasm_layout::{self as wire, Ask, Fail, RectMsg};

static mut PLUGINS: Plugins<ATLAS_MAX_SCENES> = Plugins::new();
fn plugins() -> &'static mut Plugins<ATLAS_MAX_SCENES> { unsafe { &mut *core::ptr::addr_of_mut!(PLUGINS) } }

const START_TICKS: u64 = 15_000;
const STOP_TICKS: u64 = 5_000;
static mut RECT_BUDGET: u32 = 512;
static mut TILE_BUDGET: u32 = 512;
static mut LAST_LOOK: u64 = 0;

/// The smallest window a plugin may make, as for a window the person resizes.
fn min_size() -> (u32, u32) {
    let m = crate::chrome_metrics();
    (m.px(crate::P.min_width), m.px(crate::P.min_height))
}

/// Whether the Shell may start a run now (no other run is being read or started).
unsafe fn can_start() -> bool {
    !(crate::PROGRAM_READING || crate::restricted_routes::busy() || (*core::ptr::addr_of!(crate::PROGRAM_REBIRTH)).is_some() || (*core::ptr::addr_of!(crate::LAUNCH_ASKER)).is_some()
        || crate::RECORD_SPAWN_WANTED != 0 || crate::RECORD_SPAWN_LEFT != 0 || crate::program_free_row().is_none())
}

/// What `silk layout WS plugin:...` did, for its answer line.
pub enum Set { Started, Reused, Busy, Full }

/// From `silk_ctl_glue`: workspace `scene` uses the plugin `key` from now. A
/// new plugin's run starts from Spindle's Files `ticket` (a prepared
/// `wasm layout ...`); a known plugin's run is reused and the ticket unused.
pub unsafe fn set(scene: u8, ticket: u32, key: u64) -> Set {
    let s = (scene as usize).min(ATLAS_MAX_SCENES - 1);
    let fallback = (*core::ptr::addr_of!(crate::WM_TILING))[s].layout;
    let known = plugins().slot_of(key).is_some();
    if !known && !can_start() { return Set::Busy; }
    let previous = plugins().uses[s].map(|u| u.key);
    let Ok((slot, fresh)) = plugins().set(s, key, fallback.index(), sex_pdx::get_ticks()) else { return Set::Full };
    if fresh {
        // Files starts a Spindle ticket only for the run Spindle prepared (requester 0).
        let (status, _) = crate::pdx_call(store::SHELL_ROUTE_SLOT, store::OP, store::TICKET, ticket as u64, 0);
        if status != 0 {
            plugins().drop_plugin(slot);
            sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=refused files_status={}", key, status);
            return Set::Busy;
        }
        crate::PROGRAM_READING = true;
        crate::PROGRAM_NETWORK = false;
        crate::PROGRAM_LAUNCHER = None;
        crate::PROGRAM_TICKET = 0;
        crate::PROGRAM_SCENE = None;
        sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=starting ws={} ticket={}", key, s + 1, ticket);
    }
    // The workspace's earlier plugin may be unused now.
    if let Some(old) = previous.filter(|o| *o != key && !plugins().used(*o)) {
        if let Some(i) = plugins().slot_of(old) { stop(i); }
    }
    sex_pdx::serial_record!("[shell.layout.plugin] ws={} key={:x} set=1 fallback={} fresh={}", s + 1, key, fallback.name(), fresh as u8);
    if scene == crate::ACTIVE_SCENE_IDX { crate::tile_active_scene_frames(); }
    if fresh { Set::Started } else { Set::Reused }
}

/// From `silk_ctl_glue` (`silk layout WS NAME`) and the tile hook (the
/// workspace's layout changed by a chord): the workspace stops using its plugin.
pub unsafe fn clear(scene: u8, why: &str) {
    let key = plugins().uses.get(scene as usize).and_then(|u| u.map(|u| u.key));
    let Some(key) = key else { return };
    let unused = plugins().clear(scene as usize);
    sex_pdx::serial_record!("[shell.layout.plugin] ws={} key={:x} cleared=1 reason={}", scene as u32 + 1, key, why);
    if let Some(i) = unused { stop(i); }
}

/// From tile_active_scene_frames, after `wm::tile` and `layouts_glue::tile`:
/// the plugin's answer for exactly this count and area, when there is one;
/// otherwise `rects` stay the previous layout's and the plugin is asked.
pub unsafe fn tile(t: &wm::Tiling, frames: &[u32], rects: &mut [wm::Rect]) {
    let s = crate::ACTIVE_SCENE_IDX;
    let Some(u) = plugins().uses.get(s as usize).copied().flatten() else { return };
    if Layout::from_index(u.fallback) != Some(t.layout) {
        clear(s, "layout_changed");
        return;
    }
    let area = crate::wm_work_area();
    match plugins().tile(s as usize, frames.len(), area.w, area.h) {
        Some(Tile::Apply) => {
            if let Some(got) = plugins().applied(s as usize) {
                for (r, g) in rects.iter_mut().zip(got) { *r = lp::placed(area, *g); }
            }
            if TILE_BUDGET > 0 {
                TILE_BUDGET -= 1;
                sex_pdx::serial_record!("[shell.layout.plugin.tile] ws={} key={:x} gen={} n={}", s as u32 + 1, u.key, u.generation, frames.len());
            }
        }
        Some(Tile::Ask { slot, .. }) => ask(slot),
        Some(Tile::Wait) | None => {}
    }
}

/// Sends plugin `slot`'s owed ask, when its run is there to take it.
unsafe fn ask(slot: usize) {
    let Some(p) = plugins().slots[slot].as_mut() else { return };
    let Some(q) = p.pending.as_mut().filter(|q| !q.sent) else { return };
    let Run::Running { target, .. } = p.run else { return };
    let Some(row) = app_catalog::by_target(target) else { return };
    let a = Ask { generation: q.generation, count: q.count as u32, w: q.w, h: q.h };
    let (status, _) = crate::pdx_call(row.route_slot, wire::OP_LAYOUT_ASK, a.wire()[0], a.wire()[1], a.wire()[2]);
    // A full inbox: the poll tries again.
    if status == sex_pdx::ERR_SERVICE_NOT_READY { return; }
    q.sent = true;
    sex_pdx::serial_record!("[shell.layout.plugin.ask] ws={} key={:x} gen={} n={} w={} h={} sent={}",
        q.scene as u32 + 1, p.key, q.generation, q.count, q.w, q.h, (status == 0) as u8);
}

/// A rectangle or failure from a plugin run. True when the message was one (from anyone).
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if !matches!(msg.type_id, wire::OP_LAYOUT_RECT | wire::OP_LAYOUT_FAIL) { return false; }
    let Some(slot) = plugins().slot_of_run(msg.caller_pd, msg.caller_task) else {
        sex_pdx::serial_record!("[shell.layout.plugin.reject] pd={} task={} reason=not_a_plugin_run", msg.caller_pd, msg.caller_task);
        return true;
    };
    let key = plugins().slots[slot].map_or(0, |p| p.key);
    let words = [msg.arg0, msg.arg1, msg.arg2];
    if msg.type_id == wire::OP_LAYOUT_FAIL {
        let Some((generation, why)) = Fail::decode(words) else { return true };
        match plugins().failed(slot, generation) {
            Some(scene) => sex_pdx::serial_record!("[shell.layout.plugin] ws={} key={:x} gen={} ok=0 reason={} kept={}",
                scene as u32 + 1, key, generation, why.name(), fallback_name(scene)),
            None => sex_pdx::serial_record!("[shell.layout.plugin] key={:x} gen={} ok=0 reason=stale plugin_reason={}", key, generation, why.name()),
        }
        return true;
    }
    let Some(m) = RectMsg::decode(words) else { return true };
    let raw = Raw { x: m.rect.x, y: m.rect.y, w: m.rect.w, h: m.rect.h };
    let (min_w, min_h) = min_size();
    match plugins().rect(slot, m.generation, m.index as usize, m.count as usize, raw, min_w, min_h) {
        Got::More => {}
        Got::Bad => sex_pdx::serial_record!("[shell.layout.plugin.reject] key={:x} gen={} i={} n={} reason=bad_piece", key, m.generation, m.index, m.count),
        Got::Stale { generation } => sex_pdx::serial_record!("[shell.layout.plugin] key={:x} gen={} ok=0 reason=stale", key, generation),
        Got::Done { scene, generation, result: Err(r) } => {
            sex_pdx::serial_record!("[shell.layout.plugin] ws={} key={:x} gen={} n={} ok=0 reason={} index={} kept={}",
                scene as u32 + 1, key, generation, m.count, r.reason(), r.index(), fallback_name(scene));
        }
        Got::Done { scene, generation, result: Ok(()) } => {
            let area = crate::wm_work_area();
            if let Some(got) = plugins().applied(scene as usize) {
                for (i, g) in got.iter().enumerate() {
                    if RECT_BUDGET == 0 { break; }
                    RECT_BUDGET -= 1;
                    let r = lp::placed(area, *g);
                    sex_pdx::serial_record!("[shell.layout.plugin.rect] ws={} gen={} i={} x={} y={} w={} h={}", scene as u32 + 1, generation, i, r.x, r.y, r.w, r.h);
                }
            }
            let shown = scene == crate::ACTIVE_SCENE_IDX;
            sex_pdx::serial_record!("[shell.layout.plugin] ws={} key={:x} gen={} n={} ok=1 applied={}", scene as u32 + 1, key, generation, m.count, shown as u8);
            if shown { crate::tile_active_scene_frames(); }
        }
    }
    true
}

unsafe fn fallback_name(scene: u8) -> &'static str {
    plugins().uses.get(scene as usize).and_then(|u| *u).and_then(|u| Layout::from_index(u.fallback)).map_or("-", |l| l.name())
}

/// Hook in `launch_started`: true when the run that just started is a plugin's.
/// It runs hidden: no window on any workspace, nothing in the session.
pub unsafe fn started(target: u32, task: u32) -> bool {
    if (*core::ptr::addr_of!(crate::LAUNCH_ASKER)).is_some() { return false; }
    let Some(slot) = plugins().slots.iter().position(|p| p.is_some_and(|p| matches!(p.run, Run::Starting { .. }))) else { return false };
    let Some(p) = plugins().slots[slot].as_mut() else { return false };
    p.run = Run::Running { target, task };
    if let Some(h) = (*core::ptr::addr_of_mut!(crate::HIDDEN_RUNS)).iter_mut().find(|s| s.is_none()) { *h = Some((target, task)); }
    sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=running target={} task={}", p.key, target, task);
    ask(slot);
    true
}

/// Hook in `program_refused`: true when the refused run was a plugin's.
pub unsafe fn refused(code: u32) -> bool {
    if (*core::ptr::addr_of!(crate::LAUNCH_ASKER)).is_some() { return false; }
    let Some(slot) = plugins().slots.iter().position(|p| p.is_some_and(|p| matches!(p.run, Run::Starting { .. }))) else { return false };
    let key = plugins().slots[slot].map_or(0, |p| p.key);
    let n = plugins().drop_plugin(slot);
    sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=refused code={} workspaces={}", key, code, n);
    sex_pdx::serial_record!("[shell.layout.plugin] key={:x} ok=0 reason=refused", key);
    true
}

/// Asks plugin `slot`'s run to close: no workspace uses it.
unsafe fn stop(slot: usize) {
    let Some(p) = plugins().slots[slot].as_mut() else { return };
    match p.run {
        Run::Running { target, task } => {
            if let Some(e) = (*core::ptr::addr_of_mut!(crate::APP_CUSTODY)).iter_mut().flatten().find(|e| e.target == target && e.task == task) { e.request_close(); }
            p.run = Run::Stopping { target, task, since: sex_pdx::get_ticks() };
            p.pending = None;
            sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=stopping target={} task={}", p.key, target, task);
        }
        // Not running yet: it is closed once it starts (the poll sees it unused).
        Run::Starting { .. } | Run::Stopping { .. } => {}
    }
}

/// Main loop: runs that ended or would not start, owed asks, unused runs.
pub unsafe fn poll() {
    let now = sex_pdx::get_ticks();
    if now.saturating_sub(LAST_LOOK) < 50 { return; }
    LAST_LOOK = now;
    for slot in 0..lp::MAX_PLUGINS {
        let Some(p) = plugins().slots[slot] else { continue };
        let used = plugins().used(p.key);
        match p.run {
            Run::Starting { since } => {
                if now.saturating_sub(since) > START_TICKS && !crate::PROGRAM_READING && (*core::ptr::addr_of!(crate::PROGRAM_REBIRTH)).is_none() {
                    let n = plugins().drop_plugin(slot);
                    sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=timeout workspaces={}", p.key, n);
                    sex_pdx::serial_record!("[shell.layout.plugin] key={:x} ok=0 reason=start_timeout", p.key);
                }
            }
            Run::Running { target, task } => {
                if crate::program_run(target).is_none_or(|(_, t)| t != task) {
                    // A crash: every workspace that used it keeps its previous layout.
                    let n = plugins().drop_plugin(slot);
                    sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=ended target={} task={} workspaces={}", p.key, target, task, n);
                    sex_pdx::serial_record!("[shell.layout.plugin] key={:x} ok=0 reason=ended", p.key);
                } else if !used {
                    stop(slot);
                } else {
                    ask(slot);
                }
            }
            Run::Stopping { target, task, since } => {
                if crate::program_run(target).is_none_or(|(_, t)| t != task) {
                    plugins().slots[slot] = None;
                    sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=stopped target={} task={}", p.key, target, task);
                } else if now.saturating_sub(since) > STOP_TICKS {
                    // It ignored Close: ended the way a program ignoring Close is.
                    let ok = app_catalog::by_target(target).is_some_and(|r| sex_pdx::app_lifecycle::force_end(r.lifecycle_slot).is_ok());
                    sex_pdx::serial_record!("[shell.layout.plugin.run] key={:x} state=forced target={} task={} ok={}", p.key, target, task, ok as u8);
                    if let Some(p) = plugins().slots[slot].as_mut() { p.run = Run::Stopping { target, task, since: now }; }
                }
            }
        }
    }
}

/// How long the idle Shell may sleep while a run starts, stops or owes an ask.
pub unsafe fn wake() -> Option<u64> {
    let busy = plugins().slots.iter().flatten().any(|p| match p.run {
        Run::Starting { .. } | Run::Stopping { .. } => true,
        Run::Running { .. } => p.pending.is_some_and(|q| !q.sent) || !plugins().used(p.key),
    });
    // A run that ends must be noticed even when nothing else happens.
    if busy { Some(100) } else if plugins().slots.iter().any(|p| p.is_some()) { Some(1_000) } else { None }
}
