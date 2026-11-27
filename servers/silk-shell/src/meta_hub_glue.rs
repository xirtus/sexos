// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for the Meta Control Center & Super-Launcher Bento Hub (`sex_pdx::meta_hub`).
//!
//! Coordinates live system telemetry (Display Brightness via `sexacpi`, Master Volume /
//! Speaker Mute / Mic Mute via `SLOT_BELL`, Wi-Fi status & SSID via `wifi_popover_glue`,
//! Bluetooth state via `bt_panel`, Performance / G-Mode thermal profile, Night Light,
//! macOS Noir Accent Tints, Do Not Disturb, Workspace switcher, Media transport, and
//! 6-tile Quick Launcher) plus pointer click/drag/hover/wheel handling and Voyeur/Spotlight
//! super-user quick commands.

use sex_config::id;
use sex_pdx::desktop_look as look;
use sex_pdx::meta_hub::{
    self, DragSlider, HitTarget, Layout, MODE_COMMAND_PALETTE, MODE_SPOTLIGHT, MetaHubState,
    OP_META_HUB, POWER_BALANCED, POWER_GMODE, POWER_NAMES, POWER_PERFORMANCE, POWER_SAVER,
    QuickCommand,
};
use sex_pdx::power as pwr;
use sex_pdx::sound_mixer as mixer;
use silkbar_model::UpdateKind;

struct Glue {
    state: MetaHubState,
    last_sent: MetaHubState,
    sent_once: bool,
    dirty: bool,
    last_sensor_poll: u64,
    brightness_hold_until: u64,
    profile_hold_until: u64,
    last_drag_hw_send: u64,
    last_pointer: (i32, i32),
    owed_mic_mute: Option<bool>,
}

static mut GLUE: Glue = Glue {
    state: MetaHubState::DEFAULT,
    last_sent: MetaHubState::DEFAULT,
    sent_once: false,
    dirty: true,
    last_sensor_poll: 0,
    brightness_hold_until: 0,
    profile_hold_until: 0,
    last_drag_hw_send: 0,
    last_pointer: (-1, -1),
    owed_mic_mute: None,
};

#[allow(static_mut_refs)]
fn glue() -> &'static mut Glue {
    unsafe { &mut *core::ptr::addr_of_mut!(GLUE) }
}

fn scale() -> u32 {
    crate::chrome_metrics().scale().clamp(1, 3)
}

unsafe fn current_anchor() -> (bool, u8, (i16, i16, u16, u16)) {
    if crate::spotlight_glue::is_open() {
        (true, MODE_SPOTLIGHT, crate::spotlight_glue::panel_rect())
    } else if crate::COMMAND_PALETTE_OPEN {
        let r = crate::command_palette_geometry();
        (
            true,
            MODE_COMMAND_PALETTE,
            (r.x as i16, r.y as i16, r.w as u16, r.h as u16),
        )
    } else {
        (false, MODE_COMMAND_PALETTE, (0, 0, 0, 0))
    }
}

unsafe fn current_layout() -> Option<Layout> {
    let g = glue();
    if !g.state.visible {
        return None;
    }
    let w = crate::desktop_width().max(0) as usize;
    let h = crate::desktop_height().max(0) as usize;
    let bar = crate::desktop_bar_height().max(0) as usize;
    Layout::compute(w, h, bar, scale(), g.state.mode, g.state.anchor)
}

/// Captures live audio mixer word (`volume`, `speaker_muted`, `mic_gain`, `mic_muted`).
pub unsafe fn sync_mixer_word(word: u64) {
    let g = glue();
    g.state.volume_pct = mixer::state_volume(word).min(100);
    g.state.speaker_muted = mixer::state_muted(word);
    g.state.mic_gain_pct = mixer::state_mic_volume(word).min(100);
    g.state.mic_muted = mixer::state_mic_muted(word);
    g.dirty = true;
}

/// Consumes any queued microphone mute toggle when `tray_send_owed` dispatches to Bell.
pub unsafe fn take_owed_mic_mute() -> Option<bool> {
    glue().owed_mic_mute.take()
}

/// Marks hub telemetry dirty after external workspace/look/theme changes.
pub unsafe fn mark_dirty() {
    glue().dirty = true;
}

/// True when Focus / Do Not Disturb is active and should quiet normal notification popups.
pub unsafe fn dnd_suppress_banner(urgency: u8) -> bool {
    let g = glue();
    g.state.dnd_enabled && urgency < 2
}

