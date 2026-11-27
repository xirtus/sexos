// SPDX-License-Identifier: GPL-3.0-only
//! `tether HOST[:PORT]`: lets a helper on another computer watch and drive this Spindle
//! during the first starts on new hardware (the Alienware x17 R1 has no serial port).
//!
//! Spindle connects *out* to HOST (default port 7177) over the ordinary network: nothing
//! listens on SexOS, and only the person at the keyboard can start it. Over that one TCP
//! flow it sends, as plain text lines:
//!
//! 1. `SEXOS-TETHER 1` and the kernel's copy of the boot log head (the first 256 KiB,
//!    the same text `hw all` reads), between `--- boot log head ---` markers;
//! 2. every line Spindle has shown (what is still in its scrollback), then every new line
//!    as it appears.
//!
//! Each line the other side sends back is run as if typed, and shown first as
//! `remote> LINE`, so the person sees every command. A few commands that write disks or
//! boot entries (`install go`, `update`, `format`) are refused from the tether: those stay
//! the person's to type. `tether off` or closing Spindle ends it; `tether`
//! alone says whether it is up. The host side is `scripts/tether_host.py`.
//!
//! `tether sweep HOST [ROUNDS]` runs Wi-Fi transfer trials instead (see [`SHAPES`]): each
//! trial sets the Wi-Fi test switches (`wifi ba`, `wifi replay`), connects, moves data in
//! one traffic shape and ends as pass, stall, timeout or a network failure; then `wifi`
//! logs the link counters and the next trial starts. Every result and the counters after
//! it are written to `SWEEP_FILE` on the Ventoy stick after each trial, so one unattended
//! run shows which switch and which shape breaks the link.
//!
//! The flow is the network service's flow `FLOW` of this Spindle, so it never collides
//! with `fetch` (flow 0); the shared region still carries one call at a time, so `fetch`,
//! `update` and `pkg` wait for `tether off`.

use crate::Scrollback;
use sex_pdx::crash_log as log;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;

const FLOW: u8 = 1;
/// `tether listen`: the listening flow (the connection it accepts becomes `FLOW`).
const LISTEN_FLOW: u8 = 2;
/// Between asks whether the Mac has connected.
const ACCEPT_MS: u64 = 250;
pub const DEFAULT_PORT: u16 = 7177;
/// Between receives while nothing arrives.
const POLL_MS: u64 = 40;
const OUT_MAX: usize = 16 * 1024;
/// Bytes handed to the network per send: a 16 KiB send filled the flow's whole
/// send buffer at once and stalled the x17's first tether over Wi-Fi.
const SEND_MAX: usize = 4096;
const IN_MAX: usize = 2048;
const QUEUE: usize = 16;
pub const CMD_MAX: usize = 256;
/// Empty receives between asks whether the other side is still there.
const STATE_EVERY: u32 = 25;
/// Connecting gives up after this long.
const CONNECT_MS: u64 = 20_000;
/// Automatic outbound connect attempts before failing (handles post-Wi-Fi-join ARP/DHCP warm-up).
const CONNECT_MAX_ATTEMPTS: u8 = 3;
/// Short backoff between automatic outbound connect retries.
const CONNECT_RETRY_MS: u64 = 1_750;
/// An answer not back by then is taken as lost.
const LOST_MS: u64 = 1_500;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase { Idle, Up,
    /// An IPv4 host: wait until DHCP gave this machine an IPv4 address (an IPv6-only
    /// network is up too, but cannot reach it).
    V4Wait, ResolveAsk, ResolveWait, Connect, Opening,
    /// Closing a transiently failed connect flow before backing off for a retry.
    RetryClose,
    /// Waiting `CONNECT_RETRY_MS` before the next automatic connect attempt.
    RetryWait,
    Live, Close,
    /// `tether listen`: make the listener, say the address, wait for the Mac, and drop a
    /// finished connection to wait for the next one.
    Listen, Addresses, Accept, Drop, CloseListener,
    /// `tether sweep`: between trials (counters, file, switches for the next one).
    Gap }

/// One kind of traffic a sweep trial makes.
pub struct Shape {
    pub name: &'static str,
    /// Bytes per send (one network call).
    pub chunk: usize,
    /// Pause after each send.
    pub pace_ms: u64,
    /// Bytes to send to the Mac.
    pub up: u64,
    /// Bytes the Mac sends back (`tether_host.py` answers `down=N` in the header).
    pub down: u64,
    /// Quiet time after connecting, before the data (the Mac's ARP entry ages meanwhile).
    pub idle_ms: u64,
}

pub const SHAPES: [Shape; 8] = [
    Shape { name: "bulk", chunk: 4096, pace_ms: 0, up: 512 * 1024, down: 0, idle_ms: 0 },
    Shape { name: "mid", chunk: 1024, pace_ms: 0, up: 256 * 1024, down: 0, idle_ms: 0 },
    Shape { name: "small", chunk: 128, pace_ms: 0, up: 64 * 1024, down: 0, idle_ms: 0 },
    Shape { name: "paced", chunk: 4096, pace_ms: 100, up: 256 * 1024, down: 0, idle_ms: 0 },
    Shape { name: "trickle", chunk: 256, pace_ms: 500, up: 16 * 1024, down: 0, idle_ms: 0 },
    Shape { name: "idle45", chunk: 4096, pace_ms: 0, up: 64 * 1024, down: 0, idle_ms: 45_000 },
    Shape { name: "down", chunk: 128, pace_ms: 0, up: 0, down: 512 * 1024, idle_ms: 0 },
    Shape { name: "long", chunk: 4096, pace_ms: 0, up: 2 * 1024 * 1024, down: 0, idle_ms: 0 },
];
/// Switch combinations per shape: (block-ack reorder on, replay check on).
pub const KNOBS: [(bool, bool); 4] = [(true, true), (false, true), (true, false), (false, false)];
pub const TRIALS_PER_ROUND: u32 = (SHAPES.len() * KNOBS.len()) as u32;
const SWEEP_FILE: &[u8] = b"/media/Ventoy/wifi-sweep.txt";
/// No byte moved either way for this long: the trial stalled.
const STALL_MS: u64 = 20_000;
const TRIAL_MAX_MS: u64 = 240_000;
const SWEEP_ROUNDS: u32 = 4;
const SWEEP_MAX_ROUNDS: u32 = 50;
const RESULTS_MAX: usize = 200 * 1024;
/// The sweep's results, allocated when a sweep starts: a 200 KiB static cost every
/// Spindle 50 pages at boot (about 2 MiB in all), enough to refuse Qupid's fifth window.
static mut RESULTS: alloc::vec::Vec<u8> = alloc::vec::Vec::new();

/// Trial `k`'s shape and switch setting, as indexes: all four switch settings of a shape
/// run together. `only`: every trial uses that shape.
pub fn trial_plan(k: u32, only: Option<usize>) -> (usize, usize) {
    let knob = k as usize % KNOBS.len();
    match only {
        Some(shape) => (shape, knob),
        None => ((k % TRIALS_PER_ROUND) as usize / KNOBS.len(), knob),
    }
}

fn on(b: bool) -> &'static str { if b { "on" } else { "off" } }

#[derive(Clone, Copy)]
struct Sweep {
    active: bool,
    total: u32,
    /// The trial running (or the one just ended, in the gap).
    trial: u32,
    next_trial: u32,
    gap_step: u8,
    /// `sb.total_lines` when the trial ended: the `wifi` lines after it are kept.
    mark: u32,
    started: u64,
    live_at: u64,
    progress_at: u64,
    header_sent: bool,
    /// Filler bytes queued for sending.
    filler: u64,
    bytes_in: u64,
    last_pass: bool,
    /// `tether sweep HOST ROUNDS SHAPE`: one shape only.
    only: Option<usize>,
    passes: [u16; TRIALS_PER_ROUND as usize],
    runs: [u16; TRIALS_PER_ROUND as usize],
}
const SWEEP_IDLE: Sweep = Sweep { active: false, total: 0, trial: 0, next_trial: 0, gap_step: 0, mark: 0, started: 0, live_at: 0, progress_at: 0, header_sent: false, filler: 0, bytes_in: 0, last_pass: true, only: None, passes: [0; TRIALS_PER_ROUND as usize], runs: [0; TRIALS_PER_ROUND as usize] };

#[derive(Clone, Copy)]
struct Resume {
    active: bool,
    listening: bool,
    host: [u8; 128],
    host_len: usize,
    port: u16,
}
const RESUME_NONE: Resume = Resume { active: false, listening: false, host: [0; 128], host_len: 0, port: DEFAULT_PORT };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Call { Plain, Send, Receive, State, Close }

/// The largest server image a hot swap takes.
const HOTSWAP_BUF_CAP: usize = 16 * 1024 * 1024;
/// Where a hot-swap image is gathered: pages this run asks for the first time one comes
/// (Spindle has no heap, and a static would cost every Spindle its size at boot), kept
/// for the next. (base, bytes held)
static mut HOTSWAP_MEM: (u64, usize) = (0, 0);
/// At least `need` bytes of hot-swap buffer, growing the run's pages as needed.
fn hotswap_buf(need: usize) -> Option<*mut u8> {
    use sex_pdx::program_memory as m;
    let (mut base, mut held) = unsafe { HOTSWAP_MEM };
    while held < need.max(1) {
        let pages = ((need - held) as u64).div_ceil(m::PAGE).min(m::MAX_PAGES_PER_CALL);
        let got = m::grow(pages).ok()?;
        // Pages follow the run's first page without gaps: the buffer is base..end.
        if base == 0 { base = got.base; }
        held = (got.end() - base) as usize;
    }
    unsafe { HOTSWAP_MEM = (base, held); }
    Some(base as *mut u8)
}

