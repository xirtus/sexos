// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The lock password, kept by Files and never handed out.
//!
//! A caller sends the password in 8-byte chunks (`PUT`, key = chunk index), then
//! asks Files to `CHECK` it against the stored hash, or (Settings only) to `SET` it
//! as the new password. Files keeps an Argon2id hash with a random salt in
//! `/system/password`; nobody can read it back. The staged bytes are wiped after
//! each CHECK or SET. The Shell checks passwords for the lock screen.
//!
//! Request: `type_id` [`OP`], arg0 = verb | chunk << 8, arg1 = 8 bytes (PUT) or the
//! length (CHECK, SET). Reply: tag 0xC7 << 56 | status.

pub const OP: u64 = 0x2F3;

pub const PUT: u8 = 1;
pub const CHECK: u8 = 2;
pub const SET: u8 = 3;
/// Is a password set? OK if so, `NO_PASSWORD` if not.
pub const STATE: u8 = 4;

pub const OK: u8 = 0;
pub const WRONG: u8 = 1;
pub const NO_PASSWORD: u8 = 2;
pub const DENIED: u8 = 3;
pub const INVALID: u8 = 4;
pub const NOT_SAVED: u8 = 5;

/// Longest password: eight chunks.
pub const MAX: usize = 64;
const TAG: u64 = 0xC7;

pub const fn reply(status: u8) -> u64 { TAG << 56 | status as u64 }
pub const fn decode_reply(word: u64) -> Option<u8> {
    if word >> 56 != TAG || word >> 8 & 0xFFFF_FFFF_FFFF != 0 { return None; }
    Some(word as u8)
}

/// The words that send `password` and then `verb` (CHECK or SET): (arg0, arg1) pairs.
pub fn requests(password: &[u8], verb: u8) -> impl Iterator<Item = (u64, u64)> + '_ {
    let n = password.len().min(MAX);
    password[..n].chunks(8).enumerate().map(|(i, c)| {
        let mut b = [0u8; 8];
        b[..c.len()].copy_from_slice(c);
        (crate::wire::request(PUT, i as u16), u64::from_le_bytes(b))
    }).chain(core::iter::once((crate::wire::request(verb, 0), n as u64)))
}

/// One caller's password on its way in.
pub struct Staging { bytes: [u8; MAX] }
impl Default for Staging { fn default() -> Self { Self::new() } }
impl Staging {
    pub const fn new() -> Self { Staging { bytes: [0; MAX] } }
    fn wipe(&mut self) { for b in self.bytes.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) }; } }
}

/// The stored form: `argon2id$m=4096,t=3,p=1$<salt hex>$<hash hex>`.
pub const STORED_MAX: usize = 128;
#[cfg(feature = "password")]
const M_KIB: u32 = 4096;
#[cfg(feature = "password")]
const T: u32 = 3;

/// What `serve` needs from Files: the stored hash, a place to write a new one, and
/// randomness for salts.
pub trait Store {
    /// The stored text, or None when no password is set.
    fn load(&mut self, out: &mut [u8; STORED_MAX]) -> Option<usize>;
    fn save(&mut self, text: &[u8]) -> bool;
    fn salt(&mut self) -> [u8; 16];
}

#[cfg(feature = "password")]
fn hash(password: &[u8], salt: &[u8; 16]) -> Option<[u8; 32]> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(M_KIB, T, 1, Some(32)).ok()?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(password, salt, &mut out).ok()?;
    Some(out)
}
#[cfg(not(feature = "password"))]
fn hash(_: &[u8], _: &[u8; 16]) -> Option<[u8; 32]> { None }

fn hex(bytes: &[u8], out: &mut [u8]) { for (i, b) in bytes.iter().enumerate() { out[2 * i] = b"0123456789abcdef"[(b >> 4) as usize]; out[2 * i + 1] = b"0123456789abcdef"[(b & 15) as usize]; } }
fn unhex<const N: usize>(text: &[u8]) -> Option<[u8; N]> {
    if text.len() != 2 * N { return None; }
    let mut out = [0u8; N];
    for i in 0..N { out[i] = ((text[2 * i] as char).to_digit(16)? * 16 + (text[2 * i + 1] as char).to_digit(16)?) as u8; }
    Some(out)
}
const PREFIX: &[u8] = b"argon2id$m=4096,t=3,p=1$";

fn encode(salt: &[u8; 16], digest: &[u8; 32], out: &mut [u8; STORED_MAX]) -> usize {
    out[..PREFIX.len()].copy_from_slice(PREFIX);
    let mut at = PREFIX.len();
    hex(salt, &mut out[at..at + 32]); at += 32;
    out[at] = b'$'; at += 1;
    hex(digest, &mut out[at..at + 64]); at += 64;
    at
}
fn decode(text: &[u8]) -> Option<([u8; 16], [u8; 32])> {
    let rest = text.strip_prefix(PREFIX)?;
    let rest = rest.strip_suffix(b"\n").unwrap_or(rest);
    if rest.len() != 97 || rest[32] != b'$' { return None; }
    Some((unhex(&rest[..32])?, unhex(&rest[33..])?))
}
fn same(a: &[u8; 32], b: &[u8; 32]) -> bool { a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0 }

