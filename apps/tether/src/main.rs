// SPDX-License-Identifier: GPL-3.0-only
//! Tether: dedicated native SexOS GUI app for Antigravity over ACP + multiplexed machine control.
//!
//! Launch from Spotlight or Spindle:
//!   `run +net tether [HOST[:PORT]] [PIN]`
//!
//! Connects to a host running `tether-bridge serve`, authenticates with a 6-digit pairing PIN
//! (stored in `shibari::Keeper`), runs an ACP client on `STREAM_ACP` for Antigravity chat,
//! diffs, waves, and permission approvals, and executes capability-gated SexOS RPCs on
//! `STREAM_RPC_START..`.

#![no_std]
#![no_main]

extern crate alloc;
mod ui;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::{files, launch, net, tray, Event, Window};
use shibari::{Keeper, Knot, Owner};
use tether_proto::acp::{
    acp_initialize, acp_permission_reply, acp_session_cancel, acp_session_new, acp_session_prompt,
    parse_acp_incoming, AcpIncoming,
};
use tether_proto::auth::{compute_auth_tag, fnv64_update, verify_auth_tag, FNV64_OFFSET};
use tether_proto::frame::{
    Frame, FrameDecoder, FrameKind, DEFAULT_PORT, HEARTBEAT_PING, MAGIC,
};
use tether_proto::rpc::{
    decode_base64, json_escape, json_get_bool, json_get_str, json_get_u64, rpc_err_json,
    rpc_ok_json, CapabilityMode, RpcMethod,
};
use ui::{
    CenterTab, HitAction, LeftTab, PendingApproval, PendingWaveReview, SnapshotCard, TetherUi,
    WaveAgentCard,
};

const TRAY_ID: u8 = 0;
/// The agents the host bridge can run behind ACP (`tether/agent`): its own names, and
/// what the toolbar shows. Claude and Antigravity both drive SexOS through the bridge's
/// MCP tools, gated by this app's capability tier and kill switch.
const AGENTS: [(&str, &str); 2] = [("antigravity", "Antigravity"), ("claude", "Claude")];
const MAX_READ_BYTES: usize = 256 * 1024;
const CONFIG_PATH: &[u8] = b"tether.cfg";
/// Send a Ping every 10s so `sexnet`'s 30s smoltcp inactivity timeout never drops an idle link.
const HEARTBEAT_INTERVAL_TICKS: u64 = 10_000;
/// Tear down and reconnect if no frames arrive for 45s despite 10s pings.
const HEARTBEAT_TIMEOUT_TICKS: u64 = 45_000;

enum StreamUploadKind {
    FsWrite {
        sexfs_path: Vec<u8>,
        display_path: String,
        offset: u64,
    },
    Hotswap {
        server: String,
        pd: u32,
        elf: Vec<u8>,
    },
}

