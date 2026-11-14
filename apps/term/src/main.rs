// SPDX-License-Identifier: GPL-3.0-only
//! Spindle's terminal window (the `term` program): tabs of split panes, each a
//! VT100/xterm screen (crates/sex-vt) drawn in Geist Mono, with scrollback, mouse
//! selection, copy and paste, and a small command line of its own for the files in its
//! folder. The tabs and the split tree are `spindle-panes`; each pane draws into its own
//! pixels, which the window copies into place.
//!
//! Command-line programs run inside it as Wasm guests (crates/sex-wasm): `NAME.wasm`
//! files in the current folder, or the built-in `greet`. A guest runs on a worker and
//! talks to the screen through a terminal stream (`sex_pdx::stream`), the SexOS
//! pseudo-terminal: its output, escape sequences and all, goes to the screen, and what
//! the person types reaches its standard input a line at a time. The guest gets the
//! current folder and nothing else; it holds no authority Term does not, and less.
//!
//! Any other name is a program of its own (`/bin/NAME`: Rust std, C on sex-libc, Python,
//! Lua, ripgrep...) started with Term as its console, when Spindle's `spindle` command opened this
//! Term (the `+launch` grant). Its output and input go through Files
//! (`sex_pdx::console_relay`), it opens no window, gets only its own plain words as
//! arguments and no network, Ctrl+C stops it and Ctrl+D ends its input.
//!
//! Keys: Ctrl+Shift+C copies the selection, Ctrl+Shift+V pastes, Shift+PageUp/PageDown
//! and the wheel scroll back, Ctrl+= and Ctrl+- change the text size, Ctrl+C stops the
//! running program's input, Ctrl+D ends it, Ctrl+L clears. F11 closes.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use sex_app::{files, Event, Nav, Window};
use sex_pdx::app_pointer::Kind;
use sex_pdx::stream::{Side, Stream};
use sex_vt::keys::{self, Key, Mods};
use sex_vt::render::{cell_for_height, Cache, Painter};
use sex_vt::{Parser, Screen};
use spindle_panes::{Divider, PaneId, Rect, Split, Tabs, Toward};

