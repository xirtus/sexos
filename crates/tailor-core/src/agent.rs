// SPDX-License-Identifier: GPL-3.0-only
//! Multi-turn AI coding agent state machine with capability-scoped tools and explicit
//! human-in-the-loop approval for all mutating operations.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::json::JsonValue;
use crate::provider::{build_http_request, Provider, ProviderRequest};
use crate::sse::StreamEvent;

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are Tailor, the native AI coding agent running inside SexOS (a capability-based SASOS microkernel). \
You can inspect files with read_file and list_dir, propose edits with write_file and patch_file, \
run SexOS programs with run_command, and request server hot-swaps with hotswap_server. \
All mutating tools require explicit human approval before execution. Be concise, accurate, and safe.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    ReadFile,
    WriteFile,
    PatchFile,
    ListDir,
    RunCommand,
    HotswapServer,
}

impl ToolKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "read_file" => Some(Self::ReadFile),
            "write_file" => Some(Self::WriteFile),
            "patch_file" => Some(Self::PatchFile),
            "list_dir" => Some(Self::ListDir),
            "run_command" => Some(Self::RunCommand),
            "hotswap_server" => Some(Self::HotswapServer),
            _ => None,
        }
    }

    /// Returns `true` for any mutating or side-effecting tool that requires explicit human approval.
    pub fn requires_approval(self) -> bool {
        matches!(
            self,
            Self::WriteFile | Self::PatchFile | Self::RunCommand | Self::HotswapServer
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters_schema: JsonValue,
}

fn schema_obj(props: Vec<(&str, &str, &str)>, required: &[&str]) -> JsonValue {
    let properties = props
        .into_iter()
        .map(|(k, ty, desc)| {
            (
                String::from(k),
                JsonValue::object(vec![
                    (String::from("type"), JsonValue::str(ty)),
                    (String::from("description"), JsonValue::str(desc)),
                ]),
            )
        })
        .collect();
    let req = required.iter().map(|r| JsonValue::str(*r)).collect();
    JsonValue::object(vec![
        (String::from("type"), JsonValue::str("object")),
        (String::from("properties"), JsonValue::object(properties)),
        (String::from("required"), JsonValue::array(req)),
    ])
}

/// Returns the built-in SexOS Tailor tool definitions.
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: String::from("read_file"),
            description: String::from("Read a UTF-8 text file inside the granted workspace."),
            parameters_schema: schema_obj(
                vec![("path", "string", "Relative or granted path to read")],
                &["path"],
            ),
        },
        ToolSpec {
            name: String::from("write_file"),
            description: String::from(
                "Create or overwrite a file in the granted workspace (requires human approval).",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("path", "string", "Path to write"),
                    ("content", "string", "Full UTF-8 file content"),
                ],
                &["path", "content"],
            ),
        },
        ToolSpec {
            name: String::from("patch_file"),
            description: String::from(
                "Replace a single exact substring in a workspace file (requires human approval).",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("path", "string", "Path to patch"),
                    ("old_text", "string", "Exact existing text to replace"),
                    ("new_text", "string", "Replacement text"),
                ],
                &["path", "old_text", "new_text"],
            ),
        },
        ToolSpec {
            name: String::from("list_dir"),
            description: String::from("List files and subdirectories in the granted workspace."),
            parameters_schema: schema_obj(
                vec![("path", "string", "Directory path to list (use '.' for root)")],
                &["path"],
            ),
        },
        ToolSpec {
            name: String::from("run_command"),
            description: String::from(
                "Launch a SexOS program via sex_app::launch (requires human approval).",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("program", "string", "Program name in /bin or @/path"),
                    ("args", "string", "Command arguments string"),
                    (
                        "network",
                        "boolean",
                        "Whether to pass +net grant if Tailor holds it",
                    ),
                ],
                &["program"],
            ),
        },
        ToolSpec {
            name: String::from("hotswap_server"),
            description: String::from(
                "Request a ring-3 SexOS server hot-swap (requires human approval).",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("server", "string", "Target ring-3 server name"),
                    ("binary_path", "string", "Path to replacement server binary"),
                ],
                &["server", "binary_path"],
            ),
        },
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub text: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub tool_result: Option<ToolResult>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            tool_calls: Vec::new(),
            tool_result: None,
        }
    }

    pub fn assistant(text: impl Into<String>, tool_calls: Vec<ToolCallRequest>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            tool_calls,
            tool_result: None,
        }
    }

    pub fn tool(result: ToolResult) -> Self {
        Self {
            role: Role::Tool,
            text: String::new(),
            tool_calls: Vec::new(),
            tool_result: Some(result),
        }
    }
}

