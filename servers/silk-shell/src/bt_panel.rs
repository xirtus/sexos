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
use sex_pdx::bt_ui::{self as b, Action, Lines, Shared, Snapshot, Ui};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

struct State {
    region: Option<Shared>,
    probed: bool,
    /// Cufflinks woke us: read the region on the next pass.
    woken: bool,
    ui: Ui,
    snap: Option<Snapshot>,
    /// What Display shows now (an empty `Lines`: nothing).
    shown: (Lines, Option<usize>),
}

static mut STATE: State = State { region: None, probed: false, woken: true, ui: Ui::new(), snap: None, shown: (Lines::EMPTY, None) };
fn st() -> &'static mut State { unsafe { &mut *core::ptr::addr_of_mut!(STATE) } }

/// Map the region once and tell Cufflinks a Shell is here to ask the user.
fn region() -> Option<Shared> {
    let s = st();
    if !s.probed {
        s.probed = true;
        let view = sex_pdx::memory_region_full_view(b::SLOT, QUERY_VIEW | READ | WRITE).ok()
            .filter(|v| v.length >= b::BYTES && v.virtual_address != 0);
        s.region = view.map(|v| unsafe { Shared::at(v.virtual_address as *mut u8) });
        if let Some(r) = s.region { r.shell_attach(); r.set_open(false); }
        sex_pdx::serial_record!("[shell.bt.route] ok={} slot={}", s.region.is_some() as u8, b::SLOT);
    }
    s.region
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
        Action::None => return,
        Action::Redraw | Action::Close => {}
    }
    publish();
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
