// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for macOS Noir Background App Lifecycle, Top-Right SilkBar Parked-App
//! Tray, Quick-Action Popovers, Two-Section Open/Background Switcher, Close vs Quit
//! Confirmation Sheet, Force Quit Dialog, About SexOS, and Ctrl+Left-Click Translation.
//!
//! Kept in a dedicated module so `servers/silk-shell/src/main.rs` holds only minimal
//! hook calls (preserving the passive-wait source fingerprint).

use sex_config::id;
use sex_pdx::app_catalog;
use sex_pdx::bg_apps::{
    self, AppKind, BarHit, BarLayout, ClosePolicy, CtrlClickTranslator, HoverTarget,
    MAX_OPEN, MAX_PARKED, NAME_MAX, OP_BG_APPS, OpenWindow, OverlayHit, OverlayKind,
    OverlayLayout, ParkedApp, State, apply_remember_policy, classify_name, pack_fixed_name,
    policy_bit_for_name, resolve_policy,
};
use silkbar_model::{DEFAULT_SILK_BAR, ModuleSlot};

struct Controller {
    overlay: OverlayKind,
    overlay_idx: u8,
    remember: bool,
    confirm_sid: u16,
    confirm_target: u8,
    confirm_kind: AppKind,
    confirm_name: [u8; NAME_MAX],
    confirm_name_len: u8,
    confirm_store: [u8; NAME_MAX],
    confirm_store_len: u8,
    hover: HoverTarget,
    seq: u16,
    dirty: bool,
    last_sent: State,
    last_pointer: (i32, i32),
}

impl Controller {
    const fn new() -> Self {
        Self {
            overlay: OverlayKind::None,
            overlay_idx: 0,
            remember: false,
            confirm_sid: 0,
            confirm_target: 0,
            confirm_kind: AppKind::Generic,
            confirm_name: [0; NAME_MAX],
            confirm_name_len: 0,
            confirm_store: [0; NAME_MAX],
            confirm_store_len: 0,
            hover: HoverTarget::None,
            seq: 0,
            dirty: true,
            last_sent: State::EMPTY,
            last_pointer: (-1, -1),
        }
    }

    fn close_overlay(&mut self) {
        self.overlay = OverlayKind::None;
        self.overlay_idx = 0;
        self.remember = false;
        self.confirm_sid = 0;
        self.confirm_target = 0;
        self.confirm_name = [0; NAME_MAX];
        self.confirm_name_len = 0;
        self.confirm_store = [0; NAME_MAX];
        self.confirm_store_len = 0;
        self.hover = HoverTarget::None;
    }
}

static mut CTRL: Controller = Controller::new();
static mut CTRL_CLICK: CtrlClickTranslator = CtrlClickTranslator::new();

/// Remembers recently launched program store names before Files replies to `Asked::Program`.
static mut PENDING_LAUNCH_NAME: ([u8; NAME_MAX], u8) = ([0; NAME_MAX], 0);
static mut TARGET_HINTS: [(u32, [u8; NAME_MAX], u8); 8] = [(0, [0; NAME_MAX], 0); 8];

fn ctrl() -> &'static mut Controller {
    unsafe { &mut *core::ptr::addr_of_mut!(CTRL) }
}

fn scale() -> u32 {
    crate::chrome_metrics().scale().clamp(1, 3)
}

fn unpack_zstr(words: [u64; 2]) -> ([u8; NAME_MAX], u8) {
    let mut raw = [0u8; NAME_MAX];
    raw[..8].copy_from_slice(&words[0].to_le_bytes());
    raw[8..16].copy_from_slice(&words[1].to_le_bytes());
    let len = raw.iter().position(|&b| b == 0).unwrap_or(NAME_MAX);
    let mut clean = [0u8; NAME_MAX];
    clean[..len].copy_from_slice(&raw[..len]);
    (clean, len as u8)
}

/// Records the store program name when Shell initiates `program_start_own`.
pub unsafe fn note_own_launch(packed: [u64; 2]) {
    let (buf, len) = unpack_zstr(packed);
    if len > 0 {
        PENDING_LAUNCH_NAME = (buf, len);
    }
}

/// Binds any pending own-launch program name to the launched domain target.
pub unsafe fn note_bound_target(target: u32) {
    let (buf, len) = core::mem::take(&mut *core::ptr::addr_of_mut!(PENDING_LAUNCH_NAME));
    if len == 0 {
        return;
    }
    let hints = &mut *core::ptr::addr_of_mut!(TARGET_HINTS);
    if let Some(slot) = hints.iter_mut().find(|h| h.0 == target || h.0 == 0) {
        *slot = (target, buf, len);
    } else {
        hints[0] = (target, buf, len);
    }
}

