// SPDX-License-Identifier: GPL-3.0-only
//! Model Context Protocol (MCP) JSON-RPC 2.0 over stdio server for Antigravity & fanned-out subagents.
//!
//! Register in Antigravity MCP config:
//! ```json
//! {
//!   "mcpServers": {
//!     "sexos-tether": {
//!       "command": "tether-bridge",
//!       "args": ["mcp"]
//!     }
//!   }
//! }
//! ```

use std::io::{self, BufRead, Write};
use tether_proto::rpc::{json_escape, json_get_str, json_get_u64};

use crate::bridge::control_call;

const TOOLS_LIST_JSON: &str = r#"[
  {"name":"sexos_status","description":"Check SexOS Tether connection status, capability tier, kill-switch state, and active hardware lease.","inputSchema":{"type":"object","properties":{}}},
  {"name":"sexos_sysinfo","description":"Return structured SexOS hardware, memory, disk, Wi-Fi, and power telemetry.","inputSchema":{"type":"object","properties":{}}},
  {"name":"sexos_servers","description":"List running SexOS ring-3 servers and their protection domain (PD) IDs.","inputSchema":{"type":"object","properties":{}}},
  {"name":"sexos_logs","description":"Tail recent SexOS kernel/serial and Tether logs.","inputSchema":{"type":"object","properties":{"lines":{"type":"integer","description":"Number of recent lines (default 80)"}}}},
  {"name":"sexos_exec","description":"Execute a command on SexOS (requires Dev mode + exclusive lease).","inputSchema":{"type":"object","properties":{"command":{"type":"string"},"timeout_ms":{"type":"integer"}},"required":["command"]}},
  {"name":"sexos_fs_list","description":"List directory entries on SexOS.","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}},
  {"name":"sexos_fs_read","description":"Read a file from SexOS.","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}},
  {"name":"sexos_fs_write","description":"Write a UTF-8 or raw binary file to SexOS via chunked streaming (requires Dev mode + exclusive lease). Provide one of: local_path (host file path to stream raw bytes), base64 (base64-encoded binary), or content (UTF-8 text).","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"Destination path on SexOS (e.g. /bin/splicy)"},"content":{"type":"string","description":"UTF-8 text content"},"base64":{"type":"string","description":"Base64-encoded raw binary content"},"local_path":{"type":"string","description":"Host file path to stream as raw binary bytes"}},"required":["path"]}},
  {"name":"sexos_hotswap","description":"Live hot-swap a running SexOS ring-3 server ELF (requires Dev mode + exclusive lease). elf_path may be a host file path (streamed directly) or an on-device SexOS path.","inputSchema":{"type":"object","properties":{"server":{"type":"string","description":"Server name (e.g. sexdisplay, sexnet, sexfiles, sexbell, denim) or PD ID"},"elf_path":{"type":"string","description":"Host path or SexOS path to the replacement ELF binary"},"local_path":{"type":"string","description":"Optional alias for host ELF file path"}},"required":["server","elf_path"]}},
  {"name":"sexos_screenshot","description":"Capture a live screenshot from SexOS display frame bank.","inputSchema":{"type":"object","properties":{}}},
  {"name":"sexos_a11y_tree","description":"Fetch the structured Weft/AccessKit UI tree from SexOS.","inputSchema":{"type":"object","properties":{}}},
  {"name":"sexos_silk","description":"Run a silk_ctl window/workspace management command on SexOS (requires Full Control).","inputSchema":{"type":"object","properties":{"action":{"type":"string"}},"required":["action"]}},
  {"name":"sexos_input","description":"Inject gated synthetic pointer/keyboard input into SexOS (requires Full Control + visible indicator).","inputSchema":{"type":"object","properties":{"kind":{"type":"string"},"x":{"type":"integer"},"y":{"type":"integer"},"text":{"type":"string"}},"required":["kind"]}},
  {"name":"sexos_snapshot_create","description":"Create a named SexOS snapshot checkpoint before risky edits or deploys (requires Dev mode).","inputSchema":{"type":"object","properties":{"label":{"type":"string"}},"required":["label"]}},
  {"name":"sexos_snapshot_restore","description":"Restore a previously created SexOS snapshot checkpoint (requires Dev mode).","inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}},
  {"name":"sexos_ota_push","description":"Signed A/B system update of SexOS (requires Full Control). dir = a release folder on this host holding sexos-update.txt, sexos-update.sig and sexos-boot.img (tools/sexos-release sign); they are streamed to SexOS, then Files checks the release signature and version and writes the system slot that is not running, tried once at the next start with automatic fallback. apply=false only checks whether the index is good and newer. Without dir, uses files already on SexOS (index/sig/image paths).","inputSchema":{"type":"object","properties":{"dir":{"type":"string","description":"Host folder with the signed release"},"apply":{"type":"boolean","description":"false: only check (default true)"},"index":{"type":"string"},"sig":{"type":"string"},"image":{"type":"string"}}}},
  {"name":"sexos_wave_review","description":"Push a Wave Review summary and unified diff to the SexOS Tether GUI for human approval.","inputSchema":{"type":"object","properties":{"wave":{"type":"string"},"summary":{"type":"string"},"diff":{"type":"string"}},"required":["wave","summary"]}}
]"#;

fn map_tool_to_rpc(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "sexos_status" => Some("status"),
        "sexos_sysinfo" => Some("sysinfo"),
        "sexos_servers" => Some("servers"),
        "sexos_logs" => Some("logs"),
        "sexos_exec" => Some("exec"),
        "sexos_fs_list" => Some("fs_list"),
        "sexos_fs_read" => Some("fs_read"),
        "sexos_fs_write" => Some("fs_write"),
        "sexos_hotswap" => Some("hotswap"),
        "sexos_screenshot" => Some("screenshot"),
        "sexos_a11y_tree" => Some("a11y_tree"),
        "sexos_silk" => Some("silk_ctl"),
        "sexos_input" => Some("input_inject"),
        "sexos_snapshot_create" => Some("snapshot_create"),
        "sexos_snapshot_restore" => Some("snapshot_restore"),
        "sexos_wave_review" => Some("wave_review"),
        "sexos_ota_push" => Some("ota_push"),
        _ => None,
    }
}

fn extract_arguments_object(json: &str) -> &str {
    let Some(idx) = json.find("\"arguments\"") else {
        return "{}";
    };
    let after = &json[idx + "\"arguments\"".len()..];
    let Some(colon) = after.find(':') else {
        return "{}";
    };
    let val = after[colon + 1..].trim_start();
    if !val.starts_with('{') {
        return "{}";
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
                    return &val[..=i];
                }
            }
            _ => {}
        }
    }
    "{}"
}

