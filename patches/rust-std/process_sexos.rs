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
//! * Environment variables and `current_dir` have no carrier between runs and are not
//!   passed on. Two names are read here instead: `SEXOS_RUN_WINDOWED=1` gives the run a
//!   window on the workspace (default: a hidden helper, no window, its output only in the
//!   serial record) and `SEXOS_RUN_NETWORK=1` asks for the network, which only a run
//!   that has it may hand on. `sex_std::process::CommandExt` sets them.
//! * There are no pipes between runs: `Stdio::piped()` (and so `output()` unless both
//!   streams are set to `null` or `inherit`) fails with `Unsupported`.
//! * A run that reports no status (it was killed, or it crashed) has no `code()`, as a
//!   Unix process ended by a signal.

use super::env::{CommandEnv, CommandEnvs, CommandResolvedEnvs};
pub use crate::ffi::OsString as EnvKey;
use crate::ffi::{OsStr, OsString};
use crate::num::NonZero;
use crate::path::Path;
use crate::process::StdioPipes;
use crate::sys::fs::File;
use crate::{fmt, io};

// crates/sex-std/src/process.rs.
unsafe extern "C" {
    /// Starts `program` with the NUL-ended words at `args`. `flags`: 1 network, 2 a
    /// window. The run's own id (cookie) on success, `-errno` if it did not start; its
    /// task in `*task`.
    fn sys_process_spawn(program: *const u8, program_len: usize, args: *const u8, args_len: usize, flags: u32, task: *mut u32) -> i64;
    /// 0: still running (with `block` 0); 1: ended with `*status`; 2: ended without a
    /// status; `-errno` if `id` is not a run this program started.
    fn sys_process_wait(id: i64, block: i32, status: *mut i32, task: *mut u32) -> i32;
    fn sys_process_kill(id: i64) -> i32;
    /// The run's record is no longer needed (`Child` dropped).
    fn sys_process_forget(id: i64);
    /// This run's task, the id `std::process::id` gives.
    fn sys_process_self() -> u32;
}

const FLAG_NETWORK: u32 = 1;
const FLAG_WINDOWED: u32 = 2;
const WINDOWED: &str = "SEXOS_RUN_WINDOWED";
const NETWORK: &str = "SEXOS_RUN_NETWORK";

fn no_pipes() -> io::Error {
    io::const_error!(io::ErrorKind::Unsupported, "SexOS has no pipes between programs yet: use Stdio::null() or Stdio::inherit()")
}

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
}

impl Stdio {
    fn usable(&self) -> bool {
        matches!(self, Stdio::Inherit | Stdio::Null | Stdio::ParentStdout | Stdio::ParentStderr)
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

    fn start(&self) -> io::Result<Process> {
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
        let mut task = 0u32;
        let r = unsafe { sys_process_spawn(program.as_ptr(), program.len(), words.as_ptr(), words.len(), flags, &mut task) };
        if r < 0 {
            return Err(io::Error::from_raw_os_error(-r as i32));
        }
        Ok(Process { id: r, task, status: None })
    }

    pub fn spawn(
        &mut self,
        default: Stdio,
        _needs_stdin: bool,
    ) -> io::Result<(Process, StdioPipes)> {
        let usable = |s: &Option<Stdio>| s.as_ref().unwrap_or(&default).usable();
        if !(usable(&self.stdin) && usable(&self.stdout) && usable(&self.stderr)) {
            return Err(no_pipes());
        }
        Ok((self.start()?, StdioPipes { stdin: None, stdout: None, stderr: None }))
    }
}

/// `output()`: std asks for both streams piped unless they were set; with no pipes,
/// only a command whose streams are `null` or `inherit` runs, and gives empty output.
pub fn output(cmd: &mut Command) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let set = |s: &Option<Stdio>| s.as_ref().is_some_and(Stdio::usable);
    if !(set(&cmd.stdout) && set(&cmd.stderr)) || cmd.stdin.as_ref().is_some_and(|s| !s.usable()) {
        return Err(no_pipes());
    }
    let mut process = cmd.start()?;
    let status = process.wait()?;
    Ok((status, Vec::new(), Vec::new()))
}

impl From<ChildPipe> for Stdio {
    fn from(pipe: ChildPipe) -> Stdio {
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

pub type ChildPipe = crate::sys::pipe::Pipe;

pub fn read_output(
    out: ChildPipe,
    _stdout: &mut Vec<u8>,
    _err: ChildPipe,
    _stderr: &mut Vec<u8>,
) -> io::Result<()> {
    match out.diverge() {}
}

pub fn getpid() -> u32 {
    unsafe { sys_process_self() }
}
