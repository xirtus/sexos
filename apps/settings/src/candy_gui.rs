// SPDX-License-Identifier: GPL-3.0-only
//! Flagship macOS Ventura/Sequoia Dark-Mode Noir GUI for Tatami (`apps/settings`).
//!
//! Renders a `#0B0D12` left sidebar (search pill, SexOS machine card, 11 color-badged
//! categories), `#11141D` main canvas with `#181C28` grouped rounded cards and `1px`
//! `#262D3F` top specular borders, Geist Sans proportional typography, smooth macOS
//! pill toggles, 4 Accent Tints, interactive sliders & segmented selectors, About This
//! SexOS Lips hero card, and the graphical GPT partition installer — zero heap allocation.

#[path = "noir_ui.rs"]
pub mod noir_ui;

pub use noir_ui::{
    draw_mono, draw_noir_card, draw_noir_slider, draw_noir_toggle, draw_noir_well, draw_sans,
    draw_sans_bold, draw_sans_sm, draw_sexos_lips, draw_sidebar_badge, fill_solid, noir_palette,
    Glyphs, Icon, ACCENT_TINTS,
};
#[cfg(test)]
pub use noir_ui::{NOIR_CANVAS, NOIR_CARD, NOIR_SIDEBAR, NOIR_TOP_SPECULAR};

use sex_config::{id, Config, Key, Kind, COLOUR_FILTERS, CORNER_ACTIONS, KEYS, LAYOUTS, PROGRAMS, WALLPAPERS, ZONES};
use sex_theme::candy_ui::{
    button_text_color, draw_candy_badge, draw_candy_button, draw_candy_swatch, fill_rounded_rect,
    ButtonState, ButtonStyle, CandyPalette,
};
use sex_theme::Mode;

use crate::about::Facts as KernelFacts;
use crate::facts;
use crate::grants;
use crate::model::{self, Model};
use crate::setup::{self, Setup, Step};

pub const SIDEBAR_W: i32 = 236;
pub const HEADER_H: i32 = 48;
pub const STATUS_H: i32 = 26;
pub const MAX_HITS: usize = 192;

/// Window corner radius presets (in px).
pub const CORNER_RADII: [(&str, u8); 3] = [("4 px", 4), ("8 px", 8), ("12 px", 12)];

/// Tab bar visual style presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBarStyle {
    SilkPill,
    Classic,
}
impl TabBarStyle {
    pub const fn label(self) -> &'static str {
        match self {
            TabBarStyle::SilkPill => "SilkBar Pill",
            TabBarStyle::Classic => "Classic",
        }
    }
}

/// Window tiling behavior presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TilingStyle {
    HybridCandy,
    HyprlandMaster,
    KdeFloat,
}
impl TilingStyle {
    pub const fn label(self) -> &'static str {
        match self {
            TilingStyle::HybridCandy => "Hybrid",
            TilingStyle::HyprlandMaster => "Hyprland Tile",
            TilingStyle::KdeFloat => "Floating",
        }
    }
}

/// SilkBar window close button behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseBehavior {
    QuitApp,
    MinimizeDock,
    AskFirst,
}
impl CloseBehavior {
    pub const fn label(self) -> &'static str {
        match self {
            CloseBehavior::QuitApp => "Quit App",
            CloseBehavior::MinimizeDock => "Minimize",
            CloseBehavior::AskFirst => "Ask First",
        }
    }
}

/// Hotkey profile presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeybindProfile {
    HyprlandKde,
    HyprlandPure,
    KdeClassic,
}
impl KeybindProfile {
    pub const fn label(self) -> &'static str {
        match self {
            KeybindProfile::HyprlandKde => "Hyprland + KDE",
            KeybindProfile::HyprlandPure => "Hyprland",
            KeybindProfile::KdeClassic => "KDE Classic",
        }
    }
}

/// One-click desktop look presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfilePreset {
    pub name: &'static str,
    pub subtitle: &'static str,
    pub theme: i32,
    pub accent: i32,
    pub reduce_transparency: i32,
    pub animations: i32,
    pub corner_radius_idx: usize,
    pub tab_bar: TabBarStyle,
    pub tiling: TilingStyle,
    pub keybinds: KeybindProfile,
}

pub const PROFILES: [ProfilePreset; 4] = [
    ProfilePreset {
        name: "SexOS Noir Rose",
        subtitle: "Obsidian dark canvas, SexOS Rose tint, frosted glass",
        theme: 1,
        accent: 0x00F4_3F5E,
        reduce_transparency: 0,
        animations: 1,
        corner_radius_idx: 2,
        tab_bar: TabBarStyle::SilkPill,
        tiling: TilingStyle::HybridCandy,
        keybinds: KeybindProfile::HyprlandKde,
    },
    ProfilePreset {
        name: "Noir Lavender",
        subtitle: "Deep slate glass, Noir Lavender tint, SilkBar pills",
        theme: 1,
        accent: 0x00A7_8BFA,
        reduce_transparency: 0,
        animations: 1,
        corner_radius_idx: 2,
        tab_bar: TabBarStyle::SilkPill,
        tiling: TilingStyle::HybridCandy,
        keybinds: KeybindProfile::HyprlandKde,
    },
    ProfilePreset {
        name: "Ice Cyan Studio",
        subtitle: "Crisp dark tiling, Ice Cyan tint, compact 4 px corners",
        theme: 1,
        accent: 0x0038_BDF8,
        reduce_transparency: 0,
        animations: 1,
        corner_radius_idx: 0,
        tab_bar: TabBarStyle::SilkPill,
        tiling: TilingStyle::HyprlandMaster,
        keybinds: KeybindProfile::HyprlandPure,
    },
    ProfilePreset {
        name: "Graphite Minimal",
        subtitle: "Monochrome Graphite tint, opaque surfaces, floating",
        theme: 1,
        accent: 0x0064_748B,
        reduce_transparency: 1,
        animations: 1,
        corner_radius_idx: 1,
        tab_bar: TabBarStyle::Classic,
        tiling: TilingStyle::KdeFloat,
        keybinds: KeybindProfile::KdeClassic,
    },
];

pub const HOTKEY_ROWS: [(&str, &str, &str); 8] = [
    ("Super + Enter", "Launch Spindle terminal", "Hyprland"),
    ("Super + Q / Alt + F4", "Close focused window", "Hyprland + KDE"),
    ("Super + 1..9", "Switch workspace / space", "Hyprland + KDE"),
    ("Alt + Tab / Super + J/K", "Cycle window focus", "KDE + Hyprland"),
    ("Super + F / F11", "Toggle fullscreen / zoom", "Hyprland + KDE"),
    ("Super + V", "Toggle floating / tiled mode", "Hyprland"),
    ("Super + Esc / Ctrl+Alt+L", "Lock session immediately", "KDE + Hyprland"),
    ("Super + D / Space", "Open SilkBar Spotlight", "Noir Desktop"),
];

pub const MAX_INSTALL_PARTS: usize = 16;
pub const MIN_INSTALL_SECTORS: u64 = 1536 * 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskPartKind {
    Esp,
    Msr,
    Windows,
    Linux,
    SexOs,
    FreeSpace,
    Other,
}

impl DiskPartKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Esp => "ESP",
            Self::Msr => "MSR",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::SexOs => "SexOS",
            Self::FreeSpace => "Free Space",
            Self::Other => "Other",
        }
    }

    pub const fn color(self) -> u32 {
        match self {
            Self::Esp => 0x00F5_9E0B,
            Self::Msr => 0x0064_748B,
            Self::Windows => 0x0038_BDF8,
            Self::Linux => 0x0010_B981,
            Self::SexOs => 0x00F4_3F5E,
            Self::FreeSpace => 0x0047_5569,
            Self::Other => 0x00A7_8BFA,
        }
    }

    pub fn from_code(code: &[u8]) -> Self {
        match code {
            b"esp" => Self::Esp,
            b"msr" => Self::Msr,
            b"win" => Self::Windows,
            b"lin" => Self::Linux,
            b"sex" => Self::SexOs,
            b"free" => Self::FreeSpace,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskPartRow {
    pub part_num: u16,
    pub kind: DiskPartKind,
    pub first_lba: u64,
    pub sectors: u64,
    pub replaceable: bool,
    pub name: [u8; 32],
    pub name_len: usize,
}

impl DiskPartRow {
    pub const EMPTY: Self = Self {
        part_num: 0,
        kind: DiskPartKind::Other,
        first_lba: 0,
        sectors: 0,
        replaceable: false,
        name: [0; 32],
        name_len: 0,
    };

    pub fn new(part_num: u16, kind: DiskPartKind, first_lba: u64, sectors: u64, replaceable: bool, label: &[u8]) -> Self {
        let mut row = Self {
            part_num,
            kind,
            first_lba,
            sectors,
            replaceable: replaceable
                && !matches!(kind, DiskPartKind::Esp | DiskPartKind::Msr)
                && sectors >= MIN_INSTALL_SECTORS,
            name: [0; 32],
            name_len: 0,
        };
        let n = label.len().min(row.name.len());
        row.name[..n].copy_from_slice(&label[..n]);
        row.name_len = n;
        row
    }

    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len]
    }
}

pub struct InstallState {
    pub total_sectors: u64,
    pub parts: [DiskPartRow; MAX_INSTALL_PARTS],
    pub part_count: usize,
    pub selected: Option<usize>,
    pub loaded: bool,
    pub plan_ready: bool,
    pub report: [u8; 512],
    pub report_len: usize,
}

impl InstallState {
    pub const fn new() -> Self {
        Self {
            total_sectors: 0,
            parts: [DiskPartRow::EMPTY; MAX_INSTALL_PARTS],
            part_count: 0,
            selected: None,
            loaded: false,
            plan_ready: false,
            report: [0; 512],
            report_len: 0,
        }
    }

    pub fn set_report(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.report.len());
        self.report[..n].copy_from_slice(&bytes[..n]);
        self.report_len = n;
    }

    pub fn report_bytes(&self) -> &[u8] {
        &self.report[..self.report_len]
    }

    pub fn select_part(&mut self, idx: usize) -> bool {
        if idx < self.part_count && self.parts[idx].replaceable {
            self.selected = Some(idx);
            self.plan_ready = false;
            true
        } else {
            false
        }
    }

    pub fn selected_row(&self) -> Option<&DiskPartRow> {
        self.selected.and_then(|i| (i < self.part_count).then_some(&self.parts[i]))
    }

    pub fn parse_raw(&mut self, data: &[u8]) {
        self.total_sectors = 0;
        self.part_count = 0;
        self.selected = None;
        self.plan_ready = false;
        self.loaded = true;
        for line in data.split(|&b| b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix(b"DISK ") {
                self.total_sectors = parse_u64_ascii(rest);
            } else if let Some(rest) = line.strip_prefix(b"PART ") {
                if self.part_count >= MAX_INSTALL_PARTS {
                    continue;
                }
                let (num_b, r) = split_ws(rest);
                let (kind_b, r) = split_ws(r);
                let (first_b, r) = split_ws(r);
                let (sec_b, r) = split_ws(r);
                let (rep_b, name_b) = split_ws(r);
                let num = parse_u64_ascii(num_b) as u16;
                let kind = DiskPartKind::from_code(kind_b);
                let first = parse_u64_ascii(first_b);
                let sectors = parse_u64_ascii(sec_b);
                let replaceable = rep_b == b"1";
                let label = if name_b.is_empty() { kind.label().as_bytes() } else { name_b };
                self.parts[self.part_count] = DiskPartRow::new(num, kind, first, sectors, replaceable, label);
                self.part_count += 1;
            } else if let Some(rest) = line.strip_prefix(b"FREE ") {
                if self.part_count >= MAX_INSTALL_PARTS {
                    continue;
                }
                let (first_b, sec_b) = split_ws(rest);
                let first = parse_u64_ascii(first_b);
                let sectors = parse_u64_ascii(sec_b);
                self.parts[self.part_count] =
                    DiskPartRow::new(0, DiskPartKind::FreeSpace, first, sectors, true, b"Unallocated Free Space");
                self.part_count += 1;
            } else if line.starts_with(b"ERR ") {
                self.set_report(b"No GPT partition table found. Use Spindle `install` for blank disks.");
            }
        }
        for i in 0..self.part_count {
            if self.parts[i].replaceable {
                self.selected = Some(i);
                break;
            }
        }
    }

    pub fn safety_summary(&self, out: &mut [u8; 160]) -> (usize, bool) {
        let mut n = 0usize;
        let mut push = |s: &[u8]| {
            let take = s.len().min(out.len().saturating_sub(n));
            out[n..n + take].copy_from_slice(&s[..take]);
            n += take;
        };
        let Some(row) = self.selected_row() else {
            push(b"Select a replaceable partition (>= 1.5 GiB) or Free Space. ESP & tiny partitions are locked.");
            return (n, false);
        };
        let mut sz = [0u8; 16];
        let sz_len = format_size_short(row.sectors, &mut sz);
        if row.kind == DiskPartKind::FreeSpace {
            push(b"Will install SexOS A/B + Store into Free Space (");
            push(&sz[..sz_len]);
            push(b"). All existing partitions remain untouched.");
            (n, false)
        } else {
            let mut num_buf = [0u8; 8];
            let num_len = format_u64_ascii(row.part_num as u64, &mut num_buf);
            push(b"Will replace ONLY Partition #");
            push(&num_buf[..num_len]);
            push(b" (");
            push(row.name_bytes());
            push(b", ");
            push(&sz[..sz_len]);
            push(b") with SexOS A/B + Store. All other partitions remain untouched.");
            (n, true)
        }
    }
}

fn split_ws(mut s: &[u8]) -> (&[u8], &[u8]) {
    while let Some(&b) = s.first() {
        if b == b' ' || b == b'\t' { s = &s[1..]; } else { break; }
    }
    let mut i = 0;
    while i < s.len() && s[i] != b' ' && s[i] != b'\t' {
        i += 1;
    }
    let head = &s[..i];
    let mut tail = &s[i..];
    while let Some(&b) = tail.first() {
        if b == b' ' || b == b'\t' { tail = &tail[1..]; } else { break; }
    }
    (head, tail)
}

fn parse_u64_ascii(s: &[u8]) -> u64 {
    let mut v = 0u64;
    for &b in s {
        if b.is_ascii_digit() { v = v.saturating_mul(10).saturating_add((b - b'0') as u64); } else { break; }
    }
    v
}

fn format_u64_ascii(mut v: u64, out: &mut [u8]) -> usize {
    let mut tmp = [0u8; 20];
    let mut k = 0;
    loop {
        tmp[k] = b'0' + (v % 10) as u8;
        k += 1;
        v /= 10;
        if v == 0 { break; }
    }
    let mut n = 0;
    while k > 0 && n < out.len() {
        k -= 1;
        out[n] = tmp[k];
        n += 1;
    }
    n
}

fn format_size_short(sectors: u64, out: &mut [u8; 16]) -> usize {
    let mut n = 0usize;
    if sectors < 2048 * 1024 {
        let mib = sectors / 2048;
        n += format_u64_ascii(mib, &mut out[n..]);
        let suf = b" MiB";
        let take = suf.len().min(out.len() - n);
        out[n..n + take].copy_from_slice(&suf[..take]);
        n += take;
    } else {
        let gib = sectors / (2048 * 1024);
        let tenths = sectors % (2048 * 1024) * 10 / (2048 * 1024);
        n += format_u64_ascii(gib, &mut out[n..]);
        if n < out.len() { out[n] = b'.'; n += 1; }
        if n < out.len() { out[n] = b'0' + tenths as u8; n += 1; }
        let suf = b" GiB";
        let take = suf.len().min(out.len() - n);
        out[n..n + take].copy_from_slice(&suf[..take]);
        n += take;
    }
    n
}

/// Top-level macOS Noir System Settings sidebar categories + fallback schema view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarView {
    Appearance,
    Network,
    Wifi,
    Sound,
    Display,
    Trackpad,
    Apps,
    Keyboard,
    Privacy,
    Install,
    Updates,
    About,
    /// Schema page navigated directly via `PgUp`/`PgDn`/`Tab`.
    SystemPage(usize),
}

/// Maximum saved Wi-Fi networks shared with Spindle (`/data/wifi.conf`).
pub const MAX_WIFI_KNOWN: usize = 16;
/// Maximum visible scanned Wi-Fi networks tracked in Tatami Settings.
pub const MAX_WIFI_SCAN: usize = 12;
/// Maximum serialized byte size for `/data/wifi.conf`.
pub const WIFI_CONF_MAX: usize = 2048;
/// Shared Wi-Fi configuration file path.
pub const WIFI_CONF_PATH: &[u8] = b"/data/wifi.conf";

/// Active text field focus in the Wi-Fi Quick Join form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiFieldFocus {
    None,
    Ssid,
    Password,
}

/// One saved Wi-Fi network entry from `/data/wifi.conf`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WifiKnownEntry {
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub pass: [u8; 63],
    pub pass_len: usize,
}

impl WifiKnownEntry {
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
        let mut e = Self::EMPTY;
        e.ssid[..ssid.len()].copy_from_slice(ssid);
        e.ssid_len = ssid.len();
        e.pass[..pass.len()].copy_from_slice(pass);
        e.pass_len = pass.len();
        Some(e)
    }

    pub fn ssid_bytes(&self) -> &[u8] {
        &self.ssid[..self.ssid_len]
    }

    pub fn pass_bytes(&self) -> &[u8] {
        &self.pass[..self.pass_len]
    }
}

/// One visible scanned Wi-Fi network from `sexnet`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WifiScanEntry {
    pub scan_idx: u16,
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub rssi: i8,
    pub channel: u8,
    pub security: u8,
}

impl WifiScanEntry {
    pub const EMPTY: Self = Self {
        scan_idx: 0,
        ssid: [0; 32],
        ssid_len: 0,
        rssi: 0,
        channel: 0,
        security: 0,
    };

    pub fn ssid_bytes(&self) -> &[u8] {
        &self.ssid[..self.ssid_len]
    }
}

/// Zero-heap state for the Candy Lavender Wi-Fi & Network Manager page.
pub struct WifiState {
    pub loaded: bool,
    pub connected: bool,
    pub status_label: [u8; 64],
    pub status_label_len: usize,
    pub active_ssid: [u8; 32],
    pub active_ssid_len: usize,
    pub ip_text: [u8; 32],
    pub ip_text_len: usize,
    pub signal_rssi: i8,
    pub known: [WifiKnownEntry; MAX_WIFI_KNOWN],
    pub known_count: usize,
    pub scanned: [WifiScanEntry; MAX_WIFI_SCAN],
    pub scan_count: usize,
    pub focus: WifiFieldFocus,
    pub form_ssid: [u8; 32],
    pub form_ssid_len: usize,
    pub form_pass: [u8; 63],
    pub form_pass_len: usize,
}

