// SPDX-License-Identifier: GPL-3.0-only
//! `hw`: the hardware report on screen. A laptop like the Alienware x17 R1 has no
//! serial port, and the boot log on screen ends when Display takes the framebuffer,
//! before the disk, Wi-Fi and sound drivers have said what they found. The kernel keeps
//! the boot log's first 512 KiB (`crash_log::OP_LOG_HEAD`); this picks out the lines
//! about hardware, so one photo of Spindle is the hardware report.
//!
//! `hw` shows the hardware lines; `hw all` shows the newest lines of the whole head;
//! `hw TEXT` shows the lines containing TEXT (e.g. `hw nvme`). `hw save [PATH]` writes
//! every hardware line to a text file (default `hw-report.txt` in the working
//! directory), e.g. `cd /media/STICK` then `hw save`, so the report leaves the laptop
//! on a USB stick instead of as photos.
//!
//! `hw record [PATH]` keeps doing that every 30 seconds for the rest of the run, so a
//! machine that hangs later still leaves a recent report behind. It writes two files in
//! turn (`hw-record-1.txt`, `hw-record-2.txt`, or PATH with `-1`/`-2` before its
//! extension), so one whole copy survives a hang in the middle of a write. Each copy
//! also holds Spindle's own newest lines (what was typed and answered: `wifi`, `beep`,
//! `tether`, ...). `hw record off` stops it.

use sex_pdx::crash_log as log;
use sex_pdx::serial_println;

/// Tags of the lines that say what hardware was found and how its driver got on.
pub const TAGS: &[&[u8]] = &[
    b"[hw.", b"[kernel.vmd.", b"[kernel.pci.nvme.found]", b"[gpu.", b"[iommu.enabled]", b"[iommu.absent]",
    b"[display.intel", b"[sexdrive.nvme.drive]", b"[sexdrive.nvme.namespace", b"[sexdrive.nvme.refused]",
    b"[sexdrive.nvme.reprovision.err]", b"[sexdrive.nvme.irq", b"[device.irq.", b"[sexdrive.nvme2.", b"[sexdrive.raid.serve]",
    b"[sexdrive.disk.", b"[sexdrive.ahci.", b"[wifi.chip", b"[wifi.firmware]", b"[wifi.alive]", b"[wifi.ready]",
    b"[wifi.failed]", b"[wifi.lease.bad]", b"[wifi.link.up]", b"[bell.hda.ready]", b"[bell.hda.codec]",
    // Only the few lines per service that say how it got on: `hw` shows the newest 160,
    // so a chatty prefix (every page fault, every packet) pushes the drives out.
    b"[sexnet.nic.", b"[sexnet.phase", b"[sexnet.wifi.", b"[sexnet.irq", b"[sexnet.usb.link", b"[net.dhcp.lease]",
    b"fault.kill", b"[service.restarted]", b"[service.stack.overflow]", b"[usb.host.controller.found]", b"[sexusb.device]",
    b"[sexusb.device.refused]", b"[sexusb.device.fail]", b"[sexacpi.ready]", b"[sexacpi.tables]", b"[sexacpi.start",
    b"[sexacpi.ec]", b"[sexacpi.ec.error", b"[sexacpi.touchpad", b"[sexacpi.aml.", b"[sexacpi.fadt.refused", b"[sexacpi.window.refused",
    b"[crash.record]", b"[files.fs.foreign]", b"[files.media.", b"[kernel.safe_start]",
    b"[kernel.ps2", b"[kernel.pci.xhci", b"[devmgr.lease", b"[ps2.port", b"[sexusb.controller", b"[iommu.boot_options]", b"[spindle.autorun", b"[spindle.tether.",
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
        let _ = save(if path.is_empty() { b"hw-report.txt" } else { path }, sb);
        return;
    }
    if args == b"record" || args.starts_with(b"record ") {
        let mut rest = &args[6..];
        while let [b' ', r @ ..] = rest { rest = r; }
        record(rest, sb);
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
            // Not echoed to the log: each `hw` would refill the log ring it reads with copies.
            sb.push(&line[..n]);
        }
        index += 1;
    }, pick);
    serial_println!("[spindle.hw] lines={} shown={} head_bytes={}", total, total.min(SHOW), bytes);
    let mut text = [0u8; 120];
    let mut w = Cursor { buf: &mut text, len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut w, format_args!(
        "{} of {} lines from {} KiB of the boot log (its start and newest part). hw all | hw TEXT for more.",
        total.min(SHOW), total, bytes / 1024));
    let n = w.len;
    sb.push(&text[..n]);
}

