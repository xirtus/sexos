// SPDX-License-Identifier: GPL-3.0-only
//! Durable replacement of Kaleidoscope's small Files state files.
//!
//! Each rename is a Files commit. A stopped run can leave the old live file,
//! the previous copy, or the new live file; an unfinished `.next` is never read.

use alloc::{format, string::String, vec};
use sex_app::{files, Window};

pub fn read(window: &mut Window, path: &str, max: u64) -> Option<String> {
    read_one(window, path.as_bytes(), max)
        .or_else(|| read_one(window, format!("{path}.previous").as_bytes(), max))
}

fn read_one(window: &mut Window, path: &[u8], max: u64) -> Option<String> {
    let size = files::size(window, path).ok()?;
    if size > max {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let n = files::read(window, path, &mut buf).ok()?;
    if n != size as usize {
        return None;
    }
    String::from_utf8(buf).ok()
}

pub fn replace(window: &mut Window, path: &str, data: &[u8]) -> bool {
    let next = format!("{path}.next");
    let previous = format!("{path}.previous");
    // A former interrupted write may be partial. Never promote it.
    let _ = files::remove(window, next.as_bytes());
    if files::write(window, next.as_bytes(), data).is_err() {
        return false;
    }
    if files::sync(window).is_err() {
        return false;
    }
    let had_live = files::size(window, path.as_bytes()).is_ok();
    if had_live && files::rename(window, path.as_bytes(), previous.as_bytes()).is_err() {
        return false;
    }
    if files::rename(window, next.as_bytes(), path.as_bytes()).is_err() {
        if had_live {
            let _ = files::rename(window, previous.as_bytes(), path.as_bytes());
        }
        return false;
    }
    files::sync(window).is_ok()
}
