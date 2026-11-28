// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Live Wi-Fi Scanner top-bar popover: `#![no_std]`, zero-heap wire protocol,
//! SSID deduplication/RSSI sorting, shared pixel layout, hit-testing, and inline
//! WPA2/WPA3 passphrase entry state machine.
//!
//! Shell drives Wi-Fi scans and joins via `sexnet` (`wifi_status::OP`) and sends
//! snapshots over `OP_WIFI_POPOVER` to Display; both sides share `WifiLayout` so
//! pointer coordinates match rendered controls 1:1.

/// Shell -> Display PDX opcode for the live Wi-Fi popover snapshot chunks.
pub const OP_WIFI_POPOVER: u64 = 0x301;

/// Only Silk Shell (PD 3) may push Wi-Fi popover state to Display.
pub const SHELL_PD: u32 = 3;

pub const MAX_NETWORKS: usize = 8;
pub const MAX_RAW_NETWORKS: usize = 32;
pub const MAX_KNOWN: usize = 8;
pub const SSID_MAX: usize = 32;
pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 63;
pub const STATUS_MAX: usize = 40;
pub const PANEL_W: usize = 316;

/// `wifi_status` phase mirrors (kept pure so host tests need no syscall module).
pub mod phase {
    pub const NONE: u8 = 0;
    pub const STARTING: u8 = 1;
    pub const SCANNING: u8 = 2;
    pub const READY: u8 = 3;
    pub const FAILED: u8 = 4;
    pub const JOINING: u8 = 5;
    pub const JOINED: u8 = 6;
    pub const ASSOCIATING: u8 = 7;
    pub const ASSOCIATED: u8 = 8;
    pub const HANDSHAKE: u8 = 9;
    pub const CONNECTED: u8 = 10;
    pub const OFF: u8 = 11;

    pub const fn is_active(p: u8) -> bool {
        matches!(p, JOINING | JOINED | ASSOCIATING | ASSOCIATED | HANDSHAKE | CONNECTED)
    }

    pub const fn is_connecting(p: u8) -> bool {
        matches!(p, JOINING | JOINED | ASSOCIATING | HANDSHAKE)
    }

    pub const fn is_connected(p: u8) -> bool {
        matches!(p, ASSOCIATED | CONNECTED)
    }
}

/// `wifi_status` security mirrors.
pub mod security {
    pub const OPEN: u8 = 0;
    pub const WPA2: u8 = 1;
    pub const WPA3: u8 = 2;
    pub const WPA2_WPA3: u8 = 3;
    pub const UNSUPPORTED: u8 = 4;

    pub const fn label(sec: u8) -> &'static str {
        match sec {
            OPEN => "Open",
            WPA2 => "WPA2",
            WPA3 => "WPA3",
            WPA2_WPA3 => "WPA2/WPA3",
            _ => "Unsupported",
        }
    }
}

/// Maps RSSI (dBm) to 0..=4 signal strength bars.
pub const fn signal_bars(rssi: i8) -> u8 {
    if rssi == 0 || rssi <= -96 {
        0
    } else if rssi >= -55 {
        4
    } else if rssi >= -67 {
        3
    } else if rssi >= -78 {
        2
    } else {
        1
    }
}

pub type Rect = (usize, usize, usize, usize);

#[inline]
pub const fn rect_contains(r: Rect, x: usize, y: usize) -> bool {
    r.2 > 0 && r.3 > 0 && x >= r.0 && y >= r.1 && x - r.0 < r.2 && y - r.1 < r.3
}

/// Sanitizes an SSID buffer in place: printable ASCII kept, control bytes replaced.
fn clean_ssid(src: &[u8], out: &mut [u8; SSID_MAX]) -> u8 {
    let n = src.len().min(SSID_MAX);
    *out = [0; SSID_MAX];
    let mut i = 0;
    while i < n {
        let b = src[i];
        out[i] = if (0x20..=0x7E).contains(&b) { b } else { b'?' };
        i += 1;
    }
    n as u8
}

/// Raw beacon/probe entry fetched from `sexnet`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawNetwork {
    pub scan_index: u8,
    pub channel: u8,
    pub rssi: i8,
    pub security: u8,
    pub ssid_len: u8,
    pub ssid: [u8; SSID_MAX],
}

impl RawNetwork {
    pub const EMPTY: Self = Self {
        scan_index: 0,
        channel: 0,
        rssi: -100,
        security: security::OPEN,
        ssid_len: 0,
        ssid: [0; SSID_MAX],
    };

    pub fn new(scan_index: u8, channel: u8, rssi: i8, sec: u8, name: &[u8]) -> Self {
        let mut ssid = [0u8; SSID_MAX];
        let len = clean_ssid(name, &mut ssid);
        Self { scan_index, channel, rssi, security: sec, ssid_len: len, ssid }
    }

    pub fn ssid(&self) -> &[u8] {
        &self.ssid[..(self.ssid_len as usize).min(SSID_MAX)]
    }
}

/// Deduplicated, sorted network entry ready for display and one-click/password join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkEntry {
    pub scan_index: u8,
    pub channel: u8,
    pub rssi: i8,
    pub security: u8,
    pub known: bool,
    pub connected: bool,
    pub ssid_len: u8,
    pub ssid: [u8; SSID_MAX],
}

impl NetworkEntry {
    pub const EMPTY: Self = Self {
        scan_index: 0,
        channel: 0,
        rssi: -100,
        security: security::OPEN,
        known: false,
        connected: false,
        ssid_len: 0,
        ssid: [0; SSID_MAX],
    };

    pub fn ssid(&self) -> &[u8] {
        &self.ssid[..(self.ssid_len as usize).min(SSID_MAX)]
    }
}

fn scrub(bytes: &mut [u8]) {
    for b in bytes {
        unsafe { core::ptr::write_volatile(b, 0); }
    }
}

/// One remembered network credential in Shell's memory for this boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownNetwork {
    pub used: bool,
    pub ssid_len: u8,
    pub ssid: [u8; SSID_MAX],
    pub password_len: u8,
    pub password: [u8; PASSWORD_MAX],
}

impl KnownNetwork {
    pub const EMPTY: Self = Self {
        used: false,
        ssid_len: 0,
        ssid: [0; SSID_MAX],
        password_len: 0,
        password: [0; PASSWORD_MAX],
    };

    pub fn ssid(&self) -> &[u8] {
        &self.ssid[..(self.ssid_len as usize).min(SSID_MAX)]
    }

    pub fn password(&self) -> &[u8] {
        &self.password[..(self.password_len as usize).min(PASSWORD_MAX)]
    }

    pub fn wipe(&mut self) {
        self.used = false;
        self.ssid_len = 0;
        self.password_len = 0;
        scrub(&mut self.ssid);
        scrub(&mut self.password);
    }
}

/// Fixed-capacity in-memory store of known Wi-Fi credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownNetworks {
    entries: [KnownNetwork; MAX_KNOWN],
}

impl Default for KnownNetworks {
    fn default() -> Self {
        Self::new()
    }
}

impl KnownNetworks {
    pub const fn new() -> Self {
        Self { entries: [KnownNetwork::EMPTY; MAX_KNOWN] }
    }

    pub fn is_known(&self, ssid: &[u8]) -> bool {
        !ssid.is_empty() && self.entries.iter().any(|e| e.used && e.ssid() == ssid)
    }

    pub fn password_for(&self, ssid: &[u8]) -> Option<&[u8]> {
        if ssid.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .find(|e| e.used && e.ssid() == ssid)
            .map(|e| e.password())
    }

    /// Saves or refreshes `(ssid, password)` at the front (LRU eviction at `MAX_KNOWN`).
    pub fn remember(&mut self, ssid: &[u8], password: &[u8]) -> bool {
        if ssid.is_empty() || ssid.len() > SSID_MAX || password.len() > PASSWORD_MAX {
            return false;
        }
        let mut entry = KnownNetwork::EMPTY;
        entry.used = true;
        entry.ssid_len = ssid.len() as u8;
        entry.ssid[..ssid.len()].copy_from_slice(ssid);
        entry.password_len = password.len() as u8;
        entry.password[..password.len()].copy_from_slice(password);

        if let Some(pos) = self.entries.iter().position(|e| e.used && e.ssid() == ssid) {
            self.entries[pos].wipe();
            self.entries.copy_within(0..pos, 1);
            self.entries[0] = entry;
            return true;
        }
        self.entries[MAX_KNOWN - 1].wipe();
        self.entries.copy_within(0..MAX_KNOWN - 1, 1);
        self.entries[0] = entry;
        true
    }

    pub fn forget(&mut self, ssid: &[u8]) -> bool {
        let Some(pos) = self.entries.iter().position(|e| e.used && e.ssid() == ssid) else {
            return false;
        };
        self.entries[pos].wipe();
        self.entries.copy_within(pos + 1..MAX_KNOWN, pos);
        self.entries[MAX_KNOWN - 1] = KnownNetwork::EMPTY;
        true
    }

    pub fn clear(&mut self) {
        for e in &mut self.entries {
            e.wipe();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.used).count()
    }
}

/// Deduplicates raw AP scan entries by SSID (retaining the strongest RSSI AP's
/// `scan_index`, channel, and security), skips empty/hidden or active `exclude_ssid`,
/// marks known networks, sorts descending by RSSI, and returns up to `MAX_NETWORKS` (8).
pub fn dedup_and_sort(
    raw: &[RawNetwork],
    exclude_ssid: &[u8],
    known: &KnownNetworks,
    out: &mut [NetworkEntry; MAX_NETWORKS],
) -> usize {
    *out = [NetworkEntry::EMPTY; MAX_NETWORKS];
    let mut pool = [NetworkEntry::EMPTY; MAX_RAW_NETWORKS];
    let mut n = 0usize;

    for r in raw {
        let len = (r.ssid_len as usize).min(SSID_MAX);
        if len == 0 || r.ssid[..len].iter().all(|&b| b == 0) {
            continue;
        }
        let mut clean = [0u8; SSID_MAX];
        let clean_len = clean_ssid(&r.ssid[..len], &mut clean);
        let name = &clean[..(clean_len as usize)];
        if !exclude_ssid.is_empty() && name == exclude_ssid {
            continue;
        }
        let is_known = known.is_known(name);
        if let Some(existing) = pool[..n].iter_mut().find(|e| e.ssid() == name) {
            if r.rssi > existing.rssi {
                existing.scan_index = r.scan_index;
                existing.channel = r.channel;
                existing.rssi = r.rssi;
                existing.security = r.security;
            }
            existing.known |= is_known;
        } else if n < MAX_RAW_NETWORKS {
            pool[n] = NetworkEntry {
                scan_index: r.scan_index,
                channel: r.channel,
                rssi: r.rssi,
                security: r.security,
                known: is_known,
                connected: false,
                ssid_len: clean_len,
                ssid: clean,
            };
            n += 1;
        }
    }

    // Insertion sort over <= 32 items: RSSI desc, known first, SSID asc.
    for i in 1..n {
        let cur = pool[i];
        let mut j = i;
        while j > 0 {
            let prev = pool[j - 1];
            let swap = (cur.rssi > prev.rssi)
                || (cur.rssi == prev.rssi && cur.known && !prev.known)
                || (cur.rssi == prev.rssi && cur.known == prev.known && cur.ssid() < prev.ssid());
            if !swap {
                break;
            }
            pool[j] = prev;
            j -= 1;
        }
        pool[j] = cur;
    }

    let keep = n.min(MAX_NETWORKS);
    out[..keep].copy_from_slice(&pool[..keep]);
    keep
}

