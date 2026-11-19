// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests for NVMe and ATA SMART parsing, diagnostics, and health evaluation.

use corset_core::smart::{
    AtaSmartData, DeviceProtocol, NvmeSmartLog, OverallHealth,
    ATA_ATTR_PENDING_SECTORS, ATA_ATTR_POWER_CYCLES, ATA_ATTR_POWER_ON_HOURS,
    ATA_ATTR_REALLOCATED_SECTORS, ATA_ATTR_TEMPERATURE,
    ATA_ATTR_UNCORRECTABLE_SECTORS, NVME_WARN_MEDIA_RELIABILITY, NVME_WARN_SPARE_BELOW_THRESHOLD,
    NVME_WARN_TEMPERATURE,
};

#[test]
fn test_nvme_smart_healthy_page() {
    let mut buf = [0u8; 512];
    buf[0] = 0; // No critical warnings
    // Temperature: 298 Kelvin = 25 Celsius
    buf[1..3].copy_from_slice(&298u16.to_le_bytes());
    buf[3] = 100; // 100% available spare
    buf[4] = 10;  // 10% spare threshold
    buf[5] = 2;   // 2% percentage used (wear)
    // Data units read: 5,000 units (5,000 * 1000 * 512 = 2,560,000,000 bytes)
    buf[32..48].copy_from_slice(&5000u128.to_le_bytes());
    // Data units written: 3,000 units
    buf[48..64].copy_from_slice(&3000u128.to_le_bytes());
    // Power on hours: 240 hours
    buf[128..144].copy_from_slice(&240u128.to_le_bytes());

    let log = NvmeSmartLog::parse(&buf).unwrap();
    assert_eq!(log.temperature_celsius(), 25);
    assert_eq!(log.available_spare_percent, 100);
    assert_eq!(log.data_bytes_read(), 2_560_000_000);
    assert_eq!(log.data_bytes_written(), 1_536_000_000);

    let report = log.analyze();
    assert_eq!(report.protocol, DeviceProtocol::Nvme);
    assert_eq!(report.overall, OverallHealth::Healthy);
    assert_eq!(report.wear_percent, Some(2));
    assert_eq!(report.temperature_celsius, Some(25));
    assert_eq!(report.power_on_hours, Some(240));
    assert!(report.findings.is_empty());
}

#[test]
fn test_nvme_smart_warning_page() {
    let mut buf = [0u8; 512];
    buf[0] = 0;
    // Temperature: 335 Kelvin = 62 Celsius (elevated)
    buf[1..3].copy_from_slice(&335u16.to_le_bytes());
    buf[3] = 80;
    buf[4] = 10;
    buf[5] = 90; // 90% wear (wear warning >= 85%)
    buf[144..160].copy_from_slice(&150u128.to_le_bytes()); // 150 unsafe shutdowns

    let log = NvmeSmartLog::parse(&buf).unwrap();
    let report = log.analyze();
    assert_eq!(report.overall, OverallHealth::Warning);
    assert!(report.findings.iter().any(|f| f.code == "WEAR_HIGH"));
    assert!(report.findings.iter().any(|f| f.code == "TEMPERATURE_ELEVATED"));
}

#[test]
fn test_nvme_smart_critical_page() {
    let mut buf = [0u8; 512];
    // Critical warning: spare below threshold + temperature
    buf[0] = NVME_WARN_SPARE_BELOW_THRESHOLD | NVME_WARN_TEMPERATURE | NVME_WARN_MEDIA_RELIABILITY;
    // Temperature: 353 Kelvin = 80 Celsius (critical)
    buf[1..3].copy_from_slice(&353u16.to_le_bytes());
    buf[3] = 5;  // 5% spare (below threshold)
    buf[4] = 10; // 10% threshold
    buf[5] = 102; // wear exceeded
    buf[160..176].copy_from_slice(&42u128.to_le_bytes()); // 42 media errors

    let log = NvmeSmartLog::parse(&buf).unwrap();
    let report = log.analyze();
    assert_eq!(report.overall, OverallHealth::Critical);
    assert!(report.findings.iter().any(|f| f.code == "NVME_SPARE_BELOW_THRESHOLD"));
    assert!(report.findings.iter().any(|f| f.code == "TEMPERATURE_HIGH"));
    assert!(report.findings.iter().any(|f| f.code == "MEDIA_INTEGRITY_ERRORS"));
    assert_eq!(report.media_errors, Some(42));
}

fn write_ata_attr(buf: &mut [u8; 512], idx: usize, id: u8, current: u8, worst: u8, raw: u64) {
    let off = 2 + idx * 12;
    buf[off] = id;
    buf[off + 1..off + 3].copy_from_slice(&0u16.to_le_bytes()); // flags
    buf[off + 3] = current;
    buf[off + 4] = worst;
    let raw_bytes = raw.to_le_bytes();
    buf[off + 5..off + 11].copy_from_slice(&raw_bytes[..6]);
}

#[test]
fn test_ata_smart_healthy_and_critical_attributes() {
    let mut buf = [0u8; 512];
    buf[0..2].copy_from_slice(&1u16.to_le_bytes()); // Revision 1

    // Attribute 1: Power On Hours (ID 9) = 1500 hrs
    write_ata_attr(&mut buf, 0, ATA_ATTR_POWER_ON_HOURS, 100, 100, 1500);
    // Attribute 2: Power Cycles (ID 12) = 45
    write_ata_attr(&mut buf, 1, ATA_ATTR_POWER_CYCLES, 100, 100, 45);
    // Attribute 3: Temperature (ID 194) = 34 C
    write_ata_attr(&mut buf, 2, ATA_ATTR_TEMPERATURE, 66, 60, 34);
    // Attribute 4: Reallocated Sectors (ID 5) = 0
    write_ata_attr(&mut buf, 3, ATA_ATTR_REALLOCATED_SECTORS, 100, 100, 0);

    let ata = AtaSmartData::parse(&buf).unwrap();
    let report = ata.analyze(None);
    assert_eq!(report.protocol, DeviceProtocol::Ata);
    assert_eq!(report.overall, OverallHealth::Healthy);
    assert_eq!(report.temperature_celsius, Some(34));
    assert_eq!(report.power_on_hours, Some(1500));
    assert_eq!(report.power_cycles, Some(45));

    // Now inject pending sectors and uncorrectable sectors
    write_ata_attr(&mut buf, 4, ATA_ATTR_PENDING_SECTORS, 95, 95, 8); // 8 pending sectors
    write_ata_attr(&mut buf, 5, ATA_ATTR_UNCORRECTABLE_SECTORS, 90, 90, 2); // 2 uncorrectable

    let ata_bad = AtaSmartData::parse(&buf).unwrap();
    let bad_report = ata_bad.analyze(None);
    assert_eq!(bad_report.overall, OverallHealth::Critical);
    assert!(bad_report.findings.iter().any(|f| f.code == "CURRENT_PENDING_SECTORS"));
    assert!(bad_report.findings.iter().any(|f| f.code == "OFFLINE_UNCORRECTABLE"));

    // Test threshold violation
    let mut thresh_buf = [0u8; 512];
    thresh_buf[2] = ATA_ATTR_POWER_ON_HOURS;
    thresh_buf[3] = 150; // Threshold 150 > current 100 => violation!
    let thresh_report = ata.analyze(Some(&thresh_buf));
    assert_eq!(thresh_report.overall, OverallHealth::Critical);
    assert!(thresh_report.findings.iter().any(|f| f.code == "ATA_THRESHOLD_VIOLATION"));
}