/// Capability-scoped filesystem interface implemented by `apps/tailor` on `sex_app::files`
/// and by in-memory mocks in host unit tests.
pub trait Workspace {
    fn read_file(&mut self, path: &str) -> Result<String, String>;
    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String>;
    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, String>;
}

/// Command and server-management interface implemented by `apps/tailor` on `sex_app::launch`
/// and by mocks in host unit tests.
pub trait ToolRunner {
    fn run_command(&mut self, program: &str, args: &str, network: bool) -> Result<String, String>;
    fn hotswap_server(&mut self, server: &str, binary_path: &str) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Denied(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingApproval {
    pub call: ToolCallRequest,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStep {
    /// Ready to send the next HTTP/SSE request to the active AI provider.
    NeedsModelTurn,
    /// Paused waiting for explicit human approval of a mutating tool call.
    AwaitingApproval(PendingApproval),
    /// A tool finished executing; call `step_tools` again to continue draining the queue.
    ToolExecuted(ToolResult),
    /// Assistant completed its response and no tool calls remain.
    Idle,
}

/// Tailor's multi-turn agent state machine.
pub struct Agent {
    provider: Provider,
    model: String,
    system_prompt: String,
    tools: Vec<ToolSpec>,
    history: Vec<Message>,
    streaming_text: String,
    streaming_thinking: String,
    streaming_calls: Vec<ToolCallRequest>,
    pending_calls: VecDeque<ToolCallRequest>,
    awaiting_approval: Option<PendingApproval>,
    executed_tool_in_turn: bool,
}

impl Agent {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            model: String::from(provider.default_model()),
            system_prompt: String::from(DEFAULT_SYSTEM_PROMPT),
            tools: builtin_tools(),
            history: Vec::new(),
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            streaming_calls: Vec::new(),
            pending_calls: VecDeque::new(),
            awaiting_approval: None,
            executed_tool_in_turn: false,
        }
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_provider(&mut self, provider: Provider) {
        self.provider = provider;
        self.model = String::from(provider.default_model());
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    pub fn set_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = prompt.into();
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }

    pub fn pending_approval(&self) -> Option<&PendingApproval> {
        self.awaiting_approval.as_ref()
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.streaming_text.clear();
        self.streaming_thinking.clear();
        self.streaming_calls.clear();
        self.pending_calls.clear();
        self.awaiting_approval = None;
        self.executed_tool_in_turn = false;
    }

    pub fn push_user_message(&mut self, text: impl Into<String>) {
        self.history.push(Message::user(text));
        self.executed_tool_in_turn = false;
    }

    pub fn build_request(&self, api_key: &str) -> ProviderRequest {
        build_http_request(
            self.provider,
            &self.model,
            api_key,
            &self.system_prompt,
            &self.history,
            &self.tools,
        )
    }

    /// Consumes a decoded [`StreamEvent`] from the active provider stream.
    pub fn on_stream_event(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::TextDelta(t) => self.streaming_text.push_str(t),
            StreamEvent::ThinkingDelta(t) => self.streaming_thinking.push_str(t),
            StreamEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                self.streaming_calls.push(ToolCallRequest {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                });
            }
            StreamEvent::Done { .. } | StreamEvent::Error(_) => {}
        }
    }

    /// Commits the streamed assistant message and queues any requested tool calls.
    pub fn finish_model_turn(&mut self) -> AgentStep {
        let text = core::mem::take(&mut self.streaming_text);
        self.streaming_thinking.clear();
        let calls = core::mem::take(&mut self.streaming_calls);
        if !text.is_empty() || !calls.is_empty() {
            self.history.push(Message::assistant(text, calls.clone()));
        }
        self.executed_tool_in_turn = false;
        if calls.is_empty() {
            AgentStep::Idle
        } else {
            for c in calls {
                self.pending_calls.push_back(c);
            }
            // Caller should immediately drive `step_tools` to run read-only tools or gate mutating ones.
            AgentStep::NeedsModelTurn
        }
    }