impl WifiState {
    pub const fn new() -> Self {
        Self {
            loaded: false,
            connected: false,
            status_label: [0; 64],
            status_label_len: 0,
            active_ssid: [0; 32],
            active_ssid_len: 0,
            ip_text: [0; 32],
            ip_text_len: 0,
            signal_rssi: 0,
            known: [WifiKnownEntry::EMPTY; MAX_WIFI_KNOWN],
            known_count: 0,
            scanned: [WifiScanEntry::EMPTY; MAX_WIFI_SCAN],
            scan_count: 0,
            focus: WifiFieldFocus::None,
            form_ssid: [0; 32],
            form_ssid_len: 0,
            form_pass: [0; 63],
            form_pass_len: 0,
        }
    }

    pub fn set_status(&mut self, msg: &[u8]) {
        let n = msg.len().min(self.status_label.len());
        self.status_label[..n].copy_from_slice(&msg[..n]);
        self.status_label_len = n;
    }

    pub fn status_bytes(&self) -> &[u8] {
        if self.status_label_len > 0 {
            &self.status_label[..self.status_label_len]
        } else if self.connected {
            b"Connected"
        } else {
            b"Disconnected"
        }
    }

    pub fn set_active_ssid(&mut self, ssid: &[u8]) {
        let n = ssid.len().min(self.active_ssid.len());
        self.active_ssid[..n].copy_from_slice(&ssid[..n]);
        self.active_ssid_len = n;
    }

    pub fn set_ip(&mut self, ip: &[u8]) {
        let n = ip.len().min(self.ip_text.len());
        self.ip_text[..n].copy_from_slice(&ip[..n]);
        self.ip_text_len = n;
    }

    pub fn ip_bytes(&self) -> &[u8] {
        if self.ip_text_len > 0 {
            &self.ip_text[..self.ip_text_len]
        } else {
            b"--"
        }
    }

    pub fn signal_quality_label(rssi: i8) -> &'static [u8] {
        if rssi == 0 {
            b"--"
        } else if rssi >= -55 {
            b"Excellent"
        } else if rssi >= -67 {
            b"Good"
        } else if rssi >= -78 {
            b"Fair"
        } else {
            b"Weak"
        }
    }

    pub fn find_known(&self, ssid: &[u8]) -> Option<usize> {
        self.known[..self.known_count]
            .iter()
            .position(|e| e.ssid_bytes() == ssid)
    }

    /// Parses `/data/wifi.conf` in either multi-network (`ssid=<SSID>\tpass=<PASS>\n`)
    /// or legacy 2-line (`<SSID>\n<PASS>\n` / `<SSID>\n`) format.
    pub fn parse_wifi_conf(&mut self, data: &[u8]) {
        self.known_count = 0;
        self.loaded = true;
        fn trim_cr(s: &[u8]) -> &[u8] {
            if let [rest @ .., b'\r'] = s { rest } else { s }
        }
        let has_kv = data.split(|&b| b == b'\n').map(trim_cr).any(|l| l.starts_with(b"ssid="));
        if has_kv {
            for raw in data.split(|&b| b == b'\n') {
                let line = trim_cr(raw);
                if line.is_empty() || line[0] == b'#' {
                    continue;
                }
                let Some(rest) = line.strip_prefix(b"ssid=") else { continue };
                let (ssid, pass) = if let Some(pos) = rest.windows(6).position(|w| w == b"\tpass=") {
                    (&rest[..pos], &rest[pos + 6..])
                } else if let Some(pos) = rest.iter().position(|&b| b == b'\t') {
                    let tail = &rest[pos + 1..];
                    (&rest[..pos], tail.strip_prefix(b"pass=").unwrap_or(tail))
                } else {
                    (rest, &b""[..])
                };
                if self.find_known(ssid).is_some() || self.known_count >= MAX_WIFI_KNOWN {
                    continue;
                }
                if let Some(entry) = WifiKnownEntry::from_slices(ssid, pass) {
                    self.known[self.known_count] = entry;
                    self.known_count += 1;
                }
            }
            return;
        }
        // Legacy 2-line fallback: `<ssid>\n<pass>\n` or `<ssid>\n`.
        let mut lines = data.split(|&b| b == b'\n').map(trim_cr);
        let Some(line1) = lines.next() else { return };
        if line1.is_empty() || line1.len() > 32 {
            return;
        }
        let line2 = lines.next().unwrap_or(b"");
        if line2.len() <= 63 {
            if let Some(entry) = WifiKnownEntry::from_slices(line1, line2) {
                self.known[0] = entry;
                self.known_count = 1;
            }
        }
    }

    /// Serializes known networks as `ssid=<SSID>\tpass=<PASS>\n` (MRU first).
    pub fn serialize_wifi_conf(&self, out: &mut [u8]) -> usize {
        let mut at = 0;
        let put = |out: &mut [u8], at: &mut usize, bytes: &[u8]| {
            let take = bytes.len().min(out.len().saturating_sub(*at));
            out[*at..*at + take].copy_from_slice(&bytes[..take]);
            *at += take;
        };
        for entry in &self.known[..self.known_count] {
            put(out, &mut at, b"ssid=");
            put(out, &mut at, entry.ssid_bytes());
            put(out, &mut at, b"\tpass=");
            put(out, &mut at, entry.pass_bytes());
            put(out, &mut at, b"\n");
        }
        at
    }

    /// Upserts `(ssid, pass)` to the top (MRU index 0) of the known networks list.
    pub fn upsert_known(&mut self, ssid: &[u8], pass: &[u8]) -> bool {
        let Some(entry) = WifiKnownEntry::from_slices(ssid, pass) else {
            return false;
        };
        if let Some(idx) = self.find_known(ssid) {
            for i in (1..=idx).rev() {
                self.known[i] = self.known[i - 1];
            }
            self.known[0] = entry;
            return true;
        }
        let shift_end = self.known_count.min(MAX_WIFI_KNOWN - 1);
        for i in (1..=shift_end).rev() {
            self.known[i] = self.known[i - 1];
        }
        self.known[0] = entry;
        if self.known_count < MAX_WIFI_KNOWN {
            self.known_count += 1;
        }
        true
    }

    /// Removes known network at `idx`.
    pub fn forget_known(&mut self, idx: usize) -> bool {
        if idx >= self.known_count {
            return false;
        }
        for i in idx + 1..self.known_count {
            self.known[i - 1] = self.known[i];
        }
        self.known_count -= 1;
        self.known[self.known_count] = WifiKnownEntry::EMPTY;
        true
    }

    /// Populates the quick-join form from scanned network `idx`.
    pub fn select_scanned(&mut self, idx: usize) -> bool {
        if idx >= self.scan_count {
            return false;
        }
        let s = self.scanned[idx];
        self.form_ssid = s.ssid;
        self.form_ssid_len = s.ssid_len;
        self.form_pass = [0; 63];
        self.form_pass_len = 0;
        self.focus = if s.security == 0 {
            WifiFieldFocus::Ssid
        } else {
            WifiFieldFocus::Password
        };
        true
    }

    /// Appends a printable ASCII character to the focused quick-join input field.
    pub fn type_char(&mut self, c: u8) -> bool {
        if !(0x20..0x7F).contains(&c) {
            return false;
        }
        match self.focus {
            WifiFieldFocus::Ssid => {
                if self.form_ssid_len < self.form_ssid.len() {
                    self.form_ssid[self.form_ssid_len] = c;
                    self.form_ssid_len += 1;
                    true
                } else {
                    false
                }
            }
            WifiFieldFocus::Password => {
                if self.form_pass_len < self.form_pass.len() {
                    self.form_pass[self.form_pass_len] = c;
                    self.form_pass_len += 1;
                    true
                } else {
                    false
                }
            }
            WifiFieldFocus::None => false,
        }
    }

    /// Deletes the last byte from the focused quick-join input field.
    pub fn backspace(&mut self) -> bool {
        match self.focus {
            WifiFieldFocus::Ssid if self.form_ssid_len > 0 => {
                self.form_ssid_len -= 1;
                self.form_ssid[self.form_ssid_len] = 0;
                true
            }
            WifiFieldFocus::Password if self.form_pass_len > 0 => {
                self.form_pass_len -= 1;
                self.form_pass[self.form_pass_len] = 0;
                true
            }
            _ => false,
        }
    }

    pub fn form_ssid_bytes(&self) -> &[u8] {
        &self.form_ssid[..self.form_ssid_len]
    }

    pub fn form_pass_bytes(&self) -> &[u8] {
        &self.form_pass[..self.form_pass_len]
    }
}

pub struct SidebarItem {
    pub label: &'static str,
    pub view: SidebarView,
    pub icon: Icon,
    pub badge_color: u32,
    pub schema_section: &'static str,
}

pub const SIDEBAR_ITEMS: [SidebarItem; 11] = [
    SidebarItem { label: "Appearance & Noir", view: SidebarView::Appearance, icon: Icon::Appearance, badge_color: 0x00F4_3F5E, schema_section: "appearance" },
    SidebarItem { label: "Wi-Fi & Network", view: SidebarView::Network, icon: Icon::Network, badge_color: 0x003B_82F6, schema_section: "privacy" },
    SidebarItem { label: "Sound", view: SidebarView::Sound, icon: Icon::Sound, badge_color: 0x00EF_4444, schema_section: "sound" },
    SidebarItem { label: "Display", view: SidebarView::Display, icon: Icon::Display, badge_color: 0x000E_A5E9, schema_section: "display" },
    SidebarItem { label: "Trackpad & Multitouch", view: SidebarView::Trackpad, icon: Icon::Trackpad, badge_color: 0x0064_748B, schema_section: "desktop" },
    SidebarItem { label: "Apps & Close Behavior", view: SidebarView::Apps, icon: Icon::Apps, badge_color: 0x00A8_55F7, schema_section: "open" },
    SidebarItem { label: "Keyboard & Prediction", view: SidebarView::Keyboard, icon: Icon::Keyboard, badge_color: 0x0063_66F1, schema_section: "keyboard" },
    SidebarItem { label: "Privacy & Grants", view: SidebarView::Privacy, icon: Icon::Privacy, badge_color: 0x0010_B981, schema_section: "privacy" },
    SidebarItem { label: "Storage & Disk Installer", view: SidebarView::Install, icon: Icon::Storage, badge_color: 0x00F5_9E0B, schema_section: "storage" },
    SidebarItem { label: "Software Update", view: SidebarView::Updates, icon: Icon::Update, badge_color: 0x0038_BDF8, schema_section: "updates" },
    SidebarItem { label: "About This SexOS", view: SidebarView::About, icon: Icon::About, badge_color: 0x00F4_3F5E, schema_section: "setup" },
];

impl SidebarView {
    pub fn from_section(sec: &str, idx: usize) -> Self {
        match sec {
            "appearance" => SidebarView::Appearance,
            "sound" => SidebarView::Sound,
            "display" => SidebarView::Display,
            "keyboard" => SidebarView::Keyboard,
            "privacy" => SidebarView::Privacy,
            "storage" => SidebarView::Install,
            "updates" => SidebarView::Updates,
            "setup" => SidebarView::About,
            _ => SidebarView::SystemPage(idx),
        }
    }

    pub fn active_sidebar_idx(self) -> usize {
        let (pages, n) = model::pages();
        match self {
            SidebarView::Appearance => 0,
            SidebarView::Network | SidebarView::Wifi => 1,
            SidebarView::Sound => 2,
            SidebarView::Display => 3,
            SidebarView::Trackpad => 4,
            SidebarView::Apps => 5,
            SidebarView::Keyboard => 6,
            SidebarView::Privacy => 7,
            SidebarView::Install => 8,
            SidebarView::Updates => 9,
            SidebarView::About => 10,
            SidebarView::SystemPage(idx) if idx < n => match pages[idx] {
                "appearance" => 0,
                "sound" => 2,
                "desktop" | "display" | "accessibility" => 3,
                "open" | "startup" | "notifications" => 5,
                "keyboard" | "time" | "shortcuts" => 6,
                "privacy" | "session" => 7,
                "storage" => 8,
                "updates" => 9,
                "setup" => 10,
                _ => 0,
            },
            SidebarView::SystemPage(_) => 0,
        }
    }
}

/// Snapshot of editable desktop preferences for one-click Undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefSnapshot {
    pub theme: i32,
    pub accent: i32,
    pub reduce_transparency: i32,
    pub animations: i32,
    pub corner_radius_idx: usize,
    pub tab_bar: TabBarStyle,
    pub tiling: TilingStyle,
    pub keybinds: KeybindProfile,
}

pub const SYSTEM_REPO: &str = "xirtus/sexos";
pub const DOTFILES_REPO: &str = "xirtus/sexosdotfiles";

pub struct UpdatesState {
    pub system_checked: bool,
    pub system_status: [u8; 96],
    pub system_status_len: usize,
    pub dotfiles_synced: bool,
    pub dotfiles_auto_sync: bool,
    pub dotfiles_status: [u8; 96],
    pub dotfiles_status_len: usize,
}

impl UpdatesState {
    pub const fn new() -> Self {
        Self {
            system_checked: false,
            system_status: [0; 96],
            system_status_len: 0,
            dotfiles_synced: false,
            dotfiles_auto_sync: true,
            dotfiles_status: [0; 96],
            dotfiles_status_len: 0,
        }
    }

    pub fn set_system_status(&mut self, msg: &[u8]) {
        let n = msg.len().min(self.system_status.len());
        self.system_status[..n].copy_from_slice(&msg[..n]);
        self.system_status_len = n;
    }

    pub fn system_status_bytes(&self) -> &[u8] {
        if self.system_status_len > 0 {
            &self.system_status[..self.system_status_len]
        } else {
            b"Ready - click Check Update or run 'update' in Spindle."
        }
    }

    pub fn set_dotfiles_status(&mut self, msg: &[u8]) {
        let n = msg.len().min(self.dotfiles_status.len());
        self.dotfiles_status[..n].copy_from_slice(&msg[..n]);
        self.dotfiles_status_len = n;
    }

    pub fn dotfiles_status_bytes(&self) -> &[u8] {
        if self.dotfiles_status_len > 0 {
            &self.dotfiles_status[..self.dotfiles_status_len]
        } else {
            b"Repo: xirtus/sexosdotfiles (private) - click Sync or Pull."
        }
    }
}

/// Live structured facts for About This SexOS and Storage cards.
#[derive(Clone, Copy)]
pub struct AboutState {
    pub kernel: KernelFacts,
    pub slot: [u8; 8],
    pub slot_len: usize,
    pub version: [u8; 16],
    pub version_len: usize,
    pub image_mib: Option<u64>,
    pub image_hash: [u8; 72],
    pub image_hash_len: usize,
    pub fs_health: [u8; 64],
    pub fs_health_len: usize,
    pub disk_summary: [u8; 64],
    pub disk_summary_len: usize,
}

impl AboutState {
    pub const fn new() -> Self {
        Self {
            kernel: KernelFacts {
                cores: None,
                program_limit: None,
                program_lent: None,
                free_frames: None,
                kernel_heap: None,
                domains: 0,
                uptime_s: 0,
            },
            slot: [0; 8],
            slot_len: 0,
            version: [0; 16],
            version_len: 0,
            image_mib: None,
            image_hash: [0; 72],
            image_hash_len: 0,
            fs_health: [0; 64],
            fs_health_len: 0,
            disk_summary: [0; 64],
            disk_summary_len: 0,
        }
    }

    pub fn update_about(&mut self, k: KernelFacts, facts_raw: &[u8]) {
        self.kernel = k;
        if let Some(s) = facts::field(facts_raw, "slot") {
            let n = s.len().min(self.slot.len());
            self.slot[..n].copy_from_slice(&s[..n]);
            self.slot_len = n;
        }
        if let Some(v) = facts::field(facts_raw, "version") {
            let n = v.len().min(self.version.len());
            self.version[..n].copy_from_slice(&v[..n]);
            self.version_len = n;
        }
        self.image_mib = facts::field(facts_raw, "image_bytes")
            .and_then(|v| core::str::from_utf8(v).ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|b| b >> 20);
        if let Some(h) = facts::field(facts_raw, "image") {
            let n = h.len().min(self.image_hash.len());
            self.image_hash[..n].copy_from_slice(&h[..n]);
            self.image_hash_len = n;
        }
    }

