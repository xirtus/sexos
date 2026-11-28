// SPDX-License-Identifier: GPL-3.0-only
//! Multi-turn AI coding agent state machine with capability-scoped tools,
//! Antigravity workflow state (task boundaries, notifications, artifacts, step log),
//! unified diff previews, and configurable human-in-the-loop approval policies.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::diff::format_unified_diff;
use crate::json::JsonValue;
use crate::provider::{build_http_request, Provider, ProviderRequest};
use crate::snapshot::{Snapshot, SnapshotEntry, SnapshotStore};
use crate::sse::StreamEvent;
use crate::traits::{AccessScope, WorkspaceFs};

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are Tailor, the native AI coding agent running inside SexOS (a capability-based SASOS microkernel). \
You can inspect files with read_file and list_dir, search with find_by_name and grep_search, \
propose edits with write_file and patch_file, run programs with run_command, \
track progress with task_boundary and notify_user, and request server hot-swaps with hotswap_server. \
All mutating tools require explicit human approval before execution. Be concise, accurate, and safe.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentMode {
    Idle,
    Planning,
    Execution,
    Verification,
}

impl AgentMode {
    pub fn from_str(s: &str) -> Self {
        let t = s.trim();
        if t.eq_ignore_ascii_case("planning") {
            Self::Planning
        } else if t.eq_ignore_ascii_case("execution") {
            Self::Execution
        } else if t.eq_ignore_ascii_case("verification") {
            Self::Verification
        } else {
            Self::Idle
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Planning => "PLANNING",
            Self::Execution => "EXECUTION",
            Self::Verification => "VERIFICATION",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskBoundary {
    pub mode: AgentMode,
    pub task_name: String,
    pub task_status: String,
    pub task_summary: String,
    pub predicted_task_size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyUser {
    pub message: String,
    pub paths_to_review: Vec<String>,
    pub blocked_on_user: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Require human approval for all mutating tools (default, safest).
    AlwaysAsk,
    /// Auto-approve read-only tools, artifact writes (`is_artifact: true`), and `SafeToAutoRun: true` commands.
    AutoSafe,
    /// Auto-approve all tool calls without pausing.
    Turbo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Waiting,
    Done,
    Error,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    pub id: u64,
    pub tool_name: String,
    pub title: String,
    pub detail: String,
    pub status: StepStatus,
    pub diff: Option<String>,
}

/// A queued request to spin up a parallel child subagent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentRequest {
    pub id: String,
    pub role: String,
    pub prompt: String,
    pub workspace: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    ReadFile,
    WriteFile,
    PatchFile,
    DeleteFile,
    MoveFile,
    ListDir,
    FindByName,
    GrepSearch,
    RunCommand,
    TaskBoundary,
    NotifyUser,
    HotswapServer,
    SpawnSubagent,
}

impl ToolKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "read_file" | "view_file" => Some(Self::ReadFile),
            "write_file" | "write_to_file" => Some(Self::WriteFile),
            "patch_file" | "replace_file_content" => Some(Self::PatchFile),
            "delete_file" => Some(Self::DeleteFile),
            "move_file" => Some(Self::MoveFile),
            "list_dir" => Some(Self::ListDir),
            "find_by_name" => Some(Self::FindByName),
            "grep_search" => Some(Self::GrepSearch),
            "run_command" => Some(Self::RunCommand),
            "task_boundary" => Some(Self::TaskBoundary),
            "notify_user" => Some(Self::NotifyUser),
            "hotswap_server" => Some(Self::HotswapServer),
            "spawn_subagent" => Some(Self::SpawnSubagent),
            _ => None,
        }
    }

    /// Returns `true` for any mutating or side-effecting tool that requires explicit human approval.
    pub fn requires_approval(self) -> bool {
        matches!(
            self,
            Self::WriteFile
                | Self::PatchFile
                | Self::DeleteFile
                | Self::MoveFile
                | Self::RunCommand
                | Self::HotswapServer
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

/// Returns the built-in Tailor tool definitions (SexOS + Antigravity standard tools).
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: String::from("read_file"),
            description: String::from(
                "Read a UTF-8 text file inside the granted workspace (supports optional start_line/end_line).",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("path", "string", "Relative or granted path to read"),
                    ("start_line", "integer", "Optional 1-indexed start line"),
                    ("end_line", "integer", "Optional 1-indexed inclusive end line"),
                ],
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
                    (
                        "is_artifact",
                        "boolean",
                        "Set true if writing an agent artifact (plan, walkthrough, task list)",
                    ),
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
            name: String::from("find_by_name"),
            description: String::from("Find files matching a pattern within a workspace directory."),
            parameters_schema: schema_obj(
                vec![
                    ("dir", "string", "Directory to search"),
                    ("pattern", "string", "Filename glob or substring pattern"),
                ],
                &["dir", "pattern"],
            ),
        },
        ToolSpec {
            name: String::from("grep_search"),
            description: String::from("Search for text occurrences across files in the workspace."),
            parameters_schema: schema_obj(
                vec![
                    ("path", "string", "File or directory path to search"),
                    ("query", "string", "Substring or pattern to search for"),
                ],
                &["path", "query"],
            ),
        },
        ToolSpec {
            name: String::from("run_command"),
            description: String::from(
                "Launch a program or shell command (requires human approval).",
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
                    (
                        "SafeToAutoRun",
                        "boolean",
                        "True only for read-only / non-destructive commands",
                    ),
                ],
                &["program"],
            ),
        },
        ToolSpec {
            name: String::from("task_boundary"),
            description: String::from(
                "Update the current structured task mode, name, status, and summary.",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("mode", "string", "PLANNING, EXECUTION, VERIFICATION, or IDLE"),
                    ("task_name", "string", "Short title for the current task"),
                    ("task_status", "string", "Current step status"),
                    ("task_summary", "string", "Concise summary of progress so far"),
                    (
                        "predicted_task_size",
                        "integer",
                        "Estimated number of tool calls remaining",
                    ),
                ],
                &["mode", "task_name", "task_status", "task_summary"],
            ),
        },
        ToolSpec {
            name: String::from("notify_user"),
            description: String::from(
                "Notify the user with a message and optional artifact/file paths to review.",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("message", "string", "Message to present to the user"),
                    (
                        "blocked_on_user",
                        "boolean",
                        "True if waiting for user input or approval",
                    ),
                ],
                &["message"],
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
        ToolSpec {
            name: String::from("delete_file"),
            description: String::from(
                "Delete a file in the granted workspace or allowed scope (requires human approval).",
            ),
            parameters_schema: schema_obj(
                vec![("path", "string", "Path of the file to delete")],
                &["path"],
            ),
        },
        ToolSpec {
            name: String::from("move_file"),
            description: String::from(
                "Move or rename a file (requires human approval).",
            ),
            parameters_schema: schema_obj(
                vec![
                    ("from", "string", "Source file path"),
                    ("to", "string", "Destination file path"),
                ],
                &["from", "to"],
            ),
        },
        ToolSpec {
            name: String::from("spawn_subagent"),
            description: String::from(
                "Dispatch a parallel child subagent to handle a focused subtask.",
            ),
            parameters_schema: schema_obj(
                vec![
                    (
                        "role",
                        "string",
                        "Short role title for the subagent (e.g. 'Test Runner', 'Code Reviewer')",
                    ),
                    (
                        "prompt",
                        "string",
                        "Actionable task instructions for the subagent",
                    ),
                    (
                        "workspace",
                        "string",
                        "Optional workspace path override for the subagent",
                    ),
                ],
                &["role", "prompt"],
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
    pub thought_signature: Option<String>,
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
    pub thinking: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub tool_result: Option<ToolResult>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            tool_result: None,
        }
    }

    pub fn assistant(text: impl Into<String>, tool_calls: Vec<ToolCallRequest>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            thinking: String::new(),
            tool_calls,
            tool_result: None,
        }
    }

    pub fn assistant_with_thinking(
        text: impl Into<String>,
        thinking: impl Into<String>,
        tool_calls: Vec<ToolCallRequest>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
            thinking: thinking.into(),
            tool_calls,
            tool_result: None,
        }
    }

    pub fn tool(result: ToolResult) -> Self {
        Self {
            role: Role::Tool,
            text: String::new(),
            thinking: String::new(),
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
    pub diff: Option<String>,
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

/// Lightweight adapter so `&mut dyn Workspace` can be driven through `step_tools_fs`.
struct BasicWorkspaceAdapter<'a>(&'a mut dyn Workspace);

impl Workspace for BasicWorkspaceAdapter<'_> {
    fn read_file(&mut self, path: &str) -> Result<String, String> {
        self.0.read_file(path)
    }

    fn write_file(&mut self, path: &str, content: &str) -> Result<(), String> {
        self.0.write_file(path, content)
    }

    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, String> {
        self.0.list_dir(path)
    }
}