/// The most a saved report holds: every hardware line of the head fits easily,
/// with room for the boot log's tail and Spindle's own newest lines.
const SAVE_MAX: usize = 240 * 1024;
/// Spindle's own newest lines kept in a report.
const SAVE_SPINDLE_LINES: usize = 300;
static mut OWN_LINES: [[u8; crate::MAX_LINE_BYTES]; SAVE_SPINDLE_LINES] = [[0; crate::MAX_LINE_BYTES]; SAVE_SPINDLE_LINES];
static mut OWN_LENGTHS: [u8; SAVE_SPINDLE_LINES] = [0; SAVE_SPINDLE_LINES];

/// `hw save PATH`: the hardware lines, the newest lines of the whole head, then
/// Spindle's newest lines, as text. Some(lines) when the write started.
fn save(path: &[u8], sb: &mut crate::Scrollback) -> Option<usize> {
    let (mut lines, mut bytes, mut head) = (0usize, 0usize, None);
    // Spindle's own lines are copied out first (the fill below cannot borrow `sb`), into
    // static room: 33 KB would crowd the 64 KiB stack.
    let own = unsafe { &mut *core::ptr::addr_of_mut!(OWN_LINES) };
    let own_len = unsafe { &mut *core::ptr::addr_of_mut!(OWN_LENGTHS) };
    let kept = sb.retained();
    let first = kept.saturating_sub(SAVE_SPINDLE_LINES);
    let mut own_count = 0usize;
    for i in first..kept {
        let line = sb.get_retained(i);
        let n = line.len().min(crate::MAX_LINE_BYTES);
        own[own_count][..n].copy_from_slice(&line[..n]);
        own_len[own_count] = n as u8;
        own_count += 1;
    }
    let ticks = sex_pdx::get_ticks();
    let started = unsafe { crate::fs_commands::FS.start_write_with(path, SAVE_MAX, |room| {
        let mut at = 0usize;
        let mut put = |text: &[u8], at: &mut usize| {
            if *at + text.len() + 1 > room.len() { return; }
            room[*at..*at + text.len()].copy_from_slice(text);
            room[*at + text.len()] = b'\n';
            *at += text.len() + 1;
        };
        put(b"SexOS hardware report (Spindle hw save)", &mut at);
        let mut stamp = [0u8; 64];
        let mut c = Cursor { buf: &mut stamp, len: 0 };
        let _ = core::fmt::Write::write_fmt(&mut c, format_args!("uptime_ms={}", ticks));
        let n = c.len;
        put(&stamp[..n], &mut at);
        head = walk(|line| { put(line, &mut at); lines += 1; }, Pick::Hardware);
        if head.is_none() { put(b"The boot log is not available here.", &mut at); }
        // The newest lines of the whole head, for what the tags miss.
        let mut total = 0usize;
        let _ = walk(|_| total += 1, Pick::All);
        let skip = total.saturating_sub(1200);
        put(b"", &mut at);
        put(b"--- newest lines of the boot log head ---", &mut at);
        let mut index = 0usize;
        let _ = walk(|line| { if index >= skip { put(line, &mut at); } index += 1; }, Pick::All);
        put(b"", &mut at);
        put(b"--- newest Spindle lines ---", &mut at);
        for i in 0..own_count { put(&own[i][..own_len[i] as usize], &mut at); }
        bytes = at;
        at
    }, sb) };
    if !started { return None; }
    serial_println!("[spindle.hw.save] lines={} bytes={} head_bytes={}", lines, bytes, head.unwrap_or(0));
    if unsafe { RECORDING.quiet } { return Some(lines); }
    let mut text = [0u8; 160];
    let mut w = Cursor { buf: &mut text, len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut w, format_args!(
        "Saving {} hardware lines ({} bytes) to {}.", lines, bytes, core::str::from_utf8(path).unwrap_or("?")));
    let n = w.len;
    sb.push(&text[..n]);
    Some(lines)
}

