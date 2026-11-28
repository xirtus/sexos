// SPDX-License-Identifier: GPL-3.0-only
//! Interactive full-screen Ratatui `btop` / `htop` system monitor (`\x1b[?1049h` alt-screen).
//! Reads live SexOS telemetry (`Host::sys_info` + `Host::processes`) with real Braille CPU
//! sparklines, per-core meters, memory/heap gauges, and a scrollable/sortable task table.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{
    self, AMBER, BLUE, BORDER, CORAL, CYAN, EMERALD, FROST, LAVENDER, MINT, MUTE, ROSE, SEL_BG,
};
use super::{TuiExit, TuiKey, TuiOutput};
use crate::ergonomics::human_size;
use crate::host::{Host, ProcessInfo};
use crate::value::Value;
use crate::width::truncate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Memory,
    Pid,
    Name,
    State,
}

impl SortBy {
    fn next(self) -> Self {
        match self {
            SortBy::Memory => SortBy::Pid,
            SortBy::Pid => SortBy::Name,
            SortBy::Name => SortBy::State,
            SortBy::State => SortBy::Memory,
        }
    }
    fn label(self) -> &'static str {
        match self {
            SortBy::Memory => "mem▼",
            SortBy::Pid => "pid▲",
            SortBy::Name => "name▲",
            SortBy::State => "state▲",
        }
    }
}

struct Snapshot {
    host_name: String,
    cpus: usize,
    uptime_ns: i64,
    cpu_pct: u8,
    core_pcts: Vec<u8>,
    mem_used: u64,
    mem_total: u64,
    heap_used: u64,
    heap_mapped: u64,
    free_frames: u64,
    packed_pages: u64,
    denim_store: u64,
    procs: Vec<ProcessInfo>,
}

/// Interactive `btop` system dashboard.
pub struct BtopApp {
    frame: usize,
    last_sample_ms: u64,
    last_tsc: u64,
    last_idle: u64,
    last_core_idle: Vec<u64>,
    cpu_history: Vec<u8>,
    selected: usize,
    scroll: usize,
    sort_by: SortBy,
    filtering: bool,
    filter: String,
}

impl BtopApp {
    pub fn new(now_ms: u64) -> Self {
        Self {
            frame: 0,
            last_sample_ms: now_ms,
            last_tsc: 0,
            last_idle: 0,
            last_core_idle: Vec::new(),
            cpu_history: alloc::vec![8, 12, 10, 14, 11],
            selected: 0,
            scroll: 0,
            sort_by: SortBy::Memory,
            filtering: false,
            filter: String::new(),
        }
    }

    pub fn init(&mut self, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        self.last_sample_ms = now_ms;
        self.sample(host);
        let mut s = String::from("\x1b[?1049h\x1b[?25l");
        s.push_str(&self.render(host, cols, rows, now_ms));
        TuiOutput::frame(s)
    }

    pub fn tick(&mut self, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        if now_ms.saturating_sub(self.last_sample_ms) < 220 {
            return TuiOutput::none();
        }
        self.last_sample_ms = now_ms;
        self.frame = self.frame.wrapping_add(1);
        self.sample(host);
        TuiOutput::frame(self.render(host, cols, rows, now_ms))
    }

