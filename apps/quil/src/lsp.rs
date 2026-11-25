// SPDX-License-Identifier: GPL-3.0-only
//! A bounded LSP request over a hidden child run's Files relay, without ambient Files access.
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use quil_core::lsp::{frame, take_frame};
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

fn check_inner(
    path: &[u8],
    language: &str,
    text: &str,
) -> core::result::Result<Result, &'static str> {
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
    receive(deadline, true, &mut rx)?.ok_or("Language service ended without diagnostics")
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
    let result = check_inner(path, language, text);
    launch::kill(COOKIE);
    let deadline = sex_pdx::get_ticks().saturating_add(1000);
    while sex_pdx::get_ticks() < deadline {
        if matches!(
            launch::poll(COOKIE),
            Some(launch::Event::Done { .. } | launch::Event::Refused { .. })
        ) {
            break;
        }
        sex_pdx::sched_yield();
    }
    let _ = relay::release_at(0);
    result
}
