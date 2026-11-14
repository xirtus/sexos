// SPDX-License-Identifier: GPL-3.0-only
//! Chastity on SexOS: the window, its keys, the vault file in the program's folder.
use chastity_core::vault::Kdf;
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::ui::{Host, Input, Ui};

const VAULT: &str = "vault.kdbx";
const NEW: &str = "vault.kdbx.new";
/// Locks after this long without a key.
const IDLE: Duration = Duration::from_secs(300);

struct SexOs;

impl Host for SexOs {
    fn load(&mut self) -> Option<Vec<u8>> { std::fs::read(VAULT).ok() }

    /// Written whole to a new file, made durable, then put in place: a crash leaves the
    /// old vault or the new one, never half of one.
    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        use sex_std::fs::FileExt;
        use std::io::Write;
        let mut f = std::fs::File::create(NEW).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.fsync().map_err(|e| e.to_string())?;
        drop(f);
        sex_std::fs::rename(NEW, VAULT).map_err(|e| e.to_string())
    }

    /// Shell's clipboard broker takes copies only from catalog apps (Quil, Kaleidoscope):
    /// `sex_pdx::clipboard::Participant::from_catalog` refuses `App::Program`, and a copy
    /// starts from Shell's Export intent, not from the app. So an ordinary program cannot
    /// copy yet; this asks the broker's own rule and says so. When the broker admits
    /// programs, this becomes `clipboard_client::Client::publish` as in Kaleidoscope.
    fn copy(&mut self, _text: &str) -> Result<(), String> {
        let me = sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).ok();
        match me.and_then(sex_pdx::clipboard::Participant::from_catalog) {
            None => Err("the clipboard does not take copies from ordinary programs yet".into()),
            Some(_) => Err("copying from Chastity is not wired to the clipboard yet".into()),
        }
    }

    fn record(&mut self, line: &str) { sex_std::record(line); }

    fn now(&self) -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) }
}

fn input(key: sex_pdx::app_key::Key) -> Option<Input> {
    Some(match (key.code, sex_app::nav(key)) {
        (0x1C, _) => Input::Enter,
        (0x0E, _) => Input::Backspace,
        (0x01, _) => Input::Escape,
        (_, Some(Nav::Up)) => Input::Up,
        (_, Some(Nav::Down)) => Input::Down,
        (_, Some(Nav::Delete)) => Input::Delete,
        _ if key.control => return None,
        _ => Input::Char(sex_app::text(key).filter(|c| !c.is_control())?),
    })
}

pub fn main() {
    let mut host = SexOs;
    let (target, task) = sex_std::identity();
    sex_std::record(&format!("[chastity.window] target={target} task={task}"));
    let window = sex_std::window();
    window.title(b"Chastity");
    let mut ui = Ui::new(&mut host, Kdf::STANDARD);
    let mut last_key = Instant::now();
    let mut shown = String::new();
    loop {
        let text = ui.render(&host);
        if text != shown { window.show(text.as_bytes()); shown = text; }
        // Wake each second: TOTP codes count down, and idle unlocks lock.
        match window.next_within(1000) {
            Some(Event::Close) => break,
            Some(Event::Key(key)) => {
                last_key = Instant::now();
                if let Some(i) = input(key) { ui.key(&mut host, i); }
            }
            Some(_) => {}
            None => if ui.unlocked() && last_key.elapsed() >= IDLE { ui.lock(&mut host, "idle"); },
        }
    }
    ui.lock(&mut host, "closed");
    // The shown text held no secret, but the region is cleared all the same.
    window.show(b"");
    sex_std::record("[chastity.closed]");
    std::process::exit(0);
}
