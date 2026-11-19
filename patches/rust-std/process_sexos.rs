// SPDX-License-Identifier: MIT OR Apache-2.0
//! `std::process` for SexOS programs (x86_64-sex-std.json, the Hermit-based target).
//!
//! This file is not compiled in the SexOS tree. scripts/sex_std_source.py copies the
//! toolchain's `library/` and adds it as `std/src/sys/process/sexos.rs`, chosen for
//! `target_os = "hermit"` in place of `unsupported.rs`; the build trace then builds std
//! from that copy (`__CARGO_TESTS_ONLY_SRC_ROOT`). It is std's `unsupported.rs` for this
//! toolchain with the parts that run a program filled in.
//!
//! What it does is in the SexOS library the program links: `crates/sex-std`
//! (`sys_process_*`, src/process.rs), over `sex_app::launch`, so a started program gets
//! only what this run holds, and nothing here is a system call of its own.
//!
//! * The program is `/bin/NAME`, a bare NAME, or `@/path` of a program file this run
//!   holds. Arguments travel as one quoted line (`sex_std::args`).
//! * The environment (this run's, with the command's changes) and `current_dir` go to the
//!   child through Files, with its piped streams (`sex_std::process`: pipe runs). Two
//!   names are read here and not passed on: `SEXOS_RUN_WINDOWED=1` gives the run a
//!   window on the workspace (default: a hidden helper, no window, its output only in the
//!   serial record) and `SEXOS_RUN_NETWORK=1` asks for the network, which only a run
//!   that has it may hand on. `sex_std::process::CommandExt` sets them.
//! * `Stdio::piped()` gives a `ChildPipe` over the child's relay slot in Files; `output()`
//!   reads stdout and stderr together. A child's pipe handed to another `Command`
//!   (`Stdio::from(child.stdout)`) and `Stdio::from(File)` are `Unsupported`.
//! * A run that reports no status (it was killed, or it crashed) has no `code()`, as a
//!   Unix process ended by a signal.

use super::env::{CommandEnv, CommandEnvs, CommandResolvedEnvs};
pub use crate::ffi::OsString as EnvKey;
use crate::ffi::{OsStr, OsString};
use crate::num::NonZero;
use crate::path::Path;
use crate::process::StdioPipes;
use crate::sys::fs::File;
use crate::io::{BorrowedCursor, IoSlice, IoSliceMut};
use crate::{fmt, io};

// crates/sex-std/src/process.rs.
unsafe extern "C" {
    /// 0: still running (with `block` 0); 1: ended with `*status`; 2: ended without a
    /// status; `-errno` if `id` is not a run this program started.
    fn sys_process_wait(id: i64, block: i32, status: *mut i32, task: *mut u32) -> i32;
    fn sys_process_kill(id: i64) -> i32;
    /// The run's record is no longer needed (`Child` dropped).
    fn sys_process_forget(id: i64);
    /// Starts `program` with the NUL-ended words at `args` (`flags`: 1 network, 2 a
    /// window), handing over the stdio word (2 bits per stream: 0 inherit, 1 piped,
    /// 2 null), the environment (`KEY=VALUE\0` entries) and the working folder (none when
    /// `cwd` is null). The run's own id (cookie), or `-errno` if it did not start; its
    /// task in `*task`, a pipe run's relay slot in `*slot` (else -1).
    fn sys_process_spawn_with(program: *const u8, program_len: usize, args: *const u8, args_len: usize, flags: u32,
                              stdio: u32, env: *const u8, env_len: usize, cwd: *const u8, cwd_len: usize,
                              task: *mut u32, slot: *mut i32) -> i64;
    /// Up to `len` bytes of stream 1 (stdout) or 2 (stderr): the count, 0 at the end,
    /// `-EAGAIN` when nothing waits and `block` is 0, else `-errno`.
    fn sys_process_pipe_read(slot: i32, stream: u32, out: *mut u8, len: usize, block: i32) -> isize;
    /// Some of `bytes` to the child's stdin: the count, or `-errno` (`EPIPE`: it ended).
    fn sys_process_pipe_write(slot: i32, bytes: *const u8, len: usize) -> isize;
    /// This end of stream 0, 1 or 2 is closed.
    fn sys_process_pipe_close(slot: i32, stream: u32);
    /// Lets other tasks run while waiting (`round` counts the looks so far).
    fn sys_process_pause(round: u32);
    /// This run's task, the id `std::process::id` gives.
    fn sys_process_self() -> u32;
}