    pub fn update_storage(&mut self, facts_raw: &[u8]) {
        let mut out = [0u8; 64];
        let mut at = 0usize;
        let mut push = |b: &[u8]| {
            let take = b.len().min(out.len().saturating_sub(at));
            out[at..at + take].copy_from_slice(&b[..take]);
            at += take;
        };
        if let Some(fs_v) = facts::field(facts_raw, "filesystem") {
            let mut it = fs_v.split(|&b| b == b' ');
            let _ = it.next();
            let total = it.next().and_then(|v| core::str::from_utf8(v).ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let free = it.next().and_then(|v| core::str::from_utf8(v).ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let mut tmp = [0u8; 16];
            push(b"sexfs Healthy (");
            let l1 = format_size_short(free * 8, &mut tmp);
            push(&tmp[..l1]);
            push(b" free / ");
            let l2 = format_size_short(total * 8, &mut tmp);
            push(&tmp[..l2]);
            push(b")");
        } else {
            push(b"Live Read-Only / No SexFS Mounted");
        }
        self.fs_health[..at].copy_from_slice(&out[..at]);
        self.fs_health_len = at;

        let mut dout = [0u8; 64];
        let mut dat = 0usize;
        let mut dpush = |b: &[u8]| {
            let take = b.len().min(dout.len().saturating_sub(dat));
            dout[dat..dat + take].copy_from_slice(&b[..take]);
            dat += take;
        };
        if let Some(disk_v) = facts::field(facts_raw, "disk") {
            let mut it = disk_v.split(|&b| b == b' ');
            let sectors = it.next().and_then(|v| core::str::from_utf8(v).ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            let kind = it.next().unwrap_or(b"unknown");
            let mut tmp = [0u8; 16];
            let l = format_size_short(sectors, &mut tmp);
            dpush(&tmp[..l]);
            dpush(b" (");
            dpush(kind);
            dpush(b")");
        }
        self.disk_summary[..dat].copy_from_slice(&dout[..dat]);
        self.disk_summary_len = dat;
    }

    pub fn slot_badge(&self) -> &[u8] {
        match &self.slot[..self.slot_len] {
            b"a" => b"Slot A",
            b"b" => b"Slot B",
            b"none" => b"Live USB",
            _ => b"SexOS",
        }
    }
}

/// Interactive hitbox action emitted on click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    SelectView(SidebarView),
    FocusSearch,
    ClearSearch,
    SetTheme(i32),
    SetAccent(i32),
    ToggleFrostedGlass,
    ToggleAnimations,
    ToggleBoolSetting(u16),
    SetSettingValue(u16, i32),
    StepSetting(u16, bool),
    ToggleSetBit(u16, u8),
    SetCornerRadius(usize),
    SetTabBar(TabBarStyle),
    SetTiling(TilingStyle),
    SetCloseBehavior(CloseBehavior),
    SetKeybinds(KeybindProfile),
    ToggleTrackpad(u8),
    TogglePrediction(u8),
    ToggleWifiRadio,
    ApplyProfile(usize),
    Undo,
    ResetDefaults,
    SelectRow(usize),
    StepRow(usize, bool),
    SetGrant(usize, bool),
    StartPassword,
    CancelPassword,
    SelectInstallPart(usize),
    InstallRefresh,
    InstallPlan,
    InstallExecute,
    CheckSystemUpdate,
    RefreshUpdates,
    SyncDotfiles,
    PullDotfiles,
    ToggleDotfilesAutoSync,
    WifiScan,
    WifiRefresh,
    WifiConnectKnown(usize),
    WifiForgetKnown(usize),
    WifiSelectScan(usize),
    WifiFocusSsid,
    WifiFocusPass,
    WifiJoinSubmit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub action: UiAction,
}

impl HitRect {
    pub const fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// Settings GUI state (static / stack; zero heap).
pub struct GuiState {
    pub view: SidebarView,
    pub search_focused: bool,
    pub search_selected: bool,
    pub scroll_y: i32,
    pub corner_radius_idx: usize,
    pub tab_bar: TabBarStyle,
    pub tiling: TilingStyle,
    pub close_behavior: CloseBehavior,
    pub keybinds: KeybindProfile,
    pub trackpad_tap_click: bool,
    pub trackpad_natural_scroll: bool,
    pub trackpad_three_finger_spaces: bool,
    pub trackpad_four_finger_overview: bool,
    pub text_prediction: bool,
    pub text_autocorrect: bool,
    pub wifi_enabled: bool,
    pub active_profile: Option<usize>,
    pub undo: Option<PrefSnapshot>,
    pub hovered: Option<UiAction>,
    pub pressed: Option<UiAction>,
    pub hits: [Option<HitRect>; MAX_HITS],
    pub hit_count: usize,
    pub install: InstallState,
    pub updates: UpdatesState,
    pub wifi: WifiState,
    pub about: AboutState,
    pub grant_rows: [grants::Row; grants::ROWS_MAX],
    pub grant_count: usize,
}

impl GuiState {
    pub const fn new() -> Self {
        Self {
            view: SidebarView::Appearance,
            search_focused: false,
            search_selected: false,
            scroll_y: 0,
            corner_radius_idx: 2,
            tab_bar: TabBarStyle::SilkPill,
            tiling: TilingStyle::HybridCandy,
            close_behavior: CloseBehavior::QuitApp,
            keybinds: KeybindProfile::HyprlandKde,
            trackpad_tap_click: true,
            trackpad_natural_scroll: true,
            trackpad_three_finger_spaces: true,
            trackpad_four_finger_overview: true,
            text_prediction: true,
            text_autocorrect: true,
            wifi_enabled: true,
            active_profile: Some(0),
            undo: None,
            hovered: None,
            pressed: None,
            hits: [None; MAX_HITS],
            hit_count: 0,
            install: InstallState::new(),
            updates: UpdatesState::new(),
            wifi: WifiState::new(),
            about: AboutState::new(),
            grant_rows: [grants::Row::EMPTY; grants::ROWS_MAX],
            grant_count: 0,
        }
    }

    pub fn set_grants(&mut self, rows: &[grants::Row]) {
        let n = rows.len().min(grants::ROWS_MAX);
        self.grant_rows[..n].copy_from_slice(&rows[..n]);
        self.grant_count = n;
    }

    pub fn snapshot(&self, values: &Config) -> PrefSnapshot {
        PrefSnapshot {
            theme: values.value(id::THEME),
            accent: values.value(id::ACCENT),
            reduce_transparency: values.value(id::REDUCE_TRANSPARENCY),
            animations: values.value(id::ANIMATIONS),
            corner_radius_idx: self.corner_radius_idx,
            tab_bar: self.tab_bar,
            tiling: self.tiling,
            keybinds: self.keybinds,
        }
    }

    pub fn save_undo(&mut self, values: &Config) {
        self.undo = Some(self.snapshot(values));
    }

    pub fn hit_at(&self, px: i32, py: i32) -> Option<UiAction> {
        for slot in self.hits[..self.hit_count].iter().rev() {
            if let Some(h) = slot {
                if h.contains(px, py) {
                    return Some(h.action);
                }
            }
        }
        None
    }

    fn clear_hits(&mut self) {
        self.hit_count = 0;
    }

    fn push_hit(&mut self, x: i32, y: i32, w: i32, h: i32, action: UiAction) {
        if self.hit_count < MAX_HITS && w > 0 && h > 0 {
            self.hits[self.hit_count] = Some(HitRect { x, y, w, h, action });
            self.hit_count += 1;
        }
    }

    fn btn_state(&self, action: UiAction) -> ButtonState {
        if self.pressed == Some(action) {
            ButtonState::Pressed
        } else if self.hovered == Some(action) {
            ButtonState::Hover
        } else {
            ButtonState::Rest
        }
    }

    pub fn apply_profile(&mut self, idx: usize, values: &Config) -> [(u16, i32); 4] {
        let p = PROFILES[idx.min(PROFILES.len() - 1)];
        self.save_undo(values);
        self.corner_radius_idx = p.corner_radius_idx;
        self.tab_bar = p.tab_bar;
        self.tiling = p.tiling;
        self.keybinds = p.keybinds;
        self.active_profile = Some(idx.min(PROFILES.len() - 1));
        [
            (id::THEME, p.theme),
            (id::ACCENT, p.accent),
            (id::REDUCE_TRANSPARENCY, p.reduce_transparency),
            (id::ANIMATIONS, p.animations),
        ]
    }

    pub fn take_undo(&mut self) -> Option<[(u16, i32); 4]> {
        let snap = self.undo.take()?;
        self.corner_radius_idx = snap.corner_radius_idx;
        self.tab_bar = snap.tab_bar;
        self.tiling = snap.tiling;
        self.keybinds = snap.keybinds;
        self.active_profile = None;
        Some([
            (id::THEME, snap.theme),
            (id::ACCENT, snap.accent),
            (id::REDUCE_TRANSPARENCY, snap.reduce_transparency),
            (id::ANIMATIONS, snap.animations),
        ])
    }

    pub fn reset_defaults(&mut self, values: &Config) -> [(u16, i32); 4] {
        self.apply_profile(0, values)
    }

    pub fn palette(&self, values: &Config) -> CandyPalette {
        let mode = Mode::from_setting(values.value(id::THEME));
        noir_palette(mode, values.value(id::ACCENT))
    }

    pub fn scroll_by(&mut self, delta: i32) {
        self.scroll_y = (self.scroll_y + delta).clamp(0, 900);
    }
}

/// Renders the entire macOS Ventura/Sequoia Dark-Mode Noir Tatami GUI.
pub fn render_gui(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, gui: &mut GuiState, m: &Model) {
    if w < 220 || h < 200 || buf.len() < w * h {
        return;
    }
    gui.clear_hits();
    let pal = gui.palette(&m.values);
    let (wi, hi) = (w as i32, h as i32);

    // Full Noir canvas & dark `#0B0D12` sidebar.
    fill_solid(buf, w, h, 0, 0, wi, hi, pal.bg);
    fill_solid(buf, w, h, 0, 0, SIDEBAR_W, hi, pal.panel);
    fill_solid(buf, w, h, SIDEBAR_W, 0, 1, hi, pal.specular);

    // 1. Left Sidebar: Search Pill at Top.
    let sbx = 12;
    let sby = 12;
    let sbw = SIDEBAR_W - 24;
    let sbh = 30;
    let search_border = if gui.search_focused || m.searching() { pal.accent } else { pal.specular };
    draw_noir_well(buf, w, h, sbx, sby, sbw, sbh, 8, search_border, &pal);
    noir_ui::draw_icon(buf, w, h, g, sbx + 8, sby + 8, Icon::Search, pal.text_dim);

    if m.query_len == 0 {
        draw_sans(buf, w, h, g, sbx + 27, sby + 7, sbw - 38, b"Search", pal.text_dim);
        if gui.search_focused {
            fill_solid(buf, w, h, sbx + 26, sby + 7, 1, 16, pal.accent);
        }
    } else {
        let q = &m.query[..m.query_len];
        let qw = g.measure(0, 13, q).min(sbw - 52);
        if gui.search_selected {
            fill_rounded_rect(buf, w, h, sbx + 25, sby + 5, qw + 4, 20, 4, pal.selection);
        }
        draw_sans(buf, w, h, g, sbx + 27, sby + 7, sbw - 52, q, pal.text);
        if gui.search_focused && !gui.search_selected {
            fill_solid(buf, w, h, sbx + 28 + qw, sby + 7, 1, 16, pal.accent);
        }
        // Clear `x` pill.
        let cx = sbx + sbw - 22;
        fill_rounded_rect(buf, w, h, cx, sby + 7, 16, 16, 8, pal.control);
        draw_sans_sm(buf, w, h, g, cx + 5, sby + 7, 10, b"x", pal.text_dim);
        gui.push_hit(cx - 2, sby + 4, 22, 22, UiAction::ClearSearch);
    }
    gui.push_hit(sbx, sby, if m.query_len > 0 { sbw - 24 } else { sbw }, sbh, UiAction::FocusSearch);

    // 2. SexOS Machine / Profile Card below Search.
    let prof_y = 50;
    let prof_h = 48;
    draw_noir_card(buf, w, h, sbx, prof_y, sbw, prof_h, 10, &pal);
    fill_rounded_rect(buf, w, h, sbx + 8, prof_y + 9, 30, 30, 8, 0x0026_2122);
    draw_sexos_lips(buf, w, h, sbx + 23, prof_y + 24, 22, true);
    draw_sans_bold(buf, w, h, g, sbx + 45, prof_y + 8, sbw - 54, 13, b"SexOS", pal.text);
    let sub_buf = if gui.about.slot_len > 0 {
        let mut s = [0u8; 24];
        let badge = gui.about.slot_badge();
        let n = badge.len().min(12);
        s[..n].copy_from_slice(&badge[..n]);
        let suf = b" \xE2\x80\xA2 Noir";
        let mlen = suf.len().min(s.len() - n);
        s[n..n + mlen].copy_from_slice(&suf[..mlen]);
        draw_sans_sm(buf, w, h, g, sbx + 45, prof_y + 25, sbw - 54, &s[..n + mlen], pal.text_dim);
        ()
    } else {
        draw_sans_sm(buf, w, h, g, sbx + 45, prof_y + 25, sbw - 54, b"Local Machine", pal.text_dim);
    };
    let _ = sub_buf;
    gui.push_hit(sbx, prof_y, sbw, prof_h, UiAction::SelectView(SidebarView::About));

    // 3. 11 Color-Badged Category Rows.
    let active_idx = if m.searching() { usize::MAX } else { gui.view.active_sidebar_idx() };
    let mut sy = 108;
    for (idx, item) in SIDEBAR_ITEMS.iter().enumerate() {
        if sy + 28 > hi - 8 {
            break;
        }
        let active = idx == active_idx;
        let act = UiAction::SelectView(item.view);
        let hovered = gui.hovered == Some(act);
        if active {
            fill_rounded_rect(buf, w, h, sbx, sy, sbw, 28, 7, pal.selection);
            fill_rounded_rect(buf, w, h, sbx + 2, sy + 5, 3, 18, 2, pal.accent);
        } else if hovered {
            fill_rounded_rect(buf, w, h, sbx, sy, sbw, 28, 7, pal.control);
        }
        draw_sidebar_badge(buf, w, h, g, sbx + 8, sy + 4, item.badge_color, item.icon);
        draw_sans(
            buf,
            w,
            h,
            g,
            sbx + 34,
            sy + 6,
            sbw - 40,
            item.label.as_bytes(),
            if active { pal.text } else { pal.text_dim },
        );
        gui.push_hit(sbx, sy, sbw, 28, act);
        sy += 30;
    }

    // 4. Right Content Header Bar (`HEADER_H = 48`).
    let cx = SIDEBAR_W + 22;
    let cw = (wi - SIDEBAR_W - 44).max(220);
    fill_solid(buf, w, h, SIDEBAR_W + 1, 0, wi - SIDEBAR_W - 1, HEADER_H, pal.toolbar_top);
    fill_solid(buf, w, h, SIDEBAR_W + 1, HEADER_H - 1, wi - SIDEBAR_W - 1, 1, pal.hairline);

    let title: &[u8] = if m.searching() {
        b"Search Results"
    } else {
        match gui.view {
            SidebarView::Appearance => b"Appearance & Noir",
            SidebarView::Network | SidebarView::Wifi => b"Wi-Fi & Network",
            SidebarView::Sound => b"Sound",
            SidebarView::Display => b"Display",
            SidebarView::Trackpad => b"Trackpad & Multitouch",
            SidebarView::Apps => b"Apps & SilkBar Close Behavior",
            SidebarView::Keyboard => b"Keyboard & Text Prediction",
            SidebarView::Privacy => b"Privacy & Grants",
            SidebarView::Install => b"Storage & Disk Installer",
            SidebarView::Updates => b"Software Update",
            SidebarView::About => b"About This SexOS",
            SidebarView::SystemPage(idx) => {
                let (pages, n) = model::pages();
                if idx < n { model::page_title(pages[idx]).as_bytes() } else { b"Settings" }
            }
        }
    };
    draw_sans_bold(buf, w, h, g, cx, 14, cw - 156, 16, title, pal.text);

    // Undo & Reset buttons on header right.
    let reset_w = 64;
    let undo_w = 58;
    let reset_x = wi - reset_w - 16;
    let undo_x = reset_x - undo_w - 8;
    let btn_y = 10;
    let btn_h = 26;

    let undo_style = if gui.undo.is_some() { ButtonStyle::Normal } else { ButtonStyle::Disabled };
    let undo_state = if gui.undo.is_some() { gui.btn_state(UiAction::Undo) } else { ButtonState::Rest };
    draw_candy_button(buf, w, h, undo_x, btn_y, undo_w, btn_h, 7, undo_style, undo_state, &pal);
    draw_sans_sm(buf, w, h, g, undo_x + 15, btn_y + 6, undo_w - 12, b"Undo", button_text_color(undo_style, &pal));
    if gui.undo.is_some() {
        gui.push_hit(undo_x, btn_y, undo_w, btn_h, UiAction::Undo);
    }

    let reset_state = gui.btn_state(UiAction::ResetDefaults);
    draw_candy_button(buf, w, h, reset_x, btn_y, reset_w, btn_h, 7, ButtonStyle::Normal, reset_state, &pal);
    draw_sans_sm(buf, w, h, g, reset_x + 15, btn_y + 6, reset_w - 12, b"Reset", button_text_color(ButtonStyle::Normal, &pal));
    gui.push_hit(reset_x, btn_y, reset_w, btn_h, UiAction::ResetDefaults);

    // 5. Main Scrollable Canvas Body.
    let body_top = HEADER_H + 14;
    let body_bottom = hi - STATUS_H - 8;
    let content_y = body_top - gui.scroll_y;

    let total_h = if m.searching() {
        render_search_results(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom)
    } else {
        match gui.view {
            SidebarView::Appearance => render_appearance(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Network => render_network(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Wifi => {
                render_wifi(buf, w, h, g, gui, &pal, cx, content_y, cw, (body_bottom - body_top).max(200));
                (body_bottom - body_top).max(200)
            }
            SidebarView::Sound => render_sound(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Display => render_display(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Trackpad => render_trackpad(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Apps => render_apps(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Keyboard => render_keyboard(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Privacy => render_privacy(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Install => render_storage_install(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::Updates => render_updates(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::About => render_about(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
            SidebarView::SystemPage(_) => render_system_page(buf, w, h, g, gui, m, &pal, cx, content_y, cw, body_top, body_bottom),
        }
    };
    // Clamp scroll_y cleanly if window resized or switched to shorter page.
    let view_h = (body_bottom - body_top).max(60);
    let max_scroll = (total_h - view_h).max(0);
    if gui.scroll_y > max_scroll {
        gui.scroll_y = max_scroll;
    }

    // Re-mask header & status bar over any scrolled content so text never bleeds into header/footer.
    fill_solid(buf, w, h, SIDEBAR_W + 1, 0, wi - SIDEBAR_W - 1, HEADER_H, pal.toolbar_top);
    fill_solid(buf, w, h, SIDEBAR_W + 1, HEADER_H - 1, wi - SIDEBAR_W - 1, 1, pal.hairline);
    draw_sans_bold(buf, w, h, g, cx, 14, cw - 156, 16, title, pal.text);
    draw_candy_button(buf, w, h, undo_x, btn_y, undo_w, btn_h, 7, undo_style, undo_state, &pal);
    draw_sans_sm(buf, w, h, g, undo_x + 15, btn_y + 6, undo_w - 12, b"Undo", button_text_color(undo_style, &pal));
    draw_candy_button(buf, w, h, reset_x, btn_y, reset_w, btn_h, 7, ButtonStyle::Normal, reset_state, &pal);
    draw_sans_sm(buf, w, h, g, reset_x + 15, btn_y + 6, reset_w - 12, b"Reset", button_text_color(ButtonStyle::Normal, &pal));

    // Password entry modal overlay (Ctrl+P or "Change Lock Password").
    if let Some(ref entry) = m.entry {
        render_password_modal(buf, w, h, g, gui, entry, m, &pal);
    }

    // 6. Bottom Status Bar.
    let status_y = hi - STATUS_H;
    fill_solid(buf, w, h, SIDEBAR_W + 1, status_y, wi - SIDEBAR_W - 1, STATUS_H, pal.panel);
    fill_solid(buf, w, h, SIDEBAR_W + 1, status_y, wi - SIDEBAR_W - 1, 1, pal.hairline);
    let status = if m.status_len > 0 {
        &m.status[..m.status_len]
    } else {
        b"macOS Noir System Settings  \xE2\x80\xA2  Type to search  \xE2\x80\xA2  PgUp/PgDn pages  \xE2\x80\xA2  Ctrl+P password"
    };
    draw_sans_sm(buf, w, h, g, cx, status_y + 6, cw, status, pal.text_dim);
}

/// Helper: draws a setting label + description + toggle switch inside a card row.
#[allow(clippy::too_many_arguments)]
fn draw_toggle_row(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    cw: i32,
    title: &[u8],
    subtitle: &[u8],
    on: bool,
    act: UiAction,
    divider: bool,
) {
    let row_h = if subtitle.is_empty() { 40 } else { 48 };
    if divider {
        fill_solid(buf, w, h, x + 14, y, cw - 28, 1, pal.hairline);
    }
    if subtitle.is_empty() {
        draw_sans(buf, w, h, g, x + 14, y + 12, cw - 82, title, pal.text);
    } else {
        draw_sans(buf, w, h, g, x + 14, y + 8, cw - 82, title, pal.text);
        draw_sans_sm(buf, w, h, g, x + 14, y + 26, cw - 82, subtitle, pal.text_dim);
    }
    let ty = y + (row_h - 22) / 2;
    draw_noir_toggle(buf, w, h, x + cw - 56, ty, 42, 22, on, gui.btn_state(act), pal);
    gui.push_hit(x + cw - 62, ty - 4, 54, 30, act);
}

/// Helper: draws a segmented pill bar for up to 5 choices.
#[allow(clippy::too_many_arguments)]
fn draw_segmented<F>(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    total_w: i32,
    labels: &[&[u8]],
    active_idx: usize,
    mut make_act: F,
) where
    F: FnMut(usize) -> UiAction,
{
    let n = labels.len().max(1) as i32;
    draw_noir_well(buf, w, h, x, y, total_w, 28, 8, pal.specular, pal);
    let seg_w = ((total_w - 6) / n).max(32);
    for (i, &lbl) in labels.iter().enumerate() {
        let sx = x + 3 + i as i32 * seg_w;
        let sw = if i + 1 == labels.len() { x + total_w - 3 - sx } else { seg_w - 2 };
        let act = make_act(i);
        let active = i == active_idx;
        let hovered = gui.hovered == Some(act);
        if active {
            fill_rounded_rect(buf, w, h, sx, y + 3, sw, 22, 6, pal.accent);
        } else if hovered {
            fill_rounded_rect(buf, w, h, sx, y + 3, sw, 22, 6, pal.control_hover);
        }
        let tw = g.measure(if active { 1 } else { 0 }, 11, lbl).min(sw - 6);
        let tx = sx + ((sw - tw) / 2).max(3);
        let ink = if active { pal.text_on_accent } else { pal.text_dim };
        g.draw_text(buf, w, h, tx, y + 7, sw - 6, 11, if active { 1 } else { 0 }, lbl, ink);
        gui.push_hit(sx, y + 2, sw, 24, act);
    }
}

/// Helper: draws an interactive integer slider + `-` / `+` steppers + value pill.
#[allow(clippy::too_many_arguments)]
fn draw_slider_row(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    cw: i32,
    key_id: u16,
    label: &[u8],
    val: i32,
    min: i32,
    max: i32,
    unit: &[u8],
    divider: bool,
) {
    if divider {
        fill_solid(buf, w, h, x + 14, y, cw - 28, 1, pal.hairline);
    }
    draw_sans(buf, w, h, g, x + 14, y + 10, 220, label, pal.text);

    // Numeric badge on right.
    let mut vbuf = [0u8; 24];
    let mut vn = format_i32_ascii(val, &mut vbuf);
    let take = unit.len().min(vbuf.len() - vn);
    vbuf[vn..vn + take].copy_from_slice(&unit[..take]);
    vn += take;

    let dec_act = UiAction::StepSetting(key_id, false);
    let inc_act = UiAction::StepSetting(key_id, true);
    let dec_x = x + cw - 116;
    let inc_x = x + cw - 36;
    draw_candy_button(buf, w, h, dec_x, y + 10, 22, 22, 6, ButtonStyle::Normal, gui.btn_state(dec_act), pal);
    draw_sans_bold(buf, w, h, g, dec_x + 7, y + 12, 12, 13, b"-", pal.text);
    gui.push_hit(dec_x, y + 8, 22, 26, dec_act);

    let val_w = g.measure(2, 12, &vbuf[..vn]).min(50);
    draw_mono(buf, w, h, g, dec_x + 26 + (52 - val_w) / 2, y + 13, 52, &vbuf[..vn], pal.accent);

    draw_candy_button(buf, w, h, inc_x, y + 10, 22, 22, 6, ButtonStyle::Normal, gui.btn_state(inc_act), pal);
    draw_sans_bold(buf, w, h, g, inc_x + 6, y + 12, 12, 13, b"+", pal.text);
    gui.push_hit(inc_x, y + 8, 22, 26, inc_act);

    // Interactive clickable slider segments across the track.
    let sl_x = (x + 236).min(dec_x - 84);
    let sl_w = (dec_x - sl_x - 14).max(72);
    let hov = matches!(gui.hovered, Some(UiAction::SetSettingValue(k, _)) if k == key_id);
    draw_noir_slider(buf, w, h, sl_x, y + 10, sl_w, val, min, max, hov, pal);

    // Register 10 clickable target buckets along the slider track for smooth direct-click scrubbing.
    let buckets = 10i32;
    let bw = (sl_w / buckets).max(6);
    for b in 0..buckets {
        let bx = sl_x + b * bw;
        let sw = if b + 1 == buckets { sl_x + sl_w - bx } else { bw };
        let frac_num = if b + 1 == buckets { max - min } else { ((max - min) * b) / (buckets - 1) };
        let target = (min + frac_num).clamp(min, max);
        gui.push_hit(bx, y + 6, sw, 28, UiAction::SetSettingValue(key_id, target));
    }
}

/// Helper: draws a `< value >` choice stepper row.
#[allow(clippy::too_many_arguments)]
fn draw_choice_row(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    cw: i32,
    key_id: u16,
    label: &[u8],
    value_text: &[u8],
    divider: bool,
) {
    if divider {
        fill_solid(buf, w, h, x + 14, y, cw - 28, 1, pal.hairline);
    }
    draw_sans(buf, w, h, g, x + 14, y + 11, (cw - 220).max(90), label, pal.text);

    let dec = UiAction::StepSetting(key_id, false);
    let inc = UiAction::StepSetting(key_id, true);
    let inc_x = x + cw - 36;
    let dec_x = x + cw - 192;
    draw_candy_button(buf, w, h, dec_x, y + 9, 24, 22, 6, ButtonStyle::Normal, gui.btn_state(dec), pal);
    draw_sans_bold(buf, w, h, g, dec_x + 8, y + 11, 14, 12, b"<", pal.text);
    gui.push_hit(dec_x, y + 7, 24, 26, dec);

    let vw = g.measure(0, 12, value_text).min(120);
    draw_sans(buf, w, h, g, dec_x + 28 + (126 - vw) / 2, y + 11, 126, value_text, pal.accent);

    draw_candy_button(buf, w, h, inc_x, y + 9, 24, 22, 6, ButtonStyle::Normal, gui.btn_state(inc), pal);
    draw_sans_bold(buf, w, h, g, inc_x + 8, y + 11, 14, 12, b">", pal.text);
    gui.push_hit(inc_x, y + 7, 24, 26, inc);
}

#[allow(clippy::too_many_arguments)]
fn render_live_preview(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    cw: i32,
) -> i32 {
    let ph = 92;
    let r = CORNER_RADII[gui.corner_radius_idx.min(CORNER_RADII.len() - 1)].1 as i32;
    let theme_val = m.values.value(id::THEME);
    let frosted = m.values.value(id::REDUCE_TRANSPARENCY) == 0;
    draw_noir_card(buf, w, h, x, y, cw, ph, r.max(8), pal);

    // Mini desktop bar preview reflecting desktop `appearance.theme` (Light vs Dark vs High Contrast).
    let mini_bar_bg = match theme_val {
        0 => 0x00ED_E9F6,
        2 => 0x0000_0000,
        _ => 0x000E_1119,
    };
    let mini_bar_fg = match theme_val {
        0 => 0x001E_162E,
        _ => pal.text,
    };
    fill_rounded_rect(buf, w, h, x + 2, y + 2, cw - 4, 26, (r - 2).max(4), mini_bar_bg);
    fill_rounded_rect(buf, w, h, x + 12, y + 10, 10, 10, 5, 0x00FF_5F57);
    fill_rounded_rect(buf, w, h, x + 26, y + 10, 10, 10, 5, 0x00FE_BC2E);
    fill_rounded_rect(buf, w, h, x + 40, y + 10, 10, 10, 5, 0x0028_C840);
    draw_sans_bold(buf, w, h, g, x + 60, y + 8, cw - 200, 12, b"Live Desktop Preview", mini_bar_fg);

    let mode_lbl: &[u8] = match theme_val {
        0 => b"Light Bar",
        2 => b"High Contrast",
        _ => b"Noir Dark",
    };
    draw_candy_badge(buf, w, h, x + cw - 98, y + 6, 86, 18, pal.accent, pal);
    let mlw = g.measure(1, 11, mode_lbl).min(78);
    draw_sans_bold(buf, w, h, g, x + cw - 98 + (86 - mlw) / 2, y + 7, 78, 11, mode_lbl, pal.text_on_accent);

    // Preview pills inside body.
    draw_candy_button(buf, w, h, x + 14, y + 40, 118, 28, r, ButtonStyle::Primary, ButtonState::Rest, pal);
    draw_sans_bold(buf, w, h, g, x + 28, y + 47, 94, 12, b"Accent Action", pal.text_on_accent);

    let wall_name = WALLPAPERS.get(m.values.value(id::WALLPAPER) as usize).copied().unwrap_or("sexos");
    draw_candy_button(buf, w, h, x + 142, y + 40, 134, 28, r, ButtonStyle::Normal, ButtonState::Rest, pal);
    draw_sans(buf, w, h, g, x + 154, y + 47, 112, wall_name.as_bytes(), pal.text);

    draw_sans_sm(buf, w, h, g, x + cw - 134, y + 47, 68, if frosted { b"Frosted" } else { b"Opaque" }, pal.text_dim);
    draw_noir_toggle(buf, w, h, x + cw - 58, y + 43, 42, 22, frosted, ButtonState::Rest, pal);
    ph
}

#[allow(clippy::too_many_arguments)]
fn render_appearance(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    y += render_live_preview(buf, w, h, g, gui, m, pal, x, y, cw) + 12;

    // Theme Mode Card.
    draw_noir_card(buf, w, h, x, y, cw, 66, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Desktop Theme Mode", pal.text);
    let cur_theme = m.values.value(id::THEME).clamp(0, 2) as usize;
    draw_segmented(
        buf,
        w,
        h,
        g,
        gui,
        pal,
        x + 14,
        y + 30,
        cw - 28,
        &[b"Light", b"Noir Dark", b"High Contrast"],
        cur_theme,
        |i| UiAction::SetTheme(i as i32),
    );
    y += 78;

    // 4 Accent Tints Picker Card.
    draw_noir_card(buf, w, h, x, y, cw, 82, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Accent Tint", pal.text);
    let cur_accent = (m.values.value(id::ACCENT) as u32) & 0x00FF_FFFF;
    let card_gap = 10;
    let tint_w = ((cw - 28 - card_gap * 3) / 4).max(76);
    for (i, (name, rgb_i32)) in ACCENT_TINTS.iter().enumerate() {
        let tx = x + 14 + i as i32 * (tint_w + card_gap);
        let ty = y + 32;
        let act = UiAction::SetAccent(*rgb_i32);
        let selected = cur_accent == *rgb_i32 as u32
            || (i == 0 && cur_accent == 0x00D6_336C);
        let hovered = gui.hovered == Some(act);
        let bg = if selected { pal.selection } else if hovered { pal.control_hover } else { pal.well };
        let rim = if selected { *rgb_i32 as u32 } else { pal.specular };
        fill_rounded_rect(buf, w, h, tx, ty, tint_w, 38, 8, rim);
        fill_rounded_rect(buf, w, h, tx + 1, ty + 1, tint_w - 2, 36, 7, bg);
        draw_candy_swatch(buf, w, h, tx + 10, ty + 9, 20, *rgb_i32 as u32, selected, hovered, pal);
        draw_sans_sm(
            buf,
            w,
            h,
            g,
            tx + 36,
            ty + 12,
            tint_w - 42,
            name.as_bytes(),
            if selected { pal.text } else { pal.text_dim },
        );
        gui.push_hit(tx, ty, tint_w, 38, act);
    }
    y += 94;

    // Glass, Animations, Wallpaper & Desktop Widgets grouped card.
    draw_noir_card(buf, w, h, x, y, cw, 212, 10, pal);
    let frosted_on = m.values.value(id::REDUCE_TRANSPARENCY) == 0;
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y, cw,
        b"Frosted Glass & Translucent Panels",
        b"Disable to make SilkBar and popovers solid opaque",
        frosted_on,
        UiAction::ToggleFrostedGlass,
        false,
    );
    let anim_on = m.values.value(id::ANIMATIONS) != 0;
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 48, cw,
        b"Smooth Motion & Window Animations",
        b"Fluid transitions across Spaces and SilkBar",
        anim_on,
        UiAction::ToggleAnimations,
        true,
    );
    let wall = WALLPAPERS.get(m.values.value(id::WALLPAPER) as usize).copied().unwrap_or("sexos");
    draw_choice_row(buf, w, h, g, gui, pal, x, y + 96, cw, id::WALLPAPER, b"Desktop Wallpaper", wall.as_bytes(), true);
    let widgets_on = m.values.value(id::WIDGETS) != 0;
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 136, cw,
        b"Desktop Clock & System Widgets",
        b"",
        widgets_on,
        UiAction::ToggleBoolSetting(id::WIDGETS),
        true,
    );
    let weather_on = m.values.value(id::WEATHER) != 0;
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 174, cw,
        b"Weather Widget (Open-Meteo)",
        b"",
        weather_on,
        UiAction::ToggleBoolSetting(id::WEATHER),
        true,
    );
    y += 224;

    // One-click Noir Presets.
    draw_sans_bold(buf, w, h, g, x + 2, y, cw, 12, b"One-Click Noir Presets", pal.text_dim);
    y += 20;
    for (idx, p) in PROFILES.iter().enumerate() {
        let act = UiAction::ApplyProfile(idx);
        let active = gui.active_profile == Some(idx);
        let hovered = gui.hovered == Some(act);
        draw_noir_card(buf, w, h, x, y, cw, 48, 9, pal);
        if active {
            fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, 46, 8, pal.selection);
        } else if hovered {
            fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, 46, 8, pal.control);
        }
        draw_candy_swatch(buf, w, h, x + 14, y + 12, 24, p.accent as u32, active, hovered, pal);
        draw_sans_bold(buf, w, h, g, x + 48, y + 8, cw - 140, 13, p.name.as_bytes(), pal.text);
        draw_sans_sm(buf, w, h, g, x + 48, y + 26, cw - 140, p.subtitle.as_bytes(), pal.text_dim);
        let badge_col = if active { pal.accent } else { pal.control };
        let badge_ink = if active { pal.text_on_accent } else { pal.text };
        draw_candy_badge(buf, w, h, x + cw - 78, y + 13, 64, 22, badge_col, pal);
        draw_sans_bold(buf, w, h, g, x + cw - 62, y + 16, 48, 11, if active { b"Active" } else { b"Apply" }, badge_ink);
        gui.push_hit(x, y, cw, 48, act);
        y += 56;
    }
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_network(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    // Wi-Fi Radio & Status Card.
    draw_noir_card(buf, w, h, x, y, cw, 98, 10, pal);
    draw_sidebar_badge(buf, w, h, g, x + 14, y + 14, 0x003B_82F6, Icon::Network);
    draw_sans_bold(buf, w, h, g, x + 44, y + 14, cw - 140, 14, b"Wi-Fi & SexNet TLS", pal.text);
    let status_lbl: &[u8] = if gui.wifi_enabled { b"Enabled" } else { b"Off" };
    draw_candy_badge(buf, w, h, x + cw - 134, y + 14, 64, 20, if gui.wifi_enabled { pal.mint } else { pal.control }, pal);
    draw_sans_bold(buf, w, h, g, x + cw - 122, y + 17, 50, 11, status_lbl, if gui.wifi_enabled { 0x000B_0D12 } else { pal.text_dim });
    draw_noir_toggle(buf, w, h, x + cw - 58, y + 13, 42, 22, gui.wifi_enabled, gui.btn_state(UiAction::ToggleWifiRadio), pal);
    gui.push_hit(x + cw - 62, y + 10, 54, 28, UiAction::ToggleWifiRadio);

    fill_solid(buf, w, h, x + 14, y + 48, cw - 28, 1, pal.hairline);
    draw_sans_sm(
        buf, w, h, g, x + 14, y + 58, cw - 28,
        b"Click the SilkBar Wi-Fi icon to scan nearby SSIDs, enter WPA2/WPA3 passphrases,",
        pal.text_dim,
    );
    draw_sans_sm(
        buf, w, h, g, x + 14, y + 74, cw - 28,
        b"or manage wired DHCP & TLS trust roots via SexNet.",
        pal.text_dim,
    );
    y += 110;

    // Network Permission Policy.
    draw_noir_card(buf, w, h, x, y, cw, 82, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 12, cw - 28, 13, b"First-Run Program Network Access", pal.text);
    let net_def = m.values.value(id::FIRST_RUN_NETWORK).clamp(0, 2) as usize;
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 40, cw - 28,
        &[b"Ask First", b"Deny", b"Allow"],
        net_def,
        |i| UiAction::SetSettingValue(id::FIRST_RUN_NETWORK, i as i32),
    );
    y += 94;

    // Network Time Sync.
    draw_noir_card(buf, w, h, x, y, cw, 50, 10, pal);
    let ntp = m.values.value(id::NTP) != 0;
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 2, cw,
        b"Automatic Network Time (NTP)",
        b"Sync system clock securely over SexNet",
        ntp,
        UiAction::ToggleBoolSetting(id::NTP),
        false,
    );
    y += 62;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_sound(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    draw_noir_card(buf, w, h, x, y, cw, 92, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 12, cw - 28, 13, b"Output Volume (SexBell Audio Mixer)", pal.text);
    let vol = m.values.value(id::VOLUME);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 38, cw, id::VOLUME, b"Master Volume", vol, 0, 100, b"%", false);
    y += 104;

    draw_noir_card(buf, w, h, x, y, cw, 82, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 12, cw - 28, 13, b"First-Run Microphone Access", pal.text);
    let mic_def = m.values.value(id::FIRST_RUN_MICROPHONE).clamp(0, 2) as usize;
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 40, cw - 28,
        &[b"Ask First", b"Deny", b"Allow"],
        mic_def,
        |i| UiAction::SetSettingValue(id::FIRST_RUN_MICROPHONE, i as i32),
    );
    y += 94;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_display(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    // Scale Selector Card.
    draw_noir_card(buf, w, h, x, y, cw, 74, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 13, b"Display Resolution Scale", pal.text);
    let scale = m.values.value(id::SCALE).clamp(0, 3) as usize;
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 34, cw - 28,
        &[b"Automatic", b"1x (Standard)", b"2x (HiDPI)", b"3x (Large)"],
        scale,
        |i| UiAction::SetSettingValue(id::SCALE, i as i32),
    );
    y += 86;

    // Night Light & Vision Card.
    draw_noir_card(buf, w, h, x, y, cw, 180, 10, pal);
    let nl = m.values.value(id::NIGHT_LIGHT);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 6, cw, id::NIGHT_LIGHT, b"Night Light Warmth (0-3)", nl, 0, 3, b"", false);
    let mag = m.values.value(id::MAGNIFIER_ZOOM);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 48, cw, id::MAGNIFIER_ZOOM, b"Magnifier Zoom (quarters)", mag, 4, 32, b"/4x", true);
    let cf_idx = m.values.value(id::COLOUR_FILTER) as usize;
    let cf_name = COLOUR_FILTERS.get(cf_idx).copied().unwrap_or("Off");
    draw_choice_row(buf, w, h, g, gui, pal, x, y + 92, cw, id::COLOUR_FILTER, b"Colour Filter", cf_name.as_bytes(), true);
    let sr = m.values.value(id::SCREEN_READER) != 0;
    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 134, cw, b"Screen Reader (Whisper Voice)", b"", sr, UiAction::ToggleBoolSetting(id::SCREEN_READER), true);
    y += 192;

    // Hot Corners Card.
    draw_noir_card(buf, w, h, x, y, cw, 178, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Hot Corners", pal.text);
    for (idx, (kid, lbl)) in [
        (id::CORNER_TOP_LEFT, &b"Top-Left Corner"[..]),
        (id::CORNER_TOP_RIGHT, &b"Top-Right Corner"[..]),
        (id::CORNER_BOTTOM_LEFT, &b"Bottom-Left Corner"[..]),
        (id::CORNER_BOTTOM_RIGHT, &b"Bottom-Right Corner"[..]),
    ]
    .iter()
    .enumerate()
    {
        let val = m.values.value(*kid) as usize;
        let cname = CORNER_ACTIONS.get(val).copied().unwrap_or("none");
        draw_choice_row(buf, w, h, g, gui, pal, x, y + 28 + idx as i32 * 36, cw, *kid, lbl, cname.as_bytes(), idx > 0);
    }
    y += 190;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_trackpad(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    y += render_live_preview(buf, w, h, g, gui, m, pal, x, y, cw) + 12;

    // Multitouch Gestures Card.
    draw_noir_card(buf, w, h, x, y, cw, 198, 10, pal);
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y, cw,
        b"Tap to Click",
        b"Tap trackpad lightly with one finger to click",
        gui.trackpad_tap_click,
        UiAction::ToggleTrackpad(0),
        false,
    );
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 48, cw,
        b"Natural Scrolling",
        b"Content tracks finger movement direction",
        gui.trackpad_natural_scroll,
        UiAction::ToggleTrackpad(1),
        true,
    );
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 96, cw,
        b"Three-Finger Horizontal Swipe Between Spaces",
        b"Switch workspaces smoothly with 3-finger gestures",
        gui.trackpad_three_finger_spaces,
        UiAction::ToggleTrackpad(2),
        true,
    );
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 144, cw,
        b"Four-Finger Pinch / Swipe Up for Overview",
        b"Open Space Expose & Mission Overview",
        gui.trackpad_four_finger_overview,
        UiAction::ToggleTrackpad(3),
        true,
    );
    y += 210;

    // Window Tiling & Corner Radius Card.
    draw_noir_card(buf, w, h, x, y, cw, 134, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Window Tiling Behavior", pal.text);
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 32, cw - 28,
        &[
            TilingStyle::HybridCandy.label().as_bytes(),
            TilingStyle::HyprlandMaster.label().as_bytes(),
            TilingStyle::KdeFloat.label().as_bytes(),
        ],
        match gui.tiling {
            TilingStyle::HybridCandy => 0,
            TilingStyle::HyprlandMaster => 1,
            TilingStyle::KdeFloat => 2,
        },
        |i| UiAction::SetTiling([TilingStyle::HybridCandy, TilingStyle::HyprlandMaster, TilingStyle::KdeFloat][i]),
    );
    draw_sans_bold(buf, w, h, g, x + 14, y + 72, cw - 28, 12, b"Window Corner Radius", pal.text);
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 94, cw - 28,
        &[b"4 px", b"8 px", b"12 px"],
        gui.corner_radius_idx.min(2),
        UiAction::SetCornerRadius,
    );
    y += 146;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_apps(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    // SilkBar Close Behavior & Chrome Card.
    draw_noir_card(buf, w, h, x, y, cw, 134, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"SilkBar Close Button Behavior", pal.text);
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 32, cw - 28,
        &[
            CloseBehavior::QuitApp.label().as_bytes(),
            CloseBehavior::MinimizeDock.label().as_bytes(),
            CloseBehavior::AskFirst.label().as_bytes(),
        ],
        match gui.close_behavior {
            CloseBehavior::QuitApp => 0,
            CloseBehavior::MinimizeDock => 1,
            CloseBehavior::AskFirst => 2,
        },
        |i| UiAction::SetCloseBehavior([CloseBehavior::QuitApp, CloseBehavior::MinimizeDock, CloseBehavior::AskFirst][i]),
    );
    draw_sans_bold(buf, w, h, g, x + 14, y + 72, cw - 28, 12, b"Tab Bar & Window Chrome Style", pal.text);
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 94, cw - 28,
        &[TabBarStyle::SilkPill.label().as_bytes(), TabBarStyle::Classic.label().as_bytes()],
        match gui.tab_bar { TabBarStyle::SilkPill => 0, TabBarStyle::Classic => 1 },
        |i| UiAction::SetTabBar([TabBarStyle::SilkPill, TabBarStyle::Classic][i]),
    );
    y += 146;

    // Default Applications Card.
    draw_noir_card(buf, w, h, x, y, cw, 206, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Default Applications", pal.text);
    let defs: [(u16, &[u8]); 5] = [
        (id::OPEN_PDF, b"PDF Documents"),
        (id::OPEN_IMAGE, b"Images & Photos"),
        (id::OPEN_VIDEO, b"Videos"),
        (id::OPEN_AUDIO, b"Music & Audio"),
        (id::OPEN_PAGE, b"Web Links"),
    ];
    for (idx, (kid, lbl)) in defs.iter().enumerate() {
        let mut vbuf = [0u8; 32];
        let vlen = sex_config::key(*kid).map_or(0, |k| model::show_value(k, m.values.value(*kid), &mut vbuf));
        draw_choice_row(buf, w, h, g, gui, pal, x, y + 26 + idx as i32 * 35, cw, *kid, lbl, &vbuf[..vlen], idx > 0);
    }
    y += 218;

    // Startup Programs Card (`PROGRAMS` toggles).
    let row_h = 34;
    let card_h = 34 + PROGRAMS.len() as i32 * row_h;
    draw_noir_card(buf, w, h, x, y, cw, card_h, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Open at Login (Startup Programs)", pal.text);
    for (i, prog) in PROGRAMS.iter().enumerate() {
        let ry = y + 32 + i as i32 * row_h;
        if i > 0 {
            fill_solid(buf, w, h, x + 14, ry, cw - 28, 1, pal.hairline);
        }
        draw_sans(buf, w, h, g, x + 14, ry + 8, cw - 150, prog.as_bytes(), pal.text);
        let on = m.values.holds(id::STARTUP, i);
        let act = UiAction::ToggleSetBit(id::STARTUP, i as u8);
        draw_noir_toggle(buf, w, h, x + cw - 56, ry + 6, 42, 22, on, gui.btn_state(act), pal);
        gui.push_hit(x + cw - 62, ry + 2, 54, 30, act);
    }
    y += card_h + 12;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_keyboard(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    // Key Repeat & Layout Card.
    draw_noir_card(buf, w, h, x, y, cw, 174, 10, pal);
    let lay = LAYOUTS.get(m.values.value(id::LAYOUT) as usize).copied().unwrap_or("us");
    draw_choice_row(buf, w, h, g, gui, pal, x, y + 4, cw, id::LAYOUT, b"Keyboard Layout", lay.as_bytes(), false);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 42, cw, id::REPEAT_DELAY, b"Key Repeat Delay", m.values.value(id::REPEAT_DELAY), 150, 1000, b"ms", true);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 86, cw, id::REPEAT_RATE, b"Key Repeat Rate", m.values.value(id::REPEAT_RATE), 2, 60, b"/s", true);
    let caps = m.values.value(id::CAPS_AS_CTRL) != 0;
    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 130, cw, b"Caps Lock Acts as Control", b"", caps, UiAction::ToggleBoolSetting(id::CAPS_AS_CTRL), true);
    y += 186;

    // Text Prediction & Typing Assistance Card.
    draw_noir_card(buf, w, h, x, y, cw, 214, 10, pal);
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y, cw,
        b"Inline Text Prediction & Autocomplete",
        b"Suggest words in Jot, Spindle, and search fields",
        gui.text_prediction,
        UiAction::TogglePrediction(0),
        false,
    );
    draw_toggle_row(
        buf, w, h, g, gui, pal, x, y + 48, cw,
        b"Check Spelling & Auto-Correct",
        b"Underline typos and auto-fix common spelling slips",
        gui.text_autocorrect,
        UiAction::TogglePrediction(1),
        true,
    );
    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 96, cw, b"Sticky Keys", b"", m.values.value(id::STICKY_KEYS) != 0, UiAction::ToggleBoolSetting(id::STICKY_KEYS), true);
    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 134, cw, b"Slow Keys", b"", m.values.value(id::SLOW_KEYS) != 0, UiAction::ToggleBoolSetting(id::SLOW_KEYS), true);
    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 172, cw, b"Bounce Keys", b"", m.values.value(id::BOUNCE_KEYS) != 0, UiAction::ToggleBoolSetting(id::BOUNCE_KEYS), true);
    y += 226;

    // Date & Time Card.
    draw_noir_card(buf, w, h, x, y, cw, 82, 10, pal);
    let tz = ZONES.get(m.values.value(id::ZONE) as usize).copied().unwrap_or("UTC");
    draw_choice_row(buf, w, h, g, gui, pal, x, y + 4, cw, id::ZONE, b"Time Zone", tz.as_bytes(), false);
    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 40, cw, b"24-Hour Clock", b"", m.values.value(id::HOUR24) != 0, UiAction::ToggleBoolSetting(id::HOUR24), true);
    y += 94;

    // Hotkeys & Profile Reference Card.
    draw_noir_card(buf, w, h, x, y, cw, 272, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 12, b"Hotkey Profile & Desktop Shortcuts", pal.text);
    draw_segmented(
        buf, w, h, g, gui, pal, x + 14, y + 30, cw - 28,
        &[
            KeybindProfile::HyprlandKde.label().as_bytes(),
            KeybindProfile::HyprlandPure.label().as_bytes(),
            KeybindProfile::KdeClassic.label().as_bytes(),
        ],
        match gui.keybinds {
            KeybindProfile::HyprlandKde => 0,
            KeybindProfile::HyprlandPure => 1,
            KeybindProfile::KdeClassic => 2,
        },
        |i| UiAction::SetKeybinds([KeybindProfile::HyprlandKde, KeybindProfile::HyprlandPure, KeybindProfile::KdeClassic][i]),
    );
    for (i, (keys, desc, _orig)) in HOTKEY_ROWS.iter().enumerate() {
        let ry = y + 68 + i as i32 * 24;
        draw_candy_badge(buf, w, h, x + 14, ry, 174, 19, pal.well, pal);
        draw_mono(buf, w, h, g, x + 22, ry + 2, 160, keys.as_bytes(), pal.accent);
        draw_sans_sm(buf, w, h, g, x + 198, ry + 3, cw - 212, desc.as_bytes(), pal.text_dim);
    }
    y += 284;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_privacy(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;

    // Lock Screen & Idle Security Card.
    draw_noir_card(buf, w, h, x, y, cw, 186, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 190, 13, b"Lock Screen & Session Security", pal.text);
    let pw_act = UiAction::StartPassword;
    draw_candy_button(buf, w, h, x + cw - 166, y + 7, 152, 24, 7, ButtonStyle::Primary, gui.btn_state(pw_act), pal);
    draw_sans_bold(buf, w, h, g, x + cw - 154, y + 12, 132, 11, b"Change Password...", pal.text_on_accent);
    gui.push_hit(x + cw - 166, y + 7, 152, 24, pw_act);

    draw_toggle_row(buf, w, h, g, gui, pal, x, y + 36, cw, b"Lock When Idle", b"", m.values.value(id::LOCK_ON_IDLE) != 0, UiAction::ToggleBoolSetting(id::LOCK_ON_IDLE), true);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 76, cw, id::DIM_AFTER, b"Dim Screen After (0 never)", m.values.value(id::DIM_AFTER), 0, 120, b"m", true);
    draw_slider_row(buf, w, h, g, gui, pal, x, y + 122, cw, id::LOCK_AFTER, b"Lock After (0 never)", m.values.value(id::LOCK_AFTER), 0, 240, b"m", true);
    y += 198;

    // First-Run Consent Defaults Card.
    draw_noir_card(buf, w, h, x, y, cw, 118, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 13, b"Default Consent for New Programs", pal.text);
    let net_v = m.values.value(id::FIRST_RUN_NETWORK).clamp(0, 2) as usize;
    let mic_v = m.values.value(id::FIRST_RUN_MICROPHONE).clamp(0, 2) as usize;
    draw_sans(buf, w, h, g, x + 14, y + 40, 130, b"Network", pal.text);
    draw_segmented(buf, w, h, g, gui, pal, x + 150, y + 34, cw - 164, &[b"Ask", b"Deny", b"Allow"], net_v, |i| UiAction::SetSettingValue(id::FIRST_RUN_NETWORK, i as i32));
    draw_sans(buf, w, h, g, x + 14, y + 80, 130, b"Microphone", pal.text);
    draw_segmented(buf, w, h, g, gui, pal, x + 150, y + 74, cw - 164, &[b"Ask", b"Deny", b"Allow"], mic_v, |i| UiAction::SetSettingValue(id::FIRST_RUN_MICROPHONE, i as i32));
    y += 130;

    // Per-App Permissions List (`grants::Row`).
    let rows_h = if gui.grant_count == 0 { 64 } else { 36 + gui.grant_count as i32 * 40 };
    draw_noir_card(buf, w, h, x, y, cw, rows_h, 10, pal);
    draw_sans_bold(buf, w, h, g, x + 14, y + 10, cw - 28, 13, b"Application Permissions", pal.text);
    if gui.grant_count == 0 {
        draw_sans_sm(buf, w, h, g, x + 14, y + 34, cw - 28, b"No installed packages currently request Network or Microphone grants.", pal.text_dim);
    } else {
        for i in 0..gui.grant_count {
            let row = gui.grant_rows[i];
            let ry = y + 34 + i as i32 * 40;
            if i > 0 {
                fill_solid(buf, w, h, x + 14, ry, cw - 28, 1, pal.hairline);
            }
            draw_sans_bold(buf, w, h, g, x + 14, ry + 11, 130, 13, row.program(), pal.text);
            let perm_col = if row.bit == grants::GRANTS[0].0 { 0x003B_82F6 } else { 0x00F4_3F5E };
            draw_candy_badge(buf, w, h, x + 148, ry + 9, 86, 20, perm_col, pal);
            draw_sans_bold(buf, w, h, g, x + 158, ry + 12, 70, 11, row.word().as_bytes(), 0x00FF_FFFF);

            let (st_lbl, st_col): (&[u8], u32) = if !row.answered {
                (b"Unasked", pal.control)
            } else if row.allowed {
                (b"Allowed", pal.mint)
            } else {
                (b"Denied", pal.danger)
            };
            draw_candy_badge(buf, w, h, x + 244, ry + 9, 70, 20, st_col, pal);
            draw_sans_bold(buf, w, h, g, x + 252, ry + 12, 58, 11, st_lbl, if row.answered && row.allowed { 0x000B_0D12 } else { pal.text });

            // Clickable Allow & Revoke pills.
            let allow_act = UiAction::SetGrant(i, true);
            let deny_act = UiAction::SetGrant(i, false);
            let ax = x + cw - 134;
            let dx = x + cw - 68;
            draw_candy_button(buf, w, h, ax, ry + 8, 58, 22, 6, if row.answered && row.allowed { ButtonStyle::Active } else { ButtonStyle::Normal }, gui.btn_state(allow_act), pal);
            draw_sans_sm(buf, w, h, g, ax + 12, ry + 12, 40, b"Allow", pal.text);
            gui.push_hit(ax, ry + 6, 58, 26, allow_act);

            draw_candy_button(buf, w, h, dx, ry + 8, 56, 22, 6, if row.answered && !row.allowed { ButtonStyle::Danger } else { ButtonStyle::Normal }, gui.btn_state(deny_act), pal);
            draw_sans_sm(buf, w, h, g, dx + 8, ry + 12, 44, b"Revoke", pal.text);
            gui.push_hit(dx, ry + 6, 56, 26, deny_act);
        }
    }
    y += rows_h + 12;
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_storage_install(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;

    // Storage Facts Summary Card (if loaded from Files).
    let info = &m.info[..m.info_len];
    if !info.is_empty() {
        let mut line_count = 0i32;
        for l in info.split(|&b| b == b'\n') {
            if !l.is_empty() { line_count += 1; }
        }
        let info_h = (32 + line_count.min(6) * 17).max(58);
        draw_noir_card(buf, w, h, x, y, cw, info_h, 10, pal);
        draw_sans_bold(buf, w, h, g, x + 14, y + 9, cw - 28, 12, b"Active Filesystem & Volumes", pal.text);
        let mut ly = y + 29;
        for l in info.split(|&b| b == b'\n').filter(|l| !l.is_empty()).take(6) {
            draw_sans_sm(buf, w, h, g, x + 14, ly, cw - 28, l, pal.text_dim);
            ly += 17;
        }
        y += info_h + 10;
    }

    // 1. Visual horizontal color-coded GPT Partition Bar card.
    let bar_card_h = 88;
    draw_noir_card(buf, w, h, x, y, cw, bar_card_h, 10, pal);
    let mut hdr = [0u8; 64];
    let mut hlen = 0usize;
    let prefix = b"GPT Disk Partition Map";
    hdr[..prefix.len()].copy_from_slice(prefix);
    hlen += prefix.len();
    if gui.install.total_sectors > 0 {
        let mut sz = [0u8; 16];
        let sl = format_size_short(gui.install.total_sectors, &mut sz);
        let sep = b" (";
        hdr[hlen..hlen + sep.len()].copy_from_slice(sep);
        hlen += sep.len();
        let take = sl.min(hdr.len() - hlen - 1);
        hdr[hlen..hlen + take].copy_from_slice(&sz[..take]);
        hlen += take;
        hdr[hlen] = b')';
        hlen += 1;
    }
    draw_sans_bold(buf, w, h, g, x + 14, y + 9, cw - 110, 12, &hdr[..hlen], pal.text);

    let ref_w = 78;
    let ref_act = UiAction::InstallRefresh;
    draw_candy_button(buf, w, h, x + cw - ref_w - 10, y + 6, ref_w, 22, 7, ButtonStyle::Normal, gui.btn_state(ref_act), pal);
    draw_sans_sm(buf, w, h, g, x + cw - ref_w + 6, y + 10, ref_w - 12, b"Refresh", pal.text);
    gui.push_hit(x + cw - ref_w - 10, y + 6, ref_w, 22, ref_act);

    let bar_x = x + 14;
    let bar_y = y + 34;
    let bar_w = (cw - 28).max(80);
    let bar_h = 28;
    fill_rounded_rect(buf, w, h, bar_x, bar_y, bar_w, bar_h, 6, pal.well);

    let count = gui.install.part_count;
    if count > 0 {
        let sum_sectors: u64 = gui.install.parts[..count].iter().map(|p| p.sectors.max(1)).sum::<u64>().max(1);
        let min_seg = 24.min(bar_w / (count as i32).max(1)).max(8);
        let mut sx = bar_x;
        let bar_right = bar_x + bar_w;
        for i in 0..count {
            let p = gui.install.parts[i];
            let rem_parts = (count - 1 - i) as i32;
            let raw_w = ((p.sectors as u128 * bar_w as u128) / sum_sectors as u128) as i32;
            let max_here = (bar_right - sx - rem_parts * min_seg).max(min_seg);
            let seg_w = if i + 1 == count { (bar_right - sx).max(1) } else { raw_w.clamp(min_seg, max_here) };
            let selected = gui.install.selected == Some(i);
            let col = p.kind.color();
            if selected {
                fill_rounded_rect(buf, w, h, sx, bar_y - 2, seg_w.saturating_sub(2).max(4), bar_h + 4, 5, 0x00FF_FFFF);
                fill_rounded_rect(buf, w, h, sx + 2, bar_y, seg_w.saturating_sub(6).max(2), bar_h, 4, col);
            } else {
                fill_rounded_rect(buf, w, h, sx, bar_y, seg_w.saturating_sub(2).max(4), bar_h, 4, col);
            }
            if seg_w >= 46 {
                let lbl = if p.kind == DiskPartKind::FreeSpace { b"Free" as &[u8] } else { p.kind.label().as_bytes() };
                draw_sans_bold(buf, w, h, g, sx + 6, bar_y + 7, seg_w - 10, 11, lbl, 0x000B_0D12);
            }
            if p.replaceable {
                gui.push_hit(sx, bar_y, seg_w, bar_h, UiAction::SelectInstallPart(i));
            }
            sx += seg_w;
        }
    } else {
        let msg: &[u8] = if gui.install.loaded {
            b"No GPT partitions detected (click Refresh or use Spindle `install`)."
        } else {
            b"Click Refresh to scan GPT partitions."
        };
        draw_sans_sm(buf, w, h, g, bar_x + 10, bar_y + 8, bar_w - 20, msg, pal.text_dim);
    }

    let legend: [(&[u8], u32); 5] = [
        (b"ESP", DiskPartKind::Esp.color()),
        (b"Windows", DiskPartKind::Windows.color()),
        (b"Linux", DiskPartKind::Linux.color()),
        (b"SexOS", DiskPartKind::SexOs.color()),
        (b"Free", DiskPartKind::FreeSpace.color()),
    ];
    let mut lx = bar_x;
    for (lbl, col) in legend {
        if lx + 72 > bar_x + bar_w { break; }
        fill_rounded_rect(buf, w, h, lx, y + 69, 10, 10, 3, col);
        draw_sans_sm(buf, w, h, g, lx + 14, y + 67, 56, lbl, pal.text_dim);
        lx += g.measure(0, 11, lbl) + 28;
    }
    y += bar_card_h + 10;

    // 2. Clickable Partition Rows.
    for i in 0..count {
        let p = gui.install.parts[i];
        let selected = gui.install.selected == Some(i);
        let act = UiAction::SelectInstallPart(i);
        draw_noir_card(buf, w, h, x, y, cw, 30, 8, pal);
        if selected {
            fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, 28, 7, pal.selection);
        } else if p.replaceable && gui.hovered == Some(act) {
            fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, 28, 7, pal.control);
        }
        fill_rounded_rect(buf, w, h, x + 10, y + 9, 12, 12, 4, p.kind.color());

        let mut num_lbl = [0u8; 8];
        let num_len = if p.kind == DiskPartKind::FreeSpace {
            num_lbl[..4].copy_from_slice(b"FREE");
            4
        } else {
            num_lbl[0] = b'#';
            1 + format_u64_ascii(p.part_num as u64, &mut num_lbl[1..])
        };
        draw_mono(buf, w, h, g, x + 28, y + 8, 44, &num_lbl[..num_len], pal.text);
        let name_col = if p.replaceable { pal.text } else { pal.text_dim };
        draw_sans(buf, w, h, g, x + 76, y + 8, (cw - 270).max(80), p.name_bytes(), name_col);
        draw_sans_sm(buf, w, h, g, x + cw - 190, y + 9, 64, p.kind.label().as_bytes(), pal.text_dim);

        let mut sz = [0u8; 16];
        let sz_len = format_size_short(p.sectors, &mut sz);
        draw_mono(buf, w, h, g, x + cw - 130, y + 8, 66, &sz[..sz_len], name_col);

        if !p.replaceable {
            draw_candy_badge(buf, w, h, x + cw - 68, y + 5, 60, 20, 0x003B_2F1A, pal);
            draw_sans_bold(buf, w, h, g, x + cw - 60, y + 8, 48, 10, b"LOCKED", 0x00FB_BF24);
        } else {
            let pill_col = if selected { pal.accent } else { pal.control };
            let pill_ink = if selected { pal.text_on_accent } else { pal.text };
            let pill_txt: &[u8] = if selected { b"Target" } else { b"Select" };
            draw_candy_badge(buf, w, h, x + cw - 68, y + 5, 60, 20, pill_col, pal);
            draw_sans_bold(buf, w, h, g, x + cw - 57, y + 8, 48, 10, pill_txt, pill_ink);
            gui.push_hit(x, y, cw, 30, act);
        }
        y += 34;
    }

    // 3. Safety Summary Panel & Action Buttons.
    let summary_h = 84;
    let mut sum_buf = [0u8; 160];
    let (sum_len, is_amber) = gui.install.safety_summary(&mut sum_buf);
    let has_target = gui.install.selected_row().is_some();
    let badge_bg = if !has_target { pal.control } else if is_amber { pal.honey } else { pal.mint };
    let badge_lbl: &[u8] = if !has_target { b"SAFETY" } else if is_amber { b"SINGLE-PART" } else { b"SAFE FREE" };

    draw_noir_card(buf, w, h, x, y, cw, summary_h, 10, pal);
    fill_rounded_rect(buf, w, h, x + 2, y + 6, 4, summary_h - 12, 2, badge_bg);
    draw_candy_badge(buf, w, h, x + 14, y + 8, 92, 20, badge_bg, pal);
    draw_sans_bold(buf, w, h, g, x + 22, y + 11, 80, 10, badge_lbl, 0x000B_0D12);

    let split_at = sum_buf[..sum_len].iter().take(68).rposition(|&b| b == b' ').unwrap_or(sum_len.min(68));
    draw_sans_sm(buf, w, h, g, x + 114, y + 11, cw - 128, &sum_buf[..split_at], pal.text);
    if split_at < sum_len {
        draw_sans_sm(buf, w, h, g, x + 14, y + 34, cw - 28, sum_buf[split_at..sum_len].trim_ascii_start(), pal.text);
    }

    let rep = gui.install.report_bytes();
    if !rep.is_empty() {
        let mut ry = y + 52;
        for line in rep.split(|&b| b == b'\n').filter(|l| !l.is_empty()).take(2) {
            draw_sans_sm(buf, w, h, g, x + 14, ry, cw - 28, line, pal.text_dim);
            ry += 15;
        }
    } else {
        draw_sans_sm(buf, w, h, g, x + 14, y + 54, cw - 28, b"Click 'Plan Install' for a dry-run preview before writing anything.", pal.text_dim);
    }
    y += summary_h + 10;

    let plan_w = 132;
    let go_w = 132;
    let plan_style = if has_target { ButtonStyle::Normal } else { ButtonStyle::Disabled };
    let plan_act = UiAction::InstallPlan;
    draw_candy_button(buf, w, h, x, y, plan_w, 30, 8, plan_style, if has_target { gui.btn_state(plan_act) } else { ButtonState::Rest }, pal);
    draw_sans_bold(buf, w, h, g, x + 26, y + 8, plan_w - 20, 12, b"Plan Install", button_text_color(plan_style, pal));
    if has_target { gui.push_hit(x, y, plan_w, 30, plan_act); }

    let go_x = x + plan_w + 12;
    let go_style = if has_target { ButtonStyle::Primary } else { ButtonStyle::Disabled };
    let go_act = UiAction::InstallExecute;
    draw_candy_button(buf, w, h, go_x, y, go_w, 30, 8, go_style, if has_target { gui.btn_state(go_act) } else { ButtonState::Rest }, pal);
    draw_sans_bold(buf, w, h, g, go_x + 28, y + 8, go_w - 20, 12, b"Install Now", button_text_color(go_style, pal));
    if has_target { gui.push_hit(go_x, y, go_w, 30, go_act); }
    y += 40;
    y - start_y
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[allow(clippy::too_many_arguments)]
fn render_updates(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;
    let info = &m.info[..m.info_len];

    // 1. SexOS A/B System Update Card (`xirtus/sexos`).
    let sys_h = 200;
    draw_noir_card(buf, w, h, x, y, cw, sys_h, 10, pal);
    fill_rounded_rect(buf, w, h, x + 2, y + 8, 4, sys_h - 16, 2, pal.accent);
    draw_sans_bold(buf, w, h, g, x + 14, y + 11, cw - 130, 13, b"SexOS System (xirtus/sexos A/B Slot)", pal.text);

    let (slot_badge, slot_col): (&[u8], u32) = if contains_bytes(info, b"slot A") {
        (b"SLOT A", pal.mint)
    } else if contains_bytes(info, b"slot B") {
        (b"SLOT B", pal.sky)
    } else if contains_bytes(info, b"USB stick") {
        (b"LIVE USB", pal.honey)
    } else {
        (b"A/B SLOT", pal.accent)
    };
    draw_candy_badge(buf, w, h, x + cw - 92, y + 9, 80, 20, slot_col, pal);
    draw_sans_bold(buf, w, h, g, x + cw - 80, y + 12, 64, 10, slot_badge, 0x000B_0D12);

    let mut ly = y + 38;
    if !info.is_empty() {
        for line in info.split(|&b| b == b'\n').filter(|l| !l.is_empty()).take(5) {
            draw_sans_sm(buf, w, h, g, x + 14, ly, cw - 28, line, pal.text_dim);
            ly += 18;
        }
    } else {
        draw_sans_sm(buf, w, h, g, x + 14, ly, cw - 28, b"Atomic A/B system updates from github.com/xirtus/sexos.", pal.text_dim);
        draw_sans_sm(buf, w, h, g, x + 14, ly + 18, cw - 28, b"Signed releases verify before switching the inactive slot.", pal.text_dim);
    }

    draw_sans_sm(buf, w, h, g, x + 14, y + sys_h - 58, cw - 28, gui.updates.system_status_bytes(), pal.accent);

    let sys_btn_y = y + sys_h - 38;
    let chk_w = 132;
    let chk_act = UiAction::CheckSystemUpdate;
    draw_candy_button(buf, w, h, x + 14, sys_btn_y, chk_w, 26, 7, ButtonStyle::Primary, gui.btn_state(chk_act), pal);
    draw_sans_bold(buf, w, h, g, x + 32, sys_btn_y + 6, chk_w - 20, 12, b"Check Update", pal.text_on_accent);
    gui.push_hit(x + 14, sys_btn_y, chk_w, 26, chk_act);

    let ref_x = x + 14 + chk_w + 10;
    let ref_w = 124;
    let ref_act = UiAction::RefreshUpdates;
    draw_candy_button(buf, w, h, ref_x, sys_btn_y, ref_w, 26, 7, ButtonStyle::Normal, gui.btn_state(ref_act), pal);
    draw_sans(buf, w, h, g, ref_x + 22, sys_btn_y + 6, ref_w - 20, b"Refresh Slot", pal.text);
    gui.push_hit(ref_x, sys_btn_y, ref_w, 26, ref_act);
    y += sys_h + 12;

    // 2. Private Dotfiles Sync Card (`xirtus/sexosdotfiles`).
    let dot_h = 156;
    draw_noir_card(buf, w, h, x, y, cw, dot_h, 10, pal);
    let dot_col = if gui.updates.dotfiles_synced { pal.mint } else { pal.accent };
    fill_rounded_rect(buf, w, h, x + 2, y + 8, 4, dot_h - 16, 2, dot_col);
    draw_sans_bold(buf, w, h, g, x + 14, y + 11, cw - 120, 13, b"Private Dotfiles (xirtus/sexosdotfiles)", pal.text);
    draw_candy_badge(buf, w, h, x + cw - 86, y + 9, 74, 20, dot_col, pal);
    draw_sans_bold(buf, w, h, g, x + cw - 72, y + 12, 54, 10, if gui.updates.dotfiles_synced { b"SYNCED" } else { b"READY" }, 0x000B_0D12);

    draw_sans_sm(buf, w, h, g, x + 14, y + 36, cw - 28, b"Sync Noir themes, keybinds & Spindle config via xirtus/sexosdotfiles.", pal.text_dim);
    draw_sans_sm(buf, w, h, g, x + 14, y + 54, cw - 28, gui.updates.dotfiles_status_bytes(), pal.accent);

    let tog_y = y + 78;
    draw_sans(buf, w, h, g, x + 14, tog_y + 3, cw - 88, b"Auto-sync private dotfiles on login", pal.text);
    let tog_act = UiAction::ToggleDotfilesAutoSync;
    draw_noir_toggle(buf, w, h, x + cw - 56, tog_y, 42, 22, gui.updates.dotfiles_auto_sync, gui.btn_state(tog_act), pal);
    gui.push_hit(x + cw - 62, tog_y - 3, 54, 28, tog_act);

    let dot_btn_y = y + dot_h - 36;
    let sync_w = 132;
    let sync_act = UiAction::SyncDotfiles;
    draw_candy_button(buf, w, h, x + 14, dot_btn_y, sync_w, 26, 7, ButtonStyle::Primary, gui.btn_state(sync_act), pal);
    draw_sans_bold(buf, w, h, g, x + 32, dot_btn_y + 6, sync_w - 20, 12, b"Sync Dotfiles", pal.text_on_accent);
    gui.push_hit(x + 14, dot_btn_y, sync_w, 26, sync_act);

    let pull_x = x + 14 + sync_w + 10;
    let pull_w = 118;
    let pull_act = UiAction::PullDotfiles;
    draw_candy_button(buf, w, h, pull_x, dot_btn_y, pull_w, 26, 7, ButtonStyle::Normal, gui.btn_state(pull_act), pal);
    draw_sans(buf, w, h, g, pull_x + 24, dot_btn_y + 6, pull_w - 20, b"Pull Latest", pal.text);
    gui.push_hit(pull_x, dot_btn_y, pull_w, 26, pull_act);
    y += dot_h + 12;
    y - start_y
}

