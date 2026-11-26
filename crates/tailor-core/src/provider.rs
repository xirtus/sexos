// SPDX-License-Identifier: GPL-3.0-only
//! Multi-provider AI adapters for Gemini, Claude, and Codex (OpenAI).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::agent::{Message, Role, ToolSpec};
use crate::json::{self, JsonValue};
use crate::sse::{HttpSseDecoder, SseError, SseMessage, StreamEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    Claude,
    ClaudeDeepSeek,
    Codex,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::ClaudeDeepSeek => "claude-deepseek",
            Self::Codex => "codex",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::Claude => "Claude",
            Self::ClaudeDeepSeek => "Claude-DeepSeek",
            Self::Codex => "Codex",
        }
    }

    pub fn host(self) -> &'static str {
        match self {
            Self::Gemini => "generativelanguage.googleapis.com",
            Self::Claude => "api.anthropic.com",
            Self::ClaudeDeepSeek => "api.deepseek.com",
            Self::Codex => "api.openai.com",
        }
    }

    pub fn port(self) -> u16 {
        443
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Gemini => "gemini-2.5-flash",
            Self::Claude => "claude-sonnet-4-20250514",
            Self::ClaudeDeepSeek => "deepseek-chat",
            Self::Codex => "gpt-4.1",
        }
    }

    pub fn key_filename(self) -> &'static str {
        match self {
            Self::Gemini => "gemini.key",
            Self::Claude => "claude.key",
            Self::ClaudeDeepSeek => "claude-deepseek.key",
            Self::Codex => "codex.key",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let lower = s.trim();
        if lower.eq_ignore_ascii_case("gemini") || lower.eq_ignore_ascii_case("google") {
            Some(Self::Gemini)
        } else if lower.eq_ignore_ascii_case("claude-deepseek")
            || lower.eq_ignore_ascii_case("claude_deepseek")
            || lower.eq_ignore_ascii_case("deepseek")
        {
            Some(Self::ClaudeDeepSeek)
        } else if lower.eq_ignore_ascii_case("claude") || lower.eq_ignore_ascii_case("anthropic") {
            Some(Self::Claude)
        } else if lower.eq_ignore_ascii_case("codex") || lower.eq_ignore_ascii_case("openai") {
            Some(Self::Codex)
        } else {
            None
        }
    }
}

/// Parses a JSON API key bundle into `(Provider, api_key)` pairs.
///
/// Accepts either flat string values (`{"gemini": "...", "claude-deepseek": "..."}`)
/// or nested objects (`{"claude": {"api_key": "..."}}` / `"key"` / `"token"`).
pub fn parse_keys_json(raw: &str) -> Vec<(Provider, String)> {
    let mut out = Vec::new();
    let Ok(JsonValue::Object(map)) = json::parse(raw) else {
        return out;
    };
    for (k, v) in map {
        let Some(provider) = Provider::from_name(&k) else {
            continue;
        };
        let secret = v
            .as_str()
            .or_else(|| v.get("api_key").and_then(|val| val.as_str()))
            .or_else(|| v.get("key").and_then(|val| val.as_str()))
            .or_else(|| v.get("token").and_then(|val| val.as_str()));
        if let Some(s) = secret {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push((provider, String::from(trimmed)));
            }
        }
    }
    out
}

/// Complete HTTP/1.1 request ready to send over a TLS `sex_app::net::Duplex` connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub http_bytes: Vec<u8>,
}

