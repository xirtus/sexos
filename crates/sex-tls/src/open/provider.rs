// SPDX-License-Identifier: GPL-3.0-only
//! The cryptography rustls runs on, from RustCrypto and fiat-crypto, all plain integer
//! code. TLS 1.3 and TLS 1.2 with AES-128-GCM, AES-256-GCM and ChaCha20-Poly1305;
//! key exchange on X25519, P-256 and P-384. No client certificates, so no signing.
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{cell::UnsafeCell, marker::PhantomData, sync::atomic::{AtomicBool, Ordering}};
use aes_gcm::aead::{consts::{U12, U16}, generic_array::GenericArray, AeadCore, AeadInPlace, KeyInit};
use rand_core::{RngCore, SeedableRng};
use rustls::crypto::{self, cipher::{self, AeadKey, InboundOpaqueMessage, InboundPlainMessage, Iv, KeyBlockShape, MessageDecrypter, MessageEncrypter,
    OutboundOpaqueMessage, OutboundPlainMessage, PrefixedPayload, Tls12AeadAlgorithm, Tls13AeadAlgorithm, UnsupportedOperationError},
    hash, tls12::PrfUsingHmac, tls13::HkdfUsingHmac, ActiveKeyExchange, CipherSuiteCommon, CryptoProvider, GetRandomFailed, KeyExchangeAlgorithm,
    KeyProvider, SecureRandom, SharedSecret, SupportedKxGroup, WebPkiSupportedAlgorithms};
use rustls::{pki_types::PrivateKeyDer, CipherSuite, ConnectionTrafficSecrets, ContentType, Error, NamedGroup, PeerMisbehaved, ProtocolVersion,
    SignatureScheme, SupportedCipherSuite, Tls12CipherSuite, Tls13CipherSuite};
use sha2::{Digest, Sha256, Sha384};

// ---- Randomness: one generator for the service, reseeded by every new session. ----

