// SPDX-License-Identifier: GPL-3.0-only
//! NVMe and ATA SMART disk health analyzers and diagnostic reports.

use crate::error::CorsetError;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Disk health summary status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverallHealth {
    Healthy,
    Warning,
    Critical,
}

impl OverallHealth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Warning => "WARNING",
            Self::Critical => "CRITICAL",
        }
    }
}

/// Finding severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingSeverity {
    Info,
    Warning,
    Critical,
}

/// Structured diagnostic finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthFinding {
    pub severity: FindingSeverity,
    pub code: &'static str,
    pub description: String,
}

/// Protocol origin of the SMART data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceProtocol {
    Nvme,
    Ata,
}

/// Structured disk health report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthReport {
    pub protocol: DeviceProtocol,
    pub overall: OverallHealth,
    pub wear_percent: Option<u8>,
    pub temperature_celsius: Option<i16>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub media_errors: Option<u64>,
    pub data_bytes_read: Option<u128>,
    pub data_bytes_written: Option<u128>,
    pub findings: Vec<HealthFinding>,
}

// -----------------------------------------------------------------------------
// NVMe SMART / Health Information Log Page (Log ID 0x02)
// -----------------------------------------------------------------------------

/// Critical warning bit flags.
pub const NVME_WARN_SPARE_BELOW_THRESHOLD: u8 = 1 << 0;
pub const NVME_WARN_TEMPERATURE: u8 = 1 << 1;
pub const NVME_WARN_MEDIA_RELIABILITY: u8 = 1 << 2;
pub const NVME_WARN_READ_ONLY: u8 = 1 << 3;
pub const NVME_WARN_VOLATILE_BACKUP_FAILED: u8 = 1 << 4;
pub const NVME_WARN_PMR_UNRELIABLE: u8 = 1 << 5;

/// Parsed NVMe SMART / Health Information log page (512 bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NvmeSmartLog {
    pub critical_warning: u8,
    pub composite_temperature_kelvin: u16,
    pub available_spare_percent: u8,
    pub available_spare_threshold_percent: u8,
    pub percentage_used: u8,
    pub endurance_group_critical_warning: u8,
    pub data_units_read: u128,
    pub data_units_written: u128,
    pub host_read_commands: u128,
    pub host_write_commands: u128,
    pub controller_busy_time_minutes: u128,
    pub power_cycles: u128,
    pub power_on_hours: u128,
    pub unsafe_shutdowns: u128,
    pub media_errors: u128,
    pub num_err_log_entries: u128,
    pub warning_temp_time_minutes: u32,
    pub critical_temp_time_minutes: u32,
    pub temp_sensors: [u16; 8],
}

impl NvmeSmartLog {
    /// Parse from a raw 512-byte NVMe SMART log buffer.
    pub fn parse(buf: &[u8]) -> Result<Self, CorsetError> {
        if buf.len() < 512 {
            return Err(CorsetError::SmartDataCorrupt("NVMe log buffer less than 512 bytes"));
        }

        let read_u16 = |off: usize| u16::from_le_bytes(buf[off..off + 2].try_into().unwrap());
        let read_u32 = |off: usize| u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        let read_u128 = |off: usize| u128::from_le_bytes(buf[off..off + 16].try_into().unwrap());

        let mut temp_sensors = [0u16; 8];
        for i in 0..8 {
            temp_sensors[i] = read_u16(200 + i * 2);
        }

        Ok(Self {
            critical_warning: buf[0],
            composite_temperature_kelvin: read_u16(1),
            available_spare_percent: buf[3],
            available_spare_threshold_percent: buf[4],
            percentage_used: buf[5],
            endurance_group_critical_warning: buf[6],
            data_units_read: read_u128(32),
            data_units_written: read_u128(48),
            host_read_commands: read_u128(64),
            host_write_commands: read_u128(80),
            controller_busy_time_minutes: read_u128(96),
            power_cycles: read_u128(112),
            power_on_hours: read_u128(128),
            unsafe_shutdowns: read_u128(144),
            media_errors: read_u128(160),
            num_err_log_entries: read_u128(176),
            warning_temp_time_minutes: read_u32(192),
            critical_temp_time_minutes: read_u32(196),
            temp_sensors,
        })
    }

    /// Converts composite temperature from Kelvin to Celsius.
    pub fn temperature_celsius(&self) -> i16 {
        (self.composite_temperature_kelvin as i32 - 273) as i16
    }

