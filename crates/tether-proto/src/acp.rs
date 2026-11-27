// SPDX-License-Identifier: GPL-3.0-only
//! Minimal no_std Agent Client Protocol (ACP) JSON-RPC 2.0 builders & parsers.
//!
//! Used by SexOS `apps/tether` to talk to `agy acp` (via `tether-bridge`) over `STREAM_ACP`.

use alloc::format;
use alloc::string::String;
use crate::rpc::{json_escape, json_get_str, json_get_u64};

pub fn acp_initialize(id: u64, client_name: &str, version: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"initialize\",\"params\":{{\"protocolVersion\":1,\"clientInfo\":{{\"name\":\"{}\",\"version\":\"{}\"}},\"clientCapabilities\":{{\"fs\":{{\"readTextFile\":true,\"writeTextFile\":true}},\"terminal\":true}}}}}}",
        id,
        json_escape(client_name),
        json_escape(version)
    )
}

pub fn acp_session_new(id: u64, cwd: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"session/new\",\"params\":{{\"cwd\":\"{}\",\"mcpServers\":[]}}}}",
        id,
        json_escape(cwd)
    )
}

pub fn acp_session_prompt(id: u64, session_id: &str, text: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"session/prompt\",\"params\":{{\"sessionId\":\"{}\",\"prompt\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
        id,
        json_escape(session_id),
        json_escape(text)
    )
}

pub fn acp_session_cancel(session_id: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{{\"sessionId\":\"{}\"}}}}",
        json_escape(session_id)
    )
}

pub fn acp_permission_reply(id: u64, option_id: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"outcome\":{{\"outcome\":\"selected\",\"optionId\":\"{}\"}}}}}}",
        id,
        json_escape(option_id)
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpIncoming {
    Initialized {
        id: u64,
        agent_name: String,
    },
    SessionCreated {
        id: u64,
        session_id: String,
    },
    AgentMessageChunk {
        text: String,
    },
    AgentThoughtChunk {
        text: String,
    },
    ToolCallUpdate {
        tool_call_id: String,
        title: String,
        status: String,
    },
    PermissionRequest {
        id: u64,
        title: String,
        tool_call_id: String,
    },
    PromptDone {
        id: u64,
        stop_reason: String,
    },
    Error {
        id: Option<u64>,
        message: String,
    },
    Raw(String),
}

pub fn parse_acp_incoming(json: &str) -> AcpIncoming {
    let method = json_get_str(json, "method");
    let id = json_get_u64(json, "id");

    if let Some(err_msg) = json_get_str(json, "message") {
        if json.contains("\"error\"") && method.is_none() {
            return AcpIncoming::Error { id, message: err_msg };
        }
    }

    match method.as_deref() {
        Some("session/update") => {
            let kind = json_get_str(json, "sessionUpdate").unwrap_or_default();
            match kind.as_str() {
                "agent_message_chunk" => {
                    let text = json_get_str(json, "text").unwrap_or_default();
                    AcpIncoming::AgentMessageChunk { text }
                }
                "agent_thought_chunk" => {
                    let text = json_get_str(json, "text").unwrap_or_default();
                    AcpIncoming::AgentThoughtChunk { text }
                }
                "tool_call" | "tool_call_update" => {
                    let tool_call_id = json_get_str(json, "toolCallId").unwrap_or_default();
                    let title = json_get_str(json, "title").unwrap_or_else(|| String::from("tool"));
                    let status = json_get_str(json, "status").unwrap_or_else(|| String::from("in_progress"));
                    AcpIncoming::ToolCallUpdate {
                        tool_call_id,
                        title,
                        status,
                    }
                }
                _ => AcpIncoming::Raw(String::from(json)),
            }
        }
        Some("session/request_permission") => {
            let title = json_get_str(json, "title").unwrap_or_else(|| String::from("Approve tool action?"));
            let tool_call_id = json_get_str(json, "toolCallId").unwrap_or_default();
            AcpIncoming::PermissionRequest {
                id: id.unwrap_or(0),
                title,
                tool_call_id,
            }
        }
        None => {
            if let Some(req_id) = id {
                if let Some(session_id) = json_get_str(json, "sessionId") {
                    return AcpIncoming::SessionCreated {
                        id: req_id,
                        session_id,
                    };
                }
                if let Some(stop_reason) = json_get_str(json, "stopReason") {
                    return AcpIncoming::PromptDone {
                        id: req_id,
                        stop_reason,
                    };
                }
                if json.contains("\"protocolVersion\"") {
                    let agent_name = json_get_str(json, "name").unwrap_or_else(|| String::from("Antigravity"));
                    return AcpIncoming::Initialized {
                        id: req_id,
                        agent_name,
                    };
                }
            }
            AcpIncoming::Raw(String::from(json))
        }
        _ => AcpIncoming::Raw(String::from(json)),
    }
}
