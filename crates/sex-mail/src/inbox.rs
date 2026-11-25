// SPDX-License-Identifier: GPL-3.0-only
//! Small, safe summaries for Valentine's local `.eml` inbox.
use crate::message;
use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub from: String,
    pub subject: String,
    pub date: String,
    pub preview: String,
    pub attachments: usize,
    pub remote_content: bool,
}

impl Summary {
    pub fn from_raw(raw: &[u8]) -> Self {
        let m = message::parse(raw);
        let preview = m.text.as_deref().unwrap_or("").lines().find(|line| !line.trim().is_empty())
            .unwrap_or("").chars().take(120).collect();
        Self {
            from: m.from(), subject: m.subject(),
            date: m.headers.text("date").unwrap_or_default(), preview,
            attachments: m.attachments.len(), remote_content: m.has_remote_content(),
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty() || [self.from.as_str(), self.subject.as_str(), self.preview.as_str()]
            .iter().any(|v| v.to_lowercase().contains(&q))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_mail_search_decodes_headers_and_avoids_remote_html() {
        let raw = b"From: =?UTF-8?Q?Ren=C3=A9e?= <r@example.org>\r\nSubject: Dinner\r\nDate: Tue, 24 Nov 2026 12:00:00 +0000\r\nContent-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nMeet at seven.\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=\"https://tracker.example/p\">\r\n--x--\r\n";
        let s = Summary::from_raw(raw);
        assert_eq!(s.from, "Renée <r@example.org>");
        assert_eq!(s.preview, "Meet at seven.");
        assert!(s.remote_content);
        assert!(s.matches("renée") && s.matches("SEVEN"));
        assert!(!s.matches("tracker.example"));
    }
}