/// `crates/sex-wasm/guests/greet`: Rust's std built for wasm32-wasip1.
const GREET: &[u8] = include_bytes!("../../../crates/sex-wasm/tests/greet.wasm");
const MODULE_MAX: usize = 8 << 20;
const STREAM_RING: usize = 64 * 1024;
static mut CACHE: Cache = Cache::new();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[term.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// A guest running on a worker, and the stream between it and the screen.
struct Guest {
    stream: Stream,
    _memory: Vec<u64>,
    handle: sex_app::thread::JoinHandle,
    result: &'static core::sync::atomic::AtomicI64,
}
/// What runs: a Wasm guest here, or a program of its own whose console is this terminal
/// (through Files, `sex_pdx::console_relay`).
enum Runner { Guest(Guest), Native { since: u64 } }
struct Job {
    name: String,
    runner: Runner,
    /// The line being typed for it, echoed here and sent on Enter.
    line: Vec<u8>,
}
/// How long a program may take to start before Term gives up on it.
const START_MS: u64 = 20_000;

/// One pane: a screen, its own command line and the program running in it.
struct Pane {
    screen: Screen,
    parser: Parser,
    painter: Painter,
    /// The pane's size in pixels, and its pixels (drawn here, then copied into the window).
    width: usize,
    height: usize,
    buf: Vec<u32>,
    cell_h: usize,
    /// `exit` or Ctrl+D on an empty line: the window closes this pane.
    closed: bool,
    cwd: Vec<u8>,
    line: Vec<u8>,
    at: usize,
    history: Vec<Vec<u8>>,
    recall: Option<usize>,
    job: Option<Job>,
    /// Where a mouse drag started.
    drag: Option<sex_vt::Point>,
    /// Whether the last drawing showed the cursor as focused.
    focused_drawn: bool,
    /// The title the program set (OSC 0/2).
    osc_title: Option<String>,
    last_click: (u64, usize, usize),
    /// A carriage return was the last byte fed, so a following newline is not doubled.
    after_cr: bool,
}

impl Pane {
    fn new(width: usize, height: usize, cell_h: usize) -> Pane {
        let (cw, ch) = cell_for_height(cell_h);
        let painter = Painter::new(cw, ch);
        let (width, height) = (width.max(32), height.max(32));
        let (cols, rows) = painter.grid(width, height);
        Pane {
            screen: Screen::new(cols, rows), parser: Parser::new(), painter, width, height, buf: vec![sex_vt::DEFAULT_BG; width * height], cell_h, closed: false,
            cwd: Vec::new(), line: Vec::new(), at: 0, history: Vec::new(), recall: None,
            job: None, drag: None, focused_drawn: false, osc_title: None, last_click: (0, usize::MAX, usize::MAX), after_cr: false,
        }
    }
    /// Program output: lone newlines become CR LF, as a terminal's output processing does.
    fn out(&mut self, bytes: &[u8]) {
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
    }
    fn say(&mut self, text: &str) { self.out(text.as_bytes()); }
    fn resize(&mut self, width: usize, height: usize) {
        let (width, height) = (width.max(32), height.max(32));
        if (width, height) == (self.width, self.height) { return; }
        self.width = width;
        self.height = height;
        self.buf = vec![sex_vt::DEFAULT_BG; width * height];
        let (cols, rows) = self.painter.grid(self.width, self.height);
        self.screen.resize(cols, rows);
        if let Some(Job { runner: Runner::Guest(g), .. }) = &self.job { g.stream.set_size(cols as u32, rows as u32); }
        self.screen.mark_all_dirty();
    }
    /// A new text size: `h` pixels per cell.
    fn set_cell_height(&mut self, h: usize) {
        if h == self.cell_h { return; }
        self.cell_h = h;
        let (cw, ch) = cell_for_height(h);
        self.painter = Painter::new(cw, ch);
        let (w, h) = (self.width, self.height);
        self.width = 0;
        self.resize(w, h);
    }
    /// Draws the rows that changed into the pane's pixels. True if any did.
    fn draw(&mut self, focused: bool) -> bool {
        if let Some(t) = self.screen.take_title() { self.osc_title = Some(String::from_utf8_lossy(t).into_owned()); }
        let dirty = self.screen.take_dirty();
        if !dirty.iter().any(|d| *d) && self.focused_drawn == focused { return false; }
        let dirty = if self.focused_drawn != focused { Vec::new() } else { dirty };
        self.focused_drawn = focused;
        let (w, h) = (self.width, self.height);
        let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
        self.painter.draw(&self.screen, &dirty, cache, &mut self.buf[..w * h], w, focused);
        // Below the last whole row, the terminal's background.
        let used = self.screen.rows() * self.painter.cell_h;
        if used < h { self.buf[used * w..h * w].fill(sex_vt::DEFAULT_BG); }
        true
    }
    /// The pane's title: what its program asked for, else the running program's name,
    /// else its folder.
    fn title(&self) -> String {
        if let Some(t) = &self.osc_title { return t.clone(); }
        if let Some(job) = &self.job { return job.name.clone(); }
        format!("~/{}", core::str::from_utf8(&self.cwd).unwrap_or("?"))
    }

    // ---- The command line ----

    fn prompt(&self) -> String {
        let cwd = core::str::from_utf8(&self.cwd).unwrap_or("?");
        format!("\x1b[1;34m~/{}\x1b[0m \x1b[1;32m>\x1b[0m ", cwd)
    }
    /// Redraws the line being typed, with the cursor at `self.at`.
    fn redraw_line(&mut self) {
        let p = self.prompt();
        let line = String::from_utf8_lossy(&self.line).into_owned();
        let back = self.line.len() - self.at;
        self.say(&format!("\r\x1b[K{}{}", p, line));
        if back > 0 { self.say(&format!("\x1b[{}D", back)); }
    }
    fn show_prompt(&mut self) { self.line.clear(); self.at = 0; self.recall = None; let p = self.prompt(); self.say(&p); }

    /// A path from the command line, as a path in Term's folder: relative to the current
    /// folder, `/` the folder's root, `..` going up but never above it.
    fn resolve(&self, arg: &[u8]) -> Vec<u8> {
        let mut parts: Vec<&[u8]> = if arg.first() == Some(&b'/') { Vec::new() } else { self.cwd.split(|b| *b == b'/').filter(|p| !p.is_empty()).collect() };
        for part in arg.split(|b| *b == b'/') {
            match part { b"" | b"." => {} b".." => { parts.pop(); } p => parts.push(p) }
        }
        parts.join(&b'/')
    }

    fn run_line(&mut self, window: &mut Window, line: &[u8]) {
        let line = line.trim_ascii();
        if line.is_empty() { return; }
        if self.history.last().map(|l| l.as_slice()) != Some(line) { self.history.push(line.to_vec()); }
        if self.history.len() > 500 { self.history.remove(0); }
        let words: Vec<&[u8]> = line.split(|b| *b == b' ').filter(|w| !w.is_empty()).collect();
        let (cmd, args) = (words[0], &words[1..]);
        sex_pdx::serial_record!("[term.command] {}", core::str::from_utf8(cmd).unwrap_or("?"));
        match cmd {
            b"help" => self.say(HELP),
            b"clear" => self.say("\x1b[H\x1b[2J\x1b[3J"),
            b"pwd" => { let p = format!("/{}\n", core::str::from_utf8(&self.cwd).unwrap_or("?")); self.say(&p); }
            b"echo" => { let t = args.join(&b' '); self.out(&t); self.say("\n"); }
            b"history" => {
                let lines: Vec<String> = self.history.iter().enumerate().map(|(i, l)| format!("{:4}  {}\n", i + 1, String::from_utf8_lossy(l))).collect();
                for l in lines { self.say(&l); }
            }
            b"cd" => {
                let target = self.resolve(args.first().copied().unwrap_or(b"/"));
                match files::stat(window, &target) {
                    Ok(st) if st.dir || target.is_empty() => self.cwd = target,
                    Ok(_) => self.say("cd: not a folder\n"),
                    Err(e) if target.is_empty() => { let _ = e; self.cwd.clear(); }
                    Err(e) => self.fail("cd", e),
                }
            }
            b"ls" => {
                let target = self.resolve(args.first().copied().unwrap_or(b"."));
                match files::list(window, &target) {
                    Ok(mut entries) => {
                        entries.sort_by(|a, b| (!a.dir, &a.name).cmp(&(!b.dir, &b.name)));
                        let mut text = String::new();
                        for e in entries {
                            let name = String::from_utf8_lossy(&e.name);
                            if e.dir { text.push_str(&format!("\x1b[1;34m{}/\x1b[0m\n", name)); }
                            else if name.ends_with(".wasm") { text.push_str(&format!("\x1b[1;32m{}\x1b[0m  {}\n", name, size(e.size))); }
                            else { text.push_str(&format!("{}  \x1b[2m{}\x1b[0m\n", name, size(e.size))); }
                        }
                        self.say(&text);
                    }
                    Err(e) => self.fail("ls", e),
                }
            }
            b"cat" => for a in args {
                let path = self.resolve(a);
                match read_all(window, &path, 4 << 20) { Ok(data) => self.out(&data), Err(e) => self.fail("cat", e) }
            },
            b"mkdir" => for a in args { let p = self.resolve(a); if let Err(e) = files::mkdir(window, &p) { self.fail("mkdir", e); } },
            b"rm" => for a in args { let p = self.resolve(a); if let Err(e) = files::remove(window, &p) { self.fail("rm", e); } },
            b"touch" => for a in args { let p = self.resolve(a); if files::stat(window, &p).is_err() { if let Err(e) = files::create(window, &p) { self.fail("touch", e); } } },
            b"mv" if args.len() == 2 => { let (a, b) = (self.resolve(args[0]), self.resolve(args[1])); if let Err(e) = files::rename(window, &a, &b) { self.fail("mv", e); } }
            b"write" if !args.is_empty() => {
                let path = self.resolve(args[0]);
                let mut text = args[1..].join(&b' ');
                text.push(b'\n');
                if let Err(e) = files::write(window, &path, &text) { self.fail("write", e); }
            }
            b"colors" => self.say(&colors()),
            b"vtcheck" => self.vtcheck(window),
            // The screen's text to the serial record, for gates and bug reports.
            b"snapshot" => {
                // The screen and up to 40 lines above it.
                let total = self.screen.history_len() + self.screen.rows();
                for r in total.saturating_sub(self.screen.rows() + 40)..total {
                    let row = self.screen.line_text(r);
                    if !row.is_empty() { sex_pdx::serial_record!("[term.row] r={} {}", r, row); }
                }
                sex_pdx::serial_record!("[term.snapshot] rows={} history={}", self.screen.rows(), self.screen.history_len());
            }
            b"exit" => { sex_pdx::serial_record!("[term.pane.closed] how=exit"); self.closed = true; }
            _ => self.start(window, cmd, args),
        }
    }
    /// The pane is closing: whatever runs in it is told to stop and loses its input.
    fn stop(&mut self) {
        let Some(job) = self.job.take() else { return };
        sex_pdx::serial_record!("[term.job.stop] name={}", job.name);
        match job.runner {
            // The worker finishes on its own once its input is gone; its memory stays
            // until then (the stream lives in it), so it is left behind on purpose.
            Runner::Guest(g) => { g.stream.interrupt(); g.stream.hang_up(Side::Host); core::mem::forget(g); }
            Runner::Native { .. } => { let _ = files::direct::relay::interrupt(); let _ = files::direct::relay::hang_up(); }
        }
    }
    fn fail(&mut self, what: &str, e: files::Error) {
        let why = String::from_utf8_lossy(files::message(e)).into_owned();
        self.say(&format!("\x1b[31m{}: {}\x1b[0m\n", what, why));
    }

    /// Starts `NAME.wasm` (or the built-in guest) on a worker with a stream to the screen.
    fn start(&mut self, window: &mut Window, cmd: &[u8], args: &[&[u8]]) {
        let name = cmd.strip_suffix(b".wasm").unwrap_or(cmd);
        let module: Vec<u8> = if cmd == b"greet" { GREET.to_vec() } else {
            let mut path = self.resolve(cmd);
            if !path.ends_with(b".wasm") { path.extend_from_slice(b".wasm"); }
            match read_all(window, &path, MODULE_MAX) {
                Ok(m) => m,
                Err(files::NOT_FOUND) if !cmd.contains(&b'/') && !cmd.ends_with(b".wasm") => { self.start_native(cmd, args); return; }
                Err(files::NOT_FOUND) => {
                    let n = String::from_utf8_lossy(cmd).into_owned();
                    self.say(&format!("\x1b[31m{}: no such command or program\x1b[0m (type help)\n", n));
                    return;
                }
                Err(e) => { self.fail("run", e); return; }
            }
        };
        let (cols, rows) = (self.screen.cols() as u32, self.screen.rows() as u32);
        let mut memory = vec![0u64; sex_pdx::stream::bytes_needed(STREAM_RING) / 8];
        let Some(stream) = (unsafe { Stream::init(memory.as_mut_ptr() as *mut u8, memory.len() * 8, cols, rows) }) else { return };
        let result: &'static core::sync::atomic::AtomicI64 = alloc::boxed::Box::leak(alloc::boxed::Box::new(core::sync::atomic::AtomicI64::new(i64::MIN)));
        let owned_name = name.to_vec();
        let owned_args: Vec<Vec<u8>> = args.iter().map(|a| a.to_vec()).collect();
        let folder = self.cwd.clone();
        let job = move || {
            let args: Vec<&[u8]> = owned_args.iter().map(|a| a.as_slice()).collect();
            let guest = sex_wasm::Guest {
                name: &owned_name, args: &args,
                grants: sex_wasm::Grants { folder: Some(folder), net: false },
                limits: sex_wasm::Limits { memory_bytes: 64 << 20, ..sex_wasm::Limits::default() },
            };
            let (outcome, host) = sex_wasm::run(&module, &guest, StreamHost { stream, idle: 0 });
            let code = match outcome {
                sex_wasm::Outcome::Exited(c) => c as i64,
                sex_wasm::Outcome::Trapped(why) => { host.say(&format!("\r\n[trapped: {}]\r\n", why)); -1000 }
                sex_wasm::Outcome::Rejected(why) => { host.say(&format!("\r\n[not a program: {}]\r\n", why)); -1001 }
                _ => -1002,
            };
            host.stream.hang_up(Side::Guest);
            result.store(code, core::sync::atomic::Ordering::Release);
        };
        match sex_app::thread::spawn_with_stack(2 << 20, job) {
            Ok(handle) => {
                sex_pdx::serial_record!("[term.job.start] name={}", core::str::from_utf8(name).unwrap_or("?"));
                self.job = Some(Job { name: String::from_utf8_lossy(name).into_owned(), runner: Runner::Guest(Guest { stream, _memory: memory, handle, result }), line: Vec::new() });
            }
            Err(_) => self.say("\x1b[31mrun: no worker is free to run it\x1b[0m\n"),
        }
    }
    /// Takes what the guest wrote; ends the job when it has finished. True if the screen changed.
    fn pump(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let Runner::Guest(guest) = &job.runner else { return self.pump_native() };
        let stream = guest.stream;
        let mut buf = [0u8; 4096];
        let mut changed = false;
        loop {
            let n = stream.read(Side::Host, &mut buf);
            if n == 0 { break; }
            self.out(&buf[..n]);
            changed = true;
        }
        let replies = self.screen.take_replies();
        if !replies.is_empty() { let _ = stream.write(Side::Host, &replies); }
        let Some(Job { runner: Runner::Guest(guest), .. }) = &self.job else { return changed };
        if guest.handle.finished() && stream.readable(Side::Host) == 0 {
            let job = self.job.take().unwrap();
            let Runner::Guest(guest) = job.runner else { return changed };
            let code = guest.result.load(core::sync::atomic::Ordering::Acquire);
            guest.handle.join();
            self.job_ended(&job.name, code);
            changed = true;
        }
        changed
    }
    fn job_ended(&mut self, name: &str, code: i64) {
        sex_pdx::serial_record!("[term.job.exit] name={} code={}", name, code);
        if code != 0 { self.say(&format!("\x1b[2m[{} exited with {}]\x1b[0m\n", name, code)); }
        if !self.after_cr && self.screen.cursor().0 != 0 { self.say("\n"); }
        self.show_prompt();
    }

    /// Starts `/bin/NAME`, a program of its own (Rust std, C, Python, Lua...), with this
    /// terminal as its console. Only a Term started with Spindle's `spindle` command may.
    fn start_native(&mut self, cmd: &[u8], args: &[&[u8]]) {
        let relay = files::direct::relay::release;
        let _ = relay();
        let words = args.join(&b' ');
        let n = String::from_utf8_lossy(cmd).into_owned();
        match files::direct::relay::start(cmd, &words) {
            Ok(()) => {
                sex_pdx::serial_record!("[term.native.start] name={}", n);
                self.job = Some(Job { name: n, runner: Runner::Native { since: sex_pdx::get_ticks() }, line: Vec::new() });
            }
            Err(e) if { sex_pdx::serial_record!("[term.native.refused] name={} status={}", n, e); false } => {}
            Err(files::NOT_FOUND) => self.say(&format!("\x1b[31m{}: no such command or program\x1b[0m (type help)\n", n)),
            Err(sex_pdx::fs::DENIED) => self.say(&format!("\x1b[31m{}: this window may not start programs\x1b[0m (open it with Spindle's \x1b[1mspindle\x1b[0m command)\n", n)),
            Err(sex_pdx::fs::PENDING) => self.say("\x1b[31mrun: a program is still running here\x1b[0m\n"),
            Err(e) => self.fail(&n, e),
        }
    }
    /// The native program's output and whether it ended.
    fn pump_native(&mut self) -> bool {
        use sex_pdx::console_relay as c;
        let Ok((status, bytes)) = files::direct::relay::take() else { return false };
        let changed = !bytes.is_empty();
        if changed { self.out(&bytes); }
        let replies = self.screen.take_replies();
        if !replies.is_empty() { let _ = files::direct::relay::put(&replies); }
        let Some(Job { runner: Runner::Native { since }, name, .. }) = &self.job else { return changed };
        let (since, name) = (*since, name.clone());
        let ended = match status.state {
            c::ENDED if status.code == c::NOT_STARTED => {
                self.say(&format!("\x1b[31m{}: no such command or program\x1b[0m (type help)\n", name));
                Some(-2)
            }
            c::ENDED => Some(if status.code == c::CRASHED { -1 } else { status.code as i64 }),
            c::STARTING if sex_pdx::get_ticks().saturating_sub(since) > START_MS => {
                self.say(&format!("\x1b[31m{}: it did not start\x1b[0m (too many programs open?)\n", name));
                Some(-2)
            }
            _ => None,
        };
        let Some(code) = ended else { return changed };
        // What it wrote last may still be on its way.
        if let Ok((_, rest)) = files::direct::relay::take() { self.out(&rest); }
        self.job = None;
        let _ = files::direct::relay::release();
        if code == -1 { self.say(&format!("\x1b[31m[{} stopped: it failed]\x1b[0m\n", name)); }
        self.job_ended(&name, code);
        true
    }

    /// A built-in fixture in the spirit of vttest's first screens: a frame drawn with
    /// absolute moves, coloured text and erasures, checked against the screen model and
    /// the drawn pixels. The result goes to the serial record for the gate.
    fn vtcheck(&mut self, window: &mut Window) {
        use sex_vt::check;
        self.say(check::FIXTURE);
        let s = &self.screen;
        let frame = (0..6).all(|r| s.row_text(r) == check::ROWS[r]);
        let colors = check::COLORS.iter().all(|&(x, y, fg, bg)| sex_vt::colors(&s.visible(x, y)) == (fg, bg));
        // Then the pixels: the middle-left of each checked cell after drawing is its background.
        self.screen.mark_all_dirty();
        self.draw(true);
        let _ = window;
        let (cw, ch, w) = (self.painter.cell_w, self.painter.cell_h, self.width);
        let pixels = &self.buf;
        let drawn = check::COLORS.iter().all(|&(x, y, _, bg)| pixels[(y * ch + 1) * w + x * cw] == bg);
        sex_pdx::serial_record!("[term.vtcheck] frame={} colors={} pixels={} cols={} rows={}", frame as u8, colors as u8, drawn as u8, self.screen.cols(), self.screen.rows());
        self.say(if frame && colors && drawn { "vtcheck: \x1b[32mok\x1b[0m\n" } else { "vtcheck: \x1b[31mFAILED\x1b[0m\n" });
    }

    // ---- Keys and the mouse ----

    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let (shift, control) = (key.shift, key.control);
        // Terminal-wide keys first.
        match (control, shift, key.code) {
            (false, true, 0x49 | 0x68) => { let r = self.screen.rows() as isize - 1; self.screen.scroll_view(r); return; }
            (false, true, 0x51 | 0x6D) => { let r = self.screen.rows() as isize - 1; self.screen.scroll_view(-r); return; }
            _ => {}
        }
        self.screen.clear_selection();
        if self.job.is_some() { self.job_key(key); return; }
        let typed = if control { None } else { sex_app::typed(key) };
        match (control, key.code, sex_app::nav(key), typed) {
            (_, 0x1C, ..) => {
                let line = core::mem::take(&mut self.line);
                self.say("\r\n");
                self.run_line(window, &line);
                if self.job.is_none() { self.show_prompt(); }
                return;
            }
            (true, 0x2E, ..) => { self.say("^C\r\n"); self.show_prompt(); return; }
            (true, 0x26, ..) => { self.say("\x1b[H\x1b[2J"); self.redraw_line(); return; }
            (true, 0x20, ..) if self.line.is_empty() => { sex_pdx::serial_record!("[term.pane.closed] how=ctrl-d"); self.closed = true; return; }
            (true, 0x1E, ..) => self.at = 0,
            (true, 0x12, ..) => self.at = self.line.len(),
            (true, 0x16, ..) => { self.line.drain(..self.at); self.at = 0; }
            (true, 0x25, ..) => self.line.truncate(self.at),
            (_, 0x0E, ..) => if self.at > 0 { self.at -= 1; self.line.remove(self.at); },
            (_, 0x0F, ..) => self.complete(window),
            (_, _, Some(Nav::Left), _) => self.at = self.at.saturating_sub(1),
            (_, _, Some(Nav::Right), _) => self.at = (self.at + 1).min(self.line.len()),
            (_, _, Some(Nav::Home), _) => self.at = 0,
            (_, _, Some(Nav::End), _) => self.at = self.line.len(),
            (_, _, Some(Nav::Delete), _) => if self.at < self.line.len() { self.line.remove(self.at); },
            (_, _, Some(dir @ (Nav::Up | Nav::Down)), _) => {
                if self.history.is_empty() { return; }
                let last = self.history.len();
                let next = match (self.recall, dir) {
                    (None, Nav::Up) => Some(last - 1),
                    (Some(i), Nav::Up) => Some(i.saturating_sub(1)),
                    (Some(i), _) if i + 1 < last => Some(i + 1),
                    _ => None,
                };
                self.recall = next;
                self.line = next.map(|i| self.history[i].clone()).unwrap_or_default();
                self.at = self.line.len();
            }
            (false, _, None, Some(c)) => { self.line.insert(self.at, c); self.at += 1; }
            _ => return,
        }
        self.redraw_line();
    }
    /// Completes the last word from the names in its folder (and the built-ins first).
    fn complete(&mut self, window: &mut Window) {
        let start = self.line[..self.at].iter().rposition(|b| *b == b' ').map_or(0, |i| i + 1);
        let word = self.line[start..self.at].to_vec();
        let (dir, stem) = match word.iter().rposition(|b| *b == b'/') { Some(i) => (word[..=i].to_vec(), word[i + 1..].to_vec()), None => (Vec::new(), word.clone()) };
        let mut names: Vec<Vec<u8>> = Vec::new();
        if start == 0 && dir.is_empty() { names.extend(BUILTINS.iter().map(|b| b.to_vec())); }
        if let Ok(entries) = files::list(window, &self.resolve(if dir.is_empty() { b"." } else { &dir })) {
            names.extend(entries.into_iter().map(|e| { let mut n = e.name; if e.dir { n.push(b'/'); } n }));
        }
        let hits: Vec<&Vec<u8>> = names.iter().filter(|n| n.starts_with(&stem)).collect();
        let Some(first) = hits.first() else { return };
        // The longest common prefix of all the matches.
        let common = hits.iter().fold(first.len(), |n, h| first.iter().zip(h.iter()).take(n).take_while(|(a, b)| a == b).count());
        let mut add = first[stem.len()..common].to_vec();
        if hits.len() == 1 && !first.ends_with(b"/") { add.push(b' '); }
        if add.is_empty() && hits.len() > 1 {
            let list: Vec<String> = hits.iter().map(|h| String::from_utf8_lossy(h).into_owned()).collect();
            self.say(&format!("\r\n{}\r\n", list.join("  ")));
        }
        for (i, b) in add.iter().enumerate() { self.line.insert(self.at + i, *b); }
        self.at += add.len();
    }
    /// Keys while a guest runs: a line typed locally (the guest has no terminal modes),
    /// sent whole on Enter.
    fn job_key(&mut self, key: sex_pdx::app_key::Key) {
        let job = self.job.as_mut().unwrap();
        let Runner::Guest(guest) = &job.runner else { return self.native_key(key) };
        let stream = guest.stream;
        match (key.control, key.code) {
            (true, 0x2E) => {
                // Ctrl+C: tell the guest, drop the typed line and end its input.
                stream.interrupt();
                job.line.clear();
                stream.hang_up(Side::Host);
                sex_pdx::serial_record!("[term.job.interrupt] name={}", job.name);
                self.say("^C");
            }
            (true, 0x20) => { stream.hang_up(Side::Host); }
            (_, 0x1C) => {
                let mut line = core::mem::take(&mut job.line);
                line.push(b'\n');
                let _ = stream.write(Side::Host, &line);
                self.say("\r\n");
            }
            (_, 0x0E) => { if job.line.pop().is_some() { self.say("\x08 \x08"); } }
            (false, _) => {
                if let Some(c) = sex_app::typed(key) { job.line.push(c); self.out(&[c]); }
                else if let Some(k) = nav_key(key) {
                    // Arrows and the like go straight through, for guests that read them.
                    let bytes = keys::encode(k, Mods { shift: key.shift, control: key.control, alt: false }, self.screen.app_cursor);
                    let _ = stream.write(Side::Host, &bytes);
                }
            }
            _ => {}
        }
    }
    /// Keys while a program of its own runs: the same line discipline, through Files.
    fn native_key(&mut self, key: sex_pdx::app_key::Key) {
        use files::direct::relay;
        let job = self.job.as_mut().unwrap();
        match (key.control, key.code) {
            (true, 0x2E) => {
                let _ = relay::interrupt();
                job.line.clear();
                sex_pdx::serial_record!("[term.job.interrupt] name={}", job.name);
                self.say("^C");
            }
            (true, 0x20) => { let _ = relay::hang_up(); }
            (_, 0x1C) => {
                let mut line = core::mem::take(&mut job.line);
                line.push(b'\n');
                let _ = relay::put(&line);
                self.say("\r\n");
            }
            (_, 0x0E) => {
                let mut gone = false;
                while let Some(b) = job.line.pop() { gone = true; if b & 0xC0 != 0x80 { break; } }
                if gone { self.say("\x08 \x08"); }
            }
            (false, _) => {
                if let Some(c) = sex_app::text(key) {
                    let mut utf8 = [0; 4];
                    let bytes = c.encode_utf8(&mut utf8).as_bytes();
                    job.line.extend_from_slice(bytes);
                    self.out(bytes);
                } else if let Some(k) = nav_key(key) {
                    let bytes = keys::encode(k, Mods { shift: key.shift, control: key.control, alt: false }, self.screen.app_cursor);
                    let _ = relay::put(&bytes);
                }
            }
            _ => {}
        }
    }
    /// The selection, for the window's clipboard.
    fn copy(&mut self) -> Option<Vec<u8>> {
        let text = self.screen.selection_text();
        if text.is_empty() { return None; }
        sex_pdx::serial_record!("[term.copy] bytes={}", text.len());
        Some(text.into_bytes())
    }
    fn paste(&mut self, text: Vec<u8>) {
        sex_pdx::serial_record!("[term.paste] bytes={}", text.len());
        if let Some(job) = self.job.as_mut() {
            let bytes = keys::paste(&text, false);
            for b in bytes { if b == b'\r' { job.line.push(b'\n'); let l = core::mem::take(&mut job.line); match &job.runner { Runner::Guest(g) => { let _ = g.stream.write(Side::Host, &l); } Runner::Native { .. } => { let _ = files::direct::relay::put(&l); } } } else { job.line.push(b); } }
            let shown = job.line.clone();
            self.out(&shown);
            return;
        }
        // Pasted text never runs on its own: newlines become spaces on the command line.
        for b in text { self.line.insert(self.at, if b == b'\n' || b == b'\r' { b' ' } else { b }); self.at += 1; }
        self.redraw_line();
    }
    /// The mouse, at a point inside the pane. Returns text to put on the clipboard.
    fn pointer(&mut self, p: sex_pdx::app_pointer::Pointer, clipboard: &[u8]) -> Option<Vec<u8>> {
        let (cw, ch) = (self.painter.cell_w, self.painter.cell_h);
        let (col, row) = ((p.x.max(0) as usize) / cw, (p.y.max(0) as usize) / ch);
        let at = self.screen.point_at(col, row);
        match p.kind {
            Kind::Wheel { steps } => self.screen.scroll_view(steps as isize * 3),
            Kind::Down { button: 0 } => {
                let now = sex_pdx::get_ticks();
                let double = now.saturating_sub(self.last_click.0) < 400 && (self.last_click.1, self.last_click.2) == (col, row);
                self.last_click = (now, col, row);
                if double { let (a, b) = self.screen.word_at(at); self.screen.select(a, b); self.drag = None; }
                else { self.screen.clear_selection(); self.drag = Some(at); }
            }
            Kind::Move if p.buttons & 1 != 0 => if let Some(from) = self.drag { self.screen.select(from, at); },
            Kind::Up { button: 0 } => {
                self.drag = None;
                // Selecting copies, as X terminals do; Ctrl+Shift+V pastes it.
                return self.copy();
            }
            // The middle button pastes.
            Kind::Down { button: 1 } => self.paste(clipboard.to_vec()),
            _ => {}
        }
        None
    }
}

