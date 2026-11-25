// SPDX-License-Identifier: GPL-3.0-only
//! A local, unsent RFC 5322 draft. Account identity and delivery are added only
//! after Multipass gives Valentine an authenticated submission flow.
use crate::message::{self, Message};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalDraft { pub to: String, pub subject: String, pub body: String }

impl LocalDraft {
    /// Reject controls in header fields so an imported or edited draft cannot
    /// inject an extra recipient or message header.
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.to.chars().any(char::is_control) || self.subject.chars().any(char::is_control) { return None; }
        let mut out = alloc::format!("X-Unsent: 1\r\nTo: {}\r\nSubject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n",
            self.to.trim(), message::encode_word(self.subject.trim())).into_bytes();
        for line in self.body.split('\n') { out.extend_from_slice(line.trim_end_matches('\r').as_bytes()); out.extend_from_slice(b"\r\n"); }
        Some(out)
    }

    pub fn from_message(message: &Message) -> Option<Self> {
        if message.headers.get("X-Unsent") != Some("1") { return None; }
        let text = message.text.as_deref().unwrap_or("");
        let body = text.strip_suffix("\r\n").unwrap_or(text).replace("\r\n", "\n");
        Some(Self { to: message.headers.text("To").unwrap_or_default(),
            subject: message.subject(), body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_round_trip_and_header_injection_refused() {
        let d = LocalDraft { to: "amy@example.org".into(), subject: "Café plans".into(), body: "Hello\nSecond line".into() };
        let raw = d.encode().unwrap();
        assert_eq!(crate::inbox::Summary::from_raw(&raw).from, "Draft");
        let parsed = message::parse(&raw);
        assert_eq!(LocalDraft::from_message(&parsed), Some(d.clone()));
        assert!(LocalDraft::from_message(&message::parse(b"From: a@b\r\n\r\nHi")).is_none());
        assert!(LocalDraft { to: "amy@example.org\r\nBcc: spy@example.org".into(), ..d.clone() }.encode().is_none());
        assert!(LocalDraft { subject: "Hi\nX-Evil: yes".into(), ..d }.encode().is_none());
    }
}
