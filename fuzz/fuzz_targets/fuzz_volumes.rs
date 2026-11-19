// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_volumes::{Disk, Error, Result, SECTOR};

struct MemDisk<'a> {
    data: &'a [u8],
}

impl<'a> Disk for MemDisk<'a> {
    fn sectors(&self) -> u64 {
        (self.data.len() / SECTOR) as u64
    }

    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        let start = match (lba as usize).checked_mul(SECTOR) {
            Some(s) => s,
            None => return Err(Error::Io),
        };
        let end = match start.checked_add(buf.len()) {
            Some(e) => e,
            None => return Err(Error::Io),
        };
        if end <= self.data.len() {
            buf.copy_from_slice(&self.data[start..end]);
            Ok(())
        } else {
            Err(Error::Io)
        }
    }

    fn write(&mut self, _lba: u64, _buf: &[u8]) -> Result<()> {
        Err(Error::ReadOnly)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    // Only fuzz if there is at least one 512-byte sector
    if data.len() < SECTOR {
        return;
    }

    // 1. Scan partition table & all candidate filesystems
    let mut disk1 = MemDisk { data };
    if let Ok(mut volumes) = sex_volumes::scan(&mut disk1) {
        for v in volumes.iter_mut().take(4) {
            let _ = v.volume.format();
            let _ = v.volume.label();
            let _ = v.volume.read_only();
            let _ = v.volume.statfs(&mut disk1);
            if let Ok(entries) = v.volume.list(&mut disk1, b"") {
                for entry in entries.iter().take(8) {
                    let mut buf = [0u8; 1024];
                    let _ = v.volume.read(&mut disk1, entry.name.as_bytes(), 0, &mut buf);
                }
            }
        }
    }

    // 2. Direct probe of volume at sector 0 (FAT12/16/32, exFAT, ext4, NTFS)
    let mut disk2 = MemDisk { data };
    if let Ok(mut vol) = sex_volumes::probe(&mut disk2) {
        let _ = vol.format();
        let _ = vol.statfs(&mut disk2);
        let _ = vol.list(&mut disk2, b"");
    }

    // 3. Partition table parsing (MBR & GPT)
    let mut disk3 = MemDisk { data };
    let _ = sex_volumes::partition::table(&mut disk3);
});