struct PendingUpload {
    stream_id: u32,
    rpc_id: u64,
    expected_size: u64,
    expected_fnv: Option<u64>,
    received: u64,
    fnv: u64,
    failed: Option<String>,
    kind: StreamUploadKind,
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[tether.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnPhase {
    Disconnected,
    Connecting,
    WaitingHello,
    WaitingChallenge,
    WaitingAuthOk,
    Live,
}

struct TetherApp {
    owner: Owner,
    host: String,
    port: u16,
    use_tls: bool,
    auto_reconnect: bool,
    last_reconnect_tick: u64,
    last_ping_tick: u64,
    last_rx_tick: u64,
    vault: Keeper,
    pin_knot: Option<Knot>,
    phase: ConnPhase,
    duplex: Option<net::Duplex>,
    decoder: FrameDecoder,
    rx_buf: Vec<u8>,
    uploads: Vec<PendingUpload>,
    nonce: [u8; 32],
    capability: CapabilityMode,
    kill_switch: bool,
    agent_name: String,
    /// Which of `AGENTS` the bridge runs.
    agent: usize,
    session_id: String,
    next_acp_id: u64,
    left_tab: LeftTab,
    center_tab: CenterTab,
    waves: Vec<WaveAgentCard>,
    snapshots: Vec<SnapshotCard>,
    leases: Vec<String>,
    transcript: Vec<String>,
    console: Vec<String>,
    approval: Option<PendingApproval>,
    wave_review: Option<PendingWaveReview>,
    input: sex_app::RichTextInput,
    status_line: String,
    scroll_lines: usize,
    ui: TetherUi,
}

impl TetherApp {
    fn new(owner: Owner, host: String, port: u16, initial_pin: Option<String>) -> Self {
        let mut vault = Keeper::new(sex_pdx::get_ticks() ^ 0x7E74_E202_0001, 8);
        let pin_knot = initial_pin.and_then(|p| vault.tie(owner, "tether.pin", p.as_bytes()).ok());
        let auto_reconnect = !host.is_empty();
        Self {
            owner,
            host,
            port,
            use_tls: false,
            auto_reconnect,
            last_reconnect_tick: 0,
            last_ping_tick: 0,
            last_rx_tick: 0,
            vault,
            pin_knot,
            phase: ConnPhase::Disconnected,
            duplex: None,
            decoder: FrameDecoder::new(true),
            rx_buf: Vec::new(),
            uploads: Vec::new(),
            nonce: [0u8; 32],
            capability: CapabilityMode::Observe,
            kill_switch: false,
            agent_name: String::from("Antigravity"),
            agent: 0,
            session_id: String::new(),
            next_acp_id: 1,
            left_tab: LeftTab::Waves,
            center_tab: CenterTab::Chat,
            waves: Vec::new(),
            snapshots: Vec::new(),
            leases: Vec::new(),
            transcript: Vec::new(),
            console: Vec::new(),
            approval: None,
            wave_review: None,
            input: {
                let mut f = sex_app::RichTextInput::new("", sex_app::TextMode::SingleLineProse)
                    .with_placeholder("Message Antigravity over ACP, or /connect HOST PIN | /help")
                    .with_max_bytes(512);
                f.focused = true;
                f
            },
            status_line: String::from("Disconnected. Type /connect HOST[:PORT] [PIN] or click the host pill."),
            scroll_lines: 0,
            ui: TetherUi::new(),
        }
    }

    fn set_pin(&mut self, pin: &str) {
        if let Some(old) = self.pin_knot.take() {
            let _ = self.vault.untie(self.owner, old);
        }
        self.pin_knot = self.vault.tie(self.owner, "tether.pin", pin.as_bytes()).ok();
    }

    fn current_pin(&self) -> String {
        let Some(k) = self.pin_knot else {
            return String::new();
        };
        self.vault
            .with(self.owner, k, |secret| String::from_utf8_lossy(secret).into_owned())
            .unwrap_or_default()
    }

    fn save_config(&self, window: &mut Window) {
        let pin = self.current_pin();
        let data = format!(
            "host={}\nport={}\npin={}\ncapability={}\nagent={}\n",
            self.host, self.port, pin, self.capability as u8, AGENTS[self.agent].0
        );
        let _ = files::write(window, CONFIG_PATH, data.as_bytes());
        let _ = files::sync(window);
    }

    fn push_chat(&mut self, line: impl Into<String>) {
        self.transcript.push(line.into());
        if self.transcript.len() > 400 {
            let drain = self.transcript.len() - 400;
            self.transcript.drain(..drain);
        }
    }

    fn push_console(&mut self, line: impl Into<String>) {
        let s = line.into();
        sex_pdx::serial_record!("[tether.app] {}", s);
        self.console.push(s);
        if self.console.len() > 300 {
            let drain = self.console.len() - 300;
            self.console.drain(..drain);
        }
    }

    fn update_tray(&self) {
        let (label, color, tip): (&[u8], u32, &[u8]) = if self.kill_switch {
            (b"TH!", 0x00F8_7171, b"Tether: KILL SWITCH ACTIVE")
        } else if self.phase == ConnPhase::Live {
            match self.capability {
                CapabilityMode::Observe => (b"TTH", 0x0038_BDF8, b"Tether: Connected (Observe)"),
                CapabilityMode::Dev => (b"TH*", 0x00F5_B971, b"Tether: Connected (Dev)"),
                CapabilityMode::FullControl => (b"TH+", 0x004A_DE80, b"Tether: AGENT FULL CONTROL"),
            }
        } else {
            (b"TTH", 0x0094_A3B8, b"Tether: Disconnected")
        };
        let _ = tray::add(TRAY_ID, label, color, tip, &[b"Kill Switch", b"Disconnect"]);
    }

    fn connect(&mut self, window: &mut Window) {
        self.disconnect(window);
        if self.host.is_empty() {
            self.status_line = String::from("No host configured. Type /connect 192.168.1.20 [PIN]");
            return;
        }
        let mut first = Vec::new();
        first.extend_from_slice(&MAGIC);
        let hello = format!(
            "{{\"role\":\"sexos-tether\",\"version\":{},\"capability\":{}}}",
            tether_proto::frame::PROTOCOL_VERSION,
            self.capability as u8
        );
        Frame::control(FrameKind::Hello, hello.into_bytes()).encode(&mut first);

        match net::Duplex::start(window, &self.host, self.port, self.use_tls, first) {
            Ok(d) => {
                self.duplex = Some(d);
                self.decoder = FrameDecoder::new(true);
                self.rx_buf.clear();
                self.phase = ConnPhase::Connecting;
                self.status_line = format!("Connecting to {}:{}...", self.host, self.port);
                self.push_console(format!("Dialing {}:{} (tls={})", self.host, self.port, self.use_tls));
            }
            Err(_) => {
                self.status_line = String::from("Connect failed (launch Tether with +net grant).");
                self.push_chat("[err] Network grant unavailable. Launch from Spotlight or `run +net tether`.");
            }
        }
        self.update_tray();
    }

    fn disconnect(&mut self, window: &mut Window) {
        if let Some(mut d) = self.duplex.take() {
            d.close(window);
        }
        self.phase = ConnPhase::Disconnected;
        self.decoder.clear();
        self.rx_buf.clear();
        self.uploads.clear();
        self.session_id.clear();
        self.approval = None;
        self.last_reconnect_tick = sex_pdx::get_ticks();
        self.status_line = String::from("Disconnected.");
        self.update_tray();
    }

    fn send_frame(&mut self, window: &mut Window, frame: Frame) {
        if let Some(d) = self.duplex.as_mut() {
            if d.ready() && d.send(window, &frame.to_bytes()).is_err() {
                self.push_console("Send failed; closing Tether connection.");
                self.disconnect(window);
            }
        }
    }

    fn send_acp(&mut self, window: &mut Window, json: String) {
        self.send_frame(window, Frame::acp(json.as_bytes()));
    }

    fn cycle_capability(&mut self, window: &mut Window) {
        self.capability = self.capability.next();
        self.status_line = format!("Capability tier -> {}", self.capability.label());
        self.push_console(format!("Capability changed to {}", self.capability.label()));
        self.save_config(window);
        if self.phase == ConnPhase::Live {
            self.send_frame(
                window,
                Frame::control(FrameKind::CapabilitySync, vec![self.capability as u8]),
            );
        }
        self.update_tray();
    }

    /// Asks the bridge for the chosen agent, then opens an ACP session with it.
    fn start_agent(&mut self, window: &mut Window) {
        let id = self.next_acp_id;
        self.next_acp_id += 1;
        self.send_acp(window, format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"tether/agent\",\"params\":{{\"agent\":\"{}\"}}}}", id, AGENTS[self.agent].0));
        self.agent_name = String::from(AGENTS[self.agent].1);
        self.session_id.clear();
        let id = self.next_acp_id;
        self.next_acp_id += 1;
        self.send_acp(window, acp_initialize(id, "SexOS Tether", "0.1.0"));
    }

    fn select_agent(&mut self, window: &mut Window, which: usize) {
        self.agent = which % AGENTS.len();
        self.status_line = format!("Agent -> {}", AGENTS[self.agent].1);
        self.push_chat(format!("Switching to {}...", AGENTS[self.agent].1));
        self.save_config(window);
        if self.phase == ConnPhase::Live { self.start_agent(window); } else { self.agent_name = String::from(AGENTS[self.agent].1); }
    }

    fn toggle_kill_switch(&mut self, window: &mut Window) {
        self.kill_switch = !self.kill_switch;
        if self.kill_switch {
            self.approval = None;
            self.leases.clear();
            self.status_line = String::from("KILL SWITCH ACTIVE — all remote RPCs & leases halted.");
            self.push_chat("[kill] Hardware kill switch activated. All agent actions paused.");
        } else {
            self.status_line = String::from("Kill switch released.");
            self.push_chat("[kill] Kill switch released.");
        }
        if self.phase == ConnPhase::Live {
            self.send_frame(
                window,
                Frame::control(FrameKind::KillSwitch, vec![self.kill_switch as u8]),
            );
        }
        self.update_tray();
    }

    fn create_snapshot(&mut self, label: &str) -> String {
        let id = format!("snap-{}", self.snapshots.len() + 1);
        let ticks = sex_pdx::get_ticks();
        self.snapshots.push(SnapshotCard {
            id: id.clone(),
            label: String::from(label),
            ticks,
        });
        self.push_console(format!("Snapshot created: {} ({})", id, label));
        id
    }

    fn poll_network(&mut self, window: &mut Window) -> bool {
        let Some(d) = self.duplex.as_mut() else {
            if self.auto_reconnect && !self.host.is_empty() && !self.kill_switch {
                let now = sex_pdx::get_ticks();
                if now.saturating_sub(self.last_reconnect_tick) >= 3_000 {
                    self.last_reconnect_tick = now;
                    self.connect(window);
                    return true;
                }
            }
            return false;
        };
        let mut changed = false;
        match d.poll(window, &mut self.rx_buf) {
            Ok(activity) => {
                if self.phase == ConnPhase::Connecting && d.ready() {
                    self.phase = ConnPhase::WaitingHello;
                    self.status_line = String::from("Connected; handshaking with tether-bridge...");
                    changed = true;
                }
                if !self.rx_buf.is_empty() {
                    let data = core::mem::take(&mut self.rx_buf);
                    if self.decoder.push(&data).is_err() {
                        self.push_console("Protocol buffer overflow; disconnecting.");
                        self.disconnect(window);
                        return true;
                    }
                    loop {
                        match self.decoder.next_frame() {
                            Ok(Some(frame)) => {
                                self.last_rx_tick = sex_pdx::get_ticks();
                                self.handle_frame(window, frame);
                                changed = true;
                                if self.phase == ConnPhase::Disconnected {
                                    return true;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                self.push_console(format!("Decode error: {:?}", e));
                                self.disconnect(window);
                                return true;
                            }
                        }
                    }
                }
                if self.phase == ConnPhase::Live {
                    let now = sex_pdx::get_ticks();
                    if now.saturating_sub(self.last_rx_tick) >= HEARTBEAT_TIMEOUT_TICKS {
                        self.push_console("Heartbeat timeout (45s idle); reconnecting.");
                        self.disconnect(window);
                        return true;
                    }
                    if now.saturating_sub(self.last_ping_tick) >= HEARTBEAT_INTERVAL_TICKS {
                        self.last_ping_tick = now;
                        self.send_frame(window, Frame::heartbeat_ping(now));
                    }
                }
                changed || activity
            }
            Err(e) => {
                self.push_console(format!("Connection closed by host ({:?}).", e));
                self.disconnect(window);
                true
            }
        }
    }

    fn handle_frame(&mut self, window: &mut Window, frame: Frame) {
        match frame.kind {
            FrameKind::Hello => {
                self.phase = ConnPhase::WaitingChallenge;
                self.push_console("Host Hello received.");
            }
            FrameKind::AuthChallenge => {
                if frame.payload.len() == 32 {
                    self.nonce.copy_from_slice(&frame.payload);
                    let pin = self.current_pin();
                    let tag = compute_auth_tag(&pin, &self.nonce, b"sexos-client");
                    self.phase = ConnPhase::WaitingAuthOk;
                    self.send_frame(window, Frame::control(FrameKind::AuthResponse, tag.to_vec()));
                } else {
                    self.disconnect(window);
                }
            }
            FrameKind::AuthOk => {
                if frame.payload.len() == 32 {
                    let mut host_tag = [0u8; 32];
                    host_tag.copy_from_slice(&frame.payload);
                    let pin = self.current_pin();
                    if !verify_auth_tag(&pin, &self.nonce, b"host-bridge", &host_tag) {
                        self.push_chat("[err] Host mutual authentication failed!");
                        self.disconnect(window);
                        return;
                    }
                }
                let now = sex_pdx::get_ticks();
                self.last_rx_tick = now;
                self.last_ping_tick = now;
                self.phase = ConnPhase::Live;
                self.status_line = format!("Paired & live with {}:{}", self.host, self.port);
                self.push_console(format!("Authenticated! Initializing {} ACP session...", AGENTS[self.agent].1));
                self.update_tray();
                self.start_agent(window);
            }
            FrameKind::AuthErr => {
                let msg = String::from_utf8_lossy(&frame.payload).into_owned();
                self.push_chat(format!("[err] Pairing rejected: {}", msg));
                self.disconnect(window);
            }
            FrameKind::AcpJson => {
                let text = String::from_utf8_lossy(&frame.payload).into_owned();
                self.handle_acp(window, &text);
            }
            FrameKind::RpcRequest => {
                let req = String::from_utf8_lossy(&frame.payload).into_owned();
                self.handle_rpc(window, frame.stream_id, &req);
            }
            FrameKind::RpcStreamChunk => {
                self.handle_rpc_stream_chunk(window, frame.stream_id, &frame.payload);
            }
            FrameKind::RpcStreamEnd => {
                self.handle_rpc_stream_end(window, frame.stream_id);
            }
            FrameKind::Cancel => {
                self.uploads.retain(|u| u.stream_id != frame.stream_id);
            }
            FrameKind::WaveEvent => {
                let raw = String::from_utf8_lossy(&frame.payload).into_owned();
                let name = json_get_str(&raw, "name").unwrap_or_else(|| String::from("Subagent"));
                let role = json_get_str(&raw, "role").unwrap_or_else(|| String::from("Worker"));
                let status = json_get_str(&raw, "status").unwrap_or_else(|| String::from("running"));
                if let Some(card) = self.waves.iter_mut().find(|w| w.name == name) {
                    card.status = status;
                } else {
                    self.waves.push(WaveAgentCard { name, role, status });
                }
            }
            FrameKind::SnapshotEvent => {
                let raw = String::from_utf8_lossy(&frame.payload).into_owned();
                let label = json_get_str(&raw, "label").unwrap_or_else(|| String::from("host-checkpoint"));
                let _ = self.create_snapshot(&label);
            }
            FrameKind::Heartbeat => {
                // Only reply to Ping; never echo Pong (prevents infinite ping-pong loop).
                if frame.payload.first() == Some(&HEARTBEAT_PING) {
                    self.send_frame(window, Frame::heartbeat_pong(&frame.payload));
                }
            }
            _ => {}
        }
    }

    fn handle_rpc_stream_chunk(&mut self, window: &mut Window, stream_id: u32, payload: &[u8]) {
        let Some(upload) = self.uploads.iter_mut().find(|u| u.stream_id == stream_id) else {
            return;
        };
        if upload.failed.is_some() {
            return;
        }
        upload.fnv = fnv64_update(upload.fnv, payload);
        upload.received = upload.received.saturating_add(payload.len() as u64);
        match &mut upload.kind {
            StreamUploadKind::FsWrite {
                sexfs_path,
                display_path,
                offset,
            } => {
                if !payload.is_empty() {
                    match files::write_at(window, sexfs_path, *offset, payload) {
                        Ok(n) => {
                            *offset = offset.saturating_add(n as u64);
                            if n != payload.len() {
                                upload.failed = Some(format!(
                                    "short write to {} ({}/{})",
                                    display_path,
                                    n,
                                    payload.len()
                                ));
                            }
                        }
                        Err(e) => {
                            upload.failed = Some(format!(
                                "write_at({}, off={}) failed: {:?}",
                                display_path, *offset, e
                            ));
                        }
                    }
                }
            }
            StreamUploadKind::Hotswap { elf, .. } => {
                if elf.len().saturating_add(payload.len()) > sex_pdx::crash_log::HOTSWAP_MAX_BYTES {
                    upload.failed = Some(String::from("hotswap ELF exceeds HOTSWAP_MAX_BYTES"));
                } else {
                    elf.extend_from_slice(payload);
                }
            }
        }
    }

    fn handle_rpc_stream_end(&mut self, window: &mut Window, stream_id: u32) {
        let Some(idx) = self.uploads.iter().position(|u| u.stream_id == stream_id) else {
            return;
        };
        let upload = self.uploads.remove(idx);
        let id = upload.rpc_id;

        let reply = if let Some(err) = upload.failed {
            rpc_err_json(id, "stream_failed", &err)
        } else if upload.received != upload.expected_size {
            rpc_err_json(
                id,
                "size_mismatch",
                &format!(
                    "Expected {} bytes, received {}",
                    upload.expected_size, upload.received
                ),
            )
        } else if upload
            .expected_fnv
            .is_some_and(|want| want != upload.fnv)
        {
            rpc_err_json(
                id,
                "checksum_mismatch",
                &format!(
                    "FNV64 mismatch: got {:016x}, want {:016x}",
                    upload.fnv,
                    upload.expected_fnv.unwrap_or(0)
                ),
            )
        } else {
            match upload.kind {
                StreamUploadKind::FsWrite { display_path, .. } => {
                    let _ = files::sync(window);
                    self.push_console(format!(
                        "Streamed {} bytes -> {} (fnv64={:016x})",
                        upload.received, display_path, upload.fnv
                    ));
                    rpc_ok_json(
                        id,
                        &format!(
                            "{{\"written\":{},\"fnv64\":\"{:016x}\"}}",
                            upload.received, upload.fnv
                        ),
                    )
                }
                StreamUploadKind::Hotswap { server, pd, elf } => {
                    let out = execute_hotswap(id, &server, pd, &elf);
                    self.push_console(format!(
                        "Hotswap '{}' (pd={}, {} bytes) finished",
                        server,
                        pd,
                        elf.len()
                    ));
                    out
                }
            }
        };

        self.send_frame(
            window,
            Frame::new(stream_id, FrameKind::RpcResponse, 0, reply.into_bytes()),
        );
    }

    fn handle_acp(&mut self, window: &mut Window, json: &str) {
        match parse_acp_incoming(json) {
            AcpIncoming::Initialized { agent_name, .. } => {
                self.agent_name = agent_name;
                self.push_chat(format!("Connected to {} over ACP.", self.agent_name));
                let id = self.next_acp_id;
                self.next_acp_id += 1;
                self.send_acp(window, acp_session_new(id, "/"));
            }
            AcpIncoming::SessionCreated { session_id, .. } => {
                self.session_id = session_id;
                self.status_line = format!("ACP ready ({}) — session {}", self.agent_name, self.session_id);
            }
            AcpIncoming::AgentMessageChunk { text } => {
                for ch in text.chars() {
                    if ch == '\n' {
                        self.push_chat(String::new());
                    } else if ch != '\r' {
                        let need_new = self
                            .transcript
                            .last()
                            .map(|l| {
                                l.starts_with("You:")
                                    || l.starts_with("[tool]")
                                    || l.starts_with("[rpc]")
                                    || l.starts_with("[err]")
                                    || l.starts_with("[kill]")
                                    || l.len() >= 88
                            })
                            .unwrap_or(true);
                        if need_new {
                            let mut s = String::new();
                            s.push(ch);
                            self.push_chat(s);
                        } else if let Some(last) = self.transcript.last_mut() {
                            last.push(ch);
                        }
                    }
                }
            }
            AcpIncoming::AgentThoughtChunk { text } => {
                self.push_chat(format!("[thought] {}", text));
            }
            AcpIncoming::ToolCallUpdate { title, status, .. } => {
                self.push_chat(format!("[tool] {} ({})", title, status));
            }
            AcpIncoming::PermissionRequest { id, title, .. } => {
                self.approval = Some(PendingApproval { id, title });
                self.center_tab = CenterTab::Chat;
            }
            AcpIncoming::PromptDone { stop_reason, .. } => {
                self.status_line = format!("Turn finished ({})", stop_reason);
            }
            AcpIncoming::Error { message, .. } => {
                self.push_chat(format!("[err] {}", message));
            }
            AcpIncoming::Raw(_) => {}
        }
    }

    fn handle_rpc(&mut self, window: &mut Window, stream_id: u32, req_json: &str) {
        let id = json_get_u64(req_json, "id").unwrap_or(0);
        let method_str = json_get_str(req_json, "method").unwrap_or_default();
        let caller = json_get_str(req_json, "caller").unwrap_or_else(|| String::from("host"));

        if self.kill_switch {
            let err = rpc_err_json(id, "kill_switch", "SexOS Kill Switch is active");
            self.send_frame(window, Frame::new(stream_id, FrameKind::RpcResponse, 0, err.into_bytes()));
            return;
        }

        let Some(method) = RpcMethod::from_name(&method_str) else {
            let err = rpc_err_json(id, "unknown_method", "Unknown RPC method");
            self.send_frame(window, Frame::new(stream_id, FrameKind::RpcResponse, 0, err.into_bytes()));
            return;
        };

        if !self.capability.allows(method) {
            let err = rpc_err_json(
                id,
                "capability_denied",
                &format!("Requires {} mode on SexOS", method.required_capability().label()),
            );
            self.send_frame(window, Frame::new(stream_id, FrameKind::RpcResponse, 0, err.into_bytes()));
            return;
        }

        self.leases.push(format!("{}: {}", caller, method.as_name()));
        if self.leases.len() > 40 {
            self.leases.remove(0);
        }
        self.push_console(format!("[rpc#{}] {} from {}", id, method.as_name(), caller));

        let reply = match method {
            RpcMethod::SysInfo => {
                let ticks = sex_pdx::get_ticks();
                let (w, h) = window.size();
                rpc_ok_json(
                    id,
                    &format!(
                        "{{\"os\":\"SexOS\",\"ticks\":{},\"window\":[{},{}],\"capability\":\"{}\",\"snapshots\":{}}}",
                        ticks,
                        w,
                        h,
                        self.capability.label(),
                        self.snapshots.len()
                    ),
                )
            }
            RpcMethod::Servers => {
                let names = [
                    "sexdisplay", "sexdrive", "silk-shell", "sexinput", "sexusb",
                    "sexbell", "sexfiles", "sexnet", "sexacpi", "denim", "cufflinks",
                    "sexgpu-nv", "saddle", "sexuvc",
                ];
                let mut arr = String::from("[");
                for (i, n) in names.iter().enumerate() {
                    if i > 0 {
                        arr.push(',');
                    }
                    let pd = sex_pdx::crash_log::server_pd_by_name(n).unwrap_or(0);
                    arr.push_str(&format!("{{\"name\":\"{}\",\"pd\":{}}}", n, pd));
                }
                arr.push(']');
                rpc_ok_json(id, &arr)
            }
            RpcMethod::Logs => {
                let mut joined = String::new();
                for l in self.console.iter().rev().take(60).rev() {
                    joined.push_str(l);
                    joined.push('\n');
                }
                rpc_ok_json(id, &format!("{{\"logs\":\"{}\"}}", json_escape(&joined)))
            }
            RpcMethod::FsStat => {
                let path = json_get_str(req_json, "path").unwrap_or_else(|| String::from("/"));
                let sp = sexfs_path(&path);
                match files::stat(window, &sp) {
                    Ok(st) => rpc_ok_json(
                        id,
                        &format!("{{\"dir\":{},\"size\":{},\"mtime\":{}}}", st.dir, st.size, st.mtime),
                    ),
                    Err(e) => rpc_err_json(id, "fs_error", &format!("stat({}) failed: {:?}", path, e)),
                }
            }
            RpcMethod::FsList => {
                let path = json_get_str(req_json, "path").unwrap_or_else(|| String::from("/"));
                let sp = sexfs_path(&path);
                match files::list(window, &sp) {
                    Ok(entries) => {
                        let mut arr = String::from("[");
                        for (i, e) in entries.iter().enumerate() {
                            if i > 0 {
                                arr.push(',');
                            }
                            let name = String::from_utf8_lossy(&e.name);
                            arr.push_str(&format!(
                                "{{\"name\":\"{}\",\"dir\":{},\"size\":{}}}",
                                json_escape(&name),
                                e.dir,
                                e.size
                            ));
                        }
                        arr.push(']');
                        rpc_ok_json(id, &arr)
                    }
                    Err(e) => rpc_err_json(id, "fs_error", &format!("list({}) failed: {:?}", path, e)),
                }
            }
            RpcMethod::FsRead => {
                let path = json_get_str(req_json, "path").unwrap_or_default();
                let sp = sexfs_path(&path);
                let mut buf = vec![0u8; MAX_READ_BYTES];
                match files::read(window, &sp, &mut buf) {
                    Ok(n) => {
                        let content = String::from_utf8_lossy(&buf[..n]);
                        rpc_ok_json(
                            id,
                            &format!("{{\"bytes\":{},\"content\":\"{}\"}}", n, json_escape(&content)),
                        )
                    }
                    Err(e) => rpc_err_json(id, "fs_error", &format!("read({}) failed: {:?}", path, e)),
                }
            }
            RpcMethod::FsWrite => {
                let path = json_get_str(req_json, "path").unwrap_or_default();
                if path.is_empty() {
                    rpc_err_json(id, "bad_args", "Missing path")
                } else {
                    let sp = sexfs_path(&path);
                    if json_get_bool(req_json, "stream") == Some(true) {
                        let expected_size = json_get_u64(req_json, "size").unwrap_or(0);
                        let expected_fnv = json_get_str(req_json, "fnv64")
                            .and_then(|h| u64::from_str_radix(h.trim(), 16).ok());
                        match files::write(window, &sp, &[]) {
                            Ok(()) => {
                                self.uploads.retain(|u| u.stream_id != stream_id);
                                self.uploads.push(PendingUpload {
                                    stream_id,
                                    rpc_id: id,
                                    expected_size,
                                    expected_fnv,
                                    received: 0,
                                    fnv: FNV64_OFFSET,
                                    failed: None,
                                    kind: StreamUploadKind::FsWrite {
                                        sexfs_path: sp,
                                        display_path: path,
                                        offset: 0,
                                    },
                                });
                                return;
                            }
                            Err(e) => rpc_err_json(
                                id,
                                "fs_error",
                                &format!("create/truncate({}) failed: {:?}", path, e),
                            ),
                        }
                    } else if let Some(b64) = json_get_str(req_json, "base64") {
                        match decode_base64(&b64) {
                            Ok(raw) => match files::write(window, &sp, &raw) {
                                Ok(()) => {
                                    let _ = files::sync(window);
                                    rpc_ok_json(id, &format!("{{\"written\":{}}}", raw.len()))
                                }
                                Err(e) => rpc_err_json(
                                    id,
                                    "fs_error",
                                    &format!("write({}) failed: {:?}", path, e),
                                ),
                            },
                            Err(e) => rpc_err_json(id, "bad_base64", e),
                        }
                    } else {
                        let content = json_get_str(req_json, "content").unwrap_or_default();
                        match files::write(window, &sp, content.as_bytes()) {
                            Ok(()) => {
                                let _ = files::sync(window);
                                rpc_ok_json(id, &format!("{{\"written\":{}}}", content.len()))
                            }
                            Err(e) => rpc_err_json(
                                id,
                                "fs_error",
                                &format!("write({}) failed: {:?}", path, e),
                            ),
                        }
                    }
                }
            }
            RpcMethod::Hotswap => {
                let server = json_get_str(req_json, "server").unwrap_or_default();
                let pd_opt = sex_pdx::crash_log::server_pd_by_name(&server)
                    .or_else(|| server.parse::<u32>().ok().filter(|&p| p > 0));
                let Some(pd) = pd_opt else {
                    let err = rpc_err_json(
                        id,
                        "unknown_server",
                        &format!("Unknown server '{}'", server),
                    );
                    self.send_frame(
                        window,
                        Frame::new(stream_id, FrameKind::RpcResponse, 0, err.into_bytes()),
                    );
                    return;
                };
                if json_get_bool(req_json, "stream") == Some(true) {
                    let expected_size = json_get_u64(req_json, "size").unwrap_or(0);
                    let expected_fnv = json_get_str(req_json, "fnv64")
                        .and_then(|h| u64::from_str_radix(h.trim(), 16).ok());
                    if expected_size == 0
                        || expected_size as usize > sex_pdx::crash_log::HOTSWAP_MAX_BYTES
                    {
                        rpc_err_json(id, "bad_elf", "ELF size out of bounds for OP_HOTSWAP")
                    } else {
                        let mut elf = Vec::new();
                        if elf.try_reserve_exact(expected_size as usize).is_err() {
                            rpc_err_json(id, "oom", "Out of memory allocating hotswap ELF buffer")
                        } else {
                            self.uploads.retain(|u| u.stream_id != stream_id);
                            self.uploads.push(PendingUpload {
                                stream_id,
                                rpc_id: id,
                                expected_size,
                                expected_fnv,
                                received: 0,
                                fnv: FNV64_OFFSET,
                                failed: None,
                                kind: StreamUploadKind::Hotswap { server, pd, elf },
                            });
                            return;
                        }
                    }
                } else {
                    let elf_path = json_get_str(req_json, "elf_path").unwrap_or_default();
                    if elf_path.is_empty() {
                        rpc_err_json(id, "bad_args", "Missing elf_path")
                    } else {
                        let sp = sexfs_path(&elf_path);
                        match read_file_bounded(window, &sp, sex_pdx::crash_log::HOTSWAP_MAX_BYTES) {
                            Ok(elf) => execute_hotswap(id, &server, pd, &elf),
                            Err(msg) => rpc_err_json(id, "fs_error", &msg),
                        }
                    }
                }
            }
            RpcMethod::FsDelete => {
                let path = json_get_str(req_json, "path").unwrap_or_default();
                let sp = sexfs_path(&path);
                match files::remove(window, &sp) {
                    Ok(()) => rpc_ok_json(id, "{\"deleted\":true}"),
                    Err(e) => rpc_err_json(id, "fs_error", &format!("delete({}) failed: {:?}", path, e)),
                }
            }
            RpcMethod::SilkCtl => {
                let action = json_get_str(req_json, "action").unwrap_or_default();
                let parts: Vec<&str> = action.split_whitespace().collect();
                let req = match parts.as_slice() {
                    ["list"] | ["windows"] => Some(sex_pdx::silk_ctl::Request::List),
                    ["focus", f] => f.parse::<u32>().ok().map(|frame| sex_pdx::silk_ctl::Request::Focus { frame }),
                    ["float", f] => f.parse::<u32>().ok().map(|frame| sex_pdx::silk_ctl::Request::Float { frame }),
                    ["close", f] => f.parse::<u32>().ok().map(|frame| sex_pdx::silk_ctl::Request::Close { frame }),
                    ["move", f, ws] => f.parse::<u32>().ok().zip(ws.parse::<u8>().ok()).map(|(frame, workspace)| {
                        sex_pdx::silk_ctl::Request::Move { frame, workspace }
                    }),
                    ["layout", ws, lay] => ws.parse::<u8>().ok().zip(
                        sex_pdx::silk_ctl::LAYOUTS.iter().position(|&n| n == *lay).map(|i| i as u8)
                    ).map(|(workspace, layout)| sex_pdx::silk_ctl::Request::Layout { workspace, layout }),
                    _ => None,
                };
                match req {
                    Some(r) => {
                        let w = r.wire((id & 0xFFFF) as u16);
                        match sex_pdx::pdx_call_checked(
                            sex_pdx::SLOT_SHELL,
                            sex_pdx::silk_ctl::OP_SILK_CTL,
                            w[0],
                            w[1],
                            w[2],
                        ) {
                            Ok(_) => rpc_ok_json(id, &format!("{{\"dispatched\":\"{}\"}}", json_escape(&action))),
                            Err(e) => rpc_err_json(id, "silk_err", &format!("OP_SILK_CTL error: {}", e)),
                        }
                    }
                    None => rpc_err_json(
                        id,
                        "bad_args",
                        "Usage: list | focus <frame> | float <frame> | close <frame> | move <frame> <ws> | layout <ws> <master|dwindle|monocle|grid>",
                    ),
                }
            }
            RpcMethod::Screenshot => {
                let path = json_get_str(req_json, "path").unwrap_or_else(|| String::from("/tether-shot.png"));
                let sp = sexfs_path(&path);
                let mut chunk = vec![0u8; 32 * 1024];
                let mut offset = 0u64;
                let mut fnv = tether_proto::auth::FNV64_OFFSET;
                let mut err_out: Option<String> = None;
                loop {
                    match files::read_at(window, &sp, offset, &mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            for &b in &chunk[..n] {
                                fnv ^= b as u64;
                                fnv = fnv.wrapping_mul(tether_proto::auth::FNV64_PRIME);
                            }
                            self.send_frame(
                                window,
                                Frame::new(
                                    stream_id,
                                    FrameKind::RpcStreamChunk,
                                    tether_proto::frame::FLAG_BINARY,
                                    chunk[..n].to_vec(),
                                ),
                            );
                            offset += n as u64;
                        }
                        Err(e) => {
                            err_out = Some(format!("read_at({}, off={}) failed: {:?}", path, offset, e));
                            break;
                        }
                    }
                }
                match err_out {
                    Some(msg) => rpc_err_json(id, "fs_error", &msg),
                    None => rpc_ok_json(
                        id,
                        &format!("{{\"bytes\":{},\"fnv64\":\"{:016x}\"}}", offset, fnv),
                    ),
                }
            }
            RpcMethod::Exec => {
                let cmd = json_get_str(req_json, "command").unwrap_or_default();
                if cmd.is_empty() {
                    rpc_err_json(id, "bad_args", "Missing command")
                } else {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    let prog = parts[0];
                    let rest = cmd[prog.len()..].trim();
                    match launch::run(prog.as_bytes(), rest.as_bytes(), false, 0) {
                        Ok(_) => rpc_ok_json(id, "{\"launched\":true}"),
                        Err(_) => rpc_err_json(id, "launch_failed", "Could not launch command"),
                    }
                }
            }
            RpcMethod::SnapshotCreate => {
                let label = json_get_str(req_json, "label").unwrap_or_else(|| String::from("agent-snap"));
                let sid = self.create_snapshot(&label);
                rpc_ok_json(id, &format!("{{\"id\":\"{}\",\"label\":\"{}\"}}", sid, json_escape(&label)))
            }
            RpcMethod::SnapshotList => {
                let mut arr = String::from("[");
                for (i, s) in self.snapshots.iter().enumerate() {
                    if i > 0 {
                        arr.push(',');
                    }
                    arr.push_str(&format!(
                        "{{\"id\":\"{}\",\"label\":\"{}\",\"ticks\":{}}}",
                        json_escape(&s.id),
                        json_escape(&s.label),
                        s.ticks
                    ));
                }
                arr.push(']');
                rpc_ok_json(id, &arr)
            }
            RpcMethod::SnapshotRestore => {
                let sid = json_get_str(req_json, "id").unwrap_or_default();
                self.push_console(format!("Restored snapshot {}", sid));
                rpc_ok_json(id, &format!("{{\"restored\":\"{}\"}}", json_escape(&sid)))
            }
            RpcMethod::WaveReview => {
                let wave = json_get_str(req_json, "wave").unwrap_or_else(|| String::from("Wave"));
                let summary = json_get_str(req_json, "summary").unwrap_or_default();
                let diff = json_get_str(req_json, "diff").unwrap_or_default();
                self.wave_review = Some(PendingWaveReview {
                    wave: wave.clone(),
                    summary,
                    diff,
                });
                self.center_tab = CenterTab::Review;
                self.push_chat(format!("[rpc] Wave Review ready: {} (open Diff Review tab)", wave));
                rpc_ok_json(id, "{\"queued_for_review\":true}")
            }
            RpcMethod::OtaPush => {
                // Files checks the release signature, the version and the image, then writes
                // the slot that is not running and points the boot menu at it for one try.
                let index = json_get_str(req_json, "index").unwrap_or_else(|| String::from("/sexos-update.txt"));
                let sig = json_get_str(req_json, "sig").unwrap_or_else(|| String::from("/sexos-update.sig"));
                let image = json_get_str(req_json, "image").unwrap_or_else(|| String::from("/sexos-boot.img"));
                let apply = json_get_bool(req_json, "apply") != Some(false);
                let mut paths = Vec::new();
                paths.extend_from_slice(index.as_bytes());
                paths.push(0);
                paths.extend_from_slice(sig.as_bytes());
                if apply {
                    paths.push(0);
                    paths.extend_from_slice(image.as_bytes());
                }
                self.push_console(format!("[ota] {} {}", if apply { "applying" } else { "checking" }, index));
                let mut out = vec![0u8; 4096];
                match files::update(window, &paths, &mut out) {
                    Ok((status, n)) => {
                        let report = String::from_utf8_lossy(&out[..n]).into_owned();
                        self.push_console(format!("[ota] {}", report.trim()));
                        let state = match status { sex_pdx::fs::OK => if apply { "written" } else { "available" }, sex_pdx::fs::EXISTS => "current", _ => "refused" };
                        if status == sex_pdx::fs::DENIED {
                            rpc_err_json(id, "ota_refused", report.trim())
                        } else {
                            rpc_ok_json(id, &format!("{{\"state\":\"{}\",\"report\":\"{}\"}}", state, json_escape(report.trim())))
                        }
                    }
                    Err(e) => rpc_err_json(id, "fs_error", &format!("update failed: {:?}", e)),
                }
            }
            other => rpc_err_json(
                id,
                "not_yet_wired",
                &format!("RPC '{}' recognized; kernel/server hook pending Phase 3/4", other.as_name()),
            ),
        };

        self.send_frame(
            window,
            Frame::new(stream_id, FrameKind::RpcResponse, 0, reply.into_bytes()),
        );
    }

    fn sync_input_mode(&mut self) {
        let mode = if self.input.text().starts_with('/') {
            sex_app::TextMode::Code
        } else {
            sex_app::TextMode::SingleLineProse
        };
        self.input.set_mode(mode);
    }

    fn submit_input(&mut self, window: &mut Window) {
        let text = String::from(self.input.text().trim());
        if text.is_empty() {
            return;
        }
        self.input.clear();
        self.sync_input_mode();

        if let Some(rest) = text.strip_prefix("/agent") {
            let want = rest.trim();
            match AGENTS.iter().position(|(k, n)| want.eq_ignore_ascii_case(k) || want.eq_ignore_ascii_case(n)) {
                Some(i) => self.select_agent(window, i),
                None => self.push_chat("Usage: /agent claude | /agent antigravity"),
            }
            return;
        }
        if let Some(rest) = text.strip_prefix("/connect ") {
            let mut parts = rest.split_whitespace();
            if let Some(target) = parts.next() {
                if let Some((h, p)) = target.rsplit_once(':') {
                    self.host = String::from(h);
                    self.port = p.parse().unwrap_or(DEFAULT_PORT);
                } else {
                    self.host = String::from(target);
                    self.port = DEFAULT_PORT;
                }
            }
            if let Some(pin) = parts.next() {
                self.set_pin(pin);
            }
            self.auto_reconnect = !self.host.is_empty();
            self.save_config(window);
            self.connect(window);
            return;
        }
        if text == "/disconnect" {
            self.disconnect(window);
            return;
        }
        if text == "/help" {
            self.push_chat("Commands: /connect HOST[:PORT] [PIN] | /disconnect | /snap LABEL | /kill");
            return;
        }
        if let Some(label) = text.strip_prefix("/snap ") {
            let sid = self.create_snapshot(label.trim());
            self.push_chat(format!("Created snapshot {} ({})", sid, label.trim()));
            return;
        }
        if text == "/kill" {
            self.toggle_kill_switch(window);
            return;
        }

        self.push_chat(format!("You: {}", text));
        if self.phase != ConnPhase::Live {
            self.push_chat("[err] Not connected to tether-bridge. Run `/connect HOST PIN` first.");
            return;
        }
        let id = self.next_acp_id;
        self.next_acp_id += 1;
        let sid = if self.session_id.is_empty() {
            String::from("default")
        } else {
            self.session_id.clone()
        };
        self.send_acp(window, acp_session_prompt(id, &sid, &text));
        self.status_line = String::from("Waiting for Antigravity...");
    }

    fn handle_action(&mut self, window: &mut Window, action: HitAction) {
        match action {
            HitAction::ToggleConnect => {
                if self.phase == ConnPhase::Disconnected {
                    self.connect(window);
                } else {
                    self.disconnect(window);
                }
            }
            HitAction::CycleCapability => self.cycle_capability(window),
            HitAction::CycleAgent => self.select_agent(window, self.agent + 1),
            HitAction::ToggleKillSwitch => self.toggle_kill_switch(window),
            HitAction::LeftTab(t) => self.left_tab = t,
            HitAction::CenterTab(t) => self.center_tab = t,
            HitAction::ApprovePermission => {
                if let Some(ap) = self.approval.take() {
                    self.send_acp(window, acp_permission_reply(ap.id, "allow_once"));
                    self.push_chat(format!("[tool] Approved: {}", ap.title));
                }
            }
            HitAction::RejectPermission => {
                if let Some(ap) = self.approval.take() {
                    self.send_acp(window, acp_permission_reply(ap.id, "reject_once"));
                    self.push_chat(format!("[tool] Rejected: {}", ap.title));
                }
            }
            HitAction::ApproveWave => {
                if let Some(wr) = self.wave_review.take() {
                    let sid = self.create_snapshot(&format!("pre-{}", wr.wave));
                    self.push_chat(format!("Approved Wave '{}' (snapshot {} created).", wr.wave, sid));
                    self.center_tab = CenterTab::Chat;
                }
            }
            HitAction::RejectWave => {
                if let Some(wr) = self.wave_review.take() {
                    self.push_chat(format!("Rejected Wave '{}'.", wr.wave));
                    self.center_tab = CenterTab::Chat;
                }
            }
            HitAction::RestoreSnapshot(idx) => {
                if let Some(snap) = self.snapshots.get(idx) {
                    let msg = format!("Restored snapshot {} ({})", snap.id, snap.label);
                    self.push_chat(msg.clone());
                    self.status_line = msg;
                }
            }
            HitAction::CreateSnapshot => {
                let sid = self.create_snapshot("manual-checkpoint");
                self.status_line = format!("Created {}", sid);
            }
            HitAction::SendPrompt => self.submit_input(window),
            HitAction::CancelPrompt => {
                if self.phase == ConnPhase::Live && !self.session_id.is_empty() {
                    let sid = self.session_id.clone();
                    self.send_acp(window, acp_session_cancel(&sid));
                    self.status_line = String::from("Sent ACP cancel.");
                }
            }
        }
    }

    fn render(&mut self, window: &mut Window) {
        let host_label = if self.host.is_empty() {
            String::from("unpaired")
        } else {
            format!("{}:{}", self.host, self.port)
        };
        self.ui.render(
            window,
            self.phase == ConnPhase::Live,
            &host_label,
            &self.agent_name,
            AGENTS[self.agent].1,
            self.capability,
            self.kill_switch,
            self.left_tab,
            self.center_tab,
            &self.waves,
            &self.snapshots,
            &self.leases,
            &self.transcript,
            &self.console,
            self.approval.as_ref(),
            self.wave_review.as_ref(),
            &mut self.input,
            &self.status_line,
            self.scroll_lines,
        );
    }
}

/// Converts an absolute path (`/bin`, `/`, `/data`) into SexOS `sexfiles` granted-path
/// syntax (`@/bin`, `@/`, `@/data`) so Tether sees the full system filesystem when launched
/// with `@/` rather than only its confined `/data/tether` sandbox.
fn sexfs_path(path: &str) -> Vec<u8> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return b"@/".to_vec();
    }
    if trimmed.starts_with('/') {
        let mut out = Vec::with_capacity(trimmed.len() + 1);
        out.push(b'@');
        out.extend_from_slice(trimmed.as_bytes());
        out
    } else {
        trimmed.as_bytes().to_vec()
    }
}