unsafe fn hint_name(target: u32) -> Option<([u8; NAME_MAX], u8)> {
    let hints = &*core::ptr::addr_of!(TARGET_HINTS);
    hints.iter().find(|h| h.0 == target && h.2 > 0).map(|h| (h.1, h.2))
}

unsafe fn trim_cstr(raw: &[u8; NAME_MAX]) -> ([u8; NAME_MAX], u8) {
    let len = raw.iter().position(|&b| b == 0).unwrap_or(NAME_MAX);
    let mut out = [0u8; NAME_MAX];
    out[..len].copy_from_slice(&raw[..len]);
    (out, len as u8)
}

/// Resolves `(store_name, store_len, AppKind, human_title, title_len)` for a live app.
unsafe fn app_identity(target: u32, sid: u64) -> ([u8; NAME_MAX], u8, AppKind, [u8; NAME_MAX], u8) {
    let (mut store, mut store_len) = ([0u8; NAME_MAX], 0u8);
    if let Some(n) = crate::program_notify::name(target) {
        (store, store_len) = pack_fixed_name(n.as_bytes());
    } else if let Some(row) = app_catalog::by_target(target) {
        match row.app {
            app_catalog::App::Quil => (store, store_len) = pack_fixed_name(b"quil"),
            app_catalog::App::Spindle => (store, store_len) = pack_fixed_name(b"spindle"),
            app_catalog::App::Browser => (store, store_len) = pack_fixed_name(b"browser"),
            app_catalog::App::Program => {
                if let Some(h) = hint_name(target) {
                    (store, store_len) = h;
                }
            }
        }
    }

    let mut tbuf = [0u8; NAME_MAX];
    let (mut title, mut title_len) = match crate::reader_glue::title(sid, &mut tbuf) {
        Ok(n) if n > 0 => pack_fixed_name(&tbuf[..n]),
        _ => ([0u8; NAME_MAX], 0u8),
    };

    if store_len == 0 {
        let sys_raw = sex_pdx::system_stats::name(target);
        let (sb, sl) = trim_cstr(&sys_raw);
        if sl > 0 {
            (store, store_len) = (sb, sl);
        } else if title_len > 0 {
            (store, store_len) = (title, title_len);
        }
    }

    let mut kind = classify_name(&store[..store_len as usize]);
    if kind == AppKind::Generic && title_len > 0 {
        kind = classify_name(&title[..title_len as usize]);
    }

    if title_len == 0 {
        if kind != AppKind::Generic {
            (title, title_len) = pack_fixed_name(kind.display_name().as_bytes());
        } else if store_len > 0 {
            (title, title_len) = (store, store_len);
            if title[0].is_ascii_lowercase() {
                title[0] = title[0].to_ascii_uppercase();
            }
        } else {
            (title, title_len) = pack_fixed_name(b"Program");
        }
    }

    (store, store_len, kind, title, title_len)
}

fn is_spindle_surface(sid: u64) -> bool {
    app_catalog::INCARNATIONS.iter().any(|r| {
        r.app == app_catalog::App::Spindle && (r.surface == sid || r.shell_surface == Some(sid))
    })
}

/// System-wide `Ctrl + Left-Click` -> Right-Click (`button == 2`) translation, preserving
/// press/release symmetry and keeping raw Ctrl+Click inside `Spindle` terminal content.
pub unsafe fn translate_button(button: u8, pressed: bool) -> u8 {
    let mods = (*core::ptr::addr_of!(crate::APP_MODIFIERS)).key(0);
    let in_spindle_content = if button == 1 && pressed && mods.control {
        let any_modal_or_popup = is_open()
            || crate::silkbar_menus_glue::is_open()
            || crate::spotlight_glue::is_open()
            || crate::decorations_glue::menu_shown()
            || crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None
            || crate::ATLAS_MODE_ENABLED
            || (*core::ptr::addr_of!(crate::OVERVIEW)).is_open()
            || (*core::ptr::addr_of!(crate::SCREENSHOT)).is_open();
        if any_modal_or_popup || crate::POINTER_Y < crate::desktop_bar_height() {
            false
        } else {
            matches!(
                crate::hit_test_at(crate::POINTER_X, crate::POINTER_Y),
                crate::HitTarget::Surface(sid) if is_spindle_surface(sid)
            )
        }
    } else {
        false
    };
    (*core::ptr::addr_of_mut!(CTRL_CLICK)).translate(button, pressed, mods.control, in_spindle_content)
}

