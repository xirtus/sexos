// SPDX-License-Identifier: GPL-3.0-only
//! Omanta-inspired declarative custom right-click context actions and Collar capability grant planner.
//!
//! Action definitions live in simple TOML/INI-style text files under `/data/loom/actions/*.toml`
//! and are parsed without any external parser crate. Before launching an action,
//! [`plan_collar_launch`] expands placeholders (`@file`, `@files`, `@dir`, `@stem`, `@out_ext(...)`)
//! into explicit, canonicalized Collar `@/path` file grants and an optional `+net` flag — never
//! granting ambient authority and rejecting shell metacharacters or path traversal.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// How many selected items an action requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectionArity {
    #[default]
    Single,
    Multiple,
    Any,
}

/// One declarative custom right-click context action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAction {
    pub id: String,
    pub name: String,
    pub category: String,
    pub program: String,
    pub args_template: Vec<String>,
    /// Lowercase file extensions without a leading dot; empty matches any file.
    pub extensions: Vec<String>,
    pub allow_dirs: bool,
    pub selection_arity: SelectionArity,
    pub needs_net: bool,
    pub enabled: bool,
}

impl UserAction {
    /// Returns true if this action applies to the given `(path, is_dir)` selection.
    pub fn matches_selection(&self, entries: &[(&str, bool)]) -> bool {
        if !self.enabled || !arity_matches(self.selection_arity, entries.len()) {
            return false;
        }
        for &(path, is_dir) in entries {
            if is_dir {
                if !self.allow_dirs {
                    return false;
                }
            } else if !self.extensions.is_empty() && !extension_matches(&self.extensions, path) {
                return false;
            }
        }
        true
    }
}

/// Error returned when parsing an action definition file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: &'static str,
}

impl ParseError {
    const fn new(line: usize, message: &'static str) -> Self {
        Self { line, message }
    }
}

/// Error returned when planning a Collar-confined action launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    ActionDisabled,
    EmptySelection,
    ArityMismatch,
    InvalidPath,
    PathTraversal,
    ShellMetacharacter,
    RootGrantForbidden,
    InvalidProgram,
    InvalidTemplate,
    InvalidOutputExtension,
    UnsupportedExtension,
    MismatchedParentDir,
}

/// Explicit, least-privilege Collar launch specification produced by [`plan_collar_launch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollarLaunchPlan {
    pub action_id: String,
    pub program: String,
    pub args: Vec<String>,
    /// Deduplicated explicit `@/canonical/path` grants.
    pub grants: Vec<String>,
    pub needs_net: bool,
}

impl CollarLaunchPlan {
    /// Returns `Some("+net")` when network capability is requested, otherwise `None`.
    pub const fn net_flag(&self) -> Option<&'static str> {
        if self.needs_net {
            Some("+net")
        } else {
            None
        }
    }

    /// Returns all Collar grant tokens (`@/path` grants plus `+net` if required).
    pub fn collar_tokens(&self) -> Vec<String> {
        let mut tokens = self.grants.clone();
        if let Some(net) = self.net_flag() {
            tokens.push(String::from(net));
        }
        tokens
    }
}

/// Parses a single TOML/INI-style action definition file into a [`UserAction`].
///
/// Accepts optional `[action]` or `[action.<id>]` headers, `#` or `;` comments,
/// quoted or bare scalar values, and inline or multiline `[...]` string arrays.
pub fn parse_action(input: &str) -> Result<UserAction, ParseError> {
    let mut actions = parse_actions(input)?;
    match actions.len() {
        0 => Err(ParseError::new(1, "empty action definition")),
        1 => Ok(actions.remove(0)),
        _ => Err(ParseError::new(1, "multiple actions in single-action file")),
    }
}

