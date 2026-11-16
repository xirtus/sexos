// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `silk_ctl.rs` (slice S4): Spindle's `silk` command.
//! `OP_SILK_CTL` requests come in here from one hook line in main.rs and are
//! carried out with the Shell's own actions (the workspace move Super+Shift+N
//! makes, the focus a click gives, the close button's close, the layouts of
//! `wm.rs` and `layouts_glue.rs`). Answers and live events go back to the
//! caller as `OP_SILK_OUT` plain calls: the Shell waits on nothing and the
//! caller's reply slot is never used. The main loop's poll compares the
//! desktop with the last look and keeps the events.

use crate::silk_ctl::{self as sc, Act, Feeds, Line, Watch, Win};
use crate::wm::Layout;
use crate::{FRAMES, MAX_FRAMES, ATLAS_MAX_SCENES};
use sex_pdx::app_catalog;
use sex_pdx::silk_ctl::{self as wire, Request};

/// Events kept for `silk events`.
const KEPT: usize = 32;
static mut WATCH: Watch<MAX_FRAMES, ATLAS_MAX_SCENES, KEPT> = Watch::new();
static mut FEEDS: Feeds<4> = Feeds::new();

/// Pieces waiting to go out, oldest first.
#[derive(Clone, Copy)]
struct Out { pd: u32, task: u32, words: [u64; 3] }
const OUT_MAX: usize = 384;
static mut OUTBOX: [Out; OUT_MAX] = [Out { pd: 0, task: 0, words: [0; 3] }; OUT_MAX];
static mut HEAD: usize = 0;
static mut LEN: usize = 0;
/// When a send was first refused for a full inbox, while it waits.
static mut STUCK_SINCE: Option<u64> = None;
static mut LAST_LOOK: u64 = 0;
static mut EVENT_BUDGET: u32 = 512;
static mut WINDOW_BUDGET: u32 = 256;

fn watch() -> &'static mut Watch<MAX_FRAMES, ATLAS_MAX_SCENES, KEPT> { unsafe { &mut *core::ptr::addr_of_mut!(WATCH) } }
fn feeds() -> &'static mut Feeds<4> { unsafe { &mut *core::ptr::addr_of_mut!(FEEDS) } }

fn alive(pd: u32, task: u32) -> bool { crate::running_task(pd) == Some(task) }

/// Spindle's live run, or a run a Spindle terminal started attached to it
/// while both run (a script). Nothing else may drive the desktop.
unsafe fn allowed(pd: u32, task: u32) -> bool {
    if !alive(pd, task) { return false; }
    if app_catalog::is_spindle(pd) { return true; }
    (*core::ptr::addr_of!(crate::ATTACHED_RUNS)).iter().flatten()
        .any(|r| r.0 == pd && r.1 == task && app_catalog::is_spindle(r.2) && alive(r.2, r.3))
}

/// The windows there are, as `silk windows` lists them.
unsafe fn windows(out: &mut [Win; MAX_FRAMES]) -> usize {
    let mut n = 0;
    for f in (*core::ptr::addr_of!(FRAMES)).iter().flatten() {
        let Some(tab) = f.tabs[(f.active_tab as usize).min(f.tabs.len() - 1)] else { continue };
        let sid = tab.surface_id;
        if !crate::surface_is_alive(sid) || crate::is_tombstoned(sid) || n == MAX_FRAMES { continue; }
        let (x, y, w, h) = crate::get_surface_bounds(sid).unwrap_or((0, 0, 0, 0));
        let mut win = Win { frame: f.frame_id, sid, scene: f.scene_id, x, y, w, h,
            floating: f.flags & crate::FRAME_FLAG_FLOATING != 0, zoomed: f.flags & crate::FRAME_FLAG_ZOOMED != 0,
            minimized: f.flags & crate::FRAME_FLAG_MINIMIZED != 0, tabs: f.tab_count, tab: f.active_tab, ..Win::EMPTY };
        if let Ok(len) = crate::reader_glue::title(sid, &mut win.title) { win.title_len = len.min(16) as u8; }
        out[n] = win;
        n += 1;
    }
    n
}

unsafe fn layouts() -> [u8; ATLAS_MAX_SCENES] {
    let mut l = [0u8; ATLAS_MAX_SCENES];
    for (i, t) in (*core::ptr::addr_of!(crate::WM_TILING)).iter().enumerate() { l[i] = t.layout.index(); }
    l
}

