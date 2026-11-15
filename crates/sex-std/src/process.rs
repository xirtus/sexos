// SPDX-License-Identifier: GPL-3.0-only
//! `std::process::Command` on SexOS: running another program.
//!
//! A program asks the desktop to start another program the way any program can
//! (`sex_app::launch`): Files prepares the run, refusing a grant this run does not hold,
//! and Shell starts it and later says how it ended. The started run gets only what this
//! run hands on: no network unless asked for and this run has it, an `@/path` only if
//! this run holds it. Nothing here is a system call of its own.
//!
//! std's Hermit port has no process support of its own, so the build gives std one
//! more file (patches/rust-std/process_sexos.rs, added by scripts/sex_std_source.py)
//! that calls the `sys_process_*` functions here. Then, unchanged:
//!
//! ```ignore
//! let status = std::process::Command::new("rg").arg("needle").status()?;
//! let mut child = std::process::Command::new("stdspawn").args(["sleep", "60"]).spawn()?;
//! child.kill()?; child.wait()?;   // code() None, success() false: it was ended
//! ```
//!
//! * **The program**: a bare `NAME` or `/bin/NAME` (the program store), or `@/path` of a
//!   program file this run holds. Anything else is `NotFound`.
//! * **Arguments**: passed as one line, quoted where a word needs it ([`crate::args`]);
//!   at most 256 bytes (`sex_pdx::fs::ARGS_MAX`), else `ArgumentListTooLong`.
//! * **Window**: by default the run is a hidden helper (`sex_pdx::program_store::RUN_HIDDEN`):
//!   no window on any workspace, and a std program started so knows it (its first word
//!   is [`HIDDEN_WORD`], taken out before `main`), so its output goes to the serial
//!   record only and it never waits for a window to be closed. `Command::new("rg")
//!   .arg("x").status()` therefore runs rg to its end and returns its status, as a
//!   script would expect. [`CommandExt::windowed`] gives the run its own window
//!   instead; a std program's console then stays open with its output until the
//!   person closes it, and only then does `wait` return.
//! * **Network**: [`CommandExt::network`] asks for it; Shell refuses the start
//!   (`PermissionDenied`) unless this run was started with the network itself.
//! * **Status**: what the run passed to `exit` (`code()`); a run that ended without
//!   one (killed, or it crashed) has no `code()` and is not `success()`, like a Unix
//!   process ended by a signal. `Child::id()` and `std::process::id()` are tasks: each
//!   run has its own.
//! * **Pipes**: `Stdio::piped()` for stdin, stdout or stderr, and so `output()`, work.
//!   Files holds the bytes in one of this run's relay slots (`sex_pdx::console_relay`,
//!   pipe runs): 4 KiB toward the child, 32 KiB of stdout and 16 KiB of stderr from it.
//!   A full ring holds the writer back; neither side maps the other's memory. Dropping
//!   `ChildStdin` ends the child's input; dropping `ChildStdout` makes its later writes
//!   vanish, as a closed pipe would. At most [`PIPE_SLOTS`] such children run at once.
//!   `Stdio::null()` reads nothing and drops what is written. `inherit` means the
//!   child's own console (its window, or the serial record when hidden), not this
//!   program's. A `ChildStdout` handed to another `Command` is `Unsupported`.
//! * **Environment**: the child gets this run's environment with the command's changes
//!   (`env`, `env_remove`, `env_clear`), up to 4 KiB in all (else `ArgumentListTooLong`).
//! * **Working folder**: `current_dir(dir)` names a folder as this program names it (in
//!   its own folder, or an `@/path` it holds). Files hands the child that folder as a
//!   grant (`@/absolute/path`), and the child's relative paths start there. A relative
//!   `dir` starts from this run's working folder. A folder this run does not hold, one
//!   that does not exist, a file, or a path with a space is `NotFound`.
//!
//! sex-libc's `posix_spawn`, `waitpid`, `kill` and `system` use the same functions.
use std::sync::Mutex;
use std::os::hermit::hermit_abi::errno;
use sex_app::launch::{self, Event};
use sex_pdx::{console_relay as c, fs as w, program_store as store};

