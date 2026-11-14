// SPDX-License-Identifier: GPL-3.0-only
//! The unlocked key material, behind one small type.
//!
//! While a vault is unlocked, the master password is needed again for every save (KDBX4
//! derives a fresh key with a new seed each time it writes). This module is the only
//! place that holds it. Nothing else in Chastity sees the password after unlock: the
//! vault asks the keeper for a [`DatabaseKey`] when it opens or saves, and the key is
//! dropped (and zeroized by keepass) right after.
//!
//! **Shibari.** SexOS's planned RAM-only secret keeper is Shibari: a separate organ that
//! holds secrets in its own protection key, never on disk, and hands out only the
//! operations a grant allows. When it exists, [`Keeper`] becomes a client of it: `hold`
//! sends the password to Shibari and keeps a handle, `database_key` asks Shibari for the
//! derived key (or for the whole seal/unseal), and `lock` revokes the handle. The rest
//! of Chastity does not change, because this is the whole surface:
//!
//! * [`Keeper::hold`]: take the password, once, at unlock or create.
//! * [`Keeper::database_key`]: a KDBX key for one open or save (crate-private).
//! * [`Keeper::lock`] / drop: zeroize.
//! * [`Keeper::is_held`].
use keepass::DatabaseKey;
use zeroize::{Zeroize, Zeroizing};

/// The master password of the unlocked vault, zeroized on [`Keeper::lock`] and on drop.
#[derive(Default)]
pub struct Keeper {
    password: Option<Zeroizing<String>>,
}

impl Keeper {
    /// Holds `password` for this unlock. The caller's copy is zeroized here, so a typed
    /// buffer handed over does not linger.
    pub fn hold(password: &mut String) -> Self {
        let held = Zeroizing::new(password.clone());
        password.zeroize();
        Self { password: Some(held) }
    }

    pub fn is_held(&self) -> bool { self.password.is_some() }

    /// A key for one open or save. keepass zeroizes it when it is dropped.
    pub(crate) fn database_key(&self) -> Option<DatabaseKey> {
        self.password.as_ref().map(|p| DatabaseKey::new().with_password(p))
    }

    /// Forgets the password: its bytes are overwritten before the memory is freed.
    pub fn lock(&mut self) {
        // Zeroizing overwrites the String's buffer on drop.
        self.password = None;
    }
}

impl core::fmt::Debug for Keeper {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Keeper").field("held", &self.is_held()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_takes_the_callers_copy_and_lock_forgets() {
        let mut typed = String::from("correct horse");
        let mut k = Keeper::hold(&mut typed);
        assert!(typed.is_empty(), "the typed buffer is zeroized");
        assert!(k.is_held() && k.database_key().is_some());
        assert_eq!(format!("{k:?}"), "Keeper { held: true }");
        k.lock();
        assert!(!k.is_held() && k.database_key().is_none());
    }
}
