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

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, launch, net, Event, Nav, Window};
use shibari::{Keeper, Knot, Owner};
use tailor_core::{
    Agent, AgentStep, ApprovalDecision, Provider, ProviderStreamParser, StreamEvent, ToolRunner,
    Workspace,
};

const MAX_READ_BYTES: usize = 256 * 1024;
const READ_CHUNK_BYTES: usize = 32 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 24 * 1024;
const COMMAND_TIMEOUT_TICKS: u64 = 15_000;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[tailor.fatal] {}", info);
    sex_pdx::task_finish(1);
}

struct KeyVault {
    owner: Owner,
    keeper: Keeper,
    gemini: Option<Knot>,
    claude: Option<Knot>,
    claude_deepseek: Option<Knot>,
    codex: Option<Knot>,
}

impl KeyVault {
    fn new(owner: Owner, salt: u64) -> Self {
        Self {
            owner,
            keeper: Keeper::new(salt, 8),
            gemini: None,
            claude: None,
            claude_deepseek: None,
            codex: None,
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
        Ok(())
    }

    fn has_key(&self, provider: Provider) -> bool {
        self.slot(provider).is_some()
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

    fn load_from_files(&mut self, window: &mut Window) -> usize {
        let mut loaded = 0usize;
        // 1. Load JSON key bundles first (supports gemini, claude, claude-deepseek, codex).
        let mut jbuf = vec![0u8; 4096];
        for path in [
            b"/data/tailor/keys.json".as_slice(),
            b"/media/Ventoy/tailor-keys.json".as_slice(),
            b"/media/Ventoy/keys.json".as_slice(),
            b"keys.json".as_slice(),
        ] {
            if let Ok(n) = files::read(window, path, &mut jbuf) {
                if let Ok(raw) = core::str::from_utf8(&jbuf[..n]) {
                    for (provider, secret) in tailor_core::parse_keys_json(raw) {
                        if !self.has_key(provider)
                            && self.store(provider, secret.as_bytes()).is_ok()
                        {
                            loaded += 1;
                        }
                    }
                }
            }
            for b in &mut jbuf {
                unsafe { core::ptr::write_volatile(b, 0) };
            }
        }
        // 2. Fallback to per-provider *.key files.
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
            let abs = format!("/data/tailor/{}", provider.key_filename());
            let candidates = [abs.as_bytes(), provider.key_filename().as_bytes()];
            for path in candidates {
                if let Ok(n) = files::read(window, path, &mut buf) {
                    if let Ok(raw) = core::str::from_utf8(&buf[..n]) {
                        let trimmed = raw.trim();
                        if !trimmed.is_empty() && self.store(provider, trimmed.as_bytes()).is_ok() {
                            loaded += 1;
                            break;
                        }
                    }
                }
            }
            // Wipe temporary buffer after each read.
            for b in &mut buf {
                unsafe { core::ptr::write_volatile(b, 0) };
            }
        }
        loaded
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
    duplex: net::Duplex,
    parser: ProviderStreamParser,
    rx_buf: Vec<u8>,
}

struct AppState {
    workspace_root: String,
    agent: Agent,
    vault: KeyVault,
    runner: SexToolRunner,
    transcript: String,
    input: String,
    status: String,
    scroll_lines: usize,
    stream: Option<ActiveStream>,
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

    fn start_model_turn(&mut self, window: &mut Window) {
        let provider = self.agent.provider();
        let req = match self
            .vault
            .with_key(provider, |key| self.agent.build_request(key))
        {
            Some(r) => r,
            None => {
                self.status = format!(
                    "No {} API key. Put it in /data/tailor/{} or run /key {} <secret>",
                    provider.display_name(),
                    provider.key_filename(),
                    provider.name()
                );
                return;
            }
        };

        match net::Duplex::start(window, &req.host, req.port, true, req.http_bytes) {
            Ok(duplex) => {
                self.push_delta(&format!("\n[{}] ", provider.display_name()));
                self.stream = Some(ActiveStream {
                    duplex,
                    parser: ProviderStreamParser::new(provider),
                    rx_buf: Vec::with_capacity(4096),
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

        if !active.rx_buf.is_empty() {
            changed = true;
            let events = active.parser.feed(&active.rx_buf);
            for ev in events {
                if self.apply_stream_event(&ev) {
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
            self.handle_command(cmd);
            return;
        }

        self.push_log(&format!("\n> {}", trimmed));
        self.agent.push_user_message(trimmed);
        self.start_model_turn(window);
    }

    fn handle_command(&mut self, cmd: &str) {
        let mut parts = cmd.splitn(2, ' ');
        let verb = parts.next().unwrap_or("").trim();
        let rest = parts.next().unwrap_or("").trim();

        match verb {
            "provider" => match Provider::from_name(rest) {
                Some(p) => {
                    self.agent.set_provider(p);
                    self.status = format!(
                        "Switched to {} ({}) [key={}]",
                        p.display_name(),
                        self.agent.model(),
                        if self.vault.has_key(p) { "loaded" } else { "missing" }
                    );
                    self.push_log(&format!("[tailor] {}", self.status));
                }
                None => {
                    self.status = String::from("Usage: /provider gemini|claude|codex");
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
            "key" => {
                let mut kp = rest.splitn(2, ' ');
                let prov_str = kp.next().unwrap_or("").trim();
                let secret = kp.next().unwrap_or("").trim();
                match (Provider::from_name(prov_str), secret.is_empty()) {
                    (Some(p), false) => match self.vault.store(p, secret.as_bytes()) {
                        Ok(()) => {
                            self.status = format!(
                                "Stored {} key in Shibari RAM vault.",
                                p.display_name()
                            );
                            self.push_log(&format!("[tailor] {}", self.status));
                        }
                        Err(e) => self.status = String::from(e),
                    },
                    _ => {
                        self.status = String::from("Usage: /key gemini|claude|codex <secret>");
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
                     /provider gemini|claude|codex   Switch AI provider\n\
                     /model <name>                   Override model name\n\
                     /key <provider> <secret>        Store API key in Shibari RAM vault\n\
                     /approve (or y)                 Approve pending mutating tool\n\
                     /deny [reason] (or n)           Deny pending mutating tool\n\
                     /clear                          Clear conversation history\n\
                     Up/Down scroll · Ctrl+C cancel stream · F11 close",
                );
            }
            _ => {
                self.status = format!("Unknown command '/{}'. Try /help.", verb);
            }
        }
    }

    fn render(&self, window: &mut Window) {
        let p = self.agent.provider();
        let key_badge = if self.vault.has_key(p) { "key:ok" } else { "key:none" };
        let ws_label = if self.workspace_root.is_empty() {
            "(run folder)"
        } else {
            &self.workspace_root
        };
        let header = format!(
            "Tailor · {} ({}) · {} · ws: {}\n----------------------------------------------------------------\n",
            p.display_name(),
            self.agent.model(),
            key_badge,
            ws_label
        );
        let footer = format!(
            "\n----------------------------------------------------------------\n> {}|\n{}",
            self.input, self.status
        );

        let budget = (sex_pdx::text_region::BYTES as usize)
            .saturating_sub(header.len() + footer.len() + 16);

        let lines: Vec<&str> = self.transcript.lines().collect();
        let end = lines.len().saturating_sub(self.scroll_lines);
        let mut chosen = Vec::new();
        let mut used = 0usize;
        for line in lines[..end].iter().rev() {
            let cost = line.len() + 1;
            if used + cost > budget {
                break;
            }
            used += cost;
            chosen.push(*line);
        }
        chosen.reverse();
        let body = chosen.join("\n");

        let mut frame = String::with_capacity(header.len() + body.len() + footer.len());
        frame.push_str(&header);
        frame.push_str(&body);
        frame.push_str(&footer);
        window.show(frame.as_bytes());
    }
}

fn parse_launch_args(raw: &[u8]) -> (String, Provider) {
    let text = core::str::from_utf8(raw).unwrap_or("");
    let mut workspace = String::new();
    let mut provider = Provider::Gemini;
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
    let mut vault = KeyVault::new(owner, sex_pdx::get_ticks() ^ 0x5A17_7A11_0001);
    let loaded = vault.load_from_files(&mut window);

    let mut app = AppState {
        workspace_root,
        agent: Agent::new(initial_provider),
        vault,
        runner: SexToolRunner::new(),
        transcript: String::new(),
        input: String::new(),
        status: format!(
            "Ready ({} key(s) loaded in Shibari). Type a prompt or /help.",
            loaded
        ),
        scroll_lines: 0,
        stream: None,
    };
    app.push_log("Tailor — Native Multi-Provider AI Coding Agent for SexOS.");
    app.push_log("Type a coding request, or /help for providers, keys, and approvals.");
    app.render(&mut window);

    sex_pdx::serial_record!(
        "[tailor.ready] target={} task={} provider={} keys={}",
        window.target,
        window.task,
        app.agent.provider().name(),
        loaded
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
                Event::Key(key) if key.control && key.code == 0x2e => {
                    // Ctrl+C: cancel active stream or clear input.
                    if let Some(mut active) = app.stream.take() {
                        active.duplex.close(&mut window);
                        app.push_log("\n[cancelled]");
                        app.status = String::from("Stream cancelled.");
                    } else {
                        app.input.clear();
                        app.status = String::from("Input cleared.");
                    }
                    dirty = true;
                }
                Event::Key(key) if key.code == 0x1C => {
                    // Enter
                    if app.stream.is_none() {
                        app.handle_submit(&mut window);
                        dirty = true;
                    }
                }
                Event::Key(key) if key.code == 0x0E => {
                    // Backspace
                    if app.stream.is_none() {
                        app.input.pop();
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