    /// Total bytes read (each data unit represents 1000 * 512 bytes).
    pub fn data_bytes_read(&self) -> u128 {
        self.data_units_read.saturating_mul(1000 * 512)
    }

    /// Total bytes written (each data unit represents 1000 * 512 bytes).
    pub fn data_bytes_written(&self) -> u128 {
        self.data_units_written.saturating_mul(1000 * 512)
    }

    /// Evaluates NVMe health and produces a comprehensive diagnostic `HealthReport`.
    pub fn analyze(&self) -> HealthReport {
        let mut findings = Vec::new();
        let mut overall = OverallHealth::Healthy;

        let temp_c = self.temperature_celsius();

        // 1. Critical Warning Bitmask Checks
        if self.critical_warning & NVME_WARN_SPARE_BELOW_THRESHOLD != 0 {
            overall = OverallHealth::Critical;
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "NVME_SPARE_BELOW_THRESHOLD",
                description: "Available spare capacity has fallen below threshold".to_string(),
            });
        }
        if self.critical_warning & NVME_WARN_TEMPERATURE != 0 {
            overall = OverallHealth::Critical;
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "NVME_TEMP_CRITICAL",
                description: "Temperature is above or below temperature threshold".to_string(),
            });
        }
        if self.critical_warning & NVME_WARN_MEDIA_RELIABILITY != 0 {
            overall = OverallHealth::Critical;
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "NVME_MEDIA_RELIABILITY",
                description: "NVM subsystem reliability has been degraded due to media errors".to_string(),
            });
        }
        if self.critical_warning & NVME_WARN_READ_ONLY != 0 {
            overall = OverallHealth::Critical;
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "NVME_READ_ONLY",
                description: "Media has been placed in read-only mode to preserve data".to_string(),
            });
        }
        if self.critical_warning & NVME_WARN_VOLATILE_BACKUP_FAILED != 0 {
            overall = OverallHealth::Critical;
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "NVME_VOLATILE_BACKUP_FAILED",
                description: "Volatile memory backup device has failed".to_string(),
            });
        }

        // 2. Spare and Wear Level
        if self.available_spare_percent <= self.available_spare_threshold_percent {
            if overall < OverallHealth::Critical {
                overall = OverallHealth::Critical;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "SPARE_EXHAUSTED",
                description: alloc::format!(
                    "Available spare ({}%) at or below threshold ({}%)",
                    self.available_spare_percent, self.available_spare_threshold_percent
                ),
            });
        } else if self.available_spare_percent < 20 {
            if overall < OverallHealth::Warning {
                overall = OverallHealth::Warning;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Warning,
                code: "SPARE_LOW",
                description: alloc::format!("Available spare is low ({}%)", self.available_spare_percent),
            });
        }

        if self.percentage_used >= 100 {
            if overall < OverallHealth::Warning {
                overall = OverallHealth::Warning;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Warning,
                code: "WEAR_EXCEEDED",
                description: alloc::format!(
                    "Drive wear indicator ({}%) has reached or exceeded 100% rated endurance",
                    self.percentage_used
                ),
            });
        } else if self.percentage_used >= 85 {
            if overall < OverallHealth::Warning {
                overall = OverallHealth::Warning;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Warning,
                code: "WEAR_HIGH",
                description: alloc::format!("Drive wear is elevated ({}%)", self.percentage_used),
            });
        }

        // 3. Temperature Thresholds
        if temp_c >= 75 {
            if overall < OverallHealth::Critical {
                overall = OverallHealth::Critical;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "TEMPERATURE_HIGH",
                description: alloc::format!("Controller temperature is critically high ({}°C)", temp_c),
            });
        } else if temp_c >= 60 {
            if overall < OverallHealth::Warning {
                overall = OverallHealth::Warning;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Warning,
                code: "TEMPERATURE_ELEVATED",
                description: alloc::format!("Controller temperature is elevated ({}°C)", temp_c),
            });
        }

        // 4. Media & Integrity Errors
        if self.media_errors > 0 {
            if overall < OverallHealth::Critical {
                overall = OverallHealth::Critical;
            }
            findings.push(HealthFinding {
                severity: FindingSeverity::Critical,
                code: "MEDIA_INTEGRITY_ERRORS",
                description: alloc::format!("Drive has encountered {} uncorrectable media errors", self.media_errors),
            });
        }

        // 5. Unsafe Shutdowns
        if self.unsafe_shutdowns > 100 {
            findings.push(HealthFinding {
                severity: FindingSeverity::Info,
                code: "UNSAFE_SHUTDOWNS",
                description: alloc::format!("Drive has recorded {} unsafe shutdowns", self.unsafe_shutdowns),
            });
        }

        HealthReport {
            protocol: DeviceProtocol::Nvme,
            overall,
            wear_percent: Some(self.percentage_used),
            temperature_celsius: Some(temp_c),
            power_on_hours: Some(self.power_on_hours as u64),
            power_cycles: Some(self.power_cycles as u64),
            unsafe_shutdowns: Some(self.unsafe_shutdowns as u64),
            media_errors: Some(self.media_errors as u64),
            data_bytes_read: Some(self.data_bytes_read()),
            data_bytes_written: Some(self.data_bytes_written()),
            findings,
        }
    }
}

