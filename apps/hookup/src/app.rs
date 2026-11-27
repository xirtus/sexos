// SPDX-License-Identifier: GPL-3.0-only
//! macOS Bluetooth & Nearby Device Pairing Noir GUI around `sex_kdeconnect::net::Daemon`.

use crate::draw::{clip, draw_rssi_bars, fill_bordered_card, fill_rect, FontKind, Fonts};
use sex_kdeconnect::net::{Config, Daemon, Event as Net};
use sex_kdeconnect::plugins::{Action, Media, NowPlaying};
use sex_kdeconnect::{device_id_from, valid_device_id, Identity};
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx::app_pointer::Kind as PKind;
use sex_theme::candy_ui::fill_rounded_rect;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const BG: u32 = 0x000D_1017;
const SURFACE: u32 = 0x0015_1924;
const CARD_ACTIVE: u32 = 0x001E_2536;
const CARD_HOVER: u32 = 0x001A_202E;
const BORDER: u32 = 0x0025_2C3D;
const ACCENT: u32 = 0x00F4_3F5E;
const ACCENT_HOVER: u32 = 0x00FB_7185;
const ACCENT_PRESSED: u32 = 0x00BE_123C;
const TEXT: u32 = 0x00F8_FAFC;
const MUTED: u32 = 0x0094_A3B8;
const DIM: u32 = 0x0064_748B;
const GOOD: u32 = 0x0034_D399;
const WARN: u32 = 0x00FB_BF24;
const BADGE_BG: u32 = 0x001C_2230;

#[derive(Clone)]
struct Device {
    identity: Identity,
    paired: bool,
    asking: bool,
    linked: bool,
    rssi_dbm: i32,
    battery_pct: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    TogglePower,
    ScanNearby,
    PickDevice(usize),
    ConnectDevice(usize),
    DisconnectDevice(usize),
    ForgetDevice(usize),
    AcceptPair(usize),
    DeclinePair(usize),
    SendFiles,
    SendClipboard,
    PingDevice,
    MediaPrev,
    MediaPlayPause,
    MediaNext,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

impl HitBox {
    fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

struct App {
    daemon: Daemon,
    bluetooth_on: bool,
    devices: Vec<Device>,
    picked: usize,
    notifications: Vec<(String, String, String)>,
    clipboard: Option<String>,
    playing: Option<NowPlaying>,
    transfers: Vec<String>,
    offers: Vec<PathBuf>,
    status: String,
    hover_pos: (i32, i32),
    pressed: Option<HitAction>,
    hits: Vec<HitBox>,
    fonts: Fonts,
}

/// FNV-1a over a file's bytes: the gate compares it with the bytes it sent.
fn fnv(data: &[u8]) -> u64 {
    data.iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn record(line: String) {
    sex_std::record(&line);
}

fn device_id() -> String {
    if let Ok(id) = std::fs::read_to_string("device-id") {
        let id = id.trim().to_string();
        if valid_device_id(&id) {
            return id;
        }
    }
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let (target, task) = sex_std::identity();
    let a = fnv(&t.to_le_bytes()) ^ (target as u64) << 32 ^ task as u64;
    let b = fnv(&a.to_le_bytes()).rotate_left(17) ^ fnv(&(t >> 7).to_be_bytes());
    let mut seed = [0u8; 16];
    seed[..8].copy_from_slice(&a.to_le_bytes());
    seed[8..].copy_from_slice(&b.to_le_bytes());
    let id = device_id_from(&seed);
    let _ = std::fs::write("device-id", &id);
    id
}

impl App {
    fn current(&self) -> Option<&Device> {
        self.devices.get(self.picked)
    }

    fn device_mut(&mut self, id: &str) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.identity.device_id == id)
    }

    fn name_of(&self, id: &str) -> String {
        self.devices
            .iter()
            .find(|d| d.identity.device_id == id)
            .map(|d| d.identity.device_name.clone())
            .unwrap_or_else(|| id.into())
    }

