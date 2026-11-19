// SPDX-License-Identifier: GPL-3.0-only
//! The packets Hookup speaks beyond identity and pairing.
use crate::{json, Packet, Value};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const NOTIFICATION: &str = "kdeconnect.notification";
pub const NOTIFICATION_REQUEST: &str = "kdeconnect.notification.request";
pub const CLIPBOARD: &str = "kdeconnect.clipboard";
pub const CLIPBOARD_CONNECT: &str = "kdeconnect.clipboard.connect";
pub const SHARE: &str = "kdeconnect.share.request";
pub const MPRIS: &str = "kdeconnect.mpris";
pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";

/// Packet types Hookup takes.
pub const INCOMING: &[&str] = &[crate::PING, NOTIFICATION, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, MPRIS];
/// Packet types Hookup sends.
pub const OUTGOING: &[&str] = &[crate::PING, NOTIFICATION_REQUEST, CLIPBOARD, SHARE, MPRIS_REQUEST];

/// A notification from the phone.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    /// The phone dismissed it.
    pub cancel: bool,
}

pub fn notification(p: &Packet) -> Option<Notification> {
    if p.kind != NOTIFICATION { return None; }
    let s = |k: &str| p.str(k).unwrap_or("").chars().filter(|c| !c.is_control() || *c == '\n').take(400).collect::<String>();
    let id = match p.body.get("id") { Some(Value::String(v)) => v.clone(), Some(v) => v.to_string(), None => String::new() };
    let mut title = s("title");
    let mut text = s("text");
    // Older senders put everything in the ticker.
    if title.is_empty() && text.is_empty() { text = s("ticker"); }
    if title.is_empty() { title = s("appName"); }
    Some(Notification { id, app: s("appName"), title, text, cancel: p.bool("isCancel").unwrap_or(false) })
}

/// Asks the phone for every notification it is showing.
pub fn request_notifications(id: i64) -> Packet { Packet::new(NOTIFICATION_REQUEST, json!({ "request": true }), id) }
/// Dismisses one on the phone.
pub fn dismiss_notification(key: &str, id: i64) -> Packet { Packet::new(NOTIFICATION_REQUEST, json!({ "cancel": key }), id) }

/// Clipboard text from the peer (`kdeconnect.clipboard`, or `.connect` with a
/// timestamp sent when a link opens).
pub fn clipboard(p: &Packet) -> Option<String> {
    if p.kind != CLIPBOARD && p.kind != CLIPBOARD_CONNECT { return None; }
    p.str("content").map(String::from)
}
pub fn clipboard_packet(text: &str, id: i64) -> Packet { Packet::new(CLIPBOARD, json!({ "content": text }), id) }

/// A share request.
#[derive(Debug, Clone, PartialEq)]
pub enum Share {
    /// A file: its bytes follow on a payload connection.
    File { name: String, size: u64, port: Option<u16> },
    Text(String),
    Url(String),
}

pub fn share(p: &Packet) -> Option<Share> {
    if p.kind != SHARE { return None; }
    if let Some(name) = p.str("filename") {
        return Some(Share::File { name: name.to_string(), size: p.payload_size.unwrap_or(0), port: p.payload_port });
    }
    if let Some(t) = p.str("text") { return Some(Share::Text(t.to_string())); }
    p.str("url").map(|u| Share::Url(u.to_string()))
}

/// Offers a file of `size` bytes, to be fetched from `port`.
pub fn share_file(name: &str, size: u64, port: u16, id: i64) -> Packet {
    let mut p = Packet::new(SHARE, json!({ "filename": name, "numberOfFiles": 1, "totalPayloadSize": size }), id);
    p.payload_size = Some(size);
    p.payload_port = Some(port);
    p
}
pub fn share_text(text: &str, id: i64) -> Packet { Packet::new(SHARE, json!({ "text": text }), id) }

/// What a media player on the peer is doing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NowPlaying {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Position and length in milliseconds, when known.
    pub position: Option<i64>,
    pub length: Option<i64>,
}

/// Media news from the peer: its players, or what one plays.
#[derive(Debug, Clone, PartialEq)]
pub enum Media {
    Players(Vec<String>),
    Playing(NowPlaying),
}

pub fn media(p: &Packet) -> Option<Media> {
    if p.kind != MPRIS { return None; }
    if let Some(list) = p.body.get("playerList").and_then(|v| v.as_array()) {
        return Some(Media::Players(list.iter().filter_map(|v| v.as_str().map(String::from)).collect()));
    }
    let player = p.str("player")?.to_string();
    // Older senders give "artist - title" as nowPlaying.
    let (mut artist, mut title) = (p.str("artist").unwrap_or("").to_string(), p.str("title").unwrap_or("").to_string());
    if title.is_empty() {
        if let Some(now) = p.str("nowPlaying") {
            match now.split_once(" - ") { Some((a, t)) => { artist = a.into(); title = t.into(); } None => title = now.into() }
        }
    }
    Some(Media::Playing(NowPlaying { player, title, artist, playing: p.bool("isPlaying").unwrap_or(false), position: p.int("pos"), length: p.int("length") }))
}

/// A media control action, as the protocol names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { PlayPause, Play, Pause, Stop, Next, Previous }

impl Action {
    pub fn name(self) -> &'static str {
        match self { Action::PlayPause => "PlayPause", Action::Play => "Play", Action::Pause => "Pause", Action::Stop => "Stop", Action::Next => "Next", Action::Previous => "Previous" }
    }
}

pub fn media_action(player: &str, action: Action, id: i64) -> Packet { Packet::new(MPRIS_REQUEST, json!({ "player": player, "action": action.name() }), id) }
pub fn request_players(id: i64) -> Packet { Packet::new(MPRIS_REQUEST, json!({ "requestPlayerList": true }), id) }
pub fn request_now_playing(player: &str, id: i64) -> Packet { Packet::new(MPRIS_REQUEST, json!({ "player": player, "requestNowPlaying": true }), id) }
