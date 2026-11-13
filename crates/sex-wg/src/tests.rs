use super::*;
use std::{io::{BufRead, BufReader, Write}, net::UdpSocket, process::{Command, Stdio}, string::String, vec};

fn key(b: u8) -> [u8; 32] { [b; 32] }
fn counter_rng(seed: u8) -> impl FnMut(&mut [u8]) { let mut n = seed; move |b: &mut [u8]| { for x in b.iter_mut() { n = n.wrapping_mul(31).wrapping_add(7); *x = n; } } }
/// An IPv4 packet of `len` bytes total (the header says so), as the tunnel expects.
fn ip(len: usize, fill: u8) -> Vec<u8> { let mut p = vec![fill; len]; p[0] = 0x45; p[2..4].copy_from_slice(&(len as u16).to_be_bytes()); p }

fn pair() -> (Tunnel, Tunnel, u64) {
    let (a, b) = (key(1), key(2));
    let mut i = Tunnel::new(a, public_key(&b), Some(key(9)));
    let mut r = Tunnel::new(b, public_key(&a), Some(key(9)));
    let init = i.initiate(1000, key(3), 0x1111, tai64n(1_700_000_000, 0));
    let Received::Reply(resp) = r.receive(1000, &init, &mut counter_rng(5)).unwrap() else { panic!() };
    assert_eq!(resp.len(), RESPONSE_BYTES);
    let Received::Reply(ka) = i.receive(1001, &resp, &mut counter_rng(6)).unwrap() else { panic!() };
    assert!(!r.up(1001), "responder waits for the initiator's first data");
    assert_eq!(r.receive(1001, &ka, &mut counter_rng(7)), Ok(Received::Keepalive));
    assert!(i.up(1001) && r.up(1001));
    (i, r, 1001)
}

#[test]
fn a_handshake_then_packets_both_ways() {
    let (mut i, mut r, now) = pair();
    for n in [20, 21, 1420] {
        let m = i.send(now, &ip(n, 0xab)).unwrap();
        assert_eq!(m.len() % 16, 0, "padded");
        assert_eq!(r.receive(now, &m, &mut counter_rng(1)), Ok(Received::Packet(ip(n, 0xab))));
        let back = r.send(now, &ip(n, 0xcd)).unwrap();
        assert_eq!(i.receive(now, &back, &mut counter_rng(1)), Ok(Received::Packet(ip(n, 0xcd))));
    }
}

#[test]
fn replays_tampering_and_strangers_are_refused() {
    let (mut i, mut r, now) = pair();
    let m = i.send(now, &ip(40, 1)).unwrap();
    assert!(r.receive(now, &m, &mut counter_rng(1)).is_ok());
    assert_eq!(r.receive(now, &m, &mut counter_rng(1)), Err(Error::Replay));
    let mut bad = i.send(now, &ip(40, 1)).unwrap();
    let n = bad.len();
    bad[n - 1] ^= 1;
    assert_eq!(r.receive(now, &bad, &mut counter_rng(1)), Err(Error::Authentication));
    // Out of order within the window is fine; far behind it is not.
    let later: Vec<Vec<u8>> = (0..3000).map(|_| i.send(now, &ip(20, 2)).unwrap()).collect();
    assert!(r.receive(now, &later[2999], &mut counter_rng(1)).is_ok());
    assert!(r.receive(now, &later[1500], &mut counter_rng(1)).is_ok());
    assert_eq!(r.receive(now, &later[10], &mut counter_rng(1)), Err(Error::Replay));
    // An initiation from a key this tunnel doesn't trust.
    let mut stranger = Tunnel::new(key(7), public_key(&key(2)), Some(key(9)));
    let init = stranger.initiate(now, key(8), 5, tai64n(1_700_000_100, 0));
    assert_eq!(r.receive(now, &init, &mut counter_rng(1)), Err(Error::WrongPeer));
    // A replayed initiation (same timestamp) is refused.
    let mut i2 = Tunnel::new(key(1), public_key(&key(2)), Some(key(9)));
    let old = i2.initiate(now, key(3), 0x2222, tai64n(1_700_000_000, 0));
    assert_eq!(r.receive(now, &old, &mut counter_rng(1)), Err(Error::Replay));
    // Wrong mac1 (not meant for us) is dropped before any expensive work.
    let mut bad = old;
    bad[120] ^= 1;
    assert_eq!(r.receive(now, &bad, &mut counter_rng(1)), Err(Error::Authentication));
}