/// Interactive targets inside the Wi-Fi popover (used for both hover and click).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HitTarget {
    None = 0,
    Outside = 1,
    Inside = 2,
    Toggle = 3,
    Rescan = 4,
    Disconnect = 5,
    PasswordBox = 6,
    EyeToggle = 7,
    RememberToggle = 8,
    JoinSubmit = 9,
    CancelPassword = 10,
    NetworkPrefs = 11,
    NetworkRow(u8) = 16, // wire bytes 16..=23
}

impl HitTarget {
    pub const fn wire(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Outside => 1,
            Self::Inside => 2,
            Self::Toggle => 3,
            Self::Rescan => 4,
            Self::Disconnect => 5,
            Self::PasswordBox => 6,
            Self::EyeToggle => 7,
            Self::RememberToggle => 8,
            Self::JoinSubmit => 9,
            Self::CancelPassword => 10,
            Self::NetworkPrefs => 11,
            Self::NetworkRow(i) => 16 + (i & 7),
        }
    }

    pub const fn from_wire(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Outside),
            2 => Some(Self::Inside),
            3 => Some(Self::Toggle),
            4 => Some(Self::Rescan),
            5 => Some(Self::Disconnect),
            6 => Some(Self::PasswordBox),
            7 => Some(Self::EyeToggle),
            8 => Some(Self::RememberToggle),
            9 => Some(Self::JoinSubmit),
            10 => Some(Self::CancelPassword),
            11 => Some(Self::NetworkPrefs),
            16..=23 => Some(Self::NetworkRow(v - 16)),
            _ => None,
        }
    }

    pub const fn is_interactive(self) -> bool {
        !matches!(self, Self::None | Self::Outside | Self::Inside)
    }
}

/// Compact per-row descriptor sent to Display in `PopoverView`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewNetwork {
    pub known: bool,
    pub connected: bool,
    pub security: u8,
    pub rssi: i8,
    pub channel: u8,
    pub ssid_len: u8,
    pub ssid: [u8; SSID_MAX],
}

impl ViewNetwork {
    pub const EMPTY: Self = Self {
        known: false,
        connected: false,
        security: security::OPEN,
        rssi: -100,
        channel: 0,
        ssid_len: 0,
        ssid: [0; SSID_MAX],
    };

    pub fn from_entry(e: &NetworkEntry) -> Self {
        Self {
            known: e.known,
            connected: e.connected,
            security: e.security,
            rssi: e.rssi,
            channel: e.channel,
            ssid_len: e.ssid_len,
            ssid: e.ssid,
        }
    }

    pub fn ssid(&self) -> &[u8] {
        &self.ssid[..(self.ssid_len as usize).min(SSID_MAX)]
    }
}

/// Complete Display-facing snapshot of the Wi-Fi popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopoverView {
    pub open: bool,
    pub radio_on: bool,
    pub scanning: bool,
    pub has_active: bool,
    pub active_wired: bool,
    pub password_open: bool,
    pub show_password: bool,
    pub remember_checked: bool,
    pub phase: u8,
    pub x: u16,
    pub y: u16,
    pub hover: HitTarget,
    pub spin_phase: u8,
    pub net_count: u8,
    pub password_target: Option<u8>,
    pub password_len: u8,
    pub active_rssi: i8,
    pub active_security: u8,
    pub active_ipv4: [u8; 4],
    pub active_ssid_len: u8,
    pub active_ssid: [u8; SSID_MAX],
    pub status_len: u8,
    pub status_text: [u8; STATUS_MAX],
    /// Plaintext bytes ONLY when `show_password == true`; zero otherwise.
    pub visible_password: [u8; PASSWORD_MAX],
    pub networks: [ViewNetwork; MAX_NETWORKS],
}

impl Default for PopoverView {
    fn default() -> Self {
        Self::hidden()
    }
}

impl PopoverView {
    pub const fn hidden() -> Self {
        Self {
            open: false,
            radio_on: true,
            scanning: false,
            has_active: false,
            active_wired: false,
            password_open: false,
            show_password: false,
            remember_checked: true,
            phase: phase::NONE,
            x: 0,
            y: 0,
            hover: HitTarget::None,
            spin_phase: 0,
            net_count: 0,
            password_target: None,
            password_len: 0,
            active_rssi: 0,
            active_security: security::OPEN,
            active_ipv4: [0; 4],
            active_ssid_len: 0,
            active_ssid: [0; SSID_MAX],
            status_len: 0,
            status_text: [0; STATUS_MAX],
            visible_password: [0; PASSWORD_MAX],
            networks: [ViewNetwork::EMPTY; MAX_NETWORKS],
        }
    }

    pub fn active_ssid(&self) -> &[u8] {
        &self.active_ssid[..(self.active_ssid_len as usize).min(SSID_MAX)]
    }

    pub fn status(&self) -> &[u8] {
        &self.status_text[..(self.status_len as usize).min(STATUS_MAX)]
    }

    pub fn visible_password(&self) -> &[u8] {
        if !self.show_password {
            &[]
        } else {
            &self.visible_password[..(self.password_len as usize).min(PASSWORD_MAX)]
        }
    }

    pub fn networks(&self) -> &[ViewNetwork] {
        &self.networks[..(self.net_count as usize).min(MAX_NETWORKS)]
    }
}

// ── Chunked PDX Wire Serialization (`OP_WIFI_POPOVER`) ──────────────────────

pub const FIXED_WIRE_BYTES: usize = 92;
pub const NET_WIRE_BYTES: usize = 36;
pub const MAX_WIRE_BYTES: usize = FIXED_WIRE_BYTES + MAX_NETWORKS * NET_WIRE_BYTES + PASSWORD_MAX;
pub const CHUNK_BYTES: usize = 16;
pub const MAX_CHUNKS: usize = (MAX_WIRE_BYTES + CHUNK_BYTES - 1) / CHUNK_BYTES; // 28 <= 32
const CHUNK_TAG: u64 = 0xC5;

const _: () = assert!(MAX_WIRE_BYTES == 443);
const _: () = assert!(MAX_CHUNKS <= 32);

fn sanitize_ascii(src: &[u8], out: &mut [u8]) -> usize {
    let n = src.len().min(out.len());
    for i in 0..n {
        let b = src[i];
        out[i] = if (0x20..=0x7E).contains(&b) { b } else { b'?' };
    }
    n
}

impl PopoverView {
    /// Serializes this snapshot into `buf` and returns the exact payload byte count.
    pub fn encode(&self, buf: &mut [u8; MAX_WIRE_BYTES]) -> usize {
        buf.fill(0);
        if !self.open {
            return 0;
        }
        let pw_open = self.password_open && self.password_target.is_some();
        let net_n = (self.net_count as usize).min(MAX_NETWORKS);
        let pw_len = (self.password_len as usize).min(PASSWORD_MAX);
        let show_pw = pw_open && self.show_password && pw_len > 0;
        let flags = (self.open as u8)
            | ((self.radio_on as u8) << 1)
            | ((self.scanning as u8) << 2)
            | ((self.has_active as u8) << 3)
            | ((self.active_wired as u8) << 4)
            | ((pw_open as u8) << 5)
            | ((show_pw as u8) << 6)
            | ((self.remember_checked as u8) << 7);

        buf[0] = flags;
        buf[1] = self.phase;
        buf[2..4].copy_from_slice(&self.x.to_le_bytes());
        buf[4..6].copy_from_slice(&self.y.to_le_bytes());
        buf[6] = self.hover.wire();
        buf[7] = self.spin_phase & 7;
        buf[8] = net_n as u8;
        buf[9] = if pw_open { self.password_target.unwrap_or(0xFF) } else { 0xFF };
        buf[10] = pw_len as u8;
        buf[11] = self.active_rssi as u8;
        buf[12] = self.active_security;
        let active_len = (self.active_ssid_len as usize).min(SSID_MAX);
        buf[13] = active_len as u8;
        let st_len = (self.status_len as usize).min(STATUS_MAX);
        buf[14] = st_len as u8;
        buf[15] = if show_pw { pw_len as u8 } else { 0 };

        buf[16..20].copy_from_slice(&self.active_ipv4);
        sanitize_ascii(&self.active_ssid[..active_len], &mut buf[20..52]);
        sanitize_ascii(&self.status_text[..st_len], &mut buf[52..92]);

        let mut at = FIXED_WIRE_BYTES;
        for entry in &self.networks[..net_n] {
            let e_len = (entry.ssid_len as usize).min(SSID_MAX);
            buf[at] = (entry.known as u8)
                | ((entry.connected as u8) << 1)
                | ((entry.security & 0x0F) << 2);
            buf[at + 1] = entry.rssi as u8;
            buf[at + 2] = entry.channel;
            buf[at + 3] = e_len as u8;
            sanitize_ascii(&entry.ssid[..e_len], &mut buf[at + 4..at + 36]);
            at += NET_WIRE_BYTES;
        }
        if show_pw {
            sanitize_ascii(&self.visible_password[..pw_len], &mut buf[at..at + pw_len]);
            at += pw_len;
        }
        at
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return Some(Self::hidden());
        }
        if bytes.len() < FIXED_WIRE_BYTES {
            return None;
        }
        let flags = bytes[0];
        if flags & 1 == 0 {
            return Some(Self::hidden());
        }
        let phase = bytes[1];
        if phase > phase::OFF {
            return None;
        }
        let hover = HitTarget::from_wire(bytes[6])?;
        let net_n = bytes[8] as usize;
        if net_n > MAX_NETWORKS {
            return None;
        }
        let pw_target = match bytes[9] {
            0xFF => None,
            t if (t as usize) < net_n => Some(t),
            _ => return None,
        };
        let pw_len = bytes[10] as usize;
        let active_len = bytes[13] as usize;
        let st_len = bytes[14] as usize;
        let vis_pw_len = bytes[15] as usize;
        if pw_len > PASSWORD_MAX
            || active_len > SSID_MAX
            || st_len > STATUS_MAX
            || vis_pw_len > PASSWORD_MAX
        {
            return None;
        }
        let expected_len = FIXED_WIRE_BYTES + net_n * NET_WIRE_BYTES + vis_pw_len;
        if bytes.len() != expected_len {
            return None;
        }

