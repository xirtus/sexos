// SPDX-License-Identifier: GPL-3.0-only
//! Tailor Core: pure-Rust `#![no_std]` + `alloc` engine for SexOS's native multi-provider
//! AI coding agent.
//!
//! Modules:
//! - [`json`]: minimal recursive JSON parser and serializer with full UTF-16 escape support.
//! - [`sse`]: incremental HTTP/1.1 + chunked transfer-encoding + Server-Sent Events decoder.
//! - [`provider`]: Gemini, Claude, and Codex (OpenAI) request builders and SSE stream parsers.
//! - [`agent`]: multi-turn tool-calling agent state machine with human-in-the-loop approval.
#![no_std]

extern crate alloc;

pub mod agent;
pub mod json;
pub mod provider;
pub mod sse;

pub use agent::{
    Agent, AgentStep, ApprovalDecision, Message, PendingApproval, Role, ToolCallRequest, ToolKind,
    ToolResult, ToolRunner, ToolSpec, Workspace,
};
pub use json::{JsonError, JsonValue};
pub use provider::{parse_keys_json, Provider, ProviderRequest, ProviderStreamParser};
pub use sse::{HttpSseDecoder, SseError, SseMessage, StreamEvent};
