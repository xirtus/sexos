// SPDX-License-Identifier: GPL-3.0-only
//! Parser and representation for Climax vs Linux benchmark comparison documents.
//! Parses markdown reports (e.g. `docs/CLIMAX_VS_LINUX_2026_11_14.md`), extracting
//! hardware/hypervisor metadata, bit-exact verification checksums, and measured
//! performance comparison tables with geometric mean advantages.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::str::FromStr;

use crate::ClimaxError;

/// Bit-exact hardware verification and integrity checksum row.
#[derive(Clone, Debug, PartialEq)]
pub struct ChecksumRow {
    /// Benchmark subsystem name (e.g. "Integer ChaCha/ARX").
    pub subsystem: String,
    /// Golden output checksum hexadecimal string.
    pub golden_checksum: String,
    /// Whether SexOS output matched bit-for-bit.
    pub sexos_match: bool,
    /// Whether Linux output matched bit-for-bit.
    pub linux_match: bool,
    /// Integrity verdict string (e.g. "Bit-Exact Match").
    pub status: String,
}

/// Measured performance comparison row across 9 microbenchmark workloads.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparisonRow {
    /// Subsystem workload name.
    pub subsystem: String,
    /// Unit of measurement (e.g. "Mops/s", "MB/s", "MFLOP/s").
    pub metric_unit: String,
    /// SexOS median measured throughput.
    pub sexos_median: f64,
    /// SexOS observed min - max range string.
    pub sexos_range: String,
    /// Linux median measured throughput.
    pub linux_median: f64,
    /// Linux observed min - max range string.
    pub linux_range: String,
    /// Speedup ratio percentage (e.g. 107.2 for 107.2%, or 365.8 for 365.8%).
    pub ratio_percent: f64,
}

/// Fully parsed Climax vs Linux comparison document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinuxComparisonDoc {
    /// Document title.
    pub title: String,
    /// QEMU machine target (e.g. "q35 Standard PC").
    pub qemu_machine: Option<String>,
    /// CPU emulation model (e.g. "max,+pku").
    pub cpu_model: Option<String>,
    /// CPU SMP topology (e.g. "-smp 2").
    pub cpu_topology: Option<String>,
    /// Physical memory (e.g. "-m 1G").
    pub physical_memory: Option<String>,
    /// Disk subsystem (e.g. "NVMe PCIe controller").
    pub disk_subsystem: Option<String>,
    /// Host architecture (e.g. "Apple Silicon (aarch64) running TCG x86_64").
    pub host_arch: Option<String>,
    /// Bit-exact integrity checksum verification rows.
    pub checksum_rows: Vec<ChecksumRow>,
    /// Workload performance comparison rows.
    pub comparison_rows: Vec<ComparisonRow>,
    /// Data source description (e.g. "disk:CLIMAX_VS_LINUX_2026_11_14.md").
    pub source_label: String,
}

impl LinuxComparisonDoc {
    /// Parses a markdown document (such as `docs/CLIMAX_VS_LINUX_2026_11_14.md`) into structured data.
    pub fn parse_markdown(md: &str) -> Result<Self, ClimaxError> {
        let mut doc = LinuxComparisonDoc::default();

        for line in md.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("# ") && doc.title.is_empty() {
                doc.title = trimmed.trim_start_matches("# ").trim().to_string();
            }

            // Metadata parsing
            if trimmed.starts_with("- **QEMU Machine Target**:") {
                doc.qemu_machine = Some(extract_metadata_val(trimmed, "- **QEMU Machine Target**:"));
            } else if trimmed.starts_with("- **CPU Emulation Model**:") {
                doc.cpu_model = Some(extract_metadata_val(trimmed, "- **CPU Emulation Model**:"));
            } else if trimmed.starts_with("- **CPU Topology**:") {
                doc.cpu_topology = Some(extract_metadata_val(trimmed, "- **CPU Topology**:"));
            } else if trimmed.starts_with("- **Physical Memory**:") {
                doc.physical_memory = Some(extract_metadata_val(trimmed, "- **Physical Memory**:"));
            } else if trimmed.starts_with("- **Disk Subsystem**:") {
                doc.disk_subsystem = Some(extract_metadata_val(trimmed, "- **Disk Subsystem**:"));
            } else if trimmed.starts_with("- **Host Architecture**:") {
                doc.host_arch = Some(extract_metadata_val(trimmed, "- **Host Architecture**:"));
            }

            // Markdown table parsing
            if trimmed.starts_with('|') && trimmed.ends_with('|') {
                let cells: Vec<&str> = trimmed
                    .split('|')
                    .map(|c| c.trim())
                    .collect();

                // Leading and trailing '|' yield empty strings at both boundaries
                if cells.len() < 3 {
                    continue;
                }
                let inner = &cells[1..cells.len() - 1];

                // Skip header separator rows (e.g. | :--- | :---: |)
                if inner.iter().any(|c| c.contains("---")) {
                    continue;
                }

                // Check for Section 2: 5 columns
                // | Benchmark Subsystem | Golden Output Checksum | SexOS Match | Linux Match | Integrity Status |
                if inner.len() == 5 {
                    if inner[0].eq_ignore_ascii_case("Benchmark Subsystem") {
                        continue;
                    }
                    let subsystem = inner[0].to_string();
                    let golden_checksum = inner[1].trim_matches('`').to_string();
                    let sexos_match = is_yes(inner[2]);
                    let linux_match = is_yes(inner[3]);
                    let status = inner[4].to_string();

                    doc.checksum_rows.push(ChecksumRow {
                        subsystem,
                        golden_checksum,
                        sexos_match,
                        linux_match,
                        status,
                    });
                }
                // Check for Section 3: 7 columns
                // | Benchmark Subsystem | Metric | SexOS (Median) | SexOS (Min – Max) | Linux (Median) | Linux (Min – Max) | Ratio (SexOS / Linux) |
                else if inner.len() == 7 {
                    if inner[0].eq_ignore_ascii_case("Benchmark Subsystem") {
                        continue;
                    }
                    let subsystem = inner[0].to_string();
                    let metric_unit = inner[1].to_string();
                    let sexos_median = parse_first_float(inner[2]).unwrap_or(0.0);
                    let sexos_range = inner[3].to_string();
                    let linux_median = parse_first_float(inner[4]).unwrap_or(0.0);
                    let linux_range = inner[5].to_string();
                    let ratio_percent = parse_ratio_pct(inner[6], sexos_median, linux_median);

                    doc.comparison_rows.push(ComparisonRow {
                        subsystem,
                        metric_unit,
                        sexos_median,
                        sexos_range,
                        linux_median,
                        linux_range,
                        ratio_percent,
                    });
                }
            }
        }

