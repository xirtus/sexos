// SPDX-License-Identifier: GPL-3.0-only
//! `<stdlib.h>` number parsing, sorting and searching.
use crate::{c_char, c_int, c_long, c_void, set_errno, ERANGE};

/// Parses an optionally signed integer with C's base rules (base 0: `0x`, `0`
/// prefixes). Returns the value (wrapped to i128 range) and bytes used; 0 bytes
/// used means no number.
pub fn parse_int(s: &[u8], base: u32) -> (i128, usize) {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = base;
    let hex_prefix = i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x';
    if (base == 0 || base == 16) && hex_prefix && s.get(i + 2).is_some_and(|c| (*c as char).is_digit(16)) {
        base = 16;
        i += 2;
    } else if base == 0 {
        base = if s.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let start = i;
    let mut v: i128 = 0;
    while i < s.len() {
        let Some(d) = (s[i] as char).to_digit(base) else { break };
        v = v.saturating_mul(base as i128).saturating_add(d as i128);
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    (if neg { -v } else { v }, i)
}

/// Parses a decimal or hexadecimal float, `inf` or `nan`. Returns value and bytes used.
pub fn parse_float(s: &[u8]) -> (f64, usize) {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let lead = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let neg = s.get(lead) == Some(&b'-');
    let rest = &s[i..];
    let word = |w: &[u8]| rest.len() >= w.len() && rest[..w.len()].eq_ignore_ascii_case(w);
    if word(b"infinity") {
        return (if neg { f64::NEG_INFINITY } else { f64::INFINITY }, i + 8);
    }
    if word(b"inf") {
        return (if neg { f64::NEG_INFINITY } else { f64::INFINITY }, i + 3);
    }
    if word(b"nan") {
        return (f64::NAN, i + 3);
    }
    let digits_start = i;
    let mut seen_digit = false;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
        seen_digit = true;
    }
    if i < s.len() && s[i] == b'.' {
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
            seen_digit = true;
        }
    }
    if !seen_digit {
        return (0.0, 0);
    }
    if i < s.len() && (s[i] | 0x20) == b'e' {
        let mut k = i + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            i = k;
        }
    }
    let text = core::str::from_utf8(&s[digits_start..i]).unwrap_or("0");
    let v: f64 = text.parse().unwrap_or(0.0);
    (if neg { -v } else { v }, i)
}

unsafe fn bytes<'a>(s: *const c_char) -> &'a [u8] {
    unsafe { core::slice::from_raw_parts(s.cast(), crate::string::strnlen(s, usize::MAX)) }
}

unsafe fn end(s: *const c_char, used: usize, endp: *mut *mut c_char) {
    if !endp.is_null() {
        unsafe { *endp = s.add(used) as *mut c_char };
    }
}

