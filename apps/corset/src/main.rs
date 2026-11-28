// SPDX-License-Identifier: GPL-3.0-only
//! Corset: disks, partitions, and capability shields. Lists the disks Files offers
//! (USB sticks and disks, disk images), and on one the person hands it (Files asks them,
//! naming the disk) plans changes (`corset::plan`, on corset-core): a new GPT, partitions
//! added, deleted or resized, formatted FAT32, exFAT or sexfs, or encrypted.
//!
//! The system disk is listed and never opened: Files refuses it, whatever is asked.
#![no_std]
#![no_main]
extern crate alloc;

mod dev;
mod noir;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use corset::plan::{self, size_text, ApplyError, Disk, Fs, Op, Part, Plan, Refusal, Secret, MIB, TYPES};
use noir::{
    draw_card, fill_rect, window_dims, NoirPainter, AMBER, AMBER_BG, AMBER_BORDER, BG, BORDER,
    BORDER_ACTIVE, CARD, CARD_ALT, DIM, EMERALD, EMERALD_BG, EMERALD_BORDER, HEADER, MUTED, ROSE,
    ROSE_BG, ROSE_BORDER, SKY, SKY_BG, SKY_BORDER, TEXT, WELL,
};
use sex_app::files::disks::{self, Disk as Info};
use sex_app::{Event, Nav, Window};
use sex_pdx::app_pointer::Kind as PointerKind;
use sex_pdx::fs as w;
use sex_theme::candy_ui::{fill_rounded_rect, ButtonState, ButtonStyle};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[corset.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ENTER: u8 = 0x1C;
const ESC: u8 = 0x01;
const BACKSPACE: u8 = 0x0E;
const F5: u8 = 0x3F;
const KEY_G: u8 = 0x22;
const KEY_A: u8 = 0x1E;
const KEY_D: u8 = 0x20;
const KEY_R: u8 = 0x13;
const KEY_F: u8 = 0x21;
const KEY_E: u8 = 0x12;
const KEY_U: u8 = 0x16;
const KEY_H: u8 = 0x23;

/// What the prompt at the bottom is asking for.
enum Ask {
    AddSize { first: u64, room: u64 },
    AddName { first: u64, sectors: u64 },
    AddType { first: u64, sectors: u64, name: String },
    Resize { slot: usize },
    FormatFs { slot: usize },
    FormatLabel { slot: usize, fs: Fs },
    Password { slot: usize },
    PasswordAgain { slot: usize, first: Secret },
    EncryptFs { slot: usize, password: Secret },
    EncryptLabel { slot: usize, password: Secret, fs: Fs },
    /// The disk's name, typed to apply a plan that destroys data.
    Confirm,
}
impl Ask {
    fn secret(&self) -> bool { matches!(self, Ask::Password { .. } | Ask::PasswordAgain { .. }) }
    fn prompt(&self) -> String {
        match self {
            Ask::AddSize { room, .. } => format!("Size in MiB (Enter alone: all {})", size_text(room * 512)),
            Ask::AddName { .. } => "Partition name (may be empty)".into(),
            Ask::AddType { .. } => {
                let mut t = String::from("Type (Enter alone: 1):");
                for (i, (_, name)) in TYPES.iter().enumerate() { t.push_str(&format!("  {}: {}", i + 1, name)); }
                t
            }
            Ask::Resize { .. } => "New size in MiB".into(),
            Ask::FormatFs { .. } | Ask::EncryptFs { .. } => format!(
                "Filesystem: 1 FAT32  2 exFAT  3 sexfs{}",
                if matches!(self, Ask::EncryptFs { .. }) { "  0 none inside" } else { "" }
            ),
            Ask::FormatLabel { .. } | Ask::EncryptLabel { .. } => "Label (up to 11 letters, may be empty)".into(),
            Ask::Password { .. } => "Password for the encrypted partition".into(),
            Ask::PasswordAgain { .. } => "The same password again".into(),
            Ask::Confirm => "Type the disk's name to apply".into(),
        }
    }
}

/// One line of the disk view: a partition or a run of free space.
#[derive(Clone)]
enum Row { Part(Part), Free { first: u64, sectors: u64 } }
impl Row { fn first(&self) -> u64 { match self { Row::Part(p) => p.first, Row::Free { first, .. } => *first } } }

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiskTab {
    All,
    Removable,
    Shielded,
    Audit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    Tab(DiskTab),
    Refresh,
    SelectDisk(usize),
    OpenDisk(usize),
    BackToDisks,
    SelectRow(usize),
    CmdNewGpt,
    CmdAdd,
    CmdFormat,
    CmdResize,
    CmdEncrypt,
    CmdDelete,
    CmdUndo,
    CmdReview,
    ConfirmApply,
    CancelReview,
    SubmitAsk,
    CancelAsk,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

struct AuditEntry {
    kind: &'static str,
    ok: bool,
    detail: String,
}

struct Open { info: Info, plan: Plan, cursor: usize, scroll: usize, review: bool, leaving: bool }

struct App {
    disks: Vec<Info>,
    cursor: usize,
    scroll: usize,
    tab: DiskTab,
    open: Option<Open>,
    ask: Option<(Ask, Vec<u8>)>,
    status: String,
    audit: Vec<AuditEntry>,
    hits: Vec<HitBox>,
    mouse: (i32, i32),
    mouse_down: bool,
    hovered: Option<usize>,
    painter: NoirPainter,
}

fn rows(plan: &Plan) -> Vec<Row> {
    let mut v: Vec<Row> = plan.partitions().into_iter().map(Row::Part).collect();
    v.extend(plan.free().into_iter().map(|(first, sectors)| Row::Free { first, sectors }));
    v.sort_by_key(Row::first);
    v
}
fn mib(text: &[u8]) -> Option<u64> {
    let s = core::str::from_utf8(text).ok()?.trim();
    let n: u64 = s.parse().ok()?;
    n.checked_mul(MIB).filter(|v| *v > 0)
}
fn fs_choice(text: &[u8], none: bool) -> Option<Option<Fs>> {
    match text.first().copied().unwrap_or(b' ') {
        b'1' => Some(Some(Fs::Fat32)), b'2' => Some(Some(Fs::ExFat)), b'3' => Some(Some(Fs::Sexfs)),
        b'0' if none => Some(None),
        _ => None,
    }
}
fn type_choice(text: &[u8]) -> Option<corset_core::Guid> {
    let n = match core::str::from_utf8(text).ok()?.trim() { "" => 1, t => t.parse::<usize>().ok()? };
    TYPES.get(n.checked_sub(1)?).map(|(g, _)| *g)
}
fn text(b: &[u8]) -> String { String::from_utf8_lossy(b).into_owned() }

fn part_color(slot: usize) -> u32 {
    const PAL: [u32; 5] = [ROSE, EMERALD, SKY, AMBER, 0x00A7_8BFA];
    PAL[slot % PAL.len()]
}

impl App {
    fn log_audit(&mut self, kind: &'static str, ok: bool, detail: String) {
        if self.audit.len() >= 48 {
            self.audit.remove(0);
        }
        self.audit.push(AuditEntry { kind, ok, detail });
    }

    fn filtered_disk_indices(&self) -> Vec<usize> {
        self.disks
            .iter()
            .enumerate()
            .filter(|(_, d)| match self.tab {
                DiskTab::All => true,
                DiskTab::Removable => !d.system(),
                DiskTab::Shielded => d.system() || d.read_only(),
                DiskTab::Audit => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn refresh(&mut self, window: &mut Window) {
        match disks::list(window) {
            Ok(list) => {
                let names: Vec<String> = list.iter().map(|d| format!("{}:{}:{}", d.name, d.sectors, d.flags)).collect();
                sex_pdx::serial_record!("[corset.disks] count={} list={}", list.len(), names.join(","));
                self.log_audit("SCAN", true, format!("Enumerated {} storage device(s)", list.len()));
                self.disks = list;
                // Start on the first disk Corset may change.
                if self.cursor >= self.disks.len() || self.disks.get(self.cursor).is_some_and(|d| d.system()) {
                    self.cursor = self.disks.iter().position(|d| !d.system()).unwrap_or(0);
                }
            }
            Err(e) => {
                let msg = format!("Files didn't list the disks: {}.", text(w::message(e)));
                self.log_audit("ERROR", false, msg.clone());
                self.status = msg;
            }
        }
    }

    /// Asks Files for the disk (the person answers there) and reads what it holds.
    fn open_disk(&mut self, window: &mut Window) {
        let Some(info) = self.disks.get(self.cursor).cloned() else { return };
        if info.system() {
            sex_pdx::serial_record!("[corset.refused] disk={} why=system", info.name);
            self.log_audit("BLOCKED", false, format!("System disk {} shielded: raw access refused", info.name));
            self.status = format!("SexOS runs from {}: Corset never changes it.", info.name);
            return;
        }
        let mut asked = false;
        let mut cancel_box: Option<(i32, i32, i32, i32)> = None;
        loop {
            let mut first = [0u8; disks::SECTOR];
            match disks::read(window, info.id, 0, &mut first) {
                Ok(()) => break,
                Err(w::PENDING) => {
                    if !asked {
                        asked = true;
                        sex_pdx::serial_record!("[corset.waiting] disk={}", info.name);
                        cancel_box = Some(self.render_waiting(window, &info));
                    }
                    match window.next_within(150) {
                        Some(Event::Close) => window_close(None),
                        Some(Event::Resize { .. }) => { cancel_box = Some(self.render_waiting(window, &info)); }
                        Some(Event::Key(k)) if k.code == ESC => {
                            self.status = "Stopped waiting.".into();
                            self.log_audit("CANCEL", false, format!("Cancelled grant request for {}", info.name));
                            return;
                        }
                        Some(Event::Pointer(ptr)) => {
                            if let PointerKind::Up { button: 0 } = ptr.kind {
                                if let Some((bx, by, bw, bh)) = cancel_box {
                                    let (mx, my) = (ptr.x as i32, ptr.y as i32);
                                    if mx >= bx && mx < bx + bw && my >= by && my < by + bh {
                                        self.status = "Stopped waiting.".into();
                                        self.log_audit("CANCEL", false, format!("Cancelled grant request for {}", info.name));
                                        return;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    sex_pdx::serial_record!("[corset.denied] disk={} status={}", info.name, e);
                    self.log_audit("DENIED", false, format!("Files denied {} (status {})", info.name, e));
                    self.status = format!("Files didn't give {}: {}.", info.name, text(w::message(e)));
                    return;
                }
            }
        }
        let status = Cell::new(0);
        let window = RefCell::new(window);
        let read = Disk::read(&mut dev::Raw { window: &window, id: info.id, sectors: info.sectors, status: &status }, &info.name);
        match read {
            Ok(disk) => {
                sex_pdx::serial_record!("[corset.opened] disk={} sectors={} table={} parts={} system={}", info.name, disk.sectors, disk.table.is_some() as u8, disk.parts.len(), disk.is_system() as u8);
                self.log_audit("GRANT", true, format!("Files granted {} ({} sectors, {} part(s))", info.name, disk.sectors, disk.parts.len()));
                if disk.is_system() { self.status = "This disk holds a SexOS system: Corset leaves its system partitions alone.".into(); }
                self.open = Some(Open { info, plan: Plan::new(disk), cursor: 0, scroll: 0, review: false, leaving: false });
            }
            Err(e) => {
                self.status = format!("Couldn't read {}: {} ({}).", info.name, e, text(w::message(status.get())));
                self.log_audit("ERROR", false, self.status.clone());
                let _ = disks::done(&mut window.borrow_mut(), info.id);
            }
        }
    }

    fn push(&mut self, op: Op) -> bool {
        let Some(o) = self.open.as_mut() else { return false };
        match o.plan.push(op) {
            Ok(slot) => {
                let step = o.plan.steps().last().unwrap().clone();
                sex_pdx::serial_record!("[corset.planned] steps={} destructive={} text={}", o.plan.steps().len(), step.destructive as u8, step.text);
                if let Some(i) = rows(&o.plan).iter().position(|r| matches!(r, Row::Part(p) if p.slot == slot)) { o.cursor = i; }
                self.status.clear();
                self.log_audit("PLAN", true, step.text);
                true
            }
            Err(r) => { self.refused(r); false }
        }
    }
    fn refused(&mut self, r: Refusal) {
        let why = r.text();
        sex_pdx::serial_record!("[corset.refused] why={}", why);
        self.log_audit("REFUSED", false, format!("Refused plan: {}", why));
        self.status = format!("Not planned: {}", why);
    }
    fn selected(&self) -> Option<Row> { self.open.as_ref().and_then(|o| rows(&o.plan).get(o.cursor).cloned()) }
    fn selected_slot(&mut self) -> Option<usize> {
        match self.selected() { Some(Row::Part(p)) => Some(p.slot), _ => { self.status = "Choose a partition first (Up and Down).".into(); None } }
    }

    /// The line typed at the bottom is done.
    fn answer(&mut self, window: &mut Window, ask: Ask, typed: Vec<u8>) {
        match ask {
            Ask::AddSize { first, room } => {
                let sectors = if typed.is_empty() { room } else { match mib(&typed) { Some(s) => s, None => { self.status = "That isn't a size in MiB.".into(); return; } } };
                self.ask = Some((Ask::AddName { first, sectors }, Vec::new()));
            }
            Ask::AddName { first, sectors } => self.ask = Some((Ask::AddType { first, sectors, name: text(&typed) }, Vec::new())),
            Ask::AddType { first, sectors, name } => match type_choice(&typed) {
                Some(type_guid) => { self.push(Op::Add { first, sectors, type_guid, name }); }
                None => self.status = "Choose a type by its number.".into(),
            },
            Ask::Resize { slot } => match mib(&typed) {
                Some(sectors) => { self.push(Op::Resize { slot, sectors }); }
                None => self.status = "That isn't a size in MiB.".into(),
            },
            Ask::FormatFs { slot } => match fs_choice(&typed, false) {
                Some(Some(Fs::Sexfs)) => { self.push(Op::Format { slot, fs: Fs::Sexfs, label: String::new() }); }
                Some(Some(fs)) => self.ask = Some((Ask::FormatLabel { slot, fs }, Vec::new())),
                _ => self.status = "Choose a filesystem by its number.".into(),
            },
            Ask::FormatLabel { slot, fs } => { self.push(Op::Format { slot, fs, label: text(&typed) }); }
            Ask::Password { slot } => self.ask = Some((Ask::PasswordAgain { slot, first: Secret(typed) }, Vec::new())),
            Ask::PasswordAgain { slot, first } => {
                if first.0 != typed { self.status = "The two passwords differ: nothing planned.".into(); return; }
                self.ask = Some((Ask::EncryptFs { slot, password: first }, Vec::new()));
                let mut typed = typed; typed.fill(0);
            }
            Ask::EncryptFs { slot, password } => match fs_choice(&typed, true) {
                Some(None) => { self.push(Op::Encrypt { slot, password, inner: None }); }
                Some(Some(Fs::Sexfs)) => { self.push(Op::Encrypt { slot, password, inner: Some((Fs::Sexfs, String::new())) }); }
                Some(Some(fs)) => self.ask = Some((Ask::EncryptLabel { slot, password, fs }, Vec::new())),
                None => self.status = "Choose a filesystem by its number.".into(),
            },
            Ask::EncryptLabel { slot, password, fs } => { self.push(Op::Encrypt { slot, password, inner: Some((fs, text(&typed))) }); }
            Ask::Confirm => {
                let o = self.open.as_ref().unwrap();
                if o.plan.confirms(&text(&typed)) { self.apply(window); } else {
                    sex_pdx::serial_record!("[corset.confirm.wrong]");
                    self.status = format!("That isn't \"{}\": nothing was written.", o.info.name);
                }
            }
        }
    }

    fn review(&mut self) {
        let Some(o) = self.open.as_mut() else { return };
        if o.plan.is_empty() { self.status = "Nothing planned yet.".into(); return; }
        o.review = true;
        sex_pdx::serial_record!("[corset.review] disk={} steps={} destructive={}", o.info.name, o.plan.steps().len(), o.plan.destructive() as u8);
        for (i, s) in o.plan.steps().iter().enumerate() { sex_pdx::serial_record!("[corset.step] n={} destructive={} text={}", i + 1, s.destructive as u8, s.text); }
    }

    fn apply(&mut self, window: &mut Window) {
        let Some(o) = self.open.take() else { return };
        let Some(mut random) = dev::random() else {
            self.status = "This computer has no random number generator Corset can use: nothing was written.".into();
            self.open = Some(o);
            return;
        };
        let (name, id) = (o.info.name.clone(), o.info.id);
        sex_pdx::serial_record!("[corset.apply.start] disk={} steps={}", name, o.plan.steps().len());
        let status = Cell::new(0);
        let cell = RefCell::new(window);
        let painter = RefCell::new(&mut self.painter);
        let mut shown = (0usize, u64::MAX);
        let result = {
            let mut device = dev::Raw { window: &cell, id, sectors: o.info.sectors, status: &status };
            plan::apply(&mut device, &o.plan, &mut *random, &mut |p| {
                let pct = p.pct;
                if (p.step, pct / 10) == shown { return; }
                shown = (p.step, pct / 10);
                sex_pdx::serial_record!("[corset.progress] step={} steps={} pct={}", p.step, p.steps, pct);
                render_progress(&mut painter.borrow_mut(), &mut cell.borrow_mut(), &name, p.step, p.steps, pct, p.text);
            })
        };
        let window = cell.into_inner();
        match result {
            Ok(r) => {
                sex_pdx::serial_record!("[corset.applied] disk={} table={} before={} after={} parts={}", name, r.table_written as u8, r.before_table, r.after_table, r.disk.parts.len());
                self.status = format!("Applied {} change{} to {}. Files mounts its volumes again.", o.plan.steps().len(), if o.plan.steps().len() == 1 { "" } else { "s" }, name);
                self.log_audit("APPLIED", true, self.status.clone());
            }
            Err(ApplyError::Changed) => {
                sex_pdx::serial_record!("[corset.apply.failed] disk={} error=changed", name);
                self.status = format!("{} changed since Corset read it: nothing was written. Open it again to plan on what it holds now.", name);
                self.log_audit("FAILED", false, self.status.clone());
            }
            Err(ApplyError::Engine(e)) => {
                sex_pdx::serial_record!("[corset.apply.failed] disk={} error={} status={}", name, e, status.get());
                self.status = format!("Stopped: {} ({}). The old partition table stays until its write; open the disk again to see what it holds.", e, text(w::message(status.get())));
                self.log_audit("FAILED", false, self.status.clone());
            }
        }
        self.done(window, id, &name);
        self.refresh(window);
    }
    /// Gives the disk back to Files, which mounts what it holds now.
    fn done(&mut self, window: &mut Window, id: u16, name: &str) {
        match disks::done(window, id) {
            Ok(()) => sex_pdx::serial_record!("[corset.done] disk={}", name),
            Err(e) => sex_pdx::serial_record!("[corset.done.failed] disk={} status={}", name, e),
        }
    }

    fn trigger_add(&mut self) {
        let Some(o) = self.open.as_ref() else { return };
        let has_table = o.plan.has_table();
        let free = o.plan.free();
        let selected = self.selected();
        let at = match selected { Some(Row::Free { first, sectors }) => Some((first, sectors)), _ => free.iter().copied().max_by_key(|f| f.1) };
        match at {
            Some((first, room)) => self.ask = Some((Ask::AddSize { first, room }, Vec::new())),
            None if !has_table => self.status = "The disk has no partition table: New GPT makes one.".into(),
            None => self.status = "No free space left.".into(),
        }
    }

    fn trigger_undo(&mut self) {
        let Some(o) = self.open.as_mut() else { return };
        if o.plan.pop().is_some() {
            let rem = o.plan.steps().len();
            sex_pdx::serial_record!("[corset.undone] steps={}", rem);
            o.cursor = o.cursor.min(rows(&o.plan).len().saturating_sub(1));
            self.log_audit("UNDO", true, format!("Undid step ({} remaining)", rem));
        } else {
            self.status = "Nothing to undo.".into();
        }
    }

    fn leave_open_disk(&mut self, window: &mut Window) {
        let Some(o) = self.open.as_ref() else { return };
        let steps = o.plan.steps().len();
        let leaving = o.leaving;
        if steps > 0 && !leaving {
            self.open.as_mut().unwrap().leaving = true;
            self.status = format!("Click Back or Esc again to drop {} planned change{}.", steps, if steps == 1 { "" } else { "s" });
            return;
        }
        let o = self.open.take().unwrap();
        self.done(window, o.info.id, &o.info.name);
        self.refresh(window);
    }

    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        if let Some((ask, typed)) = self.ask.as_mut() {
            match key.code {
                ENTER => { let (ask, typed) = self.ask.take().unwrap(); self.answer(window, ask, typed); }
                ESC => { if let Some((_, mut t)) = self.ask.take() { t.fill(0); } }
                BACKSPACE => { while let Some(b) = typed.pop() { if b & 0xC0 != 0x80 { break; } } }
                _ => if let Some(c) = sex_app::text(key) {
                    if typed.len() + c.len_utf8() <= 64 { typed.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()); }
                    if !ask.secret() { sex_pdx::serial_record!("[corset.input] len={}", typed.len()); }
                },
            }
            return;
        }
        if self.open.is_none() {
            match (key.code, sex_app::nav(key)) {
                (_, Some(Nav::Up)) => self.cursor = self.cursor.saturating_sub(1),
                (_, Some(Nav::Down)) => self.cursor = (self.cursor + 1).min(self.disks.len().saturating_sub(1)),
                (ENTER, _) => self.open_disk(window),
                (F5, _) => self.refresh(window),
                _ => {}
            }
            if let Some(Nav::Up | Nav::Down) = sex_app::nav(key) { sex_pdx::serial_record!("[corset.cursor] disk={}", self.cursor); }
            return;
        }
        let (review, destructive) = {
            let o = self.open.as_ref().unwrap();
            (o.review, o.plan.destructive())
        };
        if review {
            match key.code {
                ENTER if destructive => self.ask = Some((Ask::Confirm, Vec::new())),
                ENTER => self.apply(window),
                ESC => self.open.as_mut().unwrap().review = false,
                _ => {}
            }
            return;
        }
        let o = self.open.as_mut().unwrap();
        if let Some(n) = sex_app::nav(key) {
            let count = rows(&o.plan).len();
            match n { Nav::Up => o.cursor = o.cursor.saturating_sub(1), Nav::Down => o.cursor = (o.cursor + 1).min(count.saturating_sub(1)), _ => {} }
            o.leaving = false;
            sex_pdx::serial_record!("[corset.cursor] row={}", o.cursor);
            return;
        }
        if key.code != ESC { o.leaving = false; }
        self.status.clear();
        match key.code {
            KEY_G => { self.push(Op::CreateTable); }
            KEY_A => self.trigger_add(),
            KEY_D => if let Some(slot) = self.selected_slot() { self.push(Op::Delete { slot }); },
            KEY_R => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::Resize { slot }, Vec::new())); },
            KEY_F => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::FormatFs { slot }, Vec::new())); },
            KEY_E => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::Password { slot }, Vec::new())); },
            KEY_U => self.trigger_undo(),
            KEY_H => self.status = "Health: not available yet. The disk drivers don't pass SMART or NVMe health pages to programs.".into(),
            ENTER => self.review(),
            ESC => self.leave_open_disk(window),
            _ => {}
        }
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.hits.iter().rposition(|h| x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)
    }

    fn pointer(&mut self, window: &mut Window, ptr: sex_pdx::app_pointer::Pointer) -> bool {
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
                    self.dispatch_hit(window, action);
                }
                true
            }
            PointerKind::Wheel { steps } => {
                let delta = -(steps as isize);
                if let Some(o) = self.open.as_mut() {
                    let max_scroll = rows(&o.plan).len().saturating_sub(1);
                    o.scroll = (o.scroll as isize + delta).clamp(0, max_scroll as isize) as usize;
                } else {
                    let max_scroll = if self.tab == DiskTab::Audit {
                        self.audit.len().saturating_sub(1)
                    } else {
                        self.disks.len().saturating_sub(1)
                    };
                    self.scroll = (self.scroll as isize + delta).clamp(0, max_scroll as isize) as usize;
                }
                true
            }
            _ => false,
        }
    }

    fn dispatch_hit(&mut self, window: &mut Window, action: HitAction) {
        match action {
            HitAction::Tab(t) => { self.tab = t; self.scroll = 0; }
            HitAction::Refresh => self.refresh(window),
            HitAction::SelectDisk(idx) => {
                self.cursor = idx;
                sex_pdx::serial_record!("[corset.cursor] disk={}", self.cursor);
            }
            HitAction::OpenDisk(idx) => {
                self.cursor = idx;
                self.open_disk(window);
            }
            HitAction::BackToDisks => self.leave_open_disk(window),
            HitAction::SelectRow(idx) => {
                if let Some(o) = self.open.as_mut() {
                    o.cursor = idx;
                    sex_pdx::serial_record!("[corset.cursor] row={}", o.cursor);
                }
            }
            HitAction::CmdNewGpt => { self.push(Op::CreateTable); }
            HitAction::CmdAdd => self.trigger_add(),
            HitAction::CmdFormat => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::FormatFs { slot }, Vec::new())); },
            HitAction::CmdResize => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::Resize { slot }, Vec::new())); },
            HitAction::CmdEncrypt => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::Password { slot }, Vec::new())); },
            HitAction::CmdDelete => if let Some(slot) = self.selected_slot() { self.push(Op::Delete { slot }); },
            HitAction::CmdUndo => self.trigger_undo(),
            HitAction::CmdReview => self.review(),
            HitAction::ConfirmApply => {
                let destructive = self.open.as_ref().is_some_and(|o| o.plan.destructive());
                if destructive { self.ask = Some((Ask::Confirm, Vec::new())); } else { self.apply(window); }
            }
            HitAction::CancelReview => if let Some(o) = self.open.as_mut() { o.review = false; },
            HitAction::SubmitAsk => {
                if let Some((ask, typed)) = self.ask.take() {
                    self.answer(window, ask, typed);
                }
            }
            HitAction::CancelAsk => {
                if let Some((_, mut t)) = self.ask.take() { t.fill(0); }
            }
        }
    }

    fn btn_state(&self, x: i32, y: i32, w: i32, h: i32) -> ButtonState {
        let (mx, my) = self.mouse;
        if mx >= x && mx < x + w && my >= y && my < y + h {
            if self.mouse_down { ButtonState::Pressed } else { ButtonState::Hover }
        } else {
            ButtonState::Rest
        }
    }

    fn push_button(
        &mut self,
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
        } else {
            self.btn_state(x, y, w, h)
        };
        self.painter.button(buf, bw, bh, x, y, w, h, label, style, st);
        if !matches!(style, ButtonStyle::Disabled) {
            self.hits.push(HitBox { x, y, w, h, action });
        }
    }

    fn render_waiting(&mut self, window: &mut Window, info: &Info) -> (i32, i32, i32, i32) {
        let (w, h) = window_dims(window);
        let buf = window.frame(w, h);
        buf.fill(BG);
        fill_rect(buf, w, h, 0, 0, w as i32, 48, HEADER);
        fill_rect(buf, w, h, 0, 47, w as i32, 1, BORDER);
        self.painter.text(buf, w, h, false, 18, 13, 180, 15, "CORSET", TEXT);
        self.painter.pill(buf, w, h, 96, 12, 22, false, "Awaiting Grant", AMBER, AMBER_BG, AMBER_BORDER);

        let cw = (w as i32 - 64).clamp(280, 520);
        let ch = 186;
        let cx = (w as i32 - cw) / 2;
        let cy = ((h as i32 - ch) / 2).max(60);
        draw_card(buf, w, h, cx, cy, cw, ch, 12, CARD, BORDER_ACTIVE);
        self.painter.pill(buf, w, h, cx + 22, cy + 20, 22, false, "Capability Request", ROSE, ROSE_BG, ROSE_BORDER);
        self.painter.text(
            buf, w, h, false, cx + 22, cy + 54, cw - 44, 15,
            &format!("Files is asking permission for {}", info.name), TEXT,
        );
        self.painter.text(
            buf, w, h, true, cx + 22, cy + 80, cw - 44, 12,
            &format!("{}  ·  {}", info.product, size_text(info.sectors * 512)), MUTED,
        );
        self.painter.text(
            buf, w, h, false, cx + 22, cy + 106, cw - 44, 12,
            "Switch to Files and press Y to allow, or N to refuse.", MUTED,
        );
        let bx = cx + cw - 122;
        let by = cy + ch - 44;
        self.painter.button(buf, w, h, bx, by, 100, 28, "Cancel (Esc)", ButtonStyle::Normal, ButtonState::Rest);
        window.show_pixels(w, h);
        (bx, by, 100, 28)
    }

    fn render(&mut self, window: &mut Window) {
        self.hits.clear();
        let (w, h) = window_dims(window);
        let iw = w as i32;
        let ih = h as i32;
        let protected_cnt = self.disks.iter().filter(|d| d.system() || d.read_only()).count();
        let writable_cnt = self.disks.iter().filter(|d| !d.system() && !d.read_only()).count();

        let buf = window.frame(w, h);
        buf.fill(BG);

        // Top Header Toolbar (48px)
        fill_rect(buf, w, h, 0, 0, iw, 48, HEADER);
        fill_rect(buf, w, h, 0, 47, iw, 1, BORDER);
        self.painter.text(buf, w, h, false, 18, 14, 100, 15, "CORSET", TEXT);
        let px = 92 + self.painter.pill(buf, w, h, 92, 13, 22, false, "Shields Active", EMERALD, EMERALD_BG, EMERALD_BORDER) + 8;
        if iw >= 560 {
            let p2 = self.painter.pill(
                buf, w, h, px, 13, 22, true,
                &format!("{} Protected · {} Writable", protected_cnt, writable_cnt),
                MUTED, CARD, BORDER,
            );
            let _ = p2;
        }
        if self.open.is_some() {
            self.push_button(buf, w, h, iw - 122, 10, 104, 28, "Disks (Esc)", ButtonStyle::Normal, HitAction::BackToDisks);
        } else {
            self.push_button(buf, w, h, iw - 122, 10, 104, 28, "Refresh (F5)", ButtonStyle::Normal, HitAction::Refresh);
        }

        let mut body_y = 58;
        let bottom_reserve = if self.ask.is_some() { 114 } else if !self.status.is_empty() { 38 } else { 18 };
        let body_bottom = ih - bottom_reserve;

        if self.open.is_none() {
            // Filter Tabs Bar
            let tabs = [
                (DiskTab::All, "All Devices"),
                (DiskTab::Removable, "Writable Grants"),
                (DiskTab::Shielded, "System Shield"),
                (DiskTab::Audit, "Live Audit"),
            ];
            let mut tx = 18;
            for (tab, label) in tabs {
                let tw = (self.painter.measure(false, 12, label) + 24).max(86);
                if tx + tw > iw - 14 { break; }
                let style = if self.tab == tab { ButtonStyle::Active } else { ButtonStyle::Subtle };
                self.push_button(buf, w, h, tx, body_y, tw, 28, label, style, HitAction::Tab(tab));
                tx += tw + 8;
            }
            body_y += 40;

            if self.tab == DiskTab::Audit {
                draw_card(buf, w, h, 18, body_y, iw - 36, (body_bottom - body_y).max(80), 10, CARD, BORDER);
                self.painter.text(buf, w, h, false, 32, body_y + 12, iw - 80, 13, "Capability & Write-Shield Audit Log", TEXT);
                let mut ay = body_y + 38;
                let max_items = ((body_bottom - ay - 10) / 28).max(1) as usize;
                let start = self.scroll.min(self.audit.len().saturating_sub(1));
                for entry in self.audit.iter().rev().skip(start).take(max_items) {
                    let (fg, bg, brd) = if entry.ok { (EMERALD, EMERALD_BG, EMERALD_BORDER) } else { (ROSE, ROSE_BG, ROSE_BORDER) };
                    let pw = self.painter.pill(buf, w, h, 32, ay, 20, true, entry.kind, fg, bg, brd);
                    self.painter.text(buf, w, h, true, 42 + pw, ay + 2, iw - 76 - pw, 12, &entry.detail, MUTED);
                    ay += 28;
                }
                if self.audit.is_empty() {
                    self.painter.text(buf, w, h, false, 32, ay, iw - 80, 12, "No security or disk events recorded yet.", DIM);
                }
            } else {
                let indices = self.filtered_disk_indices();
                if indices.is_empty() {
                    draw_card(buf, w, h, 18, body_y, iw - 36, 92, 10, CARD, BORDER);
                    self.painter.text(buf, w, h, false, 34, body_y + 22, iw - 68, 14, "No matching storage devices", TEXT);
                    self.painter.text(buf, w, h, false, 34, body_y + 46, iw - 68, 12, "Plug in a USB disk or click Refresh (F5) to scan again.", MUTED);
                } else {
                    let card_h = 74;
                    let gap = 10;
                    let visible = ((body_bottom - body_y) / (card_h + gap)).max(1) as usize;
                    let start = self.scroll.min(indices.len().saturating_sub(1));
                    for &idx in indices.iter().skip(start).take(visible) {
                        let d = self.disks[idx].clone();
                        let selected = idx == self.cursor;
                        let border = if selected { ROSE } else { BORDER };
                        let fill = if selected { CARD_ALT } else { CARD };
                        draw_card(buf, w, h, 18, body_y, iw - 36, card_h, 10, fill, border);
                        self.hits.push(HitBox { x: 18, y: body_y, w: iw - 36, h: card_h, action: HitAction::SelectDisk(idx) });

                        self.painter.text(buf, w, h, true, 34, body_y + 13, 180, 15, &d.name, TEXT);
                        self.painter.text(buf, w, h, false, 34, body_y + 38, iw - 300, 12, &format!("{}  ·  {} sectors", d.product, d.sectors), MUTED);

                        let sz = size_text(d.sectors * 512);
                        let sz_w = self.painter.pill(buf, w, h, (iw - 295).max(160), body_y + 12, 22, true, &sz, SKY, SKY_BG, SKY_BORDER);
                        let badge_x = (iw - 295).max(160) + sz_w + 8;
                        if d.system() {
                            self.painter.pill(buf, w, h, badge_x, body_y + 12, 22, false, "System Shield", ROSE, ROSE_BG, ROSE_BORDER);
                            self.push_button(buf, w, h, iw - 148, body_y + 38, 114, 26, "Protected", ButtonStyle::Disabled, HitAction::SelectDisk(idx));
                        } else if d.read_only() {
                            self.painter.pill(buf, w, h, badge_x, body_y + 12, 22, false, "Read-Only", AMBER, AMBER_BG, AMBER_BORDER);
                            self.push_button(buf, w, h, iw - 148, body_y + 38, 114, 26, "Inspect", ButtonStyle::Normal, HitAction::OpenDisk(idx));
                        } else {
                            self.painter.pill(buf, w, h, badge_x, body_y + 12, 22, false, "Files Grant", EMERALD, EMERALD_BG, EMERALD_BORDER);
                            self.push_button(buf, w, h, iw - 148, body_y + 38, 114, 26, "Open Disk", ButtonStyle::Primary, HitAction::OpenDisk(idx));
                        }
                        body_y += card_h + gap;
                    }
                }
            }
        } else if self.open.as_ref().is_some_and(|o| o.review) {
            let (confirm_text, destructive, disk_name) = {
                let o = self.open.as_ref().unwrap();
                (o.plan.confirmation(), o.plan.destructive(), o.info.name.clone())
            };
            let ch = (body_bottom - body_y).max(150);
            draw_card(buf, w, h, 18, body_y, iw - 36, ch, 12, CARD, if destructive { ROSE } else { BORDER_ACTIVE });
            self.painter.text(buf, w, h, false, 34, body_y + 16, iw - 240, 15, &format!("Review Changes to {}", disk_name), TEXT);
            if destructive {
                self.painter.pill(buf, w, h, iw - 160, body_y + 14, 22, false, "Destructive", ROSE, ROSE_BG, ROSE_BORDER);
            } else {
                self.painter.pill(buf, w, h, iw - 140, body_y + 14, 22, false, "Non-Destructive", EMERALD, EMERALD_BG, EMERALD_BORDER);
            }
            let mut ly = body_y + 50;
            for line in confirm_text.lines() {
                if ly > body_bottom - 56 { break; }
                self.painter.text(buf, w, h, true, 34, ly, iw - 72, 12, line, MUTED);
                ly += 19;
            }
            self.push_button(buf, w, h, 34, body_bottom - 42, 152, 28, "Apply Changes (Enter)", if destructive { ButtonStyle::Danger } else { ButtonStyle::Primary }, HitAction::ConfirmApply);
            self.push_button(buf, w, h, 196, body_bottom - 42, 108, 28, "Back (Esc)", ButtonStyle::Normal, HitAction::CancelReview);
        } else {
            // Open disk partition editor
            let (disk_name, disk_product, disk_sectors, has_table, part_list, free_list, row_list, plan_steps, o_cursor, o_scroll) = {
                let o = self.open.as_ref().unwrap();
                (
                    o.info.name.clone(),
                    o.info.product.clone(),
                    o.info.sectors,
                    o.plan.has_table(),
                    o.plan.partitions(),
                    o.plan.free(),
                    rows(&o.plan),
                    o.plan.steps().to_vec(),
                    o.cursor,
                    o.scroll,
                )
            };

            // Disk Summary & Segmented Partition Map Card
            draw_card(buf, w, h, 18, body_y, iw - 36, 78, 10, CARD, BORDER);
            self.painter.text(buf, w, h, true, 32, body_y + 11, 120, 14, &disk_name, TEXT);
            self.painter.text(buf, w, h, false, 118, body_y + 12, iw - 320, 12, &format!("{}  ·  {}", disk_product, size_text(disk_sectors * 512)), MUTED);
            let tbl_label = if has_table { "GPT" } else { "No Table" };
            let (tfg, tbg, tbrd) = if has_table { (EMERALD, EMERALD_BG, EMERALD_BORDER) } else { (AMBER, AMBER_BG, AMBER_BORDER) };
            self.painter.pill(buf, w, h, iw - 122, body_y + 10, 22, true, tbl_label, tfg, tbg, tbrd);

            // Segmented Partition Bar
            let bar_x = 32;
            let bar_y = body_y + 42;
            let bar_w = (iw - 64).max(80);
            let bar_h = 22;
            fill_rounded_rect(buf, w, h, bar_x, bar_y, bar_w, bar_h, 6, WELL);
            let total_sec = disk_sectors.max(1);
            for p in &part_list {
                let x0 = bar_x + ((p.first as u128 * bar_w as u128) / total_sec as u128) as i32;
                let pw = (((p.sectors as u128 * bar_w as u128) / total_sec as u128) as i32).max(6);
                fill_rounded_rect(buf, w, h, x0, bar_y + 2, pw.min(bar_x + bar_w - x0), bar_h - 4, 4, part_color(p.slot));
            }
            let _ = free_list;
            body_y += 86;

            // Action Toolbar
            let cmds: [(&str, ButtonStyle, HitAction); 8] = [
                ("GPT (G)", ButtonStyle::Normal, HitAction::CmdNewGpt),
                ("+ Add (A)", ButtonStyle::Primary, HitAction::CmdAdd),
                ("Format (F)", ButtonStyle::Normal, HitAction::CmdFormat),
                ("Resize (R)", ButtonStyle::Normal, HitAction::CmdResize),
                ("Encrypt (E)", ButtonStyle::Normal, HitAction::CmdEncrypt),
                ("Delete (D)", ButtonStyle::Danger, HitAction::CmdDelete),
                ("Undo (U)", if plan_steps.is_empty() { ButtonStyle::Disabled } else { ButtonStyle::Normal }, HitAction::CmdUndo),
                ("Review (Ret)", if plan_steps.is_empty() { ButtonStyle::Disabled } else { ButtonStyle::Active }, HitAction::CmdReview),
            ];
            let mut bx = 18;
            for (lbl, st, act) in cmds {
                let bw = (self.painter.measure(false, 11, lbl) + 18).max(64);
                if bx + bw > iw - 14 { break; }
                self.push_button(buf, w, h, bx, body_y, bw, 26, lbl, st, act);
                bx += bw + 6;
            }
            body_y += 34;

            let queue_h = if plan_steps.is_empty() { 0 } else { (26 + plan_steps.len().min(3) as i32 * 18 + 10).min(96) };
            let list_h = (body_bottom - body_y - if queue_h > 0 { queue_h + 8 } else { 0 }).max(70);

            // Partition & Free Space Rows Card
            draw_card(buf, w, h, 18, body_y, iw - 36, list_h, 10, CARD, BORDER);
            self.painter.text(buf, w, h, true, 32, body_y + 9, iw - 64, 11, "#   NAME             TYPE                    SIZE         START        HOLDS", DIM);
            fill_rect(buf, w, h, 26, body_y + 26, iw - 52, 1, BORDER);
            let mut ry = body_y + 32;
            let row_h = 26;
            let max_rows = ((list_h - 38) / row_h).max(1) as usize;
            let start = o_scroll.min(row_list.len().saturating_sub(1));
            if row_list.is_empty() {
                let hint = if has_table { "No partitions or free space." } else { "No partition table. Click GPT (G) to create one." };
                self.painter.text(buf, w, h, false, 32, ry + 6, iw - 64, 12, hint, MUTED);
            } else {
                for (i, r) in row_list.iter().enumerate().skip(start).take(max_rows) {
                    let is_sel = i == o_cursor;
                    if is_sel {
                        fill_rounded_rect(buf, w, h, 26, ry, iw - 52, row_h - 2, 6, 0x002B_1521);
                    }
                    self.hits.push(HitBox { x: 26, y: ry, w: iw - 52, h: row_h - 2, action: HitAction::SelectRow(i) });
                    match r {
                        Row::Part(p) => {
                            fill_rounded_rect(buf, w, h, 32, ry + 6, 8, 12, 3, part_color(p.slot));
                            let line = format!(
                                "{:<3} {:<16} {:<23} {:<12} {:<12} {}",
                                p.slot + 1,
                                p.name.chars().take(15).collect::<String>(),
                                p.type_name().chars().take(22).collect::<String>(),
                                size_text(p.bytes()),
                                size_text(p.first * 512),
                                p.content.name()
                            );
                            self.painter.text(buf, w, h, true, 46, ry + 5, iw - 82, 12, &line, if is_sel { TEXT } else { MUTED });
                        }
                        Row::Free { first, sectors } => {
                            let line = format!(
                                ".   {:<16} {:<23} {:<12} {}",
                                "free space",
                                "",
                                size_text(*sectors * 512),
                                size_text(*first * 512)
                            );
                            self.painter.text(buf, w, h, true, 46, ry + 5, iw - 82, 12, &line, if is_sel { TEXT } else { DIM });
                        }
                    }
                    ry += row_h;
                }
            }
            body_y += list_h + 8;

            if queue_h > 0 {
                draw_card(buf, w, h, 18, body_y, iw - 36, queue_h, 10, WELL, BORDER);
                self.painter.text(buf, w, h, false, 30, body_y + 7, 240, 11, &format!("Planned Queue ({} step{})", plan_steps.len(), if plan_steps.len() == 1 { "" } else { "s" }), ROSE);
                let mut qy = body_y + 24;
                for (i, s) in plan_steps.iter().enumerate().take(3) {
                    self.painter.text(buf, w, h, true, 30, qy, iw - 64, 11, &format!("{}. {}", i + 1, s.text), MUTED);
                    qy += 18;
                }
            }
        }

        // Ask Modal Drawer at bottom OR status bar
        if let Some((ask, typed)) = &self.ask {
            let ay = ih - 106;
            draw_card(buf, w, h, 18, ay, iw - 36, 92, 10, CARD_ALT, ROSE);
            let prompt = ask.prompt();
            self.painter.text(buf, w, h, false, 32, ay + 12, iw - 64, 12, &prompt, TEXT);
            let shown = if ask.secret() { "*".repeat(typed.len()) } else { text(typed) };
            let inp_w = (iw - 252).max(120);
            draw_card(buf, w, h, 32, ay + 38, inp_w, 34, 8, WELL, BORDER_ACTIVE);
            self.painter.text(buf, w, h, true, 44, ay + 48, inp_w - 24, 13, &format!("{}_", shown), TEXT);
            self.push_button(buf, w, h, 32 + inp_w + 10, ay + 40, 90, 30, "OK (Ret)", ButtonStyle::Primary, HitAction::SubmitAsk);
            self.push_button(buf, w, h, 32 + inp_w + 108, ay + 40, 86, 30, "Cancel", ButtonStyle::Normal, HitAction::CancelAsk);
        } else if !self.status.is_empty() {
            let sy = ih - 32;
            fill_rounded_rect(buf, w, h, 18, sy, iw - 36, 24, 6, CARD);
            self.painter.text(buf, w, h, false, 28, sy + 5, iw - 56, 11, &self.status, MUTED);
        }

        window.show_pixels(w, h);
    }
}

