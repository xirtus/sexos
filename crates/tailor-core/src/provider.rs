// SPDX-License-Identifier: GPL-3.0-only
//! Multi-provider AI adapters for Gemini, Claude, and Codex (OpenAI).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::agent::{Message, Role, ToolSpec};
use crate::json::{self, JsonValue};
use crate::sse::{HttpSseDecoder, SseError, SseMessage, StreamEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    Antigravity,
    Claude,
    ClaudeDeepSeek,
    Codex,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Antigravity => "antigravity",
            Self::Claude => "claude",
            Self::ClaudeDeepSeek => "claude-deepseek",
            Self::Codex => "codex",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::Antigravity => "Antigravity",
            Self::Claude => "Claude",
            Self::ClaudeDeepSeek => "Claude-DeepSeek",
            Self::Codex => "Codex",
        }
    }

    pub fn host(self) -> &'static str {
        match self {
            Self::Gemini => "generativelanguage.googleapis.com",
            Self::Antigravity => "daily-cloudcode-pa.googleapis.com",
            Self::Claude => "api.anthropic.com",
            Self::ClaudeDeepSeek => "api.deepseek.com",
            Self::Codex => "api.openai.com",
        }
    }

    pub fn port(self) -> u16 {
        443
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Gemini => "gemini-2.5-flash",
            Self::Antigravity => "gemini-2.5-pro",
            Self::Claude => "claude-sonnet-4-20250514",
            Self::ClaudeDeepSeek => "deepseek-chat",
            Self::Codex => "gpt-5.1-codex",
        }
    }

    pub fn available_models(self) -> &'static [&'static str] {
        match self {
            Self::Gemini => &["gemini-2.5-pro", "gemini-2.5-flash"],
            Self::Antigravity => &[
                "gemini-2.5-pro",
                "gemini-2.5-flash",
                "claude-sonnet-4-20250514",
            ],
            Self::Claude => &[
                "claude-sonnet-4-20250514",
                "claude-opus-4-20250514",
                "claude-3-7-sonnet-20250219",
            ],
            Self::Codex => &[
                "gpt-5.1-codex",
                "gpt-5.3-codex",
                "gpt-5.2-codex",
                "gpt-5.1-codex-mini",
                "gpt-5-codex",
                "gpt-5.2",
                "gpt-5",
                "gpt-4.1",
                "o4-mini",
            ],
            Self::ClaudeDeepSeek => &["deepseek-chat", "deepseek-reasoner"],
        }
    }

    pub fn key_filename(self) -> &'static str {
        match self {
            Self::Gemini => "gemini.key",
            Self::Antigravity => "google_oauth.json",
            Self::Claude => "claude.key",
            Self::ClaudeDeepSeek => "claude-deepseek.key",
            Self::Codex => "codex.key",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let lower = s.trim();
        if lower.eq_ignore_ascii_case("antigravity")
            || lower.eq_ignore_ascii_case("cloudcode")
            || lower.eq_ignore_ascii_case("google-oauth")
            || lower.eq_ignore_ascii_case("google_oauth")
        {
            Some(Self::Antigravity)
        } else if lower.eq_ignore_ascii_case("gemini") || lower.eq_ignore_ascii_case("google") {
            Some(Self::Gemini)
        } else if lower.eq_ignore_ascii_case("claude-deepseek")
            || lower.eq_ignore_ascii_case("claude_deepseek")
            || lower.eq_ignore_ascii_case("deepseek")
        {
            Some(Self::ClaudeDeepSeek)
        } else if lower.eq_ignore_ascii_case("claude") || lower.eq_ignore_ascii_case("anthropic") {
            Some(Self::Claude)
        } else if lower.eq_ignore_ascii_case("codex") || lower.eq_ignore_ascii_case("openai") {
            Some(Self::Codex)
        } else {
            None
        }
    }
}

/// Parses a JSON API key bundle into `(Provider, api_key)` pairs.
///
/// Accepts either flat string values (`{"gemini": "...", "claude-deepseek": "..."}`)
/// or nested objects (`{"claude": {"api_key": "..."}}` / `"key"` / `"token"`).
pub fn parse_keys_json(raw: &str) -> Vec<(Provider, String)> {
    let mut out = Vec::new();
    let Ok(JsonValue::Object(map)) = json::parse(raw) else {
        return out;
    };
    for (k, v) in map {
        let Some(provider) = Provider::from_name(&k) else {
            continue;
        };
        let secret = v
            .as_str()
            .or_else(|| v.get("api_key").and_then(|val| val.as_str()))
            .or_else(|| v.get("key").and_then(|val| val.as_str()))
            .or_else(|| v.get("token").and_then(|val| val.as_str()));
        if let Some(s) = secret {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push((provider, String::from(trimmed)));
            }
        }
    }
    out
}

/// Returns `true` if `token` is a Claude Pro/Max OAuth access token (`sk-ant-oat01-*`).
pub fn is_claude_oauth_token(token: &str) -> bool {
    token.trim().starts_with("sk-ant-oat01-")
}

/// Parsed OAuth credential bundle (Claude Pro/Max or Codex / ChatGPT Plus & Pro).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_at: Option<u64>,
}

impl OAuthToken {
    /// Serializes the token to flat JSON suitable for persisting to `claude_oauth.json` or `codex_oauth.json`.
    pub fn to_json(&self) -> String {
        let mut entries = vec![(
            String::from("access_token"),
            JsonValue::str(self.access_token.as_str()),
        )];
        if let Some(ref rt) = self.refresh_token {
            entries.push((String::from("refresh_token"), JsonValue::str(rt.as_str())));
        }
        if let Some(ref aid) = self.account_id {
            entries.push((String::from("account_id"), JsonValue::str(aid.as_str())));
        }
        if let Some(exp) = self.expires_at {
            entries.push((String::from("expires_at"), JsonValue::uint(exp)));
        }
        json::encode(&JsonValue::object(entries))
    }
}

fn non_empty_trimmed(s: Option<&str>) -> Option<String> {
    let t = s?.trim();
    if t.is_empty() {
        None
    } else {
        Some(String::from(t))
    }
}

fn parse_u64_field(val: Option<&JsonValue>) -> Option<u64> {
    let v = val?;
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse::<u64>().ok();
    }
    None
}

/// Parses a Claude OAuth JSON document.
///
/// Accepts both flat `{ "access_token", "refresh_token", "expires_at" }` and
/// Claude Code's `{ "claudeAiOauth": { "accessToken", "refreshToken", "expiresAt" } }` format.
pub fn parse_claude_oauth_json(text: &str) -> Option<OAuthToken> {
    let root = json::parse(text).ok()?;
    let obj = root.get("claudeAiOauth").unwrap_or(&root);
    let access_token = non_empty_trimmed(
        obj.get("access_token")
            .or_else(|| obj.get("accessToken"))
            .and_then(JsonValue::as_str),
    )?;
    let refresh_token = non_empty_trimmed(
        obj.get("refresh_token")
            .or_else(|| obj.get("refreshToken"))
            .and_then(JsonValue::as_str),
    );
    let account_id = non_empty_trimmed(
        obj.get("account_id")
            .or_else(|| obj.get("accountId"))
            .and_then(JsonValue::as_str),
    );
    let expires_at = parse_u64_field(obj.get("expires_at").or_else(|| obj.get("expiresAt")));
    Some(OAuthToken {
        access_token,
        refresh_token,
        account_id,
        expires_at,
    })
}

/// Parses a Codex / ChatGPT OAuth JSON document.
///
/// Accepts both flat `{ "access_token", "refresh_token", "account_id" }` and
/// Codex CLI's `{ "tokens": { "access_token", "refresh_token", "account_id" } }` format.
pub fn parse_codex_oauth_json(text: &str) -> Option<OAuthToken> {
    let root = json::parse(text).ok()?;
    let obj = root.get("tokens").unwrap_or(&root);
    let access_token = non_empty_trimmed(
        obj.get("access_token")
            .or_else(|| obj.get("accessToken"))
            .and_then(JsonValue::as_str),
    )?;
    let refresh_token = non_empty_trimmed(
        obj.get("refresh_token")
            .or_else(|| obj.get("refreshToken"))
            .and_then(JsonValue::as_str),
    );
    let account_id = non_empty_trimmed(
        obj.get("account_id")
            .or_else(|| obj.get("accountId"))
            .or_else(|| root.get("account_id"))
            .or_else(|| root.get("accountId"))
            .and_then(JsonValue::as_str),
    );
    let expires_at = parse_u64_field(
        obj.get("expires_at")
            .or_else(|| obj.get("expiresAt"))
            .or_else(|| root.get("expires_at")),
    );
    Some(OAuthToken {
        access_token,
        refresh_token,
        account_id,
        expires_at,
    })
}

