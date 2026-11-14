// SPDX-License-Identifier: GPL-3.0-only
//! The Multipass service state: the stamp book, the linker, kept sessions (tokens in
//! Shibari) and act-for-account grants. `servers/multipass` wraps this in the PDX protocol
//! of `sex_pdx::accounts`.
//!
//! The SDK's promise ("open the trusted passport; the app receives no credentials")
//! becomes this rule: an app holding a grant for a stamp may ask Multipass to send a request
//! *to that account's own server*; Multipass adds the token inside Shibari's closure and
//! returns the response. There is no call that returns a token.
use crate::address::{network_account, server_account, Provider};
use crate::dpop::DpopKey;
use crate::flows::{Challenge, Env, HttpSigner, Keep, Linker, Proven};
use crate::http::{checked, Request, Response};
use crate::net::public_url;
use crate::stamps::{Event, StampBook};
use crate::{b64, ApiError, Result};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use shibari::{Keeper, Knot, Owner};

/// The domain an app runs in. Filled in by the service from the PDX call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct App(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope { Read, Write }

struct Kept { knot: Knot, api_base: String, dpop: Option<(DpopKey, Option<String>)>, basic: bool }

pub struct Multipass {
    pub book: StampBook,
    pub linker: Linker,
    keeper: Keeper,
    me: Owner,
    sessions: BTreeMap<u64, Kept>,
    grants: BTreeSet<(App, u64, Scope)>,
    pub audit: Vec<(u64, String, &'static str, String)>,
}

/// What an app may ask Multipass to do for an account.
pub struct Act<'a> {
    pub method: &'static str,
    pub path: &'a str,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
    /// Extra request headers, from [`ACT_HEADERS`] only.
    pub headers: &'a [(&'a str, &'a str)],
}

/// Headers an app may set on an act-for request. Nothing that carries credentials or
/// redirects the request elsewhere (no `authorization`, `cookie`, `destination`, `host`).
pub const ACT_HEADERS: [&str; 6] = ["accept", "depth", "if-match", "if-none-match", "prefer", "idempotency-key"];

impl Multipass {
    /// `me` is Multipass's own domain as Shibari sees it.
    pub fn new(me: Owner, keeper: Keeper) -> Self {
        Self { book: StampBook::new(), linker: Linker::new(), keeper, me, sessions: BTreeMap::new(), grants: BTreeSet::new(), audit: Vec::new() }
    }

    fn log(&mut self, now: u64, principal: &str, action: &'static str, target: &str) {
        self.audit.push((now, principal.into(), action, target.into()));
    }

    /// Save an address as a dashed stamp.
    pub fn save(&mut self, env: &dyn Env, principal: &str, provider: Provider, address: &str) -> Result<u64> {
        let account = network_account(provider, address)?;
        let (id, _) = self.book.save(principal, account, None, env.now())?;
        Ok(id)
    }

    pub fn connect(&mut self, env: &mut dyn Env, principal: &str, provider: Provider, address: &str, keep: Keep) -> Result<String> {
        self.linker.connect(env, principal, provider, address, keep)
    }

    /// Finish from the `multipass://callback/...` URL Kaleidoscope handed over.
    pub fn finish(&mut self, env: &mut dyn Env, principal: &str, url: &str) -> Result<u64> {
        let proven = self.linker.finish(env, principal, url)?;
        self.record(env.now(), proven)
    }

    pub fn proof_challenge(&mut self, env: &mut dyn Env, principal: &str, address: &str) -> Result<Challenge> {
        let (c, account) = self.linker.proof_challenge(env, principal, address)?;
        self.book.save(principal, account, None, env.now())?;
        Ok(c)
    }

    pub fn proof_verify(&mut self, env: &mut dyn Env, principal: &str, id: &str, signer: Option<&dyn HttpSigner>) -> Result<u64> {
        let proven = self.linker.proof_verify(env, principal, id, signer)?;
        self.record(env.now(), proven)
    }

