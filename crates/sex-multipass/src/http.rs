// SPDX-License-Identifier: GPL-3.0-only
//! The HTTP exchange Multipass needs, kept transport-free. On SexOS the service sends these
//! through sex-net flows with TLS; host tests answer them from fixtures.
use crate::net::public_url;
use crate::{ApiError, Result};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn get(url: String) -> Self { Self { method: "GET", url, headers: Vec::new(), body: Vec::new() } }
    pub fn post_form(url: String, body: String) -> Self {
        Self { method: "POST", url, headers: alloc::vec![("content-type".into(), "application/x-www-form-urlencoded".into())], body: body.into_bytes() }
    }
    pub fn post_json(url: String, body: String) -> Self {
        Self { method: "POST", url, headers: alloc::vec![("content-type".into(), "application/json".into())], body: body.into_bytes() }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self { self.headers.push((k.into(), v.into())); self }
    pub fn header_value(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }

impl Response {
    pub fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

/// Largest provider response kept (leeloo-multipass: 2 MiB).
pub const MAX_RESPONSE: usize = 2 * 1024 * 1024;

pub trait Transport {
    fn send(&mut self, request: &Request) -> Result<Response>;
}

/// Send one request to a public HTTPS server and require a 2xx JSON body.
pub fn json<T: serde::de::DeserializeOwned>(t: &mut dyn Transport, request: &Request) -> Result<T> {
    let r = checked(t, request)?;
    if !(200..300).contains(&r.status) {
        return Err(ApiError::new("network_provider_error", &alloc::format!("Network provider returned HTTP {}.", r.status)));
    }
    serde_json::from_slice(&r.body).map_err(|_| ApiError::new("network_provider_error", "Network provider returned something that is not JSON."))
}

/// Every request is re-checked against the public-URL rule, and responses are bounded.
/// Redirects are not followed for credential requests (the service's transport follows
/// GET redirects itself and drops `authorization` when the origin changes).
pub fn checked(t: &mut dyn Transport, request: &Request) -> Result<Response> {
    public_url(&request.url)?;
    let r = t.send(request)?;
    if r.body.len() > MAX_RESPONSE { return Err(ApiError::new("network_response_size", "Provider response was too large.")); }
    if matches!(r.status, 301 | 302 | 303 | 307 | 308) && request.method != "GET" {
        return Err(ApiError::new("network_redirect", "Provider redirected a credential request."));
    }
    Ok(r)
}
