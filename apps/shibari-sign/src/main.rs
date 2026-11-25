// SPDX-License-Identifier: GPL-3.0-only
//! Shibari's PDF signer: a separate SexOS program row and protection view.
//! `run shibari-sign @certificate.der` imports only the public certificate;
//! the person enters its P-256 scalar in this window. The scalar is never
//! persisted or handed to Vellum or Shell.

#[cfg(target_os = "hermit")]
mod guest;
mod source;

fn main() {
    #[cfg(target_os = "hermit")]
    guest::main();
}

#[cfg(test)]
mod tests {
    use shibari::{Keeper, Owner};

    #[test]
    fn imported_certificate_must_name_the_ram_only_key() {
        let certificate = include_bytes!("../../../crates/vellum-edit/tests/fixtures/cms-test-cert.der");
        let scalar = "15b6eb1d2fa7ec84fc8d80ef31bc5f6c37db2e8887945932ee00c7322c42a7ce";
        let mut key = [0u8; 32];
        for (i, pair) in scalar.as_bytes().chunks_exact(2).enumerate() {
            key[i] = u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap();
        }
        let mut keeper = Keeper::new(19, 2);
        let owner = Owner(41);
        let knot = keeper.tie(owner, "fixture certificate", &key).unwrap();
        assert_eq!(keeper.p256_public_key(owner, knot).unwrap(), vellum_edit::cms::certificate_public_point(certificate).unwrap());
        key[0] ^= 1;
        let wrong = keeper.tie(owner, "wrong key", &key).unwrap();
        assert_ne!(keeper.p256_public_key(owner, wrong).unwrap(), vellum_edit::cms::certificate_public_point(certificate).unwrap());
    }
}
