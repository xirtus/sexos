// SPDX-License-Identifier: GPL-3.0-only
//! Sockets for `std::net` (and the C library's POSIX layer): TCP connections and
//! listeners, UDP sockets and name lookup, over the program's own network flows
//! (`sex_app::net::direct`). A run reaches the network only if the person started it
//! with `run +net`; otherwise every call fails with `EACCES`.
//!
//! A socket is a descriptor like a file. Calls that wait do so here, a step at a
//! time, without holding the socket: one thread can read while another writes.
//! Bytes that have arrived but not been read wait in the socket, which is how
//! `peek` and `poll` know what is ready.
use sex_app::net::{direct, Error as NetError, IpAddr};
use std::collections::VecDeque;
use std::os::hermit::hermit_abi::{self as abi, errno};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role { Fresh, Connecting, Stream, Listening, Udp }

struct State {
    socket: Option<direct::Socket>,
    role: Role,
    /// The port given to `bind` (TCP: taken at `listen`; UDP: at once).
    local_port: u16,
    peer: Option<(IpAddr, u16)>,
    nonblocking: bool,
    receive_timeout: Option<u64>,
    send_timeout: Option<u64>,
    /// Stream bytes that arrived and have not been read.
    inbox: VecDeque<u8>,
    /// A datagram looked at but not taken.
    datagram: Option<(Vec<u8>, IpAddr, u16)>,
    /// A connection `poll` found on a listener, kept for `accept`.
    pending: Option<(direct::Socket, IpAddr, u16)>,
    peer_closed: bool,
    read_shut: bool,
    write_shut: bool,
    /// What `SO_ERROR` reports once.
    error: i32,
    /// Options std may set and read back; they change nothing here.
    options: Vec<(i32, i32, i32)>,
}

pub(crate) struct Sock { family: i32, kind: i32, state: Mutex<State> }

impl Sock {
    fn state(&self) -> std::sync::MutexGuard<'_, State> { self.state.lock().unwrap_or_else(|e| e.into_inner()) }
}

fn errno_of(e: NetError) -> i32 {
    -match e {
        NetError::NotAllowed => errno::EACCES,
        NetError::Offline => errno::ENETUNREACH,
        NetError::NoName | NetError::Address => errno::EADDRNOTAVAIL,
        NetError::Connect => errno::ECONNREFUSED,
        NetError::TooSlow => errno::ETIMEDOUT,
        NetError::TooLarge => errno::EMSGSIZE,
        NetError::Busy => errno::EMFILE,
        NetError::InUse => errno::EADDRINUSE,
        NetError::Certificate | NetError::Secure | NetError::Failed => errno::EIO,
    }
}

/// Waits (thirty seconds at most) until the network service says the network is up:
/// a program may open a socket before the lease arrives, as one started at boot does.
fn network_up() -> Result<(), i32> {
    static SEEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if SEEN.load(core::sync::atomic::Ordering::Acquire) { return Ok(()); }
    let began = sex_pdx::get_ticks();
    let mut round = 0;
    loop {
        match direct::up() {
            Ok(true) => { SEEN.store(true, core::sync::atomic::Ordering::Release); return Ok(()); }
            Ok(false) | Err(NetError::Busy) => {}
            Err(e) => return Err(errno_of(e)),
        }
        if sex_pdx::get_ticks().saturating_sub(began) > 30_000 { return Err(-errno::ENETUNREACH); }
        pause(&mut round);
    }
}

/// Waits a little between steps; `round` counts the steps that found nothing.
fn pause(round: &mut u32) { *round += 1; sex_app::thread::pause(*round); }

/// Steps `step` until it answers, the socket is non-blocking (`EAGAIN`) or `limit`
/// milliseconds pass (`EAGAIN`, as a timed-out socket read does elsewhere).
fn wait<R>(sock: &Sock, limit: impl Fn(&State) -> Option<u64>, mut step: impl FnMut(&mut State) -> Option<Result<R, i32>>) -> Result<R, i32> {
    let began = sex_pdx::get_ticks();
    let mut round = 0;
    loop {
        let (answer, nonblocking, limit) = {
            let mut st = sock.state();
            (step(&mut st), st.nonblocking, limit(&st))
        };
        if let Some(answer) = answer { return answer; }
        if nonblocking { return Err(-errno::EAGAIN); }
        if limit.is_some_and(|ms| sex_pdx::get_ticks().saturating_sub(began) >= ms) { return Err(-errno::EAGAIN); }
        pause(&mut round);
    }
}