fn format_rssi(rssi: i8, out: &mut [u8; 24]) -> usize {
    let mut n = 0usize;
    if rssi == 0 {
        out[..2].copy_from_slice(b"--");
        return 2;
    }
    if rssi < 0 {
        out[n] = b'-';
        n += 1;
    }
    let abs = rssi.unsigned_abs() as u64;
    n += format_u64_ascii(abs, &mut out[n..]);
    let suf = b" dBm";
    let take = suf.len().min(out.len().saturating_sub(n));
    out[n..n + take].copy_from_slice(&suf[..take]);
    n + take
}

#[allow(clippy::too_many_arguments)]
fn render_wifi(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    ch: i32,
) {
    let max_y = y + ch;

    // 1. Current Connection Status Card.
    let status_h = 78;
    draw_noir_card(buf, w, h, x, y, cw, status_h, 10, pal);
    let accent_bar = if gui.wifi.connected { 0x0034_D399 } else { pal.accent };
    fill_rounded_rect(buf, w, h, x + 2, y + 6, 4, status_h - 12, 2, accent_bar);

    draw_sans(buf, w, h, g, x + 14, y + 8, cw - 240, b"Current Wi-Fi Connection", pal.text);

    // Status pill badge.
    let badge_lbl: &[u8] = if gui.wifi.connected { b"CONNECTED" } else { b"DISCONNECTED" };
    let badge_col = if gui.wifi.connected { 0x0034_D399 } else { pal.control };
    let badge_ink = if gui.wifi.connected { 0x0010_0C1C } else { pal.text_dim };
    let badge_w = if gui.wifi.connected { 88 } else { 108 };
    draw_candy_badge(buf, w, h, x + 14, y + 30, badge_w, 20, badge_col, pal);
    draw_sans(buf, w, h, g, x + 20, y + 32, badge_w - 8, badge_lbl, badge_ink);

    // Connected SSID / Status text next to badge.
    let ssid_text: &[u8] = if gui.wifi.active_ssid_len > 0 {
        &gui.wifi.active_ssid[..gui.wifi.active_ssid_len]
    } else {
        gui.wifi.status_bytes()
    };
    draw_sans(buf, w, h, g, x + 22 + badge_w, y + 32, (cw - badge_w - 220).max(80), ssid_text, pal.text);

    // Second info line: IP & Signal quality.
    let mut meta = [0u8; 96];
    let mut mn = 0usize;
    let put_meta = |buf: &mut [u8; 96], at: &mut usize, bytes: &[u8]| {
        let take = bytes.len().min(buf.len().saturating_sub(*at));
        buf[*at..*at + take].copy_from_slice(&bytes[..take]);
        *at += take;
    };
    put_meta(&mut meta, &mut mn, b"IP: ");
    put_meta(&mut meta, &mut mn, gui.wifi.ip_bytes());
    put_meta(&mut meta, &mut mn, b"   Signal: ");
    let mut rbuf = [0u8; 24];
    let rlen = format_rssi(gui.wifi.signal_rssi, &mut rbuf);
    put_meta(&mut meta, &mut mn, &rbuf[..rlen]);
    if gui.wifi.signal_rssi != 0 {
        put_meta(&mut meta, &mut mn, b" (");
        put_meta(&mut meta, &mut mn, WifiState::signal_quality_label(gui.wifi.signal_rssi));
        put_meta(&mut meta, &mut mn, b")");
    }
    draw_sans(buf, w, h, g, x + 14, y + 54, cw - 220, &meta[..mn], pal.text_dim);

    // Action buttons on right of Status card: Scan Wi-Fi + Refresh.
    let scan_w = 96;
    let ref_w = 78;
    let ref_x = x + cw - ref_w - 10;
    let scan_x = ref_x - scan_w - 8;
    let scan_act = UiAction::WifiScan;
    draw_candy_button(buf, w, h, scan_x, y + 10, scan_w, 26, 8, ButtonStyle::Primary, gui.btn_state(scan_act), pal);
    draw_sans(buf, w, h, g, scan_x + 12, y + 15, scan_w - 16, b"Scan Wi-Fi", pal.text_on_accent);
    gui.push_hit(scan_x, y + 10, scan_w, 26, scan_act);

    let ref_act = UiAction::WifiRefresh;
    draw_candy_button(buf, w, h, ref_x, y + 10, ref_w, 26, 8, ButtonStyle::Normal, gui.btn_state(ref_act), pal);
    draw_sans(buf, w, h, g, ref_x + 11, y + 15, ref_w - 14, b"Refresh", pal.text);
    gui.push_hit(ref_x, y + 10, ref_w, 26, ref_act);

    y += status_h + 10;

    // 2. Known / Saved Networks Card (`/data/wifi.conf`).
    let known_count = gui.wifi.known_count;
    let known_rows = known_count.min(4);
    let known_h = (38 + known_rows.max(1) as i32 * 30 + 6).min((max_y - y - 160).max(74));
    draw_noir_card(buf, w, h, x, y, cw, known_h, 10, pal);
    draw_sans(buf, w, h, g, x + 14, y + 8, cw - 130, b"Known / Saved Networks (/data/wifi.conf)", pal.text);

    let mut cnt_buf = [0u8; 16];
    let mut cn = format_u64_ascii(known_count as u64, &mut cnt_buf);
    let suf = b" saved";
    cnt_buf[cn..cn + suf.len()].copy_from_slice(suf);
    cn += suf.len();
    draw_candy_badge(buf, w, h, x + cw - 82, y + 6, 70, 20, pal.control, pal);
    draw_sans(buf, w, h, g, x + cw - 74, y + 8, 58, &cnt_buf[..cn], pal.accent);

    let mut ky = y + 32;
    if known_count == 0 {
        draw_sans(
            buf,
            w,
            h,
            g,
            x + 14,
            ky + 4,
            cw - 28,
            b"No saved networks yet. Add or join a network below for boot auto-connect.",
            pal.text_dim,
        );
    } else {
        for i in 0..known_rows {
            if ky + 28 > y + known_h - 4 {
                break;
            }
            let entry = gui.wifi.known[i];
            draw_candy_selection_row(buf, w, h, x + 10, ky, cw - 20, 26, 7, pal);
            draw_sans(buf, w, h, g, x + 18, ky + 5, (cw - 290).max(80), entry.ssid_bytes(), pal.text);

            // Auto-connect Candy badge.
            let ac_w = 104;
            let ac_x = x + cw - 252;
            if ac_x > x + 120 {
                draw_candy_badge(buf, w, h, ac_x, ky + 3, ac_w, 20, pal.accent, pal);
                draw_sans(buf, w, h, g, ac_x + 6, ky + 5, ac_w - 8, b"Auto-connect", pal.text_on_accent);
            }

            // Connect button.
            let conn_w = 68;
            let conn_x = x + cw - 142;
            let conn_act = UiAction::WifiConnectKnown(i);
            draw_candy_button(buf, w, h, conn_x, ky + 2, conn_w, 22, 6, ButtonStyle::Normal, gui.btn_state(conn_act), pal);
            draw_sans(buf, w, h, g, conn_x + 6, ky + 5, conn_w - 8, b"Connect", pal.text);
            gui.push_hit(conn_x, ky + 2, conn_w, 22, conn_act);

            // Forget button.
            let fgt_w = 58;
            let fgt_x = x + cw - 68;
            let fgt_act = UiAction::WifiForgetKnown(i);
            draw_candy_button(buf, w, h, fgt_x, ky + 2, fgt_w, 22, 6, ButtonStyle::Subtle, gui.btn_state(fgt_act), pal);
            draw_sans(buf, w, h, g, fgt_x + 5, ky + 5, fgt_w - 8, b"Forget", pal.text_dim);
            gui.push_hit(fgt_x, ky + 2, fgt_w, 22, fgt_act);

            ky += 30;
        }
    }
    y += known_h + 10;

    // 3. Scan & Quick Join Card (available networks + SSID/Password form).
    let join_h = (max_y - y).max(126);
    draw_noir_card(buf, w, h, x, y, cw, join_h, 10, pal);
    draw_sans(buf, w, h, g, x + 14, y + 8, cw - 28, b"Available Networks & Quick Join", pal.text);

    // Available scanned networks list (above quick-join form).
    let form_y = (y + join_h - 46).max(y + 56);
    let mut sy = y + 30;
    if gui.wifi.scan_count == 0 {
        draw_sans(
            buf,
            w,
            h,
            g,
            x + 14,
            sy + 2,
            cw - 28,
            b"Click 'Scan Wi-Fi' to list nearby networks, or enter an SSID below.",
            pal.text_dim,
        );
    } else {
        for i in 0..gui.wifi.scan_count {
            if sy + 24 > form_y - 6 {
                break;
            }
            let s = gui.wifi.scanned[i];
            let act = UiAction::WifiSelectScan(i);
            let st = gui.btn_state(act);
            let style = if st == ButtonState::Hover { ButtonStyle::Active } else { ButtonStyle::Subtle };
            draw_candy_button(buf, w, h, x + 10, sy, cw - 20, 22, 6, style, st, pal);
            let name = if s.ssid_len > 0 { s.ssid_bytes() } else { b"(hidden)" };
            draw_sans(buf, w, h, g, x + 18, sy + 3, (cw - 210).max(80), name, pal.text);

            let mut rb = [0u8; 24];
            let rl = format_rssi(s.rssi, &mut rb);
            draw_sans(buf, w, h, g, x + cw - 178, sy + 3, 72, &rb[..rl], pal.text_dim);

            let sec_lbl: &[u8] = if s.security == 0 { b"Open" } else { b"WPA2/3" };
            draw_candy_badge(buf, w, h, x + cw - 86, sy + 2, 64, 18, pal.control, pal);
            draw_sans(buf, w, h, g, x + cw - 78, sy + 3, 52, sec_lbl, pal.accent);

            gui.push_hit(x + 10, sy, cw - 20, 22, act);
            sy += 24;
        }
    }

    // Quick-join input row: [SSID input] [Password input] [Save & Join].
    fill_solid(buf, w, h, x + 12, form_y - 6, cw - 24, 1, pal.hairline);
    let join_w = 104;
    let field_area = (cw - 36 - join_w).max(160);
    let ssid_w = (field_area * 48) / 100;
    let pass_w = (field_area - ssid_w - 8).max(76);
    let ssid_x = x + 12;
    let pass_x = ssid_x + ssid_w + 8;
    let join_x = x + cw - join_w - 12;

    // SSID box.
    let ssid_focused = gui.wifi.focus == WifiFieldFocus::Ssid;
    let ssid_act = UiAction::WifiFocusSsid;
    let ssid_style = if ssid_focused { ButtonStyle::Active } else { ButtonStyle::Normal };
    draw_candy_button(buf, w, h, ssid_x, form_y + 4, ssid_w, 28, 7, ssid_style, gui.btn_state(ssid_act), pal);
    if gui.wifi.form_ssid_len > 0 {
        draw_sans(buf, w, h, g, ssid_x + 8, form_y + 10, ssid_w - 14, gui.wifi.form_ssid_bytes(), pal.text);
    } else {
        draw_sans(buf, w, h, g, ssid_x + 8, form_y + 10, ssid_w - 14, b"SSID...", pal.text_dim);
    }
    gui.push_hit(ssid_x, form_y + 4, ssid_w, 28, ssid_act);

    // Password box (masked with '*').
    let pass_focused = gui.wifi.focus == WifiFieldFocus::Password;
    let pass_act = UiAction::WifiFocusPass;
    let pass_style = if pass_focused { ButtonStyle::Active } else { ButtonStyle::Normal };
    draw_candy_button(buf, w, h, pass_x, form_y + 4, pass_w, 28, 7, pass_style, gui.btn_state(pass_act), pal);
    if gui.wifi.form_pass_len > 0 {
        let mask = [b'*'; 63];
        draw_sans(buf, w, h, g, pass_x + 8, form_y + 10, pass_w - 14, &mask[..gui.wifi.form_pass_len], pal.text);
    } else {
        draw_sans(buf, w, h, g, pass_x + 8, form_y + 10, pass_w - 14, b"Passphrase...", pal.text_dim);
    }
    gui.push_hit(pass_x, form_y + 4, pass_w, 28, pass_act);

    // Save & Join button.
    let can_join = gui.wifi.form_ssid_len > 0;
    let join_act = UiAction::WifiJoinSubmit;
    let join_style = if can_join { ButtonStyle::Primary } else { ButtonStyle::Disabled };
    let join_state = if can_join { gui.btn_state(join_act) } else { ButtonState::Rest };
    draw_candy_button(buf, w, h, join_x, form_y + 4, join_w, 28, 8, join_style, join_state, pal);
    draw_sans(buf, w, h, g, join_x + 8, form_y + 10, join_w - 12, b"Save & Join", button_text_color(join_style, pal));
    gui.push_hit(join_x, form_y + 4, join_w, 28, join_act);
}