/// Queues one line of the answer tagged `tag` for `pd`.
unsafe fn say(pd: u32, task: u32, tag: u16, line: &[u8], last: bool, failed: bool) -> bool {
    let need = sc::line_pieces(line.len());
    // The answer's end must always fit, so other lines leave room for it.
    if LEN + need + if last { 0 } else { sc::END_ROOM } > OUT_MAX {
        sex_pdx::serial_record!("[shell.silk.out.full] pd={} tag={} queued={} dropped=1", pd, tag, { LEN });
        return false;
    }
    for p in wire::pieces(tag, line, last, failed) {
        OUTBOX[(HEAD + LEN) % OUT_MAX] = Out { pd, task, words: p.wire() };
        LEN += 1;
    }
    true
}
/// Outbox pieces free now.
unsafe fn free() -> usize { OUT_MAX - LEN }
unsafe fn end(pd: u32, task: u32, tag: u16, line: &Line, ok: bool) { say(pd, task, tag, line.as_bytes(), true, !ok); }
fn line(args: core::fmt::Arguments) -> Line { let mut l = Line::new(); let _ = core::fmt::Write::write_fmt(&mut l, args); l }

/// A `OP_SILK_CTL` request. True when it was one.
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != wire::OP_SILK_CTL { return false; }
    let (pd, task) = (msg.caller_pd, msg.caller_task);
    if !allowed(pd, task) {
        sex_pdx::serial_record!("[shell.silk.reject] pd={} task={} reason=not_spindle", pd, task);
        return true;
    }
    let Some((tag, req)) = Request::decode([msg.arg0, msg.arg1, msg.arg2]) else {
        sex_pdx::serial_record!("[shell.silk.reject] pd={} task={} reason=bad_request", pd, task);
        return true;
    };
    let mut wins = [Win::EMPTY; MAX_FRAMES];
    let n = windows(&mut wins);
    // The desktop as it is before this request acts: events stay in order.
    observe();
    let ok = match sc::check(req, &wins[..n], crate::ACTIVE_SCENE_IDX) {
        Err(r) => { end(pd, task, tag, &r.line(), false); sex_pdx::serial_record!("[shell.silk.refused] pd={} tag={} kind={} reason={}", pd, tag, req.name(), r.reason()); false }
        Ok(act) => act_on(pd, task, tag, act, &wins[..n]),
    };
    sex_pdx::serial_record!("[shell.silk.request] pd={} task={} tag={} kind={} ok={}", pd, task, tag, req.name(), ok as u8);
    observe();
    flush();
    true
}

