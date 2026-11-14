// SPDX-License-Identifier: GPL-3.0-only
//! WebDAV against a Python test server laid out like Nextcloud.
mod common;
use common::*;
use loom_remote::http::{Http1, H2};
use loom_remote::webdav::{Auth, WebDav};
use loom_remote::{Error, Place};

const BASE: &str = "/remote.php/dav/files/alice";

fn auth() -> Auth { Auth::Basic { user: "alice".into(), password: "s3cret pass".into() } }

#[test]
fn webdav_over_http1_does_everything_a_place_does() {
    let root = scratch("webdav");
    let Some((_server, port)) = python_server("webdav_server.py", &root, &[]) else { return };
    let mut d = WebDav::new(Http1::new(TcpConnect(port), &format!("127.0.0.1:{port}")), "http", BASE, auth());
    exercise(&mut d, 3 * 1024 * 1024 + 4321);
    assert_eq!(std::fs::read(root.join("docs/renamed.txt")).unwrap(), b"hello, remote world\n");
    assert_eq!(std::fs::read(root.join("docs/big.bin")).unwrap(), pattern(3 * 1024 * 1024 + 4321, 42));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn webdav_refuses_a_wrong_password_and_lists_files_made_elsewhere() {
    let root = scratch("webdav-auth");
    std::fs::create_dir_all(root.join("Photos/2026")).unwrap();
    std::fs::write(root.join("Photos/cat & dog.jpg"), b"jpeg").unwrap();
    let Some((_server, port)) = python_server("webdav_server.py", &root, &[]) else { return };
    let mut bad = WebDav::new(Http1::new(TcpConnect(port), &format!("127.0.0.1:{port}")), "http", BASE, Auth::Basic { user: "alice".into(), password: "nope".into() });
    assert_eq!(bad.list(""), Err(Error::Denied));
    let mut d = WebDav::new(Http1::new(TcpConnect(port), &format!("127.0.0.1:{port}")), "http", &format!("{BASE}/"), auth());
    let l = d.list("Photos").unwrap();
    assert_eq!(l.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["2026", "cat & dog.jpg"]);
    assert_eq!(l[1].size, 4);
    assert_eq!(d.list("Photos/cat & dog.jpg"), Err(Error::BadPath));
    assert_eq!(d.list("Nothing"), Err(Error::NotFound));
    let _ = std::fs::remove_dir_all(&root);
}

/// The same place over HTTP/2 (sex-http's client), through an in-test h2 front end
/// that hands each request to the Python server.
#[test]
fn webdav_over_sex_http_h2() {
    let root = scratch("webdav-h2");
    let Some((_server, port)) = python_server("webdav_server.py", &root, &[]) else { return };
    let front = h2front::start(port);
    let conn = { use loom_remote::Connect; TcpConnect(front).connect().unwrap() };
    let mut d = WebDav::new(H2::new(conn, "http", &format!("127.0.0.1:{port}")), "http", BASE, auth());
    exercise(&mut d, 1024 * 1024 + 99);
    assert_eq!(std::fs::read(root.join("docs/big.bin")).unwrap(), pattern(1024 * 1024 + 99, 42));
    let _ = std::fs::remove_dir_all(&root);
}

/// A minimal HTTP/2 server (prior knowledge, cleartext) that forwards to an HTTP/1.1
/// backend with loom-remote's own client. Only what sex-http's client sends is handled.
mod h2front {
    use super::TcpConnect;
    use loom_remote::http::{Body, Http, Http1, Request};
    use sex_http::h2::{hpack, PREFACE};
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn frame(out: &mut Vec<u8>, kind: u8, flags: u8, stream: u32, payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
        out.push(kind);
        out.push(flags);
        out.extend_from_slice(&stream.to_be_bytes());
        out.extend_from_slice(payload);
    }

    pub fn start(backend: u16) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || { for s in l.incoming() { let s = s.unwrap(); std::thread::spawn(move || serve(s, backend)); } });
        port
    }

    fn read_exact(s: &mut TcpStream, n: usize) -> Option<Vec<u8>> { let mut b = vec![0; n]; s.read_exact(&mut b).ok()?; Some(b) }

    fn serve(mut s: TcpStream, backend: u16) {
        if read_exact(&mut s, PREFACE.len()).as_deref() != Some(PREFACE) { return; }
        let mut out = Vec::new();
        // Our SETTINGS: big initial window so uploads flow; 16 KiB frames (the default).
        let mut set = Vec::new();
        set.extend_from_slice(&4u16.to_be_bytes());
        set.extend_from_slice(&(1u32 << 30).to_be_bytes());
        frame(&mut out, 4, 0, 0, &set);
        frame(&mut out, 8, 0, 0, &((1u32 << 30) - 65535).to_be_bytes());
        s.write_all(&out).unwrap();
        let mut dec = hpack::Decoder::new(4096, 1 << 20);
        let mut http = Http1::new(TcpConnect(backend), &format!("127.0.0.1:{backend}"));
        let mut reqs: HashMap<u32, (Vec<(Vec<u8>, Vec<u8>)>, Vec<u8>)> = HashMap::new();
        let mut granted = 0usize;
        loop {
            let Some(h) = read_exact(&mut s, 9) else { return };
            let len = u32::from_be_bytes([0, h[0], h[1], h[2]]) as usize;
            let (kind, flags, stream) = (h[3], h[4], u32::from_be_bytes([h[5], h[6], h[7], h[8]]) & 0x7fff_ffff);
            let Some(p) = read_exact(&mut s, len) else { return };
            let mut out = Vec::new();
            let mut end = false;
            match kind {
                4 if flags & 1 == 0 => frame(&mut out, 4, 1, 0, &[]),
                1 => {
                    assert!(flags & 4 != 0, "CONTINUATION not handled by this test server");
                    let mut block = &p[..];
                    if flags & 8 != 0 { let pad = block[0] as usize; block = &block[1..block.len() - pad]; }
                    if flags & 0x20 != 0 { block = &block[5..]; }
                    reqs.insert(stream, (dec.decode(block).unwrap(), Vec::new()));
                    end = flags & 1 != 0;
                }
                0 => {
                    reqs.get_mut(&stream).unwrap().1.extend_from_slice(&p);
                    granted += p.len();
                    end = flags & 1 != 0;
                    if granted > 1 << 29 { frame(&mut out, 8, 0, 0, &(granted as u32).to_be_bytes()); granted = 0; }
                }
                7 => return,
                _ => {}
            }
            if end {
                let (hs, body) = reqs.remove(&stream).unwrap();
                let get = |n: &[u8]| hs.iter().find(|(k, _)| k == n).map(|(_, v)| String::from_utf8(v.clone()).unwrap()).unwrap();
                let (method, path) = (get(b":method"), get(b":path"));
                let headers = hs.iter().filter(|(k, _)| !k.starts_with(b":") && k != b"content-length")
                    .map(|(k, v)| (String::from_utf8(k.clone()).unwrap(), String::from_utf8(v.clone()).unwrap())).collect();
                let r = http.send(&Request { method: &method, target: &path, headers }, if body.is_empty() { Body::Empty } else { Body::Bytes(&body) }, usize::MAX).unwrap();
                let status = r.status.to_string();
                let mut list: Vec<(&[u8], &[u8])> = vec![(b":status", status.as_bytes())];
                for (k, v) in &r.headers { if !matches!(k.as_str(), "transfer-encoding" | "connection" | "keep-alive") { list.push((k.as_bytes(), v.as_bytes())); } }
                let mut block = Vec::new();
                hpack::encode(&mut block, &list);
                let none = r.body.is_empty() || method == "HEAD";
                frame(&mut out, 1, 4 | if none { 1 } else { 0 }, stream, &block);
                if !none {
                    let n = r.body.len().div_ceil(16384);
                    for (i, c) in r.body.chunks(16384).enumerate() { frame(&mut out, 0, if i + 1 == n { 1 } else { 0 }, stream, c); }
                }
            }
            if !out.is_empty() && s.write_all(&out).is_err() { return; }
        }
    }
}