unsafe fn refresh_state(force_sensors: bool) {
    let g = glue();
    let now = sex_pdx::get_ticks();
    let (visible, mode, anchor) = current_anchor();
    g.state.visible = visible;
    g.state.mode = mode;
    g.state.anchor = anchor;

    if !visible {
        g.state.hover_target = 0;
        g.state.active_drag_slider = DragSlider::None;
        return;
    }

    // Audio & media playback status
    if (*core::ptr::addr_of!(crate::TRAY_OWED_VOLUME)).is_none() {
        g.state.volume_pct = crate::TRAY.volume.min(100);
    }
    if !crate::TRAY_OWED_MUTE {
        g.state.speaker_muted = crate::TRAY.muted;
    }
    let any_playing = (*core::ptr::addr_of!(crate::SOUND_PANEL))
        .shown()
        .any(|(_, _, muted, playing, _)| playing && !muted);
    if any_playing {
        g.state.media_playing = true;
    }

    // Wi-Fi summary + lazy background SSID fetch
    crate::wifi_popover_glue::refresh_background();
    let (wifi_on, wifi_conn, wifi_bars, ssid) = crate::wifi_popover_glue::summary();
    g.state.wifi_enabled = wifi_on;
    g.state.wifi_connected = wifi_conn;
    g.state.wifi_rssi_bars = wifi_bars;
    g.state.set_ssid(ssid);

    // Bluetooth summary
    let (bt_avail, bt_on, bt_conn) = crate::bt_panel::summary();
    g.state.bt_available = bt_avail;
    g.state.bt_enabled = bt_on;
    g.state.bt_connected_count = bt_conn;

    // Look, accent, workspace & app counts
    g.state.night_light_level = crate::DESKTOP_LOOK.1 % meta_hub::NIGHT_LEVELS;
    g.state.noir_accent = crate::ACTIVE_TINT_IDX % meta_hub::ACCENT_COUNT;
    g.state.active_space = crate::ACTIVE_SCENE_IDX.min(4);
    let (open_w, parked_w) = crate::bg_apps_glue::summary_counts();
    g.state.open_windows = open_w;
    g.state.parked_apps = parked_w;

    // ACPI sensors (throttled every 400ms while visible, or on open)
    if force_sensors || now.saturating_sub(g.last_sensor_poll) >= 400 {
        g.last_sensor_poll = now;
        if let Ok(s) = pwr::sensors() {
            if now >= g.brightness_hold_until && g.state.active_drag_slider != DragSlider::Brightness {
                if let Some(b) = s.brightness {
                    g.state.brightness_pct = b.min(100);
                }
            }
            if let Some(bp) = s.battery_percent {
                g.state.battery_known = true;
                g.state.battery_pct = bp.min(100);
                g.state.charging = s.charging;
            } else {
                g.state.battery_known = false;
            }
            if now >= g.profile_hold_until {
                if let Some(prof) = s.profile {
                    if g.state.power_mode != POWER_GMODE {
                        g.state.power_mode = match prof {
                            pwr::PROFILE_SAVER => POWER_SAVER,
                            pwr::PROFILE_PERFORMANCE => POWER_PERFORMANCE,
                            _ => POWER_BALANCED,
                        };
                    }
                }
            }
        }
    }
}

unsafe fn sync_display(force: bool) {
    let g = glue();
    if !force && g.sent_once && g.state == g.last_sent {
        return;
    }

    if !g.state.visible {
        if g.sent_once && !g.last_sent.visible && !force {
            return;
        }
        let (w0, w1, w2) = g.state.pack_state();
        if crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_META_HUB, w0, w1, w2).0 == 0 {
            g.last_sent = g.state;
            g.sent_once = true;
            g.dirty = false;
        }
        return;
    }

    let need_anchor = force
        || !g.sent_once
        || g.state.anchor != g.last_sent.anchor
        || g.state.mode != g.last_sent.mode
        || g.state.battery_pct != g.last_sent.battery_pct
        || g.state.battery_known != g.last_sent.battery_known
        || g.state.charging != g.last_sent.charging;
    if need_anchor {
        let (a0, a1, a2) = g.state.pack_anchor();
        if crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_META_HUB, a0, a1, a2).0 != 0 {
            return;
        }
    }

    let need_ssid = force
        || !g.sent_once
        || g.state.ssid != g.last_sent.ssid
        || g.state.ssid_len != g.last_sent.ssid_len
        || g.state.bt_connected_count != g.last_sent.bt_connected_count
        || g.state.open_windows != g.last_sent.open_windows
        || g.state.parked_apps != g.last_sent.parked_apps;
    if need_ssid {
        let (s0, s1, s2) = g.state.pack_ssid_telemetry();
        if crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_META_HUB, s0, s1, s2).0 != 0 {
            return;
        }
    }

    let (w0, w1, w2) = g.state.pack_state();
    if crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_META_HUB, w0, w1, w2).0 == 0 {
        g.last_sent = g.state;
        g.sent_once = true;
        g.dirty = false;
    }
}

