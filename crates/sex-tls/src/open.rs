// SPDX-License-Identifier: GPL-3.0-only
//! TLS to any host, checked against the public root store: TLS 1.3 and TLS 1.2, with
//! AES-128-GCM, AES-256-GCM or ChaCha20-Poly1305 over X25519, P-256 or P-384 (rustls's
//! state machines on the RustCrypto primitives in `provider`). The session stays open
//! for as many requests as the site allows (HTTP keep-alive): the caller queues
//! plaintext, which goes out as it comes, and reads replies as they arrive. Bytes on
//! the wire and plaintext both pass through this type, and the caller moves them.
use alloc::{sync::Arc, vec::Vec};
#[cfg(test)] extern crate std;
use rsa::{pkcs1::DecodeRsaPublicKey, signature::Verifier as _, RsaPublicKey};
use rustls::client::{UnbufferedClientConnection, WebPkiServerVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::unbuffered::{ConnectionState, EncodeError, UnbufferedStatus};
use rustls::{ClientConfig, RootCertStore, SignatureScheme};
use rustls_pki_types::{alg_id, AlgorithmIdentifier, InvalidSignature, ServerName, SignatureVerificationAlgorithm, UnixTime};
#[cfg(test)] use rustls_pki_types::CertificateDer;
use sha2::{Digest, Sha256, Sha384, Sha512};

mod provider;
mod x25519;
pub use crate::Failure;
/// Plaintext either way, and encrypted bytes waiting to go out, are held to this much.
const QUEUE: usize = 16 * 1024;
/// Encrypted bytes from the site: room for the largest record (TLS 1.2 allows 2^14
/// plus 2048 of expansion, and a 5-byte header) and a little more.
const WIRE_IN: usize = 16384 + 2048 + 5 + 1024;

macro_rules! rsa_pkcs1 { ($name:ident, $hash:ty, $sig:expr) => {
    #[derive(Debug)] struct $name;
    impl SignatureVerificationAlgorithm for $name {
        fn verify_signature(&self, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), InvalidSignature> {
            let key = RsaPublicKey::from_pkcs1_der(key).map_err(|_| InvalidSignature)?;
            let signature = rsa::pkcs1v15::Signature::try_from(signature).map_err(|_| InvalidSignature)?;
            rsa::pkcs1v15::VerifyingKey::<$hash>::new(key).verify(message, &signature).map_err(|_| InvalidSignature)
        }
        fn public_key_alg_id(&self) -> AlgorithmIdentifier { alg_id::RSA_ENCRYPTION }
        fn signature_alg_id(&self) -> AlgorithmIdentifier { $sig }
    }
}}
macro_rules! rsa_pss { ($name:ident, $hash:ty, $sig:expr) => {
    #[derive(Debug)] struct $name;
    impl SignatureVerificationAlgorithm for $name {
        fn verify_signature(&self, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), InvalidSignature> {
            let key = RsaPublicKey::from_pkcs1_der(key).map_err(|_| InvalidSignature)?;
            let signature = rsa::pss::Signature::try_from(signature).map_err(|_| InvalidSignature)?;
            rsa::pss::VerifyingKey::<$hash>::new(key).verify(message, &signature).map_err(|_| InvalidSignature)
        }
        fn public_key_alg_id(&self) -> AlgorithmIdentifier { alg_id::RSA_ENCRYPTION }
        fn signature_alg_id(&self) -> AlgorithmIdentifier { $sig }
    }
}}
rsa_pkcs1!(RsaPkcs1Sha256, Sha256, alg_id::RSA_PKCS1_SHA256);
rsa_pkcs1!(RsaPkcs1Sha384, Sha384, alg_id::RSA_PKCS1_SHA384);
rsa_pkcs1!(RsaPkcs1Sha512, Sha512, alg_id::RSA_PKCS1_SHA512);
rsa_pss!(RsaPssSha256, Sha256, alg_id::RSA_PSS_SHA256);
rsa_pss!(RsaPssSha384, Sha384, alg_id::RSA_PSS_SHA384);
rsa_pss!(RsaPssSha512, Sha512, alg_id::RSA_PSS_SHA512);

#[derive(Debug)] struct P256Sha256;
impl SignatureVerificationAlgorithm for P256Sha256 {
    fn verify_signature(&self, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), InvalidSignature> {
        let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(key).map_err(|_| InvalidSignature)?;
        let signature = p256::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        p256::ecdsa::signature::Verifier::verify(&key, message, &signature).map_err(|_| InvalidSignature)
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_P256 }
    fn signature_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_SHA256 }
}
#[derive(Debug)] struct P384Sha384;
impl SignatureVerificationAlgorithm for P384Sha384 {
    fn verify_signature(&self, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), InvalidSignature> {
        let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(key).map_err(|_| InvalidSignature)?;
        let signature = p384::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        p384::ecdsa::signature::Verifier::verify(&key, message, &signature).map_err(|_| InvalidSignature)
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_P384 }
    fn signature_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_SHA384 }
}
/// ECDSA P-384 keys may sign with SHA-256, and P-256 keys with SHA-384. Roots do both.
#[derive(Debug)] struct P384Sha256;
impl SignatureVerificationAlgorithm for P384Sha256 {
    fn verify_signature(&self, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), InvalidSignature> {
        let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(key).map_err(|_| InvalidSignature)?;
        let signature = p384::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        use p384::ecdsa::signature::hazmat::PrehashVerifier;
        key.verify_prehash(&Sha256::digest(message), &signature).map_err(|_| InvalidSignature)
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_P384 }
    fn signature_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_SHA256 }
}
#[derive(Debug)] struct P256Sha384;
impl SignatureVerificationAlgorithm for P256Sha384 {
    fn verify_signature(&self, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), InvalidSignature> {
        let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(key).map_err(|_| InvalidSignature)?;
        let signature = p256::ecdsa::Signature::from_der(signature).map_err(|_| InvalidSignature)?;
        use p256::ecdsa::signature::hazmat::PrehashVerifier;
        key.verify_prehash(&Sha384::digest(message), &signature).map_err(|_| InvalidSignature)
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_P256 }
    fn signature_alg_id(&self) -> AlgorithmIdentifier { alg_id::ECDSA_SHA384 }
}
static ALGORITHMS: &[&dyn SignatureVerificationAlgorithm] = &[&RsaPkcs1Sha256, &RsaPkcs1Sha384, &RsaPkcs1Sha512, &RsaPssSha256, &RsaPssSha384, &RsaPssSha512, &P256Sha256, &P256Sha384, &P384Sha384, &P384Sha256];
/// What each handshake signature scheme is checked with. ECDSA in TLS 1.2 names only
/// the hash, so either curve's key may answer for it.
static SUPPORTED: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms { all: ALGORITHMS, mapping: &[
    (SignatureScheme::ECDSA_NISTP256_SHA256, &[&P256Sha256, &P384Sha256]),
    (SignatureScheme::ECDSA_NISTP384_SHA384, &[&P384Sha384, &P256Sha384]),
    (SignatureScheme::RSA_PSS_SHA256, &[&RsaPssSha256]), (SignatureScheme::RSA_PSS_SHA384, &[&RsaPssSha384]), (SignatureScheme::RSA_PSS_SHA512, &[&RsaPssSha512]),
    (SignatureScheme::RSA_PKCS1_SHA256, &[&RsaPkcs1Sha256]), (SignatureScheme::RSA_PKCS1_SHA384, &[&RsaPkcs1Sha384]), (SignatureScheme::RSA_PKCS1_SHA512, &[&RsaPkcs1Sha512]),
] };

