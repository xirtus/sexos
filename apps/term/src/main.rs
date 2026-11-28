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
//! arguments and no network, Ctrl+C stops it and Ctrl+D ends its input. It reads the
//! screen's size, may ask for raw keys or no echo (an editor, a password), and a second
//! Ctrl+C within a second (or Ctrl+\) stops a run that ignores the first, through Shell.
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
use sex_vt::render::{cell_for_height, Cache, Faces, Family, Glide, Painter};
use sex_vt::{Parser, Screen};
mod shell_host;
use spindle_panes::{Divider, PaneId, Rect, Split, Tabs, Toward};
use spindle_shell::ergonomics::SHELL_EXTRA_CMDS;
use spindle_shell::shell_ui::{self, FuzzyMatch};
use spindle_shell::tui::{ActiveTui, BarsApp, BarsMode, BtopApp, TuiExit, TuiKey, TuiOutput, YaziApp};
use spindle_shell::{Completion, Host};

/// `crates/sex-wasm/guests/greet`: Rust's std built for wasm32-wasip1.
const GREET: &[u8] = include_bytes!("../../../crates/sex-wasm/tests/greet.wasm");
const MODULE_MAX: usize = 8 << 20;
/// Spindle's settings, in its folder.
const CONFIG: &[u8] = b"spindle.conf";
const STREAM_RING: usize = 64 * 1024;
static mut CACHE: Cache = Cache::new();
/// Each font family's cuts (regular, bold, italic) and the Noto fallbacks (CJK, emoji,
/// symbols), parsed once for every pane when the family is first used: Geist Mono, and
/// JetBrains Mono with its ligatures when `font = jetbrains-mono`.
static mut FACES: [core::mem::MaybeUninit<Faces>; 2] = [const { core::mem::MaybeUninit::uninit() }; 2];
/// Per family: 0 not tried, 1 parsed, 2 failed.
static mut FACES_STATE: [u8; 2] = [0; 2];
/// The family new painters draw with.
static mut FAMILY: Family = Family::GeistMono;
/// SSH domains from `spindle.conf` (`ssh.NAME = user@host:port`), for `ssh NAME`.
static mut DOMAINS: Vec<spindle_panes::config::SshDomain> = Vec::new();
/// Programs that reach the network when a pane runs them: the person typing `ssh` asks
/// for exactly that. Shell passes the network on only because this window has it.
const NETWORK_PROGRAMS: &[&[u8]] = &[b"ssh"];

/// `ssh NAME [COMMAND]` with NAME an SSH domain: the arguments `ssh` itself takes.
fn ssh_domain_args(args: &[&[u8]]) -> Option<Vec<Vec<u8>>> {
    let first = core::str::from_utf8(args.first()?).ok()?;
    // SAFETY: one thread; check_config writes DOMAINS between frames.
    let d = unsafe { (*core::ptr::addr_of!(DOMAINS)).iter().find(|d| d.name == first)?.clone() };
    let mut out = vec![b"-p".to_vec(), format!("{}", d.port).into_bytes(), format!("{}@{}", d.user, d.host).into_bytes()];
    out.extend(args[1..].iter().map(|a| a.to_vec()));
    Some(out)
}
fn painter(cw: usize, ch: usize) -> Painter {
    // SAFETY: one thread; a slot is written once, before its state says it is parsed.
    unsafe {
        let family = FAMILY;
        let i = family as usize;
        if FACES_STATE[i] == 0 {
            let ok = Faces::family_into(&mut (*core::ptr::addr_of_mut!(FACES))[i], family);
            FACES_STATE[i] = if ok { 1 } else { 2 };
            sex_pdx::serial_record!("[term.faces] font={} parsed={}", family.name(), ok as u8);
        }
        let p = Painter::new(cw, ch);
        if FACES_STATE[i] == 1 { p.with_faces((*core::ptr::addr_of!(FACES))[i].assume_init_ref()) } else { p }
    }
}

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
enum Runner { Guest(Guest), Native(Native) }
/// A program of its own: when it started, its terminal mode (`console_relay::RAW` and the
/// rest, from TAKE), its target, when Ctrl+C last interrupted it, and its command line
/// (a yarn's label if it outlives the window).
struct Native { since: u64, mode: u8, target: u32, interrupted_at: Option<u64>, stopping: bool, label: String }
struct Job {
    name: String,
    runner: Runner,
    /// The line being typed for it, echoed here and sent on Enter.
    line: Vec<u8>,
    /// Input it has not taken yet (its buffer was full), sent as it reads.
    pending: Vec<u8>,
    /// The end of its input, sent once `pending` is gone.
    eof: bool,
}
/// How long a program may take to start before Term gives up on it.
const START_MS: u64 = 20_000;

/// A program the shell started and waits on.
struct ShellWait { capture: bool, output: Vec<u8>, line: String }

/// Commands of the terminal itself, before the shell: they draw on its screen or close it.
const OWN: &[&[u8]] = &[b"theme", b"help", b"clear", b"history", b"colors", b"vtcheck", b"snapshot", b"exit", b"write", b"cat", b"attach"];

/// A way of reading the scrollback that takes the keys for a while.
enum Mode {
    Normal,
    /// Ctrl+Shift+F: a search typed at the bottom of the pane; matches are highlighted.
    Search { query: String, regex: bool, matches: Vec<sex_vt::Match>, current: Option<usize> },
    /// Ctrl+Shift+Space: labels over URLs, paths, hashes...; typing one copies it.
    Hints { hints: Vec<sex_vt::hints::Hint>, typed: String },
    /// Ctrl+Shift+X: a keyboard cursor over the scrollback with vi keys.
    Copy(sex_vt::copy_mode::CopyMode),
}

/// Active Fish multi-column Tab completion drawer below the prompt.
struct CompletionDrawer {
    hits: Vec<Completion>,
    selected: usize,
}

/// Active `Ctrl+R` fuzzy reverse history search drawer below the prompt.
struct RevSearchDrawer {
    query: String,
    matches: Vec<FuzzyMatch>,
    selected: usize,
    saved_line: Vec<u8>,
    saved_at: usize,
}

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
    recall_prefix: Vec<u8>,
    complete_menu: Option<CompletionDrawer>,
    rev_search: Option<RevSearchDrawer>,
    tui: Option<ActiveTui>,
    last_cmd_ms: Option<u64>,
    last_exit: i32,
    job: Option<Job>,
    /// Where a mouse drag started.
    drag: Option<sex_vt::Point>,
    /// Whether the last drawing showed the cursor as focused.
    focused_drawn: bool,
    /// The title the program set (OSC 0/2).
    osc_title: Option<String>,
    /// The folder the program said it is in (OSC 7), and its progress (OSC 9;4).
    osc_cwd: Option<String>,
    progress: Option<u8>,
    /// Search, hints or copy mode, when one is on.
    mode: Mode,
    /// Where the command line's prompt starts: absolute row (history + screen), column.
    prompt_at: (usize, usize),
    /// The pipeline shell (`spindle-shell`) and what it waits for: a program it started,
    /// whose output it takes (`capture`) or leaves on the screen.
    engine: spindle_shell::Engine,
    shell_wait: Option<ShellWait>,
    /// Which of the window's console relay slots this pane's programs use (one per pane,
    /// up to `console_relay::SLOTS` panes running programs of their own at once).
    slot: u8,
    last_click: (u64, usize, usize),
    /// A carriage return was the last byte fed, so a following newline is not doubled.
    after_cr: bool,
    /// Smooth scrolling: the view already moved, and what is shown glides after it.
    glide: Glide,
}