/// Parses one or more TOML/INI-style action blocks (`[[action]]` or `[section]`) from `input`.
pub fn parse_actions(input: &str) -> Result<Vec<UserAction>, ParseError> {
    let mut out = Vec::new();
    let mut builder: Option<ActionBuilder> = None;
    let mut pending_array: Option<(usize, String, String)> = None;

    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        if let Some((start_line, key, mut buf)) = pending_array.take() {
            let stripped = strip_comment(raw_line).trim();
            if !stripped.is_empty() {
                buf.push(' ');
                buf.push_str(stripped);
            }
            if array_brackets_closed(&buf) {
                let items = parse_string_array(&buf, start_line)?;
                let b = builder.get_or_insert_with(ActionBuilder::default);
                b.set_array(&key, items, start_line)?;
            } else {
                pending_array = Some((start_line, key, buf));
            }
            continue;
        }

        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(section) = parse_section_header(line) {
            if let Some(b) = builder.take() {
                if !b.is_empty() {
                    out.push(b.finish(line_no)?);
                }
            }
            let mut next = ActionBuilder::default();
            if let Some(id_hint) = section.strip_prefix("action.").or_else(|| section.strip_prefix("actions.")) {
                let trimmed = id_hint.trim();
                if !trimmed.is_empty() {
                    next.id = Some(String::from(trimmed));
                }
            }
            builder = Some(next);
            continue;
        }

        let Some((raw_key, raw_val)) = split_key_value(line) else {
            return Err(ParseError::new(line_no, "expected key = value"));
        };
        let key = raw_key.trim().to_ascii_lowercase();
        let val = raw_val.trim();
        let b = builder.get_or_insert_with(ActionBuilder::default);

        if val.starts_with('[') && !array_brackets_closed(val) {
            pending_array = Some((line_no, key, String::from(val)));
            continue;
        }

        if val.starts_with('[') {
            let items = parse_string_array(val, line_no)?;
            b.set_array(&key, items, line_no)?;
        } else {
            let scalar = parse_scalar_string(val, line_no)?;
            b.set_scalar(&key, scalar, val, line_no)?;
        }
    }

    if let Some((start_line, _, _)) = pending_array {
        return Err(ParseError::new(start_line, "unterminated array"));
    }
    if let Some(b) = builder {
        if !b.is_empty() {
            out.push(b.finish(input.lines().count().max(1))?);
        }
    }
    Ok(out)
}

/// Validates and expands `action` for `selected_paths` into an explicit Collar launch plan.
pub fn plan_collar_launch(
    action: &UserAction,
    selected_paths: &[&str],
) -> Result<CollarLaunchPlan, ActionError> {
    if !action.enabled {
        return Err(ActionError::ActionDisabled);
    }
    if selected_paths.is_empty() {
        return Err(ActionError::EmptySelection);
    }
    if !arity_matches(action.selection_arity, selected_paths.len()) {
        return Err(ActionError::ArityMismatch);
    }

    let program = normalize_program(&action.program)?;
    let mut canonical_paths = Vec::with_capacity(selected_paths.len());
    for &raw in selected_paths {
        let canon = canonicalize_collar_path(raw)?;
        if !action.extensions.is_empty() {
            let has_ext = file_extension(&canon).is_some();
            if (has_ext || !action.allow_dirs) && !extension_matches(&action.extensions, &canon) {
                return Err(ActionError::UnsupportedExtension);
            }
        }
        canonical_paths.push(canon);
    }

    let mut args = Vec::new();
    let mut grants = Vec::new();

    for tpl in &action.args_template {
        expand_template_token(tpl, &canonical_paths, &mut args, &mut grants)?;
    }

    Ok(CollarLaunchPlan {
        action_id: action.id.clone(),
        program,
        args,
        grants,
        needs_net: action.needs_net,
    })
}

