//! IMSM metadata reader. The fixtures are the last 17 sectors of member disks
//! made by scripts/make_imsm_raid.py; `mdadm --examine` reads each of them as a
//! valid IMSM member with "Checksum ... correct" and the same layout as below.
//! The end-to-end test makes whole members with a known pattern and reads the
//! volume back through `Volume::map`.

use sex_imsm::*;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}

#[test]
fn raid0_pair_is_one_volume_over_both_disks() {
    let a = from_tail(&fixture("r0.tail")).unwrap();
    let b = from_tail(&fixture("r1.tail")).unwrap();
    assert!(a.same_set(&b));
    assert_eq!(&a.version, b"1.0.00");
    assert_eq!(a.disks().len(), 2);
    assert_eq!(a.disks()[0].serial(), b"SEXRAID-0");
    assert_eq!(a.disks()[1].sectors, 131072);
    let v = a.volumes()[0];
    assert_eq!(v.name(), b"Volume0");
    assert_eq!((v.level, v.members, v.strip, v.start), (0, 2, 256, 0));
    assert_eq!(v.sectors, 241152);
    assert_eq!(v.per_member, 120576);
    assert_eq!(v.readable(0b11), Ok(()));
    assert_eq!(v.readable(0b01), Err(Unreadable::Failed));
}

#[test]
fn raid0_strips_alternate_between_members() {
    let v = from_tail(&fixture("r0.tail")).unwrap().volumes()[0];
    assert_eq!(v.map(0, 8, 0b11), Some(Extent { member: 0, lba: 0, sectors: 8 }));
    assert_eq!(v.map(250, 100, 0b11), Some(Extent { member: 0, lba: 250, sectors: 6 }));
    assert_eq!(v.map(256, 1, 0b11), Some(Extent { member: 1, lba: 0, sectors: 1 }));
    assert_eq!(v.map(512 + 3, 1, 0b11), Some(Extent { member: 0, lba: 256 + 3, sectors: 1 }));
    assert_eq!(v.map(v.sectors - 1, 1, 0b11), Some(Extent { member: 1, lba: v.per_member - 1, sectors: 1 }));
    assert_eq!(v.map(v.sectors, 1, 0b11), None);
    assert_eq!(v.map(0, 1, 0b10), None, "a RAID 0 with a member missing is not read");
}

#[test]
fn matrix_raid_has_two_volumes_and_a_two_sector_mpb() {
    let tail = fixture("m0.tail");
    assert_eq!(mpb_sectors(&tail[tail.len() - 1024..tail.len() - 512]), Ok(2));
    let m = from_tail(&tail).unwrap();
    assert_eq!(m.mpb_bytes, 648);
    assert_eq!(m.volumes().len(), 2);
    let (r0, r1) = (m.volumes()[0], m.volumes()[1]);
    assert_eq!((r0.level, r0.strip, r0.start, r0.sectors), (0, 32, 0, 120640));
    assert_eq!((r1.level, r1.start, r1.sectors), (1, 60320, 60320));
    assert_eq!(r1.map(10, 4, 0b10), Some(Extent { member: 1, lba: 60330, sectors: 4 }));
    assert!(from_tail(&fixture("m1.tail")).unwrap().same_set(&m));
    // The serial the drive reports, padded the way NVMe pads it.
    assert_eq!(m.disk_for_serial(b"S5H9NS0N123456X-1    "), Some(1));
    assert_eq!(m.disk_for_serial(b"S5H9NS0N123456X-2    "), None);
}

#[test]
fn raid1_reads_from_either_mirror_and_serial_colons_become_semicolons() {
    let m = from_tail(&fixture("a0.tail")).unwrap();
    assert_eq!(&m.version, b"1.1.00");
    assert_eq!(m.disks()[0].serial(), b"WD-WX12;34-0");
    assert_eq!(m.disk_for_serial(b"  WD-WX12:34-0"), Some(0));
    let v = m.volumes()[0];
    assert_eq!(v.map(7, 9, 0b11), Some(Extent { member: 0, lba: 7, sectors: 9 }));
    assert_eq!(v.map(7, 9, 0b10), Some(Extent { member: 1, lba: 7, sectors: 9 }));
    assert_eq!(v.readable(0), Err(Unreadable::NoMember));
}

#[test]
fn damaged_or_foreign_tails_are_refused() {
    let blank = vec![0u8; TAIL_SECTORS * SECTOR];
    assert_eq!(from_tail(&blank), Err(Error::NotMember));
    let mut bad = fixture("r0.tail");
    let anchor = bad.len() - 1024;
    bad[anchor + 0x100] ^= 1;
    assert_eq!(from_tail(&bad), Err(Error::BadChecksum));
    let mut huge = fixture("r0.tail");
    huge[anchor + 0x24..anchor + 0x28].copy_from_slice(&(64u32 * 1024).to_le_bytes());
    assert_eq!(from_tail(&huge), Err(Error::BadSize));
    // A two-sector MPB with only its anchor read cannot be assembled.
    let m0 = fixture("m0.tail");
    assert_eq!(from_tail(&m0[m0.len() - 1024..]), Err(Error::Truncated));
}

#[test]
fn migrating_volumes_are_not_read() {
    let mut v = from_tail(&fixture("r0.tail")).unwrap().volumes()[0];
    v.migrating = true;
    assert_eq!(v.readable(0b11), Err(Unreadable::Migrating));
    assert_eq!(v.map(0, 1, 0b11), None);
}

#[test]
fn generated_members_read_back_the_volume_pattern() {
    let dir = std::env::temp_dir().join(format!("sex-imsm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (level, strip, disks) in [(0, "64", 2), (0, "4", 3), (1, "128", 2)] {
        let paths: Vec<PathBuf> = (0..disks).map(|i| dir.join(format!("l{}s{}d{}.img", level, strip, i))).collect();
        let status = std::process::Command::new("python3")
            .arg(root.join("scripts/make_imsm_raid.py"))
            .args(&paths).args(["--mib", "16", "--pattern", "--level", &level.to_string(), "--strip-kib", strip])
            .status().expect("python3");
        assert!(status.success());
        let images: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let metas: Vec<Metadata> = images.iter().map(|img| from_tail(&img[img.len() - TAIL_SECTORS * SECTOR..]).unwrap()).collect();
        let v = metas[0].volumes()[0];
        assert!(metas.iter().all(|m| m.same_set(&metas[0])));
        let present = (1u8 << disks) - 1;
        let mut lba = 0u64;
        let check = v.sectors.min(4096);
        while lba < check {
            let e = v.map(lba, (check - lba).min(200), present).unwrap();
            let slot_disk = v.order[e.member as usize] as usize;
            for k in 0..e.sectors {
                let at = ((e.lba + k) as usize) * SECTOR;
                let word = u64::from_le_bytes(images[slot_disk][at..at + 8].try_into().unwrap());
                assert_eq!(word, (lba + k) ^ 0x5345_5852_4149_4400, "level {} strip {} lba {}", level, strip, lba + k);
            }
            lba += e.sectors;
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}
