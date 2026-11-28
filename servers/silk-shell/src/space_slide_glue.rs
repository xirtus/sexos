// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for macOS-style Continuous Space Sliding, Mission Control 5-Space
//! bar + window-card drag-and-drop, carry-window-across-Spaces (edge spring-load,
//! `Super+1..5` while dragging, `Super+Shift+Left/Right`, SilkBar wheel), Frosted
//! Noir snap zones (with clean un-snap on drag-off), and SilkBar quick-layout icons.

use sex_pdx::serial_println;
use sex_pdx::space_slide::{
    detect_snap_zone, quick_layout_rects, snap_rect, EdgeCarryTracker, GestureMsg,
    MissionControlLayout, OverviewSpacesState, QuickLayoutBtn, SlideCommand, SnapPreviewState,
    SnapZone, SpaceSlidePhysics, DESKTOP_SPACE_COUNT, OP_OVERVIEW_SPACES, OP_SNAP_PREVIEW,
    OP_SPACE_SLIDE, SPRING_LOAD_EDGE_MS,
};
use silkbar_model::{ModuleSlot, DEFAULT_SILK_BAR};

#[derive(Clone, Copy)]
struct OverviewDrag {
    idx: usize,
    sid: u64,
    start_x: i32,
    start_y: i32,
    cur_x: i32,
    cur_y: i32,
    dragging: bool,
}

static mut SLIDE: SpaceSlidePhysics = SpaceSlidePhysics::new();
static mut LAST_SLIDE: SlideCommand = SlideCommand::IDLE;
static mut SUPPRESS_NEXT_SWITCH: bool = false;
static mut SNAP_PREVIEW: SnapPreviewState = SnapPreviewState::HIDDEN;
static mut EDGE_CARRY: EdgeCarryTracker = EdgeCarryTracker::new();
/// Pre-snap floating `(frame_id, normal_w, normal_h)` so pulling a snapped window
/// back off its edge restores its natural size instead of dragging a giant half-screen pane.
static mut PRE_SNAP: [Option<(u32, u32, u32)>; crate::MAX_FRAMES] = [None; crate::MAX_FRAMES];
static mut OVERVIEW_DRAG: Option<OverviewDrag> = None;
static mut OVERVIEW_HOVER_SPACE: Option<u8> = None;
static mut MISSION_ONLY: bool = false;
static mut OVERVIEW_SHOWN: bool = false;
static mut LAST_BAR_WHEEL_MS: u64 = 0;
static mut LAST_BAR_WHEEL_DIR: i8 = 0;

#[inline]
unsafe fn screen_dims() -> (i32, i32, i32, u32) {
    (
        crate::desktop_width().max(1),
        crate::desktop_height().max(1),
        crate::desktop_bar_height().max(1),
        crate::chrome_metrics().scale(),
    )
}

unsafe fn send_slide(cmd: SlideCommand) {
    let cur = &mut *core::ptr::addr_of_mut!(LAST_SLIDE);
    if *cur == cmd && !cmd.prepare_switch {
        return;
    }
    *cur = cmd;
    let w = cmd.wire();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_SPACE_SLIDE, w[0], w[1], w[2]);
}

unsafe fn send_snap_preview(st: SnapPreviewState) {
    let cur = &mut *core::ptr::addr_of_mut!(SNAP_PREVIEW);
    if *cur == st {
        return;
    }
    *cur = st;
    let w = st.wire();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_SNAP_PREVIEW, w[0], w[1], w[2]);
}

pub unsafe fn clear_snap_preview() {
    send_snap_preview(SnapPreviewState::HIDDEN);
}

unsafe fn pre_snap_save(fid: u32, w: u32, h: u32) {
    let table = &mut *core::ptr::addr_of_mut!(PRE_SNAP);
    if table.iter().flatten().any(|&(f, _, _)| f == fid) {
        return;
    }
    if let Some(slot) = table.iter_mut().find(|e| e.is_none()) {
        *slot = Some((fid, w, h));
    }
}

unsafe fn pre_snap_take(fid: u32) -> Option<(u32, u32)> {
    let table = &mut *core::ptr::addr_of_mut!(PRE_SNAP);
    for slot in table.iter_mut() {
        if let Some((f, w, h)) = *slot {
            if f == fid {
                *slot = None;
                return Some((w, h));
            }
        }
    }
    None
}

unsafe fn pre_snap_peek(fid: u32) -> Option<(u32, u32)> {
    (*core::ptr::addr_of!(PRE_SNAP))
        .iter()
        .flatten()
        .find(|&&(f, _, _)| f == fid)
        .map(|&(_, w, h)| (w, h))
}

