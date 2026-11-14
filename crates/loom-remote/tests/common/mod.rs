// SPDX-License-Identifier: GPL-3.0-only
//! What every place must do, run against real servers, plus std transports.
#![allow(dead_code)]
use loom_remote::{Connect, Entry, Error, Kind, Place, Result, Source, Stream};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A TCP connection as a loom-remote stream.
pub struct Tcp(pub TcpStream);
impl Stream for Tcp {
    fn write_all(&mut self, b: &[u8]) -> Result<()> { self.0.write_all(b).map_err(|e| Error::Transport(e.to_string())) }
    fn read(&mut self, b: &mut [u8]) -> Result<usize> { self.0.read(b).map_err(|e| Error::Transport(e.to_string())) }
}
pub struct TcpConnect(pub u16);
impl Connect for TcpConnect {
    fn connect(&mut self) -> Result<Box<dyn Stream>> {
        let s = TcpStream::connect(("127.0.0.1", self.0)).map_err(|e| Error::Transport(e.to_string()))?;
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        s.set_nodelay(true).unwrap();
        Ok(Box::new(Tcp(s)))
    }
}

pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loom-remote-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A child process killed when dropped.
pub struct Server(pub Child);
impl Drop for Server { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

pub fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()))
}

/// Starts a fixture server script (tests/fixtures/<script>) on a free port.
pub fn python_server(script: &str, root: &Path, extra: &[&str]) -> Option<(Server, u16)> {
    let Some(py) = python() else { eprintln!("skipping: no python3"); return None };
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new(py).arg("-I").arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(script))
        .arg(port.to_string()).arg(root).args(extra).stdin(Stdio::null()).spawn().unwrap();
    let s = Server(child);
    let began = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(began.elapsed() < Duration::from_secs(10), "{script} did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Some((s, port))
}

/// Deterministic bytes.
pub fn pattern(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x as u8 }).collect()
}

/// A source that gives `n` bytes of `data` in odd-sized pieces, then fails.
pub struct Failing<'a> { pub data: &'a [u8], pub fail_after: usize, pub at: usize }
impl Source for Failing<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.at >= self.fail_after { return Err(Error::Source); }
        let n = buf.len().min(7919).min(self.fail_after - self.at).min(self.data.len() - self.at);
        buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}
/// A source in small uneven pieces (exercises streaming and short reads).
pub struct Trickle<'a>(pub &'a [u8], pub usize);
impl Source for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.1 = (self.1 * 31 + 7) % 50021 + 1;
        let n = buf.len().min(self.1).min(self.0.len());
        buf[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}

fn names(v: &[Entry]) -> Vec<String> { v.iter().map(|e| e.name.clone()).collect() }
fn no_temp(v: &[Entry]) { assert!(v.iter().all(|e| !e.name.contains(".loom-")), "temporary file left: {:?}", names(v)); }