unsafe fn bar_edges() -> Option<(usize, usize, u32, usize, usize)> {
    let m = crate::chrome_metrics();
    let w = crate::desktop_width().max(0) as usize;
    let bar_h = m.bar() as usize;
    let sc = m.scale().clamp(1, 3);
    let s = sc as usize;
    let target = m.px(sex_pdx::desktop_style::chrome::TARGET) as usize;
    let mut bar = DEFAULT_SILK_BAR;
    if !silkbar_model::reflow_chrome(&mut bar, w, bar_h, target) {
        return None;
    }
    let ws_last = if bar.layout[ModuleSlot::Workspace1 as usize].w > 0 {
        &bar.layout[ModuleSlot::Workspace8 as usize]
    } else {
        &bar.layout[ModuleSlot::Workspace0 as usize]
    };
    let ws_last_right = ws_last.x + ws_last.w;
    let chip0_x = bar.layout[ModuleSlot::Chip0 as usize].x;
    let tray = *core::ptr::addr_of!(crate::TRAY);
    let batt_reserve = if tray.battery.is_some() {
        if tray.charging || tray.on_ac != Some(true) { 128 * s } else { 52 * s }
    } else {
        6 * s
    };
    let right_edge = chip0_x.saturating_sub(batt_reserve);
    Some((w, bar_h, sc, ws_last_right, right_edge))
}

unsafe fn compute_bar_layout(parked_count: usize) -> Option<BarLayout> {
    let (_, bar_h, sc, ws_last_right, right_edge) = bar_edges()?;
    Some(BarLayout::compute(bar_h, sc, ws_last_right, right_edge, parked_count))
}

/// Leftmost X boundary available to custom `OP_TRAY_ITEM` icons so they sit strictly
/// left of parked app badges and the Open/Background Apps Switcher pill.
pub unsafe fn tray_right_edge(chip0_x: usize, s: usize) -> usize {
    let st = &ctrl().last_sent;
    if let Some(bl) = compute_bar_layout(st.parked_count as usize) {
        return bl.tray_right_edge;
    }
    chip0_x.saturating_sub(68 * s)
}

unsafe fn frame_for_surface_any(sid: u64) -> Option<(usize, &'static crate::ShellFrame)> {
    (*core::ptr::addr_of!(crate::FRAMES))
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| slot.as_ref().map(|f| (i, f)))
        .find(|(_, f)| f.tabs.iter().flatten().any(|t| t.surface_id == sid))
}

