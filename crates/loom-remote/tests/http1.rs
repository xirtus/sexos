// SPDX-License-Identifier: GPL-3.0-only
//! The HTTP/1.1 client's edges, against scripted raw-socket servers.
mod common;
use common::*;
use loom_remote::http::{Body, Http, Http1, Request};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

/// Serves each connection one canned reply per request, `per_conn` requests, then
/// closes it without warning. Gives the port and a log of request heads+bodies.
fn server(replies: Vec<&'static [u8]>, per_conn: usize) -> (u16, std::sync::mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut replies = replies.into_iter();
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            for _ in 0..per_conn {
                let mut head = String::new();
                loop { let mut line = String::new(); if r.read_line(&mut line).unwrap() == 0 { break; } head.push_str(&line); if line == "\r\n" { break; } }
                if head.is_empty() { break; }
                let lower = head.to_ascii_lowercase();
                if let Some(n) = lower.lines().find_map(|l| l.strip_prefix("content-length: ")) {
                    let mut b = vec![0; n.trim().parse().unwrap()]; r.read_exact(&mut b).unwrap(); head.push_str(&String::from_utf8_lossy(&b));
                } else if lower.contains("transfer-encoding: chunked") {
                    loop {
                        let mut l = String::new(); r.read_line(&mut l).unwrap();
                        let n = usize::from_str_radix(l.trim(), 16).unwrap();
                        let mut b = vec![0; n + 2]; r.read_exact(&mut b).unwrap();
                        head.push_str(&String::from_utf8_lossy(&b[..n]));
                        if n == 0 { break; }
                    }
                }
                tx.send(head).unwrap();
                let Some(reply) = replies.next() else { return };
                s.write_all(reply).unwrap();
            }
        }
    });
    (port, rx)
}

#[test]
fn reconnects_when_a_kept_alive_connection_was_closed() {
    let (port, rx) = server(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\na", b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nb", b"HTTP/1.1 204 No Content\r\n\r\n"], 1);
    let mut h = Http1::new(TcpConnect(port), "example.test");
    for want in [&b"a"[..], b"b"] {
        let r = h.send(&Request { method: "GET", target: "/x", headers: vec![] }, Body::Empty, 100).unwrap();
        assert_eq!(r.body, want);
    }
    let r = h.send(&Request { method: "DELETE", target: "/x", headers: vec![] }, Body::Empty, 100).unwrap();
    assert_eq!(r.status, 204);
    assert!(rx.recv().unwrap().contains("host: example.test\r\n"));
}

#[test]
fn chunked_both_ways_continue_and_truncation() {
    let (port, rx) = server(vec![
        b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\nX-A:  b \r\n\r\n3;ext=1\r\nabc\r\n2\r\nde\r\n0\r\nTrailer: x\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789",
        b"HTTP/1.0 200 OK\r\n\r\nuntil close",
    ], 3);
    let mut h = Http1::new(TcpConnect(port), "h");
    let data = pattern(200_000, 3);
    let r = h.send(&Request { method: "PUT", target: "/up", headers: vec![] }, Body::Stream { src: &mut Trickle(&data, 0), len: None }, 100).unwrap();
    assert_eq!((r.status, r.header("x-a"), r.body.as_slice()), (201, Some("b"), &b"abcde"[..]));
    let got = rx.recv().unwrap();
    assert!(got.starts_with("PUT /up HTTP/1.1\r\n") && got.ends_with(&*String::from_utf8_lossy(&data)));
    let r = h.send(&Request { method: "GET", target: "/t", headers: vec![] }, Body::Empty, 4).unwrap();
    assert_eq!((r.body.as_slice(), r.truncated), (&b"0123"[..], true));
    // Truncation dropped that connection; the next request opens another.
    let (port2, _rx2) = server(vec![b"HTTP/1.0 200 OK\r\n\r\nuntil close"], 1);
    let mut h2 = Http1::new(TcpConnect(port2), "h");
    let r = h2.send(&Request { method: "GET", target: "/c", headers: vec![] }, Body::Empty, 100).unwrap();
    assert_eq!(r.body, b"until close");
}
