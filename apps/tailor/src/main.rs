// SPDX-License-Identifier: GPL-3.0-only
//! Tailor: SexOS's native multi-provider AI coding agent (Antigravity, Gemini, Claude, Codex, DeepSeek).
//!
//! Launch with network and an optional workspace grant:
//!   `run +net tailor [@/workspace] [antigravity|gemini|claude|claude-deepseek|codex]`
//!
//! Credentials are loaded into `shibari::Keeper` (RAM-only zeroizing secret store) from
//! `/data/tailor/credentials.json` (unified desktop bundle), `google_oauth.json`,
//! `claude_oauth.json`, `codex_oauth.json`, `keys.json`, or per-provider `*.key` files
//! (with `@/media/Ventoy/` fallbacks), or entered interactively with `/key <provider> <secret>`.
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
    build_antigravity_request, build_claude_refresh_request, build_codex_refresh_request,
    build_codex_responses_request, build_google_refresh_request, is_claude_oauth_token, json,
    parse_claude_oauth_json, parse_codex_oauth_json, parse_google_oauth_json, Agent, AgentStep,
    ApprovalDecision, JsonValue, OAuthToken, Provider, ProviderStreamParser, StreamEvent,
    ToolRunner, Workspace,
};

const MAX_READ_BYTES: usize = 256 * 1024;
const READ_CHUNK_BYTES: usize = 32 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 24 * 1024;
const COMMAND_TIMEOUT_TICKS: u64 = 15_000;
const OAUTH_REFRESH_MAX_BYTES: usize = 64 * 1024;
const LOAD_CODE_ASSIST_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const ANTIGRAVITY_USER_AGENT: &str = "antigravity/hub/2.22.0 darwin/arm64";
const DEFAULT_ANTIGRAVITY_PROJECT: &str = "aicode-consumers";
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
    antigravity: Option<Knot>,
    claude: Option<Knot>,
    claude_deepseek: Option<Knot>,
    codex: Option<Knot>,
    antigravity_refresh: Option<Knot>,
    antigravity_client_secret: Option<Knot>,
    claude_refresh: Option<Knot>,
    codex_refresh: Option<Knot>,
    proxy_key: Option<Knot>,
    antigravity_oauth: bool,
    claude_oauth: bool,
    codex_oauth: bool,
    antigravity_project_id: Option<String>,
    antigravity_client_id: Option<String>,
    codex_account_id: Option<String>,
    antigravity_expires_at: Option<u64>,
    claude_expires_at: Option<u64>,
    codex_expires_at: Option<u64>,
}

impl KeyVault {
    fn new(owner: Owner, salt: u64) -> Self {
        Self {
            owner,
            keeper: Keeper::new(salt, 16),
            gemini: None,
            antigravity: None,
            claude: None,
            claude_deepseek: None,
            codex: None,
            antigravity_refresh: None,
            antigravity_client_secret: None,
            claude_refresh: None,
            codex_refresh: None,
            proxy_key: None,
            antigravity_oauth: false,
            claude_oauth: false,
            codex_oauth: false,
            antigravity_project_id: None,
            antigravity_client_id: None,
            codex_account_id: None,
            antigravity_expires_at: None,
            claude_expires_at: None,
            codex_expires_at: None,
        }
    }

    fn slot(&self, provider: Provider) -> Option<Knot> {
        match provider {
            Provider::Gemini => self.gemini,
            Provider::Antigravity => self.antigravity,
            Provider::Claude => self.claude,
            Provider::ClaudeDeepSeek => self.claude_deepseek,
            Provider::Codex => self.codex,
        }
    }

    fn set_slot(&mut self, provider: Provider, knot: Knot) {
        match provider {
            Provider::Gemini => self.gemini = Some(knot),
            Provider::Antigravity => self.antigravity = Some(knot),
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
        } else if provider == Provider::Antigravity {
            self.antigravity_oauth = true;
        }
        Ok(())
    }