// ---- addresses ----------------------------------------------------------------------------

/// The address a `sockaddr` names.
pub(crate) unsafe fn read_address_pub(addr: *const abi::sockaddr, len: abi::socklen_t) -> Result<(IpAddr, u16), i32> { unsafe { read_address(addr, len) } }
unsafe fn read_address(addr: *const abi::sockaddr, len: abi::socklen_t) -> Result<(IpAddr, u16), i32> {
    if addr.is_null() { return Err(-errno::EFAULT); }
    let bytes = unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) };
    if bytes.len() < 2 { return Err(-errno::EINVAL); }
    match bytes[1] as i32 {
        abi::AF_INET if bytes.len() >= 8 => Ok((IpAddr::V4(bytes[4..8].try_into().unwrap()), u16::from_be_bytes([bytes[2], bytes[3]]))),
        abi::AF_INET6 if bytes.len() >= 24 => Ok((IpAddr::V6(bytes[8..24].try_into().unwrap()), u16::from_be_bytes([bytes[2], bytes[3]]))),
        _ => Err(-errno::EAFNOSUPPORT),
    }
}

/// Writes `ip:port` as a `sockaddr` of at most `*len` bytes and sets `*len`.
pub(crate) unsafe fn write_address_pub(addr: *mut abi::sockaddr, len: *mut abi::socklen_t, ip: &IpAddr, port: u16) { unsafe { write_address(addr, len, ip, port) } }
unsafe fn write_address(addr: *mut abi::sockaddr, len: *mut abi::socklen_t, ip: &IpAddr, port: u16) {
    if addr.is_null() || len.is_null() { return; }
    let mut out = [0u8; 28];
    let n = match ip {
        IpAddr::V4(a) => { out[0] = 16; out[1] = abi::AF_INET as u8; out[4..8].copy_from_slice(a); 16 }
        IpAddr::V6(a) => { out[0] = 28; out[1] = abi::AF_INET6 as u8; out[8..24].copy_from_slice(a); 28 }
    };
    out[2..4].copy_from_slice(&port.to_be_bytes());
    unsafe {
        let room = (*len as usize).min(n);
        core::ptr::copy_nonoverlapping(out.as_ptr(), addr as *mut u8, room);
        *len = n as abi::socklen_t;
    }
}

fn unspecified(family: i32) -> IpAddr { if family == abi::AF_INET6 { IpAddr::V6([0; 16]) } else { IpAddr::V4([0; 4]) } }

// ---- the calls ----------------------------------------------------------------------------

pub(crate) fn socket(domain: i32, kind: i32, _protocol: i32) -> Result<Sock, i32> {
    if domain != abi::AF_INET && domain != abi::AF_INET6 { return Err(-errno::EAFNOSUPPORT); }
    let base = kind & !(abi::SOCK_NONBLOCK | abi::SOCK_CLOEXEC);
    if base != abi::SOCK_STREAM && base != abi::SOCK_DGRAM { return Err(-errno::EPROTOTYPE); }
    Ok(Sock { family: domain, kind: base, state: Mutex::new(State {
        socket: None, role: Role::Fresh, local_port: 0, peer: None, nonblocking: kind & abi::SOCK_NONBLOCK != 0,
        receive_timeout: None, send_timeout: None, inbox: VecDeque::new(), datagram: None, pending: None,
        peer_closed: false, read_shut: false, write_shut: false, error: 0, options: Vec::new(),
    }) })
}

pub(crate) unsafe fn bind(sock: &Sock, addr: *const abi::sockaddr, len: abi::socklen_t) -> i32 {
    let (_, port) = match unsafe { read_address(addr, len) } { Ok(a) => a, Err(e) => return e };
    if sock.kind == abi::SOCK_DGRAM { if let Err(e) = network_up() { return e; } }
    let mut st = sock.state();
    if st.role != Role::Fresh || st.local_port != 0 { return -errno::EINVAL; }
    if sock.kind == abi::SOCK_DGRAM {
        match direct::Socket::udp(port) {
            Ok((s, got)) => { st.socket = Some(s); st.local_port = got; st.role = Role::Udp; 0 }
            Err(e) => errno_of(e),
        }
    } else { st.local_port = port; 0 }
}

