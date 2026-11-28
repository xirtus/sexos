// SPDX-License-Identifier: GPL-3.0-only
//! sex-dtls against OpenSSL: a ClientHello OpenSSL 3.0 sent (captured), and, when an
//! `openssl` binary is on the host, live handshakes over loopback UDP in both roles with
//! use_srtp, comparing the exported SRTP keying material OpenSSL prints with ours.
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use sex_dtls::{Certificate, Config, Dtls, Event, Fingerprint, Role, SrtpProfile};
use std::io::{BufRead, BufReader};
use std::net::UdpSocket;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// `openssl s_client -dtls1_2 -use_srtp SRTP_AEAD_AES_128_GCM:SRTP_AES128_CM_SHA1_80`,
/// first datagram (record version DTLS 1.0, 56 suites, x25519 first, many signature
/// algorithms).
const OPENSSL_CLIENT_HELLO: &str = "16feff000000000000000000cb010000bf00000000000000bffefdf41e8687489b45658e0abfbcb792113afcf28ed732a2559290c343e82c5add9d00000038c02cc030009fcca9cca8ccaac02bc02f009ec024c028006bc023c0270067c00ac0140039c009c0130033009d009c003d003c0035002f00ff0100005d000b000403000102000a000c000a001d0017001e0019001800230000000e0007000400070001000016000000170000000d002a0028040305030603080708080809080a080b080408050806040105010601030303010302040205020602";

fn cert(seed: u8) -> Certificate {
    Certificate::generate(&mut ChaCha20Rng::from_seed([seed; 32]))
}

#[test]
fn captured_openssl_client_hello() {
    let ours = cert(7);
    let mut s = Dtls::new(Config::new(Role::Server, ours, Fingerprint([0; 32])), [1; 32]);
    s.handle_datagram(0, &hex(OPENSSL_CLIENT_HELLO)).unwrap();
    assert_eq!(s.srtp_profile(), Some(SrtpProfile::AeadAes128Gcm));
    let flight: Vec<Vec<u8>> = std::iter::from_fn(|| s.poll_transmit()).collect();
    assert!(!flight.is_empty() && flight.iter().all(|d| d.len() <= 1200));
    let d = &flight[0];
    // Handshake record, DTLS 1.2, epoch 0, ServerHello as message 0.
    assert_eq!(&d[..3], &[22, 0xfe, 0xfd]);
    assert_eq!(d[13], 2);
    assert_eq!(&d[17..19], &[0, 0]);
    // ServerHello: our suite.
    let sh = &d[25..];
    assert_eq!(&sh[..2], &[0xfe, 0xfd]);
    assert_eq!(&sh[35..37], &[0xc0, 0x2b]);
    // Cookie mode answers with a HelloVerifyRequest instead.
    let mut cfg = Config::new(Role::Server, cert(7), Fingerprint([0; 32]));
    cfg.hello_verify = true;
    let mut s = Dtls::new(cfg, [1; 32]);
    s.handle_datagram(0, &hex(OPENSSL_CLIENT_HELLO)).unwrap();
    let d = s.poll_transmit().unwrap();
    assert_eq!(d[13], 3);
}

/// An OpenSSL 1.1.1 or newer binary (LibreSSL lacks the AEAD SRTP profile and keymatexport).
/// SEX_DTLS_REQUIRE_OPENSSL=1 (the host gate on Linux) turns a missing one into a failure.
fn have_openssl() -> bool {
    let v = Command::new("openssl").arg("version").output().ok().filter(|o| o.status.success());
    let v = v.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let ok = v.strip_prefix("OpenSSL ").map_or(false, |r| !r.starts_with("0.") && !r.starts_with("1.0") && !r.starts_with("1.1.0"));
    if !ok {
        assert!(std::env::var_os("SEX_DTLS_REQUIRE_OPENSSL").is_none(), "needs OpenSSL 1.1.1+, found {v:?}");
        eprintln!("no OpenSSL 1.1.1+ binary ({}): skipped", v.trim());
    }
    ok
}

struct Tmp(std::path::PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An OpenSSL P-256 certificate: (dir, fingerprint).
fn openssl_cert(tag: &str) -> (Tmp, Fingerprint) {
    let dir = std::env::temp_dir().join(format!("sex-dtls-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let st = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"])
        .args(["-subj", "/CN=openssl", "-days", "2", "-keyout"])
        .arg(dir.join("k.pem"))
        .arg("-out")
        .arg(dir.join("c.pem"))
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success());
    let der = Command::new("openssl").args(["x509", "-outform", "der", "-in"]).arg(dir.join("c.pem")).output().unwrap();
    (Tmp(dir), Fingerprint::of(&der.stdout))
}

struct Peer {
    child: Child,
    out: Arc<Mutex<String>>,
}