unsafe fn frame_normal_size(fid: u32) -> Option<(u32, u32)> {
    (*core::ptr::addr_of!(crate::FRAMES))
        .iter()
        .flatten()
        .find(|f| f.frame_id == fid)
        .filter(|f| f.normal_w >= 160 && f.normal_h >= 120)
        .map(|f| (f.normal_w, f.normal_h))
}

/// Top Y below the Mission Control 5-Space bar for Overview window cards.
pub fn overview_top(screen_w: usize, screen_h: usize, bar_h: usize, scale: u32) -> usize {
    MissionControlLayout::new(screen_w, screen_h, bar_h, scale)
        .map_or(bar_h, |l| l.content_top)
}

pub unsafe fn is_mission_open() -> bool {
    (*core::ptr::addr_of!(crate::OVERVIEW)).is_open() || *core::ptr::addr_of!(MISSION_ONLY)
}

unsafe fn occupied_spaces_mask() -> u16 {
    let mut mask = 0u16;
    for f in (*core::ptr::addr_of!(crate::FRAMES)).iter().flatten() {
        if f.scene_id >= 9 || (f.flags & crate::FRAME_FLAG_MINIMIZED) != 0 {
            continue;
        }
        if let Some(tab) = f.tabs[f.active_tab as usize] {
            if crate::surface_is_alive(tab.surface_id) && !crate::is_tombstoned(tab.surface_id) {
                mask |= 1u16 << f.scene_id;
            }
        }
    }
    mask
}

/// Sends `OP_OVERVIEW_SPACES` to Display whenever Overview or empty Mission Control is open.
pub unsafe fn publish_overview_spaces() {
    let active_ui = (*core::ptr::addr_of!(crate::OVERVIEW)).is_open() || *core::ptr::addr_of!(MISSION_ONLY);
    if !active_ui {
        if core::mem::replace(&mut *core::ptr::addr_of_mut!(OVERVIEW_SHOWN), false) {
            let w = OverviewSpacesState::EMPTY.wire();
            let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_OVERVIEW_SPACES, w[0], w[1], w[2]);
        }
        return;
    }
    *core::ptr::addr_of_mut!(OVERVIEW_SHOWN) = true;
    let drag = *core::ptr::addr_of!(OVERVIEW_DRAG);
    let state = OverviewSpacesState {
        active_space: crate::ACTIVE_SCENE_IDX.min(crate::WORKSPACE_COUNT.saturating_sub(1)),
        occupied_mask: occupied_spaces_mask(),
        hover_space: *core::ptr::addr_of!(OVERVIEW_HOVER_SPACE),
        drag_sid: drag.filter(|d| d.dragging).map_or(0, |d| d.sid.min(u16::MAX as u64) as u16),
        drag_x: drag.filter(|d| d.dragging).map_or(0, |d| d.cur_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
        drag_y: drag.filter(|d| d.dragging).map_or(0, |d| d.cur_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
    };
    let w = state.wire();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_OVERVIEW_SPACES, w[0], w[1], w[2]);
}

/// Opens empty Mission Control strip when active Space has 0 windows so users can still jump Spaces.
pub unsafe fn open_empty_overview() -> bool {
    *core::ptr::addr_of_mut!(MISSION_ONLY) = true;
    *core::ptr::addr_of_mut!(OVERVIEW_DRAG) = None;
    *core::ptr::addr_of_mut!(OVERVIEW_HOVER_SPACE) = None;
    publish_overview_spaces();
    sex_pdx::serial_record!("[shell.overview.open] windows=0");
    true
}

pub unsafe fn close_mission_only() {
    if core::mem::replace(&mut *core::ptr::addr_of_mut!(MISSION_ONLY), false) {
        *core::ptr::addr_of_mut!(OVERVIEW_DRAG) = None;
        *core::ptr::addr_of_mut!(OVERVIEW_HOVER_SPACE) = None;
        publish_overview_spaces();
        sex_pdx::serial_record!("[shell.overview.close] keep=0 focus={}", crate::FOCUSED_SURFACE_ID);
    }
}

pub unsafe fn on_overview_closed() {
    *core::ptr::addr_of_mut!(OVERVIEW_DRAG) = None;
    *core::ptr::addr_of_mut!(OVERVIEW_HOVER_SPACE) = None;
    *core::ptr::addr_of_mut!(MISSION_ONLY) = false;
    publish_overview_spaces();
}

