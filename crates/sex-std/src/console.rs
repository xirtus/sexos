// SPDX-License-Identifier: GPL-3.0-only
//! Standard input and output: the program's window as a console.
//!
//! Output goes line by line to the serial record (`[std.out]`, `[std.err]`) and, until
//! the program takes the window for itself, into the window as text: its newest lines,
//! as many as the window's text holds. The window opens with the first output or read,
//! so a program that never prints opens none. Input is a line typed into the window,
//! echoed as it is typed, with Backspace; closing the window ends input.
use sex_app::{Event, Window};
use sex_pdx::text_region::BYTES as SHOWN;
use std::sync::Mutex;

struct Console {
    /// What the window shows (when it is a console): the newest output.
    text: Vec<u8>,
    /// The line each stream is gathering for the serial record.
    lines: [Vec<u8>; 2],
    opened: bool,
    /// The person closed the window; reads end and the program may finish.
    closed: bool,
}

static CONSOLE: Mutex<Console> = Mutex::new(Console { text: Vec::new(), lines: [Vec::new(), Vec::new()], opened: false, closed: false });
static mut WINDOW: Option<Window> = None;
/// Set once the program has the window for itself.
static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn window_mut() -> &'static mut Window {
    // One window per program; it is made once, here, before any use.
    unsafe {
        let w = &mut *core::ptr::addr_of_mut!(WINDOW);
        w.get_or_insert_with(Window::open)
    }
}

pub(crate) fn take_window() -> &'static mut Window {
    TAKEN.store(true, std::sync::atomic::Ordering::Release);
    CONSOLE.lock().map(|mut c| c.opened = true).ok();
    window_mut()
}

fn taken() -> bool { TAKEN.load(std::sync::atomic::Ordering::Acquire) }

impl Console {
    fn show(&mut self) {
        if taken() || self.closed { return; }
        self.opened = true;
        let cap = SHOWN as usize - 64;
        if self.text.len() > cap {
            // Keep whole lines.
            let cut = self.text.len() - cap;
            let at = self.text[cut..].iter().position(|b| *b == b'\n').map_or(cut, |p| cut + p + 1);
            self.text.drain(..at);
        }
        window_mut().show(&self.text);
    }
    fn record(&mut self, stream: usize) {
        let line = std::mem::take(&mut self.lines[stream]);
        let text = String::from_utf8_lossy(&line);
        if stream == 0 { sex_pdx::serial_record!("[std.out] {}", text); } else { sex_pdx::serial_record!("[std.err] {}", text); }
    }
}

/// Writes `bytes` to standard output (`stream` 0) or standard error (1).
pub(crate) fn write(stream: usize, bytes: &[u8]) {
    let Ok(mut c) = CONSOLE.lock() else { return };
    for &b in bytes {
        if b == b'\n' { c.record(stream); } else if c.lines[stream].len() < 400 { c.lines[stream].push(b); } else { c.record(stream); c.lines[stream].push(b); }
    }
    c.text.extend_from_slice(bytes);
    c.show();
}

/// Sends any unfinished lines to the serial record.
pub(crate) fn flush() {
    let Ok(mut c) = CONSOLE.lock() else { return };
    for stream in 0..2 { if !c.lines[stream].is_empty() { c.record(stream); } }
}

/// The input line typed so far but not yet read.
static PENDING: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Reads typed input into `out`: what is left of the last line, or the next line.
/// 0 when the window was closed.
pub(crate) fn read(out: &mut [u8]) -> usize {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if pending.is_empty() {
        if CONSOLE.lock().map(|c| c.closed).unwrap_or(true) { return 0; }
        let Some(line) = read_line() else { return 0 };
        *pending = line;
    }
    let n = out.len().min(pending.len());
    out[..n].copy_from_slice(&pending[..n]);
    pending.drain(..n);
    n
}

/// One line from the window's keys, echoed; `None` if the window was closed.
fn read_line() -> Option<Vec<u8>> {
    let mut line = Vec::new();
    {
        let mut c = CONSOLE.lock().ok()?;
        c.show();
    }
    loop {
        match window_mut().next() {
            Event::Close => { if let Ok(mut c) = CONSOLE.lock() { c.closed = true; } return None; }
            Event::Key(key) if key.code == 0x0E => {
                if line.pop().is_some() {
                    let mut c = CONSOLE.lock().ok()?;
                    c.text.pop();
                    c.show();
                }
            }
            Event::Key(key) => {
                let typed = if key.code == 0x1C { Some(b'\n') } else { sex_app::typed(key) };
                if let Some(b) = typed {
                    line.push(b);
                    let mut c = CONSOLE.lock().ok()?;
                    c.text.push(b);
                    c.show();
                    if b == b'\n' { return Some(line); }
                }
            }
            _ => {}
        }
    }
}

/// When the program ends: a console that showed output stays until the person has read
/// it and closes the window, with how the program ended as its last line.
pub(crate) fn finish(how: &str) {
    flush();
    let wait = {
        let Ok(mut c) = CONSOLE.lock() else { return };
        if !c.opened || c.closed || taken() { false } else {
            let line = format!("\n[{how}. Close the window to dismiss.]");
            c.text.extend_from_slice(line.as_bytes());
            c.show();
            true
        }
    };
    if wait {
        loop { if let Event::Close = window_mut().next() { break; } }
    }
}