        let show_password = (flags >> 6) & 1 != 0;
        if vis_pw_len > 0 && (!show_password || pw_target.is_none() || vis_pw_len != pw_len) {
            return None;
        }

        let mut view = Self::hidden();
        view.open = true;
        view.radio_on = (flags >> 1) & 1 != 0;
        view.scanning = (flags >> 2) & 1 != 0;
        view.has_active = (flags >> 3) & 1 != 0;
        view.active_wired = (flags >> 4) & 1 != 0;
        view.password_open = ((flags >> 5) & 1 != 0) && pw_target.is_some();
        view.show_password = show_password && view.password_open;
        view.remember_checked = (flags >> 7) & 1 != 0;
        view.phase = phase;
        view.x = u16::from_le_bytes([bytes[2], bytes[3]]);
        view.y = u16::from_le_bytes([bytes[4], bytes[5]]);
        view.hover = hover;
        view.spin_phase = bytes[7] & 7;
        view.net_count = net_n as u8;
        view.password_target = if view.password_open { pw_target } else { None };
        view.password_len = pw_len as u8;
        view.active_rssi = bytes[11] as i8;
        view.active_security = bytes[12];
        view.active_ssid_len = active_len as u8;
        view.status_len = st_len as u8;
        view.active_ipv4.copy_from_slice(&bytes[16..20]);
        sanitize_ascii(&bytes[20..20 + active_len], &mut view.active_ssid[..active_len]);
        sanitize_ascii(&bytes[52..52 + st_len], &mut view.status_text[..st_len]);

        let mut at = FIXED_WIRE_BYTES;
        for i in 0..net_n {
            let nf = bytes[at];
            let elen = (bytes[at + 3] as usize).min(SSID_MAX);
            if bytes[at + 3] as usize > SSID_MAX {
                return None;
            }
            view.networks[i].known = nf & 1 != 0;
            view.networks[i].connected = (nf >> 1) & 1 != 0;
            view.networks[i].security = (nf >> 2) & 0x0F;
            view.networks[i].rssi = bytes[at + 1] as i8;
            view.networks[i].channel = bytes[at + 2];
            view.networks[i].ssid_len = elen as u8;
            sanitize_ascii(&bytes[at + 4..at + 4 + elen], &mut view.networks[i].ssid[..elen]);
            at += NET_WIRE_BYTES;
        }
        if show_password && vis_pw_len > 0 {
            sanitize_ascii(&bytes[at..at + vis_pw_len], &mut view.visible_password[..vis_pw_len]);
        }
        Some(view)
    }
}

/// One chunk header inside `OP_WIFI_POPOVER`'s `arg0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub idx: u8,
    pub total: u8,
    pub total_bytes: u16,
    pub generation: u16,
}

impl Chunk {
    pub const fn word(self) -> u64 {
        (self.idx as u64)
            | ((self.total as u64) << 8)
            | ((self.total_bytes as u64) << 16)
            | ((self.generation as u64) << 32)
            | (CHUNK_TAG << 56)
    }

    pub fn decode(w: u64) -> Option<Self> {
        if (w >> 56) != CHUNK_TAG || ((w >> 48) & 0xFF) != 0 {
            return None;
        }
        let c = Self {
            idx: w as u8,
            total: (w >> 8) as u8,
            total_bytes: (w >> 16) as u16,
            generation: (w >> 32) as u16,
        };
        if c.total == 0 {
            return (c.idx == 0 && c.total_bytes == 0).then_some(c);
        }
        if c.total as usize > MAX_CHUNKS
            || c.idx >= c.total
            || (c.total_bytes as usize) > MAX_WIRE_BYTES
            || (c.total_bytes as usize) < FIXED_WIRE_BYTES
            || (c.total_bytes as usize + CHUNK_BYTES - 1) / CHUNK_BYTES != c.total as usize
        {
            return None;
        }
        Some(c)
    }
}

/// Encodes `view` into `(arg0, arg1, arg2)` PDX chunk tuples.
/// A closed view produces a single hide call (`total = 0`).
pub fn chunks(view: &PopoverView, generation: u16) -> ChunkIter {
    if !view.open {
        return ChunkIter {
            buf: [0; MAX_WIRE_BYTES],
            total_bytes: 0,
            total_chunks: 0,
            generation,
            idx: 0,
            done_hide: false,
        };
    }
    let mut buf = [0u8; MAX_WIRE_BYTES];
    let n = view.encode(&mut buf);
    let total_chunks = ((n + CHUNK_BYTES - 1) / CHUNK_BYTES).max(1) as u8;
    ChunkIter {
        buf,
        total_bytes: n as u16,
        total_chunks,
        generation,
        idx: 0,
        done_hide: true,
    }
}

pub struct ChunkIter {
    buf: [u8; MAX_WIRE_BYTES],
    total_bytes: u16,
    total_chunks: u8,
    generation: u16,
    idx: u8,
    done_hide: bool,
}

impl Iterator for ChunkIter {
    type Item = (u64, u64, u64);

    fn next(&mut self) -> Option<Self::Item> {
        if self.total_chunks == 0 {
            if self.done_hide {
                return None;
            }
            self.done_hide = true;
            let c = Chunk { idx: 0, total: 0, total_bytes: 0, generation: self.generation };
            return Some((c.word(), 0, 0));
        }
        if self.idx >= self.total_chunks {
            return None;
        }
        let k = self.idx;
        self.idx += 1;
        let off = k as usize * CHUNK_BYTES;
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        let limit = (self.total_bytes as usize).min(MAX_WIRE_BYTES);
        for i in 0..8 {
            if off + i < limit {
                a[i] = self.buf[off + i];
            }
            if off + 8 + i < limit {
                b[i] = self.buf[off + 8 + i];
            }
        }
        let c = Chunk {
            idx: k,
            total: self.total_chunks,
            total_bytes: self.total_bytes,
            generation: self.generation,
        };
        Some((c.word(), u64::from_le_bytes(a), u64::from_le_bytes(b)))
    }
}

/// Display-side zero-heap chunk reassembler.
pub struct Assembler {
    buf: [u8; MAX_WIRE_BYTES],
    generation: u16,
    total_chunks: u8,
    total_bytes: u16,
    seen_mask: u32,
}

impl Default for Assembler {
    fn default() -> Self {
        Self::new()
    }
}

impl Assembler {
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_WIRE_BYTES],
            generation: 0,
            total_chunks: 0,
            total_bytes: 0,
            seen_mask: 0,
        }
    }

    pub fn push(&mut self, word: u64, arg1: u64, arg2: u64) -> Option<PopoverView> {
        let c = Chunk::decode(word)?;
        if c.total == 0 {
            self.reset();
            return Some(PopoverView::hidden());
        }
        if c.idx == 0 {
            self.buf.fill(0);
            self.generation = c.generation;
            self.total_chunks = c.total;
            self.total_bytes = c.total_bytes;
            self.seen_mask = 0;
        } else if self.total_chunks != c.total
            || self.total_bytes != c.total_bytes
            || self.generation != c.generation
        {
            return None;
        }

        let off = c.idx as usize * CHUNK_BYTES;
        let end = (c.total_bytes as usize).min(MAX_WIRE_BYTES);
        let mut chunk16 = [0u8; 16];
        chunk16[..8].copy_from_slice(&arg1.to_le_bytes());
        chunk16[8..16].copy_from_slice(&arg2.to_le_bytes());
        for (i, &byte) in chunk16.iter().enumerate() {
            if off + i < end {
                self.buf[off + i] = byte;
            }
        }
        self.seen_mask |= 1u32 << c.idx;
        let complete = if c.total == 32 {
            u32::MAX
        } else {
            (1u32 << c.total) - 1
        };
        if self.seen_mask != complete {
            return None;
        }
        let view = PopoverView::decode(&self.buf[..end]);
        self.reset();
        view
    }

    fn reset(&mut self) {
        self.total_chunks = 0;
        self.total_bytes = 0;
        self.seen_mask = 0;
    }
}

// ── Shared Pixel Layout ─────────────────────────────────────────────────────

/// Screen-clamped pixel layout shared by Shell hit-testing and Display rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WifiLayout {
    pub scale: u32,
    pub panel: Rect,
    pub header: Rect,
    pub rescan: Rect,
    pub toggle: Rect,
    pub status_banner: Option<Rect>,
    pub card: Option<Rect>,
    pub disconnect: Option<Rect>,
    pub off_box: Option<Rect>,
    pub section_hdr: Option<Rect>,
    pub empty_row: Option<Rect>,
    pub visible_nets: usize,
    pub net_rows: [Rect; MAX_NETWORKS],
    pub drawer: Option<Rect>,
    pub password_box: Option<Rect>,
    pub eye_btn: Option<Rect>,
    pub remember_btn: Option<Rect>,
    pub cancel_btn: Option<Rect>,
    pub join_btn: Option<Rect>,
    pub prefs_row: Rect,
}

