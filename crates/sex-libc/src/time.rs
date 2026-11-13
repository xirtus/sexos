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
    #[cfg(target_os = "hermit")]
    if unsafe { WALL }.is_none() { return crate::posix::clock(); }
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
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn clock_gettime(id: c_int, ts: *mut timespec) -> c_int {
    let (wall, mono) = now();
    let ns = if id == 0 { wall } else { mono as i64 };
    unsafe {
        (*ts).tv_sec = ns.div_euclid(1_000_000_000);
        (*ts).tv_nsec = ns.rem_euclid(1_000_000_000) as c_long;
    }
    0
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn time(out: *mut i64) -> i64 {
    let t = now().0.div_euclid(1_000_000_000);
    if !out.is_null() {
        unsafe { *out = t };
    }
    t
}

/// Every clock counts from the system tick: one millisecond.
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn clock_getres(_id: c_int, ts: *mut timespec) -> c_int {
    if !ts.is_null() { unsafe { (*ts).tv_sec = 0; (*ts).tv_nsec = 1_000_000 }; }
    0
}

/// `<sys/times.h>`: a program's time is its running time (there are no child
/// processes, and user and system time are not told apart), in `sysconf(_SC_CLK_TCK)`
/// ticks of 10 ms.
#[repr(C)]
pub struct tms { pub tms_utime: c_long, pub tms_stime: c_long, pub tms_cutime: c_long, pub tms_cstime: c_long }
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn times(out: *mut tms) -> c_long {
    let ticks = (now().1 / 10_000_000) as c_long;
    if !out.is_null() { unsafe { *out = tms { tms_utime: ticks, tms_stime: 0, tms_cutime: 0, tms_cstime: 0 } }; }
    ticks
}

/// CLOCKS_PER_SEC is 1,000,000 in musl.
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
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

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn gmtime_r(t: *const i64, out: *mut tm) -> *mut tm {
    unsafe { *out = civil(*t) };
    out
}

/// SexOS keeps civil time in UTC; local time is UTC.
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn localtime_r(t: *const i64, out: *mut tm) -> *mut tm {
    unsafe { gmtime_r(t, out) }
}

/// Local time is UTC, so `timegm` is `mktime`.
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn timegm(t: *mut tm) -> i64 { unsafe { mktime(t) } }

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn mktime(t: *mut tm) -> i64 {
    let t = unsafe { &mut *t };
    let months = t.tm_year as i64 * 12 + t.tm_mon as i64;
    let (y, m) = (1900 + months.div_euclid(12), months.rem_euclid(12) + 1);
    let secs = days_from_civil(y, m, 1) * 86400 + (t.tm_mday as i64 - 1) * 86400 + t.tm_hour as i64 * 3600 + t.tm_min as i64 * 60 + t.tm_sec as i64;
    *t = civil(secs);
    secs
}

const DAYS: [&[u8]; 7] = [b"Sunday", b"Monday", b"Tuesday", b"Wednesday", b"Thursday", b"Friday", b"Saturday"];
const MONTHS: [&[u8]; 12] = [b"January", b"February", b"March", b"April", b"May", b"June", b"July", b"August", b"September", b"October", b"November", b"December"];

/// strftime's output so far, cut off at its capacity.
struct Out { buf: [u8; 256], n: usize }
impl Out {
    fn bytes(&mut self, b: &[u8]) { for &c in b { if self.n < self.buf.len() { self.buf[self.n] = c; self.n += 1; } } }
    /// A number, padded with `pad` to `width` places.
    fn num(&mut self, v: i64, width: usize, pad: u8) {
        if v < 0 { self.bytes(b"-"); }
        let mut d = [pad; 20];
        let (mut x, mut k) = (v.unsigned_abs(), 0);
        while x > 0 || k == 0 { d[19 - k] = b'0' + (x % 10) as u8; x /= 10; k += 1; }
        let k = k.max(width).min(20);
        self.bytes(&d[20 - k..]);
    }
}

/// Week of the year with weeks starting on `first` (0 Sunday, 1 Monday), as %U/%W.
fn week(t: &tm, first: i64) -> i64 { (t.tm_yday as i64 + 7 - (t.tm_wday as i64 - first).rem_euclid(7)) / 7 }

fn format(o: &mut Out, fmt: &[u8], t: &tm) {
    let year = t.tm_year as i64 + 1900;
    let wday = (t.tm_wday as usize).min(6);
    let mon = (t.tm_mon as usize).min(11);
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' || i + 1 >= fmt.len() { o.bytes(&fmt[i..i + 1]); i += 1; continue; }
        // C99's E and O modifiers name the same fields in the C locale.
        let mut c = fmt[i + 1];
        if (c == b'E' || c == b'O') && i + 2 < fmt.len() { i += 1; c = fmt[i + 1]; }
        match c {
            b'Y' | b'G' => o.num(year, 4, b'0'),
            b'y' | b'g' => o.num(year.rem_euclid(100), 2, b'0'),
            b'C' => o.num(year.div_euclid(100), 2, b'0'),
            b'm' => o.num(t.tm_mon as i64 + 1, 2, b'0'),
            b'd' => o.num(t.tm_mday as i64, 2, b'0'),
            b'e' => o.num(t.tm_mday as i64, 2, b' '),
            b'H' => o.num(t.tm_hour as i64, 2, b'0'),
            b'I' => o.num((t.tm_hour as i64 + 11) % 12 + 1, 2, b'0'),
            b'M' => o.num(t.tm_min as i64, 2, b'0'),
            b'S' => o.num(t.tm_sec as i64, 2, b'0'),
            b'j' => o.num(t.tm_yday as i64 + 1, 3, b'0'),
            b'u' => o.num(if t.tm_wday == 0 { 7 } else { t.tm_wday as i64 }, 1, b'0'),
            b'w' => o.num(t.tm_wday as i64, 1, b'0'),
            b'U' => o.num(week(t, 0), 2, b'0'),
            b'W' | b'V' => o.num(week(t, 1), 2, b'0'),
            b'a' => o.bytes(&DAYS[wday][..3]),
            b'A' => o.bytes(DAYS[wday]),
            b'b' | b'h' => o.bytes(&MONTHS[mon][..3]),
            b'B' => o.bytes(MONTHS[mon]),
            b'p' => o.bytes(if t.tm_hour < 12 { b"AM" } else { b"PM" }),
            b'c' => format(o, b"%a %b %e %H:%M:%S %Y", t),
            b'x' | b'D' => format(o, b"%m/%d/%y", t),
            b'X' | b'T' => format(o, b"%H:%M:%S", t),
            b'r' => format(o, b"%I:%M:%S %p", t),
            b'R' => format(o, b"%H:%M", t),
            b'F' => format(o, b"%Y-%m-%d", t),
            b'z' => o.bytes(b"+0000"),
            b'Z' => o.bytes(b"UTC"),
            b'n' => o.bytes(b"\n"),
            b't' => o.bytes(b"\t"),
            b'%' => o.bytes(b"%"),
            _ => {}
        }
        i += 2;
    }
}

