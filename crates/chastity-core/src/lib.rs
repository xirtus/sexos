// SPDX-License-Identifier: GPL-3.0-only
//! Chastity's encrypted file and in-memory authorization engine. No ambient I/O.
#![no_std]
extern crate alloc;
use alloc::{string::String, vec::Vec};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{AeadInPlace, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use zeroize::{Zeroize, Zeroizing};
pub mod kdbx;

pub const MAX_FILE: usize = 8 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_FIELD: usize = 4096;
const HEADER: usize = 8 + 12 + 16 + 24;
const MAGIC: &[u8; 8] = b"CHAST01\0";
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Format,
    Limit,
    Authentication,
    Entropy,
    Password,
    Locked,
    Denied,
    Stale,
    Unsupported,
}
/// The caller must supply a cryptographically secure, fallible entropy source.
pub trait Random {
    fn fill(&mut self, out: &mut [u8]) -> Result<(), Error>;
}

pub struct Entry {
    pub title: String,
    pub origin: String,
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub notes: Zeroizing<String>,
}
impl Entry {
    pub fn new(title: &str, origin: &str, username: &str, password: &str) -> Result<Self, Error> {
        if [title, origin, username, password]
            .iter()
            .any(|s| s.len() > MAX_FIELD || s.contains('\0'))
        {
            return Err(Error::Limit);
        }
        Ok(Self {
            title: title.into(),
            origin: origin.into(),
            username: Zeroizing::new(username.into()),
            password: Zeroizing::new(password.into()),
            notes: Zeroizing::new(String::new()),
        })
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.title.zeroize();
        self.origin.zeroize();
    }
}
#[derive(Default)]
pub struct Vault {
    entries: Vec<Entry>,
}
impl Vault {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn add(&mut self, entry: Entry) -> Result<(), Error> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::Limit);
        }
        self.entries.push(entry);
        Ok(())
    }
    pub fn remove(&mut self, index: usize) -> Result<(), Error> {
        if index >= self.entries.len() {
            return Err(Error::Format);
        }
        self.entries.remove(index);
        Ok(())
    }
    pub fn seal(&self, password: &[u8], rng: &mut impl Random) -> Result<Vec<u8>, Error> {
        self.seal_cost(password, rng, 65536, 3)
    }
    fn seal_cost(
        &self,
        password: &[u8],
        rng: &mut impl Random,
        memory: u32,
        iterations: u32,
    ) -> Result<Vec<u8>, Error> {
        if password.is_empty() || password.len() > MAX_FIELD {
            return Err(Error::Password);
        }
        let mut header = Vec::with_capacity(HEADER);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&memory.to_le_bytes());
        header.extend_from_slice(&iterations.to_le_bytes());
        header.extend_from_slice(&1u32.to_le_bytes());
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        rng.fill(&mut salt)?;
        rng.fill(&mut nonce)?;
        header.extend_from_slice(&salt);
        header.extend_from_slice(&nonce);
        let key = derive(password, &salt, memory, iterations, 1)?;
        let mut plain = self.encode()?;
        // Detached in-place AEAD never copies plaintext into an allocating helper.
        let tag = XChaCha20Poly1305::new_from_slice(&*key)
            .map_err(|_| Error::Format)?
            .encrypt_in_place_detached(XNonce::from_slice(&nonce), &header, &mut plain)
            .map_err(|_| Error::Authentication)?;
        header.extend_from_slice(&plain);
        header.extend_from_slice(&tag);
        Ok(header)
    }
    pub fn unlock(file: &[u8], password: &[u8]) -> Result<Self, Error> {
        if file.len() < HEADER + 16 || file.len() > MAX_FILE || &file[..8] != MAGIC {
            return Err(Error::Format);
        }
        if password.is_empty() || password.len() > MAX_FIELD {
            return Err(Error::Password);
        }
        let m = u32::from_le_bytes(file[8..12].try_into().unwrap());
        let t = u32::from_le_bytes(file[12..16].try_into().unwrap());
        let p = u32::from_le_bytes(file[16..20].try_into().unwrap());
        let key = derive(password, &file[20..36], m, t, p)?;
        let end = file.len() - 16;
        let mut plain = Zeroizing::new(file[HEADER..end].to_vec());
        XChaCha20Poly1305::new_from_slice(&*key)
            .map_err(|_| Error::Format)?
            .decrypt_in_place_detached(
                XNonce::from_slice(&file[36..60]),
                &file[..HEADER],
                &mut plain,
                chacha20poly1305::Tag::from_slice(&file[end..]),
            )
            .map_err(|_| Error::Authentication)?;
        Self::decode(&plain)
    }
    fn encode(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let mut size = 4usize;
        for e in &self.entries {
            for s in [&e.title, &e.origin, &*e.username, &*e.password, &*e.notes] {
                if s.len() > MAX_FIELD || s.contains('\0') {
                    return Err(Error::Limit);
                }
                size = size.checked_add(4 + s.len()).ok_or(Error::Limit)?;
                if size > MAX_FILE - HEADER - 16 {
                    return Err(Error::Limit);
                }
            }
        }
        // Allocate once before copying any plaintext. Reallocation would leave
        // unwiped prefixes in freed heap even though the final Vec is Zeroizing.
        let mut out = Zeroizing::new(Vec::with_capacity(size));
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            for s in [&e.title, &e.origin, &*e.username, &*e.password, &*e.notes] {
                if s.len() > MAX_FIELD || s.contains('\0') {
                    return Err(Error::Limit);
                }
                out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
                if out.len() > MAX_FILE - HEADER - 16 {
                    return Err(Error::Limit);
                }
            }
        }
        Ok(out)
    }
    fn decode(mut data: &[u8]) -> Result<Self, Error> {
        let n = read_u32(&mut data)? as usize;
        if n > MAX_ENTRIES {
            return Err(Error::Limit);
        }
        let mut vault = Self::default();
        for _ in 0..n {
            let mut fields = Zeroizing::new(Vec::<String>::with_capacity(5));
            for _ in 0..5 {
                let len = read_u32(&mut data)? as usize;
                if len > MAX_FIELD {
                    return Err(Error::Limit);
                }
                let s = core::str::from_utf8(take(&mut data, len)?).map_err(|_| Error::Format)?;
                fields.push(s.into());
            }
            let mut entry = Entry::new(&fields[0], &fields[1], &fields[2], &fields[3])?;
            entry.notes = Zeroizing::new(fields[4].clone());
            vault.add(entry)?;
        }
        if !data.is_empty() {
            return Err(Error::Format);
        }
        Ok(vault)
    }
}
pub(crate) fn derive(
    password: &[u8],
    salt: &[u8],
    m: u32,
    t: u32,
    p: u32,
) -> Result<Zeroizing<[u8; 32]>, Error> {
    if !(8..=262144).contains(&m)
        || !(1..=64).contains(&t)
        || m as u64 * t as u64 > 1048576
        || !(1..=4).contains(&p)
        || salt.len() < 8
        || salt.len() > 64
    {
        return Err(Error::Limit);
    }
    let params = Params::new(m, t, p, Some(32)).map_err(|_| Error::Format)?;
    transform(Algorithm::Argon2id, params, password, salt)
}
/// Argon2's allocating convenience API does not wipe its Vec<Block>. Always
/// supply an explicitly zeroizing workspace, including on a hashing error.
pub(crate) fn transform(
    algorithm: Algorithm,
    params: Params,
    password: &[u8],
    salt: &[u8],
) -> Result<Zeroizing<[u8; 32]>, Error> {
    let mut workspace = Zeroizing::new(alloc::vec![argon2::Block::new();params.block_count()]);
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(algorithm, Version::V0x13, params)
        .hash_password_into_with_memory(password, salt, &mut *key, &mut workspace[..])
        .map_err(|_| Error::Authentication)?;
    Ok(key)
}