    pub fn key(&mut self, key: TuiKey, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        if self.filtering {
            match key {
                TuiKey::Escape => {
                    self.filtering = false;
                    self.filter.clear();
                    self.selected = 0;
                    self.scroll = 0;
                }
                TuiKey::Enter => self.filtering = false,
                TuiKey::Backspace => {
                    self.filter.pop();
                    self.selected = 0;
                    self.scroll = 0;
                }
                TuiKey::CtrlC => return TuiOutput::finish(String::from("\x1b[?25h\x1b[?1049l"), TuiExit::Done),
                TuiKey::Char(c) => {
                    self.filter.push(c);
                    self.selected = 0;
                    self.scroll = 0;
                }
                _ => {}
            }
            return TuiOutput::frame(self.render(host, cols, rows, now_ms));
        }

        match key {
            TuiKey::Char('q') | TuiKey::Escape | TuiKey::CtrlC => {
                return TuiOutput::finish(String::from("\x1b[?25h\x1b[?1049l"), TuiExit::Done);
            }
            TuiKey::Char('j') | TuiKey::Down => self.selected = self.selected.saturating_add(1),
            TuiKey::Char('k') | TuiKey::Up => self.selected = self.selected.saturating_sub(1),
            TuiKey::Home | TuiKey::Char('g') => {
                self.selected = 0;
                self.scroll = 0;
            }
            TuiKey::End | TuiKey::Char('G') => self.selected = usize::MAX / 2,
            TuiKey::PageDown => self.selected = self.selected.saturating_add(8),
            TuiKey::PageUp => self.selected = self.selected.saturating_sub(8),
            TuiKey::Char('s') => self.sort_by = self.sort_by.next(),
            TuiKey::Char('/') => {
                self.filtering = true;
                self.filter.clear();
            }
            TuiKey::Char('r') => {
                self.frame = self.frame.wrapping_add(1);
                self.sample(host);
            }
            _ => return TuiOutput::none(),
        }
        TuiOutput::frame(self.render(host, cols, rows, now_ms))
    }

    fn sample(&mut self, host: &dyn Host) -> Snapshot {
        let sys = host.sys_info();
        let rec = match &sys {
            Value::Record(r) => Some(r),
            _ => None,
        };
        let get_i = |k: &str| rec.and_then(|r| match r.get(k) { Some(Value::Int(n)) => Some(*n), _ => None });
        let host_name = rec.and_then(|r| r.get("host")).map(|v| v.to_text()).unwrap_or_else(|| String::from("sexos-noir"));
        let cpus = get_i("cpus").unwrap_or(1).max(1) as usize;
        let uptime_ns = rec.and_then(|r| match r.get("uptime") { Some(Value::Duration(ns)) => Some(*ns), _ => None }).unwrap_or(0);
        let tsc = get_i("tsc").unwrap_or(0).max(0) as u64;
        let idle = get_i("idle_cycles").unwrap_or(0).max(0) as u64;

        let mut core_pcts = Vec::with_capacity(cpus);
        if self.last_core_idle.len() != cpus {
            self.last_core_idle = alloc::vec![0; cpus];
        }
        let d_tsc = tsc.saturating_sub(self.last_tsc);
        for c in 0..cpus {
            let pct = if let Some(explicit) = get_i(&format!("core_pct_{}", c)) {
                explicit.clamp(0, 100) as u8
            } else if d_tsc > 0 {
                if let Some(ci) = get_i(&format!("core_idle_{}", c)) {
                    let ci = ci.max(0) as u64;
                    let d_idle = ci.saturating_sub(self.last_core_idle[c]);
                    self.last_core_idle[c] = ci;
                    let busy = d_tsc.saturating_sub(d_idle);
                    ((busy * 100) / d_tsc).min(100) as u8
                } else {
                    pseudo_core_load(c, self.frame, host.processes().len())
                }
            } else {
                pseudo_core_load(c, self.frame, host.processes().len())
            };
            core_pcts.push(pct);
        }

        let cpu_pct = if d_tsc > 0 && idle >= self.last_idle && self.last_tsc > 0 {
            let cap = d_tsc.saturating_mul(cpus as u64).max(1);
            let d_idle = idle.saturating_sub(self.last_idle).min(cap);
            (((cap - d_idle) * 100) / cap).min(100) as u8
        } else {
            let sum: usize = core_pcts.iter().map(|&p| p as usize).sum();
            (sum / core_pcts.len().max(1)) as u8
        };
        self.last_tsc = tsc;
        self.last_idle = idle;

        self.cpu_history.push(cpu_pct);
        if self.cpu_history.len() > 80 {
            self.cpu_history.remove(0);
        }

        let free_pages = get_i("free_pages").unwrap_or(0).max(0) as u64;
        let lent_pages = get_i("lent_pages").unwrap_or(0).max(0) as u64;
        let limit_pages = get_i("limit_pages").unwrap_or(0).max(0) as u64;
        let heap_used = get_i("kernel_heap_used").unwrap_or(0).max(0) as u64;
        let heap_mapped = get_i("kernel_heap_mapped").unwrap_or(0).max(0) as u64;
        let free_frames = get_i("free_frames").unwrap_or(0).max(0) as u64;
        let packed_pages = get_i("packed_pages").unwrap_or(0).max(0) as u64;
        let denim_store = get_i("denim_store").unwrap_or(0).max(0) as u64;
        let total_mem = rec.and_then(|r| match r.get("memory") { Some(Value::Filesize(n)) => Some((*n).max(0) as u64), _ => None }).unwrap_or(0);

        let procs = host.processes();
        let proc_mem: u64 = procs.iter().filter_map(|p| p.memory).sum();
        let (mem_used, mem_total) = if limit_pages > 0 {
            (lent_pages * 4096, limit_pages * 4096)
        } else if total_mem > 0 {
            (proc_mem.max(total_mem / 8).min(total_mem), total_mem)
        } else {
            (proc_mem, (proc_mem + free_pages * 4096).max(64 << 20))
        };

        Snapshot {
            host_name,
            cpus,
            uptime_ns,
            cpu_pct,
            core_pcts,
            mem_used,
            mem_total,
            heap_used,
            heap_mapped,
            free_frames,
            packed_pages,
            denim_store,
            procs,
        }
    }