/// `hw record`'s state: the two file names it writes in turn, and when the next is due.
pub struct Recording {
    /// Absolute: resolved once at `hw record`, so a later `cd` leaves the copies where they were.
    paths: [[u8; 512]; 2],
    len: [usize; 2],
    on: bool,
    turn: usize,
    next_at: u64,
    writes: u64,
    /// Set while a recorded copy is written, so it says nothing on screen.
    quiet: bool,
}
pub static mut RECORDING: Recording = Recording { paths: [[0; 512]; 2], len: [0; 2], on: false, turn: 0, next_at: 0, writes: 0, quiet: false };
const RECORD_EVERY_MS: u64 = 30_000;

/// `NAME.EXT` -> `NAME-1.EXT` (or `NAME-1` without an extension).
fn numbered(path: &[u8], n: u8, out: &mut [u8; 256]) -> Option<usize> {
    let slash = path.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
    let dot = path[slash..].iter().rposition(|b| *b == b'.').map(|i| slash + i).filter(|i| *i > slash).unwrap_or(path.len());
    let total = path.len() + 2;
    if total > out.len() { return None; }
    out[..dot].copy_from_slice(&path[..dot]);
    out[dot] = b'-';
    out[dot + 1] = b'0' + n;
    out[dot + 2..total].copy_from_slice(&path[dot..]);
    Some(total)
}

fn record(args: &[u8], sb: &mut crate::Scrollback) {
    let r = unsafe { &mut *core::ptr::addr_of_mut!(RECORDING) };
    if args == b"off" || args == b"stop" {
        if r.on { r.on = false; serial_println!("[spindle.hw.record] on=0 writes={}", r.writes); sb.push(b"hw record stopped. The last copies stay on the disk."); }
        else { sb.push(b"hw record is not running."); }
        return;
    }
    let base: &[u8] = if args.is_empty() { b"hw-record.txt" } else { args };
    for i in 0..2 {
        let mut name = [0u8; 256];
        let Some(n) = numbered(base, i as u8 + 1, &mut name) else { sb.push(b"Path too long."); return };
        match unsafe { (*core::ptr::addr_of!(crate::fs_commands::FS)).join(&name[..n], &mut r.paths[i]) } {
            Some(n) if n > 0 => r.len[i] = n,
            _ => { sb.push(b"Path too long."); return; }
        }
    }
    r.on = true;
    r.turn = 0;
    r.next_at = 0;
    r.writes = 0;
    serial_println!("[spindle.hw.record] on=1 every_ms={}", RECORD_EVERY_MS);
    let mut text = [0u8; 160];
    let mut w = Cursor { buf: &mut text, len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut w, format_args!(
        "Recording the hardware report every 30 s to {} and {} (hw record off stops).",
        core::str::from_utf8(&r.paths[0][..r.len[0]]).unwrap_or("?"), core::str::from_utf8(&r.paths[1][..r.len[1]]).unwrap_or("?")));
    let n = w.len;
    sb.push(&text[..n]);
}

/// Whether the boot entry's command line holds `word` (the kernel prints it once at
/// boot as `[iommu.boot_options] cmdline="..."`; there is no other way to read it here).
pub fn boot_word(word: &[u8]) -> bool {
    let mut found = false;
    let _ = walk(|line| {
        let Some(at) = line.windows(9).position(|w| w == b"cmdline=\"") else { return };
        let rest = &line[at + 9..];
        let rest = &rest[..rest.iter().position(|b| *b == b'"').unwrap_or(rest.len())];
        if rest.split(|b| *b == b' ').any(|w| w == word) { found = true; }
    }, Pick::Matching(b"[iommu.boot_options]"));
    found
}