/// Built-in default toggleable context actions shipped by Loom.
pub fn builtin_actions() -> Vec<UserAction> {
    vec![
        UserAction {
            id: String::from("send-to-cabaret"),
            name: String::from("Send to Cabaret Studio"),
            category: String::from("Media"),
            program: String::from("/bin/cabaret"),
            args_template: vec![String::from("--import"), String::from("@files")],
            extensions: vec_strings(&[
                "mp4", "mkv", "mov", "webm", "avi", "wav", "flac", "mp3", "ogg", "png", "jpg",
            ]),
            allow_dirs: false,
            selection_arity: SelectionArity::Any,
            needs_net: false,
            enabled: true,
        },
        UserAction {
            id: String::from("open-in-splicy"),
            name: String::from("Open in Splicy"),
            category: String::from("Video"),
            program: String::from("/bin/splicy"),
            args_template: vec![String::from("@files")],
            extensions: vec_strings(&[
                "mp4", "mkv", "mov", "webm", "avi", "wav", "flac", "mp3", "splicy",
            ]),
            allow_dirs: false,
            selection_arity: SelectionArity::Any,
            needs_net: false,
            enabled: true,
        },
        UserAction {
            id: String::from("polish-voice-serenade"),
            name: String::from("Polish Voice with Serenade"),
            category: String::from("Audio"),
            program: String::from("/bin/serenade"),
            args_template: vec![
                String::from("--polish-voice"),
                String::from("@file"),
                String::from("--out"),
                String::from("@out_ext(wav)"),
            ],
            extensions: vec_strings(&["wav", "flac", "mp3", "ogg", "opus", "m4a", "aac"]),
            allow_dirs: false,
            selection_arity: SelectionArity::Single,
            needs_net: false,
            enabled: true,
        },
        UserAction {
            id: String::from("convert-image-gaze"),
            name: String::from("Convert Image in Gaze"),
            category: String::from("Image"),
            program: String::from("/bin/gaze"),
            args_template: vec![
                String::from("--convert"),
                String::from("@file"),
                String::from("--out"),
                String::from("@out_ext(png)"),
            ],
            extensions: vec_strings(&["png", "jpg", "jpeg", "webp", "gif", "bmp"]),
            allow_dirs: false,
            selection_arity: SelectionArity::Single,
            needs_net: false,
            enabled: true,
        },
        UserAction {
            id: String::from("extract-archive"),
            name: String::from("Extract Archive"),
            category: String::from("Archives"),
            program: String::from("/bin/archive"),
            args_template: vec![
                String::from("--extract"),
                String::from("@file"),
                String::from("--dest"),
                String::from("@dir"),
            ],
            extensions: vec_strings(&["zip", "tar", "gz", "tgz", "zst", "xz"]),
            allow_dirs: false,
            selection_arity: SelectionArity::Single,
            needs_net: false,
            enabled: true,
        },
        UserAction {
            id: String::from("send-to-phone-hookup"),
            name: String::from("Send to Phone via Hookup"),
            category: String::from("Share"),
            program: String::from("/bin/hookup"),
            args_template: vec![String::from("--send"), String::from("@files")],
            extensions: Vec::new(),
            allow_dirs: true,
            selection_arity: SelectionArity::Any,
            needs_net: true,
            enabled: true,
        },
    ]
}

fn vec_strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| String::from(*s)).collect()
}

fn arity_matches(arity: SelectionArity, count: usize) -> bool {
    match arity {
        SelectionArity::Single => count == 1,
        SelectionArity::Multiple => count >= 2,
        SelectionArity::Any => count >= 1,
    }
}

fn file_extension(path: &str) -> Option<&str> {
    let base = path.rsplit('/').next()?;
    let dot = base.rfind('.')?;
    if dot == 0 || dot + 1 >= base.len() {
        return None;
    }
    Some(&base[dot + 1..])
}

fn file_stem(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rfind('.') {
        Some(dot) if dot > 0 => &base[..dot],
        _ => base,
    }
}

fn parent_dir(path: &str) -> Result<&str, ActionError> {
    let idx = path.rfind('/').ok_or(ActionError::InvalidPath)?;
    if idx == 0 {
        return Err(ActionError::RootGrantForbidden);
    }
    Ok(&path[..idx])
}