fn render_progress(
    painter: &mut NoirPainter,
    window: &mut Window,
    disk_name: &str,
    step: usize,
    steps: usize,
    pct: u64,
    step_text: &str,
) {
    let (w, h) = window_dims(window);
    let buf = window.frame(w, h);
    buf.fill(BG);
    fill_rect(buf, w, h, 0, 0, w as i32, 48, HEADER);
    fill_rect(buf, w, h, 0, 47, w as i32, 1, BORDER);
    painter.text(buf, w, h, false, 18, 14, 100, 15, "CORSET", TEXT);
    painter.pill(buf, w, h, 92, 13, 22, false, "Applying Changes", ROSE, ROSE_BG, ROSE_BORDER);

    let cw = (w as i32 - 64).clamp(280, 540);
    let ch = 172;
    let cx = (w as i32 - cw) / 2;
    let cy = ((h as i32 - ch) / 2).max(60);
    draw_card(buf, w, h, cx, cy, cw, ch, 12, CARD, BORDER_ACTIVE);
    painter.text(buf, w, h, false, cx + 22, cy + 20, cw - 44, 15, &format!("Applying Plan to {}", disk_name), TEXT);
    painter.text(buf, w, h, true, cx + 22, cy + 48, cw - 44, 12, &format!("Step {} of {}  ·  {}%", step, steps, pct), ROSE);
    painter.text(buf, w, h, false, cx + 22, cy + 72, cw - 44, 12, step_text, MUTED);

    let bar_w = cw - 44;
    fill_rounded_rect(buf, w, h, cx + 22, cy + 106, bar_w, 12, 6, WELL);
    let fill_w = ((bar_w as u64 * pct.min(100)) / 100).max(8) as i32;
    fill_rounded_rect(buf, w, h, cx + 22, cy + 106, fill_w.min(bar_w), 12, 6, ROSE);
    painter.text(buf, w, h, false, cx + 22, cy + 132, cw - 44, 11, "Do not disconnect the storage device while writing.", DIM);
    window.show_pixels(w, h);
}

