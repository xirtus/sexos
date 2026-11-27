// SPDX-License-Identifier: GPL-3.0-only
//! Host-side Tether v2 Bridge:
//! - Listens for SexOS `apps/tether` connections on TCP (default 7178)
//! - Performs PIN-based HMAC-SHA256 mutual auth
//! - Bridges `STREAM_ACP` <-> `agy acp` (or `--mock-acp` for testing)
//! - Exposes a local control port (default `127.0.0.1:7179`) for `tether-bridge mcp` and CLI
//! - Enforces concurrent read vs exclusive mutating hardware leases for fanned-out subagents

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tether_proto::auth::{compute_auth_tag, verify_auth_tag};
use tether_proto::frame::{
    Frame, FrameDecoder, FrameKind, FLAG_ERROR, MAGIC, STREAM_RPC_START,
};
use tether_proto::rpc::{
    json_escape, json_get_str, json_get_u64, rpc_err_json, rpc_ok_json, CapabilityMode, RpcMethod,
};

pub struct BridgeConfig {
    pub listen_addr: String,
    pub control_addr: String,
    pub pin: Option<String>,
    pub acp_cmd: Vec<String>,
    pub mock_acp: bool,
}

struct PendingRpc {
    tx: mpsc::Sender<String>,
    chunks: Arc<Mutex<Vec<u8>>>,
}

pub struct BridgeState {
    pub connected: AtomicBool,
    pub session_gen: AtomicU64,
    pub peer_addr: Mutex<Option<String>>,
    pub capability_mode: Mutex<CapabilityMode>,
    pub kill_switch_active: AtomicBool,
    pub active_lease_holder: Mutex<Option<String>>,
    /// Exclusive hardware lease for mutating SexOS RPCs (prevents fanned-out agent collisions).
    pub mutating_lease: Mutex<()>,
    pub next_stream_id: AtomicU32,
    pub next_rpc_id: AtomicU64,
    pub out_tx: Mutex<Option<mpsc::Sender<Frame>>>,
    pending_rpcs: Mutex<HashMap<u32, PendingRpc>>,
    pub recent_logs: Mutex<Vec<String>>,
    pub snapshots: Mutex<Vec<String>>,
}