fn extension_matches(allowed: &[String], path: &str) -> bool {
    let Some(ext) = file_extension(path) else {
        return false;
    };
    let lower = ext.to_ascii_lowercase();
    allowed.iter().any(|candidate| candidate == &lower)
}

fn is_shell_metachar(b: u8) -> bool {
    matches!(
        b,
        b';' | b'&'
            | b'|'
            | b'$'
            | b'`'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'<'
            | b'>'
            | b'*'
            | b'?'
            | b'!'
            | b'\\'
            | b'\''
            | b'"'
            | b'~'
            | b'#'
            | b'\n'
            | b'\r'
            | b'\t'
    )
}

fn has_unsafe_chars(s: &str, allow_space: bool) -> Result<(), ActionError> {
    for b in s.bytes() {
        if b == 0 || b < 0x20 || b == 0x7f {
            return Err(ActionError::InvalidPath);
        }
        if !allow_space && b == b' ' {
            return Err(ActionError::ShellMetacharacter);
        }
        if is_shell_metachar(b) {
            return Err(ActionError::ShellMetacharacter);
        }
    }
    Ok(())
}

fn canonicalize_collar_path(raw: &str) -> Result<String, ActionError> {
    if raw.is_empty() || !raw.starts_with('/') {
        return Err(ActionError::InvalidPath);
    }
    if raw.contains('@') {
        return Err(ActionError::InvalidPath);
    }
    has_unsafe_chars(raw, false)?;

    let mut segments = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(ActionError::PathTraversal);
        }
        segments.push(seg);
    }
    if segments.is_empty() {
        return Err(ActionError::RootGrantForbidden);
    }
    let mut out = String::new();
    for seg in segments {
        out.push('/');
        out.push_str(seg);
    }
    Ok(out)
}

fn normalize_program(raw: &str) -> Result<String, ActionError> {
    if raw.is_empty() || raw.contains('@') {
        return Err(ActionError::InvalidProgram);
    }
    has_unsafe_chars(raw, false).map_err(|e| match e {
        ActionError::ShellMetacharacter => ActionError::ShellMetacharacter,
        _ => ActionError::InvalidProgram,
    })?;
    if raw.starts_with('/') {
        let canon = canonicalize_collar_path(raw).map_err(|e| match e {
            ActionError::PathTraversal => ActionError::PathTraversal,
            ActionError::ShellMetacharacter => ActionError::ShellMetacharacter,
            _ => ActionError::InvalidProgram,
        })?;
        Ok(canon)
    } else {
        if raw == "." || raw == ".." || raw.contains('/') {
            return Err(ActionError::InvalidProgram);
        }
        Ok(format!("/bin/{raw}"))
    }
}

fn push_unique_grant(grants: &mut Vec<String>, path: &str) {
    let token = format!("@{path}");
    if !grants.iter().any(|g| g == &token) {
        grants.push(token);
    }
}

