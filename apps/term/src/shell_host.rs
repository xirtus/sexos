// SPDX-License-Identifier: GPL-3.0-only
//! Spindle's shell (`spindle-shell`) on SexOS: its files are the terminal's folder through
//! Files, its programs are Wasm guests in the folder or programs of their own.
//!
//! The shell's paths are absolute (`/notes/a.txt`); the terminal's folder paths have no
//! leading slash (`notes/a.txt`).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use sex_app::files::{self, direct};
use spindle_shell::{DirEntry, EntryKind, ExternalRequest, Host, JobId, JobInfo, JobState, Value};
use sex_pdx::console_relay as c;

pub struct TermHost<'a> {
    pub cwd: &'a mut Vec<u8>,
    /// The pane's console relay slot (`console_relay::SLOTS` or more: none), through
    /// which a background job starts before it goes on as a yarn.
    pub slot: u8,
}

/// Jobs `kill` already asked to stop: asked again, Shell stops them.
static mut KILLED: Vec<JobId> = Vec::new();

fn rel(path: &str) -> &[u8] { path.trim_start_matches('/').as_bytes() }
fn why(e: files::Error) -> String { String::from_utf8_lossy(files::message(e)).into_owned() }

/// Every name made only of these is offered to Files as a program of its own; Files says
/// whether one exists when it starts.
pub fn program_like(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

impl Host for TermHost<'_> {
    fn cwd(&self) -> String { format!("/{}", String::from_utf8_lossy(self.cwd)) }
    fn set_cwd(&mut self, path: &str) -> Result<(), String> { *self.cwd = rel(path).to_vec(); Ok(()) }
    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        let entries = direct::list(rel(path)).map_err(why)?;
        Ok(entries.into_iter().map(|e| DirEntry {
            name: String::from_utf8_lossy(&e.name).into_owned(),
            kind: if e.dir { EntryKind::Dir } else { EntryKind::File },
            size: e.size,
            modified: None,
        }).collect())
    }
    fn stat(&self, path: &str) -> Result<DirEntry, String> {
        let r = rel(path);
        // The folder's root is always a folder.
        if r.is_empty() { return Ok(DirEntry { name: String::from("/"), kind: EntryKind::Dir, size: 0, modified: None }); }
        let st = direct::stat(r).map_err(why)?;
        let name = path.rsplit('/').next().unwrap_or(path);
        Ok(DirEntry { name: String::from(name), kind: if st.dir { EntryKind::Dir } else { EntryKind::File }, size: st.size, modified: (st.modified != 0).then_some(st.modified as i64) })
    }
    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let r = rel(path);
        let st = direct::stat(r).map_err(why)?;
        if st.dir { return Err(String::from("is a folder")); }
        let len = (st.size as usize).min(16 << 20);
        let mut data = vec![0u8; len];
        let mut at = 0;
        while at < len {
            let n = direct::read_at(r, at as u64, &mut data[at..]).map_err(why)?;
            if n == 0 { break; }
            at += n;
        }
        data.truncate(at);
        Ok(data)
    }
    fn write_file(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), String> {
        let r = rel(path);
        let at = match direct::stat(r) {
            Ok(st) if append => st.size,
            Ok(_) => { direct::truncate(r, 0).map_err(why)?; 0 }
            Err(_) => { direct::create(r).map_err(why)?; 0 }
        };
        let mut done = 0;
        while done < data.len() {
            let n = direct::write_at(r, at + done as u64, &data[done..]).map_err(why)?;
            if n == 0 { return Err(String::from("the disk took nothing")); }
            done += n;
        }
        Ok(())
    }
    fn make_dir(&mut self, path: &str) -> Result<(), String> { direct::mkdir(rel(path)).map_err(why) }
    fn remove(&mut self, path: &str, recursive: bool) -> Result<(), String> {
        let r = rel(path);
        if recursive {
            if let Ok(st) = direct::stat(r) {
                if st.dir {
                    for e in direct::list(r).map_err(why)? {
                        let child = format!("{}/{}", path.trim_end_matches('/'), String::from_utf8_lossy(&e.name));
                        self.remove(&child, true)?;
                    }
                }
            }
        }
        direct::remove(r).map_err(why)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> { direct::rename(rel(from), rel(to)).map_err(why) }
    fn sys_info(&self) -> Value {
        let mut r = spindle_shell::Record::new();
        r.insert("system", Value::String(String::from("SexOS")));
        r.insert("uptime", Value::Duration(sex_pdx::get_ticks() as i64 * 1_000_000));
        Value::Record(r)
    }
    fn is_external(&self, name: &str) -> bool {
        if name == "greet" { return true; }
        let wasm = format!("{}/{}.wasm", String::from_utf8_lossy(self.cwd), name.trim_end_matches(".wasm"));
        direct::stat(wasm.trim_start_matches('/').as_bytes()).is_ok() || program_like(name)
    }
    /// `cmd &`: the program starts in the pane's slot and at once goes on as a yarn (a
    /// run no window shows), whose name is the job's number. `fg` and `attach` bring it
    /// into a pane; closing a window leaves its programs as yarns the same way.
    fn spawn_job(&mut self, request: &ExternalRequest) -> Result<JobId, String> {
        let slot = self.slot as usize;
        if slot >= c::SLOTS { return Err(String::from("too many panes are running programs in this window")); }
        let args = request.args.join(" ");
        let label = if args.is_empty() { request.name.clone() } else { format!("{} {}", request.name, args) };
        // Input piped in (`a | b &`) waits in the run's input ring, then its end; the
        // ring holds it all or the job does not start.
        let input = request.input.as_deref();
        if input.is_some_and(|i| i.len() > c::INPUT_BYTES) { return Err(format!("a background job takes at most {} bytes of piped input", c::INPUT_BYTES)); }
        direct::relay::release_at(slot).map_err(why)?;
        direct::relay::start_at(slot, request.name.as_bytes(), args.as_bytes()).map_err(why)?;
        if let Some(input) = input {
            if direct::relay::put_at(slot, input).map_err(why)? < input.len() { return Err(String::from("its input did not fit")); }
            direct::relay::hang_up_at(slot).map_err(why)?;
        }
        let yarn = direct::relay::detach_at(slot, label.as_bytes(), b"").map_err(why)?;
        sex_pdx::serial_record!("[term.yarn.background] name={} yarn={}", request.name, yarn);
        Ok(yarn)
    }
    fn jobs(&self) -> Vec<JobInfo> {
        direct::relay::yarns().unwrap_or_default().into_iter().map(|y| JobInfo {
            id: y.name,
            command: String::from_utf8_lossy(y.label()).into_owned(),
            state: match (y.state, y.code) {
                (c::ENDED, c::NOT_STARTED) => JobState::Done(127),
                (c::ENDED, c::CRASHED) => JobState::Done(-1),
                (c::ENDED, code) => JobState::Done(code as i32),
                _ => JobState::Running,
            },
        }).collect()
    }
    fn job_foreground(&mut self, id: JobId) -> Result<(), String> {
        if (self.slot as usize) >= c::SLOTS { return Err(String::from("too many panes are running programs in this window")); }
        if self.jobs().iter().any(|j| j.id == id) { Ok(()) } else { Err(String::from("no such job")) }
    }
    /// `kill N`: Ctrl+C and the end of input for the job; a second `kill` of a job that
    /// runs on regardless has Shell stop it, as a second Ctrl+C does. An ended job is
    /// forgotten.
    fn kill_job(&mut self, id: JobId) -> Result<(), String> {
        let running = self.jobs().iter().any(|j| j.id == id && j.state == JobState::Running);
        let target = direct::relay::yarn_stop(id).map_err(|e| if e == files::NOT_FOUND { String::from("no such job") } else { why(e) })?;
        // SAFETY: one thread runs the shell.
        let asked = unsafe { &mut *core::ptr::addr_of_mut!(KILLED) };
        let again = asked.contains(&id);
        if running && !again { asked.push(id); }
        if running && again && target != 0 {
            let ok = direct::relay::stop(target).is_ok();
            sex_pdx::serial_record!("[term.yarn.kill] yarn={} target={} ok={}", id, target, ok as u8);
        }
        sex_pdx::serial_record!("[term.yarn.stop] yarn={} running={} again={}", id, running as u8, again as u8);
        Ok(())
    }
    fn external_names(&self) -> Vec<String> {
        let mut names: Vec<String> = ["greet", "python", "lua", "rg", "fd", "stdhello", "chello"].iter().map(|s| String::from(*s)).collect();
        if let Ok(entries) = direct::list(self.cwd) {
            names.extend(entries.into_iter().filter_map(|e| String::from_utf8_lossy(&e.name).strip_suffix(".wasm").map(String::from)));
        }
        names
    }
}