/// `strftime` in the C locale, in UTC: every C99 conversion.
#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn strftime(out: *mut c_char, cap: usize, fmt: *const c_char, t: *const tm) -> usize {
    let fmt = unsafe { core::slice::from_raw_parts(fmt.cast::<u8>(), crate::string::strnlen(fmt, usize::MAX)) };
    let mut o = Out { buf: [0; 256], n: 0 };
    format(&mut o, fmt, unsafe { &*t });
    let n = o.n;
    if n + 1 > cap {
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(o.buf.as_ptr(), out.cast::<u8>(), n);
        *out.add(n) = 0;
    }
    n
}

/// One `struct tm` that `gmtime` and `localtime` share, as C says.
static mut SHARED_TM: tm = tm { tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 0, tm_mon: 0, tm_year: 0, tm_wday: 0, tm_yday: 0, tm_isdst: 0, tm_gmtoff: 0, tm_zone: core::ptr::null() };

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn gmtime(t: *const i64) -> *mut tm {
    unsafe { gmtime_r(t, core::ptr::addr_of_mut!(SHARED_TM)) }
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub unsafe extern "C" fn localtime(t: *const i64) -> *mut tm {
    unsafe { localtime_r(t, core::ptr::addr_of_mut!(SHARED_TM)) }
}

#[cfg_attr(any(target_os = "none", target_os = "hermit"), no_mangle)]
pub extern "C" fn difftime(end: i64, start: i64) -> f64 { (end - start) as f64 }

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
        let n = unsafe { strftime(out.as_mut_ptr(), 32, b"%c|%p|%j\0".as_ptr().cast(), &civil(86_399)) };
        let s: Vec<u8> = out[..n].iter().map(|&c| c as u8).collect();
        assert_eq!(s, b"Thu Jan  1 23:59:59 1970|PM|001");
    }
}
