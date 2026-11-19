// SPDX-License-Identifier: GPL-3.0-only
//! Non-destructive GPT partition editor with 1 MiB alignment, overlap checks, and atomic dual-table commits.

use crate::device::BlockIo;
use crate::error::CorsetError;
use crate::layout::{
    create_protective_mbr, crc32, read_entries, GptHeader, Guid, PartitionEntry,
    DEFAULT_ENTRY_COUNT, DEFAULT_ENTRY_SIZE,
};
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

/// Specification for creating a new partition.
#[derive(Clone, Debug)]
pub struct CreatePartitionOptions<'a> {
    pub type_guid: Guid,
    pub partition_guid: Option<Guid>,
    pub name: &'a str,
    pub sectors: u64,
    /// Explicit starting LBA, or `None` to allocate automatically in free space.
    pub first_lba: Option<u64>,
    /// Whether automatic placement should be aligned to 1 MiB (2048 sectors @ 512B). Default true.
    pub align_1mib: bool,
    pub attributes: u64,
}

impl<'a> CreatePartitionOptions<'a> {
    pub fn new(type_guid: Guid, name: &'a str, sectors: u64) -> Self {
        Self {
            type_guid,
            partition_guid: None,
            name,
            sectors,
            first_lba: None,
            align_1mib: true,
            attributes: 0,
        }
    }
}

/// GPT Partition Table Editor.
#[derive(Clone, Debug)]
pub struct GptEditor {
    pub header: GptHeader,
    pub entries: Vec<PartitionEntry>,
    pub sector_size: u32,
    pub total_sectors: u64,
    pub recovered_from_backup: bool,
}

impl GptEditor {
    /// Opens an existing GPT table on `dev`.
    /// Recovers transparently from backup GPT if primary GPT is corrupted.
    pub fn open<D: BlockIo + ?Sized>(dev: &mut D) -> Result<Self, CorsetError> {
        let sector_size = dev.sector_size();
        let total_sectors = dev.sector_count();
        if total_sectors < 68 {
            return Err(CorsetError::InvalidParameter("device too small for GPT"));
        }

        let mut sector_buf = vec![0u8; sector_size as usize];

        // 1. Try reading Primary GPT at LBA 1
        dev.read_sectors(1, &mut sector_buf)?;
        let primary_header_res = GptHeader::parse(&sector_buf, 1);
        let primary_res = match primary_header_res {
            Ok(h) => read_entries(dev, &h).map(|entries| (h, entries)),
            Err(e) => Err(e),
        };

        // 2. Try reading Backup GPT at LBA (total_sectors - 1)
        let backup_lba = total_sectors - 1;
        dev.read_sectors(backup_lba, &mut sector_buf)?;
        let backup_header_res = GptHeader::parse(&sector_buf, backup_lba);
        let backup_res = match backup_header_res {
            Ok(h) => read_entries(dev, &h).map(|entries| (h, entries)),
            Err(e) => Err(e),
        };

        match (primary_res, backup_res) {
            (Ok((header, entries)), _) => Ok(Self {
                header,
                entries,
                sector_size,
                total_sectors,
                recovered_from_backup: false,
            }),
            (Err(_), Ok((backup_header, backup_entries))) => {
                // Recovered from backup GPT! Synthesize primary header from backup.
                let entry_sectors = backup_header.entry_sectors();
                let primary_header = GptHeader {
                    my_lba: 1,
                    alternate_lba: backup_lba,
                    first_usable_lba: backup_header.first_usable_lba,
                    last_usable_lba: backup_header.last_usable_lba,
                    disk_guid: backup_header.disk_guid,
                    partition_entry_lba: 2,
                    number_of_partition_entries: backup_header.number_of_partition_entries,
                    size_of_partition_entry: backup_header.size_of_partition_entry,
                    partition_entry_array_crc32: backup_header.partition_entry_array_crc32,
                };
                let _ = entry_sectors;
                Ok(Self {
                    header: primary_header,
                    entries: backup_entries,
                    sector_size,
                    total_sectors,
                    recovered_from_backup: true,
                })
            }
            (Err(_), Err(_)) => Err(CorsetError::GptCorrupt("both primary and backup GPT are corrupt")),
        }
    }

