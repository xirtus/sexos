// SPDX-License-Identifier: GPL-3.0-only
//! Drag (D2): UTM-style macOS Noir Virtual Machine Manager GUI + confined VMM.
//! `run drag` boots the in-tree test guest; `run drag kernel=bzImage [initrd=FILE]
//! [disk=FILE] [ro] [mem=MIB] [-- kernel words]` boots a Linux bzImage.
//!
//! Serial log: `[drag.vmm.start]`, `[drag.vmm.loaded]`, then either
//! `[drag.vmm.refused] status=3 reason=...` or `[drag.vmm.run]`, `[drag.serial] ...`,
//! `[drag.vmm.stop]`, `[drag.disk] sector1=...`, `[drag.vmm.done]`.
#![no_std]
#![no_main]

extern crate alloc;

mod noir;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use drag_vmm::memory::{self, Ram};
use drag_vmm::virtio::{Blk, Disk};
use drag_vmm::vmm::{Host, Machine, Note, Stop, Vcpu};
use noir::{
    draw_card, fill_rect, window_dims, NoirPainter, AMBER, AMBER_BG, AMBER_BORDER, BG, BORDER,
    CARD, CARD_ALT, DIM, EMERALD, EMERALD_BG, EMERALD_BORDER, HEADER, MUTED, ROSE, ROSE_BG,
    ROSE_BORDER, SIDEBAR, SKY, SKY_BG, SKY_BORDER, TEXT, WELL,
};
use sex_app::{files, Event, Window};
use sex_pdx::app_pointer::Kind as PointerKind;
use sex_pdx::vm::{self, Exit, State};
use sex_theme::candy_ui::{fill_rounded_rect, ButtonState, ButtonStyle};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[drag.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const LOAN_PAGES: u64 = 4096;
const TEST_DISK: &[u8] = b"drag-test.img";

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

struct FileDisk { path: Vec<u8>, size: u64, read_only: bool }
impl Disk for FileDisk {
    fn len(&self) -> u64 { self.size }
    fn read_only(&self) -> bool { self.read_only }
    fn read(&mut self, offset: u64, out: &mut [u8]) -> Result<(), ()> {
        match files::direct::read_at(&self.path, offset, out) { Ok(n) if n == out.len() => Ok(()), _ => Err(()) }
    }
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), ()> {
        match files::direct::write_at(&self.path, offset, data) { Ok(n) if n == data.len() => Ok(()), _ => Err(()) }
    }
    fn flush(&mut self) -> Result<(), ()> { files::direct::sync().map_err(|_| ()) }
}

fn rdtsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }

#[derive(Clone, Copy, PartialEq, Eq)]
enum VmTab {
    Console,
    Inspect,
    Configure,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VmStatus {
    Ready,
    Running,
    Halted,
    Refused,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    SelectVm(usize),
    Tab(VmTab),
    StartVm,
    CycleRam(u32),
    ToggleRo,
    PickKernel,
    PickDisk,
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
struct Telemetry {
    guest_kind: String,
    protocol: u16,
    entry_bits: u8,
    rip: u64,
    disk_sectors: u64,
    exits: u64,
    io: u64,
    mmio: u64,
    cpuid: u64,
    msr: u64,
    hlt: u64,
    injected: u64,
    blk_reqs: u64,
    blk_read: u64,
    blk_written: u64,
    stop_reason: String,
    sector1: String,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            guest_kind: "test".into(),
            protocol: 0,
            entry_bits: 64,
            rip: 0,
            disk_sectors: 0,
            exits: 0,
            io: 0,
            mmio: 0,
            cpuid: 0,
            msr: 0,
            hlt: 0,
            injected: 0,
            blk_reqs: 0,
            blk_read: 0,
            blk_written: 0,
            stop_reason: "-".into(),
            sector1: String::new(),
        }
    }
}

