// SPDX-License-Identifier: GPL-3.0-only
//! [`MemHost`]: a [`Host`] over an in-memory tree, for tests and as a reference for
//! terminals implementing the trait.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::engine::{ExternalRequest, ExternalResult};
use crate::host::{DirEntry, EntryKind, Host, JobId, JobInfo, JobState, ProcessInfo};
use crate::path;
use crate::value::{Record, Value};

/// A program in a [`MemHost`]: arguments and input to output and exit code.
pub type Program = fn(&[String], Option<&[u8]>) -> (Vec<u8>, i32);

#[derive(Clone, Debug)]
enum Node {
    Dir { modified: i64 },
    /// `size` may exceed `data` for large files that are listed but never read.
    File { data: Vec<u8>, modified: i64, size: u64 },
}

/// Folders and files in memory, programs as functions, jobs as a list.
pub struct MemHost {
    pub cwd: String,
    pub home: String,
    pub now: i64,
    nodes: BTreeMap<String, Node>,
    pub programs: BTreeMap<String, Program>,
    pub jobs: Vec<JobInfo>,
    pub processes: Vec<ProcessInfo>,
    pub sys: Value,
    /// Jobs brought to the front with `fg`, in order.
    pub foregrounded: Vec<JobId>,
}

impl Default for MemHost {
    fn default() -> MemHost { MemHost::new() }
}

fn parent(p: &str) -> &str { path::split(p).0 }

impl MemHost {
    pub fn new() -> MemHost {
        let mut nodes = BTreeMap::new();
        nodes.insert(String::from("/"), Node::Dir { modified: 0 });
        MemHost {
            cwd: String::from("/"),
            home: String::from("/"),
            now: 0,
            nodes,
            programs: BTreeMap::new(),
            jobs: Vec::new(),
            processes: Vec::new(),
            sys: Value::Record(Record::new()),
            foregrounded: Vec::new(),
        }
    }

    fn norm(&self, p: &str) -> String { path::resolve(&self.cwd, &self.home, p) }

    /// Makes a folder and the folders above it.
    pub fn add_dir(&mut self, p: &str) -> &mut MemHost {
        let p = self.norm(p);
        if p != "/" {
            let up = parent(&p).to_string();
            if !self.nodes.contains_key(&up) { self.add_dir(&up); }
        }
        self.nodes.entry(p).or_insert(Node::Dir { modified: self.now });
        self
    }

    /// Writes a file, making its folders, last changed at `modified`.
    pub fn add_file(&mut self, p: &str, data: &[u8], modified: i64) -> &mut MemHost {
        let p = self.norm(p);
        let up = parent(&p).to_string();
        self.add_dir(&up);
        self.nodes.insert(p, Node::File { data: data.to_vec(), modified, size: data.len() as u64 });
        self
    }

    /// A file that lists as `size` bytes but holds none (for big files in tests).
    pub fn add_sized(&mut self, p: &str, size: u64, modified: i64) -> &mut MemHost {
        self.add_file(p, &[], modified);
        if let Some(Node::File { size: s, .. }) = self.nodes.get_mut(&self.norm(p)) { *s = size; }
        self
    }

    pub fn add_program(&mut self, name: &str, program: Program) -> &mut MemHost {
        self.programs.insert(name.to_string(), program);
        self
    }

    /// A file's bytes.
    pub fn file(&self, p: &str) -> Option<&[u8]> {
        match self.nodes.get(&self.norm(p)) { Some(Node::File { data, .. }) => Some(data), _ => None }
    }

    pub fn exists(&self, p: &str) -> bool { self.nodes.contains_key(&self.norm(p)) }

    /// Runs a request with its program (127 and a message when there is none).
    pub fn run(&mut self, request: &ExternalRequest) -> ExternalResult {
        match self.programs.get(&request.name) {
            Some(p) => {
                let (output, exit_code) = p(&request.args, request.input.as_deref());
                ExternalResult { output, exit_code }
            }
            None => ExternalResult { output: format!("{}: not found\n", request.name).into_bytes(), exit_code: 127 },
        }
    }

    fn entry(&self, p: &str, node: &Node) -> DirEntry {
        let name = path::split(p).1.to_string();
        match node {
            Node::Dir { modified } => DirEntry { name, kind: EntryKind::Dir, size: 0, modified: Some(*modified) },
            Node::File { modified, size, .. } => DirEntry { name, kind: EntryKind::File, size: *size, modified: Some(*modified) },
        }
    }

    fn below(&self, p: &str) -> Vec<String> {
        let prefix = if p == "/" { String::from("/") } else { format!("{}/", p) };
        self.nodes.keys().filter(|k| k.starts_with(&prefix) && k.as_str() != p).cloned().collect()
    }
}