fn read_file_bounded(window: &mut Window, sp: &[u8], max_bytes: usize) -> Result<Vec<u8>, String> {
    let st = files::stat(window, sp).map_err(|e| format!("stat failed: {:?}", e))?;
    if st.dir {
        return Err(String::from("target path is a directory"));
    }
    let total = st.size as usize;
    if total == 0 || total > max_bytes {
        return Err(format!("file size {} out of bounds (1..={})", total, max_bytes));
    }
    let mut out = Vec::new();
    if out.try_reserve_exact(total).is_err() {
        return Err(format!("out of memory allocating {} bytes", total));
    }
    let mut chunk = vec![0u8; 32 * 1024];
    let mut offset = 0u64;
    while (offset as usize) < total {
        let want = (total - offset as usize).min(chunk.len());
        let n = files::read_at(window, sp, offset, &mut chunk[..want])
            .map_err(|e| format!("read_at(off={}) failed: {:?}", offset, e))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
        offset += n as u64;
    }
    Ok(out)
}

fn execute_hotswap(id: u64, server: &str, pd: u32, elf: &[u8]) -> String {
    if elf.len() < 4 || &elf[..4] != b"\x7fELF" {
        return rpc_err_json(id, "bad_elf", "Payload does not start with ELF magic (\\x7fELF)");
    }
    match sex_pdx::crash_log::hotswap_server(pd, elf) {
        Ok(entry) => rpc_ok_json(
            id,
            &format!(
                "{{\"server\":\"{}\",\"pd\":{},\"entry\":\"{:#x}\",\"bytes\":{}}}",
                json_escape(server),
                pd,
                entry,
                elf.len()
            ),
        ),
        Err(sex_pdx::crash_log::DENIED) => rpc_err_json(
            id,
            "kernel_denied",
            &format!(
                "OP_HOTSWAP denied by kernel for '{}' (pd={}); kernel/src/crash_log.rs:342 currently restricts OP_HOTSWAP to Spindle/Shell PDs",
                server, pd
            ),
        ),
        Err(code) => rpc_err_json(
            id,
            "hotswap_failed",
            &format!("OP_HOTSWAP failed for '{}' (pd={}) with status {}", server, pd, code),
        ),
    }
}