pub(crate) fn listen(sock: &Sock) -> i32 {
    if sock.kind != abi::SOCK_STREAM { return -errno::EOPNOTSUPP; }
    if let Err(e) = network_up() { return e; }
    let mut st = sock.state();
    match st.role {
        Role::Listening => return 0,
        Role::Fresh => {}
        _ => return -errno::EINVAL,
    }
    // Listening on any port means one the service picks; it says which only through
    // a connection, so ask for a fixed one instead.
    if st.local_port == 0 { return -errno::EADDRNOTAVAIL; }
    match direct::Socket::listen(st.local_port) {
        Ok(s) => { st.socket = Some(s); st.role = Role::Listening; 0 }
        Err(e) => errno_of(e),
    }
}

pub(crate) unsafe fn connect(sock: &Sock, addr: *const abi::sockaddr, len: abi::socklen_t) -> i32 {
    let to = match unsafe { read_address(addr, len) } { Ok(a) => a, Err(e) => return e };
    if let Err(e) = network_up() { return e; }
    if sock.kind == abi::SOCK_DGRAM {
        let mut st = sock.state();
        if st.socket.is_none() {
            match direct::Socket::udp(0) { Ok((s, got)) => { st.socket = Some(s); st.local_port = got; st.role = Role::Udp; } Err(e) => return errno_of(e) }
        }
        st.peer = Some(to);
        return 0;
    }
    {
        let mut st = sock.state();
        match st.role {
            Role::Fresh => {}
            Role::Connecting => return -errno::EALREADY,
            _ => return -errno::EISCONN,
        }
        match direct::Socket::connect(&to.0, to.1) {
            Ok(s) => { st.socket = Some(s); st.role = Role::Connecting; st.peer = Some(to); }
            Err(e) => return errno_of(e),
        }
        if st.nonblocking { return -errno::EINPROGRESS; }
    }
    match wait(sock, |_| None, |st| finish_connect(st).map(|r| r.map(|_| ()))) { Ok(()) => 0, Err(e) => e }
}

/// One look at a connection on its way: `None` while it still is.
fn finish_connect(st: &mut State) -> Option<Result<bool, i32>> {
    if st.role != Role::Connecting { return Some(Ok(st.role == Role::Stream)); }
    let state = st.socket.as_ref().map(|s| s.state());
    match state {
        Some(Ok(direct::State::Opening)) => None,
        Some(Ok(direct::State::Closed)) | Some(Err(_)) | None => {
            st.socket = None; st.role = Role::Fresh; st.peer = None;
            st.error = errno::ECONNREFUSED;
            Some(Err(-errno::ECONNREFUSED))
        }
        Some(Ok(_)) => { st.role = Role::Stream; Some(Ok(true)) }
    }
}

pub(crate) unsafe fn accept(sock: &Sock, addr: *mut abi::sockaddr, len: *mut abi::socklen_t) -> Result<Sock, i32> {
    if sock.state().role != Role::Listening { return Err(-errno::EINVAL); }
    let (s, ip, port) = wait(sock, |st| st.receive_timeout, |st| {
        if let Some(got) = st.pending.take() { return Some(Ok(got)); }
        match st.socket.as_ref()?.accept() {
            Ok(Some(got)) => Some(Ok(got)),
            Ok(None) => None,
            Err(e) => Some(Err(errno_of(e))),
        }
    })?;
    unsafe { write_address(addr, len, &ip, port); }
    let new = socket(if ip.is_ipv6() { abi::AF_INET6 } else { abi::AF_INET }, abi::SOCK_STREAM, 0)?;
    {
        let mut st = new.state();
        st.socket = Some(s); st.role = Role::Stream; st.peer = Some((ip, port));
        st.local_port = sock.state().local_port;
    }
    Ok(new)
}

