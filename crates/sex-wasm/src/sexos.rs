// SPDX-License-Identifier: GPL-3.0-only
//! The [`Host`] for a SexOS program that runs guests: the guest's window is the
//! program's window, its files are a subfolder of the program's folder, and `fetch`
//! uses the program's network grant, which exists only for a `run +net` start.
use crate::{Event, Host, Refusal};
use alloc::vec::Vec;
use sex_app::{files, net, Window};

pub struct AppHost {
    pub window: Window,
    /// The text the guest last showed, so the program can add to it after the guest ends.
    pub shown: Vec<u8>,
    /// The person closed the window and the guest was told.
    pub closed: bool,
}
impl AppHost {
    pub fn new(window: Window) -> Self { Self { window, shown: Vec::new(), closed: false } }
}

impl Host for AppHost {
    fn show(&mut self, text: &[u8]) {
        self.shown.clear();
        self.shown.extend_from_slice(text);
        self.window.show(text);
    }
    fn title(&mut self, name: &[u8]) { self.window.title(name); }
    fn log(&mut self, line: &[u8]) {
        let line = core::str::from_utf8(line).unwrap_or("<bytes>");
        sex_pdx::serial_record!("[wasm.guest] {}", line);
    }
    fn next(&mut self) -> Event {
        loop {
            match self.window.next() {
                sex_app::Event::Close => { self.closed = true; return Event::Close; }
                sex_app::Event::Key(key) => {
                    if key.code == 0x1C { return Event::Char(b'\n'); }
                    if key.code == 0x0E { return Event::Char(8); }
                    if let Some(n) = sex_app::nav(key) { return Event::Nav(n as u8); }
                    if let Some(c) = sex_app::typed(key) { return Event::Char(c); }
                    return Event::Key(key.code as u16);
                }
            }
        }
    }
    fn read(&mut self, path: &[u8], out: &mut [u8]) -> Result<usize, Refusal> {
        files::read(&mut self.window, path, out).map_err(refusal)
    }
    fn write(&mut self, path: &[u8], data: &[u8]) -> Result<(), Refusal> {
        files::write(&mut self.window, path, data).map_err(refusal)
    }
    fn fetch(&mut self, url: &[u8], out: &mut [u8]) -> Result<(u32, usize), Refusal> {
        net::get(&mut self.window, url, out).map_err(|e| match e {
            net::Error::NotAllowed => Refusal::NotAllowed,
            net::Error::Address | net::Error::NoName => Refusal::Invalid,
            _ => Refusal::Failed,
        })
    }
    /// Kernel ticks are milliseconds.
    fn now(&mut self) -> u64 { sex_pdx::get_ticks().saturating_mul(1_000_000) }
}

fn refusal(e: files::Error) -> Refusal {
    use sex_pdx::fs as w;
    match e {
        w::NOT_FOUND => Refusal::NotFound,
        w::DENIED => Refusal::NotAllowed,
        w::NAME | w::INVALID | w::IS_DIR | w::NOT_DIR => Refusal::Invalid,
        w::NO_SPACE => Refusal::TooLarge,
        _ => Refusal::Failed,
    }
}