const FLAG_NETWORK: u32 = 1;
const FLAG_WINDOWED: u32 = 2;
const WINDOWED: &str = "SEXOS_RUN_WINDOWED";
const NETWORK: &str = "SEXOS_RUN_NETWORK";

fn not_handed() -> io::Error {
    io::const_error!(io::ErrorKind::Unsupported, "SexOS cannot hand a child a file or another child's pipe as a stream")
}

const INHERIT: u32 = 0;
const PIPED: u32 = 1;
const NULL: u32 = 2;
const EAGAIN: i32 = 11;

////////////////////////////////////////////////////////////////////////////////
// Command
////////////////////////////////////////////////////////////////////////////////

pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    env: CommandEnv,

    cwd: Option<OsString>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
}

#[derive(Debug)]
pub enum Stdio {
    Inherit,
    Null,
    MakePipe,
    ParentStdout,
    ParentStderr,
    #[allow(dead_code)] // This variant exists only for the Debug impl
    InheritFile(File),
    #[allow(dead_code)] // Refused at spawn; kept so its end closes when dropped
    Pipe(ChildPipe),
}

impl Stdio {
    /// How the child gets it: its own (inherit), through Files (piped), or nothing.
    fn kind(&self) -> io::Result<u32> {
        match self {
            Stdio::Inherit | Stdio::ParentStdout | Stdio::ParentStderr => Ok(INHERIT),
            Stdio::Null => Ok(NULL),
            Stdio::MakePipe => Ok(PIPED),
            Stdio::InheritFile(_) | Stdio::Pipe(_) => Err(not_handed()),
        }
    }
}

impl Command {
    pub fn new(program: &OsStr) -> Command {
        Command {
            program: program.to_owned(),
            args: vec![program.to_owned()],
            env: Default::default(),
            cwd: None,
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }

    pub fn arg(&mut self, arg: &OsStr) {
        self.args.push(arg.to_owned());
    }

    pub fn env_mut(&mut self) -> &mut CommandEnv {
        &mut self.env
    }

    pub fn cwd(&mut self, dir: &OsStr) {
        self.cwd = Some(dir.to_owned());
    }

    pub fn stdin(&mut self, stdin: Stdio) {
        self.stdin = Some(stdin);
    }

    pub fn stdout(&mut self, stdout: Stdio) {
        self.stdout = Some(stdout);
    }

    pub fn stderr(&mut self, stderr: Stdio) {
        self.stderr = Some(stderr);
    }

    pub fn get_program(&self) -> &OsStr {
        &self.program
    }

    pub fn get_args(&self) -> CommandArgs<'_> {
        let mut iter = self.args.iter();
        iter.next();
        CommandArgs { iter }
    }

