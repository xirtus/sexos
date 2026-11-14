// SPDX-License-Identifier: GPL-3.0-only
//! An unlocked vault: a KDBX4 database and the [`Keeper`] that can save it again.
//!
//! Files are standard KDBX 4 (AES-256 outer cipher, ChaCha20 inner stream, GZip,
//! Argon2id), written by keepass-rs, so KeePassXC and KeePass open them, and Chastity
//! opens theirs (KDBX 3.1 and 4.x, with a password; key files and challenge-response
//! keys are not offered yet). Entries use KeePass's standard fields: Title, UserName,
//! Password (protected), URL, Notes, and `otp` for TOTP, as KeePassXC writes it.
//!
//! There is no locked `Vault`: locking drops it. The decrypted database lives only in
//! this value (keepass keeps protected fields in zeroizing `SecretBox`es) and the
//! master password only in its keeper, so [`Vault::lock`] or drop leaves neither.
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::{fields, EntryRef};
pub use keepass::db::EntryId;
use keepass::{Database, DatabaseKey};
use zeroize::{Zeroize, Zeroizing};

use crate::keeper::Keeper;
use crate::totp::Totp;

/// How hard a new vault's key is to derive (Argon2id). Opening a vault uses whatever
/// the file says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kdf {
    pub memory_bytes: u64,
    pub iterations: u64,
    pub parallelism: u32,
}

impl Kdf {
    /// New vaults on SexOS: 32 MiB, 4 passes, one lane. KeePassXC's own default is
    /// 64 MiB and is fine to open; this keeps an unlock to a few seconds on slow CPUs
    /// and under emulation, where the program's heap is also smaller.
    pub const STANDARD: Kdf = Kdf { memory_bytes: 32 * 1024 * 1024, iterations: 4, parallelism: 1 };
    /// For tests only: fast, and far too weak for a real vault.
    pub const TEST_ONLY: Kdf = Kdf { memory_bytes: 64 * 1024, iterations: 1, parallelism: 1 };
}

#[derive(Debug)]
pub enum OpenError {
    /// The password does not open this vault.
    WrongPassword,
    /// Not a KeePass file, or damaged.
    NotAVault(String),
    /// A KeePass file Chastity cannot open (KDB 1.x/2.x, or a key file is needed).
    Unsupported(String),
}

impl core::fmt::Display for OpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            OpenError::WrongPassword => f.write_str("wrong password"),
            OpenError::NotAVault(e) => write!(f, "not a vault this program can read: {e}"),
            OpenError::Unsupported(e) => write!(f, "unsupported vault: {e}"),
        }
    }
}

#[derive(Debug)]
pub enum SaveError { Locked, Failed(String) }

impl core::fmt::Display for SaveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self { SaveError::Locked => f.write_str("the vault is locked"), SaveError::Failed(e) => write!(f, "could not save: {e}") }
    }
}

/// What a list or search shows of an entry: nothing secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub id: EntryId,
    pub title: String,
    pub username: String,
    pub url: String,
    pub has_totp: bool,
}

/// An entry to add, or new values for one. The password is zeroized after use.
#[derive(Default)]
pub struct NewEntry {
    pub title: String,
    pub username: String,
    pub password: Zeroizing<String>,
    pub url: String,
    pub notes: String,
    /// An `otpauth://totp/` URI or a Base32 secret; empty for none.
    pub otp: Zeroizing<String>,
}

/// One entry in full, for showing or copying. Secret fields zeroize on drop.
pub struct Detail {
    pub summary: Summary,
    pub password: Zeroizing<String>,
    pub notes: String,
    pub otp: Option<Totp>,
}

pub struct Vault {
    db: Database,
    keeper: Keeper,
}

impl Vault {
    /// A new, empty vault named `name`, locked by `password` (the caller's buffer is
    /// zeroized). Nothing is written until [`Vault::save`].
    pub fn create(name: &str, password: &mut String, kdf: Kdf) -> Vault {
        let mut config = DatabaseConfig::default();
        config.kdf_config = KdfConfig::Argon2id { memory: kdf.memory_bytes, iterations: kdf.iterations, parallelism: kdf.parallelism, version: argon2_version() };
        let mut db = Database::with_config(config);
        db.meta.generator = Some("Chastity (SexOS)".into());
        db.meta.database_name = Some(name.into());
        db.root_mut().name = name.into();
        Vault { db, keeper: Keeper::hold(password) }
    }

