// SPDX-License-Identifier: GPL-3.0-only
//! The machine's own screen. Until Display has the framebuffer, every line the kernel
//! logs is drawn there as it is written, so a PC with no serial port still shows how far
//! a start got. Everything logged also goes to a ring in memory, which the stop screen
//! shows when the kernel cannot go on: that screen takes the framebuffer back from
//! whoever holds it.
//!
//! Nothing here allocates or takes a lock it cannot do without: a stop may come from
//! inside the allocator or from a core that held a lock when it faulted.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;

use crate::console_font::GLYPHS;

#[derive(Clone, Copy)]
struct Fb { addr: u64, width: u32, height: u32, pitch: u32, shift: [u8; 3] }
impl Fb {
    fn rgb(&self, c: u32) -> u32 {
        let (r, g, b) = ((c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff);
        (r << self.shift[0]) | (g << self.shift[1]) | (b << self.shift[2])
    }
    fn row(&self, y: u32) -> *mut u32 { (self.addr + y as u64 * self.pitch as u64) as *mut u32 }
    fn fill(&self, x: u32, y: u32, w: u32, h: u32, c: u32) {
        let px = self.rgb(c);
        let (x0, x1, y1) = (x.min(self.width), (x + w).min(self.width), (y + h).min(self.height));
        for yy in y.min(self.height)..y1 {
            // Whole rows at a time: the framebuffer is slow memory on real cards, and
            // emulated byte by byte under QEMU.
            unsafe { core::slice::from_raw_parts_mut(self.row(yy).add(x0 as usize), (x1 - x0) as usize).fill(px) };
        }
    }
    /// One cell: the glyph's ink in `fg`, the rest of its 6x9 box in `bg`.
    fn cell(&self, x: u32, y: u32, scale: u32, ch: u8, fg: u32, bg: u32) {
        let glyph = if (0x20..0x7f).contains(&ch) { GLYPHS[(ch - 0x20) as usize] } else { GLYPHS[b'?' as usize - 0x20] };
        let (fgp, bgp) = (self.rgb(fg), self.rgb(bg));
        let w = (6 * scale).min(self.width.saturating_sub(x)) as usize;
        let mut line = [0u32; 6 * 8];
        for gy in 0..9u32 {
            let bits = if (1..8).contains(&gy) { glyph[gy as usize - 1] } else { 0 };
            for (i, px) in line[..w].iter_mut().enumerate() {
                let gx = i as u32 / scale;
                *px = if gx < 5 && bits & (0x10 >> gx) != 0 { fgp } else { bgp };
            }
            for sy in 0..scale {
                let yy = y + gy * scale + sy;
                if yy >= self.height { return; }
                unsafe { core::ptr::copy_nonoverlapping(line.as_ptr(), self.row(yy).add(x as usize), w) };
            }
        }
    }
}

// ── The log ring ───────────────────────────────────────────────────────────────

const LOG_BYTES: usize = 64 * 1024;
struct Log { bytes: [u8; LOG_BYTES], written: usize }
static LOG: Mutex<Log> = Mutex::new(Log { bytes: [0; LOG_BYTES], written: 0 });

/// The last `out.len()` bytes logged, oldest first; returns how many were copied.
pub fn log_tail(out: &mut [u8]) -> usize {
    let log = match LOG.try_lock() { Some(l) => l, None => return 0 };
    copy_tail(&log, out)
}
fn copy_tail(log: &Log, out: &mut [u8]) -> usize {
    let n = out.len().min(log.written).min(LOG_BYTES);
    for (i, slot) in out[..n].iter_mut().enumerate() {
        *slot = log.bytes[(log.written - n + i) % LOG_BYTES];
    }
    n
}

// ── The boot screen ────────────────────────────────────────────────────────────

const MAX_COLS: usize = 320;
const MAX_ROWS: usize = 160;
const BOOT_BG: u32 = 0x10141a;
const BOOT_FG: u32 = 0xc8d0dc;
const BAR_BG: u32 = 0x2a3a5a;
const BAR_FG: u32 = 0xffffff;
const NOTE_BG: u32 = 0x5a3a10;
const STOP_BG: u32 = 0x4a0c14;
const STOP_FG: u32 = 0xffffff;
const STOP_DIM: u32 = 0xe8c0c4;

struct Screen {
    fb: Option<Fb>,
    scale: u32,
    cols: usize,
    rows: usize,
    /// First screen row of the scrolling log; rows above it stay put.
    top: usize,
    row: usize,
    col: usize,
    cells: [u8; MAX_COLS * MAX_ROWS],
}
static SCREEN: Mutex<Screen> = Mutex::new(Screen {
    fb: None, scale: 1, cols: 0, rows: 0, top: 1, row: 0, col: 0, cells: [b' '; MAX_COLS * MAX_ROWS],
});
/// True while the kernel draws its log on the framebuffer.
static SHOWING: AtomicBool = AtomicBool::new(false);
/// Set once, by the first core to stop the machine.
static STOPPING: AtomicBool = AtomicBool::new(false);

impl Screen {
    fn cell_w(&self) -> u32 { 6 * self.scale }
    fn cell_h(&self) -> u32 { 9 * self.scale }
    fn draw_row(&self, fb: &Fb, row: usize, fg: u32, bg: u32) {
        let y = 4 + row as u32 * self.cell_h();
        for col in 0..self.cols {
            fb.cell(4 + col as u32 * self.cell_w(), y, self.scale, self.cells[row * MAX_COLS + col], fg, bg);
        }
    }
    /// A row over background already cleared: blank cells need no drawing.
    fn draw_row_ink(&self, fb: &Fb, row: usize, fg: u32, bg: u32) {
        for col in 0..self.cols {
            let ch = self.cells[row * MAX_COLS + col];
            if ch != b' ' { fb.cell(4 + col as u32 * self.cell_w(), 4 + row as u32 * self.cell_h(), self.scale, ch, fg, bg); }
        }
    }
    fn put_row_text(&mut self, row: usize, text: &[u8]) {
        for col in 0..self.cols {
            self.cells[row * MAX_COLS + col] = *text.get(col).unwrap_or(&b' ');
        }
    }
    fn newline(&mut self, fb: &Fb) {
        self.col = 0;
        self.row += 1;
        if self.row < self.rows { return; }
        // Keep the newer half and draw the screen again: one redraw per half screen of
        // lines, instead of moving every pixel up on each line.
        let keep = (self.rows - self.top) / 2;
        for r in self.top..self.rows {
            for c in 0..self.cols {
                let from = r + (self.rows - self.top - keep);
                self.cells[r * MAX_COLS + c] = if from < self.rows { self.cells[from * MAX_COLS + c] } else { b' ' };
            }
        }
        let y = 4 + self.top as u32 * self.cell_h();
        fb.fill(0, y, fb.width, fb.height - y, BOOT_BG);
        for r in self.top..self.top + keep { self.draw_row_ink(fb, r, BOOT_FG, BOOT_BG); }
        self.row = self.top + keep;
    }
    fn byte(&mut self, fb: &Fb, b: u8) {
        match b {
            b'\n' => self.newline(fb),
            b'\r' => self.col = 0,
            _ => {
                if self.col >= self.cols { self.newline(fb); }
                self.cells[self.row * MAX_COLS + self.col] = b;
                let (x, y) = (4 + self.col as u32 * self.cell_w(), 4 + self.row as u32 * self.cell_h());
                fb.cell(x, y, self.scale, b, BOOT_FG, BOOT_BG);
                self.col += 1;
            }
        }
    }
}

/// Text size from the screen's height: about 45 rows on any screen, never smaller than 1x.
fn scale_for(height: u32) -> u32 { (height / 400).clamp(1, 8) }

/// Takes the framebuffer Limine gave the kernel and starts drawing the log on it.
pub fn init(fb: &limine::framebuffer::Framebuffer) {
    if fb.bpp != 32 || fb.address().is_null() || fb.width == 0 || fb.height == 0 { return; }
    let f = Fb { addr: fb.address() as u64, width: fb.width as u32, height: fb.height as u32, pitch: fb.pitch as u32,
        shift: [fb.red_mask_shift, fb.green_mask_shift, fb.blue_mask_shift] };
    let mut s = SCREEN.lock();
    s.scale = scale_for(f.height);
    s.cols = (((f.width - 8) / s.cell_w()) as usize).min(MAX_COLS);
    s.rows = (((f.height - 8) / s.cell_h()) as usize).min(MAX_ROWS);
    if s.cols < 20 || s.rows < 6 { return; }
    f.fill(0, 0, f.width, f.height, BOOT_BG);
    let mut title = [b' '; MAX_COLS];
    let text = b" SexOS is starting. Each line below is a step; if it stops, the last line says where.";
    title[..text.len().min(MAX_COLS)].copy_from_slice(&text[..text.len().min(MAX_COLS)]);
    s.put_row_text(0, &title);
    f.fill(0, 0, f.width, 4 + s.cell_h(), BAR_BG);
    s.draw_row(&f, 0, BAR_FG, BAR_BG);
    s.top = 1;
    s.row = 1;
    s.col = 0;
    s.fb = Some(f);
    SHOWING.store(true, Ordering::Release);
}

/// Pins a note under the title bar, above the scrolling log: what stopped last time.
pub fn pin_note(lines: &[&[u8]]) {
    if !SHOWING.load(Ordering::Acquire) { return; }
    let mut s = SCREEN.lock();
    let Some(fb) = s.fb else { return };
    let start = s.top;
    let n = lines.len().min(s.rows.saturating_sub(start + 4));
    for (i, line) in lines[..n].iter().enumerate() {
        let mut text = [b' '; MAX_COLS];
        text[0] = b' ';
        let len = line.len().min(MAX_COLS - 1);
        text[1..1 + len].copy_from_slice(&line[..len]);
        s.put_row_text(start + i, &text);
        let y = 4 + (start + i) as u32 * s.cell_h();
        fb.fill(0, y, fb.width, s.cell_h(), NOTE_BG);
        s.draw_row(&fb, start + i, BAR_FG, NOTE_BG);
    }
    s.top = start + n;
    if s.row < s.top { s.row = s.top; s.col = 0; }
}

/// Display has the framebuffer from here on; the log keeps going to memory only.
pub fn release() {
    SHOWING.store(false, Ordering::Release);
}
pub fn showing() -> bool { SHOWING.load(Ordering::Acquire) }

/// Every logged byte comes through here, from `serial::_print` and serial records.
pub fn write(bytes: &[u8]) {
    if STOPPING.load(Ordering::Relaxed) { return; }
    if let Some(mut log) = LOG.try_lock() {
        for &b in bytes {
            let at = log.written % LOG_BYTES;
            log.bytes[at] = b;
            log.written += 1;
        }
    }
    if SHOWING.load(Ordering::Acquire) {
        if let Some(mut s) = SCREEN.try_lock() {
            if let Some(fb) = s.fb { for &b in bytes { s.byte(&fb, b); } }
        }
    }
}

// ── Stopping ───────────────────────────────────────────────────────────────────

/// A fixed buffer that `write!` can fill; what does not fit is dropped.
pub struct Text<const N: usize> { pub bytes: [u8; N], pub len: usize }
impl<const N: usize> Text<N> {
    pub const fn new() -> Self { Self { bytes: [0; N], len: 0 } }
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len] }
}
impl<const N: usize> Write for Text<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if self.len == N { break; }
            self.bytes[self.len] = b;
            self.len += 1;
        }
        Ok(())
    }
}