/// One request. `may_set` says whether the kernel-named caller may set the password;
/// any caller with a route to Files may check one (that is all a lock screen needs,
/// and a wrong guess costs an Argon2id hash).
pub fn serve(stage: &mut Staging, may_set: bool, arg0: u64, arg1: u64, store: &mut impl Store) -> u8 {
    let Some((verb, chunk)) = crate::wire::decode_request(arg0) else { return INVALID };
    match verb {
        PUT => {
            let at = chunk as usize * 8;
            if at >= MAX { return INVALID; }
            stage.bytes[at..at + 8].copy_from_slice(&arg1.to_le_bytes());
            OK
        }
        STATE => { let mut t = [0; STORED_MAX]; if store.load(&mut t).is_some() { OK } else { NO_PASSWORD } }
        CHECK | SET => {
            let n = arg1 as usize;
            let result = if n > MAX || verb == SET && (!may_set || n == 0) {
                if verb == SET && !may_set { DENIED } else { INVALID }
            } else if verb == CHECK {
                let mut t = [0; STORED_MAX];
                match store.load(&mut t).and_then(|len| decode(&t[..len])) {
                    None => NO_PASSWORD,
                    Some((salt, digest)) => match hash(&stage.bytes[..n], &salt) {
                        Some(h) if same(&h, &digest) => OK,
                        Some(_) => WRONG,
                        None => INVALID,
                    },
                }
            } else {
                let salt = store.salt();
                match hash(&stage.bytes[..n], &salt) {
                    Some(digest) => {
                        let mut t = [0; STORED_MAX];
                        let len = encode(&salt, &digest, &mut t);
                        if store.save(&t[..len]) { OK } else { NOT_SAVED }
                    }
                    None => INVALID,
                }
            };
            stage.wipe();
            result
        }
        _ => INVALID,
    }
}

#[cfg(all(test, feature = "password"))]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Mem { text: Option<Vec<u8>>, salts: u8 }
    impl Store for Mem {
        fn load(&mut self, out: &mut [u8; STORED_MAX]) -> Option<usize> {
            let t = self.text.as_ref()?;
            out[..t.len()].copy_from_slice(t);
            Some(t.len())
        }
        fn save(&mut self, text: &[u8]) -> bool { self.text = Some(text.to_vec()); true }
        fn salt(&mut self) -> [u8; 16] { self.salts += 1; [self.salts; 16] }
    }
    fn send(stage: &mut Staging, store: &mut Mem, may_set: bool, password: &[u8], verb: u8) -> u8 {
        let mut last = OK;
        for (a, b) in requests(password, verb) { last = serve(stage, may_set, a, b, store); }
        last
    }

    #[test]
    fn set_then_check() {
        let (mut stage, mut store) = (Staging::new(), Mem::default());
        assert_eq!(send(&mut stage, &mut store, false, b"hunter2", CHECK), NO_PASSWORD);
        assert_eq!(serve(&mut stage, false, crate::wire::request(STATE, 0), 0, &mut store), NO_PASSWORD);
        assert_eq!(send(&mut stage, &mut store, false, b"hunter2", SET), DENIED, "only Settings sets it");
        assert_eq!(send(&mut stage, &mut store, true, b"correct horse battery staple!", SET), OK);
        let text = store.text.clone().unwrap();
        assert!(text.starts_with(b"argon2id$m=4096,t=3,p=1$"));
        assert!(!text.windows(7).any(|w| w == b"correct"), "only the hash is stored");
        assert!(stage.bytes.iter().all(|b| *b == 0), "staging wiped");
        assert_eq!(send(&mut stage, &mut store, false, b"correct horse battery staple!", CHECK), OK);
        assert_eq!(send(&mut stage, &mut store, false, b"correct horse battery staple", CHECK), WRONG);
        assert_eq!(send(&mut stage, &mut store, false, b"", CHECK), WRONG);
        assert_eq!(serve(&mut stage, false, crate::wire::request(STATE, 0), 0, &mut store), OK);
        // A new password replaces the old one, with a new salt.
        assert_eq!(send(&mut stage, &mut store, true, b"x", SET), OK);
        assert_ne!(store.text.clone().unwrap(), text);
        assert_eq!(send(&mut stage, &mut store, false, b"x", CHECK), OK);
    }

    #[test]
    fn bad_requests() {
        let (mut stage, mut store) = (Staging::new(), Mem::default());
        assert_eq!(serve(&mut stage, true, crate::wire::request(PUT, 8), 0, &mut store), INVALID);
        assert_eq!(serve(&mut stage, true, crate::wire::request(SET, 0), 65, &mut store), INVALID);
        assert_eq!(serve(&mut stage, true, crate::wire::request(SET, 0), 0, &mut store), INVALID, "empty password");
        assert_eq!(serve(&mut stage, true, crate::wire::request(9, 0), 0, &mut store), INVALID);
        store.text = Some(b"argon2id$garbage".to_vec());
        assert_eq!(send(&mut stage, &mut store, false, b"x", CHECK), NO_PASSWORD, "an unreadable hash is no password");
        assert_eq!(decode_reply(reply(WRONG)), Some(WRONG));
        assert_eq!(decode_reply(0xC6 << 56), None);
    }
}
