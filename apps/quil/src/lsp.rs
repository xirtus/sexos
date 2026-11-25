// SPDX-License-Identifier: GPL-3.0-only
//! A bounded LSP request over a hidden child run's Files relay, without ambient Files access.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use quil_core::lsp::{frame, take_frame, Symbol};
use serde_json::{json, Value};
use sex_app::{files::direct::relay, launch};
use sex_pdx::console_relay as console;

const COOKIE: u32 = 0x5155_4C53;
const TIMEOUT_MS: u64 = 15000;

pub struct Result {
    pub count: usize,
    pub first_line: Option<u32>,
    pub first: Option<String>,
}

fn uri(path: &[u8]) -> String {
    let mut out = String::from("file:///quil/");
    for &b in path.rsplit(|&b| b == b'/').next().unwrap_or(b"untitled") {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn write_all(bytes: &[u8], deadline: u64) -> core::result::Result<(), &'static str> {
    let mut at = 0;
    while at < bytes.len() {
        if sex_pdx::get_ticks() >= deadline {
            return Err("Language service input timed out");
        }
        let n = relay::put_at(0, &bytes[at..]).map_err(|_| "Language service input failed")?;
        at += n;
        if n == 0 {
            sex_pdx::sched_yield();
        }
    }
    Ok(())
}

fn send(body: Value, deadline: u64) -> core::result::Result<(), &'static str> {
    let body = serde_json::to_vec(&body).map_err(|_| "Language request is invalid")?;
    write_all(&frame(&body), deadline)
}

fn receive(
    deadline: u64,
    want_diagnostics: bool,
    rx: &mut Vec<u8>,
) -> core::result::Result<Option<Result>, &'static str> {
    while sex_pdx::get_ticks() < deadline {
        let (_, bytes) = relay::take_at(0).map_err(|_| "Language service output failed")?;
        if rx.len() + bytes.len() > quil_core::lsp::MAX_BODY + 1024 {
            return Err("Language service output is too large");
        }
        rx.extend_from_slice(&bytes);
        while let Some(body) = take_frame(rx)? {
            let value: Value =
                serde_json::from_slice(&body).map_err(|_| "Language service sent invalid JSON")?;
            if want_diagnostics && value["method"] == "textDocument/publishDiagnostics" {
                let list = value["params"]["diagnostics"]
                    .as_array()
                    .ok_or("Language service sent invalid diagnostics")?;
                let first = list.first();
                return Ok(Some(Result {
                    count: list.len(),
                    first_line: first
                        .and_then(|d| d["range"]["start"]["line"].as_u64())
                        .map(|n| n.min(u32::MAX as u64) as u32),
                    first: first
                        .and_then(|d| d["message"].as_str())
                        .map(ToString::to_string),
                }));
            }
            if !want_diagnostics && value["id"].as_u64() == Some(1) && value.get("result").is_some()
            {
                return Ok(None);
            }
        }
        sex_pdx::sched_yield();
    }
    Err("Language service response timed out")
}

fn begin(
    path: &[u8],
    language: &str,
    text: &str,
) -> core::result::Result<(Result, Vec<u8>, u64), &'static str> {
    let word = console::stdio(console::PIPED, console::PIPED, console::NULL);
    let ticket = relay::prepare_piped(b"/bin/quil-ls", b"", 0, word)
        .map_err(|_| "Install Quil's language service first")?;
    launch::start_ticket(ticket, false, true, COOKIE)
        .map_err(|_| "Could not start language service")?;
    let deadline = sex_pdx::get_ticks().saturating_add(TIMEOUT_MS);
    loop {
        if sex_pdx::get_ticks() >= deadline {
            return Err("Language service did not start");
        }
        match launch::poll(COOKIE) {
            Some(launch::Event::Started { .. }) => break,
            Some(launch::Event::Refused { .. } | launch::Event::Done { .. }) => {
                return Err("Language service was refused")
            }
            None => sex_pdx::sched_yield(),
        }
    }
    let mut rx = Vec::new();
    send(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}),
        deadline,
    )?;
    receive(deadline, false, &mut rx)?;
    send(
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        deadline,
    )?;
    send(
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri(path),"languageId":language,"version":1,"text":text}}}),
        deadline,
    )?;
    let diagnostics = receive(deadline, true, &mut rx)?.ok_or("Language service ended without diagnostics")?;
    Ok((diagnostics, rx, deadline))
}

fn completion_position(text: &str, cursor: usize) -> Option<(u64, u64)> {
    if cursor > text.len() || !text.is_char_boundary(cursor) { return None; }
    let before = &text[..cursor];
    let line = before.bytes().filter(|&b| b == b'\n').count() as u64;
    let tail = before.rsplit('\n').next().unwrap_or("");
    let character = tail.encode_utf16().count() as u64;
    Some((line, character))
}

