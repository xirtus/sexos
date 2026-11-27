// SPDX-License-Identifier: GPL-3.0-only
//! `wifi`: what sexnet's Intel Wi-Fi chip is doing and the networks its last scan
//! heard (`sex_pdx::wifi_status`). `wifi scan` asks for a fresh scan; `wifi join N`
//! connects to network N (open networks), `wifi join N PASSPHRASE` to a WPA2/WPA3
//! one: the passphrase goes to sexnet in 7-byte parts, then the join. sexnet keeps
//! the network for this boot and rejoins it by itself after a lost link; `wifi leave`
//! disconnects and forgets it.
//!
//! sexnet answers each request with one reply word, which `receive` turns into
//! lines; a network takes a few words (its facts, its BSSID, its SSID in 7-byte
//! parts), asked one after another.
use sex_pdx::wifi_status as ws;

/// sexnet's PD.
pub const NET_PD: u32 = 13;
/// The most networks one listing shows (sexnet keeps 32).
const MAX_LISTED: u64 = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Nothing,
    State { scan: bool, join: bool, leave: bool },
    Network { index: u64, part: u64 },
    /// Passphrase part `part` sent; the join of network `index` follows the last.
    Password { index: u64, part: u64 },
    /// Part `part` of why the last try failed (`ws::WHY`).
    Why { part: u64 },
    /// Boot-reconnect polling `ws::STATE` until scan is ready or join connects.
    AutoState,
    /// Boot-reconnect walking scanned networks to match `auto_ssid`.
    AutoNetwork { index: u64, part: u64 },
}

pub struct Wifi {
    waiting: Waiting,
    /// The request now waiting (action, a, b), when it was sent and how often.
    asked: (u64, u64, u64),
    asked_at: u64,
    tries: u8,
    info: ws::Info,
    bssid: [u8; 7],
    ssid: [u8; 35],
    password: [u8; ws::PASSWORD_MAX],
    password_len: usize,
    why: [u8; ws::WHY_BYTES],
    why_len: usize,
    /// Last scanned SSIDs (index 0..32) so `wifi join N` knows the SSID to save.
    ssids: [[u8; 32]; 32],
    ssid_lens: [u8; 32],
    /// Credentials for the last `wifi join` (for auto-save and `wifi save`).
    saved_ssid: [u8; 32],
    saved_ssid_len: usize,
    saved_pass: [u8; ws::PASSWORD_MAX],
    saved_pass_len: usize,
    pending_save: bool,
    connected: bool,
    /// Boot-reconnect target SSID (or index when `auto_index > 0`, or multi-network when `auto_multi`).
    auto_active: bool,
    auto_multi: bool,
    auto_scan_triggered: bool,
    auto_joined: bool,
    auto_polls: u8,
    auto_next_at: u64,
    auto_index: u64,
    auto_best_idx: Option<u64>,
    auto_best_known: usize,
    auto_best_rssi: i8,
    auto_ssid: [u8; 32],
    auto_ssid_len: usize,
    auto_pass: [u8; ws::PASSWORD_MAX],
    auto_pass_len: usize,
}
pub static mut WIFI: Wifi = Wifi {
    waiting: Waiting::Nothing,
    asked: (0, 0, 0),
    asked_at: 0,
    tries: 0,
    info: ws::Info { channel: 0, rssi: 0, security: 0, ssid_len: 0, beacons: 0 },
    bssid: [0; 7],
    ssid: [0; 35],
    password: [0; ws::PASSWORD_MAX],
    password_len: 0,
    why: [0; ws::WHY_BYTES],
    why_len: 0,
    ssids: [[0; 32]; 32],
    ssid_lens: [0; 32],
    saved_ssid: [0; 32],
    saved_ssid_len: 0,
    saved_pass: [0; ws::PASSWORD_MAX],
    saved_pass_len: 0,
    pending_save: false,
    connected: false,
    auto_active: false,
    auto_multi: false,
    auto_scan_triggered: false,
    auto_joined: false,
    auto_polls: 0,
    auto_next_at: 0,
    auto_index: 0,
    auto_best_idx: None,
    auto_best_known: usize::MAX,
    auto_best_rssi: i8::MIN,
    auto_ssid: [0; 32],
    auto_ssid_len: 0,
    auto_pass: [0; ws::PASSWORD_MAX],
    auto_pass_len: 0,
};

const USAGE: &[u8] = b"Use: wifi [scan | join N [PASSPHRASE] | saved | save [SSID [PASS]] | forget [SSID] | leave | ba on|off | replay on|off]";

