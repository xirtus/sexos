// SPDX-License-Identifier: GPL-3.0-only
//! What Corset adds to corset-core: a plan of changes, each checked against the layout the
//! earlier ones leave and each with the sentence the confirmation screen shows, and the
//! order an apply writes them in.
//!
//! 1. [`Disk::read`] snapshots a device: its GPT (or that it has none), every partition
//!    and what each holds.
//! 2. [`Plan`] collects [`Op`]s. Each is checked as it is pushed (1 MiB grid, overlap,
//!    table limits, filesystem minimum sizes, what can be resized) on a copy of the table
//!    in corset-core's [`GptEditor`]. Nothing touches the disk.
//! 3. [`apply`] reads the disk again and refuses if it changed since the snapshot. Then
//!    it writes every change that lands in space the old table leaves free (new
//!    filesystems), then the table (corset-core's commit: backup copy first, then the
//!    primary), then the changes that land inside old partitions. Until the table write
//!    the old table is intact and describes the disk exactly as before.
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use corset_core::{
    format_encrypted_volume, format_exfat, format_fat32, format_sexfs, inspect_filesystem, BlockIo, CorsetError,
    CreatePartitionOptions, Cost, EncryptedBlockIo, ExFatFormatOptions, Fat32FormatOptions, FilesystemType, GptEditor,
    Guid, PartitionIo, MASTER_KEY_BYTES, TYPE_BDSM_ENCRYPTED, TYPE_EFI_SYSTEM, TYPE_LINUX_FILESYSTEM,
    TYPE_MICROSOFT_BASIC_DATA, TYPE_SEXOS_BOOT, TYPE_SEXOS_DATA,
};

pub const SECTOR: u64 = 512;
/// Sectors in a MiB: partitions start and end on whole MiB.
pub const MIB: u64 = (1 << 20) / SECTOR;
/// Encryption's key derivation: 16 MiB of Argon2id, as a program's memory allows.
pub const COST: Cost = Cost::SMALL;

/// The partition types Corset offers, by menu number.
pub const TYPES: &[(Guid, &str)] = &[
    (TYPE_MICROSOFT_BASIC_DATA, "Basic data (FAT, exFAT)"),
    (TYPE_LINUX_FILESYSTEM, "Linux filesystem"),
    (TYPE_BDSM_ENCRYPTED, "SexOS encrypted"),
    (TYPE_EFI_SYSTEM, "EFI System"),
];
fn is_system(t: Guid) -> bool { t == TYPE_SEXOS_BOOT || t == TYPE_SEXOS_DATA }

/// `bytes` as people read sizes: "512 B", "33 MiB", "1.5 GiB".
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut u = 0;
    let mut whole = bytes;
    while whole >= 1024 && u + 1 < UNITS.len() { whole /= 1024; u += 1; }
    if u == 0 { return format!("{bytes} B"); }
    let unit = 1u128 << (10 * u);
    let tenths = (bytes as u128 * 10 + unit / 2) / unit;
    if tenths % 10 == 0 || tenths >= 1000 { format!("{} {}", (tenths + 5) / 10, UNITS[u]) } else { format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[u]) }
}
fn at(sectors: u64) -> String { size_text(sectors * SECTOR) }

/// Filesystems Corset makes, through corset-core's formatters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fs { Fat32, ExFat, Sexfs }
impl Fs {
    pub fn name(self) -> &'static str { match self { Fs::Fat32 => "FAT32", Fs::ExFat => "exFAT", Fs::Sexfs => "sexfs" } }
    fn content(self) -> Content { Content::Fs(match self { Fs::Fat32 => FilesystemType::Fat32, Fs::ExFat => FilesystemType::ExFat, Fs::Sexfs => FilesystemType::SexFs }) }
    /// Smallest partition it is made on, in sectors.
    pub fn min_sectors(self) -> u64 { match self { Fs::Fat32 => 33 * MIB, Fs::ExFat => MIB, Fs::Sexfs => 2 * MIB } }
    fn label_ok(self, label: &str) -> bool {
        match self {
            Fs::Fat32 => label.len() <= 11 && label.bytes().all(|c| (c.is_ascii_graphic() || c == b' ') && !b"\"*+,./:;<=>?[\\]|".contains(&c)),
            Fs::ExFat => label.encode_utf16().count() <= 11,
            Fs::Sexfs => label.is_empty(),
        }
    }
    fn grows(self) -> bool { self == Fs::Sexfs }
}