pub unsafe fn overview_key(code: u8, value: u64) -> bool {
    if !*core::ptr::addr_of!(MISSION_ONLY) {
        return false;
    }
    if matches!(code, 0x1D | 0x2A | 0x36 | 0x38 | 0x5B | 0x5C | 0x61 | 0x64) {
        return false;
    }
    if value == 1 && (code == 0x01 || code == 0x1C) {
        close_mission_only();
    }
    true
}

unsafe fn refresh_overview_after_drop() {
    let mut ordered = [0u64; sex_pdx::window_switcher::MAX_ENTRIES];
    let count = crate::switcher_windows(&mut ordered);
    let ov = &mut *core::ptr::addr_of_mut!(crate::OVERVIEW);
    if count > 0 && ov.open_grid(&ordered[..count], crate::FOCUSED_SURFACE_ID) {
        crate::switcher_send(ov.state());
        if let Some(l) = crate::overview_layout() {
            for (i, sid) in ordered[..count].iter().enumerate() {
                let (x, y, w, h) = l.card(i);
                sex_pdx::serial_record!("[shell.overview.card] index={} sid={} x={} y={} w={} h={}", i, sid, x, y, w, h);
            }
        }
        publish_overview_spaces();
    } else {
        let _ = ov.end(false);
        crate::switcher_send(sex_pdx::window_switcher::State::HIDDEN);
        open_empty_overview();
    }
}

/// Left button press while Overview / Mission Control is open.
pub unsafe fn overview_press(px: i32, py: i32) -> bool {
    if !is_mission_open() {
        return false;
    }
    let ov_open = (*core::ptr::addr_of!(crate::OVERVIEW)).is_open();
    let (w, h, bar, scale) = screen_dims();
    if px >= 0 && py >= 0 {
        if let Some(mc) = MissionControlLayout::new(w as usize, h as usize, bar as usize, scale) {
            if let Some(target_space) = mc.space_at(px as usize, py as usize) {
                if ov_open {
                    crate::overview_close(false);
                } else {
                    close_mission_only();
                }
                if target_space < crate::WORKSPACE_COUNT {
                    crate::switch_scene(target_space);
                }
                return true;
            }
        }
        if ov_open {
            if let Some(l) = crate::overview_layout() {
                if let Some(idx) = l.card_at(px as usize, py as usize) {
                    let sid = (*core::ptr::addr_of!(crate::OVERVIEW)).state().entries[idx] as u64;
                    if sid != 0 {
                        *core::ptr::addr_of_mut!(OVERVIEW_DRAG) = Some(OverviewDrag {
                            idx,
                            sid,
                            start_x: px,
                            start_y: py,
                            cur_x: px,
                            cur_y: py,
                            dragging: false,
                        });
                        return true;
                    }
                }
            }
        }
    }
    if ov_open {
        crate::overview_close(false);
    } else {
        close_mission_only();
    }
    true
}

/// Left button release while Overview / Mission Control is open. Returns true if consumed.
pub unsafe fn overview_release(px: i32, py: i32) -> bool {
    if !is_mission_open() {
        return false;
    }
    let Some(d) = (*core::ptr::addr_of_mut!(OVERVIEW_DRAG)).take() else {
        return false;
    };
    *core::ptr::addr_of_mut!(OVERVIEW_HOVER_SPACE) = None;
    if !d.dragging {
        let _ = (*core::ptr::addr_of_mut!(crate::OVERVIEW)).select(d.idx);
        crate::overview_close(true);
        return true;
    }
    let (w, h, bar, scale) = screen_dims();
    if px >= 0 && py >= 0 {
        if let Some(mc) = MissionControlLayout::new(w as usize, h as usize, bar as usize, scale) {
            if let Some(target_space) = mc.space_at(px as usize, py as usize) {
                if target_space < crate::WORKSPACE_COUNT {
                    if let Some(fid) = crate::frame_for_surface(d.sid) {
                        let cur_scene = (*core::ptr::addr_of!(crate::FRAMES))
                            .iter()
                            .flatten()
                            .find(|f| f.frame_id == fid)
                            .map(|f| f.scene_id);
                        if cur_scene.is_some_and(|s| s != target_space) {
                            let _ = crate::reparent_frame_to_scene(fid, target_space, "overview_drag");
                            sex_pdx::serial_record!(
                                "[shell.overview.drop] sid={} frame={} to={}",
                                d.sid,
                                fid,
                                target_space
                            );
                            refresh_overview_after_drop();
                            return true;
                        }
                    }
                }
            }
        }
    }
    publish_overview_spaces();
    true
}