fn send(action: u64, a: u64, b: u64) -> bool {
    let (status, _) = sex_pdx::pdx_call(sex_pdx::SLOT_NET, ws::OP, action, a, b);
    status == 0
}

/// sexnet answers at once, but into this domain's single reply slot: when another
/// service's answer still sits there (Files, for the history), the kernel refuses
/// sexnet's and it is gone. On the x17 R1 the network list stopped after its first
/// line. A query not answered within this many ticks (ms) is asked again.
const RESEND_TICKS: u64 = 500;
const RESENDS: u8 = 8;

fn put(out: &mut [u8], at: &mut usize, text: &[u8]) {
    for &b in text { if *at < out.len() { out[*at] = b; *at += 1; } }
}

fn number(out: &mut [u8], at: &mut usize, n: i64) {
    if n < 0 { put(out, at, b"-"); }
    let mut v = n.unsigned_abs();
    let mut digits = [0u8; 20];
    let mut len = 0;
    loop { digits[len] = b'0' + (v % 10) as u8; len += 1; v /= 10; if v == 0 { break; } }
    for i in (0..len).rev() { put(out, at, &[digits[i]]); }
}

fn hex(out: &mut [u8], at: &mut usize, b: u8) {
    const H: &[u8; 16] = b"0123456789abcdef";
    put(out, at, &[H[(b >> 4) as usize], H[(b & 15) as usize]]);
}

/// The state line for a state word.
fn state_line(s: ws::State, scan: bool, join: bool, leave: bool, out: &mut [u8]) -> usize {
    let mut at = 0;
    if leave {
        put(out, &mut at, if s.phase == ws::NONE { b"Wi-Fi: no Intel Wi-Fi chip here." as &[u8] }
            else { b"Wi-Fi: left the network and forgot it; the chip restarts and scans again." });
        return at;
    }
    match s.phase {
        ws::NONE => put(out, &mut at, b"Wi-Fi: no Intel Wi-Fi chip here, or its firmware is missing."),
        ws::STARTING => put(out, &mut at, b"Wi-Fi: starting the chip's firmware; try again in a moment."),
        ws::SCANNING => {
            put(out, &mut at, if scan { b"Wi-Fi: scanning " } else { b"Wi-Fi: still scanning " });
            number(out, &mut at, s.channels as i64);
            put(out, &mut at, b" channels (");
            number(out, &mut at, s.networks as i64);
            put(out, &mut at, b" networks so far). Type wifi to see them.");
        }
        ws::READY => {
            put(out, &mut at, b"Wi-Fi: ");
            number(out, &mut at, s.networks as i64);
            put(out, &mut at, if s.networks == 1 { b" network heard" } else { b" networks heard" });
            put(out, &mut at, b" on ");
            number(out, &mut at, s.channels as i64);
            put(out, &mut at, b" channels (scan ");
            number(out, &mut at, s.scans as i64);
            put(out, &mut at, b").");
            if scan || join { put(out, &mut at, b" That is not possible right now."); }
        }
        // Each connecting step says which one it is, so a join that stalls shows where.
        ws::JOINING if join => put(out, &mut at,
            b"Wi-Fi: connecting (the chip restarts first if it was busy); type wifi to see how it went."),
        // No scan behind it: the chip is restarting or waiting to retry the network.
        ws::JOINING if s.networks == 0 => put(out, &mut at,
            b"Wi-Fi: connecting, step 1 of 4: restarting the chip and looking for the network (retries for about 3 minutes)."),
        ws::JOINING => put(out, &mut at, b"Wi-Fi: connecting, step 2 of 4: tuning to the network's channel."),
        ws::JOINED | ws::ASSOCIATING | ws::HANDSHAKE if scan || join => put(out, &mut at, b"Wi-Fi: already joining a network."),
        ws::JOINED | ws::ASSOCIATING => put(out, &mut at, b"Wi-Fi: connecting, step 3 of 4: asking the access point to let us in."),
        ws::HANDSHAKE => put(out, &mut at, b"Wi-Fi: connecting, step 4 of 4: checking the passphrase with the network."),
        ws::ASSOCIATED | ws::CONNECTED => put(out, &mut at, if scan {
            b"Wi-Fi: connected; scanning while connected comes later (wifi leave first)." as &[u8]
        } else if join { b"Wi-Fi: connected; that network was busy, try again." } else if s.phase == ws::CONNECTED { b"Wi-Fi: connected (WPA2); the network service now uses Wi-Fi." }
        else { b"Wi-Fi: connected to the open network; the network service now uses Wi-Fi." }),
        _ => put(out, &mut at, b"Wi-Fi: the last step failed (the boot log's [wifi.failed] line says where); wifi scan restarts the chip."),
    }
    at
}

