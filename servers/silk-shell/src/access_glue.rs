// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for `access.rs` (slice S8): runs each key through the
//! accessibility aids, tells Display about magnifier and filter changes and
//! shows each change on the panel. Kept out of main.rs so main.rs only holds
//! two hook lines.

use crate::access;

static mut ACCESS: access::Access = access::Access::new();
static mut REPLAYING: bool = false;

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
}

unsafe fn replay(events: &access::Events) {
    REPLAYING = true;
    for &(code, value) in events.as_slice() { crate::desktop_shortcut(code, value); }
    REPLAYING = false;
}

unsafe fn apply(change: access::Change) {
    let status = match change {
        access::Change::View { zoom, filter } =>
            crate::pdx_call(sex_pdx::SLOT_DISPLAY, sex_pdx::access_view::OP_ACCESS_VIEW, zoom as u64, filter as u64, 0).0,
        _ => 0,
    };
    let mut text = [0u8; 64];
    let n = (*core::ptr::addr_of!(ACCESS)).notice(change, &mut text);
    let shown = crate::notify_panel(&text[..n]);
    sex_pdx::serial_record!("[shell.access] change={:?} status={:#x} notice={}", change, status, shown as u8);
}
