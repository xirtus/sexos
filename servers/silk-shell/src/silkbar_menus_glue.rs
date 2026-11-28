// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for interactive macOS/KDE-style SilkBar dropdown menus,
//! right-side status popovers, and desktop right-click context menus.
//!
//! Kept in a separate module so `servers/silk-shell/src/main.rs` only holds
//! minimal hook calls (preserving the passive-wait source fingerprint).

use sex_pdx::silkbar_menus::{
    Controller, MenuAction, MenuKind, MenuState, PressResult, TopBarTrigger,
    FLAG_CHARGING, FLAG_FLOATING, FLAG_GROUPED, FLAG_MAXIMIZED, FLAG_MUTED, FLAG_ON_AC,
    OP_SILKBAR_MENU, PROFILE_BALANCED, PROFILE_PERFORMANCE, PROFILE_QUIET,
    rect_contains, top_bar_menu_rects,
};
use silkbar_model::{DEFAULT_SILK_BAR, ModuleSlot};

static mut CTRL: Controller = Controller::new();
static mut POWER_PROFILE: u8 = PROFILE_BALANCED;

fn scale() -> u32 {
    crate::chrome_metrics().scale()
}

unsafe fn send_menu() {
    let (w0, w1) = (*core::ptr::addr_of!(CTRL)).wire();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_SILKBAR_MENU, w0, w1, 0);
}

pub unsafe fn is_open() -> bool {
    (*core::ptr::addr_of!(CTRL)).is_open() || crate::wifi_popover_glue::is_open()
}

pub unsafe fn close(reason: &str) {
    if (*core::ptr::addr_of_mut!(CTRL)).close() {
        send_menu();
        sex_pdx::serial_record!("[shell.silkbar_menu] open=0 reason={}", reason);
    }
    crate::wifi_popover_glue::close(reason);
}

pub unsafe fn display_restarted() {
    if (*core::ptr::addr_of!(CTRL)).is_open() {
        send_menu();
    }
}

pub unsafe fn launch_settings_view() {
    if !launch_named_program("tatami", true) {
        crate::toggle_scene_settings_panel();
    }
}

/// Populates a `MenuState` with live tray, window, and uptime metadata.
unsafe fn build_state(kind: MenuKind, anchor_x: i32, anchor_y: i32) -> MenuState {
    let mut st = MenuState::new(kind, anchor_x, anchor_y);
    let tray = *core::ptr::addr_of!(crate::TRAY);
    let mut flags = 0u8;
    if let Some(fid) = crate::selected_frame_id() {
        if crate::frame_is_zoomed(fid) {
            flags |= FLAG_MAXIMIZED;
        }
        if crate::wm_frame_flags(fid).map_or(false, |f| f & crate::FRAME_FLAG_FLOATING != 0) {
            flags |= FLAG_FLOATING;
        }
        if crate::layouts_glue::tab_count(fid) > 1 {
            flags |= FLAG_GROUPED;
        }
    }
    if tray.muted {
        flags |= FLAG_MUTED;
    }
    if tray.on_ac == Some(true) {
        flags |= FLAG_ON_AC;
    }
    if tray.charging {
        flags |= FLAG_CHARGING;
    }
    st.flags = flags;
    st = st.with_power_profile(*core::ptr::addr_of!(POWER_PROFILE));
    st.volume = tray.volume.min(100);
    st.battery = tray.battery.unwrap_or(0xFF);
    st.wifi_phase = tray.wifi;
    st.uptime_mins = (sex_pdx::get_ticks() / 60_000) as u32;
    st.active_space = crate::ACTIVE_SCENE_IDX.min(7);
    st
}

/// Opens the Noir WindowContext menu at `(x, y)` (from Ctrl+Click / Ctrl+RightClick on titlebar).
pub unsafe fn open_window_context(x: i32, y: i32) {
    if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
        crate::tray_close();
    }
    let st = build_state(MenuKind::WindowContext, x, y);
    (*core::ptr::addr_of_mut!(CTRL)).open(st);
    send_menu();
    sex_pdx::serial_record!(
        "[shell.silkbar_menu] open=1 kind=window_context x={} y={}",
        st.x,
        st.y
    );
}