impl BridgeState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            connected: AtomicBool::new(false),
            session_gen: AtomicU64::new(0),
            peer_addr: Mutex::new(None),
            capability_mode: Mutex::new(CapabilityMode::Observe),
            kill_switch_active: AtomicBool::new(false),
            active_lease_holder: Mutex::new(None),
            mutating_lease: Mutex::new(()),
            next_stream_id: AtomicU32::new(STREAM_RPC_START),
            next_rpc_id: AtomicU64::new(1),
            out_tx: Mutex::new(None),
            pending_rpcs: Mutex::new(HashMap::new()),
            recent_logs: Mutex::new(Vec::new()),
            snapshots: Mutex::new(Vec::new()),
        })
    }

    pub fn send_frame(&self, frame: Frame) -> Result<(), String> {
        let guard = self.out_tx.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            tx.send(frame).map_err(|_| String::from("SexOS session writer closed"))
        } else {
            Err(String::from("SexOS is not connected"))
        }
    }

    pub fn status_json(&self) -> String {
        let connected = self.connected.load(Ordering::SeqCst);
        let peer = self
            .peer_addr
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
        let cap = *self.capability_mode.lock().unwrap();
        let killed = self.kill_switch_active.load(Ordering::SeqCst);
        let lease = self
            .active_lease_holder
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
        format!(
            "{{\"connected\":{},\"peer\":\"{}\",\"capability\":\"{}\",\"kill_switch\":{},\"active_lease\":\"{}\"}}",
            connected,
            json_escape(&peer),
            cap.label(),
            killed,
            json_escape(&lease)
        )
    }

    /// Executes a typed RPC against the connected SexOS machine, enforcing hardware leases.
    pub fn call_rpc(
        &self,
        method_name: &str,
        params_json: &str,
        caller: &str,
        timeout: Duration,
    ) -> String {
        let id = self.next_rpc_id.fetch_add(1, Ordering::SeqCst);
        if method_name == "status" {
            return rpc_ok_json(id, &self.status_json());
        }
        if !self.connected.load(Ordering::SeqCst) {
            return rpc_err_json(id, "not_connected", "SexOS Tether app is not connected");
        }
        if self.kill_switch_active.load(Ordering::SeqCst) {
            return rpc_err_json(id, "kill_switch", "SexOS hardware kill switch is active");
        }
        let Some(method) = RpcMethod::from_name(method_name) else {
            return rpc_err_json(id, "unknown_method", "Unknown Tether RPC method");
        };

        let cap = *self.capability_mode.lock().unwrap();
        if !cap.allows(method) {
            return rpc_err_json(
                id,
                "capability_denied",
                &format!(
                    "Method '{}' requires '{}' mode on SexOS (current: '{}')",
                    method.as_name(),
                    method.required_capability().label(),
                    cap.label()
                ),
            );
        }

        // For Screenshot, trigger Spindle's native `capture /tether-shot.png` if the
        // Spindle tether inbox exists on this host so we always grab a fresh frame.
        if method == RpcMethod::Screenshot {
            let inbox = std::path::Path::new("sexwork/tether/logs/inbox.txt");
            if inbox.exists() {
                if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(inbox) {
                    let _ = writeln!(f, "rm /tether-shot.png");
                    let _ = writeln!(f, "capture /tether-shot.png");
                    let _ = f.flush();
                    thread::sleep(Duration::from_millis(2000));
                }
            }
        }

        // Exclusive lease for mutating operations so fanned-out agents never stomp on SexOS.
        let _lease_guard = if method.is_mutating() {
            let guard = self.mutating_lease.lock().unwrap();
            *self.active_lease_holder.lock().unwrap() =
                Some(format!("{}:{}", caller, method.as_name()));
            Some(guard)
        } else {
            None
        };

        let stream_id = self.next_stream_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        let chunks = Arc::new(Mutex::new(Vec::new()));
        self.pending_rpcs.lock().unwrap().insert(
            stream_id,
            PendingRpc {
                tx,
                chunks: Arc::clone(&chunks),
            },
        );

        let payload = format!(
            "{{\"id\":{},\"method\":\"{}\",\"caller\":\"{}\",\"params\":{}}}",
            id,
            method.as_name(),
            json_escape(caller),
            if params_json.trim().is_empty() { "{}" } else { params_json }
        );

        let res = match self.send_frame(Frame::new(
            stream_id,
            FrameKind::RpcRequest,
            0,
            payload.into_bytes(),
        )) {
            Ok(()) => match rx.recv_timeout(timeout) {
                Ok(reply) => {
                    if method == RpcMethod::Screenshot {
                        let bytes = chunks.lock().unwrap();
                        if !bytes.is_empty() {
                            let ts = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let out_path = format!("/tmp/sexos-screenshot-{}.png", ts);
                            match std::fs::write(&out_path, &*bytes) {
                                Ok(()) => rpc_ok_json(
                                    id,
                                    &format!(
                                        "{{\"path\":\"{}\",\"bytes\":{},\"fnv64\":\"{:016x}\"}}",
                                        json_escape(&out_path),
                                        bytes.len(),
                                        tether_proto::auth::fnv64(&bytes)
                                    ),
                                ),
                                Err(e) => rpc_err_json(id, "write_failed", &e.to_string()),
                            }
                        } else {
                            reply
                        }
                    } else {
                        reply
                    }
                }
                Err(_) => {
                    let _ = self.send_frame(Frame::new(stream_id, FrameKind::Cancel, 0, Vec::new()));
                    rpc_err_json(id, "timeout", "Timed out waiting for SexOS RPC response")
                }
            },
            Err(e) => rpc_err_json(id, "send_failed", &e),
        };

        self.pending_rpcs.lock().unwrap().remove(&stream_id);
        if method.is_mutating() {
            *self.active_lease_holder.lock().unwrap() = None;
        }
        res
    }
}

fn generate_nonce() -> [u8; 32] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mut seed = Vec::new();
    seed.extend_from_slice(&nanos.to_le_bytes());
    seed.extend_from_slice(&pid.to_le_bytes());
    tether_proto::auth::sha256(&seed)
}

pub fn generate_pin() -> String {
    let nonce = generate_nonce();
    let v = u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]) % 1_000_000;
    format!("{:06}", v)
}

fn read_frame_blocking(stream: &mut TcpStream, dec: &mut FrameDecoder) -> Result<Frame, String> {
    let mut tmp = [0u8; 4096];
    loop {
        if let Some(frame) = dec.next_frame().map_err(|e| format!("decode error: {:?}", e))? {
            return Ok(frame);
        }
        let n = stream.read(&mut tmp).map_err(|e| format!("read error: {}", e))?;
        if n == 0 {
            return Err(String::from("peer closed"));
        }
        dec.push(&tmp[..n]).map_err(|e| format!("buffer error: {:?}", e))?;
    }
}

