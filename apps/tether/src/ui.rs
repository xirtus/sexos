// SPDX-License-Identifier: GPL-3.0-only
//! Obsidian + Candy Lavender 3-Pane Antigravity Tether UI for SexOS:
//! - Top Toolbar (44px): TETHER brand, Connection status pill, Capability tier pill
//!   (Observe / Dev / Full Control), Connect/Disconnect button, and Kill Switch button
//! - Left Pane (~210px, responsive): Waves (fanned-out subagents) / Snaps (checkpoints) / Leases
//! - Center Pane: ACP Chat (streaming transcript + inline Permission Gate + Composer),
//!   Diff Review (pending Wave Review unified diff + Approve/Reject), and Console (RPC/log stream)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::{RichTextInput, TextFieldPalette, TextFieldRect, Window};
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_theme::candy_ui::{
    draw_candy_button, fill_rounded_rect, ButtonState, ButtonStyle, CandyPalette,
};
use tether_proto::rpc::CapabilityMode;

const BG_CANVAS: u32 = 0x000B_0D14;
const BG_PANE: u32 = 0x0011_1420;
const BG_CARD: u32 = 0x0013_1726;
const BG_ELEVATED: u32 = 0x001A_2034;
const BG_ACTIVE_ROW: u32 = 0x0022_2842;
const BG_INPUT: u32 = 0x000B_0D14;
const BG_APPROVAL: u32 = 0x0026_1E10;
const BG_KILL: u32 = 0x003B_141E;
const BG_DIFF_ADD: u32 = 0x0011_281D;
const BG_DIFF_DEL: u32 = 0x002C_141C;

const BORDER_SUBTLE: u32 = 0x0023_283D;
const LAVENDER: u32 = 0x00C6_A0F6;
const TEXT_PRIMARY: u32 = 0x00F1_F5F9;
const TEXT_MUTED: u32 = 0x0094_A3B8;
const TEXT_DIM: u32 = 0x0064_748B;