/// Called right before `ACTIVE_SCENE_IDX` switches from `from` to `to` (even from SilkBar click).
pub unsafe fn before_switch_scene(from: u8, to: u8) {
    if core::mem::take(&mut *core::ptr::addr_of_mut!(SUPPRESS_NEXT_SWITCH)) {
        return;
    }
    let slide = &mut *core::ptr::addr_of_mut!(SLIDE);
    let cmd = slide.start_discrete(from, to, sex_pdx::get_ticks(), crate::MOTION_ON);
    send_slide(cmd);
}

/// Switches Spaces by sliding cleanly to an existing adjacent Space (`-1` or `+1`) within
/// `[0..DESKTOP_SPACE_COUNT)` if available, or rubber-bands slightly when at Space 1 or 5.
unsafe fn swipe_discrete(dir: i8) {
    let cur = crate::ACTIVE_SCENE_IDX;
    let bound = SpaceSlidePhysics::effective_bound(cur);
    let next = if dir > 0 && cur + 1 < bound {
        Some(cur + 1)
    } else if dir < 0 && cur > 0 {
        Some(cur - 1)
    } else {
        None
    };
    if let Some(target) = next {
        crate::switch_scene(target);
    } else if crate::MOTION_ON {
        let bump = if dir > 0 { -600 } else { 600 };
        let slide = &mut *core::ptr::addr_of_mut!(SLIDE);
        let _ = slide.apply_move(cur, bump, true);
        let (_, cmd) = slide.end_drag(cur, 0, sex_pdx::get_ticks(), true);
        send_slide(cmd);
    }
}

unsafe fn first_minimized_frame_in_scene() -> Option<u32> {
    let scene = crate::ACTIVE_SCENE_IDX;
    (*core::ptr::addr_of!(crate::FRAMES))
        .iter()
        .flatten()
        .find(|f| f.scene_id == scene && (f.flags & crate::FRAME_FLAG_MINIMIZED) != 0)
        .map(|f| f.frame_id)
}

/// Handles `OP_TOUCHPAD_GESTURE` sent by authenticated `sexinput`.
pub unsafe fn handle_gesture(caller_pd: u32, arg0: u64, arg1: u64, arg2: u64) -> bool {
    if caller_pd != sex_pdx::shell_ready::INPUT_PD_ID {
        return false;
    }
    if crate::lock::locked()
        || crate::ATLAS_MODE_ENABLED
        || (*core::ptr::addr_of!(crate::SCREENSHOT)).is_open()
    {
        return true;
    }
    let Some(msg) = GestureMsg::decode(arg0, arg1, arg2) else {
        return false;
    };
    match msg {
        GestureMsg::SwipeMove { dx_q12, .. } => {
            if is_mission_open() {
                return true;
            }
            if crate::silkbar_menus_glue::is_open() {
                crate::silkbar_menus_glue::close("gesture");
            }
            if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
                crate::tray_close();
            }
            let cmd = (*core::ptr::addr_of_mut!(SLIDE)).apply_move(
                crate::ACTIVE_SCENE_IDX,
                dx_q12,
                crate::MOTION_ON,
            );
            send_slide(cmd);
        }
        GestureMsg::SwipeEnd { vel_q12_per_s } => {
            let slide = &mut *core::ptr::addr_of_mut!(SLIDE);
            if !slide.dragging {
                return true;
            }
            let now = sex_pdx::get_ticks();
            let (committed, cmd) =
                slide.end_drag(crate::ACTIVE_SCENE_IDX, vel_q12_per_s, now, crate::MOTION_ON);
            if let Some(target) = committed {
                *core::ptr::addr_of_mut!(SUPPRESS_NEXT_SWITCH) = true;
                send_slide(cmd);
                crate::switch_scene(target);
            } else {
                send_slide(cmd);
            }
        }
        GestureMsg::SwipeLeft => {
            if !is_mission_open() {
                swipe_discrete(1);
            }
        }
        GestureMsg::SwipeRight => {
            if !is_mission_open() {
                swipe_discrete(-1);
            }
        }
        GestureMsg::SwipeUp => {
            if !is_mission_open() {
                let _ = crate::overview_toggle();
            }
        }
        GestureMsg::SwipeDown => {
            if (*core::ptr::addr_of!(crate::OVERVIEW)).is_open() {
                crate::overview_close(false);
            } else if *core::ptr::addr_of!(MISSION_ONLY) {
                close_mission_only();
            } else if let Some(fid) = first_minimized_frame_in_scene() {
                let _ = crate::restore_minimized_frame(fid);
            }
        }
    }
    true
}

