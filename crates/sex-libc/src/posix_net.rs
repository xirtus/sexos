// SPDX-License-Identifier: GPL-3.0-only
//! `<sys/socket.h>`, `<netdb.h>`, `<poll.h>`, `<arpa/inet.h>` for ordinary C programs:
//! musl's structures and numbers, translated to the program's own sockets in sex-std
//! (Hermit's interface underneath). A run reaches the network only if it was started
//! with `run +net`; otherwise calls fail with `EACCES`.
//!
//! `select` and `pselect` are `poll` underneath, over sockets, files and the standard
//! streams alike (standard input is readable once a whole line was typed). Names:
//! `getaddrinfo` (numbers and names, well-known services by name, `AI_PASSIVE`,
//! `AI_NUMERICHOST`, `AI_NUMERICSERV`, `AI_CANONNAME`), `getnameinfo` (numeric: there
//! is no reverse lookup), and the old `gethostbyname`.
use crate::{c_char, c_int, c_void, posix_util, set_errno};
use alloc::boxed::Box;
use core::ptr::null_mut;

extern "C" {
    fn sys_socket(domain: i32, kind: i32, protocol: i32) -> i32;
    fn sys_bind(s: i32, addr: *const u8, len: u32) -> i32;
    fn sys_connect(s: i32, addr: *const u8, len: u32) -> i32;
    fn sys_listen(s: i32, backlog: i32) -> i32;
    fn sys_accept(s: i32, addr: *mut u8, len: *mut u32) -> i32;
    fn sys_recv(s: i32, buf: *mut u8, len: usize, flags: i32) -> isize;
    fn sys_recvfrom(s: i32, buf: *mut u8, len: usize, flags: i32, addr: *mut u8, alen: *mut u32) -> isize;
    fn sys_send(s: i32, buf: *const c_void, len: usize, flags: i32) -> isize;
    fn sys_sendto(s: i32, buf: *const c_void, len: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
    fn sys_shutdown(s: i32, how: i32) -> i32;
    fn sys_getpeername(s: i32, addr: *mut u8, len: *mut u32) -> i32;
    fn sys_getsockname(s: i32, addr: *mut u8, len: *mut u32) -> i32;
    fn sys_getsockopt(s: i32, level: i32, name: i32, value: *mut c_void, len: *mut u32) -> i32;
    fn sys_setsockopt(s: i32, level: i32, name: i32, value: *const c_void, len: u32) -> i32;
    fn sys_poll(fds: *mut pollfd, n: usize, timeout: i32) -> i32;
    fn sys_fcntl(fd: i32, cmd: i32, arg: i32) -> i32;
    fn sys_getaddrinfo(name: *const c_char, service: *const c_char, hints: *const HAddrinfo, out: *mut *mut HAddrinfo) -> i32;
    fn sys_freeaddrinfo(ai: *mut HAddrinfo);
}

fn ret(r: i64) -> i64 { if r < 0 { set_errno(-r as c_int); -1 } else { r } }

// musl's numbers (x86-64) and Hermit's.
const AF_INET: c_int = 2;
const AF_INET6: c_int = 10;
const H_AF_INET: c_int = 3;
const H_AF_INET6: c_int = 1;
const SOCK_CLOEXEC: c_int = 0o2000000;
const H_SOCK_CLOEXEC: c_int = 0o40000;
const SOL_SOCKET: c_int = 1;
const H_SOL_SOCKET: c_int = 4095;
const MSG_PEEK: c_int = 2;
const H_MSG_PEEK: c_int = 1;
const EAI_BADFLAGS: c_int = -1;
const EAI_NONAME: c_int = -2;
const EAI_AGAIN: c_int = -3;
const EAI_FAIL: c_int = -4;
const EAI_FAMILY: c_int = -6;
const EAI_SOCKTYPE: c_int = -7;
const EAI_SERVICE: c_int = -8;
const EAI_MEMORY: c_int = -10;
const EAI_OVERFLOW: c_int = -12;

fn family_to_h(f: c_int) -> c_int { match f { AF_INET => H_AF_INET, AF_INET6 => H_AF_INET6, 0 => 0, _ => -1 } }
fn family_from_h(f: c_int) -> c_int { match f { H_AF_INET => AF_INET, H_AF_INET6 => AF_INET6, _ => f } }
fn option_to_h(level: c_int, name: c_int) -> (c_int, c_int) {
    if level != SOL_SOCKET { return (level, name); }
    (H_SOL_SOCKET, match name {
        2 => 0x0004,  // SO_REUSEADDR
        4 => 0x1007,  // SO_ERROR
        6 => 0x0020,  // SO_BROADCAST
        7 => 0x1001,  // SO_SNDBUF
        8 => 0x1002,  // SO_RCVBUF
        9 => 0x0008,  // SO_KEEPALIVE
        13 => 0x0080, // SO_LINGER
        20 => 0x1006, // SO_RCVTIMEO
        21 => 0x1005, // SO_SNDTIMEO
        other => other | 0x4000_0000,
    })
}

/// A musl `sockaddr` as Hermit's: the same bytes after the first two.
unsafe fn addr_to_h(addr: *const c_void, len: u32) -> Result<([u8; 28], u32), c_int> {
    if addr.is_null() || len < 2 { return Err(crate::EINVAL); }
    let src = unsafe { core::slice::from_raw_parts(addr as *const u8, (len as usize).min(28)) };
    let family = u16::from_ne_bytes([src[0], src[1]]) as c_int;
    let (h, need) = match family { AF_INET => (H_AF_INET, 16), AF_INET6 => (H_AF_INET6, 28), _ => return Err(97) }; // EAFNOSUPPORT
    if src.len() < need { return Err(crate::EINVAL); }
    let mut out = [0u8; 28];
    out[..need].copy_from_slice(&src[..need]);
    out[0] = need as u8;
    out[1] = h as u8;
    Ok((out, need as u32))
}
/// Hermit's `sockaddr` back into musl's, at most `*len` bytes.
unsafe fn addr_from_h(h: &[u8; 28], hlen: u32, addr: *mut c_void, len: *mut u32) {
    if addr.is_null() || len.is_null() { return; }
    let n = (hlen as usize).min(28);
    let mut out = *h;
    let family = family_from_h(h[1] as c_int) as u16;
    out[..2].copy_from_slice(&family.to_ne_bytes());
    unsafe {
        let room = (*len as usize).min(n);
        core::ptr::copy_nonoverlapping(out.as_ptr(), addr as *mut u8, room);
        *len = n as u32;
    }
}

#[no_mangle]
pub extern "C" fn socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int {
    let d = family_to_h(domain);
    if d <= 0 { set_errno(97); return -1; }
    let k = (kind & !SOCK_CLOEXEC) | if kind & SOCK_CLOEXEC != 0 { H_SOCK_CLOEXEC } else { 0 };
    ret(unsafe { sys_socket(d, k, protocol) } as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn bind(s: c_int, addr: *const c_void, len: u32) -> c_int {
    match unsafe { addr_to_h(addr, len) } { Ok((h, n)) => ret(unsafe { sys_bind(s, h.as_ptr(), n) } as i64) as c_int, Err(e) => { set_errno(e); -1 } }
}
#[no_mangle]
pub unsafe extern "C" fn connect(s: c_int, addr: *const c_void, len: u32) -> c_int {
    match unsafe { addr_to_h(addr, len) } { Ok((h, n)) => ret(unsafe { sys_connect(s, h.as_ptr(), n) } as i64) as c_int, Err(e) => { set_errno(e); -1 } }
}
#[no_mangle]
pub extern "C" fn listen(s: c_int, backlog: c_int) -> c_int { ret(unsafe { sys_listen(s, backlog) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn accept(s: c_int, addr: *mut c_void, len: *mut u32) -> c_int {
    let (mut h, mut hlen) = ([0u8; 28], 28u32);
    let fd = ret(unsafe { sys_accept(s, h.as_mut_ptr(), &mut hlen) } as i64) as c_int;
    if fd >= 0 { unsafe { addr_from_h(&h, hlen, addr, len) }; }
    fd
}
#[no_mangle]
pub unsafe extern "C" fn accept4(s: c_int, addr: *mut c_void, len: *mut u32, flags: c_int) -> c_int {
    let fd = unsafe { accept(s, addr, len) };
    if fd >= 0 && flags & 0o4000 != 0 { unsafe { sys_fcntl(fd, 4, 0o4000) }; }
    fd
}
fn flags_to_h(flags: c_int) -> c_int { if flags & MSG_PEEK != 0 { H_MSG_PEEK } else { 0 } }
#[no_mangle]
pub unsafe extern "C" fn recv(s: c_int, buf: *mut c_void, len: usize, flags: c_int) -> isize {
    ret(unsafe { sys_recv(s, buf as *mut u8, len, flags_to_h(flags)) } as i64) as isize
}
#[no_mangle]
pub unsafe extern "C" fn recvfrom(s: c_int, buf: *mut c_void, len: usize, flags: c_int, addr: *mut c_void, alen: *mut u32) -> isize {
    let (mut h, mut hlen) = ([0u8; 28], 28u32);
    let n = ret(unsafe { sys_recvfrom(s, buf as *mut u8, len, flags_to_h(flags), h.as_mut_ptr(), &mut hlen) } as i64) as isize;
    if n >= 0 { unsafe { addr_from_h(&h, hlen, addr, alen) }; }
    n
}
#[no_mangle]
pub unsafe extern "C" fn send(s: c_int, buf: *const c_void, len: usize, _flags: c_int) -> isize {
    ret(unsafe { sys_send(s, buf, len, 0) } as i64) as isize
}
#[no_mangle]
pub unsafe extern "C" fn sendto(s: c_int, buf: *const c_void, len: usize, _flags: c_int, addr: *const c_void, alen: u32) -> isize {
    if addr.is_null() { return unsafe { send(s, buf, len, 0) }; }
    match unsafe { addr_to_h(addr, alen) } {
        Ok((h, n)) => ret(unsafe { sys_sendto(s, buf, len, 0, h.as_ptr(), n) } as i64) as isize,
        Err(e) => { set_errno(e); -1 }
    }
}
#[no_mangle]
pub extern "C" fn shutdown(s: c_int, how: c_int) -> c_int { ret(unsafe { sys_shutdown(s, how) } as i64) as c_int }
#[no_mangle]
pub unsafe extern "C" fn getpeername(s: c_int, addr: *mut c_void, len: *mut u32) -> c_int {
    let (mut h, mut hlen) = ([0u8; 28], 28u32);
    let r = ret(unsafe { sys_getpeername(s, h.as_mut_ptr(), &mut hlen) } as i64) as c_int;
    if r == 0 { unsafe { addr_from_h(&h, hlen, addr, len) }; }
    r
}
#[no_mangle]
pub unsafe extern "C" fn getsockname(s: c_int, addr: *mut c_void, len: *mut u32) -> c_int {
    let (mut h, mut hlen) = ([0u8; 28], 28u32);
    let r = ret(unsafe { sys_getsockname(s, h.as_mut_ptr(), &mut hlen) } as i64) as c_int;
    if r == 0 { unsafe { addr_from_h(&h, hlen, addr, len) }; }
    r
}
#[no_mangle]
pub unsafe extern "C" fn setsockopt(s: c_int, level: c_int, name: c_int, value: *const c_void, len: u32) -> c_int {
    let (l, n) = option_to_h(level, name);
    ret(unsafe { sys_setsockopt(s, l, n, value, len) } as i64) as c_int
}
#[no_mangle]
pub unsafe extern "C" fn getsockopt(s: c_int, level: c_int, name: c_int, value: *mut c_void, len: *mut u32) -> c_int {
    let (l, n) = option_to_h(level, name);
    ret(unsafe { sys_getsockopt(s, l, n, value, len) } as i64) as c_int
}

#[repr(C)]
pub struct pollfd { pub fd: c_int, pub events: i16, pub revents: i16 }
/// The same structure and flags in both.
#[no_mangle]
pub unsafe extern "C" fn poll(fds: *mut pollfd, n: usize, timeout: c_int) -> c_int { ret(unsafe { sys_poll(fds, n, timeout) } as i64) as c_int }

/// `F_GETFL` and `F_SETFL` (`O_NONBLOCK` on sockets); other commands succeed.
#[no_mangle]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, mut ap: ...) -> c_int {
    let arg = if cmd == 4 { unsafe { ap.next_arg::<c_int>() } } else { 0 };
    ret(unsafe { sys_fcntl(fd, cmd, arg) } as i64) as c_int
}

// ---- names --------------------------------------------------------------------------------

#[repr(C)]
pub struct HAddrinfo {
    ai_flags: i32, ai_family: i32, ai_socktype: i32, ai_protocol: i32, ai_addrlen: u32,
    ai_canonname: *mut c_char, ai_addr: *mut u8, ai_next: *mut HAddrinfo,
}
#[repr(C)]
pub struct addrinfo {
    pub ai_flags: c_int, pub ai_family: c_int, pub ai_socktype: c_int, pub ai_protocol: c_int,
    pub ai_addrlen: u32, pub ai_addr: *mut c_void, pub ai_canonname: *mut c_char, pub ai_next: *mut addrinfo,
}
const AI_PASSIVE: c_int = 1;
const AI_CANONNAME: c_int = 2;
const AI_NUMERICHOST: c_int = 4;
const AI_NUMERICSERV: c_int = 0x400;
/// Every flag musl defines; the others (`AI_V4MAPPED`, `AI_ALL`, `AI_ADDRCONFIG`) change
/// nothing here.
const AI_KNOWN: c_int = 0x43f;

fn c_bytes<'a>(p: *const c_char) -> &'a [u8] { if p.is_null() { b"" } else { unsafe { crate::stdio::c_str(p) } } }

/// A copy of `text` in the C heap, for `ai_canonname`.
fn c_copy(text: &[u8]) -> *mut c_char {
    let p = unsafe { crate::mem::malloc(text.len() + 1) } as *mut u8;
    if !p.is_null() { unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len()); *p.add(text.len()) = 0; } }
    p as *mut c_char
}

#[no_mangle]
pub unsafe extern "C" fn getaddrinfo(node: *const c_char, service: *const c_char, hints: *const addrinfo, res: *mut *mut addrinfo) -> c_int {
    if res.is_null() { return EAI_FAIL; }
    if node.is_null() && service.is_null() { return EAI_NONAME; }
    let (family, socktype, protocol, flags) = if hints.is_null() { (0, 0, 0, 0) } else { unsafe { ((*hints).ai_family, (*hints).ai_socktype, (*hints).ai_protocol, (*hints).ai_flags) } };
    if flags & !AI_KNOWN != 0 || (flags & AI_CANONNAME != 0 && node.is_null()) { return EAI_BADFLAGS; }
    if !matches!(socktype, 0 | 1 | 2) { return EAI_SOCKTYPE; }
    let port = match posix_util::service_port(c_bytes(service), flags & AI_NUMERICSERV != 0) { Ok(p) => p, Err(e) => return e };
    let hf = family_to_h(family);
    if hf < 0 { return EAI_FAMILY; }
    if flags & AI_NUMERICHOST != 0 && !node.is_null() && !posix_util::is_numeric_host(c_bytes(node)) { return EAI_NONAME; }
    // No name: the "any" address to listen on, or this machine's own.
    let fallback: &[u8] = if flags & AI_PASSIVE != 0 { if family == AF_INET6 { b"::\0" } else { b"0.0.0.0\0" } } else if family == AF_INET6 { b"::1\0" } else { b"127.0.0.1\0" };
    let name = if node.is_null() { fallback.as_ptr() as *const c_char } else { node };
    let hhints = HAddrinfo { ai_flags: 0, ai_family: hf, ai_socktype: 0, ai_protocol: 0, ai_addrlen: 0, ai_canonname: null_mut(), ai_addr: null_mut(), ai_next: null_mut() };
    let mut list: *mut HAddrinfo = null_mut();
    let e = unsafe { sys_getaddrinfo(name, null_mut(), &hhints, &mut list) };
    if e != 0 {
        return match e { 2 => EAI_AGAIN, 8 => EAI_NONAME, _ => EAI_FAIL }; // Hermit's EAI_AGAIN, EAI_NONAME
    }
    let mut head: *mut addrinfo = null_mut();
    let mut tail: *mut *mut addrinfo = &mut head;
    let mut at = list;
    while !at.is_null() {
        let h = unsafe { &*at };
        let mut bytes = [0u8; 28];
        let n = (h.ai_addrlen as usize).min(28);
        unsafe { core::ptr::copy_nonoverlapping(h.ai_addr, bytes.as_mut_ptr(), n) };
        bytes[2..4].copy_from_slice(&port.to_be_bytes());
        let addr = Box::into_raw(Box::new([0u8; 28]));
        let mut len = 28u32;
        unsafe { addr_from_h(&bytes, n as u32, addr as *mut c_void, &mut len) };
        let kinds: &[c_int] = if socktype != 0 { &[socktype][..] } else { &[1, 2][..] };
        for (i, kind) in kinds.iter().enumerate() {
            let addr = if i == 0 { addr } else { Box::into_raw(Box::new(unsafe { *addr })) };
            let entry = Box::into_raw(Box::new(addrinfo {
                ai_flags: 0, ai_family: family_from_h(h.ai_family), ai_socktype: *kind,
                ai_protocol: if protocol != 0 { protocol } else if *kind == 1 { 6 } else { 17 },
                ai_addrlen: len, ai_addr: addr as *mut c_void, ai_canonname: null_mut(), ai_next: null_mut(),
            }));
            unsafe { *tail = entry; tail = &mut (*entry).ai_next; }
        }
        at = h.ai_next;
    }
    unsafe { sys_freeaddrinfo(list); }
    if head.is_null() { return EAI_NONAME; }
    // The first entry carries the name asked for: there are no aliases to follow.
    if flags & AI_CANONNAME != 0 {
        let copy = c_copy(c_bytes(node));
        if copy.is_null() { unsafe { freeaddrinfo(head) }; return EAI_MEMORY; }
        unsafe { (*head).ai_canonname = copy; }
    }
    unsafe { *res = head; }
    0
}
#[no_mangle]
pub unsafe extern "C" fn freeaddrinfo(mut ai: *mut addrinfo) {
    while !ai.is_null() {
        let entry = unsafe { Box::from_raw(ai) };
        if !entry.ai_addr.is_null() { drop(unsafe { Box::from_raw(entry.ai_addr as *mut [u8; 28]) }); }
        if !entry.ai_canonname.is_null() { unsafe { crate::mem::free(entry.ai_canonname as *mut c_void) }; }
        ai = entry.ai_next;
    }
}
#[no_mangle]
pub extern "C" fn gai_strerror(e: c_int) -> *const c_char {
    (match e {
        0 => "Success\0",
        EAI_BADFLAGS => "Invalid flags\0",
        EAI_NONAME => "Name does not resolve\0", EAI_AGAIN => "Temporary failure in name resolution\0",
        EAI_FAIL => "Non-recoverable failure in name resolution\0",
        EAI_FAMILY => "Unrecognized address family or invalid length\0",
        EAI_SOCKTYPE => "Unrecognized socket type\0", EAI_SERVICE => "Unrecognized service\0",
        EAI_MEMORY => "Out of memory\0", EAI_OVERFLOW => "Overflow\0",
        _ => "Unknown error\0",
    }).as_ptr() as *const c_char
}

const NI_NUMERICHOST: c_int = 1;
const NI_NUMERICSERV: c_int = 2;
const NI_NAMEREQD: c_int = 8;

/// An address as text: always the number, since there is no reverse lookup (so
/// `NI_NAMEREQD` fails). The service is its well-known name unless `NI_NUMERICSERV`.
#[no_mangle]
pub unsafe extern "C" fn getnameinfo(sa: *const c_void, salen: u32, host: *mut c_char, hostlen: u32, serv: *mut c_char, servlen: u32, flags: c_int) -> c_int {
    if sa.is_null() || salen < 2 { return EAI_FAMILY; }
    let family = unsafe { *(sa as *const u16) } as c_int;
    let (need, at) = match family { AF_INET => (16, 4), AF_INET6 => (28, 8), _ => return EAI_FAMILY };
    if (salen as usize) < need { return EAI_FAMILY; }
    let bytes = unsafe { core::slice::from_raw_parts(sa as *const u8, need) };
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    if !host.is_null() && hostlen > 0 {
        if flags & NI_NAMEREQD != 0 && flags & NI_NUMERICHOST == 0 { return EAI_NONAME; }
        if unsafe { inet_ntop(family, bytes[at..].as_ptr() as *const c_void, host, hostlen) }.is_null() { return EAI_OVERFLOW; }
    }
    if !serv.is_null() && servlen > 0 {
        use core::fmt::Write;
        let mut t = Text { buf: [0; 46], len: 0 };
        let named = if flags & NI_NUMERICSERV == 0 { posix_util::service_name(port) } else { None };
        let _ = match named { Some(n) => t.write_str(n), None => write!(t, "{port}") };
        if t.len + 1 > servlen as usize { return EAI_OVERFLOW; }
        unsafe { core::ptr::copy_nonoverlapping(t.buf.as_ptr(), serv as *mut u8, t.len); *serv.add(t.len) = 0; }
    }
    0
}

/// musl's `struct hostent`.
#[repr(C)]
pub struct hostent { pub h_name: *mut c_char, pub h_aliases: *mut *mut c_char, pub h_addrtype: c_int, pub h_length: c_int, pub h_addr_list: *mut *mut c_char }
static mut HOST: hostent = hostent { h_name: null_mut(), h_aliases: null_mut(), h_addrtype: 0, h_length: 0, h_addr_list: null_mut() };
static mut HOST_NAME: [u8; 256] = [0; 256];
static mut HOST_ADDRS: [[u8; 4]; 8] = [[0; 4]; 8];
static mut HOST_LIST: [*mut c_char; 9] = [null_mut(); 9];
static mut HOST_ALIASES: [*mut c_char; 1] = [null_mut()];
#[no_mangle]
pub static mut h_errno: c_int = 0;
#[no_mangle]
pub unsafe extern "C" fn __h_errno_location() -> *mut c_int { core::ptr::addr_of_mut!(h_errno) }

/// The old name lookup, IPv4 only: up to eight addresses, in storage the next call
/// reuses (as everywhere).
#[no_mangle]
pub unsafe extern "C" fn gethostbyname(name: *const c_char) -> *mut hostent {
    const HOST_NOT_FOUND: c_int = 1;
    const TRY_AGAIN: c_int = 2;
    let hints = addrinfo { ai_flags: 0, ai_family: AF_INET, ai_socktype: 1, ai_protocol: 0, ai_addrlen: 0, ai_addr: null_mut(), ai_canonname: null_mut(), ai_next: null_mut() };
    let mut res: *mut addrinfo = null_mut();
    match unsafe { getaddrinfo(name, null_mut(), &hints, &mut res) } {
        0 => {}
        EAI_AGAIN => { unsafe { h_errno = TRY_AGAIN }; return null_mut(); }
        _ => { unsafe { h_errno = HOST_NOT_FOUND }; return null_mut(); }
    }
    unsafe {
        let text = c_bytes(name);
        let n = text.len().min(255);
        let hn = &mut *core::ptr::addr_of_mut!(HOST_NAME);
        hn[..n].copy_from_slice(&text[..n]);
        hn[n] = 0;
        let (addrs, list) = (&mut *core::ptr::addr_of_mut!(HOST_ADDRS), &mut *core::ptr::addr_of_mut!(HOST_LIST));
        let (mut at, mut count) = (res, 0);
        while !at.is_null() && count < 8 {
            let sa = (*at).ai_addr as *const u8;
            core::ptr::copy_nonoverlapping(sa.add(4), addrs[count].as_mut_ptr(), 4);
            list[count] = addrs[count].as_mut_ptr() as *mut c_char;
            count += 1;
            at = (*at).ai_next;
        }
        list[count] = null_mut();
        freeaddrinfo(res);
        HOST = hostent { h_name: hn.as_mut_ptr() as *mut c_char, h_aliases: core::ptr::addr_of_mut!(HOST_ALIASES) as *mut *mut c_char, h_addrtype: AF_INET, h_length: 4, h_addr_list: list.as_mut_ptr() };
        core::ptr::addr_of_mut!(HOST)
    }
}

// ---- select ------------------------------------------------------------------------------

#[repr(C)]
pub struct timespec { pub tv_sec: i64, pub tv_nsec: i64 }

unsafe fn select_ms(nfds: c_int, r: *mut posix_util::FdSet, w: *mut posix_util::FdSet, e: *mut posix_util::FdSet, ms: c_int) -> c_int {
    if !(0..=posix_util::FD_SETSIZE as c_int).contains(&nfds) { set_errno(crate::EINVAL); return -1; }
    let (rs, ws, es) = unsafe { (r.as_ref(), w.as_ref(), e.as_ref()) };
    let asked = posix_util::polls_of(nfds as usize, rs, ws, es);
    let mut fds: alloc::vec::Vec<pollfd> = asked.iter().map(|&(fd, events)| pollfd { fd, events, revents: 0 }).collect();
    let n = unsafe { sys_poll(fds.as_mut_ptr(), fds.len(), ms) };
    if n < 0 { set_errno(-n); return -1; }
    let answers: alloc::vec::Vec<(c_int, i16, i16)> = fds.iter().map(|p| (p.fd, p.events, p.revents)).collect();
    match posix_util::sets_of(&answers, unsafe { r.as_mut() }, unsafe { w.as_mut() }, unsafe { e.as_mut() }) {
        Some(count) => count,
        None => { set_errno(crate::EBADF); -1 }
    }
}

/// `select`, as `poll` over the descriptors in the sets. The time left is not written
/// back (POSIX allows either).
#[no_mangle]
pub unsafe extern "C" fn select(nfds: c_int, r: *mut posix_util::FdSet, w: *mut posix_util::FdSet, e: *mut posix_util::FdSet, timeout: *mut timeval) -> c_int {
    let t = if timeout.is_null() { None } else { unsafe { Some(((*timeout).tv_sec, (*timeout).tv_usec)) } };
    let Ok(ms) = posix_util::timeout_ms(t) else { set_errno(crate::EINVAL); return -1 };
    unsafe { select_ms(nfds, r, w, e, ms) }
}
/// `pselect`: there are no signals, so the mask changes nothing.
#[no_mangle]
pub unsafe extern "C" fn pselect(nfds: c_int, r: *mut posix_util::FdSet, w: *mut posix_util::FdSet, e: *mut posix_util::FdSet, timeout: *const timespec, _mask: *const c_void) -> c_int {
    let t = if timeout.is_null() { None } else { unsafe { Some(((*timeout).tv_sec, (*timeout).tv_nsec / 1000)) } };
    let Ok(ms) = posix_util::timeout_ms(t) else { set_errno(crate::EINVAL); return -1 };
    unsafe { select_ms(nfds, r, w, e, ms) }
}
/// `ppoll`: `poll` with a `timespec`; no signals, so no mask.
#[no_mangle]
pub unsafe extern "C" fn ppoll(fds: *mut pollfd, n: usize, timeout: *const timespec, _mask: *const c_void) -> c_int {
    let ms = if timeout.is_null() { -1 } else {
        let t = unsafe { &*timeout };
        if t.tv_sec < 0 || !(0..1_000_000_000).contains(&t.tv_nsec) { set_errno(crate::EINVAL); return -1; }
        t.tv_sec.saturating_mul(1000).saturating_add((t.tv_nsec + 999_999) / 1_000_000).min(c_int::MAX as i64) as c_int
    };
    unsafe { poll(fds, n, ms) }
}

// ---- byte order and addresses ---------------------------------------------------------------

#[no_mangle]
pub extern "C" fn htons(v: u16) -> u16 { v.to_be() }
#[no_mangle]
pub extern "C" fn ntohs(v: u16) -> u16 { u16::from_be(v) }
#[no_mangle]
pub extern "C" fn htonl(v: u32) -> u32 { v.to_be() }
#[no_mangle]
pub extern "C" fn ntohl(v: u32) -> u32 { u32::from_be(v) }

fn parse4(t: &[u8]) -> Option<[u8; 4]> {
    let mut a = [0u8; 4];
    let mut parts = t.split(|b| *b == b'.');
    for slot in a.iter_mut() {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.iter().all(u8::is_ascii_digit) { return None; }
        *slot = core::str::from_utf8(p).ok()?.parse().ok()?;
    }
    parts.next().is_none().then_some(a)
}
fn parse6(t: &[u8]) -> Option<[u8; 16]> {
    fn groups(part: &[u8], out: &mut [u16; 8]) -> Option<usize> {
        if part.is_empty() { return Some(0); }
        let mut n = 0;
        for g in part.split(|b| *b == b':') {
            if g.is_empty() || g.len() > 4 || n == 8 { return None; }
            out[n] = u16::from_str_radix(core::str::from_utf8(g).ok()?, 16).ok()?; n += 1;
        }
        Some(n)
    }
    let (mut head, mut tail) = ([0u16; 8], [0u16; 8]);
    let split = t.windows(2).position(|w| w == b"::");
    let (h, k) = match split { Some(i) => (groups(&t[..i], &mut head)?, groups(&t[i + 2..], &mut tail)?), None => (groups(t, &mut head)?, 0) };
    if split.is_none() && h != 8 || split.is_some() && h + k > 7 { return None; }
    let mut w = [0u16; 8];
    w[..h].copy_from_slice(&head[..h]);
    w[8 - k..].copy_from_slice(&tail[..k]);
    let mut a = [0u8; 16];
    for (i, g) in w.iter().enumerate() { a[2 * i..2 * i + 2].copy_from_slice(&g.to_be_bytes()); }
    Some(a)
}
#[no_mangle]
pub unsafe extern "C" fn inet_pton(family: c_int, src: *const c_char, dst: *mut c_void) -> c_int {
    let t = unsafe { crate::stdio::c_str(src) };
    match family {
        AF_INET => match parse4(t) { Some(a) => { unsafe { core::ptr::copy_nonoverlapping(a.as_ptr(), dst as *mut u8, 4) }; 1 } None => 0 },
        AF_INET6 => match parse6(t) { Some(a) => { unsafe { core::ptr::copy_nonoverlapping(a.as_ptr(), dst as *mut u8, 16) }; 1 } None => 0 },
        _ => { set_errno(97); -1 }
    }
}
#[no_mangle]
pub unsafe extern "C" fn inet_addr(src: *const c_char) -> u32 {
    parse4(unsafe { crate::stdio::c_str(src) }).map_or(u32::MAX, u32::from_ne_bytes)
}
struct Text { buf: [u8; 46], len: usize }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        if self.len + b.len() > self.buf.len() { return Err(core::fmt::Error); }
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
        Ok(())
    }
}
#[no_mangle]
pub unsafe extern "C" fn inet_ntop(family: c_int, src: *const c_void, dst: *mut c_char, size: u32) -> *const c_char {
    use core::fmt::Write;
    let mut t = Text { buf: [0; 46], len: 0 };
    let ok = match family {
        AF_INET => { let a = unsafe { *(src as *const [u8; 4]) }; write!(t, "{}.{}.{}.{}", a[0], a[1], a[2], a[3]).is_ok() }
        AF_INET6 => {
            let a = unsafe { *(src as *const [u8; 16]) };
            let w: [u16; 8] = core::array::from_fn(|i| u16::from_be_bytes([a[2 * i], a[2 * i + 1]]));
            let (mut best, mut at) = ((0, 0), 0);
            while at < 8 { let len = w[at..].iter().take_while(|g| **g == 0).count(); if len > best.1 { best = (at, len); } at += len.max(1); }
            let mut i = 0;
            let mut ok = true;
            while i < 8 {
                if best.1 >= 2 && i == best.0 { ok &= t.write_str(if i == 0 { "::" } else { ":" }).is_ok(); i += best.1; continue; }
                ok &= write!(t, "{:x}", w[i]).is_ok();
                if i < 7 { ok &= t.write_str(":").is_ok(); }
                i += 1;
            }
            ok
        }
        _ => { set_errno(97); return core::ptr::null(); }
    };
    if !ok || t.len + 1 > size as usize { set_errno(28); return core::ptr::null(); } // ENOSPC
    unsafe { core::ptr::copy_nonoverlapping(t.buf.as_ptr(), dst as *mut u8, t.len); *dst.add(t.len) = 0; }
    dst
}

// ---- time of day ------------------------------------------------------------------------

#[repr(C)]
pub struct timeval { pub tv_sec: i64, pub tv_usec: i64 }
#[no_mangle]
pub unsafe extern "C" fn gettimeofday(tv: *mut timeval, _tz: *mut c_void) -> c_int {
    if !tv.is_null() {
        let ns = super::posix::clock().0;
        unsafe { *tv = timeval { tv_sec: ns.div_euclid(1_000_000_000), tv_usec: ns.rem_euclid(1_000_000_000) / 1000 }; }
    }
    0
}