/// Builds a complete HTTP/1.1 POST request for the given provider, model, API key,
/// conversation history, and tool schemas.
pub fn build_http_request(
    provider: Provider,
    model: &str,
    api_key: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> ProviderRequest {
    let (path, body_json, extra_headers) = match provider {
        Provider::Gemini => {
            let path = format!("/v1beta/models/{}:streamGenerateContent?alt=sse", model);
            let body = build_gemini_body(system_prompt, messages, tools);
            let headers = format!("x-goog-api-key: {}\r\n", api_key);
            (path, body, headers)
        }
        Provider::Claude | Provider::ClaudeDeepSeek => {
            let path = if provider == Provider::ClaudeDeepSeek {
                String::from("/anthropic/v1/messages")
            } else {
                String::from("/v1/messages")
            };
            let body = build_claude_body(model, system_prompt, messages, tools);
            let headers = format!(
                "x-api-key: {}\r\nanthropic-version: 2023-06-01\r\n",
                api_key
            );
            (path, body, headers)
        }
        Provider::Codex => {
            let path = String::from("/v1/chat/completions");
            let body = build_codex_body(model, system_prompt, messages, tools);
            let headers = format!("Authorization: Bearer {}\r\n", api_key);
            (path, body, headers)
        }
    };

    let body_str = json::encode(&body_json);
    let mut http = String::with_capacity(body_str.len() + 256);
    let _ = write!(
        &mut http,
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: SexOS-Tailor/0.1\r\n\
         Content-Type: application/json\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         {}\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        path,
        provider.host(),
        extra_headers,
        body_str.len(),
        body_str
    );

    ProviderRequest {
        host: String::from(provider.host()),
        port: provider.port(),
        path,
        http_bytes: http.into_bytes(),
    }
}

fn build_gemini_body(system_prompt: &str, messages: &[Message], tools: &[ToolSpec]) -> JsonValue {
    let mut root = Vec::new();
    if !system_prompt.is_empty() {
        root.push((
            String::from("systemInstruction"),
            JsonValue::object(vec![(
                String::from("parts"),
                JsonValue::array(vec![JsonValue::object(vec![(
                    String::from("text"),
                    JsonValue::str(system_prompt),
                )])]),
            )]),
        ));
    }

    let mut contents = Vec::new();
    for msg in messages {
        match msg.role {
            Role::User => {
                contents.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (
                        String::from("parts"),
                        JsonValue::array(vec![JsonValue::object(vec![(
                            String::from("text"),
                            JsonValue::str(msg.text.as_str()),
                        )])]),
                    ),
                ]));
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if !msg.text.is_empty() {
                    parts.push(JsonValue::object(vec![(
                        String::from("text"),
                        JsonValue::str(msg.text.as_str()),
                    )]));
                }
                for tc in &msg.tool_calls {
                    parts.push(JsonValue::object(vec![(
                        String::from("functionCall"),
                        JsonValue::object(vec![
                            (String::from("name"), JsonValue::str(tc.name.as_str())),
                            (String::from("args"), tc.arguments.clone()),
                        ]),
                    )]));
                }
                if !parts.is_empty() {
                    contents.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("model")),
                        (String::from("parts"), JsonValue::array(parts)),
                    ]));
                }
            }
            Role::Tool => {
                if let Some(ref res) = msg.tool_result {
                    contents.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("user")),
                        (
                            String::from("parts"),
                            JsonValue::array(vec![JsonValue::object(vec![(
                                String::from("functionResponse"),
                                JsonValue::object(vec![
                                    (String::from("name"), JsonValue::str(res.name.as_str())),
                                    (
                                        String::from("response"),
                                        JsonValue::object(vec![
                                            (
                                                String::from("output"),
                                                JsonValue::str(res.output.as_str()),
                                            ),
                                            (
                                                String::from("is_error"),
                                                JsonValue::Bool(res.is_error),
                                            ),
                                        ]),
                                    ),
                                ]),
                            )])]),
                        ),
                    ]));
                }
            }
        }
    }
    root.push((String::from("contents"), JsonValue::array(contents)));

    if !tools.is_empty() {
        let decls = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("name"), JsonValue::str(t.name.as_str())),
                    (
                        String::from("description"),
                        JsonValue::str(t.description.as_str()),
                    ),
                    (String::from("parameters"), t.parameters_schema.clone()),
                ])
            })
            .collect();
        root.push((
            String::from("tools"),
            JsonValue::array(vec![JsonValue::object(vec![(
                String::from("functionDeclarations"),
                JsonValue::array(decls),
            )])]),
        ));
    }

    JsonValue::object(root)
}