unsafe fn collect_state() -> State {
    let c = ctrl();
    let mut st = State::EMPTY;
    st.overlay = c.overlay;
    st.overlay_idx = c.overlay_idx;
    st.remember = c.remember;
    st.confirm_sid = c.confirm_sid;
    st.confirm_target = c.confirm_target;
    st.confirm_kind = c.confirm_kind;
    st.confirm_name = c.confirm_name;
    st.confirm_name_len = c.confirm_name_len;
    st.hover = c.hover;
    if let Some((_, _, _, _, right_edge)) = bar_edges() {
        st.bar_right_edge = right_edge as u16;
    }

    let custody = &*core::ptr::addr_of!(crate::APP_CUSTODY);
    let mut seen_sids = [0u64; 16];
    let mut seen_n = 0usize;

    for entry in custody.iter().flatten() {
        if entry.dead || entry.retired || entry.close_pending {
            continue;
        }
        let sid = entry.shell_surface().unwrap_or(entry.surface);
        if sid == 0 || seen_sids[..seen_n].contains(&sid) {
            continue;
        }
        let Some((_, frame)) = frame_for_surface_any(sid) else {
            continue;
        };
        // Skip hidden background helper tasks framed off every workspace.
        if frame.scene_id == 0xFE {
            continue;
        }
        if seen_n < seen_sids.len() {
            seen_sids[seen_n] = sid;
            seen_n += 1;
        }
        let (_, _, kind, title, tlen) = app_identity(entry.target, sid);
        let parked = (frame.flags & crate::FRAME_FLAG_PARKED) != 0;
        if parked {
            if (st.parked_count as usize) < MAX_PARKED {
                st.parked[st.parked_count as usize] = ParkedApp {
                    target_pd: entry.target as u8,
                    sid: sid as u16,
                    kind,
                    status: 1,
                    name: title,
                    name_len: tlen,
                };
                st.parked_count += 1;
            }
        } else if crate::surface_is_alive(sid) && (st.open_count as usize) < MAX_OPEN {
            st.open[st.open_count as usize] = OpenWindow {
                sid: sid as u16,
                kind,
                scene: frame.scene_id,
                focused: crate::FOCUSED_SURFACE_ID == sid,
                name: title,
                name_len: tlen,
            };
            st.open_count += 1;
        }
    }

    // Include live shell-native workspace windows (Linen / Mesh) in Section 1: Open Windows.
    for slot in (*core::ptr::addr_of!(crate::FRAMES)).iter() {
        let Some(frame) = slot.as_ref() else { continue };
        if frame.scene_id == 0xFE || (frame.flags & crate::FRAME_FLAG_PARKED) != 0 {
            continue;
        }
        let Some(tab) = frame.tabs.get(frame.active_tab as usize).and_then(|t| *t) else {
            continue;
        };
        let sid = tab.surface_id;
        if sid == 0 || seen_sids[..seen_n].contains(&sid) || !crate::surface_is_alive(sid) {
            continue;
        }
        let (title, tlen, kind) = if sid == 200 {
            let (n, l) = pack_fixed_name(b"Linen");
            (n, l, AppKind::Files)
        } else if sid == 202 {
            let (n, l) = pack_fixed_name(b"Mesh Graph");
            (n, l, AppKind::Generic)
        } else if sid == 203 {
            let (n, l) = pack_fixed_name(b"Collar");
            (n, l, AppKind::Generic)
        } else if sid == 204 {
            let (n, l) = pack_fixed_name(b"Notifications");
            (n, l, AppKind::Generic)
        } else {
            continue;
        };
        if (st.open_count as usize) < MAX_OPEN {
            st.open[st.open_count as usize] = OpenWindow {
                sid: sid as u16,
                kind,
                scene: frame.scene_id,
                focused: crate::FOCUSED_SURFACE_ID == sid,
                name: title,
                name_len: tlen,
            };
            st.open_count += 1;
        }
    }

    // Reconcile active overlay against current window/app inventory.
    match st.overlay {
        OverlayKind::QuickAction => {
            if st.parked_count == 0 {
                c.close_overlay();
                st.overlay = OverlayKind::None;
                st.overlay_idx = 0;
            } else if st.overlay_idx >= st.parked_count {
                c.overlay_idx = st.parked_count - 1;
                st.overlay_idx = c.overlay_idx;
            }
        }
        OverlayKind::CloseConfirm => {
            let csid = st.confirm_sid as u64;
            let live = custody
                .iter()
                .flatten()
                .any(|e| !e.dead && !e.retired && !e.close_pending && e.owns_surface(csid));
            let already_parked = frame_for_surface_any(csid)
                .is_some_and(|(_, f)| (f.flags & crate::FRAME_FLAG_PARKED) != 0);
            if !live || already_parked {
                c.close_overlay();
                st.overlay = OverlayKind::None;
                st.confirm_sid = 0;
            }
        }
        OverlayKind::ForceQuit => {
            let total = st.open_count as usize + st.parked_count as usize;
            if total == 0 {
                c.overlay_idx = 0;
                st.overlay_idx = 0;
            } else if (st.overlay_idx as usize) >= total {
                c.overlay_idx = (total - 1) as u8;
                st.overlay_idx = c.overlay_idx;
            }
        }
        _ => {}
    }
    st
}

unsafe fn sync() {
    let next = collect_state();
    let c = ctrl();
    if !c.dirty && next == c.last_sent {
        return;
    }
    c.seq = c.seq.wrapping_add(1);
    let ok = bg_apps::send_calls(&next, c.seq, |w| {
        crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_BG_APPS, w[0], w[1], w[2]).0 == 0
    });
    if ok {
        c.dirty = false;
        c.last_sent = next;
    } else {
        c.dirty = true;
    }
}

pub unsafe fn is_open() -> bool {
    ctrl().overlay != OverlayKind::None
}

pub fn display_restarted() {
    ctrl().dirty = true;
}

unsafe fn dismiss_other_chrome(reason: &str) {
    crate::silkbar_menus_glue::close(reason);
    crate::spotlight_glue::close(reason);
    if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
        crate::tray_close();
    }
}

pub unsafe fn close(reason: &str) {
    let c = ctrl();
    if c.overlay != OverlayKind::None {
        c.close_overlay();
        sync();
        sex_pdx::serial_record!("[shell.bg_apps.overlay] open=0 reason={}", reason);
    }
}

pub unsafe fn open_switcher() {
    let c = ctrl();
    if c.overlay == OverlayKind::Switcher {
        close("toggle");
        return;
    }
    dismiss_other_chrome("switcher");
    c.close_overlay();
    c.overlay = OverlayKind::Switcher;
    sync();
    sex_pdx::serial_record!("[shell.bg_apps.overlay] open=1 kind=switcher");
}

pub unsafe fn open_force_quit() {
    dismiss_other_chrome("force_quit");
    let c = ctrl();
    c.close_overlay();
    c.overlay = OverlayKind::ForceQuit;
    c.overlay_idx = 0;
    sync();
    sex_pdx::serial_record!("[shell.bg_apps.overlay] open=1 kind=force_quit");
}

