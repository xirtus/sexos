// SPDX-License-Identifier: GPL-3.0-only
//! Incremental HTTP/1.1 + Chunked Transfer-Encoding + Server-Sent Events (SSE) decoder.

use alloc::string::String;
use alloc::vec::Vec;

use crate::json::JsonValue;

/// Provider-agnostic streaming events emitted by Tailor's AI adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ThoughtSignature(String),
    ToolCall {
        id: String,
        name: String,
        arguments: JsonValue,
    },
    Done {
        stop_reason: Option<String>,
    },
    Error(String),
}

/// One decoded Server-Sent Event frame (`event:` optional + joined `data:` payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseMessage {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SseError {
    MalformedHeaders,
    MalformedChunkSize,
    HttpStatus { status: u16, body: String },
    HeaderTooLarge,
}

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_ERROR_BODY: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    Chunked,
    ContentLength(usize),
    CloseDelimited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkState {
    SizeLine,
    Data(usize),
    DataCrLf(usize),
    Trailers,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Headers,
    Body,
    Done,
}

/// Incremental HTTP/1.1 + chunked + SSE decoder.
///
/// Accepts arbitrary byte slices (even 1 byte at a time) and emits complete [`SseMessage`]s
/// as soon as each blank-line-delimited SSE frame arrives.
pub struct HttpSseDecoder {
    stage: Stage,
    status: Option<u16>,
    framing: Framing,
    chunk_state: ChunkState,
    buf: Vec<u8>,
    sse_line: Vec<u8>,
    cur_event: Option<String>,
    cur_data: Vec<String>,
    error_body: Vec<u8>,
}

