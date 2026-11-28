// SPDX-License-Identifier: GPL-3.0-only
//! Shell glue for the Live Wi-Fi Scanner Top-Bar Popover (`sex_pdx::wifi_popover`).
//!
//! Coordinates non-blocking `sexnet` (`wifi_status::OP`) queries and joins with
//! Shell's single PDX reply slot, handles inline WPA2/WPA3 passphrase typing and
//! clipboard paste (`Ctrl+V` / `Super+V`), and publishes Display snapshots over
//! `OP_WIFI_POPOVER`.

use sex_pdx::wifi_popover::{
    self, Controller, PopoverAction, PopoverView, RawNetwork,
    MAX_RAW_NETWORKS, OP_WIFI_POPOVER, PASSWORD_MAX,
};
use sex_pdx::wifi_status as ws;

// Compile-time invariants ensuring `wifi_popover` mirror constants match `wifi_status`.
const _: () = assert!(wifi_popover::phase::NONE == ws::NONE);
const _: () = assert!(wifi_popover::phase::STARTING == ws::STARTING);
const _: () = assert!(wifi_popover::phase::SCANNING == ws::SCANNING);
const _: () = assert!(wifi_popover::phase::READY == ws::READY);
const _: () = assert!(wifi_popover::phase::FAILED == ws::FAILED);
const _: () = assert!(wifi_popover::phase::JOINING == ws::JOINING);
const _: () = assert!(wifi_popover::phase::JOINED == ws::JOINED);
const _: () = assert!(wifi_popover::phase::ASSOCIATING == ws::ASSOCIATING);
const _: () = assert!(wifi_popover::phase::ASSOCIATED == ws::ASSOCIATED);
const _: () = assert!(wifi_popover::phase::HANDSHAKE == ws::HANDSHAKE);
const _: () = assert!(wifi_popover::phase::CONNECTED == ws::CONNECTED);
const _: () = assert!(wifi_popover::phase::OFF == ws::OFF);
const _: () = assert!(wifi_popover::security::OPEN == ws::OPEN);
const _: () = assert!(wifi_popover::security::WPA2 == ws::WPA2);
const _: () = assert!(wifi_popover::security::WPA3 == ws::WPA3);
const _: () = assert!(wifi_popover::security::WPA2_WPA3 == ws::WPA2_WPA3);
const _: () = assert!(wifi_popover::security::UNSUPPORTED == ws::UNSUPPORTED);

const RESEND_TICKS: u64 = 500;
const MAX_TRIES: u8 = 6;
const ACTION_LOST_TICKS: u64 = 3_000;
const ACTIVE_POLL_TICKS: u64 = 750;
const IDLE_POLL_TICKS: u64 = 2_500;
const SPIN_STEP_TICKS: u64 = 180;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    None,
    Password { index: u64, part: u64 },
    State { repeatable: bool },
    ActiveIp4,
    ActiveInfo,
    ActiveSsid { part: u64 },
    Network { index: u64, part: u64 },
    Why { part: u64 },
}

#[derive(Clone, Copy)]
struct PendingJoin {
    scan_index: u8,
    pass: [u8; PASSWORD_MAX],
    pass_len: usize,
    has_pass: bool,
}

impl PendingJoin {
    const fn empty() -> Self {
        Self {
            scan_index: 0,
            pass: [0; PASSWORD_MAX],
            pass_len: 0,
            has_pass: false,
        }
    }
    fn scrub(&mut self) {
        self.pass_len = 0;
        self.has_pass = false;
        for b in &mut self.pass {
            unsafe { core::ptr::write_volatile(b, 0); }
        }
    }
}