pub fn stopping() -> bool { STOPPING.load(Ordering::Acquire) }

/// The machine cannot go on. Halt every other core, write the reason to serial, keep a
/// record for the next start, draw the stop screen, and restart after a countdown or a
/// key. Never returns; a second stop while this one runs just halts.
pub fn stop(what: fmt::Arguments) -> ! {
    x86_64::instructions::interrupts::disable();
    if STOPPING.swap(true, Ordering::AcqRel) {
        loop { x86_64::instructions::hlt(); }
    }
    // Other cores halt in their NMI handler, so nothing draws over this screen and no
    // lock this core needs is held by a core that is still running.
    crate::apic::stop_others();
    // The framebuffer is Display's page, under Display's key; the stop screen writes it
    // as the kernel. Every key is open from here on.
    unsafe { crate::pku::wrpkru(0) };
    let mut message: Text<1024> = Text::new();
    let _ = message.write_fmt(what);
    crate::serial::write_stopped(b"KERNEL PANIC: ");
    crate::serial::write_stopped(message.as_bytes());
    crate::serial::write_stopped(b"\n");
    let core = crate::core_local::CoreLocal::try_get().map(|c| c.core_id).unwrap_or(0);
    let pd = crate::core_local::CoreLocal::try_get().map(|c| c.current_pd()).unwrap_or(0);
    // A lock held by a halted core will never be let go.
    unsafe { LOG.force_unlock(); SCREEN.force_unlock(); }
    crate::crash_log::kernel_stop(message.as_bytes(), core, pd, &LOG.lock().bytes_tail());
    let restart_in = crate::crash_log::stop_restart_seconds();
    draw_stop(message.as_bytes(), core, pd, restart_in);
    countdown(restart_in);
    crate::serial::write_stopped(b"[console.stop.restart]\n");
    restart();
}

