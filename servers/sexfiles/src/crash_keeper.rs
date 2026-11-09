// SPDX-License-Identifier: GPL-3.0-only
//! Keeps the kernel's crash records on disk, in the reserved ring before the program
//! store (`sex_pdx::crash_log::DISK_BASE_LBA`). At start it hands the kernel every record
//! kept from earlier starts, so one list shows them all; after that it writes each new
//! record as the kernel announces it. A record already on disk is never written twice.
use crate::history_journal::SectorIo;
use sex_pdx::crash_log::{self as crash, Record, RECORD_BYTES};

const SLOTS: usize = crash::DISK_SECTORS as usize;

pub struct Keeper {
    /// Each disk slot's record identity and store index, as last read or written.
    slots: [Option<(u64, u32, u64)>; SLOTS],
    ready: bool,
}

impl Keeper {
    pub const fn new() -> Self { Self { slots: [None; SLOTS], ready: false } }

    /// Reads the ring and gives the kernel what earlier starts kept.
    pub fn load(&mut self, io: &mut impl SectorIo) {
        // Sector by sector: a service stack has no room for the whole ring at once.
        let mut sector = [0u8; RECORD_BYTES];
        for i in 0..SLOTS {
            if io.read(crash::DISK_BASE_LBA + i as u64, &mut sector).is_err() {
                sex_pdx::serial_record!("[files.crash.ring] readable=0");
                return;
            }
            self.slots[i] = Record::decode(&sector).map(|r| (r.boot, r.seq, r.store_index));
        }
        self.ready = true;
        // Newest first: the kernel puts each in front of the last, so its list keeps the
        // order they happened in, and when it has no more room the oldest are the ones left out.
        let mut order: [usize; SLOTS] = core::array::from_fn(|i| i);
        order.sort_unstable_by_key(|&i| core::cmp::Reverse(self.slots[i].map_or(0, |s| s.2)));
        let mut adopted = 0;
        for i in order {
            if self.slots[i].is_none() { continue; }
            if io.read(crash::DISK_BASE_LBA + i as u64, &mut sector).is_err() { continue; }
            match crash::call(crash::OP_ADOPT, 0, sector.as_ptr() as u64).0 {
                crash::OK => adopted += 1,
                crash::FULL => break,
                _ => {}
            }
        }
        sex_pdx::serial_record!("[files.crash.ring] readable=1 kept={}", adopted);
    }

    /// Writes every kernel record the ring does not hold yet.
    pub fn sync(&mut self, io: &mut impl SectorIo) {
        if !self.ready { return; }
        let Some((count, _)) = crash::status() else { return };
        let mut wrote = 0;
        for index in 0..count {
            let Some(mut r) = crash::read(index) else { continue };
            if self.slots.iter().flatten().any(|&(boot, seq, _)| boot == r.boot && seq == r.seq) { continue; }
            let indexes: [Option<u64>; SLOTS] = core::array::from_fn(|i| self.slots[i].map(|s| s.2));
            let (slot, store_index) = crash::next_disk_slot(&indexes);
            r.store_index = store_index;
            let mut bytes = [0u8; RECORD_BYTES];
            r.encode(&mut bytes);
            if io.write(crash::DISK_BASE_LBA + slot as u64, &bytes).is_err() { break; }
            self.slots[slot] = Some((r.boot, r.seq, store_index));
            wrote += 1;
            sex_pdx::serial_record!("[files.crash.kept] slot={} index={} seq={} kind={} pd={}", slot, store_index, r.seq, r.kind as u8, r.pd);
        }
        if wrote > 0 { let _ = io.sync(); }
    }
}