struct Glue {
    ctrl: Controller,
    gen_seq: u16,
    last_sent: Option<PopoverView>,
    waiting: Waiting,
    asked: (u64, u64, u64),
    asked_at: u64,
    tries: u8,
    active_join: PendingJoin,
    want_radio: Option<bool>,
    want_leave: bool,
    want_join: Option<PendingJoin>,
    want_scan: bool,
    want_poll: bool,
    has_state: bool,
    fetched_state: ws::State,
    fetched_scans_tag: u16,
    force_fetch_scan: bool,
    ip4: [u8; 4],
    active_info: ws::Info,
    has_active_info: bool,
    active_ssid: [u8; 35],
    active_ssid_len: usize,
    raw_buf: [RawNetwork; MAX_RAW_NETWORKS],
    raw_len: usize,
    cur_info: ws::Info,
    cur_ssid: [u8; 35],
    why: [u8; ws::WHY_BYTES],
    why_len: usize,
    last_poll_at: u64,
    last_spin_at: u64,
    last_hover: (i32, i32),
}

static mut GLUE: Glue = Glue {
    ctrl: Controller::new(),
    gen_seq: 0,
    last_sent: None,
    waiting: Waiting::None,
    asked: (0, 0, 0),
    asked_at: 0,
    tries: 0,
    active_join: PendingJoin::empty(),
    want_radio: None,
    want_leave: false,
    want_join: None,
    want_scan: false,
    want_poll: false,
    has_state: false,
    fetched_state: ws::State { phase: ws::NONE, networks: 0, scans: 0, channels: 0 },
    fetched_scans_tag: u16::MAX,
    force_fetch_scan: false,
    ip4: [10, 0, 2, 15],
    active_info: ws::Info { channel: 0, rssi: 0, security: 0, ssid_len: 0, beacons: 0 },
    has_active_info: false,
    active_ssid: [0; 35],
    active_ssid_len: 0,
    raw_buf: [RawNetwork::EMPTY; MAX_RAW_NETWORKS],
    raw_len: 0,
    cur_info: ws::Info { channel: 0, rssi: 0, security: 0, ssid_len: 0, beacons: 0 },
    cur_ssid: [0; 35],
    why: [0; ws::WHY_BYTES],
    why_len: 0,
    last_poll_at: 0,
    last_spin_at: 0,
    last_hover: (-1, -1),
};

#[allow(static_mut_refs)]
fn glue() -> &'static mut Glue {
    unsafe { &mut *core::ptr::addr_of_mut!(GLUE) }
}

fn scale() -> u32 {
    crate::chrome_metrics().scale()
}

pub unsafe fn is_open() -> bool {
    glue().ctrl.is_open()
}

pub unsafe fn is_password_focused() -> bool {
    glue().ctrl.is_password_focused()
}

fn rssi_to_bars(rssi: i8) -> u8 {
    if rssi == 0 {
        4
    } else if rssi >= -55 {
        4
    } else if rssi >= -67 {
        3
    } else if rssi >= -77 {
        2
    } else if rssi >= -88 {
        1
    } else {
        0
    }
}

/// Live Wi-Fi summary `(enabled, connected, rssi_bars, ssid_bytes)` for the Meta Control Hub.
pub unsafe fn summary() -> (bool, bool, u8, &'static [u8]) {
    let g = glue();
    let phase = if g.has_state {
        g.fetched_state.phase
    } else {
        crate::TRAY.wifi
    };
    let connected = phase == ws::CONNECTED;
    let enabled = if g.has_state {
        phase != ws::OFF && phase != ws::NONE
    } else if phase == ws::NONE {
        g.want_radio.unwrap_or(false)
    } else {
        phase != ws::OFF
    };
    let bars = if connected && g.has_active_info {
        rssi_to_bars(g.active_info.rssi)
    } else if connected {
        4
    } else {
        0
    };
    let ssid: &'static [u8] = if connected && g.has_active_info && g.active_ssid_len > 0 {
        &g.active_ssid[..g.active_ssid_len]
    } else {
        &[]
    };
    (enabled, connected, bars, ssid)
}

