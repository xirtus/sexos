// SPDX-License-Identifier: GPL-3.0-only
//! Persistent Wi-Fi (`/data/wifi.conf`) and Tether (`/data/tether.conf`) auto-save
//! and boot-time auto-reconnect for Spindle.
//!
//! File formats:
//! - `/data/wifi.conf`: multi-network format (`ssid=<SSID>\tpass=<PASS>\n`, up to
//!   16 entries, most-recently-used first). Fully backward-compatible with the
//!   legacy 2-line format (`<ssid>\n<passphrase>\n`, `<ssid>\n`, or `<index> <passphrase>`).
//! - `/data/tether.conf`: `<host[:port]>\n` (default port 7177).
//!
//! All disk I/O is non-blocking, runs only when `sexfiles` is idle, uses its own
//! sequence namespace (`0x7B_0000`), and is a silent no-op when the files do not
//! exist (such as on fresh QEMU boots).

pub const DATA_DIR: &[u8] = b"/data";
pub const WIFI_CONF_PATH: &[u8] = b"/data/wifi.conf";
pub const TETHER_CONF_PATH: &[u8] = b"/data/tether.conf";
pub const DEFAULT_TETHER_PORT: u16 = 7177;

/// Maximum number of saved Wi-Fi networks stored in `/data/wifi.conf`.
pub const MAX_KNOWN_NETWORKS: usize = 16;
/// Maximum serialized byte length for `/data/wifi.conf` (16 * ~107 bytes < 2048).
pub const KNOWN_CONF_MAX: usize = 2048;

/// Parsed legacy contents of `/data/wifi.conf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiConf<'a> {
    Ssid { ssid: &'a [u8], pass: &'a [u8] },
    Index { index: u64, pass: &'a [u8] },
}

/// One saved Wi-Fi network entry (zero-heap, fixed-size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownNetwork {
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub pass: [u8; 63],
    pub pass_len: usize,
}

impl KnownNetwork {
    pub const EMPTY: Self = Self {
        ssid: [0; 32],
        ssid_len: 0,
        pass: [0; 63],
        pass_len: 0,
    };

    pub fn from_slices(ssid: &[u8], pass: &[u8]) -> Option<Self> {
        if ssid.is_empty() || ssid.len() > 32 || pass.len() > 63 {
            return None;
        }
        let mut entry = Self::EMPTY;
        entry.ssid[..ssid.len()].copy_from_slice(ssid);
        entry.ssid_len = ssid.len();
        entry.pass[..pass.len()].copy_from_slice(pass);
        entry.pass_len = pass.len();
        Some(entry)
    }

    pub fn ssid_bytes(&self) -> &[u8] {
        &self.ssid[..self.ssid_len]
    }

    pub fn pass_bytes(&self) -> &[u8] {
        &self.pass[..self.pass_len]
    }
}

/// Ordered list of up to [`MAX_KNOWN_NETWORKS`] saved Wi-Fi networks (MRU first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownNetworks {
    pub entries: [KnownNetwork; MAX_KNOWN_NETWORKS],
    pub count: usize,
}

