// SPDX-License-Identifier: GPL-3.0-only
//! TLS 1.3 to any host, checked against the public root store. The session stays open
//! for as many requests as the site allows (HTTP keep-alive): the caller queues
//! plaintext and marks each request finished, which sends it, and reads replies as
//! they come. Bytes on the wire and plaintext both pass through this type, and the
//! caller moves them.
use alloc::{rc::Rc, vec::Vec};
use core::{cell::RefCell, future::{poll_fn, Future}, pin::pin, task::{Context, Poll, Waker}};
use embedded_io_async::{Read as _, Write as _};
#[cfg(test)] extern crate std;
use embedded_tls::{Aes128GcmSha256, CertificateEntryRef, CertificateRef, CertificateVerifyRef, CryptoProvider, SignatureScheme, TlsConfig, TlsConnection, TlsContext, TlsError, TlsVerifier};
use rand_core::SeedableRng;
use rsa::{pkcs1::DecodeRsaPublicKey, signature::Verifier as _, RsaPublicKey};
use rustls_pki_types::{alg_id, AlgorithmIdentifier, CertificateDer, InvalidSignature, ServerName, SignatureVerificationAlgorithm, UnixTime};
use sha2::{Digest, Sha256, Sha384, Sha512};

pub use crate::Failure;
const QUEUE: usize = 16 * 1024;
const SERVER_AUTH: &[u8] = &[0x2b, 6, 1, 5, 5, 7, 3, 1];
const MAX_CHAIN: usize = 8;
/// Application protocols named in the handshake (ALPN): only HTTP/1.1 for now.
const ALPN: &[&[u8]] = &[b"http/1.1"];

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

#[cfg(not(test))]
fn roots() -> &'static [rustls_pki_types::TrustAnchor<'static>] { webpki_roots::TLS_SERVER_ROOTS }
/// Host tests check a peer against a root of their own; the program never can.
#[cfg(test)]
static TEST_ROOTS: std::sync::Mutex<Vec<rustls_pki_types::TrustAnchor<'static>>> = std::sync::Mutex::new(Vec::new());
#[cfg(test)]
fn roots() -> &'static [rustls_pki_types::TrustAnchor<'static>] { std::boxed::Box::leak(TEST_ROOTS.lock().unwrap().clone().into_boxed_slice()) }