impl Log {
    /// The last 4 KiB of the log, for the record kept across the restart.
    fn bytes_tail(&self) -> [u8; crate::crash_log::TAIL_BYTES] {
        let mut out = [0u8; crate::crash_log::TAIL_BYTES];
        let n = copy_tail(self, &mut out);
        let len = out.len();
        if n < len { out.copy_within(0..n, len - n); out[..len - n].fill(0); }
        out
    }
}

fn draw_stop(message: &[u8], core: u32, pd: u32, restart_in: u32) {
    let mut s = SCREEN.lock();
    let Some(fb) = s.fb else { return };
    SHOWING.store(false, Ordering::Release);
    fb.fill(0, 0, fb.width, fb.height, STOP_BG);
    let (cols, rows) = (s.cols, s.rows);
    let mut row = 0usize;
    let mut line = |s: &mut Screen, row: &mut usize, text: &[u8], fg: u32| {
        // Long lines wrap at the screen's width.
        let mut rest = text;
        loop {
            if *row >= rows.saturating_sub(2) { return; }
            let take = rest.len().min(cols.saturating_sub(2));
            let mut cells = [b' '; MAX_COLS];
            cells[1..1 + take].copy_from_slice(&rest[..take]);
            s.put_row_text(*row, &cells);
            s.draw_row(&fb, *row, fg, STOP_BG);
            *row += 1;
            rest = &rest[take..];
            if rest.is_empty() { return; }
        }
    };
    line(&mut s, &mut row, b"SexOS stopped", STOP_FG);
    line(&mut s, &mut row, b"", STOP_FG);
    line(&mut s, &mut row, b"The kernel met a problem it cannot recover from. A record of it is kept and shown on the next start.", STOP_DIM);
    line(&mut s, &mut row, b"", STOP_FG);
    for part in message.split(|&b| b == b'\n') { line(&mut s, &mut row, part, STOP_FG); }
    let mut where_: Text<128> = Text::new();
    let _ = write!(where_, "core {}, domain {} ({})", core, pd, sex_pdx::crash_log::domain_name(pd));
    line(&mut s, &mut row, where_.as_bytes(), STOP_DIM);
    line(&mut s, &mut row, b"", STOP_FG);
    line(&mut s, &mut row, b"Last lines of the log:", STOP_DIM);
    // As many of the newest lines as are left room for.
    let room = rows.saturating_sub(row + 2);
    let mut tail = [0u8; 8192];
    let n = copy_tail(&LOG.lock(), &mut tail);
    let tail = &tail[..n];
    let lines: usize = tail.split(|&b| b == b'\n').count();
    for part in tail.split(|&b| b == b'\n').skip(lines.saturating_sub(room)) {
        let take = part.len().min(cols.saturating_sub(2));
        line(&mut s, &mut row, &part[..take], STOP_DIM);
    }
    drop(s);
    footer(restart_in);
}