    pub fn get_envs(&self) -> CommandEnvs<'_> {
        self.env.iter()
    }

    pub fn get_env_clear(&self) -> bool {
        self.env.does_clear()
    }

    pub fn get_resolved_envs(&self) -> CommandResolvedEnvs {
        CommandResolvedEnvs::new(self.env.capture())
    }

    pub fn get_current_dir(&self) -> Option<&Path> {
        self.cwd.as_ref().map(|cs| Path::new(cs))
    }

    fn flag(&self, name: &str) -> bool {
        self.env.iter().any(|(k, v)| k == OsStr::new(name) && v.is_some_and(|v| v == OsStr::new("1")))
    }

    fn start(&self, stdio: [u32; 3]) -> io::Result<(Process, i32)> {
        let mut words = Vec::new();
        for arg in &self.args[1..] {
            let bytes = arg.as_encoded_bytes();
            if bytes.contains(&0) {
                return Err(io::const_error!(io::ErrorKind::InvalidInput, "an argument holds a NUL byte"));
            }
            words.extend_from_slice(bytes);
            words.push(0);
        }
        let program = self.program.as_encoded_bytes();
        let flags = if self.flag(NETWORK) { FLAG_NETWORK } else { 0 } | if self.flag(WINDOWED) { FLAG_WINDOWED } else { 0 };
        // The child's environment: this run's, with the command's changes, less the
        // two names that only steer the start.
        let mut env = Vec::new();
        for (key, value) in self.env.capture() {
            if key.as_os_str() == OsStr::new(WINDOWED) || key.as_os_str() == OsStr::new(NETWORK) {
                continue;
            }
            let (key, value) = (key.as_encoded_bytes(), value.as_encoded_bytes());
            if key.is_empty() || key.contains(&b'=') || key.contains(&0) || value.contains(&0) {
                return Err(io::const_error!(io::ErrorKind::InvalidInput, "an environment name or value holds a NUL or `=`"));
            }
            env.extend_from_slice(key);
            env.push(b'=');
            env.extend_from_slice(value);
            env.push(0);
        }
        let cwd = self.cwd.as_ref().map(|c| c.as_encoded_bytes());
        if cwd.is_some_and(|c| c.contains(&0)) {
            return Err(io::const_error!(io::ErrorKind::InvalidInput, "the working folder holds a NUL byte"));
        }
        let word = stdio[0] | stdio[1] << 2 | stdio[2] << 4;
        let (mut task, mut slot) = (0u32, -1i32);
        let r = unsafe {
            sys_process_spawn_with(
                program.as_ptr(), program.len(), words.as_ptr(), words.len(), flags,
                word, env.as_ptr(), env.len(),
                cwd.map_or(crate::ptr::null(), |c| c.as_ptr()), cwd.map_or(0, |c| c.len()),
                &mut task, &mut slot,
            )
        };
        if r < 0 {
            return Err(io::Error::from_raw_os_error(-r as i32));
        }
        Ok((Process { id: r, task, status: None }, slot))
    }

    pub fn spawn(
        &mut self,
        default: Stdio,
        needs_stdin: bool,
    ) -> io::Result<(Process, StdioPipes)> {
        let stdin = match &self.stdin {
            Some(s) => s.kind()?,
            None if needs_stdin => default.kind()?,
            None => NULL,
        };
        let stdout = self.stdout.as_ref().unwrap_or(&default).kind()?;
        let stderr = self.stderr.as_ref().unwrap_or(&default).kind()?;
        let (process, slot) = self.start([stdin, stdout, stderr])?;
        let pipe = |kind: u32, stream: u32| (kind == PIPED && slot >= 0).then(|| ChildPipe { slot, stream });
        Ok((process, StdioPipes { stdin: pipe(stdin, 0), stdout: pipe(stdout, 1), stderr: pipe(stderr, 2) }))
    }
}

/// `output()`: stdout and stderr piped unless they were set, stdin null unless set.
pub fn output(cmd: &mut Command) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let (mut process, mut pipes) = cmd.spawn(Stdio::MakePipe, false)?;
    drop(pipes.stdin.take());
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    match (pipes.stdout.take(), pipes.stderr.take()) {
        (None, None) => {}
        (Some(out), None) => {
            out.read_to_end(&mut stdout)?;
        }
        (None, Some(err)) => {
            err.read_to_end(&mut stderr)?;
        }
        (Some(out), Some(err)) => read_output(out, &mut stdout, err, &mut stderr)?,
    }
    let status = process.wait()?;
    Ok((status, stdout, stderr))
}

impl From<ChildPipe> for Stdio {
    fn from(pipe: ChildPipe) -> Stdio {
        Stdio::Pipe(pipe)
    }
}

impl From<crate::sys::pipe::Pipe> for Stdio {
    fn from(pipe: crate::sys::pipe::Pipe) -> Stdio {
        pipe.diverge()
    }
}

impl From<io::Stdout> for Stdio {
    fn from(_: io::Stdout) -> Stdio {
        Stdio::ParentStdout
    }
}

