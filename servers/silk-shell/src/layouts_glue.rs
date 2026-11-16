// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `layouts.rs` (slice S3): tabbed window groups on the
//! Shell's own frames and tabs (a group is a frame with more than one tab, so
//! the title bar's tab strip, tab clicks and tab drags work on it as they
//! are), the dwindle split tree, and per-workspace layout memory. Chords come
//! from `wm.rs` through `wm_action`; tiling, scene visibility and the main
//! loop each call in here from one hook line in main.rs.

use crate::layouts::{self, Desk, Members};
use crate::wm::{self, Layout, WmAction};
use crate::{ShellFrame, ShellTab, FRAMES, MAX_FRAMES, MAX_TABS_PER_FRAME};

static mut DESK: Desk<{ crate::ATLAS_MAX_SCENES }> = Desk::new();
/// A frame the next tiling's new window splits, instead of the focused one.
static mut SPLIT_HINT: Option<u32> = None;
/// The workspace last reported by `poll`.
static mut REPORTED_SCENE: u8 = u8::MAX;
static mut TILE_RECORD_BUDGET: u32 = 1024;

fn desk() -> &'static mut Desk<{ crate::ATLAS_MAX_SCENES }> { unsafe { &mut *core::ptr::addr_of_mut!(DESK) } }

unsafe fn scene() -> usize { crate::ACTIVE_SCENE_IDX as usize }

unsafe fn frame(fid: u32) -> Option<&'static mut ShellFrame> {
    (*core::ptr::addr_of_mut!(FRAMES)).iter_mut().flatten().find(|f| f.frame_id == fid)
}

unsafe fn members(fid: u32) -> Members {
    match frame(fid) {
        Some(f) => Members::new(f.tabs.iter().flatten().map(|t| t.surface_id)),
        None => Members::new(core::iter::empty()),
    }
}

/// From tile_active_scene_frames, after `wm::tile`: in dwindle, the
/// workspace's split tree places the windows instead. `frames` are in tiling
/// order and `rects` match them.
pub unsafe fn tile(t: &wm::Tiling, frames: &[u32], rects: &mut [wm::Rect]) {
    let s = scene();
    let dwindle = t.layout == Layout::Dwindle;
    let fresh = desk().entered(s, dwindle);
    if !dwindle { return; }
    let focus = core::ptr::replace(core::ptr::addr_of_mut!(SPLIT_HINT), None).or_else(|| crate::selected_frame_id());
    let tree = desk().tree(s);
    tree.sync(frames, focus);
    let area = crate::wm_work_area();
    let record = TILE_RECORD_BUDGET > 0;
    if record {
        TILE_RECORD_BUDGET -= 1;
        sex_pdx::serial_record!("[shell.layout] scene={} layout=dwindle windows={} leaves={} fresh={}", s, frames.len(), tree.len(), fresh as u8);
    }
    for (i, &fid) in frames.iter().enumerate() {
        let Some(p) = tree.place(area, t.gap_inner, t.gap_outer, fid) else { continue };
        if let Some(r) = rects.get_mut(i) { *r = p.rect; }
        if record {
            let sid = crate::active_surface_for_frame(fid).unwrap_or(0);
            sex_pdx::serial_record!("[shell.layout.tile] scene={} n={}/{} frame={} sid={} depth={} split={} x={} y={} w={} h={}",
                s, i, frames.len(), fid, sid, p.depth, p.split.map_or("-", |d| d.name()), p.rect.x, p.rect.y, p.rect.w, p.rect.h);
        }
    }
}

/// From sync_scene_visibility: whether tab `ti` of frame `fid` is a group's
/// tab in the background, which stays hidden when its workspace shows.
pub unsafe fn tab_hidden(fid: u32, ti: usize) -> bool {
    frame(fid).map_or(false, |f| f.tab_count > 1 && ti != f.active_tab as usize)
}

/// Main loop: a workspace switch reports the layout, tree and groups the
/// workspace kept.
pub unsafe fn poll() {
    let s = crate::ACTIVE_SCENE_IDX;
    if s == REPORTED_SCENE { return; }
    REPORTED_SCENE = s;
    let layout = crate::wm_tiling().layout;
    let (mut groups, mut tabs) = (0u32, 0u8);
    for f in (*core::ptr::addr_of!(FRAMES)).iter().flatten() {
        if f.scene_id == s && f.tab_count > 1 { groups += 1; tabs = tabs.max(f.tab_count); }
    }
    sex_pdx::serial_record!("[shell.layout.scene] scene={} layout={} leaves={} groups={} tabs={}",
        s, layout.name(), desk().tree_ref(s as usize).len(), groups, tabs);
}