/// Parses a Google / Antigravity Cloud Code OAuth JSON document.
///
/// Extracts `access_token`, `refresh_token`, `expires_at`/`expiry_date`, and stores
/// `project_id`/`cloudaicompanionProject` in `account_id`.
pub fn parse_google_oauth_json(raw: &str) -> Option<OAuthToken> {
    let root = json::parse(raw).ok()?;
    let obj = root.get("tokens").unwrap_or(&root);
    let access_token = non_empty_trimmed(
        obj.get("access_token")
            .or_else(|| obj.get("accessToken"))
            .and_then(JsonValue::as_str),
    )?;
    let refresh_token = non_empty_trimmed(
        obj.get("refresh_token")
            .or_else(|| obj.get("refreshToken"))
            .and_then(JsonValue::as_str),
    );
    let account_id = non_empty_trimmed(
        obj.get("project_id")
            .or_else(|| obj.get("projectId"))
            .or_else(|| obj.get("cloudaicompanionProject"))
            .or_else(|| obj.get("account_id"))
            .or_else(|| root.get("project_id"))
            .or_else(|| root.get("projectId"))
            .or_else(|| root.get("cloudaicompanionProject"))
            .and_then(JsonValue::as_str),
    );
    let expires_at = parse_u64_field(
        obj.get("expires_at")
            .or_else(|| obj.get("expiresAt"))
            .or_else(|| obj.get("expiry_date"))
            .or_else(|| obj.get("expiryDate"))
            .or_else(|| root.get("expires_at"))
            .or_else(|| root.get("expiry_date")),
    );
    Some(OAuthToken {
        access_token,
        refresh_token,
        account_id,
        expires_at,
    })
}

/// OAuth token refresh HTTP POST request ready to send via `sex_app::net::request` or raw TLS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthRefreshRequest {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub body: String,
    pub http_bytes: Vec<u8>,
}

const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

fn build_oauth_refresh_request(
    host: &str,
    path: &str,
    refresh_token: &str,
    client_id: &str,
) -> OAuthRefreshRequest {
    let payload = JsonValue::object(vec![
        (String::from("grant_type"), JsonValue::str("refresh_token")),
        (
            String::from("refresh_token"),
            JsonValue::str(refresh_token.trim()),
        ),
        (String::from("client_id"), JsonValue::str(client_id)),
    ]);
    let body = json::encode(&payload);
    let mut http = String::with_capacity(body.len() + 256);
    let _ = write!(
        &mut http,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: SexOS-Tailor/0.1\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        path,
        host,
        body.len(),
        body
    );
    OAuthRefreshRequest {
        url: format!("https://{}{}", host, path),
        host: String::from(host),
        port: 443,
        path: String::from(path),
        body,
        http_bytes: http.into_bytes(),
    }
}

fn form_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(&mut out, "%{:02X}", b);
        }
    }
    out
}

/// Builds a `POST https://platform.claude.com/v1/oauth/token` refresh request.
pub fn build_claude_refresh_request(refresh_token: &str) -> OAuthRefreshRequest {
    build_oauth_refresh_request(
        "platform.claude.com",
        "/v1/oauth/token",
        refresh_token,
        CLAUDE_OAUTH_CLIENT_ID,
    )
}

/// Builds a `POST https://auth.openai.com/oauth/token` refresh request.
pub fn build_codex_refresh_request(refresh_token: &str) -> OAuthRefreshRequest {
    build_oauth_refresh_request(
        "auth.openai.com",
        "/oauth/token",
        refresh_token,
        CODEX_OAUTH_CLIENT_ID,
    )
}

/// Builds a form-encoded `POST https://oauth2.googleapis.com/token` refresh request.
pub fn build_google_refresh_request(
    refresh_token: &str,
    client_id: &str,
    client_secret: &str,
) -> OAuthRefreshRequest {
    let host = "oauth2.googleapis.com";
    let path = "/token";
    let body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}&client_secret={}",
        form_urlencode(refresh_token.trim()),
        form_urlencode(client_id.trim()),
        form_urlencode(client_secret.trim()),
    );
    let mut http = String::with_capacity(body.len() + 256);
    let _ = write!(
        &mut http,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: SexOS-Tailor/0.1\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        path,
        host,
        body.len(),
        body
    );
    OAuthRefreshRequest {
        url: format!("https://{}{}", host, path),
        host: String::from(host),
        port: 443,
        path: String::from(path),
        body,
        http_bytes: http.into_bytes(),
    }
}

/// Complete HTTP/1.1 request ready to send over a `sex_app::net::Duplex` connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub path: String,
    pub http_bytes: Vec<u8>,
}

impl ProviderRequest {
    /// Inserts or appends an HTTP header before the request body.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() {
            return self;
        }
        if let Some(sep) = find_double_crlf(&self.http_bytes) {
            let mut out = Vec::with_capacity(self.http_bytes.len() + name.len() + value.len() + 6);
            out.extend_from_slice(&self.http_bytes[..sep]);
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(value.as_bytes());
            out.extend_from_slice(&self.http_bytes[sep..]);
            self.http_bytes = out;
        }
        self
    }

    /// Attaches the `ChatGPT-Account-Id` header when using Codex / ChatGPT Plus & Pro OAuth.
    pub fn with_chatgpt_account_id(self, account_id: Option<&str>) -> Self {
        match account_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => self.with_header("ChatGPT-Account-Id", id),
            None => self,
        }
    }

    /// Rewrites this request's origin (`host`, `port`, `tls`, `Host` header, and optional base path
    /// prefix) to point at a custom proxy such as CLIProxyAPI-Rust (`http://127.0.0.1:8317`),
    /// preserving the provider API path (`/v1/messages`, `/v1/chat/completions`, `/v1beta/models/...`).
    ///
    /// If `proxy_key` is provided and no provider auth header is currently set, adds
    /// `x-api-key: <proxy_key>` (for Claude requests) or `Authorization: Bearer <proxy_key>`.
    pub fn with_proxy_base(mut self, base_url: &str, proxy_key: Option<&str>) -> Self {
        let Some((tls, host, port, base_prefix)) = parse_base_url(base_url) else {
            return self;
        };
        let default_port = if tls { 443 } else { 80 };
        let host_header = if port == default_port {
            host.clone()
        } else {
            format!("{}:{}", host, port)
        };
        let new_path = if base_prefix.is_empty() {
            self.path.clone()
        } else {
            format!("{}{}", base_prefix, self.path)
        };

        if let Some(sep) = find_double_crlf(&self.http_bytes) {
            if let Ok(head_str) = core::str::from_utf8(&self.http_bytes[..sep]) {
                let mut lines = head_str.split("\r\n");
                let first_line = lines.next().unwrap_or("");
                let method = first_line.split(' ').next().unwrap_or("POST");
                let mut rebuilt = String::with_capacity(self.http_bytes.len() + 128);
                let _ = write!(&mut rebuilt, "{} {} HTTP/1.1\r\n", method, new_path);

                let mut has_auth = false;
                let mut is_claude = self.path.contains("/v1/messages");
                let mut saw_host = false;

                for line in lines {
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        let k_trim = k.trim();
                        let v_trim = v.trim();
                        if k_trim.eq_ignore_ascii_case("host") {
                            let _ = write!(&mut rebuilt, "Host: {}\r\n", host_header);
                            saw_host = true;
                            continue;
                        }
                        if k_trim.eq_ignore_ascii_case("anthropic-version") {
                            is_claude = true;
                        }
                        if k_trim.eq_ignore_ascii_case("authorization")
                            || k_trim.eq_ignore_ascii_case("x-api-key")
                            || k_trim.eq_ignore_ascii_case("x-goog-api-key")
                        {
                            let nonempty_val = if k_trim.eq_ignore_ascii_case("authorization") {
                                v_trim
                                    .strip_prefix("Bearer")
                                    .map(|rest| !rest.trim().is_empty())
                                    .unwrap_or(!v_trim.is_empty())
                            } else {
                                !v_trim.is_empty()
                            };
                            if nonempty_val {
                                has_auth = true;
                            } else {
                                // Drop empty placeholder auth header.
                                continue;
                            }
                        }
                    }
                    rebuilt.push_str(line);
                    rebuilt.push_str("\r\n");
                }

                if !saw_host {
                    let _ = write!(&mut rebuilt, "Host: {}\r\n", host_header);
                }
                if !has_auth {
                    if let Some(pk) = proxy_key.map(str::trim).filter(|s| !s.is_empty()) {
                        if is_claude {
                            let _ = write!(&mut rebuilt, "x-api-key: {}\r\n", pk);
                        } else {
                            let _ = write!(&mut rebuilt, "Authorization: Bearer {}\r\n", pk);
                        }
                    }
                }
                rebuilt.push_str("\r\n");

                let mut bytes = rebuilt.into_bytes();
                bytes.extend_from_slice(&self.http_bytes[sep + 4..]);
                self.http_bytes = bytes;
            }
        }

        self.host = host;
        self.port = port;
        self.tls = tls;
        self.path = new_path;
        self
    }
}

