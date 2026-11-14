// SPDX-License-Identifier: GPL-3.0-only
//! What the shell asks of the system it runs on.
//!
//! Paths given to a [`Host`] are always absolute and normalized (`/home/notes.txt`): the
//! engine joins them with [`Host::cwd`] itself. Programs do not run through the host's
//! methods but through [`crate::Step::RunExternal`], so the terminal's event loop never
//! blocks; only background jobs start through [`Host::spawn_job`].

use alloc::string::String;
use alloc::vec::Vec;

use crate::engine::ExternalRequest;
use crate::value::{Record, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Link,
    Other,
}

impl EntryKind {
    pub fn name(self) -> &'static str {
        match self { EntryKind::File => "file", EntryKind::Dir => "dir", EntryKind::Link => "link", EntryKind::Other => "other" }
    }
}

/// One entry of a folder, or what `stat` says about a path.
#[derive(Clone, Debug, PartialEq)]
pub struct DirEntry {
    /// The entry's own name, without its folder.
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Last change, unix seconds.
    pub modified: Option<i64>,
}

/// A program the host knows to be running, for `ps`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessInfo {
    pub pid: u64,
    pub name: String,
    pub state: String,
    /// Bytes of memory it holds, if known.
    pub memory: Option<u64>,
}

pub type JobId = u32;

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Running,
    Stopped,
    Done(i32),
}

/// A background job, for `jobs`.
#[derive(Clone, Debug, PartialEq)]
pub struct JobInfo {
    pub id: JobId,
    pub command: String,
    pub state: JobState,
}

/// The system under the shell. Read-only methods take `&self` so completion can call
/// them; changing ones take `&mut self`. Errors are short sentences shown to the person
/// (`"not found"`, `"permission denied"`).
pub trait Host {
    /// The current folder, absolute.
    fn cwd(&self) -> String;
    /// Makes `path` (absolute, already checked to be a folder) the current folder.
    fn set_cwd(&mut self, path: &str) -> Result<(), String>;
    /// Where a bare `cd` goes.
    fn home(&self) -> String { String::from("/") }

    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String>;
    fn stat(&self, path: &str) -> Result<DirEntry, String>;
    fn read_file(&self, path: &str) -> Result<Vec<u8>, String>;
    /// Writes a whole file, creating it; `append` adds to its end instead.
    fn write_file(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), String>;
    fn make_dir(&mut self, path: &str) -> Result<(), String>;
    /// Removes a file or an empty folder; `recursive` removes a folder with its contents.
    fn remove(&mut self, path: &str, recursive: bool) -> Result<(), String>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;

    /// `sys`: a record describing the machine.
    fn sys_info(&self) -> Value { Value::Record(Record::new()) }
    /// `ps`
    fn processes(&self) -> Vec<ProcessInfo> { Vec::new() }
    /// Now, unix seconds (`date now`).
    fn now(&self) -> i64 { 0 }

    /// Whether `name` is a program that can run (a `.wasm` in the folder, `/bin/name`, ...).
    /// A name that is neither a built-in nor a program is an error with a suggestion.
    fn is_external(&self, name: &str) -> bool;
    /// Program names for Tab completion.
    fn external_names(&self) -> Vec<String> { Vec::new() }

    /// Starts a program in the background (`cmd &`) and returns its job number.
    fn spawn_job(&mut self, request: &ExternalRequest) -> Result<JobId, String> {
        let _ = request;
        Err(String::from("background jobs are not available here"))
    }
    fn jobs(&self) -> Vec<JobInfo> { Vec::new() }
    /// Prepares job `id` to come to the front (`fg`); the engine then returns
    /// [`crate::Step::WaitJob`] for the terminal to wait on it.
    fn job_foreground(&mut self, id: JobId) -> Result<(), String> {
        let _ = id;
        Err(String::from("no such job"))
    }
    /// `kill`
    fn kill_job(&mut self, id: JobId) -> Result<(), String> {
        let _ = id;
        Err(String::from("no such job"))
    }
}

/// A host that only reads: what completion runs earlier pipeline stages against.
pub(crate) struct ReadOnly<'a>(pub &'a dyn Host);

const READ_ONLY: &str = "read-only while completing";

impl Host for ReadOnly<'_> {
    fn cwd(&self) -> String { self.0.cwd() }
    fn set_cwd(&mut self, _: &str) -> Result<(), String> { Err(String::from(READ_ONLY)) }
    fn home(&self) -> String { self.0.home() }
    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> { self.0.list_dir(path) }
    fn stat(&self, path: &str) -> Result<DirEntry, String> { self.0.stat(path) }
    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> { self.0.read_file(path) }
    fn write_file(&mut self, _: &str, _: &[u8], _: bool) -> Result<(), String> { Err(String::from(READ_ONLY)) }
    fn make_dir(&mut self, _: &str) -> Result<(), String> { Err(String::from(READ_ONLY)) }
    fn remove(&mut self, _: &str, _: bool) -> Result<(), String> { Err(String::from(READ_ONLY)) }
    fn rename(&mut self, _: &str, _: &str) -> Result<(), String> { Err(String::from(READ_ONLY)) }
    fn sys_info(&self) -> Value { self.0.sys_info() }
    fn processes(&self) -> Vec<ProcessInfo> { self.0.processes() }
    fn now(&self) -> i64 { self.0.now() }
    fn is_external(&self, name: &str) -> bool { self.0.is_external(name) }
    fn external_names(&self) -> Vec<String> { self.0.external_names() }
    fn jobs(&self) -> Vec<JobInfo> { self.0.jobs() }
}
