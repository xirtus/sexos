// SPDX-License-Identifier: GPL-3.0-only
//! Agent IPC: JSON-RPC 2.0 interface for AI assistants and automation tools.
//!
//! Adapted from Rook's `rook-ipc` / `docs/IPC.md` (GPL-3.0), Scarlet's `edit_flag` / `"N need you"`
//! review counter (MIT), and Foreplay's cutscene EDL split-edit bridge.
//!
//! Features:
//! - Completely self-contained `#![no_std]` recursive-descent JSON parser and serializer with
//!   strictly bounded recursion depth (<= 32) so untrusted input can never overflow the stack.
//! - JSON-RPC 2.0 newline-delimited command dispatcher handling timeline mutations, atomic
//!   batch execution with rollback, undo/redo, review flags, gap queries, pacing telemetry,
//!   transcript search, and Foreplay cutscene EDL generation.
//! - Bidirectional event push notifications (`project.changed`, `playhead.moved`,
//!   `selection.changed`, `flags.changed`) drained via [`AgentHost::drain_events`].

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write as _;

use crate::history::History;
use crate::render::{plan, Piece};
use crate::suggest::Suggestions;
use crate::transcript::{norm, Transcript};
use crate::{EditError, MediaKind, Micros, Op, Range, Timeline, TrackKind};

/// A self-contained `#![no_std]` JSON value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Parse JSON text with safe recursive descent bounded to <= 32 recursion levels.
    pub fn parse(input: &str) -> Result<Json, &'static str> {
        let mut parser = Parser::new(input);
        let val = parser.parse_value()?;
        parser.skip_ws();
        if parser.pos != parser.bytes.len() {
            return Err("trailing data after JSON value");
        }
        Ok(val)
    }

    /// Serialize this JSON value to a string with standard escaping.
    pub fn to_string(&self) -> String {
        let mut s = String::new();
        self.write_buf(&mut s);
        s
    }

    fn write_buf(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(n) => {
                let _ = write!(out, "{}", n);
            }
            Json::Str(s) => escape_str(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_buf(out);
                }
                out.push(']');
            }
            Json::Obj(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escape_str(k, out);
                    out.push(':');
                    v.write_buf(out);
                }
                out.push('}');
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Json::Int(n) if *n >= 0 && *n <= u32::MAX as i64 => Some(*n as u32),
            _ => None,
        }
    }

    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Json::Int(n) if *n >= 0 => Some(*n as usize),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(o) => Some(o.as_slice()),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

fn escape_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Recursive-descent parser tracking input position and recursion depth.
struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser { src, bytes: src.as_bytes(), pos: 0, depth: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = self.bytes.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn parse_value(&mut self) -> Result<Json, &'static str> {
        self.skip_ws();
        let b = self.peek().ok_or("unexpected end of input")?;
        match b {
            b'n' => self.parse_null(),
            b't' => self.parse_true(),
            b'f' => self.parse_false(),
            b'"' => self.parse_string().map(Json::Str),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err("unexpected character in JSON"),
        }
    }

    fn parse_null(&mut self) -> Result<Json, &'static str> {
        if self.src[self.pos..].starts_with("null") {
            self.pos += 4;
            Ok(Json::Null)
        } else {
            Err("expected 'null'")
        }
    }

    fn parse_true(&mut self) -> Result<Json, &'static str> {
        if self.src[self.pos..].starts_with("true") {
            self.pos += 4;
            Ok(Json::Bool(true))
        } else {
            Err("expected 'true'")
        }
    }

    fn parse_false(&mut self) -> Result<Json, &'static str> {
        if self.src[self.pos..].starts_with("false") {
            self.pos += 5;
            Ok(Json::Bool(false))
        } else {
            Err("expected 'false'")
        }
    }

    fn parse_string(&mut self) -> Result<String, &'static str> {
        if self.next_byte() != Some(b'"') {
            return Err("expected '\"'");
        }
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos] != b'"' && self.bytes[self.pos] != b'\\' {
                self.pos += 1;
            }
            if self.pos > start {
                out.push_str(&self.src[start..self.pos]);
            }
            if self.pos >= self.bytes.len() {
                return Err("unterminated string");
            }
            if self.bytes[self.pos] == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if self.bytes[self.pos] == b'\\' {
                self.pos += 1;
                let esc = self.next_byte().ok_or("unterminated escape")?;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\x08'),
                    b'f' => out.push('\x0C'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if self.pos + 4 > self.bytes.len() {
                            return Err("truncated unicode escape");
                        }
                        let hex = &self.src[self.pos..self.pos + 4];
                        self.pos += 4;
                        let code = u16::from_str_radix(hex, 16).map_err(|_| "invalid unicode escape")?;
                        if let Some(c) = char::from_u32(code as u32) {
                            out.push(c);
                        } else {
                            out.push('\u{FFFD}');
                        }
                    }
                    _ => return Err("invalid escape character"),
                }
            }
        }
        Err("unterminated string")
    }

    fn parse_number(&mut self) -> Result<Json, &'static str> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digit_start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == digit_start {
            return Err("expected digits in number");
        }
        let int_end = self.pos;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let frac_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == frac_start {
                return Err("expected digits after decimal dot");
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == exp_start {
                return Err("expected digits after exponent");
            }
        }
        let s = &self.src[start..int_end];
        let n = s.parse::<i64>().map_err(|_| "failed to parse integer")?;
        Ok(Json::Int(n))
    }

    fn parse_array(&mut self) -> Result<Json, &'static str> {
        if self.depth >= 32 {
            return Err("recursion limit exceeded");
        }
        if self.next_byte() != Some(b'[') {
            return Err("expected '['");
        }
        self.depth += 1;
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Json::Arr(Vec::new()));
        }
        let mut elements = Vec::new();
        loop {
            let val = self.parse_value()?;
            elements.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.depth -= 1;
                    return Err("expected ',' or ']' in array");
                }
            }
        }
        self.depth -= 1;
        Ok(Json::Arr(elements))
    }

    fn parse_object(&mut self) -> Result<Json, &'static str> {
        if self.depth >= 32 {
            return Err("recursion limit exceeded");
        }
        if self.next_byte() != Some(b'{') {
            return Err("expected '{'");
        }
        self.depth += 1;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Json::Obj(Vec::new()));
        }
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                self.depth -= 1;
                return Err("expected string key in object");
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.next_byte() != Some(b':') {
                self.depth -= 1;
                return Err("expected ':' after key in object");
            }
            let val = self.parse_value()?;
            pairs.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    self.depth -= 1;
                    return Err("expected ',' or '}' in object");
                }
            }
        }
        self.depth -= 1;
        Ok(Json::Obj(pairs))
    }
}

/// A human review doubt or flag left on the timeline (adapted from Scarlet's `edit_flag`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditFlag {
    pub id: u32,
    pub version: usize,
    pub at: Micros,
    pub reason: String,
    pub resolved: bool,
}

/// Host environment managing timeline state, version history, agent suggestions, and IPC dispatch.
pub struct AgentHost {
    pub history: History,
    pub transcript: Option<Transcript>,
    pub suggestions: Suggestions,
    pub playhead: Micros,
    pub selected_clips: Vec<u32>,
    pub flags: Vec<EditFlag>,
    pub events: Vec<String>,
    next_flag_id: u32,
}

impl AgentHost {
    pub fn new(timeline: Timeline) -> Self {
        AgentHost {
            history: History::new(timeline),
            transcript: None,
            suggestions: Suggestions::default(),
            playhead: 0,
            selected_clips: Vec::new(),
            flags: Vec::new(),
            events: Vec::new(),
            next_flag_id: 1,
        }
    }

