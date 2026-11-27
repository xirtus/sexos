// SPDX-License-Identifier: GPL-3.0-only
//! Tailor: SexOS's native multi-provider AI coding agent (Gemini, Antigravity ACP Bridge, Claude, Codex, DeepSeek).
//!
//! Launch with network and an optional workspace grant:
//!   `run +net tailor [@/workspace] [gemini|antigravity|claude|claude-deepseek|codex]`
//!
//! Credentials are loaded into `shibari::Keeper` (RAM-only zeroizing secret store) from
//! `/data/tailor/credentials.json` (unified desktop bundle), `claude_oauth.json`,
//! `codex_oauth.json`, `keys.json`, per-provider `*.key` files (`gemini.key`, etc.),
//! `acp.conf` (official LAN `agy acp` bridge), or entered interactively with `/key` or `/acp`.
#![no_std]
#![no_main]

extern crate alloc;
mod ui;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, launch, net, Event, Nav, Window};
use shibari::{Keeper, Knot, Owner};
use tailor_core::{
    build_claude_refresh_request, build_codex_refresh_request, build_codex_responses_request,
    build_http_request, is_claude_oauth_token, json, parse_claude_oauth_json,
    parse_codex_oauth_json, AccessScope, Agent, AgentStep, ApprovalDecision, ApprovalPolicy,
    GrepMatch, JsonValue, OAuthToken, Provider, ProviderStreamParser, StreamEvent, ToolRunner,
    Workspace, WorkspaceFs,
};

const MAX_READ_BYTES: usize = 256 * 1024;
const READ_CHUNK_BYTES: usize = 32 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 24 * 1024;
const COMMAND_TIMEOUT_TICKS: u64 = 15_000;
const OAUTH_REFRESH_MAX_BYTES: usize = 64 * 1024;
const MODEL_DISCOVERY_MAX_BYTES: usize = 256 * 1024;
const CODEX_MODELS_URL: &str =
    "https://chatgpt.com/backend-api/codex/models?client_version=0.169.0";
const DEFAULT_PROXY_URL: &str = "http://127.0.0.1:8317";
const MAX_DIFF_PREVIEW_LINES: usize = 24;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[tailor.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn zeroize_slice(buf: &mut [u8]) {
    for b in buf {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
}

/// Reads a Tailor config/secret file, prioritizing SexOS confined relative paths first
/// (`<filename>` -> `/data/tailor/<filename>`), then `@/data/tailor/<filename>`,
/// `/data/tailor/<filename>`, and `@/media/Ventoy/` fallbacks.
fn read_config_file(
    window: &mut Window,
    filename: &str,
    ventoy_alias: Option<&str>,
    buf: &mut [u8],
) -> Option<usize> {
    let at_data = format!("@/data/tailor/{}", filename);
    let abs_data = format!("/data/tailor/{}", filename);
    let at_ventoy = format!("@/media/Ventoy/{}", filename);
    let abs_ventoy = format!("/media/Ventoy/{}", filename);

    let mut candidates: Vec<&[u8]> = vec![
        filename.as_bytes(),
        at_data.as_bytes(),
        abs_data.as_bytes(),
        at_ventoy.as_bytes(),
        abs_ventoy.as_bytes(),
    ];
    let at_alias;
    let abs_alias;
    if let Some(alias) = ventoy_alias {
        at_alias = format!("@/media/Ventoy/{}", alias);
        abs_alias = format!("/media/Ventoy/{}", alias);
        candidates.push(at_alias.as_bytes());
        candidates.push(abs_alias.as_bytes());
    }

    for path in candidates {
        if let Ok(n) = files::read(window, path, buf) {
            return Some(n);
        }
    }
    None
}

/// Writes a Tailor config/secret file, prioritizing SexOS confined relative paths first
/// (`<filename>` -> `/data/tailor/<filename>`), then `@/data/tailor/<filename>`,
/// `/data/tailor/<filename>`, and optional Ventoy fallbacks.
fn write_config_file(
    window: &mut Window,
    filename: &str,
    ventoy_alias: Option<&str>,
    data: &[u8],
) -> Option<String> {
    let local_path = String::from(filename);
    let at_data = format!("@/data/tailor/{}", filename);
    let abs_data = format!("/data/tailor/{}", filename);

    let mut candidates = vec![local_path, at_data, abs_data];
    if let Some(alias) = ventoy_alias {
        candidates.push(format!("@/media/Ventoy/{}", alias));
        candidates.push(format!("/media/Ventoy/{}", alias));
    }

    for (idx, path) in candidates.into_iter().enumerate() {
        if idx == 2 {
            let _ = files::mkdir(window, b"/data");
            let _ = files::mkdir(window, b"/data/tailor");
        }
        if files::write(window, path.as_bytes(), data).is_ok() {
            let _ = files::sync(window);
            return Some(path);
        }
    }
    None
}

fn non_empty_str<'a>(val: Option<&'a JsonValue>) -> Option<&'a str> {
    val.and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_u64_json(val: Option<&JsonValue>) -> Option<u64> {
    let v = val?;
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    v.as_str().and_then(|s| s.trim().parse::<u64>().ok())
}

fn parse_oauth_object(obj: &JsonValue) -> Option<OAuthToken> {
    let access_token = non_empty_str(obj.get("access_token").or_else(|| obj.get("accessToken")))?;
    let refresh_token = non_empty_str(obj.get("refresh_token").or_else(|| obj.get("refreshToken")))
        .map(String::from);
    let account_id = non_empty_str(
        obj.get("account_id")
            .or_else(|| obj.get("accountId"))
            .or_else(|| obj.get("project_id"))
            .or_else(|| obj.get("projectId"))
            .or_else(|| obj.get("cloudaicompanionProject")),
    )
    .map(String::from);
    let expires_at = parse_u64_json(
        obj.get("expires_at")
            .or_else(|| obj.get("expiresAt"))
            .or_else(|| obj.get("expiry_date"))
            .or_else(|| obj.get("expiryDate")),
    );
    Some(OAuthToken {
        access_token: String::from(access_token),
        refresh_token,
        account_id,
        expires_at,
    })
}

struct KeyVault {
    owner: Owner,
    keeper: Keeper,
    gemini: Option<Knot>,
    claude: Option<Knot>,
    claude_deepseek: Option<Knot>,
    codex: Option<Knot>,
    claude_refresh: Option<Knot>,
    codex_refresh: Option<Knot>,
    proxy_key: Option<Knot>,
    claude_oauth: bool,
    codex_oauth: bool,
    codex_account_id: Option<String>,
    claude_expires_at: Option<u64>,
    codex_expires_at: Option<u64>,
}

impl KeyVault {
    fn new(owner: Owner, salt: u64) -> Self {
        Self {
            owner,
            keeper: Keeper::new(salt, 16),
            gemini: None,
            claude: None,
            claude_deepseek: None,
            codex: None,
            claude_refresh: None,
            codex_refresh: None,
            proxy_key: None,
            claude_oauth: false,
            codex_oauth: false,
            codex_account_id: None,
            claude_expires_at: None,
            codex_expires_at: None,
        }
    }

    fn slot(&self, provider: Provider) -> Option<Knot> {
        match provider {
            Provider::Gemini => self.gemini,
            Provider::Antigravity => None,
            Provider::Claude => self.claude,
            Provider::ClaudeDeepSeek => self.claude_deepseek,
            Provider::Codex => self.codex,
        }
    }

    fn set_slot(&mut self, provider: Provider, knot: Knot) {
        match provider {
            Provider::Gemini => self.gemini = Some(knot),
            Provider::Antigravity => {}
            Provider::Claude => self.claude = Some(knot),
            Provider::ClaudeDeepSeek => self.claude_deepseek = Some(knot),
            Provider::Codex => self.codex = Some(knot),
        }
    }

    fn store_knot(
        keeper: &mut Keeper,
        owner: Owner,
        slot: &mut Option<Knot>,
        label: &str,
        secret: &[u8],
    ) -> Result<(), &'static str> {
        if secret.is_empty() {
            return Err("Empty secret");
        }
        if let Some(knot) = *slot {
            keeper
                .retie(owner, knot, secret)
                .map_err(|_| "Failed to update secret in Shibari")?;
        } else {
            let knot = keeper
                .tie(owner, label, secret)
                .map_err(|_| "Shibari key vault full")?;
            *slot = Some(knot);
        }
        Ok(())
    }

    fn store(&mut self, provider: Provider, secret: &[u8]) -> Result<(), &'static str> {
        if provider == Provider::Antigravity {
            return Err("Antigravity uses official ACP Bridge (/acp <url> or acp.conf)");
        }
        if secret.is_empty() {
            return Err("Empty API key");
        }
        if let Some(knot) = self.slot(provider) {
            self.keeper
                .retie(self.owner, knot, secret)
                .map_err(|_| "Failed to update key in Shibari")?;
        } else {
            let knot = self
                .keeper
                .tie(self.owner, provider.key_filename(), secret)
                .map_err(|_| "Shibari key vault full")?;
            self.set_slot(provider, knot);
        }
        if provider == Provider::Claude {
            if let Ok(s) = core::str::from_utf8(secret) {
                self.claude_oauth = is_claude_oauth_token(s);
            }
        }
        Ok(())
    }

    fn store_oauth(&mut self, provider: Provider, token: &OAuthToken) -> Result<(), &'static str> {
        self.store(provider, token.access_token.as_bytes())?;
        match provider {
            Provider::Claude => {
                self.claude_oauth = true;
                self.claude_expires_at = token.expires_at;
                if let Some(ref rt) = token.refresh_token {
                    Self::store_knot(
                        &mut self.keeper,
                        self.owner,
                        &mut self.claude_refresh,
                        "claude_refresh",
                        rt.as_bytes(),
                    )?;
                }
            }
            Provider::Codex => {
                self.codex_oauth = true;
                self.codex_expires_at = token.expires_at;
                if token.account_id.is_some() {
                    self.codex_account_id = token.account_id.clone();
                }
                if let Some(ref rt) = token.refresh_token {
                    Self::store_knot(
                        &mut self.keeper,
                        self.owner,
                        &mut self.codex_refresh,
                        "codex_refresh",
                        rt.as_bytes(),
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn store_proxy_key(&mut self, secret: &[u8]) -> Result<(), &'static str> {
        Self::store_knot(
            &mut self.keeper,
            self.owner,
            &mut self.proxy_key,
            "proxy.key",
            secret,
        )
    }

    fn has_key(&self, provider: Provider) -> bool {
        self.slot(provider).is_some()
    }

    fn is_oauth(&self, provider: Provider) -> bool {
        match provider {
            Provider::Claude => self.claude_oauth,
            Provider::Codex => self.codex_oauth,
            _ => false,
        }
    }

    fn codex_account_id(&self) -> Option<&str> {
        self.codex_account_id.as_deref()
    }

    fn has_refresh_token(&self, provider: Provider) -> bool {
        match provider {
            Provider::Claude => self.claude_refresh.is_some(),
            Provider::Codex => self.codex_refresh.is_some(),
            _ => false,
        }
    }

    fn with_key<R>(&self, provider: Provider, f: impl FnOnce(&str) -> R) -> Option<R> {
        let knot = self.slot(provider)?;
        self.keeper
            .with(self.owner, knot, |bytes| {
                let s = core::str::from_utf8(bytes).unwrap_or("");
                f(s)
            })
            .ok()
    }

    fn with_refresh_token<R>(&self, provider: Provider, f: impl FnOnce(&str) -> R) -> Option<R> {
        let knot = match provider {
            Provider::Claude => self.claude_refresh?,
            Provider::Codex => self.codex_refresh?,
            _ => return None,
        };
        self.keeper
            .with(self.owner, knot, |bytes| {
                let s = core::str::from_utf8(bytes).unwrap_or("");
                f(s)
            })
            .ok()
    }

    fn with_proxy_key<R>(&self, f: impl FnOnce(&str) -> R) -> Option<R> {
        let knot = self.proxy_key?;
        self.keeper
            .with(self.owner, knot, |bytes| {
                let s = core::str::from_utf8(bytes).unwrap_or("");
                f(s)
            })
            .ok()
    }

    /// Loads unified desktop `credentials.json` containing official API keys
    /// (`gemini`, `deepseek`, `claude`, `codex`), Claude/Codex OAuth, `acp_bridge`, and `proxy`.
    fn load_unified_credentials(
        &mut self,
        raw: &str,
    ) -> (usize, Option<String>, Option<String>) {
        let Ok(root) = json::parse(raw) else {
            return (0, None, None);
        };
        let mut loaded = 0usize;
        let mut proxy_base = None;
        let mut acp_bridge_url = None;

        for (key_name, provider) in [
            ("gemini", Provider::Gemini),
            ("claude", Provider::Claude),
            ("codex", Provider::Codex),
            ("deepseek", Provider::ClaudeDeepSeek),
            ("claude-deepseek", Provider::ClaudeDeepSeek),
            ("claude_deepseek", Provider::ClaudeDeepSeek),
        ] {
            if self.has_key(provider) {
                continue;
            }
            let Some(entry) = root.get(key_name) else {
                continue;
            };
            if matches!(provider, Provider::Claude | Provider::Codex) {
                if let Some(oauth_obj) = entry.get("oauth") {
                    if let Some(tok) = parse_oauth_object(oauth_obj) {
                        if self.store_oauth(provider, &tok).is_ok() {
                            loaded += 1;
                            continue;
                        }
                    }
                }
            }
            let api_key = entry
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    non_empty_str(
                        entry
                            .get("api_key")
                            .or_else(|| entry.get("key"))
                            .or_else(|| entry.get("token")),
                    )
                });
            if let Some(secret) = api_key {
                if self.store(provider, secret.as_bytes()).is_ok() {
                    loaded += 1;
                }
            }
        }

        if let Some(acp) = root.get("acp_bridge").or_else(|| root.get("acp")) {
            let url = acp
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| non_empty_str(acp.get("base_url").or_else(|| acp.get("url"))));
            if let Some(u) = url {
                acp_bridge_url = Some(String::from(u.trim_end_matches('/')));
            }
        }

        if let Some(proxy) = root.get("proxy") {
            if let Some(pk) = non_empty_str(proxy.get("api_key").or_else(|| proxy.get("key"))) {
                let _ = self.store_proxy_key(pk.as_bytes());
            }
            let enabled = proxy
                .get("enabled")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            if enabled {
                if let Some(url) = non_empty_str(proxy.get("base_url").or_else(|| proxy.get("url")))
                {
                    proxy_base = Some(String::from(url.trim_end_matches('/')));
                }
            }
        }

        (loaded, proxy_base, acp_bridge_url)
    }

    fn load_from_files(&mut self, window: &mut Window) -> (usize, Option<String>, Option<String>) {
        let mut loaded = 0usize;
        let mut proxy_base = None;
        let mut acp_bridge_url = None;

        // 0. Unified desktop `credentials.json` (single-file drop onto Ventoy USB or /data/tailor/).
        let mut cred_buf = vec![0u8; 16384];
        if let Some(n) = read_config_file(
            window,
            "credentials.json",
            Some("tailor-credentials.json"),
            &mut cred_buf,
        ) {
            if let Ok(raw) = core::str::from_utf8(&cred_buf[..n]) {
                let (c_loaded, c_proxy, c_acp) = self.load_unified_credentials(raw);
                loaded += c_loaded;
                if c_proxy.is_some() {
                    proxy_base = c_proxy;
                }
                if c_acp.is_some() {
                    acp_bridge_url = c_acp;
                }
            }
        }
        zeroize_slice(&mut cred_buf);

        // 1. Load individual OAuth credentials (Claude Pro/Max, Codex/ChatGPT).
        let mut oauth_buf = vec![0u8; 8192];
        if !self.has_key(Provider::Claude) {
            if let Some(n) = read_config_file(window, "claude_oauth.json", None, &mut oauth_buf) {
                if let Ok(raw) = core::str::from_utf8(&oauth_buf[..n]) {
                    if let Some(tok) = parse_claude_oauth_json(raw) {
                        if self.store_oauth(Provider::Claude, &tok).is_ok() {
                            loaded += 1;
                        }
                    }
                }
            }
        }
        zeroize_slice(&mut oauth_buf);

        if !self.has_key(Provider::Codex) {
            if let Some(n) = read_config_file(window, "codex_oauth.json", None, &mut oauth_buf) {
                if let Ok(raw) = core::str::from_utf8(&oauth_buf[..n]) {
                    if let Some(tok) = parse_codex_oauth_json(raw) {
                        if self.store_oauth(Provider::Codex, &tok).is_ok() {
                            loaded += 1;
                        }
                    }
                }
            }
        }
        zeroize_slice(&mut oauth_buf);

        // 2. Load JSON key bundle (supports gemini, claude, claude-deepseek, codex).
        let mut jbuf = vec![0u8; 4096];
        if let Some(n) = read_config_file(window, "keys.json", Some("tailor-keys.json"), &mut jbuf) {
            if let Ok(raw) = core::str::from_utf8(&jbuf[..n]) {
                for (provider, secret) in tailor_core::parse_keys_json(raw) {
                    if provider != Provider::Antigravity
                        && !self.has_key(provider)
                        && self.store(provider, secret.as_bytes()).is_ok()
                    {
                        loaded += 1;
                    }
                }
            }
        }
        zeroize_slice(&mut jbuf);

        // 3. Fallback to per-provider *.key files (official Gemini API key, Claude, DeepSeek, Codex).
        let mut buf = vec![0u8; 512];
        for provider in [
            Provider::Gemini,
            Provider::Claude,
            Provider::ClaudeDeepSeek,
            Provider::Codex,
        ] {
            if self.has_key(provider) {
                continue;
            }
            let ventoy_alias = format!("tailor-{}", provider.key_filename());
            if let Some(n) = read_config_file(
                window,
                provider.key_filename(),
                Some(&ventoy_alias),
                &mut buf,
            ) {
                if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() && self.store(provider, trimmed.as_bytes()).is_ok() {
                        loaded += 1;
                    }
                }
            }
            zeroize_slice(&mut buf);
        }

        // 4. Optional official Antigravity ACP bridge (`acp.conf`) & proxy (`proxy.key` / `proxy.conf`).
        if acp_bridge_url.is_none() {
            if let Some(n) =
                read_config_file(window, "acp.conf", Some("tailor-acp.conf"), &mut buf)
            {
                if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        acp_bridge_url = Some(String::from(trimmed.trim_end_matches('/')));
                    }
                }
            }
            zeroize_slice(&mut buf);
        }

        if self.proxy_key.is_none() {
            if let Some(n) = read_config_file(window, "proxy.key", None, &mut buf) {
                if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        let _ = self.store_proxy_key(trimmed.as_bytes());
                    }
                }
            }
            zeroize_slice(&mut buf);
        }

        if proxy_base.is_none() {
            if let Some(n) = read_config_file(window, "proxy.conf", None, &mut buf) {
                if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        proxy_base = Some(String::from(trimmed.trim_end_matches('/')));
                    }
                }
            }
            zeroize_slice(&mut buf);
        }

        (loaded, proxy_base, acp_bridge_url)
    }
}