fn footer(seconds: u32) {
    let mut s = SCREEN.lock();
    let Some(fb) = s.fb else { return };
    let mut text: Text<160> = Text::new();
    let _ = write!(text, " Restarting in {} s. Press any key to restart now.", seconds);
    let row = s.rows - 1;
    let mut cells = [b' '; MAX_COLS];
    let n = text.len.min(MAX_COLS);
    cells[..n].copy_from_slice(&text.bytes[..n]);
    s.put_row_text(row, &cells);
    s.draw_row(&fb, row, STOP_FG, BAR_BG);
}

/// Waits `seconds`, redrawing the footer each second; a key on a PS/2 port (or the
/// firmware's USB keyboard emulation of one) ends the wait.
fn countdown(seconds: u32) {
    use x86_64::instructions::port::Port;
    let mut status: Port<u8> = Port::new(0x64);
    let mut data: Port<u8> = Port::new(0x60);
    // Drop whatever key bytes were already waiting.
    for _ in 0..32 { if unsafe { status.read() } & 1 == 0 { break; } let _ = unsafe { data.read() }; }
    for left in (0..seconds).rev() {
        for _ in 0..100 {
            wait_10ms();
            if unsafe { status.read() } & 1 != 0 {
                let byte = unsafe { data.read() };
                // A key press, not a release (bit 7) or a controller reply.
                if byte & 0x80 == 0 && byte != 0xfa && byte != 0 { return; }
            }
        }
        footer(left);
    }
}

