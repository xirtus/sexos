// SPDX-License-Identifier: GPL-3.0-only
//! Mesh (formerly Monitor; `mesh` in Spindle): what the machine is doing, and who holds what.
//!
//! Four tabs, chosen with the keys 1 to 4, Left and Right, or a click on the tab:
//! - Programs: every service and program that has used the CPU, busiest first, with its
//!   share of one core over the last second, the memory it holds, its system calls and
//!   file reads per second and how many grants it holds. Up and Down choose a line, Home
//!   the top one; Delete or the End button asks Shell to end that program.
//! - Performance: two minutes of history as graphs: the whole CPU, each core, program
//!   memory, system calls and file reads.
//! - Grants: the capability graph as Mesh can read it: each domain's organ (its PKU key),
//!   how many capabilities it holds and which domains it may call, and the file grants
//!   of every program run (paths shown when Mesh itself holds the whole filesystem,
//!   `run mesh @/`).
//! - Hardware: cores and memory.
//!
//! Every number comes from the kernel's read-only statistics (syscall 87) and Files'
//! read-only view of program runs; ending a program is Shell's (`system_stats::end_program`).
#![no_std]
#![no_main]

use sex_app::{files, nav, typed, Event, Nav, Window};
use sex_mesh::canvas::{mix, Canvas, Glyphs, CELL_H, CELL_W};
use sex_mesh::chart::{self, Area, Style};
use sex_mesh::history::{self, History};
use sex_mesh::layout::{self, Action, Tab, PAD, ROW_H, TOP};
use sex_mesh::line::Line;
use sex_pdx::app_pointer::Kind;
use sex_pdx::fs::runs;
use sex_pdx::system_stats as stats;
use sex_pdx::mesh_stats as mesh;

const MAX: usize = 96;
const PERIOD_MS: u64 = 1000;
/// Two minutes of samples.
const SPAN: usize = 120;
/// Cores with their own graph.
const CORES: usize = 16;
const RUNS_BYTES: usize = 8192;
const GRANT_LINES: usize = 96;

const BG: u32 = 0x0014161C;
const PANEL: u32 = 0x001D2029;
const TEXT: u32 = sex_pdx::desktop_style::TEXT;
const DIM: u32 = 0x008A93A6;
const ACCENT: u32 = sex_pdx::desktop_style::FOCUS;
const CPU: u32 = 0x005AC8FA;
const MEMORY: u32 = 0x00A78BFA;
const CALLS: u32 = 0x0034D399;
const DISK: u32 = 0x00F59E0B;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[monitor.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, Default)]
struct Row { pd: u32, cycles: u64, tenths: u64, pages: u64, task: u32, syscalls: u64, rate: u64, read_bytes: u64, read_rate: u64, grants: u8 }

#[derive(Clone, Copy)]
struct Sample { rows: [Row; MAX], count: usize, tsc: u64, idle: u64, cores: u64, free_pages: u64, ms: u64, core_idle: [u64; CORES], per_core: bool, memory: Option<mesh::Memory> }

impl Sample {
    const fn empty() -> Self {
        Sample { rows: [Row { pd: 0, cycles: 0, tenths: 0, pages: 0, task: 0, syscalls: 0, rate: 0, read_bytes: 0, read_rate: 0, grants: 0 }; MAX],
            count: 0, tsc: 0, idle: 0, cores: 1, free_pages: 0, ms: 0, core_idle: [0; CORES], per_core: false, memory: None }
    }
    fn row(&self, pd: u32) -> Option<&Row> { self.rows[..self.count].iter().find(|r| r.pd == pd) }
}

struct Mesh {
    tab: Tab,
    /// First list line shown, and the chosen program (domain, task).
    top: usize,
    chosen: Option<(u32, u32)>,
    prev: Sample,
    now: Sample,
    cpu: History<SPAN>,
    cores: [History<SPAN>; CORES],
    memory: History<SPAN>,
    calls: History<SPAN>,
    disk: History<SPAN>,
    /// Files' last answer about program runs; `false` once Files refused us.
    runs: [u8; RUNS_BYTES],
    runs_len: usize,
    runs_ok: bool,
    runs_seen: u64,
    status: Line<96>,
    grants: [Line<160>; GRANT_LINES],
    grant_count: usize,
}