struct Rng { busy: AtomicBool, state: UnsafeCell<Option<rand_chacha::ChaCha20Rng>> }
// Every touch of `state` holds `busy`.
unsafe impl Sync for Rng {}
static RNG: Rng = Rng { busy: AtomicBool::new(false), state: UnsafeCell::new(None) };
impl Rng {
    fn with<T>(&self, f: impl FnOnce(&mut Option<rand_chacha::ChaCha20Rng>) -> T) -> T {
        while self.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { core::hint::spin_loop(); }
        let out = f(unsafe { &mut *self.state.get() });
        self.busy.store(false, Ordering::Release);
        out
    }
}
/// Mixes a fresh seed from the kernel's entropy into the generator: the result is no
/// weaker than the best seed it has seen.
pub fn reseed(seed: &[u8; 32]) {
    RNG.with(|state| {
        let mut mixed = Sha256::new();
        if let Some(rng) = state.as_mut() { let mut old = [0u8; 32]; rng.fill_bytes(&mut old); mixed.update(old); }
        mixed.update(seed);
        let mut key: [u8; 32] = mixed.finalize().into();
        *state = Some(rand_chacha::ChaCha20Rng::from_seed(key));
        crate::erase(&mut key);
    })
}
#[derive(Debug)]
struct Random;
impl SecureRandom for Random {
    fn fill(&self, out: &mut [u8]) -> Result<(), GetRandomFailed> {
        RNG.with(|state| match state.as_mut() { Some(rng) => { rng.fill_bytes(out); Ok(()) } None => Err(GetRandomFailed) })
    }
}
/// A generator of its own for one key, seeded from the shared one.
fn key_rng() -> Result<rand_chacha::ChaCha20Rng, Error> {
    let mut seed = [0u8; 32];
    Random.fill(&mut seed).map_err(|_| Error::FailedToGetRandomBytes)?;
    let rng = rand_chacha::ChaCha20Rng::from_seed(seed);
    crate::erase(&mut seed);
    Ok(rng)
}
#[derive(Debug)]
struct NoKeys;
impl KeyProvider for NoKeys {
    fn load_private_key(&self, _: PrivateKeyDer<'static>) -> Result<Arc<dyn rustls::sign::SigningKey>, Error> { Err(Error::General("no client keys".into())) }
}

// ---- Hashes and HMAC. ----

macro_rules! hash_impl { ($name:ident, $ctx:ident, $hmac:ident, $hkey:ident, $ty:ty, $alg:expr) => {
    struct $name;
    impl hash::Hash for $name {
        fn start(&self) -> Box<dyn hash::Context> { Box::new($ctx(<$ty>::new())) }
        fn hash(&self, data: &[u8]) -> hash::Output { hash::Output::new(&<$ty>::digest(data)) }
        fn output_len(&self) -> usize { <$ty as Digest>::output_size() }
        fn algorithm(&self) -> hash::HashAlgorithm { $alg }
    }
    struct $ctx($ty);
    impl hash::Context for $ctx {
        fn fork_finish(&self) -> hash::Output { hash::Output::new(&self.0.clone().finalize()) }
        fn fork(&self) -> Box<dyn hash::Context> { Box::new($ctx(self.0.clone())) }
        fn finish(self: Box<Self>) -> hash::Output { hash::Output::new(&self.0.finalize()) }
        fn update(&mut self, data: &[u8]) { self.0.update(data) }
    }
    struct $hmac;
    impl crypto::hmac::Hmac for $hmac {
        fn with_key(&self, key: &[u8]) -> Box<dyn crypto::hmac::Key> { Box::new($hkey(<hmac::Hmac<$ty> as hmac::Mac>::new_from_slice(key).expect("HMAC takes any key length"))) }
        fn hash_output_len(&self) -> usize { <$ty as Digest>::output_size() }
    }
    struct $hkey(hmac::Hmac<$ty>);
    impl crypto::hmac::Key for $hkey {
        fn sign_concat(&self, first: &[u8], middle: &[&[u8]], last: &[u8]) -> crypto::hmac::Tag {
            use hmac::Mac;
            let mut m = self.0.clone();
            m.update(first);
            for part in middle { m.update(part); }
            m.update(last);
            crypto::hmac::Tag::new(&m.finalize().into_bytes())
        }
        fn tag_len(&self) -> usize { <$ty as Digest>::output_size() }
    }
}}
hash_impl!(HashSha256, CtxSha256, HmacSha256, KeySha256, Sha256, hash::HashAlgorithm::SHA256);
hash_impl!(HashSha384, CtxSha384, HmacSha384, KeySha384, Sha384, hash::HashAlgorithm::SHA384);

// ---- Record protection. ----

const TAG: usize = 16;
/// One AEAD (AES-GCM or ChaCha20-Poly1305) for both protocol versions.
struct Aead<A>(PhantomData<A>);
trait Algorithm: AeadInPlace + AeadCore<NonceSize = U12, TagSize = U16> + KeyInit + Send + Sync + 'static {
    const KEY: usize;
    /// TLS 1.2 GCM sends 8 bytes of nonce with each record; ChaCha20 sends none.
    const EXPLICIT: usize;
    fn secrets(key: AeadKey, iv: Iv) -> ConnectionTrafficSecrets;
}
impl Algorithm for aes_gcm::Aes128Gcm { const KEY: usize = 16; const EXPLICIT: usize = 8; fn secrets(key: AeadKey, iv: Iv) -> ConnectionTrafficSecrets { ConnectionTrafficSecrets::Aes128Gcm { key, iv } } }
impl Algorithm for aes_gcm::Aes256Gcm { const KEY: usize = 32; const EXPLICIT: usize = 8; fn secrets(key: AeadKey, iv: Iv) -> ConnectionTrafficSecrets { ConnectionTrafficSecrets::Aes256Gcm { key, iv } } }
impl Algorithm for chacha20poly1305::ChaCha20Poly1305 { const KEY: usize = 32; const EXPLICIT: usize = 0; fn secrets(key: AeadKey, iv: Iv) -> ConnectionTrafficSecrets { ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv } } }
fn cipher<A: Algorithm>(key: &AeadKey) -> A { A::new_from_slice(key.as_ref()).expect("key length matches the suite") }

