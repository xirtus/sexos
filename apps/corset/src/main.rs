// SPDX-License-Identifier: GPL-3.0-only
//! Corset: disks and partitions. Lists the disks Files offers (USB sticks and disks, disk
//! images), and on one the person hands it (Files asks them, naming the disk) plans
//! changes (`corset::plan`, on corset-core): a new GPT, partitions added, deleted or resized, formatted
//! FAT32, exFAT or sexfs, or encrypted. Nothing is written until the person has read every
//! step on the confirmation screen, and a plan that destroys data is confirmed by typing
//! the disk's name. While it applies, a progress line; afterwards Files mounts what the
//! disk holds again under `/media`.
//!
//! The system disk is listed and never opened: Files refuses it, whatever is asked.
#![no_std]
#![no_main]
extern crate alloc;

mod dev;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use corset::plan::{self, size_text, ApplyError, Disk, Fs, Op, Part, Plan, Refusal, Secret, MIB, TYPES};
use sex_app::files::disks::{self, Disk as Info};
use sex_app::{Event, Nav, Window};
use sex_pdx::fs as w;

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
const BAR: usize = 64;
const ROWS: usize = 12;

/// What the line at the bottom is asking for.
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
                for (i, (_, name)) in TYPES.iter().enumerate() { t.push_str(&format!("  {} {}", i + 1, name)); }
                t
            }
            Ask::Resize { .. } => "New size in MiB".into(),
            Ask::FormatFs { .. } | Ask::EncryptFs { .. } => format!("Filesystem: 1 FAT32  2 exFAT  3 sexfs{}", if matches!(self, Ask::EncryptFs { .. }) { "  0 none inside" } else { "" }),
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

struct Open { info: Info, plan: Plan, cursor: usize, review: bool, leaving: bool }
struct App { disks: Vec<Info>, cursor: usize, open: Option<Open>, ask: Option<(Ask, Vec<u8>)>, status: String }

fn rows(plan: &Plan) -> Vec<Row> {
    let mut v: Vec<Row> = plan.partitions().into_iter().map(Row::Part).collect();
    v.extend(plan.free().into_iter().map(|(first, sectors)| Row::Free { first, sectors }));
    v.sort_by_key(Row::first);
    v
}
/// The disk as one line: each partition by its number, free space as dots.
fn bar(plan: &Plan) -> String {
    let total = plan.disk().sectors.max(1);
    let parts = plan.partitions();
    (0..BAR).map(|i| {
        let mid = (total as u128 * (2 * i as u128 + 1) / (2 * BAR as u128)) as u64;
        match parts.iter().find(|p| p.first <= mid && mid < p.end()) {
            Some(p) => char::from_digit(((p.slot + 1) % 36) as u32, 36).unwrap_or('#').to_ascii_uppercase(),
            None if plan.free().iter().any(|f| f.0 <= mid && mid < f.0 + f.1) => '.',
            None => ' ',
        }
    }).collect()
}
fn fit(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    while out.chars().count() < n { out.push(' '); }
    out
}
fn flags(i: &Info) -> &'static str {
    if i.system() { "the system's disk: never changed" } else if i.read_only() { "write protected" } else { "" }
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

impl App {
    fn refresh(&mut self, window: &mut Window) {
        match disks::list(window) {
            Ok(list) => {
                let names: Vec<String> = list.iter().map(|d| format!("{}:{}:{}", d.name, d.sectors, d.flags)).collect();
                sex_pdx::serial_record!("[corset.disks] count={} list={}", list.len(), names.join(","));
                self.disks = list;
                // Start on the first disk Corset may change.
                if self.cursor >= self.disks.len() || self.disks.get(self.cursor).is_some_and(|d| d.system()) {
                    self.cursor = self.disks.iter().position(|d| !d.system()).unwrap_or(0);
                }
            }
            Err(e) => self.status = format!("Files didn't list the disks: {}.", text(w::message(e))),
        }
    }