#[allow(clippy::too_many_arguments)]
fn render_about(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    _m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    _top: i32,
    _bot: i32,
) -> i32 {
    let start_y = y;

    // Centered macOS "About This Mac"-style Hero Card with SexOS Lips Emblem.
    let hero_h = 158;
    draw_noir_card(buf, w, h, x, y, cw, hero_h, 12, pal);
    let mid_x = x + cw / 2;
    fill_rounded_rect(buf, w, h, mid_x - 62, y + 14, 124, 72, 18, 0x0024_2022);
    draw_sexos_lips(buf, w, h, mid_x, y + 50, 96, false);

    let title = b"SexOS";
    let tw = g.measure(1, 20, title);
    draw_sans_bold(buf, w, h, g, mid_x - tw / 2, y + 92, cw - 32, 20, title, pal.text);

    let sub = b"Single-Address-Space Microkernel";
    let sw = g.measure(0, 12, sub);
    draw_sans_sm(buf, w, h, g, mid_x - sw / 2, y + 116, cw - 32, sub, pal.text_dim);

    let slot_b = gui.about.slot_badge();
    let badge_w = g.measure(1, 11, slot_b) + 24;
    draw_candy_badge(buf, w, h, mid_x - badge_w / 2, y + 132, badge_w, 18, pal.accent, pal);
    draw_sans_bold(buf, w, h, g, mid_x - badge_w / 2 + 12, y + 134, badge_w, 11, slot_b, pal.text_on_accent);
    y += hero_h + 12;

    // System Hardware & Health Specification Card.
    let a = &gui.about;
    let spec_h = 224;
    draw_noir_card(buf, w, h, x, y, cw, spec_h, 10, pal);
    let mut row_y = y + 4;
    let draw_spec = |buf: &mut [u32], g: &mut Glyphs, ry: i32, k: &[u8], v: &[u8], div: bool| {
        if div {
            fill_solid(buf, w, h, x + 14, ry, cw - 28, 1, pal.hairline);
        }
        draw_sans_bold(buf, w, h, g, x + 16, ry + 10, 150, 12, k, pal.text_dim);
        draw_sans(buf, w, h, g, x + 172, ry + 10, cw - 188, v, pal.text);
    };

    // 1. Processor
    let mut cpu_buf = [0u8; 48];
    let cpu_len = if let Some(c) = a.kernel.cores {
        let mut n = format_u64_ascii(c, &mut cpu_buf);
        let suf = b" Cores (x86_64 SASOS)";
        let t = suf.len().min(cpu_buf.len() - n);
        cpu_buf[n..n + t].copy_from_slice(&suf[..t]);
        n += t;
        n
    } else {
        let s = b"x86_64 Microkernel";
        cpu_buf[..s.len()].copy_from_slice(s);
        s.len()
    };
    draw_spec(buf, g, row_y, b"Processor", &cpu_buf[..cpu_len], false);
    row_y += 36;

    // 2. Memory
    let mut mem_buf = [0u8; 64];
    let mem_len = if let Some(lim) = a.kernel.program_limit {
        let mut n = format_u64_ascii(lim / 256, &mut mem_buf);
        push_bytes(&mut mem_buf, &mut n, b" MiB pool");
        if let Some(lent) = a.kernel.program_lent {
            push_bytes(&mut mem_buf, &mut n, b" (");
            n += format_u64_ascii(lent / 256, &mut mem_buf[n..]);
            push_bytes(&mut mem_buf, &mut n, b" MiB in use");
            if let Some(free) = a.kernel.free_frames {
                push_bytes(&mut mem_buf, &mut n, b", ");
                n += format_u64_ascii(free / 256, &mut mem_buf[n..]);
                push_bytes(&mut mem_buf, &mut n, b" MiB free)");
            } else {
                push_bytes(&mut mem_buf, &mut n, b")");
            }
        }
        n
    } else {
        let s = b"Shared protection pool";
        mem_buf[..s.len()].copy_from_slice(s);
        s.len()
    };
    draw_spec(buf, g, row_y, b"Memory", &mem_buf[..mem_len], true);
    row_y += 36;

    // 3. Protection Domains & Kernel Heap
    let mut dom_buf = [0u8; 64];
    let mut dn = format_u64_ascii(a.kernel.domains as u64, &mut dom_buf);
    push_bytes(&mut dom_buf, &mut dn, b" active domains");
    if let Some(kh) = a.kernel.kernel_heap {
        push_bytes(&mut dom_buf, &mut dn, b"  \xE2\x80\xA2  Heap ");
        dn += format_u64_ascii(kh / 1024, &mut dom_buf[dn..]);
        push_bytes(&mut dom_buf, &mut dn, b" KiB");
    }
    draw_spec(buf, g, row_y, b"Protection Domains", &dom_buf[..dn], true);
    row_y += 36;

    // 4. Filesystem Health
    let fs_slice = if a.fs_health_len > 0 {
        &a.fs_health[..a.fs_health_len]
    } else {
        b"Verified via Files"
    };
    draw_spec(buf, g, row_y, b"Filesystem Health", fs_slice, true);
    row_y += 36;

    // 5. Slot A/B Version & Image SHA-256
    let mut ver_buf = [0u8; 72];
    let mut vn = 0usize;
    push_bytes(&mut ver_buf, &mut vn, a.slot_badge());
    if a.version_len > 0 {
        push_bytes(&mut ver_buf, &mut vn, b" v");
        push_bytes(&mut ver_buf, &mut vn, &a.version[..a.version_len]);
    }
    if a.image_hash_len > 0 {
        push_bytes(&mut ver_buf, &mut vn, b"  \xE2\x80\xA2  ");
        let show_len = if a.image_hash[..a.image_hash_len].starts_with(b"hashing") {
            a.image_hash_len
        } else {
            a.image_hash_len.min(16)
        };
        push_bytes(&mut ver_buf, &mut vn, &a.image_hash[..show_len]);
    }
    draw_spec(buf, g, row_y, b"System Version", &ver_buf[..vn], true);
    row_y += 36;

    // 6. Uptime
    let mut up_buf = [0u8; 40];
    let mut un = format_u64_ascii(a.kernel.uptime_s / 3600, &mut up_buf);
    push_bytes(&mut up_buf, &mut un, b" h ");
    un += format_u64_ascii((a.kernel.uptime_s / 60) % 60, &mut up_buf[un..]);
    push_bytes(&mut up_buf, &mut un, b" min");
    draw_spec(buf, g, row_y, b"Uptime", &up_buf[..un], true);
    y += spec_h + 12;
    y - start_y
}