/// Wheel / 2-finger scroll over SilkBar Workspace pills switches Desktop Spaces smoothly.
pub unsafe fn wheel_steps(steps: i8) -> bool {
    let (px, py) = (crate::POINTER_X, crate::POINTER_Y);
    let (w, _, bar, _) = screen_dims();
    if px < 0 || py < 0 || py >= bar {
        return false;
    }
    let m = crate::chrome_metrics();
    let mut model = DEFAULT_SILK_BAR;
    if !silkbar_model::reflow_chrome(
        &mut model,
        w as usize,
        bar as usize,
        m.px(sex_pdx::desktop_style::chrome::TARGET) as usize,
    ) {
        return false;
    }
    let ws0 = &model.layout[ModuleSlot::Workspace0 as usize];
    let ws_last = &model.layout[ModuleSlot::Workspace8 as usize];
    if ws0.w == 0 {
        return false;
    }
    let ux = px as usize;
    let right = if ws_last.w > 0 {
        ws_last.x + ws_last.w
    } else {
        ws0.x + ws0.w
    };
    if ux < ws0.x || ux >= right {
        return false;
    }
    if steps == 0 {
        return true;
    }
    let dir: i8 = if steps < 0 { 1 } else { -1 };
    let now = sex_pdx::get_ticks();
    if dir == *core::ptr::addr_of!(LAST_BAR_WHEEL_DIR)
        && now.saturating_sub(*core::ptr::addr_of!(LAST_BAR_WHEEL_MS)) < 90
    {
        return true;
    }
    *core::ptr::addr_of_mut!(LAST_BAR_WHEEL_DIR) = dir;
    *core::ptr::addr_of_mut!(LAST_BAR_WHEEL_MS) = now;
    let cur = crate::ACTIVE_SCENE_IDX;
    let next = if dir > 0 {
        (cur + 1).min(DESKTOP_SPACE_COUNT - 1)
    } else {
        cur.saturating_sub(1)
    };
    if next != cur {
        crate::switch_scene(next);
    }
    true
}

/// Applies `zone` cleanly to `frame_id` (`surface_id`), saving pre-snap bounds so dragging off
/// restores the previous size automatically.
unsafe fn apply_snap_to_frame(frame_id: u32, surface_id: u64, zone: SnapZone) -> bool {
    let (w, h, bar, _) = screen_dims();
    if zone == SnapZone::Top {
        let already = crate::frame_is_zoomed(frame_id);
        let ok = if !already { crate::zoom_frame(frame_id) } else { false };
        if ok {
            sex_pdx::serial_record!("[silk.snap.apply] sid={} frame={} kind=top", surface_id, frame_id);
        }
        return ok;
    }
    let Some((nx, ny, nw, nh)) = snap_rect(zone, w, h, bar) else {
        return false;
    };
    if pre_snap_peek(frame_id).is_none() {
        if crate::frame_is_zoomed(frame_id) {
            if let Some((nw, nh)) = frame_normal_size(frame_id) {
                pre_snap_save(frame_id, nw, nh);
            }
        } else if let Some((_, _, cw, ch)) = crate::get_surface_bounds(surface_id) {
            pre_snap_save(frame_id, cw, ch);
        }
    }
    if crate::frame_is_zoomed(frame_id) {
        crate::set_frame_zoomed(frame_id, false);
    }
    crate::set_frame_floating(frame_id, true);
    let _ = crate::pdx_call(
        sex_pdx::SLOT_DISPLAY,
        0xEC,
        surface_id,
        ((ny as u32 as u64) << 32) | (nx as u32 as u64),
        ((nh as u64) << 32) | (nw as u64),
    );
    crate::update_local_geometry(surface_id, nx, ny, nw, nh);
    sex_pdx::serial_record!(
        "[silk.snap.apply] sid={} frame={} kind={} x={} y={} w={} h={}",
        surface_id,
        frame_id,
        zone.name(),
        nx,
        ny,
        nw,
        nh
    );
    true
}

/// Snaps or toggles maximize on the currently focused frame (SilkBar quick-layout / menus).
pub unsafe fn snap_focused(zone: SnapZone) -> bool {
    let Some(fid) = crate::selected_frame_id() else {
        return false;
    };
    if zone == SnapZone::Top {
        return crate::toggle_zoom_frame(fid);
    }
    let Some(sid) = crate::active_surface_for_frame(fid) else {
        return false;
    };
    if !crate::surface_is_alive(sid) {
        return false;
    }
    let ok = apply_snap_to_frame(fid, sid, zone);
    if ok {
        crate::retile_around_floating(sid);
    }
    ok
}