fn build_claude_body(
    model: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> JsonValue {
    let mut root = vec![
        (String::from("model"), JsonValue::str(model)),
        (String::from("max_tokens"), JsonValue::uint(4096)),
        (String::from("stream"), JsonValue::Bool(true)),
    ];
    if !system_prompt.is_empty() {
        root.push((String::from("system"), JsonValue::str(system_prompt)));
    }

    let mut out_msgs = Vec::new();
    for msg in messages {
        match msg.role {
            Role::User => {
                out_msgs.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (String::from("content"), JsonValue::str(msg.text.as_str())),
                ]));
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !msg.text.is_empty() {
                    blocks.push(JsonValue::object(vec![
                        (String::from("type"), JsonValue::str("text")),
                        (String::from("text"), JsonValue::str(msg.text.as_str())),
                    ]));
                }
                for tc in &msg.tool_calls {
                    blocks.push(JsonValue::object(vec![
                        (String::from("type"), JsonValue::str("tool_use")),
                        (String::from("id"), JsonValue::str(tc.id.as_str())),
                        (String::from("name"), JsonValue::str(tc.name.as_str())),
                        (String::from("input"), tc.arguments.clone()),
                    ]));
                }
                if !blocks.is_empty() {
                    out_msgs.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("assistant")),
                        (String::from("content"), JsonValue::array(blocks)),
                    ]));
                }
            }
            Role::Tool => {
                if let Some(ref res) = msg.tool_result {
                    out_msgs.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("user")),
                        (
                            String::from("content"),
                            JsonValue::array(vec![JsonValue::object(vec![
                                (String::from("type"), JsonValue::str("tool_result")),
                                (
                                    String::from("tool_use_id"),
                                    JsonValue::str(res.call_id.as_str()),
                                ),
                                (
                                    String::from("content"),
                                    JsonValue::str(res.output.as_str()),
                                ),
                                (String::from("is_error"), JsonValue::Bool(res.is_error)),
                            ])]),
                        ),
                    ]));
                }
            }
        }
    }
    root.push((String::from("messages"), JsonValue::array(out_msgs)));

    if !tools.is_empty() {
        let tool_defs = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("name"), JsonValue::str(t.name.as_str())),
                    (
                        String::from("description"),
                        JsonValue::str(t.description.as_str()),
                    ),
                    (String::from("input_schema"), t.parameters_schema.clone()),
                ])
            })
            .collect();
        root.push((String::from("tools"), JsonValue::array(tool_defs)));
    }

    JsonValue::object(root)
}

