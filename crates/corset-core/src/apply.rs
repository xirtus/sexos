// SPDX-License-Identifier: GPL-3.0-only
//! Carrying a plan out, in an order a power cut cannot turn into a mess.
//!
//! 1. The disk is read again; if its table regions differ from the snapshot the plan was
//!    made against, nothing is written ([`Error::Changed`]).
//! 2. Every data write that lands in space the old table leaves free goes first, in plan
//!    order: new filesystems, encryption headers, moved copies, and a wipe of the first
//!    64 KiB of each new partition nothing else writes (so a stale signature in free space
//!    is not mistaken for a volume). The old table still describes the disk exactly.
//! 3. The table, if it changed: sex-disk writes the backup entries and header at the end of
//!    the disk first, then the primary entries and header, then the protective MBR. A cut
//!    between the two leaves one whole copy (the old primary or the new backup).
//! 4. The data writes that land inside old partitions (formatting a partition that already
//!    existed, growing into a deleted one's space), in plan order.
//!
//! The device is flushed between phases.
use crate::detect::{fingerprint, Disk};
use crate::format::{self, FsKind};
use crate::plan::{Op, Plan, SimPart};
use crate::{crypt, AsSexDisk, BlockDevice, Error, Guid, Window, ALIGN, SECTOR};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_disk::gpt::{Entry, Gpt};

/// Where an apply is, for a progress bar.
#[derive(Clone, Copy, Debug)]
pub struct Progress<'a> {
    /// 1-based, of `steps`.
    pub step: usize,
    pub steps: usize,
    pub text: &'a str,
    /// Sectors done of `total` within this step (both 0 for an instant step).
    pub done: u64,
    pub total: u64,
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

enum Work<'p> {
    Wipe { first: u64 },
    Copy { from: u64, to: u64, sectors: u64 },
    Format { first: u64, sectors: u64, fs: FsKind, label: &'p str },
    Encrypt { first: u64, sectors: u64, password: &'p [u8], cost: sex_crypt::Cost, inner: Option<(FsKind, &'p str)> },
}
impl Work<'_> {
    /// The sectors it writes.
    fn range(&self) -> (u64, u64) {
        match *self {
            Work::Wipe { first } => (first, first + 128),
            Work::Copy { to, sectors, .. } => (to, to + sectors),
            Work::Format { first, sectors, .. } | Work::Encrypt { first, sectors, .. } => (first, first + sectors),
        }
    }
}

fn final_part<'a>(sim: &'a [SimPart], uid: u32) -> Option<&'a SimPart> { sim.iter().find(|s| s.uid == uid) }

