// SPDX-License-Identifier: GPL-3.0-only
//! Shared Ratatui-style ANSI drawing primitives: rounded borders, TrueColor gradient
//! sub-cell progress bars, Braille sparklines, Braille spinners, and ANSI-aware padding.

use alloc::string::String;
use core::fmt::Write;

use crate::width::char_width;

pub const BG: u32 = 0x0A0B0F;
pub const PANEL: u32 = 0x161922;
pub const SEL_BG: u32 = 0x24283B;
pub const BORDER: u32 = 0x334155;
pub const ROSE: u32 = 0xF43F5E;
pub const CORAL: u32 = 0xFB7185;
pub const CYAN: u32 = 0x22D3EE;
pub const BLUE: u32 = 0x60A5FA;
pub const EMERALD: u32 = 0x10B981;
pub const MINT: u32 = 0x34D399;
pub const AMBER: u32 = 0xF59E0B;
pub const LAVENDER: u32 = 0xC084FC;
pub const FROST: u32 = 0xECEFF4;
pub const MUTE: u32 = 0x64748B;
pub const TRACK: u32 = 0x283042;

const SPINNERS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const EIGHTHS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

#[inline]
pub fn spinner(frame: usize) -> &'static str {
    SPINNERS[frame % SPINNERS.len()]
}