    /// Count of unresolved review flags ("N need you" counter from Scarlet).
    pub fn unresolved_flag_count(&self) -> usize {
        self.flags.iter().filter(|f| !f.resolved).count()
    }

    /// Drain all queued outbound push notifications (clearing internal event buffer).
    pub fn drain_events(&mut self) -> Vec<String> {
        core::mem::take(&mut self.events)
    }

    /// Queue a JSON-RPC 2.0 notification (no `id`).
    pub fn push_event(&mut self, method: &str, params: Json) {
        let mut obj = Vec::new();
        obj.push(("jsonrpc".into(), Json::Str("2.0".into())));
        obj.push(("method".into(), Json::Str(method.into())));
        obj.push(("params".into(), params));
        self.events.push(Json::Obj(obj).to_string());
    }

    /// Handle one newline-delimited JSON-RPC 2.0 request string and return the response string.
    pub fn handle_line(&mut self, line: &str) -> String {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return format_error(Json::Null, -32700, "Parse error: empty request");
        }

        let parsed = match Json::parse(trimmed) {
            Ok(j) => j,
            Err(_) => return format_error(Json::Null, -32700, "Parse error: invalid JSON"),
        };

        let req_obj = match parsed {
            Json::Obj(ref pairs) => pairs,
            _ => return format_error(Json::Null, -32600, "Invalid Request: expected object"),
        };

        let id = req_obj.iter().find(|(k, _)| k == "id").map(|(_, v)| v.clone()).unwrap_or(Json::Null);

        let jsonrpc_val = req_obj.iter().find(|(k, _)| k == "jsonrpc").map(|(_, v)| v);
        match jsonrpc_val {
            Some(Json::Str(s)) if s == "2.0" => {}
            _ => return format_error(id, -32600, "Invalid Request: missing or invalid 'jsonrpc' version"),
        }

        let method = match req_obj.iter().find(|(k, _)| k == "method").and_then(|(_, v)| v.as_str()) {
            Some(m) => m,
            None => return format_error(id, -32600, "Invalid Request: missing 'method'"),
        };

        let params = req_obj.iter().find(|(k, _)| k == "params").map(|(_, v)| v);