impl WifiLayout {
    pub fn new(view: &PopoverView, screen_w: usize, screen_h: usize, scale: u32) -> Option<Self> {
        if !view.open {
            return None;
        }
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = PANEL_W * s;
            let margin = 6 * s;
            if screen_w < pw + 2 * margin || screen_h < 140 * s {
                return None;
            }
            let raw_nets = (view.net_count as usize).min(MAX_NETWORKS);
            let show_card = view.radio_on
                && view.phase != phase::OFF
                && (view.has_active || view.active_wired || view.phase == phase::NONE);
            let show_banner = !view.status().is_empty() && !view.password_open;

            // If tall screens are unavailable at this scale, try scaling down first before trimming rows.
            for max_nets in (0..=raw_nets).rev() {
                if sc > 1 && max_nets < raw_nets {
                    continue;
                }
                let built = Self::build_at(view, sc, screen_w, screen_h, pw, max_nets, show_card, show_banner);
                if built.panel.3 + 2 * margin <= screen_h {
                    return Some(built);
                }
            }
            None
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_at(
        view: &PopoverView,
        sc: u32,
        screen_w: usize,
        screen_h: usize,
        pw: usize,
        visible_nets: usize,
        show_card: bool,
        show_banner: bool,
    ) -> Self {
        let s = sc as usize;
        let margin = 6 * s;
        let pad = 10 * s;
        let iw = pw - 2 * pad;

        let header_h = 34 * s;
        let div_gap = 6 * s;
        let banner_h = if show_banner { 22 * s } else { 0 };
        let card_h = if show_card { 54 * s } else { 0 };
        let card_gap = if show_card { 6 * s } else { 0 };
        let off_h = if !view.radio_on || view.phase == phase::OFF { 52 * s } else { 0 };

        let show_other = view.radio_on && view.phase != phase::OFF;
        let sec_h = if show_other { 22 * s } else { 0 };
        let row_h = 30 * s;
        let empty_h = if show_other && visible_nets == 0 { 28 * s } else { 0 };

        let pw_target = view.password_target.filter(|&t| view.password_open && (t as usize) < visible_nets);
        let drawer_extra = if pw_target.is_some() && !view.status().is_empty() { 16 * s } else { 0 };
        let drawer_h = if pw_target.is_some() { 76 * s + drawer_extra } else { 0 };

        let footer_gap = 6 * s;
        let footer_h = 28 * s;

        let ph = pad
            + header_h
            + div_gap
            + banner_h
            + card_h
            + card_gap
            + off_h
            + sec_h
            + empty_h
            + visible_nets * row_h
            + drawer_h
            + footer_gap
            + footer_h
            + pad;

        let raw_x = (view.x as usize).saturating_sub(pw / 2);
        let x = raw_x.clamp(margin, screen_w.saturating_sub(pw + margin));
        let y = (view.y as usize).min(screen_h.saturating_sub(ph + margin));
        let ix = x + pad;
        let mut cy = y + pad;

        let header = (ix, cy, iw, header_h);
        let toggle_w = 42 * s;
        let toggle_h = 22 * s;
        let toggle = (ix + iw - toggle_w, cy + (header_h - toggle_h) / 2, toggle_w, toggle_h);
        let rescan_w = 62 * s;
        let rescan_h = 22 * s;
        let rescan = if show_other {
            (toggle.0.saturating_sub(rescan_w + 8 * s), cy + (header_h - rescan_h) / 2, rescan_w, rescan_h)
        } else {
            (0, 0, 0, 0)
        };
        cy += header_h + div_gap;

        let status_banner = if show_banner {
            let r = (ix, cy, iw, banner_h);
            cy += banner_h;
            Some(r)
        } else {
            None
        };

        let (card, disconnect) = if show_card {
            let c = (ix, cy, iw, card_h);
            let can_disconnect = phase::is_active(view.phase) && !view.active_wired;
            let disc = if can_disconnect {
                let dw = 78 * s;
                let dh = 24 * s;
                Some((c.0 + c.2 - dw - 8 * s, c.1 + (card_h - dh) / 2, dw, dh))
            } else {
                None
            };
            cy += card_h + card_gap;
            (Some(c), disc)
        } else {
            (None, None)
        };

        let off_box = if off_h > 0 {
            let r = (ix, cy, iw, off_h);
            cy += off_h;
            Some(r)
        } else {
            None
        };

        let section_hdr = if sec_h > 0 {
            let r = (ix, cy, iw, sec_h);
            cy += sec_h;
            Some(r)
        } else {
            None
        };

        let empty_row = if empty_h > 0 {
            let r = (ix, cy, iw, empty_h);
            cy += empty_h;
            Some(r)
        } else {
            None
        };

        let mut net_rows = [(0, 0, 0, 0); MAX_NETWORKS];
        let mut drawer = None;
        let mut password_box = None;
        let mut eye_btn = None;
        let mut remember_btn = None;
        let mut cancel_btn = None;
        let mut join_btn = None;

        for (i, slot) in net_rows.iter_mut().enumerate().take(visible_nets) {
            *slot = (ix, cy, iw, row_h);
            cy += row_h;
            if pw_target == Some(i as u8) {
                let dr = (ix, cy + 2 * s, iw, drawer_h - 4 * s);
                let din_x = dr.0 + 8 * s;
                let din_w = dr.2.saturating_sub(16 * s);
                let eye_w = 54 * s;
                let in_h = 26 * s;
                let in_y = dr.1 + 7 * s;
                let pb = (din_x, in_y, din_w.saturating_sub(eye_w + 6 * s), in_h);
                let eb = (pb.0 + pb.2 + 6 * s, in_y, eye_w, in_h);

                let btn_y = dr.1 + dr.3 - 30 * s;
                let btn_h = 24 * s;
                let join_w = 54 * s;
                let cancel_w = 56 * s;
                let jb = (dr.0 + dr.2 - 8 * s - join_w, btn_y, join_w, btn_h);
                let cb = (jb.0.saturating_sub(cancel_w + 6 * s), btn_y, cancel_w, btn_h);
                let rem_w = cb.0.saturating_sub(din_x + 6 * s);
                let rb = (din_x, btn_y, rem_w, btn_h);

                drawer = Some(dr);
                password_box = Some(pb);
                eye_btn = Some(eb);
                remember_btn = Some(rb);
                cancel_btn = Some(cb);
                join_btn = Some(jb);
                cy += drawer_h;
            }
        }

        cy += footer_gap;
        let prefs_row = (ix, cy, iw, footer_h);

        Self {
            scale: sc,
            panel: (x, y, pw, ph),
            header,
            rescan,
            toggle,
            status_banner,
            card,
            disconnect,
            off_box,
            section_hdr,
            empty_row,
            visible_nets,
            net_rows,
            drawer,
            password_box,
            eye_btn,
            remember_btn,
            cancel_btn,
            join_btn,
            prefs_row,
        }
    }

    pub fn hit(&self, px: i32, py: i32) -> HitTarget {
        if px < 0 || py < 0 {
            return HitTarget::Outside;
        }
        let (x, y) = (px as usize, py as usize);
        if !rect_contains(self.panel, x, y) {
            return HitTarget::Outside;
        }
        if rect_contains(self.toggle, x, y) {
            return HitTarget::Toggle;
        }
        if rect_contains(self.rescan, x, y) {
            return HitTarget::Rescan;
        }
        if let Some(d) = self.disconnect {
            if rect_contains(d, x, y) {
                return HitTarget::Disconnect;
            }
        }
        if let Some(eb) = self.eye_btn {
            if rect_contains(eb, x, y) {
                return HitTarget::EyeToggle;
            }
        }
        if let Some(rb) = self.remember_btn {
            if rect_contains(rb, x, y) {
                return HitTarget::RememberToggle;
            }
        }
        if let Some(cb) = self.cancel_btn {
            if rect_contains(cb, x, y) {
                return HitTarget::CancelPassword;
            }
        }
        if let Some(jb) = self.join_btn {
            if rect_contains(jb, x, y) {
                return HitTarget::JoinSubmit;
            }
        }
        if let Some(pb) = self.password_box {
            if rect_contains(pb, x, y) {
                return HitTarget::PasswordBox;
            }
        }
        for i in 0..self.visible_nets {
            if rect_contains(self.net_rows[i], x, y) {
                return HitTarget::NetworkRow(i as u8);
            }
        }
        if rect_contains(self.prefs_row, x, y) {
            return HitTarget::NetworkPrefs;
        }
        HitTarget::Inside
    }
}

// ── Popover Controller & Key/Click Logic ────────────────────────────────────

/// Side-effect requested by `Controller` for Shell glue to execute against `sexnet`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverAction {
    None,
    SetRadio(bool),
    Rescan,
    Join { scan_index: u8, has_password: bool },
    Disconnect,
    OpenNetworkPrefs,
    Closed,
}

/// US PS/2 / USB scancode fallback for printable ASCII (used in tests and US fallback).
pub fn scancode_ascii(code: u8, shift: bool) -> Option<char> {
    let ch = match (code, shift) {
        (0x02, false) => b'1', (0x02, true) => b'!',
        (0x03, false) => b'2', (0x03, true) => b'@',
        (0x04, false) => b'3', (0x04, true) => b'#',
        (0x05, false) => b'4', (0x05, true) => b'$',
        (0x06, false) => b'5', (0x06, true) => b'%',
        (0x07, false) => b'6', (0x07, true) => b'^',
        (0x08, false) => b'7', (0x08, true) => b'&',
        (0x09, false) => b'8', (0x09, true) => b'*',
        (0x0A, false) => b'9', (0x0A, true) => b'(',
        (0x0B, false) => b'0', (0x0B, true) => b')',
        (0x0C, false) => b'-', (0x0C, true) => b'_',
        (0x0D, false) => b'=', (0x0D, true) => b'+',
        (0x10, false) => b'q', (0x10, true) => b'Q',
        (0x11, false) => b'w', (0x11, true) => b'W',
        (0x12, false) => b'e', (0x12, true) => b'E',
        (0x13, false) => b'r', (0x13, true) => b'R',
        (0x14, false) => b't', (0x14, true) => b'T',
        (0x15, false) => b'y', (0x15, true) => b'Y',
        (0x16, false) => b'u', (0x16, true) => b'U',
        (0x17, false) => b'i', (0x17, true) => b'I',
        (0x18, false) => b'o', (0x18, true) => b'O',
        (0x19, false) => b'p', (0x19, true) => b'P',
        (0x1A, false) => b'[', (0x1A, true) => b'{',
        (0x1B, false) => b']', (0x1B, true) => b'}',
        (0x1E, false) => b'a', (0x1E, true) => b'A',
        (0x1F, false) => b's', (0x1F, true) => b'S',
        (0x20, false) => b'd', (0x20, true) => b'D',
        (0x21, false) => b'f', (0x21, true) => b'F',
        (0x22, false) => b'g', (0x22, true) => b'G',
        (0x23, false) => b'h', (0x23, true) => b'H',
        (0x24, false) => b'j', (0x24, true) => b'J',
        (0x25, false) => b'k', (0x25, true) => b'K',
        (0x26, false) => b'l', (0x26, true) => b'L',
        (0x27, false) => b';', (0x27, true) => b':',
        (0x28, false) => b'\'', (0x28, true) => b'"',
        (0x29, false) => b'`', (0x29, true) => b'~',
        (0x2B, false) => b'\\', (0x2B, true) => b'|',
        (0x2C, false) => b'z', (0x2C, true) => b'Z',
        (0x2D, false) => b'x', (0x2D, true) => b'X',
        (0x2E, false) => b'c', (0x2E, true) => b'C',
        (0x2F, false) => b'v', (0x2F, true) => b'V',
        (0x30, false) => b'b', (0x30, true) => b'B',
        (0x31, false) => b'n', (0x31, true) => b'N',
        (0x32, false) => b'm', (0x32, true) => b'M',
        (0x33, false) => b',', (0x33, true) => b'<',
        (0x34, false) => b'.', (0x34, true) => b'>',
        (0x35, false) => b'/', (0x35, true) => b'?',
        (0x39, _) => b' ',
        _ => return None,
    };
    Some(ch as char)
}

pub const fn is_modifier_scancode(code: u8) -> bool {
    matches!(code, 0x1D | 0x2A | 0x36 | 0x38 | 0x3A | 0x5B | 0x5C | 0x61 | 0x64)
}

