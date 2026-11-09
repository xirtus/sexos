// SPDX-License-Identifier: GPL-3.0-only
//! `<time.h>`. Time comes from the program, which reads SexOS's clock: C code
//! gets the program's view of time and nothing more.
use crate::{c_char, c_int, c_long};

/// Nanoseconds since 1970-01-01 UTC, and since the program started.
static mut WALL: Option<fn() -> (i64, u64)> = None;

/// Sets the clock: returns (Unix time in nanoseconds, monotonic nanoseconds).
pub fn set_clock(clock: fn() -> (i64, u64)) {
    unsafe { WALL = Some(clock) };
}

/// Sets the clock only if the program set none.
pub fn set_clock_default(clock: fn() -> (i64, u64)) {
    unsafe { if (*core::ptr::addr_of!(WALL)).is_none() { WALL = Some(clock) } };
}

fn now() -> (i64, u64) {
    unsafe { WALL }.map_or((0, 0), |f| f())
}

#[repr(C)]
pub struct timespec {
    pub tv_sec: i64,
    pub tv_nsec: c_long,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct tm {
    pub tm_sec: c_int,
    pub tm_min: c_int,
    pub tm_hour: c_int,
    pub tm_mday: c_int,
    pub tm_mon: c_int,
    pub tm_year: c_int,
    pub tm_wday: c_int,
    pub tm_yday: c_int,
    pub tm_isdst: c_int,
    pub tm_gmtoff: c_long,
    pub tm_zone: *const c_char,
}


/// CLOCK_REALTIME is 0; every other clock is monotonic.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn clock_gettime(id: c_int, ts: *mut timespec) -> c_int {
    let (wall, mono) = now();
    let ns = if id == 0 { wall } else { mono as i64 };
    unsafe {
        (*ts).tv_sec = ns.div_euclid(1_000_000_000);
        (*ts).tv_nsec = ns.rem_euclid(1_000_000_000) as c_long;
    }
    0
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn time(out: *mut i64) -> i64 {
    let t = now().0.div_euclid(1_000_000_000);
    if !out.is_null() {
        unsafe { *out = t };
    }
    t
}

/// CLOCKS_PER_SEC is 1,000,000 in musl.
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn clock() -> c_long {
    (now().1 / 1000) as c_long
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil(t: i64) -> tm {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    let yday = days - days_from_civil(y, 1, 1);
    tm {
        tm_sec: (secs % 60) as c_int,
        tm_min: (secs / 60 % 60) as c_int,
        tm_hour: (secs / 3600) as c_int,
        tm_mday: d as c_int,
        tm_mon: (m - 1) as c_int,
        tm_year: (y - 1900) as c_int,
        tm_wday: (days + 4).rem_euclid(7) as c_int,
        tm_yday: yday as c_int,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: core::ptr::null(),
    }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn gmtime_r(t: *const i64, out: *mut tm) -> *mut tm {
    unsafe { *out = civil(*t) };
    out
}

/// SexOS keeps civil time in UTC; local time is UTC.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn localtime_r(t: *const i64, out: *mut tm) -> *mut tm {
    unsafe { gmtime_r(t, out) }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn mktime(t: *mut tm) -> i64 {
    let t = unsafe { &mut *t };
    let months = t.tm_year as i64 * 12 + t.tm_mon as i64;
    let (y, m) = (1900 + months.div_euclid(12), months.rem_euclid(12) + 1);
    let secs = days_from_civil(y, m, 1) * 86400 + (t.tm_mday as i64 - 1) * 86400 + t.tm_hour as i64 * 3600 + t.tm_min as i64 * 60 + t.tm_sec as i64;
    *t = civil(secs);
    secs
}

/// `strftime` for the numeric fields: `%Y %m %d %H %M %S %y %j %F %T %%`.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strftime(out: *mut c_char, cap: usize, fmt: *const c_char, t: *const tm) -> usize {
    let fmt = unsafe { core::slice::from_raw_parts(fmt.cast::<u8>(), crate::string::strnlen(fmt, usize::MAX)) };
    let t = unsafe { &*t };
    let mut buf = [0u8; 256];
    let mut n = 0;
    let put = |v: i64, width: usize, buf: &mut [u8; 256], n: &mut usize| {
        let mut d = [b'0'; 20];
        let mut k = 0;
        let mut x = v.unsigned_abs();
        while x > 0 || k < width {
            d[19 - k] = b'0' + (x % 10) as u8;
            x /= 10;
            k += 1;
        }
        for &c in &d[20 - k..] {
            if *n < buf.len() {
                buf[*n] = c;
                *n += 1;
            }
        }
    };
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' || i + 1 >= fmt.len() {
            if n < buf.len() {
                buf[n] = fmt[i];
                n += 1;
            }
            i += 1;
            continue;
        }
        let year = t.tm_year as i64 + 1900;
        match fmt[i + 1] {
            b'Y' => put(year, 4, &mut buf, &mut n),
            b'y' => put(year % 100, 2, &mut buf, &mut n),
            b'm' => put(t.tm_mon as i64 + 1, 2, &mut buf, &mut n),
            b'd' => put(t.tm_mday as i64, 2, &mut buf, &mut n),
            b'H' => put(t.tm_hour as i64, 2, &mut buf, &mut n),
            b'M' => put(t.tm_min as i64, 2, &mut buf, &mut n),
            b'S' => put(t.tm_sec as i64, 2, &mut buf, &mut n),
            b'j' => put(t.tm_yday as i64 + 1, 3, &mut buf, &mut n),
            b'F' => {
                put(year, 4, &mut buf, &mut n);
                buf[n] = b'-'; n += 1;
                put(t.tm_mon as i64 + 1, 2, &mut buf, &mut n);
                buf[n] = b'-'; n += 1;
                put(t.tm_mday as i64, 2, &mut buf, &mut n);
            }
            b'T' => {
                put(t.tm_hour as i64, 2, &mut buf, &mut n);
                buf[n] = b':'; n += 1;
                put(t.tm_min as i64, 2, &mut buf, &mut n);
                buf[n] = b':'; n += 1;
                put(t.tm_sec as i64, 2, &mut buf, &mut n);
            }
            b'%' => { buf[n] = b'%'; n += 1; }
            _ => {}
        }
        i += 2;
    }
    if n + 1 > cap {
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), out.cast::<u8>(), n);
        *out.add(n) = 0;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn civil_round_trip() {
        let t = civil(1_778_242_000); // 2026-05-08
        assert_eq!((t.tm_year + 1900, t.tm_mon + 1, t.tm_mday), (2026, 5, 8));
        let mut c = t;
        assert_eq!(unsafe { mktime(&mut c) }, 1_778_242_000);
        assert_eq!(civil(0).tm_wday, 4, "1970-01-01 was a Thursday");
        assert_eq!(civil(951_782_400).tm_mday, 29, "2000-02-29");
        let mut out = [0 as c_char; 32];
        let n = unsafe { strftime(out.as_mut_ptr(), 32, b"%F %T\0".as_ptr().cast(), &civil(86_399)) };
        let s: Vec<u8> = out[..n].iter().map(|&c| c as u8).collect();
        assert_eq!(s, b"1970-01-01 23:59:59");
    }
}
