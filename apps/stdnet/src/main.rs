// SPDX-License-Identifier: GPL-3.0-only
//! An ordinary Rust program that uses `std::net` and `SystemTime`, unchanged from what
//! it would be elsewhere. `run +net stdnet HOST ECHO_PORT LISTEN_PORT` talks to a TCP
//! and UDP echo service on HOST:ECHO_PORT and takes one connection on LISTEN_PORT,
//! answering `ping` with `pong`. The last line is the score the program gate reads.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sex_std as _;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let host = args.get(1).cloned().unwrap_or_else(|| "10.0.2.2".into());
    let echo: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(7);
    let listen: u16 = args.get(3).and_then(|p| p.parse().ok()).unwrap_or(8080);
    let mut passed = 0;
    let mut total = 0;
    let mut check = |name: &str, ok: bool| {
        total += 1;
        if ok { passed += 1; }
        println!("{} {name}", if ok { "ok  " } else { "FAIL" });
    };

    // The real clock: the gate compares this with the host's.
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("civil seconds: {now}");
    sex_std::record(&format!("[stdnet.clock] seconds={now}"));
    check("SystemTime is today's", now > 1_735_689_600);

    let target = format!("{host}:{echo}");
    let resolved: Vec<_> = target.to_socket_addrs().map(|a| a.collect()).unwrap_or_default();
    check("to_socket_addrs", !resolved.is_empty());

    // TCP: connect, write, read the echo back, then the peer's close.
    let mut stream = TcpStream::connect(&*target);
    if let Err(e) = &stream { println!("connect {target}: {e}"); }
    check("TcpStream::connect", stream.is_ok());
    if let Ok(s) = stream.as_mut() {
        let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
        let said = s.write_all(b"hello over TCP\n").is_ok();
        let mut line = String::new();
        let mut reader = BufReader::new(s.try_clone().expect("clone"));
        let heard = reader.read_line(&mut line).is_ok();
        check("TCP echo", said && heard && line == "hello over TCP\n");
        check("peer_addr", s.peer_addr().is_ok_and(|a| a.port() == echo));

        // One thread writes while this one reads, on the same connection.
        let mut writer = s.try_clone().expect("clone");
        let sender = std::thread::spawn(move || {
            for i in 0..50 {
                if let Err(e) = writer.write_all(format!("line {i}\n").as_bytes()) { return Err(format!("write {i}: {e}")); }
            }
            writer.shutdown(Shutdown::Write).map_err(|e| format!("shutdown: {e}"))
        });
        let mut got = 0;
        for i in 0..50 {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(_) if line == format!("line {i}\n") => got += 1,
                Ok(_) => println!("line {i} read as {line:?}"),
                Err(e) => println!("line {i}: {e}"),
            }
        }
        let sent = sender.join().unwrap_or_else(|_| Err("writer panicked".into()));
        if let Err(e) = &sent { println!("writer: {e}"); }
        println!("lines read back: {got} of 50");
        check("TCP from two threads", sent.is_ok() && got == 50);
        let mut rest = Vec::new();
        check("end of stream", reader.read_to_end(&mut rest).is_ok() && rest.is_empty());
    }

    // A read with nothing coming times out rather than waiting forever.
    if let Ok(mut s) = TcpStream::connect(&*target) {
        let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        let mut buf = [0u8; 8];
        let e = s.read(&mut buf).err().map(|e| e.kind());
        check("read timeout", matches!(e, Some(std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)));
    } else { check("read timeout", false); }

    // Non-blocking: nothing to read is WouldBlock, not a wait.
    if let Ok(s) = TcpStream::connect(&*target) {
        let nb = s.set_nonblocking(true).is_ok();
        let e = (&s).read(&mut [0u8; 8]).err().map(|e| e.kind());
        check("non-blocking read", nb && e == Some(std::io::ErrorKind::WouldBlock));
    } else { check("non-blocking read", false); }

    // UDP: a datagram there and back.
    let udp = UdpSocket::bind("0.0.0.0:0");
    check("UdpSocket::bind", udp.is_ok());
    if let Ok(u) = udp {
        let _ = u.set_read_timeout(Some(Duration::from_secs(10)));
        let mut answer = None;
        // UDP may drop the first datagram while the route is learned; try a few times.
        for _ in 0..5 {
            if u.send_to(b"hello over UDP", &*target).is_err() { break; }
            let mut buf = [0u8; 64];
            if let Ok((n, from)) = u.recv_from(&mut buf) { answer = Some((buf[..n].to_vec(), from)); break; }
        }
        check("UDP echo", answer.as_ref().is_some_and(|(b, from)| b == b"hello over UDP" && from.port() == echo));
        let local = u.local_addr().map(|a| a.port()).unwrap_or(0);
        check("UDP local port", local != 0);
    }

    // A port nothing listens on is refused, within the time given.
    let closed = format!("{host}:1").to_socket_addrs().ok().and_then(|mut a| a.next());
    let refused = closed.map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(10)).is_err());
    check("refused connection", refused == Some(true));

    // A listener: the gate connects through the forwarded port and says ping.
    match TcpListener::bind(("0.0.0.0", listen)) {
        Ok(l) => {
            check("TcpListener::bind", l.local_addr().is_ok_and(|a| a.port() == listen));
            sex_std::record(&format!("[stdnet.listening] port={listen}"));
            match l.accept() {
                Ok((mut c, from)) => {
                    let mut buf = [0u8; 4];
                    let ok = c.read_exact(&mut buf).is_ok() && &buf == b"ping" && c.write_all(b"pong").is_ok();
                    let _ = c.shutdown(Shutdown::Write);
                    println!("accepted from {from}");
                    check("accept and answer", ok);
                }
                Err(e) => { println!("accept: {e}"); check("accept and answer", false); }
            }
        }
        Err(e) => { println!("bind: {e}"); check("TcpListener::bind", false); check("accept and answer", false); }
    }

    println!("passed {passed} of {total}");
    sex_std::record(&format!("[stdnet.{}] passed={passed}/{total}", if passed == total { "ok" } else { "fail" }));
}
