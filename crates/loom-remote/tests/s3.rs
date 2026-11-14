// SPDX-License-Identifier: GPL-3.0-only
//! S3 against a Python mock that checks every SigV4 signature independently, and
//! against MinIO when a `minio` binary is on PATH.
mod common;
use common::*;
use loom_remote::http::{Body, Http, Http1, Request};
use loom_remote::s3::{Config, S3};
use loom_remote::sigv4::{self, Credentials, Scope};
use loom_remote::{Error, Kind, Place};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() }
fn creds() -> Credentials { Credentials::new("LOOMTESTKEY", "loom/test+secret/KEY") }
fn place(port: u16, cfg: Config, creds: Credentials) -> S3<Http1<TcpConnect>> {
    S3::new(Http1::new(TcpConnect(port), &format!("127.0.0.1:{port}")), cfg, creds, Box::new(now))
}
fn stats(port: u16) -> String {
    let mut h = Http1::new(TcpConnect(port), "127.0.0.1");
    String::from_utf8(h.send(&Request { method: "GET", target: "/__stats", headers: vec![] }, Body::Empty, 1 << 20).unwrap().body).unwrap()
}
fn stat_num(s: &str, name: &str) -> u64 {
    let i = s.find(&format!("\"{name}\": ")).unwrap() + name.len() + 4;
    s[i..].split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap()
}

#[test]
fn s3_does_everything_a_place_does_with_signed_requests() {
    let dir = scratch("s3");
    let Some((_server, port)) = python_server("s3_server.py", &dir, &[]) else { return };
    let mut s = place(port, Config::new("loom-bucket", "us-east-1"), creds());
    // Listings come 3 keys a page from the mock, so paging is exercised throughout.
    exercise(&mut s, 3 * 1024 * 1024 + 1);
    let st = stats(port);
    assert_eq!(stat_num(&st, "bad_signature"), 0, "{st}");
    assert!(stat_num(&st, "copy") >= 3, "renames copy: {st}");
    // Many entries in one folder, over several pages.
    for i in 0..10 { s.write(&format!("many/f{i:02}"), &mut &b"z"[..], Some(1)).unwrap(); }
    s.mkdir("many/sub").unwrap();
    let l = s.list("many").unwrap();
    assert_eq!(l.len(), 11);
    assert_eq!((l[0].name.as_str(), l[0].kind), ("sub", Kind::Dir));
    assert_eq!(l[10].name, "f09");
    // A folder rename moves every object under it.
    s.rename("many", "lots").unwrap();
    assert_eq!(s.list("lots").unwrap().len(), 11);
    assert_eq!(s.stat("many"), Err(Error::NotFound));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s3_uploads_over_16_mib_in_parts() {
    let dir = scratch("s3-multi");
    let Some((_server, port)) = python_server("s3_server.py", &dir, &["1000"]) else { return };
    let mut s = place(port, Config::new("loom-bucket", "us-east-1"), creds());
    let exactly = pattern(16 << 20, 7);
    assert_eq!(s.write("exact.bin", &mut Trickle(&exactly, 3), None).unwrap(), 16 << 20);
    assert_eq!(stat_num(&stats(port), "multipart_complete"), 0, "16 MiB is one PUT");
    let big = pattern((16 << 20) + (1 << 20) + 5, 9);
    assert_eq!(s.write("big.bin", &mut &big[..], Some(big.len() as u64)).unwrap(), big.len() as u64);
    let st = stats(port);
    assert_eq!((stat_num(&st, "multipart_complete"), stat_num(&st, "parts")), (1, 2), "{st}");
    assert_eq!(s.stat("big.bin").unwrap().size, big.len() as u64);
    assert_eq!(s.read("big.bin", (16 << 20) - 10, 30).unwrap(), &big[(16 << 20) - 10..(16 << 20) + 20]);
    // A source failing in the second part aborts the upload and leaves the old object.
    let r = s.write("exact.bin", &mut Failing { data: &big, fail_after: (16 << 20) + 100, at: 0 }, None);
    assert_eq!(r, Err(Error::Source));
    let st = stats(port);
    assert_eq!(stat_num(&st, "uploads_open"), 0, "{st}");
    assert_eq!(s.read("exact.bin", 0, 64).unwrap(), &exactly[..64]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s3_wrong_secret_is_refused_and_prefix_scopes_the_place() {
    let dir = scratch("s3-prefix");
    let Some((_server, port)) = python_server("s3_server.py", &dir, &[]) else { return };
    let mut bad = place(port, Config::new("loom-bucket", "us-east-1"), Credentials::new("LOOMTESTKEY", "wrong"));
    match bad.list("") { Err(Error::Remote(403, m)) => assert!(m.starts_with("SignatureDoesNotMatch"), "{m}"), r => panic!("{r:?}") }
    let mut root = place(port, Config::new("loom-bucket", "us-east-1"), creds());
    root.write("outside.txt", &mut &b"o"[..], None).unwrap();
    let mut cfg = Config::new("loom-bucket", "us-east-1");
    cfg.prefix = "users/alice".into();
    let mut s = place(port, cfg, creds().with_token("session-token-xyz"));
    s.write("notes/a.txt", &mut &b"alice"[..], None).unwrap();
    assert_eq!(s.list("").unwrap().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["notes"]);
    assert_eq!(root.read("users/alice/notes/a.txt", 0, 10).unwrap(), b"alice");
    assert_eq!(stat_num(&stats(port), "bad_signature"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// MinIO, when installed (CI: download the server binary); skipped otherwise.
#[test]
fn minio_round_trip_when_available() {
    let Some(minio) = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("minio")).find(|f| f.exists())) else { eprintln!("skipping: no minio"); return };
    let dir = scratch("minio");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = std::process::Command::new(minio).args(["server", "--quiet", "--address", &format!("127.0.0.1:{port}")]).arg(dir.join("data"))
        .env("MINIO_ROOT_USER", "loomadmin").env("MINIO_ROOT_PASSWORD", "loom-secret-123").stdout(std::process::Stdio::null()).spawn().unwrap();
    let _server = Server(child);
    let mut http = Http1::new(TcpConnect(port), &format!("127.0.0.1:{port}"));
    let c = Credentials::new("loomadmin", "loom-secret-123");
    // Create the bucket with a hand-signed PUT (Loom itself never creates buckets).
    let began = std::time::Instant::now();
    loop {
        let date = loom_remote::text::amz_date(now());
        let host = http.authority().to_string();
        let h = [("host", host.as_str()), ("x-amz-date", date.as_str()), ("x-amz-content-sha256", sigv4::EMPTY_SHA256)];
        let auth = sigv4::authorization(&c, &Scope { amz_date: &date, region: "us-east-1", service: "s3" }, "PUT", "/loom", "", &h, sigv4::EMPTY_SHA256);
        let headers = vec![("x-amz-date".into(), date.clone()), ("x-amz-content-sha256".into(), sigv4::EMPTY_SHA256.into()), ("authorization".into(), auth)];
        match http.send(&Request { method: "PUT", target: "/loom", headers }, Body::Empty, 1 << 16) {
            Ok(r) if r.ok() => break,
            r => { assert!(began.elapsed().as_secs() < 20, "minio did not come up: {:?}", r.map(|r| (r.status, r.text().to_string()))); std::thread::sleep(std::time::Duration::from_millis(200)); }
        }
    }
    let mut cfg = Config::new("loom", "us-east-1");
    cfg.part_size = 5 << 20;
    let mut s = S3::new(http, cfg, c, Box::new(now));
    exercise(&mut s, (11 << 20) + 3);
    let _ = std::fs::remove_dir_all(&dir);
}