/// One network's line.
fn network_line(shown: u64, info: ws::Info, bssid: &[u8], ssid: &[u8], out: &mut [u8]) -> usize {
    let mut at = 0;
    put(out, &mut at, b"  ");
    number(out, &mut at, shown as i64);
    put(out, &mut at, b". ");
    if ssid.is_empty() || ssid.iter().all(|&b| b == 0) {
        put(out, &mut at, b"(hidden)");
    } else {
        for &b in ssid { put(out, &mut at, &[if (0x20..0x7f).contains(&b) { b } else { b'?' }]); }
    }
    put(out, &mut at, b"  ");
    number(out, &mut at, info.rssi as i64);
    put(out, &mut at, b" dBm  channel ");
    number(out, &mut at, info.channel as i64);
    put(out, &mut at, b"  ");
    put(out, &mut at, ws::security_name(info.security).as_bytes());
    put(out, &mut at, b"  ");
    for (i, &b) in bssid.iter().take(6).enumerate() {
        if i > 0 { put(out, &mut at, b":"); }
        hex(out, &mut at, b);
    }
    at
}

/// `line` without a `wifi join N PASSPHRASE` or `wifi save SSID PASSPHRASE`'s passphrase (for the history).
pub fn without_passphrase(line: &[u8]) -> &[u8] {
    let mut words = line.split(|&b| b == b' ').filter(|w| !w.is_empty());
    if words.next() != Some(b"wifi") { return line; }
    if !matches!(words.next(), Some(b"join" | b"save")) { return line; }
    let pw = passphrase(line);
    if pw.is_empty() { line } else { &line[..line.len() - pw.len()] }
}

/// The passphrase: everything after `join N ` (or `save SSID `).
fn passphrase(args: &[u8]) -> &[u8] {
    let mut rest = args;
    let skip = if args.trim_ascii_start().starts_with(b"wifi") { 3 } else { 2 };
    for _ in 0..skip {
        while rest.first() == Some(&b' ') { rest = &rest[1..]; }
        let word = rest.iter().position(|&b| b == b' ').unwrap_or(rest.len());
        rest = &rest[word..];
    }
    if rest.first() == Some(&b' ') { &rest[1..] } else { rest }
}

const AUTO_POLL_MS: u64 = 1_500;
const AUTO_MAX_POLLS: u8 = 20;

impl Wifi {
    fn ask(&mut self, action: u64, a: u64, b: u64) -> bool {
        self.asked = (action, a, b);
        self.asked_at = sex_pdx::get_ticks();
        self.tries = 0;
        send(action, a, b)
    }
    /// True once Wi-Fi has reached `ASSOCIATED` or `CONNECTED`.
    pub fn is_connected(&self) -> bool { self.connected }
    /// True while boot-reconnect is waiting for scan / joining / connecting.
    pub fn auto_joining(&self) -> bool { self.auto_active }
    /// Spindle's loop polls while a reply is due (it may never come; see `RESEND_TICKS`).
    pub fn busy(&self) -> bool { self.waiting != Waiting::Nothing || self.auto_active }

    fn stop_auto(&mut self) {
        self.auto_active = false;
        self.auto_multi = false;
        self.auto_scan_triggered = false;
        self.auto_joined = false;
        self.auto_polls = 0;
        self.auto_index = 0;
        self.auto_best_idx = None;
        self.auto_best_known = usize::MAX;
        self.auto_best_rssi = i8::MIN;
        self.auto_ssid = [0; 32];
        self.auto_ssid_len = 0;
        self.auto_pass = [0; ws::PASSWORD_MAX];
        self.auto_pass_len = 0;
    }

    /// Starts boot-time auto-reconnect against all saved known networks in `AUTOCONF`.
    pub fn start_auto_join_known(&mut self, _sb: &mut crate::Scrollback) {
        self.stop_auto();
        self.auto_multi = true;
        self.auto_active = true;
        self.auto_next_at = 0;
    }