struct Verifier { host: Vec<u8>, seconds: u32, leaf: Vec<u8>, transcript: [u8; 32], certificate_seen: bool, signature_seen: bool }
impl Verifier {
    fn chain(&mut self, cert: CertificateRef) -> Result<(), TlsError> {
        if cert.entries.is_empty() || cert.entries.len() > MAX_CHAIN { return Err(TlsError::InvalidCertificate); }
        let mut ders: Vec<CertificateDer> = Vec::new();
        for entry in cert.entries.iter() {
            let CertificateEntryRef::X509(bytes) = entry else { return Err(TlsError::InvalidCertificate) };
            ders.push(CertificateDer::from(*bytes));
        }
        let leaf = webpki::EndEntityCert::try_from(&ders[0]).map_err(|_| TlsError::InvalidCertificate)?;
        leaf.verify_for_usage(ALGORITHMS, roots(), &ders[1..],
            UnixTime::since_unix_epoch(core::time::Duration::from_secs(self.seconds as u64)),
            webpki::KeyUsage::required(SERVER_AUTH), None, None).map_err(|_| TlsError::InvalidCertificate)?;
        let host = core::str::from_utf8(&self.host).map_err(|_| TlsError::InvalidCertificate)?;
        let name = ServerName::try_from(host).map_err(|_| TlsError::InvalidCertificate)?;
        leaf.verify_is_valid_for_subject_name(&name).map_err(|_| TlsError::InvalidCertificate)?;
        self.leaf.clear();
        self.leaf.extend_from_slice(ders[0].as_ref());
        Ok(())
    }
}
impl TlsVerifier<Aes128GcmSha256> for Verifier {
    fn set_hostname_verification(&mut self, name: &str) -> Result<(), TlsError> {
        if name.as_bytes() != self.host.as_slice() { return Err(TlsError::InvalidCertificate); }
        Ok(())
    }
    fn verify_certificate(&mut self, transcript: &Sha256, cert: CertificateRef) -> Result<(), TlsError> {
        self.chain(cert)?;
        self.transcript.copy_from_slice(&transcript.clone().finalize());
        self.certificate_seen = true;
        Ok(())
    }
    fn verify_signature(&mut self, verify: CertificateVerifyRef) -> Result<(), TlsError> {
        if !self.certificate_seen { return Err(TlsError::InvalidCertificate); }
        let algorithm: &dyn SignatureVerificationAlgorithm = match verify.signature_scheme {
            SignatureScheme::EcdsaSecp256r1Sha256 => &P256Sha256,
            SignatureScheme::EcdsaSecp384r1Sha384 => &P384Sha384,
            SignatureScheme::RsaPssRsaeSha256 => &RsaPssSha256,
            SignatureScheme::RsaPssRsaeSha384 => &RsaPssSha384,
            SignatureScheme::RsaPssRsaeSha512 => &RsaPssSha512,
            _ => return Err(TlsError::InvalidSignatureScheme),
        };
        let mut message = [b' '; 130];
        message[64..98].copy_from_slice(b"TLS 1.3, server CertificateVerify\0");
        message[98..].copy_from_slice(&self.transcript);
        let der = CertificateDer::from(self.leaf.as_slice());
        let cert = webpki::EndEntityCert::try_from(&der).map_err(|_| TlsError::InvalidCertificate)?;
        cert.verify_signature(algorithm, &message, verify.signature).map_err(|_| TlsError::InvalidSignature)?;
        self.signature_seen = true;
        Ok(())
    }
}
struct Provider { rng: rand_chacha::ChaCha20Rng, verifier: Verifier }
impl CryptoProvider for Provider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = [u8; 0];
    fn rng(&mut self) -> impl rand_core::CryptoRngCore { &mut self.rng }
    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>, TlsError> { Ok(&mut self.verifier) }
}