    /// Asks Files for the disk (the person answers there) and reads what it holds.
    fn open_disk(&mut self, window: &mut Window) {
        let Some(info) = self.disks.get(self.cursor).cloned() else { return };
        if info.system() {
            sex_pdx::serial_record!("[corset.refused] disk={} why=system", info.name);
            self.status = format!("SexOS runs from {}: Corset never changes it.", info.name);
            return;
        }
        let mut asked = false;
        loop {
            let mut first = [0u8; disks::SECTOR];
            match disks::read(window, info.id, 0, &mut first) {
                Ok(()) => break,
                Err(w::PENDING) => {
                    if !asked {
                        asked = true;
                        sex_pdx::serial_record!("[corset.waiting] disk={}", info.name);
                        window.show(format!("Corset\n\nFiles is asking you whether Corset may use {} ({}, {}).\nAnswer there: Y allows, N refuses.\n\nEsc stops waiting.",
                                            info.name, info.product, size_text(info.sectors * 512)).as_bytes());
                    }
                    match window.next_within(150) {
                        Some(Event::Close) => window_close(None),
                        Some(Event::Key(k)) if k.code == ESC => { self.status = "Stopped waiting.".into(); return; }
                        _ => {}
                    }
                }
                Err(e) => {
                    sex_pdx::serial_record!("[corset.denied] disk={} status={}", info.name, e);
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
                if disk.is_system() { self.status = "This disk holds a SexOS system: Corset leaves its system partitions alone.".into(); }
                self.open = Some(Open { info, plan: Plan::new(disk), cursor: 0, review: false, leaving: false });
            }
            Err(e) => {
                self.status = format!("Couldn't read {}: {} ({}).", info.name, e, text(w::message(status.get())));
                let _ = disks::done(&mut window.borrow_mut(), info.id);
            }
        }
    }

    fn push(&mut self, op: Op) -> bool {
        let Some(o) = self.open.as_mut() else { return false };
        match o.plan.push(op) {
            Ok(slot) => {
                let step = o.plan.steps().last().unwrap();
                sex_pdx::serial_record!("[corset.planned] steps={} destructive={} text={}", o.plan.steps().len(), step.destructive as u8, step.text);
                if let Some(i) = rows(&o.plan).iter().position(|r| matches!(r, Row::Part(p) if p.slot == slot)) { o.cursor = i; }
                self.status.clear();
                true
            }
            Err(r) => { self.refused(r); false }
        }
    }
    fn refused(&mut self, r: Refusal) {
        let why = r.text();
        sex_pdx::serial_record!("[corset.refused] why={}", why);
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
        let mut shown = (0usize, u64::MAX);
        let result = {
            let mut device = dev::Raw { window: &cell, id, sectors: o.info.sectors, status: &status };
            plan::apply(&mut device, &o.plan, &mut *random, &mut |p| {
                let pct = p.pct;
                if (p.step, pct / 10) == shown { return; }
                shown = (p.step, pct / 10);
                sex_pdx::serial_record!("[corset.progress] step={} steps={} pct={}", p.step, p.steps, pct);
                let filled = (pct as usize * 40 / 100).min(40);
                let line = format!("Corset  applying to {}\n\nStep {} of {}: {}\n[{}{}] {}%\n\nDon't pull the disk out.", name, p.step, p.steps, p.text,
                                   "#".repeat(filled), ".".repeat(40 - filled), pct);
                cell.borrow_mut().show(line.as_bytes());
            })
        };
        let window = cell.into_inner();
        match result {
            Ok(r) => {
                sex_pdx::serial_record!("[corset.applied] disk={} table={} before={} after={} parts={}", name, r.table_written as u8, r.before_table, r.after_table, r.disk.parts.len());
                self.status = format!("Applied {} change{} to {}. Files mounts its volumes again.", o.plan.steps().len(), if o.plan.steps().len() == 1 { "" } else { "s" }, name);
            }
            Err(ApplyError::Changed) => {
                sex_pdx::serial_record!("[corset.apply.failed] disk={} error=changed", name);
                self.status = format!("{} changed since Corset read it: nothing was written. Open it again to plan on what it holds now.", name);
            }
            Err(ApplyError::Engine(e)) => {
                sex_pdx::serial_record!("[corset.apply.failed] disk={} error={} status={}", name, e, status.get());
                self.status = format!("Stopped: {} ({}). The old partition table stays until its write; open the disk again to see what it holds.", e, text(w::message(status.get())));
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
        let (review, destructive, steps, has_table, free) = {
            let o = self.open.as_ref().unwrap();
            (o.review, o.plan.destructive(), o.plan.steps().len(), o.plan.has_table(), o.plan.free())
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
        let leaving = core::mem::take(&mut o.leaving);
        if let Some(n) = sex_app::nav(key) {
            let count = rows(&o.plan).len();
            match n { Nav::Up => o.cursor = o.cursor.saturating_sub(1), Nav::Down => o.cursor = (o.cursor + 1).min(count.saturating_sub(1)), _ => {} }
            sex_pdx::serial_record!("[corset.cursor] row={}", o.cursor);
            return;
        }
        self.status.clear();
        let selected = self.selected();
        match key.code {
            KEY_G => { self.push(Op::CreateTable); }
            KEY_A => {
                let at = match selected { Some(Row::Free { first, sectors }) => Some((first, sectors)), _ => free.iter().copied().max_by_key(|f| f.1) };
                match at {
                    Some((first, room)) => self.ask = Some((Ask::AddSize { first, room }, Vec::new())),
                    None if !has_table => self.status = "The disk has no partition table: G makes one.".into(),
                    None => self.status = "No free space left.".into(),
                }
            }
            KEY_D => if let Some(slot) = self.selected_slot() { self.push(Op::Delete { slot }); },
            KEY_R => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::Resize { slot }, Vec::new())); },
            KEY_F => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::FormatFs { slot }, Vec::new())); },
            KEY_E => if let Some(slot) = self.selected_slot() { self.ask = Some((Ask::Password { slot }, Vec::new())); },
            KEY_U => {
                let o = self.open.as_mut().unwrap();
                if o.plan.pop().is_some() {
                    sex_pdx::serial_record!("[corset.undone] steps={}", o.plan.steps().len());
                    o.cursor = o.cursor.min(rows(&o.plan).len().saturating_sub(1));
                } else { self.status = "Nothing to undo.".into(); }
            }
            KEY_H => self.status = "Health: not available yet. The disk drivers don't pass SMART or NVMe health pages to programs.".into(),
            ENTER => self.review(),
            ESC => {
                if steps > 0 && !leaving {
                    self.open.as_mut().unwrap().leaving = true;
                    self.status = format!("Esc again drops the {} planned change{}.", steps, if steps == 1 { "" } else { "s" });
                    return;
                }
                let o = self.open.take().unwrap();
                self.done(window, o.info.id, &o.info.name);
                self.refresh(window);
            }
            _ => {}
        }
    }
    fn render(&self, window: &mut Window) {
        let mut t = String::new();
        match &self.open {
            None => {
                t.push_str("Corset  disks and partitions\n\n");
                if self.disks.is_empty() { t.push_str("  No disks. Plug in a USB stick or disk; F5 looks again.\n"); }
                for (i, d) in self.disks.iter().enumerate() {
                    t.push_str(&format!("{} {} {} {}  {}\n", if i == self.cursor { ">" } else { " " }, fit(&d.name, 8), fit(&d.product, 30), fit(&size_text(d.sectors * 512), 9), flags(d)));
                }
                t.push_str("\nEnter opens a disk (Files asks you first)  F5 looks again  F11 closes\n");
            }
            Some(o) if o.review => {
                t.push_str("Corset  confirm\n\n");
                t.push_str(&o.plan.confirmation());
                t.push_str(if o.plan.destructive() { "\nEnter, then type the disk's name, applies. Esc goes back.\n" } else { "\nEnter applies. Esc goes back.\n" });
            }
            Some(o) => {
                let plan = &o.plan;
                let table = if plan.has_table() { "GPT" } else { "no partition table" };
                t.push_str(&format!("Corset  {}  {}  {}  {}\n", o.info.name, o.info.product, size_text(o.info.sectors * 512), table));
                t.push_str(&format!("|{}|\n", bar(plan)));
                t.push_str("   #  Name             Type                    Size       Start      Holds\n");
                let rows = rows(plan);
                let top = o.cursor.saturating_sub(ROWS - 1);
                for (i, r) in rows.iter().enumerate().skip(top).take(ROWS) {
                    let mark = if i == o.cursor { ">" } else { " " };
                    match r {
                        Row::Part(p) => t.push_str(&format!("{}  {} {} {} {} {} {}\n", mark, fit(&format!("{}", p.slot + 1), 2), fit(&p.name, 16), fit(p.type_name(), 23),
                                                            fit(&size_text(p.bytes()), 10), fit(&size_text(p.first * 512), 10), p.content.name())),
                        Row::Free { first, sectors } => t.push_str(&format!("{}  .  {} {} {} {}\n", mark, fit("free space", 16), fit("", 23), fit(&size_text(sectors * 512), 10), size_text(first * 512))),
                    }
                }
                if rows.is_empty() { t.push_str(if plan.has_table() { "   (no partitions, no free space)\n" } else { "   (G makes a partition table)\n" }); }
                if !plan.is_empty() {
                    t.push_str("\nPlanned (U undoes the last):\n");
                    for (i, s) in plan.steps().iter().enumerate() { t.push_str(&format!(" {}. {}\n", i + 1, s.text)); }
                }
                t.push_str("\nHealth: not available yet.\n");
            }
        }
        if let Some((ask, typed)) = &self.ask {
            let shown = if ask.secret() { "*".repeat(typed.len()) } else { text(typed) };
            t.push_str(&format!("\n{}: {}_\n", ask.prompt(), shown));
        } else if self.open.as_ref().is_some_and(|o| !o.review) {
            t.push_str("G new GPT  A add  D delete  R resize  F format  E encrypt  U undo  H health  Enter review  Esc back\n");
        }
        if !self.status.is_empty() { t.push_str(&format!("\n{}\n", self.status)); }
        window.show(t.as_bytes());
    }
}

