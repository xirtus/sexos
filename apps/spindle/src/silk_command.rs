// SPDX-License-Identifier: GPL-3.0-only
//! `silk`: desktop scripting (slice S4), like `hyprctl`. The words after
//! `silk` become one request to the Shell (`sex_pdx::silk_ctl`); the Shell's
//! answer comes back as lines in `OP_SILK_OUT` calls and each is printed as it
//! is, so scripts can read `key=value` words. Every answer ends with a line
//! that starts `ok` or `error`.
//!
//! `silk events follow [COUNT]` keeps printing events as they happen until
//! COUNT came or Esc is pressed. One request is answered at a time.
use crate::Scrollback;
use sex_pdx::silk_ctl::{self as wire, Assembler, Got, Piece, Request};

/// The Shell's PD.
const SHELL_PD: u32 = 3;
/// An answer that never came stops holding the command back after this long.
const PATIENCE_TICKS: u64 = 3_000;

pub struct Silk { asm: Assembler, waiting: bool, following: bool, since: u64, next_tag: u16, lines: u32 }
pub static mut SILK: Silk = Silk { asm: Assembler::new(), waiting: false, following: false, since: 0, next_tag: 1, lines: 0 };

impl Silk {
    /// Whether a feed is printing (Esc stops it).
    pub fn following(&self) -> bool { self.waiting && self.following }

    pub fn command(&mut self, args: &[u8], sb: &mut Scrollback) {
        let now = sex_pdx::get_ticks();
        if self.waiting && (self.following || now.saturating_sub(self.since) < PATIENCE_TICKS) {
            sb.push(if self.following { &b"silk: events are printing; Esc stops them"[..] } else { &b"silk: still waiting for the shell"[..] });
            return;
        }
        let req = match wire::parse(args) {
            Ok(r) => r,
            Err("") => { for l in wire::USAGE { sb.push(l.as_bytes()); } return; }
            Err(e) => { sb.push(e.as_bytes()); return; }
        };
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1).max(1);
        if self.send(req, tag) {
            self.asm.expect(tag);
            self.waiting = true;
            self.following = matches!(req, Request::Follow { .. });
            self.since = now;
            self.lines = 0;
        } else {
            sb.push(b"silk: could not reach the shell");
        }
    }

    fn send(&self, req: Request, tag: u16) -> bool {
        let w = req.wire(tag);
        let ok = sex_pdx::pdx_call_checked(sex_pdx::SLOT_SHELL, wire::OP_SILK_CTL, w[0], w[1], w[2]).is_ok();
        sex_pdx::serial_record!("[spindle.silk.request] kind={} tag={} sent={}", req.name(), tag, ok as u8);
        ok
    }

    /// Esc while following: the Shell ends the feed with its last line.
    pub fn stop(&mut self, sb: &mut Scrollback) {
        if !self.following() { return; }
        if !self.send(Request::Unfollow, self.asm.tag) {
            self.waiting = false;
            self.following = false;
            sb.push(b"silk: could not reach the shell; stopped");
        }
    }

    /// A piece of the Shell's answer. True when the message was one.
    pub fn receive(&mut self, msg: &sex_pdx::PdxMessage, sb: &mut Scrollback) -> bool {
        if msg.type_id != wire::OP_SILK_OUT || msg.caller_pd != SHELL_PD { return false; }
        let Some(p) = Piece::decode([msg.arg0, msg.arg1, msg.arg2]) else { return true };
        if !self.waiting { return true; }
        let got = self.asm.take(&p);
        if got == Got::Nothing { return true; }
        let tag = self.asm.tag;
        let line = self.asm.line();
        sb.push(line);
        self.lines += 1;
        sex_pdx::serial_record!("[spindle.silk.line] tag={} n={} text={}", tag, self.lines, core::str::from_utf8(line).unwrap_or("?"));
        if let Got::LineAndEnd { failed } = got {
            self.waiting = false;
            self.following = false;
            sex_pdx::serial_record!("[spindle.silk.done] tag={} lines={} ok={}", tag, self.lines, (!failed) as u8);
        }
        true
    }
}
