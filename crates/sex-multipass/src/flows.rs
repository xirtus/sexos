// SPDX-License-Identifier: GPL-3.0-only
//! Linking flows, ported from leeloo-multipass `routes/networks.ts` and `activitypub.ts`.
//!
//! What changed for SexOS:
//! - The callback is not a web route. The authorize URL opens in Kaleidoscope; a
//!   navigation to `multipass://callback/<provider>?...` is handed to Multipass, which owns
//!   that scheme. The pending flow is keyed by `state` and tied to the person who started
//!   it (leeloo-multipass used an httpOnly cookie for the same job).
//! - Leeloo only *verifies* ownership and then signs out. SexOS accounts also serve apps,
//!   so the person chooses: [`Keep::VerifyOnly`] signs out as before, [`Keep::ForApps`]
//!   keeps the token in Shibari for act-for-account grants.
use crate::address::{form, network_account, query_param, NetworkAccount, Provider};
use crate::dpop::DpopKey;
use crate::http::{checked, json, Request, Transport};
use crate::net::public_url;
use crate::pkce::{pkce, profile_code, state};
use crate::state::Expiring;
use crate::{b64, ApiError, Result};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use serde::Deserialize;

pub const CALLBACK: &str = "multipass://callback/";
pub fn callback(p: Provider) -> String { format!("{CALLBACK}{}", p.as_str()) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep { VerifyOnly, ForApps }

/// What the service needs from the machine: the network, entropy and the clock.
pub trait Env {
    fn transport(&mut self) -> &mut dyn Transport;
    fn random(&mut self, out: &mut [u8]);
    fn now(&self) -> u64;
}

fn rand<const N: usize>(env: &mut dyn Env) -> [u8; N] { let mut b = [0; N]; env.random(&mut b); b }

pub(crate) enum Kind {
    Mastodon { client_id: String, client_secret: String, verifier: String },
    Matrix { homeserver: String },
    Bluesky { expected_did: String, pds: String, verifier: String, issuer: String, token_endpoint: String, dpop: DpopKey, nonce: Option<String> },
}

pub(crate) struct Flow { pub principal: String, pub account: NetworkAccount, pub keep: Keep, pub kind: Kind }

/// A finished link: what the provider proved, and the session to keep if asked.
pub struct Proven {
    pub principal: String,
    pub account: NetworkAccount,
    pub subject: String,
    pub session: Option<Session>,
}

/// A kept session. `token` goes straight into Shibari; the rest stays in Multipass's RAM.
pub struct Session { pub api_base: String, pub token: Vec<u8>, pub dpop: Option<(DpopKey, Option<String>)> }

/// How Multipass presents a kept secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind { Bearer, Dpop, Basic }

#[derive(Clone)]
pub struct Proof { pub principal: String, pub address: String, pub actor_url: String, pub code: String }

pub struct Challenge { pub id: String, pub code: String, pub address: String, pub actor_url: String, pub profile_url: String, pub expires_in: u64 }

pub struct Linker {
    pub(crate) flows: Expiring<Flow>,
    proofs: Expiring<Proof>,
    mastodon_apps: BTreeMap<String, (String, String)>,
    /// Where SexOS's AT Protocol client metadata is published (an HTTPS URL).
    pub bluesky_client_id: String,
}

const MASTODON_SCOPES_VERIFY: &str = "read:accounts";
const MASTODON_SCOPES_APPS: &str = "read write:statuses write:conversations write:media";
const BLUESKY_SCOPES_VERIFY: &str = "atproto";
const BLUESKY_SCOPES_APPS: &str = "atproto transition:generic transition:chat.bsky";
pub const BLUESKY_CLIENT_ID: &str = "https://sexos.org/multipass/client-metadata.json";

#[derive(Deserialize)] struct App { client_id: Option<String>, client_secret: Option<String> }
#[derive(Deserialize)] struct Token { access_token: Option<String>, sub: Option<String> }
#[derive(Deserialize)] struct MastodonProfile { id: Option<String>, username: Option<String>, acct: Option<String> }
#[derive(Deserialize)] struct LoginFlows { flows: Option<Vec<LoginFlow>> }
#[derive(Deserialize)] struct LoginFlow { #[serde(rename = "type")] kind: String }
#[derive(Deserialize)] struct WhoAmI { user_id: String }
#[derive(Deserialize)] struct Jrd { subject: Option<String>, links: Option<Vec<JrdLink>> }
#[derive(Deserialize)] struct JrdLink { rel: String, #[serde(rename = "type")] kind: Option<String>, href: Option<String> }
#[derive(Deserialize)] struct Actor { id: String, summary: Option<String> }
#[derive(Deserialize)] struct ResolveHandle { did: String }
#[derive(Deserialize)] struct DidDoc { id: String, service: Option<Vec<DidService>> }
#[derive(Deserialize)] struct DidService { id: String, #[serde(rename = "serviceEndpoint")] endpoint: String }
#[derive(Deserialize)] struct ProtectedResource { authorization_servers: Vec<String> }
#[derive(Deserialize)] struct AuthServer { issuer: String, authorization_endpoint: String, token_endpoint: String, pushed_authorization_request_endpoint: String }
#[derive(Deserialize)] struct Par { request_uri: String }

impl Default for Linker { fn default() -> Self { Self::new() } }

impl Linker {
    pub fn new() -> Self {
        Self { flows: Expiring::new(1200, 64), proofs: Expiring::new(3600, 64), mastodon_apps: BTreeMap::new(), bluesky_client_id: BLUESKY_CLIENT_ID.into() }
    }

    /// Start a verified link. Returns the URL to open in Kaleidoscope.
    pub fn connect(&mut self, env: &mut dyn Env, principal: &str, provider: Provider, address: &str, keep: Keep) -> Result<String> {
        let account = network_account(provider, address)?;
        let st = state(&rand::<18>(env));
        let (url, kind) = match provider {
            Provider::Dav => return Err(ApiError::new("unsupported_provider", "DAV servers sign in with an app password: add them as a server account.")),
            Provider::Fediverse => return Err(ApiError::new("unsupported_provider", "Save this profile as a reference, or verify it with a profile code. Signed-in stamps support Bluesky, Mastodon-compatible servers and Matrix SSO.")),
            Provider::Mastodon => self.mastodon_start(env, &account, keep, &st)?,
            Provider::Matrix => matrix_start(env, &account, &st)?,
            Provider::Bluesky => self.bluesky_start(env, &account, keep, &st)?,
        };
        public_url(&url)?;
        if !self.flows.set(&st, Flow { principal: principal.into(), account, keep, kind }, env.now()) {
            return Err(ApiError::new("busy", "Too many links are waiting. Finish or cancel one first."));
        }
        Ok(url)
    }

    fn mastodon_start(&mut self, env: &mut dyn Env, account: &NetworkAccount, keep: Keep, st: &str) -> Result<(String, Kind)> {
        let scopes = if keep == Keep::ForApps { MASTODON_SCOPES_APPS } else { MASTODON_SCOPES_VERIFY };
        let key = format!("{}|{scopes}", account.network);
        let (client_id, client_secret) = match self.mastodon_apps.get(&key) {
            Some(c) => c.clone(),
            None => {
                let app: App = json(env.transport(), &Request::post_form(format!("{}/api/v1/apps", account.network),
                    form(&[("client_name", "SexOS Multipass"), ("redirect_uris", &callback(Provider::Mastodon)), ("scopes", scopes), ("website", "https://sexos.org")])))?;
                match (app.client_id, app.client_secret) {
                    (Some(i), Some(s)) if !i.is_empty() && !s.is_empty() => { self.mastodon_apps.insert(key, (i.clone(), s.clone())); (i, s) }
                    _ => return Err(ApiError::new("invalid_provider", "Server did not return valid application credentials.")),
                }
            }
        };
        let p = pkce(&rand::<32>(env));
        let url = format!("{}/oauth/authorize?{}", account.network, form(&[("client_id", &client_id), ("redirect_uri", &callback(Provider::Mastodon)),
            ("response_type", "code"), ("scope", scopes), ("state", st), ("code_challenge", &p.challenge), ("code_challenge_method", "S256")]));
        Ok((url, Kind::Mastodon { client_id, client_secret, verifier: p.verifier }))
    }

    fn bluesky_start(&mut self, env: &mut dyn Env, account: &NetworkAccount, keep: Keep, st: &str) -> Result<(String, Kind)> {
        let t = env.transport();
        // Handle → DID → PDS → authorization server.
        let did = json::<ResolveHandle>(t, &Request::get(format!("https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle?handle={}", account.address)))?.did;
        let doc_url = if let Some(id) = did.strip_prefix("did:plc:") {
            if !id.bytes().all(|b| b.is_ascii_alphanumeric()) { return Err(ApiError::new("bluesky_did", "That handle resolved to an invalid DID.")); }
            format!("https://plc.directory/{did}")
        } else if let Some(host) = did.strip_prefix("did:web:") {
            format!("https://{host}/.well-known/did.json")
        } else { return Err(ApiError::new("bluesky_did", "That handle resolved to an unsupported DID method.")) };
        let doc: DidDoc = json(t, &Request::get(doc_url))?;
        if doc.id != did { return Err(ApiError::new("bluesky_did", "The DID document names a different account.")); }
        let pds = doc.service.unwrap_or_default().into_iter().find(|s| s.id == "#atproto_pds" || s.id.ends_with("#atproto_pds"))
            .ok_or_else(|| ApiError::new("bluesky_pds", "That account has no PDS."))?.endpoint;
        let pds = public_url(&pds)?.origin();
        let pr: ProtectedResource = json(t, &Request::get(format!("{pds}/.well-known/oauth-protected-resource")))?;
        let issuer = pr.authorization_servers.into_iter().next().ok_or_else(|| ApiError::new("bluesky_auth", "The PDS names no authorization server."))?;
        let issuer = public_url(&issuer)?.origin();
        let meta: AuthServer = json(t, &Request::get(format!("{issuer}/.well-known/oauth-authorization-server")))?;
        if meta.issuer != issuer { return Err(ApiError::new("bluesky_auth", "Authorization server metadata names a different issuer.")); }
        // Pushed authorization request with PKCE and DPoP; retry once with the server's nonce.
        let p = pkce(&rand::<32>(env));
        let dpop = loop { if let Some(k) = DpopKey::from_random(&rand::<32>(env)) { break k; } };
        let scope = if keep == Keep::ForApps { BLUESKY_SCOPES_APPS } else { BLUESKY_SCOPES_VERIFY };
        let body = form(&[("client_id", &self.bluesky_client_id), ("redirect_uri", &callback(Provider::Bluesky)), ("response_type", "code"),
            ("scope", scope), ("state", st), ("code_challenge", &p.challenge), ("code_challenge_method", "S256"), ("login_hint", &account.address)]);
        let (par, nonce): (Par, Option<String>) = dpop_json(env, &dpop, None, &meta.pushed_authorization_request_endpoint, body)?;
        let url = format!("{}?{}", meta.authorization_endpoint, form(&[("client_id", &self.bluesky_client_id), ("request_uri", &par.request_uri)]));
        Ok((url, Kind::Bluesky { expected_did: did, pds, verifier: p.verifier, issuer, token_endpoint: meta.token_endpoint, dpop, nonce }))
    }

    /// Finish a link from the `multipass://callback/...` URL.
    pub fn finish(&mut self, env: &mut dyn Env, principal: &str, url: &str) -> Result<Proven> {
        let provider = url.strip_prefix(CALLBACK).and_then(|r| Provider::parse(r.split(['?', '#']).next().unwrap_or("")))
            .ok_or_else(|| ApiError::new("link_mismatch", "This is not a Multipass callback."))?;
        let st = query_param(url, "state").ok_or_else(|| ApiError::new("link_expired", "The linking request expired. Start again in Multipass."))?;
        let now = env.now();
        match self.flows.get(&st, now) {
            Some(f) if f.principal == principal && f.account.provider == provider => {}
            Some(_) => return Err(ApiError::new("link_mismatch", "This linking request belongs to a different person or provider.")),
            None => return Err(ApiError::new("link_expired", "The linking request expired or was already used. Start again in Multipass.")),
        }
        let flow = self.flows.consume(&st, now).ok_or_else(|| ApiError::new("link_used", "That linking request was already used."))?;
        if query_param(url, "error").is_some() { return Err(ApiError::new("link_declined", "Network connection was canceled or declined.")); }
        let keep = flow.keep;
        match flow.kind {
            Kind::Mastodon { client_id, client_secret, verifier } => {
                let code = query_param(url, "code").ok_or_else(|| ApiError::new("link_code", "Provider returned no authorization code."))?;
                let token = json::<Token>(env.transport(), &Request::post_form(format!("{}/oauth/token", flow.account.network), form(&[("grant_type", "authorization_code"),
                    ("client_id", &client_id), ("client_secret", &client_secret), ("redirect_uri", &callback(provider)), ("code", &code), ("code_verifier", &verifier)])))?
                    .access_token.filter(|t| !t.is_empty()).ok_or_else(|| ApiError::new("link_token", "Provider returned no access token."))?;
                let bearer = format!("Bearer {token}");
                let profile = json::<MastodonProfile>(env.transport(), &Request::get(format!("{}/api/v1/accounts/verify_credentials", flow.account.network)).header("authorization", &bearer));
                let result = (|| {
                    let p = profile?;
                    match (p.id, p.username, p.acct) {
                        (Some(id), Some(u), Some(acct)) if !id.is_empty() && !acct.contains('@') => {
                            let host = public_url(&flow.account.network)?.host;
                            Ok((network_account(Provider::Mastodon, &format!("@{u}@{host}"))?, id))
                        }
                        _ => Err(ApiError::new("link_profile", "Provider did not return a local account.")),
                    }
                })();
                if keep == Keep::VerifyOnly || result.is_err() {
                    let _ = checked(env.transport(), &Request::post_form(format!("{}/oauth/revoke", flow.account.network),
                        form(&[("client_id", &client_id), ("client_secret", &client_secret), ("token", &token)])));
                }
                let (account, subject) = result?;
                let session = (keep == Keep::ForApps).then(|| Session { api_base: account.network.clone(), token: token.into_bytes(), dpop: None });
                Ok(Proven { principal: flow.principal, account, subject, session })
            }
            Kind::Matrix { homeserver } => {
                let login_token = query_param(url, "loginToken").ok_or_else(|| ApiError::new("link_code", "Homeserver returned no login token."))?;
                let body = format!(r#"{{"type":"m.login.token","token":{},"initial_device_display_name":"SexOS Multipass"}}"#, crate::dpop::json_str(&login_token));
                let token = json::<Token>(env.transport(), &Request::post_json(format!("{homeserver}/_matrix/client/v3/login"), body))?
                    .access_token.filter(|t| !t.is_empty()).ok_or_else(|| ApiError::new("link_token", "Homeserver returned no access token."))?;
                let bearer = format!("Bearer {token}");
                let who = json::<WhoAmI>(env.transport(), &Request::get(format!("{homeserver}/_matrix/client/v3/account/whoami")).header("authorization", &bearer));
                let result = who.and_then(|w| if w.user_id == flow.account.address { Ok(w.user_id) } else {
                    Err(ApiError::new("link_wrong_account", "You signed into a different Matrix account. Start again with its address."))
                });
                if keep == Keep::VerifyOnly || result.is_err() {
                    let _ = checked(env.transport(), &Request::post_json(format!("{homeserver}/_matrix/client/v3/logout"), "{}".into()).header("authorization", &bearer));
                }
                let subject = result?;
                let account = network_account(Provider::Matrix, &subject)?;
                let session = (keep == Keep::ForApps).then(|| Session { api_base: homeserver, token: token.into_bytes(), dpop: None });
                Ok(Proven { principal: flow.principal, account, subject, session })
            }
            Kind::Bluesky { expected_did, pds, verifier, issuer, token_endpoint, dpop, nonce } => {
                if query_param(url, "iss").as_deref() != Some(issuer.as_str()) {
                    return Err(ApiError::new("link_issuer", "The callback came from a different authorization server."));
                }
                let code = query_param(url, "code").ok_or_else(|| ApiError::new("link_code", "Provider returned no authorization code."))?;
                let body = form(&[("grant_type", "authorization_code"), ("client_id", &self.bluesky_client_id), ("redirect_uri", &callback(provider)), ("code", &code), ("code_verifier", &verifier)]);
                let (token, nonce): (Token, Option<String>) = dpop_json(env, &dpop, nonce, &token_endpoint, body)?;
                if token.sub.as_deref() != Some(expected_did.as_str()) {
                    return Err(ApiError::new("link_wrong_account", "You authorized a different Bluesky account. Start again with its handle."));
                }
                let access = token.access_token.filter(|t| !t.is_empty()).ok_or_else(|| ApiError::new("link_token", "Provider returned no access token."))?;
                let mut account = flow.account;
                account.profile_url = format!("https://bsky.app/profile/{expected_did}");
                let session = (keep == Keep::ForApps).then(|| Session { api_base: pds, token: access.into_bytes(), dpop: Some((dpop, nonce)) });
                Ok(Proven { principal: flow.principal, account, subject: expected_did, session })
            }
        }
    }

    /// Start a profile-code proof for any fediverse address (Threads, Pixelfed, ...).
    pub fn proof_challenge(&mut self, env: &mut dyn Env, principal: &str, address: &str) -> Result<(Challenge, NetworkAccount)> {
        let d = discover_actor(env.transport(), address)?;
        let id = b64::encode(&rand::<24>(env));
        let code = profile_code(&rand::<18>(env));
        let profile_url = match &d.profile_url { Some(u) => public_url(u)?.href(), None => d.account.profile_url.clone() };
        self.proofs.set(&id, Proof { principal: principal.into(), address: d.account.address.clone(), actor_url: d.actor_url.clone(), code: code.clone() }, env.now());
        Ok((Challenge { id, code, address: d.account.address.clone(), actor_url: d.actor_url, profile_url, expires_in: 3600 }, d.account))
    }

    /// Check the code is in the federated bio. `signer` signs the HTTP request (servers with
    /// authorized fetch, such as Threads, refuse unsigned ones).
    pub fn proof_verify(&mut self, env: &mut dyn Env, principal: &str, id: &str, signer: Option<&dyn HttpSigner>) -> Result<Proven> {
        let now = env.now();
        let proof = match self.proofs.get(id, now) {
            Some(p) if p.principal == principal => p.clone(),
            _ => return Err(ApiError::new("proof_expired", "This profile proof expired or belongs to another person.")),
        };
        let d = discover_actor(env.transport(), &proof.address)?;
        if d.actor_url != proof.actor_url { return Err(ApiError::new("actor_changed", "The account’s ActivityPub actor changed. Start a new proof.")); }
        let mut req = Request::get(proof.actor_url.clone()).header("accept", "application/activity+json");
        if let Some(s) = signer { req = sign_get(req, s, &http_date(now))?; }
        let actor: Actor = json(env.transport(), &req).map_err(|_| ApiError::new("activitypub_unavailable", "The ActivityPub profile could not be fetched. Confirm fediverse sharing is enabled and retry after the provider updates."))?;
        if actor.id != proof.actor_url { return Err(ApiError::new("actor_mismatch", "The server returned a different ActivityPub actor.")); }
        if !actor.summary.unwrap_or_default().contains(&proof.code) {
            return Err(ApiError::new("proof_missing", "The code is not in the federated profile bio yet. Save it on the network, allow time for updates, and try again."));
        }
        self.proofs.consume(id, now);
        Ok(Proven { principal: principal.into(), account: d.account, subject: actor.id, session: None })
    }
}

fn matrix_start(env: &mut dyn Env, account: &NetworkAccount, st: &str) -> Result<(String, Kind)> {
    let t = env.transport();
    let mut homeserver = account.network.clone();
    if let Ok(r) = checked(t, &Request::get(format!("{}/.well-known/matrix/client", account.network))) {
        if r.status == 200 {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&r.body) {
                if let Some(b) = v["m.homeserver"]["base_url"].as_str() { homeserver = public_url(b)?.origin(); }
            }
        }
    }
    let login: LoginFlows = json(t, &Request::get(format!("{homeserver}/_matrix/client/v3/login")))?;
    if !login.flows.unwrap_or_default().iter().any(|f| f.kind == "m.login.sso") {
        return Err(ApiError::new("matrix_sso_unavailable", "This homeserver does not expose browser SSO. You can save the address without a verified stamp. Multipass never asks for your password."));
    }
    let back = format!("{}?state={}", callback(Provider::Matrix), crate::address::form_encode(st));
    Ok((format!("{homeserver}/_matrix/client/v3/login/sso/redirect?{}", form(&[("redirectUrl", &back)])), Kind::Matrix { homeserver }))
}

/// POST a form with a DPoP proof, retrying once when the server asks for a fresh nonce.
fn dpop_json<T: serde::de::DeserializeOwned>(env: &mut dyn Env, key: &DpopKey, mut nonce: Option<String>, url: &str, body: String) -> Result<(T, Option<String>)> {
    for _ in 0..2 {
        let jti = b64::encode(&rand::<16>(env));
        let proof = key.proof("POST", url, env.now(), &jti, nonce.as_deref(), None);
        let r = checked(env.transport(), &Request::post_form(url.into(), body.clone()).header("dpop", &proof))?;
        if let Some(n) = r.header("dpop-nonce") { nonce = Some(n.into()); }
        if r.status == 400 && core::str::from_utf8(&r.body).map(|b| b.contains("use_dpop_nonce")).unwrap_or(false) { continue; }
        if !(200..300).contains(&r.status) { return Err(ApiError::new("network_provider_error", &format!("Authorization server returned HTTP {}.", r.status))); }
        let v = serde_json::from_slice(&r.body).map_err(|_| ApiError::new("network_provider_error", "Authorization server returned something that is not JSON."))?;
        return Ok((v, nonce));
    }
    Err(ApiError::new("dpop_nonce", "The authorization server kept refusing the DPoP nonce."))
}

pub struct Discovery { pub actor_url: String, pub profile_url: Option<String>, pub account: NetworkAccount }

/// WebFinger an address and find its ActivityPub actor.
pub fn discover_actor(t: &mut dyn Transport, address: &str) -> Result<Discovery> {
    let a = address.trim().strip_prefix('@').unwrap_or(address.trim());
    let (name, server) = a.split_once('@').ok_or_else(|| ApiError::new("activitypub_address", "Use a full federated address, such as @alice@social.example."))?;
    let origin = public_url(&format!("https://{}", server.to_ascii_lowercase()))?;
    let acct = format!("{name}@{}", origin.host);
    let finger: Jrd = json(t, &Request::get(format!("{}/.well-known/webfinger?{}", origin.origin(), form(&[("resource", &format!("acct:{acct}"))]))).header("accept", "application/jrd+json"))?;
    if !finger.subject.unwrap_or_default().eq_ignore_ascii_case(&format!("acct:{acct}")) {
        return Err(ApiError::new("activitypub_subject", "The server returned a different account."));
    }
    let links = finger.links.unwrap_or_default();
    let actor = links.iter().find(|l| l.rel == "self" && matches!(l.kind.as_deref(),
        Some("application/activity+json") | Some("application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\""))).and_then(|l| l.href.clone())
        .ok_or_else(|| ApiError::new("activitypub_not_enabled", "This address has no ActivityPub profile. Enable fediverse sharing on the network first."))?;
    let profile_url = links.iter().find(|l| l.rel == "http://webfinger.net/rel/profile-page").and_then(|l| l.href.clone());
    Ok(Discovery { actor_url: public_url(&actor)?.href(), profile_url, account: network_account(Provider::Fediverse, &format!("@{acct}"))? })
}

/// Signs draft-cavage HTTP signatures (`rsa-sha256`) with the service actor's key.
pub trait HttpSigner {
    fn key_id(&self) -> String;
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>>;
}

/// The signing string leeloo-multipass used: `(request-target) host date`.
pub fn signing_string(method: &str, url: &str, date: &str) -> Result<String> {
    let u = public_url(url)?;
    Ok(format!("(request-target): {} {}\nhost: {}\ndate: {date}", method.to_ascii_lowercase(), u.path, u.host))
}

fn sign_get(req: Request, s: &dyn HttpSigner, date: &str) -> Result<Request> {
    let sig = s.sign(signing_string("GET", &req.url, date)?.as_bytes())?;
    let header = format!(r#"keyId="{}",algorithm="rsa-sha256",headers="(request-target) host date",signature="{}""#, s.key_id(), std_b64(&sig));
    Ok(req.header("date", date).header("signature", &header))
}

fn std_b64(b: &[u8]) -> String {
    let mut s: String = b64::encode(b).chars().map(|c| match c { '-' => '+', '_' => '/', c => c }).collect();
    while s.len() % 4 != 0 { s.push('='); }
    s
}

/// RFC 7231 IMF-fixdate from Unix seconds.
pub fn http_date(unix: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = unix / 86400; let secs = unix % 86400;
    // Civil-from-days (Howard Hinnant).
    let z = days as i64 + 719_468; let era = z.div_euclid(146_097); let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{}, {:02} {} {} {:02}:{:02}:{:02} GMT", DAYS[(days % 7) as usize], d, MONTHS[(m - 1) as usize], y, secs / 3600, secs / 60 % 60, secs % 60)
}