    /// Opens a vault file's bytes with `password` (the caller's buffer is zeroized,
    /// whether or not it opens).
    pub fn open(bytes: &[u8], password: &mut String) -> Result<Vault, OpenError> {
        let keeper = Keeper::hold(password);
        let key = keeper.database_key().expect("held");
        match Database::parse(bytes, key) {
            Ok(db) => Ok(Vault { db, keeper }),
            Err(e) => Err(classify(e)),
        }
    }

    /// The vault as a KDBX4 file, sealed with the held password and a fresh seed.
    pub fn save(&self) -> Result<Vec<u8>, SaveError> {
        let key: DatabaseKey = self.keeper.database_key().ok_or(SaveError::Locked)?;
        let mut out = Vec::new();
        self.db.save(&mut out, key).map_err(|e| SaveError::Failed(e.to_string()))?;
        Ok(out)
    }

    /// Locks: the password and every decrypted field are zeroized and freed.
    pub fn lock(mut self) { self.keeper.lock(); drop(self); }

    pub fn name(&self) -> &str { self.db.meta.database_name.as_deref().unwrap_or("Vault") }

    /// Every entry outside the recycle bin, by title.
    pub fn entries(&self) -> Vec<Summary> {
        let bin = self.db.meta.recyclebin_uuid;
        let mut list: Vec<Summary> = self.db.iter_all_entries()
            .filter(|e| bin.is_none_or(|b| e.parent().id().uuid() != b))
            .map(|e| summary(&e)).collect();
        list.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.id.uuid().cmp(&b.id.uuid())));
        list
    }

    /// Entries whose title, user name or URL contains every word of `query`
    /// (case-insensitive). An empty query matches everything.
    pub fn search(&self, query: &str) -> Vec<Summary> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.entries().into_iter().filter(|s| {
            let hay = format!("{}\n{}\n{}", s.title, s.username, s.url).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        }).collect()
    }

    pub fn add(&mut self, mut new: NewEntry) -> EntryId {
        let mut root = self.db.root_mut();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, new.title.as_str());
        e.set_unprotected(fields::USERNAME, new.username.as_str());
        e.set_protected(fields::PASSWORD, new.password.as_str());
        e.set_unprotected(fields::URL, new.url.as_str());
        e.set_unprotected(fields::NOTES, new.notes.as_str());
        if !new.otp.trim().is_empty() { e.set_protected(fields::OTP, new.otp.trim()); }
        let id = e.id();
        new.notes.zeroize();
        id
    }

    pub fn remove(&mut self, id: EntryId) -> bool {
        match self.db.entry_mut(id) { Some(e) => { e.remove(); true } None => false }
    }

    pub fn detail(&self, id: EntryId) -> Option<Detail> {
        let e = self.db.entry(id)?;
        Some(Detail {
            summary: summary(&e),
            password: Zeroizing::new(e.get_password().unwrap_or("").to_string()),
            notes: e.get(fields::NOTES).unwrap_or("").to_string(),
            otp: e.get(fields::OTP).and_then(|t| Totp::parse(t).ok()),
        })
    }

    /// The entry's current TOTP code and the seconds it stays valid, at Unix time `now`.
    pub fn totp(&self, id: EntryId, now: u64) -> Option<(String, u64)> {
        let t = self.detail(id)?.otp?;
        Some((t.code(now), t.remaining(now)))
    }
}

fn summary(e: &EntryRef<'_>) -> Summary {
    Summary {
        id: e.id(),
        title: e.get_title().unwrap_or("").to_string(),
        username: e.get_username().unwrap_or("").to_string(),
        url: e.get_url().unwrap_or("").to_string(),
        has_totp: e.get(fields::OTP).is_some_and(|t| Totp::parse(t).is_ok()),
    }
}

fn argon2_version() -> argon2::Version { argon2::Version::Version13 }