        match self.dispatch_method(method, params) {
            Ok(result) => format_result(id, result),
            Err(RpcError { code, message }) => format_error(id, code, &message),
        }
    }

    fn dispatch_method(&mut self, method: &str, params: Option<&Json>) -> Result<Json, RpcError> {
        match method {
            "timeline.get" => self.rpc_timeline_get(),
            "timeline.insert_clip" => self.rpc_timeline_insert_clip(params),
            "timeline.remove_clip" => self.rpc_timeline_remove_clip(params),
            "timeline.trim_clip" => self.rpc_timeline_trim_clip(params),
            "timeline.move_clip" => self.rpc_timeline_move_clip(params),
            "timeline.split_clip" => self.rpc_timeline_split_clip(params),
            "timeline.ripple_delete" => self.rpc_timeline_ripple_delete(params),
            "timeline.set_speed" => self.rpc_timeline_set_speed(params),
            "timeline.set_volume" => self.rpc_timeline_set_volume(params),
            "timeline.add_marker" => self.rpc_timeline_add_marker(params),
            "timeline.set_playhead" => self.rpc_timeline_set_playhead(params),
            "timeline.set_selection" => self.rpc_timeline_set_selection(params),
            "batch.execute" => self.rpc_batch_execute(params),
            "undo.undo" => self.rpc_undo(),
            "undo.redo" => self.rpc_redo(),
            "undo.history" => self.rpc_undo_history(),
            "edit.flag" => self.rpc_edit_flag(params),
            "edit.resolve_flag" => self.rpc_edit_resolve_flag(params),
            "query.find_gaps" => self.rpc_query_find_gaps(),
            "query.analyze_pacing" => self.rpc_query_analyze_pacing(params),
            "query.search_transcript" => self.rpc_query_search_transcript(params),
            "foreplay.cutscene_edl" => self.rpc_foreplay_cutscene_edl(),
            _ => Err(RpcError::new(-32601, format!("Method '{}' not found", method))),
        }
    }

    fn rpc_timeline_get(&self) -> Result<Json, RpcError> {
        let tl = self.history.current();
        let mut obj = Vec::new();
        obj.push(("duration".into(), Json::Int(tl.duration())));
        obj.push(("fps_num".into(), Json::Int(tl.fps_num as i64)));
        obj.push(("fps_den".into(), Json::Int(tl.fps_den as i64)));
        obj.push(("playhead".into(), Json::Int(self.playhead)));
        obj.push(("version".into(), Json::Int(self.history.head() as i64)));
        obj.push(("needs_review_count".into(), Json::Int(self.unresolved_flag_count() as i64)));

        let mut tracks = Vec::new();
        for t in &tl.tracks {
            let mut t_obj = Vec::new();
            t_obj.push(("id".into(), Json::Int(t.id as i64)));
            let kind_str = match t.kind {
                TrackKind::Video => "video",
                TrackKind::Audio => "audio",
            };
            t_obj.push(("kind".into(), Json::Str(kind_str.into())));
            t_obj.push(("name".into(), Json::Str(t.name.clone())));
            t_obj.push(("muted".into(), Json::Bool(t.muted)));

            let mut clips = Vec::new();
            for c in &t.clips {
                let mut c_obj = Vec::new();
                c_obj.push(("id".into(), Json::Int(c.id as i64)));
                c_obj.push(("media".into(), Json::Int(c.media as i64)));
                c_obj.push(("at".into(), Json::Int(c.at)));
                c_obj.push(("duration".into(), Json::Int(c.duration())));
                c_obj.push(("source_start".into(), Json::Int(c.source.start)));
                c_obj.push(("source_end".into(), Json::Int(c.source.end)));
                c_obj.push(("speed".into(), Json::Int(c.speed as i64)));
                c_obj.push(("volume".into(), Json::Int(c.volume as i64)));
                c_obj.push(("enabled".into(), Json::Bool(c.enabled)));
                clips.push(Json::Obj(c_obj));
            }
            t_obj.push(("clips".into(), Json::Arr(clips)));
            tracks.push(Json::Obj(t_obj));
        }
        obj.push(("tracks".into(), Json::Arr(tracks)));

        let mut markers = Vec::new();
        for m in &tl.markers {
            let mut m_obj = Vec::new();
            m_obj.push(("at".into(), Json::Int(m.at)));
            m_obj.push(("label".into(), Json::Str(m.label.clone())));
            markers.push(Json::Obj(m_obj));
        }
        obj.push(("markers".into(), Json::Arr(markers)));

        let mut media_list = Vec::new();
        for m in &tl.media {
            let mut m_obj = Vec::new();
            m_obj.push(("id".into(), Json::Int(m.id as i64)));
            let kind_str = match m.kind {
                MediaKind::Video => "video",
                MediaKind::Audio => "audio",
                MediaKind::Image => "image",
            };
            m_obj.push(("kind".into(), Json::Str(kind_str.into())));
            m_obj.push(("path".into(), Json::Str(m.path.clone())));
            m_obj.push(("duration".into(), Json::Int(m.duration)));
            media_list.push(Json::Obj(m_obj));
        }
        obj.push(("media".into(), Json::Arr(media_list)));

        Ok(Json::Obj(obj))
    }

    fn rpc_timeline_insert_clip(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let track = p.get("track").or_else(|| p.get("track_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'track' parameter"))?;
        let media = p.get("media").or_else(|| p.get("media_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'media' parameter"))?;
        let at = p.get("at").and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'at' parameter"))?;

        let (source_start, source_end) = extract_range(p, "source", "source_start", "source_end")?;
        let op = Op::AddClip { track, media, at, source: Range::new(source_start, source_end) };

        self.apply_single_op(op, "Insert clip")
    }

    fn rpc_timeline_remove_clip(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let clip = p.get("clip").or_else(|| p.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'clip' parameter"))?;
        let op = Op::RemoveClip { clip };
        self.apply_single_op(op, "Remove clip")
    }

    fn rpc_timeline_trim_clip(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let clip = p.get("clip").or_else(|| p.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'clip' parameter"))?;
        let (source_start, source_end) = extract_range(p, "source", "source_start", "source_end")?;
        let op = Op::Trim { clip, source: Range::new(source_start, source_end) };
        self.apply_single_op(op, "Trim clip")
    }

    fn rpc_timeline_move_clip(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let clip = p.get("clip").or_else(|| p.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'clip' parameter"))?;
        let at = p.get("at").and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'at' parameter"))?;
        let op = Op::Move { clip, at };
        self.apply_single_op(op, "Move clip")
    }

    fn rpc_timeline_split_clip(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let clip = p.get("clip").or_else(|| p.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'clip' parameter"))?;
        let at = p.get("at").and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'at' parameter"))?;
        let op = Op::Split { clip, at };
        self.apply_single_op(op, "Split clip")
    }

    fn rpc_timeline_ripple_delete(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let (start, end) = extract_range(p, "range", "start", "end")?;
        let op = Op::RippleDelete { range: Range::new(start, end) };
        self.apply_single_op(op, "Ripple delete")
    }

    fn rpc_timeline_set_speed(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let clip = p.get("clip").or_else(|| p.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'clip' parameter"))?;
        let speed = p.get("speed").and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'speed' parameter"))?;
        let op = Op::SetSpeed { clip, speed };
        self.apply_single_op(op, "Set speed")
    }

    fn rpc_timeline_set_volume(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let clip = p.get("clip").or_else(|| p.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'clip' parameter"))?;
        let volume = p.get("volume").and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'volume' parameter"))?;
        let op = Op::SetVolume { clip, volume };
        self.apply_single_op(op, "Set volume")
    }

    fn rpc_timeline_add_marker(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let at = p.get("at").and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'at' parameter"))?;
        let label = p.get("label").and_then(|v| v.as_str()).unwrap_or("Marker");
        let op = Op::AddMarker { at, label: label.into() };
        self.apply_single_op(op, "Add marker")
    }

    fn apply_single_op(&mut self, op: Op, summary: &str) -> Result<Json, RpcError> {
        let mut next = self.history.current().clone();
        next.apply(&op).map_err(edit_error_to_rpc)?;
        self.history.commit(summary, next);
        self.notify_project_changed();
        let mut res = Vec::new();
        res.push(("ok".into(), Json::Bool(true)));
        res.push(("version".into(), Json::Int(self.history.head() as i64)));
        Ok(Json::Obj(res))
    }

    fn rpc_timeline_set_playhead(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let playhead = p.get("playhead").or_else(|| p.get("at")).and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'playhead' parameter"))?;
        self.playhead = playhead;

        let mut notif = Vec::new();
        notif.push(("playhead".into(), Json::Int(self.playhead)));
        self.push_event("playhead.moved", Json::Obj(notif));

        let mut res = Vec::new();
        res.push(("ok".into(), Json::Bool(true)));
        res.push(("playhead".into(), Json::Int(self.playhead)));
        Ok(Json::Obj(res))
    }

    fn rpc_timeline_set_selection(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let raw_clips = p.get("selected_clips").or_else(|| p.get("clips")).and_then(|v| v.as_arr()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'selected_clips' array"))?;

        let mut selected = Vec::with_capacity(raw_clips.len());
        for c in raw_clips {
            let id = c.as_u32().ok_or_else(|| RpcError::invalid_params("Invalid clip ID in selected_clips"))?;
            selected.push(id);
        }
        self.selected_clips = selected;

        let mut notif = Vec::new();
        notif.push(("selected_clips".into(), Json::Arr(self.selected_clips.iter().map(|&id| Json::Int(id as i64)).collect())));
        self.push_event("selection.changed", Json::Obj(notif));

        let mut res = Vec::new();
        res.push(("ok".into(), Json::Bool(true)));
        res.push(("selected_clips".into(), Json::Arr(self.selected_clips.iter().map(|&id| Json::Int(id as i64)).collect())));
        Ok(Json::Obj(res))
    }

    fn rpc_batch_execute(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let raw_ops = p.get("ops").and_then(|v| v.as_arr()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'ops' array"))?;
        let summary = p.get("summary").and_then(|v| v.as_str()).unwrap_or("Batch edit");

        let mut ops = Vec::with_capacity(raw_ops.len());
        for (i, op_json) in raw_ops.iter().enumerate() {
            let op = parse_op(op_json).map_err(|e| RpcError::invalid_params(format!("Invalid op at index {}: {}", i, e)))?;
            ops.push(op);
        }

        // Apply ALL ops atomically via Timeline::apply_all!
        let current = self.history.current();
        let new_tl = current.apply_all(&ops).map_err(edit_error_to_rpc)?;

        self.history.commit(summary, new_tl);
        self.notify_project_changed();

        let mut res = Vec::new();
        res.push(("ok".into(), Json::Bool(true)));
        res.push(("version".into(), Json::Int(self.history.head() as i64)));
        res.push(("ops_applied".into(), Json::Int(ops.len() as i64)));
        Ok(Json::Obj(res))
    }

    fn rpc_undo(&mut self) -> Result<Json, RpcError> {
        if self.history.undo() {
            self.notify_project_changed();
            let mut res = Vec::new();
            res.push(("ok".into(), Json::Bool(true)));
            res.push(("head".into(), Json::Int(self.history.head() as i64)));
            Ok(Json::Obj(res))
        } else {
            let mut res = Vec::new();
            res.push(("ok".into(), Json::Bool(false)));
            res.push(("head".into(), Json::Int(self.history.head() as i64)));
            res.push(("message".into(), Json::Str("Nothing to undo".into())));
            Ok(Json::Obj(res))
        }
    }

    fn rpc_redo(&mut self) -> Result<Json, RpcError> {
        if self.history.redo() {
            self.notify_project_changed();
            let mut res = Vec::new();
            res.push(("ok".into(), Json::Bool(true)));
            res.push(("head".into(), Json::Int(self.history.head() as i64)));
            Ok(Json::Obj(res))
        } else {
            let mut res = Vec::new();
            res.push(("ok".into(), Json::Bool(false)));
            res.push(("head".into(), Json::Int(self.history.head() as i64)));
            res.push(("message".into(), Json::Str("Nothing to redo".into())));
            Ok(Json::Obj(res))
        }
    }

    fn rpc_undo_history(&self) -> Result<Json, RpcError> {
        let mut list = Vec::new();
        for (i, v) in self.history.versions().iter().enumerate() {
            let mut item = Vec::new();
            item.push(("index".into(), Json::Int(i as i64)));
            item.push(("summary".into(), Json::Str(v.summary.clone())));
            list.push(Json::Obj(item));
        }
        let mut res = Vec::new();
        res.push(("head".into(), Json::Int(self.history.head() as i64)));
        res.push(("versions".into(), Json::Arr(list)));
        Ok(Json::Obj(res))
    }

    fn rpc_edit_flag(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let at = p.get("at").and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'at' parameter"))?;
        let reason = p.get("reason").and_then(|v| v.as_str()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'reason' parameter"))?;

        let id = self.next_flag_id;
        self.next_flag_id += 1;

        let flag = EditFlag { id, version: self.history.head(), at, reason: reason.into(), resolved: false };
        self.flags.push(flag);

        self.notify_flags_changed();

        let mut res = Vec::new();
        res.push(("ok".into(), Json::Bool(true)));
        res.push(("id".into(), Json::Int(id as i64)));
        res.push(("needs_review_count".into(), Json::Int(self.unresolved_flag_count() as i64)));
        Ok(Json::Obj(res))
    }

    fn rpc_edit_resolve_flag(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let id = p.get("id").or_else(|| p.get("flag_id")).and_then(|v| v.as_u32()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'id' parameter"))?;

        let flag = self.flags.iter_mut().find(|f| f.id == id).ok_or_else(|| RpcError::new(-32000, format!("Edit flag {} not found", id)))?;
        flag.resolved = true;

        self.notify_flags_changed();

        let mut res = Vec::new();
        res.push(("ok".into(), Json::Bool(true)));
        res.push(("id".into(), Json::Int(id as i64)));
        res.push(("needs_review_count".into(), Json::Int(self.unresolved_flag_count() as i64)));
        Ok(Json::Obj(res))
    }

    fn rpc_query_find_gaps(&self) -> Result<Json, RpcError> {
        let plans = plan(self.history.current());
        let mut tracks = Vec::new();
        for tp in plans {
            let mut gaps = Vec::new();
            for piece in tp.pieces {
                if let Piece::Gap { timeline } = piece {
                    gaps.push(Json::Arr(vec![Json::Int(timeline.start), Json::Int(timeline.end)]));
                }
            }
            let mut t_obj = Vec::new();
            t_obj.push(("track".into(), Json::Int(tp.track as i64)));
            t_obj.push(("gaps".into(), Json::Arr(gaps)));
            tracks.push(Json::Obj(t_obj));
        }
        let mut res = Vec::new();
        res.push(("tracks".into(), Json::Arr(tracks)));
        Ok(Json::Obj(res))
    }

    fn rpc_query_analyze_pacing(&self, params: Option<&Json>) -> Result<Json, RpcError> {
        let tl = self.history.current();
        let target_track_id = params.and_then(|p| p.get("track")).and_then(|v| v.as_u32());

        let track = if let Some(id) = target_track_id {
            tl.tracks.iter().find(|t| t.id == id)
        } else {
            tl.tracks.iter().find(|t| t.kind == TrackKind::Video).or_else(|| tl.tracks.first())
        };

        let Some(t) = track else {
            let mut empty = Vec::new();
            empty.push(("clip_count".into(), Json::Int(0)));
            empty.push(("total_duration_us".into(), Json::Int(0)));
            empty.push(("min_clip_us".into(), Json::Int(0)));
            empty.push(("max_clip_us".into(), Json::Int(0)));
            empty.push(("mean_clip_us".into(), Json::Int(0)));
            empty.push(("median_clip_us".into(), Json::Int(0)));
            empty.push(("cuts_per_minute_milli".into(), Json::Int(0)));
            empty.push(("longest_clip_id".into(), Json::Int(0)));
            return Ok(Json::Obj(empty));
        };

        let clips = &t.clips;
        let clip_count = clips.len();
        if clip_count == 0 {
            let mut empty = Vec::new();
            empty.push(("clip_count".into(), Json::Int(0)));
            empty.push(("total_duration_us".into(), Json::Int(0)));
            empty.push(("min_clip_us".into(), Json::Int(0)));
            empty.push(("max_clip_us".into(), Json::Int(0)));
            empty.push(("mean_clip_us".into(), Json::Int(0)));
            empty.push(("median_clip_us".into(), Json::Int(0)));
            empty.push(("cuts_per_minute_milli".into(), Json::Int(0)));
            empty.push(("longest_clip_id".into(), Json::Int(0)));
            return Ok(Json::Obj(empty));
        }

        let mut durs: Vec<i64> = clips.iter().map(|c| c.duration()).collect();
        let total_duration_us: i64 = durs.iter().sum();
        let min_clip_us = *durs.iter().min().unwrap_or(&0);
        let max_clip_us = *durs.iter().max().unwrap_or(&0);
        let longest_clip_id = clips.iter().max_by_key(|c| c.duration()).map(|c| c.id).unwrap_or(0);
        let mean_clip_us = total_duration_us / clip_count as i64;

        durs.sort_unstable();
        let median_clip_us = if clip_count % 2 == 1 {
            durs[clip_count / 2]
        } else {
            (durs[clip_count / 2 - 1] + durs[clip_count / 2]) / 2
        };

        // Cuts per minute in per-mille (1000 = 1 cut/min). Cuts = transitions between clips (clip_count - 1).
        let cuts = clip_count.saturating_sub(1);
        let cuts_per_minute_milli = if total_duration_us > 0 {
            ((cuts as i128 * 60_000_000 * 1000) / total_duration_us as i128) as i64
        } else {
            0
        };

        let mut res = Vec::new();
        res.push(("clip_count".into(), Json::Int(clip_count as i64)));
        res.push(("total_duration_us".into(), Json::Int(total_duration_us)));
        res.push(("min_clip_us".into(), Json::Int(min_clip_us)));
        res.push(("max_clip_us".into(), Json::Int(max_clip_us)));
        res.push(("mean_clip_us".into(), Json::Int(mean_clip_us)));
        res.push(("median_clip_us".into(), Json::Int(median_clip_us)));
        res.push(("cuts_per_minute_milli".into(), Json::Int(cuts_per_minute_milli)));
        res.push(("longest_clip_id".into(), Json::Int(longest_clip_id as i64)));
        Ok(Json::Obj(res))
    }

    fn rpc_query_search_transcript(&self, params: Option<&Json>) -> Result<Json, RpcError> {
        let p = params.ok_or_else(|| RpcError::invalid_params("Missing params object"))?;
        let query = p.get("query").or_else(|| p.get("text")).or_else(|| p.get("phrase")).and_then(|v| v.as_str()).ok_or_else(|| RpcError::invalid_params("Missing or invalid 'query' parameter"))?;

        let Some(ref tr) = self.transcript else {
            let mut empty = Vec::new();
            empty.push(("matches".into(), Json::Arr(Vec::new())));
            return Ok(Json::Obj(empty));
        };

        let q_words: Vec<String> = query.split_whitespace().map(norm).filter(|s| !s.is_empty()).collect();
        if q_words.is_empty() {
            let mut empty = Vec::new();
            empty.push(("matches".into(), Json::Arr(Vec::new())));
            return Ok(Json::Obj(empty));
        }

        let playing = tr.playing(self.history.current());
        let k = q_words.len();
        let mut matches = Vec::new();

        if playing.len() >= k {
            for i in 0..=playing.len() - k {
                let mut matches_window = true;
                for j in 0..k {
                    let word_idx = playing[i + j].0;
                    let w_norm = norm(&tr.words[word_idx].text);
                    let q = &q_words[j];
                    if w_norm != *q && !w_norm.contains(q.as_str()) {
                        matches_window = false;
                        break;
                    }
                }
                if matches_window {
                    let first_idx = playing[i].0;
                    let last_idx = playing[i + k - 1].0;
                    let source_start = tr.words[first_idx].source.start;
                    let source_end = tr.words[last_idx].source.end;
                    let timeline_at = playing[i].1;

                    let mut m_obj = Vec::new();
                    m_obj.push(("first_word".into(), Json::Int(first_idx as i64)));
                    m_obj.push(("last_word".into(), Json::Int(last_idx as i64)));
                    m_obj.push(("source_start".into(), Json::Int(source_start)));
                    m_obj.push(("source_end".into(), Json::Int(source_end)));
                    m_obj.push(("timeline_at".into(), Json::Int(timeline_at)));
                    matches.push(Json::Obj(m_obj));
                }
            }
        }

        let mut res = Vec::new();
        res.push(("matches".into(), Json::Arr(matches)));
        Ok(Json::Obj(res))
    }

    fn rpc_foreplay_cutscene_edl(&self) -> Result<Json, RpcError> {
        let tl = self.history.current();
        let mut cues = Vec::new();
        let mut splits = Vec::new();

        let video_tracks: Vec<_> = tl.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
        let audio_tracks: Vec<_> = tl.tracks.iter().filter(|t| t.kind == TrackKind::Audio).collect();

        // Detect split edits where audio clip leads or lags the video cut at the same source offset.
        for vt in &video_tracks {
            for vc in &vt.clips {
                for at in &audio_tracks {
                    for ac in &at.clips {
                        // Check identical media and identical source offset alignment!
                        if vc.media == ac.media && (vc.source.start - vc.at == ac.source.start - ac.at) {
                            // Head split:
                            if ac.at < vc.at {
                                // J-cut: audio leads picture by (vc.at - ac.at)
                                let lead_us = vc.at - ac.at;
                                let mut split_obj = Vec::new();
                                split_obj.push(("type".into(), Json::Str("j_cut".into())));
                                split_obj.push(("at".into(), Json::Int(ac.at)));
                                split_obj.push(("video_clip".into(), Json::Int(vc.id as i64)));
                                split_obj.push(("audio_clip".into(), Json::Int(ac.id as i64)));
                                split_obj.push(("video_at".into(), Json::Int(vc.at)));
                                split_obj.push(("audio_at".into(), Json::Int(ac.at)));
                                split_obj.push(("lead_us".into(), Json::Int(lead_us)));
                                split_obj.push(("media".into(), Json::Int(vc.media as i64)));
                                splits.push(Json::Obj(split_obj.clone()));
                                cues.push(Json::Obj(split_obj));
                            } else if ac.at > vc.at {
                                // L-cut: video leads picture / audio lags
                                let lag_us = ac.at - vc.at;
                                let mut split_obj = Vec::new();
                                split_obj.push(("type".into(), Json::Str("l_cut".into())));
                                split_obj.push(("at".into(), Json::Int(vc.at)));
                                split_obj.push(("video_clip".into(), Json::Int(vc.id as i64)));
                                split_obj.push(("audio_clip".into(), Json::Int(ac.id as i64)));
                                split_obj.push(("video_at".into(), Json::Int(vc.at)));
                                split_obj.push(("audio_at".into(), Json::Int(ac.at)));
                                split_obj.push(("lag_us".into(), Json::Int(lag_us)));
                                split_obj.push(("media".into(), Json::Int(vc.media as i64)));
                                splits.push(Json::Obj(split_obj.clone()));
                                cues.push(Json::Obj(split_obj));
                            }

                            // Tail split:
                            if ac.end() > vc.end() {
                                // L-cut: audio continues past video cut
                                let lag_us = ac.end() - vc.end();
                                let mut split_obj = Vec::new();
                                split_obj.push(("type".into(), Json::Str("l_cut".into())));
                                split_obj.push(("at".into(), Json::Int(vc.end())));
                                split_obj.push(("video_clip".into(), Json::Int(vc.id as i64)));
                                split_obj.push(("audio_clip".into(), Json::Int(ac.id as i64)));
                                split_obj.push(("video_end".into(), Json::Int(vc.end())));
                                split_obj.push(("audio_end".into(), Json::Int(ac.end())));
                                split_obj.push(("lag_us".into(), Json::Int(lag_us)));
                                split_obj.push(("media".into(), Json::Int(vc.media as i64)));
                                splits.push(Json::Obj(split_obj.clone()));
                                cues.push(Json::Obj(split_obj));
                            } else if ac.end() < vc.end() {
                                // J-cut: audio ends earlier than picture
                                let lead_us = vc.end() - ac.end();
                                let mut split_obj = Vec::new();
                                split_obj.push(("type".into(), Json::Str("j_cut".into())));
                                split_obj.push(("at".into(), Json::Int(ac.end())));
                                split_obj.push(("video_clip".into(), Json::Int(vc.id as i64)));
                                split_obj.push(("audio_clip".into(), Json::Int(ac.id as i64)));
                                split_obj.push(("video_end".into(), Json::Int(vc.end())));
                                split_obj.push(("audio_end".into(), Json::Int(ac.end())));
                                split_obj.push(("lead_us".into(), Json::Int(lead_us)));
                                split_obj.push(("media".into(), Json::Int(vc.media as i64)));
                                splits.push(Json::Obj(split_obj.clone()));
                                cues.push(Json::Obj(split_obj));
                            }
                        }
                    }
                }
            }
        }

        // Add timeline markers
        let mut markers_arr = Vec::new();
        for m in &tl.markers {
            let mut m_obj = Vec::new();
            m_obj.push(("type".into(), Json::Str("marker".into())));
            m_obj.push(("at".into(), Json::Int(m.at)));
            m_obj.push(("label".into(), Json::Str(m.label.clone())));
            markers_arr.push(Json::Obj(m_obj.clone()));
            cues.push(Json::Obj(m_obj));
        }

        // Sort cues chronologically by `at`
        cues.sort_by_key(|c| c.get("at").and_then(|v| v.as_i64()).unwrap_or(0));

        let mut res = Vec::new();
        res.push(("cues".into(), Json::Arr(cues)));
        res.push(("markers".into(), Json::Arr(markers_arr)));
        res.push(("splits".into(), Json::Arr(splits)));
        Ok(Json::Obj(res))
    }

    fn notify_project_changed(&mut self) {
        let mut notif = Vec::new();
        notif.push(("version".into(), Json::Int(self.history.head() as i64)));
        notif.push(("duration".into(), Json::Int(self.history.current().duration())));
        self.push_event("project.changed", Json::Obj(notif));
    }

    fn notify_flags_changed(&mut self) {
        let mut notif = Vec::new();
        notif.push(("unresolved_count".into(), Json::Int(self.unresolved_flag_count() as i64)));
        self.push_event("flags.changed", Json::Obj(notif));
    }
}

fn extract_range(p: &Json, array_key: &str, start_key: &str, end_key: &str) -> Result<(Micros, Micros), RpcError> {
    if let Some(arr) = p.get(array_key).and_then(|v| v.as_arr()) {
        if arr.len() == 2 {
            let start = arr[0].as_i64().ok_or_else(|| RpcError::invalid_params("Invalid range start"))?;
            let end = arr[1].as_i64().ok_or_else(|| RpcError::invalid_params("Invalid range end"))?;
            return Ok((start, end));
        }
    }
    let start = p.get(start_key).and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params(format!("Missing or invalid '{}'", start_key)))?;
    let end = p.get(end_key).and_then(|v| v.as_i64()).ok_or_else(|| RpcError::invalid_params(format!("Missing or invalid '{}'", end_key)))?;
    Ok((start, end))
}

fn parse_op(json: &Json) -> Result<Op, String> {
    let op_type = json.get("op").or_else(|| json.get("type")).and_then(|v| v.as_str()).ok_or_else(|| String::from("Missing 'op' type"))?;

    match op_type {
        "add_clip" | "AddClip" => {
            let track = json.get("track").or_else(|| json.get("track_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'track'"))?;
            let media = json.get("media").or_else(|| json.get("media_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'media'"))?;
            let at = json.get("at").and_then(|v| v.as_i64()).ok_or_else(|| String::from("Missing 'at'"))?;
            let (source_start, source_end) = extract_range(json, "source", "source_start", "source_end").map_err(|e| e.message)?;
            Ok(Op::AddClip { track, media, at, source: Range::new(source_start, source_end) })
        }
        "remove_clip" | "RemoveClip" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'clip'"))?;
            Ok(Op::RemoveClip { clip })
        }
        "split" | "Split" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'clip'"))?;
            let at = json.get("at").and_then(|v| v.as_i64()).ok_or_else(|| String::from("Missing 'at'"))?;
            Ok(Op::Split { clip, at })
        }
        "trim" | "Trim" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'clip'"))?;
            let (source_start, source_end) = extract_range(json, "source", "source_start", "source_end").map_err(|e| e.message)?;
            Ok(Op::Trim { clip, source: Range::new(source_start, source_end) })
        }
        "move" | "Move" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'clip'"))?;
            let at = json.get("at").and_then(|v| v.as_i64()).ok_or_else(|| String::from("Missing 'at'"))?;
            Ok(Op::Move { clip, at })
        }
        "set_speed" | "SetSpeed" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'clip'"))?;
            let speed = json.get("speed").and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'speed'"))?;
            Ok(Op::SetSpeed { clip, speed })
        }
        "set_volume" | "SetVolume" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'volume'"))?;
            let volume = json.get("volume").and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'volume'"))?;
            Ok(Op::SetVolume { clip, volume })
        }
        "set_enabled" | "SetEnabled" => {
            let clip = json.get("clip").or_else(|| json.get("clip_id")).and_then(|v| v.as_u32()).ok_or_else(|| String::from("Missing 'clip'"))?;
            let enabled = json.get("enabled").and_then(|v| v.as_bool()).ok_or_else(|| String::from("Missing 'enabled'"))?;
            Ok(Op::SetEnabled { clip, enabled })
        }
        "ripple_delete" | "RippleDelete" => {
            let (start, end) = extract_range(json, "range", "start", "end").map_err(|e| e.message)?;
            Ok(Op::RippleDelete { range: Range::new(start, end) })
        }
        "add_marker" | "AddMarker" => {
            let at = json.get("at").and_then(|v| v.as_i64()).ok_or_else(|| String::from("Missing 'at'"))?;
            let label = json.get("label").and_then(|v| v.as_str()).unwrap_or("Marker");
            Ok(Op::AddMarker { at, label: label.into() })
        }
        other => Err(format!("Unknown op type '{}'", other)),
    }
}

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into() }
    }
    fn invalid_params(message: impl Into<String>) -> Self {
        RpcError { code: -32602, message: message.into() }
    }
}