/// The guest's side of the stream, on the worker.
struct StreamHost { stream: Stream, idle: u32 }
impl StreamHost {
    fn say(&self, text: &str) { let _ = self.stream.write(Side::Guest, text.as_bytes()); }
}
impl sex_wasm::Host for StreamHost {
    fn show(&mut self, _text: &[u8]) {}
    fn log(&mut self, line: &[u8]) { sex_pdx::serial_record!("[term.guest.err] {}", core::str::from_utf8(line).unwrap_or("?")); }
    fn console(&mut self, mut bytes: &[u8]) -> bool {
        // A full ring waits for the screen to catch up; a closed terminal drops output.
        while !bytes.is_empty() && !self.stream.gone(Side::Host) {
            let n = self.stream.write(Side::Guest, bytes);
            bytes = &bytes[n..];
            if n == 0 { self.idle += 1; sex_app::thread::pause(self.idle); } else { self.idle = 0; }
        }
        true
    }
    fn input(&mut self, out: &mut [u8]) -> Option<usize> {
        loop {
            let n = self.stream.read(Side::Guest, out);
            if n > 0 { self.idle = 0; return Some(n); }
            if self.stream.gone(Side::Host) { return Some(0); }
            self.idle += 1;
            sex_app::thread::pause(self.idle.min(64));
        }
    }
    fn read(&mut self, path: &[u8], out: &mut [u8]) -> Result<usize, sex_wasm::Refusal> {
        files::direct::read_at(path, 0, out).map_err(refusal)
    }
    fn write(&mut self, path: &[u8], data: &[u8]) -> Result<(), sex_wasm::Refusal> {
        let _ = files::direct::create(path);
        files::direct::truncate(path, 0).map_err(refusal)?;
        files::direct::write_at(path, 0, data).map(|_| ()).map_err(refusal)
    }
    fn now(&mut self) -> u64 { sex_pdx::get_ticks().saturating_mul(1_000_000) }
}
fn refusal(e: files::Error) -> sex_wasm::Refusal {
    use sex_pdx::fs as w;
    match e {
        w::NOT_FOUND => sex_wasm::Refusal::NotFound,
        w::DENIED => sex_wasm::Refusal::NotAllowed,
        _ => sex_wasm::Refusal::Failed,
    }
}