    /// Advances queued tool calls one step at a time.
    ///
    /// Read-only tools (`read_file`, `list_dir`) execute immediately. Mutating tools
    /// (`write_file`, `patch_file`, `run_command`, `hotswap_server`) stop and return
    /// [`AgentStep::AwaitingApproval`]. Once all queued tool calls have been resolved,
    /// returns [`AgentStep::NeedsModelTurn`] so the model can observe the tool results.
    pub fn step_tools(
        &mut self,
        ws: &mut dyn Workspace,
        runner: &mut dyn ToolRunner,
    ) -> AgentStep {
        if let Some(ref pending) = self.awaiting_approval {
            return AgentStep::AwaitingApproval(pending.clone());
        }
        let Some(call) = self.pending_calls.pop_front() else {
            if self.executed_tool_in_turn {
                self.executed_tool_in_turn = false;
                return AgentStep::NeedsModelTurn;
            }
            return AgentStep::Idle;
        };

        let needs_gate = ToolKind::from_name(&call.name)
            .map(ToolKind::requires_approval)
            .unwrap_or(true);
        if needs_gate {
            let summary = summarize_tool_call(&call);
            let pending = PendingApproval { call, summary };
            self.awaiting_approval = Some(pending.clone());
            return AgentStep::AwaitingApproval(pending);
        }

        let result = execute_tool(&call, ws, runner);
        self.history.push(Message::tool(result.clone()));
        self.executed_tool_in_turn = true;
        AgentStep::ToolExecuted(result)
    }

    /// Applies the human's approval or denial to the currently gated tool call.
    pub fn resolve_approval(
        &mut self,
        decision: ApprovalDecision,
        ws: &mut dyn Workspace,
        runner: &mut dyn ToolRunner,
    ) -> AgentStep {
        let Some(pending) = self.awaiting_approval.take() else {
            return AgentStep::Idle;
        };
        let result = match decision {
            ApprovalDecision::Approved => execute_tool(&pending.call, ws, runner),
            ApprovalDecision::Denied(reason) => ToolResult {
                call_id: pending.call.id,
                name: pending.call.name,
                output: if reason.is_empty() {
                    String::from("Denied by user.")
                } else {
                    format!("Denied by user: {}", reason)
                },
                is_error: true,
            },
        };
        self.history.push(Message::tool(result.clone()));
        self.executed_tool_in_turn = true;
        AgentStep::ToolExecuted(result)
    }
}

