// SPDX-License-Identifier: GPL-3.0-only
//! Antigravity-style Candy Lavender pixel UI for Tailor:
//! - Instant <50ms pixel frame rendering (`window.show_pixels`)
//! - Clickable top-bar provider pills (`Antigravity`, `Claude`, `DeepSeek`, `Codex`, `Gemini`)
//! - Live Shibari key / OAuth / Proxy badge + active task boundary mode badge
//! - Scrollable chat & agent transcript panel with color-coded roles and unified diffs
//! - Clickable `Approve` / `Deny` buttons for pending file-edit tool approvals
//! - Bottom prompt input well with selection highlight (`Ctrl+A/C/V`) + `Send` button

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::Window;
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_theme::candy_ui::{
    draw_candy_badge, draw_candy_button, draw_candy_panel, draw_candy_tab, draw_candy_toolbar,
    fill_rounded_rect, ButtonState, ButtonStyle, CandyPalette,
};
use tailor_core::{AgentMode, Provider, TaskBoundary};

#[derive(Clone, Copy)]
pub enum HitAction {
    Provider(Provider),
    Approve,
    Deny,
    Send,
    Clear,
}

#[derive(Clone, Copy)]
pub struct HitBox {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub action: HitAction,
}

impl HitBox {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

pub struct TailorUi {
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
    pub hits: Vec<HitBox>,
}

impl TailorUi {
    pub fn new() -> Self {
        Self {
            font: Font::new(sex_font::GEIST_SANS),
            acc: Vec::new(),
            glyph: Vec::new(),
            hits: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        text: &str,
        color: u32,
    ) {
        let Some(font) = self.font.as_ref() else {
            return;
        };
        let px_size = px_h.max(10) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let line_h = ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let cr = ((color >> 16) & 0xFF) as u32;
        let cg = ((color >> 8) & 0xFF) as u32;
        let cb = (color & 0xFF) as u32;
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit - 6 {
                break;
            }
            let Some(id) = font.glyph(ch as u32) else {
                pen += (px_h / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            let adv = font.advance(id, size);
            let gx0 = (pen / FX) - pad as i32;
            let gw = ((adv / FX).max(0) as usize + 2 * pad + 2).min(64);
            if gw > 0 && line_h > 0 && line_h <= 64 {
                let slen = scratch_len(gw, line_h);
                if self.acc.len() < slen {
                    self.acc.resize(slen, 0);
                }
                if self.glyph.len() < gw * line_h {
                    self.glyph.resize(gw * line_h, 0);
                }
                for b in &mut self.glyph[..gw * line_h] {
                    *b = 0;
                }
                let sub = pen - (pen / FX) * FX + pad as i32 * FX;
                font.draw(
                    id,
                    size,
                    sub,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut self.glyph[..gw * line_h],
                );
                for row in 0..line_h {
                    let py = y + row as i32;
                    if py < 0 || py >= bh as i32 {
                        continue;
                    }
                    let row_off = py as usize * bw;
                    for col in 0..gw {
                        let px = gx0 + col as i32;
                        if px < 0 || px >= limit {
                            continue;
                        }
                        let a = self.glyph[row * gw + col] as u32;
                        if a == 0 {
                            continue;
                        }
                        let dst = buf[row_off + px as usize];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        let inv = 255 - a;
                        let nr = (cr * a + dr * inv) / 255;
                        let ng = (cg * a + dg * inv) / 255;
                        let nb = (cb * a + db * inv) / 255;
                        buf[row_off + px as usize] = (nr << 16) | (ng << 8) | nb;
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        window: &mut Window,
        provider: Provider,
        model: &str,
        auth_label: &str,
        auth_color: u32,
        streaming: bool,
        task_boundary: Option<&TaskBoundary>,
        pending_approval: Option<&str>,
        transcript: &str,
        scroll_lines: usize,
        input: &str,
        input_selected: bool,
        status: &str,
    ) {
        if !window.ready() {
            return;
        }
        let (w, h) = window.size();
        if w < 120 || h < 120 {
            return;
        }
        self.hits.clear();

        let pal = CandyPalette::dark();
        let buf = window.frame(w, h);
        buf.fill(pal.bg);

        let header_h = 48usize;
        let approval_h = if pending_approval.is_some() { 44usize } else { 0usize };
        let input_h = 72usize;
        let body_y = header_h + 4;
        let body_h = h.saturating_sub(header_h + approval_h + input_h + 12);

        // 1. Top Antigravity-style Toolbar
        draw_candy_toolbar(buf, w, h, 8, 8, (w - 16) as i32, (header_h - 10) as i32, &pal);
        draw_candy_badge(buf, w, h, 16, 15, 78, 24, 0x00D6_336C, &pal);
        self.draw_text(buf, w, h, 26, 19, 68, 13, "TAILOR", 0x00FF_FFFF);

        // Auth / Proxy Status Badge width computed first so pills don't overlap it
        let badge_w = ((auth_label.len() as i32) * 7 + 22).clamp(102, 210);
        let badge_x = (w as i32 - badge_w - 18).max(110);

        // Clickable Provider Pills
        let pills: [(Provider, &str, i32); 5] = [
            (Provider::Antigravity, "Antigravity", 82),
            (Provider::Claude, "Claude", 58),
            (Provider::ClaudeDeepSeek, "DeepSeek", 74),
            (Provider::Codex, "Codex", 54),
            (Provider::Gemini, "Gemini", 58),
        ];
        let mut px = 104i32;
        for (p, label, pw) in pills {
            if px + pw > badge_x - 8 {
                break;
            }
            let active = p == provider;
            draw_candy_tab(buf, w, h, px, 14, pw, 26, active, ButtonState::Rest, &pal);
            self.draw_text(
                buf,
                w,
                h,
                px + 8,
                19,
                pw - 10,
                12,
                label,
                if active { pal.text } else { pal.text_dim },
            );
            self.hits.push(HitBox {
                x: px,
                y: 14,
                w: pw,
                h: 26,
                action: HitAction::Provider(p),
            });
            px += pw + 5;
        }

        // Auth / Proxy Status Badge on right
        let final_badge_x = badge_x.max(px + 8);
        let final_badge_w = (w as i32 - final_badge_x - 16).min(badge_w).max(60);
        draw_candy_badge(buf, w, h, final_badge_x, 15, final_badge_w, 24, auth_color, &pal);
        self.draw_text(
            buf,
            w,
            h,
            final_badge_x + 10,
            19,
            final_badge_w - 14,
            12,
            auth_label,
            0x00FF_FFFF,
        );

        // 2. Main Chat & Agent Transcript Panel
        draw_candy_panel(buf, w, h, 8, body_y as i32, (w - 16) as i32, body_h as i32, 12, &pal);

        // Wrap transcript lines to fit panel width
        let inner_w = (w.saturating_sub(48)).max(120) as i32;
        let max_chars = ((inner_w / 7).max(24)) as usize;
        let mut wrapped: Vec<(u32, String)> = Vec::new();
        for raw_line in transcript.lines() {
            let color = if raw_line.starts_with("> ") {
                pal.accent
            } else if raw_line.starts_with("@@")
                || raw_line.starts_with("--- ")
                || raw_line.starts_with("+++ ")
            {
                0x0038_BDF8
            } else if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
                0x004A_DE80
            } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
                0x00F8_7171
            } else if raw_line.starts_with("[approval")
                || raw_line.starts_with("[APPROVAL")
                || raw_line.starts_with("[tool")
            {
                0x00F5_B971
            } else if raw_line.starts_with("[tailor]") || raw_line.starts_with("[error") {
                0x00C6_A0F6
            } else {
                pal.text
            };
            if raw_line.is_empty() {
                wrapped.push((color, String::new()));
                continue;
            }
            let mut cur = raw_line;
            while cur.len() > max_chars {
                let mut split = max_chars;
                while split > 0 && !cur.is_char_boundary(split) {
                    split -= 1;
                }
                if let Some(sp) = cur[..split].rfind(' ') {
                    if sp > max_chars / 2 {
                        split = sp + 1;
                    }
                }
                wrapped.push((color, String::from(cur[..split].trim_end())));
                cur = &cur[split..];
            }
            wrapped.push((color, String::from(cur)));
        }

        let line_h = 20usize;
        let max_rows = body_h.saturating_sub(24) / line_h;
        let end = wrapped.len().saturating_sub(scroll_lines);
        let start = end.saturating_sub(max_rows);
        let mut ly = body_y as i32 + 12;
        for (col, line) in &wrapped[start..end] {
            if !line.is_empty() {
                self.draw_text(buf, w, h, 22, ly, inner_w, 13, line, *col);
            }
            ly += line_h as i32;
        }

        // 3. Pending File-Edit Approval Bar (when a mutating tool needs confirmation)
        if let Some(summary) = pending_approval {
            let ay = (body_y + body_h + 6) as i32;
            draw_candy_panel(buf, w, h, 8, ay, (w - 16) as i32, (approval_h - 6) as i32, 10, &pal);
            let prompt = format!("Approve: {}", summary);
            self.draw_text(
                buf, w, h, 22, ay + 10, (w as i32 - 224).max(100), 13,
                &prompt,
                0x00F5_B971,
            );
            let ax = (w as i32 - 196).max(180);
            draw_candy_button(buf, w, h, ax, ay + 6, 86, 26, 8, ButtonStyle::Primary, ButtonState::Rest, &pal);
            self.draw_text(buf, w, h, ax + 14, ay + 11, 70, 12, "Approve", 0x00FF_FFFF);
            self.hits.push(HitBox { x: ax, y: ay + 6, w: 86, h: 26, action: HitAction::Approve });

            let dx = ax + 94;
            draw_candy_button(buf, w, h, dx, ay + 6, 78, 26, 8, ButtonStyle::Subtle, ButtonState::Rest, &pal);
            self.draw_text(buf, w, h, dx + 20, ay + 11, 60, 12, "Deny", pal.text);
            self.hits.push(HitBox { x: dx, y: ay + 6, w: 78, h: 26, action: HitAction::Deny });
        }

        // 4. Bottom Prompt Input Bar
        let iy = (h - input_h) as i32 + 2;
        let ih = (input_h - 10) as i32;
        draw_candy_panel(buf, w, h, 8, iy, (w - 16) as i32, ih, 12, &pal);

        // Input well
        let well_x = 18i32;
        let well_y = iy + 8;
        let well_w = (w as i32 - 178).max(100);
        let well_h = 28i32;
        fill_rounded_rect(buf, w, h, well_x, well_y, well_w, well_h, 8, pal.well);
        if input_selected && !input.is_empty() {
            fill_rounded_rect(buf, w, h, well_x + 4, well_y + 4, well_w - 8, well_h - 8, 5, pal.selection);
        }
        let prompt_text = if input.is_empty() {
            String::from("Ask Tailor anything, or type /provider antigravity, /proxy <url>, /key ...")
        } else {
            format!("{}|", input)
        };
        let prompt_col = if input.is_empty() { pal.text_dim } else { pal.text };
        self.draw_text(buf, w, h, well_x + 10, well_y + 6, well_w - 20, 13, &prompt_text, prompt_col);

        // Send & Clear buttons
        let sx = well_x + well_w + 8;
        draw_candy_button(buf, w, h, sx, well_y, 72, well_h, 8, ButtonStyle::Primary, ButtonState::Rest, &pal);
        let send_lbl = if streaming { "..." } else { "Send" };
        self.draw_text(buf, w, h, sx + 18, well_y + 6, 56, 13, send_lbl, 0x00FF_FFFF);
        self.hits.push(HitBox { x: sx, y: well_y, w: 72, h: well_h, action: HitAction::Send });

        let cx = sx + 80;
        draw_candy_button(buf, w, h, cx, well_y, 62, well_h, 8, ButtonStyle::Subtle, ButtonState::Rest, &pal);
        self.draw_text(buf, w, h, cx + 12, well_y + 6, 50, 12, "Clear", pal.text_dim);
        self.hits.push(HitBox { x: cx, y: well_y, w: 62, h: well_h, action: HitAction::Clear });

        // Status footer line (with optional active task_boundary mode badge)
        let mut fx = 20i32;
        let fy = iy + 41;
        let mut remaining_w = (w as i32 - 40).max(100);
        if let Some(tb) = task_boundary {
            if tb.mode != AgentMode::Idle {
                let (mode_tag, mode_col) = match tb.mode {
                    AgentMode::Planning => ("[PLANNING]", 0x0038_BDF8),
                    AgentMode::Execution => ("[EXECUTION]", 0x00A8_55F7),
                    AgentMode::Verification => ("[VERIFICATION]", 0x002E_A043),
                    AgentMode::Idle => ("[IDLE]", pal.text_dim),
                };
                let tag_w = (mode_tag.len() as i32) * 7 + 8;
                self.draw_text(buf, w, h, fx, fy, tag_w.min(remaining_w), 11, mode_tag, mode_col);
                fx += tag_w;
                remaining_w = (remaining_w - tag_w).max(40);
                let tb_info = if !tb.task_status.is_empty() {
                    tb.task_status.as_str()
                } else {
                    tb.task_name.as_str()
                };
                let footer = if tb_info.is_empty() {
                    format!("{} · {}  |  Ctrl+A/C/V", model, status)
                } else {
                    format!("{} · {} · {}  |  Ctrl+A/C/V", tb_info, model, status)
                };
                self.draw_text(buf, w, h, fx, fy, remaining_w, 11, &footer, pal.text_dim);
                window.show_pixels(w, h);
                return;
            }
        }
        let footer = format!("{} · {}  |  Ctrl+A/C/V", model, status);
        self.draw_text(buf, w, h, fx, fy, remaining_w, 11, &footer, pal.text_dim);

        window.show_pixels(w, h);
    }
}
