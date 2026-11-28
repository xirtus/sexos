// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for the OLauncher / macOS Spotlight-style Frosted Candy Lavender
//! App Launcher & Search overlay.
//!
//! Kept in a dedicated module so `servers/silk-shell/src/main.rs` only holds
//! minimal hook calls (preserving passive-wait / self-wait source fingerprints).

use sex_pdx::spotlight::{
    ClickOutcome, Controller, KeyOutcome, LaunchTarget, OP_SPOTLIGHT, OP_SPOTLIGHT_QUERY,
    resolve_program_alias,
};

static mut CTRL: Controller = Controller::new();

fn scale() -> u32 {
    crate::chrome_metrics().scale()
}

unsafe fn sync_display() {
    let ctrl = &*core::ptr::addr_of!(CTRL);
    let (q0, q1, q2) = ctrl.wire_query();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_SPOTLIGHT_QUERY, q0, q1, q2);
    let (w0, w1, w2) = ctrl.wire();
    let _ = crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_SPOTLIGHT, w0, w1, w2);
}

pub unsafe fn is_open() -> bool {
    (*core::ptr::addr_of!(CTRL)).is_open()
}

/// Screen rectangle of Spotlight when open, used by Meta Control Hub to dock beside it.
pub unsafe fn panel_rect() -> (i16, i16, u16, u16) {
    let (w, h, bar_h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
        crate::desktop_bar_height().max(0) as usize,
    );
    sex_pdx::spotlight::Layout::new(w, h, bar_h, scale()).map_or((0, 0, 0, 0), |l| {
        (l.panel.0 as i16, l.panel.1 as i16, l.panel.2 as u16, l.panel.3 as u16)
    })
}

pub unsafe fn open() {
    // Dismiss any open SilkBar dropdown or tray popup before showing Spotlight.
    crate::silkbar_menus_glue::close("spotlight");
    crate::bg_apps_glue::close("spotlight");
    if crate::TRAY_VIEW.menu != sex_pdx::tray::Menu::None {
        crate::tray_close();
    }
    if crate::COMMAND_PALETTE_OPEN {
        crate::toggle_command_palette();
    }
    (*core::ptr::addr_of_mut!(CTRL)).open();
    sync_display();
    crate::meta_hub_glue::sync_visibility();
    sex_pdx::serial_record!("[shell.spotlight] open=1");
}

pub unsafe fn close(reason: &str) {
    if (*core::ptr::addr_of_mut!(CTRL)).close() {
        sync_display();
        crate::meta_hub_glue::sync_visibility();
        sex_pdx::serial_record!("[shell.spotlight] open=0 reason={}", reason);
    }
}