#[cfg(not(test))]
fn roots() -> &'static [rustls_pki_types::TrustAnchor<'static>] {
    // QEMU gate builds only: one more root, the gate's own (hex DER, set when the image
    // is built), so the gate's local HTTPS servers can be reached. Never set otherwise.
    if let Some(hex) = option_env!("SEXOS_TLS_GATE_ROOT") { return with_gate_root(hex); }
    webpki_roots::TLS_SERVER_ROOTS
}
#[cfg(not(test))]
fn with_gate_root(hex: &str) -> &'static [rustls_pki_types::TrustAnchor<'static>] {
    use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    static ROOTS: AtomicPtr<rustls_pki_types::TrustAnchor<'static>> = AtomicPtr::new(core::ptr::null_mut());
    static LEN: AtomicUsize = AtomicUsize::new(0);
    let made = ROOTS.load(Ordering::Acquire);
    if !made.is_null() { return unsafe { core::slice::from_raw_parts(made, LEN.load(Ordering::Acquire)) }; }
    let digit = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
    let der: Vec<u8> = hex.as_bytes().chunks_exact(2).map(|p| digit(p[0]) << 4 | digit(p[1])).collect();
    let der: &'static rustls_pki_types::CertificateDer<'static> = alloc::boxed::Box::leak(alloc::boxed::Box::new(rustls_pki_types::CertificateDer::from(alloc::boxed::Box::leak(der.into_boxed_slice()) as &'static [u8])));
    let mut all = webpki_roots::TLS_SERVER_ROOTS.to_vec();
    if let Ok(anchor) = webpki::anchor_from_trusted_cert(der) { all.push(anchor); }
    let all: &'static mut [rustls_pki_types::TrustAnchor<'static>] = alloc::boxed::Box::leak(all.into_boxed_slice());
    LEN.store(all.len(), Ordering::Release);
    ROOTS.store(all.as_mut_ptr(), Ordering::Release);
    all
}
/// Host tests check a peer against a root of their own; the program never can.
#[cfg(test)]
static TEST_ROOTS: std::sync::Mutex<Vec<rustls_pki_types::TrustAnchor<'static>>> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
fn roots() -> &'static [rustls_pki_types::TrustAnchor<'static>] { std::boxed::Box::leak(TEST_ROOTS.lock().unwrap().clone().into_boxed_slice()) }

/// Certificates are checked at the time the service read when the session started.
#[derive(Debug)]
struct Clock(u32);
impl rustls::time_provider::TimeProvider for Clock {
    fn current_time(&self) -> Option<UnixTime> { Some(UnixTime::since_unix_epoch(core::time::Duration::from_secs(self.0 as u64))) }
}
fn config(seconds: u32) -> Result<ClientConfig, rustls::Error> {
    let provider = Arc::new(provider::provider(SUPPORTED));
    let mut store = RootCertStore::empty();
    store.extend(roots().iter().cloned());
    let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(store), provider.clone()).build()
        .map_err(|_| rustls::Error::General("no roots".into()))?;
    let mut config = ClientConfig::builder_with_details(provider, Arc::new(Clock(seconds)))
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])?
        .dangerous().with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = alloc::vec![b"http/1.1".to_vec()];
    Ok(config)
}
impl Failure {
    fn from_rustls(error: &rustls::Error) -> Self {
        use rustls::{CertificateError, Error as E};
        match error {
            E::InvalidCertificate(CertificateError::BadSignature
                | CertificateError::UnsupportedSignatureAlgorithmContext { .. } | CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. }) => Self::Signature,
            E::InvalidCertificate(_) | E::NoCertificatesPresented | E::InvalidCertRevocationList(_) => Self::Certificate,
            E::PeerMisbehaved(rustls::PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme | rustls::PeerMisbehaved::SignedKxWithWrongAlgorithm) => Self::Signature,
            E::DecryptError | E::InvalidMessage(_) => Self::Record,
            E::AlertReceived(_) | E::PeerIncompatible(_) | E::PeerMisbehaved(_) | E::InappropriateHandshakeMessage { .. } | E::InappropriateMessage { .. } => Self::Handshake,
            _ => Self::Protocol,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Handshake, Ready, Finished, Failed(Option<Failure>) }
pub struct Session { conn: Option<UnbufferedClientConnection>, wire_in: Vec<u8>, wire_out: Vec<u8>, plain_out: Vec<u8>, plain_in: Vec<u8>, peer_closed: bool, stage: Stage }
impl Session {
    /// Starts the handshake with `host`. The kernel-seeded `seed` is mixed into the
    /// service's generator and wiped; `seconds` is the time certificates are checked
    /// at. The session sizes its own queues.
    pub fn start(host: &str, seed: &mut [u8; 32], seconds: u32) -> Result<Self, crate::Error> {
        provider::reseed(seed);
        crate::erase(seed);
        if !(1_577_836_800..=4_102_444_799).contains(&seconds) { return Err(crate::Error::Clock); }
        let name = ServerName::try_from(host).map_err(|_| crate::Error::Policy)?.to_owned();
        let config = config(seconds).map_err(|_| crate::Error::Policy)?;
        let conn = UnbufferedClientConnection::new(Arc::new(config), name).map_err(|_| crate::Error::Policy)?;
        let mut session = Self { conn: Some(conn), wire_in: Vec::new(), wire_out: Vec::new(), plain_out: Vec::new(), plain_in: Vec::new(), peer_closed: false, stage: Stage::Handshake };
        session.wire_in.try_reserve_exact(WIRE_IN).map_err(|_| crate::Error::Memory)?;
        Ok(session)
    }
    fn fail(&mut self, failure: Option<Failure>) { self.conn = None; self.stage = Stage::Failed(failure); }
    /// Runs the protocol as far as the bytes at hand allow.
    pub fn poll(&mut self) -> Stage {
        for _ in 0..64 {
            let Some(conn) = self.conn.as_mut() else { break };
            let UnbufferedStatus { mut discard, state } = conn.process_tls_records(&mut self.wire_in);
            let mut more = true;
            match state {
                Err(error) => {
                    // A site that drops TCP without a closing alert, after replying, ended normally.
                    let failure = Failure::from_rustls(&error);
                    self.conn = None;
                    self.stage = if self.stage == Stage::Ready && self.peer_closed { Stage::Finished } else { Stage::Failed(Some(failure)) };
                    break;
                }
                Ok(ConnectionState::ReadTraffic(mut traffic)) => {
                    while let Some(record) = traffic.next_record() {
                        match record {
                            Ok(record) => { discard += record.discard; self.plain_in.extend_from_slice(record.payload); }
                            Err(error) => { let f = Failure::from_rustls(&error); drop(traffic); self.fail(Some(f)); return self.stage; }
                        }
                    }
                }
                Ok(ConnectionState::EncodeTlsData(mut encode)) => {
                    let at = self.wire_out.len();
                    self.wire_out.resize(at + 4096, 0);
                    match encode.encode(&mut self.wire_out[at..]) {
                        Ok(n) => self.wire_out.truncate(at + n),
                        Err(EncodeError::InsufficientSize(need)) => {
                            self.wire_out.resize(at + need.required_size, 0);
                            match encode.encode(&mut self.wire_out[at..]) {
                                Ok(n) => self.wire_out.truncate(at + n),
                                Err(_) => { self.wire_out.truncate(at); drop(encode); self.fail(Some(Failure::Protocol)); return self.stage; }
                            }
                        }
                        Err(_) => { self.wire_out.truncate(at); drop(encode); self.fail(Some(Failure::Protocol)); return self.stage; }
                    }
                }
                // Encoded bytes wait in `wire_out` for the caller to take.
                Ok(ConnectionState::TransmitTlsData(transmit)) => transmit.done(),
                Ok(ConnectionState::BlockedHandshake) => {
                    more = false;
                    // Nothing more will come: a partial record left now can never complete.
                    if self.peer_closed { self.fail(Some(Failure::Handshake)); return self.stage; }
                }
                Ok(ConnectionState::WriteTraffic(mut write)) => {
                    self.stage = Stage::Ready;
                    more = false;
                    // Plaintext goes out in records of up to 16 KiB while there's room.
                    if !self.plain_out.is_empty() && self.wire_out.len() < QUEUE {
                        let take = self.plain_out.len().min(16384);
                        let at = self.wire_out.len();
                        self.wire_out.resize(at + take + 64, 0);
                        match write.encrypt(&self.plain_out[..take], &mut self.wire_out[at..]) {
                            Ok(n) => { self.wire_out.truncate(at + n); self.plain_out.drain(..take); more = true; }
                            Err(_) => { self.wire_out.truncate(at); drop(write); self.fail(Some(Failure::Application)); return self.stage; }
                        }
                    } else if self.peer_closed {
                        // The site closed; any partial record left behind is cut short.
                        drop(write);
                        self.conn = None;
                        self.stage = Stage::Finished;
                        return self.stage;
                    }
                }
                Ok(ConnectionState::PeerClosed | ConnectionState::Closed) => { more = false; self.conn = None; self.stage = Stage::Finished; }
                Ok(_) => { more = false; }
            }
            self.wire_in.drain(..discard);
            if !more && discard == 0 { break; }
        }
        self.stage
    }
    pub fn stage(&self) -> Stage { self.stage }
    /// Room for encrypted bytes from the site. None while the reader has a full
    /// queue of plaintext still to take: that holds the site back.
    pub fn wire_room(&self) -> usize { if self.plain_in.len() >= QUEUE { 0 } else { WIRE_IN.saturating_sub(self.wire_in.len()) } }
    pub fn offer_wire(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(WIRE_IN.saturating_sub(self.wire_in.len()));
        self.wire_in.extend_from_slice(&bytes[..n]);
    }
    /// The site will send nothing more: every byte it sent has been offered.
    pub fn peer_closed(&mut self) { self.peer_closed = true; }
    pub fn take_wire(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.wire_out.len());
        out[..n].copy_from_slice(&self.wire_out[..n]);
        self.wire_out.drain(..n);
        n
    }
    pub fn wire_waiting(&self) -> usize { self.wire_out.len() }
    pub fn send(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(QUEUE.saturating_sub(self.plain_out.len()));
        self.plain_out.extend_from_slice(&bytes[..n]);
        n
    }
    /// Plaintext goes out as soon as it's queued; kept for callers that mark the end
    /// of each request.
    pub fn finish_request(&mut self) { self.poll(); }
    pub fn receive(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.plain_in.len());
        out[..n].copy_from_slice(&self.plain_in[..n]);
        self.plain_in.drain(..n);
        n
    }
    pub fn drained(&self) -> bool { self.plain_in.is_empty() && matches!(self.stage, Stage::Finished | Stage::Failed(_)) }
    /// The protocol version and suite agreed, once the handshake is done.
    pub fn agreed(&self) -> Option<(rustls::ProtocolVersion, rustls::CipherSuite, Option<rustls::NamedGroup>)> {
        let conn = self.conn.as_ref()?;
        Some((conn.protocol_version()?, conn.negotiated_cipher_suite()?.suite(), conn.negotiated_key_exchange_group().map(|g| g.name())))
    }
}