fn summarize_tool_call(call: &ToolCallRequest) -> String {
    match ToolKind::from_name(&call.name) {
        Some(ToolKind::WriteFile) => {
            let path = call
                .arguments
                .get("path")
                .and_then(JsonValue::as_str)
                .unwrap_or("?");
            let bytes = call
                .arguments
                .get("content")
                .and_then(JsonValue::as_str)
                .map(str::len)
                .unwrap_or(0);
            format!("write_file({}, {} bytes)", path, bytes)
        }
        Some(ToolKind::PatchFile) => {
            let path = call
                .arguments
                .get("path")
                .and_then(JsonValue::as_str)
                .unwrap_or("?");
            format!("patch_file({})", path)
        }
        Some(ToolKind::RunCommand) => {
            let prog = call
                .arguments
                .get("program")
                .and_then(JsonValue::as_str)
                .unwrap_or("?");
            let args = call
                .arguments
                .get("args")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let net = call
                .arguments
                .get("network")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            format!("run_command({} {}, net={})", prog, args, net)
        }
        Some(ToolKind::HotswapServer) => {
            let srv = call
                .arguments
                .get("server")
                .and_then(JsonValue::as_str)
                .unwrap_or("?");
            let bin = call
                .arguments
                .get("binary_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("?");
            format!("hotswap_server({} <- {})", srv, bin)
        }
        _ => format!("{}({})", call.name, call.arguments.to_json_string()),
    }
}

fn execute_tool(
    call: &ToolCallRequest,
    ws: &mut dyn Workspace,
    runner: &mut dyn ToolRunner,
) -> ToolResult {
    let outcome: Result<String, String> = match ToolKind::from_name(&call.name) {
        Some(ToolKind::ReadFile) => {
            match call.arguments.get("path").and_then(JsonValue::as_str) {
                Some(path) => ws.read_file(path),
                None => Err(String::from("Missing required argument: path")),
            }
        }
        Some(ToolKind::ListDir) => {
            let path = call
                .arguments
                .get("path")
                .and_then(JsonValue::as_str)
                .unwrap_or(".");
            ws.list_dir(path).map(|entries| entries.join("\n"))
        }
        Some(ToolKind::WriteFile) => {
            let path = call.arguments.get("path").and_then(JsonValue::as_str);
            let content = call.arguments.get("content").and_then(JsonValue::as_str);
            match (path, content) {
                (Some(p), Some(c)) => ws
                    .write_file(p, c)
                    .map(|_| format!("Wrote {} bytes to {}", c.len(), p)),
                _ => Err(String::from("Missing required arguments: path, content")),
            }
        }
        Some(ToolKind::PatchFile) => {
            let path = call.arguments.get("path").and_then(JsonValue::as_str);
            let old_text = call.arguments.get("old_text").and_then(JsonValue::as_str);
            let new_text = call.arguments.get("new_text").and_then(JsonValue::as_str);
            match (path, old_text, new_text) {
                (Some(p), Some(old), Some(new)) => apply_patch(ws, p, old, new),
                _ => Err(String::from(
                    "Missing required arguments: path, old_text, new_text",
                )),
            }
        }
        Some(ToolKind::RunCommand) => {
            let prog = call.arguments.get("program").and_then(JsonValue::as_str);
            let args = call
                .arguments
                .get("args")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let net = call
                .arguments
                .get("network")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            match prog {
                Some(p) => runner.run_command(p, args, net),
                None => Err(String::from("Missing required argument: program")),
            }
        }
        Some(ToolKind::HotswapServer) => {
            let srv = call.arguments.get("server").and_then(JsonValue::as_str);
            let bin = call
                .arguments
                .get("binary_path")
                .and_then(JsonValue::as_str);
            match (srv, bin) {
                (Some(s), Some(b)) => runner.hotswap_server(s, b),
                _ => Err(String::from(
                    "Missing required arguments: server, binary_path",
                )),
            }
        }
        None => Err(format!("Unknown tool: {}", call.name)),
    };

    match outcome {
        Ok(output) => ToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            output,
            is_error: false,
        },
        Err(err) => ToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            output: err,
            is_error: true,
        },
    }
}

