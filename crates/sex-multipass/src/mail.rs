// SPDX-License-Identifier: GPL-3.0-only
//! Multipass's SASL step over a server-authenticated mail TLS flow. The
//! transport implementation must verify the certificate and exact account
//! host before it implements [`VerifiedMailTls`]. No such SexNet app flow is
//! boot-routed yet; this module prepares the service-side secret boundary.
use crate::{ApiError, Result};
use alloc::string::{String, ToString};
use sex_mail::{Auth, Conn, imap, smtp};
use shibari::{Keeper, Knot, Owner};

/// A mail stream whose transport has verified the remote server certificate
/// and name. Implemented only by a trusted Multipass transport, never by the
/// requesting app. The fixture implementation below stands in for that proof.
pub trait VerifiedMailTls: Conn {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailServer { pub host: String, pub port: u16 }

impl MailServer {
    /// A domain and TLS port chosen in Multipass's account setup. The requesting
    /// app cannot provide or override either field when opening a flow.
    pub fn new(host: &str, port: u16) -> Result<Self> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let labels: alloc::vec::Vec<_> = host.split('.').collect();
        if host.len() > 253 || labels.len() < 2 || port == 0 || labels.iter().any(|label| {
            label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
                || !label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }) || !labels.last().is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_alphabetic())) {
            return Err(ApiError::new("mail_server", "Enter a mail server domain and a TLS port."));
        }
        Ok(Self { host, port })
    }
}

/// The trusted Multipass transport opens a TLS connection and verifies the
/// certificate for `server.host` before returning it. It must not yield a
/// plaintext or unverified flow. The app cannot implement or call this trait.
pub trait MailDialer {
    type Flow: VerifiedMailTls;
    fn open(&mut self, server: &MailServer) -> Result<Self::Flow>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailPurpose { Imap, Smtp }

pub fn user_name(user: &str) -> Result<String> {
    if user.is_empty() || user.len() > 254 || user.bytes().any(|b| b < b'!' || b == 0x7f) {
        return Err(ApiError::new("mail_user", "Enter a mail account user name without spaces or control characters."));
    }
    Ok(user.to_string())
}

/// Multipass holds the Shibari owner and knot. Valentine receives only the
/// already-authenticated connection after the network service moves its flow.
pub struct MailSasl<'a> { keeper: &'a Keeper, owner: Owner }

impl<'a> MailSasl<'a> {
    pub fn new(keeper: &'a Keeper, owner: Owner) -> Self { Self { keeper, owner } }

    pub fn imap<C: VerifiedMailTls>(&self, knot: Knot, user: &str, conn: C) -> Result<C> {
        let mut client = imap::Client::connect(conn).map_err(|_| auth_error())?;
        self.keeper.with(self.owner, knot, |secret| {
            let password = core::str::from_utf8(secret).map_err(|_| auth_error())?;
            client.authenticate(&Auth::Plain { user, password }).map_err(|_| auth_error())
        }).map_err(|_| keeper_error())??;
        Ok(client.into_conn())
    }

    pub fn smtp<C: VerifiedMailTls>(&self, knot: Knot, user: &str, conn: C) -> Result<C> {
        let mut client = smtp::Client::connect(conn, "multipass.sexos").map_err(|_| auth_error())?;
        self.keeper.with(self.owner, knot, |secret| {
            let password = core::str::from_utf8(secret).map_err(|_| auth_error())?;
            client.authenticate(&Auth::Plain { user, password }).map_err(|_| auth_error())
        }).map_err(|_| keeper_error())??;
        Ok(client.into_conn())
    }
}

fn keeper_error() -> ApiError { ApiError::new("shibari", "Multipass could not use that account secret.") }
fn auth_error() -> ApiError { ApiError::new("mail_auth", "The mail server refused the account sign-in.") }

#[cfg(test)]
mod tests {
    use super::*;
    use sex_mail::fixture::{FixtureConn, Store};

    struct TlsFixture(FixtureConn);
    impl Conn for TlsFixture {
        fn write_all(&mut self, data: &[u8]) -> sex_mail::Result<()> { self.0.write_all(data) }
        fn read(&mut self, buf: &mut [u8]) -> sex_mail::Result<usize> { self.0.read(buf) }
    }
    impl VerifiedMailTls for TlsFixture {}

    #[test]
    fn shibari_authenticates_both_flows_before_handoff() {
        let store = Store::shared();
        store.borrow_mut().add_user("alice@example.org", "private-app-password");
        let owner = Owner(42);
        let mut keeper = Keeper::new(7, 4);
        let knot = keeper.tie(owner, "mail account", b"private-app-password").unwrap();
        let sasl = MailSasl::new(&keeper, owner);
        let imap_flow = sasl.imap(knot, "alice@example.org", TlsFixture(FixtureConn::imap(&store))).unwrap();
        let mut inbox = imap::Client::resume_authenticated(imap_flow);
        assert_eq!(inbox.select("INBOX").unwrap().exists, 0);
        let smtp_flow = sasl.smtp(knot, "alice@example.org", TlsFixture(FixtureConn::smtp(&store))).unwrap();
        let mut sender = smtp::Client::resume_authenticated(smtp_flow);
        sender.send("alice@example.org", &["alice@example.org"], b"From: alice@example.org\r\nTo: alice@example.org\r\nSubject: Shibari\r\n\r\nHello\r\n").unwrap();
        assert_eq!(inbox.uid_search("ALL").unwrap(), [1]);
        assert_eq!(store.borrow().messages("alice@example.org", "INBOX").len(), 1);
    }

    #[test]
    fn wrong_owner_and_wrong_secret_do_not_handoff_a_flow() {
        let store = Store::shared();
        store.borrow_mut().add_user("alice@example.org", "right");
        let mut keeper = Keeper::new(9, 4);
        let knot = keeper.tie(Owner(42), "mail account", b"wrong").unwrap();
        let other = MailSasl::new(&keeper, Owner(43));
        assert_eq!(other.imap(knot, "alice@example.org", TlsFixture(FixtureConn::imap(&store))).err().unwrap().code, "shibari");
        let own = MailSasl::new(&keeper, Owner(42));
        assert_eq!(own.smtp(knot, "alice@example.org", TlsFixture(FixtureConn::smtp(&store))).err().unwrap().code, "mail_auth");
    }
}