    /// Starts boot-time auto-reconnect by SSID (from `/data/wifi.conf`).
    pub fn start_auto_join_ssid(&mut self, ssid: &[u8], pass: &[u8], _sb: &mut crate::Scrollback) {
        if ssid.is_empty() || ssid.len() > 32 || pass.len() > ws::PASSWORD_MAX { return; }
        self.stop_auto();
        self.auto_ssid[..ssid.len()].copy_from_slice(ssid);
        self.auto_ssid_len = ssid.len();
        self.auto_pass[..pass.len()].copy_from_slice(pass);
        self.auto_pass_len = pass.len();
        self.auto_active = true;
        self.auto_next_at = 0;
    }

    /// Starts boot-time auto-reconnect by 1-based scan index (from `/data/wifi.conf`).
    pub fn start_auto_join_index(&mut self, index: u64, pass: &[u8], _sb: &mut crate::Scrollback) {
        if !(1..=MAX_LISTED).contains(&index) || pass.len() > ws::PASSWORD_MAX { return; }
        self.stop_auto();
        self.auto_index = index;
        self.auto_pass[..pass.len()].copy_from_slice(pass);
        self.auto_pass_len = pass.len();
        self.auto_active = true;
        self.auto_next_at = 0;
    }

    fn join_best_known(&mut self, sb: &mut crate::Scrollback) -> bool {
        let Some(scan_idx) = self.auto_best_idx else { return false };
        let known_idx = self.auto_best_known;
        let known = unsafe { (*core::ptr::addr_of!(crate::autoconf::AUTOCONF)).known() };
        if known_idx >= known.len() { return false; }
        let entry = known.entries[known_idx];
        self.auto_ssid = entry.ssid;
        self.auto_ssid_len = entry.ssid_len;
        self.auto_pass = [0; ws::PASSWORD_MAX];
        let pl = entry.pass_len.min(ws::PASSWORD_MAX);
        self.auto_pass[..pl].copy_from_slice(&entry.pass[..pl]);
        self.auto_pass_len = pl;
        let mut msg = [0u8; 96];
        let mut at = 0;
        put(&mut msg, &mut at, b"boot-reconnect: joining saved Wi-Fi \"");
        put(&mut msg, &mut at, entry.ssid_bytes());
        put(&mut msg, &mut at, b"\"...");
        sb.push(&msg[..at]);
        self.begin_auto_join_idx(scan_idx);
        true
    }

    fn begin_auto_join_idx(&mut self, idx0: u64) {
        self.auto_joined = true;
        self.auto_polls = 0;
        if self.auto_pass_len >= ws::PASSWORD_MIN {
            self.password = [0; ws::PASSWORD_MAX];
            self.password[..self.auto_pass_len].copy_from_slice(&self.auto_pass[..self.auto_pass_len]);
            self.password_len = self.auto_pass_len;
            if self.send_password(idx0, 0) {
                self.waiting = Waiting::Password { index: idx0, part: 0 };
            } else {
                self.stop_auto();
            }
        } else if self.ask(ws::JOIN, idx0, 0) {
            self.waiting = Waiting::State { scan: false, join: true, leave: false };
        } else {
            self.stop_auto();
        }
    }

    fn remember_join(&mut self, idx0: u64, pw: &[u8]) {
        self.saved_ssid = [0; 32];
        self.saved_ssid_len = 0;
        if (idx0 as usize) < self.ssids.len() {
            let sl = self.ssid_lens[idx0 as usize] as usize;
            if sl > 0 && sl <= 32 {
                self.saved_ssid[..sl].copy_from_slice(&self.ssids[idx0 as usize][..sl]);
                self.saved_ssid_len = sl;
            }
        }
        self.saved_pass = [0; ws::PASSWORD_MAX];
        let pl = pw.len().min(ws::PASSWORD_MAX);
        self.saved_pass[..pl].copy_from_slice(&pw[..pl]);
        self.saved_pass_len = pl;
        self.pending_save = self.saved_ssid_len > 0;
    }

    fn flush_pending_save(&mut self) {
        if self.pending_save && self.saved_ssid_len > 0 {
            self.pending_save = false;
            unsafe {
                crate::autoconf::AUTOCONF.queue_save_wifi(
                    &self.saved_ssid[..self.saved_ssid_len],
                    &self.saved_pass[..self.saved_pass_len],
                    false,
                );
            }
        }
    }

