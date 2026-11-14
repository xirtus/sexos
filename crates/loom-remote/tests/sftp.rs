// SPDX-License-Identifier: GPL-3.0-only
//! SFTP against OpenSSH: its sftp-server over pipes, and through a real sshd with
//! sex-ssh carrying the `sftp` subsystem.
mod common;
use common::*;
use loom_remote::sftp::{SshChannel, Sftp};
use loom_remote::{Error, Kind, Place, Result, Stream};
use sex_ssh::{keys, Auth, Client, Run};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const SFTP_SERVER: &str = "/usr/lib/openssh/sftp-server";

struct Pipes { _child: Server, inp: ChildStdin, out: ChildStdout }
impl Stream for Pipes {
    fn write_all(&mut self, b: &[u8]) -> Result<()> { self.inp.write_all(b).map_err(|e| Error::Transport(e.to_string())) }
    fn read(&mut self, b: &mut [u8]) -> Result<usize> { self.out.read(b).map_err(|e| Error::Transport(e.to_string())) }
}
fn sftp_server() -> Option<Pipes> {
    if !Path::new(SFTP_SERVER).exists() { eprintln!("skipping: no {SFTP_SERVER} (apt install openssh-sftp-server)"); return None; }
    let mut c = Command::new(SFTP_SERVER).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let (inp, out) = (c.stdin.take().unwrap(), c.stdout.take().unwrap());
    Some(Pipes { _child: Server(c), inp, out })
}

#[test]
fn sftp_server_over_pipes_does_everything_a_place_does() {
    let Some(pipes) = sftp_server() else { return };
    let root = scratch("sftp-pipes");
    let mut s = Sftp::new(pipes, root.to_str().unwrap()).unwrap();
    assert!(s.has_extension("posix-rename@openssh.com"));
    assert!(s.abilities().atomic_write);
    exercise(&mut s, 3 * 1024 * 1024 + 12345);
    // What the server wrote is what is on disk.
    assert_eq!(std::fs::read(root.join("docs/big.bin")).unwrap(), pattern(3 * 1024 * 1024 + 12345, 42));
    // Other kinds and the home-relative root.
    std::os::unix::fs::symlink("docs", root.join("link")).unwrap();
    let l = s.list("").unwrap();
    assert_eq!(l.iter().find(|e| e.name == "link").unwrap().kind, Kind::Link);
    assert_eq!(s.stat("link").unwrap().kind, Kind::Dir, "stat follows links");
    std::fs::set_permissions(root.join("docs/renamed.txt"), std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let denied = s.read("docs/renamed.txt", 0, 10);
    // Root reads everything; anyone else is refused.
    assert!(denied.is_ok() || denied == Err(Error::Denied), "{denied:?}");
    let _ = std::fs::remove_dir_all(&root);
}

// --- Through sshd, reusing sex-ssh's harness (a real OpenSSH server on localhost). ---

struct Sshd { child: Child, port: u16, pid: std::path::PathBuf }
impl Drop for Sshd {
    fn drop(&mut self) {
        if let Ok(pid) = std::fs::read_to_string(&self.pid) { let _ = Command::new("sudo").args(["-n", "kill", pid.trim()]).status(); let _ = Command::new("kill").arg(pid.trim()).status(); }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn root_user() -> bool { Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout.starts_with(b"0")) }

fn sshd(dir: &Path, key: &keys::Ed25519) -> Option<Sshd> {
    if !Path::new("/usr/sbin/sshd").exists() { eprintln!("skipping: no sshd (apt install openssh-server)"); return None; }
    std::fs::create_dir_all(dir).unwrap();
    let _ = std::fs::create_dir_all("/run/sshd");
    let hk = dir.join("host_ed25519");
    if !hk.exists() { assert!(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&hk).status().unwrap().success()); }
    std::fs::write(dir.join("authorized_keys"), format!("{}\n", key.authorized_line())).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    // internal-sftp: only a subsystem request reaches it (no exec of a server path).
    let cfg = format!("Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nHostKeyAlgorithms ssh-ed25519\nAuthorizedKeysFile {}\nStrictModes no\nUsePAM no\nPermitRootLogin yes\nPasswordAuthentication no\nPidFile {}\nSubsystem sftp internal-sftp\n",
        hk.display(), dir.join("authorized_keys").display(), dir.join("pid").display());
    std::fs::write(dir.join("sshd_config"), cfg).unwrap();
    let mut cmd = if root_user() { Command::new("/usr/sbin/sshd") } else { let _ = Command::new("sudo").args(["-n", "mkdir", "-p", "/run/sshd"]).status(); let mut c = Command::new("sudo"); c.args(["-n", "/usr/sbin/sshd"]); c };
    let child = cmd.args(["-D", "-e", "-f"]).arg(dir.join("sshd_config")).stderr(std::fs::File::create(dir.join("sshd.log")).map(Stdio::from).unwrap_or(Stdio::null())).spawn().unwrap();
    let began = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() { assert!(began.elapsed() < Duration::from_secs(10)); std::thread::sleep(Duration::from_millis(20)); }
    Some(Sshd { child, port, pid: dir.join("pid") })
}

fn rng() -> Box<dyn FnMut(&mut [u8])> {
    let mut n = 0x9e3779b97f4a7c15u64 ^ std::process::id() as u64;
    Box::new(move |b: &mut [u8]| for x in b { n ^= n << 13; n ^= n >> 7; n ^= n << 17; *x = n as u8; })
}

#[test]
fn sftp_subsystem_through_sex_ssh_and_real_sshd() {
    let dir = scratch("sftp-sshd");
    let key = keys::Ed25519::from_seed([9; 32]);
    let Some(server) = sshd(&dir.join("srv"), &key) else { return };
    let user = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout).unwrap().trim().to_string();
    let tcp = TcpConnect(server.port);
    let conn = { use loom_remote::Connect; let mut c = tcp; c.connect().unwrap() };
    let client = Client::new(&user, vec![Auth::Ed25519(keys::Ed25519::from_seed([9; 32]))], Some(Run::Subsystem("sftp".into())), rng());
    let chan = SshChannel::open(client, conn, Box::new(|alg, _blob, fp| alg == "ssh-ed25519" && fp.starts_with("SHA256:"))).unwrap();
    let root = dir.join("files");
    std::fs::create_dir_all(&root).unwrap();
    let mut s = Sftp::new(chan, root.to_str().unwrap()).unwrap();
    // 2 MiB pushes the SSH windows (2 MiB each way) through several adjustments.
    exercise(&mut s, 2 * 1024 * 1024 + 777);
    assert_eq!(std::fs::read(root.join("docs/renamed.txt")).unwrap(), b"hello, remote world\n");
    drop(s);
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_host_key_stops_before_login() {
    let dir = scratch("sftp-hostkey");
    let key = keys::Ed25519::from_seed([5; 32]);
    let Some(server) = sshd(&dir, &key) else { return };
    let conn = { use loom_remote::Connect; TcpConnect(server.port).connect().unwrap() };
    let client = Client::new("nobody", vec![Auth::Ed25519(key)], Some(Run::Subsystem("sftp".into())), rng());
    assert_eq!(SshChannel::open(client, conn, Box::new(|_, _, _| false)).err(), Some(Error::Denied));
    let _ = std::fs::remove_dir_all(&dir);
}