pub unsafe fn toggle() -> bool {
    if is_open() {
        close("toggle");
        false
    } else {
        open();
        true
    }
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

fn program_needs_network(name: &str) -> bool {
    matches!(
        name,
        "kaleidoscope"
            | "tailor"
            | "tether"
            | "flirt"
            | "gossip"
            | "sexshop"
            | "brootlite"
            | "qupid"
            | "swingers"
            | "tatami"
            | "cabaret"
            | "rendezvous"
            | "hookup"
            | "lurk"
            | "needle"
            | "moan-studio"
    )
}

unsafe fn dispatch_launch(target: LaunchTarget) {
    match target {
        LaunchTarget::Browser => {
            if !launch_named_program("kaleidoscope", true) {
                let _ = crate::open_browser_in_active_scene();
            }
        }
        LaunchTarget::Spindle => {
            if !launch_named_program("spindle", true) {
                let _ = crate::open_spindle_in_active_scene();
            }
        }
        LaunchTarget::Quil => {
            let _ = crate::open_quil_in_active_scene();
        }
        LaunchTarget::PdGraph => {
            let _ = crate::open_mesh_in_active_scene();
        }
        LaunchTarget::Files => {
            if !launch_named_program("loom", false) {
                let _ = crate::open_linen_in_active_scene();
            }
        }
        LaunchTarget::Settings => {
            if !launch_named_program("tatami", true) {
                crate::toggle_scene_settings_panel();
            }
        }
        LaunchTarget::Audio => {
            if !launch_named_program("qupid", true) {
                crate::tray_toggle(sex_pdx::tray::Menu::Quick);
            }
        }
        LaunchTarget::Program(name) => {
            let resolved = resolve_program_alias(name);
            if !launch_named_program(resolved, program_needs_network(resolved))
                && resolved == "moan-studio"
            {
                let _ = launch_named_program("moan", program_needs_network("moan"));
            }
        }
        LaunchTarget::QueryProgram(query) => {
            let name = resolve_program_alias(query.as_str());
            if !name.is_empty()
                && !launch_named_program(name, true)
                && name == "moan-studio"
            {
                let _ = launch_named_program("moan", true);
            }
        }
    }
}

/// Handles keyboard input while Spotlight is open (search typing, arrows, Tab, Enter, Esc).
pub unsafe fn key(code: u8, value: u64) -> bool {
    let ctrl = &mut *core::ptr::addr_of_mut!(CTRL);
    if !ctrl.is_open() {
        return false;
    }
    if (code == 0x1C || code == 0x9C) && value == 1 {
        let q = ctrl.state().query;
        if let Some(qcmd) = sex_pdx::meta_hub::parse_quick_command(q.as_str()) {
            ctrl.close();
            sync_display();
            let _ = crate::meta_hub_glue::execute_quick_command(qcmd);
            crate::meta_hub_glue::sync_visibility();
            sex_pdx::serial_record!("[shell.spotlight] open=0 reason=quick_control");
            return true;
        }
    }
    let mods = crate::APP_MODIFIERS.key(code);
    match ctrl.handle_key(code, value, mods.control, mods.shift) {
        KeyOutcome::Ignored => false,
        KeyOutcome::Consumed => true,
        KeyOutcome::Redraw => {
            sync_display();
            true
        }
        KeyOutcome::Close => {
            sync_display();
            crate::meta_hub_glue::sync_visibility();
            sex_pdx::serial_record!("[shell.spotlight] open=0 reason=escape");
            true
        }
        KeyOutcome::Launch(target) => {
            sync_display();
            crate::meta_hub_glue::sync_visibility();
            sex_pdx::serial_record!("[shell.spotlight] open=0 reason=enter");
            dispatch_launch(target);
            true
        }
    }
}

/// Handles a left mouse press while Spotlight is open (tile launch, category switch, or outside dismiss).
pub unsafe fn press(px: i32, py: i32) -> bool {
    let ctrl = &mut *core::ptr::addr_of_mut!(CTRL);
    if !ctrl.is_open() {
        return false;
    }
    let (w, h, bar_h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
        crate::desktop_bar_height().max(0) as usize,
    );
    match ctrl.handle_press(px, py, w, h, bar_h, scale()) {
        ClickOutcome::Ignored => false,
        ClickOutcome::Inside => true,
        ClickOutcome::Redraw => {
            sync_display();
            true
        }
        ClickOutcome::Dismiss => {
            sync_display();
            crate::meta_hub_glue::sync_visibility();
            sex_pdx::serial_record!("[shell.spotlight] open=0 reason=outside");
            true
        }
        ClickOutcome::Launch(target) => {
            sync_display();
            crate::meta_hub_glue::sync_visibility();
            sex_pdx::serial_record!("[shell.spotlight] open=0 reason=click");
            dispatch_launch(target);
            true
        }
    }
}

/// Main-loop poll: updates hovered tile / category pill highlight.
pub unsafe fn poll() {
    let ctrl = &mut *core::ptr::addr_of_mut!(CTRL);
    if !ctrl.is_open() {
        return;
    }
    let (px, py) = (crate::POINTER_X, crate::POINTER_Y);
    let (w, h, bar_h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
        crate::desktop_bar_height().max(0) as usize,
    );
    if ctrl.handle_hover(px, py, w, h, bar_h, scale()) {
        sync_display();
    }
}