/// The first word of a hidden run's argument line, put there by this crate: the
/// program's console then keeps to the serial record. sex-std takes it out before
/// `main`; a program not built on sex-std sees it as its first argument.
pub const HIDDEN_WORD: &[u8] = b"+hidden";
/// The word (after [`HIDDEN_WORD`], if that is there) that tells a std program Files
/// holds its streams, environment and working folder: sex-std asks Files, which alone
/// decides, and takes the word out before `main`.
pub const PIPED_WORD: &[u8] = b"+piped";
/// How many pipe runs a program may have at once: its relay slots.
pub const PIPE_SLOTS: usize = sex_pdx::console_relay::SLOTS;

/// SexOS's choices for a `std::process::Command`.
pub trait CommandExt {
    /// Gives the run a window on the workspace (default: a hidden helper).
    fn windowed(&mut self, on: bool) -> &mut Self;
    /// Asks for the network for the run; refused unless this run has it.
    fn network(&mut self, on: bool) -> &mut Self;
}

impl CommandExt for std::process::Command {
    fn windowed(&mut self, on: bool) -> &mut Self {
        if on { self.env("SEXOS_RUN_WINDOWED", "1") } else { self.env_remove("SEXOS_RUN_WINDOWED") }
    }
    fn network(&mut self, on: bool) -> &mut Self {
        if on { self.env("SEXOS_RUN_NETWORK", "1") } else { self.env_remove("SEXOS_RUN_NETWORK") }
    }
}

/// The error a program gets for what SexOS cannot do between runs yet (a child's pipe
/// handed to another child).
pub fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "SexOS cannot hand one child's pipe to another")
}

/// A run this program started.
#[derive(Clone, Copy)]
struct Run {
    cookie: u32,
    task: u32,
    /// How it ended, once it has: the status it reported, if any.
    ended: Option<Option<u32>>,
    /// False until Shell said the run started (or ended); `spawn` is still waiting.
    started: bool,
    /// Shell's refusal code, when it refused the start.
    refused: Option<u32>,
    /// The relay slot holding its streams (a pipe run).
    slot: Option<u8>,
}

static RUNS: Mutex<Vec<Run>> = Mutex::new(Vec::new());
/// Cookies this crate gives its runs: high, away from a program's own small ones.
const COOKIE_BASE: u32 = 0x5E00_0000;
static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn runs() -> std::sync::MutexGuard<'static, Vec<Run>> { RUNS.lock().unwrap_or_else(|e| e.into_inner()) }
fn ours(cookie: u32) -> bool { cookie >= COOKIE_BASE }

/// Puts what an event says into the table. Whichever task hears an event records it
/// here, so a `wait` never loses a start another task's `spawn` is waiting for.
fn note(event: Event) {
    let mut runs = runs();
    match event {
        Event::Started { cookie, task, .. } => {
            if let Some(run) = runs.iter_mut().find(|r| r.cookie == cookie) { run.task = task; run.started = true; }
        }
        Event::Refused { cookie, code } => {
            if let Some(run) = runs.iter_mut().find(|r| r.cookie == cookie) { run.refused = Some(code); }
        }
        Event::Done { cookie, task, status, .. } => {
            if let Some(run) = runs.iter_mut().find(|r| r.cookie == cookie) { run.task = task; run.started = true; run.ended = Some(status); }
        }
    }
}

/// Lets other tasks run while waiting: the main task sleeps briefly (anything reaching
/// its inbox wakes it), a worker yields.
fn pause(round: u32) {
    if sex_app::thread::is_worker() { sex_app::thread::pause(round) } else { let _ = sex_pdx::sys_wait_self_ticks(4); }
}