    fn store_oauth(&mut self, provider: Provider, token: &OAuthToken) -> Result<(), &'static str> {
        self.store(provider, token.access_token.as_bytes())?;
        match provider {
            Provider::Antigravity => {
                self.antigravity_oauth = true;
                self.antigravity_expires_at = token.expires_at;
                if token.account_id.is_some() {
                    self.antigravity_project_id = token.account_id.clone();
                }
                if let Some(ref rt) = token.refresh_token {
                    Self::store_knot(
                        &mut self.keeper,
                        self.owner,
                        &mut self.antigravity_refresh,
                        "antigravity_refresh",
                        rt.as_bytes(),
                    )?;
                }
            }
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

    fn store_antigravity_oauth_client(
        &mut self,
        client_id: Option<&str>,
        client_secret: Option<&str>,
    ) {
        if let Some(cid) = client_id.map(str::trim).filter(|s| !s.is_empty()) {
            self.antigravity_client_id = Some(String::from(cid));
        }
        if let Some(csec) = client_secret.map(str::trim).filter(|s| !s.is_empty()) {
            let _ = Self::store_knot(
                &mut self.keeper,
                self.owner,
                &mut self.antigravity_client_secret,
                "antigravity_client_secret",
                csec.as_bytes(),
            );
        }
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
            Provider::Antigravity => self.antigravity_oauth,
            Provider::Claude => self.claude_oauth,
            Provider::Codex => self.codex_oauth,
            _ => false,
        }
    }

    fn antigravity_project_id(&self) -> Option<&str> {
        self.antigravity_project_id
            .as_deref()
            .filter(|s| !s.is_empty())
    }

    fn set_antigravity_project_id(&mut self, project_id: String) {
        if !project_id.trim().is_empty() {
            self.antigravity_project_id = Some(String::from(project_id.trim()));
        }
    }

    fn antigravity_client_id(&self) -> Option<&str> {
        self.antigravity_client_id.as_deref()
    }

    fn with_antigravity_client_secret<R>(&self, f: impl FnOnce(&str) -> R) -> Option<R> {
        let knot = self.antigravity_client_secret?;
        self.keeper
            .with(self.owner, knot, |bytes| {
                let s = core::str::from_utf8(bytes).unwrap_or("");
                f(s)
            })
            .ok()
    }

    fn codex_account_id(&self) -> Option<&str> {
        self.codex_account_id.as_deref()
    }

    fn has_refresh_token(&self, provider: Provider) -> bool {
        match provider {
            Provider::Antigravity => self.antigravity_refresh.is_some(),
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
            Provider::Antigravity => self.antigravity_refresh?,
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

    /// Loads unified desktop `credentials.json` containing OAuth bundles (`antigravity`,
    /// `claude`, `codex`), API keys (`gemini`, `deepseek`, `claude`, `codex`), and `proxy`.
    fn load_unified_credentials(&mut self, raw: &str) -> (usize, Option<String>) {
        let Ok(root) = json::parse(raw) else {
            return (0, None);
        };
        let mut loaded = 0usize;
        let mut proxy_base = None;

        if let Some(ag) = root.get("antigravity") {
            let oauth_obj = ag.get("oauth").unwrap_or(ag);
            if let Some(tok) = parse_oauth_object(oauth_obj) {
                if self.store_oauth(Provider::Antigravity, &tok).is_ok() {
                    loaded += 1;
                }
            }
            let cid = non_empty_str(
                oauth_obj
                    .get("client_id")
                    .or_else(|| oauth_obj.get("clientId"))
                    .or_else(|| ag.get("client_id"))
                    .or_else(|| ag.get("clientId")),
            );
            let csec = non_empty_str(
                oauth_obj
                    .get("client_secret")
                    .or_else(|| oauth_obj.get("clientSecret"))
                    .or_else(|| ag.get("client_secret"))
                    .or_else(|| ag.get("clientSecret")),
            );
            self.store_antigravity_oauth_client(cid, csec);
        }

        for (key_name, provider) in [
            ("claude", Provider::Claude),
            ("codex", Provider::Codex),
            ("gemini", Provider::Gemini),
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
            if let Some(oauth_obj) = entry.get("oauth") {
                if let Some(tok) = parse_oauth_object(oauth_obj) {
                    if self.store_oauth(provider, &tok).is_ok() {
                        loaded += 1;
                        continue;
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

        (loaded, proxy_base)
    }

    fn load_from_files(&mut self, window: &mut Window) -> (usize, Option<String>) {
        let mut loaded = 0usize;
        let mut proxy_base = None;

        // 0. Unified desktop `credentials.json` (single-file drop onto Ventoy USB or /data/tailor/).
        let mut cred_buf = vec![0u8; 16384];
        if let Some(n) = read_config_file(
            window,
            "credentials.json",
            Some("tailor-credentials.json"),
            &mut cred_buf,
        ) {
            if let Ok(raw) = core::str::from_utf8(&cred_buf[..n]) {
                let (c_loaded, c_proxy) = self.load_unified_credentials(raw);
                loaded += c_loaded;
                if c_proxy.is_some() {
                    proxy_base = c_proxy;
                }
            }
        }
        zeroize_slice(&mut cred_buf);

        // 1. Load individual OAuth credentials (Antigravity/Google, Claude Pro/Max, Codex/ChatGPT).
        let mut oauth_buf = vec![0u8; 8192];
        if !self.has_key(Provider::Antigravity) {
            if let Some(n) = read_config_file(
                window,
                "google_oauth.json",
                Some("tailor-google_oauth.json"),
                &mut oauth_buf,
            ) {
                if let Ok(raw) = core::str::from_utf8(&oauth_buf[..n]) {
                    if let Some(tok) = parse_google_oauth_json(raw) {
                        if self.store_oauth(Provider::Antigravity, &tok).is_ok() {
                            loaded += 1;
                        }
                    }
                    if let Ok(root) = json::parse(raw) {
                        let obj = root.get("tokens").unwrap_or(&root);
                        let cid = non_empty_str(
                            obj.get("client_id")
                                .or_else(|| obj.get("clientId"))
                                .or_else(|| root.get("client_id")),
                        );
                        let csec = non_empty_str(
                            obj.get("client_secret")
                                .or_else(|| obj.get("clientSecret"))
                                .or_else(|| root.get("client_secret")),
                        );
                        self.store_antigravity_oauth_client(cid, csec);
                    }
                }
            }
        }
        zeroize_slice(&mut oauth_buf);

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
                    if !self.has_key(provider) && self.store(provider, secret.as_bytes()).is_ok() {
                        loaded += 1;
                    }
                }
            }
        }
        zeroize_slice(&mut jbuf);

        // 3. Fallback to per-provider *.key files.
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

        // 4. Optional CLIProxyAPI-Rust proxy key & base URL.
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
                        proxy_base = Some(String::from(trimmed));
                    }
                }
            }
            zeroize_slice(&mut buf);
        }