/// Opens (or toggles closed if already open) a top-bar dropdown or status popover.
pub unsafe fn open_or_toggle_top(kind: MenuKind, anchor_x: i32) {
    crate::bg_apps_glue::close("menu");
    let y = crate::desktop_bar_height() + 4 * scale() as i32;
    if kind == MenuKind::WifiPopover {
        if crate::wifi_popover_glue::is_open() {
            close("toggle");
            return;
        }
        if (*core::ptr::addr_of_mut!(CTRL)).close() {
            send_menu();
        }
        if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
            crate::tray_close();
        }
        crate::wifi_popover_glue::open(anchor_x, y);
        return;
    }
    crate::wifi_popover_glue::close("switch");
    let cur = (*core::ptr::addr_of!(CTRL)).state();
    if cur.is_some_and(|s| s.kind == kind) {
        close("toggle");
        return;
    }
    if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
        crate::tray_close();
    }
    let st = build_state(kind, anchor_x, y);
    (*core::ptr::addr_of_mut!(CTRL)).open(st);
    send_menu();
    sex_pdx::serial_record!(
        "[shell.silkbar_menu] open=1 kind={} x={} y={}",
        kind as u8,
        st.x,
        st.y
    );
}

/// Whether `(ux, uy)` is on the bar's Bluetooth icon (shown while a controller is there).
unsafe fn bluetooth_icon_at(ux: usize, uy: usize) -> bool {
    let m = crate::chrome_metrics();
    let mut bar = DEFAULT_SILK_BAR;
    if !silkbar_model::reflow_chrome(&mut bar, crate::desktop_width().max(0) as usize, m.bar() as usize,
        m.px(sex_pdx::desktop_style::chrome::TARGET) as usize) { return false; }
    let c = &bar.layout[ModuleSlot::Chip0 as usize];
    crate::bt_panel::icon_hit((c.x, c.y, c.w, c.h), ux, uy)
}

/// Computes which top-bar menu trigger (if any) is under `(ux, uy)`.
unsafe fn top_bar_trigger_at(ux: usize, uy: usize) -> Option<(TopBarTrigger, i32)> {
    let m = crate::chrome_metrics();
    let bar_h = m.bar() as usize;
    if uy >= bar_h {
        return None;
    }
    let mut bar = DEFAULT_SILK_BAR;
    if !silkbar_model::reflow_chrome(
        &mut bar,
        crate::desktop_width().max(0) as usize,
        bar_h,
        m.px(sex_pdx::desktop_style::chrome::TARGET) as usize,
    ) {
        return None;
    }
    let s = m.scale() as usize;

    // 1. Left SexOS / System pill (Launcher slot).
    let launch = &bar.layout[ModuleSlot::Launcher as usize];
    if launch.w > 0 && rect_contains((launch.x, launch.y, launch.w, launch.h), ux, uy) {
        return Some((TopBarTrigger::System, launch.x as i32));
    }

    // 2. Top-bar File / Edit / View / Window triggers.
    let ws0 = &bar.layout[ModuleSlot::Workspace0 as usize];
    if bar.layout[ModuleSlot::Workspace1 as usize].w > 0 && ws0.w > 0 {
        let app_pill_right = 114 * s;
        for (trig, r) in top_bar_menu_rects(bar_h, m.scale(), app_pill_right, ws0.x) {
            if r.2 > 0 && rect_contains(r, ux, uy) {
                return Some((trig, r.0 as i32));
            }
        }
    }

    // 3. Right-side Wi-Fi (Chip0) and Battery (Chip2) status pills.
    //    (Chip1 = Volume uses tray::Menu::Quick and Clock uses tray::Menu::Calendar
    //     to preserve tray_gate.py invariants.)
    let chip0 = &bar.layout[ModuleSlot::Chip0 as usize];
    if chip0.w > 0 && rect_contains((chip0.x, chip0.y, chip0.w, chip0.h), ux, uy) {
        return Some((TopBarTrigger::Wifi, (chip0.x + chip0.w / 2) as i32));
    }
    let chip2 = &bar.layout[ModuleSlot::Chip2 as usize];
    if chip2.w > 0 && rect_contains((chip2.x, chip2.y, chip2.w, chip2.h), ux, uy) {
        return Some((TopBarTrigger::Battery, (chip2.x + chip2.w / 2) as i32));
    }
    None
}