/// Sets Wi-Fi radio power directly from the Meta Control Hub or Spotlight quick command.
pub unsafe fn set_radio(on: bool) {
    let g = glue();
    let now = sex_pdx::get_ticks();
    g.want_leave = false;
    if let Some(mut j) = g.want_join.take() {
        j.scrub();
    }
    g.want_scan = false;
    g.want_radio = Some(on);
    if on {
        g.force_fetch_scan = true;
        g.last_spin_at = now;
    }
    if !g.has_state {
        crate::TRAY.wifi = if on { ws::READY } else { ws::OFF };
    }
    sync_display();
    pump_net(now);
    sex_pdx::serial_record!("[shell.meta_hub.wifi] radio={}", on as u8);
}

/// Toggles Wi-Fi radio state from the Meta Control Hub quick pill.
pub unsafe fn toggle_radio() {
    let (enabled, _, _, _) = summary();
    set_radio(!enabled);
}

/// Triggers a quiet background Wi-Fi status fetch once if SSID has not yet been read.
pub unsafe fn refresh_background() {
    let g = glue();
    let now = sex_pdx::get_ticks();
    if !g.has_state && g.waiting == Waiting::None && !other_reply_owed(now) {
        g.want_poll = true;
        pump_net(now);
    }
}

/// Opens the interactive Wi-Fi Scanner Popover from the Meta Control Hub.
pub unsafe fn open_from_meta_hub() {
    if crate::COMMAND_PALETTE_OPEN {
        crate::toggle_command_palette();
    }
    if crate::spotlight_glue::is_open() {
        crate::spotlight_glue::close("wifi_scanner");
    }
    let anchor_x = (crate::desktop_width() - 320).max(16);
    let bar_h = crate::desktop_bar_height();
    open(anchor_x, bar_h);
}

/// True while waiting on `sexnet` (`SLOT_NET`) for a Wi-Fi popover reply.
pub unsafe fn answer_owed() -> bool {
    let g = glue();
    g.waiting != Waiting::None
        && sex_pdx::get_ticks().saturating_sub(g.asked_at) < ACTION_LOST_TICKS
}

/// How long idle Shell may sleep while popover is open or waiting on `sexnet`.
pub unsafe fn wake() -> Option<u64> {
    let g = glue();
    let now = sex_pdx::get_ticks();
    if g.waiting != Waiting::None {
        return Some(500);
    }
    if g.want_radio.is_some()
        || g.want_leave
        || g.want_join.is_some()
        || g.want_scan
        || g.want_poll
    {
        return Some(50);
    }
    if !g.ctrl.is_open() {
        return None;
    }
    let spin_due = if g.ctrl.scanning() || g.ctrl.connecting() {
        (g.last_spin_at + SPIN_STEP_TICKS)
            .saturating_sub(now)
            .clamp(1, SPIN_STEP_TICKS)
    } else {
        u64::MAX
    };
    let poll_every = if g.ctrl.scanning() || g.ctrl.connecting() {
        ACTIVE_POLL_TICKS
    } else {
        IDLE_POLL_TICKS
    };
    let poll_due = (g.last_poll_at + poll_every).saturating_sub(now).clamp(1, poll_every);
    Some(spin_due.min(poll_due))
}

unsafe fn sync_display() {
    let g = glue();
    let view = g.ctrl.view();
    if g.last_sent.as_ref() == Some(&view) {
        return;
    }
    g.gen_seq = g.gen_seq.wrapping_add(1).max(1);
    let seq = g.gen_seq;
    for (w0, w1, w2) in wifi_popover::chunks(&view, seq) {
        if crate::pdx_call(sex_pdx::SLOT_DISPLAY, OP_WIFI_POPOVER, w0, w1, w2).0 != 0 {
            g.last_sent = None;
            return;
        }
    }
    g.last_sent = Some(view);
}

pub unsafe fn display_restarted() {
    let g = glue();
    g.last_sent = None;
    if g.ctrl.is_open() {
        sync_display();
    }
}