fn perform_host_handshake(
    stream: &mut TcpStream,
    dec: &mut FrameDecoder,
    pin: Option<&str>,
) -> Result<CapabilityMode, String> {
    // 1. Send magic + Hello
    let mut hello_wire = Vec::new();
    hello_wire.extend_from_slice(&MAGIC);
    let hello_json = format!(
        "{{\"role\":\"host-bridge\",\"version\":{},\"auth_required\":{}}}",
        tether_proto::frame::PROTOCOL_VERSION,
        pin.is_some()
    );
    Frame::control(FrameKind::Hello, hello_json.into_bytes()).encode(&mut hello_wire);
    stream.write_all(&hello_wire).map_err(|e| e.to_string())?;

    // 2. Receive client Hello
    let client_hello = read_frame_blocking(stream, dec)?;
    if client_hello.kind != FrameKind::Hello {
        return Err(String::from("expected Hello from SexOS client"));
    }
    let cap_u8 = json_get_u64(
        &String::from_utf8_lossy(&client_hello.payload),
        "capability",
    )
    .unwrap_or(0) as u8;
    let cap = CapabilityMode::from_u8(cap_u8).unwrap_or(CapabilityMode::Observe);

    // 3. Optional PIN mutual challenge-response
    if let Some(expected_pin) = pin {
        let nonce = generate_nonce();
        let chal = Frame::control(FrameKind::AuthChallenge, nonce.to_vec());
        stream.write_all(&chal.to_bytes()).map_err(|e| e.to_string())?;

        let resp = read_frame_blocking(stream, dec)?;
        if resp.kind != FrameKind::AuthResponse || resp.payload.len() != 32 {
            let err = Frame::control(FrameKind::AuthErr, b"invalid auth response".to_vec());
            let _ = stream.write_all(&err.to_bytes());
            return Err(String::from("invalid auth response frame"));
        }
        let mut tag = [0u8; 32];
        tag.copy_from_slice(&resp.payload);
        if !verify_auth_tag(expected_pin, &nonce, b"sexos-client", &tag) {
            let err = Frame::control(FrameKind::AuthErr, b"invalid pairing PIN".to_vec());
            let _ = stream.write_all(&err.to_bytes());
            return Err(String::from("invalid pairing PIN"));
        }
        let host_tag = compute_auth_tag(expected_pin, &nonce, b"host-bridge");
        let ok = Frame::control(FrameKind::AuthOk, host_tag.to_vec());
        stream.write_all(&ok.to_bytes()).map_err(|e| e.to_string())?;
    } else {
        let ok = Frame::control(FrameKind::AuthOk, Vec::new());
        stream.write_all(&ok.to_bytes()).map_err(|e| e.to_string())?;
    }

    Ok(cap)
}

enum AcpBackend {
    AgyStream {
        child: Child,
        stdin: std::process::ChildStdin,
        out_tx: mpsc::Sender<Frame>,
        last_prompt_id: Arc<AtomicU64>,
    },
    Subprocess {
        child: Child,
        stdin: std::process::ChildStdin,
    },
    Mock {
        out_tx: mpsc::Sender<Frame>,
    },
}

fn find_agy_binary() -> Option<String> {
    let candidates = [
        "/Applications/Gemini.app/Contents/Helpers/agy",
        "/Applications/Antigravity.app/Contents/Helpers/agy",
        "agy",
    ];
    for c in candidates {
        if c.starts_with('/') {
            if std::path::Path::new(c).exists() {
                return Some(String::from(c));
            }
        } else if Command::new("which")
            .arg(c)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Some(String::from(c));
        }
    }
    None
}