unsafe fn act_on(pd: u32, task: u32, tag: u16, act: Act, wins: &[Win]) -> bool {
    match act {
        Act::List => {
            let focus = crate::FOCUSED_SURFACE_ID;
            // The whole list or none of it: never an `ok` after lost lines.
            if !sc::fit(free(), wins.iter().map(|w| sc::window_line(w, w.sid == focus).as_bytes().len())).1 {
                sex_pdx::serial_record!("[shell.silk.out.full] pd={} tag={} queued={} answer=windows busy=1", pd, tag, { LEN });
                end(pd, task, tag, &sc::busy_line("windows"), false);
                return false;
            }
            for w in wins {
                let l = sc::window_line(w, w.sid == focus);
                if WINDOW_BUDGET > 0 { WINDOW_BUDGET -= 1; sex_pdx::serial_record!("[shell.silk.window] {}", core::str::from_utf8(l.as_bytes()).unwrap_or("?")); }
                say(pd, task, tag, l.as_bytes(), false, false);
            }
            end(pd, task, tag, &sc::list_end(wins.len(), crate::ACTIVE_SCENE_IDX, crate::frame_for_surface(focus).unwrap_or(0)), true);
            true
        }
        Act::Focus { frame, sid, scene } => {
            if scene != crate::ACTIVE_SCENE_IDX { crate::switch_scene(scene); }
            if crate::layouts_glue::behind(sid).is_some() { crate::layouts_glue::show(sid); }
            let sid = crate::active_surface_for_frame(frame).unwrap_or(sid);
            let ok = crate::try_set_focus(sid);
            sex_pdx::serial_record!("[shell.silk.focus] frame={} sid={} ws={} focus={} ok={}", frame, sid, scene as u32 + 1, { crate::FOCUSED_SURFACE_ID }, ok as u8);
            end(pd, task, tag, &line(format_args!("{} focus frame={} sid={} ws={}", if ok { "ok" } else { "error" }, frame, sid, scene as u32 + 1)), ok);
            ok
        }
        Act::Stay { frame, scene } => {
            sex_pdx::serial_record!("[shell.silk.move] frame={} to={} scene={} already=1 ok=1", frame, scene as u32 + 1, scene);
            end(pd, task, tag, &line(format_args!("ok move frame={} ws={} already=1", frame, scene as u32 + 1)), true);
            true
        }
        Act::Move { frame, sid, from, to } => {
            let ok = crate::reparent_frame_to_scene(frame, to, "silk_ctl");
            let now = (*core::ptr::addr_of!(FRAMES)).iter().flatten().find(|f| f.frame_id == frame).map_or(u8::MAX, |f| f.scene_id);
            let ok = ok && now == to;
            sex_pdx::serial_record!("[shell.silk.move] frame={} sid={} from={} to={} scene={} ok={}", frame, sid, from as u32 + 1, to as u32 + 1, now, ok as u8);
            end(pd, task, tag, &line(format_args!("{} move frame={} sid={} from={} to={}", if ok { "ok" } else { "error" }, frame, sid, from as u32 + 1, to as u32 + 1)), ok);
            ok
        }
        Act::Layout { scene, layout } => {
            let Some(l) = Layout::from_index(layout) else { return false };
            let t = &mut (*core::ptr::addr_of_mut!(crate::WM_TILING))[(scene as usize).min(ATLAS_MAX_SCENES - 1)];
            t.layout = l;
            if l != Layout::Monocle { t.before_monocle = l; }
            let shown = scene == crate::ACTIVE_SCENE_IDX;
            if shown { crate::tile_active_scene_frames(); }
            sex_pdx::serial_record!("[shell.silk.layout] ws={} layout={} shown={} ok=1", scene as u32 + 1, l.name(), shown as u8);
            end(pd, task, tag, &line(format_args!("ok layout ws={} layout={}", scene as u32 + 1, l.name())), true);
            true
        }
        Act::Float { frame, sid, on, shown } => {
            let zoomed = crate::frame_is_zoomed(frame);
            if zoomed { crate::toggle_zoom_frame(frame); }
            crate::set_frame_floating(frame, on);
            if on && shown { crate::wm_place_floating(frame, 60); }
            if shown { crate::tile_active_scene_frames(); }
            // Unzooming puts the surface back on the display; a window on
            // another workspace is hidden again, as a workspace switch does.
            else if zoomed { crate::sync_scene_visibility(); }
            sex_pdx::serial_record!("[shell.silk.float] frame={} sid={} on={} shown={} ok=1", frame, sid, on as u8, shown as u8);
            end(pd, task, tag, &line(format_args!("ok float frame={} on={}", frame, on as u8)), true);
            true
        }
        Act::Close { frame, sid } => {
            let ok = crate::close_surface_from_frame_light(sid);
            sex_pdx::serial_record!("[shell.silk.close] frame={} sid={} ok={}", frame, sid, ok as u8);
            end(pd, task, tag, &line(format_args!("{} close frame={} sid={}", if ok { "ok" } else { "error" }, frame, sid)), ok);
            ok
        }
        Act::Events { since } => {
            // As many as fit; `last=` names only what was sent.
            let (room, all) = sc::fit(free(), watch().since(since).map(|e| e.line().as_bytes().len()));
            let (mut count, mut last) = (0, since);
            for e in watch().since(since).take(room) {
                if !say(pd, task, tag, e.line().as_bytes(), false, false) { break; }
                count += 1;
                last = e.seq;
            }
            let all = all && count == room;
            if all { last = watch().last(); }
            end(pd, task, tag, &sc::events_end(count, last, all), all);
            sex_pdx::serial_record!("[shell.silk.events] pd={} since={} events={} last={} all={}", pd, since, count, last, all as u8);
            all
        }
        Act::Follow { count } => {
            let last = watch().last();
            match feeds().start(pd, task, tag, count, last) {
                Err(()) => {
                    end(pd, task, tag, &line(format_args!("error follow busy (four feeds already)")), false);
                    return false;
                }
                Ok(Some(old)) if old.tag != tag => end(pd, task, old.tag, &line(format_args!("ok follow replaced")), true),
                Ok(_) => {}
            }
            say(pd, task, tag, line(format_args!("following after={} count={}", last, count)).as_bytes(), false, false);
            sex_pdx::serial_record!("[shell.silk.follow] pd={} tag={} after={} count={} ok=1", pd, tag, last, count);
            true
        }
        Act::Unfollow => {
            let old = feeds().stop(pd);
            let t = old.map_or(tag, |f| f.tag);
            end(pd, task, t, &line(format_args!("ok follow stopped")), true);
            sex_pdx::serial_record!("[shell.silk.follow] pd={} tag={} stopped={} ok=1", pd, t, old.is_some() as u8);
            true
        }
    }
}

