// SPDX-License-Identifier: GPL-3.0-only
//! A plan: the changes a person asked for, each checked against the layout the earlier ones
//! leave, each with the sentence the confirmation screen shows. Nothing here writes.
use crate::detect::{Content, Disk, Partition};
use crate::format::FsKind;
use crate::{size_text, types, Guid, ALIGN, SECTOR};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_crypt::Cost;
use sex_disk::probe::Layout;

/// A password, kept out of `Debug` output and cleared when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub Vec<u8>);
impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("Secret(..)") }
}
impl Drop for Secret {
    fn drop(&mut self) { for b in self.0.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) } } }
}

/// One change. Partitions are named by their slot in the GPT's entry array, as the layout
/// [`Plan::partitions`] shows it after the earlier ops.
#[derive(Clone, Debug)]
pub enum Op {
    /// A new, empty GPT over the whole disk. Whatever table it had is forgotten.
    CreateTable,
    /// `first` and `sectors` in 512-byte sectors, both whole MiB.
    Add { first: u64, sectors: u64, type_guid: Guid, name: String },
    Delete { slot: usize },
    /// The partition keeps its start; `sectors` is its new length (whole MiB).
    Resize { slot: usize, sectors: u64 },
    /// The partition's data is copied to start at `first` (whole MiB), into space the disk's
    /// current table leaves free; the old copy stays untouched until the table changes.
    Move { slot: usize, first: u64 },
    SetName { slot: usize, name: String },
    SetType { slot: usize, type_guid: Guid },
    /// GPT attribute bits (see [`types::attr`]).
    SetAttrs { slot: usize, attrs: u64 },
    Format { slot: usize, fs: FsKind, label: String },
    /// A sex-crypt header under `password`, then (optionally) a filesystem inside.
    Encrypt { slot: usize, password: Secret, cost: Cost, inner: Option<(FsKind, String)> },
}

