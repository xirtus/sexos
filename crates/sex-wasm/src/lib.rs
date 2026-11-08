// SPDX-License-Identifier: GPL-3.0-only
//! Wasm guests: programs that run inside another program, in a sandbox the interpreter
//! enforces instead of a hardware key. This is the start of the Guests organ
//! (`docs/handoff/PROGRAM_SLOTS_DESIGN_2026_11_07.md`): many guests can share one key
//! because a guest can touch nothing but its own linear memory and the calls below.
//!
//! A guest gets exactly what its [`Grants`] say, and the grants can only narrow what the
//! hosting program already holds: a subfolder of the program's own folder, and the
//! network only if the person started the program with `+net`. Nothing here adds
//! authority; a [`Host`] maps each call onto the program's existing grants.
//!
//! Two import sets are offered, and a module may use either or both:
//!
//! - `sex`, the native set ([`abi`]): a window's text, keys, files, one-page fetches.
//! - `wasi_snapshot_preview1`, a small subset: console output, keyboard input lines,
//!   arguments, clocks, random bytes and exit. That is enough for a Rust or C program
//!   built for `wasm32-wasip1` that prints, reads lines and exits. Other WASI calls
//!   exist but answer `ENOSYS`; files through WASI come later, mapped onto the folder.
//!
//! The interpreter is `wasmi` (MIT/Apache-2.0), `no_std` with `alloc`.
#![no_std]
extern crate alloc;

use alloc::{format, string::String, vec, vec::Vec};
use wasmi::{
    Caller, Config, Engine, Error, ExternType, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode, Val,
};

/// The native import set, module name `sex`. Pointers and lengths are `i32` offsets
/// into the guest's exported `memory`. A negative result is a [`Refusal`] code.
///
/// | Import | Signature | Does |
/// |---|---|---|
/// | `show` | `(ptr, len) -> i32` | Replace the window's text |
/// | `title` | `(ptr, len) -> i32` | Name the window, up to sixteen bytes |
/// | `log` | `(ptr, len) -> i32` | One line to the serial record |
/// | `next` | `() -> i32` | Wait for a key or close; see [`abi::encode`] |
/// | `read` | `(path, path_len, out, cap) -> i32` | Bytes read from a file in the guest's folder |
/// | `write` | `(path, path_len, data, len) -> i32` | Replace a file in the guest's folder |
/// | `fetch` | `(url, url_len, out, cap) -> i64` | `status << 32 \| length`, needs the net grant |
/// | `now` | `() -> i64` | Nanoseconds since boot |
/// | `exit` | `(code)` | End the guest |
///
/// The guest exports `memory` and `_start`; returning from `_start` is exit code 0.
#[cfg(feature = "sexos")]
pub mod sexos;

pub mod abi {
    use super::Event;
    pub const MODULE: &str = "sex";
    pub const WASI: &str = "wasi_snapshot_preview1";
    /// `next` returns 1..=255 for a typed byte (10 is Enter, 8 is Backspace),
    /// `NAV + n` for a movement key, `KEY + code` for any other key, `CLOSE` to end.
    pub const NAV: i32 = 0x100;
    pub const KEY: i32 = 0x200;
    pub const CLOSE: i32 = -1;
    pub fn encode(event: Event) -> i32 {
        match event {
            Event::Char(0) => KEY,
            Event::Char(c) => c as i32,
            Event::Nav(n) => NAV + n as i32,
            Event::Key(code) => KEY + code as i32,
            Event::Close => CLOSE,
        }
    }
}

/// Why a call was refused. Guests see `code()`, always negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The guest was not granted this.
    NotAllowed = 1,
    NotFound = 2,
    /// A pointer outside the guest's memory, or a path that leaves its folder.
    Invalid = 3,
    TooLarge = 4,
    Failed = 5,
}
impl Refusal {
    pub const fn code(self) -> i32 { -(self as i32) }
    pub const fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            -1 => Self::NotAllowed, -2 => Self::NotFound, -3 => Self::Invalid, -4 => Self::TooLarge, -5 => Self::Failed,
            _ => return None,
        })
    }
}