fn glob_match_ci(pattern: &str, text: &str) -> bool {
    let p_low = pattern.to_ascii_lowercase();
    let t_low = text.to_ascii_lowercase();
    if !p_low.contains('*') {
        return t_low.contains(&p_low);
    }
    let parts: Vec<&str> = p_low.split('*').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }
    let mut rest = t_low.as_str();
    let starts_lit = !p_low.starts_with('*');
    let ends_lit = !p_low.ends_with('*');
    for (i, part) in parts.iter().enumerate() {
        if i == 0 && starts_lit {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else if i + 1 == parts.len() && ends_lit {
            return rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

fn join_sex_path(base: &str, child: &str) -> String {
    if base.is_empty() || base == "." {
        String::from(child)
    } else if base == "/" || base == "@/" {
        format!("{}{}", base, child)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), child)
    }
}

/// Capability-scoped SexOS workspace backed by `sex_app::files`.
/// Honors [`AccessScope::WorkspaceOnly`] vs [`AccessScope::FullSystem`].
struct SexWorkspace<'a> {
    window: &'a mut Window,
    root: &'a str,
    scope: AccessScope,
}

impl<'a> SexWorkspace<'a> {
    fn resolve(&self, path: &str) -> Result<String, String> {
        let trimmed = path.trim();
        if trimmed.is_empty() || trimmed == "." {
            return Ok(if self.root.is_empty() {
                String::from(".")
            } else {
                String::from(self.root)
            });
        }
        if self.scope == AccessScope::FullSystem {
            if trimmed.starts_with('/') || trimmed.starts_with('@') || self.root.is_empty() {
                return Ok(String::from(trimmed));
            }
            let base = self.root.trim_end_matches('/');
            let rel = trimmed.trim_start_matches("./").trim_start_matches('/');
            return Ok(if base == "@" || base.is_empty() {
                format!("{}/{}", base, rel)
            } else {
                format!("{}/{}", base, rel)
            });
        }

        // WorkspaceOnly: block `..` traversal and absolute paths outside `self.root`.
        if trimmed.split('/').any(|seg| seg == "..") {
            return Err(format!(
                "Path '{}' escapes workspace (toggle Full Sys scope to allow)",
                trimmed
            ));
        }
        if trimmed.starts_with('/') || trimmed.starts_with('@') {
            let root_norm = self.root.trim_end_matches('/');
            let allowed = self.root.is_empty()
                || self.root == "/"
                || self.root == "@/"
                || trimmed == self.root
                || trimmed == root_norm
                || trimmed.starts_with(&format!("{}/", root_norm));
            if allowed {
                return Ok(String::from(trimmed));
            }
            return Err(format!(
                "Path '{}' is outside workspace '{}' (toggle Full Sys scope to allow)",
                trimmed, self.root
            ));
        }
        if self.root.is_empty() {
            return Ok(String::from(trimmed));
        }
        let rel = trimmed.trim_start_matches("./").trim_start_matches('/');
        Ok(join_sex_path(self.root, rel))
    }

    fn read_full_path(&mut self, full: &str, max_bytes: usize) -> Result<String, String> {
        let mut out = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK_BYTES];
        let mut offset = 0u64;
        loop {
            let n = files::read_at(self.window, full.as_bytes(), offset, &mut chunk)
                .map_err(|e| format!("files::read_at({}) failed: code {}", full, e))?;
            if n == 0 {
                break;
            }
            let take = n.min(max_bytes.saturating_sub(out.len()));
            out.extend_from_slice(&chunk[..take]);
            offset += n as u64;
            if out.len() >= max_bytes || n < chunk.len() {
                break;
            }
        }
        String::from_utf8(out).map_err(|_| format!("File {} is not valid UTF-8", full))
    }
}

impl<'a> Workspace for SexWorkspace<'a> {
    fn read_file(&mut self, path: &str) -> Result<String, String> {
        let full = self.resolve(path)?;
        self.read_full_path(&full, MAX_READ_BYTES)
    }

    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        let full = self.resolve(path)?;
        files::write(self.window, full.as_bytes(), content.as_bytes())
            .map_err(|e| format!("files::write({}) failed: code {}", full, e))?;
        let _ = files::sync(self.window);
        Ok(())
    }

    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, String> {
        let full = self.resolve(path)?;
        let entries = files::list(self.window, full.as_bytes())
            .map_err(|e| format!("files::list({}) failed: code {}", full, e))?;
        Ok(entries
            .into_iter()
            .map(|e| {
                let name = String::from_utf8_lossy(&e.name).into_owned();
                if e.dir {
                    format!("{}/", name)
                } else {
                    format!("{} ({} B)", name, e.size)
                }
            })
            .collect())
    }
}

