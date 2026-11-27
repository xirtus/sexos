// SPDX-License-Identifier: GPL-3.0-only
//! Persistent Wi-Fi (`/data/wifi.conf`) and Tether (`/data/tether.conf`) auto-save
//! and boot-time auto-reconnect for Spindle.
//!
//! File formats:
//! - `/data/wifi.conf`: `<ssid>\n<passphrase>\n` (or `<ssid>\n` for open networks,
//!   or `<index> <passphrase>`).
//! - `/data/tether.conf`: `<host[:port]>\n` (default port 7177).
//!
//! All disk I/O is non-blocking, runs only when `sexfiles` is idle, uses its own
//! sequence namespace (`0x7B_0000`), and is a silent no-op when the files do not
//! exist (such as on fresh QEMU boots).

pub const DATA_DIR: &[u8] = b"/data";
pub const WIFI_CONF_PATH: &[u8] = b"/data/wifi.conf";
pub const TETHER_CONF_PATH: &[u8] = b"/data/tether.conf";
pub const DEFAULT_TETHER_PORT: u16 = 7177;

/// Parsed contents of `/data/wifi.conf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiConf<'a> {
    Ssid { ssid: &'a [u8], pass: &'a [u8] },
    Index { index: u64, pass: &'a [u8] },
}

fn trim_cr(s: &[u8]) -> &[u8] {
    if let [rest @ .., b'\r'] = s { rest } else { s }
}

fn trim_ws(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t' | b'\r' | b'\n', rest @ ..] = s { s = rest; }
    while let [rest @ .., b' ' | b'\t' | b'\r' | b'\n'] = s { s = rest; }
    s
}

/// Parses `/data/wifi.conf` (`<ssid>\n<passphrase>\n`, `<ssid>\n`, or `<index> <passphrase>`).
pub fn parse_wifi_conf(data: &[u8]) -> Option<WifiConf<'_>> {
    let mut lines = data.split(|&b| b == b'\n').map(trim_cr);
    let line1 = lines.next()?;
    if line1.is_empty() { return None; }
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
    enum IoPhase { Idle, ReadingWifi, ReadingTether, MkdirData, StatFile, CreateFile, TruncateFile, WriteFile }

    struct PendingWrite {
        active: bool,
        verbose: bool,
        buf: [u8; 144],
        len: usize,
    }
    const WRITE_NONE: PendingWrite = PendingWrite { active: false, verbose: false, buf: [0; 144], len: 0 };

    pub struct AutoConf {
        boot_on: bool,
        boot: BootPhase,
        verbose: bool,
        io: IoPhase,
        seq: u32,
        sent_at: u64,
        saving_which: u8, // 1 = wifi, 2 = tether
        saving_verbose: bool,
        active_buf: [u8; 144],
        active_len: usize,
        save_wifi: PendingWrite,
        save_tether: PendingWrite,
        pending_tether: [u8; 128],
        pending_tether_len: usize,
        wait_tether_until: u64,
    }

    pub static mut AUTOCONF: AutoConf = AutoConf::new();

    fn region() -> Option<&'static mut [u8]> {
        let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
        Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
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
                active_buf: [0; 144],
                active_len: 0,
                save_wifi: WRITE_NONE,
                save_tether: WRITE_NONE,
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

        /// True while `AUTOCONF` owns the `sexfiles` shared region.
        pub fn io_busy(&self) -> bool { self.io != IoPhase::Idle }

        /// True while the main loop should keep polling `AUTOCONF`.
        pub fn busy(&self) -> bool {
            self.io != IoPhase::Idle
                || self.save_wifi.active
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

        /// Queues a non-blocking write of `/data/wifi.conf`.
        pub fn queue_save_wifi(&mut self, ssid: &[u8], pass: &[u8], verbose: bool) {
            if ssid.is_empty() || ssid.len() > 32 || pass.len() > 63 { return; }
            let n = format_wifi_conf(ssid, pass, &mut self.save_wifi.buf);
            self.save_wifi.len = n;
            self.save_wifi.verbose = verbose;
            self.save_wifi.active = true;
        }

        /// Queues a non-blocking write of `/data/tether.conf`.
        pub fn queue_save_tether(&mut self, host: &[u8], port: u16) {
            if host.is_empty() || host.len() > 127 { return; }
            let n = format_tether_conf(host, port, &mut self.save_tether.buf);
            self.save_tether.len = n;
            self.save_tether.verbose = false;
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

            // Priority 1: persist queued `/data/wifi.conf` or `/data/tether.conf`.
            if self.save_wifi.active {
                self.saving_which = 1;
                self.saving_verbose = self.save_wifi.verbose;
                self.active_len = self.save_wifi.len;
                self.active_buf[..self.active_len].copy_from_slice(&self.save_wifi.buf[..self.active_len]);
                self.save_wifi.active = false;
                if self.send_fs(w::MKDIR, DATA_DIR, 0, 0, None) {
                    self.io = IoPhase::MkdirData;
                }
                return;
            }
            if self.save_tether.active {
                self.saving_which = 2;
                self.saving_verbose = self.save_tether.verbose;
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
                    if self.send_fs(w::READ, WIFI_CONF_PATH, 0, 256, None) {
                        self.io = IoPhase::ReadingWifi;
                    }
                }
                BootPhase::ReadWifi => {
                    if self.send_fs(w::READ, WIFI_CONF_PATH, 0, 256, None) {
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
                    if status == w::OK && value > 0 {
                        if let Some(reg) = region() {
                            let n = (value as usize).min(256).min(w::DATA_MAX);
                            let mut copy = [0u8; 256];
                            copy[..n].copy_from_slice(&reg[w::PATH_BYTES..w::PATH_BYTES + n]);
                            if let Some(conf) = parse_wifi_conf(&copy[..n]) {
                                sex_pdx::serial_record!("[spindle.autoconf.wifi] loaded=1 bytes={}", n);
                                match conf {
                                    WifiConf::Ssid { ssid, pass } => {
                                        sb.push(b"boot-reconnect: reconnecting saved Wi-Fi...");
                                        unsafe { (*core::ptr::addr_of_mut!(crate::wifi_command::WIFI)).start_auto_join_ssid(ssid, pass, sb); }
                                    }
                                    WifiConf::Index { index, pass } => {
                                        sb.push(b"boot-reconnect: reconnecting saved Wi-Fi index...");
                                        unsafe { (*core::ptr::addr_of_mut!(crate::wifi_command::WIFI)).start_auto_join_index(index, pass, sb); }
                                    }
                                }
                            }
                        }
                    } else if self.verbose {
                        sb.push(b"boot-reconnect: no /data/wifi.conf found.");
                    }
                    self.boot = BootPhase::ReadTether;
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
                        let buf = self.active_buf;
                        if self.send_fs(w::WRITE, path, 0, len as u64, Some(&buf[..len])) {
                            self.io = IoPhase::WriteFile;
                        }
                    } else if self.saving_verbose {
                        sb.push(b"Wi-Fi: could not create /data/wifi.conf.");
                    }
                }
                IoPhase::WriteFile => {
                    sex_pdx::serial_record!("[spindle.autoconf.saved] which={} status={} bytes={}", self.saving_which, status, value);
                    if self.saving_verbose {
                        if status == w::OK {
                            sb.push(b"Wi-Fi: saved network to /data/wifi.conf.");
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