pub unsafe fn open(anchor_x: i32, anchor_y: i32) {
    let g = glue();
    let now = sex_pdx::get_ticks();
    if !g.has_state && crate::TRAY.wifi != ws::NONE {
        g.ctrl.apply_state(crate::TRAY.wifi, None, 0, ws::OPEN, g.ip4, &[]);
    }
    g.ctrl.open_at(anchor_x, anchor_y);
    g.last_hover = (crate::POINTER_X, crate::POINTER_Y);
    g.last_spin_at = now;
    g.last_poll_at = now;
    if g.ctrl.radio_on() && g.ctrl.phase() != ws::OFF {
        g.want_scan = true;
        g.force_fetch_scan = true;
    } else {
        g.want_poll = true;
    }
    sync_display();
    pump_net(now);
    sex_pdx::serial_record!(
        "[shell.wifi_popover] open=1 x={} y={} phase={} nets={}",
        anchor_x,
        anchor_y,
        g.ctrl.phase(),
        g.ctrl.networks().len()
    );
}

pub unsafe fn close(reason: &str) {
    let g = glue();
    if let Some(mut j) = g.want_join.take() {
        j.scrub();
    }
    g.active_join.scrub();
    if g.ctrl.close() {
        sync_display();
        sex_pdx::serial_record!("[shell.wifi_popover] open=0 reason={}", reason);
    }
}

unsafe fn other_reply_owed(now: u64) -> bool {
    (*core::ptr::addr_of!(crate::MEDIA_ASKS)).pending()
        || crate::TRAY_WIFI_ASKED.is_some_and(|t| now.saturating_sub(t) < 5_000)
        || crate::net_time_owed()
}

unsafe fn send_net(action: u64, a: u64, b: u64, next_wait: Waiting, now: u64) -> bool {
    let g = glue();
    let (status, _) = crate::pdx_call(sex_pdx::SLOT_NET, ws::OP, action, a, b);
    if status == 0 {
        g.asked = (action, a, b);
        g.asked_at = now;
        g.tries = 0;
        g.waiting = next_wait;
        true
    } else {
        false
    }
}

unsafe fn send_password_part(pass: &[u8], pass_len: usize, index: u64, part: u64, now: u64) -> bool {
    let from = part as usize * 7;
    let word = ws::bytes_word(&pass[from..pass_len.max(from)]);
    send_net(
        ws::PASSWORD,
        part | ((pass_len as u64) << 8),
        word,
        Waiting::Password { index, part },
        now,
    )
}

unsafe fn pump_net(now: u64) {
    let g = glue();
    if g.waiting != Waiting::None || other_reply_owed(now) {
        return;
    }
    if let Some(on) = g.want_radio.take() {
        if !send_net(ws::RADIO, on as u64, 0, Waiting::State { repeatable: false }, now) {
            g.want_radio = Some(on);
        }
        return;
    }
    if g.want_leave {
        g.want_leave = false;
        if !send_net(ws::LEAVE, 0, 0, Waiting::State { repeatable: false }, now) {
            g.want_leave = true;
        }
        return;
    }
    if let Some(mut job) = g.want_join.take() {
        let idx = job.scan_index as u64;
        if job.has_pass && job.pass_len >= ws::PASSWORD_MIN {
            g.active_join = job;
            job.scrub();
            let sent = send_password_part(
                &g.active_join.pass,
                g.active_join.pass_len,
                idx,
                0,
                now,
            );
            if !sent {
                g.want_join = Some(g.active_join);
                g.active_join.scrub();
            }
        } else {
            job.scrub();
            if !send_net(ws::JOIN, idx, 0, Waiting::State { repeatable: false }, now) {
                g.want_join = Some(PendingJoin {
                    scan_index: idx as u8,
                    ..PendingJoin::empty()
                });
            }
        }
        return;
    }
    if g.want_scan {
        g.want_scan = false;
        g.last_poll_at = now;
        if !send_net(ws::SCAN, 0, 0, Waiting::State { repeatable: false }, now) {
            g.want_scan = true;
        }
        return;
    }
    if g.want_poll {
        g.want_poll = false;
        g.last_poll_at = now;
        if !send_net(ws::STATE, 0, 0, Waiting::State { repeatable: true }, now) {
            g.want_poll = true;
        }
    }
}