/// What a guest can wait for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A typed byte. Enter is `b'\n'`, Backspace is `8`.
    Char(u8),
    /// Left, Right, Up, Down, Home, End, Delete as 0..=6, in `sex_app::Nav` order.
    Nav(u8),
    /// Any other key, by the keyboard code the program received.
    Key(u16),
    /// The person closed the window.
    Close,
}

/// The hosting program's side. Each call is made only after the guest's grants allow
/// it and its paths are checked, so an implementation maps calls straight onto
/// `sex_app` (or a test double). Defaults refuse.
pub trait Host {
    /// Replace the window's text with `text`.
    fn show(&mut self, text: &[u8]);
    fn title(&mut self, _name: &[u8]) {}
    fn log(&mut self, _line: &[u8]) {}
    /// Wait for the next key or close.
    fn next(&mut self) -> Event { Event::Close }
    /// `path` is already inside the guest's folder, relative to the program's own.
    fn read(&mut self, _path: &[u8], _out: &mut [u8]) -> Result<usize, Refusal> { Err(Refusal::NotAllowed) }
    fn write(&mut self, _path: &[u8], _data: &[u8]) -> Result<(), Refusal> { Err(Refusal::NotAllowed) }
    /// HTTP status and body length. Called only when the guest holds the net grant.
    fn fetch(&mut self, _url: &[u8], _out: &mut [u8]) -> Result<(u32, usize), Refusal> { Err(Refusal::NotAllowed) }
    /// Nanoseconds since boot.
    fn now(&mut self) -> u64 { 0 }
    /// Random bytes. The default is a non-cryptographic generator seeded from the clock;
    /// a host with a hardware source should override it.
    fn random(&mut self, out: &mut [u8]) {
        let mut s = self.now() ^ 0x9E37_79B9_7F4A_7C15 ^ (out.as_ptr() as u64);
        for chunk in out.chunks_mut(8) {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
        }
    }
}

/// What the guest may reach. Narrower than or equal to the hosting program's grants.
#[derive(Clone, Debug, Default)]
pub struct Grants {
    /// The guest's folder inside the program's own, e.g. `b"guests/calc"`. `None`: no files.
    pub folder: Option<Vec<u8>>,
    /// Only meaningful when the program itself was started with `+net`.
    pub net: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest linear memory, in bytes.
    pub memory_bytes: usize,
    /// Instructions' worth of fuel before the guest is stopped. `None`: no limit, which
    /// suits interactive guests; the person can still close the window.
    pub fuel: Option<u64>,
    /// Largest single read, fetch or write.
    pub io_bytes: usize,
    /// Console text kept for `show`, the tail of what the guest printed.
    pub console_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self { Self { memory_bytes: 16 << 20, fuel: None, io_bytes: 1 << 20, console_bytes: 6 * 1024 } }
}

/// One guest to run.
pub struct Guest<'a> {
    /// The program's name, `argv[0]` for WASI.
    pub name: &'a [u8],
    pub args: &'a [&'a [u8]],
    pub grants: Grants,
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `_start` returned (0), or the guest called `exit`/`proc_exit`.
    Exited(i32),
    /// The window was closed while the guest waited for input.
    Closed,
    OutOfFuel,
    /// A trap: out-of-bounds access, unreachable, division by zero, stack overflow...
    Trapped(String),
    /// The module was not run: malformed, invalid, or needs imports SexOS lacks.
    Rejected(String),
}

/// Ends a guest from inside a host call.
#[derive(Debug)]
struct Closed;
impl core::fmt::Display for Closed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("window closed") }
}
impl wasmi::errors::HostError for Closed {}

struct State<H> {
    host: H,
    grants: Grants,
    limits: Limits,
    store: StoreLimits,
    args: Vec<Vec<u8>>,
    /// What the guest wrote to stdout and stderr, shown as the window's text.
    console: Vec<u8>,
    /// A line typed for `fd_read` that the guest has not taken yet.
    pending: Vec<u8>,
    closed: bool,
    eof_seen: bool,
}