fn window_close(open: Option<(&mut Window, u16)>) -> ! {
    if let Some((window, id)) = open { let _ = disks::done(window, id); }
    sex_pdx::serial_record!("[corset.closed]");
    sex_pdx::task_finish(0)
}

/// A self-check for the gate: a raw write to the system disk must be refused by Files.
fn probe_system(app: &mut App, window: &mut Window, list: &[Info]) {
    let Some(sys) = list.iter().find(|d| d.system()) else { sex_pdx::serial_record!("[corset.probe] disk=none"); return };
    // Past the disk's end, too: should Files ever let it through, the bounds still refuse.
    let r = disks::write(window, sys.id, u64::MAX - 8, &[0u8; disks::SECTOR]);
    let status = r.err().unwrap_or(w::OK);
    sex_pdx::serial_record!("[corset.probe] disk={} status={} message={}", sys.name, status, text(w::message(status)));
    app.log_audit("SHIELD", status == w::DENIED, format!("System disk {} probe write -> status {}", sys.name, status));
}

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Corset");
    let mut args = [0u8; w::ARGS_MAX];
    let probe = sex_app::files::args(&mut window, &mut args).split(|b| *b == b' ').any(|a| a == b"--probe-system");
    let mut app = App {
        disks: Vec::new(),
        cursor: 0,
        scroll: 0,
        tab: DiskTab::All,
        open: None,
        ask: None,
        status: String::new(),
        audit: Vec::new(),
        hits: Vec::new(),
        mouse: (-1, -1),
        mouse_down: false,
        hovered: None,
        painter: NoirPainter::new(),
    };
    app.refresh(&mut window);
    if probe {
        let list = app.disks.clone();
        probe_system(&mut app, &mut window, &list);
    }
    app.render(&mut window);
    sex_pdx::serial_record!("[corset.ready] target={} task={} disks={}", window.target, window.task, app.disks.len());
    loop {
        match window.next_within(2000) {
            Some(Event::Close) => { let open = app.open.as_ref().map(|o| o.info.id); window_close(open.map(|id| (&mut window, id))) }
            Some(Event::Resize { .. }) => app.render(&mut window),
            Some(Event::Key(key)) => { app.key(&mut window, key); app.render(&mut window); }
            Some(Event::Pointer(ptr)) => {
                if app.pointer(&mut window, ptr) { app.render(&mut window); }
            }
            Some(_) => {}
            // Quiet: a stick may have come or gone.
            None if app.open.is_none() && app.ask.is_none() => {
                let before = app.disks.clone();
                if let Ok(list) = disks::list(&mut window) {
                    if list != before { app.refresh(&mut window); app.render(&mut window); }
                }
            }
            None => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! { sex_app::with_stack(1 << 20, main) }