unsafe fn finish_fetch(now: u64) {
    let g = glue();
    g.waiting = Waiting::None;
    g.has_state = true;
    let active_ssid_opt = if g.has_active_info && g.active_ssid_len > 0 {
        Some(&g.active_ssid[..g.active_ssid_len])
    } else {
        None
    };
    let why_end = g.why[..g.why_len]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(g.why_len);
    g.ctrl.apply_state(
        g.fetched_state.phase,
        active_ssid_opt,
        g.active_info.rssi,
        g.active_info.security,
        g.ip4,
        &g.why[..why_end],
    );
    sync_display();
    pump_net(now);
    if g.waiting == Waiting::None {
        crate::tray_send_owed();
    }
}

unsafe fn start_networks_or_why(now: u64) {
    let g = glue();
    let s = g.fetched_state;
    let need_nets = s.networks > 0
        && s.phase != ws::NONE
        && s.phase != ws::OFF
        && (g.force_fetch_scan
            || g.fetched_scans_tag != s.scans
            || g.ctrl.raw_count() != s.networks);
    if need_nets {
        g.raw_len = 0;
        if send_net(
            ws::NETWORK,
            0,
            ws::PART_INFO,
            Waiting::Network { index: 0, part: ws::PART_INFO },
            now,
        ) {
            return;
        }
    }
    if s.phase == ws::FAILED {
        g.why_len = 0;
        if send_net(ws::WHY, 0, 0, Waiting::Why { part: 0 }, now) {
            return;
        }
    }
    finish_fetch(now);
}

