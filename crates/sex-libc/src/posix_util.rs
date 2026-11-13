// SPDX-License-Identifier: GPL-3.0-only
//! The arithmetic behind `select`, `getaddrinfo`'s services and `mmap`'s write-back,
//! kept apart from the calls so host tests can check it.
use crate::c_int;
use alloc::vec::Vec;

// ---- select ------------------------------------------------------------------------------

/// musl's `FD_SETSIZE`: an `fd_set` is 1024 bits in 64-bit words.
pub const FD_SETSIZE: usize = 1024;
pub type FdSet = [u64; FD_SETSIZE / 64];

pub const POLLIN: i16 = 0x1;
pub const POLLPRI: i16 = 0x2;
pub const POLLOUT: i16 = 0x4;
pub const POLLERR: i16 = 0x8;
pub const POLLHUP: i16 = 0x10;
pub const POLLNVAL: i16 = 0x20;

pub fn is_set(set: &FdSet, fd: usize) -> bool { set[fd / 64] & (1 << (fd % 64)) != 0 }
pub fn put(set: &mut FdSet, fd: usize) { set[fd / 64] |= 1 << (fd % 64); }

/// The descriptors `select` watches, as `(fd, events)` for `poll`.
pub fn polls_of(nfds: usize, read: Option<&FdSet>, write: Option<&FdSet>, except: Option<&FdSet>) -> Vec<(c_int, i16)> {
    let mut out = Vec::new();
    for fd in 0..nfds.min(FD_SETSIZE) {
        let mut ev = 0;
        if read.is_some_and(|s| is_set(s, fd)) { ev |= POLLIN; }
        if write.is_some_and(|s| is_set(s, fd)) { ev |= POLLOUT; }
        if except.is_some_and(|s| is_set(s, fd)) { ev |= POLLPRI; }
        if ev != 0 { out.push((fd as c_int, ev)); }
    }
    out
}

/// `poll`'s answers written back as `select`'s sets; the count of bits set, or `None`
/// if a watched descriptor is not open (`select` then fails with EBADF).
/// As in Linux, an error or hang-up makes a descriptor readable (and writable, if
/// watched for that), so the read that follows reports it.
pub fn sets_of(answers: &[(c_int, i16, i16)], read: Option<&mut FdSet>, write: Option<&mut FdSet>, except: Option<&mut FdSet>) -> Option<c_int> {
    if answers.iter().any(|a| a.2 & POLLNVAL != 0) { return None; }
    let (mut r, mut w, mut e) = ([0u64; FD_SETSIZE / 64], [0u64; FD_SETSIZE / 64], [0u64; FD_SETSIZE / 64]);
    let mut count = 0;
    for &(fd, asked, got) in answers {
        let fd = fd as usize;
        let failed = got & (POLLERR | POLLHUP) != 0;
        if asked & POLLIN != 0 && (got & POLLIN != 0 || failed) { put(&mut r, fd); count += 1; }
        if asked & POLLOUT != 0 && (got & POLLOUT != 0 || failed) { put(&mut w, fd); count += 1; }
        if asked & POLLPRI != 0 && got & POLLPRI != 0 { put(&mut e, fd); count += 1; }
    }
    if let Some(s) = read { *s = r; }
    if let Some(s) = write { *s = w; }
    if let Some(s) = except { *s = e; }
    Some(count)
}

/// A `struct timeval` (seconds, microseconds) as `poll`'s milliseconds; no time: wait.
pub fn timeout_ms(t: Option<(i64, i64)>) -> Result<c_int, ()> {
    match t {
        None => Ok(-1),
        Some((s, us)) if s < 0 || !(0..1_000_000).contains(&us) => Err(()),
        Some((s, us)) => Ok(s.saturating_mul(1000).saturating_add((us + 999) / 1000).min(c_int::MAX as i64) as c_int),
    }
}

// ---- services ----------------------------------------------------------------------------

/// The well-known services a C program names (`/etc/services` has nothing else a
/// program here could reach): name and port, for TCP and UDP alike.
pub const SERVICES: &[(&str, u16)] = &[
    ("echo", 7), ("discard", 9), ("daytime", 13), ("ftp-data", 20), ("ftp", 21), ("ssh", 22),
    ("telnet", 23), ("smtp", 25), ("time", 37), ("domain", 53), ("gopher", 70), ("finger", 79),
    ("http", 80), ("www", 80), ("pop3", 110), ("sunrpc", 111), ("auth", 113), ("ntp", 123),
    ("imap", 143), ("snmp", 161), ("irc", 194), ("ldap", 389), ("https", 443), ("submissions", 465),
    ("smtps", 465), ("syslog", 514), ("submission", 587), ("ldaps", 636), ("rsync", 873), ("imaps", 993),
    ("pop3s", 995), ("socks", 1080), ("mqtt", 1883), ("mysql", 3306), ("postgresql", 5432),
    ("http-alt", 8080), ("gemini", 1965), ("ircs-u", 6697),
];

pub const EAI_NONAME: c_int = -2;
pub const EAI_SERVICE: c_int = -8;