impl KnownNetworks {
    pub const fn new() -> Self {
        Self {
            entries: [KnownNetwork::EMPTY; MAX_KNOWN_NETWORKS],
            count: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn as_slice(&self) -> &[KnownNetwork] {
        &self.entries[..self.count]
    }

    pub fn most_recent(&self) -> Option<&KnownNetwork> {
        if self.count > 0 { Some(&self.entries[0]) } else { None }
    }

    pub fn find_index(&self, ssid: &[u8]) -> Option<usize> {
        self.entries[..self.count].iter().position(|e| e.ssid_bytes() == ssid)
    }

    /// Parses `/data/wifi.conf` in either the multi-network (`ssid=<SSID>\tpass=<PASS>\n`)
    /// format or the legacy 2-line (`<ssid>\n<pass>\n` / `<ssid>\n`) format.
    pub fn parse(data: &[u8]) -> Self {
        let mut list = Self::new();
        let has_kv = data.split(|&b| b == b'\n').map(trim_cr).any(|l| l.starts_with(b"ssid="));
        if has_kv {
            for raw in data.split(|&b| b == b'\n') {
                let line = trim_cr(raw);
                if line.is_empty() || line[0] == b'#' { continue; }
                let Some((ssid, pass)) = parse_kv_wifi_line(line) else { continue };
                if list.find_index(ssid).is_some() || list.count >= MAX_KNOWN_NETWORKS {
                    continue;
                }
                if let Some(entry) = KnownNetwork::from_slices(ssid, pass) {
                    list.entries[list.count] = entry;
                    list.count += 1;
                }
            }
            return list;
        }
        if let Some(WifiConf::Ssid { ssid, pass }) = parse_wifi_conf(data) {
            if let Some(entry) = KnownNetwork::from_slices(ssid, pass) {
                list.entries[0] = entry;
                list.count = 1;
            }
        }
        list
    }

    /// Serializes all known networks as `ssid=<SSID>\tpass=<PASS>\n` (MRU first).
    pub fn serialize(&self, out: &mut [u8]) -> usize {
        let mut at = 0;
        let put = |out: &mut [u8], at: &mut usize, bytes: &[u8]| {
            let take = bytes.len().min(out.len().saturating_sub(*at));
            out[*at..*at + take].copy_from_slice(&bytes[..take]);
            *at += take;
        };
        for entry in &self.entries[..self.count] {
            put(out, &mut at, b"ssid=");
            put(out, &mut at, entry.ssid_bytes());
            put(out, &mut at, b"\tpass=");
            put(out, &mut at, entry.pass_bytes());
            put(out, &mut at, b"\n");
        }
        at
    }

    /// Inserts or updates `(ssid, pass)` at index 0 (most-recently-used first).
    pub fn upsert(&mut self, ssid: &[u8], pass: &[u8]) -> bool {
        let Some(entry) = KnownNetwork::from_slices(ssid, pass) else {
            return false;
        };
        if let Some(idx) = self.find_index(ssid) {
            for i in (1..=idx).rev() {
                self.entries[i] = self.entries[i - 1];
            }
            self.entries[0] = entry;
            return true;
        }
        let shift_end = self.count.min(MAX_KNOWN_NETWORKS - 1);
        for i in (1..=shift_end).rev() {
            self.entries[i] = self.entries[i - 1];
        }
        self.entries[0] = entry;
        if self.count < MAX_KNOWN_NETWORKS {
            self.count += 1;
        }
        true
    }

    /// Removes `ssid` from the known-networks list. Returns true if it was present.
    pub fn forget(&mut self, ssid: &[u8]) -> bool {
        let Some(idx) = self.find_index(ssid) else {
            return false;
        };
        for i in idx + 1..self.count {
            self.entries[i - 1] = self.entries[i];
        }
        self.count -= 1;
        self.entries[self.count] = KnownNetwork::EMPTY;
        true
    }

    /// Picks the strongest visible scanned SSID (highest RSSI; ties broken by MRU order)
    /// that matches any saved network. Returns `(scan_index, known_index)`.
    pub fn select_best_match(&self, scanned: &[(&[u8], i8)]) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, i8)> = None;
        for (scan_idx, &(ssid, rssi)) in scanned.iter().enumerate() {
            let Some(known_idx) = self.find_index(ssid) else { continue };
            let better = match best {
                None => true,
                Some((_, best_known, best_rssi)) => {
                    rssi > best_rssi || (rssi == best_rssi && known_idx < best_known)
                }
            };
            if better {
                best = Some((scan_idx, known_idx, rssi));
            }
        }
        best.map(|(scan_idx, known_idx, _)| (scan_idx, known_idx))
    }
}

fn parse_kv_wifi_line(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = line.strip_prefix(b"ssid=")?;
    let (ssid, pass) = if let Some(pos) = rest.windows(6).position(|w| w == b"\tpass=") {
        (&rest[..pos], &rest[pos + 6..])
    } else if let Some(pos) = rest.iter().position(|&b| b == b'\t') {
        let tail = &rest[pos + 1..];
        (&rest[..pos], tail.strip_prefix(b"pass=").unwrap_or(tail))
    } else {
        (rest, &b""[..])
    };
    if ssid.is_empty() || ssid.len() > 32 || pass.len() > 63 {
        return None;
    }
    Some((ssid, pass))
}

fn trim_cr(s: &[u8]) -> &[u8] {
    if let [rest @ .., b'\r'] = s { rest } else { s }
}

fn trim_ws(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t' | b'\r' | b'\n', rest @ ..] = s { s = rest; }
    while let [rest @ .., b' ' | b'\t' | b'\r' | b'\n'] = s { s = rest; }
    s
}

/// Parses `/data/wifi.conf` (multi-network `ssid=<SSID>\tpass=<PASS>\n`, or legacy
/// `<ssid>\n<passphrase>\n`, `<ssid>\n`, or `<index> <passphrase>`).
pub fn parse_wifi_conf(data: &[u8]) -> Option<WifiConf<'_>> {
    let mut lines = data.split(|&b| b == b'\n').map(trim_cr);
    let line1 = lines.next()?;
    if line1.is_empty() { return None; }
    if let Some((ssid, pass)) = parse_kv_wifi_line(line1) {
        return Some(WifiConf::Ssid { ssid, pass });
    }
    let line2 = lines.next().unwrap_or(b"");
    if !line2.is_empty() {
        if line1.len() > 32 || line2.len() > 63 { return None; }
        return Some(WifiConf::Ssid { ssid: line1, pass: line2 });
    }
    // Single line: check `<index>` or `<index> <passphrase>` (index 1..=64).
    let (head, tail) = match line1.iter().position(|&b| b == b' ') {
        Some(i) => (&line1[..i], &line1[i + 1..]),
        None => (line1, &b""[..]),
    };
    if !head.is_empty() && head.len() <= 2 && head.iter().all(u8::is_ascii_digit) {
        let mut idx = 0u64;
        for &b in head { idx = idx * 10 + (b - b'0') as u64; }
        if (1..=64).contains(&idx) && tail.len() <= 63 {
            return Some(WifiConf::Index { index: idx, pass: tail });
        }
    }
    if line1.len() <= 32 {
        return Some(WifiConf::Ssid { ssid: line1, pass: b"" });
    }
    None
}