struct HotswapRx {
    active: bool,
    discard: bool,
    pd: u32,
    name: [u8; 24],
    name_len: usize,
    expected_len: usize,
    received: usize,
    expected_fnv: u64,
}
const HOTSWAP_IDLE: HotswapRx = HotswapRx {
    active: false, discard: false, pd: 0, name: [0; 24], name_len: 0,
    expected_len: 0, received: 0, expected_fnv: 0,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PushPhase { Idle, Opening, Ready, Writing, Discard }

struct PushfileRx {
    phase: PushPhase,
    fs_submitted: bool,
    path: [u8; log::PUSHFILE_MAX_PATH],
    path_len: usize,
    expected_len: usize,
    received: usize,
    expected_fnv: u64,
    fnv: u64,
}
const PUSHFILE_IDLE: PushfileRx = PushfileRx {
    phase: PushPhase::Idle, fs_submitted: false, path: [0; log::PUSHFILE_MAX_PATH], path_len: 0,
    expected_len: 0, received: 0, expected_fnv: 0, fnv: log::FNV64_OFFSET,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PullPhase { Idle, Stat, Reading }

const PULL_CHUNK_BYTES: usize = 1024;
const PULL_OUT_HEADROOM: usize = PULL_CHUNK_BYTES * 2 + 64;

struct PullfileTx {
    phase: PullPhase,
    fs_submitted: bool,
    path: [u8; log::PUSHFILE_MAX_PATH],
    path_len: usize,
    total_len: u64,
    offset: u64,
    fnv: u64,
}
const PULLFILE_IDLE: PullfileTx = PullfileTx {
    phase: PullPhase::Idle, fs_submitted: false, path: [0; log::PUSHFILE_MAX_PATH], path_len: 0,
    total_len: 0, offset: 0, fnv: log::FNV64_OFFSET,
};

pub struct Tether {
    phase: Phase,
    seq: u32,
    sent: bool,
    /// When the call out was made: an answer lost on the way is asked for again.
    sent_at: u64,
    call: Call,
    next_at: u64,
    deadline: u64,
    /// Said once that it waits for DHCP.
    told_v4: bool,
    host: [u8; 128],
    host_len: usize,
    port: u16,
    address: [u8; 4],
    out: [u8; OUT_MAX],
    out_len: usize,
    /// The boot log head's next offset to send; `usize::MAX` once it all went.
    log_at: usize,
    log_total: usize,
    /// Scrollback lines (counted since Spindle started) already sent.
    lines_sent: u32,
    inbound: [u8; IN_MAX],
    in_len: usize,
    queue: [[u8; CMD_MAX]; QUEUE],
    queue_len: [usize; QUEUE],
    queue_head: usize,
    queue_count: usize,
    empty_receives: u32,
    recv_turn: bool,
    bytes_out: u64,
    commands: u64,
    /// `tether listen`: SexOS waits and the Mac connects (no typing needed to aim it).
    listening: bool,
    sweep: Sweep,
    hotswap: HotswapRx,
    pushfile: PushfileRx,
    pullfile: PullfileTx,
    /// Control tether to reconnect automatically after a `tether sweep` completes.
    resume: Resume,
    /// Current outbound connect attempt (`1..=CONNECT_MAX_ATTEMPTS`, or `0` when idle/live).
    connect_attempt: u8,
}

pub static mut TETHER: Tether = Tether {
    phase: Phase::Idle, seq: 0, sent: false, sent_at: 0, call: Call::Plain, next_at: 0, deadline: 0, told_v4: false,
    host: [0; 128], host_len: 0, port: DEFAULT_PORT, address: [0; 4],
    out: [0; OUT_MAX], out_len: 0, log_at: 0, log_total: 0, lines_sent: 0,
    inbound: [0; IN_MAX], in_len: 0, queue: [[0; CMD_MAX]; QUEUE], queue_len: [0; QUEUE],
    queue_head: 0, queue_count: 0, empty_receives: 0, recv_turn: false, bytes_out: 0, commands: 0, listening: false,
    sweep: SWEEP_IDLE,
    hotswap: HOTSWAP_IDLE,
    pushfile: PUSHFILE_IDLE,
    pullfile: PULLFILE_IDLE,
    resume: RESUME_NONE,
    connect_attempt: 0,
};

fn net() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(w::CLIENT_SLOT, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
}

/// `a.b.c.d` -> its four bytes.
pub fn numeric(host: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = host.split(|b| *b == b'.');
    for slot in out.iter_mut() {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.iter().all(u8::is_ascii_digit) { return None; }
        let value = part.iter().fold(0u32, |v, b| v * 10 + (*b - b'0') as u32);
        if value > 255 { return None; }
        *slot = value as u8;
    }
    if parts.next().is_some() { return None; }
    Some(out)
}

/// An IPv6 address in text (`fd00::1`, full or with one `::`).
pub fn numeric6(host: &[u8]) -> Option<[u8; 16]> {
    let groups = |part: &[u8], out: &mut [u16; 8]| -> Option<usize> {
        if part.is_empty() { return Some(0); }
        let mut n = 0;
        for g in part.split(|b| *b == b':') {
            if n == 8 || g.is_empty() || g.len() > 4 { return None; }
            let mut v = 0u16;
            for b in g { v = v << 4 | (*b as char).to_digit(16)? as u16; }
            out[n] = v; n += 1;
        }
        Some(n)
    };
    let (mut head, mut tail) = ([0u16; 8], [0u16; 8]);
    let words = match host.windows(2).position(|w| w == b"::") {
        Some(i) => {
            let (h, t) = (groups(&host[..i], &mut head)?, groups(&host[i + 2..], &mut tail)?);
            if h + t > 7 { return None; }
            let mut all = [0u16; 8];
            all[..h].copy_from_slice(&head[..h]);
            all[8 - t..].copy_from_slice(&tail[..t]);
            all
        }
        None => { if groups(host, &mut head)? != 8 { return None; } head }
    };
    let mut out = [0u8; 16];
    for (k, w) in words.iter().enumerate() { out[2 * k..2 * k + 2].copy_from_slice(&w.to_be_bytes()); }
    Some(out)
}

/// `HOST[:PORT]` -> (host, port). IPv6: `[ADDRESS]:PORT`, or a bare address (port 7177).
pub fn parse_target(arg: &[u8]) -> Option<(&[u8], u16)> {
    if let Some(rest) = arg.strip_prefix(b"[") {
        let close = rest.iter().position(|b| *b == b']')?;
        let port = match &rest[close + 1..] { b"" => DEFAULT_PORT, [b':', digits @ ..] => parse_port(digits)?, _ => return None };
        let host = &rest[..close];
        numeric6(host)?;
        return Some((host, port));
    }
    if arg.iter().filter(|b| **b == b':').count() > 1 { numeric6(arg)?; return Some((arg, DEFAULT_PORT)); }
    let (host, port) = match arg.iter().rposition(|b| *b == b':') {
        Some(i) => {
            let digits = &arg[i + 1..];
            if digits.is_empty() || digits.len() > 5 || !digits.iter().all(u8::is_ascii_digit) { return None; }
            let p = digits.iter().fold(0u32, |v, b| v * 10 + (*b - b'0') as u32);
            if !(1..=65535).contains(&p) { return None; }
            (&arg[..i], p as u16)
        }
        None => (arg, DEFAULT_PORT),
    };
    if host.is_empty() || host.len() > 127 || host.iter().any(|b| *b <= b' ') { return None; }
    Some((host, port))
}

fn parse_port(digits: &[u8]) -> Option<u16> {
    if digits.is_empty() || digits.len() > 5 || !digits.iter().all(u8::is_ascii_digit) { return None; }
    let p = digits.iter().fold(0u32, |v, b| v * 10 + (*b - b'0') as u32);
    if (1..=65535).contains(&p) { Some(p as u16) } else { None }
}

/// Commands the tether may not run: they write disks or boot entries.
pub fn refused(line: &[u8]) -> bool {
    let mut words = line.split(|b| b.is_ascii_whitespace()).filter(|w| !w.is_empty());
    match words.next() {
        // Files runs the install when `go` is any of its words.
        Some(b"install") => words.any(|w| w == b"go"),
        Some(b"update") | Some(b"format") | Some(b"mkfs") => true,
        _ => false,
    }
}

struct Text { buf: [u8; 160], len: usize }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len]);
    sex_pdx::serial_record!("[spindle.tether.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}

impl Tether {
    /// The running trial's shape, switches (ba, replay) and summary slot.
    fn plan(&self) -> (&'static Shape, (bool, bool), usize) {
        let (shape, knob) = trial_plan(self.sweep.trial, self.sweep.only);
        (&SHAPES[shape], KNOBS[knob], shape * KNOBS.len() + knob)
    }

    /// Connecting or connected.
    pub fn active(&self) -> bool { self.phase != Phase::Idle }
    /// Running a Wi-Fi benchmark sweep (`tether sweep`).
    pub fn sweeping(&self) -> bool { self.sweep.active }
    /// Has a network call out (the shared region is in use).
    pub fn in_call(&self) -> bool { self.sent }
    /// True while a live control tether has >= 512 unsent scrollback lines, so `cat`
    /// paces its reads instead of overflowing the 1024-line scrollback ring.
    pub fn backpressure(&self, sb: &Scrollback) -> bool {
        self.phase == Phase::Live && !self.sweep.active && self.log_at == usize::MAX
            && sb.total_lines.saturating_sub(self.lines_sent) >= 512
    }

    pub fn command(&mut self, args: &[u8], sb: &mut Scrollback, fetch_busy: bool) {
        let args = trim(args);
        if args.is_empty() {
            if self.phase == Phase::Live {
                say!(sb, "tether: up to {}:{}, {} bytes sent, {} commands run", core::str::from_utf8(&self.host[..self.host_len]).unwrap_or("?"), self.port, self.bytes_out, self.commands);
            } else if matches!(self.phase, Phase::RetryWait | Phase::RetryClose) {
                say!(sb, "tether: retrying ({}/{})...", self.connect_attempt, CONNECT_MAX_ATTEMPTS);
            } else if self.active() {
                sb.push(b"tether: connecting.");
            } else {
                sb.push(b"Usage: tether HOST[:PORT] (default port 7177), tether off");
                sb.push(b"Wi-Fi test run: tether sweep HOST [ROUNDS] (results in /media/Ventoy/wifi-sweep.txt)");
                sb.push(b"On the other computer: python3 scripts/tether_host.py");
            }
            return;
        }
        if args == b"off" || args == b"stop" {
            if !self.active() { sb.push(b"tether: not running."); return; }
            self.connect_attempt = 0;
            self.resume = RESUME_NONE;
            if self.phase == Phase::RetryWait {
                self.phase = Phase::Idle;
                sb.push(b"tether: closed.");
                return;
            }
            sb.push(b"tether: closing.");
            if self.sweep.active {
                self.sweep.active = false;
                self.note(b"sweep stopped by tether off");
                self.save(sb);
                unsafe {
                    let r = &mut *core::ptr::addr_of_mut!(RESULTS);
                    r.clear();
                    r.shrink_to_fit();
                }
                if self.phase == Phase::Gap { self.phase = Phase::Idle; return; }
            }
            self.end();
            return;
        }
        if args == b"sweep" || args.starts_with(b"sweep ") {
            if self.sweep.active { sb.push(b"tether: a sweep is already running (tether off to stop it)."); return; }
            if fetch_busy { sb.push(b"tether: a download is using the network; try again when it ends."); return; }
            let mut words = args[5..].split(|b| *b == b' ').filter(|w| !w.is_empty());
            let mut sweep_host = [0u8; 128];
            let (target, rounds, only) = if args == b"sweep" && self.active() && !self.listening && self.host_len > 0 {
                sweep_host[..self.host_len].copy_from_slice(&self.host[..self.host_len]);
                (Some((self.host_len, self.port)), Some(SWEEP_ROUNDS), Some(None))
            } else {
                let target = words.next().and_then(parse_target).map(|(h, p)| {
                    sweep_host[..h.len()].copy_from_slice(h);
                    (h.len(), p)
                });
                let rounds = match words.next() {
                    None => Some(SWEEP_ROUNDS),
                    Some(w) => core::str::from_utf8(w).ok().and_then(|t| t.parse::<u32>().ok()).filter(|n| (1..=SWEEP_MAX_ROUNDS).contains(n)),
                };
                let only = match words.next() {
                    None => Some(None),
                    Some(w) => SHAPES.iter().position(|s| s.name.as_bytes() == w).map(Some),
                };
                (target, rounds, only)
            };
            let (Some((hlen, port)), Some(rounds), Some(only)) = (target, rounds, only) else {
                sb.push(b"Usage: tether sweep HOST[:PORT] [ROUNDS 1-50] [SHAPE]");
                sb.push(b"Shapes: bulk mid small paced trickle idle45 down long");
                return;
            };
            let was_active = self.active();
            if was_active {
                self.resume = Resume { active: true, listening: self.listening, host: self.host, host_len: self.host_len, port: self.port };
            } else {
                self.resume = RESUME_NONE;
                self.listening = false;
            }
            self.connect_attempt = 0;
            self.host[..hlen].copy_from_slice(&sweep_host[..hlen]);
            self.host_len = hlen;
            self.port = port;
            if !was_active { self.reset(sb); }
            self.sweep = SWEEP_IDLE;
            self.sweep.active = true;
            self.sweep.only = only;
            self.sweep.total = rounds * if only.is_some() { KNOBS.len() as u32 } else { TRIALS_PER_ROUND };
            self.sweep.gap_step = 2;
            unsafe {
                let r = &mut *core::ptr::addr_of_mut!(RESULTS);
                r.clear();
                let _ = r.try_reserve_exact(RESULTS_MAX);
            }
            let host_str = core::str::from_utf8(&sweep_host[..hlen]).unwrap_or("?");
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("SexOS Wi-Fi sweep to {}:{}, {} trials ({} rounds)", host_str, port, self.sweep.total, rounds));
            let (b, n) = (t.buf, t.len);
            self.note(&b[..n]);
            self.note(b"result: pass = all data moved; stall = nothing moved for 20 s; others = the network's answer");
            if was_active {
                say!(sb, "tether sweep: pausing tether for {} trials to {}:{}; will reconnect after.", self.sweep.total, host_str, port);
                self.end();
            } else {
                say!(sb, "tether sweep: {} trials to {}:{}; tether off stops it. Results go to /media/Ventoy/wifi-sweep.txt", self.sweep.total, host_str, port);
                self.phase = Phase::Gap;
                self.next_at = 0;
            }
            return;
        }
        if self.active() { sb.push(b"tether: already running (tether off first)."); return; }
        if args == b"listen" || args.starts_with(b"listen ") {
            let rest = trim(&args[6..]);
            let port = if rest.is_empty() { Some(DEFAULT_PORT) } else { parse_port(rest) };
            let Some(port) = port else { sb.push(b"Usage: tether listen [PORT]"); return };
            self.reset(sb);
            self.listening = true;
            self.connect_attempt = 0;
            self.port = port;
            self.host_len = 0;
            self.phase = Phase::Up;
            say!(sb, "tether: waiting for the network, then listening on port {}", port);
            return;
        }
        let Some((host, port)) = parse_target(args) else { sb.push(b"Usage: tether HOST[:PORT] | tether listen [PORT] | tether sweep HOST [ROUNDS]"); return };
        self.listening = false;
        self.connect_attempt = 1;
        self.host[..host.len()].copy_from_slice(host);
        self.host_len = host.len();
        self.port = port;
        self.reset(sb);
        self.phase = Phase::Up;
        say!(sb, "tether: connecting to {}:{}", core::str::from_utf8(host).unwrap_or("?"), port);
    }

    /// A fresh session: the boot log head and Spindle's retained lines go first.
    fn reset(&mut self, sb: &Scrollback) {
        self.out_len = 0;
        self.in_len = 0;
        self.queue_count = 0;
        self.queue_head = 0;
        self.log_at = 0;
        self.log_total = 0;
        self.bytes_out = 0;
        self.commands = 0;
        self.empty_receives = 0;
        self.lines_sent = sb.total_lines.saturating_sub(sb.retained() as u32);
        self.deadline = sex_pdx::get_ticks() + CONNECT_MS;
        self.told_v4 = false;
        self.next_at = 0;
        self.sent = false;
        self.hotswap_reset();
        self.pushfile_reset();
        self.pullfile_reset();
    }

    fn hotswap_reset(&mut self) {
        self.hotswap.active = false;
        self.hotswap.discard = false;
        self.hotswap.pd = 0;
        self.hotswap.name_len = 0;
        self.hotswap.expected_len = 0;
        self.hotswap.received = 0;
        self.hotswap.expected_fnv = 0;
    }

    fn pushfile_reset(&mut self) {
        self.pushfile.phase = PushPhase::Idle;
        self.pushfile.fs_submitted = false;
        self.pushfile.path_len = 0;
        self.pushfile.expected_len = 0;
        self.pushfile.received = 0;
        self.pushfile.expected_fnv = 0;
        self.pushfile.fnv = log::FNV64_OFFSET;
    }

    fn pullfile_reset(&mut self) {
        self.pullfile.phase = PullPhase::Idle;
        self.pullfile.fs_submitted = false;
        self.pullfile.path_len = 0;
        self.pullfile.total_len = 0;
        self.pullfile.offset = 0;
        self.pullfile.fnv = log::FNV64_OFFSET;
    }

    /// `pull-file <path>`: streams `<path>` back over the live tether TCP stream in framed hex lines.
    pub fn pull_file(&mut self, args: &[u8], sb: &mut Scrollback) {
        let path = trim(args);
        if path.is_empty() {
            sb.push(b"Usage: pull-file <path>");
            return;
        }
        if self.phase != Phase::Live || self.sweep.active {
            sb.push(b"pull-file: err tether is not active.");
            return;
        }
        if self.pullfile.phase != PullPhase::Idle {
            sb.push(b"pull-file: err a pull-file transfer is already in progress.");
            return;
        }
        if path.len() > log::PUSHFILE_MAX_PATH || path.iter().any(|&b| b <= b' ' || b == 0x7f) {
            sb.push(b"pull-file: err invalid path.");
            return;
        }
        self.pullfile_reset();
        self.pullfile.path[..path.len()].copy_from_slice(path);
        self.pullfile.path_len = path.len();
        self.pullfile.phase = PullPhase::Stat;
    }

    fn end(&mut self) {
        // A call still out is answered and dropped by `receive` (phase Close).
        self.connect_attempt = 0;
        self.hotswap_reset();
        self.pushfile_reset();
        self.pullfile_reset();
        self.phase = Phase::Close;
        self.next_at = 0;
    }

    fn fail(&mut self, sb: &mut Scrollback, why: &str) {
        if self.sweep.active { self.finish(sb, why); return; }
        say!(sb, "tether: {}", why);
        sex_pdx::serial_record!("[spindle.tether.end] reason={} bytes={} commands={}", why, self.bytes_out, self.commands);
        // Listening: a session that ends drops its connection and waits for the next.
        if self.listening && self.phase == Phase::Live { self.phase = Phase::Drop; self.next_at = 0; return; }
        self.end();
    }

    fn fail_retryable(&mut self, sb: &mut Scrollback, why: &str) {
        if !self.listening
            && !self.sweep.active
            && self.connect_attempt > 0
            && self.connect_attempt < CONNECT_MAX_ATTEMPTS
        {
            let need_close = matches!(self.phase, Phase::Connect | Phase::Opening);
            self.connect_attempt += 1;
            say!(sb, "tether: retrying ({}/{})...", self.connect_attempt, CONNECT_MAX_ATTEMPTS);
            sex_pdx::serial_record!("[spindle.tether.retry] attempt={} reason={}", self.connect_attempt, why);
            if need_close {
                self.phase = Phase::RetryClose;
                self.next_at = 0;
            } else {
                self.phase = Phase::RetryWait;
                self.next_at = sex_pdx::get_ticks() + CONNECT_RETRY_MS;
            }
            return;
        }
        self.fail(sb, why);
    }

    fn net_call(&mut self, kind: u8, value: u64, call: Call) { self.net_call_on(FLOW, kind, value, call) }
    fn net_call_on(&mut self, flow: u8, kind: u8, value: u64, call: Call) {
        // High bit 0x80_0000 keeps tether sequence numbers disjoint from `fetch` (0x00_0001..=0x7F_FFFF).
        self.seq = 0x80_0000 | ((self.seq + 1) & 0x7F_FFFF);
        if sex_pdx::pdx_call_checked(sex_pdx::SLOT_NET, w::OP, w::request_on(self.seq, kind, flow), value, 0).is_ok() {
            self.sent = true;
            self.sent_at = sex_pdx::get_ticks();
            self.call = call;
        } else {
            self.next_at = sex_pdx::get_ticks() + POLL_MS;
        }
    }

    /// Fills `out` with the next text to send: the boot log head, then new scrollback lines.
    fn fill(&mut self, sb: &Scrollback) {
        if self.sweep.active { self.fill_sweep(); return; }
        if self.log_at != usize::MAX {
            if self.log_at == 0 && self.out_len == 0 {
                self.put(b"SEXOS-TETHER 1\n--- boot log head ---\n");
            }
            let mut chunk = [0u8; log::LOG_CHUNK];
            while self.out_len + log::LOG_CHUNK + 64 <= OUT_MAX {
                match log::log_head(self.log_at, &mut chunk) {
                    Some((n, total)) if n > 0 => { self.log_total = total; self.put(&chunk[..n]); self.log_at += n; if self.log_at >= total { break; } }
                    _ => break,
                }
            }
            let done = log::log_head(self.log_at, &mut chunk).map_or(true, |(n, _)| n == 0);
            if done {
                self.put(b"\n--- end of boot log head ---\n--- spindle ---\n");
                sex_pdx::serial_record!("[spindle.tether.log] bytes={}", self.log_at);
                self.log_at = usize::MAX;
            }
            return;
        }
        if self.pullfile.phase != PullPhase::Idle { return; }
        let kept = sb.retained() as u32;
        let oldest = sb.total_lines.saturating_sub(kept);
        if self.lines_sent < oldest {
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("[tether] {} lines scrolled away before they could be sent\n", oldest - self.lines_sent));
            let n = t.len;
            let b = t.buf;
            self.put(&b[..n]);
            self.lines_sent = oldest;
        }
        while self.lines_sent < sb.total_lines {
            let line = sb.get_retained((self.lines_sent - oldest) as usize);
            if self.out_len + line.len() + 1 > OUT_MAX { break; }
            self.put(line);
            self.put(b"\n");
            self.lines_sent += 1;
        }
    }

    fn put(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(OUT_MAX - self.out_len);
        self.out[self.out_len..self.out_len + n].copy_from_slice(&bytes[..n]);
        self.out_len += n;
    }

    /// Sends the next call when one is due. True when the screen may need drawing.
    pub fn step(&mut self, sb: &mut Scrollback, region_busy: bool) -> bool {
        if self.phase == Phase::Idle { return false; }
        let now = sex_pdx::get_ticks();
        // sexnet keeps one answer per program waiting: when another command's call
        // (`wifi`) took it, ours never comes. Ask again; a late answer has an old number.
        if self.sent {
            if now.saturating_sub(self.sent_at) < LOST_MS { return false; }
            sex_pdx::serial_record!("[spindle.tether.resend] phase={:?} call={:?}", self.phase, self.call);
            self.sent = false;
            self.next_at = 0;
        }
        if self.phase == Phase::Live && self.sweep.active && self.sweep_check(sb, now) { return true; }
        if now < self.next_at { return false; }
        if self.phase == Phase::Gap { self.gap(sb, now); return true; }
        if self.phase == Phase::RetryWait {
            self.deadline = now + CONNECT_MS;
            self.told_v4 = false;
            self.phase = Phase::Up;
            self.next_at = 0;
        }
        if self.phase == Phase::V4Wait && now > self.deadline {
            self.fail_retryable(sb, "no IPv4 address: the router never answered DHCP (`hw dhcp` shows the lease when it comes)");
            return true;
        }
        if !self.listening
            && !matches!(self.phase, Phase::Live | Phase::Close | Phase::RetryClose | Phase::RetryWait)
            && now > self.deadline
        {
            self.fail_retryable(sb, "could not reach the other computer (is tether_host.py running, and the firewall open?)");
            return true;
        }
        if region_busy { return false; }
        let Some(region) = net() else { self.phase = Phase::Idle; sb.push(b"tether: no network here."); return true };
        match self.phase {
            Phase::Up => self.net_call(w::UP, 0, Call::Plain),
            Phase::V4Wait => self.net_call(w::ADDRESSES, 0, Call::Plain),
            Phase::ResolveAsk => { region[..self.host_len].copy_from_slice(&self.host[..self.host_len]); self.net_call(w::RESOLVE, self.host_len as u64, Call::Plain); }
            Phase::ResolveWait => self.net_call(w::RESOLVE, 0, Call::Plain),
            Phase::Connect => match numeric6(&self.host[..self.host_len]) {
                Some(a) => { w::put_address(region, w::FAMILY_V6, &a); self.net_call(w::CONNECT_TO, self.port as u64, Call::Plain) }
                None => self.net_call(w::CONNECT, w::address_port(self.address, self.port), Call::Plain),
            },
            Phase::Opening => self.net_call(w::STATE, 0, Call::Plain),
            Phase::RetryClose => self.net_call(w::CLOSE, 0, Call::Close),
            Phase::Close => self.net_call(w::CLOSE, 0, Call::Close),
            Phase::Listen => self.net_call_on(LISTEN_FLOW, w::LISTEN, self.port as u64, Call::Plain),
            Phase::Addresses => self.net_call_on(LISTEN_FLOW, w::ADDRESSES, 0, Call::Plain),
            Phase::Accept => self.net_call_on(LISTEN_FLOW, w::ACCEPT, FLOW as u64, Call::Plain),
            Phase::Drop => self.net_call(w::CLOSE, 0, Call::Plain),
            Phase::CloseListener => self.net_call_on(LISTEN_FLOW, w::CLOSE, 0, Call::Close),
            Phase::Live => {
                self.step_pushfile(sb);
                self.step_pullfile(sb);
                if self.out_len == 0 { self.fill(sb); }
                let push_rx = matches!(self.pushfile.phase, PushPhase::Ready | PushPhase::Discard);
                let push_wait = matches!(self.pushfile.phase, PushPhase::Opening | PushPhase::Writing)
                    || (self.pushfile.phase == PushPhase::Ready && unsafe { crate::fs_commands::FS.busy() });
                // Receives and sends take turns, so commands get through during the log dump.
                self.recv_turn = !self.recv_turn;
                if self.out_len > 0 && (!self.recv_turn || push_wait) {
                    // At most SEND_MAX a call, the size the Wi-Fi sweep proved on the x17.
                    let n = self.out_len.min(SEND_MAX);
                    region[..n].copy_from_slice(&self.out[..n]);
                    self.net_call(w::SEND, n as u64, Call::Send);
                } else if push_wait {
                    self.next_at = now + 2;
                } else if self.empty_receives >= STATE_EVERY {
                    self.empty_receives = 0;
                    self.net_call(w::STATE, 0, Call::State);
                } else {
                    let room = if self.sweep.active || self.hotswap.active || push_rx { w::REGION_BYTES } else { (IN_MAX - self.in_len).max(1).min(w::REGION_BYTES) };
                    self.net_call(w::RECEIVE, room as u64, Call::Receive);
                }
            }
            Phase::Idle | Phase::Gap | Phase::RetryWait => {}
        }
        true
    }

    /// True if `word` was the answer to our call.
    pub fn receive(&mut self, caller: u32, word: u64, sb: &mut Scrollback) -> bool {
        if !self.sent || self.phase == Phase::Idle || caller != 13 { return false; }
        let Some((seq, status, value)) = w::decode_reply(word) else { return false };
        if seq != self.seq { return false; }
        self.sent = false;
        let now = sex_pdx::get_ticks();
        if self.phase == Phase::RetryClose {
            if self.call != Call::Close { return true; }
            self.phase = Phase::RetryWait;
            self.next_at = now + CONNECT_RETRY_MS;
            return true;
        }
        if self.phase == Phase::Close {
            // The answer to a call made before the end: the flow still needs its CLOSE.
            if self.call != Call::Close { return true; }
            // Listening: the listener closes too.
            if self.listening { self.phase = Phase::CloseListener; self.next_at = 0; return true; }
            if self.sweep.active {
                self.phase = Phase::Gap;
                self.sweep.gap_step = if self.sweep.next_trial == 0 { 2 } else { 0 };
                self.next_at = 0;
                return true;
            }
            self.phase = Phase::Idle;
            sex_pdx::serial_record!("[spindle.tether.closed] bytes={} commands={}", self.bytes_out, self.commands);
            sb.push(b"tether: closed.");
            return true;
        }
        if self.phase == Phase::CloseListener {
            self.listening = false;
            if self.sweep.active {
                self.phase = Phase::Gap;
                self.sweep.gap_step = if self.sweep.next_trial == 0 { 2 } else { 0 };
                self.next_at = 0;
                return true;
            }
            self.phase = Phase::Idle;
            sex_pdx::serial_record!("[spindle.tether.closed] bytes={} commands={}", self.bytes_out, self.commands);
            sb.push(b"tether: closed.");
            return true;
        }
        if self.phase == Phase::Drop {
            // The finished connection is closed whatever the answer; wait for the next.
            self.phase = Phase::Accept;
            self.next_at = now + ACCEPT_MS;
            return true;
        }
        // Listening, the network may come up late (DHCP on a cable plugged in later).
        if self.listening && self.phase == Phase::Up && status == w::OFFLINE { self.next_at = now + 2_000; return true; }
        if status == w::BUSY && self.phase == Phase::ResolveAsk { self.next_at = now + POLL_MS * 5; return true; }
        if !matches!(status, w::OK | w::WAIT) {
            let reason = match status {
                w::OFFLINE => "no network (join Wi-Fi with `wifi join N PASSWORD`, or plug in Ethernet)",
                w::NO_NAME => "name not found (use the computer's IP address)",
                w::BUSY => "the network is busy",
                w::DENIED => "not allowed to use the network",
                _ => "network error",
            };
            let connecting = matches!(
                self.phase,
                Phase::Up | Phase::V4Wait | Phase::ResolveAsk | Phase::ResolveWait | Phase::Connect | Phase::Opening
            );
            if connecting && status != w::DENIED {
                self.fail_retryable(sb, reason);
            } else {
                self.fail(sb, reason);
            }
            return true;
        }
        let Some(region) = net() else { self.phase = Phase::Idle; return true };
        match self.phase {
            Phase::Up => {
                if status == w::WAIT { self.next_at = now + POLL_MS * 5; return true; }
                if self.listening { self.phase = Phase::Listen; return true; }
                match numeric(&self.host[..self.host_len]) {
                    Some(a) => {
                        self.address = a;
                        // DHCP gets its own time: the link may have come up a moment ago.
                        self.phase = Phase::V4Wait;
                        self.deadline = now + CONNECT_MS;
                    }
                    // IPv6 needs no DHCP: the router's own advertisement gave the address.
                    None if numeric6(&self.host[..self.host_len]).is_some() => self.phase = Phase::Connect,
                    None => self.phase = Phase::ResolveAsk,
                }
            }
            Phase::V4Wait => {
                let n = (value as usize).min(w::REGION_BYTES / (w::ADDRESS_BYTES + 1));
                let v4 = (0..n).any(|k| region[k * (w::ADDRESS_BYTES + 1)] == w::FAMILY_V4);
                if v4 { self.phase = Phase::Connect; }
                else {
                    if !self.told_v4 { self.told_v4 = true; sb.push(b"tether: waiting for an IPv4 address from the router (DHCP)."); }
                    self.next_at = now + 500;
                }
            }
            Phase::ResolveAsk => { self.phase = Phase::ResolveWait; self.next_at = now + POLL_MS; }
            Phase::ResolveWait => {
                if status == w::WAIT { self.next_at = now + POLL_MS; return true; }
                self.address.copy_from_slice(&region[..4]);
                self.phase = Phase::Connect;
            }
            Phase::Connect => { self.phase = Phase::Opening; self.next_at = now + POLL_MS; }
            Phase::Opening => match value {
                w::STATE_OPEN => {
                    self.connect_attempt = 0;
                    self.phase = Phase::Live;
                    let host = core::str::from_utf8(&self.host[..self.host_len]).unwrap_or("?");
                    if self.sweep.active {
                        let (s, (ba, replay), _) = self.plan();
                        say!(sb, "sweep {}/{}: {} ba={} replay={} connected", self.sweep.trial + 1, self.sweep.total, s.name, on(ba), on(replay));
                        self.sweep.live_at = now;
                        self.sweep.progress_at = now + s.idle_ms;
                    } else {
                        say!(sb, "tether: up to {}:{}. Commands from there show as remote> lines; tether off ends it.", host, self.port);
                        if !self.listening && self.host_len > 0 {
                            unsafe { crate::autoconf::AUTOCONF.queue_save_tether(&self.host[..self.host_len], self.port); }
                        }
                    }
                    sex_pdx::serial_record!("[spindle.tether.up] port={}", self.port);
                }
                w::STATE_OPENING => self.next_at = now + POLL_MS,
                _ => self.fail_retryable(sb, "the other computer refused the connection (is tether_host.py running?)"),
            },
            Phase::Listen => { self.phase = Phase::Addresses; }
            Phase::Addresses => {
                // Say this machine's IPv4 addresses, so a photo of the screen names it too.
                let mut shown = 0;
                for k in 0..(value as usize).min(4) {
                    let at = k * (w::ADDRESS_BYTES + 1);
                    if region[at] == w::FAMILY_V4 {
                        let a = &region[at + 1..at + 5];
                        say!(sb, "tether: listening at {}.{}.{}.{}:{}. On the Mac: python3 tether_host.py connect --scan", a[0], a[1], a[2], a[3], self.port);
                        shown += 1;
                    }
                }
                if shown == 0 { say!(sb, "tether: listening on port {} (no IPv4 address yet)", self.port); }
                sex_pdx::serial_record!("[spindle.tether.listening] port={} v4={}", self.port, shown);
                self.phase = Phase::Accept;
                self.next_at = now + ACCEPT_MS;
            }
            Phase::Accept => {
                if status == w::WAIT { self.next_at = now + ACCEPT_MS; return true; }
                self.reset(sb);
                self.phase = Phase::Live;
                let a = &region[1..5];
                say!(sb, "tether: {}.{}.{}.{} connected. Commands from there show as remote> lines; tether off ends it.", a[0], a[1], a[2], a[3]);
                sex_pdx::serial_record!("[spindle.tether.up] port={} accepted=1", self.port);
            }
            Phase::Live => match self.call {
                Call::Send => {
                    let n = (value as usize).min(self.out_len);
                    self.out.copy_within(n..self.out_len, 0);
                    self.out_len -= n;
                    self.bytes_out += n as u64;
                    if n == 0 { self.next_at = now + POLL_MS; }
                    else if self.sweep.active {
                        self.sweep.progress_at = self.sweep.progress_at.max(now);
                        let pace = self.plan().0.pace_ms;
                        if pace > 0 && self.out_len == 0 { self.next_at = now + pace; }
                    }
                }
                Call::Receive => {
                    let push_rx = matches!(self.pushfile.phase, PushPhase::Ready | PushPhase::Discard);
                    let n = if self.sweep.active || self.hotswap.active || push_rx { (value as usize).min(w::REGION_BYTES) } else { (value as usize).min(IN_MAX - self.in_len) };
                    if n == 0 {
                        self.empty_receives += 1;
                        if self.out_len == 0 { self.next_at = now + POLL_MS; }
                    } else if self.sweep.active {
                        // Sweep: what the Mac sends is test data, not commands.
                        self.empty_receives = 0;
                        self.sweep.bytes_in += n as u64;
                        self.sweep.progress_at = self.sweep.progress_at.max(now);
                    } else if self.hotswap.active {
                        self.empty_receives = 0;
                        self.feed_hotswap(&region[..n], sb);
                    } else if push_rx {
                        self.empty_receives = 0;
                        self.feed_pushfile(&region[..n], sb);
                    } else {
                        self.empty_receives = 0;
                        self.inbound[self.in_len..self.in_len + n].copy_from_slice(&region[..n]);
                        self.in_len += n;
                        self.split_lines(sb);
                    }
                }
                Call::State => {
                    if matches!(value, w::STATE_PEER_CLOSED | w::STATE_CLOSED) { self.fail(sb, "the other computer closed the connection"); }
                }
                Call::Plain | Call::Close => {}
            },
            _ => {}
        }
        true
    }

    fn put_hex_line(&mut self, bytes: &[u8]) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.put(b"@@PULL-CHUNK ");
        for &b in bytes {
            if self.out_len + 2 > OUT_MAX { break; }
            self.out[self.out_len] = HEX[(b >> 4) as usize];
            self.out[self.out_len + 1] = HEX[(b & 0x0f) as usize];
            self.out_len += 2;
        }
        self.put(b"\n");
    }

    fn emit_pull_err(&mut self, path: &[u8], reason: &str, status: u8, sb: &mut Scrollback) {
        let pstr = core::str::from_utf8(path).unwrap_or("?");
        self.put(b"@@PULL-ERR ");
        self.put(path);
        let mut t = Text { buf: [0; 160], len: 0 };
        let _ = core::fmt::Write::write_fmt(&mut t, format_args!(" {} (status={})\n", reason, status));
        let (b, n) = (t.buf, t.len);
        self.put(&b[..n]);
        say!(sb, "pull-file: err {} {} (status={})", pstr, reason, status);
    }

    fn step_pullfile(&mut self, sb: &mut Scrollback) {
        if self.pullfile.phase == PullPhase::Idle || self.log_at != usize::MAX { return; }
        match self.pullfile.phase {
            PullPhase::Idle => {}
            PullPhase::Stat => {
                if !self.pullfile.fs_submitted {
                    if OUT_MAX - self.out_len < 320 { return; }
                    if unsafe { crate::fs_commands::FS.busy() || crate::play_command::PLAY.busy() } { return; }
                    let path = self.pullfile.path;
                    let plen = self.pullfile.path_len;
                    if unsafe { crate::fs_commands::FS.start_pull_stat(&path[..plen]) } {
                        self.pullfile.fs_submitted = true;
                    } else {
                        self.emit_pull_err(&path[..plen], "filesystem unavailable", 0, sb);
                        self.pullfile_reset();
                    }
                    return;
                }
                let Some((st, size)) = (unsafe { crate::fs_commands::PULLFILE_STAT.take() }) else { return };
                self.pullfile.fs_submitted = false;
                let path = self.pullfile.path;
                let plen = self.pullfile.path_len;
                if st != sex_pdx::fs::OK {
                    self.emit_pull_err(&path[..plen], "stat failed", st, sb);
                    self.pullfile_reset();
                    return;
                }
                self.pullfile.total_len = size;
                self.pullfile.offset = 0;
                self.pullfile.fnv = log::FNV64_OFFSET;
                self.put(b"@@PULL-BEGIN ");
                self.put(&path[..plen]);
                let mut t = Text { buf: [0; 160], len: 0 };
                let _ = core::fmt::Write::write_fmt(&mut t, format_args!(" {}\n", size));
                let (b, n) = (t.buf, t.len);
                self.put(&b[..n]);
                if size == 0 {
                    let mut end = Text { buf: [0; 160], len: 0 };
                    let _ = core::fmt::Write::write_fmt(&mut end, format_args!("@@PULL-END {:016x}\n", self.pullfile.fnv));
                    let (eb, en) = (end.buf, end.len);
                    self.put(&eb[..en]);
                    let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
                    say!(sb, "pull-file: ok {} (0 bytes)", pstr);
                    self.pullfile_reset();
                    return;
                }
                self.pullfile.phase = PullPhase::Reading;
            }
            PullPhase::Reading => {
                if !self.pullfile.fs_submitted {
                    if OUT_MAX - self.out_len < PULL_OUT_HEADROOM { return; }
                    if unsafe { crate::fs_commands::FS.busy() || crate::play_command::PLAY.busy() } { return; }
                    let rem = self.pullfile.total_len.saturating_sub(self.pullfile.offset);
                    let want = rem.min(PULL_CHUNK_BYTES as u64);
                    let path = self.pullfile.path;
                    let plen = self.pullfile.path_len;
                    if unsafe { crate::fs_commands::FS.start_pull_read(&path[..plen], self.pullfile.offset, want) } {
                        self.pullfile.fs_submitted = true;
                    } else {
                        self.emit_pull_err(&path[..plen], "read start failed", 0, sb);
                        self.pullfile_reset();
                    }
                    return;
                }
                let Some((st, got)) = (unsafe { crate::fs_commands::PULLFILE_READ.take() }) else { return };
                self.pullfile.fs_submitted = false;
                let path = self.pullfile.path;
                let plen = self.pullfile.path_len;
                if st != sex_pdx::fs::OK {
                    self.emit_pull_err(&path[..plen], "read failed", st, sb);
                    self.pullfile_reset();
                    return;
                }
                let n = got.min(PULL_CHUNK_BYTES);
                if n > 0 {
                    let ok = crate::fs_commands::Client::with_pull_bytes(n, |chunk| {
                        self.pullfile.fnv = log::fnv64_update(self.pullfile.fnv, chunk);
                        self.put_hex_line(chunk);
                    });
                    if !ok {
                        self.emit_pull_err(&path[..plen], "region unavailable", 0, sb);
                        self.pullfile_reset();
                        return;
                    }
                    self.pullfile.offset += n as u64;
                }
                if self.pullfile.offset >= self.pullfile.total_len || n == 0 {
                    let mut end = Text { buf: [0; 160], len: 0 };
                    let _ = core::fmt::Write::write_fmt(&mut end, format_args!("@@PULL-END {:016x}\n", self.pullfile.fnv));
                    let (eb, en) = (end.buf, end.len);
                    self.put(&eb[..en]);
                    let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
                    say!(sb, "pull-file: ok {} ({} bytes)", pstr, self.pullfile.offset);
                    self.pullfile_reset();
                }
            }
        }
    }

    fn step_pushfile(&mut self, sb: &mut Scrollback) {
        match self.pushfile.phase {
            PushPhase::Opening => {
                if !self.pushfile.fs_submitted {
                    if unsafe { crate::fs_commands::FS.busy() || crate::play_command::PLAY.busy() } {
                        return;
                    }
                    let path = self.pushfile.path;
                    let plen = self.pushfile.path_len;
                    if unsafe { crate::fs_commands::FS.start_push_open(&path[..plen]) } {
                        self.pushfile.fs_submitted = true;
                    } else {
                        let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
                        say!(sb, "push-file: err {} filesystem unavailable", pstr);
                        if self.pushfile.expected_len == 0 {
                            self.pushfile_reset();
                            if self.in_len > 0 { self.split_lines(sb); }
                        } else {
                            self.pushfile.phase = PushPhase::Discard;
                            self.drain_inbound_pushfile(sb);
                        }
                    }
                    return;
                }
                let Some(st) = (unsafe { crate::fs_commands::PUSHFILE_STATUS.take() }) else { return };
                self.pushfile.fs_submitted = false;
                let path = self.pushfile.path;
                let plen = self.pushfile.path_len;
                let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
                if st != sex_pdx::fs::OK {
                    say!(sb, "push-file: err {} open failed (status={})", pstr, st);
                    if self.pushfile.expected_len == 0 {
                        self.pushfile_reset();
                        if self.in_len > 0 { self.split_lines(sb); }
                    } else {
                        self.pushfile.phase = PushPhase::Discard;
                        self.drain_inbound_pushfile(sb);
                    }
                    return;
                }
                if self.pushfile.expected_len == 0 {
                    if self.pushfile.fnv != self.pushfile.expected_fnv {
                        say!(sb, "push-file: err {} checksum mismatch (got {:016x} want {:016x})", pstr, self.pushfile.fnv, self.pushfile.expected_fnv);
                    } else {
                        say!(sb, "push-file: ok {} (0 bytes)", pstr);
                    }
                    self.pushfile_reset();
                    if self.in_len > 0 { self.split_lines(sb); }
                    return;
                }
                self.pushfile.phase = PushPhase::Ready;
                self.drain_inbound_pushfile(sb);
            }
            PushPhase::Writing => {
                let Some(st) = (unsafe { crate::fs_commands::PUSHFILE_STATUS.take() }) else { return };
                let path = self.pushfile.path;
                let plen = self.pushfile.path_len;
                let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
                if st != sex_pdx::fs::OK {
                    say!(sb, "push-file: err {} write failed (status={})", pstr, st);
                    if self.pushfile.received >= self.pushfile.expected_len {
                        self.pushfile_reset();
                        if self.in_len > 0 { self.split_lines(sb); }
                    } else {
                        self.pushfile.phase = PushPhase::Discard;
                    }
                    return;
                }
                if self.pushfile.received >= self.pushfile.expected_len {
                    if self.pushfile.fnv != self.pushfile.expected_fnv {
                        say!(sb, "push-file: err {} checksum mismatch (got {:016x} want {:016x})", pstr, self.pushfile.fnv, self.pushfile.expected_fnv);
                    } else {
                        say!(sb, "push-file: ok {} ({} bytes)", pstr, self.pushfile.expected_len);
                    }
                    self.pushfile_reset();
                    if self.in_len > 0 { self.split_lines(sb); }
                } else {
                    self.pushfile.phase = PushPhase::Ready;
                }
            }
            _ => {}
        }
    }

    fn drain_inbound_pushfile(&mut self, sb: &mut Scrollback) {
        let left = self.in_len;
        if left > 0 {
            let mut tmp = [0u8; IN_MAX];
            tmp[..left].copy_from_slice(&self.inbound[..left]);
            self.in_len = 0;
            self.feed_pushfile(&tmp[..left], sb);
        }
    }

    fn feed_pushfile(&mut self, bytes: &[u8], sb: &mut Scrollback) {
        let need = self.pushfile.expected_len.saturating_sub(self.pushfile.received);
        let take = bytes.len().min(need);
        let rest = &bytes[take..];

        if self.pushfile.phase == PushPhase::Discard {
            self.pushfile.received += take;
            if self.pushfile.received >= self.pushfile.expected_len {
                self.pushfile_reset();
                if !rest.is_empty() {
                    let n = rest.len().min(IN_MAX - self.in_len);
                    self.inbound[self.in_len..self.in_len + n].copy_from_slice(&rest[..n]);
                    self.in_len += n;
                    self.split_lines(sb);
                }
            }
            return;
        }

        // PushPhase::Ready — stream chunk directly to `sexfiles` without heap allocation.
        if !rest.is_empty() {
            let n = rest.len().min(IN_MAX - self.in_len);
            self.inbound[self.in_len..self.in_len + n].copy_from_slice(&rest[..n]);
            self.in_len += n;
        }
        let offset = self.pushfile.received as u64;
        self.pushfile.fnv = log::fnv64_update(self.pushfile.fnv, &bytes[..take]);
        self.pushfile.received += take;
        let path = self.pushfile.path;
        let plen = self.pushfile.path_len;
        if take == 0 {
            if self.pushfile.received >= self.pushfile.expected_len {
                let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
                if self.pushfile.fnv != self.pushfile.expected_fnv {
                    say!(sb, "push-file: err {} checksum mismatch (got {:016x} want {:016x})", pstr, self.pushfile.fnv, self.pushfile.expected_fnv);
                } else {
                    say!(sb, "push-file: ok {} ({} bytes)", pstr, self.pushfile.expected_len);
                }
                self.pushfile_reset();
                if self.in_len > 0 { self.split_lines(sb); }
            }
            return;
        }
        if unsafe { crate::fs_commands::FS.start_push_write(&path[..plen], offset, &bytes[..take]) } {
            self.pushfile.phase = PushPhase::Writing;
        } else {
            let pstr = core::str::from_utf8(&path[..plen]).unwrap_or("?");
            say!(sb, "push-file: err {} write start failed", pstr);
            if self.pushfile.received >= self.pushfile.expected_len {
                self.pushfile_reset();
                if self.in_len > 0 { self.split_lines(sb); }
            } else {
                self.pushfile.phase = PushPhase::Discard;
            }
        }
    }

    fn feed_hotswap(&mut self, bytes: &[u8], sb: &mut Scrollback) {
        let need = self.hotswap.expected_len.saturating_sub(self.hotswap.received);
        let take = bytes.len().min(need);
        if !self.hotswap.discard && take > 0 {
            let start = self.hotswap.received;
            let end = start + take;
            if end <= unsafe { HOTSWAP_MEM.1 } {
                unsafe {
                    let dst = HOTSWAP_MEM.0 as *mut u8;
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.add(start), take);
                }
            }
        }
        self.hotswap.received += take;
        if self.hotswap.received < self.hotswap.expected_len { return; }
        if !self.hotswap.discard {
            let name = self.hotswap.name;
            let nlen = self.hotswap.name_len;
            let name_str = core::str::from_utf8(&name[..nlen]).unwrap_or("?");
            let pd = self.hotswap.pd;
            let len = self.hotswap.expected_len;
            let slice = unsafe {
                core::slice::from_raw_parts(HOTSWAP_MEM.0 as *const u8, len)
            };
            let got = log::fnv64(slice);
            if got != self.hotswap.expected_fnv {
                say!(sb, "hotswap: err {} checksum mismatch (got {:016x} want {:016x})", name_str, got, self.hotswap.expected_fnv);
            } else {
                let res = log::hotswap_server(pd, slice);
                match res {
                    Ok(entry) => say!(sb, "hotswap: ok {} pd={} entry={:#x} bytes={}", name_str, pd, entry, len),
                    Err(code) => say!(sb, "hotswap: err {} pd={} code={}", name_str, pd, code),
                }
            }
        }
        self.hotswap_reset();
        let rest = &bytes[take..];
        if !rest.is_empty() {
            let n = rest.len().min(IN_MAX - self.in_len);
            self.inbound[self.in_len..self.in_len + n].copy_from_slice(&rest[..n]);
            self.in_len += n;
            self.split_lines(sb);
        }
    }

    fn split_lines(&mut self, sb: &mut Scrollback) {
        while let Some(end) = self.inbound[..self.in_len].iter().position(|b| *b == b'\n') {
            let mut line = [0u8; CMD_MAX];
            let raw = &self.inbound[..end];
            let raw = if let [rest @ .., b'\r'] = raw { rest } else { raw };
            let n = raw.len().min(CMD_MAX);
            line[..n].copy_from_slice(&raw[..n]);
            self.inbound.copy_within(end + 1..self.in_len, 0);
            self.in_len -= end + 1;
            let text = trim(&line[..n]);
            if text.is_empty() { continue; }
            if text.starts_with(b"SEXOS-PUSHFILE ") {
                let Some(hdr) = core::str::from_utf8(text).ok().and_then(log::parse_pushfile_header) else {
                    say!(sb, "push-file: err invalid header");
                    continue;
                };
                self.pushfile_reset();
                self.pushfile.phase = PushPhase::Opening;
                self.pushfile.expected_len = hdr.size;
                self.pushfile.expected_fnv = hdr.fnv;
                let pb = hdr.path.as_bytes();
                let pl = pb.len().min(self.pushfile.path.len());
                self.pushfile.path[..pl].copy_from_slice(&pb[..pl]);
                self.pushfile.path_len = pl;
                self.step_pushfile(sb);
                return;
            }
            if text.starts_with(b"SEXOS-HOTSWAP ") {
                let Some(hdr) = core::str::from_utf8(text).ok().and_then(log::parse_hotswap_header) else {
                    say!(sb, "hotswap: err invalid header");
                    continue;
                };
                self.hotswap_reset();
                self.hotswap.active = true;
                self.hotswap.expected_len = hdr.size;
                self.hotswap.expected_fnv = hdr.fnv;
                let nb = hdr.name.as_bytes();
                let nl = nb.len().min(self.hotswap.name.len());
                self.hotswap.name[..nl].copy_from_slice(&nb[..nl]);
                self.hotswap.name_len = nl;
                match hdr.pd {
                    Some(pd) => {
                        self.hotswap.pd = pd;
                        if hdr.size > HOTSWAP_BUF_CAP {
                            self.hotswap.discard = true;
                            say!(sb, "hotswap: err {} too large ({} > {} bytes)", hdr.name, hdr.size, HOTSWAP_BUF_CAP);
                        } else if hotswap_buf(hdr.size).is_none() {
                            self.hotswap.discard = true;
                            say!(sb, "hotswap: err {} out of memory ({} bytes)", hdr.name, hdr.size);
                        }
                    }
                    None => {
                        self.hotswap.discard = true;
                        say!(sb, "hotswap: err unknown server {}", hdr.name);
                    }
                }
                let left = self.in_len;
                if left > 0 {
                    let mut tmp = [0u8; IN_MAX];
                    tmp[..left].copy_from_slice(&self.inbound[..left]);
                    self.in_len = 0;
                    self.feed_hotswap(&tmp[..left], sb);
                }
                return;
            }
            if self.queue_count == QUEUE { sb.push(b"tether: too many commands waiting; one dropped."); continue; }
            let slot = (self.queue_head + self.queue_count) % QUEUE;
            self.queue[slot][..text.len()].copy_from_slice(text);
            self.queue_len[slot] = text.len();
            self.queue_count += 1;
        }
        // A line longer than the buffer: drop what came so far.
        if self.in_len == IN_MAX { self.in_len = 0; sb.push(b"tether: a command line was too long."); }
    }

    /// Queues a command for Spindle to run as if typed (sweep: `wifi`, its switches).
    fn queue_own(&mut self, cmd: &[u8]) {
        if self.queue_count == QUEUE { return; }
        let slot = (self.queue_head + self.queue_count) % QUEUE;
        let n = cmd.len().min(CMD_MAX);
        self.queue[slot][..n].copy_from_slice(&cmd[..n]);
        self.queue_len[slot] = n;
        self.queue_count += 1;
    }

    /// Adds a line to the sweep's results file.
    fn note(&mut self, line: &[u8]) {
        let buf = unsafe { &mut *core::ptr::addr_of_mut!(RESULTS) };
        // Only into the room reserved at the start: the results never grow the heap mid-sweep.
        if buf.len() + line.len() + 1 > RESULTS_MAX.min(buf.capacity()) { return; }
        buf.extend_from_slice(line);
        buf.push(b'\n');
    }

    /// Writes the results so far to the stick (replacing the file).
    fn save(&mut self, sb: &mut Scrollback) {
        let len = unsafe { (*core::ptr::addr_of!(RESULTS)).len() };
        let written = unsafe { crate::fs_commands::FS.start_write_with(SWEEP_FILE, RESULTS_MAX, |room| {
            let n = len.min(room.len());
            let all = &*core::ptr::addr_of!(RESULTS); room[..n].copy_from_slice(&all[..n]);
            n
        }, sb) };
        sex_pdx::serial_record!("[spindle.sweep.save] bytes={} started={}", len, written as u8);
    }

    /// Sweep data for the Mac: a header naming the trial, then filler lines.
    fn fill_sweep(&mut self) {
        let now = sex_pdx::get_ticks();
        let (s, (ba, replay), _) = self.plan();
        if !self.sweep.header_sent {
            self.sweep.header_sent = true;
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("SEXOS-SWEEP 1 trial={} shape={} ba={} replay={} up={} down={}\n", self.sweep.trial + 1, s.name, on(ba), on(replay), s.up, s.down));
            let (b, n) = (t.buf, t.len);
            self.put(&b[..n]);
            return;
        }
        if now < self.sweep.live_at + s.idle_ms || self.sweep.filler >= s.up { return; }
        let want = (s.chunk as u64).min(s.up - self.sweep.filler) as usize;
        let start = self.out_len;
        // 64-byte lines: "t TRIAL OFFSET ....\n", cut to `want`.
        while self.out_len - start < want && self.out_len < OUT_MAX {
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("t{:04} {:09} ", self.sweep.trial + 1, self.sweep.filler + (self.out_len - start) as u64));
            while t.len < 63 { t.buf[t.len] = b'.'; t.len += 1; }
            t.buf[63] = b'\n';
            let n = 64.min(want - (self.out_len - start)).min(OUT_MAX - self.out_len);
            let b = t.buf;
            self.put(&b[..n]);
        }
        self.sweep.filler += (self.out_len - start) as u64;
    }

    /// A live sweep trial: done, stalled or too long? True when it ended now.
    fn sweep_check(&mut self, sb: &mut Scrollback, now: u64) -> bool {
        let s = self.plan().0;
        if self.sweep.header_sent && self.sweep.filler >= s.up && self.out_len == 0 && self.sweep.bytes_in >= s.down && !self.sent {
            self.finish(sb, "pass");
            return true;
        }
        if now > self.sweep.progress_at + STALL_MS { self.finish(sb, "stall"); return true; }
        if now > self.sweep.started + TRIAL_MAX_MS { self.finish(sb, "timeout"); return true; }
        false
    }

    /// Ends the running trial with `result` and closes its flow (the gap follows).
    fn finish(&mut self, sb: &mut Scrollback, result: &str) {
        let now = sex_pdx::get_ticks();
        let k = self.sweep.trial;
        let (s, (ba, replay), slot) = self.plan();
        let pass = result == "pass";
        self.sweep.runs[slot] += 1;
        if pass { self.sweep.passes[slot] += 1; }
        self.sweep.last_pass = pass;
        let live_ms = if self.sweep.live_at > 0 { now.saturating_sub(self.sweep.live_at) } else { 0 };
        let mut t = Text { buf: [0; 160], len: 0 };
        let _ = core::fmt::Write::write_fmt(&mut t, format_args!("trial {} {} ba={} replay={} -> {} up={} down={} ms={} live_ms={}", k + 1, s.name, on(ba), on(replay), result, self.bytes_out, self.sweep.bytes_in, now.saturating_sub(self.sweep.started), live_ms));
        let (b, n) = (t.buf, t.len);
        self.note(&b[..n]);
        sb.push(&b[..n.min(110)]);
        sex_pdx::serial_record!("[spindle.sweep.trial] {}", core::str::from_utf8(&b[..n]).unwrap_or("?"));
        self.end();
    }

    /// Between trials, one step per call: counters, file, switches, next connect.
    fn gap(&mut self, sb: &mut Scrollback, now: u64) {
        match self.sweep.gap_step {
            0 => {
                self.sweep.mark = sb.total_lines;
                self.queue_own(b"wifi");
                self.sweep.gap_step = 1;
                self.next_at = now + 4_000;
            }
            1 => {
                // The link counters `wifi` just showed go under the trial's result.
                let kept = sb.retained() as u32;
                let oldest = sb.total_lines.saturating_sub(kept);
                let mut i = self.sweep.mark.max(oldest);
                while i < sb.total_lines {
                    let line = sb.get_retained((i - oldest) as usize);
                    let mut copy = [0u8; 112];
                    let n = line.len().min(110);
                    copy[0] = b' '; copy[1] = b' ';
                    copy[2..2 + n].copy_from_slice(&line[..n]);
                    // The link's counters only: the network list (numbered lines and its
                    // count) would fill the file.
                    let listed = line.trim_ascii_start().first().is_some_and(u8::is_ascii_digit);
                    if !listed && !line.starts_with(b"remote>") && !line.starts_with(b"sweep ") { self.note(&copy[..n + 2]); }
                    i += 1;
                }
                self.save(sb);
                self.sweep.gap_step = 2;
                self.next_at = now + 1_000;
            }
            2 => {
                if self.sweep.next_trial >= self.sweep.total {
                    self.summary(sb);
                    self.save(sb);
                    unsafe {
                        let r = &mut *core::ptr::addr_of_mut!(RESULTS);
                        r.clear();
                        r.shrink_to_fit();
                    }
                    self.sweep.active = false;
                    say!(sb, "tether sweep: done, {} trials. Results: /media/Ventoy/wifi-sweep.txt", self.sweep.total);
                    if self.resume.active {
                        self.listening = self.resume.listening;
                        self.host = self.resume.host;
                        self.host_len = self.resume.host_len;
                        self.port = self.resume.port;
                        self.resume = RESUME_NONE;
                        self.reset(sb);
                        self.phase = Phase::Up;
                        self.next_at = now + 1_500;
                        if self.listening {
                            say!(sb, "tether: resuming listener on port {} after sweep.", self.port);
                        } else {
                            say!(sb, "tether: reconnecting to {}:{} after sweep.", core::str::from_utf8(&self.host[..self.host_len]).unwrap_or("?"), self.port);
                        }
                    } else {
                        self.phase = Phase::Idle;
                    }
                    return;
                }
                self.sweep.trial = self.sweep.next_trial;
                self.sweep.next_trial += 1;
                let ba = self.plan().1 .0;
                self.queue_own(if ba { b"wifi ba on" } else { b"wifi ba off" });
                self.sweep.gap_step = 3;
                self.next_at = now + 1_500;
            }
            3 => {
                let replay = self.plan().1 .1;
                self.queue_own(if replay { b"wifi replay on" } else { b"wifi replay off" });
                self.sweep.gap_step = 4;
                // After a failure, give the link a while to come back before the next try.
                self.next_at = now + if self.sweep.last_pass { 1_500 } else { 15_000 };
            }
            _ => {
                self.reset(sb);
                self.sweep.started = now;
                self.sweep.live_at = 0;
                self.sweep.progress_at = now;
                self.sweep.header_sent = false;
                self.sweep.filler = 0;
                self.sweep.bytes_in = 0;
                self.phase = Phase::Up;
                self.next_at = 0;
            }
        }
    }

    /// Passes per shape and switch setting, at the end of the results.
    fn summary(&mut self, sb: &mut Scrollback) {
        self.note(b"");
        self.note(b"--- summary: passes/runs (ba,replay) ---");
        sb.push(b"--- sweep summary: passes/runs (ba,replay) ---");
        for (si, s) in SHAPES.iter().enumerate() {
            if self.sweep.only.is_some_and(|o| o != si) { continue; }
            let mut t = Text { buf: [0; 160], len: 0 };
            let _ = core::fmt::Write::write_fmt(&mut t, format_args!("{:8}", s.name));
            for (ki, (ba, replay)) in KNOBS.iter().enumerate() {
                let slot = si * KNOBS.len() + ki;
                let _ = core::fmt::Write::write_fmt(&mut t, format_args!("  ba={} replay={}: {}/{}", on(*ba), on(*replay), self.sweep.passes[slot], self.sweep.runs[slot]));
            }
            let (b, n) = (t.buf, t.len);
            self.note(&b[..n]);
            sb.push(&b[..n.min(110)]);
        }
    }

    /// The next command that arrived, to run as if typed.
    pub fn next_command(&mut self, out: &mut [u8; CMD_MAX]) -> Option<usize> {
        if self.queue_count == 0 { return None; }
        let slot = self.queue_head;
        let n = self.queue_len[slot];
        out[..n].copy_from_slice(&self.queue[slot][..n]);
        self.queue_head = (self.queue_head + 1) % QUEUE;
        self.queue_count -= 1;
        self.commands += 1;
        Some(n)
    }
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = s { s = rest; }
    while let [rest @ .., b' ' | b'\t'] = s { s = rest; }
    s
}

/// `hotswap <server_name>`: shows hot-swap target info and host push command.
pub fn hotswap_command(args: &[u8], sb: &mut Scrollback) {
    let name = trim(args);
    if name.is_empty() {
        sb.push(b"Usage: hotswap <server_name>");
        sb.push(b"Servers: sexdisplay sexdrive silk-shell sexinput sexusb sexbell sexfiles sexnet sexacpi denim cufflinks sexgpu-nv saddle");
        sb.push(b"From host over active tether: python3 scripts/tether_host.py push-server <name> <elf_path>");
        return;
    }
    let s = core::str::from_utf8(name).unwrap_or("");
    match log::server_pd_by_name(s) {
        Some(pd) => {
            say!(sb, "hotswap: {} -> pd={} (push via: python3 scripts/tether_host.py push-server {} <elf_path>)", s, pd, s);
        }
        None => {
            say!(sb, "hotswap: unknown server '{}'. Run `hotswap` for valid names.", s);
        }
    }
}