/// Intercepts left-clicks in the SilkBar for System pill, File/Edit/View/Window menus,
/// and Wi-Fi / Battery popovers. Returns true when handled.
pub unsafe fn handle_top_bar_click(px: i32, py: i32) -> bool {
    if px < 0 || py < 0 || py >= crate::desktop_bar_height() {
        return false;
    }
    // The Bluetooth icon, left of the network chip: opens the Bluetooth panel.
    if bluetooth_icon_at(px as usize, py as usize) {
        if is_open() { close("bar_bluetooth"); }
        sex_pdx::serial_record!("[shell.bar.bluetooth] click=1");
        return crate::bt_panel::toggle();
    }
    let Some((trig, anchor_x)) = top_bar_trigger_at(px as usize, py as usize) else {
        // Clicking another bar slot (workspace, clock, volume) closes any open dropdown.
        if is_open() {
            close("bar_other");
        }
        return false;
    };
    open_or_toggle_top(trig.menu_kind(), anchor_x);
    true
}

unsafe fn launch_named_program(name: &str, network: bool) -> bool {
    let Some(packed) = sex_pdx::program_store::pack_name(name.as_bytes()) else {
        return false;
    };
    let ok = crate::program_start_own(packed);
    if ok && network {
        crate::PROGRAM_NETWORK = true;
    }
    ok
}

pub unsafe fn open_settings_from_ui() -> bool {
    if !launch_named_program("tatami", true) {
        crate::toggle_scene_settings_panel();
    }
    true
}

