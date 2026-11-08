// SPDX-License-Identifier: GPL-3.0-only
//! `<stdio.h>`: formatting into memory, and `stderr`/`stdout` into the program's
//! log. There are no files: `fopen` and `fdopen` refuse, so a C library that wants
//! input must take it through the callbacks its Rust host gives it.
use crate::format::{self, Args, CBuf, Sink};
use crate::{c_char, c_int, c_void, set_errno, ENOENT};
use core::ffi::VaList;
use core::ptr::null_mut;

/// An opaque C stream. Only the two log streams exist.
#[repr(C)]
pub struct FILE {
    which: u8,
}

static mut STDERR: FILE = FILE { which: 2 };
static mut STDOUT: FILE = FILE { which: 1 };

#[cfg_attr(target_os = "none", no_mangle)]
#[allow(non_upper_case_globals)]
pub static mut stderr: *mut FILE = core::ptr::addr_of_mut!(STDERR);
#[cfg_attr(target_os = "none", no_mangle)]
#[allow(non_upper_case_globals)]
pub static mut stdout: *mut FILE = core::ptr::addr_of_mut!(STDOUT);

struct Va<'a, 'f>(&'a mut VaList<'f>);

impl Args for Va<'_, '_> {
    fn int(&mut self) -> i64 {
        // Every integer argument is passed in a 64-bit slot on x86-64; the
        // engine narrows it by the length modifier.
        unsafe { self.0.next_arg::<i64>() }
    }
    fn uint(&mut self) -> u64 {
        unsafe { self.0.next_arg::<u64>() }
    }
    fn float(&mut self) -> f64 {
        unsafe { self.0.next_arg::<f64>() }
    }
    fn ptr(&mut self) -> *const u8 {
        unsafe { self.0.next_arg::<*const u8>() }
    }
}

