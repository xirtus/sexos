// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `decorations.rs` (slice S1): title bar buttons (close,
//! maximise, float), double-click to maximise, and the window menu a right
//! click on a title bar opens. Every action runs the window manager's own
//! path (the frame light close, `toggle_zoom_frame`, the Super+Shift+V float
//! toggle, `reparent_frame_to_scene`); dragging a title still moves the window
//! by the existing drag and snap code. Kept out of main.rs so main.rs only
//! holds hook lines.

use crate::decorations::{self, Action, Button, DoubleClick, Press, TitleHit, WindowMenu};

static mut MENU: WindowMenu = WindowMenu::new();
static mut DOUBLE: DoubleClick = DoubleClick::new();
/// The last frame geometry reported (frame, sid, x, y, w, h, scale).
static mut REPORTED: Option<(u32, u64, i32, i32, u32, u32, u32)> = None;

fn scale() -> u32 { crate::chrome_metrics().scale() }

/// The window whose title bar is at (x, y): (frame, active surface, hit).
unsafe fn title_at(x: i32, y: i32) -> Option<(u32, u64, TitleHit)> {
    // Atlas, the overview and a screenshot selection own the pointer while they show.
    if crate::ATLAS_MODE_ENABLED || (*core::ptr::addr_of!(crate::OVERVIEW)).is_open() || (*core::ptr::addr_of!(crate::SCREENSHOT)).is_open() { return None; }
    let frame = match crate::hit_test_at(x, y) {
        crate::HitTarget::FrameChrome { frame_id, kind } if kind == crate::FRAME_CHROME_RIM || kind == crate::FRAME_CHROME_TAB_STRIP => frame_id,
        _ => return None,
    };
    if !crate::frame_has_top_bar(frame) { return None; }
    let sid = crate::active_surface_for_frame(frame)?;
    let (sx, sy, sw, sh) = crate::get_surface_bounds(sid)?;
    match decorations::title_hit(crate::chrome_metrics(), sw, sh, x - sx, y - sy) {
        TitleHit::Outside => None,
        hit => Some((frame, sid, hit)),
    }
}

/// Whether the window menu shows (it takes every press while it does).
pub unsafe fn menu_shown() -> bool { (*core::ptr::addr_of!(MENU)).is_open() }

unsafe fn menu_send() {
    let word = (*core::ptr::addr_of!(MENU)).wire();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, sex_pdx::window_menu::OP_WINDOW_MENU, word, 0, 0);
}

unsafe fn menu_close(reason: &str) {
    if (*core::ptr::addr_of_mut!(MENU)).close() {
        menu_send();
        sex_pdx::serial_record!("[shell.decor.menu] open=0 reason={}", reason);
    }
}

unsafe fn menu_open(frame: u32, sid: u64, x: i32, y: i32) {
    let flags = crate::wm_frame_flags(frame).unwrap_or(0);
    let current = (*core::ptr::addr_of!(crate::FRAMES)).iter().flatten().find(|f| f.frame_id == frame).map_or(crate::ACTIVE_SCENE_IDX, |f| f.scene_id);
    let menu = sex_pdx::window_menu::Menu::new(x, y, crate::frame_is_zoomed(frame), flags & crate::FRAME_FLAG_FLOATING != 0,
        crate::WORKSPACE_COUNT.min(sex_pdx::window_menu::MAX_WORKSPACES), current);
    let (w, h) = (crate::desktop_width().max(0) as usize, crate::desktop_height().max(0) as usize);
    // A screen that cannot hold the menu gets none, rather than an invisible
    // modal menu that swallows clicks.
    let Some(l) = sex_pdx::window_menu::Layout::new(&menu, w, h, scale()) else {
        sex_pdx::serial_record!("[shell.decor.menu] open=0 frame={} sid={} reason=no_room", frame, sid);
        return;
    };
    (*core::ptr::addr_of_mut!(MENU)).open(frame, menu);
    menu_send();
    sex_pdx::serial_record!("[shell.decor.menu] open=1 frame={} sid={} rows={} x={} y={} w={} h={} row_h={} pad={}",
        frame, sid, l.rows, l.panel.0, l.panel.1, l.panel.2, l.panel.3, l.row_h, l.pad);
}

/// Runs a button's or a menu row's action on `frame` with the existing paths.
unsafe fn act(frame: u32, action: Action, via: &str) {
    let Some(sid) = crate::active_surface_for_frame(frame) else {
        sex_pdx::serial_record!("[shell.decor.action] frame={} sid=0 action={} via={} ok=0", frame, action.name(), via);
        return;
    };
    // Any action but Close is on the window the person is working with.
    if action != Action::Close && crate::FOCUSED_SURFACE_ID != sid { let _ = crate::try_set_focus(sid); }
    let ok = match action {
        Action::Close => crate::is_closeable_surface(sid) && crate::close_surface_from_frame_light(sid),
        Action::Minimise => crate::minimize_frame(frame),
        Action::ToggleMaximise => crate::toggle_zoom_frame(frame),
        // The same toggle as Super+Shift+V.
        Action::ToggleFloat => {
            let floating = crate::wm_frame_flags(frame).map_or(false, |f| f & crate::FRAME_FLAG_FLOATING != 0);
            if crate::frame_is_zoomed(frame) { crate::toggle_zoom_frame(frame); }
            crate::set_frame_floating(frame, !floating);
            if !floating { crate::wm_place_floating(frame, 60); }
            crate::tile_active_scene_frames();
            true
        }
        Action::MoveToWorkspace(n) => n < crate::WORKSPACE_COUNT && crate::reparent_frame_to_scene(frame, n, "window_menu"),
    };
    match action {
        Action::MoveToWorkspace(n) => { sex_pdx::serial_record!("[shell.decor.action] frame={} sid={} action=workspace to={} via={} ok={}", frame, sid, n, via, ok as u8); }
        _ => { sex_pdx::serial_record!("[shell.decor.action] frame={} sid={} action={} via={} ok={} zoomed={} floating={}", frame, sid, action.name(), via, ok as u8,
            crate::frame_is_zoomed(frame) as u8, crate::wm_frame_flags(frame).map_or(0, |f| (f & crate::FRAME_FLAG_FLOATING != 0) as u8)); }
    }
}