fn files_errno(code: u8) -> i32 {
    match code {
        w::NOT_FOUND | w::NAME => errno::ENOENT,
        w::DENIED => errno::EACCES,
        w::UNAVAILABLE => errno::EAGAIN,
        _ => errno::EIO,
    }
}

fn refusal_errno(code: u32) -> i32 {
    match code {
        store::NOT_FOUND => errno::ENOENT,
        store::NOT_ALLOWED => errno::EACCES,
        store::NO_FREE_ROW | store::NO_MORE_RECORDS | store::IMAGE_BUSY => errno::EAGAIN,
        store::TOO_LARGE => errno::E2BIG,
        store::NOT_A_PROGRAM | store::CORRUPT => errno::ENOEXEC,
        _ => errno::EIO,
    }
}

/// What a pipe run gets beyond its words: its streams (`console_relay::stdio`), its
/// environment (`KEY=VALUE\0` entries) and working folder.
#[derive(Default)]
pub struct Handed<'a> {
    pub stdio: u8,
    pub env: &'a [u8],
    pub cwd: Option<&'a [u8]>,
}
impl Handed<'_> {
    /// Nothing handed over: a plain run, with no relay slot.
    fn none(&self) -> bool { self.stdio == 0 && self.env.is_empty() && self.cwd.is_none() }
}

/// Starts `program` with `words`. Returns the run's cookie and task, or an error number.
pub fn spawn(program: &[u8], words: &[&[u8]], network: bool, windowed: bool) -> Result<(u32, u32), i32> {
    spawn_with(program, words, network, windowed, &Handed::default()).map(|(cookie, task, _)| (cookie, task))
}

/// [`spawn`], handing over what `handed` says; the relay slot too, for a pipe run.
pub fn spawn_with(program: &[u8], words: &[&[u8]], network: bool, windowed: bool, handed: &Handed) -> Result<(u32, u32, Option<u8>), i32> {
    let name: &[u8] = if program.starts_with(b"@/") { program }
        else if let Some(rest) = program.strip_prefix(store::BIN) { rest }
        else if program.contains(&b'/') || program.is_empty() { return Err(errno::ENOENT) }
        else { program };
    if !program.starts_with(b"@/") && store::pack_name(name).is_none() { return Err(errno::ENOENT); }
    if handed.env.len() > sex_pdx::console_relay::ENV_BYTES { return Err(errno::E2BIG); }
    let piped = !handed.none();
    let mut line = Vec::new();
    if !windowed { line.extend_from_slice(HIDDEN_WORD); }
    if piped { if !line.is_empty() { line.push(b' '); } line.extend_from_slice(PIPED_WORD); }
    let joined = crate::args::join(words);
    if !line.is_empty() && !joined.is_empty() { line.push(b' '); }
    line.extend_from_slice(&joined);
    if line.len() > w::ARGS_MAX { return Err(errno::E2BIG); }
    let slot = if piped { Some(slots::take().ok_or(errno::EAGAIN)?) } else { None };
    let cookie = COOKIE_BASE + (NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0x00FF_FFFF);
    // Known before Shell can answer, so any task that hears the answer records it.
    runs().push(Run { cookie, task: 0, ended: None, started: false, refused: None, slot });
    let result = start(cookie, name, &line, network, windowed, slot.map(|s| (s, handed)));
    match result {
        Ok((cookie, task)) => {
            // The run holds the slot until it is forgotten, each piped stream until closed.
            if let Some(s) = slot { slots::hold(s, 1 + (0..3).filter(|n| c::stream(handed.stdio, *n) == c::PIPED).count() as u8); }
            Ok((cookie, task, slot))
        }
        Err(e) => {
            forget_cookie(cookie);
            if let Some(s) = slot { slots::give_back(s); }
            Err(e)
        }
    }
}

