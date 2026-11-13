use super::*;
use std::{boxed::Box, format, io::{Read, Write}, net::TcpStream, path::Path, process::{Child, Command, Stdio}, string::ToString, time::{Duration, Instant}, vec};

fn rng() -> Box<dyn FnMut(&mut [u8])> { let mut n = 0x9e3779b97f4a7c15u64; Box::new(move |b: &mut [u8]| for x in b { n ^= n << 13; n ^= n >> 7; n ^= n << 17; *x = n as u8; }) }

struct Sshd { child: Child, port: u16, pid: std::path::PathBuf }
impl Drop for Sshd {
    fn drop(&mut self) {
        // Under sudo the child is sudo itself: stop sshd by its pid file too.
        if let Ok(pid) = std::fs::read_to_string(&self.pid) { let _ = Command::new("sudo").args(["-n", "kill", pid.trim()]).status(); let _ = Command::new("kill").arg(pid.trim()).status(); }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn root() -> bool { Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout.starts_with(b"0")) }

/// A real OpenSSH server on localhost offering only `host_alg`, trusting `key`.
fn sshd(dir: &Path, host_alg: &str, key: &keys::Ed25519) -> Option<Sshd> {
    if !Path::new("/usr/sbin/sshd").exists() { return None; }
    std::fs::create_dir_all(dir).unwrap();
    let _ = std::fs::create_dir_all("/run/sshd");
    let (kind, file) = match host_alg { "ssh-ed25519" => ("ed25519", "host_ed25519"), "ecdsa-sha2-nistp256" => ("ecdsa", "host_ecdsa"), _ => ("rsa", "host_rsa") };
    let hk = dir.join(file);
    if !hk.exists() { assert!(Command::new("ssh-keygen").args(["-q", "-t", kind, "-N", "", "-f"]).arg(&hk).status().unwrap().success()); }
    std::fs::write(dir.join("authorized_keys"), format!("{}\n", key.authorized_line())).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let cfg = format!("Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nHostKeyAlgorithms {host_alg}\nAuthorizedKeysFile {}\nStrictModes no\nUsePAM no\nPermitRootLogin yes\nPasswordAuthentication no\nPidFile {}\n",
        hk.display(), dir.join("authorized_keys").display(), dir.join("pid").display());
    std::fs::write(dir.join("sshd_config"), cfg).unwrap();
    // sshd needs root for privilege separation; CI runners have password-less sudo.
    let mut cmd = if root() { Command::new("/usr/sbin/sshd") } else { let _ = Command::new("sudo").args(["-n", "mkdir", "-p", "/run/sshd"]).status(); let mut c = Command::new("sudo"); c.args(["-n", "/usr/sbin/sshd"]); c };
    let child = cmd.args(["-D", "-e", "-f"]).arg(dir.join("sshd_config")).stderr(Stdio::null()).spawn().unwrap();
    let began = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() { assert!(began.elapsed() < Duration::from_secs(10)); std::thread::sleep(Duration::from_millis(20)); }
    Some(Sshd { child, port, pid: dir.join("pid") })
}

/// Runs a client against the server until it closes; gives (stdout, stderr, status, host key event).
fn drive(port: u16, mut c: Client, input: &[u8]) -> (Vec<u8>, Vec<u8>, Option<u32>, Option<Event>) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let (mut out, mut err, mut status, mut host) = (Vec::new(), Vec::new(), None, None);
    let mut buf = vec![0u8; 65536];
    let mut sent_input = false;
    while !c.closed() {
        let o = c.output();
        if !o.is_empty() { s.write_all(&o).unwrap(); }
        let n = s.read(&mut buf).unwrap();
        if n == 0 { break; }
        c.receive(&buf[..n]).unwrap();
        while let Some(e) = c.event() {
            match e {
                Event::HostKey { .. } => { host = Some(e); c.accept_host_key().unwrap(); }
                Event::SessionOpen if !input.is_empty() && !sent_input => { sent_input = true; c.send(input); }
                Event::Data { bytes, stderr } => if stderr { err.extend(bytes) } else { out.extend(bytes) },
                Event::ExitStatus(x) => status = Some(x),
                _ => {}
            }
        }
    }
    let o = c.output();
    let _ = s.write_all(&o);
    (out, err, status, host)
}