static TSC_PER_10MS: AtomicU64 = AtomicU64::new(0);
fn rdtsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }
/// 10 ms by the PIT, measured once against the TSC; with no PIT, a guess of 2 GHz.
fn wait_10ms() {
    let mut per = TSC_PER_10MS.load(Ordering::Relaxed);
    if per == 0 {
        let start = rdtsc();
        per = if pit_10ms() { (rdtsc() - start).max(1) } else { 20_000_000 };
        TSC_PER_10MS.store(per, Ordering::Relaxed);
        return;
    }
    let start = rdtsc();
    while rdtsc().wrapping_sub(start) < per { core::hint::spin_loop(); }
}
fn pit_10ms() -> bool {
    use x86_64::instructions::port::Port;
    let mut cmd: Port<u8> = Port::new(0x43);
    let mut ch2: Port<u8> = Port::new(0x42);
    let mut gate: Port<u8> = Port::new(0x61);
    let count: u16 = 11932;
    unsafe {
        let g = gate.read();
        gate.write((g & !0x02) | 0x01);
        cmd.write(0xB0);
        ch2.write((count & 0xff) as u8);
        ch2.write((count >> 8) as u8);
        for _ in 0..50_000_000u64 {
            if gate.read() & 0x20 != 0 { return true; }
            core::hint::spin_loop();
        }
    }
    false
}

/// Holds the boot screen still so a person can read it.
pub fn pause_seconds(seconds: u32) {
    for _ in 0..seconds * 100 { wait_10ms(); }
}

/// Restarts the machine: the keyboard controller's reset line, then the chipset's reset
/// register, then a triple fault.
pub fn restart() -> ! {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut kbc: Port<u8> = Port::new(0x64);
        for _ in 0..1000 { if kbc.read() & 2 == 0 { break; } }
        kbc.write(0xFE);
        for _ in 0..20 { wait_10ms(); }
        let mut cf9: Port<u8> = Port::new(0xCF9);
        cf9.write(0x02);
        cf9.write(0x06);
        for _ in 0..20 { wait_10ms(); }
        // No IDT: the next exception cannot be delivered, and the CPU resets.
        let empty = x86_64::structures::DescriptorTablePointer { limit: 0, base: x86_64::VirtAddr::new(0) };
        x86_64::instructions::tables::lidt(&empty);
        core::arch::asm!("int3");
    }
    loop { x86_64::instructions::hlt(); }
}