/// The window manager actions this slice adds.
pub unsafe fn action(a: WmAction) -> bool {
    match a {
        WmAction::Group => group(),
        WmAction::Ungroup => ungroup(),
        WmAction::NextTab => step(1),
        WmAction::PrevTab => step(-1),
        WmAction::ToggleDwindle => {
            let t = crate::wm_tiling_mut();
            let next = desk().toggle(scene(), t.layout);
            t.layout = next;
            if next != Layout::Monocle { t.before_monocle = next; }
            sex_pdx::serial_record!("[shell.layout] scene={} toggle=dwindle layout={}", scene(), next.name());
            crate::tile_active_scene_frames();
            true
        }
        _ => false,
    }
}

/// Super+G: the window before the focused one in tiling order (the one
/// after it, for the first) joins the focused window's group as its last tabs.
unsafe fn group() -> bool {
    let Some(fid) = crate::selected_frame_id() else {
        sex_pdx::serial_record!("[shell.group] join ok=0 reason=no_focus");
        return false;
    };
    if !matches!(crate::INTERACTION, crate::InteractionState::Idle) {
        sex_pdx::serial_record!("[shell.group] join frame={} ok=0 reason=busy", fid);
        return false;
    }
    let mut seen = [(0u32, 0u64, wm::Rect::default()); MAX_FRAMES];
    let n = crate::wm_visible_frames(&mut seen, false);
    let mut order = [0u32; MAX_FRAMES];
    for i in 0..n { order[i] = seen[i].0; }
    (*core::ptr::addr_of_mut!(crate::WM_ORDER)).sort(&mut order[..n]);
    let Some(other) = layouts::join_target(&order[..n], fid) else {
        sex_pdx::serial_record!("[shell.group] join frame={} ok=0 reason=alone", fid);
        return false;
    };
    let slots = &mut *core::ptr::addr_of_mut!(FRAMES);
    let (Some(di), Some(si)) = (slots.iter().position(|f| f.map_or(false, |f| f.frame_id == fid)),
                                slots.iter().position(|f| f.map_or(false, |f| f.frame_id == other))) else { return false };
    let src = slots[si].unwrap();
    let Some(dst) = slots[di].as_mut() else { return false };
    if dst.scene_id != src.scene_id || !layouts::append(&mut dst.tabs, &mut dst.tab_count, &src.tabs) {
        sex_pdx::serial_record!("[shell.group] join frame={} took={} ok=0 reason=full tabs={}+{}", fid, other, dst.tab_count, src.tab_count);
        return false;
    }
    slots[si] = None;
    // The joined windows wait behind the group's active tab.
    for t in src.tabs.iter().flatten() {
        if crate::surface_is_alive(t.surface_id) { crate::pdx_call(sex_pdx::SLOT_DISPLAY, 0xEE, t.surface_id, 0, 0); }
    }
    if crate::HOVERED_FRAME_ID == other { crate::HOVERED_FRAME_ID = 0; crate::HOVER_KIND = crate::HOVER_NONE; crate::HOVERED_FRAME_LIGHT = crate::FRAME_LIGHT_NONE; }
    if crate::TAB_DRAG_PENDING_FRAME_ID == other { crate::TAB_DRAG_PENDING_FRAME_ID = 0; crate::DRAG_PENDING_ACTIVE = false; }
    crate::send_frame_tab_info(fid);
    crate::tile_active_scene_frames();
    let active = crate::active_surface_for_frame(fid).unwrap_or(0);
    if active != 0 && crate::FOCUSED_SURFACE_ID != active { let _ = crate::try_set_focus(active); }
    crate::snap_capture_layout();
    let f = frame(fid).map_or((0, 0), |f| (f.tab_count, f.active_tab));
    sex_pdx::serial_record!("[shell.group] join frame={} took={} tabs={} active={} sid={} members={} ok=1", fid, other, f.0, f.1, active, members(fid));
    true
}