impl From<io::Stderr> for Stdio {
    fn from(_: io::Stderr) -> Stdio {
        Stdio::ParentStderr
    }
}

impl From<File> for Stdio {
    fn from(file: File) -> Stdio {
        Stdio::InheritFile(file)
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            let mut debug_command = f.debug_struct("Command");
            debug_command.field("program", &self.program).field("args", &self.args);
            if !self.env.is_unchanged() {
                debug_command.field("env", &self.env);
            }
            if self.cwd.is_some() {
                debug_command.field("cwd", &self.cwd);
            }
            if self.stdin.is_some() {
                debug_command.field("stdin", &self.stdin);
            }
            if self.stdout.is_some() {
                debug_command.field("stdout", &self.stdout);
            }
            if self.stderr.is_some() {
                debug_command.field("stderr", &self.stderr);
            }
            debug_command.finish()
        } else {
            if self.program != self.args[0] {
                write!(f, "[{:?}] ", self.program)?;
            }
            write!(f, "{:?}", self.args[0])?;
            for arg in &self.args[1..] {
                write!(f, " {:?}", arg)?;
            }
            Ok(())
        }
    }
}

/// How a run ended: the status it reported, or none (it was killed or crashed).
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatus(Option<i32>);

/// std's default status is success.
impl Default for ExitStatus {
    fn default() -> Self {
        ExitStatus(Some(0))
    }
}

impl ExitStatus {
    pub fn exit_ok(&self) -> Result<(), ExitStatusError> {
        match self.0 {
            Some(0) => Ok(()),
            _ => Err(ExitStatusError(*self)),
        }
    }

    pub fn code(&self) -> Option<i32> {
        self.0
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(code) => write!(f, "exit status: {code}"),
            None => write!(f, "ended without a status (killed or crashed)"),
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatusError(ExitStatus);

impl Into<ExitStatus> for ExitStatusError {
    fn into(self) -> ExitStatus {
        self.0
    }
}

impl ExitStatusError {
    pub fn code(self) -> Option<NonZero<i32>> {
        self.0.0.and_then(NonZero::new)
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitCode(u8);

impl ExitCode {
    pub const SUCCESS: ExitCode = ExitCode(0);
    pub const FAILURE: ExitCode = ExitCode(1);

    pub fn as_i32(&self) -> i32 {
        self.0 as i32
    }
}

impl From<u8> for ExitCode {
    fn from(code: u8) -> Self {
        Self(code)
    }
}

/// A run this program started: its id in sex-std's table and its task.
pub struct Process {
    id: i64,
    task: u32,
    status: Option<ExitStatus>,
}

impl Process {
    pub fn id(&self) -> u32 {
        self.task
    }

    pub fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        let r = unsafe { sys_process_kill(self.id) };
        if r < 0 { Err(io::Error::from_raw_os_error(-r)) } else { Ok(()) }
    }

    fn ask(&mut self, block: bool) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let (mut code, mut task) = (0i32, 0u32);
        let r = unsafe { sys_process_wait(self.id, block as i32, &mut code, &mut task) };
        let status = match r {
            0 => return Ok(None),
            1 => ExitStatus(Some(code)),
            2 => ExitStatus(None),
            e if e < 0 => return Err(io::Error::from_raw_os_error(-e)),
            _ => return Err(io::const_error!(io::ErrorKind::Other, "unexpected answer about the run")),
        };
        self.status = Some(status);
        Ok(Some(status))
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.ask(true)? {
                return Ok(status);
            }
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.ask(false)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { sys_process_forget(self.id) }
    }
}

pub struct CommandArgs<'a> {
    iter: crate::slice::Iter<'a, OsString>,
}

impl<'a> Iterator for CommandArgs<'a> {
    type Item = &'a OsStr;
    fn next(&mut self) -> Option<&'a OsStr> {
        self.iter.next().map(|os| &**os)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a> ExactSizeIterator for CommandArgs<'a> {
    fn len(&self) -> usize {
        self.iter.len()
    }
    fn is_empty(&self) -> bool {
        self.iter.is_empty()
    }
}

impl<'a> fmt::Debug for CommandArgs<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter.clone()).finish()
    }
}