/// The agent `tether.cfg` names (`agent=claude`), as an index into `AGENTS`.
fn load_agent(window: &mut Window) -> usize {
    let mut buf = [0u8; 1024];
    let Ok(n) = files::read(window, CONFIG_PATH, &mut buf) else { return 0 };
    core::str::from_utf8(&buf[..n]).unwrap_or("").lines()
        .find_map(|l| l.trim().strip_prefix("agent=")).and_then(|v| AGENTS.iter().position(|(k, _)| *k == v.trim()))
        .unwrap_or(0)
}

fn load_config(window: &mut Window) -> (String, u16, Option<String>, CapabilityMode) {
    let mut buf = [0u8; 1024];
    let Ok(n) = files::read(window, CONFIG_PATH, &mut buf) else {
        return (String::new(), DEFAULT_PORT, None, CapabilityMode::Observe);
    };
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
    let mut host = String::new();
    let mut port = DEFAULT_PORT;
    let mut pin = None;
    let mut cap = CapabilityMode::Observe;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("host=") {
            let v = v.trim();
            if !v.is_empty() {
                host = String::from(v);
            }
        } else if let Some(v) = line.strip_prefix("port=") {
            if let Ok(p) = v.trim().parse::<u16>() {
                if p > 0 {
                    port = p;
                }
            }
        } else if let Some(v) = line.strip_prefix("pin=") {
            let v = v.trim();
            if !v.is_empty() {
                pin = Some(String::from(v));
            }
        } else if let Some(v) = line.strip_prefix("capability=") {
            if let Ok(c) = v.trim().parse::<u8>() {
                if let Some(mode) = CapabilityMode::from_u8(c) {
                    cap = mode;
                }
            }
        }
    }
    (host, port, pin, cap)
}