fn edit_error_to_rpc(err: EditError) -> RpcError {
    let msg = match err {
        EditError::NoTrack(id) => format!("Track {} does not exist on timeline", id),
        EditError::NoClip(id) => format!("Clip {} does not exist on timeline", id),
        EditError::NoMedia(id) => format!("Media {} does not exist on timeline", id),
        EditError::Overlap { track, clip } => format!("Clip overlaps with existing clip {} on track {}", clip, track),
        EditError::BadSource => "Invalid source range: range is empty, negative, or exceeds media duration".into(),
        EditError::BadSpeed => "Invalid playback speed: must be between 10 and 100000 per-mille".into(),
        EditError::OutsideClip => "Split point is outside clip boundaries".into(),
        EditError::WrongTrack => "Track kind does not match media kind".into(),
    };
    RpcError::new(-32000, msg)
}

fn format_result(id: Json, result: Json) -> String {
    let mut obj = Vec::new();
    obj.push(("jsonrpc".into(), Json::Str("2.0".into())));
    obj.push(("id".into(), id));
    obj.push(("result".into(), result));
    Json::Obj(obj).to_string()
}

fn format_error(id: Json, code: i64, message: &str) -> String {
    let mut err_obj = Vec::new();
    err_obj.push(("code".into(), Json::Int(code)));
    err_obj.push(("message".into(), Json::Str(message.into())));

    let mut obj = Vec::new();
    obj.push(("jsonrpc".into(), Json::Str("2.0".into())));
    obj.push(("id".into(), id));
    obj.push(("error".into(), Json::Obj(err_obj)));
    Json::Obj(obj).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SECOND;

    fn test_timeline() -> (Timeline, u32, u32, u32) {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Video, "/test.mp4", 30 * SECOND);
        let v = tl.add_track(TrackKind::Video, "Video");
        let a = tl.add_track(TrackKind::Audio, "Audio");
        (tl, m, v, a)
    }

    #[test]
    fn test_json_parsing_and_escaping() {
        let text = r#"{"name":"Splicy","version":2,"nested":{"items":[1,true,null,"hello\n\"world\""]}}"#;
        let parsed = Json::parse(text).expect("parse should succeed");

        assert_eq!(parsed.get("name").and_then(|v| v.as_str()), Some("Splicy"));
        assert_eq!(parsed.get("version").and_then(|v| v.as_i64()), Some(2));

        let nested = parsed.get("nested").unwrap();
        let items = nested.get("items").and_then(|v| v.as_arr()).unwrap();
        assert_eq!(items[0], Json::Int(1));
        assert_eq!(items[1], Json::Bool(true));
        assert_eq!(items[2], Json::Null);
        assert_eq!(items[3], Json::Str("hello\n\"world\"".into()));

        // Roundtrip serialization
        let s = parsed.to_string();
        let reparsed = Json::parse(&s).expect("roundtrip parse should succeed");
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn test_json_depth_limit() {
        // Construct 32 nested arrays: should succeed
        let mut deep32 = String::new();
        for _ in 0..32 {
            deep32.push('[');
        }
        deep32.push_str("42");
        for _ in 0..32 {
            deep32.push(']');
        }
        assert!(Json::parse(&deep32).is_ok());

        // Construct 33 nested arrays: should fail with recursion limit error
        let mut deep33 = String::new();
        for _ in 0..33 {
            deep33.push('[');
        }
        deep33.push_str("42");
        for _ in 0..33 {
            deep33.push(']');
        }
        let err = Json::parse(&deep33).unwrap_err();
        assert_eq!(err, "recursion limit exceeded");
    }

    #[test]
    fn test_batch_execute_atomic_commit_and_rollback() {
        let (tl, m, v, a) = test_timeline();
        let mut host = AgentHost::new(tl);

        // 1. Batch execute two valid ops
        let batch_req = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"batch.execute","params":{{"summary":"Initial clips","ops":[
                {{"op":"add_clip","track":{},"media":{},"at":0,"source":[0, 5000000]}},
                {{"op":"add_clip","track":{},"media":{},"at":0,"source":[0, 5000000]}}
            ]}}}}"#,
            v, m, a, m
        );
        let resp = host.handle_line(&batch_req);
        let resp_json = Json::parse(&resp).unwrap();
        assert!(resp_json.get("result").is_some(), "Batch execute should succeed: {}", resp);
        assert_eq!(host.history.head(), 1);
        assert_eq!(host.history.current().tracks[0].clips.len(), 1);

        // 2. Batch execute where second op is invalid (overlap!)
        let invalid_batch_req = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"batch.execute","params":{{"summary":"Invalid batch","ops":[
                {{"op":"add_clip","track":{},"media":{},"at":5000000,"source":[5000000, 10000000]}},
                {{"op":"add_clip","track":{},"media":{},"at":0,"source":[0, 5000000]}}
            ]}}}}"#,
            v, m, v, m // Second op overlaps with existing clip at 0!
        );
        let err_resp = host.handle_line(&invalid_batch_req);
        let err_json = Json::parse(&err_resp).unwrap();
        let err = err_json.get("error").expect("Should return error");
        assert_eq!(err.get("code").and_then(|v| v.as_i64()), Some(-32000));
        assert!(err.get("message").and_then(|v| v.as_str()).unwrap().contains("overlaps"));

        // Verify ATOMIC ROLLBACK: version is still 1, and first op was NOT applied!
        assert_eq!(host.history.head(), 1);
        assert_eq!(host.history.current().tracks[0].clips.len(), 1);
    }

    #[test]
    fn test_undo_redo_and_history() {
        let (tl, m, v, _) = test_timeline();
        let mut host = AgentHost::new(tl);

        let insert_req = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"timeline.insert_clip","params":{{"track":{},"media":{},"at":0,"source":[0, 5000000]}}}}"#,
            v, m
        );
        host.handle_line(&insert_req);
        assert_eq!(host.history.head(), 1);

        // History list
        let hist_resp = host.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"undo.history"}"#);
        let hist_json = Json::parse(&hist_resp).unwrap();
        let versions = hist_json.get("result").unwrap().get("versions").unwrap().as_arr().unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].get("summary").unwrap().as_str(), Some("Original"));
        assert_eq!(versions[1].get("summary").unwrap().as_str(), Some("Insert clip"));

        // Undo
        let undo_resp = host.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"undo.undo"}"#);
        let undo_json = Json::parse(&undo_resp).unwrap();
        assert_eq!(undo_json.get("result").unwrap().get("ok").unwrap().as_bool(), Some(true));
        assert_eq!(host.history.head(), 0);
        assert_eq!(host.history.current().tracks[0].clips.len(), 0);

        // Redo
        let redo_resp = host.handle_line(r#"{"jsonrpc":"2.0","id":4,"method":"undo.redo"}"#);
        let redo_json = Json::parse(&redo_resp).unwrap();
        assert_eq!(redo_json.get("result").unwrap().get("ok").unwrap().as_bool(), Some(true));
        assert_eq!(host.history.head(), 1);
        assert_eq!(host.history.current().tracks[0].clips.len(), 1);
    }

    #[test]
    fn test_query_find_gaps() {
        let (mut tl, m, v, _) = test_timeline();
        // Insert clip from 5s to 15s on video track
        tl.apply(&Op::AddClip { track: v, media: m, at: 5 * SECOND, source: Range::new(0, 10 * SECOND) }).unwrap();
        // Add a marker at 20s so timeline duration extends to 20s
        tl.add_track(TrackKind::Audio, "Dummy"); // ensure audio track exists with 0 clips (full gap)
        let mut host = AgentHost::new(tl);

        let gaps_resp = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"query.find_gaps"}"#);
        let gaps_json = Json::parse(&gaps_resp).unwrap();
        let tracks = gaps_json.get("result").unwrap().get("tracks").unwrap().as_arr().unwrap();

        let v_track = tracks.iter().find(|t| t.get("track").unwrap().as_u32() == Some(v)).unwrap();
        let gaps = v_track.get("gaps").unwrap().as_arr().unwrap();
        // Gaps: [0, 5s] and [15s, 15s] (clip ends at 15s, duration is 15s, so first gap is [0, 5s])
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].as_arr().unwrap()[0].as_i64(), Some(0));
        assert_eq!(gaps[0].as_arr().unwrap()[1].as_i64(), Some(5 * SECOND));
    }

    #[test]
    fn test_query_analyze_pacing() {
        let (mut tl, m, v, _) = test_timeline();
        // 3 clips: 2s, 4s, 6s -> total 12s
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 2 * SECOND) }).unwrap();
        tl.apply(&Op::AddClip { track: v, media: m, at: 2 * SECOND, source: Range::new(2 * SECOND, 6 * SECOND) }).unwrap();
        tl.apply(&Op::AddClip { track: v, media: m, at: 6 * SECOND, source: Range::new(6 * SECOND, 12 * SECOND) }).unwrap();
        let mut host = AgentHost::new(tl);

        let resp = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"query.analyze_pacing"}"#);
        let res = Json::parse(&resp).unwrap();
        let pacing = res.get("result").unwrap();

        assert_eq!(pacing.get("clip_count").unwrap().as_i64(), Some(3));
        assert_eq!(pacing.get("total_duration_us").unwrap().as_i64(), Some(12 * SECOND));
        assert_eq!(pacing.get("min_clip_us").unwrap().as_i64(), Some(2 * SECOND));
        assert_eq!(pacing.get("max_clip_us").unwrap().as_i64(), Some(6 * SECOND));
        assert_eq!(pacing.get("mean_clip_us").unwrap().as_i64(), Some(4 * SECOND));
        assert_eq!(pacing.get("median_clip_us").unwrap().as_i64(), Some(4 * SECOND));
        // 2 cuts in 12s -> 10 cuts/min = 10,000 milli
        assert_eq!(pacing.get("cuts_per_minute_milli").unwrap().as_i64(), Some(10_000));
        let longest_id = host.history.current().tracks[0].clips[2].id;
        assert_eq!(pacing.get("longest_clip_id").unwrap().as_u32(), Some(longest_id));
    }

    #[test]
    fn test_query_search_transcript() {
        let (mut tl, m, v, _) = test_timeline();
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 20 * SECOND) }).unwrap();

        let mut host = AgentHost::new(tl);
        host.transcript = Some(Transcript {
            media: m,
            words: vec![
                crate::transcript::Word { text: "So,".into(), source: Range::new(500_000, 800_000) },
                crate::transcript::Word { text: "there".into(), source: Range::new(1_000_000, 1_500_000) },
                crate::transcript::Word { text: "are".into(), source: Range::new(1_500_000, 1_800_000) },
                crate::transcript::Word { text: "three".into(), source: Range::new(1_800_000, 2_200_000) },
                crate::transcript::Word { text: "things.".into(), source: Range::new(2_200_000, 2_800_000) },
            ],
        });

        // Search phrase "three things"
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"query.search_transcript","params":{"query":"Three Things"}}"#;
        let resp = host.handle_line(req);
        let res = Json::parse(&resp).unwrap();
        let matches = res.get("result").unwrap().get("matches").unwrap().as_arr().unwrap();

        assert_eq!(matches.len(), 1);
        let m0 = &matches[0];
        assert_eq!(m0.get("first_word").unwrap().as_usize(), Some(3));
        assert_eq!(m0.get("last_word").unwrap().as_usize(), Some(4));
        assert_eq!(m0.get("source_start").unwrap().as_i64(), Some(1_800_000));
        assert_eq!(m0.get("source_end").unwrap().as_i64(), Some(2_800_000));
        assert_eq!(m0.get("timeline_at").unwrap().as_i64(), Some(1_800_000));
    }

    #[test]
    fn test_edit_flag_and_needs_review_count() {
        let (tl, _, _, _) = test_timeline();
        let mut host = AgentHost::new(tl);

        assert_eq!(host.unresolved_flag_count(), 0);

        // Add flag 1
        let req1 = r#"{"jsonrpc":"2.0","id":1,"method":"edit.flag","params":{"at":2000000,"reason":"Check cut point"}}"#;
        let resp1 = host.handle_line(req1);
        let r1 = Json::parse(&resp1).unwrap();
        assert_eq!(r1.get("result").unwrap().get("needs_review_count").unwrap().as_usize(), Some(1));
        assert_eq!(host.unresolved_flag_count(), 1);

        // Add flag 2
        let req2 = r#"{"jsonrpc":"2.0","id":2,"method":"edit.flag","params":{"at":4000000,"reason":"Audio pop"}}"#;
        host.handle_line(req2);
        assert_eq!(host.unresolved_flag_count(), 2);

        // Check timeline.get snapshot
        let snap_resp = host.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"timeline.get"}"#);
        let snap = Json::parse(&snap_resp).unwrap();
        assert_eq!(snap.get("result").unwrap().get("needs_review_count").unwrap().as_usize(), Some(2));

        // Resolve flag 1
        let resolve_req = r#"{"jsonrpc":"2.0","id":4,"method":"edit.resolve_flag","params":{"id":1}}"#;
        let resolve_resp = host.handle_line(resolve_req);
        let r_res = Json::parse(&resolve_resp).unwrap();
        assert_eq!(r_res.get("result").unwrap().get("needs_review_count").unwrap().as_usize(), Some(1));
        assert_eq!(host.unresolved_flag_count(), 1);
    }

    #[test]
    fn test_foreplay_cutscene_edl_split_detection() {
        let (mut tl, m, v, a) = test_timeline();

        // Clip 1: Video 0..5s. Audio 0..6s (same source offset 0, audio ends 1s after video -> L-cut lag 1s)
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 5 * SECOND) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(0, 6 * SECOND) }).unwrap();

        // Clip 2: Video 7s..12s (source 10s..15s, offset 3s).
        // Audio 6s..12s (source 9s..15s, offset 3s, audio starts 1s before video -> J-cut lead 1s)
        tl.apply(&Op::AddClip { track: v, media: m, at: 7 * SECOND, source: Range::new(10 * SECOND, 15 * SECOND) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m, at: 6 * SECOND, source: Range::new(9 * SECOND, 15 * SECOND) }).unwrap();

        // Add timeline marker
        tl.apply(&Op::AddMarker { at: 8 * SECOND, label: "Boss intro".into() }).unwrap();

        let mut host = AgentHost::new(tl);
        let resp = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"foreplay.cutscene_edl"}"#);
        let res = Json::parse(&resp).unwrap();
        let cues = res.get("result").unwrap().get("cues").unwrap().as_arr().unwrap();

        // Verify detected split cues
        let l_cut = cues.iter().find(|c| c.get("type").and_then(|v| v.as_str()) == Some("l_cut")).expect("L-cut must be detected");
        assert_eq!(l_cut.get("lag_us").and_then(|v| v.as_i64()), Some(SECOND));

        let j_cut = cues.iter().find(|c| c.get("type").and_then(|v| v.as_str()) == Some("j_cut")).expect("J-cut must be detected");
        assert_eq!(j_cut.get("lead_us").and_then(|v| v.as_i64()), Some(SECOND));

        let marker = cues.iter().find(|c| c.get("type").and_then(|v| v.as_str()) == Some("marker")).expect("Marker must be present");
        assert_eq!(marker.get("label").and_then(|v| v.as_str()), Some("Boss intro"));
        assert_eq!(marker.get("at").and_then(|v| v.as_i64()), Some(8 * SECOND));
    }

    #[test]
    fn test_drain_events_push_notifications() {
        let (tl, m, v, _) = test_timeline();
        let mut host = AgentHost::new(tl);

        // Drain empty initially
        assert!(host.drain_events().is_empty());

        // 1. Playhead moved
        host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"timeline.set_playhead","params":{"playhead":3500000}}"#);

        // 2. Selection changed
        host.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"timeline.set_selection","params":{"clips":[1, 2]}}"#);

        // 3. Edit flag added
        host.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"edit.flag","params":{"at":1000000,"reason":"Review audio"}}"#);

        // 4. Project changed via insert clip
        let insert_req = format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"timeline.insert_clip","params":{{"track":{},"media":{},"at":0,"source":[0, 5000000]}}}}"#,
            v, m
        );
        host.handle_line(&insert_req);

        let events = host.drain_events();
        assert_eq!(events.len(), 4);

        let e0 = Json::parse(&events[0]).unwrap();
        assert_eq!(e0.get("method").unwrap().as_str(), Some("playhead.moved"));
        assert_eq!(e0.get("params").unwrap().get("playhead").unwrap().as_i64(), Some(3500000));

        let e1 = Json::parse(&events[1]).unwrap();
        assert_eq!(e1.get("method").unwrap().as_str(), Some("selection.changed"));

        let e2 = Json::parse(&events[2]).unwrap();
        assert_eq!(e2.get("method").unwrap().as_str(), Some("flags.changed"));
        assert_eq!(e2.get("params").unwrap().get("unresolved_count").unwrap().as_i64(), Some(1));

        let e3 = Json::parse(&events[3]).unwrap();
        assert_eq!(e3.get("method").unwrap().as_str(), Some("project.changed"));

        // Subsequent drain is empty!
        assert!(host.drain_events().is_empty());
    }

    #[test]
    fn test_timeline_individual_operations() {
        let (tl, m, v, _) = test_timeline();
        let mut host = AgentHost::new(tl);

        // 1. Insert clip (0..10s)
        let ins = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"timeline.insert_clip","params":{{"track":{},"media":{},"at":0,"source":[0, 10000000]}}}}"#,
            v, m
        ));
        assert!(ins.contains(r#""ok":true"#));
        let clip_id = host.history.current().tracks[0].clips[0].id;

        // 2. Split clip at 4s
        let split = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"timeline.split_clip","params":{{"clip":{},"at":4000000}}}}"#,
            clip_id
        ));
        assert!(split.contains(r#""ok":true"#));
        assert_eq!(host.history.current().tracks[0].clips.len(), 2);

        let second_clip_id = host.history.current().tracks[0].clips[1].id;

        // 3. Trim second clip
        let trim = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"timeline.trim_clip","params":{{"clip":{},"source":[4000000, 8000000]}}}}"#,
            second_clip_id
        ));
        assert!(trim.contains(r#""ok":true"#));

        // 4. Set speed on second clip (2000 = 2x speed)
        let speed = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"timeline.set_speed","params":{{"clip":{},"speed":2000}}}}"#,
            second_clip_id
        ));
        assert!(speed.contains(r#""ok":true"#));

        // 5. Set volume on first clip (500 = 50% gain)
        let vol = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":5,"method":"timeline.set_volume","params":{{"clip":{},"volume":500}}}}"#,
            clip_id
        ));
        assert!(vol.contains(r#""ok":true"#));

        // 6. Move second clip to 6s
        let mv = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":6,"method":"timeline.move_clip","params":{{"clip":{},"at":6000000}}}}"#,
            second_clip_id
        ));
        assert!(mv.contains(r#""ok":true"#));

        // 7. Add marker
        let marker = host.handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"timeline.add_marker","params":{"at":1500000,"label":"Scene 1"}}"#);
        assert!(marker.contains(r#""ok":true"#));
        assert_eq!(host.history.current().markers.len(), 1);

        // 8. Ripple delete (1s..2s)
        let rip = host.handle_line(r#"{"jsonrpc":"2.0","id":8,"method":"timeline.ripple_delete","params":{"range":[1000000, 2000000]}}"#);
        assert!(rip.contains(r#""ok":true"#));

        // 9. Remove clip
        let rem = host.handle_line(&format!(
            r#"{{"jsonrpc":"2.0","id":9,"method":"timeline.remove_clip","params":{{"clip":{}}}}}"#,
            clip_id
        ));
        assert!(rem.contains(r#""ok":true"#));
    }

    #[test]
    fn test_jsonrpc_error_framing() {
        let (tl, _, _, _) = test_timeline();
        let mut host = AgentHost::new(tl);

        // Empty line -> -32700
        let r1 = host.handle_line("");
        assert!(r1.contains(r#""code":-32700"#));

        // Malformed JSON -> -32700
        let r2 = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"#);
        assert!(r2.contains(r#""code":-32700"#));

        // Missing jsonrpc "2.0" -> -32600
        let r3 = host.handle_line(r#"{"id":1,"method":"timeline.get"}"#);
        assert!(r3.contains(r#""code":-32600"#));

        // Missing method -> -32600
        let r4 = host.handle_line(r#"{"jsonrpc":"2.0","id":1}"#);
        assert!(r4.contains(r#""code":-32600"#));

        // Method not found -> -32601
        let r5 = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"nonexistent.method"}"#);
        assert!(r5.contains(r#""code":-32601"#));

        // Invalid params -> -32602
        let r6 = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"timeline.insert_clip","params":{}}"#);
        assert!(r6.contains(r#""code":-32602"#));

        // Edit error -> -32000
        let r7 = host.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"timeline.remove_clip","params":{"clip":9999}}"#);
        assert!(r7.contains(r#""code":-32000"#));
        assert!(r7.contains("does not exist"));
    }
}
