// SPDX-License-Identifier: GPL-3.0-only
//! Shibari's PDF signer: a separate SexOS program row and protection view.
//! `run shibari-sign @certificate.der` imports only the public certificate;
//! the person enters its P-256 scalar in this window. The scalar is never
//! persisted or handed to Vellum or Shell.

#[cfg(target_os = "hermit")]
mod guest;

fn main() {
    #[cfg(target_os = "hermit")]
    guest::main();
}