/// This program's end of a child's stdin, stdout or stderr: a stream of the child's
/// relay slot in Files (`sex_std::process::pipe_*`).
pub struct ChildPipe {
    slot: i32,
    /// 0 stdin, 1 stdout, 2 stderr.
    stream: u32,
}

impl ChildPipe {
    fn read_some(&self, buf: &mut [u8], block: bool) -> io::Result<Option<usize>> {
        if self.stream == 0 {
            return Err(io::const_error!(io::ErrorKind::Unsupported, "a child's stdin is not read here"));
        }
        let r = unsafe { sys_process_pipe_read(self.slot, self.stream, buf.as_mut_ptr(), buf.len(), block as i32) };
        if r == -(EAGAIN as isize) && !block {
            return Ok(None);
        }
        if r < 0 { Err(io::Error::from_raw_os_error(-r as i32)) } else { Ok(Some(r as usize)) }
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        Ok(self.read_some(buf, true)?.unwrap_or(0))
    }

    pub fn read_buf(&self, mut cursor: BorrowedCursor<'_, u8>) -> io::Result<()> {
        // SAFETY: the read writes bytes into the buffer and never reads from it.
        let buf = unsafe { &mut *(cursor.as_mut() as *mut [crate::mem::MaybeUninit<u8>] as *mut [u8]) };
        let n = self.read(buf)?;
        // SAFETY: exactly `n` bytes were filled.
        unsafe { cursor.advance(n) };
        Ok(())
    }

    pub fn read_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match bufs.iter_mut().find(|b| !b.is_empty()) {
            Some(buf) => self.read(buf),
            None => Ok(0),
        }
    }

    pub fn is_read_vectored(&self) -> bool {
        false
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let start = buf.len();
        let mut chunk = vec![0u8; 32 * 1024];
        loop {
            let n = self.read(&mut chunk)?;
            if n == 0 {
                return Ok(buf.len() - start);
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        if self.stream != 0 {
            return Err(io::const_error!(io::ErrorKind::Unsupported, "a child's output is not written here"));
        }
        if buf.is_empty() {
            return Ok(0);
        }
        let r = unsafe { sys_process_pipe_write(self.slot, buf.as_ptr(), buf.len()) };
        if r < 0 { Err(io::Error::from_raw_os_error(-r as i32)) } else { Ok(r as usize) }
    }

    pub fn write_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match bufs.iter().find(|b| !b.is_empty()) {
            Some(buf) => self.write(buf),
            None => Ok(0),
        }
    }

    pub fn is_write_vectored(&self) -> bool {
        false
    }
}

impl Drop for ChildPipe {
    fn drop(&mut self) {
        unsafe { sys_process_pipe_close(self.slot, self.stream) }
    }
}

impl fmt::Debug for ChildPipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChildPipe").field("slot", &self.slot).field("stream", &self.stream).finish()
    }
}

/// Both of a child's output streams to their ends, read as they come, so a child that
/// fills one while this program waits on the other is never stuck.
pub fn read_output(
    out: ChildPipe,
    stdout: &mut Vec<u8>,
    err: ChildPipe,
    stderr: &mut Vec<u8>,
) -> io::Result<()> {
    let mut chunk = vec![0u8; 32 * 1024];
    let (mut out_done, mut err_done, mut round) = (false, false, 0u32);
    while !(out_done && err_done) {
        let mut heard = false;
        for (pipe, done, into) in [(&out, &mut out_done, &mut *stdout), (&err, &mut err_done, &mut *stderr)] {
            if *done {
                continue;
            }
            match pipe.read_some(&mut chunk, false)? {
                Some(0) => *done = true,
                Some(n) => {
                    into.extend_from_slice(&chunk[..n]);
                    heard = true;
                }
                None => {}
            }
        }
        if heard {
            round = 0;
        } else {
            round += 1;
            unsafe { sys_process_pause(round) };
        }
    }
    Ok(())
}

pub fn getpid() -> u32 {
    unsafe { sys_process_self() }
}