/// Immediately updates Meta Control Hub open/closed visibility alongside Palette or Spotlight.
pub unsafe fn sync_visibility() {
    let was = glue().state.visible;
    refresh_state(!was);
    sync_display(false);
    let now_vis = glue().state.visible;
    if now_vis != was {
        sex_pdx::serial_record!(
            "[shell.meta_hub] visible={} mode={}",
            now_vis as u8,
            glue().state.mode
        );
    }
}

/// Display restarted: re-pushes current Meta Hub overlay if visible.
pub unsafe fn display_restarted() {
    let g = glue();
    g.sent_once = false;
    refresh_state(true);
    if g.state.visible {
        sync_display(true);
    }
}

/// Main-loop poll: updates hover highlight and keeps live telemetry in sync while open.
pub unsafe fn poll() {
    let g = glue();
    let (visible, _, _) = current_anchor();
    if !visible && !g.state.visible {
        return;
    }
    refresh_state(false);
    if g.state.visible {
        let p = (crate::POINTER_X, crate::POINTER_Y);
        if p != g.last_pointer || g.dirty {
            g.last_pointer = p;
            if let Some(l) = current_layout() {
                g.state.hover_target = l.hit(p.0, p.1).hover_code();
            }
        }
    }
    sync_display(false);
}

// ── Live Hardware / Desktop Control Actions ─────────────────────────────────

unsafe fn close_launchers() {
    if crate::COMMAND_PALETTE_OPEN {
        crate::toggle_command_palette();
    }
    if crate::spotlight_glue::is_open() {
        crate::spotlight_glue::close("meta_hub");
    }
    sync_visibility();
}

unsafe fn set_brightness(pct: u8, send_hw: bool) {
    let pct = pct.min(100);
    let g = glue();
    let now = sex_pdx::get_ticks();
    g.state.brightness_pct = pct;
    g.brightness_hold_until = now + 800;
    if send_hw {
        g.last_drag_hw_send = now;
        let r = pwr::control(pwr::CTL_BRIGHTNESS_SET, pct);
        sex_pdx::serial_record!("[shell.meta_hub.brightness] pct={} ok={}", pct, r.is_ok() as u8);
    }
    sync_display(false);
}

unsafe fn set_volume(pct: u8, commit: bool) {
    let pct = pct.min(100);
    let g = glue();
    let now = sex_pdx::get_ticks();
    g.state.volume_pct = pct;
    g.state.speaker_muted = false;
    crate::TRAY.volume = pct;
    crate::TRAY.muted = false;
    if commit {
        g.last_drag_hw_send = now;
        crate::TRAY_OWED_VOLUME = Some(pct);
        crate::tray_send_owed();
        crate::config_watch::changed_here(id::VOLUME, pct as i32);
        sex_pdx::serial_record!("[shell.meta_hub.volume] pct={}", pct);
    }
    crate::tray_publish();
    sync_display(false);
}

unsafe fn set_speaker_mute(mute: bool) {
    let g = glue();
    if crate::TRAY.muted != mute {
        crate::TRAY.muted = mute;
        crate::TRAY_OWED_MUTE = !crate::TRAY_OWED_MUTE;
        crate::tray_send_owed();
    }
    g.state.speaker_muted = mute;
    crate::tray_publish();
    sync_display(false);
    sex_pdx::serial_record!("[shell.meta_hub.mute] speaker={}", mute as u8);
}

unsafe fn set_mic_mute(mute: bool) {
    let g = glue();
    g.state.mic_muted = mute;
    g.owed_mic_mute = Some(mute);
    crate::tray_send_owed();
    sync_display(false);
    sex_pdx::serial_record!("[shell.meta_hub.mute] mic={}", mute as u8);
}

