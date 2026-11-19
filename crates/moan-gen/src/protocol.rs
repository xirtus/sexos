// SPDX-License-Identifier: GPL-3.0-only
//! Upstream Protocol Codecs (ace-server / acestep-api) & 48 kHz RIFF WAV Codec.
//!
//! Provides pure `#![no_std]` JSON/HTTP request & response builders/parsers for:
//! 1. `acestep.cpp` / `acestep.vst3` `ace-server` (:8080)
//! 2. `ACE-Step-1.5` Python `acestep-api` (:8001) with memory-safe Bearer authentication
//! 3. 48 kHz stereo 16-bit and 32-bit float RIFF WAV encoder/decoder with bounds checks.

use alloc::{format, string::String, vec::Vec};
use moan_core::{Frame, RATE};
use crate::job::{MuseError, UnderstandResult};
use crate::lrc::parse_lrc;

// =========================================================================
// 1. Pure #![no_std] JSON AST & Parser
// =========================================================================

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            Self::Object(pairs) => {
                for (k, v) in pairs {
                    if k == key {
                        return Some(v);
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().map(|n| n as u64)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<JsonValue>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }
}

pub fn parse_json(input: &str) -> Result<JsonValue, MuseError> {
    let bytes = input.as_bytes();
    let mut pos = 0;
    skip_whitespace(bytes, &mut pos);
    let val = parse_value(bytes, &mut pos)?;
    skip_whitespace(bytes, &mut pos);
    Ok(val)
}

fn skip_whitespace(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t' || bytes[*pos] == b'\n' || bytes[*pos] == b'\r') {
        *pos += 1;
    }
}

fn parse_value(bytes: &[u8], pos: &mut usize) -> Result<JsonValue, MuseError> {
    skip_whitespace(bytes, pos);
    if *pos >= bytes.len() {
        return Err(MuseError::Protocol("Unexpected end of JSON"));
    }

    match bytes[*pos] {
        b'n' => parse_null(bytes, pos),
        b't' | b'f' => parse_bool(bytes, pos),
        b'"' => parse_string(bytes, pos).map(JsonValue::String),
        b'[' => parse_array(bytes, pos),
        b'{' => parse_object(bytes, pos),
        b'-' | b'0'..=b'9' => parse_number(bytes, pos),
        _ => Err(MuseError::Protocol("Invalid JSON character")),
    }
}

fn parse_null(bytes: &[u8], pos: &mut usize) -> Result<JsonValue, MuseError> {
    if *pos + 4 <= bytes.len() && &bytes[*pos..*pos + 4] == b"null" {
        *pos += 4;
        Ok(JsonValue::Null)
    } else {
        Err(MuseError::Protocol("Invalid null literal"))
    }
}

fn parse_bool(bytes: &[u8], pos: &mut usize) -> Result<JsonValue, MuseError> {
    if *pos + 4 <= bytes.len() && &bytes[*pos..*pos + 4] == b"true" {
        *pos += 4;
        Ok(JsonValue::Bool(true))
    } else if *pos + 5 <= bytes.len() && &bytes[*pos..*pos + 5] == b"false" {
        *pos += 5;
        Ok(JsonValue::Bool(false))
    } else {
        Err(MuseError::Protocol("Invalid boolean literal"))
    }
}

fn parse_string(bytes: &[u8], pos: &mut usize) -> Result<String, MuseError> {
    if *pos >= bytes.len() || bytes[*pos] != b'"' {
        return Err(MuseError::Protocol("Expected string quote"));
    }
    *pos += 1;
    let mut out = String::new();
    while *pos < bytes.len() {
        let b = bytes[*pos];
        *pos += 1;
        if b == b'"' {
            return Ok(out);
        } else if b == b'\\' {
            if *pos >= bytes.len() {
                return Err(MuseError::Protocol("Unterminated escape sequence"));
            }
            let esc = bytes[*pos];
            *pos += 1;
            match esc {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'b' => out.push('\x08'),
                b'f' => out.push('\x0c'),
                _ => out.push(esc as char),
            }
        } else {
            out.push(b as char);
        }
    }
    Err(MuseError::Protocol("Unterminated JSON string"))
}