unsafe fn dispatch_action(action: MenuAction) {
    use sex_pdx::sound_mixer as mixer;
    match action {
        MenuAction::None => {}
        MenuAction::AboutSexOs => {
            sex_pdx::serial_record!("[shell.silkbar_menu.action] action=about_sexos");
            crate::bg_apps_glue::open_about();
        }
        MenuAction::LaunchBrowser => {
            if !launch_named_program("kaleidoscope", true) {
                let _ = crate::open_browser_in_active_scene();
            }
        }
        MenuAction::LaunchTerminal | MenuAction::NewWindow => {
            if !launch_named_program("spindle", true) {
                let _ = crate::open_spindle_in_active_scene();
            }
        }
        MenuAction::LaunchTailor => {
            let _ = launch_named_program("tailor", true);
        }
        MenuAction::LaunchFlash => {
            let _ = launch_named_program("flash", false);
        }
        MenuAction::LaunchPdGraph => {
            let _ = crate::open_mesh_in_active_scene();
        }
        MenuAction::LaunchFiles => {
            if !launch_named_program("loom", false) {
                let _ = crate::open_linen_in_active_scene();
            }
        }
        MenuAction::LaunchSettings | MenuAction::DisplaySettings | MenuAction::NetworkSettings => {
            let _ = open_settings_from_ui();
        }
        MenuAction::LaunchAppStore => {
            sex_pdx::serial_record!("[shell.silkbar_menu.action] action=app_store");
            let _ = launch_named_program("sexshop", true);
        }
        MenuAction::OpenAppsSwitcher => {
            sex_pdx::serial_record!("[shell.silkbar_menu.action] action=apps_switcher");
            crate::bg_apps_glue::open_switcher();
        }
        MenuAction::ForceQuitApps => {
            sex_pdx::serial_record!("[shell.silkbar_menu.action] action=force_quit");
            crate::bg_apps_glue::open_force_quit();
        }
        MenuAction::LockScreen => {
            sex_pdx::serial_record!("[shell.silkbar_menu.action] action=lock_screen");
            let _ = crate::lock::lock();
        }
        MenuAction::RefreshDesktop => {
            crate::sync_scene_visibility();
            crate::tile_active_scene_frames();
        }
        MenuAction::Reboot => {
            let _ = crate::palette_power(crate::Command::Restart);
        }
        MenuAction::Shutdown => {
            let _ = crate::palette_power(crate::Command::ShutDown);
        }
        MenuAction::CloseWindow => {
            let sid = crate::FOCUSED_SURFACE_ID;
            if sid != 0 && crate::is_closeable_surface(sid) {
                let _ = crate::bg_apps_glue::close_window(sid);
            }
        }
        MenuAction::QuitApp => {
            let sid = crate::FOCUSED_SURFACE_ID;
            if sid != 0 {
                let _ = crate::bg_apps_glue::quit_surface(sid);
            }
        }
        MenuAction::MinimizeWindow => {
            if let Some(fid) = crate::selected_frame_id() {
                let _ = crate::minimize_frame(fid);
            }
        }
        MenuAction::ToggleMaximizeWindow => {
            if let Some(fid) = crate::selected_frame_id() {
                let _ = crate::toggle_zoom_frame(fid);
            }
        }
        MenuAction::ToggleFloatWindow => {
            if let Some(fid) = crate::selected_frame_id() {
                let floating = crate::wm_frame_flags(fid).map_or(false, |f| f & crate::FRAME_FLAG_FLOATING != 0);
                if crate::frame_is_zoomed(fid) {
                    let _ = crate::toggle_zoom_frame(fid);
                }
                crate::set_frame_floating(fid, !floating);
                if !floating {
                    crate::wm_place_floating(fid, 60);
                }
                crate::tile_active_scene_frames();
            }
        }
        MenuAction::TileLeft => {
            let _ = crate::space_slide_glue::snap_focused(sex_pdx::space_slide::SnapZone::Left);
        }
        MenuAction::TileRight => {
            let _ = crate::space_slide_glue::snap_focused(sex_pdx::space_slide::SnapZone::Right);
        }
        MenuAction::MoveToNextSpace => {
            let _ = crate::space_slide_glue::move_focused_step(1, true);
        }
        MenuAction::MoveToPrevSpace => {
            let _ = crate::space_slide_glue::move_focused_step(-1, true);
        }
        MenuAction::MoveToSpace(n) => {
            let _ = crate::space_slide_glue::move_focused_to_space(n, true);
        }
        MenuAction::BringToFront => {
            if let Some(fid) = crate::selected_frame_id() {
                if let Some(sid) = crate::active_surface_for_frame(fid) {
                    let _ = crate::try_set_focus(sid);
                }
            }
        }
        MenuAction::GroupWindow => {
            let _ = crate::layouts_glue::action(crate::wm::WmAction::Group);
        }
        MenuAction::UngroupTab => {
            let _ = crate::layouts_glue::action(crate::wm::WmAction::Ungroup);
        }
        MenuAction::NextTab => {
            let _ = crate::layouts_glue::action(crate::wm::WmAction::NextTab);
        }
        MenuAction::PrevTab => {
            let _ = crate::layouts_glue::action(crate::wm::WmAction::PrevTab);
        }
        MenuAction::EditCopy | MenuAction::EditPaste | MenuAction::EditSelectAll => {
            sex_pdx::serial_record!("[shell.silkbar_menu.action] action={}", action.name());
        }
        MenuAction::WifiRefresh => {
            crate::tray_refresh();
            if let Some(cur) = (*core::ptr::addr_of!(CTRL)).state() {
                let refreshed = build_state(cur.kind, cur.x as i32, cur.y as i32);
                (*core::ptr::addr_of_mut!(CTRL)).open(refreshed);
                send_menu();
            }
        }
        MenuAction::VolumeMuteToggle => {
            crate::TRAY.muted = !crate::TRAY.muted;
            crate::TRAY_OWED_MUTE = !crate::TRAY_OWED_MUTE;
            crate::tray_send_owed();
            crate::tray_publish();
            if let Some(cur) = (*core::ptr::addr_of!(CTRL)).state() {
                let refreshed = build_state(cur.kind, cur.x as i32, cur.y as i32);
                (*core::ptr::addr_of_mut!(CTRL)).open(refreshed);
                send_menu();
            }
        }
        MenuAction::VolumeDown | MenuAction::VolumeUp | MenuAction::VolumeSet(_) => {
            let next_vol = match action {
                MenuAction::VolumeDown => crate::TRAY.volume.saturating_sub(10),
                MenuAction::VolumeUp => crate::TRAY.volume.saturating_add(10).min(100),
                MenuAction::VolumeSet(v) => v.min(100),
                _ => crate::TRAY.volume,
            };
            let _ = mixer::SET_MASTER;
            crate::TRAY.volume = next_vol;
            crate::TRAY_OWED_VOLUME = Some(next_vol);
            crate::tray_send_owed();
            crate::tray_publish();
            if let Some(cur) = (*core::ptr::addr_of!(CTRL)).state() {
                let refreshed = build_state(cur.kind, cur.x as i32, cur.y as i32);
                (*core::ptr::addr_of_mut!(CTRL)).open(refreshed);
                send_menu();
            }
        }
        MenuAction::PowerProfileQuiet => {
            POWER_PROFILE = PROFILE_QUIET;
            let _ = crate::laptop_command(crate::Command::PowerSaver);
            if let Some(cur) = (*core::ptr::addr_of!(CTRL)).state() {
                let refreshed = build_state(cur.kind, cur.x as i32, cur.y as i32);
                (*core::ptr::addr_of_mut!(CTRL)).open(refreshed);
                send_menu();
            }
        }
        MenuAction::PowerProfileBalanced => {
            POWER_PROFILE = PROFILE_BALANCED;
            let _ = crate::laptop_command(crate::Command::PowerBalanced);
            if let Some(cur) = (*core::ptr::addr_of!(CTRL)).state() {
                let refreshed = build_state(cur.kind, cur.x as i32, cur.y as i32);
                (*core::ptr::addr_of_mut!(CTRL)).open(refreshed);
                send_menu();
            }
        }
        MenuAction::PowerProfilePerformance => {
            POWER_PROFILE = PROFILE_PERFORMANCE;
            let _ = crate::laptop_command(crate::Command::PowerPerformance);
            if let Some(cur) = (*core::ptr::addr_of!(CTRL)).state() {
                let refreshed = build_state(cur.kind, cur.x as i32, cur.y as i32);
                (*core::ptr::addr_of_mut!(CTRL)).open(refreshed);
                send_menu();
            }
        }
        MenuAction::OpenCalendar => {
            close("calendar");
            crate::tray_toggle(sex_pdx::tray::Menu::Calendar);
        }
        MenuAction::OpenSpotlight => {
            close("spotlight");
            crate::spotlight_glue::open();
        }
    }
}