impl WorkspaceFs for BasicWorkspaceAdapter<'_> {}

/// Tailor's multi-turn agent state machine.
pub struct Agent {
    provider: Provider,
    model: String,
    system_prompt: String,
    tools: Vec<ToolSpec>,
    history: Vec<Message>,
    streaming_text: String,
    streaming_thinking: String,
    pending_thought_signature: Option<String>,
    streaming_calls: Vec<ToolCallRequest>,
    pending_calls: VecDeque<ToolCallRequest>,
    awaiting_approval: Option<PendingApproval>,
    awaiting_step_id: Option<u64>,
    executed_tool_in_turn: bool,
    task_boundary: Option<TaskBoundary>,
    last_notification: Option<NotifyUser>,
    steps: Vec<StepRecord>,
    next_step_id: u64,
    approval_policy: ApprovalPolicy,
    access_scope: AccessScope,
    artifacts: Vec<(String, String)>,
    snapshots: SnapshotStore,
    pending_subagents: VecDeque<SubagentRequest>,
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
            pending_thought_signature: None,
            streaming_calls: Vec::new(),
            pending_calls: VecDeque::new(),
            awaiting_approval: None,
            awaiting_step_id: None,
            executed_tool_in_turn: false,
            task_boundary: None,
            last_notification: None,
            steps: Vec::new(),
            next_step_id: 1,
            approval_policy: ApprovalPolicy::AlwaysAsk,
            access_scope: AccessScope::WorkspaceOnly,
            artifacts: Vec::new(),
            snapshots: SnapshotStore::new(),
            pending_subagents: VecDeque::new(),
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

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }

    pub fn pending_approval(&self) -> Option<&PendingApproval> {
        self.awaiting_approval.as_ref()
    }

    pub fn task_boundary(&self) -> Option<&TaskBoundary> {
        self.task_boundary.as_ref()
    }

    pub fn mode(&self) -> AgentMode {
        self.task_boundary
            .as_ref()
            .map(|tb| tb.mode)
            .unwrap_or(AgentMode::Idle)
    }

    pub fn last_notification(&self) -> Option<&NotifyUser> {
        self.last_notification.as_ref()
    }

    pub fn clear_notification(&mut self) {
        self.last_notification = None;
    }

    pub fn steps(&self) -> &[StepRecord] {
        &self.steps
    }

    pub fn approval_policy(&self) -> ApprovalPolicy {
        self.approval_policy
    }

    pub fn set_approval_policy(&mut self, policy: ApprovalPolicy) {
        self.approval_policy = policy;
    }

    pub fn access_scope(&self) -> AccessScope {
        self.access_scope
    }

    pub fn set_access_scope(&mut self, scope: AccessScope) {
        self.access_scope = scope;
    }

    pub fn snapshots(&self) -> &[Snapshot] {
        self.snapshots.list()
    }

    pub fn rollback_snapshot(&mut self, id: u64, ws: &mut dyn WorkspaceFs) -> Result<String, String> {
        self.snapshots.restore(id, ws)
    }

    pub fn create_checkpoint(&mut self, label: &str, paths: &[String], ws: &mut dyn WorkspaceFs) -> u64 {
        let mut entries = Vec::with_capacity(paths.len());
        for path in paths {
            let (existed, before) = match ws.read_file(path) {
                Ok(content) => (true, content),
                Err(_) => (false, String::new()),
            };
            let after = if existed { Some(before.clone()) } else { None };
            entries.push(SnapshotEntry {
                path: path.clone(),
                existed,
                before,
                after,
            });
        }
        self.snapshots.record(label, entries)
    }

    pub fn take_spawned_subagents(&mut self) -> Vec<SubagentRequest> {
        self.pending_subagents.drain(..).collect()
    }

    pub fn artifacts(&self) -> &[(String, String)] {
        &self.artifacts
    }

    pub fn set_artifact(&mut self, name: impl Into<String>, content: impl Into<String>) {
        let name = name.into();
        let content = content.into();
        if let Some(existing) = self.artifacts.iter_mut().find(|(n, _)| *n == name) {
            existing.1 = content;
        } else {
            self.artifacts.push((name, content));
        }
    }

    pub fn streaming_text(&self) -> &str {
        &self.streaming_text
    }

    pub fn streaming_thinking(&self) -> &str {
        &self.streaming_thinking
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.streaming_text.clear();
        self.streaming_thinking.clear();
        self.streaming_calls.clear();
        self.pending_calls.clear();
        self.awaiting_approval = None;
        self.awaiting_step_id = None;
        self.executed_tool_in_turn = false;
        self.task_boundary = None;
        self.last_notification = None;
        self.steps.clear();
        self.artifacts.clear();
        self.snapshots.clear();
        self.pending_subagents.clear();
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
            StreamEvent::ThoughtSignature(sig) => {
                if !sig.is_empty() {
                    self.pending_thought_signature = Some(sig.clone());
                }
            }
            StreamEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                self.streaming_calls.push(ToolCallRequest {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                    thought_signature: self.pending_thought_signature.take(),
                });
            }
            StreamEvent::Done { .. } | StreamEvent::Error(_) => {}
        }
    }

    /// Commits the streamed assistant message and queues any requested tool calls.
    pub fn finish_model_turn(&mut self) -> AgentStep {
        let text = core::mem::take(&mut self.streaming_text);
        let thinking = core::mem::take(&mut self.streaming_thinking);
        let calls = core::mem::take(&mut self.streaming_calls);
        if !text.is_empty() || !thinking.is_empty() || !calls.is_empty() {
            self.history
                .push(Message::assistant_with_thinking(text, thinking, calls.clone()));
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

    /// Advances queued tool calls one step at a time using the base [`Workspace`] trait.
    ///
    /// Preserves 100% backwards compatibility with SexOS `apps/tailor`.
    pub fn step_tools(
        &mut self,
        ws: &mut dyn Workspace,
        runner: &mut dyn ToolRunner,
    ) -> AgentStep {
        let mut adapter = BasicWorkspaceAdapter(ws);
        self.step_tools_fs(&mut adapter, runner)
    }

    /// Advances queued tool calls one step at a time using [`WorkspaceFs`].
    ///
    /// Read-only and workflow tools (`read_file`/`view_file`, `list_dir`, `find_by_name`,
    /// `grep_search`, `task_boundary`, `notify_user`) execute immediately. Mutating tools
    /// respect [`ApprovalPolicy`] and compute a unified diff preview for file edits.
    pub fn step_tools_fs(
        &mut self,
        ws: &mut dyn WorkspaceFs,
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

        let diff = preview_diff(&call, ws);
        let summary = summarize_tool_call(&call);
        let needs_gate = self.should_require_approval(&call);

        if needs_gate {
            let step_id = self.push_step(
                call.name.clone(),
                summary.clone(),
                String::from("Awaiting approval"),
                StepStatus::Waiting,
                diff.clone(),
            );
            let pending = PendingApproval {
                call,
                summary,
                diff,
            };
            self.awaiting_approval = Some(pending.clone());
            self.awaiting_step_id = Some(step_id);
            return AgentStep::AwaitingApproval(pending);
        }

        let result = self.execute_tool_internal(&call, ws, runner);
        let status = if result.is_error {
            StepStatus::Error
        } else {
            StepStatus::Done
        };
        self.push_step(
            call.name.clone(),
            summary,
            result.output.clone(),
            status,
            diff,
        );
        self.history.push(Message::tool(result.clone()));
        self.executed_tool_in_turn = true;
        AgentStep::ToolExecuted(result)
    }

    /// Applies the human's approval or denial to the currently gated tool call using [`Workspace`].
    pub fn resolve_approval(
        &mut self,
        decision: ApprovalDecision,
        ws: &mut dyn Workspace,
        runner: &mut dyn ToolRunner,
    ) -> AgentStep {
        let mut adapter = BasicWorkspaceAdapter(ws);
        self.resolve_approval_fs(decision, &mut adapter, runner)
    }

    /// Applies the human's approval or denial to the currently gated tool call using [`WorkspaceFs`].
    pub fn resolve_approval_fs(
        &mut self,
        decision: ApprovalDecision,
        ws: &mut dyn WorkspaceFs,
        runner: &mut dyn ToolRunner,
    ) -> AgentStep {
        let Some(pending) = self.awaiting_approval.take() else {
            return AgentStep::Idle;
        };
        let step_id = self.awaiting_step_id.take();

        let (result, status) = match decision {
            ApprovalDecision::Approved => {
                let res = self.execute_tool_internal(&pending.call, ws, runner);
                let st = if res.is_error {
                    StepStatus::Error
                } else {
                    StepStatus::Done
                };
                (res, st)
            }
            ApprovalDecision::Denied(reason) => {
                let res = ToolResult {
                    call_id: pending.call.id,
                    name: pending.call.name,
                    output: if reason.is_empty() {
                        String::from("Denied by user.")
                    } else {
                        format!("Denied by user: {}", reason)
                    },
                    is_error: true,
                };
                (res, StepStatus::Denied)
            }
        };

        if let Some(id) = step_id {
            if let Some(rec) = self.steps.iter_mut().find(|s| s.id == id) {
                rec.status = status;
                rec.detail = result.output.clone();
            }
        }

        self.history.push(Message::tool(result.clone()));
        self.executed_tool_in_turn = true;
        AgentStep::ToolExecuted(result)
    }

    fn should_require_approval(&self, call: &ToolCallRequest) -> bool {
        let kind = ToolKind::from_name(&call.name);
        match self.approval_policy {
            ApprovalPolicy::Turbo => false,
            ApprovalPolicy::AutoSafe => match kind {
                Some(k) if !k.requires_approval() => false,
                Some(ToolKind::WriteFile) => !is_artifact_write(call),
                Some(ToolKind::RunCommand) => !is_safe_to_auto_run(call),
                _ => true,
            },
            ApprovalPolicy::AlwaysAsk => kind.map(ToolKind::requires_approval).unwrap_or(true),
        }
    }

    fn push_step(
        &mut self,
        tool_name: String,
        title: String,
        detail: String,
        status: StepStatus,
        diff: Option<String>,
    ) -> u64 {
        let id = self.next_step_id;
        self.next_step_id += 1;
        self.steps.push(StepRecord {
            id,
            tool_name,
            title,
            detail,
            status,
            diff,
        });
        id
    }

    fn execute_tool_internal(
        &mut self,
        call: &ToolCallRequest,
        ws: &mut dyn WorkspaceFs,
        runner: &mut dyn ToolRunner,
    ) -> ToolResult {
        let outcome: Result<String, String> = match ToolKind::from_name(&call.name) {
            Some(ToolKind::ReadFile) => {
                let path = arg_str(call, &["path", "AbsolutePath", "TargetFile"]);
                match path {
                    Some(p) => ws.read_file(p).map(|content| {
                        let start = arg_usize(call, &["start_line", "StartLine"]);
                        let end = arg_usize(call, &["end_line", "EndLine"]);
                        slice_lines(&content, start, end)
                    }),
                    None => Err(String::from("Missing required argument: path")),
                }
            }
            Some(ToolKind::ListDir) => {
                let path = arg_str(call, &["path", "DirectoryPath"]).unwrap_or(".");
                ws.list_dir(path).map(|entries| entries.join("\n"))
            }
            Some(ToolKind::FindByName) => {
                let dir = arg_str(call, &["dir", "path", "SearchDirectory"]).unwrap_or(".");
                let pattern = arg_str(call, &["pattern", "Pattern"]).unwrap_or("*");
                ws.find_by_name(dir, pattern)
                    .map(|entries| entries.join("\n"))
            }
            Some(ToolKind::GrepSearch) => {
                let path = arg_str(call, &["path", "SearchPath"]).unwrap_or(".");
                let query = arg_str(call, &["query", "Query"]);
                match query {
                    Some(q) => ws.grep_search(path, q).map(|matches| {
                        let mut out = String::new();
                        for (i, m) in matches.iter().enumerate() {
                            if i > 0 {
                                out.push('\n');
                            }
                            let _ = write!(&mut out, "{}:{}:{}", m.path, m.line, m.text);
                        }
                        out
                    }),
                    None => Err(String::from("Missing required argument: query")),
                }
            }
            Some(ToolKind::WriteFile) => {
                let path = arg_str(call, &["path", "TargetFile"]);
                let content = arg_str(call, &["content", "CodeContent"]);
                let artifact = is_artifact_write(call);
                match (path, content) {
                    (Some(p), Some(c)) => {
                        if artifact {
                            self.set_artifact(p, c);
                            ws.write_artifact(p, c)
                                .map(|_| format!("Wrote {} bytes to {}", c.len(), p))
                        } else {
                            let (existed, before) = match ws.read_file(p) {
                                Ok(prev) => (true, prev),
                                Err(_) => (false, String::new()),
                            };
                            ws.write_file(p, c).map(|_| {
                                self.snapshots.record(
                                    format!("{} {}", call.name, p),
                                    vec![SnapshotEntry {
                                        path: String::from(p),
                                        existed,
                                        before,
                                        after: Some(String::from(c)),
                                    }],
                                );
                                format!("Wrote {} bytes to {}", c.len(), p)
                            })
                        }
                    }
                    _ => Err(String::from("Missing required arguments: path, content")),
                }
            }
            Some(ToolKind::PatchFile) => {
                let path = arg_str(call, &["path", "TargetFile"]);
                let old_text = arg_str(call, &["old_text", "TargetContent"]);
                let new_text = arg_str(call, &["new_text", "ReplacementContent"]);
                match (path, old_text, new_text) {
                    (Some(p), Some(old), Some(new)) => {
                        apply_patch(ws, p, old, new).map(|(before, updated, msg)| {
                            self.snapshots.record(
                                format!("{} {}", call.name, p),
                                vec![SnapshotEntry {
                                    path: String::from(p),
                                    existed: true,
                                    before,
                                    after: Some(updated),
                                }],
                            );
                            msg
                        })
                    }
                    _ => Err(String::from(
                        "Missing required arguments: path, old_text, new_text",
                    )),
                }
            }
            Some(ToolKind::DeleteFile) => {
                let path = arg_str(call, &["path", "TargetFile"]);
                match path {
                    Some(p) => {
                        let (existed, before) = match ws.read_file(p) {
                            Ok(prev) => (true, prev),
                            Err(_) => (false, String::new()),
                        };
                        ws.delete_file(p).map(|_| {
                            self.snapshots.record(
                                format!("delete_file {}", p),
                                vec![SnapshotEntry {
                                    path: String::from(p),
                                    existed,
                                    before,
                                    after: None,
                                }],
                            );
                            format!("Deleted {}", p)
                        })
                    }
                    None => Err(String::from("Missing required argument: path")),
                }
            }
            Some(ToolKind::MoveFile) => {
                let from = arg_str(call, &["from", "source", "src"]);
                let to = arg_str(call, &["to", "destination", "dst"]);
                match (from, to) {
                    (Some(src), Some(dst)) => {
                        let (from_existed, from_before) = match ws.read_file(src) {
                            Ok(prev) => (true, prev),
                            Err(_) => (false, String::new()),
                        };
                        let (to_existed, to_before) = match ws.read_file(dst) {
                            Ok(prev) => (true, prev),
                            Err(_) => (false, String::new()),
                        };
                        ws.move_file(src, dst).map(|_| {
                            let moved_content = if from_existed {
                                Some(from_before.clone())
                            } else {
                                ws.read_file(dst).ok()
                            };
                            self.snapshots.record(
                                format!("move_file {} -> {}", src, dst),
                                vec![
                                    SnapshotEntry {
                                        path: String::from(src),
                                        existed: from_existed,
                                        before: from_before,
                                        after: None,
                                    },
                                    SnapshotEntry {
                                        path: String::from(dst),
                                        existed: to_existed,
                                        before: to_before,
                                        after: moved_content,
                                    },
                                ],
                            );
                            format!("Moved {} -> {}", src, dst)
                        })
                    }
                    _ => Err(String::from("Missing required arguments: from, to")),
                }
            }
            Some(ToolKind::RunCommand) => {
                let prog = arg_str(call, &["program", "CommandLine"]);
                let args = arg_str(call, &["args"]).unwrap_or("");
                let net = arg_bool(call, &["network"]).unwrap_or(false);
                match prog {
                    Some(p) => runner.run_command(p, args, net),
                    None => Err(String::from("Missing required argument: program")),
                }
            }
            Some(ToolKind::TaskBoundary) => {
                let mode_str = arg_str(call, &["mode", "Mode"]).unwrap_or("IDLE");
                let task_name = String::from(arg_str(call, &["task_name", "TaskName"]).unwrap_or(""));
                let task_status =
                    String::from(arg_str(call, &["task_status", "TaskStatus"]).unwrap_or(""));
                let task_summary =
                    String::from(arg_str(call, &["task_summary", "TaskSummary"]).unwrap_or(""));
                let predicted_task_size =
                    arg_usize(call, &["predicted_task_size", "PredictedTaskSize"]).unwrap_or(0)
                        as u32;
                let tb = TaskBoundary {
                    mode: AgentMode::from_str(mode_str),
                    task_name,
                    task_status,
                    task_summary,
                    predicted_task_size,
                };
                let msg = format!(
                    "Task boundary updated: [{}] {} — {}",
                    tb.mode.as_str(),
                    tb.task_name,
                    tb.task_status
                );
                self.task_boundary = Some(tb);
                Ok(msg)
            }
            Some(ToolKind::NotifyUser) => {
                let message = String::from(arg_str(call, &["message", "Message"]).unwrap_or(""));
                let blocked_on_user =
                    arg_bool(call, &["blocked_on_user", "BlockedOnUser"]).unwrap_or(false);
                let paths_to_review = call
                    .arguments
                    .get("paths_to_review")
                    .or_else(|| call.arguments.get("PathsToReview"))
                    .and_then(JsonValue::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(JsonValue::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                self.last_notification = Some(NotifyUser {
                    message: message.clone(),
                    paths_to_review,
                    blocked_on_user,
                });
                Ok(format!("User notified: {}", message))
            }
            Some(ToolKind::HotswapServer) => {
                let srv = arg_str(call, &["server"]);
                let bin = arg_str(call, &["binary_path"]);
                match (srv, bin) {
                    (Some(s), Some(b)) => runner.hotswap_server(s, b),
                    _ => Err(String::from(
                        "Missing required arguments: server, binary_path",
                    )),
                }
            }
            Some(ToolKind::SpawnSubagent) => {
                let role = arg_str(call, &["role", "Role"]);
                let prompt = arg_str(call, &["prompt", "Prompt"]);
                let workspace = arg_str(call, &["workspace", "Workspace"]).map(String::from);
                match (role, prompt) {
                    (Some(r), Some(p)) => {
                        self.pending_subagents.push_back(SubagentRequest {
                            id: call.id.clone(),
                            role: String::from(r),
                            prompt: String::from(p),
                            workspace,
                        });
                        Ok(format!("Dispatched subagent '{}' (id: {})", r, call.id))
                    }
                    _ => Err(String::from("Missing required arguments: role, prompt")),
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
}

fn arg_str<'a>(call: &'a ToolCallRequest, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = call.arguments.get(key).and_then(JsonValue::as_str) {
            return Some(s);
        }
    }
    None
}

fn arg_bool(call: &ToolCallRequest, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(b) = call.arguments.get(key).and_then(JsonValue::as_bool) {
            return Some(b);
        }
    }
    None
}

fn arg_usize(call: &ToolCallRequest, keys: &[&str]) -> Option<usize> {
    for key in keys {
        if let Some(n) = call.arguments.get(key).and_then(JsonValue::as_u64) {
            return Some(n as usize);
        }
    }
    None
}

fn is_artifact_write(call: &ToolCallRequest) -> bool {
    arg_bool(call, &["is_artifact", "IsArtifact"]).unwrap_or(false)
}

fn is_safe_to_auto_run(call: &ToolCallRequest) -> bool {
    arg_bool(call, &["SafeToAutoRun", "safe_to_auto_run"]).unwrap_or(false)
}

fn slice_lines(content: &str, start_line: Option<usize>, end_line: Option<usize>) -> String {
    if start_line.is_none() && end_line.is_none() {
        return String::from(content);
    }
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let start = start_line.unwrap_or(1).max(1);
    let end = end_line.unwrap_or(lines.len()).min(lines.len());
    if start > end || start > lines.len() {
        return String::new();
    }
    let mut out = String::new();
    for (idx, line) in lines[start - 1..end].iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let _ = write!(&mut out, "{}: {}", start + idx, line);
    }
    out
}

fn preview_diff(call: &ToolCallRequest, ws: &mut dyn WorkspaceFs) -> Option<String> {
    match ToolKind::from_name(&call.name) {
        Some(ToolKind::WriteFile) => {
            let path = arg_str(call, &["path", "TargetFile"])?;
            let content = arg_str(call, &["content", "CodeContent"])?;
            let old = if is_artifact_write(call) {
                ws.read_artifact(path).unwrap_or_default()
            } else {
                ws.read_file(path).unwrap_or_default()
            };
            let diff = format_unified_diff(path, &old, content);
            if diff.is_empty() {
                None
            } else {
                Some(diff)
            }
        }
        Some(ToolKind::PatchFile) => {
            let path = arg_str(call, &["path", "TargetFile"])?;
            let old_text = arg_str(call, &["old_text", "TargetContent"])?;
            let new_text = arg_str(call, &["new_text", "ReplacementContent"])?;
            if old_text.is_empty() {
                return None;
            }
            let current = ws.read_file(path).ok()?;
            if current.matches(old_text).count() != 1 {
                return None;
            }
            let updated = current.replacen(old_text, new_text, 1);
            let diff = format_unified_diff(path, &current, &updated);
            if diff.is_empty() {
                None
            } else {
                Some(diff)
            }
        }
        Some(ToolKind::DeleteFile) => {
            let path = arg_str(call, &["path", "TargetFile"])?;
            let current = ws.read_file(path).ok()?;
            let diff = format_unified_diff(path, &current, "");
            if diff.is_empty() {
                None
            } else {
                Some(diff)
            }
        }
        _ => None,
    }
}

fn summarize_tool_call(call: &ToolCallRequest) -> String {
    match ToolKind::from_name(&call.name) {
        Some(ToolKind::ReadFile) => {
            let path = arg_str(call, &["path", "AbsolutePath", "TargetFile"]).unwrap_or("?");
            format!("read_file({})", path)
        }
        Some(ToolKind::ListDir) => {
            let path = arg_str(call, &["path", "DirectoryPath"]).unwrap_or(".");
            format!("list_dir({})", path)
        }
        Some(ToolKind::FindByName) => {
            let dir = arg_str(call, &["dir", "path", "SearchDirectory"]).unwrap_or(".");
            let pat = arg_str(call, &["pattern", "Pattern"]).unwrap_or("*");
            format!("find_by_name({}, {})", dir, pat)
        }
        Some(ToolKind::GrepSearch) => {
            let path = arg_str(call, &["path", "SearchPath"]).unwrap_or(".");
            let q = arg_str(call, &["query", "Query"]).unwrap_or("?");
            format!("grep_search({}, {:?})", path, q)
        }
        Some(ToolKind::WriteFile) => {
            let path = arg_str(call, &["path", "TargetFile"]).unwrap_or("?");
            let bytes = arg_str(call, &["content", "CodeContent"])
                .map(str::len)
                .unwrap_or(0);
            format!("write_file({}, {} bytes)", path, bytes)
        }
        Some(ToolKind::PatchFile) => {
            let path = arg_str(call, &["path", "TargetFile"]).unwrap_or("?");
            format!("patch_file({})", path)
        }
        Some(ToolKind::DeleteFile) => {
            let path = arg_str(call, &["path", "TargetFile"]).unwrap_or("?");
            format!("delete_file({})", path)
        }
        Some(ToolKind::MoveFile) => {
            let from = arg_str(call, &["from", "source", "src"]).unwrap_or("?");
            let to = arg_str(call, &["to", "destination", "dst"]).unwrap_or("?");
            format!("move_file({} -> {})", from, to)
        }
        Some(ToolKind::RunCommand) => {
            let prog = arg_str(call, &["program", "CommandLine"]).unwrap_or("?");
            let args = arg_str(call, &["args"]).unwrap_or("");
            let net = arg_bool(call, &["network"]).unwrap_or(false);
            format!("run_command({} {}, net={})", prog, args, net)
        }
        Some(ToolKind::TaskBoundary) => {
            let mode = arg_str(call, &["mode", "Mode"]).unwrap_or("IDLE");
            let name = arg_str(call, &["task_name", "TaskName"]).unwrap_or("");
            format!("task_boundary({}: {})", mode, name)
        }
        Some(ToolKind::NotifyUser) => String::from("notify_user"),
        Some(ToolKind::HotswapServer) => {
            let srv = arg_str(call, &["server"]).unwrap_or("?");
            let bin = arg_str(call, &["binary_path"]).unwrap_or("?");
            format!("hotswap_server({} <- {})", srv, bin)
        }
        Some(ToolKind::SpawnSubagent) => {
            let role = arg_str(call, &["role", "Role"]).unwrap_or("?");
            format!("spawn_subagent({})", role)
        }
        _ => format!("{}({})", call.name, call.arguments.to_json_string()),
    }
}

fn apply_patch(
    ws: &mut dyn WorkspaceFs,
    path: &str,
    old_text: &str,
    new_text: &str,
) -> Result<(String, String, String), String> {
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
    let msg = format!("Patched {} ({} bytes)", path, updated.len());
    Ok((current, updated, msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::GrepMatch;
    use alloc::collections::BTreeMap;

    struct MemWorkspace {
        files: BTreeMap<String, String>,
        artifacts: BTreeMap<String, String>,
        scope: AccessScope,
    }

    impl MemWorkspace {
        fn new() -> Self {
            Self {
                files: BTreeMap::new(),
                artifacts: BTreeMap::new(),
                scope: AccessScope::WorkspaceOnly,
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

    impl WorkspaceFs for MemWorkspace {
        fn find_by_name(&mut self, _dir: &str, pattern: &str) -> Result<Vec<String>, String> {
            let needle = pattern.trim_matches('*');
            Ok(self
                .files
                .keys()
                .filter(|k| needle.is_empty() || k.contains(needle))
                .cloned()
                .collect())
        }

        fn grep_search(&mut self, _path: &str, query: &str) -> Result<Vec<GrepMatch>, String> {
            let mut out = Vec::new();
            for (path, content) in &self.files {
                for (i, line) in content.lines().enumerate() {
                    if line.contains(query) {
                        out.push(GrepMatch {
                            path: path.clone(),
                            line: i + 1,
                            text: String::from(line),
                        });
                    }
                }
            }
            Ok(out)
        }

        fn read_artifact(&mut self, name: &str) -> Result<String, String> {
            self.artifacts
                .get(name)
                .cloned()
                .ok_or_else(|| format!("artifact not found: {}", name))
        }

        fn write_artifact(&mut self, name: &str, content: &str) -> Result<(), String> {
            self.artifacts
                .insert(String::from(name), String::from(content));
            Ok(())
        }

        fn delete_file(&mut self, path: &str) -> Result<(), String> {
            self.files
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| format!("not found: {}", path))
        }

        fn move_file(&mut self, from: &str, to: &str) -> Result<(), String> {
            let content = self
                .files
                .remove(from)
                .ok_or_else(|| format!("not found: {}", from))?;
            self.files.insert(String::from(to), content);
            Ok(())
        }

        fn set_access_scope(&mut self, scope: AccessScope) {
            self.scope = scope;
        }

        fn access_scope(&self) -> AccessScope {
            self.scope
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
        agent.on_stream_event(&StreamEvent::ThinkingDelta(String::from("Let me check")));
        agent.on_stream_event(&StreamEvent::TextDelta(String::from("Inspecting file.")));
        assert_eq!(agent.streaming_thinking(), "Let me check");
        assert_eq!(agent.streaming_text(), "Inspecting file.");

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
        assert_eq!(agent.history()[1].thinking, "Let me check");

        // Step 1: read_file executes immediately without blocking.
        match agent.step_tools(&mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => {
                assert!(!res.is_error);
                assert!(res.output.contains("-> u32 { 1 }"));
            }
            other => panic!("expected read_file execution, got {:?}", other),
        }

        // Step 2: patch_file MUST pause for human approval with unified diff preview!
        match agent.step_tools(&mut ws, &mut runner) {
            AgentStep::AwaitingApproval(pending) => {
                assert_eq!(pending.call.name, "patch_file");
                assert!(pending.summary.contains("src/lib.rs"));
                let diff = pending.diff.expect("expected unified diff preview");
                assert!(diff.contains("-pub fn val() -> u32 { 1 }"));
                assert!(diff.contains("+pub fn val() -> u32 { 2 }"));
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

        // Verify step log recorded Done, Done, Denied.
        assert_eq!(agent.steps().len(), 3);
        assert_eq!(agent.steps()[0].status, StepStatus::Done);
        assert_eq!(agent.steps()[1].status, StepStatus::Done);
        assert!(agent.steps()[1].diff.is_some());
        assert_eq!(agent.steps()[2].status, StepStatus::Denied);

        // Queue drained -> ready for next model turn with tool results in history.
        assert_eq!(
            agent.step_tools(&mut ws, &mut runner),
            AgentStep::NeedsModelTurn
        );
        let req = agent.build_request("test-key");
        let req_str = core::str::from_utf8(&req.http_bytes).unwrap();
        assert!(req_str.contains("tool_result"));
        assert!(req_str.contains("Denied by user: not now"));

        // Turn 2: model replies with plain text -> Idle.
        agent.on_stream_event(&StreamEvent::TextDelta(String::from("Updated src/lib.rs.")));
        assert_eq!(agent.finish_model_turn(), AgentStep::Idle);
    }

    #[test]
    fn antigravity_workflow_tools_artifacts_and_policies() {
        let mut ws = MemWorkspace::new();
        ws.write_file("src/main.rs", "line1\nline2\nfn main() {}\nline4\n")
            .unwrap();
        let mut runner = MockRunner::new();
        let mut agent = Agent::new(Provider::Antigravity);
        agent.set_approval_policy(ApprovalPolicy::AutoSafe);
        assert_eq!(agent.approval_policy(), ApprovalPolicy::AutoSafe);
        assert_eq!(agent.mode(), AgentMode::Idle);

        // Model invokes task_boundary, view_file (sliced), find_by_name, grep_search,
        // write_to_file (artifact -> auto-approved in AutoSafe), SafeToAutoRun command, and notify_user.
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t1"),
            name: String::from("task_boundary"),
            arguments: JsonValue::object(vec![
                (String::from("mode"), JsonValue::str("PLANNING")),
                (String::from("task_name"), JsonValue::str("Upgrade Core")),
                (String::from("task_status"), JsonValue::str("Inspecting")),
                (String::from("task_summary"), JsonValue::str("Reading main.rs")),
                (String::from("predicted_task_size"), JsonValue::uint(4)),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t2"),
            name: String::from("view_file"),
            arguments: JsonValue::object(vec![
                (String::from("path"), JsonValue::str("src/main.rs")),
                (String::from("start_line"), JsonValue::uint(2)),
                (String::from("end_line"), JsonValue::uint(3)),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t3"),
            name: String::from("find_by_name"),
            arguments: JsonValue::object(vec![
                (String::from("dir"), JsonValue::str(".")),
                (String::from("pattern"), JsonValue::str("*.rs")),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t4"),
            name: String::from("grep_search"),
            arguments: JsonValue::object(vec![
                (String::from("path"), JsonValue::str(".")),
                (String::from("query"), JsonValue::str("fn main")),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t5"),
            name: String::from("write_to_file"),
            arguments: JsonValue::object(vec![
                (String::from("path"), JsonValue::str("implementation_plan.md")),
                (String::from("content"), JsonValue::str("# Plan\nStep 1\n")),
                (String::from("is_artifact"), JsonValue::Bool(true)),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t6"),
            name: String::from("run_command"),
            arguments: JsonValue::object(vec![
                (String::from("program"), JsonValue::str("cargo")),
                (String::from("args"), JsonValue::str("test")),
                (String::from("SafeToAutoRun"), JsonValue::Bool(true)),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t7"),
            name: String::from("notify_user"),
            arguments: JsonValue::object(vec![
                (String::from("message"), JsonValue::str("Plan ready.")),
                (
                    String::from("paths_to_review"),
                    JsonValue::array(vec![JsonValue::str("implementation_plan.md")]),
                ),
                (String::from("blocked_on_user"), JsonValue::Bool(true)),
            ]),
        });
        assert_eq!(agent.finish_model_turn(), AgentStep::NeedsModelTurn);

        // Drain all 7 tools; all should execute automatically under AutoSafe!
        let mut results = Vec::new();
        loop {
            match agent.step_tools_fs(&mut ws, &mut runner) {
                AgentStep::ToolExecuted(r) => results.push(r),
                AgentStep::NeedsModelTurn => break,
                other => panic!("unexpected step under AutoSafe: {:?}", other),
            }
        }
        assert_eq!(results.len(), 7);
        assert_eq!(agent.mode(), AgentMode::Planning);
        assert_eq!(agent.task_boundary().unwrap().predicted_task_size, 4);
        assert_eq!(results[1].output, "2: line2\n3: fn main() {}");
        assert!(results[2].output.contains("src/main.rs"));
        assert_eq!(results[3].output, "src/main.rs:3:fn main() {}");
        assert_eq!(ws.read_artifact("implementation_plan.md").unwrap(), "# Plan\nStep 1\n");
        assert_eq!(agent.artifacts().len(), 1);
        assert_eq!(agent.artifacts()[0].0, "implementation_plan.md");
        assert_eq!(runner.ran.len(), 1);
        let notif = agent.last_notification().unwrap();
        assert_eq!(notif.message, "Plan ready.");
        assert!(notif.blocked_on_user);
        assert_eq!(notif.paths_to_review, vec![String::from("implementation_plan.md")]);
        agent.clear_notification();
        assert!(agent.last_notification().is_none());

        // Turbo mode auto-approves regular file writes too.
        agent.set_approval_policy(ApprovalPolicy::Turbo);
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("t8"),
            name: String::from("replace_file_content"),
            arguments: JsonValue::object(vec![
                (String::from("TargetFile"), JsonValue::str("src/main.rs")),
                (String::from("TargetContent"), JsonValue::str("fn main() {}")),
                (
                    String::from("ReplacementContent"),
                    JsonValue::str("fn main() { println!(\"hi\"); }"),
                ),
            ]),
        });
        agent.finish_model_turn();
        match agent.step_tools_fs(&mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => assert!(!res.is_error),
            other => panic!("expected Turbo auto-approve, got {:?}", other),
        }
        assert!(ws.read_file("src/main.rs").unwrap().contains("println!"));
    }

    #[test]
    fn auto_snapshots_delete_move_and_rollback_work() {
        let mut ws = MemWorkspace::new();
        ws.write_file("a.txt", "hello\n").unwrap();
        let mut runner = MockRunner::new();
        let mut agent = Agent::new(Provider::Claude);

        // Verify AccessScope default and setter.
        assert_eq!(agent.access_scope(), AccessScope::WorkspaceOnly);
        agent.set_access_scope(AccessScope::FullSystem);
        assert_eq!(agent.access_scope(), AccessScope::FullSystem);

        // Create a manual checkpoint before edits.
        let cp_id = agent.create_checkpoint("before edits", &[String::from("a.txt")], &mut ws);
        assert_eq!(cp_id, 1);
        assert_eq!(agent.snapshots().len(), 1);

        // Test move_file requires approval, records snapshot, and rolls back cleanly.
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("m1"),
            name: String::from("move_file"),
            arguments: JsonValue::object(vec![
                (String::from("from"), JsonValue::str("a.txt")),
                (String::from("to"), JsonValue::str("b.txt")),
            ]),
        });
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("d1"),
            name: String::from("delete_file"),
            arguments: JsonValue::object(vec![(String::from("path"), JsonValue::str("b.txt"))]),
        });
        agent.finish_model_turn();

        match agent.step_tools_fs(&mut ws, &mut runner) {
            AgentStep::AwaitingApproval(p) => {
                assert_eq!(p.call.name, "move_file");
                assert!(p.summary.contains("a.txt -> b.txt"));
            }
            other => panic!("expected move_file approval gate, got {:?}", other),
        }
        match agent.resolve_approval_fs(ApprovalDecision::Approved, &mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => assert!(!res.is_error),
            other => panic!("expected move_file execution, got {:?}", other),
        }
        assert!(ws.read_file("a.txt").is_err());
        assert_eq!(ws.read_file("b.txt").unwrap(), "hello\n");
        let move_snap_id = agent.snapshots().last().unwrap().id;

        // Next tool: delete_file requires approval with unified diff preview.
        match agent.step_tools_fs(&mut ws, &mut runner) {
            AgentStep::AwaitingApproval(p) => {
                assert_eq!(p.call.name, "delete_file");
                assert!(p.diff.unwrap().contains("-hello"));
            }
            other => panic!("expected delete_file approval gate, got {:?}", other),
        }
        match agent.resolve_approval_fs(ApprovalDecision::Approved, &mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => assert!(!res.is_error),
            other => panic!("expected delete_file execution, got {:?}", other),
        }
        assert!(ws.read_file("b.txt").is_err());
        let del_snap_id = agent.snapshots().last().unwrap().id;

        // Rollback delete_file -> b.txt returns.
        agent.rollback_snapshot(del_snap_id, &mut ws).unwrap();
        assert_eq!(ws.read_file("b.txt").unwrap(), "hello\n");

        // Rollback move_file -> a.txt returns and b.txt is deleted!
        agent.rollback_snapshot(move_snap_id, &mut ws).unwrap();
        assert_eq!(ws.read_file("a.txt").unwrap(), "hello\n");
        assert!(ws.read_file("b.txt").is_err());

        // Write a brand new file in Turbo mode, then rollback -> new file deleted.
        agent.set_approval_policy(ApprovalPolicy::Turbo);
        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("w1"),
            name: String::from("write_file"),
            arguments: JsonValue::object(vec![
                (String::from("path"), JsonValue::str("new.txt")),
                (String::from("content"), JsonValue::str("brand new\n")),
            ]),
        });
        agent.finish_model_turn();
        match agent.step_tools_fs(&mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => assert!(!res.is_error),
            other => panic!("expected write_file execution, got {:?}", other),
        }
        assert_eq!(ws.read_file("new.txt").unwrap(), "brand new\n");
        let write_snap_id = agent.snapshots().last().unwrap().id;
        agent.rollback_snapshot(write_snap_id, &mut ws).unwrap();
        assert!(ws.read_file("new.txt").is_err());
    }

    #[test]
    fn spawn_subagent_queues_requests_without_approval_gate() {
        let mut ws = MemWorkspace::new();
        let mut runner = MockRunner::new();
        let mut agent = Agent::new(Provider::Gemini);
        assert_eq!(agent.approval_policy(), ApprovalPolicy::AlwaysAsk);

        // Verify tool spec is registered in builtin_tools.
        assert!(agent.tools().iter().any(|t| t.name == "spawn_subagent"));
        assert!(agent.tools().iter().any(|t| t.name == "delete_file"));
        assert!(agent.tools().iter().any(|t| t.name == "move_file"));

        agent.on_stream_event(&StreamEvent::ToolCall {
            id: String::from("sub1"),
            name: String::from("spawn_subagent"),
            arguments: JsonValue::object(vec![
                (String::from("role"), JsonValue::str("Test Runner")),
                (String::from("prompt"), JsonValue::str("Run cargo test")),
                (String::from("workspace"), JsonValue::str("/tmp/ws")),
            ]),
        });
        agent.finish_model_turn();

        // Must execute immediately without pausing for approval even under AlwaysAsk.
        match agent.step_tools_fs(&mut ws, &mut runner) {
            AgentStep::ToolExecuted(res) => {
                assert!(!res.is_error);
                assert!(res.output.contains("Dispatched subagent 'Test Runner'"));
            }
            other => panic!("expected immediate spawn_subagent execution, got {:?}", other),
        }
        assert_eq!(agent.steps().len(), 1);
        assert_eq!(agent.steps()[0].tool_name, "spawn_subagent");
        assert_eq!(agent.steps()[0].status, StepStatus::Done);

        let spawned = agent.take_spawned_subagents();
        assert_eq!(spawned.len(), 1);
        assert_eq!(
            spawned[0],
            SubagentRequest {
                id: String::from("sub1"),
                role: String::from("Test Runner"),
                prompt: String::from("Run cargo test"),
                workspace: Some(String::from("/tmp/ws")),
            }
        );
        // Draining again returns empty.
        assert!(agent.take_spawned_subagents().is_empty());
    }
}