fn parse_number(bytes: &[u8], pos: &mut usize) -> Result<JsonValue, MuseError> {
    let start = *pos;
    if *pos < bytes.len() && bytes[*pos] == b'-' {
        *pos += 1;
    }
    while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
        *pos += 1;
    }
    if *pos < bytes.len() && bytes[*pos] == b'.' {
        *pos += 1;
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
        }
    }
    if *pos < bytes.len() && (bytes[*pos] == b'e' || bytes[*pos] == b'E') {
        *pos += 1;
        if *pos < bytes.len() && (bytes[*pos] == b'+' || bytes[*pos] == b'-') {
            *pos += 1;
        }
        while *pos < bytes.len() && bytes[*pos] >= b'0' && bytes[*pos] <= b'9' {
            *pos += 1;
        }
    }

    let s = core::str::from_utf8(&bytes[start..*pos])
        .map_err(|_| MuseError::Protocol("Invalid UTF-8 in number"))?;

    // Simple float parse without std
    let n = parse_simple_float(s).ok_or(MuseError::Protocol("Failed to parse number"))?;
    Ok(JsonValue::Number(n))
}

fn parse_simple_float(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut idx = 0;
    let mut neg = false;
    if bytes[0] == b'-' {
        neg = true;
        idx += 1;
    }

    let mut integer_part = 0.0;
    while idx < bytes.len() && bytes[idx] >= b'0' && bytes[idx] <= b'9' {
        integer_part = integer_part * 10.0 + (bytes[idx] - b'0') as f64;
        idx += 1;
    }

    let mut frac_part = 0.0;
    let mut frac_div = 1.0;
    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        while idx < bytes.len() && bytes[idx] >= b'0' && bytes[idx] <= b'9' {
            frac_part = frac_part * 10.0 + (bytes[idx] - b'0') as f64;
            frac_div *= 10.0;
            idx += 1;
        }
    }

    let val = integer_part + (frac_part / frac_div);
    Some(if neg { -val } else { val })
}

fn parse_array(bytes: &[u8], pos: &mut usize) -> Result<JsonValue, MuseError> {
    *pos += 1; // skip '['
    let mut arr = Vec::new();
    loop {
        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() {
            return Err(MuseError::Protocol("Unterminated JSON array"));
        }
        if bytes[*pos] == b']' {
            *pos += 1;
            break;
        }
        let val = parse_value(bytes, pos)?;
        arr.push(val);
        skip_whitespace(bytes, pos);
        if *pos < bytes.len() && bytes[*pos] == b',' {
            *pos += 1;
        } else if *pos < bytes.len() && bytes[*pos] == b']' {
            *pos += 1;
            break;
        } else {
            return Err(MuseError::Protocol("Expected ',' or ']' in array"));
        }
    }
    Ok(JsonValue::Array(arr))
}

fn parse_object(bytes: &[u8], pos: &mut usize) -> Result<JsonValue, MuseError> {
    *pos += 1; // skip '{'
    let mut obj = Vec::new();
    loop {
        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() {
            return Err(MuseError::Protocol("Unterminated JSON object"));
        }
        if bytes[*pos] == b'}' {
            *pos += 1;
            break;
        }
        let key = parse_string(bytes, pos)?;
        skip_whitespace(bytes, pos);
        if *pos >= bytes.len() || bytes[*pos] != b':' {
            return Err(MuseError::Protocol("Expected ':' in object"));
        }
        *pos += 1;
        let val = parse_value(bytes, pos)?;
        obj.push((key, val));
        skip_whitespace(bytes, pos);
        if *pos < bytes.len() && bytes[*pos] == b',' {
            *pos += 1;
        } else if *pos < bytes.len() && bytes[*pos] == b'}' {
            *pos += 1;
            break;
        } else {
            return Err(MuseError::Protocol("Expected ',' or '}' in object"));
        }
    }
    Ok(JsonValue::Object(obj))
}

pub fn serialize_json(val: &JsonValue) -> String {
    let mut out = String::new();
    write_json_value(val, &mut out);
    out
}

fn write_json_value(val: &JsonValue, out: &mut String) {
    match val {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsonValue::Number(n) => {
            let as_i64 = *n as i64;
            let diff = if *n >= as_i64 as f64 { *n - as_i64 as f64 } else { as_i64 as f64 - *n };
            if diff < 1e-9 {
                out.push_str(&format!("{}", as_i64));
            } else {
                let frac_u64 = (diff * 10000.0) as u64;
                out.push_str(&format!("{}.{:04}", as_i64, frac_u64));
            }
        }
        JsonValue::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }
        JsonValue::Array(arr) => {
            out.push('[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_value(v, out);
            }
            out.push(']');
        }
        JsonValue::Object(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(k);
                out.push_str("\":");
                write_json_value(v, out);
            }
            out.push('}');
        }
    }
}

