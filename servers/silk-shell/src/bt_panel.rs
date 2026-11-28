// SPDX-License-Identifier: GPL-3.0-only
//! The Bluetooth panel and pairing prompt (Cufflinks W2b).
//!
//! Super+Shift+B opens it. It shows whether Bluetooth is on (or that there is no
//! controller), the devices Cufflinks found or has paired, and lets the user scan,
//! pair (Enter or a click) and forget (Delete). When a device shows a six-digit
//! number, the panel opens by itself and asks "Does it show 123456?": Yes (or Y)
//! accepts, No, N or Esc refuses; Cufflinks refuses on its own after 30 s.
//!
//! Cufflinks and the Shell share the `sex_pdx::bt_ui` region: Cufflinks writes what
//! there is, the Shell writes commands, and Cufflinks wakes the Shell (`OP_EVENT`)
//! when there is something to show. What the panel says and what keys do is
//! `bt_ui::Ui` (host-tested); Display draws the lines (`bt_ui::OP_PANEL`). While the
//! panel shows it takes every key that is not a Super chord, like a dialog.
//!
//! On/off: F9 (`KEY_BLUETOOTH`), the panel's switch (or B) and the `bluetooth.on`
//! setting all set one level in the region (`Shared::set_off`) that Cufflinks follows;
//! the choice is kept in settings, so it lasts across boots. The bar's Bluetooth icon
//! (`sex_pdx::tray::Status::bluetooth`) follows what Cufflinks publishes.
use sex_pdx::bt_ui::{self as b, Action, Lines, Shared, Snapshot, Ui};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

struct State {
    region: Option<Shared>,
    /// The person wants Bluetooth off (kept here too, for a region that maps late).
    off: bool,
    probed: bool,
    /// Cufflinks woke us: read the region on the next pass.
    woken: bool,
    ui: Ui,
    snap: Option<Snapshot>,
    /// What Display shows now (an empty `Lines`: nothing).
    shown: (Lines, Option<usize>),
}

static mut STATE: State = State { region: None, off: false, probed: false, woken: true, ui: Ui::new(), snap: None, shown: (Lines::EMPTY, None) };
fn st() -> &'static mut State { unsafe { &mut *core::ptr::addr_of_mut!(STATE) } }

/// Map the region once and tell Cufflinks a Shell is here to ask the user.
fn region() -> Option<Shared> {
    let s = st();
    if !s.probed {
        s.probed = true;
        let view = sex_pdx::memory_region_full_view(b::SLOT, QUERY_VIEW | READ | WRITE).ok()
            .filter(|v| v.length >= b::BYTES && v.virtual_address != 0);
        s.region = view.map(|v| unsafe { Shared::at(v.virtual_address as *mut u8) });
        if let Some(r) = s.region { r.set_off(s.off); r.shell_attach(); r.set_open(false); }
        sex_pdx::serial_record!("[shell.bt.route] ok={} slot={}", s.region.is_some() as u8, b::SLOT);
    }
    s.region
}

/// Display restarted: it shows no panel now, so send the open one again in full.
pub fn display_restarted() {
    st().shown = (Lines::EMPTY, None);
    publish();
}

fn publish() {
    let s = st();
    let want = if s.ui.open { s.ui.lines(s.snap.as_ref()) } else { (Lines::EMPTY, None) };
    if let Some(r) = s.region { r.set_open(s.ui.open); }
    if want == s.shown { return; }
    for (word, a, bytes) in b::chunks(&want.0, want.1) {
        // Display's queue is full: the next pass sends it all again.
        if sex_pdx::pdx_call(sex_pdx::SLOT_DISPLAY, b::OP_PANEL, word, a, bytes).0 != 0 { return; }
    }
    s.shown = want;
    let status = if want.0.count > 1 { want.0.line(1) } else { &[] };
    sex_pdx::serial_record!("[shell.bt.panel] open={} phase={} lines={} selected={} dialog={} status=\"{}\"",
        s.ui.open as u8, s.snap.map_or(b::UNAVAILABLE, |n| n.phase), want.0.count, want.1.map_or(-1, |x| x as i32),
        s.ui.dialog.is_some() as u8, core::str::from_utf8(status).unwrap_or("?"));
}

fn act(action: Action) {
    let s = st();
    match action {
        Action::Send(c) => {
            let sent = s.region.is_some_and(|r| r.push_command(c));
            sex_pdx::serial_record!("[shell.bt.command] {:?} sent={}", c, sent as u8);
        }
        Action::Power(on) => { choose_power(on); return; }
        Action::None => return,
        Action::Redraw | Action::Close => {}
    }
    publish();
}