impl AcpBackend {
    fn start(cfg: &BridgeConfig, out_tx: mpsc::Sender<Frame>) -> Self {
        if cfg.mock_acp {
            return Self::Mock { out_tx };
        }
        // Prefer native `agy --input-format stream-json --output-format stream-json --print=""`
        // when using default `agy acp` or when `agy` is found on macOS.
        let use_agy_stream = cfg.acp_cmd.is_empty()
            || (cfg.acp_cmd.len() == 2 && cfg.acp_cmd[0] == "agy" && cfg.acp_cmd[1] == "acp");
        if use_agy_stream {
            if let Some(agy_bin) = find_agy_binary() {
                let mut cmd = Command::new(&agy_bin);
                cmd.args([
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--dangerously-skip-permissions",
                    "--print=",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
                match cmd.spawn() {
                    Ok(mut child) => {
                        eprintln!("tether-bridge: spawned real Antigravity backend: {}", agy_bin);
                        let stdin = child.stdin.take().expect("agy stdin");
                        let stdout = child.stdout.take().expect("agy stdout");
                        let tx_reader = out_tx.clone();
                        let last_prompt_id = Arc::new(AtomicU64::new(0));
                        let prompt_id_reader = Arc::clone(&last_prompt_id);
                        thread::spawn(move || {
                            let reader = BufReader::new(stdout);
                            let mut last_tool_step: Option<u64> = None;
                            let mut streamed_any_text = false;
                            for line in reader.lines().map_while(Result::ok) {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                let event = json_get_str(trimmed, "event").unwrap_or_default();
                                match event.as_str() {
                                    "step_update" => {
                                        let step_type = json_get_str(trimmed, "step_type").unwrap_or_default();
                                        let state = json_get_str(trimmed, "state").unwrap_or_default();
                                        let step_idx = json_get_u64(trimmed, "step_index").unwrap_or(0);
                                        if step_type == "user_input" {
                                            streamed_any_text = false;
                                            last_tool_step = None;
                                        }
                                        if let Some(delta) = json_get_str(trimmed, "text_delta") {
                                            if !delta.is_empty() {
                                                streamed_any_text = true;
                                                let chunk = format!(
                                                    "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"agy-live\",\"update\":{{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":\"{}\"}}}}}}}}",
                                                    json_escape(&delta)
                                                );
                                                if tx_reader.send(Frame::acp(chunk.as_bytes())).is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                        if !step_type.is_empty()
                                            && step_type != "user_input"
                                            && step_type != "agent_response"
                                            && step_type != "system_message"
                                            && state == "ACTIVE"
                                            && last_tool_step != Some(step_idx)
                                        {
                                            last_tool_step = Some(step_idx);
                                            let title = json_get_str(trimmed, "tool_name")
                                                .or_else(|| json_get_str(trimmed, "name"))
                                                .or_else(|| json_get_str(trimmed, "summary"))
                                                .unwrap_or_else(|| step_type.clone());
                                            eprintln!("tether-bridge: [agy step#{}] {} ({})", step_idx, title, state);
                                            let tc = format!(
                                                "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"agy-live\",\"update\":{{\"sessionUpdate\":\"tool_call_update\",\"toolCallId\":\"step-{}\",\"title\":\"{}\",\"status\":\"running\"}}}}}}",
                                                step_idx,
                                                json_escape(&title)
                                            );
                                            let _ = tx_reader.send(Frame::acp(tc.as_bytes()));
                                        }
                                    }
                                    "result" => {
                                        if !streamed_any_text {
                                            if let Some(resp) = json_get_str(trimmed, "response") {
                                                if !resp.is_empty() {
                                                    let chunk = format!(
                                                        "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"agy-live\",\"update\":{{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":\"{}\"}}}}}}}}",
                                                        json_escape(&resp)
                                                    );
                                                    let _ = tx_reader.send(Frame::acp(chunk.as_bytes()));
                                                }
                                            }
                                        }
                                        if let Some(err) = json_get_str(trimmed, "error") {
                                            if !err.is_empty() {
                                                let msg = format!(
                                                    "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32000,\"message\":\"{}\"}}}}",
                                                    json_escape(&err)
                                                );
                                                let _ = tx_reader.send(Frame::acp(msg.as_bytes()));
                                            }
                                        }
                                        let pid = prompt_id_reader.load(Ordering::SeqCst);
                                        if pid > 0 {
                                            let done = format!(
                                                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"stopReason\":\"end_turn\"}}}}",
                                                pid
                                            );
                                            let _ = tx_reader.send(Frame::acp(done.as_bytes()));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        });
                        return Self::AgyStream {
                            child,
                            stdin,
                            out_tx,
                            last_prompt_id,
                        };
                    }
                    Err(e) => {
                        eprintln!("tether-bridge: failed to spawn {}: {}; falling back to mock ACP", agy_bin, e);
                    }
                }
            }
        }
        if cfg.acp_cmd.is_empty() {
            return Self::Mock { out_tx };
        }
        let mut cmd = Command::new(&cfg.acp_cmd[0]);
        if cfg.acp_cmd.len() > 1 {
            cmd.args(&cfg.acp_cmd[1..]);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        match cmd.spawn() {
            Ok(mut child) => {
                let stdin = child.stdin.take().expect("acp stdin");
                let stdout = child.stdout.take().expect("acp stdout");
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            if out_tx.send(Frame::acp(trimmed.as_bytes())).is_err() {
                                break;
                            }
                        }
                    }
                });
                Self::Subprocess { child, stdin }
            }
            Err(e) => {
                eprintln!(
                    "tether-bridge: failed to spawn ACP backend {:?} ({}); falling back to mock ACP",
                    cfg.acp_cmd, e
                );
                Self::Mock { out_tx }
            }
        }
    }

    fn send_acp_json(&mut self, json: &str) {
        match self {
            Self::AgyStream {
                stdin,
                out_tx,
                last_prompt_id,
                ..
            } => {
                let method = json_get_str(json, "method").unwrap_or_default();
                let id = json_get_u64(json, "id").unwrap_or(1);
                match method.as_str() {
                    "initialize" => {
                        let reply = format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":1,\"agentInfo\":{{\"name\":\"Antigravity\",\"version\":\"2.0\"}}}}}}",
                            id
                        );
                        let _ = out_tx.send(Frame::acp(reply.as_bytes()));
                    }
                    "session/new" => {
                        let reply = format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"sessionId\":\"agy-live\"}}}}",
                            id
                        );
                        let _ = out_tx.send(Frame::acp(reply.as_bytes()));
                    }
                    "session/prompt" => {
                        let prompt = json_get_str(json, "text").unwrap_or_default();
                        last_prompt_id.store(id, Ordering::SeqCst);
                        eprintln!("tether-bridge: [SexOS -> Antigravity] {}", prompt);
                        let ndjson = format!(
                            "{{\"event\":\"user\",\"message\":{{\"content\":\"{}\"}}}}",
                            json_escape(&prompt)
                        );
                        let _ = writeln!(stdin, "{}", ndjson);
                        let _ = stdin.flush();
                    }
                    _ => {}
                }
            }
            Self::Subprocess { stdin, .. } => {
                let _ = writeln!(stdin, "{}", json.trim());
                let _ = stdin.flush();
            }
            Self::Mock { out_tx } => {
                let method = json_get_str(json, "method").unwrap_or_default();
                let id = json_get_u64(json, "id").unwrap_or(1);
                match method.as_str() {
                    "initialize" => {
                        let reply = format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":1,\"agentInfo\":{{\"name\":\"Antigravity (Mock ACP)\",\"version\":\"2.0.0\"}}}}}}",
                            id
                        );
                        let _ = out_tx.send(Frame::acp(reply.as_bytes()));
                    }
                    "session/new" => {
                        let reply = format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"sessionId\":\"sexos-mock-1\"}}}}",
                            id
                        );
                        let _ = out_tx.send(Frame::acp(reply.as_bytes()));
                    }
                    "session/prompt" => {
                        let prompt = json_get_str(json, "text").unwrap_or_default();
                        let chunk = format!(
                            "{{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{{\"sessionId\":\"sexos-mock-1\",\"update\":{{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":\"[Antigravity ACP Bridge] Received: {}\"}}}}}}}}",
                            json_escape(&prompt)
                        );
                        let done = format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"stopReason\":\"end_turn\"}}}}",
                            id
                        );
                        let _ = out_tx.send(Frame::acp(chunk.as_bytes()));
                        let _ = out_tx.send(Frame::acp(done.as_bytes()));
                    }
                    _ => {}
                }
            }
        }
    }
}

