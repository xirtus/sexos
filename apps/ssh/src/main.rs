// SPDX-License-Identifier: GPL-3.0-only
//! `ssh`: log in to another machine. `run +net ssh [-p PORT] [-i KEY] USER@HOST [COMMAND]`
//! runs COMMAND there, or opens a shell whose lines are typed into this window.
//!
//! The first run makes this program's own key (`id_ed25519`, an OpenSSH file in its
//! folder) and prints the line to add to the server's `authorized_keys`. Host keys are
//! remembered in `known_hosts` the first time (trust on first use); a changed key is
//! refused. `--password` reads a password line instead of using the key.
use sex_ssh::{keys::Ed25519, Auth, Client, Event, Run};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

use sex_std as _;

fn record(s: &str) { sex_std::record(s); }

/// Key material: the CPU's hardware generator, hashed with a counter. Without one
/// there is nothing safe to make keys from, so the program refuses to run.
fn random() -> Option<Box<dyn FnMut(&mut [u8])>> {
    if core::arch::x86_64::__cpuid(1).ecx & (1 << 30) == 0 { return None; }
    let mut pool = [0u8; 32];
    getrandom(&mut pool);
    let mut n = 0u64;
    Some(Box::new(move |out: &mut [u8]| {
        for chunk in out.chunks_mut(32) {
            let mut fresh = [0u8; 32];
            getrandom(&mut fresh);
            n += 1;
            let block: [u8; 32] = Sha256::new().chain_update(pool).chain_update(fresh).chain_update(n.to_le_bytes()).finalize().into();
            chunk.copy_from_slice(&block[..chunk.len()]);
            pool = Sha256::digest(block).into();
        }
    }))
}
fn getrandom(out: &mut [u8]) {
    for chunk in out.chunks_mut(8) {
        let mut v = 0u64;
        // SAFETY: RDRAND exists (checked above); retried as Intel advises.
        for _ in 0..10 { if unsafe { core::arch::x86_64::_rdrand64_step(&mut v) } == 1 { break; } }
        chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
}

fn usage() -> ! { println!("usage: ssh [-p PORT] [-i KEY] [--password] USER@HOST [COMMAND]"); std::process::exit(2) }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut port, mut key_file, mut password, mut target, mut command) = (22u16, "id_ed25519".to_string(), false, None, Vec::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" if target.is_none() => port = it.next().and_then(|p| p.parse().ok()).unwrap_or_else(|| usage()),
            "-i" if target.is_none() => key_file = it.next().unwrap_or_else(|| usage()),
            "--password" if target.is_none() => password = true,
            _ if target.is_none() => target = Some(a),
            _ => command.push(a),
        }
    }
    let Some(target) = target else { usage() };
    let Some((user, host)) = target.split_once('@') else { usage() };
    let Some(mut rng) = random() else { println!("ssh: this computer has no hardware random generator to make keys with"); std::process::exit(1) };

    let key = match std::fs::read_to_string(&key_file) {
        Ok(text) => match Ed25519::from_openssh(&text) { Ok(k) => k, Err(e) => { println!("ssh: can't use {key_file}: {e:?}"); std::process::exit(1) } },
        Err(_) => {
            let mut seed = [0u8; 32];
            rng(&mut seed);
            let mut k = Ed25519::from_seed(seed);
            k.comment = "sexos".into();
            let mut check = [0u8; 4];
            rng(&mut check);
            let _ = std::fs::write(&key_file, k.to_openssh(u32::from_le_bytes(check)));
            println!("Made a new key, {key_file}. Add this line to ~/.ssh/authorized_keys on {host}:");
            println!("{}", k.authorized_line());
            k
        }
    };
    record(&format!("[ssh.key] {}", key.authorized_line()));
    let mut auth = vec![Auth::Ed25519(key)];
    let (lines_tx, lines) = mpsc::channel::<String>();
    std::thread::spawn(move || { for l in std::io::stdin().lock().lines() { if lines_tx.send(l.unwrap_or_default()).is_err() { break; } } });
    if password {
        println!("Password for {user}@{host}:");
        auth.insert(0, Auth::Password(lines.recv().unwrap_or_default()));
    }
    let run = if command.is_empty() { Run::Shell { term: "dumb".into(), cols: 100, rows: 30 } } else { Run::Exec(command.join(" ")) };
    let interactive = command.is_empty();

    let mut sock = match TcpStream::connect((host, port)) {
        Ok(s) => s,
        Err(e) => { println!("ssh: can't reach {host}:{port}: {e}"); record(&format!("[ssh.failed] connect {e}")); std::process::exit(1) }
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(50)));
    let mut c = Client::new(user, auth, Some(run), Box::new(move |b: &mut [u8]| rng(b)));
    let mut buf = vec![0u8; 65536];
    let mut status = None;
    loop {
        let o = c.output();
        if !o.is_empty() && sock.write_all(&o).is_err() { break; }
        if c.closed() { break; }
        if interactive { while let Ok(l) = lines.try_recv() { c.send(format!("{l}\n").as_bytes()); } }
        let n = match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => break,
        };
        if let Err(e) = c.receive(&buf[..n]) {
            println!("ssh: {}", match e { sex_ssh::Error::AuthFailed => "the server refused the key (is it in authorized_keys?)".into(), other => format!("{other:?}") });
            record(&format!("[ssh.failed] {e:?}"));
            std::process::exit(1);
        }
        while let Some(e) = c.event() {
            match e {
                Event::HostKey { algorithm, fingerprint, .. } => {
                    let name = if port == 22 { host.to_string() } else { format!("[{host}]:{port}") };
                    let line = format!("{name} {algorithm} {fingerprint}");
                    let known = std::fs::read_to_string("known_hosts").unwrap_or_default();
                    match known.lines().find(|l| l.split(' ').next() == Some(&name)) {
                        Some(l) if l == line => {}
                        Some(_) => {
                            println!("ssh: the host key of {name} CHANGED (now {fingerprint}). Refusing; remove its line from known_hosts if you expected this.");
                            record("[ssh.failed] host key changed");
                            c.reject_host_key();
                            let _ = sock.write_all(&c.output());
                            std::process::exit(1);
                        }
                        None => {
                            println!("New host {name}, {algorithm} key {fingerprint}: remembered.");
                            let _ = std::fs::write("known_hosts", format!("{known}{line}\n"));
                        }
                    }
                    record(&format!("[ssh.host] {line}"));
                    if c.accept_host_key().is_err() { std::process::exit(1); }
                }
                Event::Banner(b) => print!("{b}"),
                Event::Authenticated => record("[ssh.authenticated]"),
                Event::Data { bytes, stderr } => {
                    let text = String::from_utf8_lossy(&bytes);
                    if stderr { eprint!("{text}"); } else { print!("{text}"); }
                    for l in text.lines() { record(&format!("[ssh.out] {l}")); }
                }
                Event::ExitStatus(s) => status = Some(s),
                Event::SessionOpen | Event::Closed => {}
            }
        }
    }
    let _ = std::io::stdout().flush();
    record(&format!("[ssh.done] status={}", status.map_or(-1, |s| s as i64)));
    std::process::exit(status.map_or(255, |s| s as i32));
}
