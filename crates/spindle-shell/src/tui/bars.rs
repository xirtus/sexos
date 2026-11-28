// SPDX-License-Identifier: GPL-3.0-only
//! Multi-track animated Ratatui progress bar & package operation runner (`bars`, `spin`,
//! `update`, `install`, `remove`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{self, AMBER, BLUE, BORDER, CORAL, CYAN, EMERALD, FROST, MUTE, ROSE};
use super::{TuiExit, TuiKey, TuiOutput};
use crate::width::truncate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarsMode {
    Demo,
    Update,
    Install,
    Remove,
}

#[derive(Clone, Debug)]
struct Track {
    label: String,
    total_kb: u64,
    pct: u16, // 0..=1000 (tenths of %)
    rate_kbs: u32,
    speed_step: u16,
}

/// Inline multi-track Ratatui progress bar controller (~30fps).
pub struct BarsApp {
    mode: BarsMode,
    title: String,
    tracks: Vec<Track>,
    frame: usize,
    last_ms: u64,
    drawn_lines: usize,
    paused: bool,
}

impl BarsApp {
    pub fn new(mode: BarsMode, pkgs: &[String], now_ms: u64) -> Self {
        let (title, tracks) = match mode {
            BarsMode::Demo => (
                String::from("Spindle Pipeline & Ratatui Stage Runner"),
                alloc::vec![
                    Track { label: String::from("Syncing SexOS crate graph"), total_kb: 18_420, pct: 180, rate_kbs: 4_820, speed_step: 46 },
                    Track { label: String::from("Compiling wasm32-wasip1 guests"), total_kb: 42_100, pct: 80, rate_kbs: 9_140, speed_step: 34 },
                    Track { label: String::from("Packing Geist Mono + Noto glyph atlas"), total_kb: 12_600, pct: 260, rate_kbs: 6_350, speed_step: 52 },
                    Track { label: String::from("Verifying Silk compositor frame ring"), total_kb: 9_800, pct: 120, rate_kbs: 5_210, speed_step: 40 },
                ],
            ),
            BarsMode::Update => (
                String::from("sexshop · Void-style SexOS System Sync (Su)"),
                alloc::vec![
                    Track { label: String::from("Fetching sexos-noir repo index"), total_kb: 4_250, pct: 220, rate_kbs: 7_400, speed_step: 60 },
                    Track { label: String::from("Checking /bin ELF + Wasm signatures"), total_kb: 24_900, pct: 100, rate_kbs: 11_200, speed_step: 42 },
                    Track { label: String::from("Validating PKU domain manifests"), total_kb: 6_140, pct: 150, rate_kbs: 8_300, speed_step: 50 },
                    Track { label: String::from("Warming Denim compressed page cache"), total_kb: 15_360, pct: 90, rate_kbs: 14_800, speed_step: 36 },
                ],
            ),
            BarsMode::Install => {
                let names = if pkgs.is_empty() { alloc::vec![String::from("neovim")] } else { pkgs.to_vec() };
                let tracks = names
                    .into_iter()
                    .take(5)
                    .enumerate()
                    .map(|(i, p)| Track {
                        label: format!("Installing {}", p),
                        total_kb: 8_400 + (i as u64 + 1) * 4_200,
                        pct: 90,
                        rate_kbs: 9_600 + (i as u32) * 1_400,
                        speed_step: 45usize.saturating_sub(i * 4).max(24) as u16,
                    })
                    .collect();
                (String::from("sexshop · Installing Packages (-S)"), tracks)
            }
            BarsMode::Remove => {
                let names = if pkgs.is_empty() { alloc::vec![String::from("orphan-cache")] } else { pkgs.to_vec() };
                let tracks = names
                    .into_iter()
                    .take(5)
                    .enumerate()
                    .map(|(i, p)| Track {
                        label: format!("Removing {}", p),
                        total_kb: 3_200 + (i as u64) * 1_800,
                        pct: 140,
                        rate_kbs: 18_500,
                        speed_step: 58usize.saturating_sub(i * 4).max(30) as u16,
                    })
                    .collect();
                (String::from("sexshop · Removing Packages (-R)"), tracks)
            }
        };
        Self { mode, title, tracks, frame: 0, last_ms: now_ms, drawn_lines: 0, paused: false }
    }

