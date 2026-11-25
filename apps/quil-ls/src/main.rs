// SPDX-License-Identifier: GPL-3.0-only
//! Quil's native LSP 3.17 diagnostic helper. It has only inherited pipe streams;
//! the editor sends document text, never a Files grant.
use quil_core::lsp::{self, MAX_BODY};
use serde_json::{json, Value};
use sex_std as _;
use std::io::{self, BufRead, BufReader, Read, Write};

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = Vec::new();
        let n = input.read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(None);
        }
        header_bytes += n;
        if header_bytes > 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP header too long",
            ));
        }
        if line == b"\r\n" {
            break;
        }
        if let Some(value) = line.strip_prefix(b"Content-Length:") {
            let value = std::str::from_utf8(value)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "LSP length"))?;
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "LSP length"))?;
            if length.replace(parsed).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate length",
                ));
            }
        }
    }
    let n = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing length"))?;
    if n > MAX_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LSP body too large",
        ));
    }
    let mut body = vec![0; n];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

fn send(out: &mut impl Write, body: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "LSP response"))?;
    out.write_all(&lsp::frame(&bytes))?;
    out.flush()
}

fn serve(input: &mut impl BufRead, out: &mut impl Write) -> io::Result<()> {
    let mut opened: Option<(String, String, String)> = None;
    while let Some(body) = read_message(input)? {
        let Ok(request) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        match request.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                send(
                    out,
                    &json!({"jsonrpc":"2.0","id":id,"result":{"capabilities":{"textDocumentSync":1,"completionProvider":{}},"serverInfo":{"name":"Quil Language Service","version":"0.1"}}}),
                )?;
            }
            Some("textDocument/didOpen") => {
                let doc = &request["params"]["textDocument"];
                let uri = doc["uri"].as_str().unwrap_or("file:///quil/untitled");
                let language = doc["languageId"].as_str().unwrap_or("plaintext");
                let text = doc["text"].as_str().unwrap_or("");
                let diagnostics: Vec<_> = lsp::diagnose(text, language).into_iter().map(|d| json!({
                    "range":{"start":{"line":d.line,"character":d.character},"end":{"line":d.line,"character":d.character+1}},
                    "severity":2,"source":"Quil","message":d.message
                })).collect();
                send(
                    out,
                    &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":diagnostics}}),
                )?;
                opened = Some((uri.to_owned(), language.to_owned(), text.to_owned()));
            }
            Some("textDocument/completion") => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let requested_uri = request["params"]["textDocument"]["uri"].as_str();
                let pos = &request["params"]["position"];
                let items: Vec<_> = opened.as_ref().filter(|(uri, _, _)| Some(uri.as_str()) == requested_uri)
                    .and_then(|(_, language, text)| {
                        let at = lsp::offset_at_position(text, pos["line"].as_u64()?, pos["character"].as_u64()?)?;
                        Some(lsp::complete(text, language, at).1)
                    }).unwrap_or_default().into_iter().map(|label| json!({"label":label,"kind":14})).collect();
                send(out, &json!({"jsonrpc":"2.0","id":id,"result":items}))?;
            }
            Some("shutdown") => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                send(out, &json!({"jsonrpc":"2.0","id":id,"result":null}))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn main() {
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();
    if let Err(err) = serve(&mut input, &mut output) {
        eprintln!("Quil language service: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_then_open_publishes_a_real_diagnostic() {
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let open = json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///quil/broken.rs","languageId":"rust","version":1,"text":"fn main() {\n"}}});
        let mut input = lsp::frame(&serde_json::to_vec(&init).unwrap());
        input.extend_from_slice(&lsp::frame(&serde_json::to_vec(&open).unwrap()));
        let mut out = Vec::new();
        serve(&mut BufReader::new(input.as_slice()), &mut out).unwrap();
        let init_reply: Value =
            serde_json::from_slice(&lsp::take_frame(&mut out).unwrap().unwrap()).unwrap();
        assert_eq!(init_reply["id"], 1);
        let notification: Value =
            serde_json::from_slice(&lsp::take_frame(&mut out).unwrap().unwrap()).unwrap();
        assert_eq!(notification["method"], "textDocument/publishDiagnostics");
        assert_eq!(
            notification["params"]["diagnostics"][0]["message"],
            "Unclosed {"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn completion_uses_open_document_and_rejects_other_uri() {
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let open = json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///quil/test.rs","languageId":"rust","version":1,"text":"fn main() { ret"}}});
        let completion = json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///quil/test.rs"},"position":{"line":0,"character":15}}});
        let wrong = json!({"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///quil/other.rs"},"position":{"line":0,"character":15}}});
        let mut input = Vec::new();
        for value in [init, open, completion, wrong] { input.extend(lsp::frame(&serde_json::to_vec(&value).unwrap())); }
        let mut out = Vec::new();
        serve(&mut BufReader::new(input.as_slice()), &mut out).unwrap();
        let mut replies = Vec::new();
        while let Some(body) = lsp::take_frame(&mut out).unwrap() { replies.push(serde_json::from_slice::<Value>(&body).unwrap()); }
        assert_eq!(replies[0]["result"]["capabilities"]["completionProvider"], json!({}));
        assert_eq!(replies[2]["id"], 2);
        assert_eq!(replies[2]["result"][0]["label"], "return");
        assert_eq!(replies[3]["result"], json!([]));
    }
}