/// Moves focused frame (or active dragged frame!) to `target_space` and optionally follows it.
pub unsafe fn move_focused_to_space(target_space: u8, follow: bool) -> bool {
    if target_space >= crate::WORKSPACE_COUNT {
        return false;
    }
    if matches!(crate::INTERACTION, crate::InteractionState::Dragging { .. }) && follow {
        return carry_drag_to_space(target_space);
    }
    let Some(fid) = crate::selected_frame_id() else {
        return false;
    };
    let ok = crate::reparent_frame_to_scene(fid, target_space, "space_move");
    if ok && follow {
        crate::switch_scene(target_space);
        if let Some(sid) = crate::active_surface_for_frame(fid) {
            let _ = crate::try_set_focus(sid);
        }
    }
    ok
}

pub unsafe fn move_focused_step(dir: i8, follow: bool) -> bool {
    let cur = crate::ACTIVE_SCENE_IDX;
    let bound = SpaceSlidePhysics::effective_bound(cur).min(crate::WORKSPACE_COUNT);
    let target = if dir > 0 {
        (cur + 1) % bound
    } else if cur == 0 {
        bound - 1
    } else {
        cur - 1
    };
    if target == cur {
        return false;
    }
    move_focused_to_space(target, follow)
}

/// Handles clicks on the 4 compact focused-window quick-layout icon buttons in SilkBar.
pub unsafe fn handle_silkbar_click(px: i32, py: i32) -> bool {
    let (w, _, bar, scale) = screen_dims();
    if px < 0 || py < 0 || py >= bar || crate::FOCUSED_SURFACE_ID == 0 {
        return false;
    }
    let m = crate::chrome_metrics();
    let mut model = DEFAULT_SILK_BAR;
    if !silkbar_model::reflow_chrome(
        &mut model,
        w as usize,
        bar as usize,
        m.px(sex_pdx::desktop_style::chrome::TARGET) as usize,
    ) {
        return false;
    }
    let ws0 = &model.layout[ModuleSlot::Workspace0 as usize];
    let ws_full = model.layout[ModuleSlot::Workspace1 as usize].w > 0;
    let btns = quick_layout_rects(bar as usize, scale, ws0.x, ws_full, true);
    let (ux, uy) = (px as usize, py as usize);
    for (btn, r) in btns {
        if r.2 > 0 && sex_pdx::space_slide::rect_contains(r, ux, uy) {
            sex_pdx::serial_record!("[shell.silkbar.quick_layout] btn={} x={} y={}", btn.name(), ux, uy);
            match btn {
                QuickLayoutBtn::TileLeft => {
                    let _ = snap_focused(SnapZone::Left);
                }
                QuickLayoutBtn::TileRight => {
                    let _ = snap_focused(SnapZone::Right);
                }
                QuickLayoutBtn::MaxFloat => {
                    let _ = snap_focused(SnapZone::Top);
                }
                QuickLayoutBtn::NextSpace => {
                    let cur = crate::ACTIVE_SCENE_IDX;
                    let next = (cur + 1) % DESKTOP_SPACE_COUNT;
                    let _ = move_focused_to_space(next, true);
                }
            }
            return true;
        }
    }
    false
}

/// Moves the actively dragged window across Spaces while keeping live drag capture intact.
pub unsafe fn carry_drag_to_space(target_space: u8) -> bool {
    let crate::InteractionState::Dragging { surface_id, .. } = crate::INTERACTION else {
        return false;
    };
    if target_space >= crate::WORKSPACE_COUNT || target_space == crate::ACTIVE_SCENE_IDX {
        return false;
    }
    let Some(fid) = crate::frame_for_surface(surface_id) else {
        return false;
    };
    if !crate::surface_is_alive(surface_id) {
        return false;
    }
    clear_snap_preview();
    // Save bounds before reparenting clears drag state internally.
    let saved_bounds = crate::get_surface_bounds(surface_id);
    if !crate::reparent_frame_to_scene(fid, target_space, "drag_carry") {
        return false;
    }
    crate::switch_scene(target_space);
    if !crate::frame_is_zoomed(fid) {
        crate::set_frame_floating(fid, true);
    }
    if let Some((_, _, bw, bh)) = saved_bounds {
        let (sw, sh, bar, _) = screen_dims();
        let rx = (crate::POINTER_X - bw as i32 / 2).clamp(0, (sw - bw as i32).max(0));
        let ry = (crate::POINTER_Y - crate::chrome_metrics().title() as i32 / 2)
            .clamp(bar, (sh - bh as i32).max(bar));
        crate::apply_window_geometry(surface_id, rx, ry, bw, bh);
    }
    let _ = crate::try_set_focus(surface_id);
    crate::try_transition(crate::InteractionState::Dragging {
        surface_id,
        current_x: crate::POINTER_X,
        current_y: crate::POINTER_Y,
    });
    sex_pdx::serial_record!(
        "[shell.drag.carry_space] sid={} frame={} to={}",
        surface_id,
        fid,
        target_space
    );
    true
}