// =========================================================================
// 2. HTTP Request Builder & Response Parser
// =========================================================================

pub struct HttpResponse<'a> {
    pub status_code: u16,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
}

pub fn parse_http_response(raw: &[u8]) -> Result<HttpResponse<'_>, MuseError> {
    let header_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(MuseError::Protocol("HTTP response missing CRLF CRLF"))?;

    let header_str = core::str::from_utf8(&raw[..header_end])
        .map_err(|_| MuseError::Protocol("HTTP headers not UTF-8"))?;

    let mut lines = header_str.lines();
    let status_line = lines.next().ok_or(MuseError::Protocol("Empty HTTP status line"))?;
    let status_parts: Vec<&str> = status_line.split_whitespace().collect();
    if status_parts.len() < 2 {
        return Err(MuseError::Protocol("Malformed HTTP status line"));
    }
    let status_code: u16 = status_parts[1].parse().map_err(|_| MuseError::Protocol("Invalid status code"))?;

    let mut content_type = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-type") {
                content_type = Some(v.trim());
            }
        }
    }

    let body = &raw[header_end + 4..];
    Ok(HttpResponse {
        status_code,
        content_type,
        body,
    })
}

// =========================================================================
// 3. Upstream Protocol Builders (ace-server & acestep-api)
// =========================================================================

/// Properties reported by ace-server GET /props.
#[derive(Clone, Debug, PartialEq)]
pub struct AceProps {
    pub model_name: String,
    pub sample_rate: u32,
    pub max_duration_sec: u32,
    pub default_steps: u32,
}

pub fn build_ace_props_request(host: &str) -> Vec<u8> {
    format!(
        "GET /props HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        host
    )
    .into_bytes()
}

pub fn parse_ace_props_response(raw: &[u8]) -> Result<AceProps, MuseError> {
    let resp = parse_http_response(raw)?;
    if resp.status_code != 200 {
        return Err(MuseError::Protocol("ace-server /props non-200"));
    }
    let body_str = core::str::from_utf8(resp.body).map_err(|_| MuseError::Protocol("Invalid UTF-8 in /props body"))?;
    let json = parse_json(body_str)?;
    let model_name = json.get("model_name").and_then(|v| v.as_str()).unwrap_or("acestep-v15").into();
    let sample_rate = json.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(48000) as u32;
    let max_duration_sec = json.get("max_duration_sec").and_then(|v| v.as_u64()).unwrap_or(300) as u32;
    let default_steps = json.get("default_steps").and_then(|v| v.as_u64()).unwrap_or(25) as u32;
    Ok(AceProps {
        model_name,
        sample_rate,
        max_duration_sec,
        default_steps,
    })
}