    /// Asks again for an answer that never came. Only queries and passphrase parts are
    /// repeated: a repeated scan, join or leave would act twice, so those just end.
    pub fn step(&mut self, sb: &mut crate::Scrollback) {
        let now = sex_pdx::get_ticks();
        if self.waiting == Waiting::Nothing {
            if self.auto_active && now >= self.auto_next_at {
                if self.auto_polls >= AUTO_MAX_POLLS {
                    self.stop_auto();
                    return;
                }
                self.auto_polls += 1;
                if self.ask(ws::STATE, 0, 0) {
                    self.waiting = Waiting::AutoState;
                } else {
                    self.stop_auto();
                }
            }
            return;
        }
        if now.saturating_sub(self.asked_at) < RESEND_TICKS { return; }
        let repeatable = match self.waiting {
            Waiting::State { scan, join, leave } => !scan && !join && !leave,
            Waiting::Network { .. } | Waiting::Password { .. } | Waiting::Why { .. }
            | Waiting::AutoState | Waiting::AutoNetwork { .. } => true,
            Waiting::Nothing => false,
        };
        if !repeatable || self.tries >= RESENDS {
            let was_auto = matches!(self.waiting, Waiting::AutoState | Waiting::AutoNetwork { .. });
            if matches!(self.waiting, Waiting::Password { .. }) { self.forget(); }
            self.waiting = Waiting::Nothing;
            if was_auto {
                self.stop_auto();
            } else {
                sb.push(b"Wi-Fi: no answer from the network service; type wifi to see where it is.");
            }
            return;
        }
        self.tries += 1;
        self.asked_at = now;
        let (action, a, b) = self.asked;
        sex_pdx::serial_record!("[spindle.wifi.resend] action={} a={} b={} try={}", action, a, b, self.tries);
        send(action, a, b);
    }
    fn send_password(&mut self, _index: u64, part: u64) -> bool {
        let from = part as usize * 7;
        let word = ws::bytes_word(&self.password[from..self.password_len.max(from)]);
        self.ask(ws::PASSWORD, part | (self.password_len as u64) << 8, word)
    }
    /// Asks sexnet why the last try failed (`Waiting::Why`); nothing shows when none did.
    fn ask_why(&mut self) {
        self.why_len = 0;
        if self.ask(ws::WHY, 0, 0) { self.waiting = Waiting::Why { part: 0 }; }
    }
    /// Spindle keeps no copy once sexnet has it.
    fn forget(&mut self) { self.password = [0; ws::PASSWORD_MAX]; self.password_len = 0; }

