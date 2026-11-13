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

/// Typed lines not yet read (whole lines, or what is left of one).
static PENDING: Mutex<Vec<u8>> = Mutex::new(Vec::new());
/// The line being typed: echoed, not yet ended with Enter.
static TYPING: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Reads typed input into `out`: what is left of the last line, or the next line.
/// 0 when the window was closed.
pub(crate) fn read(out: &mut [u8]) -> usize {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if pending.is_empty() {
        if let Ok(mut c) = CONSOLE.lock() { c.show(); }
        loop {
            if closed() { return 0; }
            if let Some(line) = take(window_mut().next()) { *pending = line; break; }
        }
    }
    let n = out.len().min(pending.len());
    out[..n].copy_from_slice(&pending[..n]);
    pending.drain(..n);
    n
}

fn closed() -> bool { CONSOLE.lock().map(|c| c.closed).unwrap_or(true) }

/// Whether a read of standard input would return at once: a whole line was typed, or
/// the window was closed. Takes the keys already waiting, without waiting for more.
/// A program that took the window for itself reads its keys there, so never.
pub(crate) fn input_ready() -> bool {
    if taken() { return false; }
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if !pending.is_empty() || closed() { return true; }
    if let Ok(mut c) = CONSOLE.lock() { if !c.opened { c.show(); } }
    while let Some(event) = window_mut().poll() {
        if let Some(line) = take(event) { *pending = line; return true; }
        if closed() { return true; }
    }
    false
}

/// One key or close into the line being typed, echoed; the whole line once Enter ends it.
fn take(event: Event) -> Option<Vec<u8>> {
    let mut line = TYPING.lock().unwrap_or_else(|e| e.into_inner());
    match event {
        Event::Close => { if let Ok(mut c) = CONSOLE.lock() { c.closed = true; } None }
        Event::Key(key) if key.code == 0x0E => {
            // One character back, with every byte of it.
            if !line.is_empty() {
                let mut c = CONSOLE.lock().ok()?;
                while let Some(b) = line.pop() {
                    c.text.pop();
                    if b & 0xC0 != 0x80 { break; }
                }
                c.show();
            }
            None
        }
        Event::Key(key) => {
            let ch = if key.code == 0x1C { Some('\n') } else { sex_app::text(key) }?;
            let mut utf8 = [0; 4];
            let bytes = ch.encode_utf8(&mut utf8).as_bytes();
            line.extend_from_slice(bytes);
            let mut c = CONSOLE.lock().ok()?;
            c.text.extend_from_slice(bytes);
            c.show();
            if ch == '\n' { Some(std::mem::take(&mut *line)) } else { None }
        }
        _ => None,
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