pub unsafe fn open_about() {
    dismiss_other_chrome("about");
    let c = ctrl();
    c.close_overlay();
    c.overlay = OverlayKind::AboutSexOs;
    sync();
    sex_pdx::serial_record!("[shell.bg_apps.overlay] open=1 kind=about_sexos");
}

/// Interactive Window Close (`×`, `MenuAction::CloseWindow`, `Super+Ctrl+W`, `Alt+F4`,
/// or Switcher `[×]`): resolves per-app `ClosePolicy` (`KeepInSilkBar`, `QuitApp`, or `Ask`).
pub unsafe fn close_window(sid: u64) -> bool {
    if sid == 0 || !crate::is_closeable_surface(sid) {
        return false;
    }
    let custody = &*core::ptr::addr_of!(crate::APP_CUSTODY);
    let Some(entry) = custody
        .iter()
        .flatten()
        .find(|e| !e.dead && !e.retired && !e.close_pending && e.owns_surface(sid))
    else {
        return crate::close_surface_from_frame_light(sid);
    };

    // Grouped multi-tab frame: close just the requested tab safely.
    if let Some((_, frame)) = frame_for_surface_any(sid) {
        if frame.tab_count > 1 {
            let ok = crate::close_surface_from_frame_light(sid);
            sync();
            return ok;
        }
    }

    let (store, slen, kind, title, tlen) = app_identity(entry.target, sid);
    let keep_mask = crate::config_watch::value(id::CLOSE_KEEP);
    let quit_mask = crate::config_watch::value(id::CLOSE_QUIT);
    let policy = resolve_policy(&store[..slen as usize], kind, keep_mask, quit_mask);

    match policy {
        ClosePolicy::KeepInSilkBar => {
            let c = ctrl();
            if c.overlay == OverlayKind::CloseConfirm && c.confirm_sid as u64 == sid {
                c.close_overlay();
            }
            let ok = crate::park_frame_by_surface(sid);
            sync();
            sex_pdx::serial_record!(
                "[shell.bg_apps.close] sid={} target={} action=park ok={}",
                sid,
                entry.target,
                ok as u8
            );
            ok
        }
        ClosePolicy::QuitApp => {
            let ok = quit_surface(sid);
            sex_pdx::serial_record!(
                "[shell.bg_apps.close] sid={} target={} action=quit_policy ok={}",
                sid,
                entry.target,
                ok as u8
            );
            ok
        }
        ClosePolicy::Ask => {
            dismiss_other_chrome("close_confirm");
            let c = ctrl();
            c.close_overlay();
            c.overlay = OverlayKind::CloseConfirm;
            c.confirm_sid = sid as u16;
            c.confirm_target = entry.target as u8;
            c.confirm_kind = kind;
            c.confirm_name = title;
            c.confirm_name_len = tlen;
            c.confirm_store = store;
            c.confirm_store_len = slen;
            c.remember = false;
            c.hover = HoverTarget::CloseKeep;
            sync();
            sex_pdx::serial_record!(
                "[shell.bg_apps.close] sid={} target={} action=ask",
                sid,
                entry.target
            );
            true
        }
    }
}

/// Explicitly quits an open or parked app (`Super+Q`, `MenuAction::QuitApp`, Popover `Quit`).
pub unsafe fn quit_surface(sid: u64) -> bool {
    if sid == 0 {
        return false;
    }
    let c = ctrl();
    if c.overlay == OverlayKind::CloseConfirm && c.confirm_sid as u64 == sid {
        c.close_overlay();
    }
    for slot in (*core::ptr::addr_of_mut!(crate::FRAMES)).iter_mut() {
        if let Some(frame) = slot {
            if frame.tabs.iter().flatten().any(|t| t.surface_id == sid) {
                frame.flags &= !crate::FRAME_FLAG_PARKED;
            }
        }
    }
    let ok = crate::close_surface_from_frame_light(sid);
    sync();
    sex_pdx::serial_record!("[shell.bg_apps.quit] sid={} ok={}", sid, ok as u8);
    ok
}

pub unsafe fn quit_focused_or_close() -> bool {
    let Some(fid) = crate::selected_frame_id() else {
        return false;
    };
    let Some(sid) = crate::active_surface_for_frame(fid) else {
        return false;
    };
    quit_surface(sid)
}

unsafe fn reopen_and_focus(sid: u64) -> bool {
    if crate::reopen_parked_surface(sid) {
        sync();
        sex_pdx::serial_record!("[shell.bg_apps.reopen] sid={} ok=1", sid);
        true
    } else {
        sync();
        false
    }
}

