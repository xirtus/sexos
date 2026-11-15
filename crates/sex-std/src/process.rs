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
//! * **Not carried**: environment variables and `current_dir` (runs have no
//!   environment, and each program's folder is its own `/`). They are accepted and not
//!   passed on.
//! * **No pipes**: there is no byte channel between two runs yet, so `Stdio::piped()`
//!   fails with `Unsupported`, and so does `output()` unless stdout and stderr are both
//!   set to `null` or `inherit` (then it returns the status and empty buffers).
//!   `inherit` means the child's own console (its window, or the serial record), not
//!   this program's.
//!
//! sex-libc's `posix_spawn`, `waitpid`, `kill` and `system` use the same functions.
use std::sync::Mutex;
use std::os::hermit::hermit_abi::errno;
use sex_app::launch::{self, Event};
use sex_pdx::{fs as w, program_store as store};

/// The first word of a hidden run's argument line, put there by this crate: the
/// program's console then keeps to the serial record. sex-std takes it out before
/// `main`; a program not built on sex-std sees it as its first argument.
pub const HIDDEN_WORD: &[u8] = b"+hidden";

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

/// The error a program gets for what SexOS cannot do between runs yet (pipes).
pub fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "SexOS has no pipes between programs yet")
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

/// Starts `program` with `words`. Returns the run's cookie and task, or an error number.
pub fn spawn(program: &[u8], words: &[&[u8]], network: bool, windowed: bool) -> Result<(u32, u32), i32> {
    let name: &[u8] = if program.starts_with(b"@/") { program }
        else if let Some(rest) = program.strip_prefix(store::BIN) { rest }
        else if program.contains(&b'/') || program.is_empty() { return Err(errno::ENOENT) }
        else { program };
    if !program.starts_with(b"@/") && store::pack_name(name).is_none() { return Err(errno::ENOENT); }
    let mut line = Vec::new();
    if !windowed { line.extend_from_slice(HIDDEN_WORD); }
    let joined = crate::args::join(words);
    if !line.is_empty() && !joined.is_empty() { line.push(b' '); }
    line.extend_from_slice(&joined);
    if line.len() > w::ARGS_MAX { return Err(errno::E2BIG); }
    let cookie = COOKIE_BASE + (NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0x00FF_FFFF);
    // Known before Shell can answer, so any task that hears the answer records it.
    runs().push(Run { cookie, task: 0, ended: None, started: false, refused: None });
    let result = start(cookie, name, &line, network, windowed);
    if result.is_err() { forget_cookie(cookie); }
    result
}

/// Asks Shell to start the run `cookie` names and waits for its answer.
fn start(cookie: u32, name: &[u8], line: &[u8], network: bool, windowed: bool) -> Result<(u32, u32), i32> {
    // Shell reads one program at a time: a start refused as busy is asked again.
    let mut busy = 0u32;
    loop {
        match launch::run_with(name, line, network, !windowed, cookie) {
            Ok(()) => {}
            Err(launch::Error::Files(code)) => return Err(files_errno(code)),
            Err(launch::Error::Shell) if busy < 200 => { busy += 1; pause(busy); continue; }
            Err(_) => return Err(errno::EAGAIN),
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
                sex_pdx::serial_record!("[std.process.spawn] cookie={:#x} task={} hidden={} network={}", cookie, task, !windowed as u8, network as u8);
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
    let mut runs = runs();
    if let Some(at) = find(&runs, id) { runs.remove(at); }
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

#[no_mangle]
pub extern "C" fn sys_process_self() -> u32 { task() }