/// Whether `hw record` is on (the main loop then keeps its clock running).
pub fn recording() -> bool { unsafe { (*core::ptr::addr_of!(RECORDING)).on } }

/// From the main loop: writes the next recorded copy when one is due. `files_free`: no
/// other command (session save, capture, download, update, package, recording) holds the
/// Files region, whose path and data a copy would overwrite.
pub fn tick(sb: &mut crate::Scrollback, files_free: bool) -> bool {
    let r = unsafe { &mut *core::ptr::addr_of_mut!(RECORDING) };
    if !r.on || !files_free || unsafe { crate::fs_commands::FS.busy() } { return false; }
    let now = sex_pdx::get_ticks();
    if now < r.next_at { return false; }
    r.next_at = now + RECORD_EVERY_MS;
    let mut path = [0u8; 512];
    let n = r.len[r.turn];
    path[..n].copy_from_slice(&r.paths[r.turn][..n]);
    r.quiet = true;
    let wrote = save(&path[..n], sb);
    r.quiet = false;
    if wrote.is_some() {
        r.writes += 1;
        r.turn ^= 1;
        serial_println!("[spindle.hw.record.write] n={} file={}", r.writes, core::str::from_utf8(&path[..n]).unwrap_or("?"));
    }
    true
}

/// Calls `each` for every kept line of the log, in order: the head (the first 512 KiB),
/// then whatever of the newest 64 KiB came after it, so lines logged after the head
/// filled (a Wi-Fi join, an address lease) are found too. Returns the bytes read.
fn walk(mut each: impl FnMut(&[u8]), pick: Pick) -> Option<usize> {
    let mut chunk = [0u8; log::LOG_CHUNK];
    let mut line = [0u8; 256];
    let mut len = 0usize;
    let mut feed = |bytes: &[u8], len: &mut usize, line: &mut [u8; 256]| {
        for &b in bytes {
            if b == b'\n' {
                let text = trim(&line[..*len]);
                if keeps(pick, text) { each(text); }
                *len = 0;
            } else if b != b'\r' && *len < line.len() {
                line[*len] = b;
                *len += 1;
            }
        }
    };
    // The kernel answers 0 bytes while another core is writing the log: ask again.
    let mut busy = 0u32;
    let (mut offset, mut head) = (0usize, 0usize);
    loop {
        let (n, total) = log::log_head(offset, &mut chunk)?;
        if n == 0 && total == 0 && busy < 256 { busy += 1; sex_pdx::sys_yield(); continue; }
        head = head.max(total);
        feed(&chunk[..n], &mut len, &mut line);
        offset += n;
        if n == 0 || offset >= head { break; }
    }
    // The newest part, from where the head stopped (or the oldest byte still held).
    let mut written = 0usize;
    for _ in 0..256 {
        if let Some((_, w)) = log::log_at(usize::MAX, &mut chunk) { if w != 0 { written = w; break; } }
        sex_pdx::sys_yield();
    }
    let mut read = offset;
    if written > offset {
        let mut at = offset.max(written.saturating_sub(log::LOG_TAIL_BYTES));
        // Starting past the head means starting mid-line: that partial line is dropped.
        let mut skipping = at != offset;
        if skipping { len = 0; }
        let mut tries = 0u32;
        while at < written {
            let Some((n, now)) = log::log_at(at, &mut chunk) else { break };
            if n == 0 {
                if now == 0 && tries < 256 { tries += 1; sex_pdx::sys_yield(); continue; }
                // Overwritten while reading: jump to the oldest byte still held.
                let oldest = now.saturating_sub(log::LOG_TAIL_BYTES);
                if now != 0 && at < oldest { at = oldest; len = 0; skipping = true; continue; }
                break;
            }
            let mut bytes = &chunk[..n.min(written - at)];
            if skipping {
                match bytes.iter().position(|&b| b == b'\n') {
                    Some(i) => { bytes = &bytes[i + 1..]; skipping = false; }
                    None => bytes = &[],
                }
            }
            feed(bytes, &mut len, &mut line);
            at += n;
            read += n;
        }
    }
    Some(read)
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