unsafe fn remember_policy(choice: ClosePolicy) {
    let c = ctrl();
    if !c.remember {
        return;
    }
    let slen = (c.confirm_store_len as usize).min(NAME_MAX);
    let bit = policy_bit_for_name(&c.confirm_store[..slen]);
    let mut keep_mask = crate::config_watch::value(id::CLOSE_KEEP);
    let mut quit_mask = crate::config_watch::value(id::CLOSE_QUIT);
    apply_remember_policy(bit, choice, &mut keep_mask, &mut quit_mask);
    crate::config_watch::changed_here(id::CLOSE_KEEP, keep_mask);
    crate::config_watch::changed_here(id::CLOSE_QUIT, quit_mask);
    sex_pdx::serial_record!(
        "[shell.bg_apps.policy.saved] target={} choice={} bit={:?}",
        c.confirm_target,
        choice as u8,
        bit
    );
}

unsafe fn run_quick_action(parked_idx: usize, row: u8) {
    let st = collect_state();
    let Some(&app) = st.parked_slice().get(parked_idx) else {
        close("missing_parked");
        return;
    };
    let sid = app.sid as u64;
    if row == 0 {
        match app.kind {
            AppKind::Qupid => {
                crate::player_key(crate::media_keys::KEY_PLAY_PAUSE);
            }
            AppKind::Tether | AppKind::Tailor | AppKind::Swingers => {
                close("quick_action_reopen");
                let _ = reopen_and_focus(sid);
                return;
            }
            _ => {
                close("quick_action_reopen");
                let _ = reopen_and_focus(sid);
                return;
            }
        }
    } else {
        match app.kind {
            AppKind::Files | AppKind::Settings | AppKind::Browser | AppKind::Spindle | AppKind::SexShop => {
                close("quick_action_reopen");
                let _ = reopen_and_focus(sid);
                return;
            }
            _ => {
                crate::toggle_bell();
            }
        }
    }
    close("quick_action");
}

/// Top-bar click intercept for parked app badges and the Switcher pill.
pub unsafe fn handle_top_bar_click(px: i32, py: i32) -> bool {
    if px < 0 || py < 0 || py >= crate::desktop_bar_height() {
        return false;
    }
    let st = collect_state();
    let Some(bl) = compute_bar_layout(st.parked_count as usize) else {
        return false;
    };
    match bl.hit(px as usize, py as usize) {
        BarHit::SwitcherPill => {
            open_switcher();
            true
        }
        BarHit::ParkedBadge(i) => {
            let c = ctrl();
            if c.overlay == OverlayKind::QuickAction && c.overlay_idx as usize == i {
                close("toggle_badge");
            } else {
                dismiss_other_chrome("quick_action");
                c.close_overlay();
                c.overlay = OverlayKind::QuickAction;
                c.overlay_idx = i as u8;
                sync();
                sex_pdx::serial_record!("[shell.bg_apps.overlay] open=1 kind=quick_action idx={}", i);
            }
            true
        }
        BarHit::Outside => {
            if c_is_top_anchored() {
                close("bar_other");
            }
            false
        }
    }
}

unsafe fn c_is_top_anchored() -> bool {
    matches!(ctrl().overlay, OverlayKind::QuickAction | OverlayKind::Switcher)
}

unsafe fn force_quit_item(idx: usize) {
    let st = collect_state();
    let open_n = st.open_count as usize;
    let sid = if idx < open_n {
        st.open[idx].sid as u64
    } else {
        let bi = idx - open_n;
        let Some(p) = st.parked_slice().get(bi) else {
            return;
        };
        p.sid as u64
    };
    if sid == 0 {
        return;
    }
    for slot in (*core::ptr::addr_of_mut!(crate::FRAMES)).iter_mut() {
        if let Some(frame) = slot {
            if frame.tabs.iter().flatten().any(|t| t.surface_id == sid) {
                frame.flags &= !crate::FRAME_FLAG_PARKED;
            }
        }
    }
    let ended = {
        let custody = &mut *core::ptr::addr_of_mut!(crate::APP_CUSTODY);
        if let Some(e) = custody
            .iter_mut()
            .flatten()
            .find(|e| !e.dead && e.owns_surface(sid))
        {
            if app_catalog::by_surface(e.target, e.surface)
                .is_some_and(|r| r.app == app_catalog::App::Program)
            {
                e.forced = true;
                crate::force_end_and_wake(e)
            } else {
                crate::close_surface_from_frame_light(sid)
            }
        } else {
            crate::close_surface_from_frame_light(sid)
        }
    };
    let _ = ended;
    sync();
    sex_pdx::serial_record!("[shell.bg_apps.force_quit] idx={} sid={}", idx, sid);
}