fn expand_template_token(
    tpl: &str,
    paths: &[String],
    args: &mut Vec<String>,
    grants: &mut Vec<String>,
) -> Result<(), ActionError> {
    if tpl.is_empty() {
        return Err(ActionError::InvalidTemplate);
    }
    if tpl == "@files" {
        for p in paths {
            push_unique_grant(grants, p);
            args.push(format!("@{p}"));
        }
        return Ok(());
    }

    if tpl.contains("@files") {
        return Err(ActionError::InvalidTemplate);
    }

    let mut out = String::new();
    let mut rest = tpl;

    while let Some(pos) = rest.find('@') {
        let prefix = &rest[..pos];
        has_unsafe_chars(prefix, false)?;
        out.push_str(prefix);
        let after_at = &rest[pos..];

        if let Some(tail) = after_at.strip_prefix("@file") {
            if !tail.is_empty() && tail.as_bytes()[0].is_ascii_alphanumeric() {
                return Err(ActionError::InvalidTemplate);
            }
            if paths.len() != 1 {
                return Err(ActionError::ArityMismatch);
            }
            let p = &paths[0];
            push_unique_grant(grants, p);
            out.push('@');
            out.push_str(p);
            rest = tail;
        } else if let Some(tail) = after_at.strip_prefix("@dir") {
            if !tail.is_empty() && tail.as_bytes()[0].is_ascii_alphanumeric() {
                return Err(ActionError::InvalidTemplate);
            }
            let dir = shared_parent_dir(paths)?;
            push_unique_grant(grants, dir);
            out.push('@');
            out.push_str(dir);
            rest = tail;
        } else if let Some(tail) = after_at.strip_prefix("@stem") {
            if !tail.is_empty() && tail.as_bytes()[0].is_ascii_alphanumeric() {
                return Err(ActionError::InvalidTemplate);
            }
            if paths.len() != 1 {
                return Err(ActionError::ArityMismatch);
            }
            out.push_str(file_stem(&paths[0]));
            rest = tail;
        } else if let Some(inside) = after_at.strip_prefix("@out_ext(") {
            let Some(close) = inside.find(')') else {
                return Err(ActionError::InvalidOutputExtension);
            };
            let ext = inside[..close].trim();
            if ext.is_empty()
                || ext.len() > 16
                || !ext.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(ActionError::InvalidOutputExtension);
            }
            if paths.len() != 1 {
                return Err(ActionError::ArityMismatch);
            }
            let out_path = replace_extension(&paths[0], ext)?;
            push_unique_grant(grants, &out_path);
            out.push('@');
            out.push_str(&out_path);
            rest = &inside[close + 1..];
        } else {
            return Err(ActionError::InvalidTemplate);
        }
    }

    has_unsafe_chars(rest, false)?;
    out.push_str(rest);
    args.push(out);
    Ok(())
}

fn shared_parent_dir(paths: &[String]) -> Result<&str, ActionError> {
    let first = parent_dir(&paths[0])?;
    for p in &paths[1..] {
        if parent_dir(p)? != first {
            return Err(ActionError::MismatchedParentDir);
        }
    }
    Ok(first)
}

fn replace_extension(path: &str, new_ext: &str) -> Result<String, ActionError> {
    let slash = path.rfind('/').ok_or(ActionError::InvalidPath)?;
    let dir = &path[..slash];
    let base = &path[slash + 1..];
    let stem = match base.rfind('.') {
        Some(dot) if dot > 0 => &base[..dot],
        _ => base,
    };
    if dir.is_empty() {
        return Err(ActionError::RootGrantForbidden);
    }
    Ok(format!("{dir}/{stem}.{new_ext}"))
}

#[derive(Default)]
struct ActionBuilder {
    id: Option<String>,
    name: Option<String>,
    category: Option<String>,
    program: Option<String>,
    args_template: Option<Vec<String>>,
    extensions: Option<Vec<String>>,
    allow_dirs: Option<bool>,
    selection_arity: Option<SelectionArity>,
    needs_net: Option<bool>,
    enabled: Option<bool>,
}