impl Peer {
    fn spawn(mut cmd: Command) -> Peer {
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let out = Arc::new(Mutex::new(String::new()));
        for r in [Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>, Box::new(child.stderr.take().unwrap())] {
            let o = out.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(r).lines().map_while(Result::ok) {
                    let mut g = o.lock().unwrap();
                    g.push_str(&line);
                    g.push('\n');
                }
            });
        }
        Peer { child, out }
    }
    fn wait_for(&self, s: &str) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(10) {
            if self.out.lock().unwrap().contains(s) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
    fn keying_material(&self) -> Option<Vec<u8>> {
        let o = self.out.lock().unwrap();
        let l = o.lines().find(|l| l.trim_start().starts_with("Keying material: "))?;
        Some(hex(l.trim_start().trim_start_matches("Keying material: ").trim()))
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drive our endpoint on a UDP socket until connected, then send a line and close.
fn drive(d: &mut Dtls, sock: &UdpSocket, peer: &Peer, to: Option<std::net::SocketAddr>) -> std::net::SocketAddr {
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut addr = to;
    sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    d.start(now());
    let mut buf = [0u8; 4096];
    while start.elapsed() < Duration::from_secs(15) {
        if let Some(a) = addr {
            while let Some(p) = d.poll_transmit() {
                sock.send_to(&p, a).unwrap();
            }
        }
        if d.is_connected() {
            break;
        }
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            addr = Some(from);
            d.handle_datagram(now(), &buf[..n]).unwrap();
        }
        d.handle_timeout(now()).unwrap();
    }
    assert!(d.is_connected(), "no handshake with openssl:\n{}", peer.out.lock().unwrap());
    assert_eq!(d.poll_event(), Some(Event::Connected));
    addr.unwrap()
}

fn check_keys(d: &Dtls, peer: &Peer, len: usize) {
    assert!(peer.wait_for("Keying material: "), "{}", peer.out.lock().unwrap());
    let theirs = peer.keying_material().unwrap();
    let ours = d.export_keying_material(b"EXTRACTOR-dtls_srtp", len).unwrap();
    assert_eq!(theirs, ours);
}

#[test]
fn live_openssl_server() {
    if !have_openssl() {
        return;
    }
    let (dir, fp) = openssl_cert("srv");
    let port = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut cmd = Command::new("openssl");
    cmd.args(["s_server", "-dtls1_2", "-accept"]).arg(format!("127.0.0.1:{port}"));
    cmd.arg("-cert").arg(dir.0.join("c.pem")).arg("-key").arg(dir.0.join("k.pem"));
    cmd.args(["-use_srtp", "SRTP_AES128_CM_SHA1_80", "-keymatexport", "EXTRACTOR-dtls_srtp", "-keymatexportlen", "60"]);
    cmd.args(["-Verify", "1", "-naccept", "1"]);
    let peer = Peer::spawn(cmd);
    assert!(peer.wait_for("ACCEPT"), "{}", peer.out.lock().unwrap());

    let ours = cert(11);
    let mut d = Dtls::new(Config::new(Role::Client, ours, fp), [5; 32]);
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let to = drive(&mut d, &sock, &peer, Some(format!("127.0.0.1:{port}").parse().unwrap()));
    assert_eq!(d.srtp_profile(), Some(SrtpProfile::Aes128CmHmacSha1_80));
    check_keys(&d, &peer, 60);
    d.send(b"ping from sexos\n").unwrap();
    while let Some(p) = d.poll_transmit() {
        sock.send_to(&p, to).unwrap();
    }
    assert!(peer.wait_for("ping from sexos"), "{}", peer.out.lock().unwrap());
}

#[test]
fn live_openssl_client() {
    if !have_openssl() {
        return;
    }
    let (dir, fp) = openssl_cert("cli");
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = sock.local_addr().unwrap().port();
    let ours = cert(12);
    let mut cfg = Config::new(Role::Server, ours, fp);
    cfg.hello_verify = true;
    let mut d = Dtls::new(cfg, [6; 32]);
    let mut cmd = Command::new("openssl");
    cmd.args(["s_client", "-dtls1_2", "-connect"]).arg(format!("127.0.0.1:{port}"));
    cmd.arg("-cert").arg(dir.0.join("c.pem")).arg("-key").arg(dir.0.join("k.pem"));
    cmd.args(["-use_srtp", "SRTP_AEAD_AES_128_GCM:SRTP_AES128_CM_SHA1_80"]);
    cmd.args(["-keymatexport", "EXTRACTOR-dtls_srtp", "-keymatexportlen", "56"]);
    let peer = Peer::spawn(cmd);
    let to = drive(&mut d, &sock, &peer, None);
    assert_eq!(d.srtp_profile(), Some(SrtpProfile::AeadAes128Gcm));
    check_keys(&d, &peer, 56);
    d.send(b"pong from sexos\n").unwrap();
    while let Some(p) = d.poll_transmit() {
        sock.send_to(&p, to).unwrap();
    }
    assert!(peer.wait_for("pong from sexos"), "{}", peer.out.lock().unwrap());
}