fn nav_key(key: sex_pdx::app_key::Key) -> Option<Key> {
    Some(match (sex_app::nav(key), key.code) {
        (Some(Nav::Up), _) => Key::Up, (Some(Nav::Down), _) => Key::Down,
        (Some(Nav::Left), _) => Key::Left, (Some(Nav::Right), _) => Key::Right,
        (Some(Nav::Home), _) => Key::Home, (Some(Nav::End), _) => Key::End,
        (Some(Nav::Delete), _) => Key::Delete,
        (_, 0x49 | 0x68) => Key::PageUp, (_, 0x51 | 0x6D) => Key::PageDown,
        (_, 0x01) => Key::Escape, (_, 0x0F) => Key::Tab,
        (_, c @ 0x3B..=0x44) => Key::F(c - 0x3A),
        _ => return None,
    })
}

fn read_all(window: &mut Window, path: &[u8], max: usize) -> Result<Vec<u8>, files::Error> {
    let st = files::stat(window, path)?;
    if st.dir { return Err(sex_pdx::fs::IS_DIR); }
    let len = (st.size as usize).min(max);
    let mut data = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let n = files::read_at(window, path, at as u64, &mut data[at..])?;
        if n == 0 { break; }
        at += n;
    }
    data.truncate(at);
    Ok(data)
}
fn size(n: u64) -> String {
    if n < 1024 { format!("{} B", n) } else if n < 1 << 20 { format!("{} KiB", n >> 10) } else { format!("{} MiB", n >> 20) }
}
/// Sixteen colours, a slice of the 256-colour cube and a 24-bit ramp.
fn colors() -> String {
    let mut s = String::new();
    for i in 0..16 { s.push_str(&format!("\x1b[48;5;{}m  {:2}  ", i, i)); if i == 7 { s.push_str("\x1b[0m\n"); } }
    s.push_str("\x1b[0m\n");
    for i in 16..52 { s.push_str(&format!("\x1b[48;5;{}m ", i)); }
    s.push_str("\x1b[0m\n");
    for i in 0..48 { let v = i * 255 / 47; s.push_str(&format!("\x1b[48;2;{};{};{}m ", v, 64, 255 - v)); }
    s.push_str("\x1b[0m\n\x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[7minverse\x1b[0m \x1b[9mstrike\x1b[0m\n");
    s
}