/// Formats `<ssid>\n<passphrase>\n` (or `<ssid>\n` when open) into `out`.
pub fn format_wifi_conf(ssid: &[u8], pass: &[u8], out: &mut [u8]) -> usize {
    let mut at = 0;
    let put = |out: &mut [u8], at: &mut usize, bytes: &[u8]| {
        let take = bytes.len().min(out.len().saturating_sub(*at));
        out[*at..*at + take].copy_from_slice(&bytes[..take]);
        *at += take;
    };
    put(out, &mut at, ssid);
    put(out, &mut at, b"\n");
    if !pass.is_empty() {
        put(out, &mut at, pass);
        put(out, &mut at, b"\n");
    }
    at
}

/// Parses `/data/tether.conf` (`<host[:port]>\n`).
pub fn parse_tether_conf(data: &[u8]) -> Option<&[u8]> {
    for raw in data.split(|&b| b == b'\n') {
        let line = trim_ws(raw);
        if line.is_empty() || line[0] == b'#' { continue; }
        if line.len() <= 127 && !line.iter().any(|&b| b <= b' ') {
            return Some(line);
        }
        return None;
    }
    None
}

/// Formats `<host[:port]>\n` into `out`.
pub fn format_tether_conf(host: &[u8], port: u16, out: &mut [u8]) -> usize {
    let mut at = 0;
    let put = |out: &mut [u8], at: &mut usize, bytes: &[u8]| {
        let take = bytes.len().min(out.len().saturating_sub(*at));
        out[*at..*at + take].copy_from_slice(&bytes[..take]);
        *at += take;
    };
    let is_v6 = host.iter().filter(|&&b| b == b':').count() > 1;
    if port == DEFAULT_TETHER_PORT {
        put(out, &mut at, host);
    } else {
        if is_v6 { put(out, &mut at, b"["); }
        put(out, &mut at, host);
        if is_v6 { put(out, &mut at, b"]"); }
        put(out, &mut at, b":");
        let mut digits = [0u8; 6];
        let mut v = port as u32;
        let mut n = 0;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 { break; }
        }
        for i in (0..n).rev() {
            put(out, &mut at, &[digits[i]]);
        }
    }
    put(out, &mut at, b"\n");
    at
}

#[cfg(not(test))]
mod runtime {
    use super::*;
    use crate::Scrollback;
    use sex_pdx::fs as w;
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

    const SEQ_BASE: u32 = 0x7B_0000;
    const IO_TIMEOUT_MS: u64 = 10_000;
    const WAIT_WIFI_MS: u64 = 25_000;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BootPhase { NotYet, ReadWifi, ReadTether, WaitWifi, Done }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum IoPhase {
        Idle,
        ReadingWifi,
        LoadingWifiForOp,
        ReadingTether,
        MkdirData,
        StatFile,
        CreateFile,
        TruncateFile,
        WriteFile,
    }

    #[derive(Clone, Copy)]
    enum PendingWifiOp {
        None,
        Upsert { ssid: [u8; 32], ssid_len: usize, pass: [u8; 63], pass_len: usize, verbose: bool },
        Forget { ssid: [u8; 32], ssid_len: usize },
        List,
    }

    struct PendingTetherWrite {
        active: bool,
        buf: [u8; 144],
        len: usize,
    }
    const TETHER_WRITE_NONE: PendingTetherWrite = PendingTetherWrite { active: false, buf: [0; 144], len: 0 };

    pub struct AutoConf {
        boot_on: bool,
        boot: BootPhase,
        verbose: bool,
        io: IoPhase,
        seq: u32,
        sent_at: u64,
        saving_which: u8, // 1 = wifi, 2 = tether
        saving_verbose: bool,
        saving_kind: u8, // 1 = saved, 2 = forgot
        active_buf: [u8; KNOWN_CONF_MAX],
        active_len: usize,
        known: KnownNetworks,
        known_loaded: bool,
        save_wifi_pending: bool,
        save_wifi_verbose: bool,
        save_wifi_kind: u8,
        pending_op: PendingWifiOp,
        save_tether: PendingTetherWrite,
        pending_tether: [u8; 128],
        pending_tether_len: usize,
        wait_tether_until: u64,
    }

    pub static mut AUTOCONF: AutoConf = AutoConf::new();