/// Consumes a `sexnet` (`PD 13`) `wifi_status` reply intended for this popover.
pub unsafe fn receive(msg: &sex_pdx::PdxMessage) -> bool {
    let g = glue();
    if msg.type_id != 1
        || g.waiting == Waiting::None
        || msg.caller_pd != crate::TRAY_SEXNET_PD
        || !ws::is_reply(msg.arg0)
    {
        return false;
    }
    let now = sex_pdx::get_ticks();
    let waiting = g.waiting;
    g.waiting = Waiting::None;
    let val = msg.arg0;

    if val == ws::REFUSED || val == ws::NEEDS_PASSWORD {
        g.active_join.scrub();
        let notice: &[u8] = if val == ws::NEEDS_PASSWORD {
            b"Network requires a passphrase"
        } else {
            b"Network request refused"
        };
        g.ctrl.set_status(notice);
        sync_display();
        pump_net(now);
        if g.waiting == Waiting::None {
            crate::tray_send_owed();
        }
        return true;
    }

    match waiting {
        Waiting::None => return false,
        Waiting::Password { index, part } => {
            let next = part + 1;
            if (next as usize) * 7 < g.active_join.pass_len {
                if !send_password_part(&g.active_join.pass, g.active_join.pass_len, index, next, now) {
                    g.active_join.scrub();
                    finish_fetch(now);
                }
                return true;
            }
            g.active_join.scrub();
            if !send_net(ws::JOIN, index, 0, Waiting::State { repeatable: false }, now) {
                finish_fetch(now);
            }
            return true;
        }
        Waiting::State { .. } => {
            let s = ws::State::of(val);
            g.fetched_state = s;
            g.has_active_info = false;
            g.active_ssid_len = 0;
            g.why_len = 0;
            if crate::TRAY.wifi != s.phase {
                crate::TRAY.wifi = s.phase;
                crate::tray_publish();
            }
            if !send_net(ws::ACTIVE, ws::ACTIVE_IP4, 0, Waiting::ActiveIp4, now) {
                finish_fetch(now);
            }
            return true;
        }
        Waiting::ActiveIp4 => {
            if let Some((ip, _)) = ws::word_ip4(val) {
                g.ip4 = ip;
            }
            if wifi_popover::phase::is_active(g.fetched_state.phase) {
                if !send_net(ws::ACTIVE, ws::ACTIVE_INFO, 0, Waiting::ActiveInfo, now) {
                    start_networks_or_why(now);
                }
            } else {
                start_networks_or_why(now);
            }
            return true;
        }
        Waiting::ActiveInfo => {
            if val != ws::MISSING {
                let info = ws::Info::of(val);
                g.active_info = info;
                g.has_active_info = true;
                g.active_ssid = [0; 35];
                g.active_ssid_len = (info.ssid_len as usize).min(32);
                if g.active_ssid_len > 0
                    && send_net(
                        ws::ACTIVE,
                        ws::ACTIVE_SSID,
                        0,
                        Waiting::ActiveSsid { part: 0 },
                        now,
                    )
                {
                    return true;
                }
            }
            start_networks_or_why(now);
            return true;
        }
        Waiting::ActiveSsid { part } => {
            if val != ws::MISSING {
                ws::word_bytes(val, &mut g.active_ssid[(part as usize) * 7..]);
                let parts_needed = (g.active_ssid_len + 6) / 7;
                if (part + 1) < parts_needed as u64
                    && send_net(
                        ws::ACTIVE,
                        ws::ACTIVE_SSID + part + 1,
                        0,
                        Waiting::ActiveSsid { part: part + 1 },
                        now,
                    )
                {
                    return true;
                }
            }
            start_networks_or_why(now);
            return true;
        }
        Waiting::Network { index, part } => {
            if val == ws::MISSING {
                g.fetched_scans_tag = g.fetched_state.scans;
                g.force_fetch_scan = false;
                g.ctrl.set_raw_scan(&g.raw_buf[..g.raw_len]);
                if g.fetched_state.phase == ws::FAILED {
                    g.why_len = 0;
                    if send_net(ws::WHY, 0, 0, Waiting::Why { part: 0 }, now) {
                        return true;
                    }
                }
                finish_fetch(now);
                return true;
            }
            match part {
                ws::PART_INFO => {
                    g.cur_info = ws::Info::of(val);
                    g.cur_ssid = [0; 35];
                    let sl = (g.cur_info.ssid_len as usize).min(32);
                    if sl > 0 {
                        if send_net(
                            ws::NETWORK,
                            index,
                            ws::PART_SSID,
                            Waiting::Network { index, part: ws::PART_SSID },
                            now,
                        ) {
                            return true;
                        }
                    }
                }
                p if p >= ws::PART_SSID => {
                    let from = ((p - ws::PART_SSID) * 7) as usize;
                    if from < 35 {
                        ws::word_bytes(val, &mut g.cur_ssid[from..]);
                    }
                    let ssid_len = (g.cur_info.ssid_len as usize).min(32);
                    let last_part = ws::PART_SSID + ((ssid_len + 6) / 7) as u64;
                    if p + 1 < last_part
                        && send_net(
                            ws::NETWORK,
                            index,
                            p + 1,
                            Waiting::Network { index, part: p + 1 },
                            now,
                        )
                    {
                        return true;
                    }
                    if g.raw_len < MAX_RAW_NETWORKS && ssid_len > 0 {
                        g.raw_buf[g.raw_len] = RawNetwork::new(
                            index as u8,
                            g.cur_info.channel,
                            g.cur_info.rssi,
                            g.cur_info.security,
                            &g.cur_ssid[..ssid_len],
                        );
                        g.raw_len += 1;
                    }
                }
                _ => {}
            }

            let limit = (g.fetched_state.networks as usize).min(MAX_RAW_NETWORKS) as u64;
            if index + 1 < limit
                && send_net(
                    ws::NETWORK,
                    index + 1,
                    ws::PART_INFO,
                    Waiting::Network { index: index + 1, part: ws::PART_INFO },
                    now,
                )
            {
                return true;
            }
            g.fetched_scans_tag = g.fetched_state.scans;
            g.force_fetch_scan = false;
            g.ctrl.set_raw_scan(&g.raw_buf[..g.raw_len]);
            if g.fetched_state.phase == ws::FAILED {
                g.why_len = 0;
                if send_net(ws::WHY, 0, 0, Waiting::Why { part: 0 }, now) {
                    return true;
                }
            }
            finish_fetch(now);
            return true;
        }
        Waiting::Why { part } => {
            if val != ws::MISSING {
                let from = part as usize * 7;
                if from < ws::WHY_BYTES {
                    let n = (ws::WHY_BYTES - from).min(7);
                    ws::word_bytes(val, &mut g.why[from..from + n]);
                    g.why_len = from + n;
                }
                if part + 1 < ws::WHY_PARTS
                    && send_net(ws::WHY, part + 1, 0, Waiting::Why { part: part + 1 }, now)
                {
                    return true;
                }
            }
            finish_fetch(now);
            return true;
        }
    }
}

