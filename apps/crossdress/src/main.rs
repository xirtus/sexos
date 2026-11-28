// SPDX-License-Identifier: GPL-3.0-only
//! Crossdress (CW1): Parallels / CrossOver-style macOS Noir Compatibility Cell GUI.
//! Runs a 64-bit Windows program or CLAP plugin DLL in a Drag SYS_VM cell with no guest OS.
#![no_std]
#![no_main]

extern crate alloc;

mod noir;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use drag_vmm::memory::{self, Ram};
use drag_vmm::vmm::Vcpu;
use noir::{
    draw_card, fill_rect, window_dims, NoirPainter, AMBER, AMBER_BG, AMBER_BORDER, BG, BORDER,
    CARD, CARD_ALT, DIM, EMERALD, EMERALD_BG, EMERALD_BORDER, HEADER, MUTED, ROSE, ROSE_BG,
    ROSE_BORDER, SIDEBAR, SKY, SKY_BG, SKY_BORDER, TEXT, WELL,
};
use sex_app::{files, Event, Window};
use sex_crossdress::clap::{self, ClapPlugin, ParamChange};
use sex_crossdress::process::{Host, Stop, WinProcess};
use sex_pdx::app_pointer::Kind as PointerKind;
use sex_pdx::vm::{self, Exit, State};
use sex_theme::candy_ui::{fill_rounded_rect, ButtonState, ButtonStyle};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[crossdress.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const DEFAULT_CELL_MIB: usize = 64;
const LOAN_PAGES: u64 = 4096;

#[repr(C, align(16))]
struct Fx([u8; 512]);

struct KernelVcpu { vm: u64, cpu: u64, host_fx: Fx, guest_fx: Fx }

impl KernelVcpu {
    fn new(vm: u64, cpu: u64) -> KernelVcpu {
        let mut guest = Fx([0; 512]);
        guest.0[0..2].copy_from_slice(&0x037Fu16.to_le_bytes());
        guest.0[24..28].copy_from_slice(&0x1F80u32.to_le_bytes());
        KernelVcpu { vm, cpu, host_fx: Fx([0; 512]), guest_fx: guest }
    }
}

impl Vcpu for KernelVcpu {
    fn run(&mut self, exit: &mut Exit, load_regs: bool) -> Result<(), u64> {
        let flags = if load_regs { vm::RUN_LOAD_REGS } else { 0 };
        let status: u64;
        unsafe {
            core::arch::asm!(
                "fxsave64 [{host}]",
                "fxrstor64 [{guest}]",
                "syscall",
                "fxsave64 [{guest}]",
                "fxrstor64 [{host}]",
                host = in(reg) self.host_fx.0.as_mut_ptr(),
                guest = in(reg) self.guest_fx.0.as_mut_ptr(),
                inlateout("rax") vm::SYS_VM => status,
                in("rdi") vm::OP_VCPU_RUN,
                inlateout("rsi") self.vm => _,
                inlateout("rdx") self.cpu => _,
                inlateout("r10") exit as *mut Exit as u64 => _,
                inlateout("r8") flags => _,
                out("rcx") _, out("r11") _,
                options(nostack),
            );
        }
        if status == vm::OK { Ok(()) } else { Err(status) }
    }
    fn get_state(&mut self, st: &mut State) -> Result<(), u64> { vm::get_state(self.vm, self.cpu, st) }
    fn set_state(&mut self, st: &State) -> Result<(), u64> { vm::set_state(self.vm, self.cpu, st) }
    fn inject(&mut self, vector: u8, kind: u32, error: Option<u32>) -> Result<(), u64> { vm::inject(self.vm, self.cpu, vector, kind, error) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellTab {
    Console,
    Inspect,
    Configure,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellStatus {
    Ready,
    Running,
    ExitedOk,
    Refused,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    SelectCell(usize),
    Tab(CellTab),
    StartCell,
    SetRam(usize),
    PickBinary,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

#[derive(Clone)]
struct CellTelemetry {
    mode: String,
    base: u64,
    entry: u64,
    imports: usize,
    sections: usize,
    exits: u64,
    calls: u64,
    exceptions: u64,
    handled: u64,
    stop_reason: String,
    clap_id: String,
    clap_frames: usize,
    clap_exact: bool,
}

impl Default for CellTelemetry {
    fn default() -> Self {
        Self {
            mode: "Win64 PE32+".into(),
            base: 0,
            entry: 0,
            imports: 0,
            sections: 0,
            exits: 0,
            calls: 0,
            exceptions: 0,
            handled: 0,
            stop_reason: "-".into(),
            clap_id: String::new(),
            clap_frames: 0,
            clap_exact: false,
        }
    }
}

struct Program<'a> {
    window: &'a mut Window,
    painter: NoirPainter,
    line: Vec<u8>,
    lines: Vec<Vec<u8>>,
    t0: u64,
    closed: bool,
    polls: u32,
    cell_idx: usize,
    tab: CellTab,
    status: CellStatus,
    banner: String,
    cell_mib: usize,
    custom_path: Option<Vec<u8>>,
    telemetry: CellTelemetry,
    scroll: usize,
    hits: Vec<HitBox>,
    mouse: (i32, i32),
    mouse_down: bool,
    hovered: Option<usize>,
}

impl Program<'_> {
    fn render(&mut self) {
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

        let hit_btn = |painter: &mut NoirPainter, hits: &mut Vec<HitBox>, buf: &mut [u32], x: i32, y: i32, bw: i32, bh: i32, lbl: &str, st: ButtonStyle, act: HitAction| {
            let bstate = if matches!(st, ButtonStyle::Disabled) {
                ButtonState::Rest
            } else if mouse.0 >= x && mouse.0 < x + bw && mouse.1 >= y && mouse.1 < y + bh {
                if mouse_down { ButtonState::Pressed } else { ButtonState::Hover }
            } else {
                ButtonState::Rest
            };
            painter.button(buf, w, h, x, y, bw, bh, lbl, st, bstate);
            if !matches!(st, ButtonStyle::Disabled) {
                hits.push(HitBox { x, y, w: bw, h: bh, action: act });
            }
        };

        // Top Toolbar (48px)
        fill_rect(buf, w, h, 0, 0, iw, 48, HEADER);
        fill_rect(buf, w, h, 0, 47, iw, 1, BORDER);
        painter.text(buf, w, h, false, 18, 14, 130, 15, "CROSSDRESS", TEXT);
        let (s_lbl, sfg, sbg, sbrd) = match self.status {
            CellStatus::Ready => ("Ready", SKY, SKY_BG, SKY_BORDER),
            CellStatus::Running => ("Running", EMERALD, EMERALD_BG, EMERALD_BORDER),
            CellStatus::ExitedOk => ("Cell OK", EMERALD, EMERALD_BG, EMERALD_BORDER),
            CellStatus::Refused => ("VMX Unavailable", ROSE, ROSE_BG, ROSE_BORDER),
            CellStatus::Error => ("Stopped", ROSE, ROSE_BG, ROSE_BORDER),
        };
        painter.pill(buf, w, h, 134, 13, 22, false, s_lbl, sfg, sbg, sbrd);

        let running = self.status == CellStatus::Running;
        hit_btn(
            painter,
            hits,
            buf,
            iw - 120,
            10,
            104,
            28,
            if running { "Running…" } else { "Start Cell" },
            if running { ButtonStyle::Disabled } else { ButtonStyle::Primary },
            HitAction::StartCell,
        );
        if iw >= 580 {
            hit_btn(
                painter,
                hits,
                buf,
                iw - 212,
                10,
                84,
                28,
                "Configure",
                if self.tab == CellTab::Configure { ButtonStyle::Active } else { ButtonStyle::Normal },
                HitAction::Tab(CellTab::Configure),
            );
            hit_btn(
                painter,
                hits,
                buf,
                iw - 298,
                10,
                78,
                28,
                "Inspect",
                if self.tab == CellTab::Inspect { ButtonStyle::Active } else { ButtonStyle::Normal },
                HitAction::Tab(CellTab::Inspect),
            );
        }

        // Sidebar (on wide windows)
        let show_sidebar = iw >= 620;
        let sb_w = if show_sidebar { 206 } else { 0 };
        if show_sidebar {
            fill_rect(buf, w, h, 0, 48, sb_w, ih - 48, SIDEBAR);
            fill_rect(buf, w, h, sb_w - 1, 48, 1, ih - 48, BORDER);
            painter.text(buf, w, h, false, 16, 62, sb_w - 32, 11, "COMPATIBILITY CELLS", DIM);
            let cells = [
                ("Win64 PE Cell", "hello.exe · SYS_VM"),
                ("CLAP Audio Cell", "gain.clap · 48 kHz"),
                ("Custom Binary", "Pick .exe / .clap…"),
            ];
            let mut cy = 84;
            for (idx, (title, sub)) in cells.iter().enumerate() {
                let sel = self.cell_idx == idx;
                draw_card(buf, w, h, 12, cy, sb_w - 24, 54, 8, if sel { CARD_ALT } else { CARD }, if sel { ROSE } else { BORDER });
                hits.push(HitBox { x: 12, y: cy, w: sb_w - 24, h: 54, action: HitAction::SelectCell(idx) });
                fill_rounded_rect(buf, w, h, 22, cy + 14, 8, 8, 4, if sel { ROSE } else { SKY });
                painter.text(buf, w, h, false, 36, cy + 10, sb_w - 48, 12, title, if sel { TEXT } else { MUTED });
                painter.text(buf, w, h, true, 36, cy + 29, sb_w - 48, 10, sub, DIM);
                cy += 64;
            }
        }

        let main_x = sb_w + 16;
        let main_w = (iw - main_x - 16).max(200);
        let mut my = 58;

        // Allocation & Security Badges
        let mut bx = main_x;
        let badges = [
            (self.telemetry.mode.clone(), SKY, SKY_BG, SKY_BORDER),
            (format!("Cell: {} MiB", self.cell_mib), EMERALD, EMERALD_BG, EMERALD_BORDER),
            ("WRPKRU Isolated".into(), AMBER, AMBER_BG, AMBER_BORDER),
        ];
        for (lbl, fg, bbg, brd) in badges {
            let pw = painter.measure(true, 11, &lbl) + 18;
            if bx + pw > main_x + main_w { break; }
            bx += painter.pill(buf, w, h, bx, my, 22, true, &lbl, fg, bbg, brd) + 8;
        }
        my += 30;

        let tabs = [(CellTab::Console, "Output Console"), (CellTab::Inspect, "PE / Cell Inspector"), (CellTab::Configure, "Configure")];
        let mut tx = main_x;
        for (tab, lbl) in tabs {
            let tw = (painter.measure(false, 11, lbl) + 22).max(88);
            if tx + tw > main_x + main_w { break; }
            hit_btn(
                painter,
                hits,
                buf,
                tx,
                my,
                tw,
                26,
                lbl,
                if self.tab == tab { ButtonStyle::Active } else { ButtonStyle::Subtle },
                HitAction::Tab(tab),
            );
            tx += tw + 8;
        }
        my += 34;

        let card_h = (ih - my - 38).max(90);
        draw_card(buf, w, h, main_x, my, main_w, card_h, 10, CARD, BORDER);

        match self.tab {
            CellTab::Console => {
                painter.text(buf, w, h, false, main_x + 14, my + 10, main_w - 28, 12, "Windows Process / CLAP Host Output", TEXT);
                fill_rounded_rect(buf, w, h, main_x + 10, my + 32, main_w - 20, card_h - 42, 8, WELL);
                let mut ly = my + 40;
                let line_h = 18;
                let max_lines = ((card_h - 54) / line_h).max(1) as usize;
                let skip = self.lines.len().saturating_sub(max_lines + self.scroll);
                if self.lines.is_empty() {
                    painter.text(buf, w, h, true, main_x + 20, ly, main_w - 40, 11, &self.banner, MUTED);
                } else {
                    for l in self.lines.iter().skip(skip).take(max_lines) {
                        let s = core::str::from_utf8(l).unwrap_or("?");
                        painter.text(buf, w, h, true, main_x + 20, ly, main_w - 40, 11, s, EMERALD);
                        ly += line_h;
                    }
                }
            }
            CellTab::Inspect => {
                painter.text(buf, w, h, false, main_x + 16, my + 12, main_w - 32, 13, "PE32+ Loader & SYS_VM Cell Telemetry", TEXT);
                fill_rect(buf, w, h, main_x + 12, my + 36, main_w - 24, 1, BORDER);
                let items = [
                    ("Cell Type", self.telemetry.mode.clone()),
                    ("Image Base / Entry", format!("{:#x}  ->  {:#x}", self.telemetry.base, self.telemetry.entry)),
                    ("Imports / Sections", format!("{} imports · {} sections", self.telemetry.imports, self.telemetry.sections)),
                    ("Stop Reason", self.telemetry.stop_reason.clone()),
                    ("Cell Counters", format!("{} exits · {} calls · {} exceptions ({} handled)", self.telemetry.exits, self.telemetry.calls, self.telemetry.exceptions, self.telemetry.handled)),
                    ("CLAP Bridge", if self.telemetry.clap_id.is_empty() { "-".into() } else { format!("{} ({} frames, exact={})", self.telemetry.clap_id, self.telemetry.clap_frames, self.telemetry.clap_exact) }),
                ];
                let mut iy = my + 46;
                for (k, v) in items {
                    if iy > my + card_h - 20 { break; }
                    painter.text(buf, w, h, false, main_x + 16, iy, 136, 12, k, DIM);
                    painter.text(buf, w, h, true, main_x + 152, iy, main_w - 168, 12, &v, MUTED);
                    iy += 24;
                }
            }
            CellTab::Configure => {
                painter.text(buf, w, h, false, main_x + 16, my + 12, main_w - 32, 13, "Compatibility Cell Configuration", TEXT);
                fill_rect(buf, w, h, main_x + 12, my + 36, main_w - 24, 1, BORDER);
                painter.text(buf, w, h, false, main_x + 16, my + 50, 120, 12, "Cell Heap Size", MUTED);
                let mut rx = main_x + 146;
                for mib in [64usize, 128] {
                    let lbl = format!("{} MiB", mib);
                    hit_btn(
                        painter,
                        hits,
                        buf,
                        rx,
                        my + 44,
                        72,
                        26,
                        &lbl,
                        if self.cell_mib == mib { ButtonStyle::Active } else { ButtonStyle::Normal },
                        HitAction::SetRam(mib),
                    );
                    rx += 80;
                }
                painter.text(buf, w, h, false, main_x + 16, my + 90, 120, 12, "Binary Target", MUTED);
                let target_lbl = match self.cell_idx {
                    0 => "Built-in hello.exe (Win64 PE)".into(),
                    1 => "Built-in gain.clap (CLAP Bridge)".into(),
                    _ => self.custom_path.as_ref().map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_else(|| "No custom binary selected".into()),
                };
                painter.text(buf, w, h, true, main_x + 146, my + 90, main_w - 270, 11, &target_lbl, SKY);
                hit_btn(painter, hits, buf, main_x + main_w - 116, my + 84, 100, 26, "Pick File…", ButtonStyle::Subtle, HitAction::PickBinary);
                hit_btn(painter, hits, buf, main_x + 16, my + card_h - 40, 124, 28, "Start Cell", ButtonStyle::Primary, HitAction::StartCell);
            }
        }

        // Status footer
        let sy = ih - 30;
        fill_rounded_rect(buf, w, h, main_x, sy, main_w, 22, 6, CARD);
        painter.text(buf, w, h, false, main_x + 10, sy + 4, main_w - 20, 11, &self.banner, MUTED);

        self.window.show_pixels(w, h);
    }

    fn show(&mut self, status: &[u8]) {
        self.banner = String::from_utf8_lossy(status).into_owned();
        self.render();
    }

    fn finish_line(&mut self) {
        let line = core::mem::take(&mut self.line);
        sex_pdx::serial_record!("[crossdress.out] {}", core::str::from_utf8(&line).unwrap_or("?"));
        if self.lines.len() == 64 { self.lines.remove(0); }
        self.lines.push(line);
    }

    fn closing(&mut self) -> bool {
        while let Some(e) = self.window.poll() {
            if let Event::Close = e { self.closed = true; }
        }
        self.closed
    }
}

impl Host for Program<'_> {
    fn output(&mut self, _stream: u32, bytes: &[u8]) {
        for &b in bytes {
            match b {
                b'\n' => self.finish_line(),
                b'\r' => {}
                0x20..=0x7E if self.line.len() < 200 => self.line.push(b),
                _ => {}
            }
        }
        self.show(b"Crossdress: Windows program running.");
    }
    fn now_ms(&mut self) -> u64 { sex_pdx::get_ticks().saturating_sub(self.t0) }
    fn sleep(&mut self, ms: u32) { let _ = sex_pdx::sys_wait_self_ticks((ms as u64).clamp(1, 64)); }
    fn keep_going(&mut self) -> bool {
        self.polls = self.polls.wrapping_add(1);
        if self.polls % 64 != 0 { return true; }
        !self.closing()
    }
}

fn read_file(path: &[u8], limit: u64) -> Result<Vec<u8>, &'static str> {
    let st = files::direct::stat(path).map_err(|_| "file not found")?;
    if st.dir || st.size == 0 || st.size > limit { return Err("file empty, a folder, or too big"); }
    let mut data = Vec::new();
    data.try_reserve_exact(st.size as usize).map_err(|_| "no memory for the file")?;
    data.resize(st.size as usize, 0);
    match files::direct::read_at(path, 0, &mut data) { Ok(n) if n == data.len() => Ok(data), _ => Err("read failed") }
}

fn lend(machine: u64, ram: &Ram) -> Result<(), (&'static str, u64)> {
    let pages = ram.len() / memory::PAGE;
    for (first, n) in memory::loans(pages, LOAN_PAGES) {
        let mut tries = 0;
        loop {
            match vm::lend(machine, ram.host_address() + first * memory::PAGE, first * memory::PAGE, n as u32, vm::READ | vm::WRITE | vm::EXEC) {
                Ok(()) => break,
                Err(vm::BUSY) if tries < 200 => { tries += 1; let _ = sex_pdx::sys_wait_self_ticks(2); }
                Err(s) => return Err(("lend", s)),
            }
        }
    }
    Ok(())
}

fn stop_line(stop: &Stop) -> (&'static str, u64) {
    match stop {
        Stop::Exited(code) => ("exited", *code as u64),
        Stop::Unhandled { code, .. } => ("unhandled_exception", *code as u64),
        Stop::Unimplemented { .. } => ("unimplemented_import", 0),
        Stop::CellFault { vector, .. } => ("cell_fault", *vector as u64),
        Stop::TripleFault { .. } => ("triple_fault", 0),
        Stop::Unexpected { reason, .. } => ("unexpected_exit", *reason as u64),
        Stop::Hypervisor(s) => ("hypervisor", *s),
        Stop::Host => ("host", 0),
        Stop::Returned(v) => ("returned", *v),
    }
}

fn windows(p: &mut Program, exe: &[u8]) -> Result<Stop, (&'static str, u64)> {
    let cell_mib = p.cell_mib;
    let ram = Ram::new(cell_mib << 20).ok_or(("cell_ram", cell_mib as u64))?;
    let mut proc = WinProcess::load(ram, exe).map_err(|_| ("load", 0))?;
    p.telemetry.mode = "Win64 PE32+".into();
    p.telemetry.base = proc.base;
    p.telemetry.entry = proc.entry;
    p.telemetry.imports = proc.imports.len();
    p.telemetry.sections = proc.image.sections.len();
    sex_pdx::serial_record!("[crossdress.loaded] base={:#x} entry={:#x} imports={} sections={} cell_mib={}",
        proc.base, proc.entry, proc.imports.len(), proc.image.sections.len(), cell_mib);
    match vm::info() {
        Ok((true, _)) => {}
        Ok((false, reason)) => return Err(("unavailable", reason)),
        Err(s) => return Err(("info", s)),
    }
    let machine = vm::create().map_err(|s| ("create", s))?;
    let result = (|| {
        lend(machine, &proc.cell.ram)?;
        let cpu = vm::vcpu_create(machine).map_err(|s| ("vcpu", s))?;
        vm::set_state(machine, cpu, &proc.initial_state()).map_err(|s| ("set_regs", s))?;
        sex_pdx::serial_record!("[crossdress.run] vm={} pages={}", machine, proc.cell.ram.len() / memory::PAGE);
        let mut vcpu = KernelVcpu::new(machine, cpu);
        Ok(proc.run(&mut vcpu, p))
    })();
    vm::destroy(machine).map_err(|s| ("destroy", s))?;
    let stop = result?;
    if !p.line.is_empty() { p.finish_line(); }
    let (why, code) = stop_line(&stop);
    let c = proc.counters;
    p.telemetry.stop_reason = format!("{} (code {:#x})", why, code);
    p.telemetry.exits = c.exits;
    p.telemetry.calls = c.calls;
    p.telemetry.exceptions = c.exceptions;
    p.telemetry.handled = c.handled;
    sex_pdx::serial_record!("[crossdress.stop] reason={} code={:#x} exits={} calls={} exceptions={} handled={}",
        why, code, c.exits, c.calls, c.exceptions, c.handled);
    if let Stop::Unimplemented { dll, name } = &stop { sex_pdx::serial_record!("[crossdress.unimplemented] {}!{}", dll, name); }
    if let Stop::Unhandled { code, address } = &stop { sex_pdx::serial_record!("[crossdress.unhandled] code={:#x} address={:#x}", code, address); }
    Ok(stop)
}

fn clap_error(e: &clap::Error) -> (&'static str, u64) {
    match e {
        clap::Error::Load(_) => ("clap_load", 0),
        clap::Error::NotClap => ("not_clap", 0),
        clap::Error::Stopped(s) => { let (why, code) = stop_line(s); sex_pdx::serial_record!("[crossdress.clap.stopped] reason={} code={:#x}", why, code); ("clap_stopped", code) }
        clap::Error::Refused(what) => { sex_pdx::serial_record!("[crossdress.clap.refused] step={}", what); ("clap_refused", 0) }
        clap::Error::Bad(what) => { sex_pdx::serial_record!("[crossdress.clap.bad] what={}", what); ("clap_bad", 0) }
        clap::Error::Host(what) => { sex_pdx::serial_record!("[crossdress.clap.host] what={}", what); ("clap_host", 0) }
    }
}

fn signal(i: usize, step: usize) -> f32 { ((i * step) % 200) as f32 / 128.0 - 0.78125 }

fn plugin(p: &mut Program, file: &[u8], builtin: bool) -> Result<bool, (&'static str, u64)> {
    let cell_mib = p.cell_mib;
    let ram = Ram::new(cell_mib << 20).ok_or(("cell_ram", cell_mib as u64))?;
    let mut plug = ClapPlugin::load(ram, file).map_err(|e| clap_error(&e))?;
    p.telemetry.mode = "Win64 CLAP Bridge".into();
    p.telemetry.base = plug.process.base;
    p.telemetry.entry = plug.process.entry;
    p.telemetry.imports = plug.process.imports.len();
    sex_pdx::serial_record!("[crossdress.clap.loaded] base={:#x} entry={:#x} imports={} cell_mib={}",
        plug.process.base, plug.process.entry, plug.process.imports.len(), cell_mib);
    match vm::info() {
        Ok((true, _)) => {}
        Ok((false, reason)) => return Err(("unavailable", reason)),
        Err(s) => return Err(("info", s)),
    }
    let machine = vm::create().map_err(|s| ("create", s))?;
    let result = (|| {
        lend(machine, &plug.process.cell.ram)?;
        let cpu = vm::vcpu_create(machine).map_err(|s| ("vcpu", s))?;
        sex_pdx::serial_record!("[crossdress.run] vm={} pages={}", machine, plug.process.cell.ram.len() / memory::PAGE);
        let mut vcpu = KernelVcpu::new(machine, cpu);
        plug.open(&mut vcpu, p, "C:\\Program Files\\Common Files\\CLAP\\plugin.clap").map_err(|e| clap_error(&e))?;
        let d = &plug.descriptor;
        p.telemetry.clap_id = d.id.clone();
        sex_pdx::serial_record!("[crossdress.clap.open] id={} name={} vendor={} version={} params={}", d.id, d.name, d.vendor, d.version, plug.params.len());
        p.lines.push(format!("CLAP: {} ({}) v{}", d.name, d.id, d.version).into_bytes());
        for q in plug.params.iter().take(32) {
            sex_pdx::serial_record!("[crossdress.clap.param] id={} name={} min={} max={} default={}", q.id, q.name, q.min, q.max, q.default);
            p.lines.push(format!("  param[{}] {} = {} ({}..{})", q.id, q.name, q.default, q.min, q.max).into_bytes());
        }
        plug.activate(&mut vcpu, p, 48_000.0, 512).map_err(|e| clap_error(&e))?;
        let changes: [&[ParamChange]; 3] = if builtin {
            [&[], &[ParamChange { time: 100, id: 0, value: 0.5 }, ParamChange { time: 300, id: 0, value: 1.5 }], &[]]
        } else { [&[], &[], &[]] };
        let (mut at, mut gain, mut exact, mut peak, mut reported) = (0usize, 1.0f32, true, 0.0f32, 0usize);
        for (frames, ch) in [512usize, 512, 256].into_iter().zip(changes) {
            let l: Vec<f32> = (at..at + frames).map(|i| signal(i, 7)).collect();
            let r: Vec<f32> = (at..at + frames).map(|i| signal(i, 13)).collect();
            let (mut ol, mut or) = (alloc::vec![0f32; frames], alloc::vec![0f32; frames]);
            reported += plug.process(&mut vcpu, p, [&l, &r], [&mut ol, &mut or], ch).map_err(|e| clap_error(&e))?.len();
            for i in 0..frames {
                for c in ch.iter().filter(|c| c.time as usize == i) { gain = c.value as f32; }
                exact &= ol[i].to_bits() == (l[i] * gain).to_bits() && or[i].to_bits() == (r[i] * gain).to_bits();
                peak = peak.max(ol[i].abs()).max(or[i].abs());
            }
            at += frames;
        }
        plug.close(&mut vcpu, p).map_err(|e| clap_error(&e))?;
        let c = plug.process.counters;
        p.telemetry.exits = c.exits;
        p.telemetry.calls = c.calls;
        p.telemetry.clap_frames = at;
        p.telemetry.clap_exact = exact;
        p.telemetry.stop_reason = format!("clap_done (frames={})", at);
        sex_pdx::serial_record!("[crossdress.clap.done] frames={} exact={} peak={} reported={} callbacks={} calls={} exits={}",
            at, exact as u8, peak, reported, c.callbacks, c.calls, c.exits);
        p.lines.push(format!("Processed {} frames · exact={} · peak={} · reported={}", at, exact as u8, peak, reported).into_bytes());
        Ok(!builtin || (exact && reported == 2))
    })();
    vm::destroy(machine).map_err(|s| ("destroy", s))?;
    result
}

fn run_target(p: &mut Program, path: &[u8]) {
    p.status = CellStatus::Running;
    p.line.clear();
    p.lines.clear();
    p.scroll = 0;
    let builtin = path == b"plugin";
    if builtin || path.ends_with(b".clap") {
        let file = if builtin { Ok(sex_crossdress::TEST_CLAP.to_vec()) } else { read_file(path, 256 << 20) };
        let text: &[u8] = match file.map_err(|why| { sex_pdx::serial_record!("[crossdress.fail] step=read why={}", why); }) {
            Err(()) => {
                p.status = CellStatus::Error;
                b"Crossdress: could not read that plugin. F11 closes."
            }
            Ok(file) => match plugin(p, &file, builtin) {
                Ok(true) => {
                    p.status = CellStatus::ExitedOk;
                    b"Crossdress: the Windows plugin processed audio. F11 closes."
                }
                Ok(false) => {
                    p.status = CellStatus::Error;
                    sex_pdx::serial_record!("[crossdress.fail] step=clap_check status=0");
                    b"Crossdress: the plugin's audio was wrong (see log). F11 closes."
                }
                Err(("unavailable", reason)) => {
                    p.status = CellStatus::Refused;
                    p.telemetry.stop_reason = format!("VMX/SVM unavailable (reason {})", reason);
                    sex_pdx::serial_record!("[crossdress.refused] status={} reason={}", vm::UNAVAILABLE, reason);
                    b"Crossdress: this machine has no usable VMX/SVM. F11 closes."
                }
                Err((step, status)) => {
                    p.status = CellStatus::Error;
                    p.telemetry.stop_reason = format!("{} (status {})", step, status);
                    sex_pdx::serial_record!("[crossdress.fail] step={} status={}", step, status);
                    b"Crossdress: the plugin stopped with an error. F11 closes."
                }
            },
        };
        p.show(text);
        return;
    }

    let exe = if path.is_empty() { Ok(sex_crossdress::TEST_EXE.to_vec()) } else { read_file(path, 256 << 20) };
    let text: &[u8] = match exe {
        Err(why) => {
            p.status = CellStatus::Error;
            sex_pdx::serial_record!("[crossdress.fail] step=read why={}", why);
            b"Crossdress: could not read that .exe. F11 closes."
        }
        Ok(exe) => match windows(p, &exe) {
            Ok(Stop::Exited(0)) => {
                p.status = CellStatus::ExitedOk;
                sex_pdx::serial_record!("[crossdress.done] target={} task={}", p.window.target, p.window.task);
                b"Crossdress: the program finished cleanly. F11 closes."
            }
            Ok(_) => {
                p.status = CellStatus::Error;
                b"Crossdress: the program stopped (see Inspect or log). F11 closes."
            }
            Err(("unavailable", reason)) => {
                p.status = CellStatus::Refused;
                p.telemetry.stop_reason = format!("VMX/SVM unavailable (reason {})", reason);
                sex_pdx::serial_record!("[crossdress.refused] status={} reason={}", vm::UNAVAILABLE, reason);
                b"Crossdress: this machine has no usable VMX/SVM. F11 closes."
            }
            Err(_) if p.closed => sex_app::launch::exit(0),
            Err((step, status)) => {
                p.status = CellStatus::Error;
                p.telemetry.stop_reason = format!("{} (status {})", step, status);
                sex_pdx::serial_record!("[crossdress.fail] step={} status={}", step, status);
                b"Crossdress: the cell stopped with an error. F11 closes."
            }
        },
    };
    p.show(text);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Crossdress");
    let mut buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let line = files::args(&mut window, &mut buf).to_vec();
    sex_pdx::serial_record!("[crossdress.start] target={} task={} args_len={}", window.target, window.task, line.len());
    let path: Vec<u8> = line.split(|&c| c == b' ').find(|w| !w.is_empty()).map(|w| w.to_vec()).unwrap_or_default();

    let initial_idx = if path == b"plugin" || path.ends_with(b".clap") {
        1
    } else if !path.is_empty() {
        2
    } else {
        0
    };
    let mut p = Program {
        window: &mut window,
        painter: NoirPainter::new(),
        line: Vec::new(),
        lines: Vec::new(),
        t0: sex_pdx::get_ticks(),
        closed: false,
        polls: 0,
        cell_idx: initial_idx,
        tab: CellTab::Console,
        status: CellStatus::Ready,
        banner: "Crossdress Cell Ready".into(),
        cell_mib: DEFAULT_CELL_MIB,
        custom_path: if initial_idx == 2 { Some(path.clone()) } else { None },
        telemetry: CellTelemetry::default(),
        scroll: 0,
        hits: Vec::new(),
        mouse: (-1, -1),
        mouse_down: false,
        hovered: None,
    };

    run_target(&mut p, &path);

    loop {
        match p.window.next() {
            Event::Close => {
                sex_pdx::serial_record!("[crossdress.closed] target={} task={}", p.window.target, p.window.task);
                sex_app::launch::exit(0)
            }
            Event::Resize { .. } => p.render(),
            Event::Pointer(ptr) => {
                p.mouse = (ptr.x as i32, ptr.y as i32);
                match ptr.kind {
                    PointerKind::Move => {
                        let h = p.hits.iter().rposition(|b| p.mouse.0 >= b.x && p.mouse.0 < b.x + b.w && p.mouse.1 >= b.y && p.mouse.1 < b.y + b.h);
                        if h != p.hovered { p.hovered = h; p.render(); }
                    }
                    PointerKind::Leave => {
                        p.mouse_down = false;
                        if p.hovered.is_some() { p.hovered = None; p.render(); }
                    }
                    PointerKind::Down { button: 0 } => {
                        p.mouse_down = true;
                        p.render();
                    }
                    PointerKind::Up { button: 0 } => {
                        p.mouse_down = false;
                        if let Some(idx) = p.hits.iter().rposition(|b| p.mouse.0 >= b.x && p.mouse.0 < b.x + b.w && p.mouse.1 >= b.y && p.mouse.1 < b.y + b.h) {
                            let act = p.hits[idx].action;
                            match act {
                                HitAction::SelectCell(i) => p.cell_idx = i,
                                HitAction::Tab(t) => p.tab = t,
                                HitAction::SetRam(m) => p.cell_mib = m,
                                HitAction::PickBinary => {
                                    let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                                    if let Ok(picked) = files::pick(p.window, files::Pick::Open, b"", &mut out) {
                                        p.custom_path = Some(picked.to_vec());
                                        p.cell_idx = 2;
                                    }
                                }
                                HitAction::StartCell => {
                                    let target: Vec<u8> = match p.cell_idx {
                                        0 => Vec::new(),
                                        1 => b"plugin".to_vec(),
                                        _ => p.custom_path.clone().unwrap_or_default(),
                                    };
                                    run_target(&mut p, &target);
                                    continue;
                                }
                            }
                        }
                        p.render();
                    }
                    PointerKind::Wheel { steps } => {
                        let max_s = p.lines.len().saturating_sub(1);
                        p.scroll = (p.scroll as isize + steps as isize).clamp(0, max_s as isize) as usize;
                        p.render();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