/// A program file's PREPARE path: `/bin/NAME` for a bare name, or `@/path` as it is.
fn program_path(name: &[u8]) -> Result<Vec<u8>, i32> {
    if name.first() == Some(&b'@') { return Ok(name.to_vec()); }
    let mut path = [0u8; 64];
    let n = sex_pdx::launch::bin_path(name, &mut path).ok_or(errno::ENOENT)?;
    Ok(path[..n].to_vec())
}

/// Asks Files to prepare a pipe run in `slot` with what `handed` gives it, then Shell to
/// start it.
fn start_piped(cookie: u32, name: &[u8], line: &[u8], network: bool, windowed: bool, slot: u8, handed: &Handed) -> Result<(), i32> {
    use sex_app::files::direct::relay;
    let ticket = relay::prepare_piped(&program_path(name)?, line, slot as usize, handed.stdio).map_err(files_errno)?;
    if let Some(cwd) = handed.cwd {
        // As this program names it: from its own working folder.
        let resolved = crate::path::resolve(cwd).map_err(|e| match e { crate::path::Refused::AboveFolder => errno::EACCES, _ => errno::EINVAL })?;
        let folder = if resolved.first() == Some(&b'@') { resolved } else { [b"/".as_slice(), &resolved].concat() };
        relay::set_cwd_at(slot as usize, &folder).map_err(|e| if e == w::NAME { errno::ENOENT } else { files_errno(e) })?;
    }
    if !handed.env.is_empty() { relay::set_env_at(slot as usize, handed.env).map_err(|_| errno::E2BIG)?; }
    launch::start_ticket(ticket, network, !windowed, cookie).map_err(|_| errno::EAGAIN)
}

/// Asks Shell to start the run `cookie` names and waits for its answer.
fn start(cookie: u32, name: &[u8], line: &[u8], network: bool, windowed: bool, piped: Option<(u8, &Handed)>) -> Result<(u32, u32), i32> {
    // Shell reads one program at a time: a start refused as busy is asked again.
    let mut busy = 0u32;
    loop {
        let asked = match piped {
            Some((slot, handed)) => start_piped(cookie, name, line, network, windowed, slot, handed).map_err(Some),
            None => match launch::run_with(name, line, network, !windowed, cookie) {
                Ok(()) => Ok(()),
                Err(launch::Error::Files(code)) => Err(Some(files_errno(code))),
                Err(launch::Error::Shell) => Err(None),
                Err(_) => Err(Some(errno::EAGAIN)),
            },
        };
        match asked {
            Ok(()) => {}
            Err(None) | Err(Some(errno::EAGAIN)) if busy < 200 => { busy += 1; pause(busy); continue; }
            Err(e) => return Err(e.unwrap_or(errno::EAGAIN)),
        }
        let mut round = 0;
        let answer = loop {
            while let Some(e) = launch::poll_where(ours) { note(e); }
            let answer = {
                let mut runs = runs();
                let run = runs.iter_mut().find(|r| r.cookie == cookie).ok_or(errno::EIO)?;
                if run.started { Some(Ok(run.task)) } else { run.refused.take().map(Err) }
            };
            if let Some(a) = answer { break a; }
            round += 1;
            pause(round);
        };
        match answer {
            Ok(task) => {
                sex_pdx::serial_record!("[std.process.spawn] cookie={:#x} task={} hidden={} network={} piped={}", cookie, task, !windowed as u8, network as u8, piped.map_or(0, |(_, h)| 0x100 | h.stdio as u32));
                return Ok((cookie, task));
            }
            Err(code) if code == store::IMAGE_BUSY && busy < 200 => { busy += 1; for r in 0..20 { pause(r + busy); } }
            Err(code) => {
                sex_pdx::serial_record!("[std.process.refused] cookie={:#x} code={} why={}", cookie, code, String::from_utf8_lossy(store::refusal(code as u64)));
                return Err(refusal_errno(code));
            }
        }
    }
}