unsafe fn c_str<'a>(s: *const c_char) -> &'a [u8] {
    let n = unsafe { crate::string::strnlen(s, usize::MAX) };
    unsafe { core::slice::from_raw_parts(s.cast(), n) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn vsnprintf(buf: *mut c_char, cap: usize, fmt: *const c_char, mut ap: VaList) -> c_int {
    let mut sink = CBuf { ptr: buf.cast(), cap, n: 0 };
    let n = format::format(unsafe { c_str(fmt) }, &mut Va(&mut ap), &mut sink);
    sink.finish();
    n.min(c_int::MAX as usize) as c_int
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn snprintf(buf: *mut c_char, cap: usize, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { vsnprintf(buf, cap, fmt, ap) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn vsprintf(buf: *mut c_char, fmt: *const c_char, ap: VaList) -> c_int {
    unsafe { vsnprintf(buf, usize::MAX, fmt, ap) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn sprintf(buf: *mut c_char, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { vsnprintf(buf, usize::MAX, fmt, ap) }
}

/// Collects a log line and hands complete lines to the program's log.
struct Line {
    buf: [u8; 512],
    n: usize,
}

impl Sink for Line {
    fn put(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\n' || self.n == self.buf.len() {
                crate::log(&self.buf[..self.n]);
                self.n = 0;
                if b == b'\n' {
                    continue;
                }
            }
            self.buf[self.n] = b;
            self.n += 1;
        }
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn vfprintf(stream: *mut FILE, fmt: *const c_char, mut ap: VaList) -> c_int {
    if stream.is_null() {
        return -1;
    }
    let mut line = Line { buf: [0; 512], n: 0 };
    let n = format::format(unsafe { c_str(fmt) }, &mut Va(&mut ap), &mut line);
    if line.n > 0 {
        crate::log(&line.buf[..line.n]);
    }
    n as c_int
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fprintf(stream: *mut FILE, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { vfprintf(stream, fmt, ap) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn vprintf(fmt: *const c_char, ap: VaList) -> c_int {
    unsafe { vfprintf(stdout, fmt, ap) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn printf(fmt: *const c_char, ap: ...) -> c_int {
    unsafe { vfprintf(stdout, fmt, ap) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fputs(s: *const c_char, _stream: *mut FILE) -> c_int {
    let mut line = Line { buf: [0; 512], n: 0 };
    line.put(unsafe { c_str(s) });
    if line.n > 0 {
        crate::log(&line.buf[..line.n]);
    }
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    unsafe { fputs(s, stdout) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fputc(c: c_int, _stream: *mut FILE) -> c_int {
    if c as u8 != b'\n' {
        crate::log(&[c as u8]);
    }
    c
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fwrite(p: *const c_void, size: usize, count: usize, _stream: *mut FILE) -> usize {
    let n = size.saturating_mul(count);
    crate::log(unsafe { core::slice::from_raw_parts(p.cast::<u8>(), n) });
    count
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fflush(_stream: *mut FILE) -> c_int {
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn setvbuf(_stream: *mut FILE, _buf: *mut c_char, _mode: c_int, _size: usize) -> c_int {
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fopen(_path: *const c_char, _mode: *const c_char) -> *mut FILE {
    set_errno(ENOENT);
    null_mut()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fdopen(_fd: c_int, _mode: *const c_char) -> *mut FILE {
    set_errno(crate::EBADF);
    null_mut()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fclose(_stream: *mut FILE) -> c_int {
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn fread(_p: *mut c_void, _size: usize, _count: usize, _stream: *mut FILE) -> usize {
    0
}

/// `sscanf` for the few numeric patterns C libraries parse with it: `%d %i %u %x
/// %ld %lld %lu %llu %f %lf %s %c %n` and literal text. Returns fields assigned.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn vsscanf(input: *const c_char, fmt: *const c_char, mut ap: VaList) -> c_int {
    let s = unsafe { c_str(input) };
    let f = unsafe { c_str(fmt) };
    let mut out = Out(&mut ap);
    scan(s, f, &mut out)
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn sscanf(input: *const c_char, fmt: *const c_char, ap: ...) -> c_int {
    unsafe { vsscanf(input, fmt, ap) }
}

/// Where `scan` stores what it read.
pub trait Store {
    fn int(&mut self, v: i64, long: u8);
    fn float(&mut self, v: f64, long: bool);
    fn bytes(&mut self, b: &[u8], terminate: bool);
    fn count(&mut self, n: usize);
}

struct Out<'a, 'f>(&'a mut VaList<'f>);

impl Store for Out<'_, '_> {
    fn int(&mut self, v: i64, long: u8) {
        let p = unsafe { self.0.next_arg::<*mut c_void>() };
        unsafe {
            match long {
                0 => *p.cast::<i32>() = v as i32,
                1 => *p.cast::<i16>() = v as i16,
                2 => *p.cast::<i8>() = v as i8,
                _ => *p.cast::<i64>() = v,
            }
        }
    }
    fn float(&mut self, v: f64, long: bool) {
        let p = unsafe { self.0.next_arg::<*mut c_void>() };
        unsafe {
            if long { *p.cast::<f64>() = v } else { *p.cast::<f32>() = v as f32 }
        }
    }
    fn bytes(&mut self, b: &[u8], terminate: bool) {
        let p = unsafe { self.0.next_arg::<*mut u8>() };
        unsafe {
            core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
            if terminate {
                *p.add(b.len()) = 0;
            }
        }
    }
    fn count(&mut self, n: usize) {
        let p = unsafe { self.0.next_arg::<*mut c_int>() };
        unsafe { *p = n as c_int };
    }
}

pub fn scan(s: &[u8], f: &[u8], out: &mut dyn Store) -> c_int {
    let (mut i, mut j, mut assigned) = (0usize, 0usize, 0 as c_int);
    while j < f.len() {
        let c = f[j];
        if c.is_ascii_whitespace() {
            while i < s.len() && s[i].is_ascii_whitespace() {
                i += 1;
            }
            j += 1;
            continue;
        }
        if c != b'%' || f.get(j + 1) == Some(&b'%') {
            let want = if c == b'%' { j += 1; b'%' } else { c };
            if s.get(i) != Some(&want) {
                break;
            }
            i += 1;
            j += 1;
            continue;
        }
        j += 1;
        let skip = f.get(j) == Some(&b'*');
        if skip {
            j += 1;
        }
        let mut width = 0usize;
        while j < f.len() && f[j].is_ascii_digit() {
            width = width * 10 + (f[j] - b'0') as usize;
            j += 1;
        }
        let mut long = 0u8;
        let mut size = 0u8; // 0 int, 1 short, 2 char, 3 long
        while j < f.len() && matches!(f[j], b'l' | b'h' | b'z' | b'j' | b'L' | b'q') {
            match f[j] {
                b'h' => size = if size == 1 { 2 } else { 1 },
                _ => {
                    long += 1;
                    size = 3
                }
            }
            j += 1;
        }
        let Some(&conv) = f.get(j) else { break };
        j += 1;
        let limit = if width == 0 { usize::MAX } else { width };
        if conv != b'c' && conv != b'n' {
            while i < s.len() && s[i].is_ascii_whitespace() {
                i += 1;
            }
        }
        if conv != b'n' && i >= s.len() {
            return if assigned == 0 { -1 } else { assigned };
        }
        match conv {
            b'n' => {
                if !skip {
                    out.count(i)
                }
            }
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' => {
                let base = match conv { b'x' | b'X' => 16, b'o' => 8, b'i' => 0, _ => 10 };
                let end = i.saturating_add(limit).min(s.len());
                let (v, used) = crate::stdlib::parse_int(&s[i..end], base);
                if used == 0 {
                    break;
                }
                i += used;
                if !skip {
                    out.int(v as i64, size);
                    assigned += 1;
                }
            }
            b'f' | b'e' | b'g' | b'E' | b'G' | b'a' => {
                let end = i.saturating_add(limit).min(s.len());
                let (v, used) = crate::stdlib::parse_float(&s[i..end]);
                if used == 0 {
                    break;
                }
                i += used;
                if !skip {
                    out.float(v, long > 0);
                    assigned += 1;
                }
            }
            b's' => {
                let start = i;
                while i < s.len() && i - start < limit && !s[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i == start {
                    break;
                }
                if !skip {
                    out.bytes(&s[start..i], true);
                    assigned += 1;
                }
            }
            b'c' => {
                let n = if width == 0 { 1 } else { width };
                if i + n > s.len() {
                    break;
                }
                if !skip {
                    out.bytes(&s[i..i + n], false);
                    assigned += 1;
                }
                i += n;
            }
            _ => break,
        }
    }
    assigned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Got {
        ints: Vec<i64>,
        floats: Vec<f64>,
        strs: Vec<Vec<u8>>,
        n: Vec<usize>,
    }
    impl Store for Got {
        fn int(&mut self, v: i64, _: u8) {
            self.ints.push(v)
        }
        fn float(&mut self, v: f64, _: bool) {
            self.floats.push(v)
        }
        fn bytes(&mut self, b: &[u8], _: bool) {
            self.strs.push(b.to_vec())
        }
        fn count(&mut self, n: usize) {
            self.n.push(n)
        }
    }

    #[test]
    fn scans_numbers_text_and_stops_on_mismatch() {
        let mut g = Got::default();
        assert_eq!(scan(b"1920x1080", b"%dx%d", &mut g), 2);
        assert_eq!(g.ints, [1920, 1080]);
        let mut g = Got::default();
        assert_eq!(scan(b"12:34:56.5", b"%d:%d:%lf%n", &mut g), 3);
        assert_eq!((g.ints.as_slice(), g.floats.as_slice(), g.n.as_slice()), (&[12, 34][..], &[56.5][..], &[10][..]));
        let mut g = Got::default();
        assert_eq!(scan(b"  0x1f rest", b"%i %s", &mut g), 2);
        assert_eq!((g.ints[0], g.strs[0].as_slice()), (31, &b"rest"[..]));
        let mut g = Got::default();
        assert_eq!(scan(b"abc", b"%d", &mut g), 0);
        assert_eq!(scan(b"", b"%d", &mut g), -1);
        let mut g = Got::default();
        assert_eq!(scan(b"7 %", b"%d %%", &mut g), 1);
    }
}