fn find_double_crlf(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_base_url(raw: &str) -> Option<(bool, String, u16, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (tls, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (false, r)
    } else if trimmed.contains("://") {
        return None;
    } else {
        (false, trimmed)
    };
    let (authority, prefix_raw) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']')?;
        let host_part = &authority[..=end];
        let after = &authority[end + 1..];
        let port = if let Some(p_str) = after.strip_prefix(':') {
            p_str.parse::<u16>().ok()?
        } else if after.is_empty() {
            if tls { 443 } else { 80 }
        } else {
            return None;
        };
        (host_part, port)
    } else if let Some((h, p_str)) = authority.rsplit_once(':') {
        let port = p_str.parse::<u16>().ok()?;
        (h, port)
    } else {
        (authority, if tls { 443 } else { 80 })
    };
    if host.is_empty() || port == 0 {
        return None;
    }
    let prefix = prefix_raw.trim_end_matches('/');
    Some((tls, String::from(host), port, String::from(prefix)))
}

/// Builds a complete HTTP/1.1 POST request for the given provider, model, API key,
/// conversation history, and tool schemas.
pub fn build_http_request(
    provider: Provider,
    model: &str,
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> ProviderRequest {
    let key = api_key.trim();
    if provider == Provider::Antigravity {
        return build_antigravity_request(model, key, "", system_prompt, messages, tools);
    }
    let (path, body_json, extra_headers) = match provider {
        Provider::Gemini => {
            let path = format!("/v1beta/models/{}:streamGenerateContent?alt=sse", model);
            let body = build_gemini_body(system_prompt, messages, tools);
            let headers = if key.is_empty() {
                String::new()
            } else {
                format!("x-goog-api-key: {}\r\n", key)
            };
            (path, body, headers)
        }
        Provider::Antigravity => unreachable!(),
        Provider::Claude | Provider::ClaudeDeepSeek => {
            let is_oauth = provider == Provider::Claude && is_claude_oauth_token(key);
            let path = if provider == Provider::ClaudeDeepSeek {
                String::from("/anthropic/v1/messages")
            } else if is_oauth {
                String::from("/v1/messages?beta=true")
            } else {
                String::from("/v1/messages")
            };
            let body = build_claude_body(model, system_prompt, messages, tools);
            let headers = if key.is_empty() {
                String::from("anthropic-version: 2023-06-01\r\n")
            } else if is_oauth {
                format!(
                    "Authorization: Bearer {}\r\n\
                     anthropic-version: 2023-06-01\r\n\
                     anthropic-beta: claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14\r\n\
                     x-app: cli\r\n\
                     anthropic-dangerous-direct-browser-access: true\r\n",
                    key
                )
            } else {
                format!("x-api-key: {}\r\nanthropic-version: 2023-06-01\r\n", key)
            };
            (path, body, headers)
        }
        Provider::Codex => {
            let path = String::from("/v1/chat/completions");
            let body = build_codex_body(model, system_prompt, messages, tools);
            let headers = if key.is_empty() {
                String::new()
            } else {
                format!("Authorization: Bearer {}\r\n", key)
            };
            (path, body, headers)
        }
    };

    let body_str = json::encode(&body_json);
    let mut http = String::with_capacity(body_str.len() + 256);
    let _ = write!(
        &mut http,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: SexOS-Tailor/0.1\r\n\
         Content-Type: application/json\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         {}\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        path,
        provider.host(),
        extra_headers,
        body_str.len(),
        body_str
    );

    ProviderRequest {
        host: String::from(provider.host()),
        port: provider.port(),
        tls: true,
        path,
        http_bytes: http.into_bytes(),
    }
}

/// Builds a `POST https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse`
/// request for Google AI / Antigravity Cloud Code OAuth.
pub fn build_antigravity_request(
    model: &str,
    access_token: &str,
    project_id: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> ProviderRequest {
    let host = Provider::Antigravity.host();
    let path = String::from("/v1internal:streamGenerateContent?alt=sse");
    let inner = build_gemini_body(system_prompt, messages, tools);
    let envelope = JsonValue::object(vec![
        (String::from("model"), JsonValue::str(model)),
        (String::from("project"), JsonValue::str(project_id)),
        (String::from("userAgent"), JsonValue::str("antigravity")),
        (String::from("requestType"), JsonValue::str("agent")),
        (String::from("request"), inner),
    ]);
    let body_str = json::encode(&envelope);
    let token = access_token.trim();
    let extra_headers = if token.is_empty() {
        String::new()
    } else {
        format!("Authorization: Bearer {}\r\n", token)
    };
    let mut http = String::with_capacity(body_str.len() + 256);
    let _ = write!(
        &mut http,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: antigravity/hub/2.22.0 darwin/arm64\r\n\
         Content-Type: application/json\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         {}\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        path,
        host,
        extra_headers,
        body_str.len(),
        body_str
    );
    ProviderRequest {
        host: String::from(host),
        port: 443,
        tls: true,
        path,
        http_bytes: http.into_bytes(),
    }
}

/// Builds a `POST https://chatgpt.com/backend-api/codex/responses` request for
/// ChatGPT Plus/Pro OAuth using the OpenAI Responses API wire format.
pub fn build_codex_responses_request(
    model: &str,
    access_token: &str,
    account_id: Option<&str>,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> ProviderRequest {
    let host = "chatgpt.com";
    let path = String::from("/backend-api/codex/responses");

    let mut input_items = Vec::new();
    for msg in messages {
        match msg.role {
            Role::User => {
                input_items.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (
                        String::from("content"),
                        JsonValue::array(vec![JsonValue::object(vec![
                            (String::from("type"), JsonValue::str("input_text")),
                            (String::from("text"), JsonValue::str(msg.text.as_str())),
                        ])]),
                    ),
                ]));
            }
            Role::Assistant => {
                if !msg.text.is_empty() {
                    input_items.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("assistant")),
                        (
                            String::from("content"),
                            JsonValue::array(vec![JsonValue::object(vec![
                                (String::from("type"), JsonValue::str("output_text")),
                                (String::from("text"), JsonValue::str(msg.text.as_str())),
                            ])]),
                        ),
                    ]));
                }
                for tc in &msg.tool_calls {
                    input_items.push(JsonValue::object(vec![
                        (String::from("type"), JsonValue::str("function_call")),
                        (String::from("call_id"), JsonValue::str(tc.id.as_str())),
                        (String::from("name"), JsonValue::str(tc.name.as_str())),
                        (
                            String::from("arguments"),
                            JsonValue::str(json::encode(&tc.arguments)),
                        ),
                    ]));
                }
            }
            Role::Tool => {
                if let Some(ref res) = msg.tool_result {
                    input_items.push(JsonValue::object(vec![
                        (String::from("type"), JsonValue::str("function_call_output")),
                        (String::from("call_id"), JsonValue::str(res.call_id.as_str())),
                        (String::from("output"), JsonValue::str(res.output.as_str())),
                    ]));
                }
            }
        }
    }

    let mut root = vec![
        (String::from("model"), JsonValue::str(model)),
        (String::from("store"), JsonValue::Bool(false)),
        (String::from("stream"), JsonValue::Bool(true)),
        (String::from("instructions"), JsonValue::str(system_prompt)),
        (String::from("input"), JsonValue::array(input_items)),
    ];

    if !tools.is_empty() {
        let tool_defs = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("type"), JsonValue::str("function")),
                    (String::from("name"), JsonValue::str(t.name.as_str())),
                    (
                        String::from("description"),
                        JsonValue::str(t.description.as_str()),
                    ),
                    (String::from("parameters"), t.parameters_schema.clone()),
                ])
            })
            .collect();
        root.push((String::from("tools"), JsonValue::array(tool_defs)));
    }

    let body_str = json::encode(&JsonValue::object(root));
    let mut extra_headers = String::from("originator: codex-tui\r\n");
    let token = access_token.trim();
    if !token.is_empty() {
        let _ = write!(&mut extra_headers, "Authorization: Bearer {}\r\n", token);
    }
    if let Some(aid) = account_id.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = write!(&mut extra_headers, "chatgpt-account-id: {}\r\n", aid);
    }

    let mut http = String::with_capacity(body_str.len() + 256);
    let _ = write!(
        &mut http,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: codex-tui/0.169.0 (Mac OS; arm64)\r\n\
         Content-Type: application/json\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         {}\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        path,
        host,
        extra_headers,
        body_str.len(),
        body_str
    );

    ProviderRequest {
        host: String::from(host),
        port: 443,
        tls: true,
        path,
        http_bytes: http.into_bytes(),
    }
}