/// A left press, from the click path before its own hit test. True when the
/// decorations took it: a menu row or a dismissing press, a close, maximise
/// or float button, or the second press of a double click on a title bar.
/// Minimise, tabs and title drags keep the existing path.
pub unsafe fn press(x: i32, y: i32) -> bool {
    let menus = &mut *core::ptr::addr_of_mut!(MENU);
    if menus.is_open() {
        let (w, h) = (crate::desktop_width().max(0) as usize, crate::desktop_height().max(0) as usize);
        match menus.press(x, y, w, h, scale()) {
            Press::Inside => {}
            Press::Dismiss => { menu_send(); sex_pdx::serial_record!("[shell.decor.menu] open=0 reason=outside"); }
            Press::Choose { frame, action } => {
                menu_send();
                sex_pdx::serial_record!("[shell.decor.menu] open=0 reason=chose action={}", action.name());
                act(frame, action, "menu");
            }
        }
        return true;
    }
    let Some((frame, _, hit)) = title_at(x, y) else { (*core::ptr::addr_of_mut!(DOUBLE)).reset(); return false };
    match hit {
        TitleHit::Button(Button::Minimise) => { (*core::ptr::addr_of_mut!(DOUBLE)).reset(); false }
        TitleHit::Button(b) => {
            (*core::ptr::addr_of_mut!(DOUBLE)).reset();
            act(frame, decorations::button_action(b), "button");
            true
        }
        TitleHit::Title => {
            let slop = crate::chrome_metrics().px(decorations::DOUBLE_CLICK_SLOP as u32) as i32;
            if (*core::ptr::addr_of_mut!(DOUBLE)).press(frame, sex_pdx::get_ticks(), x, y, slop) {
                act(frame, Action::ToggleMaximise, "double_click");
                return true;
            }
            false
        }
        TitleHit::Outside => false,
    }
}

/// Every pointer button, before programs see it. A right press on a title
/// bar opens the window menu; while the menu shows, programs get no button.
pub unsafe fn button(button: u8, pressed: bool) -> bool {
    let open = (*core::ptr::addr_of!(MENU)).is_open();
    if button == 2 && pressed {
        if open { menu_close("right_click"); return true; }
        let (x, y) = (crate::POINTER_X, crate::POINTER_Y);
        let Some((frame, sid, _)) = title_at(x, y) else { return false };
        if crate::FOCUSED_SURFACE_ID != sid { let _ = crate::try_set_focus(sid); }
        menu_open(frame, sid, x, y);
        return true;
    }
    open
}

/// Main loop: the menu follows the pointer and closes with its window, and
/// the focused window's title bar geometry is reported when it changes.
pub unsafe fn poll() {
    let menus = &mut *core::ptr::addr_of_mut!(MENU);
    if let Some(frame) = menus.frame() {
        let alive = crate::active_surface_for_frame(frame).map_or(false, |sid| crate::surface_is_alive(sid)) && crate::frame_accepts_input(frame);
        if !alive { menu_close("window_gone"); }
        else {
            let (w, h) = (crate::desktop_width().max(0) as usize, crate::desktop_height().max(0) as usize);
            if menus.hover(crate::POINTER_X, crate::POINTER_Y, w, h, scale()) { menu_send(); }
        }
    }
    // Reported once a drag or resize has settled, not at every step.
    if matches!(crate::INTERACTION, crate::InteractionState::Dragging { .. } | crate::InteractionState::Resizing { .. } | crate::InteractionState::TabDragging { .. }) { return; }
    let sid = crate::FOCUSED_SURFACE_ID;
    let Some(frame) = crate::frame_for_surface(sid) else { return };
    if !crate::frame_has_top_bar(frame) { return; }
    let Some((x, y, w, h)) = crate::get_surface_bounds(sid) else { return };
    let now = Some((frame, sid, x, y, w, h, scale()));
    if *core::ptr::addr_of!(REPORTED) == now { return; }
    REPORTED = now;
    let m = crate::chrome_metrics();
    let c = |b: Button| { let (cx, cy) = decorations::button_centre(m, b); (x + cx as i32, y + cy as i32) };
    let (close, max, float) = (c(Button::Close), c(Button::Maximise), c(Button::Float));
    sex_pdx::serial_record!("[shell.decor.frame] frame={} sid={} x={} y={} w={} h={} title_h={} title_x={} close={},{} max={},{} float={},{}",
        frame, sid, x, y, w, h, m.title(), x + m.title_x() as i32, close.0, close.1, max.0, max.1, float.0, float.1);
}
