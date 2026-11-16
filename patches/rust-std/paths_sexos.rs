// SPDX-License-Identifier: MIT OR Apache-2.0
//! `std::env::current_dir` and `set_current_dir` for SexOS programs (x86_64-sex-std.json).
//!
//! Not compiled in the SexOS tree: scripts/sex_std_source.py puts it in place of std's
//! `sys/paths/hermit.rs` and has the Hermit arm take `chdir` from it. The working folder
//! lives in the program's library (`crates/sex-std`, path.rs): the program's own folder
//! (`/`), a folder in it, or a granted folder (`@/absolute/path`) that the program which
//! started this run handed over with `Command::current_dir`.

use crate::ffi::OsStr;
use crate::io;
use crate::os::hermit::ffi::OsStrExt;
use crate::path::{self, PathBuf};

// crates/sex-std/src/sys.rs.
unsafe extern "C" {
    /// The working folder and a NUL into `out[..len]`: its length, or `-ERANGE`.
    fn sys_sexos_getcwd(out: *mut u8, len: usize) -> isize;
    /// The working folder becomes `path[..len]`: 0, or `-errno`.
    fn sys_sexos_chdir(path: *const u8, len: usize) -> i32;
}

pub fn getcwd() -> io::Result<PathBuf> {
    let mut buf = vec![0u8; 1024];
    loop {
        let r = unsafe { sys_sexos_getcwd(buf.as_mut_ptr(), buf.len()) };
        if r >= 0 {
            buf.truncate(r as usize);
            return Ok(PathBuf::from(OsStr::from_bytes(&buf)));
        }
        if buf.len() >= 64 * 1024 {
            return Err(io::Error::from_raw_os_error(-r as i32));
        }
        buf.resize(buf.len() * 4, 0);
    }
}

pub fn chdir(p: &path::Path) -> io::Result<()> {
    let bytes = p.as_os_str().as_bytes();
    let r = unsafe { sys_sexos_chdir(bytes.as_ptr(), bytes.len()) };
    if r < 0 { Err(io::Error::from_raw_os_error(-r)) } else { Ok(()) }
}

pub fn temp_dir() -> PathBuf {
    PathBuf::from("/tmp")
}