impl<'a> WorkspaceFs for SexWorkspace<'a> {
    fn delete_file(&mut self, path: &str) -> Result<(), String> {
        let full = self.resolve(path)?;
        files::direct::remove(full.as_bytes())
            .map_err(|e| format!("files::remove({}) failed: code {}", full, e))?;
        let _ = files::sync(self.window);
        Ok(())
    }

    fn move_file(&mut self, from: &str, to: &str) -> Result<(), String> {
        let src = self.resolve(from)?;
        let dst = self.resolve(to)?;
        files::direct::rename(src.as_bytes(), dst.as_bytes())
            .map_err(|e| format!("files::rename({} -> {}) failed: code {}", src, dst, e))?;
        let _ = files::sync(self.window);
        Ok(())
    }

    fn set_access_scope(&mut self, scope: AccessScope) {
        self.scope = scope;
    }

    fn access_scope(&self) -> AccessScope {
        self.scope
    }

    fn find_by_name(&mut self, dir: &str, pattern: &str) -> Result<Vec<String>, String> {
        let root = self.resolve(dir)?;
        let mut results = Vec::new();
        let mut stack: Vec<(String, usize)> = vec![(root, 0)];
        while let Some((cur, depth)) = stack.pop() {
            if results.len() >= 200 || depth > 5 {
                continue;
            }
            let Ok(entries) = files::list(self.window, cur.as_bytes()) else {
                continue;
            };
            for e in entries {
                let name = String::from_utf8_lossy(&e.name).into_owned();
                if name == "." || name == ".." {
                    continue;
                }
                let child = join_sex_path(&cur, &name);
                if glob_match_ci(pattern, &name) || glob_match_ci(pattern, &child) {
                    results.push(if e.dir {
                        format!("{}/", child)
                    } else {
                        child.clone()
                    });
                    if results.len() >= 200 {
                        break;
                    }
                }
                if e.dir && depth < 5 {
                    stack.push((child, depth + 1));
                }
            }
        }
        Ok(results)
    }