/// Reads stream bytes into the socket's inbox: false when none had come.
fn fill(st: &mut State) -> Result<bool, i32> {
    let Some(s) = st.socket.as_ref() else { return Err(-errno::ENOTCONN) };
    let mut buf = [0u8; 16 * 1024];
    let n = s.receive(&mut buf).map_err(errno_of)?;
    if n > 0 { st.inbox.extend(&buf[..n]); return Ok(true); }
    // Bytes can land between the two calls, so a closed peer gets one last read.
    if matches!(s.state(), Ok(direct::State::PeerClosed | direct::State::Closed) | Err(_)) {
        let n = s.receive(&mut buf).map_err(errno_of)?;
        st.inbox.extend(&buf[..n]);
        if n == 0 { st.peer_closed = true; }
        return Ok(n > 0);
    }
    Ok(false)
}

pub(crate) fn recv(sock: &Sock, out: &mut [u8], peek: bool) -> isize {
    if sock.kind == abi::SOCK_DGRAM {
        return match recv_from(sock, out, peek) { Ok((n, _, _)) => n as isize, Err(e) => e as isize };
    }
    let r = wait(sock, |st| st.receive_timeout, |st| {
        if let Some(r) = finish_connect(st) { if let Err(e) = r { return Some(Err(e)); } } else { return None; }
        if st.role != Role::Stream { return Some(Err(-errno::ENOTCONN)); }
        if st.inbox.is_empty() && !st.peer_closed && !st.read_shut {
            if let Err(e) = fill(st) { return Some(Err(e)); }
        }
        if st.inbox.is_empty() { return (st.peer_closed || st.read_shut || out.is_empty()).then_some(Ok(0)); }
        let n = out.len().min(st.inbox.len());
        for (i, b) in st.inbox.iter().take(n).enumerate() { out[i] = *b; }
        if !peek { st.inbox.drain(..n); }
        Some(Ok(n))
    });
    match r { Ok(n) => n as isize, Err(e) => e as isize }
}

pub(crate) fn send(sock: &Sock, bytes: &[u8]) -> isize {
    if sock.kind == abi::SOCK_DGRAM {
        let peer = sock.state().peer;
        return match peer { Some((ip, port)) => send_to(sock, bytes, &ip, port), None => -errno::EDESTADDRREQ as isize };
    }
    if bytes.is_empty() { return 0; }
    let r = wait(sock, |st| st.send_timeout, |st| {
        if let Some(r) = finish_connect(st) { if let Err(e) = r { return Some(Err(e)); } } else { return None; }
        if st.role != Role::Stream { return Some(Err(-errno::ENOTCONN)); }
        if st.write_shut { return Some(Err(-errno::EPIPE)); }
        let s = st.socket.as_ref()?;
        match s.send(bytes) {
            Ok(0) => match s.state() {
                Ok(direct::State::Closed) => Some(Err(-errno::ECONNRESET)),
                _ => None,
            },
            Ok(n) => Some(Ok(n)),
            Err(e) => Some(Err(errno_of(e))),
        }
    });
    match r { Ok(n) => n as isize, Err(e) => e as isize }
}

fn ensure_udp(sock: &Sock) -> Result<(), i32> {
    if sock.state().socket.is_none() { network_up()?; }
    let mut st = sock.state();
    if st.socket.is_none() {
        let (s, got) = direct::Socket::udp(0).map_err(errno_of)?;
        st.socket = Some(s); st.local_port = got; st.role = Role::Udp;
    }
    Ok(())
}

pub(crate) fn send_to(sock: &Sock, bytes: &[u8], ip: &IpAddr, port: u16) -> isize {
    if sock.kind != abi::SOCK_DGRAM { return send(sock, bytes); }
    if let Err(e) = ensure_udp(sock) { return e as isize; }
    let r = wait(sock, |st| st.send_timeout, |st| match st.socket.as_ref()?.send_to(bytes, ip, port) {
        Ok(true) => Some(Ok(bytes.len())),
        Ok(false) => None,
        Err(e) => Some(Err(errno_of(e))),
    });
    match r { Ok(n) => n as isize, Err(e) => e as isize }
}

/// Takes the next datagram into `st.datagram` if none is held: false when none has
/// come. A connected socket drops datagrams from anyone but its peer.
fn next_datagram(st: &mut State) -> Result<bool, i32> {
    while st.datagram.is_none() {
        let mut buf = vec![0u8; 65536];
        let Some(s) = st.socket.as_ref() else { return Err(-errno::EINVAL) };
        match s.receive_from(&mut buf).map_err(errno_of)? {
            Some((n, ip, port)) => {
                if st.peer.is_some_and(|p| p != (ip, port)) { continue; }
                buf.truncate(n);
                st.datagram = Some((buf, ip, port));
            }
            None => return Ok(false),
        }
    }
    Ok(true)
}