impl Drop for AcpBackend {
    fn drop(&mut self) {
        match self {
            Self::AgyStream { child, .. } | Self::Subprocess { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Self::Mock { .. } => {}
        }
    }
}

fn spawn_control_server(control_addr: String, state: Arc<BridgeState>) -> Result<TcpListener, String> {
    let listener = TcpListener::bind(&control_addr).map_err(|e| e.to_string())?;
    let listener_clone = listener.try_clone().map_err(|e| e.to_string())?;
    thread::spawn(move || {
        for conn in listener_clone.incoming().flatten() {
            let st = Arc::clone(&state);
            thread::spawn(move || {
                handle_control_client(conn, st);
            });
        }
    });
    Ok(listener)
}

fn handle_control_client(mut stream: TcpStream, state: Arc<BridgeState>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let method = json_get_str(trimmed, "method").unwrap_or_else(|| String::from("status"));
            let caller = json_get_str(trimmed, "caller").unwrap_or_else(|| String::from("cli"));
            let timeout_ms = json_get_u64(trimmed, "timeout_ms").unwrap_or(15_000);
            let reply = if method == "wave_event" {
                let _ = state.send_frame(Frame::control(
                    FrameKind::WaveEvent,
                    trimmed.as_bytes().to_vec(),
                ));
                rpc_ok_json(0, "{\"sent\":true}")
            } else if method == "snapshot_event" {
                state.snapshots.lock().unwrap().push(String::from(trimmed));
                let _ = state.send_frame(Frame::control(
                    FrameKind::SnapshotEvent,
                    trimmed.as_bytes().to_vec(),
                ));
                rpc_ok_json(0, "{\"sent\":true}")
            } else {
                // Pass through raw params slice if present, else `{}`
                let params = extract_params_object(trimmed).unwrap_or("{}");
                state.call_rpc(&method, params, &caller, Duration::from_millis(timeout_ms))
            };
            let _ = writeln!(stream, "{}", reply);
            let _ = stream.flush();
        }
        line.clear();
    }
}

