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
        sex_pdx::serial_record!("[web.site.settings] origin={} shields={} box={}", origin, shield, box_name);
        alloc::format!("<title>Site settings</title><body style='margin:32px;font-size:16px;background:#fbf8f1;color:#25232b'>\
            <h1>Site settings</h1><p><strong>{}</strong></p><p>Container: {}</p>\
            <p>Shields: <strong>{}</strong> — Ctrl+Shift+S to change</p>\
            <p>Ctrl+Shift+Backspace clears this site's cookies, storage and saved code.</p>\
            <p><a href='{}'>Back to site</a></p></body>",
            escape(&origin), escape(box_name), shield, escape(&url))
    }
}
