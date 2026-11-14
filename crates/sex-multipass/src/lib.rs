// SPDX-License-Identifier: GPL-3.0-only
//! Multipass: the SexOS accounts service, nativized from Leeloo Multipass.
//!
//! Leeloo Multipass is a Node/Fastify/PostgreSQL web service: a passport ("stamp book") of
//! a person's identities on Bluesky, Mastodon-compatible servers, Matrix and the fediverse,
//! solid stamps for verified ownership and dashed stamps for saved addresses. Its SDK lets
//! another app open the passport without ever receiving credentials.
//!
//! On SexOS the same design is a system service:
//! - the Postgres tables become a [`stamps::StampBook`] kept in Files (no secrets in it);
//! - sealed flow state becomes RAM-only [`state::Expiring`] state;
//! - tokens live in Shibari ([`shibari::Keeper`]), never on disk, never handed out;
//! - Fastify routes become the PDX protocol in `sex_pdx::accounts`;
//! - the SDK's "receive no credentials" rule becomes [`service::Multipass::act`]: an app
//!   holding a grant asks Multipass to fetch or send *for* an account on that account's own
//!   server and gets the result. Asking for the token itself is refused.
#![no_std]
extern crate alloc;

pub mod address;
pub mod b64;
pub mod dpop;
pub mod flows;
pub mod http;
pub mod net;
pub mod pkce;
pub mod service;
pub mod stamps;
pub mod state;

use alloc::string::String;

/// An error with a stable code and a sentence for the person, as in leeloo-multipass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &'static str, message: &str) -> Self { Self { code, message: message.into() } }
}

pub type Result<T> = core::result::Result<T, ApiError>;