        (loaded, proxy_base)
    }
}

/// Capability-scoped SexOS workspace backed by `sex_app::files`.
struct SexWorkspace<'a> {
    window: &'a mut Window,
    root: &'a str,
}

impl<'a> SexWorkspace<'a> {
    fn resolve(&self, path: &str) -> String {
        let trimmed = path.trim();
        if trimmed.is_empty() || trimmed == "." {
            return String::from(self.root);
        }
        if trimmed.starts_with('/') || trimmed.starts_with('@') || self.root.is_empty() {
            return String::from(trimmed);
        }
        let base = self.root.trim_end_matches('/');
        let rel = trimmed.trim_start_matches("./").trim_start_matches('/');
        format!("{}/{}", base, rel)
    }
}

impl<'a> Workspace for SexWorkspace<'a> {
    fn read_file(&mut self, path: &str) -> Result<String, String> {
        let full = self.resolve(path);
        let mut out = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK_BYTES];
        let mut offset = 0u64;
        loop {
            let n = files::read_at(self.window, full.as_bytes(), offset, &mut chunk)
                .map_err(|e| format!("files::read_at({}) failed: code {}", full, e))?;
            if n == 0 {
                break;
            }
            let take = n.min(MAX_READ_BYTES.saturating_sub(out.len()));
            out.extend_from_slice(&chunk[..take]);
            offset += n as u64;
            if out.len() >= MAX_READ_BYTES || n < chunk.len() {
                break;
            }
        }
        String::from_utf8(out).map_err(|_| format!("File {} is not valid UTF-8", full))
    }

    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        let full = self.resolve(path);
        files::write(self.window, full.as_bytes(), content.as_bytes())
            .map_err(|e| format!("files::write({}) failed: code {}", full, e))?;
        let _ = files::sync(self.window);
        Ok(())
    }

    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, String> {
        let full = self.resolve(path);
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
    provider: Provider,
    duplex: net::Duplex,
    parser: ProviderStreamParser,
    rx_buf: Vec<u8>,
    retried_after_refresh: bool,
}

struct AppState {
    workspace_root: String,
    agent: Agent,
    vault: KeyVault,
    proxy_base: Option<String>,
    runner: SexToolRunner,
    transcript: String,
    input: String,
    input_selected: bool,
    status: String,
    scroll_lines: usize,
    stream: Option<ActiveStream>,
    ui: ui::TailorUi,
}

impl AppState {
    fn push_log(&mut self, line: &str) {
        self.transcript.push_str(line);
        self.transcript.push('\n');
        self.trim_transcript();
    }

    fn push_delta(&mut self, delta: &str) {
        self.transcript.push_str(delta);
        self.trim_transcript();
    }

