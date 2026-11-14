//! SMART and NVMe health parsers against byte goldens (tests/goldens/*.hex: 512-byte pages
//! as drives return them, 32 bytes per line).
use corset_core::health::*;

fn page(name: &str) -> Vec<u8> {
    let text = std::fs::read_to_string(format!("{}/tests/goldens/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let hex: String = text.split_whitespace().collect();
    let b: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    assert_eq!(b.len(), 512, "{name}");
    b
}

#[test]
fn nvme_healthy_page() {
    let s = parse_nvme(&page("nvme_qemu_smart.hex")).unwrap();
    assert_eq!(s, NvmeSmart { critical_warning: 0, temperature_k: 323, available_spare: 100, spare_threshold: 10, percentage_used: 0,
        data_units_read: 12, data_units_written: 34, host_reads: 5678, host_writes: 910, busy_minutes: 0, power_cycles: 3, power_on_hours: 17,
        unsafe_shutdowns: 1, media_errors: 0, error_log_entries: 0, warning_temp_minutes: 0, critical_temp_minutes: 0, sensors_k: [0; 8] });
    let h = Health::from_nvme(&s);
    assert_eq!(h.verdict, Verdict::Good);
    assert_eq!((h.temperature_c, h.power_on_hours, h.percentage_used, h.spare_percent, h.media_errors), (Some(50), Some(17), Some(0), Some(100), Some(0)));
    assert!(h.reasons.is_empty());
}

#[test]
fn nvme_worn_page() {
    let s = parse_nvme(&page("nvme_worn_smart.hex")).unwrap();
    assert_eq!(s.data_units_read, (1u128 << 70) + 5, "128-bit counters are kept whole");
    assert_eq!((s.critical_warning, s.percentage_used, s.media_errors, s.error_log_entries), (0x09, 103, 12, 300));
    assert_eq!((s.warning_temp_minutes, s.critical_temp_minutes), (15, 2));
    assert_eq!(&s.sensors_k[..3], &[341, 352, 0]);
    let h = Health::from_nvme(&s);
    assert_eq!(h.verdict, Verdict::Failing);
    assert_eq!(h.temperature_c, Some(85));
    assert_eq!(h.reasons, [
        "Spare capacity is 4%, below the drive's threshold of 10%.",
        "The drive has put itself in read-only mode.",
        "103% of the rated endurance is used.",
        "12 unrecovered media errors.",
    ]);
    assert_eq!(parse_nvme(&[0u8; 100]), Err(HealthError::Short));
}

#[test]
fn ata_identify_page() {
    let id = parse_identify(&page("ata_identify_qemu.hex")).unwrap();
    assert_eq!(id, AtaIdentity { model: "QEMU HARDDISK".into(), serial: "QM00001".into(), firmware: "2.5+".into(), sectors: 524288,
        smart_supported: true, smart_enabled: true, rotation_rpm: Some(0) });
    let mut bad = page("ata_identify_qemu.hex");
    bad[100] ^= 1;
    assert_eq!(parse_identify(&bad), Err(HealthError::Checksum));
}

#[test]
fn ata_smart_healthy_qemu_page() {
    let s = parse_ata_smart(&page("ata_smart_qemu.hex"), Some(&page("ata_thresholds_qemu.hex"))).unwrap();
    assert_eq!(s.revision, 1);
    let ids: Vec<u8> = s.attributes.iter().map(|a| a.id).collect();
    assert_eq!(ids, [1, 3, 4, 5, 9, 12, 190]);
    let t = s.attributes.iter().find(|a| a.id == 190).unwrap();
    assert_eq!((t.value, t.worst, t.raw, t.threshold, t.prefail()), (0x45, 0x45, 0x1f1f_001f, Some(0x32), true));
    assert_eq!(s.attributes[3].name(), "Reallocated sectors");
    let id = parse_identify(&page("ata_identify_qemu.hex")).unwrap();
    let h = Health::from_ata(Some(&id), &s);
    assert_eq!(h.verdict, Verdict::Good, "{:?}", h.reasons);
    assert_eq!((h.temperature_c, h.power_on_hours, h.reallocated_sectors, h.pending_sectors), (Some(31), Some(0), Some(0), None));
}

#[test]
fn ata_smart_failing_page() {
    let s = parse_ata_smart(&page("ata_smart_failing.hex"), Some(&page("ata_thresholds_failing.hex"))).unwrap();
    assert_eq!(s.self_test_status, 0x77);
    let h = Health::from_ata(None, &s);
    assert_eq!(h.verdict, Verdict::Failing);
    assert_eq!((h.temperature_c, h.power_on_hours, h.power_cycles), (Some(41), Some(39410), Some(2210)));
    assert_eq!((h.reallocated_sectors, h.pending_sectors, h.uncorrectable_sectors), (Some(1872), Some(16), Some(8)));
    assert_eq!(h.reasons, [
        "Reallocated sectors (5) is at 32, at or below its threshold 36: the drive predicts failure.",
        "1872 sectors have been reallocated.",
        "16 sectors are waiting to be reallocated.",
        "8 sectors could not be read.",
        "The last self-test failed.",
    ]);
    // Without thresholds the counts still warn.
    let h = Health::from_ata(None, &parse_ata_smart(&page("ata_smart_failing.hex"), None).unwrap());
    assert_eq!(h.verdict, Verdict::Failing, "the failed self-test alone");
    let mut bad = page("ata_smart_failing.hex");
    bad[3] ^= 0x40;
    assert_eq!(parse_ata_smart(&bad, None), Err(HealthError::Checksum));
}

#[test]
fn driver_command_words() {
    // NUMDL = 512 / 4 - 1 = 127 in bits 16.., LID 02h.
    assert_eq!(nvme_get_log_cdw10(512), 0x007F_0002);
    assert_eq!((ATA_SMART, ATA_SMART_READ_DATA, ATA_SMART_LBA >> 8 & 0xFF, ATA_SMART_LBA >> 16), (0xB0, 0xD0, 0x4F, 0xC2));
}