/// A service as `getaddrinfo` reads it: a number, or (unless only numbers are allowed)
/// a name from [`SERVICES`]. Empty: port 0.
pub fn service_port(s: &[u8], numeric_only: bool) -> Result<u16, c_int> {
    if s.is_empty() { return Ok(0); }
    if s.iter().all(u8::is_ascii_digit) {
        return core::str::from_utf8(s).ok().and_then(|t| t.parse::<u16>().ok()).ok_or(EAI_SERVICE);
    }
    if numeric_only { return Err(EAI_NONAME); }
    SERVICES.iter().find(|(n, _)| n.as_bytes().eq_ignore_ascii_case(s)).map(|(_, p)| *p).ok_or(EAI_SERVICE)
}

/// A port's service name, for `getnameinfo`.
pub fn service_name(port: u16) -> Option<&'static str> { SERVICES.iter().find(|(_, p)| *p == port).map(|(n, _)| *n) }

/// Whether `s` is written as an address (IPv4 dotted or IPv6), not a name to look up.
pub fn is_numeric_host(s: &[u8]) -> bool {
    let v4 = s.split(|b| *b == b'.').count() == 4 && s.split(|b| *b == b'.').all(|p| !p.is_empty() && p.len() <= 3 && p.iter().all(u8::is_ascii_digit) && core::str::from_utf8(p).ok().and_then(|t| t.parse::<u8>().ok()).is_some());
    let v6 = s.contains(&b':') && s.iter().all(|b| b.is_ascii_hexdigit() || *b == b':' || *b == b'.');
    v4 || v6
}

// ---- mmap --------------------------------------------------------------------------------

pub const PAGE: usize = 4096;

/// How many bytes of a shared mapping of `len` bytes at `offset` go back to a file of
/// `file_size` bytes: what was in the file when it was mapped, never past its end
/// (POSIX: bytes past the end of the file are not written to it).
pub fn writeback_len(len: usize, offset: u64, file_size: u64) -> usize {
    (file_size.saturating_sub(offset)).min(len as u64) as usize
}

/// A mapping's length rounded up to whole pages; `None` for 0 or too large.
pub fn map_len(len: usize) -> Option<usize> {
    if len == 0 { return None; }
    len.checked_add(PAGE - 1).map(|n| n & !(PAGE - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_sets_round_trip_through_poll() {
        let (mut r, mut w) = ([0u64; 16], [0u64; 16]);
        put(&mut r, 0); put(&mut r, 70); put(&mut w, 1); put(&mut w, 70);
        let polls = polls_of(71, Some(&r), Some(&w), None);
        assert_eq!(polls, [(0, POLLIN), (1, POLLOUT), (70, POLLIN | POLLOUT)]);
        // stdin had nothing, stdout writable, socket 70 readable only.
        let answers = [(0, POLLIN, 0), (1, POLLOUT, POLLOUT), (70, POLLIN | POLLOUT, POLLIN)];
        assert_eq!(sets_of(&answers, Some(&mut r), Some(&mut w), None), Some(2));
        assert!(!is_set(&r, 0) && is_set(&r, 70) && is_set(&w, 1) && !is_set(&w, 70));
    }
    #[test]
    fn select_beyond_nfds_is_ignored_and_bad_fd_fails() {
        let mut r = [0u64; 16];
        put(&mut r, 5);
        assert!(polls_of(5, Some(&r), None, None).is_empty());
        assert_eq!(sets_of(&[(5, POLLIN, POLLNVAL)], Some(&mut r), None, None), None);
        // A hang-up reads as readable.
        assert_eq!(sets_of(&[(5, POLLIN, POLLHUP)], Some(&mut r), None, None), Some(1));
    }
    #[test]
    fn select_timeouts() {
        assert_eq!(timeout_ms(None), Ok(-1));
        assert_eq!(timeout_ms(Some((0, 0))), Ok(0));
        assert_eq!(timeout_ms(Some((1, 1))), Ok(1001));
        assert_eq!(timeout_ms(Some((0, 1_000_000))), Err(()));
    }
    #[test]
    fn services_by_number_and_name() {
        assert_eq!(service_port(b"", false), Ok(0));
        assert_eq!(service_port(b"8080", true), Ok(8080));
        assert_eq!(service_port(b"https", false), Ok(443));
        assert_eq!(service_port(b"HTTP", false), Ok(80));
        assert_eq!(service_port(b"http", true), Err(EAI_NONAME));
        assert_eq!(service_port(b"nope", false), Err(EAI_SERVICE));
        assert_eq!(service_port(b"70000", false), Err(EAI_SERVICE));
        assert_eq!(service_name(443), Some("https"));
        assert_eq!(service_name(4), None);
    }
    #[test]
    fn numeric_hosts() {
        assert!(is_numeric_host(b"10.0.2.2") && is_numeric_host(b"::1") && is_numeric_host(b"fd00::1"));
        assert!(!is_numeric_host(b"example.org") && !is_numeric_host(b"300.1.1.1") && !is_numeric_host(b"localhost"));
    }
    #[test]
    fn mapping_lengths() {
        assert_eq!(map_len(1), Some(4096));
        assert_eq!(map_len(4096), Some(4096));
        assert_eq!(map_len(0), None);
        assert_eq!(map_len(usize::MAX), None);
        assert_eq!(writeback_len(8192, 0, 100), 100);
        assert_eq!(writeback_len(8192, 4096, 100), 0);
        assert_eq!(writeback_len(10, 4, 100), 10);
    }
}