impl<H: Host> State<H> {
    /// The path inside the program's folder, or `Invalid` if it would leave the guest's.
    fn place(&self, path: &[u8]) -> Result<Vec<u8>, Refusal> {
        let folder = self.grants.folder.as_ref().ok_or(Refusal::NotAllowed)?;
        if path.is_empty() || path.len() > 255 || path[0] == b'/' || path.contains(&0) || path.contains(&b'\\') {
            return Err(Refusal::Invalid);
        }
        if path.split(|b| *b == b'/').any(|part| part.is_empty() || part == b"." || part == b"..") {
            return Err(Refusal::Invalid);
        }
        let mut full = folder.clone();
        if !full.is_empty() && !full.ends_with(b"/") { full.push(b'/'); }
        full.extend_from_slice(path);
        Ok(full)
    }
    fn print(&mut self, bytes: &[u8]) {
        self.console.extend_from_slice(bytes);
        let keep = self.limits.console_bytes;
        if self.console.len() > keep * 2 { let cut = self.console.len() - keep; self.console.drain(..cut); }
        self.show_console();
    }
    fn show_console(&mut self) {
        let keep = self.limits.console_bytes;
        let mut start = self.console.len().saturating_sub(keep);
        // Start on a whole line when the tail was cut.
        if start > 0 {
            if let Some(nl) = self.console[start..].iter().position(|b| *b == b'\n') { start += nl + 1; }
        }
        let (host, console) = (&mut self.host, &self.console);
        host.show(&console[start..]);
    }
    fn next(&mut self) -> Event {
        if self.closed { return Event::Close; }
        let e = self.host.next();
        if e == Event::Close { self.closed = true; }
        e
    }
}

fn memory<T>(caller: &Caller<'_, T>) -> Option<Memory> { caller.get_export("memory")?.into_memory() }
fn range(len: usize, ptr: i32, n: i32) -> Result<core::ops::Range<usize>, Refusal> {
    let (p, n) = (ptr as u32 as usize, n as u32 as usize);
    let end = p.checked_add(n).ok_or(Refusal::Invalid)?;
    if end > len { return Err(Refusal::Invalid); }
    Ok(p..end)
}
/// Runs `f` with the guest's memory and the state, turning a refusal into its code.
fn with<H: Host + 'static>(
    caller: &mut Caller<'_, State<H>>,
    f: impl FnOnce(&mut [u8], &mut State<H>) -> Result<i32, Refusal>,
) -> i32 {
    let Some(mem) = memory(caller) else { return Refusal::Invalid.code() };
    let (data, state) = mem.data_and_store_mut(caller);
    f(data, state).unwrap_or_else(Refusal::code)
}