fn build_gemini_body(system_prompt: &str, messages: &[Message], tools: &[ToolSpec]) -> JsonValue {
    let mut root = Vec::new();
    if !system_prompt.is_empty() {
        root.push((
            String::from("systemInstruction"),
            JsonValue::object(vec![(
                String::from("parts"),
                JsonValue::array(vec![JsonValue::object(vec![(
                    String::from("text"),
                    JsonValue::str(system_prompt),
                )])]),
            )]),
        ));
    }

    let mut contents = Vec::new();
    for msg in messages {
        match msg.role {
            Role::User => {
                contents.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (
                        String::from("parts"),
                        JsonValue::array(vec![JsonValue::object(vec![(
                            String::from("text"),
                            JsonValue::str(msg.text.as_str()),
                        )])]),
                    ),
                ]));
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if !msg.text.is_empty() {
                    parts.push(JsonValue::object(vec![(
                        String::from("text"),
                        JsonValue::str(msg.text.as_str()),
                    )]));
                }
                for tc in &msg.tool_calls {
                    parts.push(JsonValue::object(vec![(
                        String::from("functionCall"),
                        JsonValue::object(vec![
                            (String::from("name"), JsonValue::str(tc.name.as_str())),
                            (String::from("args"), tc.arguments.clone()),
                        ]),
                    )]));
                }
                if !parts.is_empty() {
                    contents.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("model")),
                        (String::from("parts"), JsonValue::array(parts)),
                    ]));
                }
            }
            Role::Tool => {
                if let Some(ref res) = msg.tool_result {
                    contents.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("user")),
                        (
                            String::from("parts"),
                            JsonValue::array(vec![JsonValue::object(vec![(
                                String::from("functionResponse"),
                                JsonValue::object(vec![
                                    (String::from("name"), JsonValue::str(res.name.as_str())),
                                    (
                                        String::from("response"),
                                        JsonValue::object(vec![
                                            (
                                                String::from("output"),
                                                JsonValue::str(res.output.as_str()),
                                            ),
                                            (
                                                String::from("is_error"),
                                                JsonValue::Bool(res.is_error),
                                            ),
                                        ]),
                                    ),
                                ]),
                            )])]),
                        ),
                    ]));
                }
            }
        }
    }
    root.push((String::from("contents"), JsonValue::array(contents)));

    if !tools.is_empty() {
        let decls = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("name"), JsonValue::str(t.name.as_str())),
                    (
                        String::from("description"),
                        JsonValue::str(t.description.as_str()),
                    ),
                    (String::from("parameters"), t.parameters_schema.clone()),
                ])
            })
            .collect();
        root.push((
            String::from("tools"),
            JsonValue::array(vec![JsonValue::object(vec![(
                String::from("functionDeclarations"),
                JsonValue::array(decls),
            )])]),
        ));
    }

    JsonValue::object(root)
}

fn build_claude_body(
    model: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> JsonValue {
    let mut root = vec![
        (String::from("model"), JsonValue::str(model)),
        (String::from("max_tokens"), JsonValue::uint(4096)),
        (String::from("stream"), JsonValue::Bool(true)),
    ];
    if !system_prompt.is_empty() {
        root.push((String::from("system"), JsonValue::str(system_prompt)));
    }

    let mut out_msgs: Vec<JsonValue> = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let msg = &messages[i];
        match msg.role {
            Role::User => {
                out_msgs.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (String::from("content"), JsonValue::str(msg.text.as_str())),
                ]));
                i += 1;
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !msg.text.is_empty() {
                    blocks.push(JsonValue::object(vec![
                        (String::from("type"), JsonValue::str("text")),
                        (String::from("text"), JsonValue::str(msg.text.as_str())),
                    ]));
                }
                for tc in &msg.tool_calls {
                    blocks.push(JsonValue::object(vec![
                        (String::from("type"), JsonValue::str("tool_use")),
                        (String::from("id"), JsonValue::str(tc.id.as_str())),
                        (String::from("name"), JsonValue::str(tc.name.as_str())),
                        (String::from("input"), tc.arguments.clone()),
                    ]));
                }
                if !blocks.is_empty() {
                    out_msgs.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("assistant")),
                        (String::from("content"), JsonValue::array(blocks)),
                    ]));
                }
                i += 1;

                // Anthropic & DeepSeek require ALL tool_result blocks for an assistant
                // turn's tool_use calls to appear in ONE immediately-following user message.
                if !msg.tool_calls.is_empty() {
                    let mut result_blocks = Vec::new();
                    let mut answered_ids: Vec<String> = Vec::new();
                    while i < messages.len() && messages[i].role == Role::Tool {
                        if let Some(ref res) = messages[i].tool_result {
                            answered_ids.push(res.call_id.clone());
                            result_blocks.push(JsonValue::object(vec![
                                (String::from("type"), JsonValue::str("tool_result")),
                                (
                                    String::from("tool_use_id"),
                                    JsonValue::str(res.call_id.as_str()),
                                ),
                                (
                                    String::from("content"),
                                    JsonValue::str(res.output.as_str()),
                                ),
                                (String::from("is_error"), JsonValue::Bool(res.is_error)),
                            ]));
                        }
                        i += 1;
                    }
                    for tc in &msg.tool_calls {
                        if !answered_ids.iter().any(|id| id == &tc.id) {
                            result_blocks.push(JsonValue::object(vec![
                                (String::from("type"), JsonValue::str("tool_result")),
                                (String::from("tool_use_id"), JsonValue::str(tc.id.as_str())),
                                (
                                    String::from("content"),
                                    JsonValue::str("Tool execution skipped or interrupted."),
                                ),
                                (String::from("is_error"), JsonValue::Bool(true)),
                            ]));
                        }
                    }
                    if !result_blocks.is_empty() {
                        out_msgs.push(JsonValue::object(vec![
                            (String::from("role"), JsonValue::str("user")),
                            (String::from("content"), JsonValue::array(result_blocks)),
                        ]));
                    }
                }
            }
            Role::Tool => {
                // Standalone tool messages (if any) are grouped into a single user message.
                let mut result_blocks = Vec::new();
                while i < messages.len() && messages[i].role == Role::Tool {
                    if let Some(ref res) = messages[i].tool_result {
                        result_blocks.push(JsonValue::object(vec![
                            (String::from("type"), JsonValue::str("tool_result")),
                            (
                                String::from("tool_use_id"),
                                JsonValue::str(res.call_id.as_str()),
                            ),
                            (
                                String::from("content"),
                                JsonValue::str(res.output.as_str()),
                            ),
                            (String::from("is_error"), JsonValue::Bool(res.is_error)),
                        ]));
                    }
                    i += 1;
                }
                if !result_blocks.is_empty() {
                    out_msgs.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("user")),
                        (String::from("content"), JsonValue::array(result_blocks)),
                    ]));
                }
            }
        }
    }
    root.push((String::from("messages"), JsonValue::array(out_msgs)));

    if !tools.is_empty() {
        let tool_defs = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("name"), JsonValue::str(t.name.as_str())),
                    (
                        String::from("description"),
                        JsonValue::str(t.description.as_str()),
                    ),
                    (String::from("input_schema"), t.parameters_schema.clone()),
                ])
            })
            .collect();
        root.push((String::from("tools"), JsonValue::array(tool_defs)));
    }

    JsonValue::object(root)
}