fn classify(e: keepass::db::DatabaseOpenError) -> OpenError {
    use keepass::db::DatabaseOpenError as E;
    match e {
        E::Key(k) => match k.to_string().as_str() {
            "Incorrect key" => OpenError::WrongPassword,
            other => OpenError::Unsupported(other.to_string()),
        },
        E::UnsupportedVersion => OpenError::Unsupported("KeePass 1.x/2.x (KDB) files".into()),
        other => OpenError::NotAVault(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pw(s: &str) -> String { s.to_string() }

    fn sample() -> Vault {
        let mut v = Vault::create("Test vault", &mut pw("hunter2 is weak"), Kdf::TEST_ONLY);
        v.add(NewEntry { title: "Mail".into(), username: "andreas@example.org".into(), password: Zeroizing::new("s3cret!".into()), url: "https://mail.example.org/login".into(), notes: "work".into(), otp: Zeroizing::new("otpauth://totp/Mail?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()) });
        v.add(NewEntry { title: "bank".into(), username: "a.b".into(), password: Zeroizing::new("p@ss".into()), url: "https://bank.example.com".into(), ..Default::default() });
        v
    }

    #[test]
    fn create_save_reopen_round_trip() {
        let v = sample();
        let bytes = v.save().unwrap();
        // KDBX signature, version 4.
        assert_eq!(&bytes[..8], &[0x03, 0xd9, 0xa2, 0x9a, 0x67, 0xfb, 0x4b, 0xb5]);
        assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), 4);
        v.lock();
        let mut typed = pw("hunter2 is weak");
        let again = Vault::open(&bytes, &mut typed).unwrap();
        assert!(typed.is_empty(), "the typed password is zeroized");
        assert_eq!(again.name(), "Test vault");
        let list = again.entries();
        assert_eq!(list.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["bank", "Mail"]);
        let mail = again.detail(list[1].id).unwrap();
        assert_eq!((mail.summary.username.as_str(), mail.password.as_str(), mail.summary.url.as_str(), mail.notes.as_str()), ("andreas@example.org", "s3cret!", "https://mail.example.org/login", "work"));
        assert!(mail.summary.has_totp && !list[0].has_totp);
        assert_eq!(again.totp(list[1].id, 59), Some(("287082".to_string(), 1)));
        // Saved twice, the files differ (fresh seed and IV) and both open.
        let b2 = again.save().unwrap();
        assert_ne!(bytes, b2);
        assert_eq!(Vault::open(&b2, &mut pw("hunter2 is weak")).unwrap().entries().len(), 2);
    }

    #[test]
    fn wrong_password_is_refused() {
        let bytes = sample().save().unwrap();
        let mut typed = pw("hunter3 is weak");
        assert!(matches!(Vault::open(&bytes, &mut typed), Err(OpenError::WrongPassword)));
        assert!(typed.is_empty());
        assert!(matches!(Vault::open(&bytes, &mut pw("")), Err(OpenError::WrongPassword)));
    }

    #[test]
    fn garbage_is_not_a_vault() {
        assert!(matches!(Vault::open(b"hello, this is not kdbx at all", &mut pw("x")), Err(OpenError::NotAVault(_))));
        let mut bytes = sample().save().unwrap();
        let n = bytes.len();
        bytes.truncate(n / 2);
        assert!(Vault::open(&bytes, &mut pw("hunter2 is weak")).is_err());
    }

    #[test]
    fn search_matches_every_word_across_fields() {
        let v = sample();
        let titles = |q: &str| v.search(q).into_iter().map(|s| s.title).collect::<Vec<_>>();
        assert_eq!(titles(""), ["bank", "Mail"]);
        assert_eq!(titles("MAIL"), ["Mail"]);
        assert_eq!(titles("example"), ["bank", "Mail"]);
        assert_eq!(titles("example .com"), ["bank"]);
        assert_eq!(titles("a.b"), ["bank"]);
        assert!(titles("s3cret").is_empty(), "passwords are not searched");
        assert!(titles("zzz").is_empty());
    }

    #[test]
    fn remove_takes_an_entry_out() {
        let mut v = sample();
        let id = v.entries()[0].id;
        assert!(v.remove(id));
        assert!(!v.remove(id));
        let bytes = v.save().unwrap();
        assert_eq!(Vault::open(&bytes, &mut pw("hunter2 is weak")).unwrap().entries().len(), 1);
    }

    #[test]
    fn writes_argon2id_with_the_chosen_cost() {
        let v = Vault::create("k", &mut pw("p"), Kdf::TEST_ONLY);
        let bytes = v.save().unwrap();
        let opened = Vault::open(&bytes, &mut pw("p")).unwrap();
        assert!(matches!(opened.db.config.kdf_config, KdfConfig::Argon2id { memory: 65536, iterations: 1, parallelism: 1, .. }), "{:?}", opened.db.config.kdf_config);
    }
}
