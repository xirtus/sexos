// SPDX-License-Identifier: GPL-3.0-only
//! Tailor: SexOS's native multi-provider AI coding agent (Gemini, Claude, Codex).
//!
//! Launch with network and an optional workspace grant:
//!   `run +net tailor [@/workspace] [gemini|claude|codex]`
//!
//! API keys are loaded into `shibari::Keeper` (RAM-only zeroizing secret store) from
//! `/data/tailor/{gemini,claude,codex}.key` (or local `{gemini,claude,codex}.key`), or
//! entered interactively with `/key <provider> <secret>`.
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
    build_claude_refresh_request, build_codex_refresh_request, is_claude_oauth_token,
    parse_claude_oauth_json, parse_codex_oauth_json, Agent, AgentStep, ApprovalDecision,
    OAuthToken, Provider, ProviderStreamParser, StreamEvent, ToolRunner, Workspace,
};

const MAX_READ_BYTES: usize = 256 * 1024;
const READ_CHUNK_BYTES: usize = 32 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 24 * 1024;
const COMMAND_TIMEOUT_TICKS: u64 = 15_000;
const OAUTH_REFRESH_MAX_BYTES: usize = 64 * 1024;

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
            Provider::Claude => self.claude,
            Provider::ClaudeDeepSeek => self.claude_deepseek,
            Provider::Codex => self.codex,
        }
    }

    fn set_slot(&mut self, provider: Provider, knot: Knot) {
        match provider {
            Provider::Gemini => self.gemini = Some(knot),
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

    fn load_from_files(&mut self, window: &mut Window) -> (usize, Option<String>) {
        let mut loaded = 0usize;

        // 1. Load OAuth credentials first (Claude Pro/Max & Codex / ChatGPT Plus & Pro).
        let mut oauth_buf = vec![0u8; 8192];
        if let Some(n) = read_config_file(window, "claude_oauth.json", None, &mut oauth_buf) {
            if let Ok(raw) = core::str::from_utf8(&oauth_buf[..n]) {
                if let Some(tok) = parse_claude_oauth_json(raw) {
                    if self.store_oauth(Provider::Claude, &tok).is_ok() {
                        loaded += 1;
                    }
                }
            }
        }
        zeroize_slice(&mut oauth_buf);

        if let Some(n) = read_config_file(window, "codex_oauth.json", None, &mut oauth_buf) {
            if let Ok(raw) = core::str::from_utf8(&oauth_buf[..n]) {
                if let Some(tok) = parse_codex_oauth_json(raw) {
                    if self.store_oauth(Provider::Codex, &tok).is_ok() {
                        loaded += 1;
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
        if let Some(n) = read_config_file(window, "proxy.key", None, &mut buf) {
            if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    let _ = self.store_proxy_key(trimmed.as_bytes());
                }
            }
        }
        zeroize_slice(&mut buf);

        let mut proxy_base = None;
        if let Some(n) = read_config_file(window, "proxy.conf", None, &mut buf) {
            if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    proxy_base = Some(String::from(trimmed));
                }
            }
        }
        zeroize_slice(&mut buf);

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
    input: sex_app::RichTextInput,
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

    fn start_model_turn(&mut self, window: &mut Window) {
        self.start_model_turn_inner(window, false);
    }

    fn start_model_turn_inner(&mut self, window: &mut Window, retried_after_refresh: bool) {
        let provider = self.agent.provider();
        let mut req = if let Some(r) = self
            .vault
            .with_key(provider, |key| self.agent.build_request(key))
        {
            r
        } else if self.proxy_base.is_some() {
            // Proxy mode allows all providers without individual API keys.
            self.agent.build_request("")
        } else {
            self.status = format!(
                "No {} key/OAuth. Put {} or claude_oauth.json/codex_oauth.json in /data/tailor, or run /proxy <url>",
                provider.display_name(),
                provider.key_filename(),
            );
            return;
        };

        if provider == Provider::Codex {
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
        let old_account_id = if provider == Provider::Codex {
            self.vault.codex_account_id().map(String::from)
        } else {
            None
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
                    self.push_log(&format!(
                        "\n[APPROVAL REQUIRED] {}\nType /approve (or y) to allow, /deny (or n) to reject.",
                        pending.summary
                    ));
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
            && matches!(active.provider, Provider::Claude | Provider::Codex)
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

    fn sync_input_mode(&mut self) {
        let mode = if self.input.text().starts_with('/') {
            sex_app::TextMode::Code
        } else {
            sex_app::TextMode::SingleLineProse
        };
        self.input.set_mode(mode);
    }

    fn handle_submit(&mut self, window: &mut Window) {
        let line = String::from(self.input.text());
        self.input.clear();
        self.sync_input_mode();
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
                    self.status = String::from("Usage: /provider gemini|claude|claude-deepseek|codex");
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
                        self.status = String::from("Usage: /key gemini|claude|claude-deepseek|codex <secret>");
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
                     /provider gemini|claude|claude-deepseek|codex  Switch AI provider\n\
                     /model <name>                   Override model name\n\
                     /proxy <url> | /proxy off       Route via CLIProxyAPI-Rust (e.g. http://127.0.0.1:8317)\n\
                     /key <provider> <secret>        Store & persist API key or sk-ant-oat01-* OAuth token\n\
                     /approve (or y)                 Approve pending mutating tool\n\
                     /deny [reason] (or n)           Deny pending mutating tool\n\
                     /clear                          Clear conversation history\n\
                     OAuth files in /data/tailor/: claude_oauth.json, codex_oauth.json, proxy.conf, proxy.key\n\
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
        let pending_approval = self.agent.pending_approval().is_some();
        self.ui.render(
            window,
            p,
            model,
            &auth_label,
            auth_color,
            streaming,
            pending_approval,
            &self.transcript,
            self.scroll_lines,
            &mut self.input,
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
        input: {
            let mut f = sex_app::RichTextInput::new("", sex_app::TextMode::SingleLineProse)
                .with_placeholder("Ask Tailor anything, or type /proxy <url> or /key <provider> <key> ...")
                .with_max_bytes(1024);
            f.focused = true;
            f
        },
        status: String::from("Loading keys & OAuth into Shibari vault..."),
        scroll_lines: 0,
        stream: None,
        ui: ui::TailorUi::new(),
    };
    app.push_log("Tailor — Native Multi-Provider AI Coding Agent for SexOS.");
    app.push_log("Click a provider pill above (Claude / DeepSeek / Codex / Gemini), or use:");
    app.push_log("  /proxy http://127.0.0.1:8317  (CLIProxyAPI-Rust)  |  /proxy off");
    app.push_log("  OAuth: drop claude_oauth.json or codex_oauth.json in /data/tailor/");
    app.push_log("  API key: /key <provider> <secret>  |  /model <id>  |  /help");
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
            app.input.observe_event(&event);
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
                    let pf = app.input.handle_pointer(&mut window, p);
                    app.sync_input_mode();
                    if pf.handled {
                        dirty = true;
                    } else if let sex_pdx::app_pointer::Kind::Down { button: 0 } = p.kind {
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
                                    app.input.set_text("/approve");
                                    app.handle_submit(&mut window);
                                    dirty = true;
                                }
                                ui::HitAction::Deny if app.stream.is_none() => {
                                    app.input.set_text("/deny");
                                    app.handle_submit(&mut window);
                                    dirty = true;
                                }
                                ui::HitAction::Send if app.stream.is_none() => {
                                    app.handle_submit(&mut window);
                                    dirty = true;
                                }
                                ui::HitAction::Clear => {
                                    app.input.clear();
                                    app.sync_input_mode();
                                    app.status = String::from("Input cleared.");
                                    dirty = true;
                                }
                                _ => {}
                            }
                        }
                    } else if let sex_pdx::app_pointer::Kind::Wheel { steps } = p.kind {
                        if steps > 0 {
                            app.scroll_lines = app.scroll_lines.saturating_add(3);
                        } else if steps < 0 {
                            app.scroll_lines = app.scroll_lines.saturating_sub(3);
                        }
                        dirty = true;
                    }
                }
                Event::Key(key) if key.control && !key.shift && !key.alt && key.code == 0x2e && app.stream.is_some() => {
                    if let Some(mut active) = app.stream.take() {
                        active.duplex.close(&mut window);
                        app.push_log("\n[cancelled]");
                        app.status = String::from("Stream cancelled.");
                    }
                    dirty = true;
                }
                Event::Key(key) if key.control && !key.shift && !key.alt && key.code == 0x2e && app.input.editor.doc.selection().is_none() => {
                    if !app.input.text().is_empty() {
                        let _ = sex_app::clipboard::copy(&mut window, app.input.text());
                        app.status = String::from("Copied prompt to clipboard.");
                    } else if !app.transcript.is_empty() {
                        let _ = sex_app::clipboard::copy(&mut window, &app.transcript);
                        app.status = String::from("Copied transcript to clipboard.");
                    }
                    dirty = true;
                }
                Event::Key(key) if !key.shift && !key.control && !key.alt && app.input.super_down == 0 && matches!(sex_app::nav(key), Some(Nav::Up)) => {
                    app.scroll_lines = app.scroll_lines.saturating_add(3);
                    dirty = true;
                }
                Event::Key(key) if !key.shift && !key.control && !key.alt && app.input.super_down == 0 && matches!(sex_app::nav(key), Some(Nav::Down)) => {
                    app.scroll_lines = app.scroll_lines.saturating_sub(3);
                    dirty = true;
                }
                Event::Key(key) => {
                    if app.stream.is_none() && app.input.focused {
                        let ke = app.input.handle_key(&mut window, key);
                        app.sync_input_mode();
                        if ke.submitted {
                            app.handle_submit(&mut window);
                        }
                        if ke.handled || ke.changed || ke.submitted {
                            dirty = true;
                        }
                    }
                }
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