fn build_codex_body(
    model: &str,
    system_prompt: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> JsonValue {
    let mut root = vec![
        (String::from("model"), JsonValue::str(model)),
        (String::from("stream"), JsonValue::Bool(true)),
    ];

    let mut out_msgs = Vec::new();
    if !system_prompt.is_empty() {
        out_msgs.push(JsonValue::object(vec![
            (String::from("role"), JsonValue::str("system")),
            (String::from("content"), JsonValue::str(system_prompt)),
        ]));
    }
    for msg in messages {
        match msg.role {
            Role::User => {
                out_msgs.push(JsonValue::object(vec![
                    (String::from("role"), JsonValue::str("user")),
                    (String::from("content"), JsonValue::str(msg.text.as_str())),
                ]));
            }
            Role::Assistant => {
                let mut entries = vec![(String::from("role"), JsonValue::str("assistant"))];
                if msg.text.is_empty() {
                    entries.push((String::from("content"), JsonValue::Null));
                } else {
                    entries.push((String::from("content"), JsonValue::str(msg.text.as_str())));
                }
                if !msg.tool_calls.is_empty() {
                    let calls = msg
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            JsonValue::object(vec![
                                (String::from("id"), JsonValue::str(tc.id.as_str())),
                                (String::from("type"), JsonValue::str("function")),
                                (
                                    String::from("function"),
                                    JsonValue::object(vec![
                                        (String::from("name"), JsonValue::str(tc.name.as_str())),
                                        (
                                            String::from("arguments"),
                                            JsonValue::str(json::encode(&tc.arguments)),
                                        ),
                                    ]),
                                ),
                            ])
                        })
                        .collect();
                    entries.push((String::from("tool_calls"), JsonValue::array(calls)));
                }
                out_msgs.push(JsonValue::object(entries));
            }
            Role::Tool => {
                if let Some(ref res) = msg.tool_result {
                    out_msgs.push(JsonValue::object(vec![
                        (String::from("role"), JsonValue::str("tool")),
                        (
                            String::from("tool_call_id"),
                            JsonValue::str(res.call_id.as_str()),
                        ),
                        (
                            String::from("content"),
                            JsonValue::str(res.output.as_str()),
                        ),
                    ]));
                }
            }
        }
    }
    root.push((String::from("messages"), JsonValue::array(out_msgs)));

    if !tools.is_empty() {
        let tool_defs = tools
            .iter()
            .map(|t| {
                JsonValue::object(vec![
                    (String::from("type"), JsonValue::str("function")),
                    (
                        String::from("function"),
                        JsonValue::object(vec![
                            (String::from("name"), JsonValue::str(t.name.as_str())),
                            (
                                String::from("description"),
                                JsonValue::str(t.description.as_str()),
                            ),
                            (String::from("parameters"), t.parameters_schema.clone()),
                        ]),
                    ),
                ])
            })
            .collect();
        root.push((String::from("tools"), JsonValue::array(tool_defs)));
    }

    JsonValue::object(root)
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    args_json: String,
}

/// Stateful incremental HTTP + SSE + provider JSON parser.
pub struct ProviderStreamParser {
    provider: Provider,
    decoder: HttpSseDecoder,
    next_call_seq: u32,
    stop_reason: Option<String>,
    emitted_done: bool,
    partial_tools: BTreeMap<usize, PartialToolCall>,
}