fn push_bytes(dst: &mut [u8], pos: &mut usize, src: &[u8]) {
    let take = src.len().min(dst.len().saturating_sub(*pos));
    dst[*pos..*pos + take].copy_from_slice(&src[..take]);
    *pos += take;
}

#[allow(clippy::too_many_arguments)]
fn render_search_results(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    top: i32,
    bot: i32,
) -> i32 {
    let start_y = y;
    let (rows, count) = m.rows();
    if count == 0 {
        draw_noir_card(buf, w, h, x, y, cw, 74, 10, pal);
        draw_sans_bold(buf, w, h, g, x + 16, y + 14, cw - 32, 14, b"No matching settings", pal.text);
        draw_sans_sm(buf, w, h, g, x + 16, y + 38, cw - 32, b"Try searching 'theme', 'volume', 'delay', 'zone', or press Esc to clear.", pal.text_dim);
        return 86;
    }
    for (idx, row) in rows[..count].iter().enumerate() {
        let rh = 42;
        if y + rh >= top && y <= bot {
            render_model_row_card(buf, w, h, g, gui, m, pal, x, y, cw, idx, *row);
        }
        y += rh + 6;
    }
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_system_page(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    mut y: i32,
    cw: i32,
    top: i32,
    bot: i32,
) -> i32 {
    let start_y = y;
    let (rows, count) = m.rows();
    for (idx, row) in rows[..count].iter().enumerate() {
        let rh = 42;
        if y + rh >= top && y <= bot {
            render_model_row_card(buf, w, h, g, gui, m, pal, x, y, cw, idx, *row);
        }
        y += rh + 6;
    }

    let info_slice: &[u8] = if m.info_len > 0 {
        &m.info[..m.info_len]
    } else if m.extra_len > 0 {
        &m.extra[..m.extra_len]
    } else {
        b""
    };
    if !info_slice.is_empty() {
        let line_count = info_slice.split(|&b| b == b'\n').filter(|l| !l.is_empty()).count() as i32;
        let card_h = (28 + line_count * 18).max(52);
        draw_noir_card(buf, w, h, x, y, cw, card_h, 10, pal);
        let mut ly = y + 10;
        for line in info_slice.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            if ly + 16 > y + card_h - 4 { break; }
            draw_sans_sm(buf, w, h, g, x + 14, ly, cw - 28, line, pal.text_dim);
            ly += 18;
        }
        y += card_h + 10;
    }
    y - start_y
}