    fn record(&mut self, now: u64, p: Proven) -> Result<u64> {
        let provider = p.account.provider;
        let (id, ev) = self.book.save(&p.principal, p.account, Some(&p.subject), now)?;
        debug_assert_eq!(ev, Event::Verified);
        if let Some(s) = p.session {
            let label = format!("{} stamp {id}", provider.as_str());
            let knot = match self.sessions.get(&id) {
                Some(k) => { self.keeper.retie(self.me, k.knot, &s.token).map_err(|_| shibari_err())?; k.knot }
                None => self.keeper.tie(self.me, &label, &s.token).map_err(|_| shibari_err())?,
            };
            let mut token = s.token; token.iter_mut().for_each(|b| *b = 0);
            self.sessions.insert(id, Kept { knot, api_base: s.api_base, dpop: s.dpop, basic: false });
        }
        self.log(now, &p.principal, "network.verified", provider.as_str());
        Ok(id)
    }

    /// Add an account on a server that signs in with a user name and app password (CalDAV,
    /// CardDAV, WebDAV). The password goes into Shibari; the stamp stays dashed, because a
    /// password proves access, not identity. `check` is a request path Multipass sends
    /// (DAV: `PROPFIND` on the base path) to confirm the password before keeping it.
    pub fn add_server_account(&mut self, env: &mut dyn Env, principal: &str, provider: Provider, base_url: &str, user: &str, password: &str) -> Result<u64> {
        let account = server_account(provider, base_url, user)?;
        let mut secret: Vec<u8> = format!("{user}:{password}").into_bytes();
        let auth = format!("Basic {}", crate::b64::encode(&secret).replace('-', "+").replace('_', "/"));
        let auth = pad4(auth);
        let probe = Request { method: "PROPFIND", url: account.profile_url.clone(), headers: Vec::new(), body: Vec::new() }
            .header("depth", "0").header("authorization", &auth);
        let r = checked(env.transport(), &probe);
        let ok = matches!(&r, Ok(r) if r.status == 207 || (200..300).contains(&r.status));
        if !ok {
            secret.iter_mut().for_each(|b| *b = 0);
            return Err(match r { Ok(r) if r.status == 401 || r.status == 403 => ApiError::new("bad_password", "The server refused that user name or app password."),
                Ok(_) => ApiError::new("not_dav", "That address does not answer like a DAV server."), Err(e) => e });
        }
        let (id, _) = self.book.save(principal, account.clone(), None, env.now())?;
        let knot = match self.sessions.get(&id) {
            Some(k) => { self.keeper.retie(self.me, k.knot, &secret).map_err(|_| shibari_err())?; k.knot }
            None => self.keeper.tie(self.me, &format!("{} stamp {id}", provider.as_str()), &secret).map_err(|_| shibari_err())?,
        };
        secret.iter_mut().for_each(|b| *b = 0);
        self.sessions.insert(id, Kept { knot, api_base: account.network, dpop: None, basic: true });
        self.log(env.now(), principal, "account.added", provider.as_str());
        Ok(id)
    }

    /// Whether apps can act for this stamp (a session is kept in Shibari).
    pub fn signed_in(&self, id: u64) -> bool { self.sessions.contains_key(&id) }

    pub fn disconnect(&mut self, env: &dyn Env, principal: &str, id: u64) -> bool {
        if !self.book.remove(principal, id) { return false; }
        if let Some(k) = self.sessions.remove(&id) { let _ = self.keeper.untie(self.me, k.knot); }
        self.grants.retain(|(_, s, _)| *s != id);
        self.log(env.now(), principal, "network.disconnected", "");
        true
    }

    /// The person (through Tatami or a Collar prompt) lets `app` act for a stamp.
    pub fn grant(&mut self, principal: &str, app: App, id: u64, scope: Scope) -> Result<()> {
        match self.book.get(id) {
            Some(s) if s.principal == principal => { self.grants.insert((app, id, scope)); Ok(()) }
            _ => Err(ApiError::new("unknown_stamp", "No such stamp in your Multipass.")),
        }
    }
    pub fn revoke(&mut self, app: App, id: u64) { self.grants.retain(|(a, s, _)| !(*a == app && *s == id)); }