struct Tls13<A>(A, Iv);
impl<A: Algorithm> Tls13AeadAlgorithm for Aead<A> {
    fn encrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageEncrypter> { Box::new(Tls13(cipher::<A>(&key), iv)) }
    fn decrypter(&self, key: AeadKey, iv: Iv) -> Box<dyn MessageDecrypter> { Box::new(Tls13(cipher::<A>(&key), iv)) }
    fn key_len(&self) -> usize { A::KEY }
    fn extract_keys(&self, key: AeadKey, iv: Iv) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> { Ok(A::secrets(key, iv)) }
}
impl<A: Algorithm> MessageEncrypter for Tls13<A> {
    fn encrypt(&mut self, m: OutboundPlainMessage<'_>, seq: u64) -> Result<OutboundOpaqueMessage, Error> {
        let total = self.encrypted_payload_len(m.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total);
        payload.extend_from_chunks(&m.payload);
        payload.extend_from_slice(&m.typ.to_array());
        let nonce = cipher::Nonce::new(&self.1, seq).0;
        let tag = self.0.encrypt_in_place_detached(GenericArray::from_slice(&nonce), &cipher::make_tls13_aad(total), payload.as_mut()).map_err(|_| Error::EncryptError)?;
        payload.extend_from_slice(&tag);
        Ok(OutboundOpaqueMessage::new(ContentType::ApplicationData, ProtocolVersion::TLSv1_2, payload))
    }
    fn encrypted_payload_len(&self, len: usize) -> usize { len + 1 + TAG }
}
impl<A: Algorithm> MessageDecrypter for Tls13<A> {
    fn decrypt<'a>(&mut self, mut m: InboundOpaqueMessage<'a>, seq: u64) -> Result<InboundPlainMessage<'a>, Error> {
        let payload = &mut m.payload;
        if payload.len() < TAG { return Err(Error::DecryptError); }
        let aad = cipher::make_tls13_aad(payload.len());
        let nonce = cipher::Nonce::new(&self.1, seq).0;
        let at = payload.len() - TAG;
        let (body, tag) = payload.split_at_mut(at);
        self.0.decrypt_in_place_detached(GenericArray::from_slice(&nonce), &aad, body, GenericArray::from_slice(tag)).map_err(|_| Error::DecryptError)?;
        payload.truncate(at);
        m.into_tls13_unpadded_message()
    }
}

/// TLS 1.2: the fixed part of the nonce from the key block; GCM adds the record's
/// explicit part (we send the sequence number), ChaCha20 XORs in the sequence number.
struct Tls12<A>(A, [u8; 12]);
impl<A: Algorithm> Tls12AeadAlgorithm for Aead<A> {
    fn encrypter(&self, key: AeadKey, iv: &[u8], explicit: &[u8]) -> Box<dyn MessageEncrypter> {
        let mut nonce = [0u8; 12];
        if A::EXPLICIT == 0 { nonce.copy_from_slice(iv); } else { nonce[..4].copy_from_slice(iv); nonce[4..].copy_from_slice(explicit); }
        Box::new(Tls12(cipher::<A>(&key), nonce))
    }
    fn decrypter(&self, key: AeadKey, iv: &[u8]) -> Box<dyn MessageDecrypter> {
        let mut nonce = [0u8; 12];
        nonce[..iv.len()].copy_from_slice(iv);
        Box::new(Tls12(cipher::<A>(&key), nonce))
    }
    fn key_block_shape(&self) -> KeyBlockShape {
        KeyBlockShape { enc_key_len: A::KEY, fixed_iv_len: if A::EXPLICIT == 0 { 12 } else { 4 }, explicit_nonce_len: A::EXPLICIT }
    }
    fn extract_keys(&self, _: AeadKey, _: &[u8], _: &[u8]) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> { Err(UnsupportedOperationError) }
}
impl<A: Algorithm> MessageEncrypter for Tls12<A> {
    fn encrypt(&mut self, m: OutboundPlainMessage<'_>, seq: u64) -> Result<OutboundOpaqueMessage, Error> {
        let mut payload = PrefixedPayload::with_capacity(self.encrypted_payload_len(m.payload.len()));
        let nonce = cipher::Nonce::new(&Iv::new(self.1), seq).0;
        let aad = cipher::make_tls12_aad(seq, m.typ, m.version, m.payload.len());
        payload.extend_from_slice(&nonce[4..4 + A::EXPLICIT]);
        payload.extend_from_chunks(&m.payload);
        let tag = self.0.encrypt_in_place_detached(GenericArray::from_slice(&nonce), &aad, &mut payload.as_mut()[A::EXPLICIT..]).map_err(|_| Error::EncryptError)?;
        payload.extend_from_slice(&tag);
        Ok(OutboundOpaqueMessage::new(m.typ, m.version, payload))
    }
    fn encrypted_payload_len(&self, len: usize) -> usize { len + A::EXPLICIT + TAG }
}
impl<A: Algorithm> MessageDecrypter for Tls12<A> {
    fn decrypt<'a>(&mut self, mut m: InboundOpaqueMessage<'a>, seq: u64) -> Result<InboundPlainMessage<'a>, Error> {
        let len = m.payload.len();
        if len < A::EXPLICIT + TAG { return Err(Error::DecryptError); }
        let nonce = if A::EXPLICIT == 0 { cipher::Nonce::new(&Iv::new(self.1), seq).0 } else {
            let mut n = self.1; n[4..].copy_from_slice(&m.payload[..8]); n
        };
        let aad = cipher::make_tls12_aad(seq, m.typ, m.version, len - A::EXPLICIT - TAG);
        let (body, tag) = m.payload[A::EXPLICIT..].split_at_mut(len - A::EXPLICIT - TAG);
        self.0.decrypt_in_place_detached(GenericArray::from_slice(&nonce), &aad, body, GenericArray::from_slice(tag)).map_err(|_| Error::DecryptError)?;
        Ok(m.into_plain_message_range(A::EXPLICIT..len - TAG))
    }
}

