// SPDX-License-Identifier: GPL-3.0-only
//! Running other programs from C: `posix_spawn`, `posix_spawnp`, `waitpid`, `wait`,
//! `kill`, `system` and `getpid`, as a library inside the program.
//!
//! There is no fork, exec or process table. Each call becomes a call of the program's
//! Rust standard-library layer (crates/sex-std, `sys_process_*`), which asks the desktop
//! to start the program (`sex_app::launch`) with only what this run holds. So:
//!
//! * The program is `/bin/NAME`, a bare `NAME`, or `@/path` of a program file this run
//!   holds; `posix_spawnp` does no `PATH` search (programs live in one store). The
//!   started run is a hidden helper: no window, output in the serial record.
//! * `argv[1..]` become its arguments; `argv[0]` and `envp` are not passed on (a run has
//!   no environment). Spawn attributes (process groups, signal masks, scheduling) are
//!   accepted and ignored. File actions are not supported: `posix_spawn` with any
//!   returns ENOSYS, since there are no descriptors to hand on between runs.
//! * A pid is the child's task. `waitpid` takes one, or -1 (or 0) for any child, and
//!   `WNOHANG`. The status reads with the `<sys/wait.h>` macros: a run that called
//!   `exit(n)` is `WIFEXITED` with `WEXITSTATUS == n`; one that was ended (by `kill`) or
//!   crashed is `WIFSIGNALED` with `WTERMSIG == SIGKILL`.
//! * `kill(pid, SIGKILL | SIGTERM | SIGINT | SIGHUP | SIGQUIT)` ends a child this run
//!   started; there are no handlers, so every one ends it at once. Signal 0 asks whether
//!   it is still a child. Other signals are EINVAL; process groups ESRCH.
//! * `system(cmd)`: the first word of `cmd` is the program and the rest are its
//!   arguments, split like a shell splits plain words and quotes (no variables, globs,
//!   pipes or redirections: there is no shell). Returns the wait status; a program that
//!   could not start reads as `WEXITSTATUS == 127`, as from `sh`.
use crate::posix_util::{command_words, wait_status, SIGKILL, WNOHANG};
use crate::stdio::c_str;
use crate::{c_char, c_int, c_void, set_errno};
use alloc::vec::Vec;

extern "C" {
    // crates/sex-std/src/process.rs.
    fn sys_process_spawn(program: *const u8, program_len: usize, args: *const u8, args_len: usize, flags: u32, task: *mut u32) -> i64;
    fn sys_process_wait(id: i64, block: i32, status: *mut i32, task: *mut u32) -> i32;
    fn sys_process_kill(id: i64) -> i32;
    fn sys_process_forget(id: i64);
    fn sys_process_self() -> u32;
}

const ESRCH: c_int = 3;
const ECHILD: c_int = 10;

/// musl's `posix_spawn_file_actions_t`: an action added makes `actions` non-null.
#[repr(C)]
pub struct FileActions { pad0: [c_int; 2], actions: *mut c_void, pad: [c_int; 16] }
/// musl's `posix_spawnattr_t`.
#[repr(C)]
pub struct SpawnAttr { flags: c_int, pgrp: c_int, def: [u64; 16], mask: [u64; 16], prio: c_int, pol: c_int, func: *mut c_void, pad: [u8; 56] }

/// Starts `program` with `words`; the child's task, or an error number.
fn start(program: &[u8], words: &[&[u8]]) -> Result<u32, c_int> {
    let mut args = Vec::new();
    for w in words { args.extend_from_slice(w); args.push(0); }
    let mut task = 0u32;
    let r = unsafe { sys_process_spawn(program.as_ptr(), program.len(), args.as_ptr(), args.len(), 0, &mut task) };
    if r < 0 { Err(-r as c_int) } else { Ok(task) }
}