/// Live `(open_windows, parked_apps)` summary for the Meta Control Hub.
pub unsafe fn summary_counts() -> (u8, u8) {
    let st = collect_state();
    (st.open_count, st.parked_count)
}

/// Handles left clicks below the bar while a `bg_apps` overlay/sheet is open.
pub unsafe fn press(px: i32, py: i32) -> bool {
    if ctrl().overlay == OverlayKind::None {
        return false;
    }
    if px < 0 || py < 0 || py < crate::desktop_bar_height() {
        return false;
    }
    let st = collect_state();
    let Some(bl) = compute_bar_layout(st.parked_count as usize) else {
        return true;
    };
    let (w, h, bar_h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
        crate::desktop_bar_height().max(0) as usize,
    );
    let Some(ol) = OverlayLayout::compute(&st, &bl, w, h, bar_h, scale()) else {
        close("no_room");
        return true;
    };
    let hit = ol.hit(px as usize, py as usize);
    let kind = ctrl().overlay;
    match hit {
        OverlayHit::Outside => {
            if !kind.is_modal() || kind == OverlayKind::AboutSexOs || kind == OverlayKind::ForceQuit {
                close("outside");
            }
        }
        OverlayHit::Inside => {}
        OverlayHit::QuickAction(row) => {
            let idx = ctrl().overlay_idx as usize;
            run_quick_action(idx, row);
        }
        OverlayHit::QuickReopen => {
            let idx = ctrl().overlay_idx as usize;
            if let Some(&p) = st.parked_slice().get(idx) {
                ctrl().close_overlay();
                let _ = reopen_and_focus(p.sid as u64);
            } else {
                close("reopen_missing");
            }
        }
        OverlayHit::QuickQuit => {
            let idx = ctrl().overlay_idx as usize;
            ctrl().close_overlay();
            if let Some(&p) = st.parked_slice().get(idx) {
                let _ = quit_surface(p.sid as u64);
            } else {
                sync();
            }
        }
        OverlayHit::SwitcherOpenFocus(i) => {
            if let Some(&ow) = st.open_slice().get(i) {
                ctrl().close_overlay();
                let sid = ow.sid as u64;
                if (ow.scene as u8) != crate::ACTIVE_SCENE_IDX
                    && (ow.scene as usize) < crate::WORKSPACE_COUNT as usize
                {
                    crate::switch_scene(ow.scene);
                }
                if let Some(fid) = crate::frame_for_surface(sid) {
                    if crate::frame_is_minimized(fid) {
                        let _ = crate::restore_minimized_frame(fid);
                    }
                }
                let _ = crate::try_set_focus(sid);
                sync();
            }
        }
        OverlayHit::SwitcherOpenClose(i) => {
            if let Some(&ow) = st.open_slice().get(i) {
                ctrl().close_overlay();
                let _ = close_window(ow.sid as u64);
            }
        }
        OverlayHit::SwitcherParkedReopen(i) => {
            if let Some(&p) = st.parked_slice().get(i) {
                ctrl().close_overlay();
                let _ = reopen_and_focus(p.sid as u64);
            }
        }
        OverlayHit::SwitcherParkedQuit(i) => {
            if let Some(&p) = st.parked_slice().get(i) {
                let _ = quit_surface(p.sid as u64);
            }
        }
        OverlayHit::CloseRememberToggle => {
            ctrl().remember = !ctrl().remember;
            sync();
        }
        OverlayHit::CloseCancel => {
            close("cancel");
        }
        OverlayHit::CloseKeepInSilkBar => {
            let csid = ctrl().confirm_sid as u64;
            remember_policy(ClosePolicy::KeepInSilkBar);
            ctrl().close_overlay();
            let _ = crate::park_frame_by_surface(csid);
            sync();
        }
        OverlayHit::CloseQuitApp => {
            let csid = ctrl().confirm_sid as u64;
            remember_policy(ClosePolicy::QuitApp);
            ctrl().close_overlay();
            let _ = quit_surface(csid);
        }
        OverlayHit::ForceSelect(i) => {
            ctrl().overlay_idx = i as u8;
            sync();
        }
        OverlayHit::ForceQuitSelected => {
            let idx = ctrl().overlay_idx as usize;
            force_quit_item(idx);
        }
        OverlayHit::ForceClose => {
            close("force_quit_close");
        }
        OverlayHit::AboutOpenSettings => {
            close("about_settings");
            crate::silkbar_menus_glue::open_settings_from_ui();
        }
        OverlayHit::AboutClose => {
            close("about_close");
        }
    }
    true
}

