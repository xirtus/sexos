// SPDX-License-Identifier: GPL-3.0-only
//! `<string.h>` and `<strings.h>`. `memcpy`, `memmove`, `memset`, `memcmp`, `bcmp`
//! and `strlen` come from Rust's compiler builtins and are not repeated here.
use crate::{c_char, c_int, c_void};
use core::ptr::null_mut;

unsafe fn len(s: *const c_char) -> usize {
    let mut n = 0;
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strnlen(s: *const c_char, max: usize) -> usize {
    let mut n = 0;
    while n < max && unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    unsafe { strncmp(a, b, usize::MAX) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strncmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        let (x, y) = unsafe { (*a.add(i) as u8, *b.add(i) as u8) };
        if x != y || x == 0 {
            return x as c_int - y as c_int;
        }
    }
    0
}

fn lower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strcasecmp(a: *const c_char, b: *const c_char) -> c_int {
    unsafe { strncasecmp(a, b, usize::MAX) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strncasecmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        let (x, y) = unsafe { (lower(*a.add(i) as u8), lower(*b.add(i) as u8)) };
        if x != y || x == 0 {
            return x as c_int - y as c_int;
        }
    }
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let c = c as u8;
    let mut p = s;
    loop {
        let x = unsafe { *p } as u8;
        if x == c {
            return p as *mut c_char;
        }
        if x == 0 {
            return null_mut();
        }
        p = unsafe { p.add(1) };
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    let c = c as u8;
    let n = unsafe { len(s) };
    for i in (0..=n).rev() {
        if unsafe { *s.add(i) } as u8 == c {
            return unsafe { s.add(i) } as *mut c_char;
        }
    }
    null_mut()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void {
    let s = s.cast::<u8>();
    for i in 0..n {
        if unsafe { *s.add(i) } == c as u8 {
            return unsafe { s.add(i) } as *mut c_void;
        }
    }
    null_mut()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strcpy(d: *mut c_char, s: *const c_char) -> *mut c_char {
    let n = unsafe { len(s) };
    unsafe { core::ptr::copy_nonoverlapping(s, d, n + 1) };
    d
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strncpy(d: *mut c_char, s: *const c_char, n: usize) -> *mut c_char {
    let k = unsafe { strnlen(s, n) };
    unsafe {
        core::ptr::copy_nonoverlapping(s, d, k);
        core::ptr::write_bytes(d.add(k), 0, n - k);
    }
    d
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strcat(d: *mut c_char, s: *const c_char) -> *mut c_char {
    unsafe { strcpy(d.add(len(d)), s) };
    d
}

fn in_set(set: *const c_char, c: u8) -> bool {
    let mut p = set;
    loop {
        let x = unsafe { *p } as u8;
        if x == 0 {
            return false;
        }
        if x == c {
            return true;
        }
        p = unsafe { p.add(1) };
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strspn(s: *const c_char, accept: *const c_char) -> usize {
    let mut n = 0;
    while unsafe { *s.add(n) } != 0 && in_set(accept, unsafe { *s.add(n) } as u8) {
        n += 1;
    }
    n
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strcspn(s: *const c_char, reject: *const c_char) -> usize {
    let mut n = 0;
    while unsafe { *s.add(n) } != 0 && !in_set(reject, unsafe { *s.add(n) } as u8) {
        n += 1;
    }
    n
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strpbrk(s: *const c_char, accept: *const c_char) -> *mut c_char {
    let n = unsafe { strcspn(s, accept) };
    if unsafe { *s.add(n) } == 0 { null_mut() } else { unsafe { s.add(n) as *mut c_char } }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strstr(hay: *const c_char, needle: *const c_char) -> *mut c_char {
    let (h, n) = unsafe { (len(hay), len(needle)) };
    if n == 0 {
        return hay as *mut c_char;
    }
    let hb = unsafe { core::slice::from_raw_parts(hay.cast::<u8>(), h) };
    let nb = unsafe { core::slice::from_raw_parts(needle.cast::<u8>(), n) };
    match hb.windows(n).position(|w| w == nb) {
        Some(i) => unsafe { hay.add(i) as *mut c_char },
        None => null_mut(),
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strdup(s: *const c_char) -> *mut c_char {
    unsafe { strndup(s, usize::MAX) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strndup(s: *const c_char, max: usize) -> *mut c_char {
    let n = unsafe { strnlen(s, max) };
    let d = unsafe { crate::mem::malloc(n + 1) }.cast::<c_char>();
    if !d.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(s, d, n);
            *d.add(n) = 0;
        }
    }
    d
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtok_r(s: *mut c_char, delim: *const c_char, save: *mut *mut c_char) -> *mut c_char {
    let mut p = if s.is_null() { unsafe { *save } } else { s };
    if p.is_null() {
        return null_mut();
    }
    p = unsafe { p.add(strspn(p, delim)) };
    if unsafe { *p } == 0 {
        unsafe { *save = null_mut() };
        return null_mut();
    }
    let end = unsafe { p.add(strcspn(p, delim)) };
    unsafe {
        if *end == 0 {
            *save = null_mut();
        } else {
            *end = 0;
            *save = end.add(1);
        }
    }
    p
}

static UNKNOWN: &[u8] = b"error\0";

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strerror(_e: c_int) -> *mut c_char {
    UNKNOWN.as_ptr() as *mut c_char
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(s: &str) -> alloc::vec::Vec<c_char> {
        s.bytes().map(|b| b as c_char).chain(core::iter::once(0)).collect()
    }
    extern crate alloc;

    #[test]
    fn search_and_compare() {
        let hay = c("mp4 moov trak");
        unsafe {
            assert_eq!(strstr(hay.as_ptr(), c("moov").as_ptr()).offset_from(hay.as_ptr()), 4);
            assert!(strstr(hay.as_ptr(), c("mdat").as_ptr()).is_null());
            assert_eq!(strchr(hay.as_ptr(), b't' as c_int).offset_from(hay.as_ptr()), 9);
            assert_eq!(strrchr(hay.as_ptr(), b'm' as c_int).offset_from(hay.as_ptr()), 4);
            assert_eq!(strchr(hay.as_ptr(), 0).offset_from(hay.as_ptr()), 13);
            assert!(strcmp(c("abc").as_ptr(), c("abd").as_ptr()) < 0);
            assert_eq!(strncmp(c("abc").as_ptr(), c("abd").as_ptr(), 2), 0);
            assert_eq!(strcasecmp(c("H264").as_ptr(), c("h264").as_ptr()), 0);
            assert_eq!(strspn(c("  x").as_ptr(), c(" ").as_ptr()), 2);
            assert_eq!(strcspn(c("ab,c").as_ptr(), c(",").as_ptr()), 2);
            let mut buf = c("a,b,,c");
            let mut save = null_mut();
            let d = c(",");
            let mut parts = alloc::vec::Vec::new();
            let mut t = strtok_r(buf.as_mut_ptr(), d.as_ptr(), &mut save);
            while !t.is_null() {
                parts.push(len(t));
                t = strtok_r(null_mut(), d.as_ptr(), &mut save);
            }
            assert_eq!(parts, [1, 1, 1]);
        }
    }
}