const BUILTINS: &[&[u8]] = &[b"cat", b"cd", b"clear", b"colors", b"echo", b"exit", b"greet", b"help", b"history", b"ls", b"mkdir", b"mv", b"pwd", b"rm", b"snapshot", b"touch", b"vtcheck", b"write"];
const HELP: &str = "\x1b[1mSpindle\x1b[0m: the SexOS terminal. Each pane has its own command line.\n\
  ls cd pwd cat mkdir rm mv touch write FILE TEXT   files in this folder\n\
  echo history clear colors vtcheck snapshot exit\n\
  NAME [ARGS]    runs NAME.wasm from this folder (a WASI command-line program), else\n\
                 the program NAME (python, lua, rg, fd...) with this pane as its console\n\
  greet [NAME]   a built-in Rust program, compiled to Wasm\n\
Keys: Tab completes, Up/Down recall, Ctrl+Shift+C/V copy/paste, Shift+PgUp/PgDn scroll,\n\
  Ctrl+=/- text size, Ctrl+C stop, Ctrl+D end input (closes an idle pane), Ctrl+L clear.\n\
Tabs: Ctrl+Shift+T new, Ctrl+PgUp/PgDn switch, Ctrl+Shift+PgUp/PgDn move, Ctrl+Shift+1..9 pick,\n\
  click a tab to pick it, middle-click to close it.\n\
Panes: Ctrl+Shift+E split side by side, Ctrl+Shift+O split below, Ctrl+Shift+arrows move,\n\
  Ctrl+Shift+Z zoom, Ctrl+Shift+R then arrows resize (any other key ends), Ctrl+Shift+[ even out,\n\
  Ctrl+Shift+W close; drag a divider with the mouse. F11 closes the window.\n";