/// Why an op was refused, in words a person can act on ([`Refusal::text`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NoTable,
    SystemDisk,
    SystemPartition(usize),
    SystemType,
    BadCost,
    NoSuchPartition(usize),
    Unaligned,
    Empty,
    OutsideDisk,
    Overlaps(usize),
    TableFull,
    NameTooLong,
    ZeroType,
    TooSmall { fs: &'static str, min: u64 },
    TooBig { fs: &'static str, max: u64 },
    CannotShrink(Content),
    CannotGrow(Content),
    MoveNewPartition,
    MoveAfterWrite,
    MoveOntoItself,
    MoveIntoUsedSpace,
    BadLabel,
    ReservedAttrs,
    EmptyPassword,
    Unchanged,
}
impl Refusal {
    pub fn text(&self) -> String {
        match self {
            Refusal::NoTable => "The disk has no partition table: create one first.".into(),
            Refusal::SystemDisk => "This disk holds the running SexOS system; Corset does not replace its table.".into(),
            Refusal::SystemPartition(s) => format!("Partition {} belongs to the SexOS system; only the installer and updater change it.", s + 1),
            Refusal::SystemType => "SexOS system slots and stores are made by the installer, not by Corset.".into(),
            Refusal::BadCost => "That key-derivation cost is outside what SexOS reads back (8 KiB to 1 GiB, 1 to 16 passes, 1 to 8 lanes).".into(),
            Refusal::NoSuchPartition(s) => format!("There is no partition {}.", s + 1),
            Refusal::Unaligned => "Partitions start and end on whole MiB.".into(),
            Refusal::Empty => "A partition needs at least 1 MiB.".into(),
            Refusal::OutsideDisk => "That reaches past the space the partition table can use.".into(),
            Refusal::Overlaps(s) => format!("That overlaps partition {}.", s + 1),
            Refusal::TableFull => "The partition table has no free entry.".into(),
            Refusal::NameTooLong => "A partition name holds at most 36 characters.".into(),
            Refusal::ZeroType => "A partition needs a type.".into(),
            Refusal::TooSmall { fs, min } => format!("{fs} needs at least {}.", size_text(min * SECTOR as u64)),
            Refusal::TooBig { fs, max } => format!("{fs} holds at most {}.", size_text(max.saturating_mul(SECTOR as u64))),
            Refusal::CannotShrink(c) => format!("Shrinking a partition holding {} would cut its data off; Corset shrinks only empty partitions.", c.name()),
            Refusal::CannotGrow(c) => format!("{} cannot grow with its partition here; back it up, then delete, re-create and format.", c.name()),
            Refusal::MoveNewPartition => "A partition added in this plan can be placed where it is wanted instead of moved.".into(),
            Refusal::MoveAfterWrite => "Move a partition before formatting or encrypting it, and only once per plan.".into(),
            Refusal::MoveOntoItself => "A move must go to space that does not overlap the partition's current place, so the old copy stays whole until the table changes.".into(),
            Refusal::MoveIntoUsedSpace => "A move goes into space that is free on the disk now: apply the deletion first, then move.".into(),
            Refusal::BadLabel => "That label does not fit this filesystem (FAT: 11 plain characters; exFAT: 11 characters; sexfs: none).".into(),
            Refusal::ReservedAttrs => "Those attribute bits are reserved by UEFI.".into(),
            Refusal::EmptyPassword => "An encrypted partition needs a password.".into(),
            Refusal::Unchanged => "That changes nothing.".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Step {
    pub op: Op,
    /// The sentence the confirmation shows.
    pub text: String,
    /// The step loses data that is on the disk now.
    pub destructive: bool,
    /// The partition (by plan identity) the step writes data into.
    pub(crate) uid: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct SimPart {
    pub uid: u32,
    pub p: Partition,
    /// Added by this plan.
    pub new: bool,
    /// What an encrypted partition made by this plan holds inside.
    pub inner: Option<FsKind>,
}

#[derive(Clone, Debug)]
pub(crate) struct Sim {
    pub table: bool,
    pub created: bool,
    pub first_usable: u64,
    pub last_usable: u64,
    pub entries: usize,
    pub parts: Vec<SimPart>,
    next_uid: u32,
}

#[derive(Clone, Debug)]
pub struct Plan {
    disk: Disk,
    sim: Sim,
    steps: Vec<Step>,
}

fn initial(disk: &Disk) -> Sim {
    match &disk.gpt {
        Some(g) => Sim {
            table: true, created: false, first_usable: g.header.first_usable, last_usable: g.header.last_usable,
            entries: g.header.entry_count as usize,
            parts: disk.partitions.iter().map(|p| SimPart { uid: p.slot as u32, p: p.clone(), new: false, inner: None }).collect(),
            next_uid: g.header.entry_count + 1,
        },
        None => Sim { table: false, created: false, first_usable: 34, last_usable: disk.sectors.saturating_sub(34), entries: 128, parts: Vec::new(), next_uid: 1 << 16 },
    }
}

fn at(off: u64) -> String { size_text(off * SECTOR as u64) }

impl Plan {
    pub fn new(disk: Disk) -> Plan { let sim = initial(&disk); Plan { disk, sim, steps: Vec::new() } }
    pub fn disk(&self) -> &Disk { &self.disk }
    pub fn steps(&self) -> &[Step] { &self.steps }
    pub fn is_empty(&self) -> bool { self.steps.is_empty() }
    pub fn has_table(&self) -> bool { self.sim.table }
    pub(crate) fn sim(&self) -> &Sim { &self.sim }

    /// The layout after every step, in disk order.
    pub fn partitions(&self) -> Vec<Partition> {
        let mut v: Vec<Partition> = self.sim.parts.iter().map(|s| s.p.clone()).collect();
        v.sort_by_key(|p| p.first);
        v
    }
    pub fn partition(&self, slot: usize) -> Option<&Partition> { self.sim.parts.iter().find(|s| s.p.slot == slot).map(|s| &s.p) }

    /// Free runs of whole MiB in the layout after every step, in disk order.
    pub fn free(&self) -> Vec<(u64, u64)> {
        if !self.sim.table { return Vec::new(); }
        let mut spans: Vec<(u64, u64)> = self.sim.parts.iter().map(|s| (s.p.first, s.p.end())).collect();
        spans.sort_unstable();
        let mut out = Vec::new();
        let mut cursor = self.sim.first_usable;
        let end = self.sim.last_usable + 1;
        let mut push = |from: u64, to: u64| {
            let first = from.next_multiple_of(ALIGN);
            let last = to / ALIGN * ALIGN;
            if last > first { out.push((first, last - first)); }
        };
        for (a, b) in spans { if a > cursor { push(cursor, a); } cursor = cursor.max(b); }
        if end > cursor { push(cursor, end); }
        out
    }
    /// The first free place for `sectors`, if any.
    pub fn first_fit(&self, sectors: u64) -> Option<u64> { self.free().into_iter().find(|f| f.1 >= sectors).map(|f| f.0) }

    pub fn destructive(&self) -> bool { self.steps.iter().any(|s| s.destructive) }

    /// What the person confirms: the disk by name, then every step.
    pub fn confirmation(&self) -> String {
        let mut s = format!("On disk \"{}\" ({}):\n", self.disk.name, size_text(self.disk.bytes()));
        for (i, st) in self.steps.iter().enumerate() {
            s.push_str(&format!("{}. {}{}\n", i + 1, st.text, if st.destructive { " [destroys data]" } else { "" }));
        }
        if self.destructive() { s.push_str(&format!("Type the disk's name, \"{}\", to apply.\n", self.disk.name)); }
        s
    }
    /// A plan that destroys data is confirmed by typing the disk's name; others need nothing.
    pub fn confirms(&self, typed: &str) -> bool { !self.destructive() || typed.trim() == self.disk.name }

    /// Removes the last step.
    pub fn pop(&mut self) -> Option<Op> {
        let last = self.steps.pop()?.op;
        let ops: Vec<Op> = self.steps.drain(..).map(|s| s.op).collect();
        self.sim = initial(&self.disk);
        for op in ops { self.push(op).expect("a step that was accepted is accepted again"); }
        Some(last)
    }

    /// Checks `op` against the layout so far and adds it. For [`Op::Add`] the answer is the
    /// new partition's slot; otherwise the slot the op named (0 for [`Op::CreateTable`]).
    pub fn push(&mut self, op: Op) -> Result<usize, Refusal> {
        let (slot, text, destructive, uid) = self.check(&op)?;
        self.steps.push(Step { op, text, destructive, uid });
        Ok(slot)
    }

    fn base_used(&self) -> Vec<(u64, u64)> {
        if self.disk.gpt.is_some() { return self.disk.partitions.iter().map(|p| (p.first, p.end())).collect(); }
        if self.disk.layout == Layout::Blank { Vec::new() } else { alloc::vec![(0, self.disk.sectors)] }
    }
    fn idx(&self, slot: usize) -> Result<usize, Refusal> {
        if !self.sim.table { return Err(Refusal::NoTable); }
        let i = self.sim.parts.iter().position(|s| s.p.slot == slot).ok_or(Refusal::NoSuchPartition(slot))?;
        if types::is_system(self.sim.parts[i].p.type_guid) { return Err(Refusal::SystemPartition(slot)); }
        Ok(i)
    }
    fn place(&self, first: u64, sectors: u64, except: Option<u32>) -> Result<(), Refusal> {
        if sectors == 0 { return Err(Refusal::Empty); }
        if first % ALIGN != 0 || sectors % ALIGN != 0 { return Err(Refusal::Unaligned); }
        let last = first.checked_add(sectors - 1).ok_or(Refusal::OutsideDisk)?;
        if first < self.sim.first_usable || last > self.sim.last_usable { return Err(Refusal::OutsideDisk); }
        if let Some(o) = self.sim.parts.iter().find(|s| Some(s.uid) != except && first <= s.p.end() - 1 && s.p.first <= last) { return Err(Refusal::Overlaps(o.p.slot)); }
        Ok(())
    }
    fn fs_fits(fs: FsKind, sectors: u64, label: &str) -> Result<(), Refusal> {
        if sectors < fs.min_sectors() { return Err(Refusal::TooSmall { fs: fs.name(), min: fs.min_sectors().next_multiple_of(ALIGN) }); }
        if sectors > fs.max_sectors() { return Err(Refusal::TooBig { fs: fs.name(), max: fs.max_sectors() }); }
        let ok = match fs {
            FsKind::Fat32 => crate::format::fat_label(label).is_ok(),
            FsKind::ExFat => label.encode_utf16().count() <= 11,
            FsKind::Sexfs => label.is_empty(),
        };
        if ok { Ok(()) } else { Err(Refusal::BadLabel) }
    }
    fn lose(c: Content) -> String { if c.holds_data() { format!("; the {} on it is lost", c.name()) } else { String::new() } }

    fn check(&mut self, op: &Op) -> Result<(usize, String, bool, Option<u32>), Refusal> {
        let disk_name = self.disk.name.clone();
        match op {
            Op::CreateTable => {
                if self.disk.is_system() { return Err(Refusal::SystemDisk); }
                let had: Vec<&SimPart> = self.sim.parts.iter().collect();
                let destructive = !had.is_empty() || (!self.sim.table && self.disk.whole.holds_data());
                let what = if !had.is_empty() {
                    format!(" The {} partition{} on it {} forgotten and {} data lost.", had.len(), if had.len() == 1 { "" } else { "s" }, if had.len() == 1 { "is" } else { "are" }, if had.len() == 1 { "its" } else { "their" })
                } else if !self.sim.table && self.disk.whole.holds_data() {
                    format!(" The {} on the whole disk is lost.", self.disk.whole.name())
                } else { String::new() };
                if self.disk.sectors < 34 * 2 + 2048 { return Err(Refusal::TooSmall { fs: "A GPT disk", min: 34 * 2 + 2048 }); }
                self.sim = Sim { table: true, created: true, first_usable: 34, last_usable: self.disk.sectors - 34, entries: 128, parts: Vec::new(), next_uid: self.sim.next_uid };
                Ok((0, format!("Create a new GPT partition table on \"{disk_name}\" ({}).{what}", size_text(self.disk.bytes())), destructive, None))
            }
            Op::Add { first, sectors, type_guid, name } => {
                if !self.sim.table { return Err(Refusal::NoTable); }
                if type_guid.is_zero() { return Err(Refusal::ZeroType); }
                if types::is_system(*type_guid) { return Err(Refusal::SystemType); }
                if name.encode_utf16().count() > 36 { return Err(Refusal::NameTooLong); }
                self.place(*first, *sectors, None)?;
                let base_slots: Vec<usize> = if self.sim.created { Vec::new() } else { self.disk.partitions.iter().map(|p| p.slot).collect() };
                let slot = (0..self.sim.entries).find(|s| !self.sim.parts.iter().any(|p| p.p.slot == *s) && !base_slots.contains(s))
                    .or_else(|| (0..self.sim.entries).find(|s| !self.sim.parts.iter().any(|p| p.p.slot == *s)))
                    .ok_or(Refusal::TableFull)?;
                let uid = self.sim.next_uid; self.sim.next_uid += 1;
                let p = Partition { slot, first: *first, sectors: *sectors, type_guid: *type_guid, part_guid: Guid::ZERO, name: name.clone(), attrs: 0, content: Content::Empty };
                self.sim.parts.push(SimPart { uid, p, new: true, inner: None });
                Ok((slot, format!("Add partition {} \"{}\" ({}), {} from {} to {}.", slot + 1, name, types::name(*type_guid), at(*sectors), at(*first), at(first + sectors)), false, Some(uid)))
            }
            Op::Delete { slot } => {
                let i = self.idx(*slot)?;
                let s = self.sim.parts.remove(i);
                let d = s.p.content.holds_data();
                Ok((*slot, format!("Delete partition {} \"{}\" ({}, {}){}.", slot + 1, s.p.name, at(s.p.sectors), s.p.content.name(), if d { "; its data is lost" } else { "" }), d, None))
            }
            Op::Resize { slot, sectors } => {
                let i = self.idx(*slot)?;
                let s = self.sim.parts[i].clone();
                if *sectors == s.p.sectors { return Err(Refusal::Unchanged); }
                if s.p.first % ALIGN == 0 || *sectors == 0 || sectors % ALIGN != 0 { self.place(s.p.first, *sectors, Some(s.uid))?; } else { self.place_unaligned(s.p.first, *sectors, s.uid)?; }
                let grow = *sectors > s.p.sectors;
                let c = s.p.content;
                let note = if grow {
                    match c {
                        Content::Empty => String::new(),
                        Content::Unknown => "; what it holds is not grown".into(),
                        Content::Sexfs => "; sexfs takes the new space at its next mount".into(),
                        Content::Encrypted if s.inner.is_some_and(|f| f.grows()) => "; the sexfs inside takes the new space at its next unlock".into(),
                        other => return Err(Refusal::CannotGrow(other)),
                    }
                } else if c != Content::Empty { return Err(Refusal::CannotShrink(c)); } else { String::new() };
                self.sim.parts[i].p.sectors = *sectors;
                Ok((*slot, format!("{} partition {} \"{}\" from {} to {}{}.", if grow { "Grow" } else { "Shrink" }, slot + 1, s.p.name, at(s.p.sectors), at(*sectors), note), false, None))
            }
            Op::Move { slot, first } => {
                let i = self.idx(*slot)?;
                let s = self.sim.parts[i].clone();
                if s.new { return Err(Refusal::MoveNewPartition); }
                if self.steps.iter().any(|st| st.uid == Some(s.uid)) { return Err(Refusal::MoveAfterWrite); }
                if *first == s.p.first { return Err(Refusal::Unchanged); }
                if *first % ALIGN != 0 { return Err(Refusal::Unaligned); }
                self.place_unaligned(*first, s.p.sectors, s.uid)?;
                let base = self.disk.partitions.iter().find(|p| p.slot == s.p.slot).ok_or(Refusal::MoveNewPartition)?;
                let copy = base.sectors.min(s.p.sectors);
                if *first < base.end() && base.first < first + copy { return Err(Refusal::MoveOntoItself); }
                if self.base_used().iter().any(|(a, b)| *first < *b && *a < first + s.p.sectors) { return Err(Refusal::MoveIntoUsedSpace); }
                self.sim.parts[i].p.first = *first;
                Ok((*slot, format!("Move partition {} \"{}\" ({}) from {} to {}, copying its data.", slot + 1, s.p.name, at(s.p.sectors), at(base.first), at(*first)), false, Some(s.uid)))
            }
            Op::SetName { slot, name } => {
                let i = self.idx(*slot)?;
                if name.encode_utf16().count() > 36 { return Err(Refusal::NameTooLong); }
                if self.sim.parts[i].p.name == *name { return Err(Refusal::Unchanged); }
                let old = core::mem::replace(&mut self.sim.parts[i].p.name, name.clone());
                Ok((*slot, format!("Rename partition {} from \"{}\" to \"{}\".", slot + 1, old, name), false, None))
            }
            Op::SetType { slot, type_guid } => {
                let i = self.idx(*slot)?;
                if type_guid.is_zero() { return Err(Refusal::ZeroType); }
                if types::is_system(*type_guid) { return Err(Refusal::SystemType); }
                if self.sim.parts[i].p.type_guid == *type_guid { return Err(Refusal::Unchanged); }
                let old = core::mem::replace(&mut self.sim.parts[i].p.type_guid, *type_guid);
                let tx = |g: Guid| { let t = g.text(); format!("{} ({})", types::name(g), core::str::from_utf8(&t).unwrap_or("")) };
                Ok((*slot, format!("Change partition {}'s type from {} to {}.", slot + 1, tx(old), tx(*type_guid)), false, None))
            }
            Op::SetAttrs { slot, attrs } => {
                let i = self.idx(*slot)?;
                if attrs & !types::attr::SETTABLE != 0 { return Err(Refusal::ReservedAttrs); }
                if self.sim.parts[i].p.attrs == *attrs { return Err(Refusal::Unchanged); }
                self.sim.parts[i].p.attrs = *attrs;
                let names: Vec<&str> = types::attr::NAMES.iter().filter(|(b, _)| attrs & b != 0).map(|(_, n)| *n).collect();
                Ok((*slot, format!("Set partition {}'s flags to {}.", slot + 1, if names.is_empty() { "none".into() } else { names.join(", ") }), false, None))
            }
            Op::Format { slot, fs, label } => {
                let i = self.idx(*slot)?;
                let s = self.sim.parts[i].clone();
                Self::fs_fits(*fs, s.p.sectors, label)?;
                self.sim.parts[i].p.content = fs.content();
                self.sim.parts[i].inner = None;
                let l = if label.is_empty() { String::new() } else { format!(" labelled \"{label}\"") };
                Ok((*slot, format!("Format partition {} \"{}\" ({}) as {}{}{}.", slot + 1, s.p.name, at(s.p.sectors), fs.name(), l, Self::lose(s.p.content)), s.p.content.holds_data(), Some(s.uid)))
            }
            Op::Encrypt { slot, password, cost, inner } => {
                let i = self.idx(*slot)?;
                let s = self.sim.parts[i].clone();
                if password.0.is_empty() { return Err(Refusal::EmptyPassword); }
                let room = crate::crypt::inner_sectors(s.p.sectors);
                if room < ALIGN { return Err(Refusal::TooSmall { fs: "An encrypted partition", min: 2 * ALIGN }); }
                if let Some((fs, label)) = inner { Self::fs_fits(*fs, room, label).map_err(|e| match e { Refusal::TooSmall { fs, min } => Refusal::TooSmall { fs, min: min + ALIGN }, e => e })?; }
                // The bounds sex-crypt accepts when it reads a header back.
                if !(8..=1024 * 1024).contains(&cost.memory_kib) || !(1..=16).contains(&cost.passes) || !(1..=8).contains(&cost.lanes) { return Err(Refusal::BadCost); }
                self.sim.parts[i].p.content = Content::Encrypted;
                self.sim.parts[i].inner = inner.as_ref().map(|(f, _)| *f);
                let then = match inner { Some((fs, l)) if !l.is_empty() => format!(", then format it inside as {} labelled \"{l}\"", fs.name()), Some((fs, _)) => format!(", then format it inside as {}", fs.name()), None => String::new() };
                Ok((*slot, format!("Encrypt partition {} \"{}\" ({}) with a password (XTS-AES-256, Argon2id {} MiB){}{}.", slot + 1, s.p.name, at(s.p.sectors), cost.memory_kib / 1024, then, Self::lose(s.p.content)), s.p.content.holds_data(), Some(s.uid)))
            }
        }
    }
    /// A partition another tool made off the MiB grid keeps its start when resized and its
    /// length when moved: only the part Corset chooses is held to the grid.
    fn place_unaligned(&self, first: u64, sectors: u64, uid: u32) -> Result<(), Refusal> {
        let last = first.checked_add(sectors - 1).ok_or(Refusal::OutsideDisk)?;
        if first < self.sim.first_usable || last > self.sim.last_usable { return Err(Refusal::OutsideDisk); }
        if let Some(o) = self.sim.parts.iter().find(|s| s.uid != uid && first <= s.p.end() - 1 && s.p.first <= last) { return Err(Refusal::Overlaps(o.p.slot)); }
        Ok(())
    }
}