    /// Initializes a blank GPT partitioning scheme covering `dev`.
    pub fn init_blank<D: BlockIo + ?Sized>(
        dev: &mut D,
        disk_guid: Option<Guid>,
    ) -> Result<Self, CorsetError> {
        let sector_size = dev.sector_size();
        let total_sectors = dev.sector_count();
        if total_sectors < 68 + 2048 {
            return Err(CorsetError::InvalidParameter("device too small to initialize GPT"));
        }

        let guid = disk_guid.unwrap_or_else(|| {
            Guid::from_entropy([
                0x42, 0x13, 0x5e, 0x70, 0x89, 0xab, 0xcd, 0xef,
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            ])
        });

        let entry_count = DEFAULT_ENTRY_COUNT;
        let entry_size = DEFAULT_ENTRY_SIZE;
        let entry_sectors = ((entry_count as u64) * (entry_size as u64)).div_ceil(sector_size as u64);

        let first_usable = 1 + 1 + entry_sectors; // LBA 0 (MBR) + LBA 1 (Primary Header) + entries
        let last_usable = total_sectors - 1 - entry_sectors - 1; // backup header + backup entries

        let header = GptHeader {
            my_lba: 1,
            alternate_lba: total_sectors - 1,
            first_usable_lba: first_usable,
            last_usable_lba: last_usable,
            disk_guid: guid,
            partition_entry_lba: 2,
            number_of_partition_entries: entry_count,
            size_of_partition_entry: entry_size,
            partition_entry_array_crc32: 0,
        };

        let mut entries = Vec::with_capacity(entry_count as usize);
        for i in 0..entry_count as usize {
            entries.push(PartitionEntry::new(
                i,
                entry_size as usize,
                Guid::ZERO,
                Guid::ZERO,
                0,
                0,
                0,
                "",
            ));
        }

        Ok(Self {
            header,
            entries,
            sector_size,
            total_sectors,
            recovered_from_backup: false,
        })
    }

    /// Create a new partition in the table.
    pub fn create_partition(&mut self, opts: CreatePartitionOptions<'_>) -> Result<usize, CorsetError> {
        if opts.sectors == 0 {
            return Err(CorsetError::InvalidParameter("partition size cannot be 0 sectors"));
        }

        let sectors_per_1mib = (1024 * 1024) / (self.sector_size as u64);

        // Determine starting LBA
        let first_lba = match opts.first_lba {
            Some(lba) => {
                if opts.align_1mib && lba % sectors_per_1mib != 0 {
                    return Err(CorsetError::InvalidParameter("specified LBA is not 1 MiB aligned"));
                }
                lba
            }
            None => {
                // Find first free space extent
                self.find_free_lba(opts.sectors, opts.align_1mib)?
            }
        };

        let last_lba = first_lba
            .checked_add(opts.sectors - 1)
            .ok_or(CorsetError::InvalidParameter("partition extent overflow"))?;

        if first_lba < self.header.first_usable_lba || last_lba > self.header.last_usable_lba {
            return Err(CorsetError::NoSpace("partition extent outside usable LBA bounds"));
        }

        // Verify NO overlap with any existing partition
        for existing in self.entries.iter().filter(|e| e.is_used()) {
            if first_lba <= existing.last_lba && existing.first_lba <= last_lba {
                return Err(CorsetError::PartitionOverlap {
                    first: first_lba,
                    last: last_lba,
                    existing_first: existing.first_lba,
                    existing_last: existing.last_lba,
                });
            }
        }

        // Find first unused partition slot
        let slot_idx = self
            .entries
            .iter()
            .position(|e| !e.is_used())
            .ok_or(CorsetError::NoSpace("no free partition entry slots (table full)"))?;

        let part_guid = opts.partition_guid.unwrap_or_else(|| {
            let mut seed = [0u8; 16];
            seed[..8].copy_from_slice(&(first_lba ^ 0x5E05_BD53_0000_u64).to_le_bytes());
            seed[8..16].copy_from_slice(&((slot_idx as u64) ^ 0xCAFE_BABE_DEAD_BEEF).to_le_bytes());
            Guid::from_entropy(seed)
        });

        self.entries[slot_idx] = PartitionEntry::new(
            slot_idx,
            self.header.size_of_partition_entry as usize,
            opts.type_guid,
            part_guid,
            first_lba,
            last_lba,
            opts.attributes,
            opts.name,
        );

        Ok(slot_idx)
    }

