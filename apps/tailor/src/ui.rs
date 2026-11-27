// SPDX-License-Identifier: GPL-3.0-only
//! Antigravity 3-Pane Candy Lavender pixel UI for Tailor:
//! - Responsive 3-pane IDE layout (>= 720px) with automatic 1-pane collapse (< 720px)
//! - Top toolbar (44px): Brand, Provider pills with live connection dots, Model picker pill,
//!   Approval policy pill, Settings/Accounts button, Clear button
//! - Left pane (~190px): Segmented Files / Artifacts / Steps explorer
//! - Center pane (flexible): Artifact / Unified Diff / File viewer + Mission Overview card
//! - Right pane (~380-460px): Task Boundary card, color-coded Agent Transcript, Approval Gate,
//!   and clickable Candy Lavender Composer with Model Picker trigger & Send/Stop button
//! - Floating Antigravity Model Picker popover & Settings/Accounts modal with backdrop dismiss

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_app::Window;
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_theme::candy_ui::{
    draw_candy_button, fill_rounded_rect, mix_rgb, ButtonState, ButtonStyle, CandyPalette,
};
use tailor_core::{AgentMode, ApprovalPolicy, Provider, StepRecord, StepStatus, TaskBoundary};

// Antigravity Obsidian + Candy Lavender palette constants (0x00RRGGBB)
const BG_CANVAS: u32 = 0x000B_0D14;
const BG_PANE: u32 = 0x0011_1420;
const BG_CARD: u32 = 0x0013_1726;
const BG_MODAL: u32 = 0x0014_1826;
const BG_ELEVATED: u32 = 0x001A_2034;
const BG_ACTIVE_ROW: u32 = 0x0022_2842;
const BG_INPUT: u32 = 0x000B_0D14;
const BG_APPROVAL: u32 = 0x0026_1E10;
const BG_DIFF_ADD: u32 = 0x0011_281D;
const BG_DIFF_DEL: u32 = 0x002C_141C;

const BORDER_SUBTLE: u32 = 0x0023_283D;
const LAVENDER: u32 = 0x00C6_A0F6;
const LAVENDER_DIM: u32 = 0x008B_6FC0;
const TEXT_PRIMARY: u32 = 0x00F1_F5F9;
const TEXT_MUTED: u32 = 0x0094_A3B8;
const TEXT_DIM: u32 = 0x0064_748B;

const COL_EMERALD: u32 = 0x004A_DE80;
const COL_ROSE: u32 = 0x00F8_7171;
const COL_SKY: u32 = 0x0038_BDF8;
const COL_AMBER: u32 = 0x00F5_B971;
const COL_PURPLE: u32 = 0x00A8_55F7;

pub const QUICK_PROMPTS: [&str; 3] = [
    "Inspect workspace",
    "Plan a feature",
    "Review recent changes",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftTab {
    Files,
    Artifacts,
    Steps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterTab {
    Artifact,
    Diff,
    File,
}

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub tag: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProviderStatusInfo {
    pub provider: Provider,
    pub label: &'static str,
    pub subtitle: &'static str,
    pub connected: bool,
    pub auth_kind: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitAction {
    Provider(Provider),
    ToggleModelPicker,
    SelectModel(usize),
    RefreshModels,
    CyclePolicy,
    ToggleSettings,
    ReloadCreds,
    ToggleProxy,
    CloseOverlays,
    Clear,
    LeftTab(LeftTab),
    RefreshFiles,
    OpenFile(usize),
    OpenArtifact(usize),
    SelectStep(usize),
    CenterTab(CenterTab),
    QuickPrompt(usize),
    FocusInput,
    SendOrStop,
    Approve,
    Deny,
    Noop,
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

pub struct RenderState<'a> {
    pub provider: Provider,
    pub model_id: &'a str,
    pub model_display: &'a str,
    pub approval_policy: ApprovalPolicy,
    pub providers: &'a [ProviderStatusInfo],
    pub models: &'a [ModelInfo],
    pub proxy_base: Option<&'a str>,
    pub streaming: bool,
    pub task_boundary: Option<&'a TaskBoundary>,
    pub pending_approval: Option<&'a str>,
    pub transcript: &'a str,
    pub scroll_lines: usize,
    pub input: &'a str,
    pub input_focused: bool,
    pub input_selected: bool,
    pub status: &'a str,
    pub workspace_root: &'a str,
    pub left_tab: LeftTab,
    pub center_tab: CenterTab,
    pub workspace_files: &'a [String],
    pub open_file_name: Option<&'a str>,
    pub open_file_content: &'a str,
    pub artifact_names: &'a [String],
    pub active_artifact_name: Option<&'a str>,
    pub active_artifact_content: &'a str,
    pub active_diff_title: &'a str,
    pub active_diff_content: &'a str,
    pub steps: &'a [StepRecord],
    pub center_scroll: usize,
    pub model_picker_open: bool,
    pub settings_open: bool,
}

pub struct TailorUi {
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
    pub hits: Vec<HitBox>,
    pub center_rect: (i32, i32, i32, i32),
}

fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).min(bw as i32).max(0) as usize;
    let y1 = (y + h).min(bh as i32).max(0) as usize;
    for py in y0..y1 {
        let off = py * bw;
        for px in x0..x1 {
            buf[off + px] = color;
        }
    }
}

fn stroke_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    fill_rect(buf, bw, bh, x, y, w, 1, color);
    fill_rect(buf, bw, bh, x, y + h - 1, w, 1, color);
    fill_rect(buf, bw, bh, x, y, 1, h, color);
    fill_rect(buf, bw, bh, x + w - 1, y, 1, h, color);
}

fn fill_circle(buf: &mut [u32], bw: usize, bh: usize, cx: i32, cy: i32, r: i32, color: u32) {
    if r <= 0 {
        return;
    }
    let r2 = r * r;
    for dy in -r..=r {
        let py = cy + dy;
        if py < 0 || py >= bh as i32 {
            continue;
        }
        let off = py as usize * bw;
        for dx in -r..=r {
            if dx * dx + dy * dy <= r2 {
                let px = cx + dx;
                if px >= 0 && px < bw as i32 {
                    buf[off + px as usize] = color;
                }
            }
        }
    }
}