pub(crate) fn take<'a>(data: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    if n > data.len() {
        return Err(Error::Format);
    }
    let (a, b) = data.split_at(n);
    *data = b;
    Ok(a)
}
pub(crate) fn read_u32(data: &mut &[u8]) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(take(data, 4)?.try_into().unwrap()))
}

/// Unbiased rejection sampling, never a time-based fallback. The default
/// ASCII alphabet excludes ambiguous characters and includes punctuation.
pub fn generate(length: usize, rng: &mut impl Random) -> Result<Zeroizing<String>, Error> {
    const ALPHABET: &[u8] =
        b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!#$%&*+-=?@_";
    if !(16..=128).contains(&length) {
        return Err(Error::Limit);
    }
    let cutoff = 256 - (256 % ALPHABET.len());
    let mut out = Zeroizing::new(String::with_capacity(length));
    let mut attempts = 0;
    while out.len() < length {
        let mut byte = [0u8];
        rng.fill(&mut byte)?;
        attempts += 1;
        if attempts > length * 1024 {
            return Err(Error::Entropy);
        }
        if (byte[0] as usize) < cutoff {
            out.push(ALPHABET[byte[0] as usize % ALPHABET.len()] as char);
        }
        byte.zeroize();
    }
    Ok(out)
}

/// App identity must come from the authenticated IPC envelope, never its payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub domain: u32,
    pub task: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);
