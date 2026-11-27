// SPDX-License-Identifier: GPL-3.0-only
//! macOS Time Machine Noir Snapshots & Backup GUI for Aftercare.
//! Every step the person takes emits the `[aftercare.*]` line in the serial record
//! expected by `scripts/aftercare_gate.py`.
use crate::noir::{
    draw_card, fill_rect, window_dims, NoirPainter, AMBER, AMBER_BG, AMBER_BORDER, BG, BORDER,
    BORDER_ACTIVE, CARD, CARD_ALT, DIM, EMERALD, EMERALD_BG, EMERALD_BORDER, HEADER, MUTED, ROSE,
    ROSE_BG, ROSE_BORDER, SKY, SKY_BG, SKY_BORDER, TEXT, WELL,
};
use crate::rng::HwRng;
use crate::settings::{self, Settings};
use crate::store::{self, DirSink, DirSource, DirStore};
use aftercare_core::{
    Error, ForgetPlan, Id, InitOptions, KdfParams, Node, NodeKind, Observer, Progress,
    PruneOptions, Repository, Snapshot, Timestamp,
};
use sex_std::sex_app::{self, Event, Nav, Window};
use sex_std::sex_pdx::app_pointer::{Kind as PointerKind, Pointer};
use sex_theme::candy_ui::{fill_rounded_rect, ButtonState, ButtonStyle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const ESC: u8 = 0x01;
const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;
const ROWS: usize = 16;

/// Argon2id cost for repositories this app creates: 24 MiB, 2 passes, 1 lane.
pub const KDF: KdfParams = KdfParams { m_cost_kib: 24 * 1024, t_cost: 2, p_cost: 1 };

fn rec(line: &str) {
    sex_std::record(line);
}

fn now() -> Timestamp {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Purpose {
    Source,
    Destination,
    /// Restore this path of this snapshot into the chosen folder.
    RestoreTo(Id, String),
}

struct Chooser {
    purpose: Purpose,
    /// The folder shown; `None` lists the folders this run may use.
    at: Option<String>,
    rows: Vec<String>,
}

enum Screen {
    Home,
    Choose(Chooser),
    Snapshots,
    /// A snapshot (index into `snaps`), a folder in it, and that folder's entries.
    Browse(usize, String, Vec<Node>),
    Report(String, Vec<String>),
}

enum Prompt {
    Unlock,
    NewPassword,
    ConfirmPassword(String),
    Keep,
    Schedule,
    NewFolder,
    ConfirmForget(ForgetPlan),
    ConfirmDelete(Id),
}

impl Prompt {
    fn name(&self) -> &'static str {
        match self {
            Prompt::Unlock => "unlock",
            Prompt::NewPassword => "new-password",
            Prompt::ConfirmPassword(_) => "confirm-password",
            Prompt::Keep => "keep",
            Prompt::Schedule => "schedule",
            Prompt::NewFolder => "new-folder",
            Prompt::ConfirmForget(_) => "confirm-forget",
            Prompt::ConfirmDelete(_) => "confirm-delete",
        }
    }
    fn secret(&self) -> bool {
        matches!(self, Prompt::Unlock | Prompt::NewPassword | Prompt::ConfirmPassword(_))
    }
    fn takes_text(&self) -> bool {
        !matches!(self, Prompt::ConfirmForget(_) | Prompt::ConfirmDelete(_))
    }
    fn label(&self) -> String {
        match self {
            Prompt::Unlock => "Password".into(),
            Prompt::NewPassword => "New repository. Choose a password".into(),
            Prompt::ConfirmPassword(_) => "Type the password again".into(),
            Prompt::Keep => "Keep (last= hourly= daily= weekly= monthly= yearly=)".into(),
            Prompt::Schedule => "Daily backup at HH:MM UTC, or off".into(),
            Prompt::NewFolder => "New folder name".into(),
            Prompt::ConfirmForget(p) => format!(
                "Forget {} snapshot(s), keep {}, then prune unused data? Enter: yes, Esc: no",
                p.remove.len(),
                p.keep.len()
            ),
            Prompt::ConfirmDelete(id) => {
                format!("Forget snapshot {}? Enter: yes, Esc: no (Prune removes its data)", id.short())
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    Home,
    ChooseSource,
    ChooseDest,
    UnlockRepo,
    CreateSnapshot,
    VerifyRepo,
    OpenSnapshots,
    EditKeep,
    EditSchedule,
    ForgetPrune,
    ClearLocks,
    SelectRow(usize),
    ActivateRow(usize),
    BrowseSnapshot(usize),
    DeleteSnapshot(usize),
    ChooseUp,
    ChooseNewFolder,
    ChooseAskFiles,
    BrowseUp,
    RestoreHere,
    RestoreToOther,
    SubmitPrompt,
    CancelPrompt,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

fn render_busy_modal(
    painter: &mut NoirPainter,
    window: &mut Window,
    badge: &str,
    title: &str,
    subtitle: &str,
) {
    let (w, h) = window_dims(window);
    let buf = window.frame(w, h);
    buf.fill(BG);
    fill_rect(buf, w, h, 0, 0, w as i32, 48, HEADER);
    fill_rect(buf, w, h, 0, 47, w as i32, 1, BORDER);
    painter.text(buf, w, h, false, 18, 14, 140, 15, "AFTERCARE", TEXT);
    painter.pill(buf, w, h, 122, 13, 22, false, badge, ROSE, ROSE_BG, ROSE_BORDER);

    let cw = (w as i32 - 64).clamp(280, 500);
    let ch = 136;
    let cx = (w as i32 - cw) / 2;
    let cy = ((h as i32 - ch) / 2).max(60);
    draw_card(buf, w, h, cx, cy, cw, ch, 12, CARD, BORDER_ACTIVE);
    painter.text(buf, w, h, false, cx + 24, cy + 26, cw - 48, 15, title, TEXT);
    painter.text(buf, w, h, false, cx + 24, cy + 54, cw - 48, 12, subtitle, MUTED);
    let bar_w = cw - 48;
    fill_rounded_rect(buf, w, h, cx + 24, cy + 88, bar_w, 8, 4, WELL);
    fill_rounded_rect(buf, w, h, cx + 24, cy + 88, (bar_w / 2).max(40), 8, 4, ROSE);
    window.show_pixels(w, h);
}

/// Shows progress while the engine works, and stops it on Esc or when the window closes.
struct Watch<'a> {
    window: &'a mut Window,
    painter: &'a mut NoirPainter,
    what: &'static str,
    started: Instant,
    shown: Option<Instant>,
    cancel: bool,
    closed: bool,
    cancel_box: Option<(i32, i32, i32, i32)>,
}

impl Watch<'_> {
    fn new<'a>(window: &'a mut Window, painter: &'a mut NoirPainter, what: &'static str) -> Watch<'a> {
        Watch {
            window,
            painter,
            what,
            started: Instant::now(),
            shown: None,
            cancel: false,
            closed: false,
            cancel_box: None,
        }
    }
}

impl Observer for Watch<'_> {
    fn progress(&mut self, p: &Progress) {
        if self.shown.is_some_and(|t| t.elapsed().as_millis() < 250) {
            return;
        }
        self.shown = Some(Instant::now());
        let mut current = p.current.clone();
        if current.len() > 64 {
            current = format!("...{}", &current[current.len() - 61..]);
        }
        let (w, h) = window_dims(self.window);
        let buf = self.window.frame(w, h);
        buf.fill(BG);
        fill_rect(buf, w, h, 0, 0, w as i32, 48, HEADER);
        fill_rect(buf, w, h, 0, 47, w as i32, 1, BORDER);
        self.painter.text(buf, w, h, false, 18, 14, 140, 15, "AFTERCARE", TEXT);
        self.painter.pill(buf, w, h, 122, 13, 22, false, self.what, ROSE, ROSE_BG, ROSE_BORDER);

        let cw = (w as i32 - 64).clamp(280, 560);
        let ch = 196;
        let cx = (w as i32 - cw) / 2;
        let cy = ((h as i32 - ch) / 2).max(60);
        draw_card(buf, w, h, cx, cy, cw, ch, 12, CARD, BORDER_ACTIVE);
        self.painter.text(
            buf,
            w,
            h,
            false,
            cx + 22,
            cy + 20,
            cw - 44,
            15,
            &format!("Aftercare: {}", self.what),
            TEXT,
        );
        self.painter.text(
            buf,
            w,
            h,
            true,
            cx + 22,
            cy + 48,
            cw - 44,
            12,
            &format!(
                "{} files · {} dirs · {} read · {} new · {}s",
                p.files,
                p.dirs,
                settings::bytes(p.bytes),
                settings::bytes(p.bytes_added),
                self.started.elapsed().as_secs()
            ),
            EMERALD,
        );
        self.painter.text(buf, w, h, true, cx + 22, cy + 74, cw - 44, 11, &current, MUTED);

        let bar_w = cw - 44;
        fill_rounded_rect(buf, w, h, cx + 22, cy + 106, bar_w, 10, 5, WELL);
        let phase = ((self.started.elapsed().as_millis() / 40) as i32) % bar_w.max(60);
        let seg = (bar_w / 3).max(40);
        fill_rounded_rect(buf, w, h, cx + 22 + phase.min((bar_w - seg).max(0)), cy + 106, seg, 10, 5, ROSE);

        let bx = cx + cw - 122;
        let by = cy + ch - 44;
        self.painter
            .button(buf, w, h, bx, by, 100, 28, "Cancel (Esc)", ButtonStyle::Normal, ButtonState::Rest);
        self.cancel_box = Some((bx, by, 100, 28));
        self.window.show_pixels(w, h);
    }

    fn cancelled(&mut self) -> bool {
        while let Some(e) = self.window.poll() {
            match e {
                Event::Key(k) if k.code == ESC => self.cancel = true,
                Event::Close => {
                    self.cancel = true;
                    self.closed = true;
                }
                Event::Pointer(ptr) => {
                    if let PointerKind::Up { button: 0 } = ptr.kind {
                        if let Some((bx, by, bw, bh)) = self.cancel_box {
                            let (mx, my) = (ptr.x as i32, ptr.y as i32);
                            if mx >= bx && mx < bx + bw && my >= by && my < by + bh {
                                self.cancel = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.cancel
    }
}

struct App {
    window: &'static mut Window,
    painter: NoirPainter,
    settings: Settings,
    /// Folders this run was given (`@/...`).
    granted: Vec<String>,
    repo: Option<Repository<DirStore>>,
    rng: Option<HwRng>,
    screen: Screen,
    prompt: Option<(Prompt, String)>,
    status: String,
    cursor: usize,
    top: usize,
    snaps: Vec<(Id, Snapshot)>,
    last_verified_ok: Option<bool>,
    /// The day the "backup is due" notice was last shown.
    due_noticed: Option<i64>,
    quit: bool,
    /// The question last drawn, so each new one is recorded once.
    asked: Option<&'static str>,
    hits: Vec<HitBox>,
    mouse: (i32, i32),
    mouse_down: bool,
    hovered: Option<usize>,
}

impl App {
    fn save(&mut self) {
        match std::fs::write(settings::FILE, self.settings.format()) {
            Ok(()) => rec("[aftercare.settings.saved]"),
            Err(e) => self.status = format!("Could not save the settings: {e}"),
        }
    }

    fn rows(&self) -> usize {
        match &self.screen {
            Screen::Home => self.snaps.len(),
            Screen::Choose(c) => c.rows.len(),
            Screen::Snapshots => self.snaps.len(),
            Screen::Browse(_, _, nodes) => nodes.len(),
            Screen::Report(_, lines) => lines.len(),
        }
    }

    fn hit_btn(
        painter: &mut NoirPainter,
        hits: &mut Vec<HitBox>,
        mouse: (i32, i32),
        mouse_down: bool,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        label: &str,
        style: ButtonStyle,
        action: HitAction,
    ) {
        let st = if matches!(style, ButtonStyle::Disabled) {
            ButtonState::Rest
        } else if mouse.0 >= x && mouse.0 < x + w && mouse.1 >= y && mouse.1 < y + h {
            if mouse_down { ButtonState::Pressed } else { ButtonState::Hover }
        } else {
            ButtonState::Rest
        };
        painter.button(buf, bw, bh, x, y, w, h, label, style, st);
        if !matches!(style, ButtonStyle::Disabled) {
            hits.push(HitBox { x, y, w, h, action });
        }
    }

    fn draw(&mut self) {
        let asking = self.prompt.as_ref().map(|(p, _)| p.name());
        if asking != self.asked {
            if let Some(name) = asking {
                rec(&format!("[aftercare.prompt] name={name}"));
            }
            self.asked = asking;
        }
        self.hits.clear();

        let (w, h) = window_dims(self.window);
        let iw = w as i32;
        let ih = h as i32;
        let mouse = self.mouse;
        let mouse_down = self.mouse_down;
        let painter = &mut self.painter;
        let hits = &mut self.hits;
        let buf = self.window.frame(w, h);
        buf.fill(BG);

        macro_rules! btn {
            ($x:expr, $y:expr, $bw:expr, $bh:expr, $lbl:expr, $st:expr, $act:expr) => {
                Self::hit_btn(painter, hits, mouse, mouse_down, buf, w, h, $x, $y, $bw, $bh, $lbl, $st, $act)
            };
        }

        // Top Header Bar (48px)
        fill_rect(buf, w, h, 0, 0, iw, 48, HEADER);
        fill_rect(buf, w, h, 0, 47, iw, 1, BORDER);
        painter.text(buf, w, h, false, 18, 14, 130, 15, "AFTERCARE", TEXT);

        let (repo_label, rfg, rbg, rbrd) = match (&self.repo, &self.settings.destination) {
            (Some(_), _) => ("Unlocked", EMERALD, EMERALD_BG, EMERALD_BORDER),
            (None, Some(d)) if DirStore::exists(d) => ("Locked", ROSE, ROSE_BG, ROSE_BORDER),
            (None, Some(_)) => ("Uninitialized", AMBER, AMBER_BG, AMBER_BORDER),
            (None, None) => ("No Dest", DIM, CARD, BORDER),
        };
        painter.pill(buf, w, h, 118, 13, 22, false, repo_label, rfg, rbg, rbrd);

        // Header action buttons (Time Machine primary Rose CTA + Verify + Restore)
        let mut rx = iw - 16;
        let snap_lbl = "Create Snapshot Now";
        let snap_w = (painter.measure(false, 11, snap_lbl) + 22).max(136);
        rx -= snap_w;
        btn!(
            rx,
            10,
            snap_w,
            28,
            snap_lbl,
            if self.repo.is_some() { ButtonStyle::Primary } else { ButtonStyle::Disabled },
            HitAction::CreateSnapshot
        );
        if iw >= 560 {
            rx -= 78;
            btn!(
                rx,
                10,
                70,
                28,
                "Verify",
                if self.repo.is_some() { ButtonStyle::Normal } else { ButtonStyle::Disabled },
                HitAction::VerifyRepo
            );
            rx -= 84;
            btn!(
                rx,
                10,
                76,
                28,
                "Restore",
                if self.repo.is_some() { ButtonStyle::Normal } else { ButtonStyle::Disabled },
                HitAction::OpenSnapshots
            );
        }
        if !matches!(self.screen, Screen::Home) && rx > 280 {
            btn!(212, 10, 74, 28, "Home", ButtonStyle::Subtle, HitAction::Home);
        }

        let bottom_reserve = if self.prompt.is_some() {
            116
        } else if !self.status.is_empty() {
            40
        } else {
            16
        };
        let body_bottom = ih - bottom_reserve;
        let mut body_y = 58;

        match &self.screen {
            Screen::Home => {
                let s = &self.settings;
                let source_txt = s.source.clone().unwrap_or_else(|| "(not chosen)".into());
                let dest_txt = s.destination.clone().unwrap_or_else(|| "(not chosen)".into());
                let keep_txt = settings::format_keep(&s.keep);
                let sched_txt = match s.schedule {
                    Some(_) => format!("{} UTC", settings::format_time(s.schedule)),
                    None => "off".into(),
                };

                // Configuration & Storage Health Cards
                let two_col = iw >= 760;
                let card_h = 166;
                let left_w = if two_col { (iw - 48) * 58 / 100 } else { iw - 36 };
                draw_card(buf, w, h, 18, body_y, left_w, card_h, 10, CARD, BORDER);
                painter.text(buf, w, h, false, 32, body_y + 12, 220, 13, "Backup Configuration", TEXT);

                let rows_cfg = [
                    ("Source", source_txt, "Set (S)", HitAction::ChooseSource),
                    ("Destination", dest_txt, "Set (D)", HitAction::ChooseDest),
                    ("Retention", keep_txt, "Policy (K)", HitAction::EditKeep),
                    ("Daily", sched_txt, "Time (T)", HitAction::EditSchedule),
                ];
                let mut cy = body_y + 38;
                for (lbl, val, btn_lbl, act) in rows_cfg {
                    painter.text(buf, w, h, false, 32, cy + 4, 84, 12, lbl, DIM);
                    painter.text(buf, w, h, true, 120, cy + 4, left_w - 210, 12, &val, MUTED);
                    btn!(18 + left_w - 86, cy, 74, 22, btn_lbl, ButtonStyle::Subtle, act);
                    cy += 26;
                }
                let ub_lbl = if self.repo.is_some() {
                    "Unlocked"
                } else if self.settings.destination.as_deref().is_some_and(DirStore::exists) {
                    "Unlock (U)"
                } else {
                    "Create Repo (U)"
                };
                btn!(
                    32,
                    body_y + card_h - 32,
                    118,
                    24,
                    ub_lbl,
                    if self.repo.is_some() { ButtonStyle::Active } else { ButtonStyle::Primary },
                    HitAction::UnlockRepo
                );
                btn!(158, body_y + card_h - 32, 86, 24, "Prune (F)", ButtonStyle::Normal, HitAction::ForgetPrune);
                if left_w >= 360 {
                    btn!(252, body_y + card_h - 32, 98, 24, "Clear Locks", ButtonStyle::Subtle, HitAction::ClearLocks);
                }

                // Storage & Health Meter Card
                let (hx, hy, hw, hh) = if two_col {
                    (18 + left_w + 12, body_y, iw - 36 - left_w - 12, card_h)
                } else {
                    (18, body_y + card_h + 10, iw - 36, 114)
                };
                draw_card(buf, w, h, hx, hy, hw, hh, 10, CARD, BORDER);
                painter.text(buf, w, h, false, hx + 16, hy + 12, hw - 32, 13, "Storage & Integrity Health", TEXT);
                let total_logical: u64 = self.snaps.iter().map(|(_, sn)| sn.summary.bytes_total).sum();
                let total_stored: u64 = self.snaps.iter().map(|(_, sn)| sn.summary.bytes_stored).sum();
                let (health_lbl, hfg, hbg, hbrd) = match (self.repo.is_some(), self.last_verified_ok) {
                    (true, Some(true)) => ("Verified OK", EMERALD, EMERALD_BG, EMERALD_BORDER),
                    (true, Some(false)) => ("Check Errors", ROSE, ROSE_BG, ROSE_BORDER),
                    (true, None) => ("Encrypted Ready", EMERALD, EMERALD_BG, EMERALD_BORDER),
                    (false, _) => ("Locked / Offline", AMBER, AMBER_BG, AMBER_BORDER),
                };
                painter.pill(buf, w, h, hx + 16, hy + 38, 22, false, health_lbl, hfg, hbg, hbrd);
                painter.text(
                    buf,
                    w,
                    h,
                    true,
                    hx + 16,
                    hy + 68,
                    hw - 32,
                    12,
                    &format!("{} snapshots · {} stored", self.snaps.len(), settings::bytes(total_stored)),
                    MUTED,
                );
                let bar_w = (hw - 32).max(60);
                fill_rounded_rect(buf, w, h, hx + 16, hy + 92, bar_w, 10, 5, WELL);
                let ratio_w = if total_logical > 0 {
                    ((total_stored.min(total_logical) as u128 * bar_w as u128) / total_logical as u128) as i32
                } else if self.repo.is_some() {
                    bar_w / 4
                } else {
                    0
                };
                if ratio_w > 0 {
                    fill_rounded_rect(buf, w, h, hx + 16, hy + 92, ratio_w.clamp(10, bar_w), 10, 5, EMERALD);
                }
                if hh >= 140 {
                    painter.text(
                        buf,
                        w,
                        h,
                        true,
                        hx + 16,
                        hy + 114,
                        hw - 32,
                        11,
                        &format!("Logical: {} · Argon2id 24 MiB", settings::bytes(total_logical)),
                        DIM,
                    );
                    if let Some((id, snap)) = self.snaps.last() {
                        painter.text(
                            buf,
                            w,
                            h,
                            true,
                            hx + 16,
                            hy + 134,
                            hw - 32,
                            11,
                            &format!("Latest: {} ({} UTC)", id.short(), settings::civil(snap.time.secs)),
                            SKY,
                        );
                    }
                }

                body_y += card_h + if two_col { 12 } else { 124 };

                // Verified Snapshots Timeline Card
                let tl_h = (body_bottom - body_y).max(80);
                draw_card(buf, w, h, 18, body_y, iw - 36, tl_h, 10, CARD, BORDER);
                painter.text(buf, w, h, false, 34, body_y + 12, 260, 13, "Time Machine Snapshot Timeline", TEXT);
                btn!(
                    iw - 148,
                    body_y + 8,
                    116,
                    24,
                    "All Snapshots (L)",
                    if self.repo.is_some() { ButtonStyle::Subtle } else { ButtonStyle::Disabled },
                    HitAction::OpenSnapshots
                );
                fill_rect(buf, w, h, 26, body_y + 38, iw - 52, 1, BORDER);

                let mut ty = body_y + 48;
                if self.snaps.is_empty() {
                    let msg = if self.repo.is_some() {
                        "No snapshots yet. Click 'Create Snapshot Now' (B) to capture your first backup."
                    } else {
                        "Unlock or create an encrypted repository (U) to view and restore snapshots."
                    };
                    painter.text(buf, w, h, false, 34, ty + 6, iw - 68, 12, msg, MUTED);
                } else {
                    let row_h = 38;
                    let max_tl = ((body_bottom - ty - 8) / row_h).max(1) as usize;
                    let total = self.snaps.len();
                    for (rev_idx, (id, snap)) in self.snaps.iter().rev().take(max_tl).enumerate() {
                        let real_idx = total - 1 - rev_idx;
                        let dot_col = if rev_idx == 0 { ROSE } else { EMERALD };
                        fill_rounded_rect(buf, w, h, 38, ty + 11, 10, 10, 5, dot_col);
                        if rev_idx + 1 < max_tl && rev_idx + 1 < total {
                            fill_rect(buf, w, h, 42, ty + 22, 2, row_h - 10, BORDER_ACTIVE);
                        }
                        let n = snap.summary.files_new + snap.summary.files_changed + snap.summary.files_unchanged;
                        painter.text(
                            buf,
                            w,
                            h,
                            true,
                            60,
                            ty + 8,
                            iw - 340,
                            12,
                            &format!(
                                "{} UTC  ·  {}  ·  {} files  ·  {}",
                                settings::civil(snap.time.secs),
                                id.short(),
                                n,
                                settings::bytes(snap.summary.bytes_total)
                            ),
                            TEXT,
                        );
                        if iw >= 640 {
                            painter.pill(buf, w, h, iw - 230, ty + 6, 20, false, "Verified", EMERALD, EMERALD_BG, EMERALD_BORDER);
                        }
                        btn!(iw - 134, ty + 4, 100, 24, "Browse", ButtonStyle::Normal, HitAction::BrowseSnapshot(real_idx));
                        ty += row_h;
                    }
                }
            }
            Screen::Choose(c) => {
                let what = match &c.purpose {
                    Purpose::Source => "Source Folder to Back Up",
                    Purpose::Destination => "Encrypted Backup Destination",
                    Purpose::RestoreTo(_, _) => "Restore Destination Folder",
                };
                let lh = (body_bottom - body_y).max(120);
                draw_card(buf, w, h, 18, body_y, iw - 36, lh, 10, CARD, BORDER);
                painter.text(buf, w, h, false, 34, body_y + 12, iw - 240, 14, what, TEXT);
                let path_str = c.at.clone().unwrap_or_else(|| "Granted folders".into());
                painter.text(buf, w, h, true, 34, body_y + 34, iw - 68, 12, &path_str, SKY);
                fill_rect(buf, w, h, 26, body_y + 56, iw - 52, 1, BORDER);

                let mut ry = body_y + 64;
                let row_h = 28;
                let max_r = ((body_bottom - ry - 44) / row_h).max(1) as usize;
                for (i, line) in c.rows.iter().enumerate().skip(self.top).take(max_r) {
                    let is_sel = i == self.cursor;
                    if is_sel {
                        fill_rounded_rect(buf, w, h, 26, ry, iw - 52, row_h - 2, 6, 0x002B_1521);
                    }
                    hits.push(HitBox { x: 26, y: ry, w: iw - 52, h: row_h - 2, action: HitAction::ActivateRow(i) });
                    painter.text(buf, w, h, true, 38, ry + 6, iw - 80, 12, line, if is_sel { TEXT } else { MUTED });
                    ry += row_h;
                }
                let by = body_y + lh - 36;
                btn!(34, by, 96, 26, "Up (Bksp)", ButtonStyle::Normal, HitAction::ChooseUp);
                if c.at.is_some() {
                    btn!(138, by, 122, 26, "New Folder (N)", ButtonStyle::Normal, HitAction::ChooseNewFolder);
                }
                btn!(iw - 230, by, 110, 26, "Ask Files (P)", ButtonStyle::Normal, HitAction::ChooseAskFiles);
                btn!(iw - 112, by, 80, 26, "Back", ButtonStyle::Subtle, HitAction::Home);
            }
            Screen::Snapshots => {
                let lh = (body_bottom - body_y).max(120);
                draw_card(buf, w, h, 18, body_y, iw - 36, lh, 10, CARD, BORDER);
                painter.text(buf, w, h, false, 34, body_y + 12, 260, 14, "Verified Snapshots Timeline", TEXT);
                painter.pill(
                    buf,
                    w,
                    h,
                    290,
                    body_y + 10,
                    22,
                    true,
                    &format!("{} total", self.snaps.len()),
                    SKY,
                    SKY_BG,
                    SKY_BORDER,
                );
                fill_rect(buf, w, h, 26, body_y + 42, iw - 52, 1, BORDER);

                let mut ry = body_y + 50;
                let row_h = 34;
                let max_r = ((body_bottom - ry - 44) / row_h).max(1) as usize;
                let snaps_len = self.snaps.len();
                for i in (self.top..snaps_len).take(max_r) {
                    let (id, snap) = &self.snaps[i];
                    let is_sel = i == self.cursor;
                    if is_sel {
                        fill_rounded_rect(buf, w, h, 26, ry, iw - 52, row_h - 2, 6, 0x002B_1521);
                    }
                    hits.push(HitBox { x: 26, y: ry, w: iw - 220, h: row_h - 2, action: HitAction::SelectRow(i) });
                    let n = snap.summary.files_new + snap.summary.files_changed + snap.summary.files_unchanged;
                    let line = format!(
                        "{} UTC  {}  {} files  {}  {}",
                        settings::civil(snap.time.secs),
                        id.short(),
                        n,
                        settings::bytes(snap.summary.bytes_total),
                        snap.paths.join(" ")
                    );
                    painter.text(buf, w, h, true, 36, ry + 8, iw - 250, 12, &line, if is_sel { TEXT } else { MUTED });
                    btn!(iw - 196, ry + 4, 78, 24, "Browse", ButtonStyle::Primary, HitAction::BrowseSnapshot(i));
                    btn!(iw - 110, ry + 4, 76, 24, "Forget", ButtonStyle::Danger, HitAction::DeleteSnapshot(i));
                    ry += row_h;
                }
                if snaps_len == 0 {
                    painter.text(buf, w, h, false, 34, ry + 8, iw - 68, 12, "No snapshots in this repository.", MUTED);
                }
                let by = body_y + lh - 36;
                btn!(34, by, 130, 26, "Verify Data (C)", ButtonStyle::Normal, HitAction::VerifyRepo);
                btn!(iw - 112, by, 80, 26, "Back", ButtonStyle::Subtle, HitAction::Home);
            }
            Screen::Browse(i, path, nodes) => {
                let i = *i;
                let (id, snap_secs) = (self.snaps[i].0, self.snaps[i].1.time.secs);
                let lh = (body_bottom - body_y).max(120);
                draw_card(buf, w, h, 18, body_y, iw - 36, lh, 10, CARD, BORDER);
                painter.text(
                    buf,
                    w,
                    h,
                    false,
                    34,
                    body_y + 12,
                    iw - 68,
                    14,
                    &format!("Snapshot {} · {} UTC", id.short(), settings::civil(snap_secs)),
                    TEXT,
                );
                painter.text(buf, w, h, true, 34, body_y + 34, iw - 68, 12, path, SKY);
                fill_rect(buf, w, h, 26, body_y + 56, iw - 52, 1, BORDER);

                let mut ry = body_y + 64;
                let row_h = 28;
                let max_r = ((body_bottom - ry - 44) / row_h).max(1) as usize;
                for (idx, n) in nodes.iter().enumerate().skip(self.top).take(max_r) {
                    let is_sel = idx == self.cursor;
                    if is_sel {
                        fill_rounded_rect(buf, w, h, 26, ry, iw - 52, row_h - 2, 6, 0x002B_1521);
                    }
                    hits.push(HitBox { x: 26, y: ry, w: iw - 52, h: row_h - 2, action: HitAction::ActivateRow(idx) });
                    let label = match n.kind {
                        NodeKind::Dir => format!("{}/", n.name),
                        _ => format!("{}   ({})", n.name, settings::bytes(n.size)),
                    };
                    painter.text(
                        buf,
                        w,
                        h,
                        true,
                        38,
                        ry + 6,
                        iw - 80,
                        12,
                        &label,
                        if n.kind == NodeKind::Dir { SKY } else if is_sel { TEXT } else { MUTED },
                    );
                    ry += row_h;
                }
                let by = body_y + lh - 36;
                btn!(34, by, 96, 26, "Up (Bksp)", ButtonStyle::Normal, HitAction::BrowseUp);
                btn!(138, by, 146, 26, "Restore Here (R)", ButtonStyle::Primary, HitAction::RestoreHere);
                btn!(292, by, 146, 26, "Restore To… (T)", ButtonStyle::Normal, HitAction::RestoreToOther);
                btn!(iw - 112, by, 80, 26, "Snapshots", ButtonStyle::Subtle, HitAction::OpenSnapshots);
            }
            Screen::Report(title, lines) => {
                let lh = (body_bottom - body_y).max(120);
                draw_card(buf, w, h, 18, body_y, iw - 36, lh, 10, CARD, BORDER);
                painter.text(buf, w, h, false, 34, body_y + 12, iw - 140, 14, &format!("Integrity Report: {}", title), TEXT);
                fill_rect(buf, w, h, 26, body_y + 40, iw - 52, 1, BORDER);
                let mut ry = body_y + 50;
                let row_h = 24;
                let max_r = ((body_bottom - ry - 44) / row_h).max(1) as usize;
                for line in lines.iter().skip(self.top).take(max_r) {
                    let col = if line.starts_with("error") || line.starts_with("ERRORS") {
                        ROSE
                    } else if line.starts_with("No errors") {
                        EMERALD
                    } else {
                        MUTED
                    };
                    painter.text(buf, w, h, true, 34, ry + 4, iw - 68, 12, line, col);
                    ry += row_h;
                }
                btn!(iw - 112, body_y + lh - 36, 80, 26, "Back (Esc)", ButtonStyle::Normal, HitAction::Home);
            }
        }

        // Prompt Modal Drawer OR Status Bar
        if let Some((p, text)) = &self.prompt {
            let py = ih - 108;
            draw_card(buf, w, h, 18, py, iw - 36, 94, 10, CARD_ALT, ROSE);
            let lbl = p.label();
            painter.text(buf, w, h, false, 32, py + 12, iw - 64, 12, &lbl, TEXT);
            if p.takes_text() {
                let shown = if p.secret() { "*".repeat(text.chars().count()) } else { text.clone() };
                let inp_w = (iw - 256).max(120);
                draw_card(buf, w, h, 32, py + 38, inp_w, 34, 8, WELL, BORDER_ACTIVE);
                painter.text(buf, w, h, true, 44, py + 48, inp_w - 24, 13, &format!("{}_", shown), TEXT);
                btn!(32 + inp_w + 10, py + 40, 92, 30, "OK (Ret)", ButtonStyle::Primary, HitAction::SubmitPrompt);
                btn!(32 + inp_w + 110, py + 40, 86, 30, "Cancel", ButtonStyle::Normal, HitAction::CancelPrompt);
            } else {
                btn!(32, py + 46, 128, 30, "Confirm (Enter)", ButtonStyle::Danger, HitAction::SubmitPrompt);
                btn!(170, py + 46, 98, 30, "Cancel (Esc)", ButtonStyle::Normal, HitAction::CancelPrompt);
            }
        } else if !self.status.is_empty() {
            let sy = ih - 32;
            fill_rounded_rect(buf, w, h, 18, sy, iw - 36, 24, 6, CARD);
            painter.text(buf, w, h, false, 28, sy + 5, iw - 56, 11, &self.status, MUTED);
        }

        self.window.show_pixels(w, h);
    }

    fn go(&mut self, screen: Screen) {
        self.screen = screen;
        self.cursor = 0;
        self.top = 0;
    }

    // ---- folders ------------------------------------------------------------------

    fn choose(&mut self, purpose: Purpose) {
        let rows = self.roots(&purpose);
        rec(&format!("[aftercare.choose] purpose={} roots={}", purpose_name(&purpose), rows.len()));
        self.go(Screen::Choose(Chooser { purpose, at: None, rows }));
    }

    fn roots(&self, purpose: &Purpose) -> Vec<String> {
        let mut rows = self.granted.clone();
        if *purpose != Purpose::Source {
            rows.push("/".into());
        }
        rows
    }

    fn open_folder(&mut self, at: String) {
        let Screen::Choose(c) = &mut self.screen else { return };
        match store::list(&at) {
            Ok(entries) => {
                c.rows = std::iter::once("[ Use this folder ]".to_string())
                    .chain(entries.into_iter().filter(|e| e.dir).map(|e| format!("{}/", e.name)))
                    .collect();
                rec(&format!("[aftercare.folder] path={at} rows={}", c.rows.len()));
                c.at = Some(at);
                self.cursor = 0;
                self.top = 0;
                self.status.clear();
            }
            Err(e) => self.status = format!("Can't open {at}: {e}"),
        }
    }

    fn chose(&mut self, folder: String) {
        let Screen::Choose(c) = &self.screen else { return };
        let purpose = c.purpose.clone();
        rec(&format!("[aftercare.chose] purpose={} path={folder}", purpose_name(&purpose)));
        match purpose {
            Purpose::Source => {
                if !folder.starts_with("@/") {
                    self.status = "Back up a folder you gave Aftercare (run aftercare @/FOLDER).".into();
                    return;
                }
                self.settings.source = Some(folder.clone());
                self.status = format!("Source: {folder}");
                self.save();
            }
            Purpose::Destination => {
                if self.settings.destination.as_deref() != Some(&folder) {
                    self.repo = None;
                    self.snaps.clear();
                }
                self.status = if DirStore::exists(&folder) {
                    format!("Destination: {folder}. It holds a repository: U unlocks it.")
                } else {
                    format!("Destination: {folder}. U creates a repository there.")
                };
                self.settings.destination = Some(folder);
                self.save();
            }
            Purpose::RestoreTo(id, path) => {
                self.go(Screen::Home);
                self.restore(id, &path, &folder);
                return;
            }
        }
        self.go(Screen::Home);
    }

    fn pick_with_files(&mut self) {
        let mut out = [0u8; sex_std::sex_pdx::fs::PATH_BYTES];
        let picked = sex_app::files::pick(self.window, sex_app::files::Pick::Folder, b"", &mut out)
            .map(|p| String::from_utf8_lossy(p).into_owned());
        match picked {
            Ok(p) => {
                let p = if p.starts_with('@') { p } else { format!("@{p}") };
                if !self.granted.contains(&p) {
                    self.granted.push(p.clone());
                }
                self.chose(p);
            }
            Err(_) => self.status = "Files gave no folder.".into(),
        }
    }

    // ---- repository ---------------------------------------------------------------

    fn unlock(&mut self) {
        let Some(dest) = self.settings.destination.clone() else {
            self.status = "Choose a destination first (D).".into();
            return;
        };
        if self.repo.is_some() {
            self.status = "The repository is already unlocked.".into();
            return;
        }
        if DirStore::exists(&dest) {
            self.prompt = Some((Prompt::Unlock, String::new()));
        } else if self.rng.is_none() {
            self.status = "This computer has no hardware random generator: Aftercare can't make keys here.".into();
        } else {
            if let Err(e) = std::fs::metadata(&dest) {
                self.status = format!("Can't use {dest}: {e}");
                return;
            }
            self.prompt = Some((Prompt::NewPassword, String::new()));
        }
    }

    fn opened(&mut self, repo: Repository<DirStore>) {
        let mut repo = repo;
        repo.set_host("sexos");
        self.repo = Some(repo);
        self.load_snapshots();
    }

    fn load_snapshots(&mut self) {
        let Some(repo) = self.repo.as_mut() else { return };
        match repo.snapshots() {
            Ok(mut v) => {
                v.sort_by_key(|(_, s)| (s.time.secs, s.time.nanos));
                rec(&format!("[aftercare.snapshots] count={}", v.len()));
                self.snaps = v;
            }
            Err(e) => self.status = format!("Can't read the snapshots: {e}"),
        }
    }

    fn submit(&mut self, prompt: Prompt, text: String) {
        match prompt {
            Prompt::Unlock => {
                let dest = self.settings.destination.clone().unwrap_or_default();
                render_busy_modal(&mut self.painter, self.window, "Decrypting", "Unlocking Encrypted Repository", &dest);
                let t = Instant::now();
                match Repository::open(DirStore::new(&dest), text.as_bytes()) {
                    Ok(repo) => {
                        rec(&format!("[aftercare.unlock] result=ok ms={}", t.elapsed().as_millis()));
                        self.status = "Unlocked.".into();
                        self.opened(repo);
                    }
                    Err(Error::WrongPassword) => {
                        rec(&format!("[aftercare.unlock] result=wrong-password ms={}", t.elapsed().as_millis()));
                        self.status = "Wrong password. Try again, or Esc.".into();
                        self.prompt = Some((Prompt::Unlock, String::new()));
                    }
                    Err(e) => {
                        rec(&format!("[aftercare.unlock] result=error error={e}"));
                        self.status = format!("Can't open the repository: {e}");
                    }
                }
            }
            Prompt::NewPassword => {
                if text.is_empty() {
                    self.status = "The password can't be empty.".into();
                    self.prompt = Some((Prompt::NewPassword, String::new()));
                } else {
                    self.prompt = Some((Prompt::ConfirmPassword(text), String::new()));
                }
            }
            Prompt::ConfirmPassword(first) => {
                if first != text {
                    self.status = "The passwords differ. Choose one again.".into();
                    self.prompt = Some((Prompt::NewPassword, String::new()));
                    return;
                }
                let dest = self.settings.destination.clone().unwrap_or_default();
                let Some(rng) = self.rng.as_mut() else { return };
                render_busy_modal(&mut self.painter, self.window, "Initializing", "Creating Encrypted Repository", &dest);
                let t = Instant::now();
                let opts = InitOptions { kdf: KDF, ..Default::default() };
                match Repository::init(DirStore::new(&dest), text.as_bytes(), rng, &opts) {
                    Ok(repo) => {
                        rec(&format!(
                            "[aftercare.repo.init] path={dest} kdf_kib={} kdf_passes={} ms={}",
                            KDF.m_cost_kib,
                            KDF.t_cost,
                            t.elapsed().as_millis()
                        ));
                        self.status = format!("Created a repository in {dest}. Keep the password safe: without it the backups can't be read.");
                        self.opened(repo);
                    }
                    Err(e) => self.status = format!("Can't create the repository: {e}"),
                }
            }
            Prompt::Keep => match settings::parse_keep(&text) {
                Ok(p) => {
                    self.settings.keep = p;
                    rec(&format!("[aftercare.keep] policy={}", settings::format_keep(&self.settings.keep)));
                    self.status = format!("Keep: {}", settings::format_keep(&self.settings.keep));
                    self.save();
                }
                Err(e) => {
                    self.status = e;
                    self.prompt = Some((Prompt::Keep, text));
                }
            },
            Prompt::Schedule => match settings::parse_time(&text) {
                Ok(t) => {
                    self.settings.schedule = t;
                    rec(&format!("[aftercare.schedule] at={}", settings::format_time(t)));
                    self.status = format!("Daily backup: {}", settings::format_time(t));
                    self.save();
                }
                Err(e) => {
                    self.status = e;
                    self.prompt = Some((Prompt::Schedule, text));
                }
            },
            Prompt::NewFolder => {
                let Screen::Choose(c) = &self.screen else { return };
                let Some(at) = c.at.clone() else { return };
                if text.is_empty() || text.contains('/') || text == "." || text == ".." {
                    self.status = "Not a folder name.".into();
                    return;
                }
                let p = store::join(&at, &text);
                match std::fs::create_dir(&p) {
                    Ok(()) => {
                        rec(&format!("[aftercare.mkdir] path={p}"));
                        self.open_folder(at);
                    }
                    Err(e) => self.status = format!("Can't make {p}: {e}"),
                }
            }
            Prompt::ConfirmForget(_) => self.forget_and_prune(),
            Prompt::ConfirmDelete(id) => {
                let (Some(repo), Some(rng)) = (self.repo.as_mut(), self.rng.as_mut()) else { return };
                match repo.forget_snapshots(&[id], now(), rng) {
                    Ok(()) => {
                        rec(&format!("[aftercare.forget] snapshot={}", id.short()));
                        self.status = format!("Forgot snapshot {}. F prunes data no snapshot uses.", id.short());
                    }
                    Err(e) => self.status = format!("Can't forget it: {e}"),
                }
                self.load_snapshots();
                self.cursor = self.cursor.min(self.snaps.len().saturating_sub(1));
            }
        }
    }

    fn need_repo(&mut self) -> bool {
        if self.repo.is_none() {
            self.status = "Unlock the repository first (U).".into();
            return false;
        }
        true
    }

    fn backup(&mut self, scheduled: bool) {
        if !self.need_repo() {
            return;
        }
        let Some(source) = self.settings.source.clone() else {
            self.status = "Choose a source folder first (S).".into();
            return;
        };
        let Some(rng) = self.rng.as_mut() else {
            self.status = "No hardware random generator.".into();
            return;
        };
        let repo = self.repo.as_mut().unwrap();
        let path = source[1..].to_string();
        let opts = aftercare_core::BackupOptions {
            host: "sexos".into(),
            tags: if scheduled { vec!["scheduled".into()] } else { vec![] },
            time: now(),
            parent: aftercare_core::ParentChoice::Latest,
        };
        let nonce = rand_core::RngCore::next_u32(rng);
        let mut watch = Watch::new(self.window, &mut self.painter, "backing up");
        let started = Instant::now();
        let r = repo.backup(&mut DirSource::new("@", nonce), &[path.as_str()], &opts, rng, &mut watch);
        let closed = watch.closed;
        let ms = started.elapsed().as_millis();
        match r {
            Ok((id, snap)) => {
                let s = &snap.summary;
                rec(&format!(
                    "[aftercare.backup.done] snapshot={} files_new={} files_changed={} files_unchanged={} dirs={} bytes={} added={} errors={} scheduled={} ms={ms}",
                    id.short(), s.files_new, s.files_changed, s.files_unchanged, s.dirs, s.bytes_total, s.bytes_added, s.errors, scheduled as u8
                ));
                self.status = format!(
                    "Backed up {} files ({}): {} new, {} changed, {} stored. Snapshot {}.{}",
                    s.files_new + s.files_changed + s.files_unchanged,
                    settings::bytes(s.bytes_total),
                    s.files_new,
                    s.files_changed,
                    settings::bytes(s.bytes_stored),
                    id.short(),
                    if s.errors > 0 { format!(" {} could not be read.", s.errors) } else { String::new() }
                );
            }
            Err(Error::Cancelled) => {
                rec(&format!("[aftercare.backup.cancelled] ms={ms}"));
                self.status = "Backup cancelled. What was already stored is reused by the next one.".into();
            }
            Err(e) => {
                rec(&format!("[aftercare.backup.failed] error={e}"));
                self.status = match e {
                    Error::Locked(_) => format!("{e}. If no other backup is running, X clears stale locks."),
                    e => format!("Backup failed: {e}"),
                };
            }
        }
        self.quit |= closed;
        self.load_snapshots();
    }

    fn restore(&mut self, id: Id, path: &str, target: &str) {
        let Some(repo) = self.repo.as_mut() else { return };
        let mut sink = DirSink::new(target);
        let mut watch = Watch::new(self.window, &mut self.painter, "restoring");
        let started = Instant::now();
        let r = repo.restore(&id, path, &mut sink, &mut watch);
        self.quit |= watch.closed;
        for w in &sink.written {
            rec(&format!("[aftercare.restore.file] path={} bytes={} sha256={}", w.path, w.bytes, w.sha256));
        }
        match r {
            Ok(s) => {
                rec(&format!(
                    "[aftercare.restore.done] path={path} into={target} files={} dirs={} bytes={} ms={}",
                    s.files, s.dirs, s.bytes, started.elapsed().as_millis()
                ));
                self.status = format!("Restored {path} into {target}: {} files, {} (every chunk verified).", s.files, settings::bytes(s.bytes));
            }
            Err(Error::Cancelled) => {
                rec("[aftercare.restore.cancelled]");
                self.status = format!("Restore cancelled after {} files.", sink.written.len());
            }
            Err(e) => {
                rec(&format!("[aftercare.restore.failed] error={e}"));
                self.status = format!("Restore failed: {e}");
            }
        }
    }

    fn forget_and_prune(&mut self) {
        let (Some(repo), Some(rng)) = (self.repo.as_mut(), self.rng.as_mut()) else { return };
        render_busy_modal(&mut self.painter, self.window, "Pruning", "Forgetting & Pruning Repository", "Reclaiming unused encrypted pack blobs...");
        let t = now();
        let plan = match repo.forget(&self.settings.keep, t, rng) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Forget failed: {e}");
                return;
            }
        };
        rec(&format!("[aftercare.forget] kept={} removed={}", plan.keep.len(), plan.remove.len()));
        match repo.prune(&PruneOptions { now: t, ..Default::default() }, rng) {
            Ok(p) => {
                rec(&format!(
                    "[aftercare.prune] packs_deleted={} packs_repacked={} blobs_removed={} bytes_before={} bytes_after={}",
                    p.packs_deleted, p.packs_repacked, p.blobs_removed, p.bytes_before, p.bytes_after
                ));
                self.status = format!(
                    "Forgot {} snapshot(s), kept {}. Pruned {} unused pieces: {} -> {}.",
                    plan.remove.len(),
                    plan.keep.len(),
                    p.blobs_removed,
                    settings::bytes(p.bytes_before),
                    settings::bytes(p.bytes_after)
                );
            }
            Err(e) => self.status = format!("Forgot {} snapshot(s); prune failed: {e}", plan.remove.len()),
        }
        self.load_snapshots();
    }

    fn check(&mut self, read_data: bool) {
        let Some(repo) = self.repo.as_mut() else { return };
        render_busy_modal(
            &mut self.painter,
            self.window,
            "Verifying",
            if read_data { "Verifying All Repository Data Blobs" } else { "Checking Repository Index & Trees" },
            "Checking cryptographic hashes and pack integrity...",
        );
        let t = Instant::now();
        match repo.check(read_data) {
            Ok(r) => {
                self.last_verified_ok = Some(r.is_ok());
                rec(&format!(
                    "[aftercare.check] errors={} warnings={} snapshots={} trees={} packs={} blobs_verified={} read_data={} ms={}",
                    r.errors.len(), r.warnings.len(), r.snapshots, r.trees, r.packs, r.blobs_verified, read_data as u8, t.elapsed().as_millis()
                ));
                let mut lines = vec![format!(
                    "{}: {} snapshots, {} folders, {} packs{}",
                    if r.is_ok() { "No errors" } else { "ERRORS FOUND" },
                    r.snapshots,
                    r.trees,
                    r.packs,
                    if read_data { format!(", {} pieces read and verified", r.blobs_verified) } else { String::new() }
                )];
                lines.extend(r.errors.iter().map(|e| format!("error: {e}")));
                lines.extend(r.warnings.iter().map(|w| format!("warning: {w}")));
                self.go(Screen::Report("check".into(), lines));
            }
            Err(e) => self.status = format!("Check failed: {e}"),
        }
    }

    fn browse(&mut self, i: usize, path: String) {
        let id = self.snaps[i].0;
        let Some(repo) = self.repo.as_mut() else { return };
        match repo.ls(&id, &path) {
            Ok(nodes) => {
                rec(&format!("[aftercare.browse] snapshot={} path={path} rows={}", id.short(), nodes.len()));
                self.go(Screen::Browse(i, path, nodes));
            }
            Err(e) => self.status = format!("Can't open {path}: {e}"),
        }
    }

    /// The daily backup, while the app is open.
    fn tick(&mut self) -> bool {
        let t = now().secs;
        if self.prompt.is_some() || !self.settings.due(t) {
            return false;
        }
        let day = t.div_euclid(86_400);
        if self.repo.is_none() || self.settings.source.is_none() {
            if self.due_noticed != Some(day) {
                self.due_noticed = Some(day);
                rec("[aftercare.schedule.due] unlocked=0");
                self.status = "The daily backup is due: unlock the repository (U) and it starts.".into();
                return true;
            }
            return false;
        }
        self.settings.last_scheduled_day = Some(day);
        self.save();
        rec("[aftercare.schedule.run]");
        self.go(Screen::Home);
        self.backup(true);
        true
    }

    fn open_snapshots_screen(&mut self) {
        if self.need_repo() {
            self.load_snapshots();
            let last = self.snaps.len().saturating_sub(1);
            self.go(Screen::Snapshots);
            self.cursor = last;
            self.top = last.saturating_sub(ROWS - 1);
        }
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.hits.iter().rposition(|h| x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)
    }

    fn pointer(&mut self, ptr: Pointer) -> bool {
        self.mouse = (ptr.x as i32, ptr.y as i32);
        match ptr.kind {
            PointerKind::Move => {
                let h = self.hit_at(self.mouse.0, self.mouse.1);
                if h != self.hovered {
                    self.hovered = h;
                    return true;
                }
                false
            }
            PointerKind::Leave => {
                self.mouse_down = false;
                if self.hovered.is_some() {
                    self.hovered = None;
                    return true;
                }
                false
            }
            PointerKind::Down { button: 0 } => {
                self.mouse_down = true;
                self.hovered = self.hit_at(self.mouse.0, self.mouse.1);
                true
            }
            PointerKind::Up { button: 0 } => {
                self.mouse_down = false;
                if let Some(idx) = self.hit_at(self.mouse.0, self.mouse.1) {
                    let action = self.hits[idx].action;
                    self.dispatch_hit(action);
                }
                true
            }
            PointerKind::Wheel { steps } => {
                let delta = -(steps as isize);
                let max_top = self.rows().saturating_sub(1);
                self.top = (self.top as isize + delta).clamp(0, max_top as isize) as usize;
                true
            }
            _ => false,
        }
    }

    fn dispatch_hit(&mut self, action: HitAction) {
        match action {
            HitAction::Home => { self.status.clear(); self.go(Screen::Home); }
            HitAction::ChooseSource => self.choose(Purpose::Source),
            HitAction::ChooseDest => self.choose(Purpose::Destination),
            HitAction::UnlockRepo => self.unlock(),
            HitAction::CreateSnapshot => self.backup(false),
            HitAction::VerifyRepo => if self.need_repo() { self.check(true); },
            HitAction::OpenSnapshots => self.open_snapshots_screen(),
            HitAction::EditKeep => self.prompt = Some((Prompt::Keep, settings::format_keep(&self.settings.keep))),
            HitAction::EditSchedule => self.prompt = Some((Prompt::Schedule, settings::format_time(self.settings.schedule))),
            HitAction::ForgetPrune => {
                if self.need_repo() {
                    match self.repo.as_mut().unwrap().plan_forget(&self.settings.keep) {
                        Ok(plan) => self.prompt = Some((Prompt::ConfirmForget(plan), String::new())),
                        Err(e) => self.status = format!("Can't plan: {e}"),
                    }
                }
            }
            HitAction::ClearLocks => {
                if self.need_repo() {
                    match self.repo.as_mut().unwrap().remove_locks(None) {
                        Ok(n) => {
                            rec(&format!("[aftercare.locks.removed] count={n}"));
                            self.status = format!("Removed {n} lock(s).");
                        }
                        Err(e) => self.status = format!("Can't remove locks: {e}"),
                    }
                }
            }
            HitAction::SelectRow(idx) => {
                self.cursor = idx;
                rec(&format!("[aftercare.cursor] row={}", self.cursor));
            }
            HitAction::ActivateRow(idx) => {
                self.cursor = idx;
                match &self.screen {
                    Screen::Choose(c) => {
                        let (at, row) = (c.at.clone(), c.rows.get(idx).cloned());
                        match (at, row) {
                            (None, Some(root)) => self.open_folder(root),
                            (Some(at), Some(_)) if idx == 0 => self.chose(at),
                            (Some(at), Some(row)) => self.open_folder(store::join(&at, row.trim_end_matches('/'))),
                            _ => {}
                        }
                    }
                    Screen::Browse(i, path, nodes) => {
                        let (i, path, node) = (*i, path.clone(), nodes.get(idx).cloned());
                        if let Some(n) = node.filter(|n| n.kind == NodeKind::Dir) {
                            self.browse(i, store::join(&path, &n.name));
                        }
                    }
                    _ => {}
                }
            }
            HitAction::BrowseSnapshot(idx) => {
                if idx < self.snaps.len() {
                    self.browse(idx, "/".into());
                }
            }
            HitAction::DeleteSnapshot(idx) => {
                if let Some((id, _)) = self.snaps.get(idx) {
                    self.prompt = Some((Prompt::ConfirmDelete(*id), String::new()));
                }
            }
            HitAction::ChooseUp => {
                if let Screen::Choose(c) = &self.screen {
                    let at = c.at.clone();
                    let purpose = c.purpose.clone();
                    match at {
                        Some(at) if self.roots(&purpose).contains(&at) => {
                            let rows = self.roots(&purpose);
                            if let Screen::Choose(c) = &mut self.screen {
                                c.at = None;
                                c.rows = rows;
                            }
                            self.cursor = 0;
                            self.top = 0;
                        }
                        Some(at) => self.open_folder(store::parent(&at)),
                        None => {}
                    }
                }
            }
            HitAction::ChooseNewFolder => self.prompt = Some((Prompt::NewFolder, String::new())),
            HitAction::ChooseAskFiles => self.pick_with_files(),
            HitAction::BrowseUp => {
                if let Screen::Browse(i, path, _) = &self.screen {
                    let (i, path) = (*i, path.clone());
                    if path == "/" {
                        self.go(Screen::Snapshots);
                        self.cursor = i;
                    } else {
                        self.browse(i, store::parent(&path));
                    }
                }
            }
            HitAction::RestoreHere => {
                if let Screen::Browse(i, path, nodes) = &self.screen {
                    if let Some(n) = nodes.get(self.cursor).cloned() {
                        let id = self.snaps[*i].0;
                        let target = format!("@{path}");
                        let full = store::join(path, &n.name);
                        self.restore(id, &full, &target);
                    }
                }
            }
            HitAction::RestoreToOther => {
                if let Screen::Browse(i, path, nodes) = &self.screen {
                    if let Some(n) = nodes.get(self.cursor).cloned() {
                        let id = self.snaps[*i].0;
                        self.choose(Purpose::RestoreTo(id, store::join(path, &n.name)));
                    }
                }
            }
            HitAction::SubmitPrompt => {
                if let Some((prompt, text)) = self.prompt.take() {
                    self.status.clear();
                    self.submit(prompt, text);
                }
            }
            HitAction::CancelPrompt => {
                self.prompt = None;
                self.status = "Cancelled.".into();
            }
        }
    }

    // ---- keys -----------------------------------------------------------------------

    fn key(&mut self, key: sex_std::sex_pdx::app_key::Key) {
        if let Some((prompt, mut text)) = self.prompt.take() {
            match key.code {
                ESC => {
                    self.status = "Cancelled.".into();
                    return;
                }
                ENTER => {
                    self.status.clear();
                    return self.submit(prompt, text);
                }
                BACKSPACE => {
                    text.pop();
                }
                _ => {
                    if let Some(c) = sex_app::text(key).filter(|c| !c.is_control()) {
                        if prompt.takes_text() && text.len() < 200 {
                            text.push(c);
                        }
                    }
                }
            }
            rec(&format!("[aftercare.input] len={}", text.chars().count()));
            self.prompt = Some((prompt, text));
            return;
        }
        let rows = self.rows();
        match sex_app::nav(key) {
            Some(Nav::Down) => {
                self.cursor = (self.cursor + 1).min(rows.saturating_sub(1));
                if self.cursor >= self.top + ROWS {
                    self.top = self.cursor + 1 - ROWS;
                }
                rec(&format!("[aftercare.cursor] row={}", self.cursor));
                return;
            }
            Some(Nav::Up) => {
                self.cursor = self.cursor.saturating_sub(1);
                self.top = self.top.min(self.cursor);
                rec(&format!("[aftercare.cursor] row={}", self.cursor));
                return;
            }
            Some(Nav::Home) => {
                self.cursor = 0;
                self.top = 0;
                return;
            }
            Some(Nav::End) => {
                self.cursor = rows.saturating_sub(1);
                self.top = self.cursor.saturating_sub(ROWS - 1);
                return;
            }
            Some(Nav::Delete) => {
                if let Screen::Snapshots = self.screen {
                    if let Some((id, _)) = self.snaps.get(self.cursor) {
                        self.prompt = Some((Prompt::ConfirmDelete(*id), String::new()));
                    }
                }
                return;
            }
            _ => {}
        }
        let ch = sex_app::typed(key).map(|c| c.to_ascii_lowercase());
        match &self.screen {
            Screen::Home => match ch {
                Some(b's') => self.choose(Purpose::Source),
                Some(b'd') => self.choose(Purpose::Destination),
                Some(b'u') => self.unlock(),
                Some(b'b') => self.backup(false),
                Some(b'l') => self.open_snapshots_screen(),
                Some(b'k') => self.prompt = Some((Prompt::Keep, settings::format_keep(&self.settings.keep))),
                Some(b't') => self.prompt = Some((Prompt::Schedule, settings::format_time(self.settings.schedule))),
                Some(b'f') => {
                    if self.need_repo() {
                        match self.repo.as_mut().unwrap().plan_forget(&self.settings.keep) {
                            Ok(plan) => self.prompt = Some((Prompt::ConfirmForget(plan), String::new())),
                            Err(e) => self.status = format!("Can't plan: {e}"),
                        }
                    }
                }
                Some(b'c') => {
                    if self.need_repo() {
                        self.check(key.shift);
                    }
                }
                Some(b'x') => {
                    if self.need_repo() {
                        match self.repo.as_mut().unwrap().remove_locks(None) {
                            Ok(n) => {
                                rec(&format!("[aftercare.locks.removed] count={n}"));
                                self.status = format!("Removed {n} lock(s).");
                            }
                            Err(e) => self.status = format!("Can't remove locks: {e}"),
                        }
                    }
                }
                _ => {}
            },
            Screen::Choose(c) => {
                let (at, row) = (c.at.clone(), c.rows.get(self.cursor).cloned());
                let purpose = c.purpose.clone();
                match (key.code, ch) {
                    (ESC, _) => {
                        self.status.clear();
                        self.go(Screen::Home)
                    }
                    (ENTER, _) => match (at, row) {
                        (None, Some(root)) => self.open_folder(root),
                        (Some(at), Some(_)) if self.cursor == 0 => self.chose(at),
                        (Some(at), Some(row)) => self.open_folder(store::join(&at, row.trim_end_matches('/'))),
                        _ => {}
                    },
                    (BACKSPACE, _) => match at {
                        Some(at) if self.roots(&purpose).contains(&at) => {
                            let rows = self.roots(&purpose);
                            if let Screen::Choose(c) = &mut self.screen {
                                c.at = None;
                                c.rows = rows;
                            }
                            self.cursor = 0;
                            self.top = 0;
                        }
                        Some(at) => self.open_folder(store::parent(&at)),
                        None => {}
                    },
                    (_, Some(b'n')) if at.is_some() => self.prompt = Some((Prompt::NewFolder, String::new())),
                    (_, Some(b'p')) => self.pick_with_files(),
                    _ => {}
                }
            }
            Screen::Snapshots => match key.code {
                ESC => self.go(Screen::Home),
                ENTER if self.cursor < self.snaps.len() => self.browse(self.cursor, "/".into()),
                _ => {}
            },
            Screen::Browse(i, path, nodes) => {
                let (i, path, node) = (*i, path.clone(), nodes.get(self.cursor).cloned());
                let id = self.snaps[i].0;
                match (key.code, ch) {
                    (ESC, _) => {
                        let last = i;
                        self.go(Screen::Snapshots);
                        self.cursor = last;
                    }
                    (ENTER, _) => {
                        if let Some(n) = node.filter(|n| n.kind == NodeKind::Dir) {
                            self.browse(i, store::join(&path, &n.name));
                        }
                    }
                    (BACKSPACE, _) => {
                        if path == "/" {
                            self.go(Screen::Snapshots);
                            self.cursor = i;
                        } else {
                            self.browse(i, store::parent(&path));
                        }
                    }
                    (_, Some(b'r')) => {
                        if let Some(n) = node {
                            let target = format!("@{path}");
                            self.restore(id, &store::join(&path, &n.name), &target);
                        }
                    }
                    (_, Some(b't')) => {
                        if let Some(n) = node {
                            self.choose(Purpose::RestoreTo(id, store::join(&path, &n.name)));
                        }
                    }
                    _ => {}
                }
            }
            Screen::Report(..) => {
                if key.code == ESC {
                    self.go(Screen::Home);
                }
            }
        }
    }
}

fn purpose_name(p: &Purpose) -> &'static str {
    match p {
        Purpose::Source => "source",
        Purpose::Destination => "destination",
        Purpose::RestoreTo(..) => "restore",
    }
}

pub fn main() {
    let granted: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a.starts_with("@/"))
        .map(|a| if a == "@/" { a } else { a.trim_end_matches('/').to_string() })
        .collect();
    let settings = std::fs::read_to_string(settings::FILE).map(|t| Settings::parse(&t)).unwrap_or_default();
    let window = sex_std::window();
    window.title(b"Aftercare");
    let (target, task) = sex_std::identity();
    let rng = HwRng::new();
    rec(&format!(
        "[aftercare.ready] target={target} task={task} granted={} source={} destination={} rng={}",
        granted.join(","),
        settings.source.as_deref().unwrap_or("-"),
        settings.destination.as_deref().unwrap_or("-"),
        rng.is_some() as u8
    ));
    let mut app = App {
        window,
        painter: NoirPainter::new(),
        settings,
        granted,
        repo: None,
        rng,
        screen: Screen::Home,
        prompt: None,
        status: String::new(),
        cursor: 0,
        top: 0,
        snaps: Vec::new(),
        last_verified_ok: None,
        due_noticed: None,
        quit: false,
        asked: None,
        hits: Vec::new(),
        mouse: (-1, -1),
        mouse_down: false,
        hovered: None,
    };
    if app.granted.is_empty() {
        app.status = "Give Aftercare folders on launch (e.g. run aftercare @/home/ann @/media/USB) or Ask Files when choosing.".into();
    } else if let Some(d) = app.settings.destination.clone() {
        if DirStore::exists(&d) {
            app.status = format!("{d} holds a repository: U unlocks it.");
        }
    }
    app.draw();
    loop {
        let changed = match app.window.next_within(1000) {
            Some(Event::Close) => break,
            Some(Event::Key(k)) => {
                app.key(k);
                true
            }
            Some(Event::Pointer(ptr)) => app.pointer(ptr),
            Some(Event::Resize { .. }) => true,
            _ => app.tick(),
        };
        if app.quit {
            break;
        }
        if changed {
            app.draw();
        }
    }
    rec("[aftercare.closed]");
    std::process::exit(0);
}