/// Handles a left press below the bar while a SilkBar menu/popover/context menu is open.
pub unsafe fn press(px: i32, py: i32) -> bool {
    if crate::wifi_popover_glue::is_open() {
        return crate::wifi_popover_glue::press(px, py);
    }
    let ctrl = &mut *core::ptr::addr_of_mut!(CTRL);
    if !ctrl.is_open() {
        return false;
    }
    if py < crate::desktop_bar_height() {
        return false;
    }
    let (w, h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
    );
    match ctrl.press(px, py, w, h, scale()) {
        PressResult::Inside => {}
        PressResult::Dismiss => {
            send_menu();
            sex_pdx::serial_record!("[shell.silkbar_menu] open=0 reason=outside");
        }
        PressResult::Action { action, closed } => {
            if closed {
                send_menu();
                sex_pdx::serial_record!("[shell.silkbar_menu] open=0 reason=chose action={}", action.name());
            }
            dispatch_action(action);
        }
    }
    true
}

/// Handles secondary mouse button (right-click) on the desktop background.
pub unsafe fn button(button: u8, pressed: bool) -> bool {
    let open = is_open();
    if button == 2 && pressed {
        let (x, y) = (crate::POINTER_X, crate::POINTER_Y);
        if crate::ATLAS_MODE_ENABLED
            || (*core::ptr::addr_of!(crate::OVERVIEW)).is_open()
            || (*core::ptr::addr_of!(crate::SCREENSHOT)).is_open()
        {
            return open;
        }
        // Right-clicking the desktop background opens the frosted Noir Desktop Context Menu.
        if y >= crate::desktop_bar_height() && matches!(crate::hit_test_at(x, y), crate::HitTarget::None) {
            crate::bg_apps_glue::close("desktop_context");
            if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
                crate::tray_close();
            }
            crate::wifi_popover_glue::close("right_click");
            let st = build_state(MenuKind::DesktopContext, x, y);
            (*core::ptr::addr_of_mut!(CTRL)).open(st);
            send_menu();
            sex_pdx::serial_record!("[shell.silkbar_menu] open=1 kind=desktop_context x={} y={}", x, y);
            return true;
        }
        if open {
            close("right_click");
            return true;
        }
    }
    open
}