static mut MESH: Mesh = Mesh {
    tab: Tab::Processes, top: 0, chosen: None, prev: Sample::empty(), now: Sample::empty(),
    cpu: History::new(), cores: [const { History::new() }; CORES], memory: History::new(), calls: History::new(), disk: History::new(),
    runs: [0; RUNS_BYTES], runs_len: 0, runs_ok: true, runs_seen: 0, status: Line::new(),
    grants: [const { Line::new() }; GRANT_LINES], grant_count: 0,
};
static mut GLYPHS: Glyphs = Glyphs::new();

/// The name a domain gave itself, or the service's name, or its number.
fn name_into<const N: usize>(l: &mut Line<N>, pd: u32, width: usize) {
    let start = l.len();
    let n = stats::name(pd);
    let len = n.iter().position(|&c| c == 0).unwrap_or(16);
    if len > 0 { l.put(&n[..len]); }
    else if (1..=15).contains(&pd) { l.put(sex_pdx::crash_log::domain_name(pd).as_bytes()); }
    else { l.put(b"domain ").num(pd as u64, 1); }
    l.pad(start + width);
}

fn is_program(pd: u32) -> bool { sex_pdx::app_catalog::by_target(pd).is_some_and(|r| r.app == sex_pdx::app_catalog::App::Program) }