impl Ticket {
    pub fn word(self) -> u64 {
        self.0
    }
    pub fn from_word(word: u64) -> Self {
        Self(word)
    }
}
struct Grant {
    ticket: Ticket,
    caller: Caller,
    index: usize,
    expires: u64,
}
/// An integration API: a trusted presenter obtains consent, then issues a one-use
/// ticket tied to exact run identity and entry. Locking revokes every ticket.
/// The app's IPC adapter must authenticate caller identity before calling redeem.
pub struct Session {
    vault: Option<Vault>,
    grants: Vec<Grant>,
    next: u64,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            vault: None,
            grants: Vec::new(),
            next: 1,
        }
    }
}
impl Session {
    pub fn unlock(&mut self, vault: Vault) {
        self.lock();
        self.vault = Some(vault);
    }
    pub fn vault(&self) -> Result<&Vault, Error> {
        self.vault.as_ref().ok_or(Error::Locked)
    }
    pub fn vault_mut(&mut self) -> Result<&mut Vault, Error> {
        self.grants.clear();
        self.vault.as_mut().ok_or(Error::Locked)
    }
    pub fn lock(&mut self) {
        self.vault = None;
        self.grants.clear();
    }
    /// Only call after the user approves this exact caller, origin, and entry.
    pub fn approve(
        &mut self,
        caller: Caller,
        index: usize,
        origin: &str,
        now: u64,
    ) -> Result<Ticket, Error> {
        self.grants.retain(|g| g.expires > now);
        let entry = self.vault()?.entries.get(index).ok_or(Error::Denied)?;
        if caller.domain == 0
            || caller.task == 0
            || entry.origin != origin
            || origin.is_empty()
            || self.grants.len() >= 64
        {
            return Err(Error::Denied);
        }
        let ticket = Ticket(self.next);
        self.next = self.next.checked_add(1).ok_or(Error::Limit)?;
        self.grants.push(Grant {
            ticket,
            caller,
            index,
            expires: now.saturating_add(30000),
        });
        Ok(ticket)
    }
    pub fn redeem<R>(
        &mut self,
        caller: Caller,
        ticket: Ticket,
        now: u64,
        use_secret: impl FnOnce(&str, &str) -> R,
    ) -> Result<R, Error> {
        let pos = self
            .grants
            .iter()
            .position(|g| g.ticket == ticket && g.caller == caller)
            .ok_or(Error::Denied)?;
        let grant = self.grants.remove(pos);
        if now >= grant.expires {
            return Err(Error::Stale);
        }
        let e = self
            .vault()?
            .entries
            .get(grant.index)
            .ok_or(Error::Denied)?;
        Ok(use_secret(&e.username, &e.password))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct R(u8);
    impl Random for R {
        fn fill(&mut self, out: &mut [u8]) -> Result<(), Error> {
            for b in out {
                *b = self.0;
                self.0 = self.0.wrapping_add(1);
            }
            Ok(())
        }
    }
    fn vault() -> Vault {
        let mut v = Vault::default();
        v.add(Entry::new("Example", "https://example.org", "alice", "correct horse").unwrap())
            .unwrap();
        v
    }
    #[test]
    fn plaintext_serialization_and_generator_have_final_capacity_before_copying() {
        let mut v = Vault::default();
        for _ in 0..31 {
            v.add(Entry::new("title", "origin", &"u".repeat(113), &"p".repeat(311)).unwrap())
                .unwrap();
        }
        let encoded = v.encode().unwrap();
        assert_eq!(encoded.capacity(), encoded.len());
        let password = generate(24, &mut R(1)).unwrap();
        assert_eq!(password.capacity(), 24);
    }
    #[test]
    fn expired_autofill_grants_do_not_permanently_exhaust_capacity() {
        let mut s = Session::default();
        s.unlock(vault());
        let c = Caller {
            domain: 55,
            task: 7,
        };
        for _ in 0..64 {
            s.approve(c, 0, "https://example.org", 0).unwrap();
        }
        assert!(s.approve(c, 0, "https://example.org", 1).is_err());
        assert!(s.approve(c, 0, "https://example.org", 30000).is_ok());
    }
    #[test]
    fn encrypted_roundtrip_and_header_authentication() {
        let file = vault()
            .seal_cost(b"test password", &mut R(1), 8, 1)
            .unwrap();
        assert!(!file.windows(13).any(|s| s == b"correct horse"));
        let read = Vault::unlock(&file, b"test password").unwrap();
        assert_eq!(&*read.entries()[0].password, "correct horse");
        assert!(matches!(
            Vault::unlock(&file, b"wrong"),
            Err(Error::Authentication)
        ));
        for i in [20, 36, 60, file.len() - 1] {
            let mut changed = file.clone();
            changed[i] ^= 1;
            assert!(Vault::unlock(&changed, b"test password").is_err());
        }
        for n in 0..file.len() {
            assert!(Vault::unlock(&file[..n], b"test password").is_err());
        }
    }
    #[test]
    fn kdf_costs_are_bounded_before_allocation() {
        let mut file = vault().seal_cost(b"password", &mut R(1), 8, 1).unwrap();
        file[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            Vault::unlock(&file, b"password"),
            Err(Error::Limit)
        ));
    }
    #[test]
    fn fresh_entropy_changes_ciphertext() {
        let a = vault().seal_cost(b"password", &mut R(1), 8, 1).unwrap();
        let b = vault().seal_cost(b"password", &mut R(2), 8, 1).unwrap();
        assert_ne!(a, b);
    }
    #[test]
    fn generation_refuses_weak_length_and_entropy_failure() {
        let a = generate(24, &mut R(1)).unwrap();
        assert_eq!(a.len(), 24);
        assert!(generate(8, &mut R(1)).is_err());
        struct Failed;
        impl Random for Failed {
            fn fill(&mut self, _: &mut [u8]) -> Result<(), Error> {
                Err(Error::Entropy)
            }
        }
        assert!(generate(24, &mut Failed).is_err());
        assert!(vault().seal(b"password", &mut Failed).is_err());
    }
    #[test]
    fn autofill_requires_exact_run_and_consumes_ticket() {
        let mut s = Session::default();
        s.unlock(vault());
        let c = Caller {
            domain: 55,
            task: 7,
        };
        let ticket = s.approve(c, 0, "https://example.org", 100).unwrap();
        assert!(s
            .redeem(
                Caller {
                    domain: 55,
                    task: 8
                },
                ticket,
                101,
                |_, _| ()
            )
            .is_err());
        assert_eq!(
            s.redeem(c, ticket, 101, |u, p| (u == "alice", p == "correct horse")),
            Ok((true, true))
        );
        assert!(s.redeem(c, ticket, 102, |_, _| ()).is_err());
    }
    #[test]
    fn mutation_lock_expiry_and_origin_revoke_autofill() {
        let mut s = Session::default();
        s.unlock(vault());
        let c = Caller {
            domain: 55,
            task: 7,
        };
        assert!(s.approve(c, 0, "https://evil.example", 0).is_err());
        let t = s.approve(c, 0, "https://example.org", 0).unwrap();
        assert_eq!(s.redeem(c, t, 30000, |_, _| ()), Err(Error::Stale));
        let t = s.approve(c, 0, "https://example.org", 0).unwrap();
        s.vault_mut().unwrap();
        assert!(s.redeem(c, t, 0, |_, _| ()).is_err());
        let t = s.approve(c, 0, "https://example.org", 0).unwrap();
        s.lock();
        assert!(s.redeem(c, t, 0, |_, _| ()).is_err());
        assert!(s.vault().is_err());
    }
    #[test]
    fn imports_argon2id_chacha20_fixture_with_escaped_username() {
        let file = include_bytes!("../tests/fixtures/argon2id-chacha20.kdbx");
        let vault = kdbx::import(file, b"fixture-password-only").unwrap();
        assert_eq!(&*vault.entries()[0].username, "alice&bob");
        assert_eq!(&*vault.entries()[0].password, "fixture<&>secret");
    }
    #[test]
    fn imports_independent_kdbx_fixture_and_rejects_tampering() {
        let file = include_bytes!("../tests/fixtures/keepassxc-compatible.kdbx");
        let imported = kdbx::import(file, b"fixture-password-only").unwrap();
        assert_eq!(imported.entries().len(), 1);
        let e = &imported.entries()[0];
        assert_eq!(e.title, "Fixture account");
        assert_eq!(&*e.username, "alice");
        assert_eq!(&*e.password, "fixture-secret");
        assert_eq!(&*e.notes, "original test note");
        assert!(kdbx::import(file, b"wrong").is_err());
        let mut changed = file.to_vec();
        let n = changed.len();
        changed[n - 40] ^= 1;
        assert!(kdbx::import(&changed, b"fixture-password-only").is_err());
        for n in [0, 8, 12, 60, file.len() - 1] {
            assert!(kdbx::import(&file[..n], b"fixture-password-only").is_err());
        }
    }
}
