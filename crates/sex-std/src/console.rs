// SPDX-License-Identifier: GPL-3.0-only
//! Standard input and output: the program's window as a console.
//!
//! Output goes line by line to the serial record (`[std.out]`, `[std.err]`) and, until
//! the program takes the window for itself, into the window as text: its newest lines,
//! as many as the window's text holds. The window opens with the first output or read,
//! so a program that never prints opens none. Input is a line typed into the window,
//! echoed as it is typed, with Backspace; closing the window ends input.
//!
//! A program that writes an escape sequence (colours, cursor movement, clearing: what
//! command-line ports print) turns the window into a real terminal: a VT100/xterm screen
//! (`sex-vt`) drawn in the window's pixels, with what was printed before carried over,
//! scrollback on the wheel and Shift+PageUp, and the terminal's answers (a cursor
//! position report, say) arriving on standard input as they would from a terminal.
use sex_app::{Event, Window};
use sex_pdx::text_region::BYTES as SHOWN;
use std::sync::Mutex;

/// The window as a terminal, once the program wrote an escape sequence.
struct Vt {
    screen: sex_vt::Screen,
    parser: sex_vt::Parser,
    painter: sex_vt::render::Painter,
    cache: Box<sex_vt::render::Cache>,
    size: (usize, usize),
    after_cr: bool,
}
impl Vt {
    fn new() -> Box<Vt> {
        let (w, h) = window_mut().size();
        let (cw, ch) = sex_vt::render::cell_for_height(18);
        let painter = sex_vt::render::Painter::new(cw, ch);
        let (cols, rows) = painter.grid(w, h);
        // An empty cache is all zeroes; building it on the stack would not fit.
        let cache = unsafe { Box::<sex_vt::render::Cache>::new_zeroed().assume_init() };
        Box::new(Vt { screen: sex_vt::Screen::new(cols, rows), parser: sex_vt::Parser::new(), painter, cache, size: (w.max(32), h.max(32)), after_cr: false })
    }
    /// Output, with lone newlines as CR LF (a terminal's output processing).
    fn feed(&mut self, bytes: &[u8]) {
        let mut start = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' && !(if i == 0 { self.after_cr } else { bytes[i - 1] == b'\r' }) {
                self.screen.feed(&mut self.parser, &bytes[start..i]);
                self.screen.feed(&mut self.parser, b"\r\n");
                start = i + 1;
            }
        }
        self.screen.feed(&mut self.parser, &bytes[start..]);
        if let Some(&last) = bytes.last() { self.after_cr = last == b'\r'; }
        // The terminal's answers are input, as from a real terminal.
        let replies = self.screen.take_replies();
        if !replies.is_empty() { PENDING.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&replies); }
    }
    fn resize(&mut self, w: usize, h: usize) {
        self.size = (w.max(32), h.max(32));
        let (cols, rows) = self.painter.grid(self.size.0, self.size.1);
        self.screen.resize(cols, rows);
    }
    fn draw(&mut self) {
        let window = window_mut();
        if let Some(title) = self.screen.take_title() { let t = title.to_vec(); window.title(&t); }
        let dirty = self.screen.take_dirty();
        if !dirty.iter().any(|d| *d) { return; }
        let (w, h) = self.size;
        let pixels = window.pixels();
        if pixels.len() < w * h { return; }
        self.painter.draw(&self.screen, &dirty, &mut self.cache, &mut pixels[..w * h], w, true);
        let used = self.screen.rows() * self.painter.cell_h;
        if used < h { pixels[used * w..h * w].fill(sex_vt::DEFAULT_BG); }
        window.show_pixels(w, h);
    }
}

struct Console {
    vt: Option<Box<Vt>>,
    /// What the window shows (when it is a console): the newest output.
    text: Vec<u8>,
    /// The line each stream is gathering for the serial record.
    lines: [Vec<u8>; 2],
    opened: bool,
    /// The person closed the window; reads end and the program may finish.
    closed: bool,
}

static CONSOLE: Mutex<Console> = Mutex::new(Console { vt: None, text: Vec::new(), lines: [Vec::new(), Vec::new()], opened: false, closed: false });
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
        if let Some(vt) = self.vt.as_mut() { vt.draw(); return; }
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
    if c.vt.is_none() && !taken() && bytes.contains(&0x1b) {
        // The first escape sequence: from here on the window is a terminal, starting
        // with what was printed before.
        let mut vt = Vt::new();
        let before = core::mem::take(&mut c.text);
        vt.feed(&before);
        c.vt = Some(vt);
    }
    match c.vt.as_mut() { Some(vt) => vt.feed(bytes), None => c.text.extend_from_slice(bytes) }
    c.show();
}

/// Echo of typed input, through the terminal when there is one.
fn echo(c: &mut Console, bytes: &[u8]) {
    match c.vt.as_mut() { Some(vt) => vt.feed(bytes), None => c.text.extend_from_slice(bytes) }
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
            if line.pop().is_some() {
                let mut c = CONSOLE.lock().ok()?;
                if c.vt.is_some() { echo(&mut c, b"\x08 \x08"); } else { c.text.pop(); c.show(); }
            }
            None
        }
        Event::Resize { width, height } => {
            let mut c = CONSOLE.lock().ok()?;
            if let Some(vt) = c.vt.as_mut() { vt.resize(width, height); }
            c.show();
            None
        }
        Event::Pointer(p) => {
            if let sex_pdx::app_pointer::Kind::Wheel { steps } = p.kind {
                let mut c = CONSOLE.lock().ok()?;
                if let Some(vt) = c.vt.as_mut() { vt.screen.scroll_view(steps as isize * 3); }
                c.show();
            }
            None
        }
        Event::Key(key) if key.shift && matches!(key.code, 0x49 | 0x68 | 0x51 | 0x6D) => {
            let mut c = CONSOLE.lock().ok()?;
            if let Some(vt) = c.vt.as_mut() {
                let page = vt.screen.rows() as isize - 1;
                vt.screen.scroll_view(if matches!(key.code, 0x49 | 0x68) { page } else { -page });
            }
            c.show();
            None
        }
        Event::Key(key) => {
            let b = if key.code == 0x1C { Some(b'\n') } else { sex_app::typed(key) }?;
            line.push(b);
            let mut c = CONSOLE.lock().ok()?;
            echo(&mut c, &[b]);
            if b == b'\n' { Some(std::mem::take(&mut *line)) } else { None }
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
            echo(&mut c, line.as_bytes());
            true
        }
    };
    if wait {
        loop { if let Event::Close = window_mut().next() { break; } }
    }
}