fn clamp(v: i128, lo: i128, hi: i128) -> i128 {
    if v < lo || v > hi {
        set_errno(ERANGE);
    }
    v.clamp(lo, hi)
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtoll(s: *const c_char, endp: *mut *mut c_char, base: c_int) -> i64 {
    let (v, used) = parse_int(unsafe { bytes(s) }, base as u32);
    unsafe { end(s, used, endp) };
    clamp(v, i64::MIN as i128, i64::MAX as i128) as i64
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtol(s: *const c_char, endp: *mut *mut c_char, base: c_int) -> c_long {
    unsafe { strtoll(s, endp, base) as c_long }
}

/// C's unsigned parse: a leading minus negates in the unsigned type.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtoull(s: *const c_char, endp: *mut *mut c_char, base: c_int) -> u64 {
    let (v, used) = parse_int(unsafe { bytes(s) }, base as u32);
    unsafe { end(s, used, endp) };
    if v > u64::MAX as i128 || v < -(u64::MAX as i128) {
        set_errno(ERANGE);
        return u64::MAX;
    }
    if v < 0 { (v.unsigned_abs() as u64).wrapping_neg() } else { v as u64 }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtoul(s: *const c_char, endp: *mut *mut c_char, base: c_int) -> u64 {
    unsafe { strtoull(s, endp, base) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtod(s: *const c_char, endp: *mut *mut c_char) -> f64 {
    let (v, used) = parse_float(unsafe { bytes(s) });
    unsafe { end(s, used, endp) };
    v
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtof(s: *const c_char, endp: *mut *mut c_char) -> f32 {
    unsafe { strtod(s, endp) as f32 }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn atoi(s: *const c_char) -> c_int {
    unsafe { strtol(s, core::ptr::null_mut(), 10) as c_int }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn atol(s: *const c_char) -> c_long {
    unsafe { strtol(s, core::ptr::null_mut(), 10) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn atof(s: *const c_char) -> f64 {
    unsafe { strtod(s, core::ptr::null_mut()) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn abs(v: c_int) -> c_int {
    v.wrapping_abs()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn labs(v: c_long) -> c_long {
    v.wrapping_abs()
}

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn llabs(v: i64) -> i64 {
    v.wrapping_abs()
}

type Compare = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

unsafe fn swap(a: *mut u8, b: *mut u8, size: usize) {
    unsafe { core::ptr::swap_nonoverlapping(a, b, size) };
}

/// Heapsort: no recursion, no allocation, O(n log n) whatever the input.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn qsort(base: *mut c_void, count: usize, size: usize, cmp: Compare) {
    if count < 2 || size == 0 {
        return;
    }
    let base = base.cast::<u8>();
    let at = |i: usize| unsafe { base.add(i * size) };
    let sift = |mut root: usize, end: usize| loop {
        let mut child = 2 * root + 1;
        if child >= end {
            break;
        }
        if child + 1 < end && unsafe { cmp(at(child).cast(), at(child + 1).cast()) } < 0 {
            child += 1;
        }
        if unsafe { cmp(at(root).cast(), at(child).cast()) } >= 0 {
            break;
        }
        unsafe { swap(at(root), at(child), size) };
        root = child;
    };
    for start in (0..count / 2).rev() {
        sift(start, count);
    }
    for end in (1..count).rev() {
        unsafe { swap(at(0), at(end), size) };
        sift(0, end);
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn bsearch(key: *const c_void, base: *const c_void, count: usize, size: usize, cmp: Compare) -> *mut c_void {
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let p = unsafe { base.cast::<u8>().add(mid * size) };
        match unsafe { cmp(key, p.cast()) } {
            0 => return p as *mut c_void,
            c if c < 0 => hi = mid,
            _ => lo = mid + 1,
        }
    }
    core::ptr::null_mut()
}

static mut SEED: u64 = 1;

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn srand(seed: core::ffi::c_uint) {
    unsafe { SEED = seed as u64 };
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn rand() -> c_int {
    unsafe {
        SEED = SEED.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((SEED >> 33) & 0x7fff_ffff) as c_int
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_like_c() {
        assert_eq!(parse_int(b"  -123abc", 10), (-123, 6));
        assert_eq!(parse_int(b"0x1F", 0), (31, 4));
        assert_eq!(parse_int(b"0x", 0), (0, 1), "0x without digits is the number 0");
        assert_eq!(parse_int(b"017", 0), (15, 3));
        assert_eq!(parse_int(b"ff", 16), (255, 2));
        assert_eq!(parse_int(b"x", 10), (0, 0));
        let s = b"99999999999999999999999\0";
        let mut e = core::ptr::null_mut();
        unsafe {
            assert_eq!(strtoll(s.as_ptr().cast(), &mut e, 10), i64::MAX);
            assert_eq!(e.offset_from(s.as_ptr().cast()), 23);
            assert_eq!(strtoull(b"-1\0".as_ptr().cast(), core::ptr::null_mut(), 10), u64::MAX);
            assert_eq!(atoi(b" 42 \0".as_ptr().cast()), 42);
        }
    }

    #[test]
    fn floats_like_c() {
        assert_eq!(parse_float(b"3.5e2x"), (350.0, 5));
        assert_eq!(parse_float(b"-.25"), (-0.25, 4));
        assert_eq!(parse_float(b"1e"), (1.0, 1));
        assert_eq!(parse_float(b"inf"), (f64::INFINITY, 3));
        assert!(parse_float(b"NaN").0.is_nan());
        assert_eq!(parse_float(b"."), (0.0, 0));
        assert_eq!(parse_float(b"29.97"), (29.97, 5));
    }

    unsafe extern "C" fn by_int(a: *const c_void, b: *const c_void) -> c_int {
        unsafe { (*a.cast::<i32>()).cmp(&*b.cast::<i32>()) as c_int }
    }

    #[test]
    fn sort_and_search() {
        let mut v: Vec<i32> = (0..1000).map(|i| (i * 7919) % 1009 - 500).collect();
        let mut want = v.clone();
        want.sort();
        unsafe {
            qsort(v.as_mut_ptr().cast(), v.len(), 4, by_int);
            assert_eq!(v, want);
            let key = want[321];
            let p = bsearch((&key as *const i32).cast(), v.as_ptr().cast(), v.len(), 4, by_int);
            assert_eq!(*p.cast::<i32>(), key);
            let missing = 100_000;
            assert!(bsearch((&missing as *const i32).cast(), v.as_ptr().cast(), v.len(), 4, by_int).is_null());
        }
    }
}
