// SPDX-License-Identifier: GPL-3.0-only
//! `hw`: the hardware report on screen. A laptop like the Alienware x17 R1 has no
//! serial port, and the boot log on screen ends when Display takes the framebuffer,
//! before the disk, Wi-Fi and sound drivers have said what they found. The kernel keeps
//! the boot log's first 256 KiB (`crash_log::OP_LOG_HEAD`); this picks out the lines
//! about hardware, so one photo of Spindle is the hardware report.
//!
//! `hw` shows the hardware lines; `hw all` shows the newest lines of the whole head;
//! `hw TEXT` shows the lines containing TEXT (e.g. `hw nvme`). `hw save [PATH]` writes
//! every hardware line to a text file (default `hw-report.txt` in the working
//! directory), e.g. `cd /media/STICK` then `hw save`, so the report leaves the laptop
//! on a USB stick instead of as photos.

use sex_pdx::crash_log as log;
use sex_pdx::serial_println;

/// Tags of the lines that say what hardware was found and how its driver got on.
pub const TAGS: &[&[u8]] = &[
    b"[hw.", b"[kernel.vmd.", b"[kernel.pci.nvme.found]", b"[gpu.", b"[iommu.enabled]", b"[iommu.absent]",
    b"[display.intel", b"[sexdrive.nvme.drive]", b"[sexdrive.nvme.namespace", b"[sexdrive.nvme.refused]",
    b"[sexdrive.nvme.reprovision.err]", b"[sexdrive.nvme.irq]", b"[sexdrive.nvme2.", b"[sexdrive.raid.serve]",
    b"[sexdrive.disk.", b"[sexdrive.ahci.", b"[wifi.chip", b"[wifi.firmware]", b"[wifi.alive]", b"[wifi.ready]",
    b"[wifi.failed]", b"[wifi.lease.bad]", b"[wifi.link.up]", b"[bell.hda.ready]", b"[bell.hda.codec]",
    b"[sexnet.nic.", b"[net.dhcp.lease]", b"[usb.host.controller.found]", b"[sexusb.device]",
    b"[sexusb.device.refused]", b"[sexusb.device.fail]", b"[sexacpi.ready]", b"[sexacpi.tables]", b"[sexacpi.ec]",
    b"[sexacpi.touchpad]", b"[crash.record]",
];

/// Whether `line` is a hardware line: one of `TAGS`, at its start or after a prefix.
pub fn is_hardware(line: &[u8]) -> bool {
    let start = line.iter().position(|&b| b == b'[').unwrap_or(line.len());
    let line = &line[start..];
    TAGS.iter().any(|t| line.starts_with(t))
}

fn contains(line: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || line.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// What `hw ARGS` keeps.
#[derive(Clone, Copy)]
pub enum Pick<'a> { Hardware, All, Matching(&'a [u8]) }

pub fn pick(args: &[u8]) -> Pick<'_> {
    match args {
        b"" => Pick::Hardware,
        b"all" => Pick::All,
        text => Pick::Matching(text),
    }
}

pub fn keeps(pick: Pick, line: &[u8]) -> bool {
    match pick {
        Pick::Hardware => is_hardware(line),
        Pick::All => !line.is_empty(),
        Pick::Matching(text) => contains(line, text),
    }
}

/// Lines shown at most: the newest ones when more match.
const SHOW: usize = 160;
const LINE: usize = 158;