unsafe fn set_power_mode(mode: u8) {
    let mode = mode % 4;
    let g = glue();
    let now = sex_pdx::get_ticks();
    g.state.power_mode = mode;
    g.profile_hold_until = now + 1500;

    let (cpu_prof, thermal_prof) = match mode {
        POWER_SAVER => (pwr::PROFILE_SAVER, pwr::THERMAL_QUIET),
        POWER_PERFORMANCE => (pwr::PROFILE_PERFORMANCE, pwr::THERMAL_PERFORMANCE),
        POWER_GMODE => (pwr::PROFILE_PERFORMANCE, pwr::THERMAL_GMODE),
        _ => (pwr::PROFILE_BALANCED, pwr::THERMAL_BALANCED),
    };
    let r_cpu = pwr::control(pwr::CTL_PROFILE, cpu_prof);
    let r_th = pwr::control(pwr::CTL_THERMAL_PROFILE, thermal_prof);
    sync_display(false);
    sex_pdx::serial_record!(
        "[shell.meta_hub.power] mode={} cpu_ok={} thermal_ok={}",
        POWER_NAMES[mode as usize],
        r_cpu.is_ok() as u8,
        r_th.is_ok() as u8
    );
}

unsafe fn set_night_light(level: u8) {
    let lvl = level % meta_hub::NIGHT_LEVELS;
    let wp = crate::DESKTOP_LOOK.0;
    if crate::desktop_look_apply(wp, lvl) {
        crate::config_watch::changed_here(id::NIGHT_LIGHT, lvl as i32);
    }
    glue().state.night_light_level = crate::DESKTOP_LOOK.1 % meta_hub::NIGHT_LEVELS;
    sync_display(false);
    sex_pdx::serial_record!("[shell.meta_hub.night_light] level={}", lvl);
}

unsafe fn set_noir_accent(idx: u8) {
    let idx = idx % meta_hub::ACCENT_COUNT;
    crate::ACTIVE_TINT_IDX = idx;
    crate::apply_custom_tint_bundle(idx as usize);
    let tokens = crate::resolve_scene_render_tokens();
    crate::push_token_preset(&tokens);
    crate::send_silkbar_phase2_update(UpdateKind::SetTintAccent as u32, idx as u64, 0);
    if crate::config_watch::value(id::ACCENT) != look::DEFAULT_ACCENT as i32 {
        crate::config_watch::changed_here(id::ACCENT, look::DEFAULT_ACCENT as i32);
    }
    let _ = crate::desktop_look_apply(crate::DESKTOP_LOOK.0, crate::DESKTOP_LOOK.1);
    glue().state.noir_accent = idx;
    sync_display(false);
    sex_pdx::serial_record!(
        "[shell.meta_hub.accent] idx={} name={}",
        idx,
        meta_hub::ACCENT_NAMES[idx as usize]
    );
}