// ---- Key exchange. ----

#[derive(Debug)]
struct X25519;
struct X25519Share { secret: [u8; 32], public: [u8; 32] }
impl Drop for X25519Share { fn drop(&mut self) { crate::erase(&mut self.secret); } }
impl SupportedKxGroup for X25519 {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let mut secret = [0u8; 32];
        Random.fill(&mut secret).map_err(|_| Error::FailedToGetRandomBytes)?;
        let public = super::x25519::x25519(&secret, &super::x25519::BASE);
        Ok(Box::new(X25519Share { secret, public }))
    }
    fn name(&self) -> NamedGroup { NamedGroup::X25519 }
}
impl ActiveKeyExchange for X25519Share {
    fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
        let peer: [u8; 32] = peer.try_into().map_err(|_| Error::from(PeerMisbehaved::InvalidKeyShare))?;
        let shared = super::x25519::x25519(&self.secret, &peer);
        // An all-zero result means the peer sent a point of small order (RFC 7748 6.1).
        if shared.iter().fold(0u8, |a, b| a | b) == 0 { return Err(PeerMisbehaved::InvalidKeyShare.into()); }
        Ok(SharedSecret::from(&shared[..]))
    }
    fn pub_key(&self) -> &[u8] { &self.public }
    fn group(&self) -> NamedGroup { NamedGroup::X25519 }
}
macro_rules! nist { ($group:ident, $share:ident, $curve:ident, $name:expr) => {
    #[derive(Debug)]
    struct $group;
    struct $share { secret: $curve::ecdh::EphemeralSecret, public: Vec<u8> }
    impl SupportedKxGroup for $group {
        fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
            let secret = $curve::ecdh::EphemeralSecret::random(&mut key_rng()?);
            let public = $curve::EncodedPoint::from(secret.public_key()).as_bytes().to_vec();
            Ok(Box::new($share { secret, public }))
        }
        fn name(&self) -> NamedGroup { $name }
    }
    impl ActiveKeyExchange for $share {
        fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
            let peer = $curve::PublicKey::from_sec1_bytes(peer).map_err(|_| Error::from(PeerMisbehaved::InvalidKeyShare))?;
            Ok(SharedSecret::from(&self.secret.diffie_hellman(&peer).raw_secret_bytes()[..]))
        }
        fn pub_key(&self) -> &[u8] { &self.public }
        fn group(&self) -> NamedGroup { $name }
    }
}}
nist!(P256, P256Share, p256, NamedGroup::secp256r1);
nist!(P384, P384Share, p384, NamedGroup::secp384r1);

// ---- Suites. ----

static AES128: Aead<aes_gcm::Aes128Gcm> = Aead(PhantomData);
static AES256: Aead<aes_gcm::Aes256Gcm> = Aead(PhantomData);
static CHACHA: Aead<chacha20poly1305::ChaCha20Poly1305> = Aead(PhantomData);
// Safe to share: these hold no state, only the type of the cipher.
unsafe impl<A> Sync for Aead<A> {}
unsafe impl<A> Send for Aead<A> {}