/// Complete Shell-side controller for the Live Wi-Fi Scanner popover.
pub struct Controller {
    open: bool,
    anchor_x: u16,
    anchor_y: u16,
    radio_on: bool,
    phase: u8,
    scanning: bool,
    spin_phase: u8,
    has_active: bool,
    active_ssid_len: u8,
    active_ssid: [u8; SSID_MAX],
    active_rssi: i8,
    active_security: u8,
    active_ipv4: [u8; 4],
    raw: [RawNetwork; MAX_RAW_NETWORKS],
    raw_count: u8,
    networks: [NetworkEntry; MAX_NETWORKS],
    net_count: u8,
    pub known: KnownNetworks,
    password_target: Option<u8>,
    target_ssid_len: u8,
    target_ssid: [u8; SSID_MAX],
    password_buf: [u8; PASSWORD_MAX],
    password_len: u8,
    show_password: bool,
    remember_network: bool,
    status_len: u8,
    status_text: [u8; STATUS_MAX],
    hover: HitTarget,
    /// Transient credential buffer handed to Shell glue upon Join submission.
    pending_join_pass: [u8; PASSWORD_MAX],
    pending_join_pass_len: u8,
    /// Credentials queued to save into `known` once association/handshake connects.
    pending_save_ssid: [u8; SSID_MAX],
    pending_save_ssid_len: u8,
    pending_save_pass: [u8; PASSWORD_MAX],
    pending_save_pass_len: u8,
    pending_save_armed: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    pub const fn new() -> Self {
        Self {
            open: false,
            anchor_x: 0,
            anchor_y: 0,
            radio_on: true,
            phase: phase::NONE,
            scanning: false,
            spin_phase: 0,
            has_active: false,
            active_ssid_len: 0,
            active_ssid: [0; SSID_MAX],
            active_rssi: 0,
            active_security: security::OPEN,
            active_ipv4: [0; 4],
            raw: [RawNetwork::EMPTY; MAX_RAW_NETWORKS],
            raw_count: 0,
            networks: [NetworkEntry::EMPTY; MAX_NETWORKS],
            net_count: 0,
            known: KnownNetworks::new(),
            password_target: None,
            target_ssid_len: 0,
            target_ssid: [0; SSID_MAX],
            password_buf: [0; PASSWORD_MAX],
            password_len: 0,
            show_password: false,
            remember_network: true,
            status_len: 0,
            status_text: [0; STATUS_MAX],
            hover: HitTarget::None,
            pending_join_pass: [0; PASSWORD_MAX],
            pending_join_pass_len: 0,
            pending_save_ssid: [0; SSID_MAX],
            pending_save_ssid_len: 0,
            pending_save_pass: [0; PASSWORD_MAX],
            pending_save_pass_len: 0,
            pending_save_armed: false,
        }
    }

    pub const fn is_open(&self) -> bool {
        self.open
    }

    pub const fn is_password_focused(&self) -> bool {
        self.open && self.password_target.is_some()
    }

    pub const fn scanning(&self) -> bool {
        self.scanning
    }

    pub const fn connecting(&self) -> bool {
        phase::is_connecting(self.phase)
    }

    pub const fn radio_on(&self) -> bool {
        self.radio_on
    }

    pub const fn phase(&self) -> u8 {
        self.phase
    }

    pub const fn raw_count(&self) -> u8 {
        self.raw_count
    }

    pub fn networks(&self) -> &[NetworkEntry] {
        &self.networks[..(self.net_count as usize).min(MAX_NETWORKS)]
    }

    pub fn password_bytes(&self) -> &[u8] {
        &self.password_buf[..(self.password_len as usize).min(PASSWORD_MAX)]
    }

    pub fn open_at(&mut self, anchor_x: i32, anchor_y: i32) {
        self.open = true;
        self.anchor_x = anchor_x.clamp(0, u16::MAX as i32) as u16;
        self.anchor_y = anchor_y.clamp(0, u16::MAX as i32) as u16;
        self.hover = HitTarget::None;
        if self.radio_on && self.phase != phase::NONE && self.phase != phase::OFF {
            self.scanning = true;
        }
    }

