// SPDX-License-Identifier: GPL-3.0-only
//! Tailor Core: pure-Rust `#![no_std]` + `alloc` engine for SexOS and desktop multi-provider
//! AI coding agents (Antigravity-compatible).
//!
//! Modules:
//! - [`json`]: minimal recursive JSON parser and serializer with full UTF-16 escape support.
//! - [`sse`]: incremental HTTP/1.1 + chunked transfer-encoding + Server-Sent Events decoder.
//! - [`diff`]: pure `no_std + alloc` line diff engine and unified diff formatter.
//! - [`skills`]: `SKILL.md` YAML frontmatter parser and system-prompt injector.
//! - [`traits`]: pluggable HTTP, filesystem, and command-runner I/O traits.
//! - [`provider`]: Gemini, Antigravity (Cloud Code), Claude, DeepSeek, and Codex request builders and SSE stream parsers.
//! - [`agent`]: multi-turn tool-calling agent state machine with workflow tracking, diff previews, and approval policies.
#![no_std]

extern crate alloc;

pub mod agent;
pub mod diff;
pub mod json;
pub mod provider;
pub mod skills;
pub mod snapshot;
pub mod sse;
pub mod traits;

pub use agent::{
    builtin_tools, Agent, AgentMode, AgentStep, ApprovalDecision, ApprovalPolicy, Message,
    NotifyUser, PendingApproval, Role, StepRecord, StepStatus, SubagentRequest, TaskBoundary,
    ToolCallRequest, ToolKind, ToolResult, ToolRunner, ToolSpec, Workspace, DEFAULT_SYSTEM_PROMPT,
};
pub use diff::{compute_diff, format_unified_diff, DiffHunk, DiffLine, DiffLineKind};
pub use json::{JsonError, JsonValue};
pub use provider::{
    build_antigravity_request, build_claude_refresh_request, build_codex_refresh_request,
    build_codex_responses_request, build_google_refresh_request, build_http_request,
    is_claude_oauth_token, parse_claude_oauth_json, parse_codex_oauth_json,
    parse_google_oauth_json, parse_keys_json, OAuthRefreshRequest, OAuthToken, Provider,
    ProviderRequest, ProviderStreamParser,
};
pub use skills::{format_skills_prompt, parse_skill_md, SkillDoc};
pub use snapshot::{Snapshot, SnapshotEntry, SnapshotStore};
pub use sse::{HttpSseDecoder, SseError, SseMessage, StreamEvent};
pub use traits::{
    AccessScope, CommandOutput, CommandRunner, GrepMatch, HttpStream, HttpTransport, WorkspaceFs,
};