const fn common(suite: CipherSuite, hash: &'static dyn hash::Hash) -> CipherSuiteCommon {
    // RFC 9147 limits: about 2^24.5 records for AES-GCM; ChaCha20 has none worth naming.
    CipherSuiteCommon { suite, hash_provider: hash, confidentiality_limit: 1 << 23 }
}
static TLS13_AES_128_GCM_SHA256: Tls13CipherSuite = Tls13CipherSuite { common: common(CipherSuite::TLS13_AES_128_GCM_SHA256, &HashSha256), hkdf_provider: &HkdfUsingHmac(&HmacSha256), aead_alg: &AES128, quic: None };
static TLS13_AES_256_GCM_SHA384: Tls13CipherSuite = Tls13CipherSuite { common: common(CipherSuite::TLS13_AES_256_GCM_SHA384, &HashSha384), hkdf_provider: &HkdfUsingHmac(&HmacSha384), aead_alg: &AES256, quic: None };
static TLS13_CHACHA20_POLY1305_SHA256: Tls13CipherSuite = Tls13CipherSuite { common: CipherSuiteCommon { confidentiality_limit: u64::MAX, ..common(CipherSuite::TLS13_CHACHA20_POLY1305_SHA256, &HashSha256) }, hkdf_provider: &HkdfUsingHmac(&HmacSha256), aead_alg: &CHACHA, quic: None };

static ECDSA: &[SignatureScheme] = &[SignatureScheme::ECDSA_NISTP256_SHA256, SignatureScheme::ECDSA_NISTP384_SHA384];
static RSA: &[SignatureScheme] = &[SignatureScheme::RSA_PSS_SHA256, SignatureScheme::RSA_PSS_SHA384, SignatureScheme::RSA_PSS_SHA512,
    SignatureScheme::RSA_PKCS1_SHA256, SignatureScheme::RSA_PKCS1_SHA384, SignatureScheme::RSA_PKCS1_SHA512];
macro_rules! tls12 { ($name:ident, $suite:ident, $hash:expr, $hmac:expr, $sign:expr, $aead:expr, $limit:expr) => {
    static $name: Tls12CipherSuite = Tls12CipherSuite { common: CipherSuiteCommon { confidentiality_limit: $limit, ..common(CipherSuite::$suite, $hash) },
        prf_provider: &PrfUsingHmac($hmac), kx: KeyExchangeAlgorithm::ECDHE, sign: $sign, aead_alg: $aead };
}}
tls12!(ECDHE_ECDSA_AES128, TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256, &HashSha256, &HmacSha256, ECDSA, &AES128, 1 << 23);
tls12!(ECDHE_ECDSA_AES256, TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384, &HashSha384, &HmacSha384, ECDSA, &AES256, 1 << 23);
tls12!(ECDHE_ECDSA_CHACHA, TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256, &HashSha256, &HmacSha256, ECDSA, &CHACHA, u64::MAX);
tls12!(ECDHE_RSA_AES128, TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256, &HashSha256, &HmacSha256, RSA, &AES128, 1 << 23);
tls12!(ECDHE_RSA_AES256, TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384, &HashSha384, &HmacSha384, RSA, &AES256, 1 << 23);
tls12!(ECDHE_RSA_CHACHA, TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256, &HashSha256, &HmacSha256, RSA, &CHACHA, u64::MAX);

/// Preference order: TLS 1.3 first, AES-128-GCM first (cheapest), ChaCha20 for
/// servers that prefer it.
pub fn suites() -> Vec<SupportedCipherSuite> {
    alloc::vec![
        SupportedCipherSuite::Tls13(&TLS13_AES_128_GCM_SHA256), SupportedCipherSuite::Tls13(&TLS13_CHACHA20_POLY1305_SHA256), SupportedCipherSuite::Tls13(&TLS13_AES_256_GCM_SHA384),
        SupportedCipherSuite::Tls12(&ECDHE_ECDSA_AES128), SupportedCipherSuite::Tls12(&ECDHE_RSA_AES128), SupportedCipherSuite::Tls12(&ECDHE_ECDSA_CHACHA),
        SupportedCipherSuite::Tls12(&ECDHE_RSA_CHACHA), SupportedCipherSuite::Tls12(&ECDHE_ECDSA_AES256), SupportedCipherSuite::Tls12(&ECDHE_RSA_AES256),
    ]
}
static GROUPS: [&dyn SupportedKxGroup; 3] = [&X25519, &P256, &P384];

pub fn provider(algorithms: WebPkiSupportedAlgorithms) -> CryptoProvider {
    CryptoProvider { cipher_suites: suites(), kx_groups: GROUPS.to_vec(), signature_verification_algorithms: algorithms, secure_random: &Random, key_provider: &NoKeys }
}