#[cfg(test)]
mod tests {
    //! Real peers on the host (Python's ssl and `openssl s_server`), with roots made for
    //! each test: keep-alive, every suite and group, and refusal of a stranger.
    use super::*;
    use std::{boxed::Box, format, io::{BufRead, BufReader, Read, Write}, net::TcpStream, process::{Command, Stdio}, string::String, time::{Duration, Instant, SystemTime, UNIX_EPOCH}, vec};
    const HOST: &str = "keepalive.sexos.test";
    const PEER: &str = r#"
import socket,ssl,sys
c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.minimum_version=ssl.TLSVersion.TLSv1_3;c.set_ecdh_curve('prime256v1');c.num_tickets=0
c.load_cert_chain(sys.argv[1],sys.argv[2])
s=socket.socket();s.bind(('127.0.0.1',0));s.listen(4);print(s.getsockname()[1],flush=True)
conns=0;reqs=0
while reqs<3:
 raw,_=s.accept();conns+=1
 t=c.wrap_socket(raw,server_side=True);f=t.makefile('rb')
 while True:
  line=f.readline()
  if not line:break
  while f.readline() not in (b'\r\n',b''):pass
  reqs+=1;body=('reply %d to %s'%(reqs,line.split()[1].decode())).encode()
  t.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n'%len(body)+body)
  if reqs==3:break
 t.close()
print('connections=%d requests=%d'%(conns,reqs),flush=True)
"#;
    fn openssl(dir: &std::path::Path, args: &[&str]) { assert!(Command::new("openssl").args(args).current_dir(dir).stderr(Stdio::null()).status().unwrap().success(), "{:?}", args); }