fn extract_params_object(json: &str) -> Option<&str> {
    let idx = json.find("\"params\"")?;
    let after = &json[idx + "\"params\"".len()..];
    let colon = after.find(':')?;
    let val = after[colon + 1..].trim_start();
    if !val.starts_with('{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, b) in val.bytes().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&val[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn handle_sexos_session(
    mut stream: TcpStream,
    peer: String,
    cfg: &BridgeConfig,
    state: Arc<BridgeState>,
) {
    let mut dec = FrameDecoder::new(true);
    let cap = match perform_host_handshake(&mut stream, &mut dec, cfg.pin.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tether-bridge: handshake failed with {}: {}", peer, e);
            return;
        }
    };

    let my_gen = state.session_gen.fetch_add(1, Ordering::SeqCst) + 1;
    eprintln!(
        "tether-bridge: SexOS connected from {} (gen={}, capability: {})",
        peer,
        my_gen,
        cap.label()
    );
    state.connected.store(true, Ordering::SeqCst);
    state.kill_switch_active.store(false, Ordering::SeqCst);
    *state.peer_addr.lock().unwrap() = Some(peer.clone());
    *state.capability_mode.lock().unwrap() = cap;

    let (out_tx, out_rx) = mpsc::channel::<Frame>();
    *state.out_tx.lock().unwrap() = Some(out_tx.clone());

    let mut write_stream = stream.try_clone().expect("clone tcp stream");
    let writer_handle = thread::spawn(move || {
        while let Ok(frame) = out_rx.recv() {
            if write_stream.write_all(&frame.to_bytes()).is_err() {
                break;
            }
        }
    });

    let mut acp = AcpBackend::start(cfg, out_tx.clone());

    let mut tmp = [0u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if dec.push(&tmp[..n]).is_err() {
                    break;
                }
                while let Ok(Some(frame)) = dec.next_frame() {
                    match frame.kind {
                        FrameKind::AcpJson => {
                            if let Ok(s) = core::str::from_utf8(&frame.payload) {
                                acp.send_acp_json(s);
                            }
                        }
                        FrameKind::RpcStreamChunk => {
                            if let Some(pending) =
                                state.pending_rpcs.lock().unwrap().get(&frame.stream_id)
                            {
                                pending.chunks.lock().unwrap().extend_from_slice(&frame.payload);
                            }
                        }
                        FrameKind::RpcResponse => {
                            let reply = String::from_utf8_lossy(&frame.payload).into_owned();
                            if let Some(pending) =
                                state.pending_rpcs.lock().unwrap().remove(&frame.stream_id)
                            {
                                let _ = pending.tx.send(reply);
                            }
                        }
                        FrameKind::CapabilitySync => {
                            if let Some(&b) = frame.payload.first() {
                                if let Some(new_cap) = CapabilityMode::from_u8(b) {
                                    *state.capability_mode.lock().unwrap() = new_cap;
                                    eprintln!(
                                        "tether-bridge: SexOS capability changed to {}",
                                        new_cap.label()
                                    );
                                }
                            }
                        }
                        FrameKind::KillSwitch => {
                            let active = frame.payload.first().copied().unwrap_or(1) != 0;
                            state.kill_switch_active.store(active, Ordering::SeqCst);
                            eprintln!("tether-bridge: SexOS kill switch = {}", active);
                            if active {
                                let mut pending = state.pending_rpcs.lock().unwrap();
                                for (_, p) in pending.drain() {
                                    let _ = p.tx.send(rpc_err_json(
                                        0,
                                        "kill_switch",
                                        "Aborted by SexOS Kill Switch",
                                    ));
                                }
                            }
                        }
                        FrameKind::LogEvent => {
                            let line = String::from_utf8_lossy(&frame.payload).into_owned();
                            let mut logs = state.recent_logs.lock().unwrap();
                            logs.push(line);
                            if logs.len() > 2000 {
                                let drain = logs.len() - 2000;
                                logs.drain(..drain);
                            }
                        }
                        FrameKind::Heartbeat => {
                            let _ = out_tx.send(Frame::control(FrameKind::Heartbeat, frame.payload));
                        }
                        _ => {
                            if (frame.flags & FLAG_ERROR) != 0 {
                                eprintln!("tether-bridge: error frame on stream {}", frame.stream_id);
                            }
                        }
                    }
                }
            }
        }
    }

    // Only clear shared state if no newer session has already preempted us.
    if state.session_gen.load(Ordering::SeqCst) == my_gen {
        state.connected.store(false, Ordering::SeqCst);
        *state.peer_addr.lock().unwrap() = None;
        *state.out_tx.lock().unwrap() = None;
    }
    drop(out_tx);
    let _ = writer_handle.join();
    eprintln!("tether-bridge: SexOS {} (gen={}) disconnected", peer, my_gen);
}