/// Applies `plan` to `dev`, which must be the disk the plan was made from. `random` fills
/// buffers with fresh random bytes (GUIDs, salts, keys, volume serials).
pub fn apply(dev: &mut dyn BlockDevice, plan: &Plan, random: &mut dyn FnMut(&mut [u8]), progress: &mut dyn FnMut(Progress<'_>)) -> Result<Report, Error> {
    let disk = plan.disk();
    if dev.sectors() != disk.sectors || fingerprint(dev)? != disk.fingerprint { return Err(Error::Changed); }
    let sim = plan.sim();

    // The data work, each against the partition's final place.
    let mut work: Vec<(String, Work<'_>)> = Vec::new();
    for (i, st) in plan.steps().iter().enumerate() {
        let Some(uid) = st.uid else { continue };
        let Some(p) = final_part(&sim.parts, uid) else { continue };
        let (first, sectors) = (p.p.first, p.p.sectors);
        let later_write = plan.steps()[i + 1..].iter().any(|s| s.uid == Some(uid) && matches!(s.op, Op::Format { .. } | Op::Encrypt { .. }));
        let w = match &st.op {
            Op::Add { .. } if !later_write => Work::Wipe { first },
            Op::Add { .. } => continue,
            Op::Move { slot, .. } => {
                let base = disk.partitions.iter().find(|b| b.slot == *slot).ok_or(Error::Changed)?;
                Work::Copy { from: base.first, to: first, sectors: base.sectors.min(sectors) }
            }
            // A later format or encryption of the same partition replaces this one.
            Op::Format { .. } | Op::Encrypt { .. } if later_write => continue,
            Op::Format { fs, label, .. } => Work::Format { first, sectors, fs: *fs, label },
            Op::Encrypt { password, cost, inner, .. } => Work::Encrypt { first, sectors, password: &password.0, cost: *cost, inner: inner.as_ref().map(|(f, l)| (*f, l.as_str())) },
            _ => continue,
        };
        work.push((st.text.clone(), w));
    }

    // The table the plan ends with.
    let mut gpt = match (&disk.gpt, sim.created || disk.gpt.is_none()) {
        (Some(g), false) => g.clone(),
        _ => {
            let mut g = [0u8; 16]; random(&mut g);
            Gpt::new_empty(disk.sectors, Guid::random(g)).map_err(|_| Error::Format)?
        }
    };
    let size = gpt.header.entry_size as usize;
    // Entries of partitions the plan deleted are cleared; the rest are kept byte for byte
    // apart from the fields the plan changes.
    for (slot, e) in gpt.entries.iter_mut().enumerate() {
        if !sim.parts.iter().any(|s| !s.new && s.p.slot == slot) { e.raw.fill(0); }
    }
    for s in &sim.parts {
        let e = &mut gpt.entries[s.p.slot];
        if s.new || sim.created {
            let mut g = [0u8; 16]; random(&mut g);
            *e = Entry::new(size, s.p.type_guid, Guid::random(g), s.p.first, s.p.end() - 1, &s.p.name);
        } else {
            e.raw[0..16].copy_from_slice(&s.p.type_guid.0);
            e.raw[32..40].copy_from_slice(&s.p.first.to_le_bytes());
            e.raw[40..48].copy_from_slice(&(s.p.end() - 1).to_le_bytes());
            e.raw[56..128].fill(0);
            for (i, u) in s.p.name.encode_utf16().take(36).enumerate() { e.raw[56 + 2 * i..58 + 2 * i].copy_from_slice(&u.to_le_bytes()); }
        }
        e.raw[48..56].copy_from_slice(&s.p.attrs.to_le_bytes());
    }
    gpt.check().map_err(|_| Error::Corrupt)?;
    let table_changes = match &disk.gpt { Some(old) => sim.created || old.entries != gpt.entries || old.from_backup, None => true };

    // Space the old table uses: data written there must wait for the new table.
    let used: Vec<(u64, u64)> = match &disk.gpt {
        Some(_) => disk.partitions.iter().map(|p| (p.first, p.end())).collect(),
        None if disk.layout == sex_disk::probe::Layout::Blank => Vec::new(),
        None => vec![(0, disk.sectors)],
    };
    let in_used = |w: &Work<'_>| { let (a, b) = w.range(); used.iter().any(|(x, y)| a < *y && *x < b) };
    let (before, after): (Vec<_>, Vec<_>) = work.into_iter().partition(|(_, w)| !in_used(w));
    let steps = before.len() + after.len() + table_changes as usize;
    let mut n = 0;
    for (text, w) in &before { n += 1; run(dev, w, random, &mut |d, t| progress(Progress { step: n, steps, text, done: d, total: t }))?; }
    dev.flush()?;
    if table_changes {
        n += 1;
        progress(Progress { step: n, steps, text: "Write the partition table (backup copy, then primary)", done: 0, total: 0 });
        gpt.write(&mut AsSexDisk(dev)).map_err(|e| if e == sex_disk::Error::Io { Error::Io } else { Error::Corrupt })?;
        dev.flush()?;
    }
    for (text, w) in &after { n += 1; run(dev, w, random, &mut |d, t| progress(Progress { step: n, steps, text, done: d, total: t }))?; }
    dev.flush()?;
    Ok(Report { table_written: table_changes, before_table: before.len(), after_table: after.len(), disk: Disk::read(dev, &disk.name)? })
}

fn run(dev: &mut dyn BlockDevice, w: &Work<'_>, random: &mut dyn FnMut(&mut [u8]), report: &mut dyn FnMut(u64, u64)) -> Result<(), Error> {
    match *w {
        Work::Wipe { first } => { report(0, 0); dev.write(first, &[0u8; 128 * SECTOR]) }
        Work::Copy { from, to, sectors } => {
            let mut buf = vec![0u8; ALIGN as usize * SECTOR];
            let mut done = 0;
            while done < sectors {
                let k = (sectors - done).min(ALIGN);
                let b = &mut buf[..k as usize * SECTOR];
                dev.read(from + done, b)?;
                dev.write(to + done, b)?;
                done += k;
                report(done, sectors);
            }
            Ok(())
        }
        Work::Format { first, sectors, fs, label } => {
            report(0, sectors);
            let mut serial = [0u8; 4]; random(&mut serial);
            format::format(&mut Window::new(dev, first, sectors)?, fs, label, u32::from_le_bytes(serial))?;
            report(sectors, sectors);
            Ok(())
        }
        Work::Encrypt { first, sectors, password, cost, inner } => {
            report(0, sectors);
            let mut salt = [0u8; 32]; random(&mut salt);
            let mut master = [0u8; sex_crypt::KEY]; random(&mut master);
            let mut part = Window::new(dev, first, sectors)?;
            crypt::create(&mut part, password, &salt, &master, cost)?;
            if let Some((fs, label)) = inner {
                let mut serial = [0u8; 4]; random(&mut serial);
                format::format(&mut crypt::CryptDev::with_key(&mut part, &master), fs, label, u32::from_le_bytes(serial))?;
            }
            for b in master.iter_mut() { unsafe { core::ptr::write_volatile(b, 0) } }
            report(sectors, sectors);
            Ok(())
        }
    }
}