pub(crate) fn recv_from(sock: &Sock, out: &mut [u8], peek: bool) -> Result<(usize, IpAddr, u16), i32> {
    if sock.kind != abi::SOCK_DGRAM {
        let n = recv(sock, out, peek);
        let peer = sock.state().peer.unwrap_or((unspecified(sock.family), 0));
        return if n < 0 { Err(n as i32) } else { Ok((n as usize, peer.0, peer.1)) };
    }
    if sock.state().socket.is_none() { return Err(-errno::EINVAL); }
    wait(sock, |st| st.receive_timeout, |st| {
        match next_datagram(st) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(e) => return Some(Err(e)),
        }
        let (data, ip, port) = st.datagram.as_ref()?;
        let n = out.len().min(data.len());
        out[..n].copy_from_slice(&data[..n]);
        let answer = (n, *ip, *port);
        if !peek { st.datagram = None; }
        Some(Ok(answer))
    })
}

pub(crate) fn shutdown(sock: &Sock, how: i32) -> i32 {
    let mut st = sock.state();
    if st.role != Role::Stream { return -errno::ENOTCONN; }
    if how == abi::SHUT_RD || how == abi::SHUT_RDWR { st.read_shut = true; }
    if (how == abi::SHUT_WR || how == abi::SHUT_RDWR) && !st.write_shut {
        st.write_shut = true;
        if let Some(Err(e)) = st.socket.as_ref().map(|s| s.shutdown()) { return errno_of(e); }
    }
    0
}

pub(crate) unsafe fn peer_name(sock: &Sock, addr: *mut abi::sockaddr, len: *mut abi::socklen_t) -> i32 {
    match sock.state().peer { Some((ip, port)) => { unsafe { write_address(addr, len, &ip, port) }; 0 } None => -errno::ENOTCONN }
}
/// The local port; the address is "any" (a program has no say in which it uses).
pub(crate) unsafe fn sock_name(sock: &Sock, addr: *mut abi::sockaddr, len: *mut abi::socklen_t) -> i32 {
    let port = sock.state().local_port;
    unsafe { write_address(addr, len, &unspecified(sock.family), port) };
    0
}

fn timeout_ms(value: *const core::ffi::c_void, len: abi::socklen_t) -> Option<Option<u64>> {
    if (len as usize) < core::mem::size_of::<abi::timeval>() || value.is_null() { return None; }
    let t = unsafe { *(value as *const abi::timeval) };
    let ms = (t.tv_sec as u64).saturating_mul(1000).saturating_add(t.tv_usec as u64 / 1000);
    Some(if t.tv_sec == 0 && t.tv_usec == 0 { None } else { Some(ms.max(1)) })
}

pub(crate) fn set_option(sock: &Sock, level: i32, name: i32, value: *const core::ffi::c_void, len: abi::socklen_t) -> i32 {
    let mut st = sock.state();
    match (level, name) {
        (abi::SOL_SOCKET, abi::SO_RCVTIMEO) => match timeout_ms(value, len) { Some(t) => { st.receive_timeout = t; 0 } None => -errno::EINVAL },
        (abi::SOL_SOCKET, abi::SO_SNDTIMEO) => match timeout_ms(value, len) { Some(t) => { st.send_timeout = t; 0 } None => -errno::EINVAL },
        _ => {
            if value.is_null() || (len as usize) < 4 { return -errno::EINVAL; }
            let v = unsafe { *(value as *const i32) };
            st.options.retain(|(l, n, _)| (*l, *n) != (level, name));
            st.options.push((level, name, v));
            0
        }
    }
}