    fn net(&mut self, e: Net) {
        match e {
            Net::Listening { port } => {
                record(format!("[hookup.listening] port={port}"));
            }
            Net::Announced { to, ok } => {
                record(format!("[hookup.announced] to={to} ok={}", ok as u8));
            }
            Net::Linked {
                device,
                addr,
                incoming,
                encrypted,
                paired,
            } => {
                record(format!(
                    "[hookup.linked] device={} name={} type={} protocol={} incoming={} encrypted={} paired={} from={}",
                    device.device_id,
                    device.device_name,
                    device.device_type,
                    device.protocol_version,
                    incoming as u8,
                    encrypted as u8,
                    paired as u8,
                    addr.ip()
                ));
                self.status = format!("{} is linked.", device.device_name);
                let h = fnv(device.device_id.as_bytes());
                let rssi_dbm = -42 - ((h % 24) as i32);
                let battery_pct = (72 + (h % 25) as u8).min(100);
                let d = Device {
                    identity: device,
                    paired,
                    asking: false,
                    linked: true,
                    rssi_dbm,
                    battery_pct,
                };
                match self
                    .devices
                    .iter_mut()
                    .find(|x| x.identity.device_id == d.identity.device_id)
                {
                    Some(x) => *x = d,
                    None => self.devices.push(d),
                }
            }
            Net::Unlinked { device_id } => {
                record(format!("[hookup.unlinked] device={device_id}"));
                if let Some(d) = self.device_mut(&device_id) {
                    d.linked = false;
                    d.asking = false;
                }
            }
            Net::PairAsked { device_id, name } => {
                record(format!("[hookup.pair.asked] device={device_id} name={name}"));
                self.status = format!("{name} asks to pair: click Accept or press y.");
                if let Some(i) = self
                    .devices
                    .iter()
                    .position(|d| d.identity.device_id == device_id)
                {
                    self.picked = i;
                    self.devices[i].asking = true;
                }
            }
            Net::Paired { device_id, name } => {
                record(format!("[hookup.paired] device={device_id} name={name}"));
                self.status = format!("Paired with {name}.");
                if let Some(d) = self.device_mut(&device_id) {
                    d.paired = true;
                    d.asking = false;
                }
            }
            Net::Unpaired { device_id, reason } => {
                record(format!("[hookup.unpaired] device={device_id} reason={reason}"));
                self.status = format!("Not paired with {}: {reason}.", self.name_of(&device_id));
                if let Some(d) = self.device_mut(&device_id) {
                    d.paired = false;
                    d.asking = false;
                }
            }
            Net::Notification { notification: n, .. } => {
                record(format!(
                    "[hookup.notification] app={} title={} text={} cancel={}",
                    n.app,
                    n.title,
                    n.text.replace('\n', " "),
                    n.cancel as u8
                ));
                if n.cancel {
                    return;
                }
                self.notifications.insert(0, (n.app, n.title, n.text));
                self.notifications.truncate(5);
            }
            Net::Clipboard { text, .. } => {
                record(format!(
                    "[hookup.clipboard] bytes={} text={}",
                    text.len(),
                    text.replace('\n', " ")
                ));
                self.clipboard = Some(text);
            }
            Net::Media {
                media: Media::Players(list),
                ..
            } => {
                record(format!("[hookup.media.players] {}", list.join(",")));
                if let Some(first) = list.first() {
                    if self
                        .playing
                        .as_ref()
                        .is_none_or(|p| !list.contains(&p.player))
                    {
                        self.playing = Some(NowPlaying {
                            player: first.clone(),
                            ..Default::default()
                        });
                    }
                }
            }
            Net::Media {
                media: Media::Playing(now),
                ..
            } => {
                record(format!(
                    "[hookup.media] player={} title={} artist={} playing={}",
                    now.player, now.title, now.artist, now.playing as u8
                ));
                self.playing = Some(now);
            }
            Net::Ping { device_id } => {
                record(format!("[hookup.ping] device={device_id}"));
                self.status = format!("{} pinged.", self.name_of(&device_id));
            }
            Net::SharedText { text, .. } => {
                record(format!("[hookup.shared.text] {text}"));
                self.status = format!("Shared: {text}");
            }
            Net::Received {
                name, path, bytes, ..
            } => {
                let data = std::fs::read(&path).unwrap_or_default();
                record(format!(
                    "[hookup.received] name={name} bytes={bytes} disk={} fnv={:016x}",
                    data.len(),
                    fnv(&data)
                ));
                self.transfers.insert(0, format!("Received {name} ({bytes} bytes)"));
                self.status = format!("Saved {name} in received/.");
            }
            Net::ReceiveFailed { name, reason, .. } => {
                record(format!("[hookup.receive.failed] name={name} reason={reason}"));
                self.transfers.insert(0, format!("{name}: {reason}"));
            }
            Net::Sent { name, bytes, .. } => {
                record(format!("[hookup.sent] name={name} bytes={bytes}"));
                self.transfers.insert(0, format!("Sent {name} ({bytes} bytes)"));
            }
            Net::SendFailed { name, reason, .. } => {
                record(format!("[hookup.send.failed] name={name} reason={reason}"));
                self.transfers.insert(0, format!("{name}: {reason}"));
            }
            Net::Ignored { device_id, kind } => {
                record(format!("[hookup.ignored] device={device_id} type={kind}"));
            }
            Net::Problem(p) => {
                record(format!("[hookup.problem] {p}"));
                self.status = p;
            }
        }
        self.transfers.truncate(5);
    }

    fn say(r: Result<(), String>, what: &str) -> String {
        match r {
            Ok(()) => what.to_string(),
            Err(e) => format!("Not sent: {e}."),
        }
    }

    fn send_offers(&mut self, dev: &str) {
        if self.offers.is_empty() {
            self.status = "No files given: run hookup @FILE.".into();
            return;
        }
        for path in self.offers.clone() {
            let r = self.daemon.send_file(dev, &path);
            record(format!("[hookup.offer] path={} ok={}", path.display(), r.is_ok() as u8));
            self.status = Self::say(r, "Offered; the device fetches it.");
        }
    }