/// The `bluetooth.on` setting (at start, or changed in Settings).
pub fn set_power(on: bool) {
    let s = st();
    s.off = !on;
    let mapped = region().map(|r| r.set_off(!on)).is_some();
    sex_pdx::serial_record!("[shell.bt.power] on={} region={}", on as u8, mapped as u8);
    s.woken = true;
}

/// The person turned Bluetooth on or off here (F9, the panel): follow it, keep it in
/// settings, and say so in the bar for a moment.
fn choose_power(on: bool) {
    set_power(on);
    crate::config_watch::changed_here(sex_config::id::BLUETOOTH, on as i32);
    let text: &[u8] = if on { b"Bluetooth on" } else { b"Bluetooth off" };
    let shown = unsafe { crate::notify_panel(text) };
    if shown { unsafe { crate::NOTIFY_NOTICE_UNTIL = sex_pdx::get_ticks() + crate::media_keys::OSD_TICKS; } }
    sex_pdx::serial_record!("[shell.bt.power.key] on={} osd={}", on as u8, shown as u8);
    publish();
}

/// F9, the Bluetooth key: on if it was off, off if it was on.
pub fn toggle_power() -> bool {
    choose_power(st().off);
    true
}

/// The bar's Bluetooth icon for what Cufflinks published last.
fn tray_follow() {
    let icon = sex_pdx::bt_ui::tray_icon(st().snap.as_ref());
    unsafe {
        if crate::TRAY.bluetooth == icon { return; }
        let moved = (crate::TRAY.bluetooth == sex_pdx::tray::BT_NONE) != (icon == sex_pdx::tray::BT_NONE);
        crate::TRAY.bluetooth = icon;
        crate::tray_publish();
        // The icon came or went: program tray icons move over by one chip.
        if moved { crate::tray_items_glue::display_restarted(); }
    }
}

/// The bar's Bluetooth icon is shown and (x, y) is on it: a click there opens the panel.
pub fn icon_hit(chip0: sex_pdx::tray::Rect, x: usize, y: usize) -> bool {
    if unsafe { crate::TRAY.bluetooth } == sex_pdx::tray::BT_NONE { return false; }
    let (bx, by, bw, bh) = sex_pdx::tray::bluetooth_rect(chip0);
    chip0.2 != 0 && x >= bx && y >= by && x < bx + bw && y < by + bh
}

/// Every pass of the Shell's loop: follow Cufflinks while the panel shows or after
/// a wake (a comparison opens the panel by itself).
pub fn poll() {
    let Some(r) = region() else { return };
    let s = st();
    if !(s.woken || s.ui.open || s.shown.0.count != 0) { return; }
    s.woken = false;
    let snap = r.read();
    if snap != s.snap {
        s.snap = snap;
        s.ui.update(s.snap.as_ref());
        tray_follow();
    }
    publish();
}

/// Cufflinks's wake. True if the message was ours (it gets no reply).
pub fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    if msg.type_id != b::OP_EVENT || msg.caller_pd != sex_pdx::bt_hci::CUFFLINKS_PD { return false; }
    st().woken = true;
    true
}

/// Super+Shift+B. Without the region (no Cufflinks in this boot) the panel says so.
pub fn toggle() -> bool {
    let s = st();
    s.snap = region().and_then(|r| r.read());
    s.ui.update(s.snap.as_ref());
    act(s.ui.toggle());
    true
}

/// A key the desktop did not take for a Super chord. True if the panel used it.
/// Releases pass on, so a program never keeps a key it saw pressed.
pub fn key(code: u8, value: u64) -> bool {
    let s = st();
    if !s.ui.open || value == 0 { return false; }
    // The Bluetooth key works with the panel open too.
    if code == crate::desktop_keys::KEY_BLUETOOTH { if value == 1 { toggle_power(); } return true; }
    act(s.ui.key(code, s.snap.as_ref()));
    true
}

/// A click below the bar. True if the panel used it (a click outside closes it).
pub fn click(px: i32, py: i32, width: i32, bar: i32) -> bool {
    let s = st();
    if !s.ui.open || py < bar { return false; }
    let count = s.shown.0.count as usize;
    let scale = sex_pdx::desktop_style::ui_scale() as usize;
    let (x, y) = (px.max(0) as usize, py.max(0) as usize);
    let Some(panel) = b::Panel::new(width.max(0) as usize, bar as usize, scale, count) else { return false };
    let line = if panel.inside(x, y) { panel.row_at(x, y) } else { None };
    if panel.inside(x, y) && line.is_none() { return true; }
    act(s.ui.click(line, s.snap.as_ref()));
    true
}