impl Host for MemHost {
    fn cwd(&self) -> String { self.cwd.clone() }
    fn set_cwd(&mut self, p: &str) -> Result<(), String> { self.cwd = p.to_string(); Ok(()) }
    fn home(&self) -> String { self.home.clone() }

    fn list_dir(&self, p: &str) -> Result<Vec<DirEntry>, String> {
        match self.nodes.get(p) {
            Some(Node::Dir { .. }) => {}
            Some(_) => return Err(String::from("not a folder")),
            None => return Err(String::from("not found")),
        }
        Ok(self.nodes.iter().filter(|(k, _)| k.as_str() != "/" && parent(k) == p).map(|(k, n)| self.entry(k, n)).collect())
    }
    fn stat(&self, p: &str) -> Result<DirEntry, String> {
        self.nodes.get(p).map(|n| self.entry(p, n)).ok_or_else(|| String::from("not found"))
    }
    fn read_file(&self, p: &str) -> Result<Vec<u8>, String> {
        match self.nodes.get(p) {
            Some(Node::File { data, .. }) => Ok(data.clone()),
            Some(_) => Err(String::from("is a folder")),
            None => Err(String::from("not found")),
        }
    }
    fn write_file(&mut self, p: &str, data: &[u8], append: bool) -> Result<(), String> {
        if !matches!(self.nodes.get(parent(p)), Some(Node::Dir { .. })) { return Err(String::from("its folder does not exist")); }
        let now = self.now;
        match self.nodes.get_mut(p) {
            Some(Node::Dir { .. }) => Err(String::from("is a folder")),
            Some(Node::File { data: d, modified, size }) => {
                if !append { d.clear(); }
                d.extend_from_slice(data);
                *modified = now;
                *size = d.len() as u64;
                Ok(())
            }
            None => { self.nodes.insert(p.to_string(), Node::File { data: data.to_vec(), modified: now, size: data.len() as u64 }); Ok(()) }
        }
    }
    fn make_dir(&mut self, p: &str) -> Result<(), String> {
        if self.nodes.contains_key(p) { return Err(String::from("already exists")); }
        if !matches!(self.nodes.get(parent(p)), Some(Node::Dir { .. })) { return Err(String::from("its folder does not exist")); }
        self.nodes.insert(p.to_string(), Node::Dir { modified: self.now });
        Ok(())
    }
    fn remove(&mut self, p: &str, recursive: bool) -> Result<(), String> {
        if p == "/" { return Err(String::from("can't remove the root")); }
        if !self.nodes.contains_key(p) { return Err(String::from("not found")); }
        let below = self.below(p);
        if !below.is_empty() && !recursive { return Err(String::from("folder is not empty")); }
        for k in below { self.nodes.remove(&k); }
        self.nodes.remove(p);
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        let node = self.nodes.remove(from).ok_or_else(|| String::from("not found"))?;
        if !matches!(self.nodes.get(parent(to)), Some(Node::Dir { .. })) {
            self.nodes.insert(from.to_string(), node);
            return Err(String::from("the destination folder does not exist"));
        }
        for k in self.below(from) {
            let n = self.nodes.remove(&k).unwrap();
            self.nodes.insert(format!("{}{}", to, &k[from.len()..]), n);
        }
        self.nodes.insert(to.to_string(), node);
        Ok(())
    }

    fn sys_info(&self) -> Value { self.sys.clone() }
    fn processes(&self) -> Vec<ProcessInfo> { self.processes.clone() }
    fn now(&self) -> i64 { self.now }
    fn is_external(&self, name: &str) -> bool { self.programs.contains_key(name) }
    fn external_names(&self) -> Vec<String> { self.programs.keys().cloned().collect() }

    fn spawn_job(&mut self, request: &ExternalRequest) -> Result<JobId, String> {
        if !self.programs.contains_key(&request.name) { return Err(String::from("no such program")); }
        let id = self.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
        let mut command = request.name.clone();
        for a in &request.args { command.push(' '); command.push_str(a); }
        self.jobs.push(JobInfo { id, command, state: JobState::Running });
        Ok(id)
    }
    fn jobs(&self) -> Vec<JobInfo> { self.jobs.clone() }
    fn job_foreground(&mut self, id: JobId) -> Result<(), String> {
        match self.jobs.iter().find(|j| j.id == id) {
            Some(j) if !matches!(j.state, JobState::Done(_)) => { self.foregrounded.push(id); Ok(()) }
            Some(_) => Err(String::from("it has already ended")),
            None => Err(String::from("no such job")),
        }
    }
    fn kill_job(&mut self, id: JobId) -> Result<(), String> {
        match self.jobs.iter_mut().find(|j| j.id == id) {
            Some(j) => { j.state = JobState::Done(-9); Ok(()) }
            None => Err(String::from("no such job")),
        }
    }
}