impl ActionBuilder {
    fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.name.is_none()
            && self.category.is_none()
            && self.program.is_none()
            && self.args_template.is_none()
            && self.extensions.is_none()
            && self.allow_dirs.is_none()
            && self.selection_arity.is_none()
            && self.needs_net.is_none()
            && self.enabled.is_none()
    }

    fn set_array(&mut self, key: &str, items: Vec<String>, line: usize) -> Result<(), ParseError> {
        match key {
            "args_template" | "args" => self.args_template = Some(items),
            "extensions" | "ext" => {
                self.extensions = Some(normalize_extensions(items, line)?);
            }
            _ => return Err(ParseError::new(line, "unexpected array for key")),
        }
        Ok(())
    }

    fn set_scalar(
        &mut self,
        key: &str,
        scalar: String,
        raw_val: &str,
        line: usize,
    ) -> Result<(), ParseError> {
        match key {
            "id" => self.id = Some(scalar),
            "name" => self.name = Some(scalar),
            "category" => self.category = Some(scalar),
            "program" | "exec" => self.program = Some(scalar),
            "args_template" | "args" => {
                let parts = scalar
                    .split_ascii_whitespace()
                    .map(String::from)
                    .collect();
                self.args_template = Some(parts);
            }
            "extensions" | "ext" => {
                let items: Vec<String> = scalar
                    .split(',')
                    .map(|s| String::from(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
                self.extensions = Some(normalize_extensions(items, line)?);
            }
            "allow_dirs" => self.allow_dirs = Some(parse_bool(raw_val, line)?),
            "selection_arity" | "arity" => {
                self.selection_arity = Some(parse_arity(&scalar, line)?);
            }
            "needs_net" | "net" => self.needs_net = Some(parse_bool(raw_val, line)?),
            "enabled" => self.enabled = Some(parse_bool(raw_val, line)?),
            _ => return Err(ParseError::new(line, "unknown action key")),
        }
        Ok(())
    }

    fn finish(self, line: usize) -> Result<UserAction, ParseError> {
        let id = self.id.filter(|s| !s.is_empty()).ok_or(ParseError::new(line, "missing id"))?;
        let name = self.name.filter(|s| !s.is_empty()).ok_or(ParseError::new(line, "missing name"))?;
        let program = self.program.filter(|s| !s.is_empty()).ok_or(ParseError::new(line, "missing program"))?;
        Ok(UserAction {
            id,
            name,
            category: self.category.unwrap_or_else(|| String::from("General")),
            program,
            args_template: self.args_template.unwrap_or_default(),
            extensions: self.extensions.unwrap_or_default(),
            allow_dirs: self.allow_dirs.unwrap_or(false),
            selection_arity: self.selection_arity.unwrap_or(SelectionArity::Single),
            needs_net: self.needs_net.unwrap_or(false),
            enabled: self.enabled.unwrap_or(true),
        })
    }
}

fn normalize_extensions(items: Vec<String>, line: usize) -> Result<Vec<String>, ParseError> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let trimmed = item.trim().trim_start_matches('.');
        if trimmed.is_empty() {
            return Err(ParseError::new(line, "empty extension"));
        }
        let lower = trimmed.to_ascii_lowercase();
        if !out.iter().any(|e| e == &lower) {
            out.push(lower);
        }
    }
    Ok(out)
}

fn parse_bool(raw: &str, line: usize) -> Result<bool, ParseError> {
    match raw.trim().trim_matches('"').trim_matches('\'').to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ParseError::new(line, "invalid boolean value")),
    }
}

fn parse_arity(raw: &str, line: usize) -> Result<SelectionArity, ParseError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "single" => Ok(SelectionArity::Single),
        "multiple" | "multi" => Ok(SelectionArity::Multiple),
        "any" => Ok(SelectionArity::Any),
        _ => Err(ParseError::new(line, "invalid selection_arity")),
    }
}

fn parse_section_header(line: &str) -> Option<&str> {
    if line.starts_with("[[") && line.ends_with("]]") && line.len() >= 4 {
        Some(line[2..line.len() - 2].trim())
    } else if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
        Some(line[1..line.len() - 1].trim())
    } else {
        None
    }
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    line.split_once('=')
}

fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                escape = true;
            } else if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'#' || b == b';' {
            return &line[..i];
        }
    }
    line
}

fn array_brackets_closed(s: &str) -> bool {
    let mut quote: Option<u8> = None;
    let mut escape = false;
    let mut depth = 0i32;
    for b in s.bytes() {
        if escape {
            escape = false;
            continue;
        }
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                escape = true;
            } else if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth <= 0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

fn parse_string_array(raw: &str, line: usize) -> Result<Vec<String>, ParseError> {
    let trimmed = raw.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(ParseError::new(line, "malformed array"));
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let (s, next) = parse_quoted(&inner[i..], line)?;
            items.push(s);
            i += next;
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' && bytes[i] != b']' {
                i += 1;
            }
            let token = inner[start..i].trim();
            if !token.is_empty() {
                items.push(token.to_string());
            }
        }
    }
    Ok(items)
}