impl Default for HttpSseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpSseDecoder {
    pub fn new() -> Self {
        Self {
            stage: Stage::Headers,
            status: None,
            framing: Framing::CloseDelimited,
            chunk_state: ChunkState::SizeLine,
            buf: Vec::new(),
            sse_line: Vec::new(),
            cur_event: None,
            cur_data: Vec::new(),
            error_body: Vec::new(),
        }
    }

    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done
    }

    /// Feeds newly arrived raw HTTP bytes and returns any newly completed SSE messages.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<SseMessage>, SseError> {
        if self.stage == Stage::Done || bytes.is_empty() {
            return Ok(Vec::new());
        }
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();

        if self.stage == Stage::Headers {
            if let Some((head_end, sep_len)) = find_header_end(&self.buf) {
                let header_bytes = self.buf[..head_end].to_vec();
                self.buf.drain(..head_end + sep_len);
                let (status, framing) = parse_headers(&header_bytes)?;
                self.status = Some(status);
                self.framing = framing;
                self.stage = match framing {
                    Framing::ContentLength(0) => Stage::Done,
                    _ => Stage::Body,
                };
            } else if self.buf.len() > MAX_HEADER_BYTES {
                return Err(SseError::HeaderTooLarge);
            } else {
                return Ok(out);
            }
        }

        if self.stage == Stage::Body {
            let mut decoded_body = Vec::new();
            self.drain_body(&mut decoded_body)?;
            self.consume_body_bytes(&decoded_body, &mut out);
        }

        if self.stage == Stage::Done {
            self.flush_sse(&mut out);
            if let Some(status) = self.status {
                if !(200..300).contains(&status) {
                    let body = String::from_utf8_lossy(&self.error_body).into_owned();
                    return Err(SseError::HttpStatus { status, body });
                }
            }
        }

        Ok(out)
    }

    /// Signals EOF from the peer, flushing any trailing SSE event or HTTP error body.
    pub fn finish(&mut self) -> Result<Vec<SseMessage>, SseError> {
        let mut out = Vec::new();
        if self.stage == Stage::Headers && !self.buf.is_empty() {
            return Err(SseError::MalformedHeaders);
        }
        if self.stage == Stage::Body {
            if self.framing == Framing::CloseDelimited && !self.buf.is_empty() {
                let rest = core::mem::take(&mut self.buf);
                self.consume_body_bytes(&rest, &mut out);
            }
            self.stage = Stage::Done;
        }
        self.flush_sse(&mut out);
        if let Some(status) = self.status {
            if !(200..300).contains(&status) {
                let body = String::from_utf8_lossy(&self.error_body).into_owned();
                return Err(SseError::HttpStatus { status, body });
            }
        }
        Ok(out)
    }

    fn drain_body(&mut self, out: &mut Vec<u8>) -> Result<(), SseError> {
        match self.framing {
            Framing::CloseDelimited => {
                out.extend_from_slice(&self.buf);
                self.buf.clear();
            }
            Framing::ContentLength(ref mut remaining) => {
                let take = (*remaining).min(self.buf.len());
                out.extend_from_slice(&self.buf[..take]);
                self.buf.drain(..take);
                *remaining -= take;
                if *remaining == 0 {
                    self.stage = Stage::Done;
                }
            }
            Framing::Chunked => {
                self.drain_chunked(out)?;
            }
        }
        Ok(())
    }

    fn drain_chunked(&mut self, out: &mut Vec<u8>) -> Result<(), SseError> {
        loop {
            match self.chunk_state {
                ChunkState::SizeLine => {
                    let Some(line_end) = find_crlf(&self.buf) else {
                        break;
                    };
                    let size_line = core::str::from_utf8(&self.buf[..line_end])
                        .map_err(|_| SseError::MalformedChunkSize)?;
                    let hex_part = size_line.split(';').next().unwrap_or("").trim();
                    let size = usize::from_str_radix(hex_part, 16)
                        .map_err(|_| SseError::MalformedChunkSize)?;
                    self.buf.drain(..line_end + 2);
                    if size == 0 {
                        self.chunk_state = ChunkState::Trailers;
                    } else {
                        self.chunk_state = ChunkState::Data(size);
                    }
                }
                ChunkState::Data(rem) => {
                    if self.buf.is_empty() {
                        break;
                    }
                    let take = rem.min(self.buf.len());
                    out.extend_from_slice(&self.buf[..take]);
                    self.buf.drain(..take);
                    let left = rem - take;
                    if left == 0 {
                        self.chunk_state = ChunkState::DataCrLf(2);
                    } else {
                        self.chunk_state = ChunkState::Data(left);
                    }
                }
                ChunkState::DataCrLf(rem) => {
                    if self.buf.is_empty() {
                        break;
                    }
                    let take = rem.min(self.buf.len());
                    self.buf.drain(..take);
                    let left = rem - take;
                    if left == 0 {
                        self.chunk_state = ChunkState::SizeLine;
                    } else {
                        self.chunk_state = ChunkState::DataCrLf(left);
                    }
                }
                ChunkState::Trailers => {
                    // Trailers end with an empty CRLF line.
                    let Some(line_end) = find_crlf(&self.buf) else {
                        break;
                    };
                    self.buf.drain(..line_end + 2);
                    if line_end == 0 {
                        self.chunk_state = ChunkState::Done;
                        self.stage = Stage::Done;
                        break;
                    }
                }
                ChunkState::Done => {
                    self.stage = Stage::Done;
                    break;
                }
            }
        }
        Ok(())
    }

    fn consume_body_bytes(&mut self, bytes: &[u8], out: &mut Vec<SseMessage>) {
        if let Some(status) = self.status {
            if !(200..300).contains(&status) {
                let space = MAX_ERROR_BODY.saturating_sub(self.error_body.len());
                let take = bytes.len().min(space);
                self.error_body.extend_from_slice(&bytes[..take]);
                return;
            }
        }
        for &b in bytes {
            if b == b'\n' {
                if self.sse_line.last() == Some(&b'\r') {
                    self.sse_line.pop();
                }
                let line = core::mem::take(&mut self.sse_line);
                self.process_sse_line(&line, out);
            } else {
                self.sse_line.push(b);
            }
        }
    }

    fn flush_sse(&mut self, out: &mut Vec<SseMessage>) {
        if !self.sse_line.is_empty() {
            if self.sse_line.last() == Some(&b'\r') {
                self.sse_line.pop();
            }
            let line = core::mem::take(&mut self.sse_line);
            self.process_sse_line(&line, out);
        }
        self.dispatch_sse_event(out);
    }

    fn process_sse_line(&mut self, line: &[u8], out: &mut Vec<SseMessage>) {
        if line.is_empty() {
            self.dispatch_sse_event(out);
            return;
        }
        if line[0] == b':' {
            // SSE comment / keepalive ping.
            return;
        }
        let Ok(text) = core::str::from_utf8(line) else {
            return;
        };
        let (field, raw_val) = match text.find(':') {
            Some(idx) => {
                let val = &text[idx + 1..];
                let val = val.strip_prefix(' ').unwrap_or(val);
                (&text[..idx], val)
            }
            None => (text, ""),
        };
        match field {
            "event" => {
                self.cur_event = Some(String::from(raw_val));
            }
            "data" => {
                self.cur_data.push(String::from(raw_val));
            }
            _ => {}
        }
    }

    fn dispatch_sse_event(&mut self, out: &mut Vec<SseMessage>) {
        if self.cur_data.is_empty() {
            self.cur_event = None;
            return;
        }
        let data = self.cur_data.join("\n");
        self.cur_data.clear();
        let event = self.cur_event.take();
        out.push(SseMessage { event, data });
    }
}