pub fn command(args: &[u8], sb: &mut crate::Scrollback) {
    if args == b"save" || args.starts_with(b"save ") {
        let mut path = &args[4..];
        while let [b' ', rest @ ..] = path { path = rest; }
        save(if path.is_empty() { b"hw-report.txt" } else { path }, sb);
        return;
    }
    let pick = pick(args);
    // Two passes over the head: count matches, then show the newest SHOW of them.
    let mut total = 0usize;
    let Some(bytes) = walk(|_| { total += 1; }, pick) else {
        sb.push(b"The boot log is not available here.");
        return;
    };
    let skip = total.saturating_sub(SHOW);
    let mut index = 0usize;
    let _ = walk(|line| {
        if index >= skip {
            let n = line.len().min(LINE);
            sb.push(&line[..n]);
            serial_println!("[spindle.hw.line] {}", core::str::from_utf8(&line[..n]).unwrap_or("?"));
        }
        index += 1;
    }, pick);
    serial_println!("[spindle.hw] lines={} shown={} head_bytes={}", total, total.min(SHOW), bytes);
    let mut text = [0u8; 120];
    let mut w = Cursor { buf: &mut text, len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut w, format_args!(
        "{} of {} lines from the first {} KiB of the boot log. hw all | hw TEXT for more.",
        total.min(SHOW), total, bytes / 1024));
    let n = w.len;
    sb.push(&text[..n]);
}

/// The most a saved report holds: every hardware line of a 256 KiB head fits easily.
const SAVE_MAX: usize = 64 * 1024;

/// `hw save PATH`: the hardware lines, then the newest lines of the whole head, as text.
fn save(path: &[u8], sb: &mut crate::Scrollback) {
    let (mut lines, mut bytes, mut head) = (0usize, 0usize, None);
    let started = unsafe { crate::fs_commands::FS.start_write_with(path, SAVE_MAX, |room| {
        let mut at = 0usize;
        let mut put = |text: &[u8], at: &mut usize| {
            if *at + text.len() + 1 > room.len() { return; }
            room[*at..*at + text.len()].copy_from_slice(text);
            room[*at + text.len()] = b'\n';
            *at += text.len() + 1;
        };
        put(b"SexOS hardware report (Spindle hw save)", &mut at);
        head = walk(|line| { put(line, &mut at); lines += 1; }, Pick::Hardware);
        if head.is_none() { put(b"The boot log is not available here.", &mut at); }
        // The newest 200 lines of the whole head, for what the tags miss.
        let mut total = 0usize;
        let _ = walk(|_| total += 1, Pick::All);
        let skip = total.saturating_sub(200);
        put(b"", &mut at);
        put(b"--- newest lines of the boot log head ---", &mut at);
        let mut index = 0usize;
        let _ = walk(|line| { if index >= skip { put(line, &mut at); } index += 1; }, Pick::All);
        bytes = at;
        at
    }, sb) };
    if !started { return; }
    serial_println!("[spindle.hw.save] lines={} bytes={} head_bytes={}", lines, bytes, head.unwrap_or(0));
    let mut text = [0u8; 160];
    let mut w = Cursor { buf: &mut text, len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut w, format_args!(
        "Saving {} hardware lines ({} bytes) to {}.", lines, bytes, core::str::from_utf8(path).unwrap_or("?")));
    let n = w.len;
    sb.push(&text[..n]);
}

/// Calls `each` for every kept line of the head, in order; returns the head's length.
fn walk(mut each: impl FnMut(&[u8]), pick: Pick) -> Option<usize> {
    let mut chunk = [0u8; log::LOG_CHUNK];
    let mut line = [0u8; 256];
    let (mut len, mut offset, mut total) = (0usize, 0usize, 0usize);
    loop {
        let (n, head) = log::log_head(offset, &mut chunk)?;
        total = head;
        for &b in &chunk[..n] {
            if b == b'\n' {
                let text = trim(&line[..len]);
                if keeps(pick, text) { each(text); }
                len = 0;
            } else if b != b'\r' && len < line.len() {
                line[len] = b;
                len += 1;
            }
        }
        offset += n;
        if n == 0 || offset >= head { break; }
    }
    Some(total)
}

fn trim(mut line: &[u8]) -> &[u8] {
    while let [rest @ .., b' '] = line { line = rest; }
    line
}

struct Cursor<'a> { buf: &'a mut [u8], len: usize }
impl core::fmt::Write for Cursor<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() { if self.len < self.buf.len() { self.buf[self.len] = b; self.len += 1; } }
        Ok(())
    }
}