/// What a partition (or a whole disk without a table) holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content { Empty, Fs(FilesystemType) }
impl Content {
    pub fn name(self) -> &'static str {
        match self { Content::Empty => "empty", Content::Fs(FilesystemType::Unknown) => "unknown data", Content::Fs(t) => t.display_name() }
    }
    /// A change over it destroys something a person may want.
    pub fn holds_data(self) -> bool { self != Content::Empty }
}
/// Its first 8 KiB are zero: nothing anyone would miss. Otherwise what corset-core sees.
pub fn content<D: BlockIo + ?Sized>(dev: &mut D) -> Result<Content, CorsetError> {
    let mut head = vec![0u8; (16 * SECTOR).min(dev.sector_count() * SECTOR) as usize];
    dev.read_sectors(0, &mut head)?;
    if head.iter().all(|b| *b == 0) { return Ok(Content::Empty); }
    Ok(Content::Fs(inspect_filesystem(dev).map(|i| i.fs_type).unwrap_or(FilesystemType::Unknown)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part { pub slot: usize, pub first: u64, pub sectors: u64, pub type_guid: Guid, pub name: String, pub content: Content }
impl Part {
    pub fn end(&self) -> u64 { self.first + self.sectors }
    pub fn bytes(&self) -> u64 { self.sectors * SECTOR }
    pub fn type_name(&self) -> &'static str {
        TYPES.iter().find(|(g, _)| *g == self.type_guid).map_or_else(|| self.type_guid.human_type_name(), |(_, n)| n)
    }
}

/// A snapshot of a disk.
#[derive(Clone, Debug)]
pub struct Disk {
    /// The name the person types to confirm a destructive plan.
    pub name: String,
    pub sectors: u64,
    pub table: Option<GptEditor>,
    pub parts: Vec<Part>,
    /// What the whole disk holds when it has no GPT.
    pub whole: Content,
    /// The table areas as read, to see whether anything changed before an apply.
    fingerprint: Vec<u8>,
}
impl Disk {
    pub fn read<D: BlockIo + ?Sized>(dev: &mut D, name: &str) -> Result<Disk, CorsetError> {
        if dev.sector_size() as u64 != SECTOR { return Err(CorsetError::UnsupportedSectorSize(dev.sector_size())); }
        let sectors = dev.sector_count();
        if sectors < 34 * 2 + MIB { return Err(CorsetError::NoSpace("disk too small for a partition table")); }
        let mut fingerprint = vec![0u8; 2 * 34 * SECTOR as usize];
        dev.read_sectors(0, &mut fingerprint[..34 * SECTOR as usize])?;
        dev.read_sectors(sectors - 34, &mut fingerprint[34 * SECTOR as usize..])?;
        let table = GptEditor::open(dev).ok();
        let mut parts = Vec::new();
        if let Some(t) = &table {
            for e in t.entries.iter().filter(|e| e.is_used()) {
                let c = content(&mut PartitionIo::new(dev, e.first_lba, e.sector_count())?)?;
                parts.push(Part { slot: e.index, first: e.first_lba, sectors: e.sector_count(), type_guid: e.type_guid, name: e.name.clone(), content: c });
            }
            parts.sort_by_key(|p| p.first);
        }
        let whole = if table.is_some() { Content::Empty } else { content(dev)? };
        Ok(Disk { name: name.into(), sectors, table, parts, whole, fingerprint })
    }
    pub fn bytes(&self) -> u64 { self.sectors * SECTOR }
    /// The disk holds a SexOS system (its slots or store): Corset leaves those alone.
    pub fn is_system(&self) -> bool { self.parts.iter().any(|p| is_system(p.type_guid)) }
}

/// A password, kept out of `Debug` output and cleared when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub Vec<u8>);
impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("Secret(..)") }
}
impl Drop for Secret {
    fn drop(&mut self) { for b in self.0.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) } } }
}

/// One change. Partitions are named by their GPT slot, as [`Plan::partitions`] shows them.
#[derive(Clone, Debug)]
pub enum Op {
    /// A new, empty GPT over the whole disk. Whatever table it had is forgotten.
    CreateTable,
    /// `first` and `sectors` in 512-byte sectors, both whole MiB.
    Add { first: u64, sectors: u64, type_guid: Guid, name: String },
    Delete { slot: usize },
    /// The partition keeps its start; `sectors` is its new length (whole MiB).
    Resize { slot: usize, sectors: u64 },
    Format { slot: usize, fs: Fs, label: String },
    /// A sex-crypt header under `password`, then (optionally) a filesystem inside.
    Encrypt { slot: usize, password: Secret, inner: Option<(Fs, String)> },
}