fn window_close(open: Option<(&mut Window, u16)>) -> ! {
    if let Some((window, id)) = open { let _ = disks::done(window, id); }
    sex_pdx::serial_record!("[corset.closed]");
    sex_pdx::task_finish(0)
}

/// A self-check for the gate: a raw write to the system disk must be refused by Files.
fn probe_system(window: &mut Window, list: &[Info]) {
    let Some(sys) = list.iter().find(|d| d.system()) else { sex_pdx::serial_record!("[corset.probe] disk=none"); return };
    // Past the disk's end, too: should Files ever let it through, the bounds still refuse.
    let r = disks::write(window, sys.id, u64::MAX - 8, &[0u8; disks::SECTOR]);
    let status = r.err().unwrap_or(w::OK);
    sex_pdx::serial_record!("[corset.probe] disk={} status={} message={}", sys.name, status, text(w::message(status)));
}

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Corset");
    let mut args = [0u8; w::ARGS_MAX];
    let probe = sex_app::files::args(&mut window, &mut args).split(|b| *b == b' ').any(|a| a == b"--probe-system");
    let mut app = App { disks: Vec::new(), cursor: 0, open: None, ask: None, status: String::new() };
    app.refresh(&mut window);
    if probe { probe_system(&mut window, &app.disks.clone()); }
    app.render(&mut window);
    sex_pdx::serial_record!("[corset.ready] target={} task={} disks={}", window.target, window.task, app.disks.len());
    loop {
        match window.next_within(2000) {
            Some(Event::Close) => { let open = app.open.as_ref().map(|o| o.info.id); window_close(open.map(|id| (&mut window, id))) }
            Some(Event::Key(key)) => { app.key(&mut window, key); app.render(&mut window); }
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