#[allow(clippy::too_many_arguments)]
fn render_model_row_card(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    m: &Model,
    pal: &CandyPalette,
    x: i32,
    y: i32,
    cw: i32,
    idx: usize,
    row: model::Row,
) {
    let k: &Key = &KEYS[row.key];
    let selected = m.row == idx;
    draw_noir_card(buf, w, h, x, y, cw, 40, 8, pal);
    if selected {
        fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, 38, 7, pal.selection);
        fill_rounded_rect(buf, w, h, x + 2, y + 6, 3, 28, 2, pal.accent);
    }
    let label = match (k.kind, row.item) {
        (Kind::Set(names), Some(item)) => names.get(item as usize).copied().unwrap_or(k.label),
        _ => k.label,
    };
    draw_sans_sm(buf, w, h, g, x + 14, y + 5, 110, model::page_title(k.section()).as_bytes(), pal.accent);
    draw_sans(buf, w, h, g, x + 14, y + 19, (cw - 170).max(80), label.as_bytes(), pal.text);
    gui.push_hit(x, y, cw - 130, 40, UiAction::SelectRow(idx));

    let val = m.values.value(k.id);
    match (k.kind, row.item) {
        (Kind::Bool, _) => {
            let act = UiAction::StepRow(idx, true);
            draw_noir_toggle(buf, w, h, x + cw - 56, y + 9, 42, 22, val != 0, gui.btn_state(act), pal);
            gui.push_hit(x + cw - 62, y + 4, 54, 32, act);
        }
        (Kind::Set(_), Some(item)) => {
            let on = (val & (1 << item)) != 0;
            let act = UiAction::StepRow(idx, !on);
            draw_noir_toggle(buf, w, h, x + cw - 56, y + 9, 42, 22, on, gui.btn_state(act), pal);
            gui.push_hit(x + cw - 62, y + 4, 54, 32, act);
        }
        (Kind::Color, _) => {
            let act = UiAction::StepRow(idx, true);
            draw_candy_swatch(buf, w, h, x + cw - 40, y + 9, 22, (val as u32) & 0x00FF_FFFF, selected, false, pal);
            gui.push_hit(x + cw - 46, y + 4, 36, 32, act);
        }
        _ => {
            let dec = UiAction::StepRow(idx, false);
            let inc = UiAction::StepRow(idx, true);
            draw_candy_button(buf, w, h, x + cw - 126, y + 9, 24, 22, 6, ButtonStyle::Normal, gui.btn_state(dec), pal);
            draw_sans_bold(buf, w, h, g, x + cw - 118, y + 11, 14, 12, b"<", pal.text);
            gui.push_hit(x + cw - 126, y + 7, 24, 26, dec);

            let mut vbuf = [0u8; 16];
            let vlen = format_short_value(k.kind, val, row.item, &mut vbuf);
            let vw = g.measure(0, 12, &vbuf[..vlen]).min(60);
            draw_sans(buf, w, h, g, x + cw - 98 + (60 - vw) / 2, y + 11, 60, &vbuf[..vlen], pal.accent);

            draw_candy_button(buf, w, h, x + cw - 36, y + 9, 24, 22, 6, ButtonStyle::Normal, gui.btn_state(inc), pal);
            draw_sans_bold(buf, w, h, g, x + cw - 28, y + 11, 14, 12, b">", pal.text);
            gui.push_hit(x + cw - 36, y + 7, 24, 26, inc);
        }
    }
}