#[inline]
pub fn push_fg(out: &mut String, rgb: u32) {
    let _ = write!(out, "\x1b[38;2;{};{};{}m", (rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
}

#[inline]
pub fn push_bg(out: &mut String, rgb: u32) {
    let _ = write!(out, "\x1b[48;2;{};{};{}m", (rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
}

#[inline]
pub fn fg(rgb: u32) -> String {
    let mut s = String::new();
    push_fg(&mut s, rgb);
    s
}

#[inline]
pub fn bg(rgb: u32) -> String {
    let mut s = String::new();
    push_bg(&mut s, rgb);
    s
}

/// Linearly interpolates between `a` and `b` (`t_pct` clamped to `0..=100`).
pub fn lerp_rgb(a: u32, b: u32, t_pct: usize) -> u32 {
    let t = t_pct.min(100) as i32;
    let ar = ((a >> 16) & 0xFF) as i32;
    let ag = ((a >> 8) & 0xFF) as i32;
    let ab = (a & 0xFF) as i32;
    let br = ((b >> 16) & 0xFF) as i32;
    let bgc = ((b >> 8) & 0xFF) as i32;
    let bb = (b & 0xFF) as i32;
    let r = (ar + (br - ar) * t / 100).clamp(0, 255) as u32;
    let g = (ag + (bgc - ag) * t / 100).clamp(0, 255) as u32;
    let b_ch = (ab + (bb - ab) * t / 100).clamp(0, 255) as u32;
    (r << 16) | (g << 8) | b_ch
}

/// Visible terminal cells of a string, skipping CSI `\x1b[...m` sequences.
pub fn ansi_width(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut cols = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            i += 2;
            while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        cols += char_width(ch);
        i += ch.len_utf8();
    }
    cols
}

/// Clips `s` to at most `max_w` visible cells while preserving preceding CSI SGR colors.
pub fn truncate_ansi(s: &str, max_w: usize) -> String {
    if ansi_width(s) <= max_w {
        return String::from(s);
    }
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let mut cols = 0;
    let mut saw_csi = false;
    let limit = max_w.saturating_sub(1);
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let start = i;
            i += 2;
            while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            out.push_str(&s[start..i]);
            saw_csi = true;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        let w = char_width(ch);
        if cols + w > limit {
            if max_w > 0 {
                out.push('…');
            }
            break;
        }
        out.push(ch);
        cols += w;
        i += ch.len_utf8();
    }
    if saw_csi {
        out.push_str("\x1b[0m");
    }
    out
}

/// Clips `s` to `width` visual cells and pads the remainder with spaces.
pub fn pad_ansi(s: &str, width: usize) -> String {
    let mut out = truncate_ansi(s, width);
    let w = ansi_width(&out);
    for _ in w..width {
        out.push(' ');
    }
    out
}

/// Sub-cell 1/8th-precision TrueColor gradient progress bar (`█▉▊▋▌▍▎▏` + dim track).
pub fn smooth_bar(width: usize, pct: usize, c0: u32, c1: u32) -> String {
    if width == 0 {
        return String::new();
    }
    let pct = pct.min(100);
    let total_eighths = pct * width * 8 / 100;
    let full = (total_eighths / 8).min(width);
    let rem = total_eighths % 8;
    let mut out = String::with_capacity(width * 22);
    for col in 0..width {
        let grad = lerp_rgb(c0, c1, col * 100 / width.max(1));
        if col < full {
            push_fg(&mut out, grad);
            out.push('█');
        } else if col == full && rem > 0 {
            push_fg(&mut out, grad);
            out.push(EIGHTHS[rem]);
        } else {
            push_fg(&mut out, TRACK);
            out.push('░');
        }
    }
    out.push_str("\x1b[0m");
    out
}

/// Dual-column Unicode Braille sparkline (`⠀..⣿`), packing 2 time samples per cell.
pub fn braille_spark(samples: &[u8], cols: usize, c0: u32, c1: u32) -> String {
    if cols == 0 {
        return String::new();
    }
    // 4 vertical dots per half-cell: bottom-to-top bit masks.
    const LEFT_BITS: [u32; 4] = [0x40, 0x04, 0x02, 0x01];
    const RIGHT_BITS: [u32; 4] = [0x80, 0x20, 0x10, 0x08];
    let needed = cols * 2;
    let pad = needed.saturating_sub(samples.len());
    let sample_at = |slot: usize| -> u8 {
        if slot < pad {
            0
        } else {
            samples[(slot - pad) + samples.len().saturating_sub(needed)].min(100)
        }
    };
    let dots = |v: u8| -> usize {
        if v == 0 {
            0
        } else {
            ((v as usize * 4 + 99) / 100).clamp(1, 4)
        }
    };
    let mut out = String::with_capacity(cols * 20);
    for c in 0..cols {
        let a = sample_at(c * 2);
        let b = sample_at(c * 2 + 1);
        let da = dots(a);
        let db = dots(b);
        let mut mask = 0x2800u32;
        for row in 0..da {
            mask |= LEFT_BITS[row];
        }
        for row in 0..db {
            mask |= RIGHT_BITS[row];
        }
        let peak = a.max(b) as usize;
        push_fg(&mut out, lerp_rgb(c0, c1, peak));
        out.push(char::from_u32(mask).unwrap_or('⠀'));
    }
    out.push_str("\x1b[0m");
    out
}

/// Rounded top box border: `╭─ title ─────── badge ─╮`.
pub fn box_top(title: &str, badge: &str, width: usize, border: u32, title_col: u32, badge_col: u32) -> String {
    if width < 4 {
        return String::from("╭──╮");
    }
    let inner = width - 2;
    let t_max = if badge.is_empty() { inner.saturating_sub(3) } else { inner / 2 };
    let t = crate::width::truncate(title, t_max);
    let tw = crate::width::str_width(&t);
    let b_max = inner.saturating_sub(tw + 6);
    let b = crate::width::truncate(badge, b_max);
    let bw = crate::width::str_width(&b);

    let mut out = String::new();
    push_fg(&mut out, border);
    out.push_str("╭─ ");
    push_fg(&mut out, title_col);
    out.push_str("\x1b[1m");
    out.push_str(&t);
    out.push_str("\x1b[0m");
    push_fg(&mut out, border);
    if bw > 0 {
        let fill = inner.saturating_sub(tw + bw + 5).max(1);
        out.push(' ');
        for _ in 0..fill {
            out.push('─');
        }
        out.push(' ');
        push_fg(&mut out, badge_col);
        out.push_str(&b);
        push_fg(&mut out, border);
        out.push_str(" ─╮\x1b[0m");
    } else {
        let fill = inner.saturating_sub(tw + 3).max(1);
        out.push(' ');
        for _ in 0..fill {
            out.push('─');
        }
        out.push_str("╮\x1b[0m");
    }
    out
}

/// Framed middle line inside a rounded box of total cell width `width`.
pub fn box_row(body_ansi: &str, width: usize, border: u32) -> String {
    if width < 4 {
        return String::from("││");
    }
    let inner = width - 4;
    let padded = pad_ansi(body_ansi, inner);
    let mut out = String::new();
    push_fg(&mut out, border);
    out.push_str("│ \x1b[0m");
    out.push_str(&padded);
    push_fg(&mut out, border);
    out.push_str(" │\x1b[0m");
    out
}

/// Horizontal separator `├─...─┤`.
pub fn box_sep(width: usize, border: u32) -> String {
    let mut out = String::new();
    push_fg(&mut out, border);
    out.push('├');
    for _ in 0..width.saturating_sub(2) {
        out.push('─');
    }
    out.push_str("┤\x1b[0m");
    out
}

/// Rounded bottom box border: `╰─ footer ──────╯`.
pub fn box_bot(footer: &str, width: usize, border: u32, footer_col: u32) -> String {
    if width < 4 {
        return String::from("╰──╯");
    }
    let inner = width - 2;
    let mut out = String::new();
    push_fg(&mut out, border);
    out.push('╰');
    if footer.is_empty() {
        for _ in 0..inner {
            out.push('─');
        }
    } else {
        let f = crate::width::truncate(footer, inner.saturating_sub(4));
        let fw = crate::width::str_width(&f);
        let fill = inner.saturating_sub(fw + 3);
        out.push_str("─ ");
        push_fg(&mut out, footer_col);
        out.push_str(&f);
        push_fg(&mut out, border);
        out.push(' ');
        for _ in 0..fill {
            out.push('─');
        }
    }
    out.push_str("╯\x1b[0m");
    out
}
