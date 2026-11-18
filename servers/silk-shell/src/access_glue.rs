// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `access.rs` (slice S8): runs each key through the
//! accessibility aids, tells Display about magnifier and filter changes and
//! shows each change on the panel. The choices are kept in settings
//! (`accessibility.*`), so they hold after a restart and Settings can change
//! them. Kept out of main.rs so main.rs only holds two hook lines.

use crate::access;

static mut ACCESS: access::Access = access::Access::new();
static mut REPLAYING: bool = false;
/// A view from settings Display has not taken yet (it was busy or starting).
static mut VIEW_OWED: bool = false;

/// True when the aids took the event (and handled what replaced it).
pub unsafe fn key(code: u8, value: u64) -> bool {
    if REPLAYING { return false; }
    let mut events = access::Events::new();
    let access = &mut *core::ptr::addr_of_mut!(ACCESS);
    match access.key(code, value, sex_pdx::get_ticks(), &mut events) {
        access::Verdict::Pass => false,
        access::Verdict::Replace => { replay(&events); true }
        access::Verdict::Chord(change) => {
            // The window manager still sees the key, so Super's tap knows it
            // was used in a chord; its own binding for the key is not run.
            let _ = crate::WM_CHORDS.event(code, value);
            apply(change);
            true
        }
    }
}

/// Slow keys: a key held long enough is typed now.
pub unsafe fn poll() {
    let mut events = access::Events::new();
    let access = &mut *core::ptr::addr_of_mut!(ACCESS);
    if access.poll(sex_pdx::get_ticks(), &mut events) { replay(&events); }
    if VIEW_OWED {
        let change = access::Change::View { zoom: access.zoom, filter: access.filter };
        VIEW_OWED = carry_out(change) != 0;
        if !VIEW_OWED { sex_pdx::serial_record!("[shell.access.setting] owed={:?} status=0x0", change); }
    }
    crate::reader_glue::poll();
}

unsafe fn replay(events: &access::Events) {
    REPLAYING = true;
    for &(code, value) in events.as_slice() { crate::desktop_shortcut(code, value); }
    REPLAYING = false;
}

/// An accessibility setting from Files (at start, or changed in Settings):
/// carried out quietly, with no panel notice.
pub unsafe fn setting(key: u16, value: i32) {
    use sex_config::id;
    let access = &mut *core::ptr::addr_of_mut!(ACCESS);
    let on = value != 0;
    let wanted = match key {
        id::MAGNIFIER_ZOOM => access::Change::View { zoom: value.clamp(0, 255) as u8, filter: access.filter },
        id::COLOUR_FILTER => access::Change::View { zoom: access.zoom, filter: value.clamp(0, 255) as u8 },
        id::STICKY_KEYS => access::Change::StickyKeys(on),
        id::SLOW_KEYS => access::Change::SlowKeys(on),
        id::BOUNCE_KEYS => access::Change::BounceKeys(on),
        id::SCREEN_READER => access::Change::Reader(on),
        _ => return,
    };
    let Some(change) = access.restore(wanted) else { return };
    let status = carry_out(change);
    VIEW_OWED = matches!(change, access::Change::View { .. }) && status != 0;
    sex_pdx::serial_record!("[shell.access.setting] key={} value={} change={:?} status={:#x}", key, value, change, status);
}

/// Keeps a change made with the keys in settings, so it holds after a restart.
fn persist(change: access::Change) {
    use sex_config::id;
    match change {
        access::Change::View { zoom, filter } => {
            crate::config_watch::changed_here(id::MAGNIFIER_ZOOM, zoom as i32);
            crate::config_watch::changed_here(id::COLOUR_FILTER, filter as i32);
        }
        access::Change::StickyKeys(on) => crate::config_watch::changed_here(id::STICKY_KEYS, on as i32),
        access::Change::SlowKeys(on) => crate::config_watch::changed_here(id::SLOW_KEYS, on as i32),
        access::Change::BounceKeys(on) => crate::config_watch::changed_here(id::BOUNCE_KEYS, on as i32),
        access::Change::Reader(on) => crate::config_watch::changed_here(id::SCREEN_READER, on as i32),
    }
}

unsafe fn carry_out(change: access::Change) -> u64 {
    match change {
        access::Change::View { zoom, filter } =>
            crate::pdx_call(sex_pdx::SLOT_DISPLAY, sex_pdx::access_view::OP_ACCESS_VIEW, zoom as u64, filter as u64, 0).0,
        access::Change::Reader(on) => { crate::reader_glue::set(on); 0 }
        _ => 0,
    }
}

unsafe fn apply(change: access::Change) {
    persist(change);
    let status = carry_out(change);
    let mut text = [0u8; 64];
    let n = (*core::ptr::addr_of!(ACCESS)).notice(change, &mut text);
    let shown = crate::notify_panel(&text[..n]);
    sex_pdx::serial_record!("[shell.access] change={:?} status={:#x} notice={}", change, status, shown as u8);
}