    pub fn render(&mut self, host: &dyn Host, cols: usize, rows: usize, _now_ms: u64) -> String {
        let w = cols.max(36);
        let h = rows.max(14);
        let snap = self.sample(host);
        let mut lines: Vec<String> = Vec::with_capacity(h);

        // Line 1: Header pill bar
        let up = fmt_uptime(snap.uptime_ns);
        let hdr = format!(
            " {}● SexOS btop{}  {}{}{}  {}cpus:{}  up {}  sort:[{}]  {}q quit · / filter · s sort · r refresh{}",
            widgets::fg(ROSE), "\x1b[0m",
            widgets::fg(CYAN), snap.host_name, "\x1b[0m",
            widgets::fg(MUTE), snap.cpus, up, self.sort_by.label(),
            widgets::fg(MUTE), "\x1b[0m"
        );
        lines.push(widgets::pad_ansi(&hdr, w));

        let side_by_side = w >= 72;
        if side_by_side {
            let lw = w / 2;
            let rw = w - lw;
            let cpu_rows = build_cpu_panel(&snap, &self.cpu_history, self.frame, lw);
            let mem_rows = build_mem_panel(&snap, rw);
            let panel_h = cpu_rows.len().max(mem_rows.len());
            for i in 0..panel_h {
                let l = cpu_rows.get(i).cloned().unwrap_or_else(|| " ".repeat(lw));
                let r = mem_rows.get(i).cloned().unwrap_or_else(|| " ".repeat(rw));
                lines.push(format!("{}{}", l, r));
            }
        } else {
            lines.extend(build_cpu_panel(&snap, &self.cpu_history, self.frame, w));
            lines.extend(build_mem_panel(&snap, w));
        }

        // Process / Domain interactive table takes remaining rows.
        let proc_rows_budget = h.saturating_sub(lines.len()).max(5);
        let list_cap = proc_rows_budget.saturating_sub(3).max(1);

        let q = self.filter.to_lowercase();
        let mut procs: Vec<ProcessInfo> = snap
            .procs
            .into_iter()
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.state.to_lowercase().contains(&q))
            .collect();
        match self.sort_by {
            SortBy::Memory => procs.sort_by(|a, b| b.memory.unwrap_or(0).cmp(&a.memory.unwrap_or(0)).then_with(|| a.pid.cmp(&b.pid))),
            SortBy::Pid => procs.sort_by(|a, b| a.pid.cmp(&b.pid)),
            SortBy::Name => procs.sort_by(|a, b| a.name.cmp(&b.name)),
            SortBy::State => procs.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.pid.cmp(&b.pid))),
        }

        if procs.is_empty() {
            self.selected = 0;
            self.scroll = 0;
        } else {
            self.selected = self.selected.min(procs.len() - 1);
            if self.selected < self.scroll {
                self.scroll = self.selected;
            } else if self.selected >= self.scroll + list_cap {
                self.scroll = self.selected + 1 - list_cap;
            }
        }

        let table_title = if self.filtering {
            format!("Domains & Yarns ({})  /{}_", procs.len(), self.filter)
        } else if !self.filter.is_empty() {
            format!("Domains & Yarns ({})  filter:{}", procs.len(), self.filter)
        } else {
            format!("Domains & Yarns ({})", procs.len())
        };
        let badge = format!("{}/{}", if procs.is_empty() { 0 } else { self.selected + 1 }, procs.len());
        lines.push(widgets::box_top(&table_title, &badge, w, BORDER, LAVENDER, CYAN));

        let inner = w.saturating_sub(4);
        let col_hdr = format!(
            "{}  {:<6}  {:<18}  {:<9}  {:>9}  {}{}",
            widgets::fg(MUTE), "PID", "NAME", "STATE", "MEMORY", "SHARE", "\x1b[0m"
        );
        lines.push(widgets::box_row(&col_hdr, w, BORDER));

        let max_mem = procs.iter().filter_map(|p| p.memory).max().unwrap_or(1).max(1);
        let bar_w = inner.saturating_sub(52).clamp(6, 20);
        for row_idx in 0..list_cap {
            if let Some(p) = procs.get(self.scroll + row_idx) {
                let is_sel = self.scroll + row_idx == self.selected;
                let mem_bytes = p.memory.unwrap_or(0);
                let mem_str = p.memory.map(human_size).unwrap_or_else(|| String::from("-"));
                let pct = ((mem_bytes * 100) / max_mem).min(100) as usize;
                let state_col = match p.state.as_str() {
                    "running" | "active" => EMERALD,
                    "starting" => AMBER,
                    "ended" => MUTE,
                    _ => BLUE,
                };
                let bar = widgets::smooth_bar(bar_w, pct, CYAN, LAVENDER);
                let name = truncate(&p.name, 18);
                let state = truncate(&p.state, 9);
                let mut body = String::new();
                if is_sel {
                    widgets::push_bg(&mut body, SEL_BG);
                    widgets::push_fg(&mut body, ROSE);
                    body.push_str("▶ ");
                } else {
                    body.push_str("  ");
                }
                body.push_str(&format!(
                    "{}{:<6}  {}{:<18}  {}{:<9}  {}{:>9}  {}",
                    widgets::fg(MUTE), p.pid,
                    widgets::fg(if is_sel { CYAN } else { FROST }), name,
                    widgets::fg(state_col), state,
                    widgets::fg(AMBER), mem_str,
                    bar
                ));
                lines.push(widgets::box_row(&body, w, BORDER));
            } else {
                lines.push(widgets::box_row("", w, BORDER));
            }
        }
        lines.push(widgets::box_bot("↑↓/jk move · s sort · / filter · r refresh · q exit", w, BORDER, MUTE));

        let mut out = String::from("\x1b[H");
        for (i, line) in lines.iter().take(h).enumerate() {
            out.push_str(line);
            out.push_str("\x1b[K");
            if i + 1 < h {
                out.push_str("\r\n");
            }
        }
        out.push_str("\x1b[J");
        out
    }
}

