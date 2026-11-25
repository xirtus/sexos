// SPDX-License-Identifier: GPL-3.0-only
//! Local per-site controls. A web page may link here, but changing a setting
//! requires a browser shortcut pressed by the person.

use alloc::string::String;
use super::Browser;

const PREFIX: &str = "about:site?url=";

pub(super) fn target(address: &str) -> Option<String> {
    let encoded = address.strip_prefix(PREFIX)?;
    if encoded.len() > 4096 { return None; }
    let url = super::extensions::decode(encoded);
    sex_fetch::Url::parse(&url).ok()?;
    Some(url)
}

pub(super) fn address(url: &str) -> String {
    alloc::format!("{PREFIX}{}", super::keys::encode(url))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\'', "&#39;")
}

impl Browser {
    pub(super) fn site_settings_page(&self, address: &str) -> String {
        let Some(url) = target(address) else {
            return super::message_page("Site settings", "Open a web page first, then press Ctrl+Shift+E.", address);
        };
        let parsed = sex_fetch::Url::parse(&url).expect("validated site address");
        let origin = parsed.origin();
        let shield = if self.shields.up(&url) { "On" } else { "Off" };
        let box_name = self.jarbox.name().unwrap_or("Main");
        let cookies = self.jar.borrow().count_site(&parsed.host);
        let storage = self.storage.borrow();
        let stored = storage.len(&origin);
        let bytes = storage.area(&origin).map_or(0, |area| area.bytes());
        web_record!("[web.site.settings] origin={} shields={} box={} cookies={} storage={} bytes={}", origin, shield, box_name, cookies, stored, bytes);
        alloc::format!("<title>Site settings</title><body style='margin:32px;font-size:16px;background:#fbf8f1;color:#25232b'>\
            <h1>Site settings</h1><p><strong>{}</strong></p><p>Container: {}</p>\
            <p>Shields: <strong>{}</strong> — Ctrl+Shift+S to change</p>\
            <h2>Stored data</h2><p>{} cookies · {} local storage items ({} bytes)</p>\
            <p>Ctrl+Alt+C clears only cookies; Ctrl+Alt+S clears only local storage.</p>\
            <p>Ctrl+Shift+Backspace clears this site's cookies, storage and saved code.</p>\
            <p><a href='{}'>Back to site</a></p></body>",
            escape(&origin), escape(box_name), shield, cookies, stored, bytes, escape(&url))
    }
}