/// Why an op was refused, in words a person can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NoTable,
    SystemDisk,
    SystemPartition(usize),
    SystemType,
    NoSuchPartition(usize),
    Unaligned,
    Empty,
    OutsideDisk,
    Overlaps(usize),
    TableFull,
    NameTooLong,
    TooSmall { fs: &'static str, min: u64 },
    CannotShrink(Content),
    CannotGrow(Content),
    BadLabel,
    EmptyPassword,
    Unchanged,
    Engine(String),
}
impl Refusal {
    pub fn text(&self) -> String {
        match self {
            Refusal::NoTable => "The disk has no partition table: create one first.".into(),
            Refusal::SystemDisk => "This disk holds a SexOS system; Corset does not replace its table.".into(),
            Refusal::SystemPartition(s) => format!("Partition {} belongs to the SexOS system; only the installer and updater change it.", s + 1),
            Refusal::SystemType => "SexOS system slots and stores are made by the installer, not by Corset.".into(),
            Refusal::NoSuchPartition(s) => format!("There is no partition {}.", s + 1),
            Refusal::Unaligned => "Partitions start and end on whole MiB.".into(),
            Refusal::Empty => "A partition needs at least 1 MiB.".into(),
            Refusal::OutsideDisk => "That reaches past the space the partition table can use.".into(),
            Refusal::Overlaps(s) => format!("That overlaps partition {}.", s + 1),
            Refusal::TableFull => "The partition table has no free entry.".into(),
            Refusal::NameTooLong => "A partition name holds at most 36 characters.".into(),
            Refusal::TooSmall { fs, min } => format!("{fs} needs at least {}.", at(*min)),
            Refusal::CannotShrink(c) => format!("Shrinking a partition holding {} would cut its data off; Corset shrinks only empty partitions.", c.name()),
            Refusal::CannotGrow(c) => format!("{} cannot grow with its partition here; back it up, then delete, re-create and format.", c.name()),
            Refusal::BadLabel => "That label does not fit this filesystem (FAT: 11 plain characters; exFAT: 11 characters; sexfs: none).".into(),
            Refusal::EmptyPassword => "An encrypted partition needs a password.".into(),
            Refusal::Unchanged => "That changes nothing.".into(),
            Refusal::Engine(e) => format!("Corset's engine refused: {e}."),
        }
    }
}
fn engine(e: CorsetError) -> Refusal {
    match e {
        CorsetError::NoSpace(m) if m.contains("table full") => Refusal::TableFull,
        CorsetError::NoSpace(_) => Refusal::OutsideDisk,
        e => Refusal::Engine(format!("{e}")),
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
    uid: Option<u32>,
}

#[derive(Clone, Debug)]
struct SimPart { uid: u32, p: Part, new: bool, inner: Option<Fs> }
#[derive(Clone, Debug)]
struct Sim { table: Option<GptEditor>, created: bool, parts: Vec<SimPart>, next_uid: u32 }

/// A disk of `sectors`, for laying a blank table out without touching anything.
struct Size(u64);
impl BlockIo for Size {
    fn sector_size(&self) -> u32 { SECTOR as u32 }
    fn sector_count(&self) -> u64 { self.0 }
    fn read_sectors(&mut self, _: u64, _: &mut [u8]) -> Result<(), CorsetError> { Err(CorsetError::InvalidParameter("no IO")) }
    fn write_sectors(&mut self, _: u64, _: &[u8]) -> Result<(), CorsetError> { Err(CorsetError::InvalidParameter("no IO")) }
}

#[derive(Clone, Debug)]
pub struct Plan { disk: Disk, sim: Sim, steps: Vec<Step> }

fn initial(disk: &Disk) -> Sim {
    Sim { table: disk.table.clone(), created: false, parts: disk.parts.iter().map(|p| SimPart { uid: p.slot as u32, p: p.clone(), new: false, inner: None }).collect(), next_uid: 1 << 16 }
}

impl Plan {
    pub fn new(disk: Disk) -> Plan { let sim = initial(&disk); Plan { disk, sim, steps: Vec::new() } }
    pub fn disk(&self) -> &Disk { &self.disk }
    pub fn steps(&self) -> &[Step] { &self.steps }
    pub fn is_empty(&self) -> bool { self.steps.is_empty() }
    pub fn has_table(&self) -> bool { self.sim.table.is_some() }
    /// The layout after every step, in disk order.
    pub fn partitions(&self) -> Vec<Part> {
        let mut v: Vec<Part> = self.sim.parts.iter().map(|s| s.p.clone()).collect();
        v.sort_by_key(|p| p.first);
        v
    }
    /// Free runs of whole MiB in the layout after every step, in disk order.
    pub fn free(&self) -> Vec<(u64, u64)> {
        let Some(t) = &self.sim.table else { return Vec::new() };
        let mut spans: Vec<(u64, u64)> = self.sim.parts.iter().map(|s| (s.p.first, s.p.end())).collect();
        spans.sort_unstable();
        let mut out = Vec::new();
        let mut cursor = t.header.first_usable_lba;
        let end = t.header.last_usable_lba + 1;
        let mut push = |from: u64, to: u64| {
            let first = from.next_multiple_of(MIB);
            let last = to / MIB * MIB;
            if last > first { out.push((first, last - first)); }
        };
        for (a, b) in spans { if a > cursor { push(cursor, a); } cursor = cursor.max(b); }
        if end > cursor { push(cursor, end); }
        out
    }
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
        let mut sim = self.sim.clone();
        let (slot, text, destructive, uid) = Self::check(&self.disk, &mut sim, &op)?;
        self.sim = sim;
        self.steps.push(Step { op, text, destructive, uid });
        Ok(slot)
    }

    fn idx(sim: &Sim, slot: usize) -> Result<usize, Refusal> {
        if sim.table.is_none() { return Err(Refusal::NoTable); }
        let i = sim.parts.iter().position(|s| s.p.slot == slot).ok_or(Refusal::NoSuchPartition(slot))?;
        if is_system(sim.parts[i].p.type_guid) { return Err(Refusal::SystemPartition(slot)); }
        Ok(i)
    }
    fn fs_fits(fs: Fs, sectors: u64, label: &str) -> Result<(), Refusal> {
        if sectors < fs.min_sectors() { return Err(Refusal::TooSmall { fs: fs.name(), min: fs.min_sectors() }); }
        if fs.label_ok(label) { Ok(()) } else { Err(Refusal::BadLabel) }
    }
    fn lose(c: Content) -> String { if c.holds_data() { format!("; the {} on it is lost", c.name()) } else { String::new() } }

    fn check(disk: &Disk, sim: &mut Sim, op: &Op) -> Result<(usize, String, bool, Option<u32>), Refusal> {
        let disk_name = &disk.name;
        match op {
            Op::CreateTable => {
                if disk.is_system() { return Err(Refusal::SystemDisk); }
                let n = sim.parts.len();
                let had_whole = sim.table.is_none() && !sim.created && disk.whole.holds_data();
                let destructive = n > 0 || had_whole;
                let what = if n > 0 {
                    format!(" The {} partition{} on it {} forgotten and {} data lost.", n, if n == 1 { "" } else { "s" }, if n == 1 { "is" } else { "are" }, if n == 1 { "its" } else { "their" })
                } else if had_whole { format!(" The {} on the whole disk is lost.", disk.whole.name()) } else { String::new() };
                sim.table = Some(GptEditor::init_blank(&mut Size(disk.sectors), None).map_err(engine)?);
                sim.created = true;
                sim.parts.clear();
                Ok((0, format!("Create a new GPT partition table on \"{disk_name}\" ({}).{what}", size_text(disk.bytes())), destructive, None))
            }
            Op::Add { first, sectors, type_guid, name } => {
                let Some(t) = sim.table.as_mut() else { return Err(Refusal::NoTable) };
                if is_system(*type_guid) { return Err(Refusal::SystemType); }
                if name.encode_utf16().count() > 36 { return Err(Refusal::NameTooLong); }
                if *sectors == 0 { return Err(Refusal::Empty); }
                if first % MIB != 0 || sectors % MIB != 0 { return Err(Refusal::Unaligned); }
                let last = first.checked_add(sectors - 1).ok_or(Refusal::OutsideDisk)?;
                if *first < t.header.first_usable_lba || last > t.header.last_usable_lba { return Err(Refusal::OutsideDisk); }
                if let Some(o) = sim.parts.iter().find(|s| *first < s.p.end() && s.p.first <= last) { return Err(Refusal::Overlaps(o.p.slot)); }
                let mut opts = CreatePartitionOptions::new(*type_guid, name, *sectors);
                opts.first_lba = Some(*first);
                opts.align_1mib = true;
                let slot = t.create_partition(opts).map_err(engine)?;
                let uid = sim.next_uid; sim.next_uid += 1;
                let p = Part { slot, first: *first, sectors: *sectors, type_guid: *type_guid, name: name.clone(), content: Content::Empty };
                let type_name = p.type_name();
                sim.parts.push(SimPart { uid, p, new: true, inner: None });
                Ok((slot, format!("Add partition {} \"{}\" ({}), {} from {} to {}.", slot + 1, name, type_name, at(*sectors), at(*first), at(first + sectors)), false, Some(uid)))
            }
            Op::Delete { slot } => {
                let i = Self::idx(sim, *slot)?;
                sim.table.as_mut().unwrap().delete_partition(*slot).map_err(engine)?;
                let s = sim.parts.remove(i);
                let d = s.p.content.holds_data();
                Ok((*slot, format!("Delete partition {} \"{}\" ({}, {}){}.", slot + 1, s.p.name, at(s.p.sectors), s.p.content.name(), if d { "; its data is lost" } else { "" }), d, None))
            }
            Op::Resize { slot, sectors } => {
                let i = Self::idx(sim, *slot)?;
                let s = sim.parts[i].clone();
                if *sectors == s.p.sectors { return Err(Refusal::Unchanged); }
                if *sectors == 0 { return Err(Refusal::Empty); }
                if sectors % MIB != 0 { return Err(Refusal::Unaligned); }
                let grow = *sectors > s.p.sectors;
                let c = s.p.content;
                let note = if grow {
                    match c {
                        Content::Empty => String::new(),
                        Content::Fs(FilesystemType::Unknown) => "; what it holds is not grown".into(),
                        Content::Fs(FilesystemType::SexFs) => "; sexfs takes the new space at its next mount".into(),
                        Content::Fs(FilesystemType::EncryptedBdsm) if s.inner.is_some_and(Fs::grows) => "; the sexfs inside takes the new space at its next unlock".into(),
                        other => return Err(Refusal::CannotGrow(other)),
                    }
                } else if c != Content::Empty { return Err(Refusal::CannotShrink(c)); } else { String::new() };
                let t = sim.table.as_mut().unwrap();
                if grow {
                    if let Some(o) = sim.parts.iter().find(|o| o.uid != s.uid && s.p.end() < o.p.end() && o.p.first < s.p.first + sectors) { return Err(Refusal::Overlaps(o.p.slot)); }
                    t.grow_partition(*slot, sectors - s.p.sectors).map_err(engine)?;
                } else {
                    t.shrink_partition(*slot, s.p.sectors - sectors, MIB).map_err(engine)?;
                }
                sim.parts[i].p.sectors = *sectors;
                Ok((*slot, format!("{} partition {} \"{}\" from {} to {}{}.", if grow { "Grow" } else { "Shrink" }, slot + 1, s.p.name, at(s.p.sectors), at(*sectors), note), false, None))
            }
            Op::Format { slot, fs, label } => {
                let i = Self::idx(sim, *slot)?;
                let s = sim.parts[i].clone();
                Self::fs_fits(*fs, s.p.sectors, label)?;
                sim.parts[i].p.content = fs.content();
                sim.parts[i].inner = None;
                let l = if label.is_empty() { String::new() } else { format!(" labelled \"{label}\"") };
                Ok((*slot, format!("Format partition {} \"{}\" ({}) as {}{}{}.", slot + 1, s.p.name, at(s.p.sectors), fs.name(), l, Self::lose(s.p.content)), s.p.content.holds_data(), Some(s.uid)))
            }
            Op::Encrypt { slot, password, inner } => {
                let i = Self::idx(sim, *slot)?;
                let s = sim.parts[i].clone();
                if password.0.is_empty() { return Err(Refusal::EmptyPassword); }
                let room = s.p.sectors.saturating_sub(8);
                if room < MIB { return Err(Refusal::TooSmall { fs: "An encrypted partition", min: 2 * MIB }); }
                if let Some((fs, label)) = inner { Self::fs_fits(*fs, room, label).map_err(|e| match e { Refusal::TooSmall { fs, min } => Refusal::TooSmall { fs, min: min + MIB }, e => e })?; }
                sim.parts[i].p.content = Content::Fs(FilesystemType::EncryptedBdsm);
                sim.parts[i].inner = inner.as_ref().map(|(f, _)| *f);
                let then = match inner { Some((fs, l)) if !l.is_empty() => format!(", then format it inside as {} labelled \"{l}\"", fs.name()), Some((fs, _)) => format!(", then format it inside as {}", fs.name()), None => String::new() };
                Ok((*slot, format!("Encrypt partition {} \"{}\" ({}) with a password (XTS-AES-256, Argon2id {} MiB){}{}.", slot + 1, s.p.name, at(s.p.sectors), COST.memory_kib / 1024, then, Self::lose(s.p.content)), s.p.content.holds_data(), Some(s.uid)))
            }
        }
    }
}

