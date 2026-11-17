// SPDX-License-Identifier: GPL-3.0-only
//! Omanta custom context actions, Collar launch planner, and transactional undo journal wiring
//! for Loom (`apps/files`). Pure `#![no_std]` + `alloc`.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use loom_core::actions::{
    builtin_actions, parse_actions, plan_collar_launch, ActionError, CollarLaunchPlan, UserAction,
};
use loom_core::journal::{
    ConflictKind, FileOp, FsProbe, PathMeta, PlannedAction, TransferProgress, UndoPlan,
};
use sex_app::files;
use sex_app::Window;

use crate::{at, base, shown, App, ENTER, ESC};

/// Directory holding user-defined `.toml` context actions on SexOS.
pub const ACTIONS_DIR: &[u8] = b"/data/loom/actions";
const MAX_ACTION_FILE_BYTES: usize = 64 * 1024;
static ACTION_COOKIES: AtomicU32 = AtomicU32::new(0x0A11_0000);

/// Loaded Omanta context action registry, falling back to [`builtin_actions()`].
pub struct Registry {
    pub actions: Vec<UserAction>,
    pub loaded: bool,
    pub loaded_from_disk: bool,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            actions: builtin_actions(),
            loaded: false,
            loaded_from_disk: false,
        }
    }

    /// Loads `/data/loom/actions/*.toml`, falling back to [`builtin_actions()`] if empty or absent.
    pub fn load(&mut self, window: &mut Window) {
        self.loaded = true;
        let Ok(list) = files::list(window, &at(ACTIONS_DIR)) else {
            self.actions = builtin_actions();
            self.loaded_from_disk = false;
            return;
        };

        let mut parsed = Vec::new();
        for entry in list {
            if entry.dir || !entry.name.ends_with(b".toml") {
                continue;
            }
            if entry.size == 0 || entry.size > MAX_ACTION_FILE_BYTES as u64 {
                continue;
            }
            let path = crate::join(ACTIONS_DIR, &entry.name);
            let mut raw = vec![0u8; entry.size as usize];
            let Ok(n) = files::read(window, &at(&path), &mut raw) else {
                continue;
            };
            raw.truncate(n);
            if let Ok(text) = core::str::from_utf8(&raw) {
                if let Ok(mut items) = parse_actions(text) {
                    parsed.append(&mut items);
                }
            }
        }

        if parsed.is_empty() {
            self.actions = builtin_actions();
            self.loaded_from_disk = false;
        } else {
            self.actions = parsed;
            self.loaded_from_disk = true;
        }
    }

    /// Returns all enabled actions matching the given `(path, is_dir)` selection.
    pub fn matching(&self, entries: &[(&str, bool)]) -> Vec<UserAction> {
        self.actions
            .iter()
            .filter(|a| a.matches_selection(entries))
            .cloned()
            .collect()
    }

    /// Plans a scoped Collar launch (`@/path` grants + `+net` flag) for `action`.
    pub fn plan(
        action: &UserAction,
        selected_paths: &[&str],
    ) -> Result<CollarLaunchPlan, ActionError> {
        plan_collar_launch(action, selected_paths)
    }

    /// Converts a planned program path (`/bin/foo`) into a SexOS `@/bin/foo` grant token.
    pub fn program_token(program: &str) -> Vec<u8> {
        if program.starts_with('@') {
            program.as_bytes().to_vec()
        } else {
            let mut out = Vec::with_capacity(program.len() + 1);
            out.push(b'@');
            out.extend_from_slice(program.as_bytes());
            out
        }
    }

    /// Formats `plan.args` into a space-delimited argument buffer bounded by `ARGS_MAX`.
    pub fn args_line(plan: &CollarLaunchPlan) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        for (i, arg) in plan.args.iter().enumerate() {
            if i > 0 {
                out.push(b' ');
            }
            out.extend_from_slice(arg.as_bytes());
        }
        if out.len() > sex_pdx::fs::ARGS_MAX {
            None
        } else {
            Some(out)
        }
    }
}

/// Interactive context-action palette for the selected entry (`Ctrl+Shift+O`).
pub struct Palette {
    pub path: Vec<u8>,
    pub is_dir: bool,
    pub matches: Vec<UserAction>,
    pub cursor: usize,
}