#[test]
fn logs_in_with_an_ed25519_key_and_runs_a_command_for_every_host_key_type() {
    let dir = std::env::temp_dir().join(format!("sexos-ssh-{}", std::process::id()));
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    for alg in ["ssh-ed25519", "ecdsa-sha2-nistp256", "rsa-sha2-512"] {
        let key = keys::Ed25519::from_seed([7; 32]);
        let Some(server) = sshd(&dir, alg, &key) else { return };
        let c = Client::new(&user, vec![Auth::Ed25519(keys::Ed25519::from_seed([7; 32]))], Some(Run::Exec("echo hello-from-$((6*7)); echo oops >&2; exit 3".into())), rng());
        let (out, err, status, host) = drive(server.port, c, b"");
        assert_eq!(out, b"hello-from-42\n", "{alg}");
        assert_eq!(err, b"oops\n");
        assert_eq!(status, Some(3));
        std::eprintln!("checked {alg}");
        let Some(Event::HostKey { algorithm, fingerprint, .. }) = host else { panic!() };
        assert_eq!(algorithm, alg);
        // The fingerprint matches what ssh-keygen says of the server's key.
        let file = match alg { "ssh-ed25519" => "host_ed25519", "ecdsa-sha2-nistp256" => "host_ecdsa", _ => "host_rsa" };
        let shown = Command::new("ssh-keygen").args(["-l", "-f"]).arg(dir.join(format!("{file}.pub"))).output().unwrap();
        assert!(String::from_utf8_lossy(&shown.stdout).contains(&fingerprint), "{fingerprint}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn megabytes_of_output_flow_through_the_window_and_a_pty_shell_takes_input() {
    let dir = std::env::temp_dir().join(format!("sexos-ssh-big-{}", std::process::id()));
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let key = keys::Ed25519::from_seed([8; 32]);
    let Some(server) = sshd(&dir, "ssh-ed25519", &key) else { return };
    let c = Client::new(&user, vec![Auth::Ed25519(keys::Ed25519::from_seed([8; 32]))], Some(Run::Exec("head -c 5000000 /dev/zero | tr '\\0' a".into())), rng());
    let (out, _, status, _) = drive(server.port, c, b"");
    assert_eq!(out.len(), 5_000_000);
    assert!(out.iter().all(|b| *b == b'a'));
    assert_eq!(status, Some(0));
    let c = Client::new(&user, vec![Auth::Ed25519(keys::Ed25519::from_seed([8; 32]))], Some(Run::Shell { term: "xterm".into(), cols: 100, rows: 30 }), rng());
    let (out, _, status, _) = drive(server.port, c, b"stty size; echo typed-$((2+3)); exit 0\n");
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("30 100") && text.contains("typed-5"), "{text}");
    assert_eq!(status, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_key_is_refused_and_openssh_key_files_load() {
    let dir = std::env::temp_dir().join(format!("sexos-ssh-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("id");
    assert!(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "me@sexos", "-f"]).arg(&f).status().unwrap().success());
    let key = keys::Ed25519::from_openssh(&std::fs::read_to_string(&f).unwrap()).unwrap();
    assert_eq!(key.comment, "me@sexos");
    let public = std::fs::read_to_string(dir.join("id.pub")).unwrap();
    assert!(public.starts_with(&key.authorized_line()), "{public}");
    // Written back out, OpenSSH reads it and derives the same public key.
    std::fs::write(dir.join("again"), key.to_openssh(0x1234)).unwrap();
    std::fs::set_permissions(dir.join("again"), std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let derived = Command::new("ssh-keygen").args(["-y", "-f"]).arg(dir.join("again")).output().unwrap();
    assert!(String::from_utf8_lossy(&derived.stdout).starts_with(&key.authorized_line()));
    assert_eq!(keys::Ed25519::from_openssh(&key.to_openssh(9)).unwrap().authorized_line(), key.authorized_line());
    assert!(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "secret", "-f"]).arg(dir.join("locked")).status().unwrap().success());
    assert_eq!(keys::Ed25519::from_openssh(&std::fs::read_to_string(dir.join("locked")).unwrap()).err(), Some(keys::KeyError::Encrypted));
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let Some(server) = sshd(&dir.join("srv"), "ssh-ed25519", &key) else { return };
    let mut c = Client::new(&user, vec![Auth::Ed25519(keys::Ed25519::from_seed([1; 32]))], Some(Run::Exec("true".into())), rng());
    let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let mut buf = vec![0u8; 65536];
    let result = loop {
        let o = c.output();
        if !o.is_empty() { s.write_all(&o).unwrap(); }
        let n = s.read(&mut buf).unwrap();
        if let Err(e) = c.receive(&buf[..n]) { break e; }
        while let Some(e) = c.event() { if matches!(e, Event::HostKey { .. }) { c.accept_host_key().unwrap(); } }
    };
    assert_eq!(result, Error::AuthFailed);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = "".to_string();
}