impl ProviderStreamParser {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            decoder: HttpSseDecoder::new(),
            next_call_seq: 1,
            stop_reason: None,
            emitted_done: false,
            partial_tools: BTreeMap::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.emitted_done || self.decoder.is_complete()
    }

    /// Feeds raw HTTP response bytes and returns decoded provider-agnostic [`StreamEvent`]s.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        match self.decoder.feed(bytes) {
            Ok(messages) => {
                for msg in messages {
                    self.handle_sse(&msg, &mut events);
                }
            }
            Err(err) => {
                events.push(StreamEvent::Error(format_sse_error(err)));
                self.emitted_done = true;
            }
        }
        events
    }

    /// Signals EOF from the underlying transport, flushing any buffered tool calls or Done event.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        match self.decoder.finish() {
            Ok(messages) => {
                for msg in messages {
                    self.handle_sse(&msg, &mut events);
                }
            }
            Err(err) => {
                events.push(StreamEvent::Error(format_sse_error(err)));
                self.emitted_done = true;
                return events;
            }
        }
        self.flush_partial_tools(&mut events);
        if !self.emitted_done {
            self.emitted_done = true;
            events.push(StreamEvent::Done {
                stop_reason: self.stop_reason.take(),
            });
        }
        events
    }

    fn handle_sse(&mut self, msg: &SseMessage, out: &mut Vec<StreamEvent>) {
        let data = msg.data.trim();
        if data.is_empty() {
            return;
        }
        if data == "[DONE]" {
            self.flush_partial_tools(out);
            if !self.emitted_done {
                self.emitted_done = true;
                out.push(StreamEvent::Done {
                    stop_reason: self.stop_reason.take(),
                });
            }
            return;
        }
        let val = match json::parse(data) {
            Ok(v) => v,
            Err(_) => return,
        };
        match self.provider {
            Provider::Gemini => self.parse_gemini_chunk(&val, out),
            Provider::Claude | Provider::ClaudeDeepSeek => {
                self.parse_claude_chunk(msg.event.as_deref(), &val, out)
            }
            Provider::Codex => self.parse_codex_chunk(&val, out),
        }
    }

    fn parse_gemini_chunk(&mut self, val: &JsonValue, out: &mut Vec<StreamEvent>) {
        if let Some(err_msg) = val.get_path(&["error", "message"]).and_then(JsonValue::as_str) {
            out.push(StreamEvent::Error(String::from(err_msg)));
            return;
        }
        let Some(cand) = val.get("candidates").and_then(|c| c.index(0)) else {
            return;
        };
        if let Some(parts) = cand
            .get_path(&["content", "parts"])
            .and_then(JsonValue::as_array)
        {
            for part in parts {
                if let Some(text) = part.get("text").and_then(JsonValue::as_str) {
                    if part.get("thought").and_then(JsonValue::as_bool) == Some(true) {
                        out.push(StreamEvent::ThinkingDelta(String::from(text)));
                    } else if !text.is_empty() {
                        out.push(StreamEvent::TextDelta(String::from(text)));
                    }
                }
                if let Some(fc) = part.get("functionCall") {
                    if let Some(name) = fc.get("name").and_then(JsonValue::as_str) {
                        let id = format!("gemini-call-{}", self.next_call_seq);
                        self.next_call_seq += 1;
                        let args = fc
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| JsonValue::object(Vec::new()));
                        out.push(StreamEvent::ToolCall {
                            id,
                            name: String::from(name),
                            arguments: args,
                        });
                    }
                }
            }
        }
        if let Some(reason) = cand.get("finishReason").and_then(JsonValue::as_str) {
            if !reason.is_empty() {
                self.stop_reason = Some(String::from(reason));
                if !self.emitted_done {
                    self.emitted_done = true;
                    out.push(StreamEvent::Done {
                        stop_reason: self.stop_reason.take(),
                    });
                }
            }
        }
    }

    fn parse_claude_chunk(
        &mut self,
        event_name: Option<&str>,
        val: &JsonValue,
        out: &mut Vec<StreamEvent>,
    ) {
        let kind = val
            .get("type")
            .and_then(JsonValue::as_str)
            .or(event_name)
            .unwrap_or("");
        match kind {
            "content_block_start" => {
                let idx = val
                    .get("index")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(block) = val.get("content_block") {
                    if block.get("type").and_then(JsonValue::as_str) == Some("tool_use") {
                        let id = block
                            .get("id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("claude-call");
                        let name = block
                            .get("name")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("");
                        self.partial_tools.insert(
                            idx,
                            PartialToolCall {
                                id: String::from(id),
                                name: String::from(name),
                                args_json: String::new(),
                            },
                        );
                    }
                }
            }
            "content_block_delta" => {
                let idx = val
                    .get("index")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
                let Some(delta) = val.get("delta") else {
                    return;
                };
                match delta.get("type").and_then(JsonValue::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(t) = delta.get("text").and_then(JsonValue::as_str) {
                            if !t.is_empty() {
                                out.push(StreamEvent::TextDelta(String::from(t)));
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(t) = delta.get("thinking").and_then(JsonValue::as_str) {
                            if !t.is_empty() {
                                out.push(StreamEvent::ThinkingDelta(String::from(t)));
                            }
                        }
                    }
                    "input_json_delta" => {
                        if let Some(part) = delta.get("partial_json").and_then(JsonValue::as_str) {
                            if let Some(entry) = self.partial_tools.get_mut(&idx) {
                                entry.args_json.push_str(part);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let idx = val
                    .get("index")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(tool) = self.partial_tools.remove(&idx) {
                    emit_partial_tool(tool, out);
                }
            }
            "message_delta" => {
                if let Some(reason) = val
                    .get_path(&["delta", "stop_reason"])
                    .and_then(JsonValue::as_str)
                {
                    self.stop_reason = Some(String::from(reason));
                }
            }
            "message_stop" => {
                self.flush_partial_tools(out);
                if !self.emitted_done {
                    self.emitted_done = true;
                    out.push(StreamEvent::Done {
                        stop_reason: self.stop_reason.take(),
                    });
                }
            }
            "error" => {
                let msg = val
                    .get_path(&["error", "message"])
                    .and_then(JsonValue::as_str)
                    .unwrap_or("Claude API error");
                out.push(StreamEvent::Error(String::from(msg)));
            }
            _ => {}
        }
    }

    fn parse_codex_chunk(&mut self, val: &JsonValue, out: &mut Vec<StreamEvent>) {
        if let Some(err_msg) = val.get_path(&["error", "message"]).and_then(JsonValue::as_str) {
            out.push(StreamEvent::Error(String::from(err_msg)));
            return;
        }
        let Some(choice) = val.get("choices").and_then(|c| c.index(0)) else {
            return;
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(JsonValue::as_str) {
                if !text.is_empty() {
                    out.push(StreamEvent::TextDelta(String::from(text)));
                }
            }
            if let Some(thought) = delta.get("reasoning_content").and_then(JsonValue::as_str) {
                if !thought.is_empty() {
                    out.push(StreamEvent::ThinkingDelta(String::from(thought)));
                }
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(JsonValue::as_array) {
                for tc in tcs {
                    let idx = tc
                        .get("index")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(0) as usize;
                    let entry = self.partial_tools.entry(idx).or_default();
                    if let Some(id) = tc.get("id").and_then(JsonValue::as_str) {
                        if !id.is_empty() {
                            entry.id = String::from(id);
                        }
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(JsonValue::as_str) {
                            if !name.is_empty() {
                                entry.name.push_str(name);
                            }
                        }
                        if let Some(args) = func.get("arguments").and_then(JsonValue::as_str) {
                            entry.args_json.push_str(args);
                        }
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(JsonValue::as_str) {
            if !reason.is_empty() {
                self.stop_reason = Some(String::from(reason));
                self.flush_partial_tools(out);
            }
        }
    }

    fn flush_partial_tools(&mut self, out: &mut Vec<StreamEvent>) {
        let pending = core::mem::take(&mut self.partial_tools);
        for (_, tool) in pending {
            emit_partial_tool(tool, out);
        }
    }
}

fn emit_partial_tool(tool: PartialToolCall, out: &mut Vec<StreamEvent>) {
    if tool.name.is_empty() {
        return;
    }
    let arguments = if tool.args_json.trim().is_empty() {
        JsonValue::object(Vec::new())
    } else {
        json::parse(&tool.args_json).unwrap_or_else(|_| JsonValue::object(Vec::new()))
    };
    out.push(StreamEvent::ToolCall {
        id: if tool.id.is_empty() {
            String::from("call")
        } else {
            tool.id
        },
        name: tool.name,
        arguments,
    });
}

fn format_sse_error(err: SseError) -> String {
    match err {
        SseError::MalformedHeaders => String::from("Malformed HTTP response headers"),
        SseError::MalformedChunkSize => String::from("Malformed HTTP chunk size"),
        SseError::HeaderTooLarge => String::from("HTTP response headers too large"),
        SseError::HttpStatus { status, body } => {
            if body.is_empty() {
                format!("HTTP {}", status)
            } else {
                format!("HTTP {}: {}", status, body.trim())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::builtin_tools;

    fn chunked_response(payload: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            payload.len(),
            payload
        )
        .into_bytes()
    }

    #[test]
    fn builds_provider_http_requests_with_auth_and_tools() {
        let msgs = vec![Message::user("List files")];
        let tools = builtin_tools();

        let gem = build_http_request(
            Provider::Gemini,
            "gemini-2.5-flash",
            "GKEY",
            "You are Tailor.",
            &msgs,
            &tools,
        );
        let gem_txt = core::str::from_utf8(&gem.http_bytes).unwrap();
        assert!(gem_txt.starts_with("POST /v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse HTTP/1.1\r\n"));
        assert!(gem_txt.contains("x-goog-api-key: GKEY\r\n"));
        assert!(gem_txt.contains("\"functionDeclarations\""));

        let cla = build_http_request(
            Provider::Claude,
            "claude-sonnet-4-20250514",
            "CKEY",
            "You are Tailor.",
            &msgs,
            &tools,
        );
        let cla_txt = core::str::from_utf8(&cla.http_bytes).unwrap();
        assert!(cla_txt.starts_with("POST /v1/messages HTTP/1.1\r\n"));
        assert!(cla_txt.contains("x-api-key: CKEY\r\n"));
        assert!(cla_txt.contains("anthropic-version: 2023-06-01\r\n"));
        assert!(cla_txt.contains("\"input_schema\""));

        let cod = build_http_request(
            Provider::Codex,
            "gpt-4.1",
            "OKEY",
            "You are Tailor.",
            &msgs,
            &tools,
        );
        let cod_txt = core::str::from_utf8(&cod.http_bytes).unwrap();
        assert!(cod_txt.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        assert!(cod_txt.contains("Authorization: Bearer OKEY\r\n"));
        assert!(cod_txt.contains("\"stream\":true"));
    }

    #[test]
    fn parses_gemini_sse_thinking_text_and_tool_call() {
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"thought\":true,\"text\":\"Checking...\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Reading main.rs\"},{\"functionCall\":{\"name\":\"read_file\",\"args\":{\"path\":\"src/main.rs\"}}}]},\"finishReason\":\"STOP\"}]}\n\n"
        );
        let wire = chunked_response(sse);
        let mut p = ProviderStreamParser::new(Provider::Gemini);
        let mut events = p.feed(&wire);
        events.extend(p.finish());
        assert_eq!(events.len(), 4);
        assert_eq!(events[0], StreamEvent::ThinkingDelta(String::from("Checking...")));
        assert_eq!(events[1], StreamEvent::TextDelta(String::from("Reading main.rs")));
        match &events[2] {
            StreamEvent::ToolCall { name, arguments, .. } => {
                assert_eq!(name, "read_file");
                assert_eq!(
                    arguments.get("path").and_then(JsonValue::as_str),
                    Some("src/main.rs")
                );
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[3],
            StreamEvent::Done {
                stop_reason: Some(String::from("STOP"))
            }
        );
    }

    #[test]
    fn parses_claude_sse_streamed_tool_use() {
        let sse = concat!(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Let me patch it.\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"patch_file\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"a.rs\\\",\" }}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"old_text\\\":\\\"x\\\",\\\"new_text\\\":\\\"y\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let wire = chunked_response(sse);
        let mut p = ProviderStreamParser::new(Provider::Claude);
        let mut events = Vec::new();
        for chunk in wire.chunks(7) {
            events.extend(p.feed(chunk));
        }
        events.extend(p.finish());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], StreamEvent::TextDelta(String::from("Let me patch it.")));
        match &events[1] {
            StreamEvent::ToolCall { id, name, arguments } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "patch_file");
                assert_eq!(arguments.get("new_text").and_then(JsonValue::as_str), Some("y"));
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[2],
            StreamEvent::Done {
                stop_reason: Some(String::from("tool_use"))
            }
        );
    }

    #[test]
    fn parses_codex_sse_streamed_tool_calls() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Listing dir\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\".\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let wire = chunked_response(sse);
        let mut p = ProviderStreamParser::new(Provider::Codex);
        let mut events = p.feed(&wire);
        events.extend(p.finish());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], StreamEvent::TextDelta(String::from("Listing dir")));
        match &events[1] {
            StreamEvent::ToolCall { id, name, arguments } => {
                assert_eq!(id, "call_9");
                assert_eq!(name, "list_dir");
                assert_eq!(arguments.get("path").and_then(JsonValue::as_str), Some("."));
            }
            other => panic!("unexpected event: {:?}", other),
        }
        assert_eq!(
            events[2],
            StreamEvent::Done {
                stop_reason: Some(String::from("tool_calls"))
            }
        );
    }
}