/// Where an apply is, for a progress bar.
#[derive(Clone, Copy, Debug)]
pub struct Progress<'a> {
    /// 1-based, of `steps`.
    pub step: usize,
    pub steps: usize,
    pub text: &'a str,
    /// 0 when the step starts, 100 when it is done.
    pub pct: u64,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub table_written: bool,
    /// Data steps run before the table write, and after it.
    pub before_table: usize,
    pub after_table: usize,
    /// The disk as it reads now.
    pub disk: Disk,
}
#[derive(Debug)]
pub enum ApplyError {
    /// The disk is not what the plan was made against any more: read it again.
    Changed,
    Engine(CorsetError),
}
impl From<CorsetError> for ApplyError { fn from(e: CorsetError) -> Self { ApplyError::Engine(e) } }

enum Work<'p> {
    /// Zeroes a new partition's first 64 KiB, so nothing an earlier table left there is
    /// taken for a filesystem.
    Wipe { first: u64, text: String },
    Format { first: u64, sectors: u64, fs: Fs, label: &'p str, text: &'p str },
    Encrypt { first: u64, sectors: u64, password: &'p [u8], inner: Option<(Fs, &'p str)>, text: &'p str },
}
impl Work<'_> {
    fn range(&self) -> (u64, u64) {
        match *self { Work::Wipe { first, .. } => (first, first + 128), Work::Format { first, sectors, .. } | Work::Encrypt { first, sectors, .. } => (first, first + sectors) }
    }
    fn text(&self) -> &str { match self { Work::Wipe { text, .. } => text, Work::Format { text, .. } | Work::Encrypt { text, .. } => text } }
}