#[test]
fn under_load_a_cookie_reply_makes_the_next_initiation_carry_mac2() {
    let (a, b) = (key(1), key(2));
    let mut i = Tunnel::new(a, public_key(&b), None);
    let r = Tunnel::new(b, public_key(&a), None);
    let init = i.initiate(0, key(3), 42, tai64n(1_700_000_000, 0));
    assert_eq!(&init[132..], &[0; 16], "no cookie yet");
    let reply = r.cookie_reply(&init, &key(0x55), b"10.0.2.15:51820", [6; 24]).unwrap();
    assert_eq!(i.receive(1, &reply, &mut counter_rng(1)), Ok(Received::Cookie));
    let again = i.initiate(2, key(4), 43, tai64n(1_700_000_001, 0));
    let cookie = mac(&key(0x55), b"10.0.2.15:51820");
    assert_eq!(&again[132..], &mac(&cookie, &again[..132]), "mac2 from the cookie");
}

#[test]
fn timers_retry_rekey_keepalive_and_give_up() {
    let mut i = Tunnel::new(key(1), public_key(&key(2)), None);
    i.initiate(0, key(3), 1, tai64n(1_700_000_000, 0));
    assert_eq!(i.tick(REKEY_TIMEOUT - 1), None);
    assert_eq!(i.tick(REKEY_TIMEOUT), Some(Due::Handshake));
    assert_eq!(i.tick(REKEY_ATTEMPT_TIME), Some(Due::GiveUp));
    let (mut i, mut r, now) = pair();
    // The responder got data and sent none back: after KEEPALIVE_TIMEOUT it says so.
    let m = i.send(now, &ip(20, 0)).unwrap();
    r.receive(now + 1, &m, &mut counter_rng(1)).unwrap();
    let Some(Due::Keepalive(k)) = r.tick(now + 1 + KEEPALIVE_TIMEOUT) else { panic!() };
    assert_eq!(i.receive(now + 1 + KEEPALIVE_TIMEOUT, &k, &mut counter_rng(1)), Ok(Received::Keepalive));
    assert_eq!(i.tick(now + REKEY_AFTER_TIME), Some(Due::Handshake), "initiator rekeys");
    i.set_persistent_keepalive(Some(25));
    assert!(matches!(r.tick(now + 30_000), None | Some(Due::Keepalive(_))));
    assert_eq!(i.send(now + REJECT_AFTER_TIME, &ip(20, 0)), Err(Error::NoSession));
}

/// An independent WireGuard peer in Python (its own BLAKE2s, X25519 and ChaCha20-Poly1305
/// from `hashlib` and `cryptography`): it answers our initiation, decrypts our packet
/// and sends it back through the tunnel.
const PEER: &str = r#"
import socket,sys,hashlib,hmac,struct
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey,X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives import serialization as S
raw=lambda k:k.public_key().public_bytes(S.Encoding.Raw,S.PublicFormat.Raw)
H=lambda *p:hashlib.blake2s(b''.join(p)).digest()
MAC=lambda k,d:hashlib.blake2s(d,key=k,digest_size=16).digest()
HM=lambda k,d:hmac.new(k,d,lambda:hashlib.blake2s()).digest()
def KDF(n,c,x):
 t0=HM(c,x);o=[];p=b''
 for i in range(n):p=HM(t0,p+bytes([i+1]));o.append(p)
 return o