fn render_password_modal(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    gui: &mut GuiState,
    entry: &model::Entry,
    m: &Model,
    pal: &CandyPalette,
) {
    let mw = (w as i32 - 80).clamp(280, 420);
    let mh = 180;
    let mx = (w as i32 - mw) / 2;
    let my = (h as i32 - mh) / 2;
    fill_rounded_rect(buf, w, h, mx - 2, my - 2, mw + 4, mh + 4, 14, pal.accent);
    draw_noir_card(buf, w, h, mx, my, mw, mh, 12, pal);
    draw_sidebar_badge(buf, w, h, g, mx + 18, my + 16, pal.accent, Icon::Privacy);
    draw_sans_bold(buf, w, h, g, mx + 48, my + 18, mw - 66, 15, b"Change Lock Password", pal.text);
    let prompt: &[u8] = if entry.confirming { b"Confirm your new lock password:" } else { b"Enter a new lock password:" };
    draw_sans_sm(buf, w, h, g, mx + 18, my + 50, mw - 36, prompt, pal.text_dim);

    draw_noir_well(buf, w, h, mx + 18, my + 74, mw - 36, 32, 8, pal.accent, pal);
    let mut stars = [b'*'; 32];
    let slen = entry.len.min(stars.len());
    if slen > 0 {
        draw_mono(buf, w, h, g, mx + 28, my + 82, mw - 56, &stars[..slen], pal.text);
    } else {
        stars[0] = b'_';
        draw_mono(buf, w, h, g, mx + 28, my + 82, mw - 56, &stars[..1], pal.text_dim);
    }

    if m.status_len > 0 {
        draw_sans_sm(buf, w, h, g, mx + 18, my + 114, mw - 36, &m.status[..m.status_len], pal.accent);
    } else {
        draw_sans_sm(buf, w, h, g, mx + 18, my + 114, mw - 36, b"Press Enter to continue, or Esc to cancel.", pal.text_dim);
    }

    let cancel_act = UiAction::CancelPassword;
    draw_candy_button(buf, w, h, mx + mw - 98, my + mh - 38, 80, 26, 7, ButtonStyle::Normal, gui.btn_state(cancel_act), pal);
    draw_sans_sm(buf, w, h, g, mx + mw - 78, my + mh - 32, 60, b"Cancel", pal.text);
    gui.push_hit(mx + mw - 98, my + mh - 38, 80, 26, cancel_act);
}

/// Renders the First-Start macOS Noir Setup Assistant (`setup::Setup`).
pub fn render_setup_gui(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, s: &Setup, status: &[u8]) {
    if w < 200 || h < 200 || buf.len() < w * h {
        return;
    }
    let pal = noir_palette(Mode::Dark, 0x00F4_3F5E);
    let (wi, hi) = (w as i32, h as i32);
    fill_solid(buf, w, h, 0, 0, wi, hi, pal.bg);

    let cw = (wi - 64).clamp(320, 560);
    let ch = (hi - 64).clamp(280, 440);
    let cx = (wi - cw) / 2;
    let cy = (hi - ch) / 2;
    draw_noir_card(buf, w, h, cx, cy, cw, ch, 14, &pal);

    // Lips Emblem + Title.
    draw_sexos_lips(buf, w, h, cx + cw / 2, cy + 36, 72, false);
    let heading = b"Welcome to SexOS";
    let hw = g.measure(1, 18, heading);
    draw_sans_bold(buf, w, h, g, cx + (cw - hw) / 2, cy + 64, cw - 32, 18, heading, pal.text);

    // 4 step dots.
    let step_idx = match s.step { Step::Layout => 0, Step::Zone => 1, Step::Password => 2, Step::Idle => 3, Step::Done => 4 };
    let dots_x = cx + (cw - 4 * 18) / 2;
    for i in 0..4 {
        fill_rounded_rect(buf, w, h, dots_x + i * 18, cy + 92, 10, 6, 3, if i as usize == step_idx { pal.accent } else { pal.control });
    }

    let step_title: &[u8] = match s.step {
        Step::Layout => b"Step 1 of 4: Choose Keyboard Layout",
        Step::Zone => b"Step 2 of 4: Select Time Zone (type to filter)",
        Step::Password => b"Step 3 of 4: Set Lock Password (optional)",
        Step::Idle => b"Step 4 of 4: Idle & Screen Lock Behavior",
        Step::Done => b"Setup Complete",
    };
    draw_sans_bold(buf, w, h, g, cx + 24, cy + 114, cw - 48, 13, step_title, pal.text);

    let mut ry = cy + 142;
    match s.step {
        Step::Layout => {
            for (i, l) in LAYOUTS.iter().enumerate() {
                let sel = i == s.layout;
                fill_rounded_rect(buf, w, h, cx + 24, ry, cw - 48, 32, 7, if sel { pal.selection } else { pal.well });
                draw_sans(buf, w, h, g, cx + 36, ry + 8, cw - 72, l.as_bytes(), if sel { pal.text } else { pal.text_dim });
                ry += 38;
            }
        }
        Step::Zone => {
            draw_noir_well(buf, w, h, cx + 24, ry, cw - 48, 30, 8, pal.accent, &pal);
            if s.filter_len > 0 {
                draw_sans(buf, w, h, g, cx + 34, ry + 7, cw - 68, &s.filter[..s.filter_len], pal.text);
            } else {
                draw_sans(buf, w, h, g, cx + 34, ry + 7, cw - 68, b"Type city or region...", pal.text_dim);
            }
            ry += 38;
            let mut list = [0usize; 64];
            let mut n = 0;
            for z in s.zones().take(64) { list[n] = z; n += 1; }
            if n == 0 {
                draw_sans(buf, w, h, g, cx + 30, ry + 6, cw - 60, b"Nothing matches.", pal.text_dim);
            }
            let at_sel = list[..n].iter().position(|z| *z == s.zone).unwrap_or(0);
            let first = at_sel.saturating_sub(2).min(n.saturating_sub(6));
            for &z in &list[first..(first + 6).min(n)] {
                let sel = z == s.zone;
                fill_rounded_rect(buf, w, h, cx + 24, ry, cw - 48, 26, 6, if sel { pal.selection } else { pal.well });
                draw_sans_sm(buf, w, h, g, cx + 34, ry + 6, cw - 68, ZONES[z].as_bytes(), if sel { pal.text } else { pal.text_dim });
                ry += 29;
            }
        }
        Step::Password => {
            draw_noir_well(buf, w, h, cx + 24, ry, cw - 48, 34, 8, pal.accent, &pal);
            let stars = [b'*'; 32];
            let sl = s.password_len.min(32);
            if sl > 0 {
                draw_mono(buf, w, h, g, cx + 34, ry + 9, cw - 68, &stars[..sl], pal.text);
            } else {
                draw_sans(buf, w, h, g, cx + 34, ry + 9, cw - 68, b"Leave empty for no lock password", pal.text_dim);
            }
        }
        Step::Idle => {
            for (i, (label, ..)) in setup::IDLE.iter().enumerate() {
                let sel = i == s.idle;
                fill_rounded_rect(buf, w, h, cx + 24, ry, cw - 48, 34, 7, if sel { pal.selection } else { pal.well });
                draw_sans_sm(buf, w, h, g, cx + 34, ry + 10, cw - 68, label.as_bytes(), if sel { pal.text } else { pal.text_dim });
                ry += 40;
            }
        }
        Step::Done => {
            draw_sans(buf, w, h, g, cx + 24, ry + 8, cw - 48, b"All set. Opening Tatami Settings...", pal.text);
        }
    }

    if !status.is_empty() {
        draw_sans_sm(buf, w, h, g, cx + 24, cy + ch - 52, cw - 48, status, pal.accent);
    }
    draw_sans_sm(buf, w, h, g, cx + 24, cy + ch - 30, cw - 48, b"Up/Down: Choose   \xE2\x80\xA2   Enter: Continue   \xE2\x80\xA2   Esc: Back", pal.text_dim);
}

fn format_i32_ascii(mut v: i32, out: &mut [u8]) -> usize {
    let mut n = 0usize;
    if v < 0 && n < out.len() {
        out[n] = b'-';
        n += 1;
        v = v.saturating_neg();
    }
    let mut tmp = [0u8; 12];
    let mut k = 0;
    loop {
        tmp[k] = b'0' + (v % 10) as u8;
        k += 1;
        v /= 10;
        if v == 0 { break; }
    }
    while k > 0 && n < out.len() {
        k -= 1;
        out[n] = tmp[k];
        n += 1;
    }
    n
}

fn format_short_value(kind: Kind, value: i32, item: Option<u8>, out: &mut [u8; 16]) -> usize {
    match kind {
        Kind::Choice(names) => {
            let s = names.get(value as usize).copied().unwrap_or("?").as_bytes();
            let n = s.len().min(out.len());
            out[..n].copy_from_slice(&s[..n]);
            n
        }
        Kind::Set(_) => {
            let on = item.is_some_and(|i| (value & (1 << i)) != 0);
            let s: &[u8] = if on { b"Yes" } else { b"No" };
            out[..s.len()].copy_from_slice(s);
            s.len()
        }
        _ => format_i32_ascii(value, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noir_palette_uses_locked_colors_and_accent_tints() {
        let mut cfg = Config::new();
        let gui = GuiState::new();
        let p = gui.palette(&cfg);
        assert_eq!(p.panel, NOIR_SIDEBAR);
        assert_eq!(p.bg, NOIR_CANVAS);
        assert_eq!(p.card, NOIR_CARD);
        assert_eq!(p.specular, NOIR_TOP_SPECULAR);
        assert_eq!(p.accent, 0x00F4_3F5E);

        cfg.set(id::ACCENT, 0x0038_BDF8).unwrap();
        let cyan = gui.palette(&cfg);
        assert_eq!(cyan.accent, 0x0038_BDF8);
    }

    #[test]
    fn appearance_and_all_eleven_categories_render_with_hitboxes() {
        let cfg = Config::new();
        let m = Model::new(cfg);
        let mut gui = GuiState::new();
        let mut g = Glyphs::new();
        let mut buf = vec![0u32; 920 * 640];

        render_gui(&mut buf, 920, 640, &mut g, &mut gui, &m);
        for item in &SIDEBAR_ITEMS {
            let has = gui.hits[..gui.hit_count]
                .iter()
                .flatten()
                .any(|h| h.action == UiAction::SelectView(item.view));
            assert!(has, "missing sidebar hitbox for {}", item.label);
        }
        for &(_name, rgb) in &ACCENT_TINTS {
            let has = gui.hits[..gui.hit_count]
                .iter()
                .flatten()
                .any(|h| h.action == UiAction::SetAccent(rgb));
            assert!(has, "missing accent tint hitbox {rgb:#x}");
        }
        assert!(gui.hits[..gui.hit_count].iter().flatten().any(|h| h.action == UiAction::FocusSearch));

        // Render all 11 sidebar views plus search, password modal, and setup assistant cleanly.
        for item in &SIDEBAR_ITEMS {
            gui.view = item.view;
            gui.scroll_y = 0;
            render_gui(&mut buf, 920, 640, &mut g, &mut gui, &m);
            assert!(gui.hit_count >= SIDEBAR_ITEMS.len());
        }
        let mut s = Setup::new();
        for _ in 0..4 {
            render_setup_gui(&mut buf, 920, 640, &mut g, &s, b"");
            s.next();
        }
        render_setup_gui(&mut buf, 920, 640, &mut g, &s, b"Done");
    }

    #[test]
    fn install_gui_locks_esp_msr_tiny_and_registers_actions() {
        let cfg = Config::new();
        let m = Model::new(cfg);
        let mut gui = GuiState::new();
        gui.view = SidebarView::Install;
        let raw = concat!(
            "DISK 1000215216\n",
            "PART 1 esp 2048 532480 0 EFI system partition\n",
            "PART 2 msr 534528 32768 0 Microsoft reserved\n",
            "PART 3 win 567296 209715200 1 Windows\n",
            "PART 6 lin 419997696 131072 0 Tiny Boot\n",
            "FREE 420128768 50000000\n",
        );
        gui.install.parse_raw(raw.as_bytes());
        assert!(!gui.install.select_part(0));
        assert!(!gui.install.select_part(1));
        assert!(!gui.install.select_part(3));
        assert!(gui.install.select_part(2));

        let mut g = Glyphs::new();
        let mut buf = vec![0u32; 920 * 640];
        render_gui(&mut buf, 920, 640, &mut g, &mut gui, &m);
        let has = |act: UiAction| gui.hits[..gui.hit_count].iter().flatten().any(|h| h.action == act);
        assert!(!has(UiAction::SelectInstallPart(0)));
        assert!(!has(UiAction::SelectInstallPart(1)));
        assert!(!has(UiAction::SelectInstallPart(3)));
        assert!(has(UiAction::SelectInstallPart(2)));
        assert!(has(UiAction::InstallPlan));
        assert!(has(UiAction::InstallExecute));
    }

    #[test]
    fn about_hero_renders_facts_and_lips() {
        let cfg = Config::new();
        let m = Model::new(cfg);
        let mut gui = GuiState::new();
        gui.view = SidebarView::About;
        gui.about.update_about(
            KernelFacts {
                cores: Some(8),
                program_limit: Some(1024 * 256),
                program_lent: Some(64 * 256),
                free_frames: Some(512 * 256),
                kernel_heap: Some(4 << 20),
                domains: 27,
                uptime_s: 3665,
            },
            b"slot=a\nversion=12\nimage_bytes=33554432\nimage=deadbeef01234567\n",
        );
        gui.about.update_storage(b"disk=1000000 store\nfilesystem=sexfs 262144 131072\n");
        let mut g = Glyphs::new();
        let mut buf = vec![0u32; 920 * 640];
        render_gui(&mut buf, 920, 640, &mut g, &mut gui, &m);
        assert_eq!(gui.about.slot_badge(), b"Slot A");
    }

    #[test]
    fn wifi_gui_renders_status_known_networks_scan_and_join_form() {
        let cfg = Config::new();
        let m = Model::new(cfg);
        let mut gui = GuiState::new();
        gui.view = SidebarView::Wifi;

        // Parse multi-network /data/wifi.conf format shared with Spindle.
        gui.wifi.parse_wifi_conf(b"ssid=HomeNet\tpass=test-pass-123\nssid=CafeSpot\tpass=\n");
        assert_eq!(gui.wifi.known_count, 2);
        assert_eq!(gui.wifi.known[0].ssid_bytes(), b"HomeNet");
        assert_eq!(gui.wifi.known[1].ssid_bytes(), b"CafeSpot");

        // Upsert moves/adds to MRU top and roundtrips cleanly.
        assert!(gui.wifi.upsert_known(b"Lab 5G", b"lab-pass-456"));
        assert_eq!(gui.wifi.known_count, 3);
        assert_eq!(gui.wifi.known[0].ssid_bytes(), b"Lab 5G");
        let mut ser = [0u8; WIFI_CONF_MAX];
        let slen = gui.wifi.serialize_wifi_conf(&mut ser);
        assert_eq!(
            &ser[..slen],
            b"ssid=Lab 5G\tpass=lab-pass-456\nssid=HomeNet\tpass=test-pass-123\nssid=CafeSpot\tpass=\n"
        );

        // Simulate connected state + scanned network.
        gui.wifi.connected = true;
        gui.wifi.set_active_ssid(b"Lab 5G");
        gui.wifi.set_ip(b"192.0.2.15");
        gui.wifi.signal_rssi = -52;
        gui.wifi.scanned[0] = WifiScanEntry {
            scan_idx: 0,
            ssid: {
                let mut s = [0u8; 32];
                s[..6].copy_from_slice(b"Lab 5G");
                s
            },
            ssid_len: 6,
            rssi: -52,
            channel: 36,
            security: 2,
        };
        gui.wifi.scan_count = 1;

        let mut g = Glyphs::new();
        let mut buf = vec![0u32; 900 * 600];
        render_gui(&mut buf, 900, 600, &mut g, &mut gui, &m);

        let has = |act: UiAction| gui.hits[..gui.hit_count].iter().flatten().any(|h| h.action == act);
        assert!(has(UiAction::WifiScan), "missing Scan Wi-Fi hitbox");
        assert!(has(UiAction::WifiRefresh), "missing Refresh hitbox");
        assert!(has(UiAction::WifiConnectKnown(0)), "missing Connect known hitbox");
        assert!(has(UiAction::WifiForgetKnown(0)), "missing Forget known hitbox");
        assert!(has(UiAction::WifiSelectScan(0)), "missing Scanned network hitbox");
        assert!(has(UiAction::WifiFocusSsid), "missing SSID input hitbox");
        assert!(has(UiAction::WifiFocusPass), "missing Password input hitbox");
        assert!(has(UiAction::WifiJoinSubmit), "missing Save & Join hitbox");

        // Forget removes entry cleanly.
        assert!(gui.wifi.forget_known(1));
        assert_eq!(gui.wifi.known_count, 2);
        assert_eq!(gui.wifi.known[1].ssid_bytes(), b"CafeSpot");
    }
}