// -----------------------------------------------------------------------------
// ATA SMART Data Structure
// -----------------------------------------------------------------------------

/// Well-known ATA SMART attribute IDs.
pub const ATA_ATTR_REALLOCATED_SECTORS: u8 = 5;
pub const ATA_ATTR_POWER_ON_HOURS: u8 = 9;
pub const ATA_ATTR_POWER_CYCLES: u8 = 12;
pub const ATA_ATTR_TEMPERATURE: u8 = 194;
pub const ATA_ATTR_PENDING_SECTORS: u8 = 197;
pub const ATA_ATTR_UNCORRECTABLE_SECTORS: u8 = 198;
pub const ATA_ATTR_UDMA_CRC_ERRORS: u8 = 199;

/// Individual 12-byte ATA SMART vendor attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtaSmartAttribute {
    pub id: u8,
    pub flags: u16,
    pub current: u8,
    pub worst: u8,
    pub raw: [u8; 6],
}

impl AtaSmartAttribute {
    /// Interpret 6 raw bytes as a little-endian unsigned 48-bit integer.
    pub fn raw_u64(&self) -> u64 {
        let mut b = [0u8; 8];
        b[..6].copy_from_slice(&self.raw);
        u64::from_le_bytes(b)
    }

    /// Descriptive name for well-known attributes.
    pub fn name(&self) -> &'static str {
        match self.id {
            1 => "Raw Read Error Rate",
            5 => "Reallocated Sectors Count",
            7 => "Seek Error Rate",
            9 => "Power-On Hours",
            10 => "Spin Retry Count",
            12 => "Power Cycle Count",
            187 => "Reported Uncorrectable Errors",
            194 => "Temperature Celsius",
            197 => "Current Pending Sector Count",
            198 => "Offline Uncorrectable Sector Count",
            199 => "UltraDMA CRC Error Count",
            _ => "Vendor Attribute",
        }
    }
}

/// Parsed 512-byte ATA SMART data sector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtaSmartData {
    pub revision: u16,
    pub attributes: Vec<AtaSmartAttribute>,
}

impl AtaSmartData {
    /// Parse from a raw 512-byte ATA SMART buffer.
    pub fn parse(buf: &[u8]) -> Result<Self, CorsetError> {
        if buf.len() < 512 {
            return Err(CorsetError::SmartDataCorrupt("ATA SMART buffer less than 512 bytes"));
        }

        let revision = u16::from_le_bytes([buf[0], buf[1]]);
        let mut attributes = Vec::new();

        // 30 vendor attributes starting at offset 2, each 12 bytes
        for i in 0..30 {
            let offset = 2 + i * 12;
            let id = buf[offset];
            if id == 0 {
                continue; // Unused attribute slot
            }
            let flags = u16::from_le_bytes([buf[offset + 1], buf[offset + 2]]);
            let current = buf[offset + 3];
            let worst = buf[offset + 4];
            let mut raw = [0u8; 6];
            raw.copy_from_slice(&buf[offset + 5..offset + 11]);

            attributes.push(AtaSmartAttribute {
                id,
                flags,
                current,
                worst,
                raw,
            });
        }

        Ok(Self {
            revision,
            attributes,
        })
    }

    /// Find an attribute by ID.
    pub fn find_attribute(&self, id: u8) -> Option<&AtaSmartAttribute> {
        self.attributes.iter().find(|a| a.id == id)
    }