pub(crate) fn get_option(sock: &Sock, level: i32, name: i32, value: *mut core::ffi::c_void, len: *mut abi::socklen_t) -> i32 {
    if value.is_null() || len.is_null() { return -errno::EFAULT; }
    let mut st = sock.state();
    let room = unsafe { *len } as usize;
    match (level, name) {
        (abi::SOL_SOCKET, abi::SO_RCVTIMEO | abi::SO_SNDTIMEO) => {
            if room < core::mem::size_of::<abi::timeval>() { return -errno::EINVAL; }
            let ms = if name == abi::SO_RCVTIMEO { st.receive_timeout } else { st.send_timeout }.unwrap_or(0);
            unsafe {
                *(value as *mut abi::timeval) = abi::timeval { tv_sec: (ms / 1000) as _, tv_usec: ((ms % 1000) * 1000) as _ };
                *len = core::mem::size_of::<abi::timeval>() as abi::socklen_t;
            }
            0
        }
        (abi::SOL_SOCKET, abi::SO_LINGER) => {
            if room < core::mem::size_of::<abi::linger>() { return -errno::EINVAL; }
            let on = st.options.iter().find(|o| (o.0, o.1) == (level, name)).map_or(0, |o| o.2);
            unsafe { *(value as *mut abi::linger) = abi::linger { l_onoff: on, l_linger: 0 }; *len = 8; }
            0
        }
        _ => {
            if room < 4 { return -errno::EINVAL; }
            let v = if (level, name) == (abi::SOL_SOCKET, abi::SO_ERROR) { core::mem::take(&mut st.error) }
                else { st.options.iter().find(|o| (o.0, o.1) == (level, name)).map_or(0, |o| o.2) };
            unsafe { *(value as *mut i32) = v; *len = 4; }
            0
        }
    }
}

pub(crate) fn set_nonblocking(sock: &Sock, on: bool) { sock.state().nonblocking = on; }
pub(crate) fn nonblocking(sock: &Sock) -> bool { sock.state().nonblocking }

/// What `poll` would report now for `events`.
pub(crate) fn ready(sock: &Sock, events: i16) -> i16 {
    let mut st = sock.state();
    let mut got = 0i16;
    match st.role {
        Role::Connecting => match finish_connect(&mut st) {
            None => {}
            Some(Ok(_)) => got |= abi::POLLOUT & events,
            Some(Err(_)) => got |= abi::POLLHUP | abi::POLLERR,
        },
        Role::Stream => {
            if events & abi::POLLIN != 0 && st.inbox.is_empty() && !st.peer_closed { let _ = fill(&mut st); }
            if !st.inbox.is_empty() || st.peer_closed || st.read_shut { got |= abi::POLLIN & events; }
            if st.peer_closed { got |= abi::POLLRDHUP & events; }
            if !st.write_shut { got |= abi::POLLOUT & events; }
        }
        Role::Listening => {
            // A waiting connection is only found by taking it: keep it for `accept`.
            if events & abi::POLLIN != 0 && st.pending.is_none() {
                if let Some(Ok(Some(found))) = st.socket.as_ref().map(|s| s.accept()) { st.pending = Some(found); }
            }
            if st.pending.is_some() { got |= abi::POLLIN & events; }
        }
        Role::Udp => {
            if events & abi::POLLIN != 0 { let _ = next_datagram(&mut st); }
            if st.datagram.is_some() { got |= abi::POLLIN & events; }
            got |= abi::POLLOUT & events;
        }
        Role::Fresh => got |= abi::POLLOUT & events,
    }
    got
}

// ---- names ------------------------------------------------------------------------------

/// The addresses of `host`, waiting for the answer (thirty seconds at most).
pub(crate) fn lookup(host: &[u8]) -> Result<Vec<IpAddr>, i32> {
    // An address is itself, network or not.
    if let Some(ip) = IpAddr::parse(host) { return Ok(vec![ip]); }
    let began = sex_pdx::get_ticks();
    let mut round = 0;
    // The network must be up before a name can be asked.
    match network_up() {
        Ok(()) => {}
        Err(e) if e == -errno::EACCES => return Err(abi::EAI_FAIL),
        Err(_) => return Err(abi::EAI_AGAIN),
    }
    let mut asked = false;
    loop {
        match direct::resolve_step(host, &mut asked) {
            Ok(Some(found)) => return Ok(found),
            Ok(None) | Err(NetError::Busy) => {}
            Err(NetError::NoName) => return Err(abi::EAI_NONAME),
            Err(NetError::TooSlow) => return Err(abi::EAI_AGAIN),
            Err(_) => return Err(abi::EAI_FAIL),
        }
        if sex_pdx::get_ticks().saturating_sub(began) > 30_000 { direct::resolve_cancel(&mut asked); return Err(abi::EAI_AGAIN); }
        pause(&mut round);
    }
}