fn parse_scalar_string(raw: &str, line: usize) -> Result<String, ParseError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        let (s, used) = parse_quoted(trimmed, line)?;
        if !trimmed[used..].trim().is_empty() {
            return Err(ParseError::new(line, "trailing characters after string"));
        }
        Ok(s)
    } else {
        Ok(String::from(trimmed))
    }
}

fn parse_quoted(s: &str, line: usize) -> Result<(String, usize), ParseError> {
    let quote = s.as_bytes()[0];
    let mut out = String::new();
    let mut escape = false;
    for (idx, ch) in s[1..].char_indices() {
        if escape {
            let resolved = match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '\\' => '\\',
                '"' => '"',
                '\'' => '\'',
                _ => return Err(ParseError::new(line, "unknown escape sequence")),
            };
            out.push(resolved);
            escape = false;
            continue;
        }
        if ch == '\\' && quote == b'"' {
            escape = true;
            continue;
        }
        if ch as u32 == quote as u32 {
            return Ok((out, 1 + idx + ch.len_utf8()));
        }
        out.push(ch);
    }
    Err(ParseError::new(line, "unterminated string literal"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_and_ini_action_files() {
        let toml = r#"
# Custom action for Gaze
[action]
id = "resize-1080p"
name = "Resize to 1080p"
category = "Image"
program = "gaze"
args_template = [
  "--resize",
  "1920x1080",
  "@file",
  "--out",
  "@out_ext(webp)",
]
extensions = [".PNG", "jpg", "jpeg"]
allow_dirs = false
selection_arity = "single"
needs_net = false
enabled = true
"#;
        let parsed = parse_action(toml).unwrap();
        assert_eq!(parsed.id, "resize-1080p");
        assert_eq!(parsed.name, "Resize to 1080p");
        assert_eq!(parsed.category, "Image");
        assert_eq!(parsed.program, "gaze");
        assert_eq!(parsed.extensions, vec!["png", "jpg", "jpeg"]);
        assert_eq!(parsed.selection_arity, SelectionArity::Single);
        assert!(!parsed.allow_dirs);
        assert!(!parsed.needs_net);
        assert!(parsed.enabled);

        let plan = plan_collar_launch(&parsed, &["/home/user//pics/./shot.PNG"]).unwrap();
        assert_eq!(plan.program, "/bin/gaze");
        assert_eq!(
            plan.args,
            vec![
                "--resize",
                "1920x1080",
                "@/home/user/pics/shot.PNG",
                "--out",
                "@/home/user/pics/shot.webp",
            ]
        );
        assert_eq!(
            plan.grants,
            vec![
                "@/home/user/pics/shot.PNG",
                "@/home/user/pics/shot.webp",
            ]
        );
        assert_eq!(plan.net_flag(), None);
    }

    #[test]
    fn parses_multiple_actions_and_ini_style() {
        let doc = r#"
[action.sync_phone]
name = Send to Phone
program = /bin/hookup
args_template = "--send @files"
allow_dirs = true
selection_arity = any
needs_net = true

[[action]]
id = merge_clips
name = Merge Clips
program = splicy
args_template = ["--merge", "@files"]
selection_arity = multiple
"#;
        let actions = parse_actions(doc).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].id, "sync_phone");
        assert!(actions[0].needs_net);
        assert_eq!(actions[0].selection_arity, SelectionArity::Any);

        let plan = plan_collar_launch(
            &actions[0],
            &["/home/user/video.mkv", "/home/user/notes"],
        )
        .unwrap();
        assert_eq!(
            plan.grants,
            vec!["@/home/user/video.mkv", "@/home/user/notes"]
        );
        assert_eq!(plan.net_flag(), Some("+net"));
        assert_eq!(
            plan.collar_tokens(),
            vec!["@/home/user/video.mkv", "@/home/user/notes", "+net"]
        );

        assert_eq!(
            plan_collar_launch(&actions[1], &["/home/user/only_one.mkv"]),
            Err(ActionError::ArityMismatch)
        );
    }

    #[test]
    fn rejects_traversal_metacharacters_and_root_grants() {
        let builtins = builtin_actions();
        let convert = builtins
            .iter()
            .find(|a| a.id == "convert-image-gaze")
            .unwrap();
        assert_eq!(
            plan_collar_launch(convert, &["/home/user/../../etc/passwd.png"]),
            Err(ActionError::PathTraversal)
        );
        assert_eq!(
            plan_collar_launch(convert, &["/home/user/a;rm.png"]),
            Err(ActionError::ShellMetacharacter)
        );
        assert_eq!(
            plan_collar_launch(convert, &["/home/user/$(id).png"]),
            Err(ActionError::ShellMetacharacter)
        );
        assert_eq!(
            plan_collar_launch(convert, &["relative/a.png"]),
            Err(ActionError::InvalidPath)
        );
        assert_eq!(
            plan_collar_launch(convert, &["/home/user/notes.txt"]),
            Err(ActionError::UnsupportedExtension)
        );
        assert_eq!(
            plan_collar_launch(convert, &["/top.png"]),
            Err(ActionError::RootGrantForbidden)
        );

        let extract = builtins
            .iter()
            .find(|a| a.id == "extract-archive")
            .unwrap();
        assert_eq!(
            plan_collar_launch(extract, &["/archive.zip"]),
            Err(ActionError::RootGrantForbidden)
        );
        let ok_extract = plan_collar_launch(extract, &["/home/user/dl/pack.zip"]).unwrap();
        assert_eq!(
            ok_extract.args,
            vec![
                "--extract",
                "@/home/user/dl/pack.zip",
                "--dest",
                "@/home/user/dl",
            ]
        );
        assert_eq!(
            ok_extract.grants,
            vec!["@/home/user/dl/pack.zip", "@/home/user/dl"]
        );
    }

    #[test]
    fn builtin_actions_cover_expected_suite_and_disabled_check() {
        let mut actions = builtin_actions();
        assert_eq!(actions.len(), 6);
        let names: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Send to Cabaret Studio"));
        assert!(names.contains(&"Open in Splicy"));
        assert!(names.contains(&"Polish Voice with Serenade"));
        assert!(names.contains(&"Convert Image in Gaze"));
        assert!(names.contains(&"Extract Archive"));
        assert!(names.contains(&"Send to Phone via Hookup"));

        let cabaret = &mut actions[0];
        assert!(cabaret.matches_selection(&[("/home/u/v.mkv", false)]));
        assert!(!cabaret.matches_selection(&[("/home/u/folder", true)]));
        cabaret.enabled = false;
        assert_eq!(
            plan_collar_launch(cabaret, &["/home/u/v.mkv"]),
            Err(ActionError::ActionDisabled)
        );
    }

    #[test]
    fn supports_stem_and_deduplicates_grants() {
        let action = UserAction {
            id: String::from("stem-test"),
            name: String::from("Stem Test"),
            category: String::from("Tools"),
            program: String::from("serenade"),
            args_template: vec![
                String::from("--title"),
                String::from("@stem"),
                String::from("--in"),
                String::from("@file"),
                String::from("--again"),
                String::from("@file"),
            ],
            extensions: vec![String::from("wav")],
            allow_dirs: false,
            selection_arity: SelectionArity::Single,
            needs_net: false,
            enabled: true,
        };
        let plan = plan_collar_launch(&action, &["/home/u/voice.wav"]).unwrap();
        assert_eq!(
            plan.args,
            vec![
                "--title",
                "voice",
                "--in",
                "@/home/u/voice.wav",
                "--again",
                "@/home/u/voice.wav",
            ]
        );
        assert_eq!(plan.grants, vec!["@/home/u/voice.wav"]);
    }
}