fn apply_patch(
    ws: &mut dyn Workspace,
    path: &str,
    old_text: &str,
    new_text: &str,
) -> Result<String, String> {
    if old_text.is_empty() {
        return Err(String::from("old_text must not be empty"));
    }
    let current = ws.read_file(path)?;
    let count = current.matches(old_text).count();
    if count == 0 {
        return Err(format!("old_text not found in {}", path));
    }
    if count > 1 {
        return Err(format!(
            "old_text matched {} times in {} (must be unique)",
            count, path
        ));
    }
    let updated = current.replacen(old_text, new_text, 1);
    ws.write_file(path, &updated)?;
    Ok(format!("Patched {} ({} bytes)", path, updated.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    struct MemWorkspace {
        files: BTreeMap<String, String>,
    }

    impl MemWorkspace {
        fn new() -> Self {
            Self {
                files: BTreeMap::new(),
            }
        }
    }

    impl Workspace for MemWorkspace {
        fn read_file(&mut self, path: &str) -> Result<String, String> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| format!("not found: {}", path))
        }

        fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
            self.files.insert(String::from(path), String::from(content));
            Ok(())
        }

        fn list_dir(&mut self, _path: &str) -> Result<Vec<String>, String> {
            Ok(self.files.keys().cloned().collect())
        }
    }

    struct MockRunner {
        ran: Vec<(String, String, bool)>,
    }

    impl MockRunner {
        fn new() -> Self {
            Self { ran: Vec::new() }
        }
    }

    impl ToolRunner for MockRunner {
        fn run_command(&mut self, program: &str, args: &str, network: bool) -> Result<String, String> {
            self.ran
                .push((String::from(program), String::from(args), network));
            Ok(format!("exit 0 ({})", program))
        }

        fn hotswap_server(&mut self, server: &str, binary_path: &str) -> Result<String, String> {
            Ok(format!("hotswap queued: {} <- {}", server, binary_path))
        }
    }

    #[test]
    fn multi_turn_tool_loop_enforces_approval_gate() {
        let mut ws = MemWorkspace::new();
        ws.write_file("src/lib.rs", "pub fn val() -> u32 { 1 }\n")
            .unwrap();
        let mut runner = MockRunner::new();
        let mut agent = Agent::new(Provider::Claude);

        agent.push_user_message("Bump val() to 2 and run tests");

        // Turn 1: model calls read_file (auto-allowed) and patch_file (requires approval).
        agent.on_stream_event(&StreamEvent::TextDelta(String::from("Inspecting file.")));
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("c1"),
            name: String::from("read_file"),
            arguments: JsonValue::object(vec![(
                String::from("path"),
                JsonValue::str("src/lib.rs"),
            )]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("c2"),
            name: String::from("patch_file"),
            arguments: JsonValue::object(vec![
                (String::from("path"), JsonValue::str("src/lib.rs")),
                (String::from("old_text"), JsonValue::str("-> u32 { 1 }")),
                (String::from("new_text"), JsonValue::str("-> u32 { 2 }")),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("c3"),
            name: String::from("run_command"),
            arguments: JsonValue::object(vec![
                (String::from("program"), JsonValue::str("hello")),
                (String::from("args"), JsonValue::str("--check")),
            ]),
        });
        assert_eq!(agent.finish_model_turn(), AgentStep::NeedsModelTurn);

        // Step 1: read_file executes immediately without blocking.
        match agent.step_tools(&mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => {
                assert!(!res.is_error);
                assert!(res.output.contains("-> u32 { 1 }"));
            }
            other => panic!("expected read_file execution, got {:?}", other),
        }

        // Step 2: patch_file MUST pause for human approval!
        match agent.step_tools(&mut ws, &mut runner) {
            AgentStep::AwaitingApproval(pending) => {
                assert_eq!(pending.call.name, "patch_file");
                assert!(pending.summary.contains("src/lib.rs"));
            }
            other => panic!("expected approval gate, got {:?}", other),
        }
        // File must remain untouched until approved.
        assert_eq!(
            ws.read_file("src/lib.rs").unwrap(),
            "pub fn val() -> u32 { 1 }\n"
        );

        // Human approves patch_file.
        match agent.resolve_approval(ApprovalDecision::Approved, &mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => assert!(!res.is_error),
            other => panic!("expected patch execution, got {:?}", other),
        }
        assert_eq!(
            ws.read_file("src/lib.rs").unwrap(),
            "pub fn val() -> u32 { 2 }\n"
        );

        // Step 3: run_command MUST also pause for approval; human denies it.
        match agent.step_tools(&mut ws, &mut runner) {
            AgentStep::AwaitingApproval(pending) => assert_eq!(pending.call.name, "run_command"),
            other => panic!("expected run_command approval gate, got {:?}", other),
        }
        match agent.resolve_approval(
            ApprovalDecision::Denied(String::from("not now")),
            &mut ws,
            &mut runner,
        ) {
            AgentStep::ToolExecuted(res) => {
                assert!(res.is_error);
                assert!(res.output.contains("Denied by user: not now"));
            }
            other => panic!("expected denial result, got {:?}", other),
        }
        assert!(runner.ran.is_empty());

        // Queue drained -> ready for next model turn with tool results in history.
        assert_eq!(agent.step_tools(&mut ws, &mut runner), AgentStep::NeedsModelTurn);
        let req = agent.build_request("test-key");
        let req_str = core::str::from_utf8(&req.http_bytes).unwrap();
        assert!(req_str.contains("tool_result"));
        assert!(req_str.contains("Denied by user: not now"));

        // Turn 2: model replies with plain text -> Idle.
        agent.on_stream_event(&StreamEvent::TextDelta(String::from("Updated src/lib.rs.")));
        assert_eq!(agent.finish_model_turn(), AgentStep::Idle);
    }
}