/// Super+Shift+G: the focused tab leaves its group for a frame of its own,
/// tiled next to the group.
unsafe fn ungroup() -> bool {
    let Some(fid) = crate::selected_frame_id() else {
        sex_pdx::serial_record!("[shell.group] split ok=0 reason=no_focus");
        return false;
    };
    if !matches!(crate::INTERACTION, crate::InteractionState::Idle) {
        sex_pdx::serial_record!("[shell.group] split frame={} ok=0 reason=busy", fid);
        return false;
    }
    let slots = &mut *core::ptr::addr_of_mut!(FRAMES);
    let Some(free) = slots.iter().position(|f| f.is_none()) else {
        sex_pdx::serial_record!("[shell.group] split frame={} ok=0 reason=no_frame_slot", fid);
        return false;
    };
    // Dynamic frame ids start at 10, as for app surfaces; skip one in use.
    let mut new_id = (free + 10) as u32;
    while slots.iter().flatten().any(|f| f.frame_id == new_id) { new_id += 1; }
    let Some(g) = slots.iter_mut().flatten().find(|f| f.frame_id == fid) else { return false };
    let bounds = g.tabs[g.active_tab as usize].and_then(|t| crate::get_surface_bounds(t.surface_id));
    let idx = g.active_tab;
    let Some(tab) = layouts::take(&mut g.tabs, &mut g.tab_count, &mut g.active_tab, idx) else {
        sex_pdx::serial_record!("[shell.group] split frame={} ok=0 reason=alone", fid);
        return false;
    };
    let (scene, flags, nx, ny, nw, nh) = (g.scene_id, g.flags, g.normal_x, g.normal_y, g.normal_w, g.normal_h);
    let shown = g.tabs[g.active_tab as usize].map(|t| t.surface_id);
    let mut tabs: [Option<ShellTab>; MAX_TABS_PER_FRAME as usize] = [None; MAX_TABS_PER_FRAME as usize];
    tabs[0] = Some(tab);
    slots[free] = Some(ShellFrame { frame_id: new_id, active_tab: 0, tab_count: 1, tabs, scene_id: scene,
        flags: flags & crate::FRAME_FLAG_TOP_BAR, normal_x: nx, normal_y: ny, normal_w: nw, normal_h: nh });
    // The group shows its next tab where the window that left was.
    if let (Some(sid), Some((x, y, w, h))) = (shown, bounds) {
        crate::pdx_call(sex_pdx::SLOT_DISPLAY, 0xEC, sid, (y as u64) << 32 | x as u32 as u64, (h as u64) << 32 | w as u64);
        crate::update_local_geometry(sid, x, y, w, h);
    }
    crate::send_frame_tab_info(fid);
    // Dwindle: the new tile splits the group's.
    SPLIT_HINT = Some(fid);
    crate::tile_active_scene_frames();
    SPLIT_HINT = None;
    let _ = crate::try_set_focus(tab.surface_id);
    crate::send_frame_tab_info(new_id);
    crate::snap_capture_layout();
    let left = frame(fid).map_or(0, |f| f.tab_count);
    sex_pdx::serial_record!("[shell.group] split frame={} sid={} to={} tabs={} active={} members={} ok=1",
        fid, tab.surface_id, new_id, left, shown.unwrap_or(0), members(fid));
    true
}

/// Super+. / Super+,: the next or previous tab of the focused group.
unsafe fn step(by: i8) -> bool {
    let Some(fid) = crate::selected_frame_id() else {
        sex_pdx::serial_record!("[shell.group] tab ok=0 reason=no_focus");
        return false;
    };
    let Some((count, active)) = frame(fid).map(|f| (f.tab_count, f.active_tab)) else { return false };
    if count < 2 {
        sex_pdx::serial_record!("[shell.group] tab frame={} ok=0 reason=alone", fid);
        return false;
    }
    let next = layouts::cycle(active, count, by);
    let ok = crate::switch_to_tab(fid, next as u32);
    let (sid, focus) = (crate::active_surface_for_frame(fid).unwrap_or(0), crate::FOCUSED_SURFACE_ID);
    sex_pdx::serial_record!("[shell.group] tab frame={} step={} tab={}/{} sid={} focus={} members={} ok={}",
        fid, by, next, count, sid, focus, members(fid), ok as u8);
    ok
}

/// Super+Shift+H/J/K/L swapped two windows' places in tiling order; in
/// dwindle their tiles swap too.
pub unsafe fn swapped(a: u32, b: u32) { desk().tree(scene()).swap(a, b); }
