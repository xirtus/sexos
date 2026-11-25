// SPDX-License-Identifier: GPL-3.0-only
//! Local per-site controls. A web page may link here, but changing a setting
//! requires a browser shortcut pressed by the person.

use alloc::string::String;
use sex_app::{files, Window};
use sex_web::site_scripts::Policy;
use super::Browser;

const PREFIX: &str = "about:site?url=";
const SCRIPTS_OFF: &str = "Web/scripts-off.tsv";
const SCRIPTS_MAX: u64 = 64 << 10;

pub(super) fn load_scripts(window: &mut Window) -> Policy {
    let text = super::state_file::read(window, SCRIPTS_OFF, SCRIPTS_MAX).unwrap_or_default();
    Policy::from_text(&text)
}

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
    /// A browser key is the only way to change this preference; page links
    /// cannot write it. A failed durable save leaves the old behavior in use.
    pub(super) fn toggle_site_scripts(&mut self, url: &str) {
        let before = self.site_scripts.clone();
        let Some(enabled) = self.site_scripts.toggle(url) else { return };
        let _ = files::mkdir(&mut self.window, b"Web");
        if !super::state_file::replace(&mut self.window, SCRIPTS_OFF, self.site_scripts.text().as_bytes()) {
            self.site_scripts = before;
            sex_pdx::serial_record!("[web.site.scripts.save.failed] url={}", url);
            return;
        }
        sex_pdx::serial_record!("[web.site.scripts] enabled={} origin={}", enabled as u8, sex_fetch::Url::parse(url).map(|u| u.origin()).unwrap_or_default());
    }
    pub(super) fn site_settings_page(&self, address: &str) -> String {
        let Some(url) = target(address) else {
            return super::message_page("Site settings", "Open a web page first, then press Ctrl+Shift+E.", address);
        };
        let parsed = sex_fetch::Url::parse(&url).expect("validated site address");
        let origin = parsed.origin();
        let shield = if self.shields.up(&url) { "On" } else { "Off" };
        let scripts = if self.site_scripts.enabled(&url) { "On" } else { "Off" };
        let box_name = self.jarbox.name().unwrap_or("Main");
        let cookies = self.jar.borrow().count_site(&parsed.host);
        let storage = self.storage.borrow();
        let stored = storage.len(&origin);
        let bytes = storage.area(&origin).map_or(0, |area| area.bytes());
        sex_pdx::serial_record!("[web.site.settings] origin={} shields={} box={} cookies={} storage={} bytes={} scripts={}", origin, shield, box_name, cookies, stored, bytes, scripts);
        alloc::format!("<title>Site settings</title><body style='margin:32px;font-size:16px;background:#fbf8f1;color:#25232b'>\
            <h1>Site settings</h1><p><strong>{}</strong></p><p>Container: {}</p>\
            <p>Shields: <strong>{}</strong> — Ctrl+Shift+S to change</p>\
            <p>JavaScript: <strong>{}</strong> — Ctrl+Shift+Y to change for this origin</p>\
            <h2>Stored data</h2><p>{} cookies · {} local storage items ({} bytes)</p>\
            <p>Ctrl+Shift+Backspace clears this site's cookies, storage and saved code.</p>\
            <p><a href='{}'>Back to site</a></p></body>",
            escape(&origin), escape(box_name), shield, scripts, cookies, stored, bytes, escape(&url))
    }
}