pub fn run_mcp_stdio(control_addr: &str) -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let method = json_get_str(trimmed, "method").unwrap_or_default();
        let id = json_get_u64(trimmed, "id");

        let response = match method.as_str() {
            "initialize" => Some(format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"sexos-tether\",\"version\":\"0.1.0\"}}}}}}",
                id.unwrap_or(0)
            )),
            "notifications/initialized" => None,
            "tools/list" => Some(format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"tools\":{}}}}}",
                id.unwrap_or(0),
                // One JSON message per line: MCP stdio clients (Claude Code) drop a
                // reply that spans lines, and then see no tools at all.
                TOOLS_LIST_JSON.replace('\n', "")
            )),
            "tools/call" => {
                let tool = json_get_str(trimmed, "name").unwrap_or_default();
                let args = extract_arguments_object(trimmed);
                let text = match map_tool_to_rpc(&tool) {
                    Some(rpc_method) => {
                        let req = format!(
                            "{{\"method\":\"{}\",\"caller\":\"mcp\",\"params\":{}}}",
                            rpc_method, args
                        );
                        match control_call(control_addr, &req) {
                            Ok(reply) => reply,
                            Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", json_escape(&e)),
                        }
                    }
                    None => format!(
                        "{{\"ok\":false,\"error\":\"unknown MCP tool '{}'\"}}",
                        json_escape(&tool)
                    ),
                };
                Some(format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}",
                    id.unwrap_or(0),
                    json_escape(&text)
                ))
            }
            _ => id.map(|req_id| {
                format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":-32601,\"message\":\"Method not found\"}}}}",
                    req_id
                )
            }),
        };

        if let Some(resp) = response {
            writeln!(stdout, "{}", resp).map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