/// Keyboard input while a SilkBar dropdown, Wi-Fi popover, Noir sheet, or context menu is open.
pub unsafe fn key(code: u8, value: u64) -> bool {
    if crate::bg_apps_glue::key(code, value) {
        return true;
    }
    if crate::wifi_popover_glue::key(code, value) {
        return true;
    }
    if !is_open() || code != 0x01 {
        return false;
    }
    if value == 1 {
        close("escape");
    }
    true
}

/// Main loop poll: updates row hover highlight and performs macOS-style top-bar hover switching.
pub unsafe fn poll() {
    crate::wifi_popover_glue::poll();
    let (px, py) = (crate::POINTER_X, crate::POINTER_Y);
    let bar_h = crate::desktop_bar_height();
    if crate::wifi_popover_glue::is_open() {
        if py >= 0 && py < bar_h && px >= 0 && !crate::wifi_popover_glue::is_password_focused() {
            if let Some((trig, anchor_x)) = top_bar_trigger_at(px as usize, py as usize) {
                if trig != TopBarTrigger::Wifi {
                    crate::wifi_popover_glue::close("hover");
                    let y = bar_h + 4 * scale() as i32;
                    let st = build_state(trig.menu_kind(), anchor_x, y);
                    (*core::ptr::addr_of_mut!(CTRL)).open(st);
                    send_menu();
                }
            }
        }
        return;
    }
    let ctrl = &mut *core::ptr::addr_of_mut!(CTRL);
    if !ctrl.is_open() {
        return;
    }
    // macOS-style hover-switching across top-bar menu items while a top-bar menu is open.
    if py >= 0 && py < bar_h && px >= 0 {
        if let Some((trig, anchor_x)) = top_bar_trigger_at(px as usize, py as usize) {
            if trig == TopBarTrigger::Wifi {
                if ctrl.close() {
                    send_menu();
                }
                let y = bar_h + 4 * scale() as i32;
                crate::wifi_popover_glue::open(anchor_x, y);
                return;
            }
            if ctrl.hover_top_bar(trig, anchor_x, bar_h, scale()) {
                send_menu();
                return;
            }
        }
    }
    let (w, h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
    );
    if ctrl.hover(px, py, w, h, scale()) {
        send_menu();
    }
}