    /// Starts a `wifi` command; the answer arrives through `receive`.
    pub fn start(&mut self, args: &[u8], sb: &mut crate::Scrollback) {
        let mut words = args.split(|&b| b == b' ').filter(|w| !w.is_empty());
        let parse = |w: &[u8]| core::str::from_utf8(w).ok().and_then(|t| t.parse::<u64>().ok()).filter(|n| (1..=MAX_LISTED).contains(n));
        let (action, a, scan, join) = match (words.next(), words.next(), words.next()) {
            (None, _, _) => (ws::STATE, 0, false, false),
            (Some(b"scan"), None, _) => (ws::SCAN, 0, true, false),
            (Some(b"saved"), None, _) => {
                unsafe { crate::autoconf::AUTOCONF.list_saved_wifi(sb); }
                return;
            }
            (Some(b"save"), None, _) => {
                if self.saved_ssid_len == 0 {
                    sb.push(b"Wi-Fi: no joined network to save yet (run `wifi`, then `wifi join N [PASS]`, or `wifi save SSID [PASS]`).");
                    return;
                }
                unsafe {
                    crate::autoconf::AUTOCONF.queue_save_wifi(
                        &self.saved_ssid[..self.saved_ssid_len],
                        &self.saved_pass[..self.saved_pass_len],
                        true,
                    );
                }
                return;
            }
            (Some(b"save"), Some(ssid), rest) => {
                if ssid.len() > 32 {
                    sb.push(b"Wi-Fi: SSID must be at most 32 bytes.");
                    return;
                }
                let pw = if rest.is_none() { &b""[..] } else { passphrase(args) };
                if !pw.is_empty() && !(ws::PASSWORD_MIN..=ws::PASSWORD_MAX).contains(&pw.len()) {
                    sb.push(b"Wi-Fi: a Wi-Fi passphrase has 8 to 63 characters.");
                    return;
                }
                self.saved_ssid = [0; 32];
                self.saved_ssid[..ssid.len()].copy_from_slice(ssid);
                self.saved_ssid_len = ssid.len();
                self.saved_pass = [0; ws::PASSWORD_MAX];
                self.saved_pass[..pw.len()].copy_from_slice(pw);
                self.saved_pass_len = pw.len();
                unsafe { crate::autoconf::AUTOCONF.queue_save_wifi(ssid, pw, true); }
                return;
            }
            (Some(b"forget"), Some(_), _) => {
                // Everything after `forget ` (trimmed) is the SSID, supporting spaces.
                let trimmed = args.trim_ascii();
                let ssid = trimmed
                    .strip_prefix(b"forget")
                    .map(|r| r.trim_ascii())
                    .unwrap_or(b"");
                unsafe { crate::autoconf::AUTOCONF.queue_forget_wifi(ssid, sb); }
                return;
            }
            (Some(b"leave") | Some(b"forget"), None, _) => {
                self.connected = false;
                self.pending_save = false;
                self.stop_auto();
                if self.ask(ws::LEAVE, 0, 0) { self.waiting = Waiting::State { scan: false, join: false, leave: true }; }
                else { sb.push(b"Wi-Fi: the network service is unreachable."); }
                return;
            }
            (Some(k @ (b"ba" | b"replay")), Some(v @ (b"on" | b"off")), None) => {
                let knob = if k == b"ba" { ws::TUNE_BA } else { ws::TUNE_REPLAY };
                if self.ask(ws::TUNE, knob, (v == b"on") as u64) { self.waiting = Waiting::State { scan: false, join: false, leave: false }; }
                else { sb.push(b"Wi-Fi: the network service is unreachable."); }
                return;
            }
            (Some(b"join"), Some(n), rest) => {
                let Some(n) = parse(n) else { sb.push(USAGE); return; };
                self.stop_auto();
                if rest.is_none() {
                    self.remember_join(n - 1, b"");
                    (ws::JOIN, n - 1, false, true)
                } else {
                    // Everything after N is the passphrase (it may hold spaces).
                    let pw = passphrase(args);
                    if !(ws::PASSWORD_MIN..=ws::PASSWORD_MAX).contains(&pw.len()) {
                        sb.push(b"Wi-Fi: a Wi-Fi passphrase has 8 to 63 characters.");
                        return;
                    }
                    self.remember_join(n - 1, pw);
                    self.password = [0; ws::PASSWORD_MAX];
                    self.password[..pw.len()].copy_from_slice(pw);
                    self.password_len = pw.len();
                    if self.send_password(n - 1, 0) { self.waiting = Waiting::Password { index: n - 1, part: 0 }; }
                    else { sb.push(b"Wi-Fi: the network service is unreachable."); }
                    return;
                }
            }
            _ => { sb.push(USAGE); return; }
        };
        if self.ask(action, a, 0) { self.waiting = Waiting::State { scan, join, leave: false }; } else { sb.push(b"Wi-Fi: the network service is unreachable."); }
    }