fn link_native<H: Host + 'static>(linker: &mut Linker<State<H>>) -> Result<(), wasmi::errors::LinkerError> {
    use abi::MODULE as M;
    linker.func_wrap(M, "show", |mut c: Caller<'_, State<H>>, p: i32, n: i32| -> i32 {
        with(&mut c, |d, s| { let r = range(d.len(), p, n)?; s.host.show(&d[r]); Ok(0) })
    })?;
    linker.func_wrap(M, "title", |mut c: Caller<'_, State<H>>, p: i32, n: i32| -> i32 {
        with(&mut c, |d, s| { let r = range(d.len(), p, n)?; let r = r.start..r.end.min(r.start + 16); s.host.title(&d[r]); Ok(0) })
    })?;
    linker.func_wrap(M, "log", |mut c: Caller<'_, State<H>>, p: i32, n: i32| -> i32 {
        with(&mut c, |d, s| { let r = range(d.len(), p, n)?; let r = r.start..r.end.min(r.start + 512); s.host.log(&d[r]); Ok(0) })
    })?;
    // After the close event, waiting again ends the guest: it did not stop by itself.
    linker.func_wrap(M, "next", |mut c: Caller<'_, State<H>>| -> Result<i32, Error> {
        if c.data().closed { return Err(Error::host(Closed)); }
        Ok(abi::encode(c.data_mut().next()))
    })?;
    linker.func_wrap(M, "read", |mut c: Caller<'_, State<H>>, pp: i32, pn: i32, op: i32, on: i32| -> i32 {
        with(&mut c, |d, s| {
            let path = s.place(&d[range(d.len(), pp, pn)?])?;
            let out = range(d.len(), op, on)?;
            let out = out.start..out.end.min(out.start + s.limits.io_bytes);
            let got = s.host.read(&path, &mut d[out.clone()])?;
            Ok(got.min(out.len()) as i32)
        })
    })?;
    linker.func_wrap(M, "write", |mut c: Caller<'_, State<H>>, pp: i32, pn: i32, dp: i32, dn: i32| -> i32 {
        with(&mut c, |d, s| {
            let path = s.place(&d[range(d.len(), pp, pn)?])?;
            let data = range(d.len(), dp, dn)?;
            if data.len() > s.limits.io_bytes { return Err(Refusal::TooLarge); }
            s.host.write(&path, &d[data])?;
            Ok(0)
        })
    })?;
    linker.func_wrap(M, "fetch", |mut c: Caller<'_, State<H>>, up: i32, un: i32, op: i32, on: i32| -> i64 {
        let Some(mem) = memory(&c) else { return Refusal::Invalid.code() as i64 };
        let (d, s) = mem.data_and_store_mut(&mut c);
        let mut go = || -> Result<i64, Refusal> {
            if !s.grants.net { return Err(Refusal::NotAllowed); }
            let url: Vec<u8> = d[range(d.len(), up, un)?].to_vec();
            let out = range(d.len(), op, on)?;
            let out = out.start..out.end.min(out.start + s.limits.io_bytes);
            let (status, len) = s.host.fetch(&url, &mut d[out.clone()])?;
            Ok((status as i64) << 32 | len.min(out.len()) as i64)
        };
        go().unwrap_or_else(|r| r.code() as i64)
    })?;
    linker.func_wrap(M, "now", |mut c: Caller<'_, State<H>>| -> i64 { c.data_mut().host.now() as i64 })?;
    linker.func_wrap(M, "exit", |_: Caller<'_, State<H>>, code: i32| -> Result<(), Error> { Err(Error::i32_exit(code)) })?;
    Ok(())
}

/// WASI errno values used here.
mod errno {
    pub const SUCCESS: i32 = 0;
    pub const BADF: i32 = 8;
    pub const FAULT: i32 = 21;
    pub const INVAL: i32 = 28;
    pub const NOSYS: i32 = 52;
    pub const SPIPE: i32 = 70;
}

fn put_u32(d: &mut [u8], at: i32, v: u32) -> Result<(), Refusal> { let r = range(d.len(), at, 4)?; d[r].copy_from_slice(&v.to_le_bytes()); Ok(()) }
fn put_u64(d: &mut [u8], at: i32, v: u64) -> Result<(), Refusal> { let r = range(d.len(), at, 8)?; d[r].copy_from_slice(&v.to_le_bytes()); Ok(()) }
fn get_u32(d: &[u8], at: i32) -> Result<u32, Refusal> { let r = range(d.len(), at, 4)?; Ok(u32::from_le_bytes(d[r].try_into().unwrap())) }
/// WASI calls answer an errno; a bad pointer is `EFAULT`, not a trap.
fn wasi<H: Host + 'static>(c: &mut Caller<'_, State<H>>, f: impl FnOnce(&mut [u8], &mut State<H>) -> Result<i32, Refusal>) -> i32 {
    let Some(mem) = memory(c) else { return errno::FAULT };
    let (d, s) = mem.data_and_store_mut(c);
    f(d, s).unwrap_or(errno::FAULT)
}
/// `(iovec base, length)` pairs from the guest.
fn iovecs(d: &[u8], iovs: i32, count: i32) -> Result<Vec<core::ops::Range<usize>>, Refusal> {
    if !(0..=1024).contains(&count) { return Err(Refusal::Invalid); }
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let at = iovs.checked_add(i * 8).ok_or(Refusal::Invalid)?;
        let (base, len) = (get_u32(d, at)?, get_u32(d, at + 4)?);
        out.push(range(d.len(), base as i32, len as i32)?);
    }
    Ok(out)
}
/// `argv` (or an environment) laid out as WASI wants: pointers, then NUL-ended strings.
fn strings(list: &[Vec<u8>]) -> (u32, u32) { (list.len() as u32, list.iter().map(|a| a.len() as u32 + 1).sum()) }
fn put_strings(d: &mut [u8], list: &[Vec<u8>], ptrs: i32, buf: i32) -> Result<i32, Refusal> {
    let mut at = buf as u32;
    for (i, s) in list.iter().enumerate() {
        put_u32(d, ptrs + 4 * i as i32, at)?;
        let r = range(d.len(), at as i32, s.len() as i32 + 1)?;
        d[r.start..r.end - 1].copy_from_slice(s);
        d[r.end - 1] = 0;
        at += s.len() as u32 + 1;
    }
    Ok(errno::SUCCESS)
}