/// The behaviour Loom relies on, for any place. `now` is the server's clock (seconds).
pub fn exercise(p: &mut dyn Place, big: usize) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(p.list("").unwrap(), vec![]);
    // Folders.
    p.mkdir("docs").unwrap();
    assert_eq!(p.mkdir("docs"), Err(Error::Exists));
    p.mkdir("/docs/inner/").unwrap();
    assert_eq!(p.stat("docs").unwrap().kind, Kind::Dir);
    // A folder in a missing folder: refused, except on S3 where folders are only prefixes.
    if p.mkdir("nope/deeper").is_ok() { p.delete("nope/deeper").unwrap(); }

    // Small file with an awkward name.
    let name = "docs/a b%#?ü 日本.txt";
    let text = b"hello, remote world\n";
    assert_eq!(p.write(name, &mut &text[..], Some(text.len() as u64)).unwrap(), text.len() as u64);
    let st = p.stat(name).unwrap();
    assert_eq!((st.name.as_str(), st.kind, st.size), ("a b%#?ü 日本.txt", Kind::File, text.len() as u64));
    let m = st.mtime.expect("mtime");
    assert!(m + 3600 > now && m < now + 3600, "mtime {m} vs now {now}");
    let l = p.list("docs").unwrap();
    assert_eq!(names(&l), vec!["inner", "a b%#?ü 日本.txt"]);
    assert_eq!(l[0].kind, Kind::Dir);
    assert_eq!((l[1].kind, l[1].size), (Kind::File, text.len() as u64));
    assert_eq!(p.list("").unwrap().iter().map(|e| (e.name.as_str(), e.kind)).collect::<Vec<_>>(), vec![("docs", Kind::Dir)]);

    // Ranges.
    assert_eq!(p.read(name, 0, 1000).unwrap(), text);
    assert_eq!(p.read(name, 7, 6).unwrap(), b"remote");
    assert_eq!(p.read(name, 15, 1000).unwrap(), b"orld\n");
    assert_eq!(p.read(name, 100, 10).unwrap(), b"");
    assert_eq!(p.read(name, 3, 0).unwrap(), b"");
    assert_eq!(p.read("docs/missing", 0, 10), Err(Error::NotFound));
    assert_eq!(p.stat("docs/missing"), Err(Error::NotFound));
    assert_eq!(p.stat("../etc"), Err(Error::BadPath));

    // A big streamed file, length unknown, replacing a smaller one.
    let data = pattern(big, 42);
    p.write("docs/big.bin", &mut &b"old"[..], None).unwrap();
    assert_eq!(p.write("docs/big.bin", &mut Trickle(&data, 1), None).unwrap(), data.len() as u64);
    assert_eq!(p.stat("docs/big.bin").unwrap().size, data.len() as u64);
    let back = p.read("docs/big.bin", 0, data.len() as u64 + 10).unwrap();
    assert!(back == data, "big file differs ({} vs {} bytes)", back.len(), data.len());
    let mid = big / 2 + 3;
    assert_eq!(p.read("docs/big.bin", mid as u64, 100_003).unwrap(), &data[mid..(mid + 100_003).min(big)]);

    // A failed upload leaves the old file and no temporary.
    let r = p.write(name, &mut Failing { data: &data, fail_after: 70_001, at: 0 }, Some(data.len() as u64));
    assert!(r.is_err(), "{r:?}");
    assert_eq!(p.read(name, 0, 1000).unwrap(), text);
    no_temp(&p.list("docs").unwrap());

    // Rename.
    p.rename(name, "docs/renamed.txt").unwrap();
    assert_eq!(p.stat(name), Err(Error::NotFound));
    assert_eq!(p.read("docs/renamed.txt", 0, 100).unwrap(), text);
    assert_eq!(p.rename("docs/renamed.txt", "docs/big.bin"), Err(Error::Exists));
    p.rename("docs/inner", "docs/moved").unwrap();
    assert_eq!(p.stat("docs/moved").unwrap().kind, Kind::Dir);
    p.write("docs/moved/x.txt", &mut &b"x"[..], Some(1)).unwrap();
    p.rename("docs/moved", "top").unwrap();
    assert_eq!(p.read("top/x.txt", 0, 10).unwrap(), b"x");
    assert_eq!(p.list("top").unwrap().len(), 1);

    // Delete.
    assert_eq!(p.delete("top"), Err(Error::NotEmpty));
    p.delete("top/x.txt").unwrap();
    assert_eq!(p.list("top").unwrap(), vec![]);
    p.delete("top").unwrap();
    assert_eq!(p.stat("top"), Err(Error::NotFound));
    assert_eq!(p.delete("top"), Err(Error::NotFound));
    assert_eq!(p.delete(""), Err(Error::BadPath));
    let l = p.list("docs").unwrap();
    no_temp(&l);
    assert_eq!(names(&l), vec!["big.bin", "renamed.txt"]);
}