unsafe fn spawn(pid: *mut c_int, path: *const c_char, actions: *const FileActions, argv: *const *const c_char) -> c_int {
    if path.is_null() { return crate::EINVAL; }
    if !actions.is_null() && unsafe { !(*actions).actions.is_null() } { return crate::ENOSYS; }
    let program = unsafe { c_str(path) };
    let mut words: Vec<&[u8]> = Vec::new();
    if !argv.is_null() {
        let mut i = 1;
        // argv[0] names the program to itself; the run gets its own.
        while unsafe { !(*argv).is_null() && !(*argv.add(i)).is_null() } {
            words.push(unsafe { c_str(*argv.add(i)) });
            i += 1;
        }
    }
    match start(program, &words) {
        Ok(task) => { if !pid.is_null() { unsafe { *pid = task as c_int } } 0 }
        Err(e) => e,
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn(pid: *mut c_int, path: *const c_char, actions: *const FileActions, _attr: *const SpawnAttr,
                                     argv: *const *const c_char, _envp: *const *const c_char) -> c_int {
    unsafe { spawn(pid, path, actions, argv) }
}
/// As `posix_spawn`: a bare name is already looked up in the program store.
#[no_mangle]
pub unsafe extern "C" fn posix_spawnp(pid: *mut c_int, file: *const c_char, actions: *const FileActions, _attr: *const SpawnAttr,
                                      argv: *const *const c_char, _envp: *const *const c_char) -> c_int {
    unsafe { spawn(pid, file, actions, argv) }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_init(fa: *mut FileActions) -> c_int {
    unsafe { core::ptr::write_bytes(fa, 0, 1) };
    0
}
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_destroy(_fa: *mut FileActions) -> c_int { 0 }
/// Recorded only so `posix_spawn` can refuse it: runs share no descriptors.
unsafe fn add_action(fa: *mut FileActions) -> c_int {
    if fa.is_null() { return crate::EINVAL; }
    unsafe { (*fa).actions = fa.cast() };
    0
}
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_addopen(fa: *mut FileActions, _fd: c_int, _path: *const c_char, _flags: c_int, _mode: u32) -> c_int { unsafe { add_action(fa) } }
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_addclose(fa: *mut FileActions, _fd: c_int) -> c_int { unsafe { add_action(fa) } }
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_adddup2(fa: *mut FileActions, _fd: c_int, _to: c_int) -> c_int { unsafe { add_action(fa) } }

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_init(attr: *mut SpawnAttr) -> c_int {
    unsafe { core::ptr::write_bytes(attr, 0, 1) };
    0
}
#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_destroy(_attr: *mut SpawnAttr) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setflags(attr: *mut SpawnAttr, flags: i16) -> c_int {
    unsafe { (*attr).flags = flags as c_int };
    0
}
#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getflags(attr: *const SpawnAttr, flags: *mut i16) -> c_int {
    unsafe { *flags = (*attr).flags as i16 };
    0
}

#[no_mangle]
pub unsafe extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    let id = if pid <= 0 { -1 } else { pid as i64 };
    let (mut code, mut task) = (0i32, 0u32);
    let r = unsafe { sys_process_wait(id, (options & WNOHANG == 0) as i32, &mut code, &mut task) };
    let ended = match r {
        0 => return 0,
        1 => Some(code as u32),
        2 => None,
        _ => { set_errno(ECHILD); return -1; }
    };
    if !status.is_null() { unsafe { *status = wait_status(ended) } }
    // Reaped: a second wait for it is ECHILD, as for a Unix child.
    unsafe { sys_process_forget(task as i64) };
    task as c_int
}

#[no_mangle]
pub unsafe extern "C" fn wait(status: *mut c_int) -> c_int { unsafe { waitpid(-1, status, 0) } }

#[no_mangle]
pub unsafe extern "C" fn kill(pid: c_int, sig: c_int) -> c_int {
    if pid <= 0 { set_errno(ESRCH); return -1; }
    if pid as u32 == unsafe { sys_process_self() } && matches!(sig, 1 | 2 | 3 | SIGKILL | 15) { unsafe { crate::posix::raise(sig) }; }
    let r = match sig {
        0 => { let (mut c, mut t) = (0, 0); unsafe { sys_process_wait(pid as i64, 0, &mut c, &mut t) } }
        1 | 2 | 3 | SIGKILL | 15 => unsafe { sys_process_kill(pid as i64) },
        _ => { set_errno(crate::EINVAL); return -1; }
    };
    if r < 0 { set_errno(ESRCH); -1 } else { 0 }
}

/// This run's task: the same kind of id `posix_spawn` gives a child.
#[no_mangle]
pub unsafe extern "C" fn getpid() -> c_int { unsafe { sys_process_self() as c_int } }

/// Runs `command`'s first word as a program with the rest as its arguments, and waits.
/// `system(NULL)` says programs can be run (1).
#[no_mangle]
pub unsafe extern "C" fn system(command: *const c_char) -> c_int {
    if command.is_null() { return 1; }
    let words = command_words(unsafe { c_str(command) });
    let Some((program, rest)) = words.split_first() else { return wait_status(Some(127)) };
    let rest: Vec<&[u8]> = rest.iter().map(|w| w.as_slice()).collect();
    let task = match start(program, &rest) { Ok(task) => task, Err(_) => return wait_status(Some(127)) };
    let (mut code, mut t) = (0i32, 0u32);
    let r = unsafe { sys_process_wait(task as i64, 1, &mut code, &mut t) };
    unsafe { sys_process_forget(task as i64) };
    match r { 1 => wait_status(Some(code as u32)), _ => wait_status(None) }
}