/// Restores a zoomed/snapped window back to its normal floating dimensions when dragged off-edge.
pub unsafe fn maybe_unsnap_on_drag_move(sid: u64, px: i32, py: i32) -> bool {
    let Some(fid) = crate::frame_for_surface(sid) else {
        return false;
    };
    let zoomed = crate::frame_is_zoomed(fid);
    let saved = pre_snap_peek(fid);
    if !zoomed && saved.is_none() {
        return false;
    }
    let (sw, sh, bar, scale) = screen_dims();
    let zone = detect_snap_zone(px, py, sw, sh, bar, scale);
    // Do not un-snap while the pointer hasn't left the top-bar band of a zoomed window.
    if zoomed && zone == SnapZone::Top {
        return false;
    }
    let (nw, nh) = if zoomed {
        frame_normal_size(fid).unwrap_or(((sw as u32 * 3) / 5, (sh.saturating_sub(bar) as u32 * 7) / 10))
    } else if let Some((pw, ph)) = pre_snap_take(fid) {
        (pw, ph)
    } else {
        return false;
    };
    if zoomed {
        crate::set_frame_zoomed(fid, false);
    }
    crate::set_frame_floating(fid, true);
    let title_mid = (crate::chrome_metrics().title() as i32 / 2).max(12);
    let nx = (px - nw as i32 / 2).clamp(0, (sw - nw as i32).max(0));
    let ny = (py - title_mid).clamp(bar, (sh - nh as i32).max(bar));
    crate::apply_window_geometry(sid, nx, ny, nw, nh);
    sex_pdx::serial_record!(
        "[shell.snap.restore_on_drag] sid={} frame={} w={} h={}",
        sid,
        fid,
        nw,
        nh
    );
    true
}

/// Called on pointer motion (`EV_ABS`, `EV_REL`, `OP_USB_MOUSE_REPORT`) to drive
/// Overview card drag-and-drop, frosted Noir snap preview, and edge-carry arming.
pub unsafe fn on_pointer_move(px: i32, py: i32) {
    let (w, h, bar, scale) = screen_dims();
    if is_mission_open() {
        let mut dirty = false;
        let next_hover = if px >= 0 && py >= 0 {
            MissionControlLayout::new(w as usize, h as usize, bar as usize, scale)
                .and_then(|mc| mc.space_at(px as usize, py as usize))
        } else {
            None
        };
        let prev_hover = &mut *core::ptr::addr_of_mut!(OVERVIEW_HOVER_SPACE);
        if *prev_hover != next_hover {
            *prev_hover = next_hover;
            dirty = true;
        }
        if let Some(d) = &mut *core::ptr::addr_of_mut!(OVERVIEW_DRAG) {
            if (px - d.start_x).abs().max((py - d.start_y).abs()) >= 6 {
                d.dragging = true;
            }
            if d.dragging && (d.cur_x != px || d.cur_y != py) {
                d.cur_x = px;
                d.cur_y = py;
                dirty = true;
            }
        }
        if dirty {
            publish_overview_spaces();
        }
        return;
    }

    if let crate::InteractionState::Dragging { surface_id, .. } = crate::INTERACTION {
        let now = sex_pdx::get_ticks();
        if let Some(dir) = (*core::ptr::addr_of_mut!(EDGE_CARRY)).update(px, w, scale, now) {
            let cur = crate::ACTIVE_SCENE_IDX;
            let bound = SpaceSlidePhysics::effective_bound(cur).min(DESKTOP_SPACE_COUNT);
            let next = if dir > 0 && cur + 1 < bound {
                Some(cur + 1)
            } else if dir < 0 && cur > 0 {
                Some(cur - 1)
            } else {
                None
            };
            if let Some(target) = next {
                let _ = carry_drag_to_space(target);
                return;
            }
        }
        let zone = detect_snap_zone(px, py, w, h, bar, scale);
        let preview = match snap_rect(zone, w, h, bar) {
            Some((sx, sy, sw, sh)) => SnapPreviewState {
                zone,
                x: sx.max(0) as u16,
                y: sy.max(0) as u16,
                w: sw.min(u16::MAX as u32) as u16,
                h: sh.min(u16::MAX as u32) as u16,
            },
            None => SnapPreviewState::HIDDEN,
        };
        send_snap_preview(preview);
        let _ = surface_id;
    } else {
        clear_snap_preview();
        (*core::ptr::addr_of_mut!(EDGE_CARRY)).reset();
    }
}