fn build_codex_body(
    model: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> JsonValue {
    let mut root = vec![
        (String::from("model"), JsonValue::str(model)),
        (String::from("stream"), JsonValue::Bool(true)),
    ];

    let mut out_msgs = Vec::new();
    if !system_prompt.is_empty() {
        out_msgs.push(JsonValue::object(vec![
            (String::from("role"), JsonValue::str("system")),
            (String::from("content"), JsonValue::str(system_prompt)),
        ]));
    }
    for msg in messages {
        match msg.role {
            Role::User => {
                out_msgs.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (String::from("content"), JsonValue::str(msg.text.as_str())),
                ]));
            }
            Role::Assistant => {
                let mut entries = vec![(String::from("role"), JsonValue::str("assistant"))];
                if msg.text.is_empty() {
                    entries.push((String::from("content"), JsonValue::Null));
                } else {
                    entries.push((String::from("content"), JsonValue::str(msg.text.as_str())));
                }
                if !msg.tool_calls.is_empty() {
                    let calls = msg
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            JsonValue::object(vec![
                                (String::from("id"), JsonValue::str(tc.id.as_str())),
                                (String::from("type"), JsonValue::str("function")),
                                (
                                    String::from("function"),
                                    JsonValue::object(vec![
                                        (String::from("name"), JsonValue::str(tc.name.as_str())),
                                        (
                                            String::from("arguments"),
                                            JsonValue::str(json::encode(&tc.arguments)),
                                        ),
                                    ]),
                                ),
                            ])
                        })
                        .collect();
                    entries.push((String::from("tool_calls"), JsonValue::array(calls)));
                }
                out_msgs.push(JsonValue::object(entries));
            }
            Role::Tool => {
                if let Some(ref res) = msg.tool_result {
                    out_msgs.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("tool")),
                        (
                            String::from("tool_call_id"),
                            JsonValue::str(res.call_id.as_str()),
                        ),
                        (
                            String::from("content"),
                            JsonValue::str(res.output.as_str()),
                        ),
                    ]));
                }
            }
        }
    }
    root.push((String::from("messages"), JsonValue::array(out_msgs)));

    if !tools.is_empty() {
        let tool_defs = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("type"), JsonValue::str("function")),
                    (
                        String::from("function"),
                        JsonValue::object(vec![
                            (String::from("name"), JsonValue::str(t.name.as_str())),
                            (
                                String::from("description"),
                                JsonValue::str(t.description.as_str()),
                            ),
                            (String::from("parameters"), t.parameters_schema.clone()),
                        ]),
                    ),
                ])
            })
            .collect();
        root.push((String::from("tools"), JsonValue::array(tool_defs)));
    }

    JsonValue::object(root)
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args_json: String,
}

/// Stateful incremental HTTP + SSE + provider JSON parser.
pub struct ProviderStreamParser {
    provider: Provider,
    decoder: HttpSseDecoder,
    next_call_seq: u32,
    stop_reason: Option<String>,
    emitted_done: bool,
    partial_tools: BTreeMap<usize, PartialToolCall>,
}