    /// Delete a partition by slot index.
    pub fn delete_partition(&mut self, index: usize) -> Result<(), CorsetError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(CorsetError::NotFound("partition index out of range"))?;
        if !entry.is_used() {
            return Err(CorsetError::NotFound("partition slot is already empty"));
        }
        *entry = PartitionEntry::new(
            index,
            self.header.size_of_partition_entry as usize,
            Guid::ZERO,
            Guid::ZERO,
            0,
            0,
            0,
            "",
        );
        Ok(())
    }

    /// Rename a partition.
    pub fn rename_partition(&mut self, index: usize, new_name: &str) -> Result<(), CorsetError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(CorsetError::NotFound("partition index out of range"))?;
        if !entry.is_used() {
            return Err(CorsetError::NotFound("partition slot is empty"));
        }
        entry.name = new_name.to_string();
        let enc = entry.encode(self.header.size_of_partition_entry as usize);
        entry.raw = enc;
        Ok(())
    }

    /// Change a partition's type GUID.
    pub fn change_partition_type(&mut self, index: usize, new_type: Guid) -> Result<(), CorsetError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(CorsetError::NotFound("partition index out of range"))?;
        if !entry.is_used() {
            return Err(CorsetError::NotFound("partition slot is empty"));
        }
        entry.type_guid = new_type;
        let enc = entry.encode(self.header.size_of_partition_entry as usize);
        entry.raw = enc;
        Ok(())
    }

    /// Change a partition's attribute flags.
    pub fn change_partition_flags(&mut self, index: usize, new_flags: u64) -> Result<(), CorsetError> {
        let entry = self
            .entries
            .get_mut(index)
            .ok_or(CorsetError::NotFound("partition index out of range"))?;
        if !entry.is_used() {
            return Err(CorsetError::NotFound("partition slot is empty"));
        }
        entry.attributes = new_flags;
        let enc = entry.encode(self.header.size_of_partition_entry as usize);
        entry.raw = enc;
        Ok(())
    }

    /// Grow a partition into adjacent trailing free space.
    pub fn grow_partition(&mut self, index: usize, additional_sectors: u64) -> Result<u64, CorsetError> {
        if additional_sectors == 0 {
            return Ok(self.entries[index].sector_count());
        }
        let (first, old_last) = {
            let entry = self
                .entries
                .get(index)
                .ok_or(CorsetError::NotFound("partition index out of range"))?;
            if !entry.is_used() {
                return Err(CorsetError::NotFound("partition slot is empty"));
            }
            (entry.first_lba, entry.last_lba)
        };

        let new_last = old_last
            .checked_add(additional_sectors)
            .ok_or(CorsetError::InvalidParameter("partition growth overflow"))?;

        if new_last > self.header.last_usable_lba {
            return Err(CorsetError::NoSpace("growing partition exceeds usable disk space"));
        }

        // Verify trailing space is free (no other partition occupies old_last + 1 ..= new_last)
        for (i, other) in self.entries.iter().enumerate() {
            if i != index && other.is_used() {
                if (old_last + 1) <= other.last_lba && other.first_lba <= new_last {
                    return Err(CorsetError::PartitionOverlap {
                        first: old_last + 1,
                        last: new_last,
                        existing_first: other.first_lba,
                        existing_last: other.last_lba,
                    });
                }
            }
        }

        let entry = &mut self.entries[index];
        entry.last_lba = new_last;
        let enc = entry.encode(self.header.size_of_partition_entry as usize);
        entry.raw = enc;

        Ok(new_last - first + 1)
    }

    /// Shrink a partition by `reduce_by_sectors`, refusing to shrink below `min_allowed_sectors`.
    pub fn shrink_partition(
        &mut self,
        index: usize,
        reduce_by_sectors: u64,
        min_allowed_sectors: u64,
    ) -> Result<u64, CorsetError> {
        if reduce_by_sectors == 0 {
            return Ok(self.entries[index].sector_count());
        }
        let (first, old_last) = {
            let entry = self
                .entries
                .get(index)
                .ok_or(CorsetError::NotFound("partition index out of range"))?;
            if !entry.is_used() {
                return Err(CorsetError::NotFound("partition slot is empty"));
            }
            (entry.first_lba, entry.last_lba)
        };

        let current_sectors = old_last - first + 1;
        if reduce_by_sectors >= current_sectors {
            return Err(CorsetError::PartitionShrinkTooSmall {
                requested: 0,
                minimum: min_allowed_sectors,
            });
        }

        let new_sectors = current_sectors - reduce_by_sectors;
        if new_sectors < min_allowed_sectors || new_sectors == 0 {
            return Err(CorsetError::PartitionShrinkTooSmall {
                requested: new_sectors,
                minimum: min_allowed_sectors,
            });
        }

        let new_last = first + new_sectors - 1;
        let entry = &mut self.entries[index];
        entry.last_lba = new_last;
        let enc = entry.encode(self.header.size_of_partition_entry as usize);
        entry.raw = enc;

        Ok(new_sectors)
    }

    /// Helper to find free LBA with alignment.
    fn find_free_lba(&self, sectors_needed: u64, align_1mib: bool) -> Result<u64, CorsetError> {
        let sectors_per_1mib = (1024 * 1024) / (self.sector_size as u64);
        let mut spans: Vec<(u64, u64)> = self
            .entries
            .iter()
            .filter(|e| e.is_used())
            .map(|e| (e.first_lba, e.last_lba))
            .collect();
        spans.sort_unstable();

        let mut cursor = self.header.first_usable_lba;
        for (start, end) in spans {
            if start > cursor {
                let candidate_start = if align_1mib {
                    cursor.next_multiple_of(sectors_per_1mib)
                } else {
                    cursor
                };
                if start > candidate_start && (start - candidate_start) >= sectors_needed {
                    return Ok(candidate_start);
                }
            }
            cursor = cursor.max(end + 1);
        }

        let last_usable_end = self.header.last_usable_lba + 1;
        if last_usable_end > cursor {
            let candidate_start = if align_1mib {
                cursor.next_multiple_of(sectors_per_1mib)
            } else {
                cursor
            };
            if last_usable_end > candidate_start && (last_usable_end - candidate_start) >= sectors_needed {
                return Ok(candidate_start);
            }
        }

        Err(CorsetError::NoSpace("not enough contiguous free space for partition"))
    }

    /// Commit the updated partition table atomically to `dev`.
    /// Recomputes entry array CRC32 and writes Backup GPT first, then Primary GPT and protective MBR.
    pub fn commit<D: BlockIo + ?Sized>(&mut self, dev: &mut D) -> Result<(), CorsetError> {
        let total_sectors = dev.sector_count();
        let entry_sectors = self.header.entry_sectors_for(self.sector_size);
        let entry_size = self.header.size_of_partition_entry as usize;
        let entry_count = self.header.number_of_partition_entries as usize;

        let total_entry_bytes = (entry_sectors as usize) * (self.sector_size as usize);
        let mut entry_bytes = vec![0u8; total_entry_bytes];

        for (i, entry) in self.entries.iter().enumerate().take(entry_count) {
            let raw = entry.encode(entry_size);
            let offset = i * entry_size;
            entry_bytes[offset..offset + entry_size].copy_from_slice(&raw[..entry_size]);
        }

        let used_bytes = entry_count * entry_size;
        let entries_crc32 = crc32(&entry_bytes[..used_bytes]);
        self.header.partition_entry_array_crc32 = entries_crc32;

        let backup_lba = total_sectors - 1;
        let backup_entries_lba = backup_lba - entry_sectors;

        // 1. Write Backup Partition Entries
        dev.write_sectors(backup_entries_lba, &entry_bytes)?;

        // 2. Write Backup GPT Header
        let backup_header = GptHeader {
            my_lba: backup_lba,
            alternate_lba: 1,
            first_usable_lba: self.header.first_usable_lba,
            last_usable_lba: self.header.last_usable_lba,
            disk_guid: self.header.disk_guid,
            partition_entry_lba: backup_entries_lba,
            number_of_partition_entries: self.header.number_of_partition_entries,
            size_of_partition_entry: self.header.size_of_partition_entry,
            partition_entry_array_crc32: entries_crc32,
        };
        let backup_sector = backup_header.encode();
        dev.write_sectors(backup_lba, &backup_sector)?;

        // 3. Write Primary Partition Entries at LBA 2
        let primary_entries_lba = 2;
        dev.write_sectors(primary_entries_lba, &entry_bytes)?;

        // 4. Write Primary GPT Header at LBA 1
        let primary_header = GptHeader {
            my_lba: 1,
            alternate_lba: backup_lba,
            first_usable_lba: self.header.first_usable_lba,
            last_usable_lba: self.header.last_usable_lba,
            disk_guid: self.header.disk_guid,
            partition_entry_lba: primary_entries_lba,
            number_of_partition_entries: self.header.number_of_partition_entries,
            size_of_partition_entry: self.header.size_of_partition_entry,
            partition_entry_array_crc32: entries_crc32,
        };
        let primary_sector = primary_header.encode();
        dev.write_sectors(1, &primary_sector)?;

        // 5. Write Protective MBR at LBA 0
        let mbr = create_protective_mbr(total_sectors);
        dev.write_sectors(0, &mbr)?;

        // 6. Flush changes to physical medium
        dev.flush()?;

        self.header = primary_header;
        self.recovered_from_backup = false;
        Ok(())
    }
}