/// Evaluates snap zone on drag release (replacing / extending `try_snap_on_drag_release`).
pub unsafe fn on_drag_release(surface_id: u64, px: i32, py: i32) -> bool {
    clear_snap_preview();
    (*core::ptr::addr_of_mut!(EDGE_CARRY)).reset();
    if !crate::surface_is_alive(surface_id) || crate::is_tombstoned(surface_id) {
        serial_println!("[silk.snap.reject.dead] sid={} reason=dead", surface_id);
        return false;
    }
    let Some(frame_id) = crate::frame_for_surface(surface_id) else {
        serial_println!("[silk.snap.none] sid={} reason=no_frame x={} y={}", surface_id, px, py);
        return false;
    };
    if crate::frame_is_minimized(frame_id) {
        serial_println!("[silk.snap.reject.dead] sid={} reason=minimized", surface_id);
        return false;
    }
    let (w, h, bar, scale) = screen_dims();
    let zone = detect_snap_zone(px, py, w, h, bar, scale);
    match zone {
        SnapZone::None => {
            serial_println!("[silk.snap.none] sid={} x={} y={}", surface_id, px, py);
            false
        }
        SnapZone::Top => {
            serial_println!("[silk.snap.hit.top] sid={} frame={} x={} y={}", surface_id, frame_id, px, py);
            if !crate::frame_is_zoomed(frame_id) && crate::zoom_frame(frame_id) {
                serial_println!("[silk.snap.apply] sid={} frame={} kind=top", surface_id, frame_id);
                return true;
            }
            serial_println!("[silk.snap.none] sid={} reason=top_zoom_skip x={} y={}", surface_id, px, py);
            false
        }
        other => {
            serial_println!(
                "[silk.snap.hit.{}] sid={} frame={} x={} y={}",
                other.name(),
                surface_id,
                frame_id,
                px,
                py
            );
            apply_snap_to_frame(frame_id, surface_id, other)
        }
    }
}

/// Main loop tick: advances Space slide physics settle & fires edge-carry spring-load timer.
pub unsafe fn poll() {
    let now = sex_pdx::get_ticks();
    let _ = (*core::ptr::addr_of_mut!(SLIDE)).step(now);

    if let crate::InteractionState::Dragging { .. } = crate::INTERACTION {
        let (w, _, _, scale) = screen_dims();
        if let Some(dir) = (*core::ptr::addr_of_mut!(EDGE_CARRY)).update(crate::POINTER_X, w, scale, now) {
            let cur = crate::ACTIVE_SCENE_IDX;
            let bound = SpaceSlidePhysics::effective_bound(cur).min(DESKTOP_SPACE_COUNT);
            let next = if dir > 0 && cur + 1 < bound {
                Some(cur + 1)
            } else if dir < 0 && cur > 0 {
                Some(cur - 1)
            } else {
                None
            };
            if let Some(target) = next {
                let _ = carry_drag_to_space(target);
            }
        }
    } else {
        clear_snap_preview();
        (*core::ptr::addr_of_mut!(EDGE_CARRY)).reset();
    }
}

/// Optional idle wake interval (hooked into `tray_wake()`): ensures edge-carry spring-load
/// triggers cleanly within ~350ms even if the user holds the mouse stationary against the screen edge.
pub unsafe fn wake() -> Option<u64> {
    if matches!(crate::INTERACTION, crate::InteractionState::Dragging { .. }) {
        let (w, _, _, scale) = screen_dims();
        let thresh = (3 * scale.clamp(1, 3) as i32).max(2);
        let at_edge = crate::POINTER_X <= thresh || crate::POINTER_X >= w - 1 - thresh;
        if at_edge {
            return Some(SPRING_LOAD_EDGE_MS.min(48));
        }
    }
    None
}