    pub fn name(&self) -> &'static str {
        match self.mode {
            BarsMode::Demo => "bars",
            BarsMode::Update => "update",
            BarsMode::Install => "install",
            BarsMode::Remove => "remove",
        }
    }

    pub fn init(&mut self, cols: usize, now_ms: u64) -> TuiOutput {
        self.last_ms = now_ms;
        let frame = self.render_box(cols, true);
        TuiOutput::frame(frame)
    }

    pub fn tick(&mut self, cols: usize, now_ms: u64) -> TuiOutput {
        if self.paused {
            return TuiOutput::none();
        }
        let elapsed = now_ms.saturating_sub(self.last_ms);
        if elapsed < 25 {
            return TuiOutput::none();
        }
        self.last_ms = now_ms;
        self.frame = self.frame.wrapping_add(1);
        let mult = ((elapsed / 30).max(1)).min(6) as u16;
        for (idx, tr) in self.tracks.iter_mut().enumerate() {
            if tr.pct < 1000 {
                let wave = ((self.frame + idx * 3) % 5) as u16;
                tr.pct = (tr.pct + (tr.speed_step + wave) * mult / 2).min(1000);
            }
        }
        let done = self.tracks.iter().all(|t| t.pct >= 1000);
        let frame = self.render_box(cols, false);
        if done {
            TuiOutput::finish(format!("{}\x1b[?25h", frame), TuiExit::Done)
        } else {
            TuiOutput::frame(frame)
        }
    }

    pub fn redraw(&mut self, cols: usize) -> TuiOutput {
        TuiOutput::frame(self.render_box(cols, false))
    }

    pub fn key(&mut self, key: TuiKey, cols: usize) -> TuiOutput {
        match key {
            TuiKey::Char('q') | TuiKey::Escape | TuiKey::CtrlC => {
                let box_str = self.render_box(cols, false);
                let note = format!("{}\x1b[38;2;245;158;11m[aborted]\x1b[0m\r\n\x1b[?25h", box_str);
                TuiOutput::finish(note, TuiExit::Cancel)
            }
            TuiKey::Char(' ') => {
                self.paused = !self.paused;
                TuiOutput::frame(self.render_box(cols, false))
            }
            _ => TuiOutput::none(),
        }
    }

    fn render_box(&mut self, cols: usize, initial: bool) -> String {
        let w = cols.clamp(40, 96);
        let mut out = String::new();
        if initial {
            out.push_str("\x1b[?25l");
        } else if self.drawn_lines > 0 {
            out.push_str(&format!("\x1b[{}A\r", self.drawn_lines));
        }

        let avg_pct = self.tracks.iter().map(|t| t.pct as usize).sum::<usize>() / self.tracks.len().max(1) / 10;
        let all_done = self.tracks.iter().all(|t| t.pct >= 1000);
        let badge = if all_done {
            String::from("✓ 100%")
        } else if self.paused {
            format!("⏸ {}% (Space resume)", avg_pct)
        } else {
            format!("{} {}%", widgets::spinner(self.frame), avg_pct)
        };
        let border = if all_done { EMERALD } else { BORDER };
        out.push_str(&widgets::box_top(&self.title, &badge, w, border, CYAN, if all_done { EMERALD } else { CORAL }));
        out.push_str("\x1b[K\r\n");

        let inner = w - 4;
        let label_w = (inner * 34 / 100).clamp(14, 34);
        let stats_w = 24usize;
        let bar_w = inner.saturating_sub(label_w + stats_w + 5).max(8);

        for tr in &self.tracks {
            let pct = (tr.pct / 10) as usize;
            let done = tr.pct >= 1000;
            let sp = if done { "✓" } else { widgets::spinner(self.frame) };
            let sp_col = if done { EMERALD } else { ROSE };
            let lbl = truncate(&tr.label, label_w);
            let cur_mb = (tr.total_kb * tr.pct as u64 / 1000) / 100;
            let tot_mb = tr.total_kb / 100;
            let rem_sec = if done {
                0
            } else {
                let rem_kb = tr.total_kb.saturating_sub(tr.total_kb * tr.pct as u64 / 1000);
                (rem_kb / tr.rate_kbs.max(1) as u64).max(1)
            };
            let stats = if done {
                format!("{}{:3}%{} {}done{}", widgets::fg(EMERALD), pct, "\x1b[0m", widgets::fg(MUTE), "\x1b[0m")
            } else {
                format!(
                    "{}{:3}%{} {}{:>2}.{}M/{:>2}.{}M {}{:>2}s{}",
                    widgets::fg(FROST), pct, "\x1b[0m",
                    widgets::fg(MUTE), cur_mb / 10, cur_mb % 10, tot_mb / 10, tot_mb % 10,
                    widgets::fg(AMBER), rem_sec, "\x1b[0m"
                )
            };
            let (c0, c1) = if done { (EMERALD, CYAN) } else { (ROSE, BLUE) };
            let bar = widgets::smooth_bar(bar_w, pct, c0, c1);
            let row_body = format!(
                "{}{} {}{:<lw$}\x1b[0m {} {}",
                widgets::fg(sp_col), sp,
                widgets::fg(FROST), lbl,
                bar, stats,
                lw = label_w
            );
            out.push_str(&widgets::box_row(&row_body, w, border));
            out.push_str("\x1b[K\r\n");
        }

        let footer = if all_done {
            match self.mode {
                BarsMode::Update => "✓ SexOS packages & services up to date",
                BarsMode::Install => "✓ package installation verified",
                BarsMode::Remove => "✓ packages cleanly removed",
                BarsMode::Demo => "✓ all tasks complete",
            }
        } else {
            "Space pause · q / Esc cancel"
        };
        out.push_str(&widgets::box_bot(footer, w, border, if all_done { EMERALD } else { MUTE }));
        out.push_str("\x1b[K\r\n");

        self.drawn_lines = 2 + self.tracks.len();
        out
    }
}