fn forget_cookie(cookie: u32) {
    let mut runs = runs();
    if let Some(at) = runs.iter().position(|r| r.cookie == cookie) { runs.remove(at); }
}

/// The run `id` names: its cookie or its task.
fn find(runs: &[Run], id: i64) -> Option<usize> {
    runs.iter().position(|r| r.started && (id == r.cookie as i64 || id == r.task as i64))
}

/// How a run ended. `id` is a cookie or task, or -1 for any run started here. Returns
/// None while it runs (when not `block`), else (task, status); Err(errno) if `id`
/// names no run started here.
pub fn wait(id: i64, block: bool) -> Result<Option<(u32, Option<u32>)>, i32> {
    let mut round = 0;
    loop {
        {
            let runs = runs();
            if id == -1 {
                if !runs.iter().any(|r| r.started) { return Err(errno::ECHILD); }
                if let Some(r) = runs.iter().find(|r| r.ended.is_some()) { return Ok(Some((r.task, r.ended.unwrap()))); }
            } else {
                let at = find(&runs, id).ok_or(errno::ECHILD)?;
                if let Some(status) = runs[at].ended { return Ok(Some((runs[at].task, status))); }
            }
        }
        // Every event for runs started here is taken: one waiter may hear another's.
        let mut heard = false;
        while let Some(e) = launch::poll_where(ours) { note(e); heard = true; }
        if heard { continue; }
        if !block { return Ok(None); }
        round += 1;
        pause(round);
    }
}

/// Ends run `id` (a cookie or task). A run that already ended is left as it is.
pub fn kill(id: i64) -> Result<(), i32> {
    let run = { let runs = runs(); runs[find(&runs, id).ok_or(errno::ESRCH)?] };
    if run.ended.is_none() {
        sex_pdx::serial_record!("[std.process.kill] cookie={:#x} task={}", run.cookie, run.task);
        launch::kill(run.cookie);
    }
    Ok(())
}

/// Forgets run `id` (its status was read, or nobody will ask).
pub fn forget(id: i64) {
    let slot = {
        let mut runs = runs();
        match find(&runs, id) { Some(at) => runs.remove(at).slot, None => None }
    };
    if let Some(s) = slot { slots::give_back(s); }
}

/// This run's relay slots for pipe runs: who holds each (the run, and each of its
/// piped streams still open). A slot whose holders are all gone is given back to Files
/// (RELEASE), once its run has ended: until then Files keeps it, and so does this table.
mod slots {
    use super::c;
    use std::sync::Mutex;
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Slot { Free, Held(u8), Ending }
    static SLOTS: Mutex<[Slot; c::SLOTS]> = Mutex::new([Slot::Free; c::SLOTS]);
    fn table() -> std::sync::MutexGuard<'static, [Slot; c::SLOTS]> { SLOTS.lock().unwrap_or_else(|e| e.into_inner()) }
    /// Gives slot `s` back to Files; false while its run still runs.
    fn release(s: usize) -> bool { sex_app::files::direct::relay::release_at(s) != Err(sex_pdx::fs::PENDING) }
    /// A free slot, now taken; slots whose runs ended since are given back first.
    pub fn take() -> Option<u8> {
        let mut t = table();
        for s in 0..c::SLOTS { if t[s] == Slot::Ending && release(s) { t[s] = Slot::Free; } }
        let s = t.iter().position(|x| *x == Slot::Free)?;
        t[s] = Slot::Held(0);
        Some(s as u8)
    }
    /// The run in `s` started: `holders` hold it.
    pub fn hold(s: u8, holders: u8) { table()[s as usize] = Slot::Held(holders); }
    /// One holder of `s` is gone (the start failed, the run was forgotten, a pipe closed).
    pub fn give_back(s: u8) {
        let mut t = table();
        let s = s as usize;
        let left = match t[s] { Slot::Held(n) => n.saturating_sub(1), _ => return };
        t[s] = if left > 0 { Slot::Held(left) } else if release(s) { Slot::Free } else { Slot::Ending };
    }
}