impl Palette {
    pub fn render(&self) -> String {
        let kind = if self.is_dir { "folder" } else { "file" };
        let mut out = format!(
            "Context Actions for {} ({}, {} available)\n\n",
            shown(&self.path),
            kind,
            self.matches.len()
        );
        if self.matches.is_empty() {
            out.push_str("  (no matching actions for this selection)\n");
        } else {
            for (i, act) in self.matches.iter().enumerate() {
                let mark = if i == self.cursor { "> " } else { "  " };
                let net = if act.needs_net { " [+net]" } else { "" };
                out.push_str(&format!(
                    "{}[{}] {}{}  ({})\n",
                    mark, act.category, act.name, net, act.program
                ));
            }
            if let Some(selected) = self.matches.get(self.cursor) {
                if let Ok(path_str) = core::str::from_utf8(&self.path) {
                    out.push('\n');
                    match Registry::plan(selected, &[path_str]) {
                        Ok(plan) => {
                            out.push_str(&format!(
                                "Collar launch: {} {}\nGrants: {}\n",
                                plan.program,
                                plan.args.join(" "),
                                plan.collar_tokens().join(" ")
                            ));
                        }
                        Err(err) => {
                            out.push_str(&format!("Plan blocked: {}\n", action_error_label(err)));
                        }
                    }
                }
            }
        }
        out.push_str("\nUp/Down select  Enter run with scoped Collar grants  Esc back\n");
        out
    }
}

/// Read-only `FsProbe` backed by `sex_app::files::stat` for `UndoJournal` conflict pre-checks.
pub struct WindowFsProbe<'a>(pub &'a mut Window);

impl FsProbe for WindowFsProbe<'_> {
    fn stat(&mut self, path: &[u8]) -> Option<PathMeta> {
        let st = files::stat(self.0, &at(path)).ok()?;
        Some(PathMeta {
            exists: true,
            is_dir: st.dir,
            size: st.size,
            mtime: st.mtime,
        })
    }
}

pub fn action_error_label(err: ActionError) -> &'static str {
    match err {
        ActionError::ActionDisabled => "action is disabled",
        ActionError::EmptySelection => "nothing selected",
        ActionError::ArityMismatch => "selection count mismatch",
        ActionError::InvalidPath => "invalid path",
        ActionError::PathTraversal => "path traversal rejected",
        ActionError::ShellMetacharacter => "unsafe shell character rejected",
        ActionError::RootGrantForbidden => "root grant forbidden",
        ActionError::InvalidProgram => "invalid program path",
        ActionError::InvalidTemplate => "invalid argument template",
        ActionError::InvalidOutputExtension => "invalid output extension",
        ActionError::UnsupportedExtension => "unsupported file extension",
        ActionError::MismatchedParentDir => "items are not in the same folder",
    }
}

pub fn format_conflicts(plan: &UndoPlan) -> String {
    let mut parts = Vec::new();
    for c in plan.conflicts.iter().take(3) {
        let kind = match c.kind {
            ConflictKind::DestinationExists => "already exists",
            ConflictKind::SourceMissing => "missing",
            ConflictKind::ModifiedSinceRecord => "modified since recorded",
            ConflictKind::KindMismatch => "type changed",
        };
        parts.push(format!("{} ({})", shown(base(&c.path)), kind));
    }
    parts.join(", ")
}

pub fn format_transfer_eta(tp: &TransferProgress) -> String {
    match tp.eta_ms {
        Some(ms) if ms >= 1000 => format!("ETA {}s", (ms + 500) / 1000),
        Some(ms) if ms > 0 => format!("ETA {}ms", ms),
        _ => String::new(),
    }
}

impl App {
    /// Opens the Omanta context-action palette (`Ctrl+Shift+O`) for the selected entry.
    pub(crate) fn actions_open(&mut self, window: &mut Window) {
        if !self.loom.actions.loaded {
            self.loom.actions.load(window);
        }
        let Some(entry) = self.entries.get(self.cursor) else {
            self.status = "Select a file or folder first.".into();
            return;
        };
        let path = crate::join(&self.cwd, &entry.name);
        let Ok(path_str) = core::str::from_utf8(&path) else {
            self.status = "Action paths must be valid UTF-8.".into();
            return;
        };
        let matches = self.loom.actions.matching(&[(path_str, entry.dir)]);
        self.loom.mode = crate::loom::Mode::Actions(Palette {
            path,
            is_dir: entry.dir,
            matches,
            cursor: 0,
        });
    }