    fn grep_search(&mut self, path: &str, query: &str) -> Result<Vec<GrepMatch>, String> {
        let full = self.resolve(path)?;
        let q_low = query.to_ascii_lowercase();
        let mut matches = Vec::new();

        // Try searching as a directory first; if not a directory, search as a single file.
        if files::list(self.window, full.as_bytes()).is_err() {
            let content = self.read_full_path(&full, 64 * 1024)?;
            for (idx, line) in content.lines().enumerate() {
                if line.to_ascii_lowercase().contains(&q_low) {
                    matches.push(GrepMatch {
                        path: full.clone(),
                        line: idx + 1,
                        text: String::from(line),
                    });
                    if matches.len() >= 100 {
                        break;
                    }
                }
            }
            return Ok(matches);
        }

        let mut stack: Vec<(String, usize)> = vec![(full, 0)];
        let mut scanned_files = 0usize;
        while let Some((cur, depth)) = stack.pop() {
            if matches.len() >= 100 || scanned_files >= 64 || depth > 4 {
                continue;
            }
            let Ok(entries) = files::list(self.window, cur.as_bytes()) else {
                continue;
            };
            for e in entries {
                if matches.len() >= 100 || scanned_files >= 64 {
                    break;
                }
                let name = String::from_utf8_lossy(&e.name).into_owned();
                if name == "." || name == ".." {
                    continue;
                }
                let child = join_sex_path(&cur, &name);
                if e.dir {
                    if depth < 4 {
                        stack.push((child, depth + 1));
                    }
                } else if e.size <= 64 * 1024 {
                    scanned_files += 1;
                    if let Ok(content) = self.read_full_path(&child, 64 * 1024) {
                        for (idx, line) in content.lines().enumerate() {
                            if line.to_ascii_lowercase().contains(&q_low) {
                                matches.push(GrepMatch {
                                    path: child.clone(),
                                    line: idx + 1,
                                    text: String::from(line),
                                });
                                if matches.len() >= 100 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(matches)
    }
}

/// SexOS command launcher backed by `sex_app::launch`.
struct SexToolRunner {
    next_cookie: u32,
}

impl SexToolRunner {
    fn new() -> Self {
        Self { next_cookie: 1 }
    }
}

impl ToolRunner for SexToolRunner {
    fn run_command(&mut self, program: &str, args: &str, network: bool) -> Result<String, String> {
        let cookie = self.next_cookie;
        self.next_cookie = self.next_cookie.wrapping_add(1).max(1);
        launch::run(program.as_bytes(), args.as_bytes(), network, cookie)
            .map_err(|e| format!("launch::run({}) refused: {:?}", program, e))?;

        let start = sex_pdx::get_ticks();
        let mut started = false;
        loop {
            if let Some(ev) = launch::poll(cookie) {
                match ev {
                    launch::Event::Started { .. } => {
                        started = true;
                    }
                    launch::Event::Refused { code, .. } => {
                        return Err(format!("{} refused by Shell (code {})", program, code));
                    }
                    launch::Event::Done { status, .. } => {
                        return Ok(match status {
                            Some(code) => format!("{} exited with status {}", program, code),
                            None => format!("{} finished", program),
                        });
                    }
                }
            }
            if sex_pdx::get_ticks().saturating_sub(start) > COMMAND_TIMEOUT_TICKS {
                launch::kill(cookie);
                return Err(format!(
                    "{} timed out after {} ms (started={})",
                    program, COMMAND_TIMEOUT_TICKS, started
                ));
            }
            let _ = sex_pdx::sys_wait_self_ticks(4);
        }
    }

    fn hotswap_server(&mut self, server: &str, binary_path: &str) -> Result<String, String> {
        Err(format!(
            "Server hot-swap ({} <- {}) requires Spindle supervisor capability",
            server, binary_path
        ))
    }
}

struct ActiveStream {
    session_idx: usize,
    provider: Provider,
    duplex: net::Duplex,
    parser: ProviderStreamParser,
    rx_buf: Vec<u8>,
    retried_after_refresh: bool,
}

struct AgentSession {
    title: String,
    agent: Agent,
    transcript: String,
    workspace_dir: String,
    pending_prompt: Option<String>,
}

fn provider_index(provider: Provider) -> usize {
    match provider {
        Provider::Antigravity => 0,
        Provider::Codex => 1,
        Provider::Claude => 2,
        Provider::ClaudeDeepSeek => 3,
        Provider::Gemini => 4,
    }
}

fn curated_models(provider: Provider) -> Vec<ui::ModelInfo> {
    let mk = |id: &str, name: &str, tag: Option<&str>| ui::ModelInfo {
        id: String::from(id),
        display_name: String::from(name),
        tag: tag.map(String::from),
    };
    match provider {
        Provider::Antigravity => vec![
            mk("gemini-2.5-pro", "Gemini 2.5 Pro (ACP)", Some("ACP")),
            mk("gemini-2.5-flash", "Gemini 2.5 Flash (ACP)", Some("ACP")),
            mk("claude-sonnet-4-20250514", "Claude Sonnet 4 (ACP)", None),
        ],
        Provider::Codex => vec![
            mk("gpt-6.2-sol", "GPT-6.2-Sol", Some("New")),
            mk("gpt-6.2-astra", "GPT-6.2-Astra", Some("New")),
            mk("gpt-6.1-terra", "GPT-6.1 Terra", None),
            mk("gpt-6-sol", "GPT-6-Sol", None),
            mk("gpt-5.1-codex", "GPT-5.1 Codex", None),
        ],
        Provider::Claude => vec![
            mk("claude-sonnet-4-20250514", "Claude Sonnet 4", None),
            mk("claude-opus-4-20250514", "Claude Opus 4", None),
        ],
        Provider::ClaudeDeepSeek => vec![
            mk("deepseek-chat", "DeepSeek Chat", None),
            mk("deepseek-reasoner", "DeepSeek Reasoner", None),
        ],
        Provider::Gemini => vec![
            mk("gemini-2.5-pro", "Gemini 2.5 Pro", None),
            mk("gemini-2.5-flash", "Gemini 2.5 Flash", None),
        ],
    }
}

fn default_model_for_provider(provider: Provider, is_oauth: bool) -> &'static str {
    match provider {
        Provider::Antigravity => "gemini-2.5-pro",
        Provider::Codex => {
            if is_oauth {
                "gpt-6.2-sol"
            } else {
                "gpt-6.2-sol"
            }
        }
        Provider::Claude => "claude-sonnet-4-20250514",
        Provider::ClaudeDeepSeek => "deepseek-chat",
        Provider::Gemini => "gemini-2.5-pro",
    }
}

fn parent_sex_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "@" || trimmed == "." {
        return String::from("@/");
    }
    if let Some(pos) = trimmed.rfind('/') {
        if pos == 0 {
            String::from("/")
        } else {
            let parent = &trimmed[..pos];
            if parent == "@" {
                String::from("@/")
            } else {
                String::from(parent)
            }
        }
    } else {
        String::from("@/")
    }
}

struct AppState {
    workspace_root: String,
    access_scope: AccessScope,
    sessions: Vec<AgentSession>,
    active_session: usize,
    next_session_num: usize,
    vault: KeyVault,
    proxy_base: Option<String>,
    acp_bridge_url: Option<String>,
    runner: SexToolRunner,
    input: String,
    input_focused: bool,
    input_selected: bool,
    status: String,
    scroll_lines: usize,
    center_scroll: usize,
    stream: Option<ActiveStream>,
    ui: ui::TailorUi,
    // 3-pane IDE & overlay state
    left_tab: ui::LeftTab,
    center_tab: ui::CenterTab,
    workspace_files: Vec<String>,
    open_file_name: Option<String>,
    open_file_content: String,
    active_artifact_name: Option<String>,
    active_diff_title: String,
    active_diff_content: String,
    model_picker_open: bool,
    settings_open: bool,
    models_by_provider: [Vec<ui::ModelInfo>; 5],
    models_fetched: [bool; 5],
    last_pointer_x: i32,
    last_pointer_y: i32,
}

impl AppState {
    fn active(&self) -> &AgentSession {
        &self.sessions[self.active_session]
    }

    fn active_mut(&mut self) -> &mut AgentSession {
        &mut self.sessions[self.active_session]
    }

    fn agent(&self) -> &Agent {
        &self.sessions[self.active_session].agent
    }

    fn agent_mut(&mut self) -> &mut Agent {
        &mut self.sessions[self.active_session].agent
    }

    fn trim_session_transcript(transcript: &mut String) {
        if transcript.len() > MAX_TRANSCRIPT_BYTES {
            let cut = transcript.len() - (MAX_TRANSCRIPT_BYTES * 3 / 4);
            let mut idx = cut;
            while idx < transcript.len() && !transcript.is_char_boundary(idx) {
                idx += 1;
            }
            transcript.drain(..idx);
        }
    }

    fn push_log_for(&mut self, idx: usize, line: &str) {
        if let Some(sess) = self.sessions.get_mut(idx) {
            sess.transcript.push_str(line);
            sess.transcript.push('\n');
            Self::trim_session_transcript(&mut sess.transcript);
        }
    }

    fn push_delta_for(&mut self, idx: usize, delta: &str) {
        if let Some(sess) = self.sessions.get_mut(idx) {
            sess.transcript.push_str(delta);
            Self::trim_session_transcript(&mut sess.transcript);
        }
    }

    fn push_log(&mut self, line: &str) {
        let idx = self.active_session;
        self.push_log_for(idx, line);
    }

    fn set_access_scope(&mut self, scope: AccessScope) {
        self.access_scope = scope;
        for sess in &mut self.sessions {
            sess.agent.set_access_scope(scope);
        }
        self.status = match scope {
            AccessScope::WorkspaceOnly => {
                String::from("Access Scope: Workspace Only (confined to active space)")
            }
            AccessScope::FullSystem => {
                String::from("Access Scope: Full System (@/, /data, /bin, /projects unlocked)")
            }
        };
        self.push_log(&format!("[tailor] {}", self.status));
    }

    fn toggle_access_scope(&mut self) {
        let next = match self.access_scope {
            AccessScope::WorkspaceOnly => AccessScope::FullSystem,
            AccessScope::FullSystem => AccessScope::WorkspaceOnly,
        };
        self.set_access_scope(next);
    }

    fn spawn_new_session(&mut self, title: Option<String>, workspace_dir: Option<String>) -> usize {
        let prov = self.agent().provider();
        let model = String::from(self.agent().model());
        let policy = self.agent().approval_policy();
        let ws_dir = workspace_dir.unwrap_or_else(|| self.workspace_root.clone());

        let mut agent = Agent::new(prov);
        agent.set_model(&model);
        agent.set_approval_policy(policy);
        agent.set_access_scope(self.access_scope);

        let num = self.next_session_num;
        self.next_session_num += 1;
        let sess_title = title.unwrap_or_else(|| format!("Agent {}", num));
        let idx = self.sessions.len();
        self.sessions.push(AgentSession {
            title: sess_title.clone(),
            agent,
            transcript: format!(
                "[tailor] Started {} in {}\n",
                sess_title,
                if ws_dir.is_empty() { "." } else { ws_dir.as_str() }
            ),
            workspace_dir: ws_dir,
            pending_prompt: None,
        });
        idx
    }

    fn drain_spawned_subagents(&mut self, session_idx: usize) {
        let requests = if let Some(sess) = self.sessions.get_mut(session_idx) {
            sess.agent.take_spawned_subagents()
        } else {
            return;
        };
        if requests.is_empty() {
            return;
        }
        let parent_prov = self.sessions[session_idx].agent.provider();
        let parent_model = String::from(self.sessions[session_idx].agent.model());
        let parent_policy = self.sessions[session_idx].agent.approval_policy();
        let parent_ws = self.sessions[session_idx].workspace_dir.clone();

        for req in requests {
            let ws_dir = req.workspace.unwrap_or_else(|| parent_ws.clone());
            let mut child_agent = Agent::new(parent_prov);
            child_agent.set_model(&parent_model);
            child_agent.set_approval_policy(parent_policy);
            child_agent.set_access_scope(self.access_scope);

            let title = if req.role.trim().is_empty() {
                format!("Subagent {}", req.id)
            } else {
                req.role.clone()
            };
            let intro = format!(
                "[subagent:{}] Role: {}\nSpace: {}\nPrompt: {}\n\nClick 'Run' in the Agents tab (or press Enter) to execute.\n",
                req.id,
                title,
                if ws_dir.is_empty() { "." } else { ws_dir.as_str() },
                req.prompt
            );
            self.sessions.push(AgentSession {
                title: title.clone(),
                agent: child_agent,
                transcript: intro,
                workspace_dir: ws_dir,
                pending_prompt: Some(req.prompt),
            });
            self.push_log_for(
                session_idx,
                &format!(
                    "[tailor] Spawned child agent '{}' — inspect or Run in Agents tab.",
                    title
                ),
            );
        }
    }

    fn session_infos(&self) -> Vec<ui::AgentSessionInfo> {
        let streaming_idx = self.stream.as_ref().map(|s| s.session_idx);
        self.sessions
            .iter()
            .enumerate()
            .map(|(i, s)| ui::AgentSessionInfo {
                title: s.title.clone(),
                workspace_dir: s.workspace_dir.clone(),
                step_count: s.agent.steps().len(),
                snapshot_count: s.agent.snapshots().len(),
                has_pending_prompt: s.pending_prompt.is_some(),
                active: i == self.active_session,
                streaming: streaming_idx == Some(i),
            })
            .collect()
    }

    fn auth_mode_label(&self, provider: Provider) -> &'static str {
        if provider == Provider::Antigravity {
            if self.acp_bridge_url.is_some() || self.proxy_base.is_some() {
                "acp-bridge"
            } else {
                "bridge-not-set"
            }
        } else if self.proxy_base.is_some() {
            "proxy"
        } else if self.vault.is_oauth(provider) {
            "oauth"
        } else if self.vault.has_key(provider) {
            "api-key"
        } else {
            "missing"
        }
    }

    fn provider_statuses(&self) -> Vec<ui::ProviderStatusInfo> {
        let specs: [(Provider, &'static str, &'static str); 5] = [
            (Provider::Antigravity, "Antigravity", "ACP Bridge"),
            (Provider::Gemini, "Gemini", "Official API Key"),
            (Provider::Codex, "Codex", "ChatGPT OAuth"),
            (Provider::Claude, "Claude", "Anthropic"),
            (Provider::ClaudeDeepSeek, "DeepSeek", "API Key"),
        ];
        specs
            .into_iter()
            .map(|(provider, label, subtitle)| {
                if provider == Provider::Antigravity {
                    let connected = self.acp_bridge_url.is_some() || self.proxy_base.is_some();
                    let auth_kind = if connected {
                        "ACP Bridge"
                    } else {
                        "Set /acp or acp.conf"
                    };
                    return ui::ProviderStatusInfo {
                        provider,
                        label,
                        subtitle,
                        connected,
                        auth_kind,
                    };
                }
                let connected = self.vault.has_key(provider) || self.proxy_base.is_some();
                let auth_kind = if self.proxy_base.is_some() {
                    "Proxy"
                } else if self.vault.is_oauth(provider) {
                    "OAuth"
                } else if self.vault.has_key(provider) {
                    "API Key"
                } else {
                    "Not connected"
                };
                ui::ProviderStatusInfo {
                    provider,
                    label,
                    subtitle,
                    connected,
                    auth_kind,
                }
            })
            .collect()
    }

    fn active_model_display(&self) -> String {
        let cur = self.agent().model();
        let idx = provider_index(self.agent().provider());
        if let Some(m) = self.models_by_provider[idx].iter().find(|m| m.id == cur) {
            return m.display_name.clone();
        }
        String::from(cur)
    }

    fn switch_provider(&mut self, window: &mut Window, provider: Provider) {
        let is_oauth = self.vault.is_oauth(provider);
        let def_model = default_model_for_provider(provider, is_oauth);
        self.agent_mut().set_provider(provider);
        self.agent_mut().set_model(def_model);
        self.status = format!(
            "Switched to {} ({}) [auth={}]",
            provider.display_name(),
            self.agent().model(),
            self.auth_mode_label(provider)
        );
        self.push_log(&format!("[tailor] {}", self.status));
        self.maybe_fetch_models(window, provider, false);
    }

    fn maybe_fetch_models(&mut self, window: &mut Window, provider: Provider, force: bool) {
        let idx = provider_index(provider);
        if !force && self.models_fetched[idx] {
            return;
        }
        if self.proxy_base.is_some() || !self.vault.has_key(provider) {
            return;
        }
        match provider {
            Provider::Codex if self.vault.is_oauth(Provider::Codex) => {
                self.models_fetched[idx] = true;
                if let Some(list) = self.fetch_codex_models(window, false) {
                    if !list.is_empty() {
                        self.models_by_provider[idx] = list;
                        self.status = format!(
                            "Loaded {} Codex models.",
                            self.models_by_provider[idx].len()
                        );
                    }
                }
            }
            _ => {
                self.models_fetched[idx] = true;
            }
        }
    }

    fn fetch_codex_models(
        &mut self,
        window: &mut Window,
        retried: bool,
    ) -> Option<Vec<ui::ModelInfo>> {
        let mut auth_header = self
            .vault
            .with_key(Provider::Codex, |tok| format!("Bearer {}", tok.trim()))?;
        let acct = self.vault.codex_account_id().map(String::from);
        let mut headers: Vec<(&str, &str)> = vec![
            ("Authorization", auth_header.as_str()),
            ("originator", "codex-tui"),
            ("Accept", "application/json"),
        ];
        if let Some(ref aid) = acct {
            headers.push(("chatgpt-account-id", aid.as_str()));
        }
        let req = sex_fetch::Request {
            method: "GET",
            url: CODEX_MODELS_URL,
            headers: &headers,
            body: &[],
            trusted_origin: false,
            referrer: None,
        };
        let resp_res = net::request(window, &req, None, MODEL_DISCOVERY_MAX_BYTES);
        unsafe { zeroize_slice(auth_header.as_bytes_mut()) };

        let resp = resp_res.ok()?;
        if resp.head.status == 401 && !retried && self.vault.has_refresh_token(Provider::Codex) {
            if self.try_refresh_oauth(window, Provider::Codex).is_ok() {
                return self.fetch_codex_models(window, true);
            }
            return None;
        }
        if !(200..300).contains(&resp.head.status) {
            return None;
        }
        let body_str = core::str::from_utf8(&resp.body).ok()?;
        let root = json::parse(body_str).ok()?;
        let items = root
            .get("models")
            .or_else(|| root.get("data"))
            .and_then(JsonValue::as_array)?;
        let mut out = Vec::new();
        for item in items {
            let Some(id) = non_empty_str(item.get("slug").or_else(|| item.get("id"))) else {
                continue;
            };
            let display_name = non_empty_str(
                item.get("display_name")
                    .or_else(|| item.get("title"))
                    .or_else(|| item.get("name")),
            )
            .unwrap_or(id);
            let tag = non_empty_str(item.get("tag").or_else(|| item.get("badge")))
                .map(String::from);
            if !out.iter().any(|m: &ui::ModelInfo| m.id == id) {
                out.push(ui::ModelInfo {
                    id: String::from(id),
                    display_name: String::from(display_name),
                    tag,
                });
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn refresh_workspace_files(&mut self, window: &mut Window) {
        let path = if self.workspace_root.is_empty() {
            String::from(".")
        } else {
            self.workspace_root.clone()
        };
        if let Ok(entries) = files::list(window, path.as_bytes()) {
            let mut list: Vec<String> = Vec::new();
            if !self.workspace_root.is_empty()
                && self.workspace_root != "/"
                && self.workspace_root != "@/"
            {
                list.push(String::from("../"));
            }
            let mut items: Vec<String> = entries
                .into_iter()
                .map(|e| {
                    let name = String::from_utf8_lossy(&e.name).into_owned();
                    if e.dir {
                        format!("{}/", name)
                    } else {
                        name
                    }
                })
                .filter(|n| n != "./" && n != "../")
                .collect();
            items.sort();
            list.extend(items);
            self.workspace_files = list;
        } else {
            self.workspace_files.clear();
        }
    }

    fn jump_to_space(&mut self, window: &mut Window, path: &str) {
        self.workspace_root = String::from(path);
        self.active_mut().workspace_dir = String::from(path);
        if self.access_scope == AccessScope::WorkspaceOnly
            && (path.starts_with('/') || path.starts_with('@'))
        {
            self.set_access_scope(AccessScope::FullSystem);
        }
        self.refresh_workspace_files(window);
        self.status = format!(
            "Space: {} ({} entries)",
            self.workspace_root,
            self.workspace_files.len()
        );
    }

    fn open_workspace_file(&mut self, window: &mut Window, name: &str) {
        if name == "../" {
            let parent = parent_sex_path(&self.workspace_root);
            self.jump_to_space(window, &parent);
            return;
        }
        if let Some(dir_name) = name.strip_suffix('/') {
            let next_dir = join_sex_path(&self.workspace_root, dir_name);
            self.workspace_root = next_dir.clone();
            self.active_mut().workspace_dir = next_dir;
            self.refresh_workspace_files(window);
            self.status = format!(
                "Entered {} ({} entries)",
                self.workspace_root,
                self.workspace_files.len()
            );
            return;
        }
        let content_res = {
            let mut ws = SexWorkspace {
                window,
                root: &self.workspace_root,
                scope: self.access_scope,
            };
            ws.read_file(name)
        };
        match content_res {
            Ok(content) => {
                self.open_file_name = Some(String::from(name));
                self.open_file_content = content;
                self.center_tab = ui::CenterTab::File;
                self.center_scroll = 0;
                self.status = format!("Opened {}", name);
            }
            Err(e) => {
                self.status = format!("Read error: {}", e);
            }
        }
    }

    fn create_manual_checkpoint(&mut self, window: &mut Window) {
        let mut paths: Vec<String> = Vec::new();
        if let Some(ref open) = self.open_file_name {
            paths.push(open.clone());
        } else {
            for f in &self.workspace_files {
                if !f.ends_with('/') {
                    paths.push(f.clone());
                    if paths.len() >= 8 {
                        break;
                    }
                }
            }
        }
        let label = if let Some(ref open) = self.open_file_name {
            format!("checkpoint {}", open)
        } else {
            format!(
                "checkpoint {}",
                if self.workspace_root.is_empty() {
                    "."
                } else {
                    self.workspace_root.as_str()
                }
            )
        };
        let scope = self.access_scope;
        let root = self.workspace_root.clone();
        let idx = self.active_session;
        let id = {
            let mut ws = SexWorkspace {
                window,
                root: &root,
                scope,
            };
            self.sessions[idx]
                .agent
                .create_checkpoint(&label, &paths, &mut ws)
        };
        self.status = format!("Created snapshot #{} ({} file(s)).", id, paths.len());
        self.push_log(&format!("[tailor] {}", self.status));
    }

    fn view_snapshot_diff(&mut self, id: u64) {
        let snap = self.agent().snapshots().iter().find(|s| s.id == id).cloned();
        let Some(snap) = snap else {
            self.status = format!("Snapshot #{} not found.", id);
            return;
        };
        let diff = snap.unified_diff();
        self.active_diff_title = snap.summary();
        self.active_diff_content = if diff.is_empty() {
            format!("Snapshot #{} ({}) — no file line changes recorded.", snap.id, snap.label)
        } else {
            diff
        };
        self.center_tab = ui::CenterTab::Diff;
        self.center_scroll = 0;
        self.status = format!("Viewing diff for snapshot #{}", id);
    }

    fn rollback_snapshot(&mut self, window: &mut Window, id: u64) {
        let scope = self.access_scope;
        let root = self.workspace_root.clone();
        let idx = self.active_session;
        let res = {
            let mut ws = SexWorkspace {
                window,
                root: &root,
                scope,
            };
            self.sessions[idx].agent.rollback_snapshot(id, &mut ws)
        };
        match res {
            Ok(msg) => {
                self.status = msg.clone();
                self.push_log(&format!("[tailor] {}", msg));
                self.refresh_workspace_files(window);
                if let Some(open) = self.open_file_name.clone() {
                    self.open_workspace_file(window, &open);
                }
            }
            Err(err) => {
                self.status = format!("Rollback failed: {}", err);
                self.push_log(&format!("[error] {}", self.status));
            }
        }
    }

    fn artifact_names(&self) -> Vec<String> {
        let mut names = vec![
            String::from("task.md"),
            String::from("implementation_plan.md"),
            String::from("walkthrough.md"),
        ];
        for (k, _) in self.agent().artifacts() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        names
    }

    fn artifact_content(&self, name: &str) -> String {
        for (k, v) in self.agent().artifacts() {
            if k == name || k.ends_with(name) {
                return v.clone();
            }
        }
        String::new()
    }

    fn start_model_turn(&mut self, window: &mut Window) {
        let idx = self.active_session;
        self.start_model_turn_inner(window, idx, false);
    }

    fn start_model_turn_inner(
        &mut self,
        window: &mut Window,
        session_idx: usize,
        retried_after_refresh: bool,
    ) {
        let provider = self.sessions[session_idx].agent.provider();
        let mut req = if provider == Provider::Antigravity {
            // ToS-compliant Antigravity: route ONLY via user's official LAN `agy acp` bridge
            // (`acp.conf` or `/proxy`), never direct unofficial `v1internal` OAuth spoofing.
            let Some(bridge_url) = self
                .acp_bridge_url
                .clone()
                .or_else(|| self.proxy_base.clone())
            else {
                self.status = String::from(
                    "Antigravity requires an official ACP bridge: run 'agy acp' on your Mac and set /acp <url> (or acp.conf), or use Gemini API key.",
                );
                self.push_log_for(session_idx, &format!("[tailor] {}", self.status));
                return;
            };
            let sess = &self.sessions[session_idx];
            let base_req = build_http_request(
                Provider::Gemini,
                sess.agent.model(),
                "",
                sess.agent.system_prompt(),
                sess.agent.history(),
                sess.agent.tools(),
            );
            match self
                .vault
                .with_proxy_key(|pk| base_req.clone().with_proxy_base(&bridge_url, Some(pk)))
            {
                Some(rewritten) => rewritten,
                None => base_req.with_proxy_base(&bridge_url, None),
            }
        } else if provider == Provider::Codex
            && self.proxy_base.is_none()
            && self.vault.is_oauth(Provider::Codex)
        {
            let sess = &self.sessions[session_idx];
            let effective_model = match sess.agent.model() {
                "gpt-4.1" | "o4-mini" => "gpt-5.1-codex",
                other => other,
            };
            let Some(r) = self.vault.with_key(Provider::Codex, |key| {
                build_codex_responses_request(
                    effective_model,
                    key,
                    self.vault.codex_account_id(),
                    sess.agent.system_prompt(),
                    sess.agent.history(),
                    sess.agent.tools(),
                )
            }) else {
                return;
            };
            r
        } else if let Some(r) = self.vault.with_key(provider, |key| {
            self.sessions[session_idx].agent.build_request(key)
        }) {
            r
        } else if self.proxy_base.is_some() {
            // Proxy mode allows providers without individual API keys.
            self.sessions[session_idx].agent.build_request("")
        } else {
            self.status = format!(
                "No {} key/OAuth. Drop credentials.json or {} in /data/tailor (or Ventoy), or /key {} <secret>",
                provider.display_name(),
                provider.key_filename(),
                provider.name(),
            );
            return;
        };

        if provider == Provider::Codex && !self.vault.is_oauth(Provider::Codex) {
            req = req.with_chatgpt_account_id(self.vault.codex_account_id());
        }

        if provider != Provider::Antigravity {
            if let Some(ref proxy_url) = self.proxy_base {
                req = match self
                    .vault
                    .with_proxy_key(|pk| req.clone().with_proxy_base(proxy_url, Some(pk)))
                {
                    Some(rewritten) => rewritten,
                    None => req.with_proxy_base(proxy_url, Some("cliproxy")),
                };
            }
        }

        match net::Duplex::start(window, &req.host, req.port, req.tls, req.http_bytes) {
            Ok(duplex) => {
                if !retried_after_refresh {
                    self.push_delta_for(session_idx, &format!("\n[{}] ", provider.display_name()));
                }
                let model_name = String::from(self.sessions[session_idx].agent.model());
                self.stream = Some(ActiveStream {
                    session_idx,
                    provider,
                    duplex,
                    parser: ProviderStreamParser::new(provider),
                    rx_buf: Vec::with_capacity(4096),
                    retried_after_refresh,
                });
                self.status = format!(
                    "Streaming from {} ({})... Ctrl+C to cancel.",
                    provider.display_name(),
                    model_name
                );
            }
            Err(net::Error::NotAllowed) => {
                self.status = String::from("Network not granted. Launch with: run +net tailor");
            }
            Err(e) => {
                self.status = format!("Connection failed: {:?}", e);
            }
        }
    }

    fn try_refresh_oauth(&mut self, window: &mut Window, provider: Provider) -> Result<(), String> {
        let refresh_req = match provider {
            Provider::Claude => self
                .vault
                .with_refresh_token(Provider::Claude, build_claude_refresh_request),
            Provider::Codex => self
                .vault
                .with_refresh_token(Provider::Codex, build_codex_refresh_request),
            _ => None,
        }
        .ok_or_else(|| String::from("No refresh_token available"))?;

        let old_refresh = self
            .vault
            .with_refresh_token(provider, |s| String::from(s));
        let old_account_id = match provider {
            Provider::Codex => self.vault.codex_account_id().map(String::from),
            _ => None,
        };

        let headers = [
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ];
        let req = sex_fetch::Request {
            method: "POST",
            url: &refresh_req.url,
            headers: &headers,
            body: refresh_req.body.as_bytes(),
            trusted_origin: false,
            referrer: None,
        };
        let resp = net::request(window, &req, None, OAUTH_REFRESH_MAX_BYTES)
            .map_err(|e| format!("OAuth refresh request failed: {:?}", e))?;
        if !(200..300).contains(&resp.head.status) {
            return Err(format!("OAuth refresh returned HTTP {}", resp.head.status));
        }
        let body_str = core::str::from_utf8(&resp.body)
            .map_err(|_| String::from("OAuth refresh response was not UTF-8"))?;
        let mut parsed = match provider {
            Provider::Claude => parse_claude_oauth_json(body_str),
            Provider::Codex => parse_codex_oauth_json(body_str),
            _ => None,
        }
        .ok_or_else(|| String::from("OAuth refresh response missing access_token"))?;

        if parsed.refresh_token.is_none() {
            parsed.refresh_token = old_refresh;
        }
        if parsed.account_id.is_none() {
            parsed.account_id = old_account_id;
        }

        self.vault
            .store_oauth(provider, &parsed)
            .map_err(String::from)?;

        let filename = match provider {
            Provider::Claude => "claude_oauth.json",
            Provider::Codex => "codex_oauth.json",
            _ => return Ok(()),
        };
        let mut json_bytes = parsed.to_json().into_bytes();
        let _ = write_config_file(window, filename, None, &json_bytes);
        zeroize_slice(&mut json_bytes);
        Ok(())
    }

    fn drive_tools(&mut self, window: &mut Window, session_idx: usize) {
        let mut mutated = false;
        loop {
            let root = self.sessions[session_idx].workspace_dir.clone();
            let scope = self.access_scope;
            let step = {
                let mut ws = SexWorkspace {
                    window,
                    root: &root,
                    scope,
                };
                self.sessions[session_idx]
                    .agent
                    .step_tools_fs(&mut ws, &mut self.runner)
            };
            self.drain_spawned_subagents(session_idx);

            match step {
                AgentStep::ToolExecuted(res) => {
                    if matches!(
                        res.name.as_str(),
                        "write_file"
                            | "write_to_file"
                            | "patch_file"
                            | "replace_file_content"
                            | "delete_file"
                            | "move_file"
                    ) {
                        mutated = true;
                    }
                    let tag = if res.is_error { "ERR" } else { "OK" };
                    let preview = if res.output.len() > 240 {
                        let mut end = 240;
                        while end > 0 && !res.output.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}...", &res.output[..end])
                    } else {
                        res.output.clone()
                    };
                    self.push_log_for(session_idx, &format!("[tool:{} {}] {}", res.name, tag, preview));
                }
                AgentStep::AwaitingApproval(pending) => {
                    self.push_log_for(session_idx, &format!("\n[APPROVAL REQUIRED] {}", pending.summary));
                    if let Some(ref diff) = pending.diff {
                        self.active_diff_title = pending.summary.clone();
                        self.active_diff_content = diff.clone();
                        self.center_tab = ui::CenterTab::Diff;
                        self.center_scroll = 0;
                        let mut shown = 0usize;
                        let mut extra = 0usize;
                        for line in diff.lines() {
                            if shown < MAX_DIFF_PREVIEW_LINES {
                                self.push_log_for(session_idx, line);
                                shown += 1;
                            } else {
                                extra += 1;
                            }
                        }
                        if extra > 0 {
                            self.push_log_for(
                                session_idx,
                                &format!("... (+{} more diff lines)", extra),
                            );
                        }
                    }
                    self.push_log_for(
                        session_idx,
                        "Type /approve (or y) to allow, /deny (or n) to reject.",
                    );
                    self.status = format!("Awaiting approval: {}", pending.summary);
                    if mutated && session_idx == self.active_session {
                        self.refresh_workspace_files(window);
                    }
                    return;
                }
                AgentStep::NeedsModelTurn => {
                    if mutated && session_idx == self.active_session {
                        self.refresh_workspace_files(window);
                    }
                    self.start_model_turn_inner(window, session_idx, false);
                    return;
                }
                AgentStep::Idle => {
                    if mutated && session_idx == self.active_session {
                        self.refresh_workspace_files(window);
                    }
                    self.status = String::from("Ready.");
                    return;
                }
            }
        }
    }

    fn poll_stream(&mut self, window: &mut Window) -> bool {
        let Some(mut active) = self.stream.take() else {
            return false;
        };
        let sidx = active.session_idx;
        let mut changed = false;
        active.rx_buf.clear();
        let poll_res = active.duplex.poll(window, &mut active.rx_buf);
        let mut done = false;
        let can_auto_refresh = !active.retried_after_refresh
            && matches!(active.provider, Provider::Claude | Provider::Codex)
            && self.vault.has_refresh_token(active.provider);

        if !active.rx_buf.is_empty() {
            changed = true;
            let events = active.parser.feed(&active.rx_buf);
            let will_refresh = can_auto_refresh && active.parser.http_status() == Some(401);
            for ev in events {
                if will_refresh && matches!(ev, StreamEvent::Error(_)) {
                    done = true;
                } else if self.apply_stream_event(sidx, &ev) {
                    done = true;
                }
            }
        }

        match poll_res {
            Ok(_) if !done && !active.parser.is_complete() => {
                self.stream = Some(active);
            }
            Ok(_) | Err(_) => {
                let tail = active.parser.finish();
                let will_refresh = can_auto_refresh && active.parser.http_status() == Some(401);
                if will_refresh {
                    active.duplex.close(window);
                    self.status = format!(
                        "Refreshing {} OAuth token...",
                        active.provider.display_name()
                    );
                    self.push_log_for(sidx, &format!("\n[tailor] {}", self.status));
                    changed = true;
                    match self.try_refresh_oauth(window, active.provider) {
                        Ok(()) => {
                            self.push_log_for(sidx, "[tailor] OAuth token refreshed; retrying...");
                            self.start_model_turn_inner(window, sidx, true);
                        }
                        Err(err) => {
                            self.push_log_for(sidx, &format!("[error] {}", err));
                            self.status = format!("Error: {}", err);
                            let _ = self.sessions[sidx].agent.finish_model_turn();
                        }
                    }
                    return changed;
                }

                for ev in tail {
                    self.apply_stream_event(sidx, &ev);
                }
                active.duplex.close(window);
                self.push_delta_for(sidx, "\n");
                changed = true;
                match self.sessions[sidx].agent.finish_model_turn() {
                    AgentStep::NeedsModelTurn => self.drive_tools(window, sidx),
                    AgentStep::Idle => {
                        self.status = String::from("Ready.");
                    }
                    _ => {}
                }
            }
        }
        changed
    }

    fn apply_stream_event(&mut self, session_idx: usize, ev: &StreamEvent) -> bool {
        self.sessions[session_idx].agent.on_stream_event(ev);
        match ev {
            StreamEvent::TextDelta(t) => {
                self.push_delta_for(session_idx, t);
                false
            }
            StreamEvent::ThinkingDelta(t) => {
                self.status = format!("Thinking ({} chars)...", t.len());
                false
            }
            StreamEvent::ThoughtSignature(_) => false,
            StreamEvent::ToolCall { name, .. } => {
                self.push_log_for(session_idx, &format!("\n[requesting tool: {}]", name));
                false
            }
            StreamEvent::Done { .. } => true,
            StreamEvent::Error(msg) => {
                self.push_log_for(session_idx, &format!("\n[error] {}", msg));
                self.status = format!("Error: {}", msg);
                true
            }
        }
    }

    fn handle_submit(&mut self, window: &mut Window) {
        let line = core::mem::take(&mut self.input);
        let trimmed = line.trim();
        let idx = self.active_session;
        if trimmed.is_empty() {
            // If active session is a newly spawned subagent with a pending prompt, run it on Enter!
            if let Some(pending_prompt) = self.sessions[idx].pending_prompt.take() {
                self.push_log_for(idx, &format!("\n> {}", pending_prompt));
                self.sessions[idx].agent.push_user_message(&pending_prompt);
                self.start_model_turn_inner(window, idx, false);
            }
            return;
        }
        self.scroll_lines = 0;

        // Shortcut y/n when an approval gate is active.
        if self.agent().pending_approval().is_some() {
            let root = self.sessions[idx].workspace_dir.clone();
            let scope = self.access_scope;
            if trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("/approve") {
                let step = {
                    let mut ws = SexWorkspace {
                        window,
                        root: &root,
                        scope,
                    };
                    self.sessions[idx].agent.resolve_approval_fs(
                        ApprovalDecision::Approved,
                        &mut ws,
                        &mut self.runner,
                    )
                };
                self.drain_spawned_subagents(idx);
                if let AgentStep::ToolExecuted(res) = step {
                    let tag = if res.is_error { "ERR" } else { "OK" };
                    self.push_log_for(idx, &format!("[approved {} {}] {}", res.name, tag, res.output));
                    self.refresh_workspace_files(window);
                }
                self.drive_tools(window, idx);
                return;
            }
            if trimmed.eq_ignore_ascii_case("n")
                || trimmed.eq_ignore_ascii_case("/deny")
                || trimmed.starts_with("/deny ")
            {
                let reason = trimmed.strip_prefix("/deny").unwrap_or("").trim();
                let step = {
                    let mut ws = SexWorkspace {
                        window,
                        root: &root,
                        scope,
                    };
                    self.sessions[idx].agent.resolve_approval_fs(
                        ApprovalDecision::Denied(String::from(reason)),
                        &mut ws,
                        &mut self.runner,
                    )
                };
                if let AgentStep::ToolExecuted(res) = step {
                    self.push_log_for(idx, &format!("[denied {}] {}", res.name, res.output));
                }
                self.drive_tools(window, idx);
                return;
            }
        }

        if let Some(cmd) = trimmed.strip_prefix('/') {
            self.handle_command(window, cmd);
            return;
        }

        self.sessions[idx].pending_prompt = None;
        self.push_log_for(idx, &format!("\n> {}", trimmed));
        self.sessions[idx].agent.push_user_message(trimmed);
        self.start_model_turn(window);
    }

    fn handle_command(&mut self, window: &mut Window, cmd: &str) {
        let mut parts = cmd.splitn(2, ' ');
        let verb = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("").trim();

        match verb {
            "provider" => match Provider::from_name(rest) {
                Some(p) => {
                    self.switch_provider(window, p);
                }
                None => {
                    self.status = String::from(
                        "Usage: /provider gemini|antigravity|claude|claude-deepseek|codex",
                    );
                }
            },
            "model" => {
                if rest.is_empty() {
                    self.status = format!("Active model: {}", self.agent().model());
                } else {
                    self.agent_mut().set_model(rest);
                    self.status = format!("Model set to {}", self.agent().model());
                    self.push_log(&format!("[tailor] {}", self.status));
                }
            }
            "scope" => {
                if rest.eq_ignore_ascii_case("full") || rest.eq_ignore_ascii_case("system") {
                    self.set_access_scope(AccessScope::FullSystem);
                } else if rest.eq_ignore_ascii_case("workspace") || rest.eq_ignore_ascii_case("ws")
                {
                    self.set_access_scope(AccessScope::WorkspaceOnly);
                } else {
                    self.status = format!(
                        "Access Scope: {} (usage: /scope workspace | /scope full)",
                        self.access_scope.label()
                    );
                }
            }
            "acp" => {
                if rest.is_empty() {
                    self.status = match &self.acp_bridge_url {
                        Some(url) => format!("ACP Bridge active: {} (use /acp off to disable)", url),
                        None => String::from(
                            "ACP Bridge is off. Usage: /acp http://<mac-lan-ip>:8317 | /acp off",
                        ),
                    };
                } else if rest.eq_ignore_ascii_case("off")
                    || rest.eq_ignore_ascii_case("none")
                    || rest.eq_ignore_ascii_case("clear")
                {
                    self.acp_bridge_url = None;
                    let _ = write_config_file(window, "acp.conf", Some("tailor-acp.conf"), b"");
                    self.status = String::from("ACP Bridge disabled (cleared acp.conf).");
                    self.push_log(&format!("[tailor] {}", self.status));
                } else {
                    let normalized = if rest.starts_with("http://") || rest.starts_with("https://")
                    {
                        String::from(rest.trim_end_matches('/'))
                    } else {
                        format!("http://{}", rest.trim_end_matches('/'))
                    };
                    let saved = write_config_file(
                        window,
                        "acp.conf",
                        Some("tailor-acp.conf"),
                        normalized.as_bytes(),
                    );
                    self.acp_bridge_url = Some(normalized.clone());
                    self.status = if let Some(where_saved) = saved {
                        format!("ACP Bridge set to {} (saved to {}).", normalized, where_saved)
                    } else {
                        format!("ACP Bridge set to {} (RAM only).", normalized)
                    };
                    self.push_log(&format!("[tailor] {}", self.status));
                }
            }
            "proxy" => {
                if rest.is_empty() {
                    self.status = match &self.proxy_base {
                        Some(url) => format!("Proxy active: {} (use /proxy off to disable)", url),
                        None => String::from(
                            "Proxy is off. Usage: /proxy http://127.0.0.1:8317 | /proxy off",
                        ),
                    };
                } else if rest.eq_ignore_ascii_case("off")
                    || rest.eq_ignore_ascii_case("none")
                    || rest.eq_ignore_ascii_case("clear")
                {
                    self.proxy_base = None;
                    let _ = write_config_file(window, "proxy.conf", None, b"");
                    self.status = String::from("Proxy disabled (cleared proxy.conf).");
                    self.push_log(&format!("[tailor] {}", self.status));
                } else {
                    let normalized = if rest.starts_with("http://") || rest.starts_with("https://")
                    {
                        String::from(rest.trim_end_matches('/'))
                    } else {
                        format!("http://{}", rest.trim_end_matches('/'))
                    };
                    let saved =
                        write_config_file(window, "proxy.conf", None, normalized.as_bytes());
                    self.proxy_base = Some(normalized.clone());
                    self.status = if let Some(where_saved) = saved {
                        format!("Proxy set to {} (saved to {}).", normalized, where_saved)
                    } else {
                        format!("Proxy set to {} (RAM only).", normalized)
                    };
                    self.push_log(&format!("[tailor] {}", self.status));
                }
            }
            "key" => {
                let mut kp = rest.splitn(2, ' ');
                let prov_str = kp.next().unwrap_or("").trim();
                let secret = kp.next().unwrap_or("").trim();
                match (Provider::from_name(prov_str), secret.is_empty()) {
                    (Some(p), false) => match self.vault.store(p, secret.as_bytes()) {
                        Ok(()) => {
                            let saved = self.persist_key(window, p, secret.as_bytes());
                            self.status = if let Some(where_saved) = saved {
                                format!(
                                    "Stored {} key in Shibari vault & saved to {}.",
                                    p.display_name(),
                                    where_saved
                                )
                            } else {
                                format!("Stored {} key in Shibari RAM vault.", p.display_name())
                            };
                            self.push_log(&format!("[tailor] {}", self.status));
                        }
                        Err(e) => self.status = String::from(e),
                    },
                    _ => {
                        self.status = String::from(
                            "Usage: /key gemini|claude|claude-deepseek|codex <secret>",
                        );
                    }
                }
            }
            "clear" => {
                self.active_mut().agent.clear();
                self.active_mut().transcript.clear();
                self.status = String::from("Conversation cleared.");
            }
            "approve" | "deny" => {
                self.status = String::from("No tool call is currently awaiting approval.");
            }
            "help" => {
                self.push_log(
                    "Commands:\n\
                     /provider gemini|antigravity|claude|claude-deepseek|codex  Switch AI provider\n\
                     /model <name>                   Override model (e.g. gemini-2.5-pro, gpt-6.2-sol)\n\
                     /scope workspace | /scope full  Toggle Workspace vs Full System SexOS access\n\
                     /acp <url> | /acp off           Connect official LAN Antigravity ACP bridge (acp.conf)\n\
                     /proxy <url> | /proxy off       Route via proxy (e.g. http://127.0.0.1:8317)\n\
                     /key <provider> <secret>        Store & persist API key (gemini, claude, codex, deepseek)\n\
                     /approve (or y)                 Approve pending mutating tool\n\
                     /deny [reason] (or n)           Deny pending mutating tool\n\
                     /clear                          Clear active session history\n\
                     Auto-load (/data/tailor/ or @/media/Ventoy/): credentials.json, gemini.key, acp.conf, claude_oauth.json, codex_oauth.json\n\
                     Up/Down scroll · Ctrl+C cancel stream · F11 close",
                );
            }
            _ => {
                self.status = format!("Unknown command '/{}'. Try /help.", verb);
            }
        }
    }

    fn persist_key(&self, window: &mut Window, provider: Provider, secret: &[u8]) -> Option<String> {
        let ventoy_alias = format!("tailor-{}", provider.key_filename());
        write_config_file(window, provider.key_filename(), Some(&ventoy_alias), secret)
    }

    fn render(&mut self, window: &mut Window) {
        let p = self.agent().provider();
        let providers = self.provider_statuses();
        let model_display = self.active_model_display();
        let artifact_names = self.artifact_names();
        let active_artifact_content = self
            .active_artifact_name
            .as_deref()
            .map(|n| self.artifact_content(n))
            .unwrap_or_default();
        let session_infos = self.session_infos();
        let pidx = provider_index(p);
        let sess = &self.sessions[self.active_session];
        let state = ui::RenderState {
            provider: p,
            model_id: sess.agent.model(),
            model_display: &model_display,
            approval_policy: sess.agent.approval_policy(),
            access_scope: self.access_scope,
            providers: &providers,
            models: &self.models_by_provider[pidx],
            proxy_base: self.proxy_base.as_deref(),
            acp_bridge_url: self.acp_bridge_url.as_deref(),
            streaming: self.stream.is_some(),
            task_boundary: sess.agent.task_boundary(),
            pending_approval: sess.agent.pending_approval().map(|pa| pa.summary.as_str()),
            transcript: &sess.transcript,
            scroll_lines: self.scroll_lines,
            input: &self.input,
            input_focused: self.input_focused,
            input_selected: self.input_selected,
            status: &self.status,
            workspace_root: &self.workspace_root,
            left_tab: self.left_tab,
            center_tab: self.center_tab,
            sessions: &session_infos,
            workspace_files: &self.workspace_files,
            open_file_name: self.open_file_name.as_deref(),
            open_file_content: &self.open_file_content,
            snapshots: sess.agent.snapshots(),
            artifact_names: &artifact_names,
            active_artifact_name: self.active_artifact_name.as_deref(),
            active_artifact_content: &active_artifact_content,
            active_diff_title: &self.active_diff_title,
            active_diff_content: &self.active_diff_content,
            center_scroll: self.center_scroll,
            model_picker_open: self.model_picker_open,
            settings_open: self.settings_open,
        };
        self.ui.render(window, &state);
    }
}

fn parse_launch_args(raw: &[u8]) -> (String, Option<Provider>) {
    let text = core::str::from_utf8(raw).unwrap_or("");
    let mut workspace = String::new();
    let mut provider = None;
    for word in text.split_whitespace() {
        if let Some(p) = Provider::from_name(word) {
            provider = Some(p);
        } else if workspace.is_empty() {
            workspace = String::from(word);
        }
    }
    (workspace, provider)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Tailor");

    let mut args_buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let (workspace_root, arg_provider) = parse_launch_args(files::args(&mut window, &mut args_buf));
    let boot_provider = arg_provider.unwrap_or(Provider::Gemini);

    let owner = Owner(window.target);
    let vault = KeyVault::new(owner, sex_pdx::get_ticks() ^ 0x5A17_7A11_0001);

    let mut agent = Agent::new(boot_provider);
    agent.set_model(default_model_for_provider(boot_provider, false));
    let root_session = AgentSession {
        title: String::from("Main Agent"),
        agent,
        transcript: String::new(),
        workspace_dir: workspace_root.clone(),
        pending_prompt: None,
    };

    let mut app = AppState {
        workspace_root,
        access_scope: AccessScope::WorkspaceOnly,
        sessions: vec![root_session],
        active_session: 0,
        next_session_num: 2,
        vault,
        proxy_base: None,
        acp_bridge_url: None,
        runner: SexToolRunner::new(),
        input: String::new(),
        input_focused: true,
        input_selected: false,
        status: String::from("Loading keys & OAuth into Shibari vault..."),
        scroll_lines: 0,
        center_scroll: 0,
        stream: None,
        ui: ui::TailorUi::new(),
        left_tab: ui::LeftTab::Files,
        center_tab: ui::CenterTab::Artifact,
        workspace_files: Vec::new(),
        open_file_name: None,
        open_file_content: String::new(),
        active_artifact_name: None,
        active_diff_title: String::new(),
        active_diff_content: String::new(),
        model_picker_open: false,
        settings_open: false,
        models_by_provider: [
            curated_models(Provider::Antigravity),
            curated_models(Provider::Codex),
            curated_models(Provider::Claude),
            curated_models(Provider::ClaudeDeepSeek),
            curated_models(Provider::Gemini),
        ],
        models_fetched: [false; 5],
        last_pointer_x: 0,
        last_pointer_y: 0,
    };
    app.push_log("Tailor — Native Inside-Out Multi-Provider AI Coding Agent for SexOS.");
    app.push_log("Use Gemini API key, Codex/Claude/DeepSeek, or Antigravity (ACP Bridge via /acp).");
    app.render(&mut window);

    let (loaded, proxy_base, acp_bridge_url) = app.vault.load_from_files(&mut window);
    app.proxy_base = proxy_base;
    app.acp_bridge_url = acp_bridge_url;

    // Auto-select first connected provider when no explicit provider CLI arg was passed.
    if arg_provider.is_none() {
        let mut selected = None;
        for cand in [
            Provider::Gemini,
            Provider::Codex,
            Provider::Claude,
            Provider::ClaudeDeepSeek,
        ] {
            if app.vault.has_key(cand) {
                selected = Some(cand);
                break;
            }
        }
        if selected.is_none() && (app.acp_bridge_url.is_some() || app.proxy_base.is_some()) {
            selected = Some(Provider::Antigravity);
        }
        if let Some(cand) = selected {
            let def_m = default_model_for_provider(cand, app.vault.is_oauth(cand));
            app.agent_mut().set_provider(cand);
            app.agent_mut().set_model(def_m);
        }
    } else {
        let p = app.agent().provider();
        let def_m = default_model_for_provider(p, app.vault.is_oauth(p));
        app.agent_mut().set_model(def_m);
    }

    app.refresh_workspace_files(&mut window);
    app.status = if let Some(ref url) = app.acp_bridge_url {
        format!(
            "Ready (ACP Bridge: {}, {} key(s) in Shibari). Type a prompt or /help.",
            url, loaded
        )
    } else if let Some(ref url) = app.proxy_base {
        format!(
            "Ready (Proxy: {}, {} key(s) in Shibari). Type a prompt or /help.",
            url, loaded
        )
    } else {
        format!(
            "Ready ({} key(s) in Shibari). Type a prompt or /help.",
            loaded
        )
    };
    app.render(&mut window);

    sex_pdx::serial_record!(
        "[tailor.ready] target={} task={} provider={} model={} keys={} proxy={} acp={}",
        window.target,
        window.task,
        app.agent().provider().name(),
        app.agent().model(),
        loaded,
        app.proxy_base.as_deref().unwrap_or("none"),
        app.acp_bridge_url.as_deref().unwrap_or("none")
    );

    loop {
        let mut dirty = false;
        let timeout = if app.stream.is_some() { 20 } else { 250 };
        if let Some(event) = window.next_within(timeout) {
            match event {
                Event::Close => {
                    if let Some(mut active) = app.stream.take() {
                        active.duplex.close(&mut window);
                    }
                    sex_pdx::serial_record!("[tailor.closed]");
                    window.close();
                }
                Event::Resize { .. } => {
                    dirty = true;
                }
                Event::Pointer(p) => {
                    let px = p.x as i32;
                    let py = p.y as i32;
                    app.last_pointer_x = px;
                    app.last_pointer_y = py;
                    match p.kind {
                        sex_pdx::app_pointer::Kind::Down { button: 0 } => {
                            if let Some(hit) =
                                app.ui.hits.iter().rev().find(|h| h.contains(px, py)).copied()
                            {
                                match hit.action {
                                    ui::HitAction::Noop => {}
                                    ui::HitAction::CloseOverlays => {
                                        app.model_picker_open = false;
                                        app.settings_open = false;
                                        dirty = true;
                                    }
                                    ui::HitAction::Provider(prov) => {
                                        app.model_picker_open = false;
                                        app.switch_provider(&mut window, prov);
                                        dirty = true;
                                    }
                                    ui::HitAction::ToggleModelPicker => {
                                        app.settings_open = false;
                                        app.model_picker_open = !app.model_picker_open;
                                        if app.model_picker_open {
                                            let cur_p = app.agent().provider();
                                            app.maybe_fetch_models(&mut window, cur_p, false);
                                        }
                                        dirty = true;
                                    }
                                    ui::HitAction::SelectModel(idx) => {
                                        let pidx = provider_index(app.agent().provider());
                                        if let Some(m) = app.models_by_provider[pidx].get(idx) {
                                            let id = m.id.clone();
                                            let name = m.display_name.clone();
                                            app.agent_mut().set_model(&id);
                                            app.status = format!("Model set to {} ({})", name, id);
                                            app.push_log(&format!("[tailor] {}", app.status));
                                        }
                                        app.model_picker_open = false;
                                        dirty = true;
                                    }
                                    ui::HitAction::RefreshModels => {
                                        let cur_p = app.agent().provider();
                                        app.status = format!(
                                            "Refreshing {} models...",
                                            cur_p.display_name()
                                        );
                                        app.render(&mut window);
                                        app.maybe_fetch_models(&mut window, cur_p, true);
                                        dirty = true;
                                    }
                                    ui::HitAction::CyclePolicy => {
                                        let next = match app.agent().approval_policy() {
                                            ApprovalPolicy::AlwaysAsk => ApprovalPolicy::AutoSafe,
                                            ApprovalPolicy::AutoSafe => ApprovalPolicy::Turbo,
                                            ApprovalPolicy::Turbo => ApprovalPolicy::AlwaysAsk,
                                        };
                                        app.agent_mut().set_approval_policy(next);
                                        let lbl = match next {
                                            ApprovalPolicy::AlwaysAsk => "Ask (all mutating tools)",
                                            ApprovalPolicy::AutoSafe => "Auto (safe tools & artifacts)",
                                            ApprovalPolicy::Turbo => "Turbo (auto-approve all)",
                                        };
                                        app.status = format!("Approval policy: {}", lbl);
                                        dirty = true;
                                    }
                                    ui::HitAction::ToggleAccessScope => {
                                        app.toggle_access_scope();
                                        dirty = true;
                                    }
                                    ui::HitAction::ToggleSettings => {
                                        app.model_picker_open = false;
                                        app.settings_open = !app.settings_open;
                                        dirty = true;
                                    }
                                    ui::HitAction::ReloadCreds => {
                                        let (reloaded, new_proxy, new_acp) =
                                            app.vault.load_from_files(&mut window);
                                        if new_proxy.is_some() {
                                            app.proxy_base = new_proxy;
                                        }
                                        if new_acp.is_some() {
                                            app.acp_bridge_url = new_acp;
                                        }
                                        app.models_fetched = [false; 5];
                                        let cur_p = app.agent().provider();
                                        app.maybe_fetch_models(&mut window, cur_p, true);
                                        app.status = format!(
                                            "Reloaded /data/tailor ({} credential(s) loaded).",
                                            reloaded
                                        );
                                        app.push_log(&format!("[tailor] {}", app.status));
                                        dirty = true;
                                    }
                                    ui::HitAction::ToggleProxy => {
                                        if app.proxy_base.is_some() {
                                            app.proxy_base = None;
                                            let _ = write_config_file(
                                                &mut window,
                                                "proxy.conf",
                                                None,
                                                b"",
                                            );
                                            app.status = String::from("Proxy mode disabled.");
                                        } else {
                                            app.proxy_base = Some(String::from(DEFAULT_PROXY_URL));
                                            let _ = write_config_file(
                                                &mut window,
                                                "proxy.conf",
                                                None,
                                                DEFAULT_PROXY_URL.as_bytes(),
                                            );
                                            app.status = format!(
                                                "Proxy mode enabled ({}).",
                                                DEFAULT_PROXY_URL
                                            );
                                        }
                                        app.push_log(&format!("[tailor] {}", app.status));
                                        dirty = true;
                                    }
                                    ui::HitAction::LeftTab(tab) => {
                                        app.left_tab = tab;
                                        if tab == ui::LeftTab::Files && app.workspace_files.is_empty()
                                        {
                                            app.refresh_workspace_files(&mut window);
                                        }
                                        dirty = true;
                                    }
                                    ui::HitAction::NewSession => {
                                        let new_idx = app.spawn_new_session(None, None);
                                        app.active_session = new_idx;
                                        app.scroll_lines = 0;
                                        app.status = format!(
                                            "Switched to new session '{}'.",
                                            app.active().title
                                        );
                                        dirty = true;
                                    }
                                    ui::HitAction::SelectSession(idx) => {
                                        if idx < app.sessions.len() {
                                            app.active_session = idx;
                                            app.scroll_lines = 0;
                                            let ws_dir = app.active().workspace_dir.clone();
                                            if ws_dir != app.workspace_root {
                                                app.workspace_root = ws_dir;
                                                app.refresh_workspace_files(&mut window);
                                            }
                                            app.status =
                                                format!("Active agent: {}", app.active().title);
                                            dirty = true;
                                        }
                                    }
                                    ui::HitAction::RunSession(idx) => {
                                        if idx < app.sessions.len() {
                                            app.active_session = idx;
                                            app.scroll_lines = 0;
                                            let ws_dir = app.active().workspace_dir.clone();
                                            if ws_dir != app.workspace_root {
                                                app.workspace_root = ws_dir;
                                                app.refresh_workspace_files(&mut window);
                                            }
                                            if app.stream.is_some() {
                                                app.status = String::from(
                                                    "Wait for current stream to finish (or Stop) before running child agent.",
                                                );
                                            } else if let Some(prompt) =
                                                app.sessions[idx].pending_prompt.take()
                                            {
                                                app.push_log_for(idx, &format!("\n> {}", prompt));
                                                app.sessions[idx].agent.push_user_message(&prompt);
                                                app.start_model_turn_inner(&mut window, idx, false);
                                            }
                                            dirty = true;
                                        }
                                    }
                                    ui::HitAction::SelectSpace(idx) => {
                                        if let Some((path, _)) = ui::SPACES.get(idx) {
                                            app.jump_to_space(&mut window, path);
                                            app.left_tab = ui::LeftTab::Files;
                                            dirty = true;
                                        }
                                    }
                                    ui::HitAction::RefreshFiles => {
                                        app.refresh_workspace_files(&mut window);
                                        app.status = format!(
                                            "Synced workspace ({} entries).",
                                            app.workspace_files.len()
                                        );
                                        dirty = true;
                                    }
                                    ui::HitAction::OpenFile(idx) => {
                                        if let Some(name) = app.workspace_files.get(idx).cloned() {
                                            app.open_workspace_file(&mut window, &name);
                                            dirty = true;
                                        }
                                    }
                                    ui::HitAction::CreateCheckpoint => {
                                        app.create_manual_checkpoint(&mut window);
                                        dirty = true;
                                    }
                                    ui::HitAction::ViewSnapshotDiff(id) => {
                                        app.view_snapshot_diff(id);
                                        dirty = true;
                                    }
                                    ui::HitAction::RollbackSnapshot(id) => {
                                        app.rollback_snapshot(&mut window, id);
                                        dirty = true;
                                    }
                                    ui::HitAction::OpenArtifact(idx) => {
                                        let names = app.artifact_names();
                                        if let Some(name) = names.get(idx).cloned() {
                                            app.active_artifact_name = Some(name.clone());
                                            app.center_tab = ui::CenterTab::Artifact;
                                            app.center_scroll = 0;
                                            app.status = format!("Viewing artifact {}", name);
                                            dirty = true;
                                        }
                                    }
                                    ui::HitAction::CenterTab(tab) => {
                                        app.center_tab = tab;
                                        app.center_scroll = 0;
                                        dirty = true;
                                    }
                                    ui::HitAction::QuickPrompt(idx) => {
                                        if app.stream.is_none() {
                                            if let Some(prompt) = ui::QUICK_PROMPTS.get(idx) {
                                                app.input = String::from(*prompt);
                                                app.input_focused = true;
                                                app.input_selected = false;
                                                app.handle_submit(&mut window);
                                                dirty = true;
                                            }
                                        }
                                    }
                                    ui::HitAction::FocusInput => {
                                        app.model_picker_open = false;
                                        app.settings_open = false;
                                        app.input_focused = true;
                                        app.input_selected = false;
                                        dirty = true;
                                    }
                                    ui::HitAction::Approve if app.stream.is_none() => {
                                        app.input = String::from("/approve");
                                        app.handle_submit(&mut window);
                                        dirty = true;
                                    }
                                    ui::HitAction::Deny if app.stream.is_none() => {
                                        app.input = String::from("/deny");
                                        app.handle_submit(&mut window);
                                        dirty = true;
                                    }
                                    ui::HitAction::SendOrStop => {
                                        if let Some(mut active) = app.stream.take() {
                                            active.duplex.close(&mut window);
                                            app.push_log_for(active.session_idx, "\n[stopped]");
                                            app.status = String::from("Stream stopped.");
                                        } else {
                                            app.input_focused = true;
                                            app.input_selected = false;
                                            app.handle_submit(&mut window);
                                        }
                                        dirty = true;
                                    }
                                    ui::HitAction::Clear => {
                                        app.input.clear();
                                        app.input_selected = false;
                                        app.status = String::from("Input cleared.");
                                        dirty = true;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        sex_pdx::app_pointer::Kind::Wheel { steps } => {
                            let (cx, cy, cw, ch) = app.ui.center_rect;
                            let in_center = cw > 0
                                && px >= cx
                                && px < cx + cw
                                && py >= cy
                                && py < cy + ch;
                            if in_center {
                                if steps > 0 {
                                    app.center_scroll = app.center_scroll.saturating_sub(3);
                                } else if steps < 0 {
                                    app.center_scroll = app.center_scroll.saturating_add(3);
                                }
                            } else if steps > 0 {
                                app.scroll_lines = app.scroll_lines.saturating_add(3);
                            } else if steps < 0 {
                                app.scroll_lines = app.scroll_lines.saturating_sub(3);
                            }
                            dirty = true;
                        }
                        _ => {}
                    }
                }
                Event::Key(key) if key.code == 0x01 => {
                    // Escape: close any open popovers/modals
                    if app.model_picker_open || app.settings_open {
                        app.model_picker_open = false;
                        app.settings_open = false;
                        dirty = true;
                    }
                }
                Event::Key(key) if key.control && key.code == 0x1E => {
                    // Ctrl+A: select all prompt text
                    app.input_focused = true;
                    app.input_selected = !app.input.is_empty();
                    dirty = true;
                }
                Event::Key(key) if key.control && key.code == 0x2F => {
                    // Ctrl+V: paste clipboard into prompt
                    if app.stream.is_none() {
                        app.input_focused = true;
                        if let Ok(clip) = sex_app::clipboard::paste(&mut window) {
                            if app.input_selected {
                                app.input.clear();
                                app.input_selected = false;
                            }
                            for ch in clip.chars() {
                                if !ch.is_control() && app.input.len() < 1024 {
                                    app.input.push(ch);
                                }
                            }
                            app.status = String::from("Pasted from clipboard.");
                            dirty = true;
                        }
                    }
                }
                Event::Key(key) if key.control && key.code == 0x2E => {
                    // Ctrl+C: cancel active stream, or copy prompt/transcript to clipboard.
                    if let Some(mut active) = app.stream.take() {
                        active.duplex.close(&mut window);
                        app.push_log_for(active.session_idx, "\n[cancelled]");
                        app.status = String::from("Stream cancelled.");
                    } else if !app.input.is_empty() {
                        let _ = sex_app::clipboard::copy(&mut window, &app.input);
                        app.status = String::from("Copied prompt to clipboard.");
                    } else if !app.active().transcript.is_empty() {
                        let _ = sex_app::clipboard::copy(&mut window, &app.active().transcript);
                        app.status = String::from("Copied transcript to clipboard.");
                    }
                    dirty = true;
                }
                Event::Key(key)
                    if key.code == 0x1C || matches!(sex_app::text(key), Some('\n' | '\r')) =>
                {
                    // Enter
                    if app.model_picker_open || app.settings_open {
                        app.model_picker_open = false;
                        app.settings_open = false;
                        dirty = true;
                    } else if app.stream.is_none() {
                        app.input_focused = true;
                        app.input_selected = false;
                        app.handle_submit(&mut window);
                        dirty = true;
                    }
                }
                Event::Key(key)
                    if key.code == 0x0E || matches!(sex_app::text(key), Some('\x08' | '\x7f')) =>
                {
                    // Backspace
                    if app.stream.is_none() {
                        app.input_focused = true;
                        if app.input_selected {
                            app.input.clear();
                            app.input_selected = false;
                        } else {
                            app.input.pop();
                        }
                        dirty = true;
                    }
                }
                Event::Key(key) => match sex_app::nav(key) {
                    Some(Nav::Up) => {
                        app.scroll_lines = app.scroll_lines.saturating_add(3);
                        dirty = true;
                    }
                    Some(Nav::Down) => {
                        app.scroll_lines = app.scroll_lines.saturating_sub(3);
                        dirty = true;
                    }
                    _ => {
                        if app.stream.is_none() && !app.model_picker_open && !app.settings_open {
                            if let Some(ch) = sex_app::text(key) {
                                if !ch.is_control() && app.input.len() < 1024 {
                                    app.input_focused = true;
                                    if app.input_selected {
                                        app.input.clear();
                                        app.input_selected = false;
                                    }
                                    app.input.push(ch);
                                    dirty = true;
                                }
                            }
                        }
                    }
                },
                _ => {}
            }
        }

        if app.poll_stream(&mut window) {
            dirty = true;
        }
        if dirty {
            app.render(&mut window);
        }
    }
}