/// Spindle's terminal window: tabs of split panes, a clipboard they share, and the text
/// size they all use.
struct Spindle {
    panes: BTreeMap<PaneId, Pane>,
    tabs: Tabs,
    next_id: PaneId,
    width: usize,
    height: usize,
    cell_h: usize,
    clipboard: Vec<u8>,
    /// The layout changed: every pane is drawn and copied again.
    relayout: bool,
    /// A divider being dragged.
    dragging: Option<Divider>,
    /// Resize mode (Ctrl+Shift+R): arrows move the focused pane's dividers.
    resizing: bool,
    title: String,
    painter: Painter,
}

/// Pixels between panes, and the colours of the window's own parts.
const GAP: usize = 2;
const DIVIDER: u32 = 0x313244;
const DIVIDER_FOCUS: u32 = 0x89B4FA;
const TAB_BG: u32 = 0x181825;
const TAB_ACTIVE: u32 = 0x313244;
const TAB_TEXT: u32 = 0xA6ADC8;
const TAB_TEXT_ACTIVE: u32 = 0xCDD6F4;

impl Spindle {
    fn new(width: usize, height: usize) -> Spindle {
        let cell_h = 18;
        let (cw, ch) = cell_for_height(14);
        let mut s = Spindle {
            panes: BTreeMap::new(), tabs: Tabs::new(1), next_id: 2, width: width.max(64), height: height.max(64), cell_h,
            clipboard: Vec::new(), relayout: true, dragging: None, resizing: false, title: String::new(), painter: Painter::new(cw, ch),
        };
        let area = s.area();
        s.panes.insert(1, Pane::new(area.w, area.h, cell_h));
        s
    }
    /// The height of the tab bar: shown once there are two tabs.
    fn bar_h(&self) -> usize { if self.tabs.len() > 1 { self.painter.cell_h + 8 } else { 0 } }
    /// Where the panes go.
    fn area(&self) -> Rect { let b = self.bar_h(); Rect::new(0, b, self.width, self.height.saturating_sub(b)) }
    fn focused(&mut self) -> &mut Pane { let id = self.tabs.focus(); self.panes.get_mut(&id).expect("the focused pane exists") }
    fn new_pane(&mut self, cwd: Vec<u8>) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        let area = self.area();
        let mut pane = Pane::new(area.w / 2, area.h / 2, self.cell_h);
        pane.cwd = cwd;
        pane.show_prompt();
        self.panes.insert(id, pane);
        id
    }
    fn open_tab(&mut self) {
        let cwd = self.focused().cwd.clone();
        let id = self.new_pane(cwd);
        self.tabs.open(id);
        self.relayout = true;
        sex_pdx::serial_record!("[term.tab.open] tabs={} pane={}", self.tabs.len(), id);
    }
    fn split(&mut self, split: Split) {
        let (from, cwd) = (self.tabs.focus(), self.focused().cwd.clone());
        let id = self.new_pane(cwd);
        self.tabs.current_mut().split(from, split, id);
        self.relayout = true;
        sex_pdx::serial_record!("[term.pane.split] from={} pane={} across={}", from, id, (split == Split::Across) as u8);
    }
    /// Closes a pane, stopping what runs in it. The window closes with its last pane.
    fn close_pane(&mut self, id: PaneId) {
        if let Some(mut pane) = self.panes.remove(&id) { pane.stop(); }
        let area = self.area();
        self.tabs.remove_pane(id, area);
        sex_pdx::serial_record!("[term.pane.gone] pane={} panes={} tabs={}", id, self.panes.len(), self.tabs.len());
        if self.tabs.is_empty() { sex_pdx::serial_record!("[term.closed] how=last-pane"); sex_pdx::task_finish(0) }
        self.relayout = true;
    }
    fn set_text_size(&mut self, step: isize) {
        let h = (self.cell_h as isize + step).clamp(10, 48) as usize;
        if h == self.cell_h { return; }
        self.cell_h = h;
        for pane in self.panes.values_mut() { pane.set_cell_height(h); }
        self.relayout = true;
    }

    /// The window's own keys; false when the key belongs to the focused pane.
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> bool {
        let area = self.area();
        if self.resizing {
            let toward = match sex_app::nav(key) { Some(Nav::Left) => Some(Toward::Left), Some(Nav::Right) => Some(Toward::Right), Some(Nav::Up) => Some(Toward::Up), Some(Nav::Down) => Some(Toward::Down), _ => None };
            match toward {
                Some(t) => { if self.tabs.current_mut().resize(t, 50) { self.relayout = true; } }
                None => { self.resizing = false; sex_pdx::serial_record!("[term.resize.mode] on=0"); }
            }
            return true;
        }
        let toward = |key: sex_pdx::app_key::Key| match sex_app::nav(key) { Some(Nav::Left) => Some(Toward::Left), Some(Nav::Right) => Some(Toward::Right), Some(Nav::Up) => Some(Toward::Up), Some(Nav::Down) => Some(Toward::Down), _ => None };
        match (key.control, key.shift, key.code) {
            (true, true, 0x2E) => { if let Some(t) = self.focused().copy() { self.clipboard = t; } }
            (true, true, 0x2F) => { let c = self.clipboard.clone(); self.focused().paste(c); }
            (true, false, 0x0D) => self.set_text_size(2),
            (true, false, 0x0C) => self.set_text_size(-2),
            // Tabs: Ctrl+Shift+T opens, Ctrl+PageUp/PageDown switch, with Shift they move,
            // Ctrl+Shift+1..9 pick one.
            (true, true, 0x14) => self.open_tab(),
            (true, false, 0x49 | 0x68) => { self.tabs.next(false); self.relayout = true; }
            (true, false, 0x51 | 0x6D) => { self.tabs.next(true); self.relayout = true; }
            (true, true, 0x49 | 0x68) => { self.tabs.shift(false); self.relayout = true; }
            (true, true, 0x51 | 0x6D) => { self.tabs.shift(true); self.relayout = true; }
            (true, true, c @ 0x02..=0x0A) => { self.tabs.select((c - 2) as usize); self.relayout = true; }
            // Panes: Ctrl+Shift+E splits side by side, Ctrl+Shift+O one above the other,
            // Ctrl+Shift+arrows move between them, Ctrl+Shift+Z zooms one, Ctrl+Shift+R
            // resizes with the arrows, Ctrl+Shift+W closes one.
            (true, true, 0x12) => self.split(Split::Across),
            (true, true, 0x18) => self.split(Split::Down),
            (true, true, 0x2C) => { let t = self.tabs.current_mut(); if t.count() > 1 { t.zoomed = !t.zoomed; self.relayout = true; } }
            (true, true, 0x13) => { self.resizing = true; sex_pdx::serial_record!("[term.resize.mode] on=1"); }
            (true, true, 0x11) => { let id = self.tabs.focus(); self.close_pane(id); }
            (true, true, 0x1A) => { self.tabs.current_mut().equalize(); self.relayout = true; }
            (true, true, _) if toward(key).is_some() => {
                if self.tabs.current_mut().focus_toward(area, toward(key).unwrap()) { self.relayout = true; }
            }
            _ => { let _ = window; return false; }
        }
        true
    }

    /// The mouse: the tab bar, dividers, and the panes (which get the pointer in their
    /// own coordinates; pressing in a pane focuses it).
    fn pointer(&mut self, mut p: sex_pdx::app_pointer::Pointer) {
        let (x, y) = (p.x.max(0) as usize, p.y.max(0) as usize);
        let area = self.area();
        if y < self.bar_h() {
            if let Kind::Down { button } = p.kind {
                if let Some(i) = self.tab_at(x) {
                    if button == 1 { let panes = self.tabs.close(i); for id in panes { if let Some(mut pane) = self.panes.remove(&id) { pane.stop(); } } if self.tabs.is_empty() { sex_pdx::task_finish(0) } }
                    else { self.tabs.select(i); }
                    self.relayout = true;
                } else if button == 0 { self.open_tab(); }
            }
            return;
        }
        if let Some(d) = &self.dragging {
            match p.kind {
                Kind::Move => { let d = d.clone(); self.tabs.current_mut().drag_divider(&d, x, y); self.relayout = true; }
                Kind::Up { .. } => self.dragging = None,
                _ => {}
            }
            return;
        }
        if let Kind::Down { button: 0 } = p.kind {
            if let Some(d) = self.tabs.current().divider_at(area, GAP, x, y, 3) { self.dragging = Some(d); return; }
        }
        let tab = self.tabs.current();
        // A press picks the pane; moves and releases go to the pane being dragged in.
        let target = if matches!(p.kind, Kind::Down { .. } | Kind::Wheel { .. }) { tab.pane_at(area, GAP, x, y) } else { Some(tab.focus) };
        let Some(id) = target else { return };
        if matches!(p.kind, Kind::Down { .. }) && id != self.tabs.focus() { self.tabs.current_mut().focus = id; self.relayout = true; }
        let Some(r) = self.tabs.current().rect_of(id, area, GAP) else { return };
        p.x = x.saturating_sub(r.x) as i32 as _;
        p.y = y.saturating_sub(r.y) as i32 as _;
        let clip = self.clipboard.clone();
        if let Some(pane) = self.panes.get_mut(&id) { if let Some(t) = pane.pointer(p, &clip) { self.clipboard = t; } }
    }
    /// The tab under `x` in the tab bar, or None past the last (the "new tab" space).
    fn tab_at(&self, x: usize) -> Option<usize> {
        let n = self.tabs.len();
        let w = (self.width / n.max(1)).min(240).max(1);
        (x / w < n).then_some(x / w)
    }

    /// Pumps every pane's program; closes panes whose shell ended.
    fn pump(&mut self) {
        let mut gone = Vec::new();
        for (id, pane) in self.panes.iter_mut() {
            pane.pump();
            if pane.screen.take_bells() > 0 { sex_pdx::serial_record!("[term.bell] pane={}", id); }
            if pane.closed { gone.push(*id); }
        }
        for id in gone { self.close_pane(id); }
    }
    fn running(&self) -> bool { self.panes.values().any(|p| p.job.is_some()) }

    fn draw(&mut self, window: &mut Window) {
        let area = self.area();
        let (w, h) = (self.width, self.height);
        let pixels = window.pixels();
        if pixels.len() < w * h { return; }
        let relayout = core::mem::take(&mut self.relayout);
        let tab = self.tabs.current().clone();
        let layout = tab.layout(area, GAP);
        let mut changed = relayout;
        if relayout {
            pixels[..w * h].fill(sex_vt::DEFAULT_BG);
            let rects: Vec<String> = layout.iter().map(|(id, r)| format!("{}:{}x{}+{}+{}", id, r.w, r.h, r.x, r.y)).collect();
            sex_pdx::serial_record!("[term.panes] tabs={} active={} focus={} zoomed={} {}", self.tabs.len(), self.tabs.active, tab.focus, tab.zoomed as u8, rects.join(" "));
        }
        for (id, r) in &layout {
            let Some(pane) = self.panes.get_mut(id) else { continue };
            if relayout { pane.resize(r.w, r.h); pane.screen.mark_all_dirty(); }
            let focused = *id == tab.focus;
            if !pane.draw(focused) { continue; }
            changed = true;
            // Copy the pane's pixels into its place.
            let (pw, ph) = (pane.width.min(r.w), pane.height.min(r.h));
            for row in 0..ph {
                let at = (r.y + row) * w + r.x;
                if at + pw > pixels.len() { break; }
                pixels[at..at + pw].copy_from_slice(&pane.buf[row * pane.width..row * pane.width + pw]);
            }
        }
        if changed {
            // The dividers, the one beside the focused pane highlighted.
            let focus_rect = tab.rect_of(tab.focus, area, GAP);
            for d in tab.dividers(area, GAP) {
                let near = focus_rect.is_some_and(|f| d.x + d.w >= f.x && d.x <= f.x + f.w && d.y + d.h >= f.y && d.y <= f.y + f.h);
                for row in d.y..(d.y + d.h).min(h) { let at = row * w; pixels[at + d.x..at + (d.x + d.w).min(w)].fill(if near && layout.len() > 1 { DIVIDER_FOCUS } else { DIVIDER }); }
            }
            if relayout { self.draw_tab_bar(pixels); }
            window.show_pixels(w, h);
        }
        // The window's title: the focused pane's, and how many tabs there are.
        let focused = self.panes.get(&tab.focus).map(|p| p.title()).unwrap_or_default();
        let title = if self.tabs.len() > 1 { format!("Spindle [{}/{}]: {}", self.tabs.active + 1, self.tabs.len(), focused) } else { format!("Spindle: {}", focused) };
        if title != self.title { window.title(title.as_bytes()); self.title = title; }
    }
    fn draw_tab_bar(&mut self, pixels: &mut [u32]) {
        let bar = self.bar_h();
        if bar == 0 { return; }
        let w = self.width;
        pixels[..bar * w].fill(TAB_BG);
        let n = self.tabs.len();
        let tw = (w / n).min(240);
        let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
        for i in 0..n {
            let active = i == self.tabs.active;
            let x0 = i * tw;
            let bg = if active { TAB_ACTIVE } else { TAB_BG };
            for row in 2..bar { pixels[row * w + x0..row * w + (x0 + tw - 2).min(w)].fill(bg); }
            let tab = &self.tabs.tabs[i];
            let title = self.panes.get(&tab.focus).map(|p| p.title()).unwrap_or_default();
            let panes = tab.count();
            let label = if panes > 1 { format!("{} {} [{}]", i + 1, title, panes) } else { format!("{} {}", i + 1, title) };
            let fit = (tw.saturating_sub(16)) / self.painter.cell_w;
            let label: String = label.chars().take(fit).collect();
            self.painter.text(cache, pixels, w, x0 + 8, 5, &label, if active { TAB_TEXT_ACTIVE } else { TAB_TEXT }, bg);
            if active { pixels[(bar - 2) * w + x0..(bar - 2) * w + (x0 + tw - 2).min(w)].fill(DIVIDER_FOCUS); }
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Spindle");
    let (w, h) = window.size();
    let mut spindle = Spindle::new(w, h);
    let term = spindle.focused();
    term.say("\x1b[1mSpindle\x1b[0m. Type \x1b[1mhelp\x1b[0m for commands and keys. Ctrl+Shift+T opens a tab, Ctrl+Shift+E splits.\n");
    // `run term CMD...` runs that command first.
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    // Spindle's grant (`spindle` adds `+launch`) is not a command.
    let given: Vec<u8> = files::args(&mut window, &mut args).split(|b| *b == b' ')
        .filter(|w| !w.is_empty() && *w != sex_pdx::console_relay::GRANT_TOKEN).collect::<Vec<_>>().join(&b' ');
    let term = spindle.focused();
    term.show_prompt();
    if !given.is_empty() {
        term.line = given.clone();
        term.at = given.len();
        term.redraw_line();
        let line = core::mem::take(&mut term.line);
        term.say("\r\n");
        term.run_line(&mut window, &line);
        if term.job.is_none() { term.show_prompt(); }
    }
    spindle.draw(&mut window);
    let (cols, rows) = (spindle.focused().screen.cols(), spindle.focused().screen.rows());
    sex_pdx::serial_record!("[term.ready] target={} task={} cols={} rows={}", window.target, window.task, cols, rows);
    loop {
        // While a program runs, look at its output often; otherwise sleep until an event.
        let event = if spindle.running() { window.next_within(16) } else { Some(window.next()) };
        match event {
            Some(Event::Close) => { sex_pdx::serial_record!("[term.closed] how=window"); window.close() }
            Some(Event::Resize { width, height }) => { spindle.width = width.max(64); spindle.height = height.max(64); spindle.relayout = true; }
            Some(Event::Key(key)) => if !spindle.key(&mut window, key) { spindle.focused().key(&mut window, key) },
            Some(Event::Pointer(p)) => spindle.pointer(p),
            None => {}
        }
        spindle.pump();
        spindle.draw(&mut window);
    }
}