fn receive_completion(deadline: u64, rx: &mut Vec<u8>) -> core::result::Result<Vec<String>, &'static str> {
    while sex_pdx::get_ticks() < deadline {
        let (_, bytes) = relay::take_at(0).map_err(|_| "Language service output failed")?;
        if rx.len() + bytes.len() > quil_core::lsp::MAX_BODY + 1024 { return Err("Language service output is too large"); }
        rx.extend_from_slice(&bytes);
        while let Some(body) = take_frame(rx)? {
            let value: Value = serde_json::from_slice(&body).map_err(|_| "Language service sent invalid JSON")?;
            if value["id"].as_u64() == Some(2) {
                let list = value["result"].as_array().ok_or("Language service sent invalid completions")?;
                if list.len() > 8 { return Err("Language service sent too many completions"); }
                let mut items = Vec::new();
                for item in list {
                    let label = item["label"].as_str().ok_or("Language service sent invalid completion")?;
                    if label.len() > 64 || !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return Err("Language service sent invalid completion"); }
                    items.push(label.to_string());
                }
                return Ok(items);
            }
        }
        sex_pdx::sched_yield();
    }
    Err("Language completion timed out")
}

fn receive_symbols(deadline: u64, rx: &mut Vec<u8>) -> core::result::Result<Vec<Symbol>, &'static str> {
    while sex_pdx::get_ticks() < deadline {
        let (_, bytes) = relay::take_at(0).map_err(|_| "Language service output failed")?;
        if rx.len() + bytes.len() > quil_core::lsp::MAX_BODY + 1024 { return Err("Language service output is too large"); }
        rx.extend_from_slice(&bytes);
        while let Some(body) = take_frame(rx)? {
            let value: Value = serde_json::from_slice(&body).map_err(|_| "Language service sent invalid JSON")?;
            if value["id"].as_u64() != Some(2) { continue; }
            let list = value["result"].as_array().ok_or("Language service sent invalid symbols")?;
            if list.len() > 128 { return Err("Language service sent too many symbols"); }
            let mut symbols = Vec::new();
            for item in list {
                let name = item["name"].as_str().ok_or("Language service sent invalid symbol")?;
                let position = &item["selectionRange"]["start"];
                let line = position["line"].as_u64().ok_or("Language service sent invalid symbol")?;
                let character = position["character"].as_u64().ok_or("Language service sent invalid symbol")?;
                let kind = item["kind"].as_u64().ok_or("Language service sent invalid symbol")?;
                if name.is_empty() || name.len() > 64 || name.chars().any(|c| c.is_control())
                    || line > u32::MAX as u64 || character > u32::MAX as u64 || kind > u32::MAX as u64 {
                    return Err("Language service sent invalid symbol");
                }
                symbols.push(Symbol { name: name.to_string(), line: line as u32, character: character as u32, kind: kind as u32 });
            }
            return Ok(symbols);
        }
        sex_pdx::sched_yield();
    }
    Err("Language symbols timed out")
}

fn complete_inner(path: &[u8], language: &str, text: &str, cursor: usize) -> core::result::Result<Vec<String>, &'static str> {
    let (line, character) = completion_position(text, cursor).ok_or("Language completion cursor is invalid")?;
    let (_, mut rx, deadline) = begin(path, language, text)?;
    send(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":uri(path)},"position":{"line":line,"character":character}}}), deadline)?;
    receive_completion(deadline, &mut rx)
}

fn symbols_inner(path: &[u8], language: &str, text: &str) -> core::result::Result<Vec<Symbol>, &'static str> {
    let (_, mut rx, deadline) = begin(path, language, text)?;
    send(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":uri(path)}}}), deadline)?;
    receive_symbols(deadline, &mut rx)
}

fn cleanup() {
    launch::kill(COOKIE);
    let deadline = sex_pdx::get_ticks().saturating_add(1000);
    while sex_pdx::get_ticks() < deadline {
        if matches!(launch::poll(COOKIE), Some(launch::Event::Done { .. } | launch::Event::Refused { .. })) { break; }
        sex_pdx::sched_yield();
    }
    let _ = relay::release_at(0);
}

/// Checks the current buffer. The server sees a bounded text snapshot, not a Files grant.
pub fn check(
    path: &[u8],
    language: &str,
    text: &str,
) -> core::result::Result<Result, &'static str> {
    if text.len() > 256 * 1024 {
        return Err("Language check supports files up to 256 KiB");
    }
    let result = begin(path, language, text).map(|(diagnostics, _, _)| diagnostics);
    cleanup();
    result
}

/// Requests at most eight labels for one immutable editor snapshot.
pub fn complete(path: &[u8], language: &str, text: &str, cursor: usize) -> core::result::Result<Vec<String>, &'static str> {
    if text.len() > 256 * 1024 { return Err("Language completion supports files up to 256 KiB"); }
    let result = complete_inner(path, language, text, cursor);
    cleanup();
    result
}

/// Requests the declaration outline for one immutable editor snapshot.
pub fn symbols(path: &[u8], language: &str, text: &str) -> core::result::Result<Vec<Symbol>, &'static str> {
    if text.len() > 256 * 1024 { return Err("Language symbols support files up to 256 KiB"); }
    let result = symbols_inner(path, language, text);
    cleanup();
    result
}