unsafe fn dispatch_action(action: PopoverAction) {
    let g = glue();
    let now = sex_pdx::get_ticks();
    match action {
        PopoverAction::None => {}
        PopoverAction::Closed => {
            if let Some(mut j) = g.want_join.take() {
                j.scrub();
            }
            sync_display();
            sex_pdx::serial_record!("[shell.wifi_popover] open=0 reason=dismiss");
        }
        PopoverAction::SetRadio(on) => {
            g.want_leave = false;
            if let Some(mut j) = g.want_join.take() {
                j.scrub();
            }
            g.want_scan = false;
            g.want_radio = Some(on);
            if on {
                g.force_fetch_scan = true;
                g.last_spin_at = now;
            }
            sync_display();
            pump_net(now);
            sex_pdx::serial_record!("[shell.wifi_popover.radio] on={}", on as u8);
        }
        PopoverAction::Rescan => {
            g.want_scan = true;
            g.force_fetch_scan = true;
            g.last_spin_at = now;
            sync_display();
            pump_net(now);
            sex_pdx::serial_record!("[shell.wifi_popover.rescan]");
        }
        PopoverAction::Disconnect => {
            if let Some(mut j) = g.want_join.take() {
                j.scrub();
            }
            g.want_scan = false;
            g.want_leave = true;
            sync_display();
            pump_net(now);
            sex_pdx::serial_record!("[shell.wifi_popover.disconnect]");
        }
        PopoverAction::Join { scan_index, has_password } => {
            let mut pass = [0u8; PASSWORD_MAX];
            let pass_len = if has_password {
                g.ctrl.take_pending_join_password(&mut pass)
            } else {
                0
            };
            if let Some(mut old) = g.want_join.replace(PendingJoin {
                scan_index,
                pass,
                pass_len,
                has_pass: has_password && pass_len >= ws::PASSWORD_MIN,
            }) {
                old.scrub();
            }
            for b in &mut pass {
                unsafe { core::ptr::write_volatile(b, 0); }
            }
            g.force_fetch_scan = true;
            g.last_spin_at = now;
            sync_display();
            pump_net(now);
            sex_pdx::serial_record!(
                "[shell.wifi_popover.join] scan_index={} secured={}",
                scan_index,
                has_password as u8
            );
        }
        PopoverAction::OpenNetworkPrefs => {
            sync_display();
            sex_pdx::serial_record!("[shell.wifi_popover.prefs]");
            crate::silkbar_menus_glue::launch_settings_view();
        }
    }
}

/// Handles a left press below the top bar while the Wi-Fi popover is open.
pub unsafe fn press(px: i32, py: i32) -> bool {
    let g = glue();
    if !g.ctrl.is_open() {
        return false;
    }
    let (w, h) = (
        crate::desktop_width().max(0) as usize,
        crate::desktop_height().max(0) as usize,
    );
    let (redraw, action) = g.ctrl.press(px, py, w, h, scale());
    if redraw {
        sync_display();
    }
    if action != PopoverAction::None {
        dispatch_action(action);
    }
    true
}