/// Blocks underlying context menus during modal sheets and dismisses non-modal popovers on right-click.
pub unsafe fn button(button: u8, pressed: bool) -> bool {
    let overlay = ctrl().overlay;
    if overlay == OverlayKind::None {
        return false;
    }
    if overlay.is_modal() {
        return true;
    }
    if button == 2 && pressed {
        close("right_click");
        return true;
    }
    false
}

/// Handles Escape (and Enter on confirmation sheets / Force Quit).
pub unsafe fn key(code: u8, value: u64) -> bool {
    let overlay = ctrl().overlay;
    if overlay == OverlayKind::None {
        return false;
    }
    if value == 0 {
        return overlay.is_modal();
    }
    if value != 1 {
        return overlay.is_modal();
    }
    match code {
        0x01 => {
            // Escape cancels sheets/popovers cleanly.
            close("escape");
            true
        }
        0x1C | 0x9C if overlay == OverlayKind::CloseConfirm => {
            // Enter confirms primary highlighted action ([Keep Running in SilkBar]).
            let csid = ctrl().confirm_sid as u64;
            remember_policy(ClosePolicy::KeepInSilkBar);
            ctrl().close_overlay();
            let _ = crate::park_frame_by_surface(csid);
            sync();
            true
        }
        0x48 if overlay == OverlayKind::ForceQuit => {
            let c = ctrl();
            if c.overlay_idx > 0 {
                c.overlay_idx -= 1;
                sync();
            }
            true
        }
        0x50 if overlay == OverlayKind::ForceQuit => {
            let c = ctrl();
            c.overlay_idx = c.overlay_idx.saturating_add(1);
            sync();
            true
        }
        _ => overlay.is_modal(),
    }
}

fn hover_from_hit(hit: OverlayHit) -> HoverTarget {
    match hit {
        OverlayHit::QuickAction(i) => HoverTarget::QuickRow(i),
        OverlayHit::SwitcherOpenFocus(i) => HoverTarget::SwitcherOpenFocus(i as u8),
        OverlayHit::SwitcherOpenClose(i) => HoverTarget::SwitcherOpenClose(i as u8),
        OverlayHit::SwitcherParkedReopen(i) => HoverTarget::SwitcherParkedReopen(i as u8),
        OverlayHit::SwitcherParkedQuit(i) => HoverTarget::SwitcherParkedQuit(i as u8),
        OverlayHit::CloseRememberToggle => HoverTarget::CloseRemember,
        OverlayHit::CloseKeepInSilkBar => HoverTarget::CloseKeep,
        OverlayHit::CloseQuitApp => HoverTarget::CloseQuit,
        OverlayHit::CloseCancel => HoverTarget::CloseCancel,
        OverlayHit::ForceSelect(i) => HoverTarget::ForceRow(i as u8),
        OverlayHit::ForceQuitSelected => HoverTarget::ForceQuitBtn,
        OverlayHit::ForceClose => HoverTarget::ForceCloseBtn,
        OverlayHit::AboutOpenSettings => HoverTarget::AboutSettingsBtn,
        OverlayHit::AboutClose => HoverTarget::AboutCloseBtn,
        _ => HoverTarget::None,
    }
}

/// Main loop poll: tracks pointer hover state on SilkBar chips / Noir overlays and syncs
/// live window + parked background app inventory to Display when anything changes.
pub unsafe fn poll() {
    let p = (crate::POINTER_X, crate::POINTER_Y);
    let c = ctrl();
    let pointer_moved = p != c.last_pointer;
    c.last_pointer = p;

    let st = collect_state();
    if pointer_moved || c.overlay != OverlayKind::None {
        let mut next_hover = HoverTarget::None;
        if p.0 >= 0 && p.1 >= 0 {
            let (ux, uy) = (p.0 as usize, p.1 as usize);
            let bar_h = crate::desktop_bar_height().max(0) as usize;
            if let Some(bl) = compute_bar_layout(st.parked_count as usize) {
                if uy < bar_h {
                    next_hover = match bl.hit(ux, uy) {
                        BarHit::SwitcherPill => HoverTarget::SwitcherPill,
                        BarHit::ParkedBadge(i) => HoverTarget::ParkedBadge(i as u8),
                        BarHit::Outside => HoverTarget::None,
                    };
                } else if st.overlay != OverlayKind::None {
                    let (w, h) = (
                        crate::desktop_width().max(0) as usize,
                        crate::desktop_height().max(0) as usize,
                    );
                    if let Some(ol) = OverlayLayout::compute(&st, &bl, w, h, bar_h, scale()) {
                        next_hover = hover_from_hit(ol.hit(ux, uy));
                    }
                }
            }
        }
        ctrl().hover = next_hover;
    }

    sync();
}