unsafe fn set_dnd(enabled: bool) {
    let g = glue();
    g.state.dnd_enabled = enabled;
    sync_display(false);
    sex_pdx::serial_record!("[shell.meta_hub.dnd] enabled={}", enabled as u8);
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

unsafe fn run_quick_launch(idx: usize) {
    match idx {
        0 => {
            close_launchers();
            let _ = crate::open_spindle_in_active_scene();
        }
        1 => {
            close_launchers();
            let _ = crate::silkbar_menus_glue::open_settings_from_ui();
        }
        2 => {
            close_launchers();
            let _ = launch_named_program("sexshop", true);
        }
        3 => {
            crate::wifi_popover_glue::open_from_meta_hub();
        }
        4 => {
            close_launchers();
            let _ = crate::lock::lock();
        }
        _ => {
            close_launchers();
            crate::screenshot_open();
        }
    }
    sex_pdx::serial_record!("[shell.meta_hub.launch] idx={}", idx);
}

/// Executes a typed quick-control command from Voyeur (Meta Launcher) or Spotlight.
pub unsafe fn execute_quick_command(cmd: QuickCommand) -> bool {
    refresh_state(false);
    match cmd {
        QuickCommand::VolumeSet(pct) => set_volume(pct, true),
        QuickCommand::VolumeStep(step) => {
            let cur = glue().state.volume_pct as i32;
            let next = (cur + step as i32).clamp(0, 100) as u8;
            set_volume(next, true);
        }
        QuickCommand::MuteSet(m) => set_speaker_mute(m),
        QuickCommand::MicMuteToggle => {
            let cur = glue().state.mic_muted;
            set_mic_mute(!cur);
        }
        QuickCommand::MicMuteSet(m) => set_mic_mute(m),
        QuickCommand::BrightnessSet(pct) => set_brightness(pct, true),
        QuickCommand::BrightnessStep(step) => {
            let cur = glue().state.brightness_pct as i32;
            let next = (cur + step as i32).clamp(5, 100) as u8;
            set_brightness(next, true);
        }
        QuickCommand::WifiSet(on) => {
            crate::wifi_popover_glue::set_radio(on);
            glue().state.wifi_enabled = on;
            sync_display(false);
        }
        QuickCommand::WifiScan => {
            crate::wifi_popover_glue::open_from_meta_hub();
        }
        QuickCommand::BtSet(on) => {
            let _ = crate::bt_panel::set_scan(on);
            refresh_state(false);
            sync_display(false);
        }
        QuickCommand::BtPanel => {
            close_launchers();
            let _ = crate::bt_panel::toggle();
        }
        QuickCommand::PowerMode(m) => set_power_mode(m),
        QuickCommand::NightLightCycle => {
            let next = (crate::DESKTOP_LOOK.1 + 1) % meta_hub::NIGHT_LEVELS;
            set_night_light(next);
        }
        QuickCommand::NightLightSet(lvl) => set_night_light(lvl),
        QuickCommand::NoirAccentCycle => {
            let next = (crate::ACTIVE_TINT_IDX + 1) % meta_hub::ACCENT_COUNT;
            set_noir_accent(next);
        }
        QuickCommand::NoirAccentSet(idx) => set_noir_accent(idx),
        QuickCommand::DndToggle => {
            let cur = glue().state.dnd_enabled;
            set_dnd(!cur);
        }
        QuickCommand::DndSet(on) => set_dnd(on),
        QuickCommand::LockScreen => {
            close_launchers();
            return crate::lock::lock();
        }
        QuickCommand::Screenshot => {
            close_launchers();
            crate::screenshot_open();
        }
        QuickCommand::SwitchSpace(idx) => {
            crate::switch_scene(idx);
            refresh_state(false);
            sync_display(false);
        }
    }
    true
}

// ── Pointer Click, Drag, Right-Click & Wheel Handling ──────────────────────

/// Left-button press handler (runs before Spotlight dismiss / window focus).
pub unsafe fn press(px: i32, py: i32) -> bool {
    let Some(l) = current_layout() else {
        return false;
    };
    let hit = l.hit(px, py);
    if hit == HitTarget::Outside {
        return false;
    }

    let g = glue();
    g.state.hover_target = hit.hover_code();
    match hit {
        HitTarget::Outside => return false,
        HitTarget::Panel => {}
        HitTarget::SpacePill => {
            let next = (crate::ACTIVE_SCENE_IDX + 1) % crate::WORKSPACE_COUNT.max(1);
            crate::switch_scene(next);
            refresh_state(false);
            sync_display(false);
        }
        HitTarget::TileWifi => {
            crate::wifi_popover_glue::toggle_radio();
            g.state.wifi_enabled = !g.state.wifi_enabled;
            sync_display(false);
        }
        HitTarget::TileWifiExpand => {
            crate::wifi_popover_glue::open_from_meta_hub();
        }
        HitTarget::TileBluetooth => {
            close_launchers();
            let _ = crate::bt_panel::toggle();
        }
        HitTarget::TilePower => {
            let next = (g.state.power_mode + 1) % 4;
            set_power_mode(next);
        }
        HitTarget::TileNightLight => {
            let next = (crate::DESKTOP_LOOK.1 + 1) % meta_hub::NIGHT_LEVELS;
            set_night_light(next);
        }
        HitTarget::TileAccent => {
            let next = (crate::ACTIVE_TINT_IDX + 1) % meta_hub::ACCENT_COUNT;
            set_noir_accent(next);
        }
        HitTarget::TileDnd => {
            let next = !g.state.dnd_enabled;
            set_dnd(next);
        }
        HitTarget::BtnLock => {
            close_launchers();
            let _ = crate::lock::lock();
        }
        HitTarget::BtnShot => {
            close_launchers();
            crate::screenshot_open();
        }
        HitTarget::SliderBrightness => {
            g.state.active_drag_slider = DragSlider::Brightness;
            let pct = Layout::percent_at(l.slider_brightness, px);
            set_brightness(pct, true);
        }
        HitTarget::BtnSpeakerMute => {
            let next = !g.state.speaker_muted;
            set_speaker_mute(next);
        }
        HitTarget::SliderVolume => {
            g.state.active_drag_slider = DragSlider::Volume;
            let pct = Layout::percent_at(l.slider_volume, px);
            set_volume(pct, true);
        }
        HitTarget::BtnMicMute => {
            let next = !g.state.mic_muted;
            set_mic_mute(next);
        }
        HitTarget::MediaPrev => {
            crate::player_key(crate::media_keys::KEY_PREVIOUS);
        }
        HitTarget::MediaPlayPause => {
            g.state.media_playing = !g.state.media_playing;
            crate::player_key(crate::media_keys::KEY_PLAY_PAUSE);
            sync_display(false);
        }
        HitTarget::MediaNext => {
            crate::player_key(crate::media_keys::KEY_NEXT);
        }
        other => {
            if let Some(qidx) = other.quick_launch_index() {
                run_quick_launch(qidx);
            }
        }
    }
    true
}

/// Mouse button hook inside `screenshot_button` (handles right-click on Wi-Fi tile and
/// ends slider drags on left release without intercepting normal `ClickPending -> Idle`).
pub unsafe fn button(btn: u8, pressed: bool) -> bool {
    let g = glue();
    if !g.state.visible {
        return false;
    }
    if btn == 1 && !pressed {
        let _ = release(crate::POINTER_X, crate::POINTER_Y);
        return false;
    }
    if btn == 2 && pressed {
        if let Some(l) = current_layout() {
            let hit = l.hit(crate::POINTER_X, crate::POINTER_Y);
            if hit != HitTarget::Outside {
                if matches!(hit, HitTarget::TileWifi | HitTarget::TileWifiExpand) {
                    crate::wifi_popover_glue::open_from_meta_hub();
                } else if hit == HitTarget::TileBluetooth {
                    close_launchers();
                    let _ = crate::bt_panel::toggle();
                }
                return true;
            }
        }
    }
    false
}

/// Ends active slider drag on left-button release.
pub unsafe fn release(px: i32, _py: i32) -> bool {
    let g = glue();
    let drag = g.state.active_drag_slider;
    if drag == DragSlider::None {
        return false;
    }
    g.state.active_drag_slider = DragSlider::None;
    if let Some(l) = current_layout() {
        match drag {
            DragSlider::Brightness => {
                let pct = Layout::percent_at(l.slider_brightness, px);
                set_brightness(pct, true);
            }
            DragSlider::Volume => {
                let pct = Layout::percent_at(l.slider_volume, px);
                set_volume(pct, true);
            }
            DragSlider::None => {}
        }
    } else {
        sync_display(false);
    }
    true
}

/// Pointer motion hook for smooth continuous slider dragging and live hover tracking.
pub unsafe fn on_pointer_move(px: i32, py: i32) {
    let g = glue();
    if !g.state.visible {
        return;
    }
    let Some(l) = current_layout() else {
        return;
    };
    let now = sex_pdx::get_ticks();
    match g.state.active_drag_slider {
        DragSlider::Brightness => {
            let pct = Layout::percent_at(l.slider_brightness, px);
            if pct != g.state.brightness_pct {
                let send_hw = now.saturating_sub(g.last_drag_hw_send) >= 60;
                set_brightness(pct, send_hw);
            }
        }
        DragSlider::Volume => {
            let pct = Layout::percent_at(l.slider_volume, px);
            if pct != g.state.volume_pct || g.state.speaker_muted {
                let commit = now.saturating_sub(g.last_drag_hw_send) >= 60;
                set_volume(pct, commit);
            }
        }
        DragSlider::None => {
            let hov = l.hit(px, py).hover_code();
            if hov != g.state.hover_target {
                g.state.hover_target = hov;
                sync_display(false);
            }
        }
    }
}

/// Scroll-wheel hook for Display Brightness & Master Volume sliders (±5% per detent).
pub unsafe fn wheel(steps: i8) -> bool {
    if steps == 0 {
        return false;
    }
    let Some(l) = current_layout() else {
        return false;
    };
    let hit = l.hit(crate::POINTER_X, crate::POINTER_Y);
    let delta = steps as i32 * 5;
    let g = glue();
    match hit {
        HitTarget::SliderBrightness => {
            let next = (g.state.brightness_pct as i32 + delta).clamp(0, 100) as u8;
            set_brightness(next, true);
            true
        }
        HitTarget::SliderVolume | HitTarget::BtnSpeakerMute => {
            let next = (g.state.volume_pct as i32 + delta).clamp(0, 100) as u8;
            set_volume(next, true);
            true
        }
        _ => false,
    }
}