/// Handles keyboard input (typing passphrase, Eye toggle Tab, Enter Join, Esc, Ctrl+V Paste).
pub unsafe fn key(code: u8, value: u64) -> bool {
    let g = glue();
    if !g.ctrl.is_open() {
        return false;
    }
    let mods = (*core::ptr::addr_of!(crate::APP_MODIFIERS)).key(code);
    let shortcut = if mods.control {
        crate::keyboard_layout::shortcut_code(code)
    } else {
        code
    };
    let typed = if value != 0
        && !mods.control
        && !wifi_popover::is_modifier_scancode(code)
        && g.ctrl.is_password_focused()
    {
        crate::keyboard_layout::typed_char(code)
            .or_else(|| wifi_popover::scancode_ascii(code, mods.shift))
    } else {
        None
    };
    let clip = if (shortcut == 0x2F) && mods.control {
        (*core::ptr::addr_of!(crate::CLIPBOARD)).current_text()
    } else {
        None
    };
    let (consumed, redraw, action) =
        g.ctrl.handle_key_event(shortcut, value, mods.control, false, typed, clip);
    if !consumed {
        return false;
    }
    if redraw {
        sync_display();
    }
    if action != PopoverAction::None {
        dispatch_action(action);
    }
    true
}

/// Intercepts `Super+V` while inline password entry has focus so pasting works seamlessly.
pub unsafe fn paste_from_clipboard() -> bool {
    let g = glue();
    if !g.ctrl.is_password_focused() {
        return false;
    }
    if let Some(clip) = (*core::ptr::addr_of!(crate::CLIPBOARD)).current_text() {
        if g.ctrl.paste_bytes(clip) {
            sync_display();
        }
    }
    true
}

/// Main-loop tick: handles timeouts/resends, animations, hover highlights, and periodic refresh.
pub unsafe fn poll() {
    let g = glue();
    let now = sex_pdx::get_ticks();

    // Handle lost / timed-out replies cleanly.
    if g.waiting != Waiting::None && now.saturating_sub(g.asked_at) >= RESEND_TICKS {
        let repeatable = match g.waiting {
            Waiting::State { repeatable } => repeatable,
            Waiting::Password { .. }
            | Waiting::ActiveIp4
            | Waiting::ActiveInfo
            | Waiting::ActiveSsid { .. }
            | Waiting::Network { .. }
            | Waiting::Why { .. } => true,
            Waiting::None => false,
        };
        if repeatable && g.tries < MAX_TRIES {
            g.tries += 1;
            g.asked_at = now;
            let (act, a, b) = g.asked;
            let _ = crate::pdx_call(sex_pdx::SLOT_NET, ws::OP, act, a, b);
            return;
        } else if !repeatable && now.saturating_sub(g.asked_at) < ACTION_LOST_TICKS {
            // Non-idempotent SCAN/JOIN/LEAVE/RADIO stays pending until timeout.
            return;
        } else {
            g.active_join.scrub();
            g.waiting = Waiting::None;
            crate::tray_send_owed();
        }
    }

    pump_net(now);

    if !g.ctrl.is_open() {
        return;
    }

    let (px, py) = (crate::POINTER_X, crate::POINTER_Y);
    if (px, py) != g.last_hover {
        g.last_hover = (px, py);
        let (w, h) = (
            crate::desktop_width().max(0) as usize,
            crate::desktop_height().max(0) as usize,
        );
        if g.ctrl.hover(px, py, w, h, scale()) {
            sync_display();
        }
    }

    if (g.ctrl.scanning() || g.ctrl.connecting())
        && now.saturating_sub(g.last_spin_at) >= SPIN_STEP_TICKS
    {
        g.last_spin_at = now;
        if g.ctrl.step_spinner() {
            sync_display();
        }
    }

    let poll_every = if g.ctrl.scanning() || g.ctrl.connecting() {
        ACTIVE_POLL_TICKS
    } else {
        IDLE_POLL_TICKS
    };
    if g.waiting == Waiting::None && now.saturating_sub(g.last_poll_at) >= poll_every {
        g.want_poll = true;
        pump_net(now);
    }
}
