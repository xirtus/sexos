// SPDX-License-Identifier: GPL-3.0-only
//! A screen as bytes for another screen: what Spindle hands a yarn (a run that goes on
//! without its window) so the window that adopts it later shows what this one did.

use super::*;
use alloc::format;

/// The SGR sequence that draws `cell`'s colours and style from a reset pen.
fn sgr(cell: &Cell) -> Vec<u8> {
    let mut parts: Vec<String> = alloc::vec![String::from("0")];
    for (bit, code) in [(BOLD, "1"), (DIM, "2"), (ITALIC, "3"), (UNDERLINE, "4"), (INVERSE, "7"), (HIDDEN, "8"), (STRIKE, "9")] {
        if cell.flags & bit != 0 { parts.push(String::from(code)); }
    }
    for (color, base, bright, ext) in [(cell.fg, 30, 90, 38), (cell.bg, 40, 100, 48)] {
        match color {
            Color::Default => {}
            Color::Indexed(i) if i < 8 => parts.push(format!("{}", base + i as u32)),
            Color::Indexed(i) if i < 16 => parts.push(format!("{}", bright + i as u32 - 8)),
            Color::Indexed(i) => parts.push(format!("{};5;{}", ext, i)),
            Color::Rgb(r, g, b) => parts.push(format!("{};2;{};{};{}", ext, r, g, b)),
        }
    }
    format!("\x1b[{}m", parts.join(";")).into_bytes()
}

/// One line's cells with their colours, its trailing blanks left out.
fn encode(line: &Line) -> Vec<u8> {
    let blank = |c: &Cell| c.ch == ' ' && c.bg == Color::Default && c.flags & (INVERSE | UNDERLINE | STRIKE) == 0;
    let end = line.cells.iter().rposition(|c| !blank(c)).map_or(0, |i| i + 1);
    let mut out = Vec::new();
    let mut pen = Cell::BLANK;
    for c in &line.cells[..end] {
        if c.ch == SPACER { continue; }
        if (c.fg, c.bg, c.flags & !WIDE) != (pen.fg, pen.bg, pen.flags & !WIDE) { out.extend_from_slice(&sgr(c)); pen = *c; }
        let mut b = [0u8; 4];
        out.extend_from_slice(c.ch.encode_utf8(&mut b).as_bytes());
    }
    if (pen.fg, pen.bg, pen.flags & !WIDE) != (Color::Default, Color::Default, 0) { out.extend_from_slice(b"\x1b[0m"); }
    out
}

impl Screen {
    /// What this screen shows, as at most `max` bytes that draw it again on a fresh
    /// screen: the lines up to the cursor's (the newest first to stay, then as much
    /// scrollback as fits), their colours, the cursor's place and the modes a program
    /// set (application cursor keys, bracketed paste, a hidden cursor, the mouse). A
    /// full-screen program's alternate screen comes back as that screen, row by row.
    pub fn replay(&self, max: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let (x, y) = (self.x.min(self.cols.saturating_sub(1)), self.y.min(self.rows.saturating_sub(1)));
        if self.alternate() { out.extend_from_slice(b"\x1b[?1049h\x1b[H\x1b[2J"); }
        // The modes first: what follows only places the cursor, which they must not move.
        for (on, mode) in [(self.app_cursor, "1"), (self.bracketed_paste, "2004"), (!self.cursor_visible, "25")] {
            if on { out.extend_from_slice(format!("\x1b[?{}{}", mode, if mode == "25" { "l" } else { "h" }).as_bytes()); }
        }
        if self.mouse != 0 { out.extend_from_slice(format!("\x1b[?{}h", self.mouse).as_bytes()); }
        if self.mouse_sgr { out.extend_from_slice(b"\x1b[?1006h"); }
        if self.alternate() {
            for (r, line) in self.lines.iter().enumerate() {
                let text = encode(line);
                if text.is_empty() { continue; }
                if out.len() + text.len() + 16 > max { break; }
                out.extend_from_slice(format!("\x1b[{};1H", r + 1).as_bytes());
                out.extend_from_slice(&text);
            }
            out.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
        } else {
            // History and screen down to the cursor's row, newest kept first.
            let last = self.history.len() + y;
            let mut pieces: Vec<Vec<u8>> = Vec::new();
            let mut used = 0;
            for row in (0..=last).rev() {
                let Some(line) = self.abs_line(row) else { break };
                let mut text = encode(line);
                // A soft-wrapped line continues on the next: no line break, so the new
                // screen wraps it at its own width.
                if row != last && !line.wrapped { text.extend_from_slice(b"\r\n"); }
                if used + text.len() + 32 > max { break; }
                used += text.len();
                pieces.push(text);
            }
            for p in pieces.iter().rev() { out.extend_from_slice(p); }
            out.push(b'\r');
            if x > 0 { out.extend_from_slice(format!("\x1b[{}C", x).as_bytes()); }
        }
        // A character just filled the last column: the next one wraps first. Placing the
        // cursor forgets that, so the last column's character is written again last.
        if self.pending_wrap && x + 1 == self.cols {
            let line = if self.alternate() { self.lines.get(y) } else { self.abs_line(self.history.len() + y) };
            if let Some(cell) = line.and_then(|l| l.cells.get(x)).filter(|c| c.ch != SPACER) {
                let mut b = [0u8; 4];
                out.extend_from_slice(&sgr(cell));
                out.extend_from_slice(cell.ch.encode_utf8(&mut b).as_bytes());
                out.extend_from_slice(b"\x1b[0m");
            }
        }
        out
    }
}