impl Mesh {
    fn run_of(&self, pd: u32, task: u32) -> Option<runs::Run<'_>> {
        runs::parse(&self.runs[..self.runs_len]).find(|r| r.target == pd && r.task == task)
    }

    /// Files' view of program runs: reads and grants. Logged when it changes.
    fn ask_files(&mut self, window: &mut Window) {
        if !self.runs_ok { return; }
        match files::runs(window, &mut self.runs) {
            Ok(n) => self.runs_len = n,
            Err(e) => {
                self.runs_len = 0;
                if e == sex_pdx::fs::DENIED || e == sex_pdx::fs::INVALID { self.runs_ok = false; }
                sex_pdx::serial_record!("[monitor.files] status={}", e);
            }
        }
    }

    fn sample(&mut self, window: &mut Window) {
        self.ask_files(window);
        core::mem::swap(&mut self.prev, &mut self.now);
        let prev = &self.prev;
        let ms = sex_pdx::get_ticks();
        let mut s = Sample::empty();
        s.ms = ms;
        if let Some(o) = stats::overview() { s.tsc = o.tsc; s.idle = o.idle_cycles; s.cores = o.cores.max(1); s.free_pages = o.free_pages; }
        // Per-core idle time, if this kernel answers it (older ones say INVALID).
        s.per_core = mesh::core(0).is_some();
        for (i, idle) in s.core_idle.iter_mut().enumerate().take(if s.per_core { s.cores as usize } else { 0 }) {
            *idle = mesh::core(i as u32).map_or(0, |c| c.idle_cycles);
        }
        s.memory = mesh::memory();
        let span_ms = ms.saturating_sub(prev.ms).max(1);
        let had = prev.tsc != 0;
        let mut after = 0;
        while let Some(pd) = stats::next(after) {
            after = pd;
            if s.count == MAX { break; }
            let Some(d) = stats::domain(pd) else { continue };
            let before = prev.row(pd).filter(|b| b.task == d.task);
            let tenths = before.map_or(0, |b| stats::tenths_of_core(b.cycles, d.cycles, prev.tsc, s.tsc));
            let rate = before.map_or(0, |b| history::per_second(d.syscalls.saturating_sub(b.syscalls), span_ms));
            let run = self.run_of(pd, d.task);
            let read_bytes = run.map_or(0, |r| r.read_bytes);
            let read_rate = before.map_or(0, |b| history::per_second(read_bytes.saturating_sub(b.read_bytes), span_ms));
            s.rows[s.count] = Row { pd, cycles: d.cycles, tenths, pages: d.pages, task: d.task, syscalls: d.syscalls, rate, read_bytes, read_rate, grants: run.map_or(0, |r| r.grants) };
            s.count += 1;
        }
        // Busiest first; ties by domain so the list does not jump about.
        s.rows[..s.count].sort_unstable_by(|a, b| b.tenths.cmp(&a.tenths).then(a.pd.cmp(&b.pd)));
        self.now = s;
        if had { self.record_history(); }
    }

    fn record_history(&mut self) {
        let (s, p) = (&self.now, &self.prev);
        let span = s.tsc.saturating_sub(p.tsc);
        let busy = history::busy_permille(s.idle.saturating_sub(p.idle), span, s.cores);
        self.cpu.push(busy);
        let cores = (s.cores as usize).min(CORES);
        for i in 0..if s.per_core { cores } else { 0 } { self.cores[i].push(history::busy_permille(s.core_idle[i].saturating_sub(p.core_idle[i]), span, 1)); }
        let memory = s.memory.map_or(0, |m| history::permille(m.lent_pages, m.limit_pages));
        self.memory.push(memory);
        let calls: u64 = s.rows[..s.count].iter().map(|r| r.rate).sum();
        self.calls.push(calls.min(u32::MAX as u64) as u32);
        let disk: u64 = s.rows[..s.count].iter().map(|r| r.read_rate).sum();
        self.disk.push(disk.min(u32::MAX as u64) as u32);
        let first = s.rows[..s.count].first().copied().unwrap_or_default();
        sex_pdx::serial_record!("[monitor.sample] rows={} busy_tenths={} top_pd={} top_tenths={} top_task={} free_pages={}", s.count, busy, first.pd, first.tenths, first.task, s.free_pages);
        for r in s.rows[..s.count].iter().take(4) {
            sex_pdx::serial_record!("[monitor.row] pd={} tenths={} pages={} calls_per_s={}", r.pd, r.tenths, r.pages, r.rate);
        }
        sex_pdx::serial_record!("[monitor.graph] cores={} points={} cpu_permille={} core0_permille={} memory_permille={} calls_per_s={} disk_bytes_per_s={}",
            s.cores, self.cpu.len(), busy, self.cores[0].latest().unwrap_or(0), memory, calls, disk);
        self.log_runs();
    }

    /// Each program run's grants, once whenever Files' answer about them changes.
    fn log_runs(&mut self) {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for r in runs::parse(&self.runs[..self.runs_len]) {
            for b in r.target.to_le_bytes().iter().chain(&r.task.to_le_bytes()).chain(&[r.grants]).chain(r.text) { hash = (hash ^ *b as u64).wrapping_mul(0x100_0000_01b3); }
        }
        if hash == self.runs_seen { return; }
        self.runs_seen = hash;
        for r in runs::parse(&self.runs[..self.runs_len]) {
            let live = self.now.row(r.target).is_some_and(|row| row.task == r.task);
            sex_pdx::serial_record!("[monitor.grant] target={} task={} live={} grants={} read_bytes={} text={}", r.target, r.task, live as u8, r.grants, r.read_bytes, core::str::from_utf8(r.text).unwrap_or("?"));
        }
    }

    /// The chosen program's place in the list now, if it is still there.
    fn chosen_index(&self) -> Option<usize> {
        let (pd, task) = self.chosen?;
        self.now.rows[..self.now.count].iter().position(|r| r.pd == pd && r.task == task)
    }
    fn choose(&mut self, index: usize) {
        let s = &self.now;
        if s.count == 0 { self.chosen = None; return; }
        let r = s.rows[index.min(s.count - 1)];
        self.chosen = Some((r.pd, r.task));
        sex_pdx::serial_record!("[monitor.select] pd={} task={} program={}", r.pd, r.task, is_program(r.pd) as u8);
        sex_pdx::serial_record!("[monitor.choose] target={}", r.pd);
    }
    fn show(&mut self, tab: Tab, via: &str) {
        self.tab = tab;
        self.top = 0;
        sex_pdx::serial_record!("[monitor.view] tab={} via={}", tab.name(), via);
        if tab == Tab::Grants { self.build_grants(true); }
    }

    /// Asks Shell to end the chosen program (`system_stats::end_program`): Shell takes it
    /// only from the focused Mesh, and only for a program, never a service.
    fn end(&mut self, _me: u32) {
        self.status.clear();
        let Some((pd, _task)) = self.chosen else { self.status.put(b"Choose a program first."); return };
        let status = stats::end_program(pd);
        sex_pdx::serial_record!("[monitor.end] target={} status={}", pd, status);
        self.status.put(match status {
            stats::OK => b"Ended ",
            stats::NOT_A_PROGRAM => b"Only programs can be ended, not system services: ",
            stats::NOT_FOCUSED => b"The shell did not take the request (focus Mesh and try again): ",
            _ => b"Could not end ",
        });
        name_into(&mut self.status, pd, 0);
        self.status.put(b".");
    }

    fn key(&mut self, window: &Window, key: sex_pdx::app_key::Key) -> bool {
        match (nav(key), typed(key)) {
            (_, Some(c)) if Tab::from_key(c).is_some() => self.show(Tab::from_key(c).unwrap(), "key"),
            (Some(Nav::Left), _) => self.show(self.tab.prev(), "key"),
            (Some(Nav::Right), _) => self.show(self.tab.next(), "key"),
            (Some(Nav::Up), _) if self.tab == Tab::Processes => { let i = self.chosen_index().unwrap_or(0); self.choose(i.saturating_sub(1)); }
            (Some(Nav::Down), _) if self.tab == Tab::Processes => { let i = self.chosen_index().map_or(0, |i| i + 1); self.choose(i); }
            (Some(Nav::Home), _) if self.tab == Tab::Processes => { self.top = 0; self.choose(0); }
            (Some(Nav::Delete), _) if self.tab == Tab::Processes => self.end(window.target),
            (Some(Nav::Up), _) => self.top = self.top.saturating_sub(1),
            (Some(Nav::Down), _) => self.top += 1,
            (Some(Nav::Home), _) => self.top = 0,
            _ => return false,
        }
        true
    }

    fn pointer(&mut self, window: &Window, p: sex_pdx::app_pointer::Pointer) -> bool {
        let (w, h) = window.size();
        match p.kind {
            Kind::Down { button: 0 } => match layout::click(self.tab, p.x as i32, p.y as i32, w as i32, h as i32) {
                Some(Action::Show(t)) => self.show(t, "mouse"),
                Some(Action::Select(row)) => self.choose(self.top + row),
                Some(Action::End) => self.end(window.target),
                Some(Action::Scroll(_)) | None => return false,
            },
            Kind::Wheel { steps } => self.top = if steps > 0 { self.top.saturating_sub(steps as usize) } else { self.top + (-(steps as i32)) as usize },
            _ => return false,
        }
        true
    }

    /// The Grants tab's lines: each domain's organ and whom it may call, then each
    /// program run's file grants. `log` records them for the gate.
    fn build_grants(&mut self, log: bool) {
        let mut n = 0;
        let mut after = 0;
        while let Some(pd) = stats::next(after) {
            after = pd;
            if n == GRANT_LINES { break; }
            let l = &mut self.grants[n];
            l.clear();
            name_into(l, pd, 16);
            l.put(b" pd ").num(pd as u64, 3);
            let organ = mesh::organ(pd);
            match organ {
                Some(o) => { l.put(b"  key ").num(o.pku_key as u64, 2).put(b"  caps ").num(o.capabilities, 3); }
                None => { l.put(b"  key  -  caps   -"); }
            }
            l.put(b"  calls:");
            let (mut to, mut links) = (0u32, 0usize);
            while let Some(d) = mesh::link(pd, to) {
                to = d;
                links += 1;
                if links <= 6 { l.put(b" "); name_into(l, d, 0); }
            }
            if links == 0 { l.put(b" -"); } else if links > 6 { l.put(b" +").num(links as u64 - 6, 1); }
            if log { sex_pdx::serial_record!("[monitor.organ] pd={} key={} caps={} links={}", pd, organ.map_or(-1, |o| o.pku_key as i32), organ.map_or(0, |o| o.capabilities), links); }
            n += 1;
            let task = stats::domain(pd).map_or(0, |d| d.task);
            if let Some(run) = runs::parse(&self.runs[..self.runs_len]).find(|r| r.target == pd && r.task == task && r.grants > 0) {
                if n == GRANT_LINES { break; }
                let g = &mut self.grants[n];
                g.clear();
                g.put(b"    file grants ").num(run.grants as u64, 1).put(b": ");
                if run.text.is_empty() { g.put(b"(paths shown when Mesh holds @/)"); } else { g.put(run.text); }
                n += 1;
            }
        }
        self.grant_count = n;
    }

    fn render(&mut self, window: &mut Window) {
        let (w, h) = window.size();
        let px = window.pixels();
        let w = w.clamp(64, 4096);
        let h = h.clamp(64, px.len() / w);
        let Some(mut c) = Canvas::new(&mut px[..w * h], w, h) else { return };
        let g = unsafe { &mut *core::ptr::addr_of_mut!(GLYPHS) };
        c.fill(0, 0, w as i32, h as i32, BG);
        for t in layout::TABS {
            let r = layout::tab_rect(t);
            let on = t == self.tab;
            c.fill(r.x, r.y, r.w, r.h, if on { PANEL } else { BG });
            if on { c.fill(r.x, r.y + r.h - 2, r.w, 2, ACCENT); }
            c.text(g, r.x + PAD, r.y + 3, t.label(), if on { TEXT } else { DIM });
        }
        match self.tab {
            Tab::Processes => self.draw_programs(&mut c, g, window.target),
            Tab::Performance => self.draw_performance(&mut c, g),
            Tab::Grants => { self.build_grants(false); self.draw_lines(&mut c, g, true); }
            Tab::Hardware => self.draw_lines(&mut c, g, false),
        }
        window.show_pixels(w, h);
    }

    fn summary(&self) -> Line<96> {
        let mut l = Line::new();
        l.put(b"CPU ").tenths(self.cpu.latest().unwrap_or(0) as u64).put(b" of ").num(self.now.cores, 1).put(b" cores   free").pages(self.now.free_pages);
        l
    }

    fn draw_programs(&mut self, c: &mut Canvas, g: &mut Glyphs, me: u32) {
        let (w, h) = (c.w as i32, c.h as i32);
        c.text(g, PAD, TOP, b"NAME             DOMAIN    CPU     MEMORY  CALLS/S    READS/S  GRANTS", DIM);
        let fit = layout::rows_fit(h);
        let s = &self.now;
        let chosen = self.chosen_index();
        // Keep the chosen line in view.
        if let Some(i) = chosen { if i < self.top { self.top = i; } else if fit > 0 && i >= self.top + fit { self.top = i + 1 - fit; } }
        self.top = self.top.min(s.count.saturating_sub(1));
        for (k, r) in s.rows[self.top..s.count].iter().take(fit).enumerate() {
            let y = layout::list_top() + k as i32 * ROW_H;
            if chosen == Some(self.top + k) { c.fill(0, y, w, ROW_H, mix(BG, ACCENT, 60)); c.fill(0, y, 3, ROW_H, ACCENT); }
            let mut l: Line<128> = Line::new();
            name_into(&mut l, r.pd, 16);
            l.num(r.pd as u64, 6).put(b"  ").tenths(r.tenths).put(b" ").pages(r.pages).num(r.rate, 9).put(b" ").size(r.read_rate).put(b"/s").num(r.grants as u64, 5);
            c.text(g, PAD, y + 1, l.as_bytes(), if r.pd == me { DIM } else { TEXT });
        }
        let bar = layout::button_bar(h);
        c.fill(0, bar - 4, w, 1, PANEL);
        let e = layout::end_button(w, h);
        let can_end = self.chosen.is_some_and(|(pd, _)| pd != me && is_program(pd));
        c.fill(e.x, e.y, e.w, e.h, if can_end { mix(BG, 0x00E5484D, 160) } else { PANEL });
        c.text(g, e.x + PAD, e.y + 2, layout::END_LABEL, if can_end { TEXT } else { DIM });
        let left = if self.status.is_empty() { self.summary() } else { let mut l = Line::new(); l.put(self.status.as_bytes()); l };
        let room = ((e.x - 2 * PAD) / CELL_W as i32).max(0) as usize;
        c.text(g, PAD, bar + 2, &left.as_bytes()[..left.len().min(room)], DIM);
    }

    fn graph<const N: usize>(c: &mut Canvas, g: &mut Glyphs, area: Area, title: &[u8], h: &History<N>, top: u32, color: u32) -> usize {
        c.text(g, area.x, area.y, title, TEXT);
        let plot = Area { x: area.x, y: area.y + CELL_H as i32 + 2, w: area.w, h: area.h - CELL_H as i32 - 2 };
        chart::plot(c, plot, h, top, 3, Style { back: PANEL, grid: mix(PANEL, DIM, 50), line: color, fill: mix(PANEL, color, 70) })
    }

    fn draw_performance(&mut self, c: &mut Canvas, g: &mut Glyphs) {
        let (w, h) = (c.w as i32, c.h as i32);
        let (x0, y0, ww, hh) = (PAD, TOP, w - 2 * PAD, h - TOP - PAD);
        let top_h = hh * 11 / 20;
        let cores = (self.now.cores as usize).clamp(1, CORES);
        // The whole CPU on the left, each core on the right.
        let mut t: Line<64> = Line::new();
        t.put(b"CPU ").tenths(self.cpu.latest().unwrap_or(0) as u64).put(b"  ").num(self.now.cores, 1).put(b" cores");
        let split = cores > 1 && self.now.per_core;
        let left_w = if split { ww / 2 - PAD / 2 } else { ww };
        Self::graph(c, g, Area { x: x0, y: y0, w: left_w, h: top_h }, t.as_bytes(), &self.cpu, 1000, CPU);
        if split {
            let cols = (1..=cores).find(|k| k * k >= cores).unwrap_or(1) as i32;
            let rows = ((cores as i32) + cols - 1) / cols;
            let (gx, gw) = (x0 + left_w + PAD, ww - left_w - PAD);
            let (cw, ch) = (gw / cols, top_h / rows);
            for i in 0..cores {
                let (cx, cy) = (gx + (i as i32 % cols) * cw, y0 + (i as i32 / cols) * ch);
                let mut t: Line<32> = Line::new();
                t.put(b"core ").num(i as u64, 1).put(b" ").tenths(self.cores[i].latest().unwrap_or(0) as u64);
                Self::graph(c, g, Area { x: cx, y: cy, w: cw - 4, h: ch - 4 }, t.as_bytes(), &self.cores[i], 1000, CPU);
            }
        }
        // Memory, system calls and file reads below.
        let (by, bh, bw) = (y0 + top_h + PAD, hh - top_h - PAD, (ww - 2 * PAD) / 3);
        let mut t: Line<64> = Line::new();
        match self.now.memory {
            Some(m) => { t.put(b"Program memory").pages(m.lent_pages); }
            None => { t.put(b"Program memory -"); }
        }
        Self::graph(c, g, Area { x: x0, y: by, w: bw, h: bh }, t.as_bytes(), &self.memory, 1000, MEMORY);
        let mut t: Line<64> = Line::new();
        t.put(b"Calls/s").num(self.calls.latest().unwrap_or(0) as u64, 8);
        Self::graph(c, g, Area { x: x0 + bw + PAD, y: by, w: bw, h: bh }, t.as_bytes(), &self.calls, history::nice_ceiling(self.calls.max() as u64) as u32, CALLS);
        let mut t: Line<64> = Line::new();
        t.put(b"File reads").size(self.disk.latest().unwrap_or(0) as u64).put(b"/s");
        Self::graph(c, g, Area { x: x0 + 2 * (bw + PAD), y: by, w: bw, h: bh }, t.as_bytes(), &self.disk, history::nice_ceiling(self.disk.max() as u64).min(u32::MAX as u64) as u32, DISK);
    }

    /// The Grants tab (`grants` true) or the Hardware tab: lines of text, scrolled.
    fn draw_lines(&mut self, c: &mut Canvas, g: &mut Glyphs, grants: bool) {
        let mut hw: [Line<128>; 24] = [const { Line::new() }; 24];
        let lines: &[Line<160>];
        let hw_count;
        if grants {
            c.text(g, PAD, TOP, b"DOMAIN           ORGAN (PKU KEY), CAPABILITIES AND WHOM IT MAY CALL", DIM);
            lines = &self.grants[..self.grant_count];
            hw_count = 0;
        } else {
            c.text(g, PAD, TOP, b"HARDWARE", DIM);
            let s = &self.now;
            let mut n = 0;
            hw[n].put(b"Cores online      ").num(s.cores, 1); n += 1;
            for i in 0..if s.per_core { (s.cores as usize).min(CORES) } else { 0 } {
                hw[n].put(b"  core ").num(i as u64, 2).put(b"  busy ").tenths(self.cores[i].latest().unwrap_or(0) as u64).put(b"  running ");
                match mesh::core(i as u32) { Some(k) if k.running != 0 => name_into(&mut hw[n], k.running, 0), _ => { hw[n].put(b"nothing"); } }
                n += 1;
            }
            if let Some(m) = s.memory {
                hw[n].put(b"Program memory    ").pages(m.limit_pages).put(b" at most,").pages(m.lent_pages).put(b" in use,").pages(m.peak_pages).put(b" most used"); n += 1;
            }
            hw[n].put(b"Free for programs ").pages(s.free_pages); n += 1;
            n += 1;
            hw[n].put(b"Not read yet: temperatures, fans, battery wear, NVMe SMART"); n += 1;
            hw[n].put(b"and the boot timeline (they need the sensor readings of H9)."); n += 1;
            lines = &[];
            hw_count = n;
        }
        let fit = ((c.h as i32 - layout::list_top() - PAD) / ROW_H).max(0) as usize;
        let count = if grants { lines.len() } else { hw_count };
        self.top = self.top.min(count.saturating_sub(fit));
        for k in 0..fit.min(count.saturating_sub(self.top)) {
            let y = layout::list_top() + k as i32 * ROW_H;
            let text = if grants { lines[self.top + k].as_bytes() } else { hw[self.top + k].as_bytes() };
            let color = if text.starts_with(b"    ") || text.starts_with(b"Not") || text.starts_with(b"and") { ACCENT } else { TEXT };
            c.text(g, PAD, y + 1, text, color);
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Mesh");
    let mesh = unsafe { &mut *core::ptr::addr_of_mut!(MESH) };
    mesh.sample(&mut window);
    mesh.render(&mut window);
    sex_pdx::serial_record!("[monitor.ready] target={} task={}", window.target, window.task);
    let mut due = sex_pdx::get_ticks() + PERIOD_MS;
    loop {
        let wait = due.saturating_sub(sex_pdx::get_ticks());
        // Keys first: a sample that runs past its period must never starve them.
        let event = window.poll().or_else(|| if wait == 0 { None } else { window.next_within(wait) });
        let sampled = event.is_none();
        let redraw = match event {
            Some(Event::Close) => window.close(),
            Some(Event::Key(key)) => mesh.key(&window, key),
            Some(Event::Pointer(p)) => mesh.pointer(&window, p),
            Some(Event::Resize { .. }) => true,
            Some(_) => false,
            None => { mesh.sample(&mut window); true }
        };
        if redraw { mesh.render(&mut window); }
        // The next sample is a whole period after this one's work, however long it took.
        if sampled { due = sex_pdx::get_ticks() + PERIOD_MS; }
    }
}
