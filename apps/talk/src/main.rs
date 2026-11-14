// SPDX-License-Identifier: GPL-3.0-only
//! Flirt (formerly Talk): TCP and UDP by hand, over IPv4 or IPv6. `run +net flirt tcp HOST PORT TEXT`
//! sends a line and shows the reply; `udp HOST PORT TEXT` does the same with one
//! datagram; `listen PORT` answers one connection by echoing it; `addresses` shows the
//! machine's addresses; `check ADDRESS PORT` does all of these against an echo server.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use core::fmt::Write;
use sex_app::net::{self, IpAddr, TcpListener, TcpStream, UdpSocket};
use sex_app::{files, Event, Window};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[talk.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// What the window shows, built up a line at a time.
struct Screen(alloc::string::String);
impl Screen {
    fn line(&mut self, window: &mut Window, text: &str) {
        self.0.push_str(text);
        self.0.push('\n');
        window.show(self.0.as_bytes());
    }
}

const WAIT_MS: u64 = 15_000;

fn words(args: &[u8]) -> Vec<&[u8]> { args.split(|b| *b == b' ').filter(|w| !w.is_empty()).collect() }
fn number(w: &[u8]) -> Option<u16> { core::str::from_utf8(w).ok()?.parse().ok() }
fn text(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }

/// Sends `say` over TCP and reads until the peer has sent as many bytes back or closed.
fn tcp(window: &mut Window, host: &[u8], port: u16, say: &[u8], reply: &mut [u8]) -> Result<(IpAddr, usize), net::Error> {
    let mut stream = TcpStream::connect(window, host, port)?;
    let peer = stream.peer().0;
    stream.write_all(window, say)?;
    let mut got = 0;
    while got < say.len().min(reply.len()) {
        let n = stream.read_within(window, &mut reply[got..], WAIT_MS)?;
        if n == 0 { break; }
        got += n;
    }
    stream.close(window);
    Ok((peer, got))
}

fn udp(window: &mut Window, host: &[u8], port: u16, say: &[u8], reply: &mut [u8]) -> Result<(IpAddr, u16, usize), net::Error> {
    let to = *net::resolve(window, host)?.first().ok_or(net::Error::NoName)?;
    let mut socket = UdpSocket::bind(window, 0)?;
    let began = sex_pdx::get_ticks();
    while !socket.send_to(window, say, &to, port)? {
        if sex_pdx::get_ticks() - began > WAIT_MS { socket.close(window); return Err(net::Error::TooSlow); }
        sex_pdx::sys_yield();
    }
    let r = socket.recv_from_within(window, reply, WAIT_MS);
    socket.close(window);
    let (n, from, from_port) = r?;
    Ok((from, from_port, n))
}

/// Takes one connection on `port` and echoes it back until the peer closes.
fn echo_one(window: &mut Window, listener: &mut TcpListener) -> Result<(IpAddr, u16, usize), net::Error> {
    let mut stream = listener.accept_within(window, 60_000)?;
    let (peer, peer_port) = stream.peer();
    let mut buf = [0u8; 2048];
    let mut total = 0;
    loop {
        let n = stream.read_within(window, &mut buf, 60_000)?;
        if n == 0 { break; }
        stream.write_all(window, &buf[..n])?;
        total += n;
    }
    stream.close(window);
    Ok((peer, peer_port, total))
}

fn addresses(window: &mut Window, screen: &mut Screen) -> Result<Vec<(IpAddr, u8)>, net::Error> {
    net::up(window)?;
    let list = net::addresses(window)?;
    for (a, prefix) in &list {
        let mut s = alloc::string::String::new();
        let _ = write!(s, "address {}/{}", a, prefix);
        screen.line(window, &s);
        sex_pdx::serial_record!("[talk.address] {}/{}", a, prefix);
    }
    Ok(list)
}

fn run(window: &mut Window, screen: &mut Screen, w: &[&[u8]]) -> Result<(), net::Error> {
    let mut reply = [0u8; 2048];
    let mut s = alloc::string::String::new();
    match w {
        [b"addresses"] => { addresses(window, screen)?; }
        [b"tcp", host, port, say @ ..] if !say.is_empty() => {
            let port = number(port).ok_or(net::Error::Address)?;
            let line: Vec<u8> = say.join(&b' ').into_iter().chain(*b"\n").collect();
            let (peer, n) = tcp(window, host, port, &line, &mut reply)?;
            let _ = write!(s, "{} said: {}", peer, text(&reply[..n]));
            screen.line(window, &s);
        }
        [b"udp", host, port, say @ ..] if !say.is_empty() => {
            let port = number(port).ok_or(net::Error::Address)?;
            let line = say.join(&b' ');
            let (from, from_port, n) = udp(window, host, port, &line, &mut reply)?;
            let _ = write!(s, "[{}]:{} said: {}", from, from_port, text(&reply[..n]));
            screen.line(window, &s);
        }
        [b"listen", port] => {
            let port = number(port).ok_or(net::Error::Address)?;
            net::up(window)?;
            let mut listener = TcpListener::bind(window, port)?;
            let _ = write!(s, "listening on port {}", listener.port());
            screen.line(window, &s);
            sex_pdx::serial_record!("[talk.listening] port={}", listener.port());
            let (peer, peer_port, n) = echo_one(window, &mut listener)?;
            listener.close(window);
            s.clear();
            let _ = write!(s, "[{}]:{} sent {} bytes, echoed", peer, peer_port, n);
            screen.line(window, &s);
        }
        // Everything above against one echo server (TCP and UDP on `port`), then a
        // connection taken on port 7070. The QEMU gate drives it.
        [b"check", address, port] => {
            let port = number(port).ok_or(net::Error::Address)?;
            addresses(window, screen)?;
            let say = b"hello over the network, by TCP";
            let (peer, n) = tcp(window, address, port, say, &mut reply)?;
            sex_pdx::serial_record!("[talk.tcp] peer={} sent={} echoed={} match={}", peer, say.len(), n, (&reply[..n] == say) as u8);
            let _ = write!(s, "tcp {} echoed {} bytes", peer, n); screen.line(window, &s); s.clear();
            let say = b"hello by UDP";
            let (from, from_port, n) = udp(window, address, port, say, &mut reply)?;
            sex_pdx::serial_record!("[talk.udp] peer={} port={} sent={} echoed={} match={}", from, from_port, say.len(), n, (&reply[..n] == say) as u8);
            let _ = write!(s, "udp {} echoed {} bytes", from, n); screen.line(window, &s); s.clear();
            let mut listener = TcpListener::bind(window, 7070)?;
            sex_pdx::serial_record!("[talk.listening] port={}", listener.port());
            let (peer, peer_port, n) = echo_one(window, &mut listener)?;
            listener.close(window);
            sex_pdx::serial_record!("[talk.accepted] peer={} port={} echoed={}", peer, peer_port, n);
            let _ = write!(s, "accepted {} and echoed {} bytes", peer, n); screen.line(window, &s);
        }
        _ => screen.line(window, "Usage: run +net flirt (addresses | tcp HOST PORT TEXT | udp HOST PORT TEXT | listen PORT | check ADDRESS PORT)"),
    }
    Ok(())
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Flirt");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let args = files::args(&mut window, &mut args).to_vec();
    let mut screen = Screen(alloc::string::String::from("Talk. F11 closes.\n"));
    screen.line(&mut window, "");
    match run(&mut window, &mut screen, &words(&args)) {
        Ok(()) => sex_pdx::serial_record!("[talk.done] target={} task={}", window.target, window.task),
        Err(e) => {
            sex_pdx::serial_record!("[talk.failed] error={:?}", e);
            screen.line(&mut window, text(net::message(e)));
        }
    }
    loop {
        if let Event::Close = window.next() { window.close() }
    }
}
