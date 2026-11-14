// SPDX-License-Identifier: GPL-3.0-only
//! Keys, sealing (XChaCha20-Poly1305), blob ids (HMAC-SHA256) and password key files (Argon2id).

use crate::codec::{Dec, Enc};
use crate::{Error, Id};
use alloc::vec::Vec;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
/// Bytes a sealed message adds to its plaintext.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// The repository's master key: one key for sealing, one for chunk ids.
pub(crate) struct MasterKey {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        // Best effort; volatile so the stores are kept.
        for b in self.enc.iter_mut().chain(self.mac.iter_mut()) {
            unsafe { core::ptr::write_volatile(b, 0) };
        }
    }
}

impl MasterKey {
    pub fn generate(rng: &mut dyn Rng) -> Self {
        let mut k = MasterKey { enc: [0; 32], mac: [0; 32] };
        rng.fill_bytes(&mut k.enc);
        rng.fill_bytes(&mut k.mac);
        k
    }

    /// `nonce || ciphertext || tag`.
    pub fn seal(&self, rng: &mut dyn Rng, aad: &[u8], plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        rng.fill_bytes(&mut nonce);
        seal_with(&self.enc, &nonce, aad, plain)
    }

    pub fn open(&self, aad: &[u8], sealed: &[u8], what: &str) -> Result<Vec<u8>, Error> {
        open_with(&self.enc, aad, sealed)
            .ok_or_else(|| Error::Corrupt(alloc::format!("{what}: authentication failed")))
    }

    /// Keyed blob id: HMAC-SHA256(mac key, plaintext).
    pub fn blob_id(&self, data: &[u8]) -> Id {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.mac).expect("hmac key");
        m.update(data);
        Id(m.finalize().into_bytes().into())
    }
}

/// The caller's source of randomness (keys, nonces, salts, lock names): any
/// `rand_core` 0.6 `RngCore + CryptoRng`. Object safe, so operations take `&mut dyn Rng`.
pub trait Rng: RngCore + CryptoRng {}
impl<T: RngCore + CryptoRng> Rng for T {}

fn seal_with(key: &[u8; 32], nonce: &[u8; NONCE_LEN], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let c = XChaCha20Poly1305::new(key.into());
    let ct = c
        .encrypt(XNonce::from_slice(nonce), Payload { msg: plain, aad })
        .expect("xchacha20poly1305 encrypt");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

fn open_with(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < OVERHEAD {
        return None;
    }
    let c = XChaCha20Poly1305::new(key.into());
    c.decrypt(XNonce::from_slice(&sealed[..NONCE_LEN]), Payload { msg: &sealed[NONCE_LEN..], aad })
        .ok()
}

/// Plain SHA-256, used to name packs, index and snapshot files after their stored bytes.
pub fn sha256(data: &[u8]) -> Id {
    Id(Sha256::digest(data).into())
}

/// Argon2id cost parameters stored in each key file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_cost_kib: u32,
    /// Passes.
    pub t_cost: u32,
    /// Lanes.
    pub p_cost: u32,
}

impl Default for KdfParams {
    /// 64 MiB, 3 passes, 1 lane (RFC 9106's second recommended option).
    fn default() -> Self {
        KdfParams { m_cost_kib: 64 * 1024, t_cost: 3, p_cost: 1 }
    }
}

const KEY_MAGIC: &[u8; 4] = b"ACKY";
const KEY_VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;
const SALT_LEN: usize = 32;

fn derive(password: &[u8], salt: &[u8], p: KdfParams) -> Result<[u8; 32], Error> {
    let params = argon2::Params::new(p.m_cost_kib, p.t_cost, p.p_cost, Some(32))
        .map_err(|e| Error::Invalid(alloc::format!("argon2 parameters: {e}")))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = [0u8; 32];
    a.hash_password_into(password, salt, &mut out)
        .map_err(|e| Error::Invalid(alloc::format!("argon2: {e}")))?;
    Ok(out)
}

/// Key file: plaintext header (authenticated as AAD) followed by the sealed master key.
pub(crate) fn wrap_key(
    key: &MasterKey,
    password: &[u8],
    params: KdfParams,
    rng: &mut dyn Rng,
) -> Result<Vec<u8>, Error> {
    let mut salt = [0u8; SALT_LEN];
    rng.fill_bytes(&mut salt);
    let mut e = Enc::new(KEY_MAGIC, KEY_VERSION);
    e.u8(KDF_ARGON2ID);
    e.u32(params.m_cost_kib);
    e.u32(params.t_cost);
    e.u32(params.p_cost);
    e.buf.extend_from_slice(&salt);
    let header = e.buf.clone();
    let mut kek = derive(password, &salt, params)?;
    let mut plain = [0u8; 64];
    plain[..32].copy_from_slice(&key.enc);
    plain[32..].copy_from_slice(&key.mac);
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut nonce);
    let sealed = seal_with(&kek, &nonce, &header, &plain);
    plain.fill(0);
    kek.fill(0);
    let mut out = header;
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub(crate) fn unwrap_key(file: &[u8], password: &[u8]) -> Result<MasterKey, Error> {
    let mut d = Dec::new(file, KEY_MAGIC, KEY_VERSION, "key file")?;
    if d.u8()? != KDF_ARGON2ID {
        return Err(Error::Unsupported("key file: unknown KDF".into()));
    }
    let params = KdfParams { m_cost_kib: d.u32()?, t_cost: d.u32()?, p_cost: d.u32()? };
    // Refuse absurd costs from a hostile store before allocating (4 GiB cap).
    if params.m_cost_kib > 4 * 1024 * 1024 || params.t_cost > 64 || params.p_cost > 64 {
        return Err(Error::Corrupt("key file: unreasonable KDF parameters".into()));
    }
    let salt = d.take(SALT_LEN)?;
    let header_len = 4 + 2 + 1 + 12 + SALT_LEN;
    let mut kek = derive(password, salt, params)?;
    let plain = open_with(&kek, &file[..header_len], &file[header_len..]);
    kek.fill(0);
    let mut plain = plain.ok_or(Error::WrongPassword)?;
    if plain.len() != 64 {
        return Err(Error::Corrupt("key file: wrong key length".into()));
    }
    let mut k = MasterKey { enc: [0; 32], mac: [0; 32] };
    k.enc.copy_from_slice(&plain[..32]);
    k.mac.copy_from_slice(&plain[32..]);
    plain.fill(0);
    Ok(k)
}