    fn region() -> Option<&'static mut [u8]> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
        Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
    }

    fn print_saved_list(known: &KnownNetworks, sb: &mut Scrollback) {
        if known.is_empty() {
            sb.push(b"Wi-Fi: no saved networks in /data/wifi.conf.");
            return;
        }
        let mut hdr = [0u8; 64];
        let mut at = 0;
        let put = |out: &mut [u8], at: &mut usize, bytes: &[u8]| {
            let take = bytes.len().min(out.len().saturating_sub(*at));
            out[*at..*at + take].copy_from_slice(&bytes[..take]);
            *at += take;
        };
        put(&mut hdr, &mut at, b"Wi-Fi saved networks (");
        let n = known.len();
        if n >= 10 { put(&mut hdr, &mut at, &[b'0' + (n / 10) as u8]); }
        put(&mut hdr, &mut at, &[b'0' + (n % 10) as u8]);
        put(&mut hdr, &mut at, b", most recent first):");
        sb.push(&hdr[..at]);
        for (i, entry) in known.as_slice().iter().enumerate() {
            let mut line = [0u8; 96];
            let mut k = 0;
            put(&mut line, &mut k, b"  ");
            let idx = i + 1;
            if idx >= 10 { put(&mut line, &mut k, &[b'0' + (idx / 10) as u8]); }
            put(&mut line, &mut k, &[b'0' + (idx % 10) as u8]);
            put(&mut line, &mut k, b". ");
            put(&mut line, &mut k, entry.ssid_bytes());
            if entry.pass_len > 0 {
                put(&mut line, &mut k, b"  [WPA2/WPA3, auto-connect]");
            } else {
                put(&mut line, &mut k, b"  [open, auto-connect]");
            }
            sb.push(&line[..k]);
        }
    }

    impl AutoConf {
        pub const fn new() -> Self {
            Self {
                boot_on: false,
                boot: BootPhase::NotYet,
                verbose: false,
                io: IoPhase::Idle,
                seq: 0,
                sent_at: 0,
                saving_which: 0,
                saving_verbose: false,
                saving_kind: 1,
                active_buf: [0; KNOWN_CONF_MAX],
                active_len: 0,
                known: KnownNetworks::new(),
                known_loaded: false,
                save_wifi_pending: false,
                save_wifi_verbose: false,
                save_wifi_kind: 1,
                pending_op: PendingWifiOp::None,
                save_tether: TETHER_WRITE_NONE,
                pending_tether: [0; 128],
                pending_tether_len: 0,
                wait_tether_until: 0,
            }
        }

        /// Only the boot terminal performs automatic boot-time reconnect.
        pub fn init(&mut self, me: u32) {
            self.boot_on = sex_pdx::app_catalog::SPINDLE_TARGETS.first() == Some(&me);
            if !self.boot_on {
                self.boot = BootPhase::Done;
            }
        }

        /// In-memory view of saved known Wi-Fi networks (MRU first).
        pub fn known(&self) -> &KnownNetworks {
            &self.known
        }

        /// True while `AUTOCONF` owns the `sexfiles` shared region.
        pub fn io_busy(&self) -> bool { self.io != IoPhase::Idle }

        /// True while the main loop should keep polling `AUTOCONF`.
        pub fn busy(&self) -> bool {
            self.io != IoPhase::Idle
                || self.save_wifi_pending
                || !matches!(self.pending_op, PendingWifiOp::None)
                || self.save_tether.active
                || (self.boot_on && self.boot != BootPhase::Done)
                || self.boot == BootPhase::WaitWifi
        }

        /// Triggers a manual `boot-reconnect` read of `/data/wifi.conf` and `/data/tether.conf`.
        pub fn trigger_reconnect(&mut self, sb: &mut Scrollback) {
            if self.io != IoPhase::Idle {
                sb.push(b"boot-reconnect: already reading saved network settings.");
                return;
            }
            self.boot_on = true;
            self.verbose = true;
            self.pending_tether_len = 0;
            self.boot = BootPhase::ReadWifi;
            sb.push(b"boot-reconnect: checking /data/wifi.conf and /data/tether.conf...");
        }

        /// Queues a non-blocking upsert to `/data/wifi.conf` (MRU first, up to 16 networks).
        pub fn queue_save_wifi(&mut self, ssid: &[u8], pass: &[u8], verbose: bool) {
            if ssid.is_empty() || ssid.len() > 32 || pass.len() > 63 { return; }
            if self.known_loaded {
                self.known.upsert(ssid, pass);
                self.save_wifi_verbose = verbose;
                self.save_wifi_kind = 1;
                self.save_wifi_pending = true;
            } else {
                let mut s = [0u8; 32];
                s[..ssid.len()].copy_from_slice(ssid);
                let mut p = [0u8; 63];
                p[..pass.len()].copy_from_slice(pass);
                self.pending_op = PendingWifiOp::Upsert {
                    ssid: s,
                    ssid_len: ssid.len(),
                    pass: p,
                    pass_len: pass.len(),
                    verbose,
                };
            }
        }

        /// Removes `ssid` from `/data/wifi.conf` (or loads first if not yet loaded).
        pub fn queue_forget_wifi(&mut self, ssid: &[u8], sb: &mut Scrollback) {
            if ssid.is_empty() || ssid.len() > 32 {
                sb.push(b"Wi-Fi: specify a valid SSID to forget.");
                return;
            }
            if self.known_loaded {
                if self.known.forget(ssid) {
                    self.save_wifi_verbose = true;
                    self.save_wifi_kind = 2;
                    self.save_wifi_pending = true;
                } else {
                    sb.push(b"Wi-Fi: that SSID is not in /data/wifi.conf.");
                }
            } else {
                let mut s = [0u8; 32];
                s[..ssid.len()].copy_from_slice(ssid);
                self.pending_op = PendingWifiOp::Forget { ssid: s, ssid_len: ssid.len() };
            }
        }

        /// Displays saved Wi-Fi networks from `/data/wifi.conf`.
        pub fn list_saved_wifi(&mut self, sb: &mut Scrollback) {
            if self.known_loaded {
                print_saved_list(&self.known, sb);
            } else {
                self.pending_op = PendingWifiOp::List;
            }
        }

        /// Queues a non-blocking write of `/data/tether.conf`.
        pub fn queue_save_tether(&mut self, host: &[u8], port: u16) {
            if host.is_empty() || host.len() > 127 { return; }
            let n = format_tether_conf(host, port, &mut self.save_tether.buf);
            self.save_tether.len = n;
            self.save_tether.active = true;
        }

        fn send_fs(&mut self, kind: u8, path: &[u8], offset: u64, length: u64, data: Option<&[u8]>) -> bool {
            let Some(reg) = region() else { return false };
            if path.len() > w::PATH_BYTES { return false; }
            reg[..path.len()].copy_from_slice(path);
            if let Some(bytes) = data {
                if bytes.len() > w::DATA_MAX { return false; }
                reg[w::PATH_BYTES..w::PATH_BYTES + bytes.len()].copy_from_slice(bytes);
            }
            self.seq = (self.seq + 1) & 0xFFFF;
            let full_seq = SEQ_BASE | self.seq;
            let req = w::Request { seq: full_seq, kind, path_len: path.len() as u16, offset, length };
            let a = req.args();
            if sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]).is_ok() {
                self.sent_at = sex_pdx::get_ticks();
                true
            } else {
                false
            }
        }

        fn active_path(&self) -> &'static [u8] {
            if self.saving_which == 1 { WIFI_CONF_PATH } else { TETHER_CONF_PATH }
        }

        /// Advances boot-time reconnect and queued config saves when `files_idle` is true.
        pub fn step(&mut self, files_idle: bool, sb: &mut Scrollback) {
            let now = sex_pdx::get_ticks();
            if self.io != IoPhase::Idle {
                if now.saturating_sub(self.sent_at) >= IO_TIMEOUT_MS {
                    sex_pdx::serial_record!("[spindle.autoconf.timeout] io={}", self.io as u8);
                    self.io = IoPhase::Idle;
                    self.pending_op = PendingWifiOp::None;
                    if matches!(self.boot, BootPhase::ReadWifi | BootPhase::ReadTether) {
                        self.boot = BootPhase::Done;
                    }
                }
                return;
            }

            if self.boot == BootPhase::WaitWifi {
                let wifi_connected = unsafe { (*core::ptr::addr_of!(crate::wifi_command::WIFI)).is_connected() };
                let wifi_joining = unsafe { (*core::ptr::addr_of!(crate::wifi_command::WIFI)).auto_joining() };
                if wifi_connected || !wifi_joining || now >= self.wait_tether_until {
                    let len = self.pending_tether_len;
                    self.pending_tether_len = 0;
                    self.boot = BootPhase::Done;
                    if len > 0 && unsafe { !crate::tether_command::TETHER.active() } {
                        let target = self.pending_tether;
                        sb.push(b"boot-reconnect: starting saved tether...");
                        unsafe {
                            let busy = crate::fetch_command::FETCH.busy()
                                || crate::update_command::UPDATE.busy()
                                || crate::pkg_command::PKG.busy();
                            (*core::ptr::addr_of_mut!(crate::tether_command::TETHER)).command(&target[..len], sb, busy);
                        }
                    }
                }
                return;
            }

            if !files_idle { return; }

            // Priority 0: load `/data/wifi.conf` before applying a pending upsert/forget/list.
            if !matches!(self.pending_op, PendingWifiOp::None) {
                if self.send_fs(w::READ, WIFI_CONF_PATH, 0, KNOWN_CONF_MAX as u64, None) {
                    self.io = IoPhase::LoadingWifiForOp;
                }
                return;
            }

            // Priority 1: persist queued `/data/wifi.conf` or `/data/tether.conf`.
            if self.save_wifi_pending {
                self.saving_which = 1;
                self.saving_verbose = self.save_wifi_verbose;
                self.saving_kind = self.save_wifi_kind;
                self.active_len = self.known.serialize(&mut self.active_buf);
                self.save_wifi_pending = false;
                if self.send_fs(w::MKDIR, DATA_DIR, 0, 0, None) {
                    self.io = IoPhase::MkdirData;
                }
                return;
            }
            if self.save_tether.active {
                self.saving_which = 2;
                self.saving_verbose = false;
                self.saving_kind = 1;
                self.active_len = self.save_tether.len;
                self.active_buf[..self.active_len].copy_from_slice(&self.save_tether.buf[..self.active_len]);
                self.save_tether.active = false;
                if self.send_fs(w::MKDIR, DATA_DIR, 0, 0, None) {
                    self.io = IoPhase::MkdirData;
                }
                return;
            }

            // Priority 2: boot-time (or manual `boot-reconnect`) config reads.
            if !self.boot_on { return; }
            match self.boot {
                BootPhase::NotYet => {
                    self.boot = BootPhase::ReadWifi;
                    if self.send_fs(w::READ, WIFI_CONF_PATH, 0, KNOWN_CONF_MAX as u64, None) {
                        self.io = IoPhase::ReadingWifi;
                    }
                }
                BootPhase::ReadWifi => {
                    if self.send_fs(w::READ, WIFI_CONF_PATH, 0, KNOWN_CONF_MAX as u64, None) {
                        self.io = IoPhase::ReadingWifi;
                    }
                }
                BootPhase::ReadTether => {
                    if self.send_fs(w::READ, TETHER_CONF_PATH, 0, 256, None) {
                        self.io = IoPhase::ReadingTether;
                    }
                }
                BootPhase::WaitWifi | BootPhase::Done => {}
            }
        }

        /// Handles `sexfiles` replies matching `AUTOCONF`'s sequence number.
        pub fn receive(&mut self, word: u64, sb: &mut Scrollback) -> bool {
            if self.io == IoPhase::Idle { return false; }
            let Some((seq, status, value)) = w::decode_reply(word) else { return false };
            if seq != (SEQ_BASE | self.seq) { return false; }
            let io = self.io;
            self.io = IoPhase::Idle;

            match io {
                IoPhase::ReadingWifi => {
                    self.known_loaded = true;
                    if status == w::OK && value > 0 {
                        if let Some(reg) = region() {
                            let n = (value as usize).min(KNOWN_CONF_MAX).min(w::DATA_MAX);
                            let mut copy = [0u8; KNOWN_CONF_MAX];
                            copy[..n].copy_from_slice(&reg[w::PATH_BYTES..w::PATH_BYTES + n]);
                            let known = KnownNetworks::parse(&copy[..n]);
                            if !known.is_empty() {
                                self.known = known;
                                sex_pdx::serial_record!("[spindle.autoconf.wifi] loaded={} bytes={}", known.len(), n);
                                sb.push(b"boot-reconnect: scanning for saved Wi-Fi networks...");
                                unsafe { (*core::ptr::addr_of_mut!(crate::wifi_command::WIFI)).start_auto_join_known(sb); }
                            } else if let Some(WifiConf::Index { index, pass }) = parse_wifi_conf(&copy[..n]) {
                                sex_pdx::serial_record!("[spindle.autoconf.wifi] loaded=index bytes={}", n);
                                sb.push(b"boot-reconnect: reconnecting saved Wi-Fi index...");
                                unsafe { (*core::ptr::addr_of_mut!(crate::wifi_command::WIFI)).start_auto_join_index(index, pass, sb); }
                            }
                        }
                    } else if self.verbose {
                        sb.push(b"boot-reconnect: no /data/wifi.conf found.");
                    }
                    self.boot = BootPhase::ReadTether;
                }
                IoPhase::LoadingWifiForOp => {
                    self.known_loaded = true;
                    if status == w::OK && value > 0 {
                        if let Some(reg) = region() {
                            let n = (value as usize).min(KNOWN_CONF_MAX).min(w::DATA_MAX);
                            let mut copy = [0u8; KNOWN_CONF_MAX];
                            copy[..n].copy_from_slice(&reg[w::PATH_BYTES..w::PATH_BYTES + n]);
                            self.known = KnownNetworks::parse(&copy[..n]);
                        }
                    }
                    let op = self.pending_op;
                    self.pending_op = PendingWifiOp::None;
                    match op {
                        PendingWifiOp::Upsert { ssid, ssid_len, pass, pass_len, verbose } => {
                            self.known.upsert(&ssid[..ssid_len], &pass[..pass_len]);
                            self.save_wifi_verbose = verbose;
                            self.save_wifi_kind = 1;
                            self.save_wifi_pending = true;
                        }
                        PendingWifiOp::Forget { ssid, ssid_len } => {
                            if self.known.forget(&ssid[..ssid_len]) {
                                self.save_wifi_verbose = true;
                                self.save_wifi_kind = 2;
                                self.save_wifi_pending = true;
                            } else {
                                sb.push(b"Wi-Fi: that SSID is not in /data/wifi.conf.");
                            }
                        }
                        PendingWifiOp::List => {
                            print_saved_list(&self.known, sb);
                        }
                        PendingWifiOp::None => {}
                    }
                }
                IoPhase::ReadingTether => {
                    let mut started_tether = false;
                    if status == w::OK && value > 0 {
                        if let Some(reg) = region() {
                            let n = (value as usize).min(256).min(w::DATA_MAX);
                            let mut copy = [0u8; 256];
                            copy[..n].copy_from_slice(&reg[w::PATH_BYTES..w::PATH_BYTES + n]);
                            if let Some(target) = parse_tether_conf(&copy[..n]) {
                                sex_pdx::serial_record!("[spindle.autoconf.tether] loaded=1 len={}", target.len());
                                let wifi_joining = unsafe { (*core::ptr::addr_of!(crate::wifi_command::WIFI)).auto_joining() };
                                if wifi_joining {
                                    self.pending_tether[..target.len()].copy_from_slice(target);
                                    self.pending_tether_len = target.len();
                                    self.wait_tether_until = sex_pdx::get_ticks() + WAIT_WIFI_MS;
                                    self.boot = BootPhase::WaitWifi;
                                    started_tether = true;
                                } else if unsafe { !crate::tether_command::TETHER.active() } {
                                    sb.push(b"boot-reconnect: starting saved tether...");
                                    unsafe {
                                        let busy = crate::fetch_command::FETCH.busy()
                                            || crate::update_command::UPDATE.busy()
                                            || crate::pkg_command::PKG.busy();
                                        (*core::ptr::addr_of_mut!(crate::tether_command::TETHER)).command(target, sb, busy);
                                    }
                                    self.boot = BootPhase::Done;
                                    started_tether = true;
                                }
                            }
                        }
                    } else if self.verbose {
                        sb.push(b"boot-reconnect: no /data/tether.conf found.");
                    }
                    if !started_tether {
                        self.boot = BootPhase::Done;
                    }
                    self.verbose = false;
                }
                IoPhase::MkdirData => {
                    // Ignore EXISTS (or OK); proceed to STAT the target file.
                    let path = self.active_path();
                    if self.send_fs(w::STAT, path, 0, 0, None) {
                        self.io = IoPhase::StatFile;
                    }
                }
                IoPhase::StatFile => {
                    let path = self.active_path();
                    if status == w::NOT_FOUND {
                        if self.send_fs(w::CREATE, path, 0, 0, None) {
                            self.io = IoPhase::CreateFile;
                        }
                    } else if status == w::OK {
                        if self.send_fs(w::TRUNCATE, path, 0, 0, None) {
                            self.io = IoPhase::TruncateFile;
                        }
                    } else if self.saving_verbose {
                        sb.push(b"Wi-Fi: could not save /data/wifi.conf.");
                    }
                }
                IoPhase::CreateFile | IoPhase::TruncateFile => {
                    if status == w::OK {
                        let path = self.active_path();
                        let len = self.active_len;
                        if len == 0 {
                            sex_pdx::serial_record!("[spindle.autoconf.saved] which={} status=0 bytes=0", self.saving_which);
                            if self.saving_verbose {
                                sb.push(if self.saving_kind == 2 {
                                    b"Wi-Fi: forgot network from /data/wifi.conf."
                                } else {
                                    b"Wi-Fi: saved /data/wifi.conf."
                                });
                            }
                        } else {
                            let buf = self.active_buf;
                            if self.send_fs(w::WRITE, path, 0, len as u64, Some(&buf[..len])) {
                                self.io = IoPhase::WriteFile;
                            }
                        }
                    } else if self.saving_verbose {
                        sb.push(b"Wi-Fi: could not create /data/wifi.conf.");
                    }
                }
                IoPhase::WriteFile => {
                    sex_pdx::serial_record!("[spindle.autoconf.saved] which={} status={} bytes={}", self.saving_which, status, value);
                    if self.saving_verbose {
                        if status == w::OK {
                            sb.push(if self.saving_kind == 2 {
                                b"Wi-Fi: forgot network and updated /data/wifi.conf."
                            } else {
                                b"Wi-Fi: saved network to /data/wifi.conf."
                            });
                        } else {
                            sb.push(b"Wi-Fi: failed to write /data/wifi.conf.");
                        }
                    }
                }
                IoPhase::Idle => {}
            }
            true
        }
    }
}