fn find_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len() {
        if buf.get(i..i + 4) == Some(b"\r\n\r\n") {
            return Some((i, 4));
        }
        if buf.get(i..i + 2) == Some(b"\n\n") {
            return Some((i, 2));
        }
    }
    None
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\r\n")
}

fn parse_headers(raw: &[u8]) -> Result<(u16, Framing), SseError> {
    let text = core::str::from_utf8(raw).map_err(|_| SseError::MalformedHeaders)?;
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let status_line = lines.next().ok_or(SseError::MalformedHeaders)?;
    let mut parts = status_line.split_whitespace();
    let proto = parts.next().ok_or(SseError::MalformedHeaders)?;
    if !proto.starts_with("HTTP/") {
        return Err(SseError::MalformedHeaders);
    }
    let code_str = parts.next().ok_or(SseError::MalformedHeaders)?;
    let status = code_str
        .parse::<u16>()
        .map_err(|_| SseError::MalformedHeaders)?;

    let mut framing = Framing::CloseDelimited;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if value
                .split(',')
                .any(|tok| tok.trim().eq_ignore_ascii_case("chunked"))
            {
                framing = Framing::Chunked;
            }
        } else if name.eq_ignore_ascii_case("content-length") && framing != Framing::Chunked {
            if let Ok(len) = value.parse::<usize>() {
                framing = Framing::ContentLength(len);
            }
        }
    }
    Ok((status, framing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn decodes_chunked_sse_byte_by_byte() {
        let sse1 = "event: message\r\ndata: {\"a\":1}\r\n\r\n";
        let sse2 = ": keepalive\ndata: line1\ndata: line2\n\n";
        let wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            sse1.len(),
            sse1,
            sse2.len(),
            sse2
        );

        let mut dec = HttpSseDecoder::new();
        let mut msgs = Vec::new();
        for b in wire.as_bytes() {
            msgs.extend(dec.feed(&[*b]).unwrap());
        }
        msgs.extend(dec.finish().unwrap());
        assert_eq!(dec.status(), Some(200));
        assert!(dec.is_complete());
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].event.as_deref(), Some("message"));
        assert_eq!(msgs[0].data, r#"{"a":1}"#);
        assert_eq!(msgs[1].event, None);
        assert_eq!(msgs[1].data, "line1\nline2");
    }

    #[test]
    fn decodes_content_length_http_error() {
        let body = r#"{"error":"bad key"}"#;
        let wire = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let mut dec = HttpSseDecoder::new();
        let err = dec.feed(wire.as_bytes()).unwrap_err();
        assert_eq!(
            err,
            SseError::HttpStatus {
                status: 401,
                body: String::from(body),
            }
        );
    }
}
