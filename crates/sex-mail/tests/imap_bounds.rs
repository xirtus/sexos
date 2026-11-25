// SPDX-License-Identifier: GPL-3.0-only
use sex_mail::{imap, Conn, Error, Lines, Result, MAX_IMAP_LITERAL};

struct Wire { bytes: Vec<u8>, at: usize }
impl Wire { fn new(bytes: Vec<u8>) -> Self { Self { bytes, at: 0 } } }
impl Conn for Wire {
    fn write_all(&mut self, _: &[u8]) -> Result<()> { Ok(()) }
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        let n = out.len().min(self.bytes.len().saturating_sub(self.at));
        out[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[test]
fn rejects_announced_oversize_literal_before_waiting_for_body() {
    let mut lines = Lines::new(Wire::new(Vec::new()));
    assert!(matches!(lines.bytes(MAX_IMAP_LITERAL + 1), Err(Error::Protocol(_))));
    let header = format!("* 1 FETCH (BODY[] {{{}}}\r\n", MAX_IMAP_LITERAL + 1);
    let mut client = imap::Client::resume_authenticated(Wire::new(header.into_bytes()));
    assert!(matches!(client.read_response(), Err(Error::Protocol(_))));
}

#[test]
fn rejects_many_continuations_before_response_text_grows_unbounded() {
    let mut wire = Vec::new();
    let mut line = vec![b'a'; 65_530];
    line.extend_from_slice(b" {0}\r\n");
    for _ in 0..33 { wire.extend_from_slice(&line); }
    let mut client = imap::Client::resume_authenticated(Wire::new(wire));
    assert!(matches!(client.read_response(), Err(Error::Protocol(_))));
}