    pub fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        self.hover = HitTarget::None;
        self.clear_password();
        self.clear_status();
        true
    }

    pub fn clear_password(&mut self) {
        self.password_target = None;
        self.target_ssid_len = 0;
        scrub(&mut self.target_ssid);
        self.password_len = 0;
        scrub(&mut self.password_buf);
        self.show_password = false;
        self.remember_network = true;
    }

    fn clear_pending_join(&mut self) {
        self.pending_join_pass_len = 0;
        scrub(&mut self.pending_join_pass);
    }

    fn clear_pending_save(&mut self) {
        self.pending_save_armed = false;
        self.pending_save_ssid_len = 0;
        self.pending_save_pass_len = 0;
        scrub(&mut self.pending_save_ssid);
        scrub(&mut self.pending_save_pass);
    }

    /// Takes and immediately scrubs the passphrase queued by the latest `Join`.
    pub fn take_pending_join_password(&mut self, out: &mut [u8; PASSWORD_MAX]) -> usize {
        let len = (self.pending_join_pass_len as usize).min(PASSWORD_MAX);
        out[..len].copy_from_slice(&self.pending_join_pass[..len]);
        self.clear_pending_join();
        len
    }

    pub fn set_status(&mut self, msg: &[u8]) {
        self.status_text = [0; STATUS_MAX];
        let n = sanitize_ascii(msg, &mut self.status_text);
        self.status_len = n as u8;
    }

    pub fn status(&self) -> &[u8] {
        &self.status_text[..(self.status_len as usize).min(STATUS_MAX)]
    }

    pub fn clear_status(&mut self) {
        self.status_len = 0;
        self.status_text = [0; STATUS_MAX];
    }

    fn rebuild_networks(&mut self) {
        if !self.radio_on || self.phase == phase::OFF {
            self.net_count = 0;
            self.networks = [NetworkEntry::EMPTY; MAX_NETWORKS];
            self.clear_password();
            return;
        }
        let mut active_clean = [0u8; SSID_MAX];
        let exclude = if self.has_active && phase::is_active(self.phase) && self.active_ssid_len > 0 {
            let n = clean_ssid(&self.active_ssid[..(self.active_ssid_len as usize).min(SSID_MAX)], &mut active_clean);
            &active_clean[..n as usize]
        } else {
            &[][..]
        };
        let count = dedup_and_sort(
            &self.raw[..(self.raw_count as usize).min(MAX_RAW_NETWORKS)],
            exclude,
            &self.known,
            &mut self.networks,
        );
        self.net_count = count as u8;

        // Preserve open inline password drawer across scan updates by matching SSID.
        if self.password_target.is_some() && self.target_ssid_len > 0 {
            let want = &self.target_ssid[..(self.target_ssid_len as usize).min(SSID_MAX)];
            if let Some(pos) = self.networks[..count].iter().position(|e| e.ssid() == want && e.security != security::OPEN) {
                self.password_target = Some(pos as u8);
            } else {
                self.clear_password();
            }
        }
    }

    /// Replaces the raw scan table and updates sorted/deduplicated `networks`.
    pub fn set_raw_scan(&mut self, entries: &[RawNetwork]) {
        let n = entries.len().min(MAX_RAW_NETWORKS);
        self.raw = [RawNetwork::EMPTY; MAX_RAW_NETWORKS];
        self.raw[..n].copy_from_slice(&entries[..n]);
        self.raw_count = n as u8;
        self.rebuild_networks();
    }

    /// Updates state from `sexnet` replies (`STATE`, `ACTIVE`, `WHY`).
    pub fn apply_state(
        &mut self,
        new_phase: u8,
        active_ssid: Option<&[u8]>,
        active_rssi: i8,
        active_security: u8,
        ipv4: [u8; 4],
        why: &[u8],
    ) {
        self.phase = new_phase;
        if new_phase == phase::OFF {
            self.radio_on = false;
            self.scanning = false;
            self.has_active = false;
            self.active_ssid_len = 0;
            self.active_ssid = [0; SSID_MAX];
            self.clear_pending_save();
            self.clear_status();
            self.rebuild_networks();
            return;
        }
        if new_phase != phase::NONE {
            self.radio_on = true;
        }
        self.scanning = matches!(new_phase, phase::STARTING | phase::SCANNING);
        self.active_ipv4 = ipv4;

        if phase::is_active(new_phase) {
            if let Some(ssid) = active_ssid.filter(|s| !s.is_empty()) {
                let len = clean_ssid(ssid, &mut self.active_ssid);
                self.active_ssid_len = len;
                self.active_rssi = active_rssi;
                self.active_security = active_security;
                self.has_active = true;
            }
        } else {
            self.has_active = false;
            self.active_ssid_len = 0;
            self.active_ssid = [0; SSID_MAX];
            self.active_rssi = 0;
        }

        if phase::is_connected(new_phase) {
            if self.pending_save_armed && self.pending_save_ssid_len > 0 {
                let sl = self.pending_save_ssid_len as usize;
                let pl = self.pending_save_pass_len as usize;
                let mut ss = [0u8; SSID_MAX];
                let mut pw = [0u8; PASSWORD_MAX];
                ss[..sl].copy_from_slice(&self.pending_save_ssid[..sl]);
                pw[..pl].copy_from_slice(&self.pending_save_pass[..pl]);
                self.known.remember(&ss[..sl], &pw[..pl]);
                scrub(&mut pw);
            }
            self.clear_pending_save();
            if self.status().starts_with(b"Connecting") {
                self.clear_status();
            }
        } else if new_phase == phase::FAILED {
            // If authentication/handshake failed, drop pending save (never store bad passwords).
            self.clear_pending_save();
            if !why.is_empty() {
                self.set_status(why);
            } else {
                self.set_status(b"Connection failed");
            }
        } else if new_phase == phase::READY && self.status().starts_with(b"Connecting") {
            self.clear_status();
        }
        self.rebuild_networks();
    }

    pub fn step_spinner(&mut self) -> bool {
        if !self.open || (!self.scanning && !self.connecting()) {
            return false;
        }
        self.spin_phase = (self.spin_phase + 1) & 7;
        true
    }

    pub fn hover(&mut self, px: i32, py: i32, screen_w: usize, screen_h: usize, scale: u32) -> bool {
        if !self.open {
            return false;
        }
        let view = self.view();
        let next = WifiLayout::new(&view, screen_w, screen_h, scale)
            .map(|l| l.hit(px, py))
            .filter(|h| h.is_interactive())
            .unwrap_or(HitTarget::None);
        if next == self.hover {
            return false;
        }
        self.hover = next;
        true
    }

    pub fn press(
        &mut self,
        px: i32,
        py: i32,
        screen_w: usize,
        screen_h: usize,
        scale: u32,
    ) -> (bool, PopoverAction) {
        if !self.open {
            return (false, PopoverAction::None);
        }
        let view = self.view();
        let hit = WifiLayout::new(&view, screen_w, screen_h, scale)
            .map_or(HitTarget::Outside, |l| l.hit(px, py));
        match hit {
            HitTarget::Outside => {
                self.close();
                (true, PopoverAction::Closed)
            }
            HitTarget::Inside | HitTarget::PasswordBox | HitTarget::None => (false, PopoverAction::None),
            HitTarget::Toggle => {
                self.radio_on = !self.radio_on;
                self.clear_status();
                if !self.radio_on {
                    self.scanning = false;
                    self.has_active = false;
                    self.clear_pending_save();
                    self.clear_password();
                    self.phase = phase::OFF;
                    self.rebuild_networks();
                } else {
                    self.scanning = true;
                    if self.phase == phase::OFF {
                        self.phase = phase::STARTING;
                    }
                }
                (true, PopoverAction::SetRadio(self.radio_on))
            }
            HitTarget::Rescan => {
                if !self.radio_on || self.phase == phase::NONE || self.phase == phase::OFF {
                    return (false, PopoverAction::None);
                }
                self.scanning = true;
                self.clear_status();
                (true, PopoverAction::Rescan)
            }
            HitTarget::Disconnect => {
                self.clear_password();
                self.clear_pending_save();
                self.has_active = false;
                self.active_ssid_len = 0;
                self.clear_status();
                if self.radio_on && self.phase != phase::NONE {
                    self.phase = phase::STARTING;
                    self.scanning = true;
                }
                self.rebuild_networks();
                (true, PopoverAction::Disconnect)
            }
            HitTarget::EyeToggle => {
                if self.password_target.is_some() {
                    self.show_password = !self.show_password;
                    (true, PopoverAction::None)
                } else {
                    (false, PopoverAction::None)
                }
            }
            HitTarget::RememberToggle => {
                if self.password_target.is_some() {
                    self.remember_network = !self.remember_network;
                    (true, PopoverAction::None)
                } else {
                    (false, PopoverAction::None)
                }
            }
            HitTarget::CancelPassword => {
                if self.password_target.is_some() {
                    self.clear_password();
                    self.clear_status();
                    (true, PopoverAction::None)
                } else {
                    (false, PopoverAction::None)
                }
            }
            HitTarget::JoinSubmit => self.submit_password(),
            HitTarget::NetworkRow(idx) => self.click_network(idx),
            HitTarget::NetworkPrefs => {
                self.close();
                (true, PopoverAction::OpenNetworkPrefs)
            }
        }
    }

    fn click_network(&mut self, idx: u8) -> (bool, PopoverAction) {
        let i = idx as usize;
        if i >= (self.net_count as usize).min(MAX_NETWORKS) {
            return (false, PopoverAction::None);
        }
        let net = self.networks[i];
        if net.security == security::OPEN {
            self.clear_password();
            self.clear_pending_join();
            self.clear_pending_save();
            self.set_status(b"Connecting...");
            return (true, PopoverAction::Join { scan_index: net.scan_index, has_password: false });
        }
        if net.known {
            let mut copy = [0u8; PASSWORD_MAX];
            let len = self
                .known
                .password_for(net.ssid())
                .map(|saved_pw| {
                    let l = saved_pw.len().min(PASSWORD_MAX);
                    copy[..l].copy_from_slice(&saved_pw[..l]);
                    l
                })
                .unwrap_or(0);
            if len >= PASSWORD_MIN {
                self.clear_password();
                self.clear_pending_join();
                self.pending_join_pass[..len].copy_from_slice(&copy[..len]);
                self.pending_join_pass_len = len as u8;
                scrub(&mut copy);
                self.set_status(b"Connecting...");
                return (true, PopoverAction::Join { scan_index: net.scan_index, has_password: true });
            }
            scrub(&mut copy);
        }
        if net.security == security::UNSUPPORTED {
            self.clear_password();
            self.set_status(b"Unsupported security mode");
            return (true, PopoverAction::None);
        }
        // Toggle closed if clicking the same secured row again; else open inline drawer.
        if self.password_target == Some(idx) {
            self.clear_password();
            self.clear_status();
        } else {
            self.clear_password();
            self.clear_status();
            self.password_target = Some(idx);
            self.target_ssid_len = net.ssid_len;
            self.target_ssid = net.ssid;
            self.remember_network = true;
        }
        (true, PopoverAction::None)
    }

    fn submit_password(&mut self) -> (bool, PopoverAction) {
        let Some(idx) = self.password_target else {
            return (false, PopoverAction::None);
        };
        let i = idx as usize;
        if i >= (self.net_count as usize).min(MAX_NETWORKS) {
            self.clear_password();
            return (true, PopoverAction::None);
        }
        let len = self.password_len as usize;
        if !(PASSWORD_MIN..=PASSWORD_MAX).contains(&len) {
            self.set_status(b"Passphrase needs 8-63 characters");
            return (true, PopoverAction::None);
        }
        let net = self.networks[i];
        self.clear_pending_join();
        self.pending_join_pass[..len].copy_from_slice(&self.password_buf[..len]);
        self.pending_join_pass_len = len as u8;

        self.clear_pending_save();
        if self.remember_network {
            self.pending_save_armed = true;
            self.pending_save_ssid_len = net.ssid_len;
            self.pending_save_ssid = net.ssid;
            self.pending_save_pass_len = len as u8;
            self.pending_save_pass[..len].copy_from_slice(&self.password_buf[..len]);
        }

        self.clear_password();
        self.set_status(b"Connecting...");
        (true, PopoverAction::Join { scan_index: net.scan_index, has_password: true })
    }

    /// Pastes ASCII bytes (`0x20..=0x7E`) into the active inline password field.
    pub fn paste_bytes(&mut self, bytes: &[u8]) -> bool {
        if !self.is_password_focused() {
            return false;
        }
        let mut added = false;
        for &b in bytes {
            if (self.password_len as usize) >= PASSWORD_MAX {
                break;
            }
            if (0x20..=0x7E).contains(&b) {
                self.password_buf[self.password_len as usize] = b;
                self.password_len += 1;
                added = true;
            }
        }
        if added {
            self.clear_status();
        }
        added
    }

    /// Handles a keyboard edge while the Wi-Fi popover is open.
    /// Returns `(consumed, redraw_needed, action)`.
    pub fn handle_key_event(
        &mut self,
        code: u8,
        value: u64,
        control: bool,
        super_held: bool,
        typed: Option<char>,
        clipboard: Option<&[u8]>,
    ) -> (bool, bool, PopoverAction) {
        if !self.open {
            return (false, false, PopoverAction::None);
        }
        // Let releases and modifier edges pass so focus windows don't retain stuck modifiers.
        if value == 0 || is_modifier_scancode(code) {
            return (false, false, PopoverAction::None);
        }
        if code == 0x01 {
            // Esc first cancels inline password entry cleanly, then dismisses the popover.
            if self.password_target.is_some() {
                self.clear_password();
                self.clear_status();
                return (true, true, PopoverAction::None);
            }
            self.close();
            return (true, true, PopoverAction::Closed);
        }
        if self.password_target.is_none() {
            return (false, false, PopoverAction::None);
        }

        // Inline password field is focused: take all non-modifier presses.
        match code {
            0x1C | 0x9C => {
                let (redraw, action) = self.submit_password();
                (true, redraw, action)
            }
            0x0E => {
                if self.password_len > 0 {
                    self.password_len -= 1;
                    self.password_buf[self.password_len as usize] = 0;
                    self.clear_status();
                    (true, true, PopoverAction::None)
                } else {
                    (true, false, PopoverAction::None)
                }
            }
            0x0F => {
                self.show_password = !self.show_password;
                (true, true, PopoverAction::None)
            }
            0x2F if control || super_held => {
                let pasted = self.paste_bytes(clipboard.unwrap_or(&[]));
                (true, pasted, PopoverAction::None)
            }
            0x16 if control => {
                if self.password_len > 0 {
                    self.password_len = 0;
                    scrub(&mut self.password_buf);
                    self.clear_status();
                    (true, true, PopoverAction::None)
                } else {
                    (true, false, PopoverAction::None)
                }
            }
            _ => {
                if control || super_held {
                    return (true, false, PopoverAction::None);
                }
                if let Some(ch) = typed {
                    if ch.is_ascii() && (0x20..=0x7E).contains(&(ch as u32)) {
                        if (self.password_len as usize) < PASSWORD_MAX {
                            self.password_buf[self.password_len as usize] = ch as u8;
                            self.password_len += 1;
                            self.clear_status();
                            return (true, true, PopoverAction::None);
                        }
                    }
                }
                (true, false, PopoverAction::None)
            }
        }
    }

    /// Builds the Display-safe snapshot (never copies password bytes unless `show_password == true`).
    pub fn view(&self) -> PopoverView {
        if !self.open {
            return PopoverView::hidden();
        }
        let mut out = PopoverView {
            open: true,
            radio_on: self.radio_on,
            scanning: self.scanning,
            has_active: self.has_active,
            active_wired: self.phase == phase::NONE && self.radio_on,
            password_open: self.password_target.is_some(),
            show_password: self.show_password && self.password_target.is_some(),
            remember_checked: self.remember_network,
            phase: self.phase,
            x: self.anchor_x,
            y: self.anchor_y,
            hover: self.hover,
            spin_phase: self.spin_phase,
            net_count: self.net_count,
            password_target: self.password_target,
            password_len: self.password_len,
            active_rssi: self.active_rssi,
            active_security: self.active_security,
            active_ipv4: self.active_ipv4,
            active_ssid_len: self.active_ssid_len,
            active_ssid: self.active_ssid,
            status_len: self.status_len,
            status_text: self.status_text,
            visible_password: [0; PASSWORD_MAX],
            networks: [ViewNetwork::EMPTY; MAX_NETWORKS],
        };
        let n = (self.net_count as usize).min(MAX_NETWORKS);
        for i in 0..n {
            out.networks[i] = ViewNetwork::from_entry(&self.networks[i]);
        }
        if out.show_password {
            let pl = (self.password_len as usize).min(PASSWORD_MAX);
            out.visible_password[..pl].copy_from_slice(&self.password_buf[..pl]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(r: Rect) -> (i32, i32) {
        ((r.0 + r.2 / 2) as i32, (r.1 + r.3 / 2) as i32)
    }

    fn type_ascii(ctrl: &mut Controller, code: u8, shift: bool) -> (bool, bool, PopoverAction) {
        let ch = scancode_ascii(code, shift);
        ctrl.handle_key_event(code, 1, false, false, ch, None)
    }

    #[test]
    fn signal_bars_map_rssi_thresholds() {
        assert_eq!(signal_bars(0), 0);
        assert_eq!(signal_bars(-96), 0);
        assert_eq!(signal_bars(-95), 1);
        assert_eq!(signal_bars(-79), 1);
        assert_eq!(signal_bars(-78), 2);
        assert_eq!(signal_bars(-68), 2);
        assert_eq!(signal_bars(-67), 3);
        assert_eq!(signal_bars(-56), 3);
        assert_eq!(signal_bars(-55), 4);
        assert_eq!(signal_bars(-38), 4);
    }

    #[test]
    fn dedup_keeps_strongest_ap_sorts_by_rssi_and_caps_at_eight() {
        let mut known = KnownNetworks::new();
        assert!(known.remember(b"HomeMesh", b"secret-key"));

        let raw = [
            RawNetwork::new(0, 6, -78, security::WPA2, b"HomeMesh"),
            RawNetwork::new(1, 1, -90, security::OPEN, b""),
            RawNetwork::new(2, 11, -60, security::WPA2, b"Cafe_Guest"),
            RawNetwork::new(3, 36, -50, security::WPA3, b"ActiveNet"),
            RawNetwork::new(4, 149, -42, security::WPA2, b"HomeMesh"),
            RawNetwork::new(5, 44, -55, security::WPA2_WPA3, b"Office5G"),
            RawNetwork::new(6, 1, -82, security::OPEN, b"AirportFree"),
            RawNetwork::new(7, 6, -64, security::WPA2, b"Library"),
            RawNetwork::new(8, 11, -71, security::WPA2, b"Studio"),
            RawNetwork::new(9, 40, -68, security::OPEN, b"HotelLobby"),
            RawNetwork::new(10, 48, -48, security::WPA2, b"LabWiFi"),
            RawNetwork::new(11, 1, -85, security::WPA2, b"WeakAP"),
            RawNetwork::new(12, 11, -52, security::WPA2, b"Office5G"),
        ];

        let mut out = [NetworkEntry::EMPTY; MAX_NETWORKS];
        let count = dedup_and_sort(&raw, b"ActiveNet", &known, &mut out);
        assert_eq!(count, MAX_NETWORKS);

        // ActiveNet & empty SSID excluded; strongest HomeMesh (-42, idx 4) & Office5G (-52, idx 12) kept.
        assert_eq!(out[0].ssid(), b"HomeMesh");
        assert_eq!((out[0].rssi, out[0].scan_index, out[0].known), (-42, 4, true));
        assert_eq!(out[1].ssid(), b"LabWiFi");
        assert_eq!((out[1].rssi, out[1].scan_index, out[1].known), (-48, 10, false));
        assert_eq!(out[2].ssid(), b"Office5G");
        assert_eq!((out[2].rssi, out[2].scan_index), (-52, 12));
        assert_eq!(out[3].ssid(), b"Cafe_Guest");
        assert_eq!(out[4].ssid(), b"Library");
        assert_eq!(out[5].ssid(), b"HotelLobby");
        assert_eq!(out[6].ssid(), b"Studio");
        assert_eq!(out[7].ssid(), b"AirportFree");
        for i in 1..count {
            assert!(out[i - 1].rssi >= out[i].rssi);
            assert_ne!(out[i].ssid(), b"ActiveNet");
        }
    }

    #[test]
    fn known_networks_remember_lru_evict_and_forget() {
        let mut kn = KnownNetworks::new();
        for i in 0..(MAX_KNOWN + 2) {
            let ssid = [b'N', b'e', b't', b'0' + i as u8];
            assert!(kn.remember(&ssid, b"12345678"));
        }
        assert_eq!(kn.len(), MAX_KNOWN);
        assert!(!kn.is_known(b"Net0"));
        assert!(!kn.is_known(b"Net1"));
        assert!(kn.is_known(b"Net2"));
        assert_eq!(kn.password_for(b"Net9"), Some(b"12345678".as_slice()));
        assert!(kn.forget(b"Net9"));
        assert!(!kn.is_known(b"Net9"));
        assert_eq!(kn.password_for(b"Net9"), None);
    }

    #[test]
    fn wire_chunks_assembler_round_trip_and_password_masking() {
        let mut c = Controller::new();
        assert!(c.known.remember(b"SavedNet", b"old-secret-pw"));
        c.set_raw_scan(&[
            RawNetwork::new(0, 6, -45, security::WPA2, b"SavedNet"),
            RawNetwork::new(1, 36, -54, security::WPA2_WPA3, b"LockNet"),
            RawNetwork::new(2, 1, -66, security::OPEN, b"GuestOpen"),
        ]);
        c.apply_state(phase::CONNECTED, Some(b"Work5G"), -41, security::WPA2, [192, 168, 1, 77], b"");
        c.open_at(1040, 32);

        // Open inline password drawer on LockNet (row 1).
        let view0 = c.view();
        let lay0 = WifiLayout::new(&view0, 1280, 800, 1).unwrap();
        let (rx, ry) = center(lay0.net_rows[1]);
        let (redraw, action) = c.press(rx, ry, 1280, 800, 1);
        assert!(redraw && action == PopoverAction::None);
        type_ascii(&mut c, 0x1E, false); // 'a'
        type_ascii(&mut c, 0x30, true);  // 'B'
        assert_eq!(c.password_bytes(), b"aB");

        // While show_password == false, Display snapshot MUST NOT carry password bytes!
        let hidden_view = c.view();
        assert!(!hidden_view.show_password);
        assert_eq!(hidden_view.password_len, 2);
        assert!(hidden_view.visible_password.iter().all(|&b| b == 0));

        let mut asm = Assembler::new();
        let mut decoded = None;
        for (w0, w1, w2) in chunks(&hidden_view, 7) {
            decoded = asm.push(w0, w1, w2);
        }
        assert_eq!(decoded, Some(hidden_view));

        // Corrupt chunk generation must be ignored cleanly.
        let mut iter = chunks(&hidden_view, 8);
        let first = iter.next().unwrap();
        let second = iter.next().unwrap();
        assert!(asm.push(first.0, first.1, first.2).is_none());
        let wrong_gen = second.0 ^ (1u64 << 32);
        assert!(asm.push(wrong_gen, second.1, second.2).is_none());

        // Toggle Show eye button: visible_password now carries sanitized text cleanly.
        let lay1 = WifiLayout::new(&hidden_view, 1280, 800, 1).unwrap();
        let (ex, ey) = center(lay1.eye_btn.unwrap());
        let (r2, a2) = c.press(ex, ey, 1280, 800, 1);
        assert!(r2 && a2 == PopoverAction::None);
        let shown_view = c.view();
        assert!(shown_view.show_password);
        assert_eq!(shown_view.visible_password(), b"aB");

        let mut decoded2 = None;
        for (w0, w1, w2) in chunks(&shown_view, 9) {
            decoded2 = asm.push(w0, w1, w2);
        }
        assert_eq!(decoded2, Some(shown_view));

        // Hidden view produces single close chunk.
        c.close();
        let mut close_chunks = chunks(&c.view(), 10);
        let cw = close_chunks.next().unwrap();
        assert!(close_chunks.next().is_none());
        assert_eq!(asm.push(cw.0, cw.1, cw.2), Some(PopoverView::hidden()));
    }

    #[test]
    fn hit_testing_covers_all_controls_and_screen_bounds() {
        let mut c = Controller::new();
        c.set_raw_scan(&[
            RawNetwork::new(0, 6, -44, security::WPA2, b"Alpha"),
            RawNetwork::new(1, 1, -60, security::OPEN, b"Beta"),
        ]);
        c.apply_state(phase::CONNECTED, Some(b"CurNet"), -49, security::WPA2, [10, 0, 2, 15], b"");
        c.open_at(1260, 32);

        // Open inline password on Alpha (row 0) so drawer controls are visible.
        let v_init = c.view();
        let l_init = WifiLayout::new(&v_init, 1280, 800, 1).unwrap();
        assert!(l_init.panel.0 + l_init.panel.2 <= 1280);
        let (ax, ay) = center(l_init.net_rows[0]);
        c.press(ax, ay, 1280, 800, 1);

        let v = c.view();
        for scale in [1u32, 2u32] {
            let l = WifiLayout::new(&v, 1280, 800, scale).unwrap();
            assert!(l.panel.0 + l.panel.2 <= 1280);
            assert!(l.panel.1 + l.panel.3 <= 800);
            assert_eq!(l.hit(center(l.toggle).0, center(l.toggle).1), HitTarget::Toggle);
            assert_eq!(l.hit(center(l.rescan).0, center(l.rescan).1), HitTarget::Rescan);
            let disc = l.disconnect.unwrap();
            assert_eq!(l.hit(center(disc).0, center(disc).1), HitTarget::Disconnect);
            assert_eq!(l.hit(center(l.net_rows[0]).0, center(l.net_rows[0]).1), HitTarget::NetworkRow(0));
            assert_eq!(l.hit(center(l.net_rows[1]).0, center(l.net_rows[1]).1), HitTarget::NetworkRow(1));
            assert_eq!(l.hit(center(l.password_box.unwrap()).0, center(l.password_box.unwrap()).1), HitTarget::PasswordBox);
            assert_eq!(l.hit(center(l.eye_btn.unwrap()).0, center(l.eye_btn.unwrap()).1), HitTarget::EyeToggle);
            assert_eq!(l.hit(center(l.remember_btn.unwrap()).0, center(l.remember_btn.unwrap()).1), HitTarget::RememberToggle);
            assert_eq!(l.hit(center(l.cancel_btn.unwrap()).0, center(l.cancel_btn.unwrap()).1), HitTarget::CancelPassword);
            assert_eq!(l.hit(center(l.join_btn.unwrap()).0, center(l.join_btn.unwrap()).1), HitTarget::JoinSubmit);
            assert_eq!(l.hit(center(l.prefs_row).0, center(l.prefs_row).1), HitTarget::NetworkPrefs);
            assert_eq!(l.hit(10, 700), HitTarget::Outside);
        }
    }

    #[test]
    fn one_click_open_known_and_inline_password_typing_paste_enter_flow() {
        let mut c = Controller::new();
        assert!(c.known.remember(b"MyHome", b"home-pass-99"));
        c.set_raw_scan(&[
            RawNetwork::new(3, 6, -45, security::OPEN, b"CafeOpen"),
            RawNetwork::new(7, 36, -52, security::WPA2, b"MyHome"),
            RawNetwork::new(9, 149, -58, security::WPA2_WPA3, b"Corp5G"),
        ]);
        c.apply_state(phase::READY, None, 0, security::OPEN, [10, 0, 2, 15], b"");
        c.open_at(1050, 32);

        // 1. One-click Open join
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (_, act) = c.press(center(l.net_rows[0]).0, center(l.net_rows[0]).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::Join { scan_index: 3, has_password: false });

        // 2. One-click Known join
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (_, act) = c.press(center(l.net_rows[1]).0, center(l.net_rows[1]).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::Join { scan_index: 7, has_password: true });
        let mut pw = [0u8; PASSWORD_MAX];
        let plen = c.take_pending_join_password(&mut pw);
        assert_eq!(&pw[..plen], b"home-pass-99");
        // Second take is scrubbed.
        assert_eq!(c.take_pending_join_password(&mut pw), 0);

        // 3. Unknown secured network expands inline password entry
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (_, act) = c.press(center(l.net_rows[2]).0, center(l.net_rows[2]).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::None);
        assert!(c.is_password_focused());

        // Short passphrase (< 8 chars) rejected with status message
        type_ascii(&mut c, 0x1E, false); // 'a'
        type_ascii(&mut c, 0x30, false); // 'b'
        let (cons, redr, act) = c.handle_key_event(0x1C, 1, false, false, None, None);
        assert!(cons && redr && act == PopoverAction::None);
        assert!(c.status().starts_with(b"Passphrase needs"));

        // Backspace removes 'b', Ctrl+V pastes remaining valid ASCII chars, filtering control bytes.
        let (cons, redr, _) = c.handle_key_event(0x0E, 1, false, false, None, None);
        assert!(cons && redr);
        assert_eq!(c.password_bytes(), b"a");
        let (cons, redr, _) = c.handle_key_event(0x2F, 1, true, false, None, Some(b"123\x07456\n789"));
        assert!(cons && redr);
        assert_eq!(c.password_bytes(), b"a123456789");

        // Tab toggles password visibility
        assert!(!c.view().show_password);
        let _ = c.handle_key_event(0x0F, 1, false, false, None, None);
        assert!(c.view().show_password);

        // Enter submits Join & clears password box
        let (cons, redr, act) = c.handle_key_event(0x1C, 1, false, false, None, None);
        assert!(cons && redr && !c.is_password_focused());
        assert_eq!(act, PopoverAction::Join { scan_index: 9, has_password: true });
        let plen2 = c.take_pending_join_password(&mut pw);
        assert_eq!(&pw[..plen2], b"a123456789");

        // Authentication succeeds -> Corp5G is saved to KnownNetworks!
        c.apply_state(phase::CONNECTED, Some(b"Corp5G"), -58, security::WPA2_WPA3, [192, 168, 2, 10], b"");
        assert!(c.known.is_known(b"Corp5G"));
        assert_eq!(c.known.password_for(b"Corp5G"), Some(b"a123456789".as_slice()));

        // Disconnect removes active connection; Corp5G returns to list with known=true.
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let disc = l.disconnect.unwrap();
        let (_, act) = c.press(center(disc).0, center(disc).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::Disconnect);
        c.apply_state(phase::READY, None, 0, security::OPEN, [10, 0, 2, 15], b"");
        let corp = c.networks().iter().find(|n| n.ssid() == b"Corp5G").unwrap();
        assert!(corp.known);
    }

    #[test]
    fn failed_handshake_does_not_save_bad_passphrase_and_esc_hierarchy() {
        let mut c = Controller::new();
        c.set_raw_scan(&[RawNetwork::new(2, 6, -50, security::WPA2, b"StrictAP")]);
        c.apply_state(phase::READY, None, 0, security::OPEN, [0; 4], b"");
        c.open_at(1000, 32);

        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        c.press(center(l.net_rows[0]).0, center(l.net_rows[0]).1, 1280, 800, 1);
        assert!(c.is_password_focused());
        assert!(c.paste_bytes(b"wrong-password-1"));

        // First Esc closes inline drawer ONLY, keeping popover open!
        let (cons, redr, act) = c.handle_key_event(0x01, 1, false, false, None, None);
        assert!(cons && redr && act == PopoverAction::None);
        assert!(c.is_open() && !c.is_password_focused());

        // Second Esc dismisses popover.
        let (cons, redr, act) = c.handle_key_event(0x01, 1, false, false, None, None);
        assert!(cons && redr && act == PopoverAction::Closed && !c.is_open());

        // Reopen, submit wrong password, fail handshake -> must NOT become known.
        c.open_at(1000, 32);
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        c.press(center(l.net_rows[0]).0, center(l.net_rows[0]).1, 1280, 800, 1);
        assert!(c.paste_bytes(b"wrong-password-1"));
        let (_, _, act) = c.handle_key_event(0x1C, 1, false, false, None, None);
        assert_eq!(act, PopoverAction::Join { scan_index: 2, has_password: true });
        c.apply_state(phase::FAILED, None, 0, security::OPEN, [0; 4], b"handshake Refused");
        assert!(!c.known.is_known(b"StrictAP"));
        assert_eq!(c.status(), b"handshake Refused");
    }

    #[test]
    fn background_scan_preserves_password_drawer_ssid_binding_and_toggle_controls() {
        let mut c = Controller::new();
        c.set_raw_scan(&[
            RawNetwork::new(0, 1, -48, security::WPA2, b"NetOne"),
            RawNetwork::new(1, 6, -62, security::WPA2, b"TargetNet"),
        ]);
        c.apply_state(phase::READY, None, 0, security::OPEN, [0; 4], b"");
        c.open_at(1050, 32);

        // Expand drawer on TargetNet (row 1).
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        c.press(center(l.net_rows[1]).0, center(l.net_rows[1]).1, 1280, 800, 1);
        assert!(c.paste_bytes(b"halfpass"));
        assert_eq!(c.view().password_target, Some(1));

        // Background scan reports StrongerNew (-35) and TargetNet (-40) above NetOne (-48):
        // TargetNet moves from row 1 -> row 1, or let's push NetOne to -50 and StrongerA (-30), StrongerB (-35)
        c.set_raw_scan(&[
            RawNetwork::new(0, 36, -30, security::WPA2, b"StrongerA"),
            RawNetwork::new(1, 44, -35, security::WPA2, b"StrongerB"),
            RawNetwork::new(2, 6, -42, security::WPA2, b"TargetNet"),
        ]);
        assert_eq!(c.view().password_target, Some(2));
        assert_eq!(c.password_bytes(), b"halfpass");

        // Toggle Off clears scan list and closes password drawer.
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (_, act) = c.press(center(l.toggle).0, center(l.toggle).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::SetRadio(false));
        assert!(!c.radio_on());
        assert!(!c.is_password_focused());
        assert_eq!(c.networks().len(), 0);

        // Toggle On -> Rescan -> Network Preferences.
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (_, act) = c.press(center(l.toggle).0, center(l.toggle).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::SetRadio(true));
        let l = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (_, act) = c.press(center(l.rescan).0, center(l.rescan).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::Rescan);
        let (_, act) = c.press(center(l.prefs_row).0, center(l.prefs_row).1, 1280, 800, 1);
        assert_eq!(act, PopoverAction::OpenNetworkPrefs);
        assert!(!c.is_open());
    }

    #[test]
    fn super_v_paste_ctrl_u_clear_max_len_remember_off_and_buttons() {
        let mut c = Controller::new();
        c.set_raw_scan(&[
            RawNetwork::new(4, 36, -45, security::WPA2, b"HotelGuest"),
            RawNetwork::new(5, 6, -55, security::WPA3, b"LabGuest"),
        ]);
        c.apply_state(phase::READY, None, 0, security::OPEN, [0; 4], b"");
        c.open_at(1080, 32);

        // Expand drawer on HotelGuest (row 0), test Super+V paste + Ctrl+U clear + Cancel button.
        let l0 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        c.press(center(l0.net_rows[0]).0, center(l0.net_rows[0]).1, 1280, 800, 1);
        assert!(c.is_password_focused());

        // Super+V pastes ASCII passphrase, clamping safely at PASSWORD_MAX (63).
        let long_pw = [b'X'; 80];
        let (cons, redr, _) = c.handle_key_event(0x2F, 1, false, true, None, Some(&long_pw));
        assert!(cons && redr);
        assert_eq!(c.password_bytes().len(), PASSWORD_MAX);
        // Extra typing past PASSWORD_MAX is absorbed cleanly without growing buffer.
        let (cons_extra, redr_extra, _) = type_ascii(&mut c, 0x1E, false);
        assert!(cons_extra && !redr_extra);
        assert_eq!(c.password_bytes().len(), PASSWORD_MAX);

        // Ctrl+U clears the entire password buffer.
        let (cons_u, redr_u, _) = c.handle_key_event(0x16, 1, true, false, None, None);
        assert!(cons_u && redr_u);
        assert_eq!(c.password_bytes(), b"");

        // Cancel button collapses drawer and clears status.
        let l1 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (cx, cy) = center(l1.cancel_btn.unwrap());
        let (redr_cancel, act_cancel) = c.press(cx, cy, 1280, 800, 1);
        assert!(redr_cancel && act_cancel == PopoverAction::None);
        assert!(!c.is_password_focused());

        // Reopen on LabGuest (row 1), uncheck Remember, toggle Eye off->on->off, and click Join pill.
        let l2 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        c.press(center(l2.net_rows[1]).0, center(l2.net_rows[1]).1, 1280, 800, 1);
        let (cons_p, redr_p, _) = c.handle_key_event(0x2F, 1, false, true, None, Some(b"ephemeral-9"));
        assert!(cons_p && redr_p);

        let l3 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (rx, ry) = center(l3.remember_btn.unwrap());
        c.press(rx, ry, 1280, 800, 1);
        assert!(!c.view().remember_checked);

        let (ex, ey) = center(l3.eye_btn.unwrap());
        c.press(ex, ey, 1280, 800, 1);
        assert!(c.view().show_password);
        assert_eq!(c.view().visible_password(), b"ephemeral-9");
        c.press(ex, ey, 1280, 800, 1);
        assert!(!c.view().show_password);
        assert!(c.view().visible_password.iter().all(|&b| b == 0));

        let l4 = WifiLayout::new(&c.view(), 1280, 800, 1).unwrap();
        let (jx, jy) = center(l4.join_btn.unwrap());
        let (_, act_join) = c.press(jx, jy, 1280, 800, 1);
        assert_eq!(act_join, PopoverAction::Join { scan_index: 5, has_password: true });
        let mut pw = [0u8; PASSWORD_MAX];
        let plen = c.take_pending_join_password(&mut pw);
        assert_eq!(&pw[..plen], b"ephemeral-9");

        // Connection succeeds, but Remember was unchecked so credentials MUST NOT be saved.
        c.apply_state(phase::CONNECTED, Some(b"LabGuest"), -55, security::WPA3, [192, 168, 4, 20], b"");
        assert!(!c.known.is_known(b"LabGuest"));
        assert!(c.view().has_active);
        assert_eq!(c.view().active_ssid(), b"LabGuest");
    }
}