pub fn build_ace_lm_request(
    host: &str,
    prompt: &str,
    lyrics: &str,
    bpm: u16,
    key_scale: &str,
    thinking_cot: bool,
    duration_sec: u32,
    batch_count: u8,
) -> Result<Vec<u8>, MuseError> {
    let json_body = serialize_json(&JsonValue::Object(alloc::vec![
        (String::from("prompt"), JsonValue::String(String::from(prompt))),
        (String::from("lyrics"), JsonValue::String(String::from(lyrics))),
        (String::from("bpm"), JsonValue::Number(bpm as f64)),
        (String::from("key"), JsonValue::String(String::from(key_scale))),
        (String::from("cot"), JsonValue::Bool(thinking_cot)),
        (String::from("duration"), JsonValue::Number(duration_sec as f64)),
        (String::from("batch"), JsonValue::Number(batch_count as f64)),
    ]));

    let mut req = Vec::new();
    let header = format!(
        "POST /lm HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        host,
        json_body.len()
    );
    req.try_reserve_exact(header.len() + json_body.len()).map_err(|_| MuseError::Memory)?;
    req.extend_from_slice(header.as_bytes());
    req.extend_from_slice(json_body.as_bytes());
    Ok(req)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AceLmResponse {
    pub tokens: Vec<u32>,
    pub thinking_cot: Option<String>,
    pub lyrics_alignment: Option<String>,
}

pub fn parse_ace_lm_response(raw: &[u8]) -> Result<AceLmResponse, MuseError> {
    let resp = parse_http_response(raw)?;
    if resp.status_code != 200 {
        return Err(MuseError::Protocol("ace-server /lm non-200"));
    }
    let body_str = core::str::from_utf8(resp.body).map_err(|_| MuseError::Protocol("Invalid UTF-8 in /lm body"))?;
    let json = parse_json(body_str)?;
    let mut tokens = Vec::new();
    if let Some(arr) = json.get("tokens").and_then(|v| v.as_array()) {
        for t in arr {
            if let Some(tok) = t.as_u64() {
                tokens.push(tok as u32);
            }
        }
    }
    let thinking_cot = json.get("cot").and_then(|v| v.as_str()).map(String::from);
    let lyrics_alignment = json.get("alignment").and_then(|v| v.as_str()).map(String::from);
    Ok(AceLmResponse {
        tokens,
        thinking_cot,
        lyrics_alignment,
    })
}

pub fn build_ace_synth_request(
    host: &str,
    prompt: &str,
    wav_guide: Option<&[u8]>,
    steps: u32,
) -> Result<Vec<u8>, MuseError> {
    let path = "/synth?wav=1";
    let body = wav_guide.unwrap_or(b"");
    let content_type = if wav_guide.is_some() { "audio/wav" } else { "application/octet-stream" };
    let header = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nX-Prompt: {}\r\nX-Steps: {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        path,
        host,
        prompt,
        steps,
        content_type,
        body.len()
    );
    let mut req = Vec::new();
    req.try_reserve_exact(header.len() + body.len()).map_err(|_| MuseError::Memory)?;
    req.extend_from_slice(header.as_bytes());
    req.extend_from_slice(body);
    Ok(req)
}

pub fn parse_ace_synth_response(raw: &[u8]) -> Result<Vec<Frame>, MuseError> {
    let resp = parse_http_response(raw)?;
    if resp.status_code != 200 {
        return Err(MuseError::Protocol("ace-server /synth non-200"));
    }
    decode_wav(resp.body)
}

pub fn build_ace_understand_request(host: &str, wav_data: &[u8]) -> Result<Vec<u8>, MuseError> {
    let header = format!(
        "POST /understand HTTP/1.1\r\nHost: {}\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        host,
        wav_data.len()
    );
    let mut req = Vec::new();
    req.try_reserve_exact(header.len() + wav_data.len()).map_err(|_| MuseError::Memory)?;
    req.extend_from_slice(header.as_bytes());
    req.extend_from_slice(wav_data);
    Ok(req)
}

pub fn parse_ace_understand_response(raw: &[u8]) -> Result<UnderstandResult, MuseError> {
    let resp = parse_http_response(raw)?;
    if resp.status_code != 200 {
        return Err(MuseError::Protocol("ace-server /understand non-200"));
    }
    let body_str = core::str::from_utf8(resp.body).map_err(|_| MuseError::Protocol("Invalid UTF-8 in understand response"))?;
    let json = parse_json(body_str)?;

    let detected_bpm = json.get("detected_bpm").and_then(|v| v.as_u64()).unwrap_or(120) as u16;
    let detected_key = json.get("detected_key").and_then(|v| v.as_str()).unwrap_or("C Major").into();
    let caption = json.get("caption").and_then(|v| v.as_str()).unwrap_or("Studio audio track").into();
    let lrc_str = json.get("lyrics").and_then(|v| v.as_str()).unwrap_or("");
    let lyrics = parse_lrc(lrc_str);

    Ok(UnderstandResult {
        detected_bpm,
        detected_key,
        caption,
        lyrics,
    })
}

// =========================================================================
// 4. ACE-Step-1.5 Python acestep-api Client
// =========================================================================

pub fn build_release_task_request(
    host: &str,
    bearer_token: &str,
    prompt: &str,
    lyrics: &str,
    vocal_language: &str,
    bpm: u16,
    key_scale: &str,
    duration_sec: u32,
) -> Result<Vec<u8>, MuseError> {
    let json_body = serialize_json(&JsonValue::Object(alloc::vec![
        (String::from("prompt"), JsonValue::String(String::from(prompt))),
        (String::from("lyrics"), JsonValue::String(String::from(lyrics))),
        (String::from("language"), JsonValue::String(String::from(vocal_language))),
        (String::from("bpm"), JsonValue::Number(bpm as f64)),
        (String::from("key"), JsonValue::String(String::from(key_scale))),
        (String::from("duration"), JsonValue::Number(duration_sec as f64)),
    ]));

    let header = format!(
        "POST /release_task HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        host,
        bearer_token,
        json_body.len()
    );

    let mut req = Vec::new();
    req.try_reserve_exact(header.len() + json_body.len()).map_err(|_| MuseError::Memory)?;
    req.extend_from_slice(header.as_bytes());
    req.extend_from_slice(json_body.as_bytes());
    Ok(req)
}

pub fn parse_release_task_response(raw: &[u8]) -> Result<String, MuseError> {
    let resp = parse_http_response(raw)?;
    if resp.status_code != 200 && resp.status_code != 202 {
        return Err(MuseError::Protocol("acestep-api /release_task rejected"));
    }
    let body_str = core::str::from_utf8(resp.body).map_err(|_| MuseError::Protocol("Invalid UTF-8 in /release_task"))?;
    let json = parse_json(body_str)?;
    let task_id = json.get("task_id").and_then(|v| v.as_str()).ok_or(MuseError::Protocol("Missing task_id"))?;
    Ok(String::from(task_id))
}

pub fn build_query_result_request(
    host: &str,
    bearer_token: &str,
    task_id: &str,
) -> Result<Vec<u8>, MuseError> {
    let json_body = serialize_json(&JsonValue::Object(alloc::vec![
        (String::from("task_id"), JsonValue::String(String::from(task_id))),
    ]));

    let header = format!(
        "POST /query_result HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        host,
        bearer_token,
        json_body.len()
    );

    let mut req = Vec::new();
    req.try_reserve_exact(header.len() + json_body.len()).map_err(|_| MuseError::Memory)?;
    req.extend_from_slice(header.as_bytes());
    req.extend_from_slice(json_body.as_bytes());
    Ok(req)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AceApiTaskStatus {
    pub task_id: String,
    pub status: String,
    pub progress: f64,
    pub audio_url: Option<String>,
    pub lyrics: Option<String>,
}

pub fn parse_query_result_response(raw: &[u8]) -> Result<AceApiTaskStatus, MuseError> {
    let resp = parse_http_response(raw)?;
    if resp.status_code != 200 {
        return Err(MuseError::Protocol("acestep-api /query_result non-200"));
    }
    let body_str = core::str::from_utf8(resp.body).map_err(|_| MuseError::Protocol("Invalid UTF-8 in /query_result"))?;
    let json = parse_json(body_str)?;

    let task_id = json.get("task_id").and_then(|v| v.as_str()).unwrap_or("").into();
    let status = json.get("status").and_then(|v| v.as_str()).unwrap_or("unknown").into();
    let progress = json.get("progress").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let audio_url = json.get("audio_url").and_then(|v| v.as_str()).map(String::from);
    let lyrics = json.get("lyrics").and_then(|v| v.as_str()).map(String::from);

    Ok(AceApiTaskStatus {
        task_id,
        status,
        progress,
        audio_url,
        lyrics,
    })
}

// =========================================================================
// 5. 48 kHz Stereo RIFF WAV Encoder & Decoder
// =========================================================================

/// Encode 48 kHz stereo 16-bit PCM frames to RIFF WAV bytes.
pub fn encode_wav_16bit(frames: &[Frame]) -> Result<Vec<u8>, MuseError> {
    let data_bytes = frames.len().checked_mul(4).ok_or(MuseError::Memory)?;
    let total_bytes = data_bytes.checked_add(44).ok_or(MuseError::Memory)?;

    let mut out = Vec::new();
    out.try_reserve_exact(total_bytes).map_err(|_| MuseError::Memory)?;

    // RIFF chunk descriptor
    out.extend_from_slice(b"RIFF");
    let riff_size = (total_bytes - 8) as u32;
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // "fmt " sub-chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    out.extend_from_slice(&1u16.to_le_bytes());  // AudioFormat = 1 (PCM)
    out.extend_from_slice(&2u16.to_le_bytes());  // NumChannels = 2
    out.extend_from_slice(&RATE.to_le_bytes());   // SampleRate = 48000
    let byte_rate = RATE * 2 * 2;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = 2 * 2u16;
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // BitsPerSample = 16

    // "data" sub-chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());

    for f in frames {
        out.extend_from_slice(&f[0].to_le_bytes());
        out.extend_from_slice(&f[1].to_le_bytes());
    }

    Ok(out)
}

/// Encode 48 kHz stereo 32-bit float samples to RIFF WAV bytes (format = 3 IEEE Float).
pub fn encode_wav_float(samples: &[[f32; 2]]) -> Result<Vec<u8>, MuseError> {
    let data_bytes = samples.len().checked_mul(8).ok_or(MuseError::Memory)?;
    let total_bytes = data_bytes.checked_add(44).ok_or(MuseError::Memory)?;

    let mut out = Vec::new();
    out.try_reserve_exact(total_bytes).map_err(|_| MuseError::Memory)?;

    // RIFF chunk descriptor
    out.extend_from_slice(b"RIFF");
    let riff_size = (total_bytes - 8) as u32;
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // "fmt " sub-chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());  // AudioFormat = 3 (IEEE Float)
    out.extend_from_slice(&2u16.to_le_bytes());  // NumChannels = 2
    out.extend_from_slice(&RATE.to_le_bytes());   // SampleRate = 48000
    let byte_rate = RATE * 2 * 4;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = 2 * 4u16;
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes()); // BitsPerSample = 32

    // "data" sub-chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_bytes as u32).to_le_bytes());

    for s in samples {
        out.extend_from_slice(&s[0].to_le_bytes());
        out.extend_from_slice(&s[1].to_le_bytes());
    }

    Ok(out)
}

/// Decode RIFF WAV bytes (supporting 16-bit PCM or 32-bit IEEE float, mono or stereo) into 48 kHz Frames.
pub fn decode_wav(bytes: &[u8]) -> Result<Vec<Frame>, MuseError> {
    if bytes.len() < 44 {
        return Err(MuseError::WavFormat("WAV file too short for header"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(MuseError::WavFormat("Missing RIFF / WAVE signature"));
    }

    let mut pos = 12;
    let mut audio_format = 1u16;
    let mut channels = 2u16;
    let mut _sample_rate = RATE;
    let mut bits_per_sample = 16u16;
    let mut data_slice: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_len = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        pos += 8;

        if pos + chunk_len > bytes.len() {
            // Some encoders write extra length; clamp to remaining bytes
            let available = bytes.len() - pos;
            if chunk_id == b"data" {
                data_slice = Some(&bytes[pos..pos + available]);
            }
            break;
        }

        if chunk_id == b"fmt " {
            if chunk_len < 16 {
                return Err(MuseError::WavFormat("fmt chunk too small"));
            }
            audio_format = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
            channels = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]);
            _sample_rate = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]);
            bits_per_sample = u16::from_le_bytes([bytes[pos + 14], bytes[pos + 15]]);
        } else if chunk_id == b"data" {
            data_slice = Some(&bytes[pos..pos + chunk_len]);
        }

        pos += chunk_len;
        if chunk_len % 2 == 1 {
            pos += 1; // Word align
        }
    }

    let data = data_slice.ok_or(MuseError::WavFormat("Missing data chunk"))?;
    if channels == 0 || (channels != 1 && channels != 2) {
        return Err(MuseError::WavFormat("Unsupported channel count (expected mono or stereo)"));
    }

    let bytes_per_sample = (bits_per_sample / 8) as usize;
    if bytes_per_sample == 0 {
        return Err(MuseError::WavFormat("Invalid bits per sample"));
    }

    let total_samples = data.len() / (bytes_per_sample * channels as usize);
    let mut frames = Vec::new();
    frames.try_reserve_exact(total_samples).map_err(|_| MuseError::Memory)?;

    let mut cursor = 0;
    for _ in 0..total_samples {
        let (left, right) = if audio_format == 1 && bits_per_sample == 16 {
            // 16-bit PCM
            let l = i16::from_le_bytes([data[cursor], data[cursor + 1]]);
            cursor += 2;
            let r = if channels == 2 {
                let val = i16::from_le_bytes([data[cursor], data[cursor + 1]]);
                cursor += 2;
                val
            } else {
                l
            };
            (l, r)
        } else if audio_format == 3 && bits_per_sample == 32 {
            // 32-bit Float
            let fl = f32::from_le_bytes([data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3]]);
            cursor += 4;
            let fr = if channels == 2 {
                let val = f32::from_le_bytes([data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3]]);
                cursor += 4;
                val
            } else {
                fl
            };

            if !fl.is_finite() || !fr.is_finite() {
                return Err(MuseError::WavFormat("Non-finite sample in float WAV"));
            }

            let l = (fl.clamp(-1.0, 1.0) * 32767.0) as i16;
            let r = (fr.clamp(-1.0, 1.0) * 32767.0) as i16;
            (l, r)
        } else {
            return Err(MuseError::WavFormat("Unsupported audio format or bit depth (expected 16-bit PCM or 32-bit float)"));
        };

        frames.push([left, right]);
    }

    Ok(frames)
}