fn dim_backdrop(buf: &mut [u32]) {
    for px in buf.iter_mut() {
        *px = mix_rgb(*px, 0x0004_050A, 155);
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut count = 0usize;
    for ch in s.chars() {
        if count + 1 >= max_chars && s.chars().count() > max_chars {
            out.push_str("..");
            break;
        }
        out.push(ch);
        count += 1;
        if count >= max_chars {
            break;
        }
    }
    out
}

fn tail_str(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    let total = s.chars().count();
    if total <= max_chars {
        return s;
    }
    let skip = total - max_chars;
    for (idx, (byte_idx, _)) in s.char_indices().enumerate() {
        if idx == skip {
            return &s[byte_idx..];
        }
    }
    s
}

impl TailorUi {
    pub fn new() -> Self {
        Self {
            font: Font::new(sex_font::GEIST_SANS),
            acc: Vec::new(),
            glyph: Vec::new(),
            hits: Vec::new(),
            center_rect: (0, 0, 0, 0),
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
        if max_w <= 6 || text.is_empty() {
            return;
        }
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
            if pen / FX >= limit - 4 {
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

    fn is_provider_connected(&self, state: &RenderState<'_>, prov: Provider) -> bool {
        if state.proxy_base.is_some() {
            return true;
        }
        state
            .providers
            .iter()
            .find(|p| p.provider == prov)
            .map(|p| p.connected)
            .unwrap_or(false)
    }

    pub fn render(&mut self, window: &mut Window, state: &RenderState<'_>) {
        if !window.ready() {
            return;
        }
        let (w, h) = window.size();
        if w < 160 || h < 160 {
            return;
        }
        self.hits.clear();
        self.center_rect = (0, 0, 0, 0);

        let pal = CandyPalette::dark();
        let buf = window.frame(w, h);
        buf.fill(BG_CANVAS);

        let toolbar_h = 44i32;
        self.render_toolbar(buf, w, h, toolbar_h, state);

        let body_y = toolbar_h;
        let body_h = (h as i32 - body_y).max(80);

        if w >= 720 {
            let left_w = if w >= 960 { 192 } else { 172 };
            let right_w = if w >= 1240 {
                440
            } else if w >= 1024 {
                400
            } else {
                ((w as i32) * 44 / 100).clamp(320, 400)
            };
            let center_x = left_w;
            let center_w = (w as i32 - left_w - right_w).max(180);
            let right_x = center_x + center_w;
            let actual_right_w = (w as i32 - right_x).max(200);

            self.center_rect = (center_x, body_y, center_w, body_h);
            self.render_left_pane(buf, w, h, 0, body_y, left_w, body_h, state);
            self.render_center_pane(buf, w, h, center_x, body_y, center_w, body_h, state);
            self.render_right_pane(buf, w, h, right_x, body_y, actual_right_w, body_h, state, &pal);
        } else {
            self.render_right_pane(buf, w, h, 0, body_y, w as i32, body_h, state, &pal);
        }

        // Floating overlays (pushed last so reverse hit-test prioritizes them!)
        if state.model_picker_open {
            self.render_model_picker(buf, w, h, toolbar_h, state);
        } else if state.settings_open {
            self.render_settings_modal(buf, w, h, state);
        }

        window.show_pixels(w, h);
    }

    fn render_toolbar(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        toolbar_h: i32,
        state: &RenderState<'_>,
    ) {
        fill_rect(buf, bw, bh, 0, 0, bw as i32, toolbar_h, BG_PANE);
        fill_rect(buf, bw, bh, 0, toolbar_h - 1, bw as i32, 1, BORDER_SUBTLE);

        // Brand title "TAILOR" in Candy Lavender
        fill_circle(buf, bw, bh, 16, 22, 4, LAVENDER);
        self.draw_text(buf, bw, bh, 26, 14, 68, 14, "TAILOR", LAVENDER);

        // Right-side controls (laid out right-to-left so they never get clipped)
        let mut rx = bw as i32 - 8;

        // Clear button
        let clear_w = 48;
        rx -= clear_w;
        fill_rounded_rect(buf, bw, bh, rx, 9, clear_w, 26, 6, BG_ELEVATED);
        stroke_rect(buf, bw, bh, rx, 9, clear_w, 26, BORDER_SUBTLE);
        self.draw_text(buf, bw, bh, rx + 9, 15, clear_w - 10, 11, "Clear", TEXT_MUTED);
        self.hits.push(HitBox {
            x: rx,
            y: 9,
            w: clear_w,
            h: 26,
            action: HitAction::Clear,
        });
        rx -= 6;

        // Settings / Accounts button with connection dot
        let settings_w = 82;
        rx -= settings_w;
        let active_conn = self.is_provider_connected(state, state.provider);
        let settings_bg = if state.settings_open { BG_ACTIVE_ROW } else { BG_ELEVATED };
        let settings_border = if state.settings_open { LAVENDER } else { BORDER_SUBTLE };
        fill_rounded_rect(buf, bw, bh, rx, 9, settings_w, 26, 6, settings_bg);
        stroke_rect(buf, bw, bh, rx, 9, settings_w, 26, settings_border);
        fill_circle(
            buf,
            bw,
            bh,
            rx + 11,
            22,
            3,
            if active_conn { COL_EMERALD } else { COL_AMBER },
        );
        self.draw_text(buf, bw, bh, rx + 20, 15, settings_w - 22, 11, "Settings", TEXT_PRIMARY);
        self.hits.push(HitBox {
            x: rx,
            y: 9,
            w: settings_w,
            h: 26,
            action: HitAction::ToggleSettings,
        });
        rx -= 6;

        // Approval Policy pill (Ask / Auto / Turbo)
        let (pol_lbl, pol_col) = match state.approval_policy {
            ApprovalPolicy::AlwaysAsk => ("Ask", COL_AMBER),
            ApprovalPolicy::AutoSafe => ("Auto", COL_SKY),
            ApprovalPolicy::Turbo => ("Turbo", COL_PURPLE),
        };
        let pol_w = 54;
        rx -= pol_w;
        fill_rounded_rect(buf, bw, bh, rx, 9, pol_w, 26, 6, BG_ELEVATED);
        stroke_rect(buf, bw, bh, rx, 9, pol_w, 26, mix_rgb(BORDER_SUBTLE, pol_col, 110));
        fill_circle(buf, bw, bh, rx + 10, 22, 3, pol_col);
        self.draw_text(buf, bw, bh, rx + 18, 15, pol_w - 20, 11, pol_lbl, pol_col);
        self.hits.push(HitBox {
            x: rx,
            y: 9,
            w: pol_w,
            h: 26,
            action: HitAction::CyclePolicy,
        });
        rx -= 6;

        // Model Selector Pill
        let model_pill_w = if bw >= 920 { 156 } else if bw >= 720 { 126 } else { 112 };
        rx -= model_pill_w;
        let mp_bg = if state.model_picker_open { BG_ACTIVE_ROW } else { BG_ELEVATED };
        let mp_border = if state.model_picker_open { LAVENDER } else { LAVENDER_DIM };
        fill_rounded_rect(buf, bw, bh, rx, 9, model_pill_w, 26, 6, mp_bg);
        stroke_rect(buf, bw, bh, rx, 9, model_pill_w, 26, mp_border);
        let max_mchars = ((model_pill_w - 26) / 6).max(6) as usize;
        let m_label = format!("{} v", truncate_str(state.model_display, max_mchars));
        self.draw_text(buf, bw, bh, rx + 8, 15, model_pill_w - 12, 11, &m_label, TEXT_PRIMARY);
        self.hits.push(HitBox {
            x: rx,
            y: 9,
            w: model_pill_w,
            h: 26,
            action: HitAction::ToggleModelPicker,
        });

        let right_limit = rx - 8;

        // Clickable Provider Pills: Antigravity | Codex | Claude | DeepSeek | Gemini
        let pills: [(Provider, &str, i32); 5] = [
            (Provider::Antigravity, "Antigravity", 88),
            (Provider::Codex, "Codex", 60),
            (Provider::Claude, "Claude", 62),
            (Provider::ClaudeDeepSeek, "DeepSeek", 76),
            (Provider::Gemini, "Gemini", 62),
        ];
        let mut px = 94i32;
        for (prov, label, pw) in pills {
            if px + pw > right_limit {
                break;
            }
            let active = prov == state.provider;
            let connected = self.is_provider_connected(state, prov);
            let bg = if active { BG_ACTIVE_ROW } else { BG_CARD };
            let border = if active { LAVENDER } else { BORDER_SUBTLE };
            fill_rounded_rect(buf, bw, bh, px, 9, pw, 26, 6, bg);
            stroke_rect(buf, bw, bh, px, 9, pw, 26, border);
            let dot_col = if connected { COL_EMERALD } else { TEXT_DIM };
            fill_circle(buf, bw, bh, px + 9, 22, 3, dot_col);
            self.draw_text(
                buf,
                bw,
                bh,
                px + 17,
                15,
                pw - 20,
                11,
                label,
                if active { TEXT_PRIMARY } else { TEXT_MUTED },
            );
            self.hits.push(HitBox {
                x: px,
                y: 9,
                w: pw,
                h: 26,
                action: HitAction::Provider(prov),
            });
            px += pw + 5;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_left_pane(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        state: &RenderState<'_>,
    ) {
        fill_rect(buf, bw, bh, x, y, w, h, BG_PANE);
        fill_rect(buf, bw, bh, x + w - 1, y, 1, h, BORDER_SUBTLE);

        // Segmented tabs: Files | Artifacts | Steps
        let tab_h = 30;
        fill_rect(buf, bw, bh, x, y + tab_h - 1, w - 1, 1, BORDER_SUBTLE);
        let tabs = [
            (LeftTab::Files, "Files"),
            (LeftTab::Artifacts, "Art."),
            (LeftTab::Steps, "Steps"),
        ];
        let tw = (w - 12) / 3;
        for (i, (tab, lbl)) in tabs.iter().enumerate() {
            let tx = x + 6 + (i as i32) * tw;
            let active = state.left_tab == *tab;
            if active {
                fill_rounded_rect(buf, bw, bh, tx, y + 4, tw - 2, 22, 5, BG_ACTIVE_ROW);
                fill_rect(buf, bw, bh, tx + 4, y + tab_h - 2, tw - 10, 2, LAVENDER);
            }
            self.draw_text(
                buf,
                bw,
                bh,
                tx + 8,
                y + 8,
                tw - 10,
                11,
                lbl,
                if active { TEXT_PRIMARY } else { TEXT_MUTED },
            );
            self.hits.push(HitBox {
                x: tx,
                y: y + 2,
                w: tw - 2,
                h: 26,
                action: HitAction::LeftTab(*tab),
            });
        }

        let content_y = y + tab_h + 6;
        let max_y = y + h - 8;

        match state.left_tab {
            LeftTab::Files => {
                // Header + Refresh button
                let ws_lbl = if state.workspace_root.is_empty() {
                    "WORKSPACE"
                } else {
                    state.workspace_root
                };
                self.draw_text(
                    buf,
                    bw,
                    bh,
                    x + 10,
                    content_y + 2,
                    w - 64,
                    11,
                    &truncate_str(ws_lbl, 16),
                    TEXT_DIM,
                );
                let rbx = x + w - 50;
                fill_rounded_rect(buf, bw, bh, rbx, content_y - 1, 42, 18, 4, BG_ELEVATED);
                self.draw_text(buf, bw, bh, rbx + 6, content_y + 2, 34, 10, "Sync", LAVENDER);
                self.hits.push(HitBox {
                    x: rbx,
                    y: content_y - 1,
                    w: 42,
                    h: 18,
                    action: HitAction::RefreshFiles,
                });

                let mut ry = content_y + 24;
                if state.workspace_files.is_empty() {
                    self.draw_text(
                        buf,
                        bw,
                        bh,
                        x + 10,
                        ry,
                        w - 20,
                        11,
                        "No files listed.",
                        TEXT_DIM,
                    );
                    self.draw_text(
                        buf,
                        bw,
                        bh,
                        x + 10,
                        ry + 18,
                        w - 20,
                        10,
                        "Click Sync to refresh",
                        TEXT_DIM,
                    );
                } else {
                    for (idx, fname) in state.workspace_files.iter().enumerate() {
                        if ry + 22 > max_y {
                            break;
                        }
                        let is_open = state
                            .open_file_name
                            .map(|n| n == fname.as_str())
                            .unwrap_or(false);
                        if is_open {
                            fill_rounded_rect(buf, bw, bh, x + 6, ry, w - 13, 21, 5, BG_ACTIVE_ROW);
                        }
                        let is_dir = fname.ends_with('/');
                        let col = if is_open {
                            LAVENDER
                        } else if is_dir {
                            COL_SKY
                        } else {
                            TEXT_PRIMARY
                        };
                        let disp = truncate_str(fname, ((w - 22) / 6).max(8) as usize);
                        self.draw_text(buf, bw, bh, x + 12, ry + 4, w - 22, 11, &disp, col);
                        self.hits.push(HitBox {
                            x: x + 6,
                            y: ry,
                            w: w - 13,
                            h: 21,
                            action: HitAction::OpenFile(idx),
                        });
                        ry += 23;
                    }
                }
            }
            LeftTab::Artifacts => {
                self.draw_text(buf, bw, bh, x + 10, content_y + 2, w - 20, 11, "ARTIFACTS", TEXT_DIM);
                let mut ry = content_y + 22;
                for (idx, aname) in state.artifact_names.iter().enumerate() {
                    if ry + 24 > max_y {
                        break;
                    }
                    let is_active = state
                        .active_artifact_name
                        .map(|n| n == aname.as_str())
                        .unwrap_or(false)
                        && state.center_tab == CenterTab::Artifact;
                    if is_active {
                        fill_rounded_rect(buf, bw, bh, x + 6, ry, w - 13, 24, 5, BG_ACTIVE_ROW);
                    }
                    fill_circle(
                        buf,
                        bw,
                        bh,
                        x + 15,
                        ry + 12,
                        3,
                        if is_active { LAVENDER } else { COL_EMERALD },
                    );
                    let disp = truncate_str(aname, ((w - 30) / 6).max(8) as usize);
                    self.draw_text(
                        buf,
                        bw,
                        bh,
                        x + 24,
                        ry + 6,
                        w - 30,
                        11,
                        &disp,
                        if is_active { TEXT_PRIMARY } else { TEXT_MUTED },
                    );
                    self.hits.push(HitBox {
                        x: x + 6,
                        y: ry,
                        w: w - 13,
                        h: 24,
                        action: HitAction::OpenArtifact(idx),
                    });
                    ry += 26;
                }
            }
            LeftTab::Steps => {
                self.draw_text(
                    buf,
                    bw,
                    bh,
                    x + 10,
                    content_y + 2,
                    w - 20,
                    11,
                    "AGENT STEPS",
                    TEXT_DIM,
                );
                let mut ry = content_y + 22;
                if state.steps.is_empty() {
                    self.draw_text(
                        buf,
                        bw,
                        bh,
                        x + 10,
                        ry,
                        w - 20,
                        11,
                        "No tool steps yet.",
                        TEXT_DIM,
                    );
                } else {
                    for (idx, step) in state.steps.iter().rev().enumerate() {
                        if ry + 28 > max_y {
                            break;
                        }
                        let orig_idx = state.steps.len() - 1 - idx;
                        let dot_col = match step.status {
                            StepStatus::Done => COL_EMERALD,
                            StepStatus::Running | StepStatus::Pending => COL_SKY,
                            StepStatus::Waiting => COL_AMBER,
                            StepStatus::Error | StepStatus::Denied => COL_ROSE,
                        };
                        fill_rounded_rect(buf, bw, bh, x + 6, ry, w - 13, 26, 5, BG_CARD);
                        fill_circle(buf, bw, bh, x + 15, ry + 13, 3, dot_col);
                        let label = if !step.title.is_empty() {
                            step.title.as_str()
                        } else {
                            step.tool_name.as_str()
                        };
                        let disp = truncate_str(label, ((w - 30) / 6).max(8) as usize);
                        self.draw_text(buf, bw, bh, x + 24, ry + 6, w - 30, 11, &disp, TEXT_PRIMARY);
                        self.hits.push(HitBox {
                            x: x + 6,
                            y: ry,
                            w: w - 13,
                            h: 26,
                            action: HitAction::SelectStep(orig_idx),
                        });
                        ry += 29;
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_center_pane(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        state: &RenderState<'_>,
    ) {
        fill_rect(buf, bw, bh, x, y, w, h, BG_CANVAS);

        // Tab bar: Artifact | Diff | File
        let tab_h = 30;
        fill_rect(buf, bw, bh, x, y, w, tab_h, BG_PANE);
        fill_rect(buf, bw, bh, x, y + tab_h - 1, w, 1, BORDER_SUBTLE);

        let tabs = [
            (CenterTab::Artifact, "Artifact", 72),
            (CenterTab::Diff, "Diff", 54),
            (CenterTab::File, "File", 54),
        ];
        let mut tx = x + 8;
        for (tab, lbl, tw) in tabs {
            let active = state.center_tab == tab;
            if active {
                fill_rounded_rect(buf, bw, bh, tx, y + 4, tw, 22, 5, BG_ACTIVE_ROW);
                fill_rect(buf, bw, bh, tx + 6, y + tab_h - 2, tw - 12, 2, LAVENDER);
            }
            self.draw_text(
                buf,
                bw,
                bh,
                tx + 12,
                y + 8,
                tw - 14,
                11,
                lbl,
                if active { TEXT_PRIMARY } else { TEXT_MUTED },
            );
            self.hits.push(HitBox {
                x: tx,
                y: y + 2,
                w: tw,
                h: 26,
                action: HitAction::CenterTab(tab),
            });
            tx += tw + 6;
        }

        // Right-aligned active title
        let title_str = match state.center_tab {
            CenterTab::Artifact => state.active_artifact_name.unwrap_or("Overview"),
            CenterTab::Diff => {
                if state.active_diff_title.is_empty() {
                    "No diff"
                } else {
                    state.active_diff_title
                }
            }
            CenterTab::File => state.open_file_name.unwrap_or("No file"),
        };
        let avail_tw = (x + w - tx - 12).max(20);
        if avail_tw > 40 {
            self.draw_text(
                buf,
                bw,
                bh,
                tx + 8,
                y + 9,
                avail_tw,
                11,
                &truncate_str(title_str, (avail_tw / 6) as usize),
                TEXT_DIM,
            );
        }

        let cy = y + tab_h + 8;
        let ch = (h - tab_h - 12).max(40);

        match state.center_tab {
            CenterTab::Artifact => {
                if state.active_artifact_content.is_empty() {
                    self.render_mission_overview(buf, bw, bh, x + 12, cy, w - 24, ch, state);
                } else {
                    self.render_document_lines(
                        buf,
                        bw,
                        bh,
                        x + 12,
                        cy,
                        w - 24,
                        ch,
                        state.active_artifact_content,
                        state.center_scroll,
                        false,
                    );
                }
            }
            CenterTab::Diff => {
                if state.active_diff_content.is_empty() {
                    self.render_mission_overview(buf, bw, bh, x + 12, cy, w - 24, ch, state);
                } else {
                    self.render_document_lines(
                        buf,
                        bw,
                        bh,
                        x + 12,
                        cy,
                        w - 24,
                        ch,
                        state.active_diff_content,
                        state.center_scroll,
                        true,
                    );
                }
            }
            CenterTab::File => {
                if state.open_file_content.is_empty() && state.open_file_name.is_none() {
                    self.render_mission_overview(buf, bw, bh, x + 12, cy, w - 24, ch, state);
                } else {
                    self.render_document_lines(
                        buf,
                        bw,
                        bh,
                        x + 12,
                        cy,
                        w - 24,
                        ch,
                        state.open_file_content,
                        state.center_scroll,
                        false,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_mission_overview(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        state: &RenderState<'_>,
    ) {
        let card_h = h.min(340).max(160);
        fill_rounded_rect(buf, bw, bh, x, y, w, card_h, 10, BG_CARD);
        stroke_rect(buf, bw, bh, x, y, w, card_h, BORDER_SUBTLE);

        let mut cy = y + 16;
        self.draw_text(
            buf,
            bw,
            bh,
            x + 16,
            cy,
            w - 32,
            15,
            "Antigravity Mission Control",
            LAVENDER,
        );
        cy += 24;

        let ws_line = format!(
            "Workspace: {}   |   Model: {}",
            if state.workspace_root.is_empty() {
                "."
            } else {
                state.workspace_root
            },
            state.model_display
        );
        self.draw_text(buf, bw, bh, x + 16, cy, w - 32, 11, &ws_line, TEXT_MUTED);
        cy += 24;

        // Connected Subscriptions Summary
        self.draw_text(
            buf,
            bw,
            bh,
            x + 16,
            cy,
            w - 32,
            11,
            "CONNECTED SUBSCRIPTIONS",
            TEXT_DIM,
        );
        cy += 18;

        let mut connected_count = 0usize;
        for info in state.providers {
            if cy + 20 > y + card_h - 80 {
                break;
            }
            let conn = info.connected || state.proxy_base.is_some();
            if conn {
                connected_count += 1;
            }
            fill_circle(
                buf,
                bw,
                bh,
                x + 22,
                cy + 6,
                3,
                if conn { COL_EMERALD } else { TEXT_DIM },
            );
            let line = format!("{}  —  {}", info.label, info.auth_kind);
            self.draw_text(
                buf,
                bw,
                bh,
                x + 32,
                cy,
                w - 48,
                11,
                &line,
                if conn { TEXT_PRIMARY } else { TEXT_DIM },
            );
            cy += 19;
        }

        if connected_count == 0 {
            self.draw_text(
                buf,
                bw,
                bh,
                x + 16,
                cy + 2,
                w - 32,
                11,
                "Open Settings to reload /data/tailor or set /key",
                COL_AMBER,
            );
            cy += 20;
        }

        cy += 10;
        if cy + 52 <= y + card_h {
            self.draw_text(buf, bw, bh, x + 16, cy, w - 32, 11, "QUICK ACTIONS", TEXT_DIM);
            cy += 18;
            let mut qx = x + 16;
            for (idx, prompt) in QUICK_PROMPTS.iter().enumerate() {
                let qw = (prompt.len() as i32) * 7 + 20;
                if qx + qw > x + w - 16 {
                    qx = x + 16;
                    cy += 30;
                }
                if cy + 26 > y + card_h - 6 {
                    break;
                }
                fill_rounded_rect(buf, bw, bh, qx, cy, qw, 24, 6, BG_ELEVATED);
                stroke_rect(buf, bw, bh, qx, cy, qw, 24, LAVENDER_DIM);
                self.draw_text(buf, bw, bh, qx + 10, cy + 5, qw - 14, 11, prompt, TEXT_PRIMARY);
                self.hits.push(HitBox {
                    x: qx,
                    y: cy,
                    w: qw,
                    h: 24,
                    action: HitAction::QuickPrompt(idx),
                });
                qx += qw + 8;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_document_lines(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        content: &str,
        scroll: usize,
        is_diff: bool,
    ) {
        fill_rounded_rect(buf, bw, bh, x, y, w, h, 8, BG_CARD);
        stroke_rect(buf, bw, bh, x, y, w, h, BORDER_SUBTLE);

        let line_h = 18i32;
        let max_rows = ((h - 16) / line_h).max(1) as usize;
        let lines: Vec<&str> = content.lines().collect();
        let start = scroll.min(lines.len().saturating_sub(1));
        let end = (start + max_rows).min(lines.len());
        let gutter_w = 38i32;
        let text_w = (w - gutter_w - 16).max(40);

        let mut ly = y + 8;
        for (offset, raw_line) in lines[start..end].iter().enumerate() {
            let line_no = start + offset + 1;
            let (fg, row_bg) = if is_diff {
                if raw_line.starts_with("@@") {
                    (COL_SKY, None)
                } else if raw_line.starts_with("---") || raw_line.starts_with("+++") {
                    (LAVENDER, None)
                } else if raw_line.starts_with('+') {
                    (COL_EMERALD, Some(BG_DIFF_ADD))
                } else if raw_line.starts_with('-') {
                    (COL_ROSE, Some(BG_DIFF_DEL))
                } else {
                    (TEXT_PRIMARY, None)
                }
            } else if raw_line.starts_with('#') {
                (LAVENDER, None)
            } else if raw_line.trim_start().starts_with("- [x]") {
                (COL_EMERALD, None)
            } else if raw_line.trim_start().starts_with("- [ ]") {
                (COL_AMBER, None)
            } else if raw_line.starts_with("```") {
                (COL_SKY, None)
            } else {
                (TEXT_PRIMARY, None)
            };

            if let Some(bg) = row_bg {
                fill_rect(buf, bw, bh, x + 2, ly - 1, w - 4, line_h, bg);
            }
            let num_s = format!("{:>3}", line_no);
            self.draw_text(buf, bw, bh, x + 6, ly + 1, gutter_w - 8, 11, &num_s, TEXT_DIM);
            self.draw_text(buf, bw, bh, x + gutter_w + 4, ly + 1, text_w, 12, raw_line, fg);
            ly += line_h;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_right_pane(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        state: &RenderState<'_>,
        pal: &CandyPalette,
    ) {
        fill_rect(buf, bw, bh, x, y, w, h, BG_PANE);
        if x > 0 {
            fill_rect(buf, bw, bh, x, y, 1, h, BORDER_SUBTLE);
        }

        let pad = 10i32;
        let inner_x = x + pad;
        let inner_w = (w - pad * 2).max(120);

        // 1. Pinned Task Boundary Card (when active)
        let mut top_y = y + 8;
        if let Some(tb) = state.task_boundary {
            if tb.mode != AgentMode::Idle {
                let tb_h = 52;
                let (mode_lbl, mode_col) = match tb.mode {
                    AgentMode::Planning => ("[PLANNING]", COL_SKY),
                    AgentMode::Execution => ("[EXECUTION]", COL_PURPLE),
                    AgentMode::Verification => ("[VERIFICATION]", COL_EMERALD),
                    AgentMode::Idle => ("[IDLE]", TEXT_DIM),
                };
                fill_rounded_rect(buf, bw, bh, inner_x, top_y, inner_w, tb_h, 8, BG_CARD);
                stroke_rect(buf, bw, bh, inner_x, top_y, inner_w, tb_h, BORDER_SUBTLE);
                fill_rect(buf, bw, bh, inner_x, top_y + 6, 3, tb_h - 12, mode_col);

                self.draw_text(buf, bw, bh, inner_x + 12, top_y + 7, 110, 11, mode_lbl, mode_col);
                let title = if tb.task_name.is_empty() {
                    "Active Task"
                } else {
                    tb.task_name.as_str()
                };
                self.draw_text(
                    buf,
                    bw,
                    bh,
                    inner_x + 114,
                    top_y + 7,
                    inner_w - 122,
                    12,
                    title,
                    TEXT_PRIMARY,
                );
                if !tb.task_status.is_empty() {
                    self.draw_text(
                        buf,
                        bw,
                        bh,
                        inner_x + 12,
                        top_y + 27,
                        inner_w - 20,
                        11,
                        &tb.task_status,
                        TEXT_MUTED,
                    );
                }
                top_y += tb_h + 8;
            }
        }

        // Layout bottom sections: Composer (96px) + optional Approval Gate (52px)
        let composer_h = 96i32;
        let approval_h = if state.pending_approval.is_some() { 52 } else { 0 };
        let bottom_y = y + h - composer_h - approval_h - 6;
        let chat_h = (bottom_y - top_y - 4).max(40);

        // 2. Scrollable Conversation & Agent Transcript
        fill_rounded_rect(buf, bw, bh, inner_x, top_y, inner_w, chat_h, 8, BG_CANVAS);
        stroke_rect(buf, bw, bh, inner_x, top_y, inner_w, chat_h, BORDER_SUBTLE);

        let text_w = (inner_w - 20).max(80);
        let max_chars = ((text_w / 7).max(20)) as usize;
        let mut wrapped: Vec<(u32, String)> = Vec::new();
        for raw_line in state.transcript.lines() {
            let color = if raw_line.starts_with("> ") {
                LAVENDER
            } else if raw_line.starts_with("@@")
                || raw_line.starts_with("--- ")
                || raw_line.starts_with("+++ ")
            {
                COL_SKY
            } else if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
                COL_EMERALD
            } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
                COL_ROSE
            } else if raw_line.starts_with("[APPROVAL")
                || raw_line.starts_with("[approval")
                || raw_line.starts_with("[tool")
                || raw_line.starts_with("[requesting tool")
            {
                COL_AMBER
            } else if raw_line.starts_with("[error") {
                COL_ROSE
            } else if raw_line.starts_with("[thinking") {
                LAVENDER_DIM
            } else if raw_line.starts_with("[tailor]") {
                LAVENDER
            } else {
                TEXT_PRIMARY
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
                if split == 0 {
                    split = cur.len();
                }
                wrapped.push((color, String::from(cur[..split].trim_end())));
                cur = &cur[split..];
            }
            wrapped.push((color, String::from(cur)));
        }

        let line_h = 18usize;
        let max_rows = (chat_h.saturating_sub(14) as usize) / line_h;
        let end = wrapped.len().saturating_sub(state.scroll_lines);
        let start = end.saturating_sub(max_rows);
        let mut ly = top_y + 8;
        for (col, line) in &wrapped[start..end] {
            if !line.is_empty() {
                self.draw_text(buf, bw, bh, inner_x + 10, ly, text_w, 12, line, *col);
            }
            ly += line_h as i32;
        }

        // 3. Amber Approval Gate bar (when pending_approval is present)
        let mut cur_y = bottom_y;
        if let Some(summary) = state.pending_approval {
            fill_rounded_rect(buf, bw, bh, inner_x, cur_y, inner_w, 46, 8, BG_APPROVAL);
            stroke_rect(buf, bw, bh, inner_x, cur_y, inner_w, 46, COL_AMBER);
            let prompt = format!("Approve: {}", summary);
            self.draw_text(
                buf,
                bw,
                bh,
                inner_x + 10,
                cur_y + 6,
                inner_w - 20,
                11,
                &prompt,
                COL_AMBER,
            );

            let btn_y = cur_y + 22;
            let app_w = 76;
            let den_w = 64;
            let ax = inner_x + inner_w - app_w - den_w - 14;
            fill_rounded_rect(buf, bw, bh, ax, btn_y, app_w, 20, 5, 0x001B_5E3B);
            stroke_rect(buf, bw, bh, ax, btn_y, app_w, 20, COL_EMERALD);
            self.draw_text(buf, bw, bh, ax + 12, btn_y + 4, app_w - 12, 11, "Approve", TEXT_PRIMARY);
            self.hits.push(HitBox {
                x: ax,
                y: btn_y,
                w: app_w,
                h: 20,
                action: HitAction::Approve,
            });

            let dx = ax + app_w + 6;
            fill_rounded_rect(buf, bw, bh, dx, btn_y, den_w, 20, 5, 0x004C_1D24);
            stroke_rect(buf, bw, bh, dx, btn_y, den_w, 20, COL_ROSE);
            self.draw_text(buf, bw, bh, dx + 16, btn_y + 4, den_w - 16, 11, "Deny", TEXT_PRIMARY);
            self.hits.push(HitBox {
                x: dx,
                y: btn_y,
                w: den_w,
                h: 20,
                action: HitAction::Deny,
            });

            cur_y += approval_h;
        }

        // 4. Clickable Composer & Input Bar
        fill_rounded_rect(buf, bw, bh, inner_x, cur_y, inner_w, composer_h - 4, 10, BG_CARD);
        let box_border = if state.input_focused { LAVENDER } else { BORDER_SUBTLE };
        stroke_rect(buf, bw, bh, inner_x, cur_y, inner_w, composer_h - 4, box_border);

        // Input well (entire upper region is clickable HitAction::FocusInput)
        let well_x = inner_x + 6;
        let well_y = cur_y + 6;
        let well_w = inner_w - 12;
        let well_h = 36;
        fill_rounded_rect(buf, bw, bh, well_x, well_y, well_w, well_h, 6, BG_INPUT);
        if state.input_focused {
            stroke_rect(buf, bw, bh, well_x, well_y, well_w, well_h, LAVENDER);
        } else {
            stroke_rect(buf, bw, bh, well_x, well_y, well_w, well_h, BORDER_SUBTLE);
        }
        if state.input_selected && !state.input.is_empty() {
            fill_rounded_rect(
                buf,
                bw,
                bh,
                well_x + 4,
                well_y + 4,
                well_w - 8,
                well_h - 8,
                4,
                pal.selection,
            );
        }

        let visible_chars = ((well_w - 20) / 7).max(12) as usize;
        let (prompt_text, prompt_col) = if state.input.is_empty() {
            if state.input_focused {
                (String::from("| Ask Tailor anything..."), TEXT_MUTED)
            } else {
                (String::from("Click here to type a prompt..."), TEXT_DIM)
            }
        } else {
            let tail = tail_str(state.input, visible_chars);
            if state.input_focused {
                (format!("{}|", tail), TEXT_PRIMARY)
            } else {
                (String::from(tail), TEXT_PRIMARY)
            }
        };
        self.draw_text(
            buf,
            bw,
            bh,
            well_x + 8,
            well_y + 10,
            well_w - 16,
            13,
            &prompt_text,
            prompt_col,
        );
        self.hits.push(HitBox {
            x: well_x,
            y: well_y,
            w: well_w,
            h: well_h,
            action: HitAction::FocusInput,
        });

        // Bottom composer row: Model Picker Trigger + Send/Stop button
        let row_y = well_y + well_h + 6;
        let send_w = 72;
        let send_x = inner_x + inner_w - send_w - 6;

        // Model Picker Trigger pill
        let trigger_w = (send_x - well_x - 8).min(200).max(80);
        fill_rounded_rect(buf, bw, bh, well_x, row_y, trigger_w, 26, 6, BG_ELEVATED);
        stroke_rect(
            buf,
            bw,
            bh,
            well_x,
            row_y,
            trigger_w,
            26,
            if state.model_picker_open { LAVENDER } else { BORDER_SUBTLE },
        );
        let m_chars = ((trigger_w - 22) / 6).max(6) as usize;
        let m_lbl = format!("{} ^", truncate_str(state.model_display, m_chars));
        self.draw_text(buf, bw, bh, well_x + 8, row_y + 6, trigger_w - 12, 11, &m_lbl, LAVENDER);
        self.hits.push(HitBox {
            x: well_x,
            y: row_y,
            w: trigger_w,
            h: 26,
            action: HitAction::ToggleModelPicker,
        });

        // Send / Stop button
        let btn_style = if state.streaming {
            ButtonStyle::Danger
        } else {
            ButtonStyle::Primary
        };
        draw_candy_button(
            buf,
            bw,
            bh,
            send_x,
            row_y,
            send_w,
            26,
            6,
            btn_style,
            ButtonState::Rest,
            pal,
        );
        let send_lbl = if state.streaming { "Stop" } else { "Send" };
        self.draw_text(buf, bw, bh, send_x + 20, row_y + 6, send_w - 20, 12, send_lbl, 0x00FF_FFFF);
        self.hits.push(HitBox {
            x: send_x,
            y: row_y,
            w: send_w,
            h: 26,
            action: HitAction::SendOrStop,
        });

        // Status strip
        self.draw_text(
            buf,
            bw,
            bh,
            well_x + 2,
            row_y + 29,
            well_w - 4,
            10,
            state.status,
            TEXT_DIM,
        );
    }

    fn render_model_picker(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        toolbar_h: i32,
        state: &RenderState<'_>,
    ) {
        dim_backdrop(buf);
        self.hits.push(HitBox {
            x: 0,
            y: 0,
            w: bw as i32,
            h: bh as i32,
            action: HitAction::CloseOverlays,
        });

        let mw = (bw as i32 - 32).min(340).max(240);
        let row_h = 34;
        let max_rows = ((bh as i32 - toolbar_h - 110) / row_h).max(3) as usize;
        let shown = state.models.len().min(max_rows).max(1);
        let mh = 44 + (shown as i32) * row_h + 12;
        let mx = (bw as i32 - mw - 18).max(12);
        let my = toolbar_h + 6;

        fill_rounded_rect(buf, bw, bh, mx, my, mw, mh, 10, BG_MODAL);
        stroke_rect(buf, bw, bh, mx, my, mw, mh, LAVENDER);
        self.hits.push(HitBox {
            x: mx,
            y: my,
            w: mw,
            h: mh,
            action: HitAction::Noop,
        });

        // Header
        let hdr = format!("Models · {}", state.provider.display_name());
        self.draw_text(buf, bw, bh, mx + 14, my + 12, mw - 110, 13, &hdr, LAVENDER);

        // Refresh button
        let rfx = mx + mw - 88;
        fill_rounded_rect(buf, bw, bh, rfx, my + 8, 54, 22, 5, BG_ELEVATED);
        stroke_rect(buf, bw, bh, rfx, my + 8, 54, 22, BORDER_SUBTLE);
        self.draw_text(buf, bw, bh, rfx + 7, my + 13, 44, 10, "Refresh", COL_SKY);
        self.hits.push(HitBox {
            x: rfx,
            y: my + 8,
            w: 54,
            h: 22,
            action: HitAction::RefreshModels,
        });

        // Close X button
        let cx = mx + mw - 28;
        fill_rounded_rect(buf, bw, bh, cx, my + 8, 20, 22, 5, BG_ELEVATED);
        self.draw_text(buf, bw, bh, cx + 6, my + 12, 14, 12, "X", TEXT_MUTED);
        self.hits.push(HitBox {
            x: cx,
            y: my + 8,
            w: 20,
            h: 22,
            action: HitAction::CloseOverlays,
        });

        fill_rect(buf, bw, bh, mx + 1, my + 37, mw - 2, 1, BORDER_SUBTLE);

        let mut ry = my + 44;
        for (idx, m) in state.models.iter().take(max_rows).enumerate() {
            let active = m.id == state.model_id;
            if active {
                fill_rounded_rect(buf, bw, bh, mx + 8, ry, mw - 16, row_h - 4, 6, BG_ACTIVE_ROW);
                stroke_rect(buf, bw, bh, mx + 8, ry, mw - 16, row_h - 4, LAVENDER_DIM);
            }
            fill_circle(
                buf,
                bw,
                bh,
                mx + 18,
                ry + 14,
                3,
                if active { LAVENDER } else { TEXT_DIM },
            );
            let tag_w = m.tag.as_ref().map(|t| (t.len() as i32) * 6 + 16).unwrap_or(0);
            self.draw_text(
                buf,
                bw,
                bh,
                mx + 28,
                ry + 4,
                mw - 46 - tag_w,
                12,
                &m.display_name,
                if active { TEXT_PRIMARY } else { TEXT_MUTED },
            );
            self.draw_text(
                buf,
                bw,
                bh,
                mx + 28,
                ry + 18,
                mw - 46 - tag_w,
                10,
                &m.id,
                TEXT_DIM,
            );
            if let Some(ref tag) = m.tag {
                let tx = mx + mw - tag_w - 14;
                let tcol = if tag.eq_ignore_ascii_case("New") {
                    COL_EMERALD
                } else {
                    COL_AMBER
                };
                fill_rounded_rect(buf, bw, bh, tx, ry + 7, tag_w, 16, 4, BG_ELEVATED);
                self.draw_text(buf, bw, bh, tx + 6, ry + 9, tag_w - 8, 10, tag, tcol);
            }
            self.hits.push(HitBox {
                x: mx + 8,
                y: ry,
                w: mw - 16,
                h: row_h - 4,
                action: HitAction::SelectModel(idx),
            });
            ry += row_h;
        }
    }

    fn render_settings_modal(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        state: &RenderState<'_>,
    ) {
        dim_backdrop(buf);
        self.hits.push(HitBox {
            x: 0,
            y: 0,
            w: bw as i32,
            h: bh as i32,
            action: HitAction::CloseOverlays,
        });

        let mw = (bw as i32 - 32).min(500).max(300);
        let mh = (bh as i32 - 48).min(380).max(280);
        let mx = (bw as i32 - mw) / 2;
        let my = (bh as i32 - mh) / 2;

        fill_rounded_rect(buf, bw, bh, mx, my, mw, mh, 12, BG_MODAL);
        stroke_rect(buf, bw, bh, mx, my, mw, mh, LAVENDER);
        self.hits.push(HitBox {
            x: mx,
            y: my,
            w: mw,
            h: mh,
            action: HitAction::Noop,
        });

        // Header
        self.draw_text(buf, bw, bh, mx + 16, my + 14, mw - 60, 15, "Settings & Accounts", LAVENDER);
        let cx = mx + mw - 32;
        fill_rounded_rect(buf, bw, bh, cx, my + 10, 22, 22, 5, BG_ELEVATED);
        self.draw_text(buf, bw, bh, cx + 7, my + 14, 14, 12, "X", TEXT_PRIMARY);
        self.hits.push(HitBox {
            x: cx,
            y: my + 10,
            w: 22,
            h: 22,
            action: HitAction::CloseOverlays,
        });
        fill_rect(buf, bw, bh, mx + 1, my + 40, mw - 2, 1, BORDER_SUBTLE);

        // Providers list
        let mut ry = my + 50;
        for info in state.providers {
            if ry + 34 > my + mh - 108 {
                break;
            }
            let is_active = info.provider == state.provider;
            fill_rounded_rect(
                buf,
                bw,
                bh,
                mx + 14,
                ry,
                mw - 28,
                32,
                6,
                if is_active { BG_ACTIVE_ROW } else { BG_CARD },
            );
            stroke_rect(
                buf,
                bw,
                bh,
                mx + 14,
                ry,
                mw - 28,
                32,
                if is_active { LAVENDER_DIM } else { BORDER_SUBTLE },
            );
            let conn = info.connected || state.proxy_base.is_some();
            fill_circle(
                buf,
                bw,
                bh,
                mx + 26,
                ry + 16,
                4,
                if conn { COL_EMERALD } else { TEXT_DIM },
            );
            let title = format!("{} ({})", info.label, info.subtitle);
            self.draw_text(buf, bw, bh, mx + 38, ry + 4, mw - 210, 12, &title, TEXT_PRIMARY);
            let status_txt = if conn {
                format!("CONNECTED · {}", info.auth_kind)
            } else {
                String::from("NOT CONNECTED")
            };
            self.draw_text(
                buf,
                bw,
                bh,
                mx + 38,
                ry + 18,
                mw - 210,
                10,
                &status_txt,
                if conn { COL_EMERALD } else { TEXT_DIM },
            );

            let bx = mx + mw - 84;
            fill_rounded_rect(
                buf,
                bw,
                bh,
                bx,
                ry + 5,
                62,
                22,
                5,
                if is_active { LAVENDER_DIM } else { BG_ELEVATED },
            );
            self.draw_text(
                buf,
                bw,
                bh,
                bx + 12,
                ry + 10,
                50,
                11,
                if is_active { "Active" } else { "Select" },
                TEXT_PRIMARY,
            );
            self.hits.push(HitBox {
                x: bx,
                y: ry + 5,
                w: 62,
                h: 22,
                action: HitAction::Provider(info.provider),
            });
            ry += 36;
        }

        // Proxy Mode Row
        ry += 6;
        fill_rounded_rect(buf, bw, bh, mx + 14, ry, mw - 28, 34, 6, BG_CARD);
        stroke_rect(buf, bw, bh, mx + 14, ry, mw - 28, 34, BORDER_SUBTLE);
        let proxy_txt = match state.proxy_base {
            Some(url) => format!("Proxy (CLIProxyAPI-Rust): {}", url),
            None => String::from("Proxy (CLIProxyAPI-Rust): Off (127.0.0.1:8317)"),
        };
        self.draw_text(buf, bw, bh, mx + 24, ry + 11, mw - 120, 11, &proxy_txt, TEXT_PRIMARY);
        let px = mx + mw - 84;
        let proxy_on = state.proxy_base.is_some();
        fill_rounded_rect(
            buf,
            bw,
            bh,
            px,
            ry + 6,
            62,
            22,
            5,
            if proxy_on { 0x001B_5E3B } else { BG_ELEVATED },
        );
        self.draw_text(
            buf,
            bw,
            bh,
            px + 14,
            ry + 11,
            50,
            11,
            if proxy_on { "On" } else { "Enable" },
            if proxy_on { COL_EMERALD } else { TEXT_MUTED },
        );
        self.hits.push(HitBox {
            x: px,
            y: ry + 6,
            w: 62,
            h: 22,
            action: HitAction::ToggleProxy,
        });

        // Reload /data/tailor button + CLI hint
        let act_y = ry + 42;
        fill_rounded_rect(buf, bw, bh, mx + 14, act_y, 168, 28, 6, BG_ACTIVE_ROW);
        stroke_rect(buf, bw, bh, mx + 14, act_y, 168, 28, LAVENDER);
        self.draw_text(
            buf,
            bw,
            bh,
            mx + 26,
            act_y + 8,
            150,
            12,
            "Reload /data/tailor",
            TEXT_PRIMARY,
        );
        self.hits.push(HitBox {
            x: mx + 14,
            y: act_y,
            w: 168,
            h: 28,
            action: HitAction::ReloadCreds,
        });

        self.draw_text(
            buf,
            bw,
            bh,
            mx + 14,
            act_y + 35,
            mw - 28,
            10,
            "CLI: /key <provider> <token>  |  /proxy <url>  |  /model <id>",
            TEXT_DIM,
        );
    }
}