    pub(crate) fn actions_key(&mut self, _window: &mut Window, key: sex_pdx::app_key::Key) {
        let crate::loom::Mode::Actions(pal) = &mut self.loom.mode else {
            return;
        };
        match (key.code, sex_app::nav(key)) {
            (ESC, _) => {
                self.loom.mode = crate::loom::Mode::None;
                self.status.clear();
            }
            (_, Some(sex_app::Nav::Up)) => {
                pal.cursor = pal.cursor.saturating_sub(1);
            }
            (_, Some(sex_app::Nav::Down)) => {
                if !pal.matches.is_empty() {
                    pal.cursor = (pal.cursor + 1).min(pal.matches.len() - 1);
                }
            }
            (ENTER, _) => {
                let path = pal.path.clone();
                let action = pal.matches.get(pal.cursor).cloned();
                self.loom.mode = crate::loom::Mode::None;
                if let Some(act) = action {
                    self.launch_planned_action(&act, &path);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn actions_render(&self) -> Option<String> {
        let crate::loom::Mode::Actions(pal) = &self.loom.mode else {
            return None;
        };
        Some(pal.render())
    }

    pub(crate) fn launch_planned_action(&mut self, action: &UserAction, path: &[u8]) {
        let Ok(path_str) = core::str::from_utf8(path) else {
            self.status = "Action path is not valid UTF-8.".into();
            return;
        };
        let plan = match Registry::plan(action, &[path_str]) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Can't run {}: {}.", action.name, action_error_label(e));
                return;
            }
        };
        let Some(args) = Registry::args_line(&plan) else {
            self.status = format!("{} arguments exceed command limit.", action.name);
            return;
        };
        let prog = Registry::program_token(&plan.program);
        let cookie = ACTION_COOKIES.fetch_add(1, Ordering::Relaxed);
        match sex_app::launch::run_with(&prog, &args, plan.needs_net, false, cookie) {
            Ok(()) => {
                sex_pdx::serial_record!(
                    "[files.app.action.launch] id={} program={} net={} grants={}",
                    plan.action_id,
                    plan.program,
                    plan.needs_net as u8,
                    plan.grants.len()
                );
                self.status = format!("Started {} ({}).", action.name, plan.program);
                self.opening = Some((cookie, prog));
            }
            Err(e) => {
                sex_pdx::serial_record!(
                    "[files.app.action.failed] id={} program={} error={:?}",
                    plan.action_id,
                    plan.program,
                    e
                );
                self.status = format!("Couldn't start {}: {}.", action.name, plan.program);
            }
        }
    }

    pub(crate) fn journal_record(&mut self, op: FileOp) {
        self.loom.journal.record(op, sex_pdx::get_ticks());
    }

    pub(crate) fn journal_check_undo(&mut self, window: &mut Window) -> Result<Option<u64>, String> {
        let plan = {
            let mut probe = WindowFsProbe(window);
            self.loom.journal.plan_undo(&mut probe)
        };
        let Some(plan) = plan else {
            return Ok(None);
        };
        if !plan.is_clean() {
            return Err(format!(
                "Undo blocked by conflict: {}.",
                format_conflicts(&plan)
            ));
        }
        Ok(Some(plan.entry_id))
    }

    pub(crate) fn journal_redo(&mut self, window: &mut Window) {
        let plan = {
            let mut probe = WindowFsProbe(window);
            self.loom.journal.plan_redo(&mut probe)
        };
        let Some(plan) = plan else {
            self.status = "Nothing to redo.".into();
            return;
        };
        if !plan.is_clean() {
            self.status = format!("Redo blocked by conflict: {}.", format_conflicts(&plan));
            return;
        }
        for act in &plan.actions {
            let res = match act {
                PlannedAction::Move { from, to } => self.move_path(window, from, to),
                PlannedAction::CreateDir { path } => files::mkdir(window, &at(path)),
                PlannedAction::RemoveFileOrTree { path } => self.remove_all(window, path),
                PlannedAction::RemoveEmptyDir { path } => files::remove(window, &at(path)),
                PlannedAction::Copy { from, to } => {
                    let job = crate::jobs::Job::copy(window, from, to);
                    self.start(window, job, None, Some(base(to).to_vec()));
                    self.loom.journal.commit_redo(plan.entry_id);
                    return;
                }
                PlannedAction::ExtractArchive { .. } => {
                    self.status = "Redo archive extract is not supported directly.".into();
                    return;
                }
            };
            if let Err(e) = res {
                self.status = format!("Couldn't redo: {}.", shown(files::message(e)));
                return;
            }
        }
        self.loom.journal.commit_redo(plan.entry_id);
        let _ = files::sync(window);
        self.load(window);
        self.status = "Redone.".into();
    }
}