const COL_EMERALD: u32 = 0x004A_DE80;
const COL_ROSE: u32 = 0x00F8_7171;
const COL_SKY: u32 = 0x0038_BDF8;
const COL_AMBER: u32 = 0x00F5_B971;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftTab {
    Waves,
    Snaps,
    Leases,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterTab {
    Chat,
    Review,
    Console,
}

#[derive(Clone, Debug)]
pub struct WaveAgentCard {
    pub name: String,
    pub role: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct SnapshotCard {
    pub id: String,
    pub label: String,
    pub ticks: u64,
}

#[derive(Clone, Debug)]
pub struct PendingApproval {
    pub id: u64,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct PendingWaveReview {
    pub wave: String,
    pub summary: String,
    pub diff: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitAction {
    ToggleConnect,
    CycleCapability,
    ToggleKillSwitch,
    LeftTab(LeftTab),
    CenterTab(CenterTab),
    ApprovePermission,
    RejectPermission,
    ApproveWave,
    RejectWave,
    RestoreSnapshot(usize),
    CreateSnapshot,
    SendPrompt,
    CancelPrompt,
}

#[derive(Clone, Copy, Debug)]
pub struct HitBox {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub action: HitAction,
}

pub struct TetherUi {
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
    pub hits: Vec<HitBox>,
}

impl TetherUi {
    pub fn new() -> Self {
        Self {
            font: Font::new(sex_font::GEIST_SANS),
            acc: Vec::new(),
            glyph: Vec::new(),
            hits: Vec::new(),
        }
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<HitAction> {
        for h in self.hits.iter().rev() {
            if x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h {
                return Some(h.action);
            }
        }
        None
    }

    fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x + w).min(bw as i32).max(0) as usize;
        let y1 = (y + h).min(bh as i32).max(0) as usize;
        for row in y0..y1 {
            let off = row * bw;
            for col in x0..x1 {
                buf[off + col] = color;
            }
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
        let line_h =
            ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
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
        connected: bool,
        host_label: &str,
        agent_name: &str,
        capability: CapabilityMode,
        kill_switch: bool,
        left_tab: LeftTab,
        center_tab: CenterTab,
        waves: &[WaveAgentCard],
        snapshots: &[SnapshotCard],
        leases: &[String],
        transcript: &[String],
        console: &[String],
        approval: Option<&PendingApproval>,
        wave_review: Option<&PendingWaveReview>,
        input: &mut RichTextInput,
        status_line: &str,
        scroll_lines: usize,
    ) {
        let (w, h) = window.size();
        if w < 160 || h < 120 {
            return;
        }
        let buf = window.frame(w, h);
        let pal = CandyPalette::dark();
        self.hits.clear();

        // Full canvas background
        Self::fill_rect(buf, w, h, 0, 0, w as i32, h as i32, BG_CANVAS);

        // ── Top Toolbar (44px) ───────────────────────────────────────────────
        let tb_h = 44i32;
        Self::fill_rect(buf, w, h, 0, 0, w as i32, tb_h, BG_PANE);
        Self::fill_rect(buf, w, h, 0, tb_h - 1, w as i32, 1, BORDER_SUBTLE);

        // Brand badge
        fill_rounded_rect(buf, w, h, 12, 8, 86, 28, 8, BG_ACTIVE_ROW);
        self.draw_text(buf, w, h, 22, 14, 72, 14, "TETHER", LAVENDER);

        // Connection status pill
        let conn_x = 108i32;
        let conn_w = 190i32.min((w as i32 - 360).max(110));
        fill_rounded_rect(buf, w, h, conn_x, 8, conn_w, 28, 8, BG_CARD);
        let dot_col = if kill_switch {
            COL_ROSE
        } else if connected {
            COL_EMERALD
        } else {
            COL_AMBER
        };
        fill_rounded_rect(buf, w, h, conn_x + 10, 18, 8, 8, 4, dot_col);
        let conn_text = if connected {
            format!("{} ({})", agent_name, host_label)
        } else {
            format!("Offline: {}", host_label)
        };
        self.draw_text(buf, w, h, conn_x + 24, 14, conn_w - 30, 13, &conn_text, TEXT_PRIMARY);
        self.hits.push(HitBox {
            x: conn_x,
            y: 8,
            w: conn_w,
            h: 28,
            action: HitAction::ToggleConnect,
        });

        // Capability pill
        let cap_x = conn_x + conn_w + 10;
        let cap_w = 114i32;
        let cap_col = match capability {
            CapabilityMode::Observe => COL_SKY,
            CapabilityMode::Dev => COL_AMBER,
            CapabilityMode::FullControl => COL_EMERALD,
        };
        fill_rounded_rect(buf, w, h, cap_x, 8, cap_w, 28, 8, BG_ELEVATED);
        self.draw_text(buf, w, h, cap_x + 10, 14, cap_w - 16, 13, capability.label(), cap_col);
        self.hits.push(HitBox {
            x: cap_x,
            y: 8,
            w: cap_w,
            h: 28,
            action: HitAction::CycleCapability,
        });

        // Kill switch button (right-aligned)
        let kill_w = 108i32;
        let kill_x = w as i32 - kill_w - 12;
        fill_rounded_rect(
            buf,
            w,
            h,
            kill_x,
            8,
            kill_w,
            28,
            8,
            if kill_switch { COL_ROSE } else { BG_KILL },
        );
        self.draw_text(
            buf,
            w,
            h,
            kill_x + 10,
            14,
            kill_w - 14,
            13,
            if kill_switch { "KILLED [F12]" } else { "KILL [F12]" },
            if kill_switch { BG_CANVAS } else { COL_ROSE },
        );
        self.hits.push(HitBox {
            x: kill_x,
            y: 8,
            w: kill_w,
            h: 28,
            action: HitAction::ToggleKillSwitch,
        });

        // ── Body Geometry ────────────────────────────────────────────────────
        let status_h = 24i32;
        let body_y = tb_h;
        let body_h = (h as i32 - tb_h - status_h).max(80);
        let show_left = w >= 660;
        let left_w = if show_left { 220i32 } else { 0 };

        // ── Left Pane: Waves / Snaps / Leases ────────────────────────────────
        if show_left {
            Self::fill_rect(buf, w, h, 0, body_y, left_w, body_h, BG_PANE);
            Self::fill_rect(buf, w, h, left_w - 1, body_y, 1, body_h, BORDER_SUBTLE);

            let tabs = [
                (LeftTab::Waves, "Waves"),
                (LeftTab::Snaps, "Snaps"),
                (LeftTab::Leases, "Leases"),
            ];
            let tw = (left_w - 16) / 3;
            for (idx, (tab, label)) in tabs.iter().enumerate() {
                let tx = 8 + idx as i32 * tw;
                let active = *tab == left_tab;
                fill_rounded_rect(
                    buf,
                    w,
                    h,
                    tx,
                    body_y + 8,
                    tw - 4,
                    26,
                    6,
                    if active { BG_ACTIVE_ROW } else { BG_CARD },
                );
                self.draw_text(
                    buf,
                    w,
                    h,
                    tx + 8,
                    body_y + 13,
                    tw - 10,
                    12,
                    label,
                    if active { LAVENDER } else { TEXT_MUTED },
                );
                self.hits.push(HitBox {
                    x: tx,
                    y: body_y + 8,
                    w: tw - 4,
                    h: 26,
                    action: HitAction::LeftTab(*tab),
                });
            }

            let mut cy = body_y + 44;
            match left_tab {
                LeftTab::Waves => {
                    if waves.is_empty() {
                        self.draw_text(
                            buf,
                            w,
                            h,
                            12,
                            cy,
                            left_w - 24,
                            12,
                            "No active subagent waves.",
                            TEXT_DIM,
                        );
                    } else {
                        for agent in waves.iter().take(10) {
                            if cy + 44 > body_y + body_h {
                                break;
                            }
                            fill_rounded_rect(buf, w, h, 8, cy, left_w - 16, 40, 8, BG_CARD);
                            self.draw_text(buf, w, h, 16, cy + 5, left_w - 32, 12, &agent.name, TEXT_PRIMARY);
                            let sub = format!("{} · {}", agent.role, agent.status);
                            self.draw_text(buf, w, h, 16, cy + 21, left_w - 32, 11, &sub, COL_SKY);
                            cy += 46;
                        }
                    }
                }
                LeftTab::Snaps => {
                    draw_candy_button(
                        buf,
                        w,
                        h,
                        8,
                        cy,
                        left_w - 16,
                        26,
                        7,
                        ButtonStyle::Primary,
                        ButtonState::Rest,
                        &pal,
                    );
                    self.draw_text(buf, w, h, 22, cy + 6, left_w - 32, 12, "+ Create Snapshot", TEXT_PRIMARY);
                    self.hits.push(HitBox {
                        x: 8,
                        y: cy,
                        w: left_w - 16,
                        h: 26,
                        action: HitAction::CreateSnapshot,
                    });
                    cy += 34;
                    for (idx, snap) in snapshots.iter().rev().take(8).enumerate() {
                        if cy + 42 > body_y + body_h {
                            break;
                        }
                        fill_rounded_rect(buf, w, h, 8, cy, left_w - 16, 38, 8, BG_CARD);
                        self.draw_text(buf, w, h, 14, cy + 4, left_w - 72, 12, &snap.label, TEXT_PRIMARY);
                        self.draw_text(buf, w, h, 14, cy + 20, left_w - 72, 11, &snap.id, TEXT_DIM);
                        let rx = left_w - 58;
                        fill_rounded_rect(buf, w, h, rx, cy + 8, 44, 22, 6, BG_ELEVATED);
                        self.draw_text(buf, w, h, rx + 7, cy + 12, 36, 11, "Undo", COL_AMBER);
                        let orig_idx = snapshots.len() - 1 - idx;
                        self.hits.push(HitBox {
                            x: rx,
                            y: cy + 8,
                            w: 44,
                            h: 22,
                            action: HitAction::RestoreSnapshot(orig_idx),
                        });
                        cy += 44;
                    }
                }
                LeftTab::Leases => {
                    if leases.is_empty() {
                        self.draw_text(buf, w, h, 12, cy, left_w - 24, 12, "No active MCP leases.", TEXT_DIM);
                    } else {
                        for lease in leases.iter().rev().take(12) {
                            if cy + 24 > body_y + body_h {
                                break;
                            }
                            self.draw_text(buf, w, h, 12, cy, left_w - 24, 12, lease, COL_EMERALD);
                            cy += 20;
                        }
                    }
                }
            }
        }

        // ── Center Pane: ACP Chat / Diff Review / Console ────────────────────
        let cx = left_w;
        let cw = (w as i32 - left_w).max(160);

        // Center tab bar (34px)
        Self::fill_rect(buf, w, h, cx, body_y, cw, 34, BG_PANE);
        Self::fill_rect(buf, w, h, cx, body_y + 33, cw, 1, BORDER_SUBTLE);
        let ctabs = [
            (CenterTab::Chat, "ACP Chat"),
            (CenterTab::Review, "Diff Review"),
            (CenterTab::Console, "Console"),
        ];
        for (idx, (tab, label)) in ctabs.iter().enumerate() {
            let tx = cx + 10 + idx as i32 * 108;
            let active = *tab == center_tab;
            fill_rounded_rect(
                buf,
                w,
                h,
                tx,
                body_y + 4,
                100,
                26,
                6,
                if active { BG_ACTIVE_ROW } else { BG_CARD },
            );
            self.draw_text(
                buf,
                w,
                h,
                tx + 12,
                body_y + 9,
                84,
                12,
                label,
                if active { LAVENDER } else { TEXT_MUTED },
            );
            self.hits.push(HitBox {
                x: tx,
                y: body_y + 4,
                w: 100,
                h: 26,
                action: HitAction::CenterTab(*tab),
            });
        }

        let content_y = body_y + 36;
        let composer_h = if center_tab == CenterTab::Chat { 48 } else { 0 };
        let approval_h = if approval.is_some() && center_tab == CenterTab::Chat { 44 } else { 0 };
        let content_h = (body_h - 36 - composer_h - approval_h).max(40);

        match center_tab {
            CenterTab::Chat => {
                let line_h = 18i32;
                let max_lines = (content_h / line_h).max(1) as usize;
                let total = transcript.len();
                let end = total.saturating_sub(scroll_lines);
                let start = end.saturating_sub(max_lines);
                let mut ly = content_y + 8;
                for line in &transcript[start..end] {
                    let col = if line.starts_with("You:") {
                        LAVENDER
                    } else if line.starts_with("[tool]") || line.starts_with("[rpc]") {
                        COL_SKY
                    } else if line.starts_with("[err]") || line.starts_with("[kill]") {
                        COL_ROSE
                    } else if line.starts_with("[thought]") {
                        TEXT_DIM
                    } else {
                        TEXT_PRIMARY
                    };
                    self.draw_text(buf, w, h, cx + 14, ly, cw - 28, 13, line, col);
                    ly += line_h;
                }

                // Inline ACP Approval Gate
                if let Some(ap) = approval {
                    let ay = content_y + content_h;
                    Self::fill_rect(buf, w, h, cx + 8, ay + 2, cw - 16, 38, BG_APPROVAL);
                    self.draw_text(
                        buf,
                        w,
                        h,
                        cx + 16,
                        ay + 12,
                        cw - 200,
                        13,
                        &format!("Approve: {}", ap.title),
                        COL_AMBER,
                    );
                    let ax = cx + cw - 178;
                    draw_candy_button(
                        buf,
                        w,
                        h,
                        ax,
                        ay + 8,
                        78,
                        26,
                        7,
                        ButtonStyle::Primary,
                        ButtonState::Rest,
                        &pal,
                    );
                    self.draw_text(buf, w, h, ax + 14, ay + 13, 60, 12, "Approve", TEXT_PRIMARY);
                    self.hits.push(HitBox {
                        x: ax,
                        y: ay + 8,
                        w: 78,
                        h: 26,
                        action: HitAction::ApprovePermission,
                    });
                    let rx = ax + 86;
                    draw_candy_button(
                        buf,
                        w,
                        h,
                        rx,
                        ay + 8,
                        72,
                        26,
                        7,
                        ButtonStyle::Subtle,
                        ButtonState::Rest,
                        &pal,
                    );
                    self.draw_text(buf, w, h, rx + 16, ay + 13, 50, 12, "Reject", COL_ROSE);
                    self.hits.push(HitBox {
                        x: rx,
                        y: ay + 8,
                        w: 72,
                        h: 26,
                        action: HitAction::RejectPermission,
                    });
                }

                // Prompt Composer
                let comp_y = body_y + body_h - composer_h;
                Self::fill_rect(buf, w, h, cx, comp_y, cw, composer_h, BG_PANE);
                Self::fill_rect(buf, w, h, cx, comp_y, cw, 1, BORDER_SUBTLE);
                let well_w = (cw - 168).max(80);
                let mut input_pal = TextFieldPalette::noir();
                input_pal.bg = BG_INPUT;
                input_pal.border = BORDER_SUBTLE;
                input_pal.text = TEXT_PRIMARY;
                input_pal.placeholder = TEXT_DIM;
                input.render(
                    buf,
                    w,
                    h,
                    TextFieldRect::new(cx + 10, comp_y + 8, well_w, 32),
                    input.focused,
                    &input_pal,
                );

                let sx = cx + 10 + well_w + 8;
                draw_candy_button(
                    buf,
                    w,
                    h,
                    sx,
                    comp_y + 8,
                    70,
                    32,
                    8,
                    ButtonStyle::Primary,
                    ButtonState::Rest,
                    &pal,
                );
                self.draw_text(buf, w, h, sx + 18, comp_y + 15, 50, 13, "Send", TEXT_PRIMARY);
                self.hits.push(HitBox {
                    x: sx,
                    y: comp_y + 8,
                    w: 70,
                    h: 32,
                    action: HitAction::SendPrompt,
                });

                let stop_x = sx + 78;
                draw_candy_button(
                    buf,
                    w,
                    h,
                    stop_x,
                    comp_y + 8,
                    62,
                    32,
                    8,
                    ButtonStyle::Subtle,
                    ButtonState::Rest,
                    &pal,
                );
                self.draw_text(buf, w, h, stop_x + 14, comp_y + 15, 44, 13, "Stop", TEXT_MUTED);
                self.hits.push(HitBox {
                    x: stop_x,
                    y: comp_y + 8,
                    w: 62,
                    h: 32,
                    action: HitAction::CancelPrompt,
                });
            }
            CenterTab::Review => {
                if let Some(wr) = wave_review {
                    let mut ly = content_y + 10;
                    fill_rounded_rect(buf, w, h, cx + 10, ly, cw - 20, 48, 8, BG_CARD);
                    self.draw_text(
                        buf,
                        w,
                        h,
                        cx + 20,
                        ly + 6,
                        cw - 240,
                        14,
                        &format!("Wave Review: {}", wr.wave),
                        LAVENDER,
                    );
                    self.draw_text(buf, w, h, cx + 20, ly + 25, cw - 240, 12, &wr.summary, TEXT_MUTED);

                    let ax = cx + cw - 204;
                    draw_candy_button(
                        buf,
                        w,
                        h,
                        ax,
                        ly + 10,
                        108,
                        28,
                        7,
                        ButtonStyle::Primary,
                        ButtonState::Rest,
                        &pal,
                    );
                    self.draw_text(buf, w, h, ax + 10, ly + 16, 94, 12, "Approve Wave", TEXT_PRIMARY);
                    self.hits.push(HitBox {
                        x: ax,
                        y: ly + 10,
                        w: 108,
                        h: 28,
                        action: HitAction::ApproveWave,
                    });
                    let rx = ax + 114;
                    draw_candy_button(
                        buf,
                        w,
                        h,
                        rx,
                        ly + 10,
                        72,
                        28,
                        7,
                        ButtonStyle::Subtle,
                        ButtonState::Rest,
                        &pal,
                    );
                    self.draw_text(buf, w, h, rx + 14, ly + 16, 54, 12, "Reject", COL_ROSE);
                    self.hits.push(HitBox {
                        x: rx,
                        y: ly + 10,
                        w: 72,
                        h: 28,
                        action: HitAction::RejectWave,
                    });
                    ly += 58;

                    for line in wr.diff.lines().take(((content_h - 68) / 17).max(1) as usize) {
                        let (bg, fg) = if line.starts_with('+') && !line.starts_with("+++") {
                            (Some(BG_DIFF_ADD), COL_EMERALD)
                        } else if line.starts_with('-') && !line.starts_with("---") {
                            (Some(BG_DIFF_DEL), COL_ROSE)
                        } else if line.starts_with("@@") {
                            (None, COL_SKY)
                        } else {
                            (None, TEXT_PRIMARY)
                        };
                        if let Some(bgc) = bg {
                            Self::fill_rect(buf, w, h, cx + 10, ly - 1, cw - 20, 17, bgc);
                        }
                        self.draw_text(buf, w, h, cx + 16, ly, cw - 32, 12, line, fg);
                        ly += 17;
                    }
                } else {
                    self.draw_text(
                        buf,
                        w,
                        h,
                        cx + 18,
                        content_y + 18,
                        cw - 36,
                        13,
                        "No pending Wave Review. When Antigravity stages a wave, diffs appear here.",
                        TEXT_DIM,
                    );
                }
            }
            CenterTab::Console => {
                let line_h = 17i32;
                let max_lines = (content_h / line_h).max(1) as usize;
                let start = console.len().saturating_sub(max_lines);
                let mut ly = content_y + 8;
                for line in &console[start..] {
                    self.draw_text(buf, w, h, cx + 14, ly, cw - 28, 12, line, TEXT_MUTED);
                    ly += line_h;
                }
            }
        }

        // ── Bottom Status Bar (24px) ─────────────────────────────────────────
        let sy = h as i32 - status_h;
        Self::fill_rect(buf, w, h, 0, sy, w as i32, status_h, BG_PANE);
        Self::fill_rect(buf, w, h, 0, sy, w as i32, 1, BORDER_SUBTLE);
        self.draw_text(buf, w, h, 12, sy + 5, w as i32 - 24, 11, status_line, TEXT_MUTED);

        let mut menu_pal = TextFieldPalette::noir();
        menu_pal.bg = BG_INPUT;
        menu_pal.border = BORDER_SUBTLE;
        input.render_context_menu(buf, w, h, &menu_pal);

        window.show_pixels(w, h);
    }
}