    /// A reply from sexnet. False when it was not for a `wifi` command.
    pub fn receive(&mut self, caller: u32, value: u64, sb: &mut crate::Scrollback) -> bool {
        if caller != NET_PD || self.waiting == Waiting::Nothing || !ws::is_reply(value) { return false; }
        let waiting = core::mem::replace(&mut self.waiting, Waiting::Nothing);
        if value == ws::REFUSED {
            self.forget();
            self.pending_save = false;
            if matches!(waiting, Waiting::AutoState | Waiting::AutoNetwork { .. }) { self.stop_auto(); return true; }
            sb.push(b"Wi-Fi: the network service refused that.");
            return true;
        }
        if value == ws::NEEDS_PASSWORD {
            self.pending_save = false;
            if self.auto_active { self.stop_auto(); }
            sb.push(b"Wi-Fi: that network needs its passphrase: wifi join N PASSPHRASE");
            return true;
        }
        let mut line = [0u8; 128];
        match waiting {
            Waiting::Nothing => return false,
            Waiting::AutoState => {
                let s = ws::State::of(value);
                self.connected = matches!(s.phase, ws::ASSOCIATED | ws::CONNECTED);
                if self.connected {
                    self.stop_auto();
                    sb.push(b"boot-reconnect: Wi-Fi connected.");
                    return true;
                }
                if s.phase == ws::NONE || s.phase == ws::FAILED {
                    self.stop_auto();
                    return true;
                }
                if self.auto_joined {
                    self.auto_next_at = sex_pdx::get_ticks() + AUTO_POLL_MS;
                    return true;
                }
                if s.phase == ws::READY {
                    if s.networks > 0 {
                        if self.auto_index > 0 {
                            if self.auto_index <= s.networks as u64 {
                                self.begin_auto_join_idx(self.auto_index - 1);
                            } else {
                                self.stop_auto();
                            }
                        } else {
                            self.auto_best_idx = None;
                            self.auto_best_known = usize::MAX;
                            self.auto_best_rssi = i8::MIN;
                            if self.ask(ws::NETWORK, 0, ws::PART_INFO) {
                                self.waiting = Waiting::AutoNetwork { index: 0, part: ws::PART_INFO };
                            } else {
                                self.stop_auto();
                            }
                        }
                        return true;
                    }
                    // Empty scan while READY: trigger one fresh scan before retrying.
                    if !self.auto_scan_triggered {
                        self.auto_scan_triggered = true;
                        let _ = send(ws::SCAN, 0, 0);
                    }
                }
                // Still STARTING, SCANNING, or waiting for scan results: poll again shortly.
                self.auto_next_at = sex_pdx::get_ticks() + AUTO_POLL_MS;
            }
            Waiting::AutoNetwork { index, part } => {
                if value == ws::MISSING {
                    if self.auto_multi && self.join_best_known(sb) {
                        return true;
                    }
                    // Target SSID not in this scan; retry on next poll if budget remains.
                    self.auto_next_at = sex_pdx::get_ticks() + AUTO_POLL_MS;
                    return true;
                }
                match part {
                    ws::PART_INFO => { self.info = ws::Info::of(value); self.ssid = [0; 35]; }
                    ws::PART_BSSID => ws::word_bytes(value, &mut self.bssid),
                    p => ws::word_bytes(value, &mut self.ssid[((p - ws::PART_SSID) * 7) as usize..]),
                }
                let ssid_len = (self.info.ssid_len as u64).min(32);
                let last = ws::PART_SSID + (ssid_len + 6) / 7;
                let next = part + 1;
                if next < last.max(ws::PART_SSID) && self.ask(ws::NETWORK, index, next) {
                    self.waiting = Waiting::AutoNetwork { index, part: next };
                    return true;
                }
                if (index as usize) < self.ssids.len() {
                    let sl = ssid_len as usize;
                    self.ssids[index as usize][..sl].copy_from_slice(&self.ssid[..sl]);
                    self.ssid_lens[index as usize] = sl as u8;
                }
                if self.auto_multi {
                    let scanned_ssid = &self.ssid[..ssid_len as usize];
                    let known = unsafe { (*core::ptr::addr_of!(crate::autoconf::AUTOCONF)).known() };
                    if let Some(known_idx) = known.find_index(scanned_ssid) {
                        let rssi = self.info.rssi;
                        let better = self.auto_best_idx.is_none()
                            || rssi > self.auto_best_rssi
                            || (rssi == self.auto_best_rssi && known_idx < self.auto_best_known);
                        if better {
                            self.auto_best_idx = Some(index);
                            self.auto_best_known = known_idx;
                            self.auto_best_rssi = rssi;
                        }
                    }
                } else if ssid_len as usize == self.auto_ssid_len
                    && self.ssid[..self.auto_ssid_len] == self.auto_ssid[..self.auto_ssid_len]
                {
                    self.begin_auto_join_idx(index);
                    return true;
                }
                if index + 1 < MAX_LISTED && self.ask(ws::NETWORK, index + 1, ws::PART_INFO) {
                    self.waiting = Waiting::AutoNetwork { index: index + 1, part: ws::PART_INFO };
                } else if self.auto_multi && self.join_best_known(sb) {
                    return true;
                } else {
                    self.auto_next_at = sex_pdx::get_ticks() + AUTO_POLL_MS;
                }
            }
            Waiting::State { scan, join, leave } => {
                let s = ws::State::of(value);
                self.connected = matches!(s.phase, ws::ASSOCIATED | ws::CONNECTED);
                if join && !matches!(s.phase, ws::NONE | ws::FAILED) {
                    self.flush_pending_save();
                } else if self.connected {
                    self.flush_pending_save();
                }
                if self.auto_active {
                    if self.connected { self.stop_auto(); }
                    else { self.auto_next_at = sex_pdx::get_ticks() + AUTO_POLL_MS; }
                }
                let n = state_line(s, scan, join, leave, &mut line);
                sb.push(&line[..n]);
                sex_pdx::serial_record!("[spindle.wifi] phase={} networks={} scans={} channels={}", s.phase, s.networks, s.scans, s.channels);
                // On a network: the link's counters, which sexnet logged just before
                // answering (typing alone can push older Wi-Fi lines out of the log).
                if matches!(s.phase, ws::ASSOCIATED | ws::CONNECTED) && !scan && !join && !leave {
                    crate::hw_command::newest(b"[wifi.link.now]", sb);
                    crate::hw_command::newest(b"[wifi.link.now2]", sb);
                    crate::hw_command::newest(b"[wifi.link.now4]", sb);
                    crate::hw_command::newest(b"[wifi.link.now5]", sb);
                    crate::hw_command::newest(b"[wifi.link.now6]", sb);
                    crate::hw_command::newest(b"[wifi.link.now7]", sb);
                    crate::hw_command::newest(b"[wifi.link.now3]", sb);
                }
                // Connecting, or a try failed: why the last one failed says more than
                // the list (the boot log may have moved past its [wifi.failed] line).
                let trying = matches!(s.phase, ws::FAILED | ws::JOINING | ws::JOINED | ws::ASSOCIATING | ws::HANDSHAKE);
                if trying && !leave { self.ask_why(); return true; }
                // A scan just started has nothing finished to list yet.
                if s.networks > 0 && !join && !(scan && s.phase == ws::SCANNING) && self.ask(ws::NETWORK, 0, ws::PART_INFO) {
                    self.waiting = Waiting::Network { index: 0, part: ws::PART_INFO };
                } else if !join && !leave {
                    // Restarting or rescanning after a refused passphrase or a given-up
                    // rejoin: that failure still says the most.
                    self.ask_why();
                }
            }
            Waiting::Password { index, part } => {
                let next = part + 1;
                if (next as usize) * 7 < self.password_len {
                    if self.send_password(index, next) { self.waiting = Waiting::Password { index, part: next }; }
                    return true;
                }
                self.forget();
                if self.ask(ws::JOIN, index, 0) { self.waiting = Waiting::State { scan: false, join: true, leave: false }; }
            }
            Waiting::Why { part } => {
                if value != ws::MISSING {
                    let from = part as usize * 7;
                    let n = (ws::WHY_BYTES - from.min(ws::WHY_BYTES)).min(7);
                    ws::word_bytes(value, &mut self.why[from..from + n]);
                    self.why_len = from + n;
                    if part + 1 < ws::WHY_PARTS && self.ask(ws::WHY, part + 1, 0) {
                        self.waiting = Waiting::Why { part: part + 1 };
                        return true;
                    }
                }
                let len = self.why[..self.why_len].iter().position(|&b| b == 0).unwrap_or(self.why_len);
                if len > 0 {
                    let mut at = 0;
                    put(&mut line, &mut at, b"Wi-Fi: the last try failed: ");
                    for &b in &self.why[..len] { put(&mut line, &mut at, &[if (0x20..0x7f).contains(&b) { b } else { b'?' }]); }
                    sb.push(&line[..at]);
                }
            }
            Waiting::Network { index, part } => {
                // The list ended: then why the last try failed, if one did.
                if value == ws::MISSING { self.ask_why(); return true; }
                match part {
                    ws::PART_INFO => { self.info = ws::Info::of(value); self.ssid = [0; 35]; }
                    ws::PART_BSSID => ws::word_bytes(value, &mut self.bssid),
                    p => ws::word_bytes(value, &mut self.ssid[((p - ws::PART_SSID) * 7) as usize..]),
                }
                // SSID parts needed for this network's length.
                let ssid_len = (self.info.ssid_len as u64).min(32);
                let last = ws::PART_SSID + (ssid_len + 6) / 7;
                let next = part + 1;
                if next < last.max(ws::PART_SSID) && self.ask(ws::NETWORK, index, next) {
                    self.waiting = Waiting::Network { index, part: next };
                    return true;
                }
                if (index as usize) < self.ssids.len() {
                    let sl = ssid_len as usize;
                    self.ssids[index as usize][..sl].copy_from_slice(&self.ssid[..sl]);
                    self.ssid_lens[index as usize] = sl as u8;
                }
                let n = network_line(index + 1, self.info, &self.bssid, &self.ssid[..ssid_len as usize], &mut line);
                sb.push(&line[..n]);
                if index + 1 < MAX_LISTED && self.ask(ws::NETWORK, index + 1, ws::PART_INFO) {
                    self.waiting = Waiting::Network { index: index + 1, part: ws::PART_INFO };
                } else { self.ask_why(); }
            }
        }
        true
    }
}