fn link_wasi<H: Host + 'static>(linker: &mut Linker<State<H>>) -> Result<(), wasmi::errors::LinkerError> {
    use abi::WASI as W;
    type C<'a, H> = Caller<'a, State<H>>;
    linker.func_wrap(W, "fd_write", |mut c: C<'_, H>, fd: i32, iovs: i32, count: i32, written: i32| -> i32 {
        wasi(&mut c, |d, s| {
            if fd != 1 && fd != 2 { return Ok(errno::BADF); }
            let mut bytes = Vec::new();
            for r in iovecs(d, iovs, count)? { bytes.extend_from_slice(&d[r]); }
            if fd == 2 { for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) { s.host.log(&line[..line.len().min(512)]); } }
            s.print(&bytes);
            put_u32(d, written, bytes.len() as u32)?;
            Ok(errno::SUCCESS)
        })
    })?;
    // Standard input is the keyboard, a line at a time, echoed to the console.
    // A close reads as end of input once; reading again ends the guest.
    linker.func_wrap(W, "fd_read", |mut c: C<'_, H>, fd: i32, iovs: i32, count: i32, read: i32| -> Result<i32, Error> {
        let Some(mem) = memory(&c) else { return Ok(errno::FAULT) };
        if fd != 0 { return Ok(errno::BADF); }
        if c.data().closed && c.data().pending.is_empty() {
            if c.data().eof_seen { return Err(Error::host(Closed)); }
            c.data_mut().eof_seen = true;
        }
        let (d, s) = mem.data_and_store_mut(&mut c);
        let mut go = || -> Result<i32, Refusal> {
            let vecs = iovecs(d, iovs, count)?;
            if s.pending.is_empty() && !s.closed {
                let mut line = Vec::new();
                loop {
                    match s.next() {
                        Event::Char(b'\n') => { line.push(b'\n'); s.print(b"\n"); break; }
                        Event::Char(8) => { if line.pop().is_some() { s.console.pop(); s.show_console(); } }
                        Event::Char(ch) => { line.push(ch); s.print(&[ch]); }
                        Event::Close => break,
                        _ => {}
                    }
                }
                s.pending = line;
            }
            let mut n = 0;
            for r in vecs {
                let take = r.len().min(s.pending.len() - n);
                d[r.start..r.start + take].copy_from_slice(&s.pending[n..n + take]);
                n += take;
                if n == s.pending.len() { break; }
            }
            s.pending.drain(..n);
            put_u32(d, read, n as u32)?;
            Ok(errno::SUCCESS)
        };
        Ok(go().unwrap_or(errno::FAULT))
    })?;
    linker.func_wrap(W, "proc_exit", |_: C<'_, H>, code: i32| -> Result<(), Error> { Err(Error::i32_exit(code)) })?;
    linker.func_wrap(W, "args_sizes_get", |mut c: C<'_, H>, count: i32, size: i32| -> i32 {
        wasi(&mut c, |d, s| { let (n, b) = strings(&s.args); put_u32(d, count, n)?; put_u32(d, size, b)?; Ok(errno::SUCCESS) })
    })?;
    linker.func_wrap(W, "args_get", |mut c: C<'_, H>, ptrs: i32, buf: i32| -> i32 {
        wasi(&mut c, |d, s| put_strings(d, &s.args, ptrs, buf))
    })?;
    // No environment: settings are not ambient on SexOS.
    linker.func_wrap(W, "environ_sizes_get", |mut c: C<'_, H>, count: i32, size: i32| -> i32 {
        wasi(&mut c, |d, _| { put_u32(d, count, 0)?; put_u32(d, size, 0)?; Ok(errno::SUCCESS) })
    })?;
    linker.func_wrap(W, "environ_get", |_: C<'_, H>, _: i32, _: i32| -> i32 { errno::SUCCESS })?;
    linker.func_wrap(W, "clock_time_get", |mut c: C<'_, H>, id: i32, _precision: i64, time: i32| -> i32 {
        wasi(&mut c, |d, s| {
            // Realtime, monotonic, process and thread clocks all read time since boot for now.
            if !(0..=3).contains(&id) { return Ok(errno::INVAL); }
            let now = s.host.now();
            put_u64(d, time, now)?;
            Ok(errno::SUCCESS)
        })
    })?;
    linker.func_wrap(W, "clock_res_get", |mut c: C<'_, H>, id: i32, res: i32| -> i32 {
        wasi(&mut c, |d, _| { if !(0..=3).contains(&id) { return Ok(errno::INVAL); } put_u64(d, res, 1_000_000)?; Ok(errno::SUCCESS) })
    })?;
    linker.func_wrap(W, "random_get", |mut c: C<'_, H>, buf: i32, len: i32| -> i32 {
        wasi(&mut c, |d, s| { let r = range(d.len(), buf, len)?; s.host.random(&mut d[r]); Ok(errno::SUCCESS) })
    })?;
    linker.func_wrap(W, "sched_yield", |_: C<'_, H>| -> i32 { errno::SUCCESS })?;
    linker.func_wrap(W, "fd_fdstat_get", |mut c: C<'_, H>, fd: i32, stat: i32| -> i32 {
        wasi(&mut c, |d, _| {
            if !(0..=2).contains(&fd) { return Ok(errno::BADF); }
            // filetype character_device, no flags, rights: read for 0, write for 1 and 2.
            let r = range(d.len(), stat, 24)?;
            d[r.clone()].fill(0);
            d[r.start] = 2;
            let rights: u64 = if fd == 0 { 1 << 1 } else { 1 << 6 };
            d[r.start + 8..r.start + 16].copy_from_slice(&rights.to_le_bytes());
            Ok(errno::SUCCESS)
        })
    })?;
    linker.func_wrap(W, "fd_close", |_: C<'_, H>, fd: i32| -> i32 { if (0..=2).contains(&fd) { errno::SUCCESS } else { errno::BADF } })?;
    linker.func_wrap(W, "fd_seek", |_: C<'_, H>, fd: i32, _: i64, _: i32, _: i32| -> i32 { if (0..=2).contains(&fd) { errno::SPIPE } else { errno::BADF } })?;
    // No preopened folders yet, so a WASI libc sees no filesystem.
    linker.func_wrap(W, "fd_prestat_get", |_: C<'_, H>, _: i32, _: i32| -> i32 { errno::BADF })?;
    linker.func_wrap(W, "fd_prestat_dir_name", |_: C<'_, H>, _: i32, _: i32, _: i32| -> i32 { errno::BADF })?;
    Ok(())
}

/// Validates, links and runs `wasm` to completion. The host comes back for reuse.
pub fn run<H: Host + 'static>(wasm: &[u8], guest: &Guest<'_>, host: H) -> (Outcome, H) {
    let mut config = Config::default();
    config.consume_fuel(guest.limits.fuel.is_some());
    let engine = Engine::new(&config);
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(guest.limits.memory_bytes)
        .memories(1)
        .tables(4)
        .table_elements(100_000)
        .instances(1)
        .build();
    let mut args = vec![guest.name.to_vec()];
    args.extend(guest.args.iter().map(|a| a.to_vec()));
    let state = State {
        host, grants: guest.grants.clone(), limits: guest.limits, store: store_limits, args,
        console: Vec::new(), pending: Vec::new(), closed: false, eof_seen: false,
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.store);
    if let Some(fuel) = guest.limits.fuel {
        if store.set_fuel(fuel).is_err() { return (Outcome::Rejected("fuel".into()), store.into_data().host); }
    }
    let outcome = start(&engine, &mut store, wasm);
    (outcome, store.into_data().host)
}

fn start<H: Host + 'static>(engine: &Engine, store: &mut Store<State<H>>, wasm: &[u8]) -> Outcome {
    let module = match Module::new(engine, wasm) {
        Ok(m) => m,
        Err(e) => return Outcome::Rejected(format!("not a valid Wasm module: {e}")),
    };
    let mut linker = Linker::<State<H>>::new(engine);
    if link_native(&mut linker).and_then(|_| link_wasi(&mut linker)).is_err() {
        return Outcome::Rejected("host link failed".into());
    }
    // Every import must be one of ours. Unknown WASI calls answer ENOSYS so a libc's
    // unused paths still link; anything else is refused by name.
    for import in module.imports() {
        let (m, n) = (import.module(), import.name());
        if defined(m, n) { continue; }
        match (m, import.ty()) {
            (abi::WASI, ExternType::Func(ty)) if ty.results().len() == 1 => {
                let ty = ty.clone();
                let _ = linker.func_new(m, n, ty, |_, _, results: &mut [Val]| { results[0] = Val::I32(errno::NOSYS); Ok(()) });
            }
            _ => return Outcome::Rejected(format!("needs {m}.{n}, which SexOS does not provide")),
        }
    }
    let instance = match linker.instantiate_and_start(&mut *store, &module) {
        Ok(i) => i,
        Err(e) => return finish(e),
    };
    let Ok(main) = instance.get_typed_func::<(), ()>(&mut *store, "_start") else {
        return Outcome::Rejected("exports no `_start` function".into());
    };
    match main.call(&mut *store, ()) {
        Ok(()) => Outcome::Exited(0),
        Err(e) => finish(e),
    }
}

/// Names `link_native` and `link_wasi` define, since a `Linker` lookup does not report
/// host functions.
fn defined(module: &str, name: &str) -> bool {
    const NATIVE: &[&str] = &["show", "title", "log", "next", "read", "write", "fetch", "now", "exit"];
    const WASI: &[&str] = &[
        "fd_write", "fd_read", "proc_exit", "args_sizes_get", "args_get", "environ_sizes_get", "environ_get",
        "clock_time_get", "clock_res_get", "random_get", "sched_yield", "fd_fdstat_get", "fd_close", "fd_seek",
        "fd_prestat_get", "fd_prestat_dir_name",
    ];
    match module { abi::MODULE => NATIVE.contains(&name), abi::WASI => WASI.contains(&name), _ => false }
}

fn finish(e: Error) -> Outcome {
    if let Some(code) = e.i32_exit_status() { return Outcome::Exited(code); }
    if e.downcast_ref::<Closed>().is_some() { return Outcome::Closed; }
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => Outcome::OutOfFuel,
        Some(code) => Outcome::Trapped(format!("{code}")),
        None => Outcome::Trapped(format!("{e}")),
    }
}