        if doc.comparison_rows.is_empty() {
            return Err(ClimaxError::ParseError("no comparison rows found in markdown"));
        }

        doc.source_label = String::from("parsed:markdown");
        Ok(doc)
    }

    /// Geometric mean speedup percentage (e.g. 143.9 for 143.9%).
    pub fn geometric_mean_ratio_pct(&self) -> f64 {
        if self.comparison_rows.is_empty() {
            return 0.0;
        }
        let mut sum_logs = 0.0f64;
        for row in &self.comparison_rows {
            let r = if row.ratio_percent > 0.0 && row.ratio_percent.is_finite() {
                row.ratio_percent
            } else {
                100.0
            };
            sum_logs += libm::log(r);
        }
        let mean = libm::exp(sum_logs / (self.comparison_rows.len() as f64));
        if mean.is_finite() && mean > 0.0 {
            mean
        } else {
            0.0
        }
    }

    /// Finds the row with the largest speedup advantage (e.g. Sattolo L3 at 365.8%).
    pub fn fastest_advantage_row(&self) -> Option<&ComparisonRow> {
        self.comparison_rows.iter().max_by(|a, b| {
            a.ratio_percent.partial_cmp(&b.ratio_percent).unwrap_or(core::cmp::Ordering::Equal)
        })
    }

    /// Number of verified checksum rows where both SexOS and Linux match golden output.
    pub fn checksum_verified_count(&self) -> usize {
        self.checksum_rows
            .iter()
            .filter(|r| r.sexos_match && r.linux_match)
            .count()
    }

    /// Overlays live benchmark metrics onto published comparison rows.
    pub fn overlay_live_suite<'a>(
        &'a self,
        live_metrics: &[(&str, f64)],
    ) -> Vec<(&'a ComparisonRow, Option<f64>)> {
        self.comparison_rows
            .iter()
            .map(|row| {
                let live = live_metrics.iter().find_map(|(key, val)| {
                    let k = key.to_ascii_lowercase();
                    let s = row.subsystem.to_ascii_lowercase();
                    if s.contains(&k) {
                        Some(*val)
                    } else {
                        None
                    }
                });
                (row, live)
            })
            .collect()
    }
}

fn is_yes(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.eq_ignore_ascii_case("Yes")
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("PASS")
}

fn extract_metadata_val(line: &str, prefix: &str) -> String {
    let rest = line.strip_prefix(prefix).unwrap_or(line).trim();
    rest.trim_matches('`').trim().to_string()
}

fn parse_first_float(s: &str) -> Option<f64> {
    let mut start = None;
    let mut end = None;
    for (i, b) in s.bytes().enumerate() {
        if b.is_ascii_digit() || b == b'.' || b == b'-' {
            if start.is_none() {
                start = Some(i);
            }
            end = Some(i + 1);
        } else if start.is_some() {
            break;
        }
    }
    let sub = &s[start? .. end?];
    f64::from_str(sub).ok()
}

fn parse_ratio_pct(s: &str, sexos: f64, linux: f64) -> f64 {
    let clean = s.trim_matches(|c: char| c == '*' || c == '%' || c.is_whitespace());
    if let Ok(val) = f64::from_str(clean) {
        if val > 0.0 && val.is_finite() {
            return val;
        }
    }
    if linux > 0.0 && linux.is_finite() && sexos.is_finite() {
        (sexos / linux) * 100.0
    } else {
        100.0
    }
}