#[cfg(not(test))]
pub use runtime::AUTOCONF;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_conf_roundtrip_ssid_and_pass() {
        let mut buf = [0u8; 128];
        let n = format_wifi_conf(b"HomeNet 5G", b"super-secret-pass", &mut buf);
        assert_eq!(&buf[..n], b"HomeNet 5G\nsuper-secret-pass\n");
        assert_eq!(
            parse_wifi_conf(&buf[..n]),
            Some(WifiConf::Ssid { ssid: b"HomeNet 5G", pass: b"super-secret-pass" })
        );
    }

    #[test]
    fn wifi_conf_parses_open_and_index_formats() {
        assert_eq!(
            parse_wifi_conf(b"CafeWiFi\n"),
            Some(WifiConf::Ssid { ssid: b"CafeWiFi", pass: b"" })
        );
        assert_eq!(
            parse_wifi_conf(b"3 mypassphrase\n"),
            Some(WifiConf::Index { index: 3, pass: b"mypassphrase" })
        );
        assert_eq!(
            parse_wifi_conf(b"2\r\n"),
            Some(WifiConf::Index { index: 2, pass: b"" })
        );
        assert_eq!(parse_wifi_conf(b""), None);
    }

    #[test]
    fn known_networks_parse_serialize_and_backward_compat() {
        // Legacy 2-line format loads cleanly as 1 known network.
        let legacy = KnownNetworks::parse(b"OfficeNet\noffice-pass-123\n");
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy.entries[0].ssid_bytes(), b"OfficeNet");
        assert_eq!(legacy.entries[0].pass_bytes(), b"office-pass-123");

        // Legacy open format loads cleanly as 1 known network with empty pass.
        let legacy_open = KnownNetworks::parse(b"AirportFree\n");
        assert_eq!(legacy_open.len(), 1);
        assert_eq!(legacy_open.entries[0].ssid_bytes(), b"AirportFree");
        assert_eq!(legacy_open.entries[0].pass_bytes(), b"");

        // Multi-network roundtrip with spaces in SSID and open network.
        let mut list = KnownNetworks::new();
        assert!(list.upsert(b"Cafe Spot", b""));
        assert!(list.upsert(b"Lab 5G", b"lab-passphrase"));
        assert!(list.upsert(b"HomeNet", b"home-pass-99"));

        let mut buf = [0u8; KNOWN_CONF_MAX];
        let n = list.serialize(&mut buf);
        assert_eq!(
            &buf[..n],
            b"ssid=HomeNet\tpass=home-pass-99\nssid=Lab 5G\tpass=lab-passphrase\nssid=Cafe Spot\tpass=\n"
        );
        let parsed = KnownNetworks::parse(&buf[..n]);
        assert_eq!(parsed, list);

        // `parse_wifi_conf` also stays compatible with the new multi-network format.
        assert_eq!(
            parse_wifi_conf(&buf[..n]),
            Some(WifiConf::Ssid { ssid: b"HomeNet", pass: b"home-pass-99" })
        );
    }

    #[test]
    fn known_networks_upsert_mru_cap_and_forget() {
        let mut list = KnownNetworks::new();
        for i in 0..20u8 {
            let mut ssid = [0u8; 8];
            ssid[..6].copy_from_slice(b"Net-00");
            ssid[4] = b'0' + (i / 10);
            ssid[5] = b'0' + (i % 10);
            assert!(list.upsert(&ssid[..6], b"pass1234"));
        }
        // Capped at MAX_KNOWN_NETWORKS (16), MRU first.
        assert_eq!(list.len(), MAX_KNOWN_NETWORKS);
        assert_eq!(list.entries[0].ssid_bytes(), b"Net-19");
        assert_eq!(list.entries[MAX_KNOWN_NETWORKS - 1].ssid_bytes(), b"Net-04");

        // Re-upserting an existing entry moves it to index 0 and updates passphrase.
        assert!(list.upsert(b"Net-07", b"new-pass-77"));
        assert_eq!(list.len(), MAX_KNOWN_NETWORKS);
        assert_eq!(list.entries[0].ssid_bytes(), b"Net-07");
        assert_eq!(list.entries[0].pass_bytes(), b"new-pass-77");
        assert_eq!(list.entries[1].ssid_bytes(), b"Net-19");

        // Forgetting removes the entry and shifts remaining entries up.
        assert!(list.forget(b"Net-19"));
        assert_eq!(list.len(), MAX_KNOWN_NETWORKS - 1);
        assert_eq!(list.find_index(b"Net-19"), None);
        assert_eq!(list.entries[0].ssid_bytes(), b"Net-07");
        assert!(!list.forget(b"DoesNotExist"));
    }

    #[test]
    fn known_networks_select_best_match_prefers_strongest_then_mru() {
        let mut list = KnownNetworks::new();
        list.upsert(b"CafeWiFi", b"");
        list.upsert(b"OfficeWiFi", b"office-pass");
        list.upsert(b"HomeWiFi", b"home-pass");
        // MRU order: 0=HomeWiFi, 1=OfficeWiFi, 2=CafeWiFi.

        // Strongest matching RSSI wins (-48 OfficeWiFi beats -65 HomeWiFi).
        let scan1: [(&[u8], i8); 3] = [
            (b"StrangerNet", -30),
            (b"HomeWiFi", -65),
            (b"OfficeWiFi", -48),
        ];
        assert_eq!(list.select_best_match(&scan1), Some((2, 1)));

        // Equal RSSI tie breaks by MRU priority (HomeWiFi idx 0 beats CafeWiFi idx 2).
        let scan2: [(&[u8], i8); 2] = [
            (b"CafeWiFi", -55),
            (b"HomeWiFi", -55),
        ];
        assert_eq!(list.select_best_match(&scan2), Some((1, 0)));

        // Empty scan returns None; fallback is `most_recent()`.
        assert_eq!(list.select_best_match(&[]), None);
        assert_eq!(list.most_recent().map(|e| e.ssid_bytes()), Some(&b"HomeWiFi"[..]));
    }

    #[test]
    fn tether_conf_roundtrip_v4_and_v6() {
        let mut buf = [0u8; 128];
        let n = format_tether_conf(b"192.168.1.42", 7177, &mut buf);
        assert_eq!(&buf[..n], b"192.168.1.42\n");
        assert_eq!(parse_tether_conf(&buf[..n]), Some(&b"192.168.1.42"[..]));

        let n2 = format_tether_conf(b"10.0.2.2", 9999, &mut buf);
        assert_eq!(&buf[..n2], b"10.0.2.2:9999\n");
        assert_eq!(parse_tether_conf(&buf[..n2]), Some(&b"10.0.2.2:9999"[..]));

        let n3 = format_tether_conf(b"fd00::1", 8080, &mut buf);
        assert_eq!(&buf[..n3], b"[fd00::1]:8080\n");
        assert_eq!(parse_tether_conf(&buf[..n3]), Some(&b"[fd00::1]:8080"[..]));

        assert_eq!(parse_tether_conf(b"# comment\n\n  192.168.1.50  \r\n"), Some(&b"192.168.1.50"[..]));
    }
}