/// Compares the desktop with the last look; new events go to the feeds.
unsafe fn observe() {
    let mut seen = [(0u32, 0u64, 0u8); MAX_FRAMES];
    let mut n = 0;
    for f in (*core::ptr::addr_of!(FRAMES)).iter().flatten() {
        let Some(tab) = f.tabs[(f.active_tab as usize).min(f.tabs.len() - 1)] else { continue };
        if !crate::surface_is_alive(tab.surface_id) || crate::is_tombstoned(tab.surface_id) || n == MAX_FRAMES { continue; }
        seen[n] = (f.frame_id, tab.surface_id, f.scene_id);
        n += 1;
    }
    let focus = crate::FOCUSED_SURFACE_ID;
    let focus_frame = crate::frame_for_surface(focus).unwrap_or(0);
    let before = watch().last();
    if watch().observe(&seen[..n], focus, focus_frame, crate::ACTIVE_SCENE_IDX, &layouts()) != 0 {
        for e in watch().since(before) {
            if EVENT_BUDGET > 0 { EVENT_BUDGET -= 1; sex_pdx::serial_record!("[shell.silk.event] {}", core::str::from_utf8(e.line().as_bytes()).unwrap_or("?")); }
        }
    }
    // Every look, not only one that saw a change: a feed stopped by a full
    // outbox catches up once it drains.
    if !feeds().behind(watch().last()) { return; }
    for slot in feeds().iter_mut() {
        let Some(f) = slot.as_mut() else { continue };
        let mut done = false;
        for e in watch().since(f.after) {
            if !say(f.pd, f.task, f.tag, e.line().as_bytes(), false, false) { break; }
            if f.sent(e.seq) { done = true; break; }
        }
        if done {
            let (pd, task, tag) = (f.pd, f.task, f.tag);
            *slot = None;
            end(pd, task, tag, &line(format_args!("ok follow done")), true);
            sex_pdx::serial_record!("[shell.silk.follow] pd={} tag={} done=1 ok=1", pd, tag);
        }
    }
}

/// Whether a send waits to be retried soon (keeps the main loop from blocking).
pub unsafe fn busy() -> bool {
    STUCK_SINCE.is_some_and(|t| sex_pdx::get_ticks().saturating_sub(t) < 2_000)
}

/// Main loop: events, ended callers' feeds, waiting sends.
pub unsafe fn poll() {
    observe();
    let now = sex_pdx::get_ticks();
    if feeds().any() && now.saturating_sub(LAST_LOOK) >= 250 {
        LAST_LOOK = now;
        let gone = feeds().retain(alive);
        if gone != 0 { sex_pdx::serial_record!("[shell.silk.follow] gone={}", gone); }
    }
    if LEN != 0 { flush(); }
}

/// Sends the waiting pieces, in order, to callers that still run.
unsafe fn flush() {
    let mut stuck = false;
    while LEN != 0 {
        let o = OUTBOX[HEAD];
        let route = app_catalog::by_target(o.pd).map(|r| r.route_slot).filter(|_| alive(o.pd, o.task));
        if let Some(route) = route {
            // The raw call: the Shell's wrapper would take this Spindle's slot for the shown one's.
            let (status, _) = sex_pdx::pdx_call(route, wire::OP_SILK_OUT, o.words[0], o.words[1], o.words[2]);
            if status == sex_pdx::ERR_SERVICE_NOT_READY { stuck = true; break; }
            if status != 0 { sex_pdx::serial_record!("[shell.silk.out] pd={} status={:#x} dropped=1", o.pd, status); }
        }
        HEAD = (HEAD + 1) % OUT_MAX;
        LEN -= 1;
    }
    STUCK_SINCE = if stuck { Some(STUCK_SINCE.unwrap_or(sex_pdx::get_ticks())) } else { None };
}