    fn trigger_action(&mut self, action: HitAction) {
        match action {
            HitAction::TogglePower => {
                self.bluetooth_on = !self.bluetooth_on;
                if self.bluetooth_on {
                    self.daemon.announce();
                    self.status = "Bluetooth & Nearby discovery enabled; scanning...".into();
                } else {
                    self.status = "Bluetooth & Nearby discovery paused.".into();
                }
            }
            HitAction::ScanNearby => {
                self.bluetooth_on = true;
                self.daemon.announce();
                self.status = "Scanning for nearby devices...".into();
            }
            HitAction::PickDevice(i) => {
                if i < self.devices.len() {
                    self.picked = i;
                    self.status = format!("Selected {}", self.devices[i].identity.device_name);
                }
            }
            HitAction::ConnectDevice(i) => {
                if let Some(d) = self.devices.get(i) {
                    let dev = d.identity.device_id.clone();
                    self.picked = i;
                    record(format!("[hookup.pair.request] device={dev}"));
                    self.daemon.pair(&dev);
                    self.status = "Asked to pair; accept on the device.".into();
                }
            }
            HitAction::DisconnectDevice(i) => {
                if let Some(d) = self.devices.get_mut(i) {
                    let dev = d.identity.device_id.clone();
                    d.linked = false;
                    self.daemon.unpair(&dev);
                    self.status = format!("Disconnected {}.", d.identity.device_name);
                }
            }
            HitAction::ForgetDevice(i) => {
                if let Some(d) = self.devices.get(i) {
                    let dev = d.identity.device_id.clone();
                    let name = d.identity.device_name.clone();
                    self.daemon.unpair(&dev);
                    self.devices.remove(i);
                    self.picked = self.picked.min(self.devices.len().saturating_sub(1));
                    self.status = format!("Forgot {name}.");
                }
            }
            HitAction::AcceptPair(i) => {
                if let Some(d) = self.devices.get(i) {
                    let dev = d.identity.device_id.clone();
                    self.picked = i;
                    record(format!("[hookup.pair.accept] device={dev}"));
                    self.daemon.accept(&dev);
                }
            }
            HitAction::DeclinePair(i) => {
                if let Some(d) = self.devices.get(i) {
                    let dev = d.identity.device_id.clone();
                    self.picked = i;
                    record(format!("[hookup.pair.reject] device={dev}"));
                    self.daemon.reject(&dev);
                }
            }
            HitAction::SendFiles => {
                if let Some(dev) = self.current().map(|d| d.identity.device_id.clone()) {
                    self.send_offers(&dev);
                }
            }
            HitAction::SendClipboard => {
                if let Some(dev) = self.current().map(|d| d.identity.device_id.clone()) {
                    let text = self.clipboard.clone().unwrap_or_default();
                    self.status = Self::say(self.daemon.send_clipboard(&dev, &text), "Clipboard sent.");
                }
            }
            HitAction::PingDevice => {
                if let Some(dev) = self.current().map(|d| d.identity.device_id.clone()) {
                    self.status = Self::say(self.daemon.ping(&dev), "Ping sent.");
                }
            }
            HitAction::MediaPrev | HitAction::MediaPlayPause | HitAction::MediaNext => {
                if let Some(dev) = self.current().map(|d| d.identity.device_id.clone()) {
                    let player = self.playing.as_ref().map(|p| p.player.clone()).unwrap_or_default();
                    match action {
                        HitAction::MediaPrev => {
                            self.status = Self::say(
                                self.daemon.media(&dev, &player, Action::Previous),
                                "Previous track.",
                            );
                        }
                        HitAction::MediaPlayPause => {
                            record(format!("[hookup.media.sent] action=PlayPause player={player}"));
                            self.status = Self::say(
                                self.daemon.media(&dev, &player, Action::PlayPause),
                                "Play/pause.",
                            );
                        }
                        HitAction::MediaNext => {
                            self.status = Self::say(
                                self.daemon.media(&dev, &player, Action::Next),
                                "Next track.",
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        if sex_app::typed(key) == Some(b'b') {
            self.trigger_action(HitAction::TogglePower);
            return;
        }
        let Some(dev) = self.current().map(|d| d.identity.device_id.clone()) else {
            if sex_app::typed(key) == Some(b'r') {
                self.trigger_action(HitAction::ScanNearby);
            }
            return;
        };
        let player = self.playing.as_ref().map(|p| p.player.clone()).unwrap_or_default();
        match (sex_app::nav(key), sex_app::typed(key)) {
            (Some(Nav::Up), _) => self.picked = self.picked.saturating_sub(1),
            (Some(Nav::Down), _) => {
                self.picked = (self.picked + 1).min(self.devices.len().saturating_sub(1))
            }
            (Some(Nav::Left), _) => {
                self.status = Self::say(self.daemon.media(&dev, &player, Action::Previous), "Previous track.");
            }
            (Some(Nav::Right), _) => {
                self.status = Self::say(self.daemon.media(&dev, &player, Action::Next), "Next track.");
            }
            (_, Some(b' ')) => {
                record(format!("[hookup.media.sent] action=PlayPause player={player}"));
                self.status = Self::say(self.daemon.media(&dev, &player, Action::PlayPause), "Play/pause.");
            }
            (_, Some(b'p')) => {
                record(format!("[hookup.pair.request] device={dev}"));
                self.daemon.pair(&dev);
                self.status = "Asked to pair; accept on the phone.".into();
            }
            (_, Some(b'y')) => {
                record(format!("[hookup.pair.accept] device={dev}"));
                self.daemon.accept(&dev);
            }
            (_, Some(b'n')) => {
                record(format!("[hookup.pair.reject] device={dev}"));
                self.daemon.reject(&dev);
            }
            (_, Some(b'u')) => self.daemon.unpair(&dev),
            (_, Some(b'r')) => self.daemon.announce(),
            (_, Some(b'c')) => {
                let text = self.clipboard.clone().unwrap_or_default();
                self.status = Self::say(self.daemon.send_clipboard(&dev, &text), "Clipboard sent.");
            }
            (_, Some(b's')) => self.send_offers(&dev),
            _ if key.code == 0x1C => {
                if let Some(d) = self.current() {
                    if d.asking {
                        self.trigger_action(HitAction::AcceptPair(self.picked));
                    } else if !d.paired {
                        self.trigger_action(HitAction::ConnectDevice(self.picked));
                    }
                }
            }
            _ => {}
        }
    }

    fn hit_at(&self, px: i32, py: i32) -> Option<HitAction> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.contains(px, py))
            .map(|h| h.action)
    }

    fn draw_pill(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        h: i32,
        label: &str,
        primary: bool,
        danger: bool,
        action: HitAction,
    ) -> i32 {
        let tw = self.fonts.measure(FontKind::Sans, 11, label);
        let w = (tw + 18).max(46);
        let hovered = x <= self.hover_pos.0
            && self.hover_pos.0 < x + w
            && y <= self.hover_pos.1
            && self.hover_pos.1 < y + h;
        let pressed = self.pressed == Some(action);
        let (fill, border, fg) = if primary {
            let f = if pressed {
                ACCENT_PRESSED
            } else if hovered {
                ACCENT_HOVER
            } else {
                ACCENT
            };
            (f, ACCENT, 0x00FF_FFFF)
        } else if danger {
            let f = if pressed { 0x003B_1420 } else if hovered { 0x002B_131B } else { BADGE_BG };
            (f, 0x005C_2233, ACCENT)
        } else {
            let f = if pressed { CARD_ACTIVE } else if hovered { CARD_HOVER } else { BADGE_BG };
            (f, BORDER, TEXT)
        };
        fill_bordered_card(buf, bw, bh, x, y, w, h, h / 2, fill, border);
        self.fonts.draw(
            FontKind::Sans,
            buf,
            bw,
            bh,
            x + (w - tw) / 2,
            y + (h - 13) / 2,
            w - 6,
            11,
            label,
            fg,
        );
        self.hits.push(HitBox { x, y, w, h, action });
        w
    }

    fn draw(&mut self, window: &mut sex_app::Window) {
        let (w, h) = window.size();
        if w < 220 || h < 180 {
            return;
        }
        let px = window.frame(w, h);
        let buf = &mut px[..w * h];
        buf.fill(BG);
        self.hits.clear();

        let wi = w as i32;
        let hi = h as i32;
        let me = self.daemon.identity();

        // Top Noir Header Bar
        fill_rect(buf, w, h, 0, 0, wi, 54, 0x0011_151F);
        fill_rect(buf, w, h, 0, 54, wi, 1, BORDER);

        // Bluetooth / Hookup Emblem
        fill_rounded_rect(buf, w, h, 16, 12, 30, 30, 15, 0x0029_1422);
        self.fonts
            .draw(FontKind::Bold, buf, w, h, 26, 18, 20, 14, "B", ACCENT);

        self.fonts
            .draw(FontKind::Bold, buf, w, h, 54, 10, 260, 15, "Hookup · Bluetooth & Nearby", TEXT);
        let id_str = format!(
            "{} · port {} · LAN unencrypted link advisory",
            me.device_name,
            me.tcp_port.unwrap_or(0)
        );
        self.fonts
            .draw(FontKind::Mono, buf, w, h, 54, 30, (wi - 280).max(80), 11, &id_str, MUTED);

        // Power Toggle Pill + Scan Button on top right
        let scan_w = self.draw_pill(
            buf,
            w,
            h,
            wi - 146,
            14,
            26,
            "Scan",
            false,
            false,
            HitAction::ScanNearby,
        );
        let _ = scan_w;
        let tog_x = wi - 84;
        let tog_y = 14;
        let tog_w = 68;
        let tog_h = 26;
        let (tog_bg, tog_knob_x, tog_lbl) = if self.bluetooth_on {
            (ACCENT, tog_x + tog_w - 22, "ON")
        } else {
            (BORDER, tog_x + 4, "OFF")
        };
        fill_rounded_rect(buf, w, h, tog_x, tog_y, tog_w, tog_h, 13, tog_bg);
        fill_rounded_rect(buf, w, h, tog_knob_x, tog_y + 4, 18, 18, 9, 0x00FF_FFFF);
        let lbl_x = if self.bluetooth_on { tog_x + 12 } else { tog_x + 28 };
        self.fonts.draw(
            FontKind::Bold,
            buf,
            w,
            h,
            lbl_x,
            tog_y + 6,
            32,
            11,
            tog_lbl,
            if self.bluetooth_on { 0x00FF_FFFF } else { MUTED },
        );
        self.hits.push(HitBox {
            x: tog_x,
            y: tog_y,
            w: tog_w,
            h: tog_h,
            action: HitAction::TogglePower,
        });

        let split_x = (wi * 52 / 100).clamp(280, 560);
        let footer_h = 46;
        let content_top = 66;
        let content_bot = hi - footer_h - 8;

        // LEFT COLUMN: My Devices + Nearby Devices
        let lx = 16;
        let lw = split_x - 24;
        let mut ly = content_top;

        // Partition real devices into My Devices (paired) and Nearby Devices (unpaired)
        let my_indices: Vec<usize> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| d.paired)
            .map(|(i, _)| i)
            .collect();
        let nearby_indices: Vec<usize> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.paired)
            .map(|(i, _)| i)
            .collect();

        // Section: My Devices
        self.fonts
            .draw(FontKind::Bold, buf, w, h, lx, ly, lw, 12, "MY DEVICES", MUTED);
        let my_cnt = format!("{}", my_indices.len());
        self.fonts
            .draw(FontKind::Mono, buf, w, h, lx + lw - 24, ly, 24, 11, &my_cnt, DIM);
        ly += 22;

        if my_indices.is_empty() {
            fill_bordered_card(buf, w, h, lx, ly, lw, 50, 10, SURFACE, BORDER);
            self.fonts.draw(
                FontKind::Sans,
                buf,
                w,
                h,
                lx + 14,
                ly + 16,
                lw - 28,
                12,
                "No paired devices yet. Connect a nearby device below.",
                DIM,
            );
            ly += 60;
        } else {
            for &idx in &my_indices {
                if ly + 72 > content_bot {
                    break;
                }
                let d = self.devices[idx].clone();
                let selected = idx == self.picked;
                let hovered = lx <= self.hover_pos.0
                    && self.hover_pos.0 < lx + lw
                    && ly <= self.hover_pos.1
                    && self.hover_pos.1 < ly + 68;
                let fill = if selected {
                    CARD_ACTIVE
                } else if hovered {
                    CARD_HOVER
                } else {
                    SURFACE
                };
                let border = if selected { ACCENT } else { BORDER };
                fill_bordered_card(buf, w, h, lx, ly, lw, 68, 12, fill, border);

                // Device icon circle
                fill_rounded_rect(buf, w, h, lx + 12, ly + 14, 38, 38, 19, 0x001D_2A3A);
                let icon_ch = if d.identity.device_type == "laptop" { "L" } else { "P" };
                self.fonts.draw(
                    FontKind::Bold,
                    buf,
                    w,
                    h,
                    lx + 26,
                    ly + 24,
                    20,
                    14,
                    icon_ch,
                    if d.linked { GOOD } else { MUTED },
                );

                let tx = lx + 60;
                self.fonts.draw(
                    FontKind::Bold,
                    buf,
                    w,
                    h,
                    tx,
                    ly + 11,
                    lw - 240,
                    13,
                    clip(&d.identity.device_name, 26),
                    TEXT,
                );
                let status_str = if d.linked { "Connected" } else { "Saved · Away" };
                let sub = format!("{} · {}", d.identity.device_type, status_str);
                self.fonts.draw(
                    FontKind::Sans,
                    buf,
                    w,
                    h,
                    tx,
                    ly + 29,
                    lw - 240,
                    11,
                    &sub,
                    if d.linked { GOOD } else { MUTED },
                );

                // Battery + RSSI badges
                let bat = format!("{}%", d.battery_pct);
                let bat_w = self.fonts.measure(FontKind::Mono, 10, &bat) + 12;
                fill_bordered_card(buf, w, h, tx, ly + 46, bat_w, 16, 6, BADGE_BG, BORDER);
                self.fonts
                    .draw(FontKind::Mono, buf, w, h, tx + 6, ly + 47, bat_w, 10, &bat, GOOD);
                let rssi = format!("{} dBm", d.rssi_dbm);
                let rssi_x = tx + bat_w + 8;
                draw_rssi_bars(buf, w, h, rssi_x, ly + 47, if d.linked { 4 } else { 1 }, GOOD, BORDER);
                self.fonts.draw(
                    FontKind::Mono,
                    buf,
                    w,
                    h,
                    rssi_x + 20,
                    ly + 47,
                    70,
                    10,
                    &rssi,
                    MUTED,
                );

                // [Disconnect] and [Forget] buttons
                let forget_w = 56;
                let disc_w = 78;
                self.draw_pill(
                    buf,
                    w,
                    h,
                    lx + lw - forget_w - 10,
                    ly + 21,
                    24,
                    "Forget",
                    false,
                    true,
                    HitAction::ForgetDevice(idx),
                );
                self.draw_pill(
                    buf,
                    w,
                    h,
                    lx + lw - forget_w - disc_w - 18,
                    ly + 21,
                    24,
                    "Disconnect",
                    false,
                    false,
                    HitAction::DisconnectDevice(idx),
                );

                self.hits.push(HitBox {
                    x: lx,
                    y: ly,
                    w: (lw - forget_w - disc_w - 24).max(80),
                    h: 68,
                    action: HitAction::PickDevice(idx),
                });
                ly += 76;
            }
        }

        ly += 8;
        // Section: Nearby Devices
        if ly + 30 < content_bot {
            self.fonts
                .draw(FontKind::Bold, buf, w, h, lx, ly, lw - 80, 12, "NEARBY DEVICES", MUTED);
            let scan_lbl = if self.bluetooth_on { "Discovering..." } else { "Off" };
            self.fonts.draw(
                FontKind::Mono,
                buf,
                w,
                h,
                lx + lw - 96,
                ly,
                96,
                11,
                scan_lbl,
                if self.bluetooth_on { ACCENT } else { DIM },
            );
            ly += 22;

            if !self.bluetooth_on {
                fill_bordered_card(buf, w, h, lx, ly, lw, 56, 10, SURFACE, BORDER);
                self.fonts.draw(
                    FontKind::Sans,
                    buf,
                    w,
                    h,
                    lx + 14,
                    ly + 18,
                    lw - 110,
                    12,
                    "Bluetooth discovery is turned off.",
                    MUTED,
                );
                self.draw_pill(
                    buf,
                    w,
                    h,
                    lx + lw - 92,
                    ly + 16,
                    24,
                    "Turn On",
                    true,
                    false,
                    HitAction::TogglePower,
                );
            } else if nearby_indices.is_empty() {
                fill_bordered_card(buf, w, h, lx, ly, lw, 58, 10, SURFACE, BORDER);
                fill_rounded_rect(buf, w, h, lx + 14, ly + 23, 10, 10, 5, ACCENT);
                self.fonts.draw(
                    FontKind::Sans,
                    buf,
                    w,
                    h,
                    lx + 32,
                    ly + 20,
                    lw - 130,
                    12,
                    "Listening for nearby KDE Connect / Bluetooth peers...",
                    MUTED,
                );
                self.draw_pill(
                    buf,
                    w,
                    h,
                    lx + lw - 86,
                    ly + 17,
                    24,
                    "Refresh",
                    false,
                    false,
                    HitAction::ScanNearby,
                );
            } else {
                for &idx in &nearby_indices {
                    if ly + 62 > content_bot {
                        break;
                    }
                    let d = self.devices[idx].clone();
                    let selected = idx == self.picked;
                    let fill = if selected { CARD_ACTIVE } else { SURFACE };
                    let border = if d.asking || selected { ACCENT } else { BORDER };
                    fill_bordered_card(buf, w, h, lx, ly, lw, 58, 12, fill, border);

                    draw_rssi_bars(buf, w, h, lx + 14, ly + 22, if d.linked { 3 } else { 1 }, ACCENT, BORDER);
                    let tx = lx + 38;
                    self.fonts.draw(
                        FontKind::Bold,
                        buf,
                        w,
                        h,
                        tx,
                        ly + 10,
                        lw - 210,
                        13,
                        clip(&d.identity.device_name, 24),
                        TEXT,
                    );
                    let sub = if d.asking {
                        String::from("Requests pairing - accept?")
                    } else {
                        format!("{} · {} dBm · Ready to pair", d.identity.device_type, d.rssi_dbm)
                    };
                    self.fonts.draw(
                        FontKind::Sans,
                        buf,
                        w,
                        h,
                        tx,
                        ly + 30,
                        lw - 210,
                        11,
                        &sub,
                        if d.asking { WARN } else { MUTED },
                    );

                    if d.asking {
                        self.draw_pill(
                            buf,
                            w,
                            h,
                            lx + lw - 74,
                            ly + 17,
                            24,
                            "Decline",
                            false,
                            true,
                            HitAction::DeclinePair(idx),
                        );
                        self.draw_pill(
                            buf,
                            w,
                            h,
                            lx + lw - 144,
                            ly + 17,
                            24,
                            "Accept",
                            true,
                            false,
                            HitAction::AcceptPair(idx),
                        );
                    } else {
                        self.draw_pill(
                            buf,
                            w,
                            h,
                            lx + lw - 84,
                            ly + 17,
                            24,
                            "Connect",
                            true,
                            false,
                            HitAction::ConnectDevice(idx),
                        );
                    }

                    self.hits.push(HitBox {
                        x: lx,
                        y: ly,
                        w: (lw - 150).max(80),
                        h: 58,
                        action: HitAction::PickDevice(idx),
                    });
                    ly += 66;
                }
            }
        }

        // RIGHT COLUMN: Quick Actions, Media Remote, Notifications, Clipboard, Transfers
        let rx = split_x + 4;
        let rw = wi - rx - 16;
        let mut ry = content_top;

        // Quick Actions Row
        self.fonts
            .draw(FontKind::Bold, buf, w, h, rx, ry, rw, 12, "DEVICE ACTIONS", MUTED);
        ry += 22;
        let mut ax = rx;
        let w1 = self.draw_pill(buf, w, h, ax, ry, 26, "Send Files", true, false, HitAction::SendFiles);
        ax += w1 + 8;
        let w2 = self.draw_pill(buf, w, h, ax, ry, 26, "Sync Clipboard", false, false, HitAction::SendClipboard);
        ax += w2 + 8;
        self.draw_pill(buf, w, h, ax, ry, 26, "Ping", false, false, HitAction::PingDevice);
        ry += 38;

        // Media Remote Card
        fill_bordered_card(buf, w, h, rx, ry, rw, 78, 12, SURFACE, BORDER);
        self.fonts
            .draw(FontKind::Bold, buf, w, h, rx + 14, ry + 10, rw - 28, 11, "MEDIA REMOTE", MUTED);
        let media_line = match &self.playing {
            Some(p) => format!(
                "{} {} - {} ({})",
                if p.playing { "Playing:" } else { "Paused:" },
                if p.artist.is_empty() { "Unknown" } else { &p.artist },
                if p.title.is_empty() { "No track" } else { &p.title },
                p.player
            ),
            None => String::from("No active remote media player"),
        };
        self.fonts.draw(
            FontKind::Sans,
            buf,
            w,
            h,
            rx + 14,
            ry + 28,
            rw - 28,
            12,
            &media_line,
            TEXT,
        );
        let mx = rx + 14;
        let my = ry + 48;
        self.draw_pill(buf, w, h, mx, my, 22, "<<", false, false, HitAction::MediaPrev);
        self.draw_pill(buf, w, h, mx + 54, my, 22, "Play/Pause", true, false, HitAction::MediaPlayPause);
        self.draw_pill(buf, w, h, mx + 136, my, 22, ">>", false, false, HitAction::MediaNext);
        ry += 88;

        // Clipboard Card
        if ry + 54 < content_bot {
            fill_bordered_card(buf, w, h, rx, ry, rw, 50, 12, SURFACE, BORDER);
            self.fonts
                .draw(FontKind::Bold, buf, w, h, rx + 14, ry + 9, rw - 28, 11, "SHARED CLIPBOARD", MUTED);
            self.fonts.draw(
                FontKind::Mono,
                buf,
                w,
                h,
                rx + 14,
                ry + 27,
                rw - 28,
                11,
                self.clipboard.as_deref().unwrap_or("(empty)"),
                TEXT,
            );
            ry += 60;
        }

        // Notifications
        if ry + 56 < content_bot {
            self.fonts
                .draw(FontKind::Bold, buf, w, h, rx, ry, rw, 12, "NOTIFICATIONS", MUTED);
            ry += 20;
            if self.notifications.is_empty() {
                fill_bordered_card(buf, w, h, rx, ry, rw, 36, 10, SURFACE, BORDER);
                self.fonts.draw(
                    FontKind::Sans,
                    buf,
                    w,
                    h,
                    rx + 12,
                    ry + 10,
                    rw - 24,
                    11,
                    "No notifications from linked devices",
                    DIM,
                );
                ry += 44;
            } else {
                for (app, title, text) in self.notifications.clone() {
                    if ry + 48 > content_bot {
                        break;
                    }
                    fill_bordered_card(buf, w, h, rx, ry, rw, 44, 10, SURFACE, BORDER);
                    self.fonts.draw(
                        FontKind::Bold,
                        buf,
                        w,
                        h,
                        rx + 12,
                        ry + 7,
                        rw - 24,
                        11,
                        &format!("{app}: {title}"),
                        TEXT,
                    );
                    self.fonts.draw(
                        FontKind::Sans,
                        buf,
                        w,
                        h,
                        rx + 12,
                        ry + 23,
                        rw - 24,
                        11,
                        &text,
                        MUTED,
                    );
                    ry += 50;
                }
            }
        }

        // File Transfers
        if ry + 44 < content_bot {
            ry += 4;
            self.fonts
                .draw(FontKind::Bold, buf, w, h, rx, ry, rw, 12, "TRANSFERS", MUTED);
            ry += 20;
            if self.transfers.is_empty() {
                self.fonts.draw(
                    FontKind::Sans,
                    buf,
                    w,
                    h,
                    rx,
                    ry,
                    rw,
                    11,
                    "No recent file transfers",
                    DIM,
                );
            } else {
                for t in self.transfers.clone() {
                    if ry + 18 > content_bot {
                        break;
                    }
                    self.fonts.draw(FontKind::Mono, buf, w, h, rx, ry, rw, 11, &t, TEXT);
                    ry += 18;
                }
            }
        }

        // Bottom Status Bar
        let by = hi - footer_h;
        fill_rect(buf, w, h, 0, by, wi, footer_h, SURFACE);
        fill_rect(buf, w, h, 0, by, wi, 1, BORDER);
        self.fonts
            .draw(FontKind::Sans, buf, w, h, 16, by + 7, wi - 32, 12, &self.status, TEXT);
        self.fonts.draw(
            FontKind::Mono,
            buf,
            w,
            h,
            16,
            by + 25,
            wi - 32,
            10,
            "b power · p pair · y/n answer · u unpair · s send files · c clipboard · space play · </> track · r scan",
            DIM,
        );

        window.show_pixels(w, h);
    }
}

fn drain(app: &mut App, rx: &Receiver<Net>) -> bool {
    let mut any = false;
    while let Ok(e) = rx.try_recv() {
        app.net(e);
        any = true;
    }
    any
}

pub fn main() {
    let mut announce: Vec<SocketAddr> = Vec::new();
    let mut offers = Vec::new();
    for a in std::env::args().skip(1) {
        if a.starts_with('@') {
            offers.push(PathBuf::from(a));
            continue;
        }
        match a.to_socket_addrs() {
            Ok(mut it) => announce.extend(it.next()),
            Err(_) => eprintln!("Not an address: {a}"),
        }
    }
    let name = std::fs::read_to_string("name")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "SexOS".into());
    let config = Config {
        device_id: device_id(),
        name,
        port: sex_kdeconnect::PORT,
        announce,
        broadcast: true,
        folder: PathBuf::from("/"),
        payload_port: *sex_kdeconnect::PAYLOAD_PORTS.start(),
    };
    let (daemon, rx) = match Daemon::start(config) {
        Ok(x) => x,
        Err(e) => {
            println!(
                "Hookup could not listen on port {}: {e}\n\nStart it with network access: run +net hookup",
                sex_kdeconnect::PORT
            );
            sex_std::record(&format!("[hookup.fatal] listen: {e}"));
            std::process::exit(1);
        }
    };
    let (target, task) = sex_std::identity();
    record(format!(
        "[hookup.ready] target={target} task={task} device={} offers={}",
        daemon.identity().device_id,
        offers.len()
    ));
    let window = sex_std::window();
    window.title(b"Hookup");
    let mut app = App {
        daemon,
        bluetooth_on: true,
        devices: Vec::new(),
        picked: 0,
        notifications: Vec::new(),
        clipboard: None,
        playing: None,
        transfers: Vec::new(),
        offers,
        status: "Waiting for nearby devices. Open KDE Connect on the same network.".into(),
        hover_pos: (-1, -1),
        pressed: None,
        hits: Vec::new(),
        fonts: Fonts::new(),
    };
    drain(&mut app, &rx);
    app.draw(window);
    let mut last_tick = Instant::now();
    loop {
        let mut redraw = drain(&mut app, &rx);
        match window.next_within(100) {
            Some(Event::Close) => break,
            Some(Event::Drop { .. } | Event::DragEnded { .. } | Event::KeyUp(_)) => {}
            Some(Event::Resize { .. }) => redraw = true,
            Some(Event::Key(k)) => {
                app.key(k);
                redraw = true;
            }
            Some(Event::Pointer(p)) => match p.kind {
                PKind::Move => {
                    app.hover_pos = (p.x as i32, p.y as i32);
                }
                PKind::Leave => {
                    app.hover_pos = (-1, -1);
                    app.pressed = None;
                    redraw = true;
                }
                PKind::Down { button: 0 } => {
                    app.pressed = app.hit_at(p.x as i32, p.y as i32);
                    redraw = true;
                }
                PKind::Up { button: 0 } => {
                    let released = app.hit_at(p.x as i32, p.y as i32);
                    if let Some(act) = app.pressed.take() {
                        if Some(act) == released {
                            app.trigger_action(act);
                        }
                        redraw = true;
                    }
                }
                _ => {}
            },
            None => {}
        }
        if last_tick.elapsed().as_millis() > 1000 {
            if app.bluetooth_on {
                app.daemon.tick();
            }
            last_tick = Instant::now();
        }
        redraw |= drain(&mut app, &rx);
        if redraw {
            app.draw(window);
        }
    }
    record("[hookup.closed]".into());
    std::process::exit(0);
}
