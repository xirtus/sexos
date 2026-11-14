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
use spindle_shell::{DirEntry, EntryKind, Host, Value};

pub struct TermHost<'a> {
    pub cwd: &'a mut Vec<u8>,
}

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
    fn external_names(&self) -> Vec<String> {
        let mut names: Vec<String> = ["greet", "python", "lua", "rg", "fd", "stdhello", "chello"].iter().map(|s| String::from(*s)).collect();
        if let Ok(entries) = direct::list(self.cwd) {
            names.extend(entries.into_iter().filter_map(|e| String::from_utf8_lossy(&e.name).strip_suffix(".wasm").map(String::from)));
        }
        names
    }
}