fn make_fs<D: BlockIo + ?Sized>(dev: &mut D, fs: Fs, label: &str) -> Result<(), CorsetError> {
    match fs {
        Fs::Fat32 => format_fat32(dev, Fat32FormatOptions { volume_label: label, sectors_per_cluster: None }),
        Fs::ExFat => format_exfat(dev, ExFatFormatOptions { volume_label: label }),
        Fs::Sexfs => format_sexfs(dev),
    }
}

/// Applies `plan` to `dev`, which must be the disk the plan was made from. `random` fills
/// buffers with fresh random bytes (GUIDs, salts, keys).
pub fn apply<D: BlockIo + ?Sized>(dev: &mut D, plan: &Plan, random: &mut dyn FnMut(&mut [u8]), progress: &mut dyn FnMut(Progress<'_>)) -> Result<Report, ApplyError> {
    let now = Disk::read(dev, &plan.disk.name)?;
    if now.sectors != plan.disk.sectors || now.fingerprint != plan.disk.fingerprint || now.parts != plan.disk.parts { return Err(ApplyError::Changed); }
    let sim = &plan.sim;
    let table_changes = plan.steps.iter().any(|s| matches!(s.op, Op::CreateTable | Op::Add { .. } | Op::Delete { .. } | Op::Resize { .. }));
    // The data each step writes, into where its partition ends up.
    let mut work: Vec<Work<'_>> = sim.parts.iter().filter(|s| s.new)
        .map(|s| Work::Wipe { first: s.p.first, text: format!("Clear the start of partition {} \"{}\".", s.p.slot + 1, s.p.name) }).collect();
    for st in &plan.steps {
        let Some(part) = st.uid.and_then(|u| sim.parts.iter().find(|s| s.uid == u)) else { continue };
        let (first, sectors) = (part.p.first, part.p.sectors);
        match &st.op {
            Op::Format { fs, label, .. } => work.push(Work::Format { first, sectors, fs: *fs, label, text: &st.text }),
            Op::Encrypt { password, inner, .. } => work.push(Work::Encrypt { first, sectors, password: &password.0, inner: inner.as_ref().map(|(f, l)| (*f, l.as_str())), text: &st.text }),
            _ => {}
        }
    }
    // Space the disk's current table leaves free can be written before the table changes.
    let used: Vec<(u64, u64)> = if plan.disk.table.is_some() { plan.disk.parts.iter().map(|p| (p.first, p.end())).collect() }
        else if plan.disk.whole.holds_data() { vec![(0, plan.disk.sectors)] } else { Vec::new() };
    let free_now = |w: &Work<'_>| { let (a, b) = w.range(); !used.iter().any(|(x, y)| a < *y && *x < b) };
    let (before, after): (Vec<Work<'_>>, Vec<Work<'_>>) = work.into_iter().partition(free_now);
    let steps = before.len() + after.len() + table_changes as usize;
    let mut n = 0;
    #[allow(clippy::too_many_arguments)]
    fn run<D: BlockIo + ?Sized>(dev: &mut D, w: &Work<'_>, n: &mut usize, steps: usize, random: &mut dyn FnMut(&mut [u8]), progress: &mut dyn FnMut(Progress<'_>)) -> Result<(), CorsetError> {
        *n += 1;
        progress(Progress { step: *n, steps, text: w.text(), pct: 0 });
        match *w {
            Work::Wipe { first, .. } => dev.write_sectors(first, &vec![0u8; 128 * SECTOR as usize])?,
            Work::Format { first, sectors, fs, label, .. } => make_fs(&mut PartitionIo::new(dev, first, sectors)?, fs, label)?,
            Work::Encrypt { first, sectors, password, inner, .. } => {
                let mut salt = [0u8; 32]; random(&mut salt);
                let mut master = [0u8; MASTER_KEY_BYTES]; random(&mut master);
                let mut part = PartitionIo::new(dev, first, sectors)?;
                let r = format_encrypted_volume(&mut part, password, &salt, Some(&master), Some(COST)).and_then(|_| match inner {
                    Some((fs, label)) => make_fs(&mut EncryptedBlockIo::new(&mut part, &master)?, fs, label),
                    None => Ok(()),
                });
                for b in master.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) } }
                r?
            }
        }
        progress(Progress { step: *n, steps, text: w.text(), pct: 100 });
        Ok(())
    }
    for w in &before { run(dev, w, &mut n, steps, random, progress)?; }
    dev.flush()?;
    if table_changes {
        let mut table = sim.table.clone().ok_or(CorsetError::InvalidParameter("no table"))?;
        n += 1;
        let text = "Write the partition table (backup copy, then the primary).";
        progress(Progress { step: n, steps, text, pct: 0 });
        // Fresh identities for a new disk and each new partition.
        let mut g = [0u8; 16];
        if sim.created { random(&mut g); table.header.disk_guid = Guid::from_entropy(g); }
        for s in sim.parts.iter().filter(|s| s.new) {
            random(&mut g);
            let e = &mut table.entries[s.p.slot];
            e.partition_guid = Guid::from_entropy(g);
            e.raw = e.encode(table.header.size_of_partition_entry as usize);
        }
        table.commit(dev)?;
        progress(Progress { step: n, steps, text, pct: 100 });
    }
    for w in &after { run(dev, w, &mut n, steps, random, progress)?; }
    dev.flush()?;
    Ok(Report { table_written: table_changes, before_table: before.len(), after_table: after.len(), disk: Disk::read(dev, &plan.disk.name)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use corset_core::{scan_disk_map, MemBlockDevice, SexVolumesAdapter};

    fn rng(seed: u64) -> impl FnMut(&mut [u8]) {
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        move |out: &mut [u8]| for b in out.iter_mut() { s ^= s << 13; s ^= s >> 7; s ^= s << 17; *b = s as u8; }
    }
    fn blank(mib: u64) -> MemBlockDevice { MemBlockDevice::new_512(mib * MIB).unwrap() }
    fn add(p: &mut Plan, first: u64, mib: u64, name: &str) -> usize {
        p.push(Op::Add { first: first * MIB, sectors: mib * MIB, type_guid: TYPE_MICROSOFT_BASIC_DATA, name: name.into() }).unwrap()
    }

    /// The gate's plan: GPT, a FAT32 and an exFAT partition, both mounted by sex-volumes.
    #[test]
    fn blank_disk_to_fat32_and_exfat() {
        let mut d = blank(256);
        let disk = Disk::read(&mut d, "usb0").unwrap();
        assert!(disk.table.is_none() && disk.whole == Content::Empty);
        let mut p = Plan::new(disk);
        assert_eq!(p.push(Op::Add { first: MIB, sectors: MIB, type_guid: TYPE_MICROSOFT_BASIC_DATA, name: "x".into() }).unwrap_err(), Refusal::NoTable);
        p.push(Op::CreateTable).unwrap();
        assert_eq!(p.free(), vec![(MIB, 254 * MIB)]);
        let fat = add(&mut p, 1, 100, "SHARED");
        p.push(Op::Format { slot: fat, fs: Fs::Fat32, label: "CORSETFAT".into() }).unwrap();
        let (first, room) = p.free()[0];
        assert_eq!((first, room), (101 * MIB, 154 * MIB));
        let ex = p.push(Op::Add { first, sectors: room, type_guid: TYPE_MICROSOFT_BASIC_DATA, name: "MORE".into() }).unwrap();
        p.push(Op::Format { slot: ex, fs: Fs::ExFat, label: "CORSETEX".into() }).unwrap();
        assert!(!p.destructive() && p.confirms(""));
        let text = p.confirmation();
        assert!(text.contains("On disk \"usb0\" (256 MiB):"), "{text}");
        assert!(text.contains("2. Add partition 1 \"SHARED\" (Basic data (FAT, exFAT)), 100 MiB from 1 MiB to 101 MiB."), "{text}");
        assert!(text.contains("3. Format partition 1 \"SHARED\" (100 MiB) as FAT32 labelled \"CORSETFAT\"."), "{text}");

        let mut seen = Vec::new();
        let r = apply(&mut d, &p, &mut rng(1), &mut |pr| if pr.pct == 0 { seen.push(pr.text.to_string()) }).unwrap();
        // A blank disk is all free space: everything lands before the table.
        assert!(r.table_written && r.before_table == 4 && r.after_table == 0, "{r:?}");
        assert_eq!(seen.last().unwrap(), "Write the partition table (backup copy, then the primary).");
        let map = scan_disk_map(&mut d).unwrap();
        assert!(map.primary_valid && map.backup_valid);
        assert_eq!(r.disk.parts.len(), 2);
        assert_eq!(r.disk.parts[0].content, Content::Fs(FilesystemType::Fat32));
        assert_eq!(r.disk.parts[1].content, Content::Fs(FilesystemType::ExFat));
        // As Files will find it: both volumes, by label.
        let found = sex_volumes::scan(&mut SexVolumesAdapter(&mut d)).unwrap();
        let labels: Vec<String> = found.iter().map(|f| f.volume.label().trim().to_string()).collect();
        assert_eq!(labels, ["CORSETFAT", "CORSETEX"]);
        assert_eq!((found[0].start, found[0].len), (MIB, 100 * MIB));
        // The partition GUIDs came from `random`, not the engine's fixed seed.
        assert_ne!(map.partitions[0].partition_guid, map.partitions[1].partition_guid);
    }

    #[test]
    fn destructive_plans_are_confirmed_by_name_and_refused_when_the_disk_changed() {
        let mut d = blank(128);
        let mut p = Plan::new(Disk::read(&mut d, "usb1").unwrap());
        p.push(Op::CreateTable).unwrap();
        let s = add(&mut p, 1, 64, "DATA");
        p.push(Op::Format { slot: s, fs: Fs::Fat32, label: "OLD".into() }).unwrap();
        apply(&mut d, &p, &mut rng(2), &mut |_| {}).unwrap();

        let disk = Disk::read(&mut d, "usb1").unwrap();
        assert_eq!(disk.parts[0].content, Content::Fs(FilesystemType::Fat32));
        let mut p = Plan::new(disk);
        // A FAT volume neither shrinks nor grows here.
        assert_eq!(p.push(Op::Resize { slot: 0, sectors: 40 * MIB }).unwrap_err(), Refusal::CannotShrink(Content::Fs(FilesystemType::Fat32)));
        assert!(matches!(p.push(Op::Resize { slot: 0, sectors: 80 * MIB }).unwrap_err(), Refusal::CannotGrow(_)));
        assert_eq!(p.push(Op::Format { slot: 0, fs: Fs::Sexfs, label: "x".into() }).unwrap_err(), Refusal::BadLabel);
        p.push(Op::Format { slot: 0, fs: Fs::ExFat, label: "NEW".into() }).unwrap();
        assert!(p.destructive());
        assert!(p.steps()[0].text.ends_with("; the FAT32 on it is lost."), "{}", p.steps()[0].text);
        assert!(!p.confirms("usb") && p.confirms(" usb1 "));
        assert!(p.confirmation().contains("Type the disk's name, \"usb1\", to apply."));

        // Something else wrote to the disk meanwhile: nothing is applied.
        let mut other = Plan::new(Disk::read(&mut d, "usb1").unwrap());
        other.push(Op::Delete { slot: 0 }).unwrap();
        apply(&mut d, &other, &mut rng(3), &mut |_| {}).unwrap();
        assert!(matches!(apply(&mut d, &p, &mut rng(4), &mut |_| {}), Err(ApplyError::Changed)));

        // Undo replays the rest; a format of an old partition is written after the table.
        let mut p = Plan::new(Disk::read(&mut d, "usb1").unwrap());
        assert!(p.partitions().is_empty() && p.has_table());
        let s = add(&mut p, 1, 32, "A");
        p.push(Op::Format { slot: s, fs: Fs::Sexfs, label: String::new() }).unwrap();
        p.push(Op::Resize { slot: s, sectors: 48 * MIB }).unwrap();
        assert!(p.pop().is_some());
        assert_eq!(p.partitions()[0].sectors, 32 * MIB);
        let r = apply(&mut d, &p, &mut rng(5), &mut |_| {}).unwrap();
        assert_eq!(r.disk.parts[0].content, Content::Fs(FilesystemType::SexFs));
    }

    #[test]
    fn encrypted_partition_opens_with_its_password_only() {
        let mut d = blank(64);
        let mut p = Plan::new(Disk::read(&mut d, "usb2").unwrap());
        p.push(Op::CreateTable).unwrap();
        let s = add(&mut p, 1, 40, "SAFE");
        assert_eq!(p.push(Op::Encrypt { slot: s, password: Secret(Vec::new()), inner: None }).unwrap_err(), Refusal::EmptyPassword);
        p.push(Op::Encrypt { slot: s, password: Secret(b"correct horse".to_vec()), inner: Some((Fs::ExFat, "VAULT".into())) }).unwrap();
        assert!(p.steps().last().unwrap().text.contains("then format it inside as exFAT labelled \"VAULT\""));
        let r = apply(&mut d, &p, &mut rng(6), &mut |_| {}).unwrap();
        assert_eq!(r.disk.parts[0].content, Content::Fs(FilesystemType::EncryptedBdsm));
        let mut part = PartitionIo::new(&mut d, MIB, 40 * MIB).unwrap();
        assert!(corset_core::unlock_encrypted_volume(&mut part, b"wrong horse").is_err());
        let mut inside = corset_core::unlock_encrypted_volume(&mut part, b"correct horse").unwrap();
        let found = sex_volumes::scan(&mut SexVolumesAdapter(&mut inside)).unwrap();
        assert_eq!(found[0].volume.label().trim(), "VAULT");
    }

    #[test]
    fn system_disks_and_bad_places_are_refused() {
        let mut d = blank(64);
        let mut t = GptEditor::init_blank(&mut d, None).unwrap();
        let mut o = CreatePartitionOptions::new(TYPE_SEXOS_DATA, "SexOS", 16 * MIB);
        o.first_lba = Some(MIB); o.align_1mib = true;
        t.create_partition(o).unwrap();
        t.commit(&mut d).unwrap();
        let disk = Disk::read(&mut d, "sys").unwrap();
        assert!(disk.is_system());
        let mut p = Plan::new(disk);
        assert_eq!(p.push(Op::CreateTable).unwrap_err(), Refusal::SystemDisk);
        assert_eq!(p.push(Op::Delete { slot: 0 }).unwrap_err(), Refusal::SystemPartition(0));
        assert_eq!(p.push(Op::Add { first: 20 * MIB, sectors: MIB, type_guid: TYPE_SEXOS_BOOT, name: "x".into() }).unwrap_err(), Refusal::SystemType);
        assert_eq!(p.push(Op::Add { first: 8 * MIB, sectors: 16 * MIB, type_guid: TYPE_LINUX_FILESYSTEM, name: "x".into() }).unwrap_err(), Refusal::Overlaps(0));
        assert_eq!(p.push(Op::Add { first: 20 * MIB + 1, sectors: MIB, type_guid: TYPE_LINUX_FILESYSTEM, name: "x".into() }).unwrap_err(), Refusal::Unaligned);
        assert_eq!(p.push(Op::Add { first: 60 * MIB, sectors: 8 * MIB, type_guid: TYPE_LINUX_FILESYSTEM, name: "x".into() }).unwrap_err(), Refusal::OutsideDisk);
        let s = p.push(Op::Add { first: 20 * MIB, sectors: 8 * MIB, type_guid: TYPE_LINUX_FILESYSTEM, name: "x".into() }).unwrap();
        assert!(matches!(p.push(Op::Format { slot: s, fs: Fs::Fat32, label: String::new() }).unwrap_err(), Refusal::TooSmall { fs: "FAT32", .. }));
        assert_eq!(size_text(3 << 29), "1.5 GiB");
    }
}