    fn trim_transcript(&mut self) {
        if self.transcript.len() > MAX_TRANSCRIPT_BYTES {
            let cut = self.transcript.len() - (MAX_TRANSCRIPT_BYTES * 3 / 4);
            let mut idx = cut;
            while idx < self.transcript.len() && !self.transcript.is_char_boundary(idx) {
                idx += 1;
            }
            self.transcript.drain(..idx);
        }
    }

    fn auth_mode_label(&self, provider: Provider) -> &'static str {
        if self.proxy_base.is_some() {
            "proxy"
        } else if self.vault.is_oauth(provider) {
            "oauth"
        } else if self.vault.has_key(provider) {
            "loaded"
        } else {
            "missing"
        }
    }

    fn auth_badge(&self, provider: Provider) -> (String, u32) {
        if let Some(ref base) = self.proxy_base {
            let short = base
                .trim()
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/');
            return (format!("Proxy: {}", short), 0x0002_84C7);
        }
        if provider == Provider::Antigravity && self.vault.is_oauth(Provider::Antigravity) {
            return (String::from("Antigravity OAuth"), 0x002E_A043);
        }
        if provider == Provider::Claude && self.vault.is_oauth(Provider::Claude) {
            return (String::from("Claude OAuth"), 0x002E_A043);
        }
        if provider == Provider::Codex && self.vault.is_oauth(Provider::Codex) {
            return (String::from("Codex OAuth"), 0x002E_A043);
        }
        if self.vault.has_key(provider) {
            (String::from("Shibari: OK"), 0x002E_A043)
        } else {
            (String::from("No API Key"), 0x00D9_7706)
        }
    }

    fn discover_antigravity_project(&mut self, window: &mut Window) -> Result<String, u16> {
        let mut auth_header = match self
            .vault
            .with_key(Provider::Antigravity, |tok| format!("Bearer {}", tok.trim()))
        {
            Some(h) => h,
            None => return Ok(String::from(DEFAULT_ANTIGRAVITY_PROJECT)),
        };

        let body = b"{\"metadata\":{\"ideType\":\"ANTIGRAVITY\",\"platform\":\"PLATFORM_UNSPECIFIED\",\"pluginType\":\"GEMINI\"}}";
        let headers = [
            ("Authorization", auth_header.as_str()),
            ("User-Agent", ANTIGRAVITY_USER_AGENT),
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ];
        let req = sex_fetch::Request {
            method: "POST",
            url: LOAD_CODE_ASSIST_URL,
            headers: &headers,
            body,
            trusted_origin: false,
            referrer: None,
        };
        let resp_res = net::request(window, &req, None, OAUTH_REFRESH_MAX_BYTES);
        unsafe { zeroize_slice(auth_header.as_bytes_mut()) };

        let Ok(resp) = resp_res else {
            return Ok(String::from(DEFAULT_ANTIGRAVITY_PROJECT));
        };
        if resp.head.status == 401 {
            return Err(401);
        }
        if (200..300).contains(&resp.head.status) {
            if let Ok(body_str) = core::str::from_utf8(&resp.body) {
                if let Ok(root) = json::parse(body_str) {
                    let project = root.get("cloudaicompanionProject").and_then(|v| {
                        v.as_str()
                            .or_else(|| v.get("id").and_then(JsonValue::as_str))
                            .or_else(|| v.get("projectId").and_then(JsonValue::as_str))
                    });
                    if let Some(p) = project.map(str::trim).filter(|s| !s.is_empty()) {
                        let proj = String::from(p);
                        self.vault.set_antigravity_project_id(proj.clone());
                        return Ok(proj);
                    }
                }
            }
        }
        let fallback = String::from(DEFAULT_ANTIGRAVITY_PROJECT);
        self.vault.set_antigravity_project_id(fallback.clone());
        Ok(fallback)
    }

    fn start_model_turn(&mut self, window: &mut Window) {
        self.start_model_turn_inner(window, false);
    }

    fn start_model_turn_inner(&mut self, window: &mut Window, retried_after_refresh: bool) {
        let provider = self.agent.provider();
        let mut req = if provider == Provider::Antigravity
            && self.proxy_base.is_none()
            && self.vault.has_key(Provider::Antigravity)
        {
            let project_id = if let Some(pid) = self.vault.antigravity_project_id() {
                String::from(pid)
            } else {
                match self.discover_antigravity_project(window) {
                    Ok(pid) => pid,
                    Err(401)
                        if !retried_after_refresh
                            && self.vault.has_refresh_token(Provider::Antigravity) =>
                    {
                        self.status = String::from("Refreshing Antigravity OAuth token...");
                        self.push_log(&format!("\n[tailor] {}", self.status));
                        match self.try_refresh_oauth(window, Provider::Antigravity) {
                            Ok(()) => {
                                self.push_log("[tailor] OAuth token refreshed; retrying...");
                                self.start_model_turn_inner(window, true);
                                return;
                            }
                            Err(err) => {
                                self.push_log(&format!("[error] {}", err));
                                self.status = format!("Error: {}", err);
                                return;
                            }
                        }
                    }
                    Err(_) => String::from(DEFAULT_ANTIGRAVITY_PROJECT),
                }
            };
            let Some(r) = self.vault.with_key(Provider::Antigravity, |key| {
                build_antigravity_request(
                    self.agent.model(),
                    key,
                    &project_id,
                    self.agent.system_prompt(),
                    self.agent.history(),
                    self.agent.tools(),
                )
            }) else {
                return;
            };
            r
        } else if provider == Provider::Codex
            && self.proxy_base.is_none()
            && self.vault.is_oauth(Provider::Codex)
        {
            let effective_model = match self.agent.model() {
                "gpt-4.1" | "o4-mini" => "gpt-5.1-codex",
                other => other,
            };
            let Some(r) = self.vault.with_key(Provider::Codex, |key| {
                build_codex_responses_request(
                    effective_model,
                    key,
                    self.vault.codex_account_id(),
                    self.agent.system_prompt(),
                    self.agent.history(),
                    self.agent.tools(),
                )
            }) else {
                return;
            };
            r
        } else if let Some(r) = self
            .vault
            .with_key(provider, |key| self.agent.build_request(key))
        {
            r
        } else if self.proxy_base.is_some() {
            // Proxy mode allows all providers without individual API keys.
            self.agent.build_request("")
        } else {
            self.status = format!(
                "No {} key/OAuth. Drop credentials.json or {} in /data/tailor (or Ventoy), or /proxy <url>",
                provider.display_name(),
                provider.key_filename(),
            );
            return;
        };

        if provider == Provider::Codex && !self.vault.is_oauth(Provider::Codex) {
            req = req.with_chatgpt_account_id(self.vault.codex_account_id());
        }

        if let Some(ref proxy_url) = self.proxy_base {
            req = match self
                .vault
                .with_proxy_key(|pk| req.clone().with_proxy_base(proxy_url, Some(pk)))
            {
                Some(rewritten) => rewritten,
                None => req.with_proxy_base(proxy_url, Some("cliproxy")),
            };
        }

        match net::Duplex::start(window, &req.host, req.port, req.tls, req.http_bytes) {
            Ok(duplex) => {
                if !retried_after_refresh {
                    self.push_delta(&format!("\n[{}] ", provider.display_name()));
                }
                self.stream = Some(ActiveStream {
                    provider,
                    duplex,
                    parser: ProviderStreamParser::new(provider),
                    rx_buf: Vec::with_capacity(4096),
                    retried_after_refresh,
                });
                self.status = format!(
                    "Streaming from {} ({})... Ctrl+C to cancel.",
                    provider.display_name(),
                    self.agent.model()
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
            Provider::Antigravity => {
                let cid = self.vault.antigravity_client_id().unwrap_or("");
                let mut csec = self
                    .vault
                    .with_antigravity_client_secret(|s| String::from(s))
                    .unwrap_or_default();
                let req = self.vault.with_refresh_token(Provider::Antigravity, |rt| {
                    build_google_refresh_request(rt, cid, &csec)
                });
                unsafe { zeroize_slice(csec.as_bytes_mut()) };
                req
            }
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
            Provider::Antigravity => self.vault.antigravity_project_id().map(String::from),
            Provider::Codex => self.vault.codex_account_id().map(String::from),
            _ => None,
        };

        let content_type = if provider == Provider::Antigravity {
            "application/x-www-form-urlencoded"
        } else {
            "application/json"
        };
        let headers = [
            ("Content-Type", content_type),
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
            Provider::Antigravity => parse_google_oauth_json(body_str),
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
            Provider::Antigravity => "google_oauth.json",
            Provider::Claude => "claude_oauth.json",
            Provider::Codex => "codex_oauth.json",
            _ => return Ok(()),
        };
        let mut json_bytes = parsed.to_json().into_bytes();
        let _ = write_config_file(window, filename, None, &json_bytes);
        zeroize_slice(&mut json_bytes);
        Ok(())
    }

    fn drive_tools(&mut self, window: &mut Window) {
        loop {
            let step = {
                let mut ws = SexWorkspace {
                    window,
                    root: &self.workspace_root,
                };
                self.agent.step_tools(&mut ws, &mut self.runner)
            };
            match step {
                AgentStep::ToolExecuted(res) => {
                    let tag = if res.is_error { "ERR" } else { "OK" };
                    let preview = if res.output.len() > 240 {
                        format!("{}...", &res.output[..240])
                    } else {
                        res.output.clone()
                    };
                    self.push_log(&format!("[tool:{} {}] {}", res.name, tag, preview));
                }
                AgentStep::AwaitingApproval(pending) => {
                    self.push_log(&format!("\n[APPROVAL REQUIRED] {}", pending.summary));
                    if let Some(ref diff) = pending.diff {
                        let mut shown = 0usize;
                        let mut extra = 0usize;
                        for line in diff.lines() {
                            if shown < MAX_DIFF_PREVIEW_LINES {
                                self.push_log(line);
                                shown += 1;
                            } else {
                                extra += 1;
                            }
                        }
                        if extra > 0 {
                            self.push_log(&format!("... (+{} more diff lines)", extra));
                        }
                    }
                    self.push_log("Type /approve (or y) to allow, /deny (or n) to reject.");
                    self.status = format!("Awaiting approval: {}", pending.summary);
                    return;
                }
                AgentStep::NeedsModelTurn => {
                    self.start_model_turn(window);
                    return;
                }
                AgentStep::Idle => {
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
        let mut changed = false;
        active.rx_buf.clear();
        let poll_res = active.duplex.poll(window, &mut active.rx_buf);
        let mut done = false;
        let can_auto_refresh = !active.retried_after_refresh
            && matches!(
                active.provider,
                Provider::Antigravity | Provider::Claude | Provider::Codex
            )
            && self.vault.has_refresh_token(active.provider);

        if !active.rx_buf.is_empty() {
            changed = true;
            let events = active.parser.feed(&active.rx_buf);
            let will_refresh = can_auto_refresh && active.parser.http_status() == Some(401);
            for ev in events {
                if will_refresh && matches!(ev, StreamEvent::Error(_)) {
                    done = true;
                } else if self.apply_stream_event(&ev) {
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
                    self.push_log(&format!("\n[tailor] {}", self.status));
                    changed = true;
                    match self.try_refresh_oauth(window, active.provider) {
                        Ok(()) => {
                            self.push_log("[tailor] OAuth token refreshed; retrying...");
                            self.start_model_turn_inner(window, true);
                        }
                        Err(err) => {
                            self.push_log(&format!("[error] {}", err));
                            self.status = format!("Error: {}", err);
                            let _ = self.agent.finish_model_turn();
                        }
                    }
                    return changed;
                }

                for ev in tail {
                    self.apply_stream_event(&ev);
                }
                active.duplex.close(window);
                self.push_delta("\n");
                changed = true;
                match self.agent.finish_model_turn() {
                    AgentStep::NeedsModelTurn => self.drive_tools(window),
                    AgentStep::Idle => {
                        self.status = String::from("Ready.");
                    }
                    _ => {}
                }
            }
        }
        changed
    }

    fn apply_stream_event(&mut self, ev: &StreamEvent) -> bool {
        self.agent.on_stream_event(ev);
        match ev {
            StreamEvent::TextDelta(t) => {
                self.push_delta(t);
                false
            }
            StreamEvent::ThinkingDelta(t) => {
                self.status = format!("Thinking ({} chars)...", t.len());
                false
            }
            StreamEvent::ToolCall { name, .. } => {
                self.push_log(&format!("\n[requesting tool: {}]", name));
                false
            }
            StreamEvent::Done { .. } => true,
            StreamEvent::Error(msg) => {
                self.push_log(&format!("\n[error] {}", msg));
                self.status = format!("Error: {}", msg);
                true
            }
        }
    }

    fn handle_submit(&mut self, window: &mut Window) {
        let line = core::mem::take(&mut self.input);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        self.scroll_lines = 0;

        // Shortcut y/n when an approval gate is active.
        if self.agent.pending_approval().is_some() {
            if trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("/approve") {
                let step = {
                    let mut ws = SexWorkspace {
                        window,
                        root: &self.workspace_root,
                    };
                    self.agent
                        .resolve_approval(ApprovalDecision::Approved, &mut ws, &mut self.runner)
                };
                if let AgentStep::ToolExecuted(res) = step {
                    let tag = if res.is_error { "ERR" } else { "OK" };
                    self.push_log(&format!("[approved {} {}] {}", res.name, tag, res.output));
                }
                self.drive_tools(window);
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
                        root: &self.workspace_root,
                    };
                    self.agent.resolve_approval(
                        ApprovalDecision::Denied(String::from(reason)),
                        &mut ws,
                        &mut self.runner,
                    )
                };
                if let AgentStep::ToolExecuted(res) = step {
                    self.push_log(&format!("[denied {}] {}", res.name, res.output));
                }
                self.drive_tools(window);
                return;
            }
        }

        if let Some(cmd) = trimmed.strip_prefix('/') {
            self.handle_command(window, cmd);
            return;
        }

        self.push_log(&format!("\n> {}", trimmed));
        self.agent.push_user_message(trimmed);
        self.start_model_turn(window);
    }

    fn handle_command(&mut self, window: &mut Window, cmd: &str) {
        let mut parts = cmd.splitn(2, ' ');
        let verb = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("").trim();

        match verb {
            "provider" => match Provider::from_name(rest) {
                Some(p) => {
                    self.agent.set_provider(p);
                    self.status = format!(
                        "Switched to {} ({}) [auth={}]",
                        p.display_name(),
                        self.agent.model(),
                        self.auth_mode_label(p)
                    );
                    self.push_log(&format!("[tailor] {}", self.status));
                }
                None => {
                    self.status = String::from(
                        "Usage: /provider antigravity|gemini|claude|claude-deepseek|codex",
                    );
                }
            },
            "model" => {
                if rest.is_empty() {
                    self.status = format!("Active model: {}", self.agent.model());
                } else {
                    self.agent.set_model(rest);
                    self.status = format!("Model set to {}", self.agent.model());
                    self.push_log(&format!("[tailor] {}", self.status));
                }
            }
            "proxy" => {
                if rest.is_empty() {
                    self.status = match &self.proxy_base {
                        Some(url) => format!("Proxy active: {} (use /proxy off to disable)", url),
                        None => String::from("Proxy is off. Usage: /proxy http://127.0.0.1:8317 | /proxy off"),
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
                    let normalized = if rest.starts_with("http://") || rest.starts_with("https://") {
                        String::from(rest.trim_end_matches('/'))
                    } else {
                        format!("http://{}", rest.trim_end_matches('/'))
                    };
                    let saved = write_config_file(window, "proxy.conf", None, normalized.as_bytes());
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
                                format!(
                                    "Stored {} key in Shibari RAM vault.",
                                    p.display_name()
                                )
                            };
                            self.push_log(&format!("[tailor] {}", self.status));
                        }
                        Err(e) => self.status = String::from(e),
                    },
                    _ => {
                        self.status = String::from(
                            "Usage: /key antigravity|gemini|claude|claude-deepseek|codex <secret>",
                        );
                    }
                }
            }
            "clear" => {
                self.agent.clear();
                self.transcript.clear();
                self.status = String::from("Conversation cleared.");
            }
            "approve" | "deny" => {
                self.status = String::from("No tool call is currently awaiting approval.");
            }
            "help" => {
                self.push_log(
                    "Commands:\n\
                     /provider antigravity|gemini|claude|claude-deepseek|codex  Switch AI provider\n\
                     /model <name>                   Override model (e.g. gemini-4-neon-high, gpt-6.2-sol)\n\
                     /proxy <url> | /proxy off       Route via CLIProxyAPI-Rust (e.g. http://127.0.0.1:8317)\n\
                     /key <provider> <secret>        Store & persist API key or OAuth token\n\
                     /approve (or y)                 Approve pending mutating tool\n\
                     /deny [reason] (or n)           Deny pending mutating tool\n\
                     /clear                          Clear conversation history\n\
                     Auto-load (/data/tailor/ or @/media/Ventoy/): credentials.json, google_oauth.json, claude_oauth.json, codex_oauth.json\n\
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
        let p = self.agent.provider();
        let (auth_label, auth_color) = self.auth_badge(p);
        let model = self.agent.model();
        let streaming = self.stream.is_some();
        let task_boundary = self.agent.task_boundary();
        let pending_approval = self.agent.pending_approval().map(|pa| pa.summary.as_str());
        self.ui.render(
            window,
            p,
            model,
            &auth_label,
            auth_color,
            streaming,
            task_boundary,
            pending_approval,
            &self.transcript,
            self.scroll_lines,
            &self.input,
            self.input_selected,
            &self.status,
        );
    }
}

fn parse_launch_args(raw: &[u8]) -> (String, Provider) {
    let text = core::str::from_utf8(raw).unwrap_or("");
    let mut workspace = String::new();
    let mut provider = Provider::ClaudeDeepSeek;
    for word in text.split_whitespace() {
        if let Some(p) = Provider::from_name(word) {
            provider = p;
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
    let (workspace_root, initial_provider) = parse_launch_args(files::args(&mut window, &mut args_buf));

    let owner = Owner(window.target);
    let vault = KeyVault::new(owner, sex_pdx::get_ticks() ^ 0x5A17_7A11_0001);

    let mut app = AppState {
        workspace_root,
        agent: Agent::new(initial_provider),
        vault,
        proxy_base: None,
        runner: SexToolRunner::new(),
        transcript: String::new(),
        input: String::new(),
        input_selected: false,
        status: String::from("Loading keys & OAuth into Shibari vault..."),
        scroll_lines: 0,
        stream: None,
        ui: ui::TailorUi::new(),
    };
    app.push_log("Tailor — Native Multi-Provider AI Coding Agent for SexOS.");
    app.push_log("Click a provider pill above (Antigravity / Claude / DeepSeek / Codex / Gemini), or use:");
    app.push_log("  Unified drop: copy credentials.json (or google_oauth.json) to /data/tailor/ or Ventoy");
    app.push_log("  /proxy http://127.0.0.1:8317  (CLIProxyAPI-Rust)  |  /proxy off");
    app.push_log("  /key <provider> <secret>  |  /model <id>  |  /help");
    app.render(&mut window);

    let (loaded, proxy_base) = app.vault.load_from_files(&mut window);
    app.proxy_base = proxy_base;
    app.status = if let Some(ref url) = app.proxy_base {
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
        "[tailor.ready] target={} task={} provider={} keys={} proxy={}",
        window.target,
        window.task,
        app.agent.provider().name(),
        loaded,
        app.proxy_base.as_deref().unwrap_or("none")
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
                Event::Pointer(p) => match p.kind {
                    sex_pdx::app_pointer::Kind::Down { button: 0 } => {
                        let px = p.x as i32;
                        let py = p.y as i32;
                        if let Some(hit) = app.ui.hits.iter().find(|h| h.contains(px, py)).copied() {
                            match hit.action {
                                ui::HitAction::Provider(prov) => {
                                    app.agent.set_provider(prov);
                                    app.status = format!(
                                        "Switched to {} ({}) [auth={}]",
                                        prov.display_name(),
                                        app.agent.model(),
                                        app.auth_mode_label(prov)
                                    );
                                    app.push_log(&format!("[tailor] {}", app.status));
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
                                ui::HitAction::Send if app.stream.is_none() => {
                                    app.handle_submit(&mut window);
                                    app.input_selected = false;
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
                        if steps > 0 {
                            app.scroll_lines = app.scroll_lines.saturating_add(3);
                        } else if steps < 0 {
                            app.scroll_lines = app.scroll_lines.saturating_sub(3);
                        }
                        dirty = true;
                    }
                    _ => {}
                },
                Event::Key(key) if key.control && key.code == 0x1E => {
                    // Ctrl+A: select all prompt text
                    app.input_selected = !app.input.is_empty();
                    dirty = true;
                }
                Event::Key(key) if key.control && key.code == 0x2F => {
                    // Ctrl+V: paste clipboard into prompt
                    if app.stream.is_none() {
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
                Event::Key(key) if key.control && key.code == 0x2e => {
                    // Ctrl+C: cancel active stream, or copy prompt/transcript to clipboard.
                    if let Some(mut active) = app.stream.take() {
                        active.duplex.close(&mut window);
                        app.push_log("\n[cancelled]");
                        app.status = String::from("Stream cancelled.");
                    } else if !app.input.is_empty() {
                        let _ = sex_app::clipboard::copy(&mut window, &app.input);
                        app.status = String::from("Copied prompt to clipboard.");
                    } else if !app.transcript.is_empty() {
                        let _ = sex_app::clipboard::copy(&mut window, &app.transcript);
                        app.status = String::from("Copied transcript to clipboard.");
                    }
                    dirty = true;
                }
                Event::Key(key) if key.code == 0x1C => {
                    // Enter
                    if app.stream.is_none() {
                        app.input_selected = false;
                        app.handle_submit(&mut window);
                        dirty = true;
                    }
                }
                Event::Key(key) if key.code == 0x0E => {
                    // Backspace
                    if app.stream.is_none() {
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
                        if app.stream.is_none() {
                            if let Some(ch) = sex_app::text(key) {
                                if !ch.is_control() && app.input.len() < 1024 {
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