// ---- pipes, this program's side ----------------------------------------------------------

/// Up to `out.len()` bytes of the child's stdout (`err`: stderr) in `slot`. 0 at its end:
/// the child ended and everything it wrote was read. Waits for some unless `!block`,
/// when nothing yet is `EAGAIN`.
pub fn pipe_read(slot: u8, err: bool, out: &mut [u8], block: bool) -> Result<usize, i32> {
    use sex_app::files::direct::relay;
    let mut round = 0;
    loop {
        let (status, n) = relay::take_into(slot as usize, err, out).map_err(files_errno)?;
        if n > 0 || out.is_empty() { return Ok(n); }
        if matches!(status.state, c::ENDED | c::NONE) { return Ok(0); }
        if !block { return Err(errno::EAGAIN); }
        round += 1;
        pause(round);
    }
}
/// Some of `bytes` to the child's stdin in `slot`, waiting while its ring is full;
/// `EPIPE` once the child ended.
pub fn pipe_write(slot: u8, bytes: &[u8]) -> Result<usize, i32> {
    let mut round = 0;
    loop {
        match sex_app::files::direct::relay::put_at(slot as usize, bytes) {
            Ok(0) if !bytes.is_empty() => { round += 1; pause(round); }
            Ok(n) => return Ok(n),
            Err(w::NOT_FOUND) => return Err(errno::EPIPE),
            Err(e) => return Err(files_errno(e)),
        }
    }
}
/// This program is done with the child's stream `n` (0 stdin, 1 stdout, 2 stderr).
pub fn pipe_close(slot: u8, n: u32) {
    use sex_app::files::direct::relay;
    let _ = match n { 0 => relay::hang_up_at(slot as usize), 1 => relay::close_read_at(slot as usize, 1), _ => relay::close_read_at(slot as usize, 2) };
    slots::give_back(slot);
}

/// This run's task.
pub fn task() -> u32 { crate::identity().1 }

// ---- the calls std's process module (and sex-libc) make --------------------------------

/// `program[..program_len]` with the NUL-ended words in `args[..args_len]`; `flags`: 1
/// network, 2 a window. The cookie, or `-errno`; the task in `*task`.
#[no_mangle]
pub unsafe extern "C" fn sys_process_spawn(program: *const u8, program_len: usize, args: *const u8, args_len: usize, flags: u32, task: *mut u32) -> i64 {
    let program = unsafe { core::slice::from_raw_parts(program, program_len) };
    let args = if args_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(args, args_len) } };
    // An empty buffer is no words at all, not one empty word.
    let mut words: Vec<&[u8]> = if args.is_empty() { Vec::new() } else { args.split(|b| *b == 0).collect() };
    // The last word's NUL leaves an empty piece after it.
    if args.last() == Some(&0) { words.pop(); }
    match spawn(program, &words, flags & 1 != 0, flags & 2 != 0) {
        Ok((cookie, t)) => { if !task.is_null() { unsafe { *task = t } } cookie as i64 }
        Err(e) => -(e as i64),
    }
}

/// 0: still running (`block` 0); 1: ended with `*status`; 2: ended without a status;
/// `-errno` if `id` (a cookie, a task, or -1 for any) names no run started here.
#[no_mangle]
pub unsafe extern "C" fn sys_process_wait(id: i64, block: i32, status: *mut i32, task: *mut u32) -> i32 {
    match wait(id, block != 0) {
        Ok(None) => 0,
        Ok(Some((t, s))) => {
            if !task.is_null() { unsafe { *task = t } }
            match s { Some(s) => { if !status.is_null() { unsafe { *status = s as i32 } } 1 } None => 2 }
        }
        Err(e) => -e,
    }
}