pub fn run_serve(cfg: BridgeConfig) -> Result<(), String> {
    let state = BridgeState::new();
    let _ctrl = spawn_control_server(cfg.control_addr.clone(), Arc::clone(&state))?;
    let listener = TcpListener::bind(&cfg.listen_addr).map_err(|e| e.to_string())?;
    eprintln!("tether-bridge v2 listening on {}", cfg.listen_addr);
    eprintln!("  control port : {} (for `tether-bridge mcp` & CLI)", cfg.control_addr);
    if let Some(pin) = &cfg.pin {
        eprintln!("  pairing PIN  : {}  (enter in SexOS Tether app)", pin);
    } else {
        eprintln!("  pairing PIN  : DISABLED (--no-auth lab mode)");
    }
    eprintln!(
        "  ACP backend  : {}",
        if cfg.mock_acp {
            "mock-acp".to_string()
        } else {
            cfg.acp_cmd.join(" ")
        }
    );

    let cfg = Arc::new(cfg);
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| String::from("unknown"));
                let st = Arc::clone(&state);
                let cfg_c = Arc::clone(&cfg);
                thread::spawn(move || {
                    handle_sexos_session(stream, peer, &cfg_c, st);
                });
            }
            Err(e) => eprintln!("tether-bridge: accept error: {}", e),
        }
    }
    Ok(())
}

/// Sends a single JSON command to a running `tether-bridge serve` control port.
pub fn control_call(control_addr: &str, req_json: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(control_addr)
        .map_err(|e| format!("could not connect to tether-bridge at {}: {}", control_addr, e))?;
    writeln!(stream, "{}", req_json).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim().to_string())
}

/// Starts an ephemeral bridge on OS-assigned ports (used by integration tests).
#[cfg(test)]
pub fn start_test_bridge(pin: Option<String>) -> (String, String, Arc<BridgeState>) {
    let state = BridgeState::new();
    let ctrl = spawn_control_server(String::from("127.0.0.1:0"), Arc::clone(&state)).unwrap();
    let ctrl_addr = ctrl.local_addr().unwrap().to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let listen_addr = listener.local_addr().unwrap().to_string();
    let cfg = BridgeConfig {
        listen_addr: listen_addr.clone(),
        control_addr: ctrl_addr.clone(),
        pin,
        acp_cmd: Vec::new(),
        mock_acp: true,
    };
    let st = Arc::clone(&state);
    thread::spawn(move || {
        let _keep_ctrl = ctrl;
        if let Ok((stream, addr)) = listener.accept() {
            handle_sexos_session(stream, addr.to_string(), &cfg, st);
        }
    });
    (listen_addr, ctrl_addr, state)
}