impl ProviderStreamParser {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            decoder: HttpSseDecoder::new(),
            next_call_seq: 1,
            stop_reason: None,
            emitted_done: false,
            partial_tools: BTreeMap::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.emitted_done || self.decoder.is_complete()
    }

    pub fn http_status(&self) -> Option<u16> {
        self.decoder.status()
    }

    /// Feeds raw HTTP response bytes and returns decoded provider-agnostic [`StreamEvent`]s.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        match self.decoder.feed(bytes) {
            Ok(messages) => {
                for msg in messages {
                    self.handle_sse(&msg, &mut events);
                }
            }
            Err(err) => {
                events.push(StreamEvent::Error(format_sse_error(err)));
                self.emitted_done = true;
            }
        }
        events
    }

    /// Signals EOF from the underlying transport, flushing any buffered tool calls or Done event.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        match self.decoder.finish() {
            Ok(messages) => {
                for msg in messages {
                    self.handle_sse(&msg, &mut events);
                }
            }
            Err(err) => {
                events.push(StreamEvent::Error(format_sse_error(err)));
                self.emitted_done = true;
                return events;
            }
        }
        self.flush_partial_tools(&mut events);
        if !self.emitted_done {
            self.emitted_done = true;
            events.push(StreamEvent::Done {
                stop_reason: self.stop_reason.take(),
            });
        }
        events
    }

    fn handle_sse(&mut self, msg: &SseMessage, out: &mut Vec<StreamEvent>) {
        let data = msg.data.trim();
        if data.is_empty() {
            return;
        }
        if data == "[DONE]" {
            self.flush_partial_tools(out);
            if !self.emitted_done {
                self.emitted_done = true;
                out.push(StreamEvent::Done {
                    stop_reason: self.stop_reason.take(),
                });
            }
            return;
        }
        let val = match json::parse(data) {
            Ok(v) => v,
            Err(_) => return,
        };
        match self.provider {
            Provider::Gemini | Provider::Antigravity => self.parse_gemini_chunk(&val, out),
            Provider::Claude | Provider::ClaudeDeepSeek => {
                self.parse_claude_chunk(msg.event.as_deref(), &val, out)
            }
            Provider::Codex => self.parse_codex_chunk(msg.event.as_deref(), &val, out),
        }
    }

    fn parse_gemini_chunk(&mut self, val: &JsonValue, out: &mut Vec<StreamEvent>) {
        if let Some(err_msg) = val.get_path(&["error", "message"]).and_then(JsonValue::as_str) {
            out.push(StreamEvent::Error(String::from(err_msg)));
            return;
        }
        // Automatically unwrap Antigravity Cloud Code `{"response": { ... }}` envelope when present.
        let inner = val.get("response").unwrap_or(val);
        if let Some(err_msg) = inner
            .get_path(&["error", "message"])
            .and_then(JsonValue::as_str)
        {
            out.push(StreamEvent::Error(String::from(err_msg)));
            return;
        }
        let Some(cand) = inner.get("candidates").and_then(|c| c.index(0)) else {
            return;
        };
        if let Some(parts) = cand
            .get_path(&["content", "parts"])
            .and_then(JsonValue::as_array)
        {
            for part in parts {
                if let Some(text) = part.get("text").and_then(JsonValue::as_str) {
                    if part.get("thought").and_then(JsonValue::as_bool) == Some(true) {
                        out.push(StreamEvent::ThinkingDelta(String::from(text)));
                    } else if !text.is_empty() {
                        out.push(StreamEvent::TextDelta(String::from(text)));
                    }
                }
                if let Some(fc) = part.get("functionCall") {
                    if let Some(name) = fc.get("name").and_then(JsonValue::as_str) {
                        let id = format!("gemini-call-{}", self.next_call_seq);
                        self.next_call_seq += 1;
                        let args = fc
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| JsonValue::object(Vec::new()));
                        out.push(StreamEvent::ToolCall {
                            id,
                            name: String::from(name),
                            arguments: args,
                        });
                    }
                }
            }
        }
        if let Some(reason) = cand.get("finishReason").and_then(JsonValue::as_str) {
            if !reason.is_empty() {
                self.stop_reason = Some(String::from(reason));
                if !self.emitted_done {
                    self.emitted_done = true;
                    out.push(StreamEvent::Done {
                        stop_reason: self.stop_reason.take(),
                    });
                }
            }
        }
    }

    fn parse_claude_chunk(
        &mut self,
        event_name: Option<&str>,
        val: &JsonValue,
        out: &mut Vec<StreamEvent>,
    ) {
        let kind = val
            .get("type")
            .and_then(JsonValue::as_str)
            .or(event_name)
            .unwrap_or("");
        match kind {
            "content_block_start" => {
                let idx = val
                    .get("index")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(block) = val.get("content_block") {
                    if block.get("type").and_then(JsonValue::as_str) == Some("tool_use") {
                        let id = block
                            .get("id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("claude-call");
                        let name = block
                            .get("name")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("");
                        self.partial_tools.insert(
                            idx,
                            PartialToolCall {
                                id: String::from(id),
                                name: String::from(name),
                                args_json: String::new(),
                            },
                        );
                    }
                }
            }
            "content_block_delta" => {
                let idx = val
                    .get("index")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
                let Some(delta) = val.get("delta") else {
                    return;
                };
                match delta.get("type").and_then(JsonValue::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(t) = delta.get("text").and_then(JsonValue::as_str) {
                            if !t.is_empty() {
                                out.push(StreamEvent::TextDelta(String::from(t)));
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(t) = delta.get("thinking").and_then(JsonValue::as_str) {
                            if !t.is_empty() {
                                out.push(StreamEvent::ThinkingDelta(String::from(t)));
                            }
                        }
                    }
                    "input_json_delta" => {
                        if let Some(part) = delta.get("partial_json").and_then(JsonValue::as_str) {
                            if let Some(entry) = self.partial_tools.get_mut(&idx) {
                                entry.args_json.push_str(part);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let idx = val
                    .get("index")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(tool) = self.partial_tools.remove(&idx) {
                    emit_partial_tool(tool, out);
                }
            }
            "message_delta" => {
                if let Some(reason) = val
                    .get_path(&["delta", "stop_reason"])
                    .and_then(JsonValue::as_str)
                {
                    self.stop_reason = Some(String::from(reason));
                }
            }
            "message_stop" => {
                self.flush_partial_tools(out);
                if !self.emitted_done {
                    self.emitted_done = true;
                    out.push(StreamEvent::Done {
                        stop_reason: self.stop_reason.take(),
                    });
                }
            }
            "error" => {
                let msg = val
                    .get_path(&["error", "message"])
                    .and_then(JsonValue::as_str)
                    .unwrap_or("Claude API error");
                out.push(StreamEvent::Error(String::from(msg)));
            }
            _ => {}
        }
    }

    fn parse_codex_chunk(
        &mut self,
        event_name: Option<&str>,
        val: &JsonValue,
        out: &mut Vec<StreamEvent>,
    ) {
        if let Some(err_msg) = val.get_path(&["error", "message"]).and_then(JsonValue::as_str) {
            out.push(StreamEvent::Error(String::from(err_msg)));
            return;
        }

        // OpenAI Responses API SSE events (ChatGPT Plus/Pro Codex backend).
        if let Some(kind) = val.get("type").and_then(JsonValue::as_str).or(event_name) {
            match kind {
                "response.output_text.delta" => {
                    if let Some(delta) = val.get("delta").and_then(JsonValue::as_str) {
                        if !delta.is_empty() {
                            out.push(StreamEvent::TextDelta(String::from(delta)));
                        }
                    }
                    return;
                }
                "response.reasoning_summary_text.delta" => {
                    if let Some(delta) = val.get("delta").and_then(JsonValue::as_str) {
                        if !delta.is_empty() {
                            out.push(StreamEvent::ThinkingDelta(String::from(delta)));
                        }
                    }
                    return;
                }
                "response.output_item.added" => {
                    let idx = val
                        .get("output_index")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(0) as usize;
                    if let Some(item) = val.get("item") {
                        if item.get("type").and_then(JsonValue::as_str) == Some("function_call") {
                            let id = item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(JsonValue::as_str)
                                .unwrap_or("codex-call");
                            let name = item
                                .get("name")
                                .and_then(JsonValue::as_str)
                                .unwrap_or("");
                            let args = item
                                .get("arguments")
                                .and_then(JsonValue::as_str)
                                .unwrap_or("");
                            self.partial_tools.insert(
                                idx,
                                PartialToolCall {
                                    id: String::from(id),
                                    name: String::from(name),
                                    args_json: String::from(args),
                                },
                            );
                        }
                    }
                    return;
                }
                "response.function_call_arguments.delta" => {
                    let idx = val
                        .get("output_index")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(0) as usize;
                    if let Some(delta) = val.get("delta").and_then(JsonValue::as_str) {
                        let entry = self.partial_tools.entry(idx).or_default();
                        entry.args_json.push_str(delta);
                    }
                    return;
                }
                "response.output_item.done" => {
                    let idx = val
                        .get("output_index")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(0) as usize;
                    if let Some(item) = val.get("item") {
                        if item.get("type").and_then(JsonValue::as_str) == Some("function_call") {
                            let mut tool = self.partial_tools.remove(&idx).unwrap_or_default();
                            if tool.id.is_empty() {
                                if let Some(id) = item
                                    .get("call_id")
                                    .or_else(|| item.get("id"))
                                    .and_then(JsonValue::as_str)
                                {
                                    tool.id = String::from(id);
                                }
                            }
                            if tool.name.is_empty() {
                                if let Some(name) = item.get("name").and_then(JsonValue::as_str) {
                                    tool.name = String::from(name);
                                }
                            }
                            if tool.args_json.trim().is_empty() {
                                if let Some(args) = item.get("arguments").and_then(JsonValue::as_str)
                                {
                                    tool.args_json = String::from(args);
                                }
                            }
                            emit_partial_tool(tool, out);
                        }
                    }
                    return;
                }
                "response.completed" => {
                    self.flush_partial_tools(out);
                    if let Some(status) = val
                        .get_path(&["response", "status"])
                        .and_then(JsonValue::as_str)
                    {
                        self.stop_reason = Some(String::from(status));
                    }
                    if !self.emitted_done {
                        self.emitted_done = true;
                        out.push(StreamEvent::Done {
                            stop_reason: self.stop_reason.take(),
                        });
                    }
                    return;
                }
                "error" => {
                    let msg = val
                        .get("message")
                        .and_then(JsonValue::as_str)
                        .or_else(|| val.get_path(&["error", "message"]).and_then(JsonValue::as_str))
                        .unwrap_or("Codex API error");
                    out.push(StreamEvent::Error(String::from(msg)));
                    return;
                }
                _ => {}
            }
        }

        // Standard OpenAI Chat Completions SSE format.
        let Some(choice) = val.get("choices").and_then(|c| c.index(0)) else {
            return;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(JsonValue::as_str) {
                if !text.is_empty() {
                    out.push(StreamEvent::TextDelta(String::from(text)));
                }
            }
            if let Some(thought) = delta.get("reasoning_content").and_then(JsonValue::as_str) {
                if !thought.is_empty() {
                    out.push(StreamEvent::ThinkingDelta(String::from(thought)));
                }
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(JsonValue::as_array) {
                for tc in tcs {
                    let idx = tc
                        .get("index")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(0) as usize;
                    let entry = self.partial_tools.entry(idx).or_default();
                    if let Some(id) = tc.get("id").and_then(JsonValue::as_str) {
                        if !id.is_empty() {
                            entry.id = String::from(id);
                        }
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(JsonValue::as_str) {
                            if !name.is_empty() {
                                entry.name.push_str(name);
                            }
                        }
                        if let Some(args) = func.get("arguments").and_then(JsonValue::as_str) {
                            entry.args_json.push_str(args);
                        }
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(JsonValue::as_str) {
            if !reason.is_empty() {
                self.stop_reason = Some(String::from(reason));
                self.flush_partial_tools(out);
            }
        }
    }

    fn flush_partial_tools(&mut self, out: &mut Vec<StreamEvent>) {
        let pending = core::mem::take(&mut self.partial_tools);
        for (_, tool) in pending {
            emit_partial_tool(tool, out);
        }
    }
}

fn emit_partial_tool(tool: PartialToolCall, out: &mut Vec<StreamEvent>) {
    if tool.name.is_empty() {
        return;
    }
    let arguments = if tool.args_json.trim().is_empty() {
        JsonValue::object(Vec::new())
    } else {
        json::parse(&tool.args_json).unwrap_or_else(|_| JsonValue::object(Vec::new()))
    };
    out.push(StreamEvent::ToolCall {
        id: if tool.id.is_empty() {
            String::from("call")
        } else {
            tool.id
        },
        name: tool.name,
        arguments,
    });
}

fn format_sse_error(err: SseError) -> String {
    match err {
        SseError::MalformedHeaders => String::from("Malformed HTTP response headers"),
        SseError::MalformedChunkSize => String::from("Malformed HTTP chunk size"),
        SseError::HeaderTooLarge => String::from("HTTP response headers too large"),
        SseError::HttpStatus { status, body } => {
            if body.is_empty() {
                format!("HTTP {}", status)
            } else {
                format!("HTTP {}: {}", status, body.trim())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::builtin_tools;

    fn chunked_response(payload: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            payload.len(),
            payload
        )
        .into_bytes()
    }

    #[test]
    fn builds_provider_http_requests_with_auth_and_tools() {
        let msgs = vec![Message::user("List files")];
        let tools = builtin_tools();

        let gem = build_http_request(
            Provider::Gemini,
            "gemini-2.5-flash",
            "GKEY",
            "You are Tailor.",
            &msgs,
            &tools,
        );
        let gem_txt = core::str::from_utf8(&gem.http_bytes).unwrap();
        assert!(gem_txt.starts_with("POST /v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse HTTP/1.1\r\n"));
        assert!(gem_txt.contains("x-goog-api-key: GKEY\r\n"));
        assert!(gem_txt.contains("\"functionDeclarations\""));

        let cla = build_http_request(
            Provider::Claude,
            "claude-sonnet-4-20250514",
            "CKEY",
            "You are Tailor.",
            &msgs,
            &tools,
        );
        let cla_txt = core::str::from_utf8(&cla.http_bytes).unwrap();
        assert!(cla_txt.starts_with("POST /v1/messages HTTP/1.1\r\n"));
        assert!(cla_txt.contains("x-api-key: CKEY\r\n"));
        assert!(cla_txt.contains("anthropic-version: 2023-06-01\r\n"));
        assert!(cla_txt.contains("\"input_schema\""));

        let cod = build_http_request(
            Provider::Codex,
            "gpt-4.1",
            "OKEY",
            "You are Tailor.",
            &msgs,
            &tools,
        );
        let cod_txt = core::str::from_utf8(&cod.http_bytes).unwrap();
        assert!(cod_txt.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        assert!(cod_txt.contains("Authorization: Bearer OKEY\r\n"));
        assert!(cod_txt.contains("\"stream\":true"));
    }

    #[test]
    fn parses_gemini_sse_thinking_text_and_tool_call() {
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"thought\":true,\"text\":\"Checking...\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Reading main.rs\"},{\"functionCall\":{\"name\":\"read_file\",\"args\":{\"path\":\"src/main.rs\"}}}]},\"finishReason\":\"STOP\"}]}\n\n"
        );
        let wire = chunked_response(sse);
        let mut p = ProviderStreamParser::new(Provider::Gemini);
        let mut events = p.feed(&wire);
        events.extend(p.finish());
        assert_eq!(events.len(), 4);
        assert_eq!(events[0], StreamEvent::ThinkingDelta(String::from("Checking...")));
        assert_eq!(events[1], StreamEvent::TextDelta(String::from("Reading main.rs")));
        match &events[2] {
            StreamEvent::ToolCall { name, arguments, .. } => {
                assert_eq!(name, "read_file");
                assert_eq!(
                    arguments.get("path").and_then(JsonValue::as_str),
                    Some("src/main.rs")
                );
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[3],
            StreamEvent::Done {
                stop_reason: Some(String::from("STOP"))
            }
        );
    }

    #[test]
    fn parses_claude_sse_streamed_tool_use() {
        let sse = concat!(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Let me patch it.\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"patch_file\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\",\" }}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"old_text\\\":\\\"x\\\",\\\"new_text\\\":\\\"y\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let wire = chunked_response(sse);
        let mut p = ProviderStreamParser::new(Provider::Claude);
        let mut events = Vec::new();
        for chunk in wire.chunks(7) {
            events.extend(p.feed(chunk));
        }
        events.extend(p.finish());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], StreamEvent::TextDelta(String::from("Let me patch it.")));
        match &events[1] {
            StreamEvent::ToolCall { id, name, arguments } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "patch_file");
                assert_eq!(arguments.get("new_text").and_then(JsonValue::as_str), Some("y"));
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[2],
            StreamEvent::Done {
                stop_reason: Some(String::from("tool_use"))
            }
        );
    }

    #[test]
    fn parses_codex_sse_streamed_tool_calls() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Listing dir\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\".\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let wire = chunked_response(sse);
        let mut p = ProviderStreamParser::new(Provider::Codex);
        let mut events = p.feed(&wire);
        events.extend(p.finish());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], StreamEvent::TextDelta(String::from("Listing dir")));
        match &events[1] {
            StreamEvent::ToolCall { id, name, arguments } => {
                assert_eq!(id, "call_9");
                assert_eq!(name, "list_dir");
                assert_eq!(arguments.get("path").and_then(JsonValue::as_str), Some("."));
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[2],
            StreamEvent::Done {
                stop_reason: Some(String::from("tool_calls"))
            }
        );
    }

    #[test]
    fn claude_oauth_token_uses_bearer_and_beta_header() {
        let msgs = vec![Message::user("Hello")];
        let oauth_req = build_http_request(
            Provider::Claude,
            "claude-sonnet-4-20250514",
            "sk-ant-oat01-example-token",
            "System",
            &msgs,
            &[],
        );
        let oauth_txt = core::str::from_utf8(&oauth_req.http_bytes).unwrap();
        assert_eq!(oauth_req.path, "/v1/messages?beta=true");
        assert!(oauth_txt.starts_with("POST /v1/messages?beta=true HTTP/1.1\r\n"));
        assert!(oauth_txt.contains("Authorization: Bearer sk-ant-oat01-example-token\r\n"));
        assert!(oauth_txt.contains(
            "anthropic-beta: claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14\r\n"
        ));
        assert!(oauth_txt.contains("x-app: cli\r\n"));
        assert!(oauth_txt.contains("anthropic-dangerous-direct-browser-access: true\r\n"));
        assert!(oauth_txt.contains("anthropic-version: 2023-06-01\r\n"));
        assert!(!oauth_txt.contains("x-api-key:"));

        let api_req = build_http_request(
            Provider::Claude,
            "claude-sonnet-4-20250514",
            "sk-ant-api03-example-key",
            "System",
            &msgs,
            &[],
        );
        let api_txt = core::str::from_utf8(&api_req.http_bytes).unwrap();
        assert_eq!(api_req.path, "/v1/messages");
        assert!(api_txt.contains("x-api-key: sk-ant-api03-example-key\r\n"));
        assert!(!api_txt.contains("Authorization: Bearer"));
        assert!(!api_txt.contains("anthropic-beta:"));
    }

    #[test]
    fn codex_chatgpt_account_id_header_is_attached() {
        let msgs = vec![Message::user("Hi")];
        let req = build_http_request(Provider::Codex, "gpt-4.1", "tok-123", "", &msgs, &[])
            .with_chatgpt_account_id(Some("acct_abc999"));
        let txt = core::str::from_utf8(&req.http_bytes).unwrap();
        assert!(txt.contains("Authorization: Bearer tok-123\r\n"));
        assert!(txt.contains("ChatGPT-Account-Id: acct_abc999\r\n"));
    }

    #[test]
    fn with_proxy_base_rewrites_all_providers_and_preserves_paths() {
        let msgs = vec![Message::user("Ping")];

        let claude = build_http_request(
            Provider::Claude,
            "claude-sonnet-4-20250514",
            "",
            "",
            &msgs,
            &[],
        )
        .with_proxy_base("http://127.0.0.1:8317", Some("proxy-secret"));
        assert_eq!(claude.host, "127.0.0.1");
        assert_eq!(claude.port, 8317);
        assert!(!claude.tls);
        assert_eq!(claude.path, "/v1/messages");
        let claude_txt = core::str::from_utf8(&claude.http_bytes).unwrap();
        assert!(claude_txt.starts_with("POST /v1/messages HTTP/1.1\r\n"));
        assert!(claude_txt.contains("Host: 127.0.0.1:8317\r\n"));
        assert!(claude_txt.contains("x-api-key: proxy-secret\r\n"));

        let codex = build_http_request(Provider::Codex, "gpt-4.1", "", "", &msgs, &[])
            .with_proxy_base("http://127.0.0.1:8317", Some("proxy-secret"));
        assert_eq!(codex.host, "127.0.0.1");
        assert_eq!(codex.port, 8317);
        assert!(!codex.tls);
        assert_eq!(codex.path, "/v1/chat/completions");
        let codex_txt = core::str::from_utf8(&codex.http_bytes).unwrap();
        assert!(codex_txt.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        assert!(codex_txt.contains("Host: 127.0.0.1:8317\r\n"));
        assert!(codex_txt.contains("Authorization: Bearer proxy-secret\r\n"));

        let gemini = build_http_request(
            Provider::Gemini,
            "gemini-2.5-flash",
            "",
            "",
            &msgs,
            &[],
        )
        .with_proxy_base("http://127.0.0.1:8317", Some("proxy-secret"));
        assert_eq!(gemini.host, "127.0.0.1");
        assert_eq!(gemini.port, 8317);
        assert!(!gemini.tls);
        assert_eq!(
            gemini.path,
            "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        let gemini_txt = core::str::from_utf8(&gemini.http_bytes).unwrap();
        assert!(gemini_txt.starts_with(
            "POST /v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse HTTP/1.1\r\n"
        ));
        assert!(gemini_txt.contains("Host: 127.0.0.1:8317\r\n"));
        assert!(gemini_txt.contains("Authorization: Bearer proxy-secret\r\n"));

        let deepseek = build_http_request(
            Provider::ClaudeDeepSeek,
            "deepseek-chat",
            "",
            "",
            &msgs,
            &[],
        )
        .with_proxy_base("https://proxy.example.test", Some("proxy-secret"));
        assert_eq!(deepseek.host, "proxy.example.test");
        assert_eq!(deepseek.port, 443);
        assert!(deepseek.tls);
        assert_eq!(deepseek.path, "/anthropic/v1/messages");
        let ds_txt = core::str::from_utf8(&deepseek.http_bytes).unwrap();
        assert!(ds_txt.contains("Host: proxy.example.test\r\n"));
        assert!(ds_txt.contains("x-api-key: proxy-secret\r\n"));

        // Existing provider key must NOT be overwritten by proxy_key.
        let keep_key = build_http_request(
            Provider::Claude,
            "claude-sonnet-4-20250514",
            "sk-ant-api03-existing",
            "",
            &msgs,
            &[],
        )
        .with_proxy_base("http://127.0.0.1:8317", Some("proxy-secret"));
        let keep_txt = core::str::from_utf8(&keep_key.http_bytes).unwrap();
        assert!(keep_txt.contains("x-api-key: sk-ant-api03-existing\r\n"));
        assert!(!keep_txt.contains("proxy-secret"));
    }

    #[test]
    fn parses_claude_and_codex_oauth_json_formats() {
        let claude_flat = r#"{"access_token":"sk-ant-oat01-flat","refresh_token":"sk-ant-ort01-ref","expires_at":1790000000}"#;
        let t1 = parse_claude_oauth_json(claude_flat).unwrap();
        assert_eq!(t1.access_token, "sk-ant-oat01-flat");
        assert_eq!(t1.refresh_token.as_deref(), Some("sk-ant-ort01-ref"));
        assert_eq!(t1.expires_at, Some(1790000000));

        let claude_nested = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-nested","refreshToken":"sk-ant-ort01-nref","expiresAt":1790001234}}"#;
        let t2 = parse_claude_oauth_json(claude_nested).unwrap();
        assert_eq!(t2.access_token, "sk-ant-oat01-nested");
        assert_eq!(t2.refresh_token.as_deref(), Some("sk-ant-ort01-nref"));
        assert_eq!(t2.expires_at, Some(1790001234));

        // Round-trip serialization
        let serialized = t2.to_json();
        let t2_rt = parse_claude_oauth_json(&serialized).unwrap();
        assert_eq!(t2_rt, t2);

        let codex_flat = r#"{"access_token":"eyJ-flat","refresh_token":"rt-flat","account_id":"acct_1"}"#;
        let c1 = parse_codex_oauth_json(codex_flat).unwrap();
        assert_eq!(c1.access_token, "eyJ-flat");
        assert_eq!(c1.refresh_token.as_deref(), Some("rt-flat"));
        assert_eq!(c1.account_id.as_deref(), Some("acct_1"));

        let codex_nested = r#"{"tokens":{"access_token":"eyJ-nested","refresh_token":"rt-nested","account_id":"acct_2"}}"#;
        let c2 = parse_codex_oauth_json(codex_nested).unwrap();
        assert_eq!(c2.access_token, "eyJ-nested");
        assert_eq!(c2.refresh_token.as_deref(), Some("rt-nested"));
        assert_eq!(c2.account_id.as_deref(), Some("acct_2"));
    }

    #[test]
    fn builds_claude_and_codex_oauth_refresh_requests() {
        let claude_ref = build_claude_refresh_request("sk-ant-ort01-test");
        assert_eq!(claude_ref.url, "https://platform.claude.com/v1/oauth/token");
        assert_eq!(claude_ref.host, "platform.claude.com");
        assert_eq!(claude_ref.port, 443);
        assert_eq!(claude_ref.path, "/v1/oauth/token");
        assert!(claude_ref.body.contains("\"grant_type\":\"refresh_token\""));
        assert!(claude_ref.body.contains("\"refresh_token\":\"sk-ant-ort01-test\""));
        let raw_c = core::str::from_utf8(&claude_ref.http_bytes).unwrap();
        assert!(raw_c.starts_with("POST /v1/oauth/token HTTP/1.1\r\n"));
        assert!(raw_c.contains("Host: platform.claude.com\r\n"));

        let codex_ref = build_codex_refresh_request("rt-openai-test");
        assert_eq!(codex_ref.url, "https://auth.openai.com/oauth/token");
        assert_eq!(codex_ref.host, "auth.openai.com");
        assert_eq!(codex_ref.port, 443);
        assert_eq!(codex_ref.path, "/oauth/token");
        assert!(codex_ref.body.contains("\"grant_type\":\"refresh_token\""));
        assert!(codex_ref.body.contains("\"refresh_token\":\"rt-openai-test\""));
        let raw_o = core::str::from_utf8(&codex_ref.http_bytes).unwrap();
        assert!(raw_o.starts_with("POST /oauth/token HTTP/1.1\r\n"));
        assert!(raw_o.contains("Host: auth.openai.com\r\n"));
    }

    #[test]
    fn parses_google_oauth_and_builds_refresh_request() {
        let raw = r#"{"access_token":"ya29.abc","refresh_token":"1//ref+1","expiry_date":1790005555,"cloudaicompanionProject":"proj-123"}"#;
        let tok = parse_google_oauth_json(raw).unwrap();
        assert_eq!(tok.access_token, "ya29.abc");
        assert_eq!(tok.refresh_token.as_deref(), Some("1//ref+1"));
        assert_eq!(tok.expires_at, Some(1790005555));
        assert_eq!(tok.account_id.as_deref(), Some("proj-123"));

        let req = build_google_refresh_request("1//ref+1", "cid-1", "csec/2");
        assert_eq!(req.url, "https://oauth2.googleapis.com/token");
        assert_eq!(req.host, "oauth2.googleapis.com");
        assert_eq!(req.path, "/token");
        assert!(req.body.contains("grant_type=refresh_token"));
        assert!(req.body.contains("refresh_token=1%2F%2Fref%2B1"));
        assert!(req.body.contains("client_secret=csec%2F2"));
        let wire = core::str::from_utf8(&req.http_bytes).unwrap();
        assert!(wire.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
    }

    #[test]
    fn builds_and_parses_antigravity_cloudcode_stream() {
        assert_eq!(Provider::from_name("antigravity"), Some(Provider::Antigravity));
        assert_eq!(Provider::Antigravity.host(), "daily-cloudcode-pa.googleapis.com");
        assert_eq!(Provider::Antigravity.default_model(), "gemini-2.5-pro");
        assert_eq!(Provider::Antigravity.key_filename(), "google_oauth.json");
        assert!(Provider::Antigravity.available_models().contains(&"gemini-2.5-pro"));
        assert!(Provider::Codex.available_models().contains(&"gpt-5.1-codex"));

        let msgs = vec![Message::user("Inspect workspace")];
        let req = build_antigravity_request(
            "gemini-2.5-pro",
            "ya29.token",
            "proj-42",
            "You are Antigravity.",
            &msgs,
            &builtin_tools(),
        );
        assert_eq!(req.host, "daily-cloudcode-pa.googleapis.com");
        assert_eq!(req.path, "/v1internal:streamGenerateContent?alt=sse");
        let wire = core::str::from_utf8(&req.http_bytes).unwrap();
        assert!(wire.contains("Authorization: Bearer ya29.token\r\n"));
        assert!(wire.contains("\"project\":\"proj-42\""));
        assert!(wire.contains("\"userAgent\":\"antigravity\""));
        assert!(wire.contains("\"requestType\":\"agent\""));
        assert!(wire.contains("\"request\":{"));

        // Envelope-wrapped SSE response: {"response": {"candidates": ...}}
        let sse = concat!(
            "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"thought\":true,\"text\":\"Planning...\"}]}}]}}\n\n",
            "data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Done!\"}]},\"finishReason\":\"STOP\"}]}}\n\n"
        );
        let wire = chunked_response(sse);
        let mut parser = ProviderStreamParser::new(Provider::Antigravity);
        let mut events = parser.feed(&wire);
        events.extend(parser.finish());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], StreamEvent::ThinkingDelta(String::from("Planning...")));
        assert_eq!(events[1], StreamEvent::TextDelta(String::from("Done!")));
        assert_eq!(
            events[2],
            StreamEvent::Done {
                stop_reason: Some(String::from("STOP"))
            }
        );
    }

    #[test]
    fn builds_and_parses_codex_responses_api_stream() {
        let msgs = vec![Message::user("List files")];
        let req = build_codex_responses_request(
            "gpt-5.1-codex",
            "eyJ-tok",
            Some("acct_99"),
            "System instructions.",
            &msgs,
            &builtin_tools(),
        );
        assert_eq!(req.host, "chatgpt.com");
        assert_eq!(req.path, "/backend-api/codex/responses");
        let wire = core::str::from_utf8(&req.http_bytes).unwrap();
        assert!(wire.contains("Authorization: Bearer eyJ-tok\r\n"));
        assert!(wire.contains("chatgpt-account-id: acct_99\r\n"));
        assert!(wire.contains("originator: codex-tui\r\n"));
        assert!(wire.contains("\"store\":false"));
        assert!(wire.contains("\"instructions\":\"System instructions.\""));
        assert!(wire.contains("\"input\":["));

        let sse = concat!(
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Thinking...\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Checking dir\"}\n\n",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"fc_1\",\"name\":\"list_dir\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{\\\"path\\\":\\\"\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\".\\\"}\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"fc_1\",\"name\":\"list_dir\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
        );
        let wire = chunked_response(sse);
        let mut parser = ProviderStreamParser::new(Provider::Codex);
        let mut events = parser.feed(&wire);
        events.extend(parser.finish());
        assert_eq!(events.len(), 4);
        assert_eq!(events[0], StreamEvent::ThinkingDelta(String::from("Thinking...")));
        assert_eq!(events[1], StreamEvent::TextDelta(String::from("Checking dir")));
        match &events[2] {
            StreamEvent::ToolCall { id, name, arguments } => {
                assert_eq!(id, "fc_1");
                assert_eq!(name, "list_dir");
                assert_eq!(arguments.get("path").and_then(JsonValue::as_str), Some("."));
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[3],
            StreamEvent::Done {
                stop_reason: Some(String::from("completed"))
            }
        );
    }
}