struct Pipe { wire_in: Vec<u8>, wire_out: Vec<u8>, plain_out: Vec<u8>, plain_in: Vec<u8>, request_done: bool, peer_closed: bool, ready: bool }
#[derive(Clone)]
struct Wire(Rc<RefCell<Pipe>>);
impl embedded_io::ErrorType for Wire { type Error = embedded_io::ErrorKind; }
impl embedded_io_async::Read for Wire {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        poll_fn(|_| {
            if out.is_empty() { return Poll::Ready(Ok(0)); }
            let mut pipe = self.0.borrow_mut();
            if pipe.wire_in.is_empty() {
                return if pipe.peer_closed { Poll::Ready(Err(embedded_io::ErrorKind::ConnectionReset)) } else { Poll::Pending };
            }
            let n = out.len().min(pipe.wire_in.len());
            out[..n].copy_from_slice(&pipe.wire_in[..n]);
            pipe.wire_in.drain(..n);
            Poll::Ready(Ok(n))
        }).await
    }
}
impl embedded_io_async::Write for Wire {
    async fn write(&mut self, input: &[u8]) -> Result<usize, Self::Error> {
        poll_fn(|_| {
            if input.is_empty() { return Poll::Ready(Ok(0)); }
            let mut pipe = self.0.borrow_mut();
            if pipe.wire_out.len() >= QUEUE { return Poll::Pending; }
            let n = input.len().min(QUEUE - pipe.wire_out.len());
            pipe.wire_out.extend_from_slice(&input[..n]);
            Poll::Ready(Ok(n))
        }).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> { Ok(()) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Handshake, Ready, Finished, Failed(Option<Failure>) }
pub struct Session { task: Option<crate::Task>, pipe: Rc<RefCell<Pipe>>, stage: Stage }
impl Session {
    /// `read` needs room for a whole TLS record, 16,640 bytes, since few servers shrink them.
    pub fn start(host: &str, seed: &mut [u8; 32], seconds: u32, read: &'static mut [u8], write: &'static mut [u8]) -> Result<Self, crate::Error> {
        let private = crate::Seed(*seed);
        crate::erase(seed);
        if !(1_577_836_800..=4_102_444_799).contains(&seconds) { return Err(crate::Error::Clock); }
        let name = alloc::string::String::from(host);
        let verifier = Verifier { host: host.as_bytes().to_vec(), seconds, leaf: Vec::new(), transcript: [0; 32], certificate_seen: false, signature_seen: false };
        let pipe = Rc::new(RefCell::new(Pipe { wire_in: Vec::new(), wire_out: Vec::new(), plain_out: Vec::new(), plain_in: Vec::new(), request_done: false, peer_closed: false, ready: false }));
        let (wire, shared) = (Wire(pipe.clone()), pipe.clone());
        let task = crate::Task::new(async move {
            let mut provider = Provider { rng: rand_chacha::ChaCha20Rng::from_seed(private.0), verifier };
            drop(private);
            // ALPN: say we speak HTTP/1.1, as browsers do; some servers (YouTube's CDN among
            // them) expect a protocol named before they answer.
            let config = TlsConfig::new().with_server_name(&name).with_alpn(ALPN).enable_rsa_signatures();
            let mut tls = TlsConnection::<_, Aes128GcmSha256>::new(wire, read, write);
            tls.open(TlsContext::new(&config, &mut provider)).await?;
            // The library can accept Finished without its callbacks. Require both.
            if !provider.verifier.certificate_seen || !provider.verifier.signature_seen { return Err(TlsError::InvalidCertificate); }
            shared.borrow_mut().ready = true;
            let (mut reader, mut writer) = tls.split();
            let (to_send, to_read) = (shared.clone(), shared.clone());
            // Requests go out while replies come in, for as long as the site keeps the
            // connection: each finished request is flushed, then the writer waits again.
            let send = async move {
                let mut chunk = [0u8; 1024];
                loop {
                    let (take, flush) = poll_fn(|_| {
                        let mut pipe = to_send.borrow_mut();
                        if !pipe.plain_out.is_empty() {
                            let n = pipe.plain_out.len().min(chunk.len());
                            chunk[..n].copy_from_slice(&pipe.plain_out[..n]);
                            pipe.plain_out.drain(..n);
                            return Poll::Ready((n, false));
                        }
                        if pipe.request_done { pipe.request_done = false; return Poll::Ready((0, true)); }
                        Poll::Pending
                    }).await;
                    if flush { writer.flush().await?; continue; }
                    let mut at = 0;
                    while at < take {
                        let n = writer.write(&chunk[at..take]).await?;
                        if n == 0 { return Err(TlsError::InvalidApplicationData); }
                        at += n;
                    }
                }
                #[allow(unreachable_code)]
                Ok::<(), TlsError>(())
            };
            let receive = async move {
                let mut chunk = [0u8; 1024];
                loop {
                    poll_fn(|_| if to_read.borrow().plain_in.len() < QUEUE { Poll::Ready(()) } else { Poll::Pending }).await;
                    match reader.read(&mut chunk).await {
                        Ok(0) | Err(TlsError::ConnectionClosed) => break,
                        Ok(n) => to_read.borrow_mut().plain_in.extend_from_slice(&chunk[..n]),
                        // Plenty of servers drop TCP without a closing alert. The caller
                        // checks lengths, so this ends the reply instead of failing it.
                        Err(TlsError::Io(_)) if to_read.borrow().peer_closed => break,
                        Err(error) => return Err(error),
                    }
                }
                Ok(())
            };
            let (mut send, mut receive) = (pin!(send), pin!(receive));
            // The session ends when the site closes it (or either side fails).
            poll_fn(|cx| {
                if let Poll::Ready(done) = receive.as_mut().poll(cx) { return Poll::Ready(done); }
                match send.as_mut().poll(cx) { Poll::Ready(Err(e)) => Poll::Ready(Err(e)), _ => Poll::Pending }
            }).await?;
            Ok(())
        })?;
        Ok(Self { task: Some(task), pipe, stage: Stage::Handshake })
    }
    pub fn poll(&mut self) -> Stage {
        let Some(task) = self.task.as_mut() else { return self.stage };
        match task.poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Ok(())) => { self.task = None; self.stage = Stage::Finished; }
            Poll::Ready(Err(error)) => { self.task = None; self.stage = Stage::Failed(Some(Failure::from_error(error))); }
            Poll::Pending => if self.pipe.borrow().ready { self.stage = Stage::Ready; },
        }
        self.stage
    }
    pub fn stage(&self) -> Stage { self.stage }
    pub fn wire_room(&self) -> usize { QUEUE.saturating_sub(self.pipe.borrow().wire_in.len()) }
    pub fn offer_wire(&mut self, bytes: &[u8]) { self.pipe.borrow_mut().wire_in.extend_from_slice(bytes); }
    pub fn peer_closed(&mut self) { self.pipe.borrow_mut().peer_closed = true; }
    pub fn take_wire(&mut self, out: &mut [u8]) -> usize {
        let mut pipe = self.pipe.borrow_mut();
        let n = out.len().min(pipe.wire_out.len());
        out[..n].copy_from_slice(&pipe.wire_out[..n]);
        pipe.wire_out.drain(..n);
        n
    }
    pub fn wire_waiting(&self) -> usize { self.pipe.borrow().wire_out.len() }
    pub fn send(&mut self, bytes: &[u8]) -> usize {
        let mut pipe = self.pipe.borrow_mut();
        let n = bytes.len().min(QUEUE.saturating_sub(pipe.plain_out.len()));
        pipe.plain_out.extend_from_slice(&bytes[..n]);
        n
    }
    pub fn finish_request(&mut self) { self.pipe.borrow_mut().request_done = true; }
    pub fn receive(&mut self, out: &mut [u8]) -> usize {
        let mut pipe = self.pipe.borrow_mut();
        let n = out.len().min(pipe.plain_in.len());
        out[..n].copy_from_slice(&pipe.plain_in[..n]);
        pipe.plain_in.drain(..n);
        n
    }
    pub fn drained(&self) -> bool { self.pipe.borrow().plain_in.is_empty() && matches!(self.stage, Stage::Finished | Stage::Failed(_)) }
}