struct Program<'a> {
    window: &'a mut Window,
    painter: NoirPainter,
    tsc_hz: u64,
    tsc0: u64,
    lines: Vec<Vec<u8>>,
    closed: bool,
    polls: u32,
    vm_idx: usize,
    tab: VmTab,
    status: VmStatus,
    banner: String,
    mem_mib: u32,
    read_only: bool,
    custom_kernel: Option<Vec<u8>>,
    custom_disk: Option<Vec<u8>>,
    telemetry: Telemetry,
    scroll: usize,
    hits: Vec<HitBox>,
    mouse: (i32, i32),
    mouse_down: bool,
    hovered: Option<usize>,
}

impl Program<'_> {
    fn btn_state(&self, x: i32, y: i32, w: i32, h: i32) -> ButtonState {
        let (mx, my) = self.mouse;
        if mx >= x && mx < x + w && my >= y && my < y + h {
            if self.mouse_down { ButtonState::Pressed } else { ButtonState::Hover }
        } else {
            ButtonState::Rest
        }
    }

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
        painter.text(buf, w, h, false, 18, 14, 110, 15, "DRAG VMM", TEXT);
        let (s_lbl, sfg, sbg, sbrd) = match self.status {
            VmStatus::Ready => ("Ready", SKY, SKY_BG, SKY_BORDER),
            VmStatus::Running => ("Running", EMERALD, EMERALD_BG, EMERALD_BORDER),
            VmStatus::Halted => ("Halted · OK", EMERALD, EMERALD_BG, EMERALD_BORDER),
            VmStatus::Refused => ("VMX Unavailable", ROSE, ROSE_BG, ROSE_BORDER),
            VmStatus::Error => ("Stopped", ROSE, ROSE_BG, ROSE_BORDER),
        };
        painter.pill(buf, w, h, 118, 13, 22, false, s_lbl, sfg, sbg, sbrd);

        // Top right UTM controls: [Start VM] [Inspect] [Configure]
        let running = self.status == VmStatus::Running;
        hit_btn(
            painter,
            hits,
            buf,
            iw - 116,
            10,
            100,
            28,
            if running { "Running…" } else { "Start VM" },
            if running { ButtonStyle::Disabled } else { ButtonStyle::Primary },
            HitAction::StartVm,
        );
        if iw >= 560 {
            hit_btn(
                painter,
                hits,
                buf,
                iw - 208,
                10,
                84,
                28,
                "Configure",
                if self.tab == VmTab::Configure { ButtonStyle::Active } else { ButtonStyle::Normal },
                HitAction::Tab(VmTab::Configure),
            );
            hit_btn(
                painter,
                hits,
                buf,
                iw - 294,
                10,
                78,
                28,
                "Inspect",
                if self.tab == VmTab::Inspect { ButtonStyle::Active } else { ButtonStyle::Normal },
                HitAction::Tab(VmTab::Inspect),
            );
        }

        // Sidebar (on wide windows >= 620px)
        let show_sidebar = iw >= 620;
        let sb_w = if show_sidebar { 200 } else { 0 };
        if show_sidebar {
            fill_rect(buf, w, h, 0, 48, sb_w, ih - 48, SIDEBAR);
            fill_rect(buf, w, h, sb_w - 1, 48, 1, ih - 48, BORDER);
            painter.text(buf, w, h, false, 16, 62, sb_w - 32, 11, "VIRTUAL MACHINES", DIM);
            let vms = [
                ("Drag Test Guest", "bzImage · 8 MiB · virtio"),
                ("Custom Linux VM", "bzImage + virtio-blk"),
            ];
            let mut vy = 84;
            for (idx, (title, sub)) in vms.iter().enumerate() {
                let sel = self.vm_idx == idx;
                draw_card(buf, w, h, 12, vy, sb_w - 24, 54, 8, if sel { CARD_ALT } else { CARD }, if sel { ROSE } else { BORDER });
                hits.push(HitBox { x: 12, y: vy, w: sb_w - 24, h: 54, action: HitAction::SelectVm(idx) });
                fill_rounded_rect(buf, w, h, 22, vy + 14, 8, 8, 4, if sel { ROSE } else { EMERALD });
                painter.text(buf, w, h, false, 36, vy + 10, sb_w - 48, 12, title, if sel { TEXT } else { MUTED });
                painter.text(buf, w, h, true, 36, vy + 29, sb_w - 48, 10, sub, DIM);
                vy += 64;
            }
        }

        let main_x = sb_w + 16;
        let main_w = (iw - main_x - 16).max(200);
        let mut my = 58;

        // Allocation Badges Row
        let mut bx = main_x;
        let badges = [
            (format!("1 vCPU · x86_{}", self.telemetry.entry_bits), SKY, SKY_BG, SKY_BORDER),
            (format!("RAM: {} MiB", self.mem_mib), EMERALD, EMERALD_BG, EMERALD_BORDER),
            (format!("virtio-blk ({} sec)", self.telemetry.disk_sectors), AMBER, AMBER_BG, AMBER_BORDER),
        ];
        for (lbl, fg, bbg, brd) in badges {
            let pw = painter.measure(true, 11, &lbl) + 18;
            if bx + pw > main_x + main_w { break; }
            bx += painter.pill(buf, w, h, bx, my, 22, true, &lbl, fg, bbg, brd) + 8;
        }
        my += 30;

        // View Tabs
        let tabs = [(VmTab::Console, "Serial Console"), (VmTab::Inspect, "VM Inspector"), (VmTab::Configure, "Configure")];
        let mut tx = main_x;
        for (tab, lbl) in tabs {
            let tw = (painter.measure(false, 11, lbl) + 22).max(86);
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
            VmTab::Console => {
                painter.text(buf, w, h, false, main_x + 14, my + 10, main_w - 28, 12, "16550 UART Serial Console", TEXT);
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
            VmTab::Inspect => {
                painter.text(buf, w, h, false, main_x + 16, my + 12, main_w - 32, 13, "SYS_VM Hypervisor & Device Telemetry", TEXT);
                fill_rect(buf, w, h, main_x + 12, my + 36, main_w - 24, 1, BORDER);
                let items = [
                    ("Guest Mode", format!("{} (protocol {:#x})", self.telemetry.guest_kind, self.telemetry.protocol)),
                    ("Entry RIP", format!("{:#x} ({}-bit)", self.telemetry.rip, self.telemetry.entry_bits)),
                    ("Stop Reason", self.telemetry.stop_reason.clone()),
                    ("VM Exits", format!("{} total · {} I/O · {} MMIO · {} HLT", self.telemetry.exits, self.telemetry.io, self.telemetry.mmio, self.telemetry.hlt)),
                    ("CPUID / MSR", format!("{} CPUID · {} MSR · {} IRQ", self.telemetry.cpuid, self.telemetry.msr, self.telemetry.injected)),
                    ("virtio-blk", format!("{} reqs · {}B read · {}B written", self.telemetry.blk_reqs, self.telemetry.blk_read, self.telemetry.blk_written)),
                    ("Disk Sector 1", if self.telemetry.sector1.is_empty() { "-".into() } else { self.telemetry.sector1.clone() }),
                ];
                let mut iy = my + 46;
                for (k, v) in items {
                    if iy > my + card_h - 20 { break; }
                    painter.text(buf, w, h, false, main_x + 16, iy, 120, 12, k, DIM);
                    painter.text(buf, w, h, true, main_x + 136, iy, main_w - 152, 12, &v, MUTED);
                    iy += 24;
                }
            }
            VmTab::Configure => {
                painter.text(buf, w, h, false, main_x + 16, my + 12, main_w - 32, 13, "VM Hardware & Boot Configuration", TEXT);
                fill_rect(buf, w, h, main_x + 12, my + 36, main_w - 24, 1, BORDER);
                painter.text(buf, w, h, false, main_x + 16, my + 50, 120, 12, "Memory Allocation", MUTED);
                let mut rx = main_x + 146;
                for mib in [8u32, 32, 64, 128] {
                    if rx + 68 > main_x + main_w { break; }
                    let lbl = format!("{} MiB", mib);
                    hit_btn(
                        painter,
                        hits,
                        buf,
                        rx,
                        my + 44,
                        64,
                        26,
                        &lbl,
                        if self.mem_mib == mib { ButtonStyle::Active } else { ButtonStyle::Normal },
                        HitAction::CycleRam(mib),
                    );
                    rx += 72;
                }
                painter.text(buf, w, h, false, main_x + 16, my + 88, 120, 12, "virtio-blk Mode", MUTED);
                hit_btn(
                    painter,
                    hits,
                    buf,
                    main_x + 146,
                    my + 82,
                    124,
                    26,
                    if self.read_only { "Read-Only" } else { "Read-Write" },
                    if self.read_only { ButtonStyle::Active } else { ButtonStyle::Normal },
                    HitAction::ToggleRo,
                );

                painter.text(buf, w, h, false, main_x + 16, my + 126, 120, 12, "Kernel bzImage", MUTED);
                let k_lbl = self.custom_kernel.as_ref().map(|k| String::from_utf8_lossy(k).into_owned()).unwrap_or_else(|| "Built-in Test Guest".into());
                painter.text(buf, w, h, true, main_x + 146, my + 126, main_w - 266, 11, &k_lbl, SKY);
                hit_btn(painter, hits, buf, main_x + main_w - 114, my + 120, 98, 26, "Pick Kernel…", ButtonStyle::Subtle, HitAction::PickKernel);

                painter.text(buf, w, h, false, main_x + 16, my + 162, 120, 12, "Disk Image", MUTED);
                let d_lbl = self.custom_disk.as_ref().map(|d| String::from_utf8_lossy(d).into_owned()).unwrap_or_else(|| "drag-test.img".into());
                painter.text(buf, w, h, true, main_x + 146, my + 162, main_w - 266, 11, &d_lbl, SKY);
                hit_btn(painter, hits, buf, main_x + main_w - 114, my + 156, 98, 26, "Pick Disk…", ButtonStyle::Subtle, HitAction::PickDisk);

                hit_btn(painter, hits, buf, main_x + 16, my + card_h - 40, 124, 28, "Boot VM Now", ButtonStyle::Primary, HitAction::StartVm);
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

    fn closing(&mut self) -> bool {
        while let Some(e) = self.window.poll() {
            if let Event::Close = e { self.closed = true; }
        }
        self.closed
    }
}

impl Host for Program<'_> {
    fn now_ns(&mut self) -> u64 { ((rdtsc().wrapping_sub(self.tsc0)) as u128 * 1_000_000_000 / self.tsc_hz as u128) as u64 }
    fn tsc(&mut self) -> u64 { rdtsc() }
    fn cpuid(&mut self, leaf: u32, subleaf: u32) -> [u32; 4] {
        let r = core::arch::x86_64::__cpuid_count(leaf, subleaf);
        [r.eax, r.ebx, r.ecx, r.edx]
    }
    fn console(&mut self, line: &[u8]) {
        let text = core::str::from_utf8(line).unwrap_or("?");
        sex_pdx::serial_record!("[drag.serial] {}", text);
        if self.lines.len() == 64 { self.lines.remove(0); }
        self.lines.push(line.to_vec());
        self.show(b"Drag: guest running.");
    }
    fn note(&mut self, note: Note) {
        match note {
            Note::Port { port, write } => sex_pdx::serial_record!("[drag.vmm.note] port={:#x} write={}", port, write as u8),
            Note::Mmio { gpa, write } => sex_pdx::serial_record!("[drag.vmm.note] mmio={:#x} write={}", gpa, write as u8),
            Note::Msr { index, write } => sex_pdx::serial_record!("[drag.vmm.note] msr={:#x} write={} gp=1", index, write as u8),
            Note::Injected { vector } => sex_pdx::serial_record!("[drag.vmm.note] injected={:#x}", vector),
        }
    }
    fn idle(&mut self, until: Option<u64>) -> bool {
        if self.closing() { return false; }
        let ms = match until { Some(t) => (t.saturating_sub(self.now_ns()) / 1_000_000).clamp(1, 64), None => 16 };
        let _ = sex_pdx::sys_wait_self_ticks(ms);
        true
    }
    fn keep_going(&mut self) -> bool {
        self.polls = self.polls.wrapping_add(1);
        if self.polls % 64 != 0 { return true; }
        !self.closing()
    }
}

fn tsc_hz() -> u64 {
    let t0 = sex_pdx::get_ticks();
    while sex_pdx::get_ticks() == t0 { sex_pdx::sys_yield(); }
    let (start, c0) = (sex_pdx::get_ticks(), rdtsc());
    while sex_pdx::get_ticks() < start + 20 { let _ = sex_pdx::sys_wait_self_ticks(1); }
    let hz = (rdtsc() - c0) * 1000 / (sex_pdx::get_ticks() - start).max(1);
    hz.max(1_000_000)
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

fn stop_line(stop: &Stop) -> (&'static str, u64, u64) {
    match *stop {
        Stop::Halted { rip } => ("halted", rip, 0),
        Stop::TripleFault { rip } => ("triple_fault", rip, 0),
        Stop::EntryFailed { reason } => ("entry_failed", 0, reason as u64),
        Stop::Hypervisor { status } => ("hypervisor", 0, status),
        Stop::Mmio { gpa, rip, .. } => ("mmio_undecoded", rip, gpa),
        Stop::LostMemory { gpa } => ("lost_memory", 0, gpa),
        Stop::Unsupported { reason, rip, .. } => ("unsupported_exit", rip, reason as u64),
        Stop::Host => ("host", 0, 0),
    }
}

fn guest(p: &mut Program, args: &drag_vmm::Args) -> Result<(), (&'static str, u64)> {
    let test = args.kernel.is_none();
    p.status = VmStatus::Running;
    p.lines.clear();
    p.scroll = 0;
    let disk = match (&args.disk, test) {
        (Some(path), _) => {
            let st = files::direct::stat(path).map_err(|e| ("disk_stat", e as u64))?;
            Some(FileDisk { path: path.clone(), size: st.size & !511, read_only: args.read_only })
        }
        (None, true) => {
            files::write(p.window, TEST_DISK, &drag_vmm::test_disk()).map_err(|e| ("disk_create", e as u64))?;
            Some(FileDisk { path: TEST_DISK.to_vec(), size: drag_vmm::test_disk().len() as u64, read_only: false })
        }
        (None, false) => None,
    };
    let kernel = match &args.kernel {
        Some(path) => read_file(path, 64 << 20).map_err(|_| ("kernel_read", 0))?,
        None => drag_vmm::TEST_GUEST.to_vec(),
    };
    let initrd = match &args.initrd { Some(path) => Some(read_file(path, 512 << 20).map_err(|_| ("initrd_read", 0))?), None => None };
    let ram = Ram::new((args.mem_mib as usize) << 20).ok_or(("guest_ram", args.mem_mib as u64))?;
    let blk = disk.map(|d| Blk::new(d, drag_vmm::BLK_BASE, drag_vmm::BLK_IRQ));
    let mut m = Machine::new(ram, blk);
    let mut cmdline = Vec::new();
    cmdline.extend_from_slice(drag_vmm::DEFAULT_CMDLINE);
    if !args.cmdline.is_empty() { cmdline.push(b' '); cmdline.extend_from_slice(&args.cmdline); }
    let boot = m.load_linux(&kernel, initrd.as_deref(), &cmdline).map_err(|_| ("load", 0))?;
    drop(kernel);
    p.telemetry.guest_kind = if test { "test" } else { "bzimage" }.into();
    p.telemetry.protocol = boot.header.version;
    p.telemetry.entry_bits = if boot.header.entry64 { 64 } else { 32 };
    p.telemetry.rip = boot.entry.regs.rip;
    p.telemetry.disk_sectors = m.blk.as_ref().map_or(0, |b| b.disk.len() / 512);
    sex_pdx::serial_record!("[drag.vmm.loaded] guest={} protocol={:#x} entry={} rip={:#x} kernel={:#x}+{:#x} mem_mib={} disk={}",
        if test { "test" } else { "bzimage" }, boot.header.version, if boot.header.entry64 { 64 } else { 32 }, boot.entry.regs.rip,
        boot.kernel.0, boot.kernel.1, args.mem_mib, m.blk.as_ref().map_or(0, |b| b.disk.len() / 512));

    match vm::info() {
        Ok((true, _)) => {}
        Ok((false, reason)) => return Err(("unavailable", reason)),
        Err(s) => return Err(("info", s)),
    }
    let machine = vm::create().map_err(|s| ("create", s))?;
    let result = (|| {
        lend(machine, &m.ram)?;
        let cpu = vm::vcpu_create(machine).map_err(|s| ("vcpu", s))?;
        vm::set_state(machine, cpu, &boot.entry).map_err(|s| ("set_regs", s))?;
        sex_pdx::serial_record!("[drag.vmm.run] vm={} pages={}", machine, m.ram.len() / memory::PAGE);
        p.show(b"Drag: guest running.");
        let mut vcpu = KernelVcpu::new(machine, cpu);
        let stop = m.run(&mut vcpu, p, u64::MAX);
        if !m.console_pending().is_empty() { p.console(&m.console_pending().to_vec()); }
        let (why, rip, detail) = stop_line(&stop);
        let c = m.counters;
        p.telemetry.stop_reason = why.into();
        p.telemetry.exits = c.exits;
        p.telemetry.io = c.io;
        p.telemetry.mmio = c.mmio;
        p.telemetry.cpuid = c.cpuid;
        p.telemetry.msr = c.msr;
        p.telemetry.hlt = c.hlt;
        p.telemetry.injected = c.injected;
        sex_pdx::serial_record!("[drag.vmm.stop] reason={} rip={:#x} detail={:#x} exits={} io={} mmio={} cpuid={} msr={} hlt={} injected={}",
            why, rip, detail, c.exits, c.io, c.mmio, c.cpuid, c.msr, c.hlt, c.injected);
        if let Stop::Mmio { bytes, len, .. } = stop { sex_pdx::serial_record!("[drag.vmm.mmio] bytes={:02x?}", &bytes[..len as usize]); }
        if let Some(b) = &m.blk {
            let s = b.stats;
            p.telemetry.blk_reqs = s.requests;
            p.telemetry.blk_read = s.read_bytes;
            p.telemetry.blk_written = s.written_bytes;
            sex_pdx::serial_record!("[drag.vmm.blk] requests={} read={} written={} errors={}", s.requests, s.read_bytes, s.written_bytes, s.errors);
        }
        Ok(stop)
    })();
    vm::destroy(machine).map_err(|s| ("destroy", s))?;
    let stop = result?;
    if test {
        let mut sector = [0u8; 31];
        let _ = files::direct::read_at(TEST_DISK, 512, &mut sector);
        let s1 = core::str::from_utf8(&sector).unwrap_or("?");
        p.telemetry.sector1 = s1.into();
        sex_pdx::serial_record!("[drag.disk] sector1={}", s1);
    }
    match stop { Stop::Halted { .. } => Ok(()), other => Err((stop_line(&other).0, 0)) }
}

fn run_vm_into(p: &mut Program, args: &drag_vmm::Args) {
    let r = guest(p, args);
    let closed = p.closed;
    let banner = match r {
        Ok(()) => {
            p.status = VmStatus::Halted;
            sex_pdx::serial_record!("[drag.vmm.done] target={} task={}", p.window.target, p.window.task);
            "Drag: the guest finished cleanly. F11 closes."
        }
        Err(("unavailable", reason)) => {
            p.status = VmStatus::Refused;
            p.telemetry.stop_reason = format!("VMX/SVM unavailable (reason {})", reason);
            sex_pdx::serial_record!("[drag.vmm.refused] status={} reason={}", vm::UNAVAILABLE, reason);
            "Drag: this machine has no usable VMX/SVM. F11 closes."
        }
        Err(_) if closed => {
            sex_pdx::serial_record!("[drag.vmm.closed] target={} task={}", p.window.target, p.window.task);
            sex_app::launch::exit(0)
        }
        Err((step, status)) => {
            p.status = VmStatus::Error;
            p.telemetry.stop_reason = format!("{} (status {})", step, status);
            sex_pdx::serial_record!("[drag.vmm.fail] step={} status={}", step, status);
            "Drag: the guest stopped with an error (see Inspect or log). F11 closes."
        }
    };
    p.show(banner.as_bytes());
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Drag");
    let mut buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let line = files::args(&mut window, &mut buf).to_vec();
    sex_pdx::serial_record!("[drag.vmm.start] target={} task={} args_len={}", window.target, window.task, line.len());

    let mut p = Program {
        window: &mut window,
        painter: NoirPainter::new(),
        tsc_hz: 0,
        tsc0: rdtsc(),
        lines: Vec::new(),
        closed: false,
        polls: 0,
        vm_idx: 0,
        tab: VmTab::Console,
        status: VmStatus::Ready,
        banner: "Drag VMM Ready".into(),
        mem_mib: 8,
        read_only: false,
        custom_kernel: None,
        custom_disk: None,
        telemetry: Telemetry::default(),
        scroll: 0,
        hits: Vec::new(),
        mouse: (-1, -1),
        mouse_down: false,
        hovered: None,
    };

    match drag_vmm::Args::parse(&line) {
        Err(why) => {
            p.status = VmStatus::Error;
            sex_pdx::serial_record!("[drag.vmm.fail] step=args why={}", why);
            p.show(b"Usage: run drag [kernel=FILE] [initrd=FILE] [disk=FILE] [ro] [mem=MIB] [-- words]");
        }
        Ok(args) => {
            p.mem_mib = args.mem_mib;
            p.read_only = args.read_only;
            p.custom_kernel = args.kernel.clone();
            p.custom_disk = args.disk.clone();
            if p.custom_kernel.is_some() { p.vm_idx = 1; }
            p.tsc_hz = tsc_hz();
            run_vm_into(&mut p, &args);
        }
    }

    loop {
        match p.window.next() {
            Event::Close => {
                sex_pdx::serial_record!("[drag.vmm.closed] target={} task={}", p.window.target, p.window.task);
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
                        let _ = p.btn_state(0, 0, 0, 0);
                        p.render();
                    }
                    PointerKind::Up { button: 0 } => {
                        p.mouse_down = false;
                        if let Some(idx) = p.hits.iter().rposition(|b| p.mouse.0 >= b.x && p.mouse.0 < b.x + b.w && p.mouse.1 >= b.y && p.mouse.1 < b.y + b.h) {
                            let act = p.hits[idx].action;
                            match act {
                                HitAction::SelectVm(i) => {
                                    p.vm_idx = i;
                                    if i == 0 { p.custom_kernel = None; p.custom_disk = None; }
                                }
                                HitAction::Tab(t) => p.tab = t,
                                HitAction::CycleRam(m) => p.mem_mib = m,
                                HitAction::ToggleRo => p.read_only = !p.read_only,
                                HitAction::PickKernel => {
                                    let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                                    if let Ok(path) = files::pick(p.window, files::Pick::Open, b"", &mut out) {
                                        p.custom_kernel = Some(path.to_vec());
                                        p.vm_idx = 1;
                                    }
                                }
                                HitAction::PickDisk => {
                                    let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                                    if let Ok(path) = files::pick(p.window, files::Pick::Open, b"", &mut out) {
                                        p.custom_disk = Some(path.to_vec());
                                        p.vm_idx = 1;
                                    }
                                }
                                HitAction::StartVm => {
                                    if p.tsc_hz == 0 { p.tsc_hz = tsc_hz(); }
                                    let args = drag_vmm::Args {
                                        kernel: if p.vm_idx == 1 { p.custom_kernel.clone() } else { None },
                                        initrd: None,
                                        disk: if p.vm_idx == 1 { p.custom_disk.clone() } else { None },
                                        read_only: p.read_only,
                                        mem_mib: p.mem_mib,
                                        cmdline: Vec::new(),
                                    };
                                    run_vm_into(&mut p, &args);
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