    /// A root and a leaf for `host`, signed with `key` ("ec:P-256", "ec:P-384" or "rsa:2048").
    fn chain(dir: &std::path::Path, host: &str, key: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("leaf.ext"), format!("subjectAltName=DNS:{host}\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n")).unwrap();
        let new_key: std::vec::Vec<&str> = match key.split_once(':').unwrap() { ("ec", curve) => vec!["-newkey", "ec", "-pkeyopt", if curve == "P-256" { "ec_paramgen_curve:P-256" } else { "ec_paramgen_curve:P-384" }], _ => vec!["-newkey", "rsa:2048"] };
        let mut ca = vec!["req", "-x509"]; ca.extend(&new_key); ca.extend(["-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-days", "30", "-subj", "/CN=SexOS matrix root", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign"]);
        openssl(dir, &ca);
        let subject = format!("/CN={host}");
        let mut leaf = vec!["req"]; leaf.extend(&new_key); leaf.extend(["-nodes", "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", &subject]);
        openssl(dir, &leaf);
        openssl(dir, &["x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "leaf.pem", "-days", "30", "-extfile", "leaf.ext"]);
        openssl(dir, &["x509", "-in", "ca.pem", "-outform", "DER", "-out", "ca.der"]);
        let der: &'static CertificateDer<'static> = Box::leak(Box::new(CertificateDer::from(Box::leak(std::fs::read(dir.join("ca.der")).unwrap().into_boxed_slice()) as &'static [u8])));
        TEST_ROOTS.lock().unwrap().push(webpki::anchor_from_trusted_cert(der).unwrap());
    }

    /// Every version, suite and group the session offers, each against OpenSSL's own
    /// server held to just that one: the handshake completes, the agreed parameters
    /// are the ones forced, and a request gets its reply.
    #[test]
    fn every_suite_and_group_meets_openssl() {
        use rustls::{CipherSuite as C, NamedGroup as G, ProtocolVersion as V};
        let cases: &[(&str, &str, &str, &str, V, C, G)] = &[
            ("ec:P-256", "-tls1_3", "TLS_AES_128_GCM_SHA256", "X25519", V::TLSv1_3, C::TLS13_AES_128_GCM_SHA256, G::X25519),
            ("ec:P-256", "-tls1_3", "TLS_AES_256_GCM_SHA384", "P-256", V::TLSv1_3, C::TLS13_AES_256_GCM_SHA384, G::secp256r1),
            ("rsa:2048", "-tls1_3", "TLS_CHACHA20_POLY1305_SHA256", "P-384", V::TLSv1_3, C::TLS13_CHACHA20_POLY1305_SHA256, G::secp384r1),
            ("ec:P-384", "-tls1_3", "TLS_AES_128_GCM_SHA256", "X25519", V::TLSv1_3, C::TLS13_AES_128_GCM_SHA256, G::X25519),
            ("ec:P-256", "-tls1_2", "ECDHE-ECDSA-AES128-GCM-SHA256", "X25519", V::TLSv1_2, C::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256, G::X25519),
            ("ec:P-384", "-tls1_2", "ECDHE-ECDSA-AES256-GCM-SHA384", "P-384", V::TLSv1_2, C::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384, G::secp384r1),
            ("ec:P-256", "-tls1_2", "ECDHE-ECDSA-CHACHA20-POLY1305", "P-256", V::TLSv1_2, C::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256, G::secp256r1),
            ("rsa:2048", "-tls1_2", "ECDHE-RSA-AES128-GCM-SHA256", "P-256", V::TLSv1_2, C::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, G::secp256r1),
            ("rsa:2048", "-tls1_2", "ECDHE-RSA-AES256-GCM-SHA384", "X25519", V::TLSv1_2, C::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384, G::X25519),
            ("rsa:2048", "-tls1_2", "ECDHE-RSA-CHACHA20-POLY1305", "X25519", V::TLSv1_2, C::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256, G::X25519),
        ];
        for (n, (key, version, suite, group, want_version, want_suite, want_group)) in cases.iter().enumerate() {
            let host = format!("m{n}.matrix.sexos.test");
            let dir = std::env::temp_dir().join(format!("sexos-tls-matrix-{}-{n}", std::process::id()));
            chain(&dir, &host, key);
            // A second on, so a certificate made this second is already valid.
            let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32 + 1;
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let suite_flag = if *version == "-tls1_3" { "-ciphersuites" } else { "-cipher" };
            let mut server = Command::new("openssl").current_dir(&dir).args(["s_server", "-quiet", "-www", "-accept", &format!("{port}"), "-cert", "leaf.pem", "-key", "leaf.key",
                version, suite_flag, suite, "-groups", group, "-naccept", "1"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            let began = Instant::now();
            let mut socket = loop {
                match TcpStream::connect(("127.0.0.1", port)) { Ok(s) => break s, Err(_) if began.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(20)), Err(e) => panic!("{e}") }
            };
            socket.set_nonblocking(true).unwrap();
            let mut session = Session::start(&host, &mut [n as u8 + 1; 32], seconds).unwrap();
            let request = format!("GET / HTTP/1.0\r\nHost: {host}\r\n\r\n");
            let (mut sent, mut got) = (false, std::vec::Vec::new());
            loop {
                let mut chunk = [0u8; 8192];
                match socket.read(&mut chunk[..session.wire_room().min(8192)]) { Ok(0) => session.peer_closed(), Ok(k) => session.offer_wire(&chunk[..k]), Err(_) => {} }
                session.poll();
                if session.stage() == Stage::Ready && !sent {
                    assert_eq!(session.agreed(), Some((*want_version, *want_suite, Some(*want_group))), "case {n}: {suite} {group}");
                    assert_eq!(session.send(request.as_bytes()), request.len());
                    session.finish_request();
                    sent = true;
                }
                let k = session.take_wire(&mut chunk);
                if k > 0 { let _ = socket.write_all(&chunk[..k]); }
                let k = session.receive(&mut chunk);
                got.extend_from_slice(&chunk[..k]);
                assert!(!matches!(session.stage(), Stage::Failed(_)), "case {n} {suite} {group}: {:?}", session.stage());
                if session.drained() || got.windows(7).any(|w| w == b"</HTML>" || w == b"</html>") { break; }
                assert!(began.elapsed() < Duration::from_secs(20), "case {n} stuck at {:?}", session.stage());
                std::thread::sleep(Duration::from_millis(1));
            }
            let text = String::from_utf8_lossy(&got);
            assert!(sent && text.starts_with("HTTP/1.0 200"), "case {n}: {text}");
            let _ = server.kill();
            let _ = server.wait();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// A server whose certificate the public roots don't vouch for is refused, and the
    /// failure says it was the certificate.
    #[test]
    fn an_untrusted_certificate_is_refused() {
        let dir = std::env::temp_dir().join(format!("sexos-tls-untrusted-{}", std::process::id()));
        // Some root is trusted, just not one that signed this server.
        chain(&dir.join("other"), "other.sexos.test", "ec:P-256");
        // A self-signed leaf that no test root (and no public root) signed.
        openssl(&dir, &["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-keyout", "leaf.key", "-out", "leaf.pem", "-days", "30", "-subj", "/CN=stranger.sexos.test", "-addext", "subjectAltName=DNS:stranger.sexos.test"]);
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut server = Command::new("openssl").current_dir(&dir).args(["s_server", "-quiet", "-www", "-accept", &format!("{port}"), "-cert", "leaf.pem", "-key", "leaf.key", "-naccept", "1"])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let began = Instant::now();
        let mut socket = loop {
            match TcpStream::connect(("127.0.0.1", port)) { Ok(s) => break s, Err(_) if began.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(20)), Err(e) => panic!("{e}") }
        };
        socket.set_nonblocking(true).unwrap();
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        let mut session = Session::start("stranger.sexos.test", &mut [9; 32], seconds).unwrap();
        while !matches!(session.stage(), Stage::Failed(_)) {
            let mut chunk = [0u8; 8192];
            match socket.read(&mut chunk[..session.wire_room().min(8192)]) { Ok(0) => session.peer_closed(), Ok(k) => session.offer_wire(&chunk[..k]), Err(_) => {} }
            session.poll();
            let k = session.take_wire(&mut chunk);
            if k > 0 { let _ = socket.write_all(&chunk[..k]); }
            assert_ne!(session.stage(), Stage::Ready);
            assert!(began.elapsed() < Duration::from_secs(20), "stuck at {:?}", session.stage());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(session.stage(), Stage::Failed(Some(Failure::Certificate)));
        let _ = server.kill();
        let _ = server.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A site that sends part of a record and hangs up ends the session, never leaves
    /// it waiting for bytes that can't come.
    #[test]
    fn a_record_cut_short_by_the_close_ends_the_session() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut hello = [0u8; 2048];
            let _ = s.read(&mut hello);
            // A handshake record header promising 512 bytes, then three of them.
            s.write_all(&[22, 3, 3, 2, 0, 2, 0, 0]).unwrap();
        });
        let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket.set_nonblocking(true).unwrap();
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        chain(&std::env::temp_dir().join(format!("sexos-tls-cut-{}", std::process::id())), "cut.sexos.test", "ec:P-256");
        let mut session = Session::start("cut.sexos.test", &mut [5; 32], seconds).unwrap();
        let began = Instant::now();
        while !matches!(session.stage(), Stage::Failed(_)) {
            let mut chunk = [0u8; 4096];
            match socket.read(&mut chunk) { Ok(0) => session.peer_closed(), Ok(k) => session.offer_wire(&chunk[..k]), Err(_) => {} }
            session.poll();
            let k = session.take_wire(&mut chunk);
            if k > 0 { let _ = socket.write_all(&chunk[..k]); }
            assert!(began.elapsed() < Duration::from_secs(10), "stuck at {:?}", session.stage());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(session.drained());
        peer.join().unwrap();
    }

    #[test]
    fn one_session_carries_several_requests() {
        let dir = std::env::temp_dir().join(format!("sexos-open-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("leaf.ext"), format!("subjectAltName=DNS:{HOST}\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n")).unwrap();
        openssl(&dir, &["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-days", "30", "-subj", "/CN=SexOS keep-alive test root", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign"]);
        openssl(&dir, &["req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", &format!("/CN={HOST}")]);
        openssl(&dir, &["x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "leaf.pem", "-days", "30", "-extfile", "leaf.ext"]);
        openssl(&dir, &["x509", "-in", "ca.pem", "-outform", "DER", "-out", "ca.der"]);
        let der: &'static CertificateDer<'static> = Box::leak(Box::new(CertificateDer::from(Box::leak(std::fs::read(dir.join("ca.der")).unwrap().into_boxed_slice()) as &'static [u8])));
        TEST_ROOTS.lock().unwrap().push(webpki::anchor_from_trusted_cert(der).unwrap());

        let mut peer = Command::new("python3").args(["-c", PEER, dir.join("leaf.pem").to_str().unwrap(), dir.join("leaf.key").to_str().unwrap()]).stdout(Stdio::piped()).spawn().unwrap();
        let mut out = BufReader::new(peer.stdout.take().unwrap());
        let mut port = String::new();
        out.read_line(&mut port).unwrap();
        let mut socket = TcpStream::connect(("127.0.0.1", port.trim().parse::<u16>().unwrap())).unwrap();
        socket.set_nonblocking(true).unwrap();

        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        let mut session = Session::start(HOST, &mut [7u8; 32], seconds).unwrap();
        // Moves bytes between the socket and the session until `done` says so.
        let mut pump = |session: &mut Session, done: &mut dyn FnMut(&mut Session) -> bool| {
            let began = Instant::now();
            loop {
                let mut chunk = [0u8; 4096];
                match socket.read(&mut chunk[..session.wire_room().min(4096)]) { Ok(0) => session.peer_closed(), Ok(n) => session.offer_wire(&chunk[..n]), Err(_) => {} }
                session.poll();
                let n = session.take_wire(&mut chunk);
                if n > 0 { socket.write_all(&chunk[..n]).unwrap(); }
                if done(session) { return; }
                assert!(!matches!(session.stage(), Stage::Failed(_)), "{:?}", session.stage());
                assert!(began.elapsed() < Duration::from_secs(20), "stuck at {:?}", session.stage());
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        pump(&mut session, &mut |s| s.stage() == Stage::Ready);
        for n in 1..=3 {
            let request = format!("GET /item/{n} HTTP/1.1\r\nHost: {HOST}\r\n\r\n");
            assert_eq!(session.send(request.as_bytes()), request.len());
            session.finish_request();
            let want = format!("reply {n} to /item/{n}");
            let mut got = Vec::new();
            pump(&mut session, &mut |s| {
                let mut buf = [0u8; 512];
                let k = s.receive(&mut buf);
                got.extend_from_slice(&buf[..k]);
                got.ends_with(want.as_bytes())
            });
            assert!(String::from_utf8_lossy(&got).starts_with("HTTP/1.1 200 OK"), "{}", String::from_utf8_lossy(&got));
        }
        let mut summary = String::new();
        out.read_line(&mut summary).unwrap();
        assert_eq!(summary.trim(), "connections=1 requests=3");
        let _ = peer.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