impl Pane {
    fn new(width: usize, height: usize, cell_h: usize) -> Pane {
        let (cw, ch) = cell_for_height(cell_h);
        let painter = painter(cw, ch);
        let (width, height) = (width.max(32), height.max(32));
        let (cols, rows) = painter.grid(width, height);
        Pane {
            screen: Screen::new(cols, rows), parser: Parser::new(), painter, width, height, buf: vec![0; width * height], cell_h, closed: false,
            cwd: Vec::new(), line: Vec::new(), at: 0, history: Vec::new(), recall: None, recall_prefix: Vec::new(),
            complete_menu: None, rev_search: None, tui: None, last_cmd_ms: None, last_exit: 0,
            job: None, drag: None, focused_drawn: false, osc_title: None, osc_cwd: None, progress: None, mode: Mode::Normal, prompt_at: (0, 0), slot: 0, engine: spindle_shell::Engine::new(), shell_wait: None, last_click: (0, usize::MAX, usize::MAX), after_cr: false, glide: Glide::default(),
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
        self.buf = vec![self.screen.theme().bg; width * height];
        self.glide = Glide::default();
        let (cols, rows) = self.painter.grid(self.width, self.height);
        self.screen.resize(cols, rows);
        if let Some(Job { runner: Runner::Guest(g), .. }) = &self.job { g.stream.set_size(cols as u32, rows as u32); }
        if let Some(Job { runner: Runner::Native(_), .. }) = &self.job { self.native_size(); }
        if self.tui.is_some() {
            let now = sex_pdx::get_ticks();
            let host = shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot };
            let out = self.tui.as_mut().unwrap().resize(&host, cols, rows, now);
            self.apply_tui_output(out);
        }
        self.screen.mark_all_dirty();
    }
    /// Draws with the font family now chosen, at the same size.
    fn new_font(&mut self) {
        let (cw, ch) = cell_for_height(self.cell_h);
        self.painter = painter(cw, ch);
        self.screen.mark_all_dirty();
    }
    /// A new text size: `h` pixels per cell.
    fn set_cell_height(&mut self, h: usize) {
        if h == self.cell_h { return; }
        self.cell_h = h;
        let (cw, ch) = cell_for_height(h);
        self.painter = painter(cw, ch);
        let (w, h) = (self.width, self.height);
        self.width = 0;
        self.resize(w, h);
    }
    /// Draws the rows that changed into the pane's pixels. True if any did.
    fn draw(&mut self, focused: bool) -> bool {
        if let Some(t) = self.screen.take_title() { self.osc_title = Some(String::from_utf8_lossy(t).into_owned()); }
        let dirty = self.screen.take_dirty();
        let gliding = self.glide.left != 0 || !self.glide.shown.is_empty();
        if !dirty.iter().any(|d| *d) && self.focused_drawn == focused && !gliding { return false; }
        let dirty = if self.focused_drawn != focused { Vec::new() } else { dirty };
        self.focused_drawn = focused;
        let (w, h) = (self.width, self.height);
        let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
        self.painter.draw(&self.screen, &dirty, cache, &mut self.buf[..w * h], w, focused);
        // Below the last whole row, the terminal's background.
        let used = self.screen.rows() * self.painter.cell_h;
        if used < h { let bg = self.screen.theme().bg; self.buf[used * w..h * w].fill(bg); }
        self.draw_mode(cache);
        if self.glide.left != 0 {
            self.glide.compose(&self.buf, w, h);
            self.glide.step();
        } else if !self.glide.shown.is_empty() {
            // The glide ended: the pane's own pixels are shown again.
            self.glide.shown = Vec::new();
            self.glide.from = Vec::new();
        }
        true
    }
    /// What the window shows for this pane: the glide while one runs, else its pixels.
    fn shown(&self) -> &[u32] { if self.glide.active() && !self.glide.shown.is_empty() { &self.glide.shown } else { &self.buf } }
    /// Scrolls the view `lines` back (negative: forward), gliding there over a few frames.
    fn scroll_smoothly(&mut self, lines: isize) {
        let before = self.screen.view_offset() as isize;
        self.screen.scroll_view(lines);
        let moved = self.screen.view_offset() as isize - before;
        if moved == 0 { return; }
        let px = moved as i32 * self.painter.cell_h as i32;
        let h = self.height as i32;
        let on_screen = self.shown().to_vec();
        if !self.glide.retarget(px, &on_screen, h) { return; }
        let left = self.glide.left;
        sex_pdx::serial_record!("[term.glide] lines={} px={}", moved, left);
    }
    /// What the current mode shows over the screen: hint labels, or a status line.
    fn draw_mode(&mut self, cache: &mut Cache) {
        let (w, cols, rows) = (self.width, self.screen.cols(), self.screen.rows());
        let status = match &self.mode {
            Mode::Normal => return,
            Mode::Hints { hints, typed } => {
                self.painter.draw_hints(hints, typed, cache, &mut self.buf, w, cols, rows);
                format!(" hints: type a label to copy it ({} found), Esc leaves ", hints.len())
            }
            Mode::Search { query, regex, matches, current } => {
                let at = current.map(|c| c + 1).unwrap_or(0);
                format!(" find{}: {}_   {}/{}   Enter older, Shift+Enter newer, Ctrl+R regex, Esc leaves ", if *regex { " (regex)" } else { "" }, query, at, matches.len())
            }
            Mode::Copy(c) => match c.search_prompt() {
                Some((q, fwd)) => format!(" {}{}_ ", if fwd { '/' } else { '?' }, q),
                None => String::from(" COPY  hjkl/wbe move, v V Ctrl+V select, y copies, / ? n N search, q leaves "),
            },
        };
        // The status line sits on the pane's last row.
        let y0 = (rows - 1) * self.painter.cell_h;
        let text: String = status.chars().take(cols).collect();
        let x = self.painter.text(cache, &mut self.buf, w, 0, y0, &text, 0x11111B, 0xF9E2AF);
        for y in y0..y0 + self.painter.cell_h { if x < w { self.buf[y * w + x..y * w + w].fill(0xF9E2AF); } }
    }
    /// A mode's key. Returns text for the clipboard when the mode yields some.
    fn mode_key(&mut self, key: sex_pdx::app_key::Key) -> Option<Vec<u8>> {
        let escape = key.code == 0x01;
        let mut out = None;
        let mut leave = escape;
        match &mut self.mode {
            Mode::Normal => return None,
            Mode::Hints { hints, typed } => if let Some(c) = sex_app::text(key).filter(|_| !escape) {
                typed.push(c);
                match sex_vt::hints::resolve(hints, typed) {
                    sex_vt::hints::Resolve::Exact(i) => {
                        let h = &hints[i];
                        sex_pdx::serial_record!("[term.hint] kind={:?} text={}", h.kind, h.text);
                        out = Some(h.text.clone().into_bytes());
                        leave = true;
                    }
                    sex_vt::hints::Resolve::Prefix => {}
                    _ => leave = true,
                }
            },
            Mode::Search { query, regex, matches, current } => {
                let mut changed = false;
                match (key.control, key.code) {
                    _ if escape => {}
                    (_, 0x1C) => if !matches.is_empty() {
                        // Enter goes up to older output; Shift+Enter comes back down.
                        let n = matches.len();
                        let c = current.unwrap_or(n);
                        *current = Some(if key.shift { (c + 1).min(n - 1) } else { c.saturating_sub(1) });
                    },
                    (_, 0x0E) => { query.pop(); changed = true; }
                    (true, 0x13) => { *regex = !*regex; changed = true; }
                    (false, _) => if let Some(c) = sex_app::text(key) { query.push(c); changed = true; },
                    _ => {}
                }
                if changed {
                    // Smart case: a capital letter makes the search case-sensitive.
                    let case = query.chars().any(|c| c.is_uppercase());
                    *matches = if query.is_empty() { Vec::new() } else { self.screen.search(query, *regex, case) };
                    *current = matches.len().checked_sub(1);
                    sex_pdx::serial_record!("[term.search] query={} matches={}", query, matches.len());
                }
                let (m, c) = (matches.clone(), *current);
                if let Some(at) = c.and_then(|c| m.get(c)) { self.screen.scroll_to(at.start.row); }
                self.screen.set_highlights(m, c);
            }
            Mode::Copy(cm) => {
                let k = sex_app::text(key).filter(|_| !key.control || key.code != 0x1C).map(Key::Char).or_else(|| nav_key(key)).or(if key.code == 0x1C { Some(Key::Enter) } else if key.code == 0x0E { Some(Key::Backspace) } else { None });
                let k = match (k, key.control) { (None, true) => sex_app::typed(sex_pdx::app_key::Key { control: false, ..key }).map(|c| Key::Char(c as char)), (k, _) => k };
                if let Some(k) = k {
                    match cm.key(&mut self.screen, k, Mods { shift: key.shift, control: key.control, alt: false }) {
                        sex_vt::copy_mode::Action::Yank(t) => { sex_pdx::serial_record!("[term.copy_mode.yank] bytes={}", t.len()); out = Some(t.into_bytes()); }
                        sex_vt::copy_mode::Action::Exit => leave = true,
                        _ => {}
                    }
                    if out.is_some() { self.mode = Mode::Normal; }
                }
            }
        }
        if leave { self.leave_mode(); }
        self.screen.mark_all_dirty();
        out
    }
    fn enter_mode(&mut self, mode: &str) {
        self.leave_mode();
        self.mode = match mode {
            "search" => Mode::Search { query: String::new(), regex: false, matches: Vec::new(), current: None },
            "hints" => {
                let hints = sex_vt::hints::find(&self.screen);
                sex_pdx::serial_record!("[term.hints] found={}", hints.len());
                if hints.is_empty() { return; }
                Mode::Hints { hints, typed: String::new() }
            }
            _ => Mode::Copy(sex_vt::copy_mode::CopyMode::new(&mut self.screen)),
        };
        sex_pdx::serial_record!("[term.mode] {}", mode);
        self.screen.mark_all_dirty();
    }
    fn leave_mode(&mut self) {
        match core::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Copy(mut cm) => cm.exit(&mut self.screen),
            Mode::Search { .. } => self.screen.clear_highlights(),
            _ => {}
        }
        self.screen.mark_all_dirty();
    }
    /// The pane's title: active TUI, what its program asked for, else the running program's
    /// name, else its folder.
    fn title(&self) -> String {
        if let Some(tui) = &self.tui { return String::from(tui.name()); }
        let p = self.progress.map(|p| format!(" [{}%]", p)).unwrap_or_default();
        if let Some(t) = &self.osc_title { return format!("{}{}", t, p); }
        if let Some(job) = &self.job { return format!("{}{}", job.name, p); }
        if let Some(c) = &self.osc_cwd { return c.clone(); }
        format!("~/{}", core::str::from_utf8(&self.cwd).unwrap_or("?"))
    }

    // ---- The command line ----

    /// Powerlevel10k-inspired header row emitted above the editable `╰─ ~/cwd > ` prompt.
    fn say_prompt_header(&mut self) {
        let cols = self.screen.cols();
        if cols < 24 { return; }
        let jobs_n = {
            let mut cwd_copy = self.cwd.clone();
            shell_host::TermHost { cwd: &mut cwd_copy, slot: self.slot }
                .jobs()
                .into_iter()
                .filter(|j| matches!(j.state, spindle_shell::JobState::Running))
                .count()
        };
        let cwd = core::str::from_utf8(&self.cwd).unwrap_or("?");
        let status = if self.last_exit == 0 {
            "\x1b[1;38;2;16;185;129m✓\x1b[0m"
        } else {
            "\x1b[1;38;2;244;63;94m✗\x1b[0m"
        };
        let dur = match self.last_cmd_ms {
            Some(ms) if ms >= 5 => format!(" \x1b[38;2;245;158;11m{}ms\x1b[0m", ms),
            _ => String::new(),
        };
        let job_badge = if jobs_n > 0 { format!(" \x1b[38;2;192;132;252m⚙{}\x1b[0m", jobs_n) } else { String::new() };
        let hdr = format!(
            "\x1b[38;2;100;116;139m╭─ \x1b[1;38;2;244;63;94m● sexos\x1b[38;2;100;116;139m │ \x1b[1;38;2;96;165;250m~/{}\x1b[38;2;100;116;139m │ {}{}{}\x1b[0m\r\n",
            cwd, status, dur, job_badge
        );
        self.say(&hdr);
    }

    fn prompt(&self) -> String {
        let cwd = core::str::from_utf8(&self.cwd).unwrap_or("?");
        format!("\x1b[38;2;100;116;139m╰─ \x1b[1;38;2;96;165;250m~/{}\x1b[0m \x1b[1;38;2;16;185;129m>\x1b[0m ", cwd)
    }

    fn prompt_cell_len(&self) -> usize {
        let cwd = core::str::from_utf8(&self.cwd).unwrap_or("?");
        // "╰─ ~/" (5) + cwd + " > " (3)
        8 + spindle_shell::width::str_width(cwd)
    }

    fn history_strings(&self) -> Vec<String> {
        self.history.iter().map(|l| String::from_utf8_lossy(l).into_owned()).collect()
    }

    fn current_ghost(&self) -> Option<String> {
        if self.tui.is_some() || self.job.is_some() || self.rev_search.is_some() || self.at != self.line.len() || self.line.is_empty() {
            return None;
        }
        let text = core::str::from_utf8(&self.line).ok()?;
        let hist = self.history_strings();
        let mut cwd_copy = self.cwd.clone();
        let host = shell_host::TermHost { cwd: &mut cwd_copy, slot: self.slot };
        shell_ui::ghost_suggestion(text, self.at, &hist, &host, &self.engine, SHELL_EXTRA_CMDS, &spindle_panes::config::THEME_NAMES)
    }

    /// The line being typed, coloured with Fish/Void TrueColor syntax highlighting.
    fn coloured_line(&self) -> String {
        let Ok(text) = core::str::from_utf8(&self.line) else { return String::from_utf8_lossy(&self.line).into_owned() };
        let mut cwd_copy = self.cwd.clone();
        let host = shell_host::TermHost { cwd: &mut cwd_copy, slot: self.slot };
        let mut out = String::new();
        let mut at = 0;
        for (r, style) in self.engine.highlight_with(text, &host, SHELL_EXTRA_CMDS) {
            if r.start < at || r.end > text.len() || !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end) { continue; }
            out.push_str(&text[at..r.start]);
            let sgr = style.ansi_truecolor();
            if !sgr.is_empty() {
                out.push_str(sgr);
                out.push_str(&text[r.start..r.end]);
                out.push_str("\x1b[0m");
            } else {
                out.push_str(&text[r.start..r.end]);
            }
            at = r.end;
        }
        out.push_str(&text[at..]);
        out
    }

    /// Redraws the command line, inline `fg=6` ghost suggestion, and active completion or
    /// `Ctrl+R` reverse-search overlay below the prompt.
    fn redraw_line(&mut self) {
        let p = self.prompt();
        let line = self.coloured_line();
        let ghost = self.current_ghost();
        let ghost_ansi = ghost.as_deref().map(|g| format!("\x1b[38;2;45;180;196m{}\x1b[0m", g)).unwrap_or_default();
        let cols = self.screen.cols();
        let rows_left = self.screen.rows().saturating_sub(2).max(2);
        let drawer = if let Some(menu) = &self.complete_menu {
            shell_ui::format_completion_menu(&menu.hits, menu.selected, cols, rows_left.min(8), true).0
        } else if let Some(rs) = &self.rev_search {
            shell_ui::format_reverse_search(&rs.query, &rs.matches, rs.selected, cols, rows_left.min(8), true).0
        } else {
            String::new()
        };

        let (abs, col) = self.prompt_at;
        let row = abs.saturating_sub(self.screen.history_len());
        self.say(&format!("\x1b[{};{}H\x1b[J{}{}{}{}", row + 1, col + 1, p, line, ghost_ansi, drawer));

        // Position cursor on `self.at`, re-reading `history_len()` in case a bottom drawer scrolled.
        let row = abs.saturating_sub(self.screen.history_len());
        let cursor_str = String::from_utf8_lossy(&self.line[..self.at.min(self.line.len())]);
        let off = col + self.prompt_cell_len() + spindle_shell::width::str_width(&cursor_str);
        let (r, c) = ((row + off / cols).min(self.screen.rows().saturating_sub(1)), off % cols);
        self.say(&format!("\x1b[{};{}H", r + 1, c + 1));
    }

    /// The prompt, marked for shell integration (OSC 133): jumping between prompts and
    /// selecting a command's output work for Spindle's own command line too.
    fn show_prompt(&mut self) {
        self.line.clear();
        self.at = 0;
        self.recall = None;
        self.recall_prefix.clear();
        self.complete_menu = None;
        self.rev_search = None;
        self.say("\x1b]133;A\x07");
        self.say_prompt_header();
        let p = self.prompt();
        let (x, y) = self.screen.cursor();
        self.prompt_at = (self.screen.history_len() + y, x);
        self.say(&p);
        self.say("\x1b]133;B\x07");
    }

    /// A path from the command line, as a path in Term's folder: relative to the current
    /// folder, `/` the folder's root, `..` going up but never above it.
    fn resolve(&self, arg: &[u8]) -> Vec<u8> {
        let mut parts: Vec<&[u8]> = if arg.first() == Some(&b'/') { Vec::new() } else { self.cwd.split(|b| *b == b'/').filter(|p| !p.is_empty()).collect() };
        for part in arg.split(|b| *b == b'/') {
            match part { b"" | b"." => {} b".." => { parts.pop(); } p => parts.push(p) }
        }
        parts.join(&b'/')
    }

    /// Launches interactive Ratatui TUI commands when typed as standalone commands (not piped).
    fn try_start_tui(&mut self, expanded: &str) -> bool {
        if expanded.contains('|') || expanded.contains(';') || expanded.contains('>') || expanded.contains('&') {
            return false;
        }
        let words: Vec<&str> = expanded.split_whitespace().collect();
        let Some(&cmd) = words.first() else { return false };
        if cmd.starts_with('^') {
            return false;
        }
        let args: Vec<String> = words[1..].iter().map(|s| String::from(*s)).collect();
        let now = sex_pdx::get_ticks();
        let (cols, rows) = (self.screen.cols(), self.screen.rows());
        let mut tui = match cmd {
            "bars" | "spin" => ActiveTui::Bars(BarsApp::new(BarsMode::Demo, &args, now)),
            "update" => ActiveTui::Bars(BarsApp::new(BarsMode::Update, &args, now)),
            "install" => ActiveTui::Bars(BarsApp::new(BarsMode::Install, &args, now)),
            "remove" => ActiveTui::Bars(BarsApp::new(BarsMode::Remove, &args, now)),
            "btop" | "htop" | "top" => ActiveTui::Btop(BtopApp::new(now)),
            "yazi" | "y" | "fm" | "ranger" => {
                let host = shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot };
                match YaziApp::new(&host, args.first().map(|s| s.as_str())) {
                    Ok(app) => ActiveTui::Yazi(app),
                    Err(e) => {
                        self.last_exit = 1;
                        self.say(&format!("\x1b[38;2;244;63;94myazi: {}\x1b[0m\n", e));
                        return true;
                    }
                }
            }
            _ => return false,
        };
        sex_pdx::serial_record!("[term.tui.start] name={}", tui.name());
        let host = shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot };
        let out = tui.init(&host, cols, rows, now);
        self.tui = Some(tui);
        self.apply_tui_output(out);
        true
    }

    fn apply_tui_output(&mut self, out: TuiOutput) {
        if !out.bytes.is_empty() {
            self.screen.feed(&mut self.parser, out.bytes.as_bytes());
        }
        if let Some(exit) = out.exit {
            let was_alt = self.tui.as_ref().is_some_and(|t| t.is_alt_screen());
            self.tui = None;
            let code = match exit {
                TuiExit::Done => 0,
                TuiExit::Cd(path) => {
                    let clean = path.trim_start_matches('/').as_bytes().to_vec();
                    self.cwd = clean;
                    0
                }
                TuiExit::Cancel => 0,
            };
            self.last_exit = code;
            if !was_alt && !self.after_cr && self.screen.cursor().0 != 0 {
                self.say("\n");
            }
            self.say(&format!("\x1b]133;D;{}\x07", code));
            self.show_prompt();
        }
    }

    fn run_line(&mut self, window: &mut Window, line: &[u8]) {
        let line = line.trim_ascii();
        if line.is_empty() { return; }
        if self.history.last().map(|l| l.as_slice()) != Some(line) { self.history.push(line.to_vec()); }
        if self.history.len() > 500 { self.history.remove(0); }
        let raw_text = String::from_utf8_lossy(line).into_owned();
        let expanded = self.engine.expand_aliases(&raw_text);
        let started_ms = sex_pdx::get_ticks();
        let trimmed = expanded.trim();
        let words: Vec<&[u8]> = trimmed.as_bytes().split(|b| *b == b' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() { return; }
        let (cmd, args) = (words[0], &words[1..]);

        if self.try_start_tui(trimmed) {
            self.last_cmd_ms = Some(sex_pdx::get_ticks().saturating_sub(started_ms));
            return;
        }
        // `cat -n FILE` or `bat FILE` renders syntax-highlighted numbered output via engine.
        let cat_numbered = cmd == b"cat" && args.iter().any(|a| *a == b"-n" || *a == b"--number");
        if cat_numbered {
            let files: Vec<String> = args.iter().filter(|a| *a != b"-n" && *a != b"--number").map(|a| String::from_utf8_lossy(a).into_owned()).collect();
            if !files.is_empty() {
                let host = shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot };
                let cols = self.screen.cols();
                match spindle_shell::ergonomics::render_bat(&files, &host, cols, true) {
                    Ok(out) => { self.say(&out); self.last_exit = 0; }
                    Err((_, e)) => { self.say(&format!("\x1b[38;2;244;63;94mcat: {}\x1b[0m\n", e)); self.last_exit = 1; }
                }
                self.last_cmd_ms = Some(sex_pdx::get_ticks().saturating_sub(started_ms));
                return;
            }
        }

        // Everything but the terminal's own commands is the shell's: pipelines, tables,
        // files, and programs as stages.
        if !OWN.contains(&cmd) || (cmd == b"help" && !args.is_empty()) {
            let _ = window;
            let text = if cmd == b"help" && args == [b"commands"] { String::from("help") } else { raw_text };
            sex_pdx::serial_record!("[term.shell] {}", text);
            let cols = self.screen.cols();
            let step = self.engine.run(&text, &mut shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot }, cols);
            self.shell_step(step, text);
            self.last_cmd_ms = Some(sex_pdx::get_ticks().saturating_sub(started_ms));
            return;
        }
        sex_pdx::serial_record!("[term.command] {}", core::str::from_utf8(cmd).unwrap_or("?"));
        self.last_exit = 0;
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
                    Ok(_) => { self.say("cd: not a folder\n"); self.last_exit = 1; }
                    Err(e) if target.is_empty() => { let _ = e; self.cwd.clear(); }
                    Err(e) => { self.fail("cd", e); self.last_exit = 1; }
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
                    Err(e) => { self.fail("ls", e); self.last_exit = 1; }
                }
            }
            b"cat" => for a in args {
                let path = self.resolve(a);
                match read_all(window, &path, 4 << 20) { Ok(data) => self.out(&data), Err(e) => { self.fail("cat", e); self.last_exit = 1; } }
            },
            b"mkdir" => for a in args { let p = self.resolve(a); if let Err(e) = files::mkdir(window, &p) { self.fail("mkdir", e); self.last_exit = 1; } },
            b"rm" => for a in args { let p = self.resolve(a); if let Err(e) = files::remove(window, &p) { self.fail("rm", e); self.last_exit = 1; } },
            b"touch" => for a in args { let p = self.resolve(a); if files::stat(window, &p).is_err() { if let Err(e) = files::create(window, &p) { self.fail("touch", e); self.last_exit = 1; } } },
            b"mv" if args.len() == 2 => { let (a, b) = (self.resolve(args[0]), self.resolve(args[1])); if let Err(e) = files::rename(window, &a, &b) { self.fail("mv", e); self.last_exit = 1; } }
            b"write" if !args.is_empty() => {
                let path = self.resolve(args[0]);
                let mut text = args[1..].join(&b' ');
                text.push(b'\n');
                if let Err(e) = files::write(window, &path, &text) { self.fail("write", e); self.last_exit = 1; }
            }
            b"colors" => self.say(&colors()),
            // `theme NAME` saves it in spindle.conf; the window sees the change and every
            // pane takes it.
            b"theme" => match args.first() {
                None => {
                    let list = spindle_panes::config::THEME_NAMES.join(" ");
                    let now = self.screen.theme().name;
                    self.say(&format!("themes: {}\nnow: {} (set with theme NAME, or edit spindle.conf)\n", list, now));
                }
                Some(name) => {
                    let name = String::from_utf8_lossy(name).into_owned();
                    match sex_vt::theme::Theme::named(&name) {
                        None => { self.say(&format!("\x1b[31mtheme: no theme called {}\x1b[0m (type theme)\n", name)); self.last_exit = 1; }
                        Some(t) => {
                            let old = read_all_direct(CONFIG, 64 << 10).unwrap_or_default();
                            let text = spindle_panes::config::with_setting(&String::from_utf8_lossy(&old), "theme", t.name);
                            let _ = files::direct::create(CONFIG);
                            let saved = files::direct::truncate(CONFIG, 0).and_then(|_| files::direct::write_at(CONFIG, 0, text.as_bytes())).is_ok();
                            sex_pdx::serial_record!("[term.theme.saved] name={} ok={}", t.name, saved as u8);
                        }
                    }
                }
            },
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
            // A program that went on without its window (or `cmd &`) comes back here.
            b"attach" => match args.first().map(|a| core::str::from_utf8(a).ok().and_then(|a| a.trim_start_matches('%').parse::<u32>().ok())) {
                Some(None) => { self.say("\x1b[31mattach: expected a number\x1b[0m (type jobs)\n"); self.last_exit = 1; }
                Some(Some(n)) => { if !self.attach(Some(n)) { self.last_exit = 1; } }
                None => { if !self.attach(None) { self.last_exit = 1; } }
            },
            _ => { let _ = window; self.start_program(cmd, args) }
        }
        self.last_cmd_ms = Some(sex_pdx::get_ticks().saturating_sub(started_ms));
    }
    /// The pane or its window is closing: a program of its own goes on as a yarn, which
    /// `attach` (or `fg`) brings back in any Spindle window; a Wasm guest stops.
    fn leave(&mut self) {
        if let Some(tui) = self.tui.take() {
            if tui.is_alt_screen() {
                self.screen.feed(&mut self.parser, b"\x1b[?25h\x1b[?1049l");
            }
        }
        let slot = self.slot as usize;
        if let Some(Job { runner: Runner::Native(n), name, .. }) = &self.job {
            if !n.stopping {
                let screen = self.screen.replay(sex_pdx::console_relay::OUTPUT_BYTES / 2);
                match files::direct::relay::detach_at(slot, n.label.as_bytes(), &screen) {
                    Ok(yarn) => {
                        sex_pdx::serial_record!("[term.yarn.detached] name={} yarn={} bytes={}", name, yarn, screen.len());
                        self.job = None;
                        return;
                    }
                    Err(e) => sex_pdx::serial_record!("[term.yarn.refused] name={} status={}", name, e),
                }
            }
        }
        self.stop();
    }
    /// Yarn `name` (the newest still running when None) runs in this pane again: the
    /// screen it left, then what it wrote since. False when it could not.
    fn attach(&mut self, name: Option<u32>) -> bool {
        use sex_pdx::console_relay as c;
        let slot = self.slot as usize;
        if slot >= c::SLOTS {
            self.say("\x1b[31mattach: too many panes are running programs in this window\x1b[0m (open a new window)\n");
            return false;
        }
        let yarns = match files::direct::relay::yarns() { Ok(y) => y, Err(e) => { self.fail("attach", e); return false; } };
        let pick = match name {
            Some(n) => yarns.iter().find(|y| y.name == n),
            None => yarns.iter().rev().find(|y| y.state != c::ENDED).or(yarns.last()),
        };
        let Some(y) = pick.copied() else {
            match name {
                Some(n) => self.say(&format!("\x1b[31mattach: there is no job {}\x1b[0m (type jobs)\n", n)),
                None => self.say("attach: no program is running without its window\n"),
            }
            return false;
        };
        let label = String::from_utf8_lossy(y.label()).into_owned();
        let job = String::from(label.split(' ').next().unwrap_or("?"));
        match files::direct::relay::adopt_at(slot, y.name) {
            Ok(()) => {
                sex_pdx::serial_record!("[term.yarn.attached] name={} yarn={} state={}", job, y.name, y.state);
                self.job = Some(Job { name: job, runner: Runner::Native(Native { since: sex_pdx::get_ticks(), mode: 0, target: y.target, interrupted_at: None, stopping: false, label }), line: Vec::new(), pending: Vec::new(), eof: false });
                self.native_size();
                true
            }
            Err(sex_pdx::fs::PENDING) => { self.say("\x1b[31mattach: a program is still running here\x1b[0m\n"); false }
            Err(e) => { self.fail("attach", e); false }
        }
    }
    /// Whatever runs in the pane is told to stop and loses its input.
    fn stop(&mut self) {
        let Some(job) = self.job.take() else { return };
        sex_pdx::serial_record!("[term.job.stop] name={}", job.name);
        match job.runner {
            // The worker finishes on its own once its input is gone; its memory stays
            // until then (the stream lives in it), so it is left behind on purpose.
            Runner::Guest(g) => { g.stream.interrupt(); g.stream.hang_up(Side::Host); core::mem::forget(g); }
            // A program of its own is stopped through Shell when it runs; its slot is freed.
            Runner::Native(n) => {
                let slot = self.slot as usize;
                if n.target != 0 { let _ = files::direct::relay::stop(n.target); }
                let _ = files::direct::relay::interrupt_at(slot);
                let _ = files::direct::relay::hang_up_at(slot);
                let _ = files::direct::relay::release_at(slot);
            }
        }
    }
    /// Carries out what the shell asks next: show its output, or start a program.
    fn shell_step(&mut self, step: spindle_shell::Step, line: String) {
        use spindle_shell::Step;
        match step {
            Step::Done(out) => {
                self.last_exit = out.exit_code;
                sex_pdx::serial_record!("[term.shell.done] exit={} error={}", out.exit_code, out.error.is_some() as u8);
                self.out(out.text.as_bytes());
                self.shell_wait = None;
            }
            Step::WaitJob { print, job, capture } => {
                // `fg`: the background job (a yarn) runs in this pane until it ends.
                self.out(print.as_bytes());
                sex_pdx::serial_record!("[term.shell.fg] job={} capture={}", job, capture as u8);
                self.shell_wait = Some(ShellWait { capture, output: Vec::new(), line });
                if !self.attach(Some(job)) {
                    let step = self.engine.resume(spindle_shell::ExternalResult { output: Vec::new(), exit_code: 1 }, &mut shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot });
                    let line = self.shell_wait.take().map(|w| w.line).unwrap_or_default();
                    return self.shell_step(step, line);
                }
            }
            Step::RunExternal { print, request } => {
                self.out(print.as_bytes());
                let name = request.name.clone();
                let args: Vec<Vec<u8>> = request.args.iter().map(|a| a.as_bytes().to_vec()).collect();
                let args: Vec<&[u8]> = args.iter().map(|a| a.as_slice()).collect();
                sex_pdx::serial_record!("[term.shell.run] name={} capture={} input={}", name, request.capture as u8, request.input.as_ref().map_or(0, |i| i.len()));
                self.shell_wait = Some(ShellWait { capture: request.capture, output: Vec::new(), line });
                self.start_program(name.as_bytes(), &args);
                if self.job.is_none() {
                    // It did not start: the shell hears it failed.
                    let step = self.engine.resume(spindle_shell::ExternalResult { output: Vec::new(), exit_code: 127 }, &mut shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot });
                    let line = self.shell_wait.take().map(|w| w.line).unwrap_or_default();
                    return self.shell_step(step, line);
                }
                if let Some(input) = request.input { self.job_input(&input); }
            }
        }
    }
    /// Bytes piped into the running program, then the end of its input.
    fn job_input(&mut self, input: &[u8]) {
        if let Some(job) = self.job.as_mut() { job.eof = true; }
        self.send(input);
    }
    /// Input for the running program, in order: what it cannot take yet waits in the job.
    fn send(&mut self, bytes: &[u8]) {
        if let Some(job) = self.job.as_mut() { job.pending.extend_from_slice(bytes); }
        self.feed();
    }
    /// Hands the program as much waiting input as it takes, then its end when all is taken.
    fn feed(&mut self) {
        let slot = self.slot as usize;
        let Some(job) = self.job.as_mut() else { return };
        while !job.pending.is_empty() {
            let n = match &job.runner {
                Runner::Guest(g) => g.stream.write(Side::Host, &job.pending),
                Runner::Native(_) => files::direct::relay::put_at(slot, &job.pending).unwrap_or(0),
            };
            if n == 0 { break; }
            job.pending.drain(..n);
        }
        if job.eof && job.pending.is_empty() {
            job.eof = false;
            match &job.runner {
                Runner::Guest(g) => g.stream.hang_up(Side::Host),
                Runner::Native(_) => { let _ = files::direct::relay::hang_up_at(slot); }
            }
        }
    }
    /// What a running program wrote: the screen's, or the shell's when it takes it.
    fn job_out(&mut self, bytes: &[u8]) {
        match &mut self.shell_wait {
            Some(w) if w.capture => w.output.extend_from_slice(bytes),
            _ => self.out(bytes),
        }
    }
    fn fail(&mut self, what: &str, e: files::Error) {
        let why = String::from_utf8_lossy(files::message(e)).into_owned();
        self.say(&format!("\x1b[31m{}: {}\x1b[0m\n", what, why));
    }

    /// Starts `NAME.wasm` (or the built-in guest) on a worker with a stream to the screen.
    fn start_program(&mut self, cmd: &[u8], args: &[&[u8]]) {
        let name = cmd.strip_suffix(b".wasm").unwrap_or(cmd);
        let module: Vec<u8> = if cmd == b"greet" { GREET.to_vec() } else {
            let mut path = self.resolve(cmd);
            if !path.ends_with(b".wasm") { path.extend_from_slice(b".wasm"); }
            match read_all_direct(&path, MODULE_MAX) {
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
                self.job = Some(Job { name: String::from_utf8_lossy(name).into_owned(), runner: Runner::Guest(Guest { stream, _memory: memory, handle, result }), line: Vec::new(), pending: Vec::new(), eof: false });
            }
            Err(_) => self.say("\x1b[31mrun: no worker is free to run it\x1b[0m\n"),
        }
    }
    /// Takes what the guest wrote or ticks active Ratatui TUIs. True if the screen changed.
    fn pump(&mut self) -> bool {
        if self.tui.is_some() {
            let now = sex_pdx::get_ticks();
            let (cols, rows) = (self.screen.cols(), self.screen.rows());
            let mut cwd_copy = self.cwd.clone();
            let host = shell_host::TermHost { cwd: &mut cwd_copy, slot: self.slot };
            let out = self.tui.as_mut().unwrap().tick(&host, cols, rows, now);
            let changed = !out.bytes.is_empty() || out.exit.is_some();
            if changed {
                self.apply_tui_output(out);
            }
            return changed;
        }
        self.feed();
        let Some(job) = &self.job else { return false };
        let Runner::Guest(guest) = &job.runner else { return self.pump_native() };
        let stream = guest.stream;
        let mut buf = [0u8; 4096];
        let mut changed = false;
        loop {
            let n = stream.read(Side::Host, &mut buf);
            if n == 0 { break; }
            self.job_out(&buf[..n]);
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
        self.last_exit = code as i32;
        sex_pdx::serial_record!("[term.job.exit] name={} code={}", name, code);
        // A program the shell started: the shell goes on with the rest of the line.
        if let Some(w) = self.shell_wait.take() {
            let output = if w.capture { w.output } else { Vec::new() };
            if !w.capture && !self.after_cr && self.screen.cursor().0 != 0 { self.say("\n"); }
            let step = self.engine.resume(spindle_shell::ExternalResult { output, exit_code: code as i32 }, &mut shell_host::TermHost { cwd: &mut self.cwd, slot: self.slot });
            self.shell_step(step, w.line);
            if self.job.is_some() { return; }
        }
        if code != 0 { self.say(&format!("\x1b[2m[{} exited with {}]\x1b[0m\n", name, code)); }
        if !self.after_cr && self.screen.cursor().0 != 0 { self.say("\n"); }
        self.say(&format!("\x1b]133;D;{}\x07", code));
        self.show_prompt();
    }

    /// Starts `/bin/NAME`, a program of its own (Rust std, C, Python, Lua...), with this
    /// terminal as its console. Only a Term started with Spindle's `spindle` command may.
    fn start_native(&mut self, cmd: &[u8], args: &[&[u8]]) {
        if self.slot as usize >= sex_pdx::console_relay::SLOTS {
            self.say("\x1b[31mrun: too many panes are running programs in this window\x1b[0m (open a new window)\n");
            return;
        }
        let network = NETWORK_PROGRAMS.contains(&cmd);
        let domain = if cmd == b"ssh" { ssh_domain_args(args) } else { None };
        if let Some(d) = &domain { sex_pdx::serial_record!("[term.ssh.domain] name={} args={}", String::from_utf8_lossy(args[0]), String::from_utf8_lossy(&d.join(&b' '))); }
        let words = match &domain { Some(d) => d.join(&b' '), None => args.join(&b' ') };
        let n = String::from_utf8_lossy(cmd).into_owned();
        let label = if words.is_empty() { n.clone() } else { format!("{} {}", n, String::from_utf8_lossy(&words)) };
        // A general launch ticket is not permission to attach a terminal console: the
        // relay's own answer (no grant, or a run still going) comes first.
        let slot = self.slot as usize;
        match files::direct::relay::release_at(slot).and_then(|_| files::direct::relay::start_net_at(slot, cmd, &words, network)) {
            Ok(()) => {
                sex_pdx::serial_record!("[term.native.start] name={}", n);
                if network { sex_pdx::serial_record!("[term.native.network] name={}", n); }
                self.job = Some(Job { name: n, runner: Runner::Native(Native { since: sex_pdx::get_ticks(), mode: 0, target: 0, interrupted_at: None, stopping: false, label }), line: Vec::new(), pending: Vec::new(), eof: false });
                self.native_size();
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
        let Ok((status, bytes)) = files::direct::relay::take_at(self.slot as usize) else { return false };
        self.native_status(status);
        let changed = !bytes.is_empty();
        if changed { self.job_out(&bytes); }
        let replies = self.screen.take_replies();
        if !replies.is_empty() { let _ = files::direct::relay::put_at(self.slot as usize, &replies); }
        let Some(Job { runner: Runner::Native(Native { since, stopping, .. }), name, .. }) = &self.job else { return changed };
        let (since, stopping, name) = (*since, *stopping, name.clone());
        let ended = match status.state {
            c::ENDED if status.code == c::NOT_STARTED => {
                self.say(&format!("\x1b[31m{}: no such command or program\x1b[0m (type help)\n", name));
                Some(-2)
            }
            // Stopped on the person's second Ctrl+C: 128 + 9, as a killed Unix program.
            c::ENDED if status.code == c::CRASHED && stopping => Some(137),
            c::ENDED => Some(if status.code == c::CRASHED { -1 } else { status.code as i64 }),
            c::STARTING if sex_pdx::get_ticks().saturating_sub(since) > START_MS => {
                self.say(&format!("\x1b[31m{}: it did not start\x1b[0m (too many programs open?)\n", name));
                Some(-2)
            }
            _ => None,
        };
        let Some(code) = ended else { return changed };
        // What it wrote last may still be on its way.
        if let Ok((_, rest)) = files::direct::relay::take_at(self.slot as usize) { self.job_out(&rest); }
        self.job = None;
        let _ = files::direct::relay::release_at(self.slot as usize);
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
        let frame = (0..6).all(|r| self.screen.row_text(r) == check::ROWS[r]);
        let colors = check::COLORS.iter().all(|&(x, y, fg, bg)| sex_vt::colors(&self.screen.visible(x, y)) == (fg, bg));
        // Then the pixels: the middle-left of each checked cell after drawing is its active-theme background
        // (and 256-color palette slot 27 at (2, 2) is theme-independent 0x005FFF).
        self.screen.mark_all_dirty();
        self.draw(true);
        let _ = window;
        let (cw, ch, w) = (self.painter.cell_w, self.painter.cell_h, self.width);
        let pixels = &self.buf;
        let drawn = check::COLORS.iter().all(|&(x, y, _, fixed_bg)| {
            let want_bg = if x == 2 && y == 2 { fixed_bg } else { self.screen.colors(&self.screen.visible(x, y)).1 };
            pixels[(y * ch + 1) * w + x * cw] == want_bg
        });
        sex_pdx::serial_record!("[term.vtcheck] frame={} colors={} pixels={} cols={} rows={}", frame as u8, colors as u8, drawn as u8, self.screen.cols(), self.screen.rows());
        self.say(if frame && colors && drawn { "vtcheck: \x1b[32mok\x1b[0m\n" } else { "vtcheck: \x1b[31mFAILED\x1b[0m\n" });
    }

    // ---- Keys and the mouse ----

    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) -> Option<Vec<u8>> {
        if !matches!(self.mode, Mode::Normal) { return self.mode_key(key); }
        self.key_normal(window, key);
        None
    }

    fn redraw_line_clean(&mut self) {
        self.complete_menu = None;
        self.rev_search = None;
        let p = self.prompt();
        let line = self.coloured_line();
        let (abs, col) = self.prompt_at;
        let row = abs.saturating_sub(self.screen.history_len());
        self.say(&format!("\x1b[{};{}H\x1b[J{}{}", row + 1, col + 1, p, line));
    }

    fn handle_tui_key(&mut self, key: sex_pdx::app_key::Key) {
        let Some(tk) = map_tui_key(key) else { return };
        let now = sex_pdx::get_ticks();
        let (cols, rows) = (self.screen.cols(), self.screen.rows());
        let mut cwd_copy = self.cwd.clone();
        let host = shell_host::TermHost { cwd: &mut cwd_copy, slot: self.slot };
        let out = self.tui.as_mut().unwrap().key(tk, &host, cols, rows, now);
        self.apply_tui_output(out);
    }

    fn handle_rev_search_key(&mut self, key: sex_pdx::app_key::Key) -> bool {
        let Some(rs) = self.rev_search.as_mut() else { return false };
        match (key.control, key.code, sex_app::nav(key)) {
            (_, 0x01, _) | (true, 0x2E, _) => {
                let saved = core::mem::take(&mut rs.saved_line);
                let saved_at = rs.saved_at;
                self.rev_search = None;
                self.line = saved;
                self.at = saved_at;
                self.redraw_line();
                return true;
            }
            (_, 0x1C, _) | (_, 0x0F, _) | (_, _, Some(Nav::Right)) => {
                if let Some(hit) = rs.matches.get(rs.selected) {
                    self.line = hit.text.clone().into_bytes();
                    self.at = self.line.len();
                }
                self.rev_search = None;
                self.redraw_line();
                return true;
            }
            (true, 0x13, _) | (_, _, Some(Nav::Down)) => {
                if !rs.matches.is_empty() {
                    rs.selected = (rs.selected + 1) % rs.matches.len();
                }
                self.redraw_line();
                return true;
            }
            (_, _, Some(Nav::Up)) => {
                if !rs.matches.is_empty() {
                    rs.selected = rs.selected.checked_sub(1).unwrap_or(rs.matches.len() - 1);
                }
                self.redraw_line();
                return true;
            }
            (_, 0x0E, _) => {
                rs.query.pop();
            }
            (false, _, None) if !key.alt => {
                if let Some(c) = sex_app::text(key) {
                    rs.query.push(c);
                } else {
                    return true;
                }
            }
            _ => return true,
        }
        let q = self.rev_search.as_ref().unwrap().query.clone();
        let hist = self.history_strings();
        let matches = shell_ui::fuzzy_search_history(&hist, &q, 16);
        if let Some(rs) = self.rev_search.as_mut() {
            rs.matches = matches;
            rs.selected = 0;
        }
        self.redraw_line();
        true
    }

    fn accept_menu_selection(&mut self) {
        let Some(menu) = self.complete_menu.take() else { return };
        let Some(hit) = menu.hits.get(menu.selected) else { return };
        let Ok(text) = core::str::from_utf8(&self.line) else { return };
        if hit.replace.end > text.len() { return; }
        let mut next = String::from(&text[..hit.replace.start]);
        next.push_str(&hit.text);
        if !hit.text.ends_with('/') { next.push(' '); }
        let at = next.len();
        next.push_str(&text[hit.replace.end..]);
        self.line = next.into_bytes();
        self.at = at;
    }

    fn key_normal(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let (shift, control, alt) = (key.shift, key.control, key.alt);
        // Terminal-wide keys first.
        match (control, shift, key.code) {
            (false, true, 0x49 | 0x68) => { let r = self.screen.rows() as isize - 1; self.screen.scroll_view(r); return; }
            (false, true, 0x51 | 0x6D) => { let r = self.screen.rows() as isize - 1; self.screen.scroll_view(-r); return; }
            _ => {}
        }
        self.screen.clear_selection();
        if self.tui.is_some() { self.handle_tui_key(key); return; }
        if self.job.is_some() { self.job_key(key); return; }
        if self.rev_search.is_some() { self.handle_rev_search_key(key); return; }

        if self.complete_menu.is_some() {
            match (key.code, sex_app::nav(key), key.shift) {
                (0x01, _, _) => { self.complete_menu = None; self.redraw_line(); return; }
                (0x0F, _, false) | (_, Some(Nav::Down | Nav::Right), _) => {
                    let m = self.complete_menu.as_mut().unwrap();
                    m.selected = (m.selected + 1) % m.hits.len().max(1);
                    self.redraw_line();
                    return;
                }
                (0x0F, _, true) | (_, Some(Nav::Up | Nav::Left), _) => {
                    let m = self.complete_menu.as_mut().unwrap();
                    m.selected = m.selected.checked_sub(1).unwrap_or(m.hits.len().saturating_sub(1));
                    self.redraw_line();
                    return;
                }
                (0x1C, _, _) => {
                    self.accept_menu_selection();
                    self.redraw_line();
                    return;
                }
                _ => { self.complete_menu = None; }
            }
        }

        // Alt+Right / Alt+F (`forward-word` + word-wise autosuggestion accept).
        if alt && (sex_app::nav(key) == Some(Nav::Right) || key.code == 0x21) {
            self.accept_word_or_move();
            self.redraw_line();
            return;
        }
        // Alt+Left / Alt+B (`backward-word`).
        if alt && (sex_app::nav(key) == Some(Nav::Left) || key.code == 0x30) {
            self.at = prev_word_boundary(&self.line, self.at);
            self.redraw_line();
            return;
        }

        let nav = sex_app::nav(key);
        let text_ch = if control || alt { None } else { sex_app::text(key) };
        match (control, key.code, nav, text_ch) {
            (_, 0x1C, ..) => {
                self.redraw_line_clean();
                let line = core::mem::take(&mut self.line);
                self.say("\r\n\x1b]133;C\x07");
                self.run_line(window, &line);
                if self.job.is_none() && self.tui.is_none() {
                    let code = self.last_exit;
                    self.say(&format!("\x1b]133;D;{}\x07", code));
                    self.show_prompt();
                }
                return;
            }
            // Ctrl+C: clear overlays and start fresh prompt.
            (true, 0x2E, ..) => { self.redraw_line_clean(); self.say("^C\r\n"); self.last_exit = 130; self.show_prompt(); return; }
            // Ctrl+L: clear screen preserving line.
            (true, 0x26, ..) => {
                self.say("\x1b[H\x1b[2J");
                self.say_prompt_header();
                let (x, y) = self.screen.cursor();
                self.prompt_at = (self.screen.history_len() + y, x);
                self.redraw_line();
                return;
            }
            (true, 0x20, ..) if self.line.is_empty() => { sex_pdx::serial_record!("[term.pane.closed] how=ctrl-d"); self.closed = true; return; }
            // Ctrl+R: fuzzy reverse history search TUI.
            (true, 0x13, ..) => {
                let hist = self.history_strings();
                let matches = shell_ui::fuzzy_search_history(&hist, "", 16);
                self.rev_search = Some(RevSearchDrawer {
                    query: String::new(),
                    matches,
                    selected: 0,
                    saved_line: self.line.clone(),
                    saved_at: self.at,
                });
                self.redraw_line();
                return;
            }
            // Ctrl+F: accept next word of ghost autosuggestion, else forward one char.
            (true, 0x21, ..) => self.accept_word_or_move(),
            // Ctrl+B: backward one char.
            (true, 0x30, ..) => self.at = prev_char_boundary(&self.line, self.at),
            // Ctrl+A / Home.
            (true, 0x1E, ..) | (_, _, Some(Nav::Home), _) => self.at = 0,
            // Ctrl+E: accept full ghost at EOL, else move to EOL.
            (true, 0x12, ..) => {
                if self.at == self.line.len() {
                    if let Some(g) = self.current_ghost() {
                        self.line.extend_from_slice(g.as_bytes());
                    }
                }
                self.at = self.line.len();
            }
            (_, _, Some(Nav::End), _) => self.at = self.line.len(),
            (true, 0x16, ..) => { self.line.drain(..self.at); self.at = 0; self.recall = None; }
            (true, 0x25, ..) => { self.line.truncate(self.at); self.recall = None; }
            // Ctrl+W: backward kill word.
            (true, 0x11, ..) => {
                let start = prev_word_boundary(&self.line, self.at);
                self.line.drain(start..self.at);
                self.at = start;
                self.recall = None;
            }
            (_, 0x0E, ..) => {
                if self.at > 0 {
                    let prev = prev_char_boundary(&self.line, self.at);
                    self.line.drain(prev..self.at);
                    self.at = prev;
                    self.recall = None;
                }
            }
            (_, 0x0F, ..) => self.complete(window),
            (_, _, Some(Nav::Left), _) => self.at = prev_char_boundary(&self.line, self.at),
            (_, _, Some(Nav::Right), _) => {
                if self.at == self.line.len() {
                    if let Some(g) = self.current_ghost() {
                        self.line.extend_from_slice(g.as_bytes());
                        self.at = self.line.len();
                    }
                } else {
                    self.at = next_char_boundary(&self.line, self.at);
                }
            }
            (_, _, Some(Nav::Delete), _) => {
                if self.at < self.line.len() {
                    let next = next_char_boundary(&self.line, self.at);
                    self.line.drain(self.at..next);
                    self.recall = None;
                }
            }
            (_, _, Some(dir @ (Nav::Up | Nav::Down)), _) => {
                if self.history.is_empty() { return; }
                let up = dir == Nav::Up;
                if self.recall.is_none() && up {
                    self.recall_prefix = self.line.clone();
                }
                let prefix = String::from_utf8_lossy(&self.recall_prefix).into_owned();
                let hist = self.history_strings();
                if let Some(next) = shell_ui::history_search_prefix(&hist, &prefix, self.recall, up) {
                    self.recall = next;
                    self.line = match next {
                        Some(idx) => self.history[idx].clone(),
                        None => self.recall_prefix.clone(),
                    };
                    self.at = self.line.len();
                } else {
                    return;
                }
            }
            (false, _, None, Some(c)) => {
                let mut utf8 = [0u8; 4];
                let bytes = c.encode_utf8(&mut utf8).as_bytes();
                for (i, &b) in bytes.iter().enumerate() {
                    self.line.insert(self.at + i, b);
                }
                self.at += bytes.len();
                self.recall = None;
            }
            _ => return,
        }
        self.redraw_line();
    }

    fn accept_word_or_move(&mut self) {
        if self.at == self.line.len() {
            if let Some(g) = self.current_ghost() {
                let mut s = String::from_utf8_lossy(&self.line).into_owned();
                let mut cur = s.len();
                if shell_ui::accept_ghost_word(&mut s, &mut cur, &g) {
                    self.line = s.into_bytes();
                    self.at = self.line.len();
                    return;
                }
            }
        }
        self.at = next_word_boundary(&self.line, self.at);
    }

    /// Tab through the shell: commands, aliases, themes, flags, `$variables`, columns and
    /// paths — opens Fish-style multi-column completion drawer when ambiguous.
    fn complete_shell(&mut self) -> bool {
        let Ok(text) = core::str::from_utf8(&self.line) else { return false };
        let text = String::from(text);
        let mut cwd_copy = self.cwd.clone();
        let host = shell_host::TermHost { cwd: &mut cwd_copy, slot: self.slot };
        let hits = self.engine.complete_with(&text, self.at, &host, SHELL_EXTRA_CMDS, &spindle_panes::config::THEME_NAMES);
        let Some(first) = hits.first() else { return false };
        let range = first.replace.clone();
        if hits.iter().any(|h| h.replace != range) || range.end > text.len() { return false; }
        // The longest common prefix of the offers.
        let mut common = first.text.clone();
        for h in &hits[1..] {
            let n = common.char_indices().zip(h.text.chars()).take_while(|((_, a), b)| a == b).count();
            let end = common.char_indices().nth(n).map_or(common.len(), |(i, _)| i);
            common.truncate(end);
        }
        let current = &text[range.clone()];
        if common.len() <= current.len() && hits.len() > 1 {
            self.complete_menu = Some(CompletionDrawer { hits, selected: 0 });
            return true;
        }
        let mut new = String::from(&text[..range.start]);
        new.push_str(&common);
        if hits.len() == 1 && !common.ends_with('/') { new.push(' '); }
        let at = new.len();
        new.push_str(&text[range.end..]);
        self.line = new.into_bytes();
        self.at = at;
        true
    }
    /// Completes the last word from the names in its folder (and the built-ins first).
    fn complete(&mut self, window: &mut Window) {
        if self.complete_shell() { return; }
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
            let (_, y) = self.screen.cursor();
            self.prompt_at = (self.screen.history_len() + y, 0);
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
        if self.native_direct_key(key) { return; }
        let echo = self.native_mode() & sex_pdx::console_relay::NO_ECHO == 0;
        let job = self.job.as_mut().unwrap();
        match (key.control, key.code) {
            (true, 0x20) => { job.eof = true; self.feed(); }
            (_, 0x1C) => {
                let mut line = core::mem::take(&mut job.line);
                line.push(b'\n');
                self.send(&line);
                if echo { self.say("\r\n"); }
            }
            (_, 0x0E) => {
                let mut gone = false;
                while let Some(b) = job.line.pop() { gone = true; if b & 0xC0 != 0x80 { break; } }
                if gone && echo { self.say("\x08 \x08"); }
            }
            (false, _) => {
                if let Some(c) = sex_app::text(key) {
                    let mut utf8 = [0; 4];
                    let bytes = c.encode_utf8(&mut utf8).as_bytes();
                    job.line.extend_from_slice(bytes);
                    if echo { self.out(bytes); }
                } else if let Some(k) = nav_key(key) {
                    let bytes = keys::encode(k, Mods { shift: key.shift, control: key.control, alt: false }, self.screen.app_cursor);
                    self.send(&bytes);
                }
            }
            _ => {}
        }
    }

    // ---- A program of its own: its size, its terminal mode, Ctrl+C and stopping ----

    /// The screen's size in cells, for the program (`TIOCGWINSZ`, `sex_std::terminal::size`).
    fn native_size(&self) {
        let (cols, rows) = (self.screen.cols() as u16, self.screen.rows() as u16);
        let ok = files::direct::relay::set_size_at(self.slot as usize, cols, rows).is_ok();
        sex_pdx::serial_record!("[term.native.size] cols={} rows={} ok={}", cols, rows, ok as u8);
    }
    fn native_mode(&self) -> u8 {
        match &self.job { Some(Job { runner: Runner::Native(n), .. }) => n.mode, _ => 0 }
    }
    /// What TAKE said: the mode the program asked for, and its target once it runs.
    fn native_status(&mut self, status: sex_pdx::console_relay::Status) {
        let Some(Job { runner: Runner::Native(n), name, .. }) = self.job.as_mut() else { return };
        if n.mode != status.mode { sex_pdx::serial_record!("[term.native.mode] name={} mode={}", name, status.mode); }
        n.mode = status.mode;
        if status.target != 0 { n.target = status.target; }
    }
    /// Ctrl+C: an interrupt, which the run sees at its next read, write, sleep or look.
    /// Pressed again within a second (or Ctrl+\ at once), the run is stopped through Shell,
    /// since a program computing without asking anything never sees the interrupt.
    fn native_interrupt(&mut self, stop: bool) {
        use files::direct::relay;
        let Some(Job { runner: Runner::Native(n), name, line, .. }) = self.job.as_mut() else { return };
        let now = sex_pdx::get_ticks();
        line.clear();
        if (stop || n.interrupted_at.is_some_and(|at| now.saturating_sub(at) < 1000)) && n.target != 0 {
            let ok = relay::stop(n.target).is_ok();
            n.stopping = true;
            sex_pdx::serial_record!("[term.job.stop] name={} target={} ok={} ms={}", name, n.target, ok as u8, now);
            self.say("^\\");
        } else {
            let _ = relay::interrupt_at(self.slot as usize);
            n.interrupted_at = Some(now);
            sex_pdx::serial_record!("[term.job.interrupt] name={} ms={}", name, now);
            self.say("^C");
        }
    }
    /// Keys that skip the line: Ctrl+\ and Ctrl+C (unless the run takes Ctrl+C as a byte),
    /// and in raw mode every key, sent at once as a terminal encodes it, unechoed.
    /// True when the key was handled here.
    fn native_direct_key(&mut self, key: sex_pdx::app_key::Key) -> bool {
        use sex_pdx::console_relay as c;
        let mode = self.native_mode();
        if key.control && key.code == 0x2B { self.native_interrupt(true); return true; }
        let ctrl_c = key.control && key.code == 0x2E;
        if ctrl_c && mode & c::NO_SIGNALS == 0 { self.native_interrupt(false); return true; }
        if mode & c::RAW == 0 && !ctrl_c { return false; }
        let us = || sex_pdx::app_key::us_char(sex_pdx::app_key::Key { control: false, ..key }).map(|b| Key::Char(b as char));
        let k = match key.code { 0x1C => Some(Key::Enter), 0x0E => Some(Key::Backspace), _ => None }
            .or_else(|| nav_key(key))
            .or_else(|| if key.control { us() } else { sex_app::text(key).map(Key::Char) });
        if let Some(k) = k {
            let bytes = keys::encode(k, Mods { shift: key.shift, control: key.control, alt: false }, self.screen.app_cursor);
            self.send(&bytes);
        }
        true
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
        if self.job.is_some() {
            let mode = self.native_mode();
            // Raw mode: the program reads the paste as it comes, bracketed if it asked.
            if mode & sex_pdx::console_relay::RAW != 0 {
                let bytes = keys::paste(&text, self.screen.bracketed_paste);
                self.send(&bytes);
                return;
            }
            // Cooked: each pasted line goes as if typed and Enter pressed; the rest waits
            // on the line. Echoed unless the program turned echo off (a password).
            let mut shown = Vec::new();
            for b in keys::paste(&text, false) {
                let job = self.job.as_mut().unwrap();
                if b == b'\r' {
                    job.line.push(b'\n');
                    let l = core::mem::take(&mut job.line);
                    shown.extend_from_slice(b"\r\n");
                    self.send(&l);
                } else { job.line.push(b); shown.push(b); }
            }
            if mode & sex_pdx::console_relay::NO_ECHO == 0 { self.out(&shown); }
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
            Kind::Wheel { steps } => self.scroll_smoothly(steps as isize * 3),
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

/// What a program asked of the terminal with an escape sequence.
fn pane_event(id: PaneId, pane: &mut Pane, e: sex_vt::Event, clipboard: &mut Vec<u8>) {
    use sex_vt::Event as E;
    match e {
        // OSC 52: a program may set the clipboard (remote editors copy this way). Reading
        // it is refused: a program never learns what else was copied.
        E::Clipboard { set, .. } => { sex_pdx::serial_record!("[term.osc52.set] pane={} bytes={}", id, set.len()); *clipboard = set; }
        E::ClipboardQuery { .. } => sex_pdx::serial_record!("[term.osc52.query] pane={} refused=1", id),
        // OSC 9 / 777: a desktop notification through Bell.
        E::Notify { title, body } => {
            let title = if title.is_empty() { pane.title() } else { title };
            let posted = sex_pdx::notify::post(title.as_bytes(), body.as_bytes(), sex_pdx::notify::URGENCY_NORMAL);
            sex_pdx::serial_record!("[term.notify] pane={} posted={} title={} body={}", id, posted as u8, title, body);
        }
        E::Cwd(c) => pane.osc_cwd = Some(c),
        E::Progress(p) => {
            use sex_vt::Progress as P;
            pane.progress = match p { P::Clear => None, P::Set(v) | P::Error(v) | P::Paused(v) => Some(v), P::Indeterminate => Some(0) };
        }
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

fn map_tui_key(key: sex_pdx::app_key::Key) -> Option<TuiKey> {
    if key.control && key.code == 0x2E {
        return Some(TuiKey::CtrlC);
    }
    match (sex_app::nav(key), key.code, key.shift) {
        (Some(Nav::Up), _, _) => return Some(TuiKey::Up),
        (Some(Nav::Down), _, _) => return Some(TuiKey::Down),
        (Some(Nav::Left), _, _) => return Some(TuiKey::Left),
        (Some(Nav::Right), _, _) => return Some(TuiKey::Right),
        (Some(Nav::Home), _, _) => return Some(TuiKey::Home),
        (Some(Nav::End), _, _) => return Some(TuiKey::End),
        (_, 0x49 | 0x68, _) => return Some(TuiKey::PageUp),
        (_, 0x51 | 0x6D, _) => return Some(TuiKey::PageDown),
        (_, 0x1C, _) => return Some(TuiKey::Enter),
        (_, 0x0E, _) => return Some(TuiKey::Backspace),
        (_, 0x01, _) => return Some(TuiKey::Escape),
        (_, 0x0F, true) => return Some(TuiKey::BackTab),
        (_, 0x0F, false) => return Some(TuiKey::Tab),
        _ => {}
    }
    if !key.control && !key.alt {
        if let Some(c) = sex_app::text(key) {
            return Some(TuiKey::Char(c));
        }
    }
    None
}

fn prev_char_boundary(bytes: &[u8], at: usize) -> usize {
    let mut i = at.min(bytes.len());
    if i == 0 { return 0; }
    i -= 1;
    while i > 0 && (bytes[i] & 0xC0) == 0x80 { i -= 1; }
    i
}

fn next_char_boundary(bytes: &[u8], at: usize) -> usize {
    let mut i = at.min(bytes.len());
    if i >= bytes.len() { return bytes.len(); }
    i += 1;
    while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 { i += 1; }
    i
}

fn prev_word_boundary(bytes: &[u8], at: usize) -> usize {
    let mut i = at.min(bytes.len());
    while i > 0 && bytes[i - 1].is_ascii_whitespace() { i -= 1; }
    while i > 0 && !bytes[i - 1].is_ascii_whitespace() { i -= 1; }
    i
}

fn next_word_boundary(bytes: &[u8], at: usize) -> usize {
    let mut i = at.min(bytes.len());
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() { i += 1; }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
    i
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

fn read_all_direct(path: &[u8], max: usize) -> Result<Vec<u8>, files::Error> {
    let st = files::direct::stat(path)?;
    if st.dir { return Err(sex_pdx::fs::IS_DIR); }
    let len = (st.size as usize).min(max);
    let mut data = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let n = files::direct::read_at(path, at as u64, &mut data[at..])?;
        if n == 0 { break; }
        at += n;
    }
    data.truncate(at);
    Ok(data)
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

const BUILTINS: &[&[u8]] = &[b"attach", b"bars", b"bat", b"btop", b"cat", b"cd", b"clear", b"colors", b"echo", b"exit", b"fastfetch", b"fetch", b"greet", b"help", b"history", b"ll", b"ls", b"mkdir", b"mv", b"pwd", b"rm", b"snapshot", b"touch", b"tree", b"update", b"vtcheck", b"which", b"write", b"y", b"yazi"];
const HELP: &str = "\x1b[1;38;2;244;63;94m● Spindle\x1b[0m: SexOS Noir Terminal (Fish + P10k + Ratatui).\n\
  \x1b[38;2;34;211;238mShell & Files\x1b[0m  ls ll la tree bat cat mkdir rm mv touch cd .. ... pwd which alias\n\
  \x1b[38;2;34;211;238mRatatui TUIs\x1b[0m   \x1b[1mbtop\x1b[0m system monitor · \x1b[1my\x1b[0m/\x1b[1myazi\x1b[0m 3-pane file manager (q cd+quit) · \x1b[1mbars\x1b[0m/\x1b[1mupdate\x1b[0m · \x1b[1mfetch\x1b[0m\n\
  \x1b[38;2;34;211;238mPipelines\x1b[0m      ls | where size > 1mb | sort-by name · help [CMD] · theme [NAME]\n\
  \x1b[38;2;34;211;238mPrograms\x1b[0m       NAME [ARGS] runs NAME.wasm or /bin/NAME · greet [NAME] · NAME &\n\
                 jobs · fg N · attach [N] · kill N · ssh USER@HOST · clear colors vtcheck exit\n\
\x1b[38;2;192;132;252mFish/Zsh Keys\x1b[0m  Tab menu · → / Ctrl+E accept ghost · Ctrl+F / Alt+→ accept word\n\
               Up/Down prefix history · Ctrl+R fuzzy reverse history · Ctrl+W delete word\n\
               Ctrl+Shift+C/V copy/paste · Ctrl+=/- font size · Ctrl+C stop · Ctrl+L clear\n\
\x1b[38;2;192;132;252mTabs & Panes\x1b[0m   Ctrl+Shift+T tab · Ctrl+PgUp/Dn switch · Ctrl+Shift+E/O split · Ctrl+Shift+Z zoom\n\
               Ctrl+Shift+arrows focus · Ctrl+Shift+F find · Ctrl+Shift+Space hints · Ctrl+Shift+X copy · F11 close\n";

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
    status_sig: String,
    painter: Painter,
    /// `spindle.conf`: what it says, the theme it makes, and when it was last read.
    config: spindle_panes::config::Config,
    theme: sex_vt::theme::Theme,
    config_stamp: Option<(u64, u64)>,
    config_checked: u64,
}

/// Pixels between panes, and the SexOS Noir palette of the window's chrome.
const GAP: usize = 2;
const DIVIDER: u32 = 0x24283B;
const DIVIDER_FOCUS: u32 = 0xF43F5E;
const TAB_BG: u32 = 0x0D0F16;
const TAB_ACTIVE: u32 = 0x1A1E2B;
const TAB_TEXT: u32 = 0x64748B;
const TAB_TEXT_ACTIVE: u32 = 0xECEFF4;
const STATUS_BG: u32 = 0x0D0F16;
const STATUS_PILL: u32 = 0x181C28;
const STATUS_ROSE: u32 = 0xF43F5E;
const STATUS_CYAN: u32 = 0x22D3EE;
const STATUS_MUTED: u32 = 0x64748B;

impl Spindle {
    fn new(width: usize, height: usize) -> Spindle {
        let cell_h = 18;
        let (cw, ch) = cell_for_height(14);
        let mut s = Spindle {
            panes: BTreeMap::new(), tabs: Tabs::new(1), next_id: 2, width: width.max(64), height: height.max(64), cell_h,
            clipboard: Vec::new(), relayout: true, dragging: None, resizing: false, title: String::new(), status_sig: String::new(), painter: painter(cw, ch),
            config: Default::default(), theme: sex_vt::theme::NOIR, config_stamp: None, config_checked: 0,
        };
        let area = s.area();
        let mut pane = Pane::new(area.w, area.h, cell_h);
        pane.screen.set_theme(sex_vt::theme::NOIR);
        s.panes.insert(1, pane);
        s.check_config(true);
        s
    }
    /// Reads `spindle.conf` again when it changed (checked at most once a second unless
    /// `now`), and applies it to every pane: theme, text size, padding, scrollback.
    fn check_config(&mut self, now: bool) {
        let t = sex_pdx::get_ticks();
        if !now && t.saturating_sub(self.config_checked) < 1000 { return; }
        self.config_checked = t;
        let stamp = files::direct::stat(CONFIG).ok().map(|st| (st.size, st.modified));
        if stamp == self.config_stamp { return; }
        self.config_stamp = stamp;
        let text = stamp.and_then(|_| read_all_direct(CONFIG, 64 << 10).ok()).unwrap_or_default();
        let config = spindle_panes::config::parse(&String::from_utf8_lossy(&text));
        let mut theme = sex_vt::theme::Theme::named(&config.theme).unwrap_or(sex_vt::theme::NOIR);
        for (key, c) in &config.colors {
            match key.as_str() {
                "foreground" => theme.fg = *c, "background" => theme.bg = *c,
                "cursor" => theme.cursor = *c, "selection" => theme.selection = *c,
                k => if let Some(i) = k.strip_prefix("color").and_then(|n| n.parse::<usize>().ok()) { theme.ansi[i] = *c; },
            }
        }
        for p in &config.problems { sex_pdx::serial_record!("[term.config.problem] {}", p); }
        sex_pdx::serial_record!("[term.config] theme={} font={} size={} padding={} scrollback={} colors={} problems={}", theme.name, config.font, config.font_size, config.padding, config.scrollback, config.colors.len(), config.problems.len());
        let family = Family::named(&config.font).unwrap_or_default();
        // SAFETY: one thread; painters read FAMILY only when they are made.
        if unsafe { FAMILY } != family {
            unsafe { FAMILY = family; }
            for pane in self.panes.values_mut() { pane.new_font(); }
            self.painter = painter(self.painter.cell_w, self.painter.cell_h);
        }
        // SAFETY: one thread; panes read DOMAINS when they start `ssh`.
        unsafe { *core::ptr::addr_of_mut!(DOMAINS) = config.ssh.clone(); }
        self.theme = theme;
        for pane in self.panes.values_mut() {
            pane.screen.set_theme(theme);
            pane.screen.set_history_max(config.scrollback);
            pane.engine.theme_name = String::from(theme.name);
            pane.engine.font_name = config.font.clone();
        }
        let step = config.font_size as isize - self.cell_h as isize;
        self.config = config;
        self.set_text_size(step);
        self.relayout = true;
    }
    /// A pane's text area inside its place: the padding all round.
    fn inner(&self, r: Rect) -> Rect {
        let p = self.config.padding.min(r.w / 4).min(r.h / 4);
        Rect::new(r.x + p, r.y + p, r.w - 2 * p, r.h - 2 * p)
    }
    /// The height of the tab bar: shown once there are two tabs.
    fn bar_h(&self) -> usize { if self.tabs.len() > 1 { self.painter.cell_h + 8 } else { 0 } }
    /// Compact Kitty-style Noir status bar at the bottom of the window.
    fn status_h(&self) -> usize { self.painter.cell_h + 6 }
    /// Where the panes go (`y == 0` for single tab so split coords preserve `+0+0`).
    fn area(&self) -> Rect {
        let top = self.bar_h();
        let bot = self.status_h().min(self.height.saturating_sub(top + 32));
        Rect::new(0, top, self.width, self.height.saturating_sub(top + bot))
    }
    fn focused(&mut self) -> &mut Pane { let id = self.tabs.focus(); self.panes.get_mut(&id).expect("the focused pane exists") }
    fn new_pane(&mut self, cwd: Vec<u8>) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        let area = self.area();
        let mut pane = Pane::new(area.w / 2, area.h / 2, self.cell_h);
        pane.cwd = cwd;
        pane.screen.set_theme(self.theme);
        pane.screen.set_history_max(self.config.scrollback);
        pane.engine.theme_name = String::from(self.theme.name);
        pane.engine.font_name = self.config.font.clone();
        // The lowest relay slot no other pane holds; past the last, the pane runs only
        // Wasm programs.
        let used: Vec<u8> = self.panes.values().map(|p| p.slot).collect();
        pane.slot = (0..sex_pdx::console_relay::SLOTS as u8).find(|s| !used.contains(s)).unwrap_or(u8::MAX);
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
        if let Some(mut pane) = self.panes.remove(&id) { pane.leave(); }
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
            (true, true, 0x21) => self.focused().enter_mode("search"),
            (true, true, 0x39) => self.focused().enter_mode("hints"),
            (true, true, 0x2D) => self.focused().enter_mode("copy"),
            (true, true, c @ (0x25 | 0x24)) => { let p = self.focused(); let r = p.screen.jump_prompt(if c == 0x25 { -1 } else { 1 }); sex_pdx::serial_record!("[term.prompt.jump] row={:?}", r); }
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
                    if button == 1 { let panes = self.tabs.close(i); for id in panes { if let Some(mut pane) = self.panes.remove(&id) { pane.leave(); } } if self.tabs.is_empty() { sex_pdx::task_finish(0) } }
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
        let Some(r) = self.tabs.current().rect_of(id, area, GAP).map(|r| self.inner(r)) else { return };
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
            for e in pane.screen.take_events() { pane_event(*id, pane, e, &mut self.clipboard); }
            if pane.closed { gone.push(*id); }
        }
        for id in gone { self.close_pane(id); }
    }
    /// A program or animated TUI runs or a pane is gliding: the loop wakes at ~60fps/30fps.
    fn running(&self) -> bool {
        self.panes.values().any(|p| {
            p.job.is_some()
                || p.tui.as_ref().is_some_and(|t| matches!(t, ActiveTui::Bars(_) | ActiveTui::Btop(_)))
                || p.glide.left != 0
                || !p.glide.shown.is_empty()
        })
    }

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
            pixels[..w * h].fill(self.theme.bg);
            let rects: Vec<String> = layout.iter().map(|(id, r)| format!("{}:{}x{}+{}+{}", id, r.w, r.h, r.x, r.y)).collect();
            sex_pdx::serial_record!("[term.panes] tabs={} active={} focus={} zoomed={} {}", self.tabs.len(), self.tabs.active, tab.focus, tab.zoomed as u8, rects.join(" "));
        }
        let layout: Vec<(PaneId, Rect)> = layout.into_iter().map(|(id, r)| (id, self.inner(r))).collect();
        // A pane out of sight (another tab, or zoomed past) stops gliding, so the loop
        // does not keep waking for it.
        for (id, pane) in self.panes.iter_mut() { if !layout.iter().any(|(l, _)| l == id) && (pane.glide.active() || !pane.glide.shown.is_empty()) { pane.glide = Glide::default(); } }
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
                pixels[at..at + pw].copy_from_slice(&pane.shown()[row * pane.width..row * pane.width + pw]);
            }
        }
        let status_sig = self.current_status_sig();
        let status_dirty = relayout || changed || status_sig != self.status_sig;
        if changed || status_dirty {
            // The dividers, the one beside the focused pane highlighted.
            let focus_rect = tab.rect_of(tab.focus, area, GAP);
            for d in tab.dividers(area, GAP) {
                let near = focus_rect.is_some_and(|f| d.x + d.w >= f.x && d.x <= f.x + f.w && d.y + d.h >= f.y && d.y <= f.y + f.h);
                for row in d.y..(d.y + d.h).min(h) { let at = row * w; pixels[at + d.x..at + (d.x + d.w).min(w)].fill(if near && layout.len() > 1 { DIVIDER_FOCUS } else { DIVIDER }); }
            }
            if relayout { self.draw_tab_bar(pixels); }
            if status_dirty {
                self.status_sig = status_sig;
                self.draw_status_bar(pixels);
            }
            window.show_pixels(w, h);
        }
        // The window's title: the focused pane's, and how many tabs there are.
        let focused = self.panes.get(&tab.focus).map(|p| p.title()).unwrap_or_default();
        let title = if self.tabs.len() > 1 { format!("Spindle [{}/{}]: {}", self.tabs.active + 1, self.tabs.len(), focused) } else { format!("Spindle: {}", focused) };
        if title != self.title { window.title(title.as_bytes()); self.title = title; }
    }
    fn current_status_sig(&self) -> String {
        let id = self.tabs.focus();
        let Some(p) = self.panes.get(&id) else { return String::new() };
        let mode_lbl = if let Some(t) = &p.tui {
            format!("tui:{}", t.name())
        } else {
            match &p.mode {
                Mode::Normal => String::from("shell"),
                Mode::Search { .. } => String::from("find"),
                Mode::Hints { .. } => String::from("hints"),
                Mode::Copy(_) => String::from("copy"),
            }
        };
        let cwd = core::str::from_utf8(&p.cwd).unwrap_or("?");
        format!(
            "{}:{}:{}:{}x{}:{}:{}:{}",
            id, mode_lbl, cwd, p.screen.cols(), p.screen.rows(), self.theme.name, self.config.font, self.tabs.len()
        )
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
    fn draw_status_bar(&mut self, pixels: &mut [u32]) {
        let sh = self.status_h();
        let (w, h) = (self.width, self.height);
        let area_bot = self.bar_h() + self.area().h;
        if area_bot >= h || sh < 8 { return; }
        let y0 = area_bot;
        let row_h = (h - y0).min(sh);
        for row in y0..y0 + row_h {
            pixels[row * w..(row + 1) * w].fill(STATUS_BG);
        }
        pixels[y0 * w..(y0 + 1) * w].fill(DIVIDER);

        let id = self.tabs.focus();
        let Some(p) = self.panes.get(&id) else { return };
        let (mode_lbl, mode_col) = if let Some(t) = &p.tui {
            (format!(" TUI:{} ", t.name().to_uppercase()), STATUS_ROSE)
        } else {
            match &p.mode {
                Mode::Normal => (String::from(" SHELL "), 0x10B981),
                Mode::Search { .. } => (String::from(" FIND "), 0xF59E0B),
                Mode::Hints { .. } => (String::from(" HINTS "), STATUS_CYAN),
                Mode::Copy(_) => (String::from(" COPY "), 0xC084FC),
            }
        };
        let cwd = core::str::from_utf8(&p.cwd).unwrap_or("?");
        let right = if self.config.opacity < 100 {
            format!("{}x{} · {} · {}% ", p.screen.cols(), p.screen.rows(), self.theme.name, self.config.opacity)
        } else {
            format!("{}x{} · {} · {} ", p.screen.cols(), p.screen.rows(), self.theme.name, self.config.font)
        };

        let ty = y0 + row_h.saturating_sub(self.painter.cell_h) / 2;
        let cache = unsafe { &mut *core::ptr::addr_of_mut!(CACHE) };
        let mut x = 6usize;
        x = self.painter.text(cache, pixels, w, x, ty, "● SPINDLE ", STATUS_ROSE, STATUS_BG);
        let pill_end = (x + mode_lbl.chars().count() * self.painter.cell_w).min(w);
        for row in y0 + 2..y0 + row_h.saturating_sub(1) {
            if x < pill_end { pixels[row * w + x..row * w + pill_end].fill(STATUS_PILL); }
        }
        x = self.painter.text(cache, pixels, w, x, ty, &mode_lbl, mode_col, STATUS_PILL);
        x += self.painter.cell_w;
        let cwd_text = format!("~/{}  [Ctrl+R find · btop · y]", cwd);
        let right_w = right.chars().count() * self.painter.cell_w;
        let avail_cols = w.saturating_sub(x + right_w + 12) / self.painter.cell_w.max(1);
        let mid: String = cwd_text.chars().take(avail_cols).collect();
        self.painter.text(cache, pixels, w, x, ty, &mid, TAB_TEXT_ACTIVE, STATUS_BG);
        if w > right_w + 8 {
            self.painter.text(cache, pixels, w, w - right_w - 6, ty, &right, STATUS_MUTED, STATUS_BG);
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }

extern "C" fn main() -> ! {
    let mut window = Window::open();
    let _ = window.want_keys(sex_pdx::app_key::DESKTOP_KEYS);
    window.title(b"Spindle");
    let (w, h) = window.size();
    let mut spindle = Spindle::new(w, h);
    let term = spindle.focused();
    term.say("\x1b[1;38;2;244;63;94m● Spindle\x1b[0m \x1b[38;2;100;116;139m· SexOS Noir Terminal (Fish + P10k + Ratatui)\x1b[0m\n");
    term.say("Type \x1b[1;38;2;34;211;238mhelp\x1b[0m for commands · \x1b[38;2;16;185;129mTab\x1b[0m menu · \x1b[38;2;45;180;196m→/Ctrl+F\x1b[0m ghost · \x1b[38;2;192;132;252mbtop\x1b[0m · \x1b[38;2;245;158;11my\x1b[0m files · \x1b[38;2;244;63;94mfetch/bars\x1b[0m\n");
    // `run term CMD...` runs that command first.
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    // Spindle's grant (`spindle` adds `+launch`) is not a command.
    let given: Vec<u8> = files::args(&mut window, &mut args).split(|b| *b == b' ')
        .filter(|w| !w.is_empty() && *w != sex_pdx::console_relay::GRANT_TOKEN).collect::<Vec<_>>().join(&b' ');
    let term = spindle.focused();
    // Programs left running when their windows closed (yarns): say how to get them back.
    if let Ok(yarns) = files::direct::relay::yarns() {
        let running = yarns.iter().filter(|y| y.state != sex_pdx::console_relay::ENDED).count();
        if running > 0 { term.say(&format!("{} program{} kept running after {} window closed: type \x1b[1mjobs\x1b[0m, then \x1b[1mattach N\x1b[0m.\n", running, if running == 1 { "" } else { "s" }, if running == 1 { "its" } else { "their" })); }
    }
    term.show_prompt();
    if !given.is_empty() {
        term.line = given.clone();
        term.at = given.len();
        term.redraw_line();
        let line = core::mem::take(&mut term.line);
        term.say("\r\n");
        term.run_line(&mut window, &line);
        if term.job.is_none() && term.tui.is_none() { term.show_prompt(); }
    }
    spindle.draw(&mut window);
    let (cols, rows) = (spindle.focused().screen.cols(), spindle.focused().screen.rows());
    sex_pdx::serial_record!("[term.ready] target={} task={} cols={} rows={}", window.target, window.task, cols, rows);
    loop {
        // While a program runs, look at its output often; otherwise sleep until an event.
        let event = if spindle.running() { window.next_within(16) } else { window.next_within(1000) };
        spindle.check_config(false);
        match event {
            Some(Event::Close) => {
                // Programs of their own go on as yarns: `attach` in any Spindle window.
                for pane in spindle.panes.values_mut() { pane.leave(); }
                sex_pdx::serial_record!("[term.closed] how=window");
                window.close()
            }
            Some(Event::Resize { width, height }) => { spindle.width = width.max(64); spindle.height = height.max(64); spindle.relayout = true; }
            Some(Event::Key(key)) => if !spindle.key(&mut window, key) { if let Some(t) = spindle.focused().key(&mut window, key) { spindle.clipboard = t; } },
            Some(Event::Pointer(p)) => spindle.pointer(p),
            Some(Event::Drop { .. } | Event::DragEnded { .. } | Event::KeyUp(_)) => {}
            None => {}
        }
        spindle.pump();
        spindle.draw(&mut window);
    }
}