DH=lambda priv,pub:priv.exchange(X25519PublicKey.from_public_bytes(pub))
A=lambda k,n,p,ad:ChaCha20Poly1305(k).encrypt(b'\0'*4+struct.pack('<Q',n),p,ad)
D=lambda k,n,c,ad:ChaCha20Poly1305(k).decrypt(b'\0'*4+struct.pack('<Q',n),c,ad)
me=X25519PrivateKey.from_private_bytes(bytes([2])*32);psk=bytes([9])*32;mepub=raw(me)
s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.bind(('127.0.0.1',0));print(s.getsockname()[1],flush=True)
m,addr=s.recvfrom(2048);assert m[0]==1 and len(m)==148
assert MAC(H(b'mac1----',mepub),m[:116])==m[116:132]
C=H(b'Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s');h=H(H(C,b'WireGuard v1 zx2c4 Jason@zx2c4.com'),mepub)
ei=m[8:40];C,=KDF(1,C,ei);h=H(h,ei);C,k=KDF(2,C,DH(me,ei));spub=D(k,0,m[40:88],h);h=H(h,m[40:88])
C,k=KDF(2,C,DH(me,spub));ts=D(k,0,m[88:116],h);h=H(h,m[88:116])
er=X25519PrivateKey.from_private_bytes(bytes([7])*32);erp=raw(er)
C,=KDF(1,C,erp);h=H(h,erp);C,=KDF(1,C,DH(er,ei));C,=KDF(1,C,DH(er,spub));C,tau,k=KDF(3,C,psk);h=H(h,tau)
empty=A(k,0,b'',h);theirs=m[4:8]
r=b'\x02\0\0\0'+struct.pack('<I',777)+theirs+erp+empty
r+=MAC(H(b'mac1----',spub),r)+b'\0'*16
s.sendto(r,addr)
recv,send=KDF(2,C,b'')
got=0;out=0
while got<2:
 m,addr=s.recvfrom(4096);assert m[0]==4 and m[4:8]==struct.pack('<I',777)
 n=struct.unpack('<Q',m[8:16])[0];p=D(recv,n,m[16:],b'');got+=1
 if p:
  s.sendto(b'\x04\0\0\0'+theirs+struct.pack('<Q',out)+A(send,out,p,b''),addr);out+=1
print('ok timestamp=%s'%ts[:8].hex(),flush=True)
"#;

#[test]
fn interoperates_with_an_independent_python_peer() {
    if Command::new("python3").args(["-c", "import cryptography"]).status().map_or(true, |s| !s.success()) { return; }
    let mut peer = Command::new("python3").args(["-c", PEER]).stdout(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(peer.stdout.take().unwrap());
    let mut port = String::new();
    lines.read_line(&mut port).unwrap();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.connect(("127.0.0.1", port.trim().parse::<u16>().unwrap())).unwrap();
    let mut t = Tunnel::new(key(1), public_key(&key(2)), Some(key(9)));
    sock.send(&t.initiate(0, key(3), 0xabcd, tai64n(1_700_000_000, 5))).unwrap();
    let mut buf = [0u8; 4096];
    let n = sock.recv(&mut buf).unwrap();
    let Ok(Received::Reply(k)) = t.receive(1, &buf[..n], &mut counter_rng(1)) else { panic!("response refused") };
    sock.send(&k).unwrap();
    let packet = ip(84, 0x5a);
    sock.send(&t.send(2, &packet).unwrap()).unwrap();
    let n = sock.recv(&mut buf).unwrap();
    assert_eq!(t.receive(3, &buf[..n], &mut counter_rng(1)), Ok(Received::Packet(packet)));
    let mut done = String::new();
    lines.read_line(&mut done).unwrap();
    peer.wait().unwrap();
    assert_eq!(done.trim(), format!("ok timestamp={}", hex8(&tai64n(1_700_000_000, 5))));
    let _ = std::io::stdout().flush();
}
fn hex8(t: &[u8; 12]) -> String { t[..8].iter().map(|b| format!("{b:02x}")).collect() }
use std::format;