#[no_mangle]
pub extern "C" fn sys_process_kill(id: i64) -> i32 { match kill(id) { Ok(()) => 0, Err(e) => -e } }

#[no_mangle]
pub extern "C" fn sys_process_forget(id: i64) { forget(id) }

/// [`sys_process_spawn`] handing over more: `stdio` the stdio word
/// (`console_relay::stdio`), `env[..env_len]` the environment (`KEY=VALUE\0` entries),
/// `cwd[..cwd_len]` the working folder (none when `cwd` is null). The relay slot of a
/// pipe run in `*slot` (-1 for a plain run).
#[no_mangle]
pub unsafe extern "C" fn sys_process_spawn_with(program: *const u8, program_len: usize, args: *const u8, args_len: usize, flags: u32,
                                                stdio: u32, env: *const u8, env_len: usize, cwd: *const u8, cwd_len: usize,
                                                task: *mut u32, slot: *mut i32) -> i64 {
    let program = unsafe { core::slice::from_raw_parts(program, program_len) };
    let args = if args_len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(args, args_len) } };
    let mut words: Vec<&[u8]> = if args.is_empty() { Vec::new() } else { args.split(|b| *b == 0).collect() };
    if args.last() == Some(&0) { words.pop(); }
    let env = if env_len == 0 || env.is_null() { &[][..] } else { unsafe { core::slice::from_raw_parts(env, env_len) } };
    let cwd = if cwd.is_null() { None } else { Some(unsafe { core::slice::from_raw_parts(cwd, cwd_len) }) };
    let handed = Handed { stdio: stdio as u8 & 0x3F, env, cwd };
    match spawn_with(program, &words, flags & 1 != 0, flags & 2 != 0, &handed) {
        Ok((cookie, t, s)) => {
            if !task.is_null() { unsafe { *task = t } }
            if !slot.is_null() { unsafe { *slot = s.map_or(-1, |s| s as i32) } }
            cookie as i64
        }
        Err(e) => -(e as i64),
    }
}
/// Up to `len` bytes of a pipe run's stdout (`stream` 1) or stderr (2) into `out`: the
/// count, 0 at its end, `-EAGAIN` with nothing yet and `block` 0.
#[no_mangle]
pub unsafe extern "C" fn sys_process_pipe_read(slot: i32, stream: u32, out: *mut u8, len: usize, block: i32) -> isize {
    if !(0..PIPE_SLOTS as i32).contains(&slot) || out.is_null() { return -(errno::EBADF as isize); }
    let out = unsafe { core::slice::from_raw_parts_mut(out, len) };
    match pipe_read(slot as u8, stream == 2, out, block != 0) { Ok(n) => n as isize, Err(e) => -(e as isize) }
}
/// Some of `bytes[..len]` to a pipe run's stdin: the count, or `-EPIPE` once it ended.
#[no_mangle]
pub unsafe extern "C" fn sys_process_pipe_write(slot: i32, bytes: *const u8, len: usize) -> isize {
    if !(0..PIPE_SLOTS as i32).contains(&slot) || bytes.is_null() { return -(errno::EBADF as isize); }
    let bytes = unsafe { core::slice::from_raw_parts(bytes, len) };
    match pipe_write(slot as u8, bytes) { Ok(n) => n as isize, Err(e) => -(e as isize) }
}
/// This program closes its end of a pipe run's stream (0 stdin, 1 stdout, 2 stderr).
#[no_mangle]
pub extern "C" fn sys_process_pipe_close(slot: i32, stream: u32) {
    if (0..PIPE_SLOTS as i32).contains(&slot) { pipe_close(slot as u8, stream.min(2)); }
}
/// Lets other tasks run while waiting on something (std's `read_output`).
#[no_mangle]
pub extern "C" fn sys_process_pause(round: u32) { pause(round) }

#[no_mangle]
pub extern "C" fn sys_process_self() -> u32 { task() }