    /// Always refused: tokens never leave Multipass and Shibari.
    pub fn token(&self, _app: App, _id: u64) -> Result<Vec<u8>> {
        Err(ApiError::new("token_never_leaves", "Apps never receive account tokens. Ask Multipass to act for the account instead."))
    }

    /// Send one request for an account, to that account's own server.
    pub fn act(&mut self, env: &mut dyn Env, app: App, id: u64, act: &Act) -> Result<Response> {
        let scope = if matches!(act.method, "GET" | "HEAD" | "PROPFIND" | "REPORT") { Scope::Read } else { Scope::Write };
        if !matches!(act.method, "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "PROPFIND" | "REPORT" | "PROPPATCH" | "MKCOL" | "MKCALENDAR") {
            return Err(ApiError::new("act_method", "That request method is not allowed."));
        }
        if !self.grants.contains(&(app, id, scope)) && !(scope == Scope::Read && self.grants.contains(&(app, id, Scope::Write))) {
            return Err(ApiError::new("no_grant", "This app has no permission to act for that account."));
        }
        let kept = self.sessions.get_mut(&id).ok_or_else(|| ApiError::new("not_signed_in", "That account is verified but not signed in for apps."))?;
        // Path only, on the account's own server: a token can never be sent elsewhere.
        let p = act.path;
        if !p.starts_with('/') || p.starts_with("//") || p.contains('\\') || p.contains('#') || p.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return Err(ApiError::new("act_path", "Give a path on the account's own server."));
        }
        let url = format!("{}{}", kept.api_base, p);
        if public_url(&url)?.origin() != kept.api_base { return Err(ApiError::new("act_path", "Give a path on the account's own server.")); }
        let mut req = Request { method: act.method, url, headers: Vec::new(), body: act.body.to_vec() };
        if let Some(ct) = act.content_type { req = req.header("content-type", ct); }
        for (k, v) in act.headers {
            if !ACT_HEADERS.iter().any(|a| a.eq_ignore_ascii_case(k)) || v.bytes().any(|b| b < b' ' || b == 0x7f) {
                return Err(ApiError::new("act_header", "That request header is not allowed."));
            }
            req = req.header(k, v);
        }
        let mut jti = [0u8; 16]; env.random(&mut jti);
        let now = env.now();
        let (keeper, me) = (&self.keeper, self.me);
        let basic = kept.basic;
        let req = keeper.with(me, kept.knot, |token| {
            if basic { return req.clone().header("authorization", &pad4(format!("Basic {}", crate::b64::encode(token).replace('-', "+").replace('_', "/")))); }
            let token = core::str::from_utf8(token).unwrap_or("");
            match &kept.dpop {
                Some((key, nonce)) => {
                    let proof = key.proof(act.method, &req.url, now, &b64::encode(&jti), nonce.as_deref(), Some(token));
                    req.clone().header("authorization", &format!("DPoP {token}")).header("dpop", &proof)
                }
                None => req.clone().header("authorization", &format!("Bearer {token}")),
            }
        }).map_err(|_| shibari_err())?;
        let mut r = checked(env.transport(), &req)?;
        if let (Some((_, nonce)), Some(n)) = (kept.dpop.as_mut(), r.header("dpop-nonce")) { *nonce = Some(n.into()); }
        r.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization") && !k.eq_ignore_ascii_case("set-cookie") && !k.eq_ignore_ascii_case("dpop-nonce"));
        Ok(r)
    }
}

fn pad4(mut s: String) -> String { while (s.len() - 6) % 4 != 0 { s.push('='); } s }

fn shibari_err() -> ApiError { ApiError::new("shibari", "Shibari could not keep the account's session.") }