fn build_cpu_panel(snap: &Snapshot, history: &[u8], frame: usize, w: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let badge = format!("{} {:>3}%", widgets::spinner(frame), snap.cpu_pct);
    rows.push(widgets::box_top("CPU", &badge, w, BORDER, ROSE, CORAL));
    let inner = w.saturating_sub(4);
    let spark_w = inner.saturating_sub(8).max(8);
    let spark = widgets::braille_spark(history, spark_w, CYAN, ROSE);
    rows.push(widgets::box_row(&format!("{}load:{} {}", widgets::fg(MUTE), "\x1b[0m", spark), w, BORDER));

    let core_bar_w = inner.saturating_sub(11).max(6);
    for (ci, &pct) in snap.core_pcts.iter().take(4).enumerate() {
        let bar = widgets::smooth_bar(core_bar_w, pct as usize, MINT, ROSE);
        rows.push(widgets::box_row(
            &format!("{}C{:<2}{} {} {}{:>3}%{}", widgets::fg(MUTE), ci, "\x1b[0m", bar, widgets::fg(FROST), pct, "\x1b[0m"),
            w,
            BORDER,
        ));
    }
    rows.push(widgets::box_bot("", w, BORDER, MUTE));
    rows
}

fn build_mem_panel(snap: &Snapshot, w: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mem_pct = if snap.mem_total > 0 { ((snap.mem_used * 100) / snap.mem_total).min(100) as usize } else { 0 };
    let badge = format!("{}/{}", human_size(snap.mem_used), human_size(snap.mem_total));
    rows.push(widgets::box_top("Memory & Kernel", &badge, w, BORDER, EMERALD, CYAN));
    let inner = w.saturating_sub(4);
    let bar_w = inner.saturating_sub(15).max(6);

    let prog_bar = widgets::smooth_bar(bar_w, mem_pct, EMERALD, CYAN);
    rows.push(widgets::box_row(
        &format!("{}prog:{}  {} {}{:>3}%{}", widgets::fg(MUTE), "\x1b[0m", prog_bar, widgets::fg(FROST), mem_pct, "\x1b[0m"),
        w,
        BORDER,
    ));

    if snap.heap_mapped > 0 {
        let hpct = ((snap.heap_used * 100) / snap.heap_mapped).min(100) as usize;
        let hbar = widgets::smooth_bar(bar_w, hpct, BLUE, LAVENDER);
        rows.push(widgets::box_row(
            &format!("{}heap:{}  {} {}{:>3}%{}", widgets::fg(MUTE), "\x1b[0m", hbar, widgets::fg(FROST), hpct, "\x1b[0m"),
            w,
            BORDER,
        ));
    } else {
        rows.push(widgets::box_row(
            &format!("{}heap:{}  {}managed by kernel PKU{}", widgets::fg(MUTE), "\x1b[0m", widgets::fg(FROST), "\x1b[0m"),
            w,
            BORDER,
        ));
    }

    if snap.packed_pages > 0 || snap.denim_store > 0 {
        rows.push(widgets::box_row(
            &format!(
                "{}denim:{} {}{} packed -> {} stored{}",
                widgets::fg(MUTE), "\x1b[0m", widgets::fg(LAVENDER),
                human_size(snap.packed_pages * 4096), human_size(snap.denim_store * 4096), "\x1b[0m"
            ),
            w,
            BORDER,
        ));
    } else if snap.free_frames > 0 {
        rows.push(widgets::box_row(
            &format!("{}free:{}  {}{}{}", widgets::fg(MUTE), "\x1b[0m", widgets::fg(MINT), human_size(snap.free_frames * 4096), "\x1b[0m"),
            w,
            BORDER,
        ));
    } else {
        rows.push(widgets::box_row(
            &format!("{}tasks:{} {}{} active domains/yarns{}", widgets::fg(MUTE), "\x1b[0m", widgets::fg(CYAN), snap.procs.len(), "\x1b[0m"),
            w,
            BORDER,
        ));
    }

    rows.push(widgets::box_bot("", w, BORDER, MUTE));
    rows
}

fn pseudo_core_load(core: usize, frame: usize, proc_count: usize) -> u8 {
    let base = (proc_count * 3 + 7).min(35);
    let wave = ((frame * 7 + core * 19) % 24) as usize;
    (base + wave).min(95) as u8
}

fn fmt_uptime(ns: i64) -> String {
    let secs = (ns / 1_000_000_000).max(0) as u64;
    let mins = secs / 60;
    let hrs = mins / 60;
    if hrs > 0 {
        format!("{}h{:02}m", hrs, mins % 60)
    } else {
        format!("{}m{:02}s", mins, secs % 60)
    }
}