    /// Analyze ATA SMART attributes and compare against thresholds if provided.
    pub fn analyze(&self, thresholds_buf: Option<&[u8]>) -> HealthReport {
        let mut findings = Vec::new();
        let mut overall = OverallHealth::Healthy;

        let mut power_on_hours = None;
        let mut power_cycles = None;
        let mut temp_c = None;
        let mut media_errors = None;

        // Parse optional 512-byte thresholds sector if provided
        let thresholds_map = thresholds_buf.and_then(|tbuf| {
            if tbuf.len() < 512 {
                return None;
            }
            let mut map = [0u8; 256];
            for i in 0..30 {
                let off = 2 + i * 12;
                let id = tbuf[off];
                let thresh = tbuf[off + 1];
                if id != 0 {
                    map[id as usize] = thresh;
                }
            }
            Some(map)
        });

        for attr in &self.attributes {
            // Check threshold violation
            if let Some(ref map) = thresholds_map {
                let thresh = map[attr.id as usize];
                if thresh > 0 && attr.current <= thresh {
                    overall = OverallHealth::Critical;
                    findings.push(HealthFinding {
                        severity: FindingSeverity::Critical,
                        code: "ATA_THRESHOLD_VIOLATION",
                        description: alloc::format!(
                            "Attribute {} ({}) value {} is below failure threshold {}",
                            attr.id, attr.name(), attr.current, thresh
                        ),
                    });
                }
            }

            match attr.id {
                ATA_ATTR_REALLOCATED_SECTORS => {
                    let reallocated = attr.raw_u64();
                    if reallocated > 0 {
                        if overall < OverallHealth::Warning {
                            overall = OverallHealth::Warning;
                        }
                        findings.push(HealthFinding {
                            severity: if reallocated > 50 { FindingSeverity::Critical } else { FindingSeverity::Warning },
                            code: "REALLOCATED_SECTORS",
                            description: alloc::format!("Drive has {} reallocated sectors", reallocated),
                        });
                    }
                }
                ATA_ATTR_PENDING_SECTORS => {
                    let pending = attr.raw_u64();
                    if pending > 0 {
                        overall = OverallHealth::Critical;
                        findings.push(HealthFinding {
                            severity: FindingSeverity::Critical,
                            code: "CURRENT_PENDING_SECTORS",
                            description: alloc::format!("Drive has {} unstable sectors waiting to be reallocated", pending),
                        });
                    }
                }
                ATA_ATTR_UNCORRECTABLE_SECTORS => {
                    let uncorrectable = attr.raw_u64();
                    if uncorrectable > 0 {
                        overall = OverallHealth::Critical;
                        findings.push(HealthFinding {
                            severity: FindingSeverity::Critical,
                            code: "OFFLINE_UNCORRECTABLE",
                            description: alloc::format!("Drive has {} uncorrectable sectors", uncorrectable),
                        });
                        media_errors = Some(uncorrectable);
                    }
                }
                ATA_ATTR_UDMA_CRC_ERRORS => {
                    let crc_errors = attr.raw_u64();
                    if crc_errors > 0 {
                        findings.push(HealthFinding {
                            severity: FindingSeverity::Warning,
                            code: "UDMA_CRC_ERRORS",
                            description: alloc::format!("Drive reported {} interface CRC errors (check cable)", crc_errors),
                        });
                    }
                }
                ATA_ATTR_TEMPERATURE => {
                    let temp = (attr.raw[0] as i16).max(attr.raw[1] as i16);
                    temp_c = Some(temp);
                    if temp >= 65 {
                        if overall < OverallHealth::Warning {
                            overall = OverallHealth::Warning;
                        }
                        findings.push(HealthFinding {
                            severity: if temp >= 75 { FindingSeverity::Critical } else { FindingSeverity::Warning },
                            code: "TEMPERATURE_HIGH",
                            description: alloc::format!("Disk temperature is high ({}°C)", temp),
                        });
                    }
                }
                ATA_ATTR_POWER_ON_HOURS => {
                    power_on_hours = Some(attr.raw_u64());
                }
                ATA_ATTR_POWER_CYCLES => {
                    power_cycles = Some(attr.raw_u64());
                }
                _ => {}
            }
        }

        HealthReport {
            protocol: DeviceProtocol::Ata,
            overall,
            wear_percent: None,
            temperature_celsius: temp_c,
            power_on_hours,
            power_cycles,
            unsafe_shutdowns: None,
            media_errors,
            data_bytes_read: None,
            data_bytes_written: None,
            findings,
        }
    }
}