#[cfg(test)]
mod tests {
    //! A real TLS 1.3 peer (Python's ssl over OpenSSL) on the host, with a root made for
    //! this test: one session must carry several HTTP requests (keep-alive).
    use super::*;
    use std::{boxed::Box, format, io::{BufRead, BufReader, Read, Write}, net::TcpStream, process::{Command, Stdio}, string::String, time::{Duration, Instant, SystemTime, UNIX_EPOCH}, vec};
    const HOST: &str = "keepalive.sexos.test";
    const PEER: &str = r#"
import socket,ssl,sys
c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.minimum_version=ssl.TLSVersion.TLSv1_3;c.set_ecdh_curve('prime256v1');c.num_tickets=0
c.load_cert_chain(sys.argv[1],sys.argv[2]);c.set_alpn_protocols(['h2','http/1.1'])
s=socket.socket();s.bind(('127.0.0.1',0));s.listen(4);print(s.getsockname()[1],flush=True)
conns=0;reqs=0
while reqs<3:
 raw,_=s.accept();conns+=1
 t=c.wrap_socket(raw,server_side=True);f=t.makefile('rb');alpn=t.selected_alpn_protocol()
 while True:
  line=f.readline()
  if not line:break
  while f.readline() not in (b'\r\n',b''):pass
  reqs+=1;body=('reply %d to %s'%(reqs,line.split()[1].decode())).encode()
  t.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n'%len(body)+body)
  if reqs==3:break
 t.close()
print('connections=%d requests=%d alpn=%s'%(conns,reqs,alpn),flush=True)
"#;
    fn openssl(dir: &std::path::Path, args: &[&str]) { assert!(Command::new("openssl").args(args).current_dir(dir).stderr(Stdio::null()).status().unwrap().success(), "{:?}", args); }

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
        let (read, write) = (Box::leak(vec![0u8; 16640].into_boxed_slice()), Box::leak(vec![0u8; 4096].into_boxed_slice()));
        let mut session = Session::start(HOST, &mut [7u8; 32], seconds, read, write).unwrap();
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
        assert_eq!(summary.trim(), "connections=1 requests=3 alpn=http/1.1");
        let _ = peer.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