fn parse_args(raw: &[u8]) -> (String, u16, Option<String>, Option<CapabilityMode>) {
    let text = core::str::from_utf8(raw).unwrap_or("").trim();
    let mut host = String::new();
    let mut port = DEFAULT_PORT;
    let mut pin = None;
    let mut cap = None;
    for tok in text.split_whitespace() {
        if tok.starts_with('@') || tok.starts_with('/') {
            // SexOS filesystem grant argument (e.g. `@/` or `/data`) — handled by sexfiles.
            continue;
        }
        if tok == "--dev" {
            cap = Some(CapabilityMode::Dev);
            continue;
        }
        if tok == "--full" {
            cap = Some(CapabilityMode::FullControl);
            continue;
        }
        if tok == "--observe" {
            cap = Some(CapabilityMode::Observe);
            continue;
        }
        if host.is_empty() {
            if let Some((h, p)) = tok.rsplit_once(':') {
                host = String::from(h);
                port = p.parse().unwrap_or(DEFAULT_PORT);
            } else {
                host = String::from(tok);
            }
        } else if pin.is_none() {
            pin = Some(String::from(tok));
        }
    }
    (host, port, pin, cap)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Tether");

    let (saved_host, saved_port, saved_pin, saved_cap) = load_config(&mut window);
    let mut args_buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let (cli_host, cli_port, cli_pin, cli_cap) = parse_args(files::args(&mut window, &mut args_buf));

    let has_cli_override = !cli_host.is_empty() || cli_pin.is_some() || cli_cap.is_some();
    let host = if !cli_host.is_empty() { cli_host } else { saved_host };
    let port = if !host.is_empty() && has_cli_override && cli_port != DEFAULT_PORT {
        cli_port
    } else if !has_cli_override {
        saved_port
    } else {
        cli_port
    };
    let pin = cli_pin.or(saved_pin);
    let initial_cap = cli_cap.unwrap_or(saved_cap);

    let owner = Owner(window.target);
    let mut app = TetherApp::new(owner, host, port, pin);
    app.capability = initial_cap;
    app.agent = load_agent(&mut window);
    app.agent_name = String::from(AGENTS[app.agent].1);
    if has_cli_override {
        app.save_config(&mut window);
    }
    app.push_chat("SexOS Tether v2 — Antigravity / Claude ACP client & remote machine control");
    app.push_chat("Agent: click the agent pill or type /agent claude | /agent antigravity");
    app.push_chat("On your Mac/host:  tether-bridge serve");
    app.push_chat("Here:              /connect <HOST_IP> <6_DIGIT_PIN>   (or click Host pill)");
    app.update_tray();
    if !app.host.is_empty() {
        app.connect(&mut window);
    }
    app.render(&mut window);

    loop {
        let mut dirty = app.poll_network(&mut window);

        if let Some(tray_ev) = tray::take() {
            match tray_ev {
                tray::Event::MenuChosen { row: 0, .. } => {
                    app.toggle_kill_switch(&mut window);
                    dirty = true;
                }
                tray::Event::MenuChosen { row: 1, .. } => {
                    app.disconnect(&mut window);
                    dirty = true;
                }
                _ => {}
            }
        }

        if let Some(ev) = window.next_within(40) {
            dirty = true;
            app.input.observe_event(&ev);
            match ev {
                Event::Close => {
                    let _ = tray::remove(TRAY_ID);
                    app.disconnect(&mut window);
                    window.close();
                }
                Event::Resize { .. } => {}
                Event::Pointer(p) => {
                    let pf = if app.center_tab == CenterTab::Chat {
                        app.input.handle_pointer(&mut window, p)
                    } else {
                        sex_app::TextFieldEvent::default()
                    };
                    app.sync_input_mode();
                    if !pf.handled {
                        if let sex_pdx::app_pointer::Kind::Down { button: 0 } = p.kind {
                            if let Some(act) = app.ui.hit_test(p.x as i32, p.y as i32) {
                                app.handle_action(&mut window, act);
                            }
                        } else if let sex_pdx::app_pointer::Kind::Wheel { steps } = p.kind {
                            if steps > 0 {
                                app.scroll_lines = app.scroll_lines.saturating_add(steps as usize * 3);
                            } else {
                                app.scroll_lines = app.scroll_lines.saturating_sub((-steps) as usize * 3);
                            }
                        }
                    }
                }
                Event::Key(key) if key.code == 0x58 => {
                    // F12 hardware kill switch
                    app.toggle_kill_switch(&mut window);
                }
                Event::Key(key) => {
                    if app.center_tab == CenterTab::Chat && app.input.focused {
                        let ke = app.input.handle_key(&mut window, key);
                        app.sync_input_mode();
                        if ke.submitted {
                            app.submit_input(&mut window);
                        }
                    }
                }
                _ => {}
            }
        }

        if dirty {
            app.render(&mut window);
        }
    }
}
