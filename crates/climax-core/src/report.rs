//! Score and report formatter for Climax benchmarks:
//! Subsystem scoring, throughput percentiles (p50/p95/p99), stability percentages,
//! and fixed-width ASCII/Unicode visual bar charts for Spindle/Weft display.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use crate::thermal::{ThermalAnalysis, ThrottleState};

/// Individual subsystem score breakdown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubsystemScores {
    /// Integer ALU / ARX throughput score.
    pub cpu_int: u64,
    /// SHA-256 cryptographic throughput score.
    pub sha256: u64,
    /// Floating-point matrix multiply score.
    pub gemm_fp: u64,
    /// Vectorizable DSP audio filter score.
    pub dsp_fp: u64,
    /// Memory STREAM bandwidth score.
    pub memory_bandwidth: u64,
    /// Cache/TLB pointer-chase latency score.
    pub pointer_latency: u64,
    /// Unified composite benchmark score.
    pub composite: u64,
}

impl SubsystemScores {
    /// Computes subsystem scores and geometric composite score.
    pub fn compute(
        arx_ops: u64,
        sha_ops: u64,
        gemm_ops: u64,
        dsp_ops: u64,
        stream_bytes: u64,
        ladder_hops: u64,
    ) -> Self {
        // Normalize raw operation counts into balanced point metrics
        let cpu_int = (arx_ops / 100).max(1);
        let sha256 = (sha_ops / 100).max(1);
        let gemm_fp = (gemm_ops / 100).max(1);
        let dsp_fp = (dsp_ops / 100).max(1);
        let memory_bandwidth = (stream_bytes / 1024).max(1);
        let pointer_latency = (ladder_hops / 10).max(1);

        // Geometric mean of subsystem metrics
        let values = [
            cpu_int as f64,
            sha256 as f64,
            gemm_fp as f64,
            dsp_fp as f64,
            memory_bandwidth as f64,
            pointer_latency as f64,
        ];

        let mut sum_logs = 0.0f64;
        for &v in &values {
            let log_val = if v > 0.0 && v.is_finite() {
                libm::log(v)
            } else {
                0.0
            };
            sum_logs += log_val;
        }
        let geom_mean = libm::exp(sum_logs / 6.0);
        let composite = if geom_mean.is_finite() && geom_mean > 0.0 {
            (geom_mean as u64).max(1)
        } else {
            1
        };

        Self {
            cpu_int,
            sha256,
            gemm_fp,
            dsp_fp,
            memory_bandwidth,
            pointer_latency,
            composite,
        }
    }
}

/// Computes p50, p95, and p99 percentiles from a series of throughput values.
pub fn compute_percentiles(values: Vec<f64>) -> (f64, f64, f64) {
    let mut clean: Vec<f64> = values
        .into_iter()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .collect();
    if clean.is_empty() {
        return (0.0, 0.0, 0.0);
    }

    clean.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = clean.len();

    let p50 = clean[(n * 50) / 100];
    let p95_idx = ((n * 95) / 100).min(n - 1);
    let p99_idx = ((n * 99) / 100).min(n - 1);

    (p50, clean[p95_idx], clean[p99_idx])
}

/// Comprehensive Climax benchmark summary report.
#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkReport {
    /// Subsystem breakdown scores.
    pub scores: SubsystemScores,
    /// Percentiles of observed throughput: (p50, p95, p99) in ops/sec.
    pub percentiles: (f64, f64, f64),
    /// Thermal stability analysis.
    pub thermal: ThermalAnalysis,
    /// Peak memory footprint tested in bytes.
    pub peak_memory_bytes: usize,
}

impl BenchmarkReport {
    /// Renders a fixed-width visual box report with ASCII/Unicode bar charts.
    pub fn render_terminal_report(&self) -> String {
        let mut out = String::new();

        let max_score = [
            self.scores.cpu_int,
            self.scores.sha256,
            self.scores.gemm_fp,
            self.scores.dsp_fp,
            self.scores.memory_bandwidth,
            self.scores.pointer_latency,
        ]
        .into_iter()
        .max()
        .unwrap_or(1)
        .max(1);

        let make_bar = |score: u64, max: u64, width: usize| -> String {
            if width == 0 {
                return String::from("[]");
            }
            let max_val = max.max(1);
            let score_clamped = score.min(max_val);
            let filled = (((score_clamped as u128) * (width as u128)) / (max_val as u128)) as usize;
            let filled = filled.min(width);
            let mut s = String::from("[");
            for i in 0..width {
                if i < filled {
                    s.push_str("█");
                } else {
                    s.push(' ');
                }
            }
            s.push(']');
            s
        };

        let bar_width = 18;
        let b_int = make_bar(self.scores.cpu_int, max_score, bar_width);
        let b_sha = make_bar(self.scores.sha256, max_score, bar_width);
        let b_gemm = make_bar(self.scores.gemm_fp, max_score, bar_width);
        let b_dsp = make_bar(self.scores.dsp_fp, max_score, bar_width);
        let b_mem = make_bar(self.scores.memory_bandwidth, max_score, bar_width);
        let b_lat = make_bar(self.scores.pointer_latency, max_score, bar_width);

        let (p50, p95, p99) = self.percentiles;

        let throttle_desc = match &self.thermal.throttle_state {
            ThrottleState::Warmup => String::from("Warmup (Insufficient Steps)"),
            ThrottleState::Sustained => String::from("Sustained (No Throttle)"),
            ThrottleState::Throttled { drop_percent, onset_ms } => {
                let mut s = String::new();
                let _ = write!(s, "THROTTLED (-{:.1}% at {}ms)", drop_percent, onset_ms);
                s
            }
        };

        let corruption_str = if self.thermal.corruption_detected {
            "FAIL (Bit Flips Detected!)"
        } else {
            "PASS (0 bit flips)"
        };

        let _ = writeln!(out, "┌────────────────────────────────────────────────────────┐");
        let _ = writeln!(out, "│                CLIMAX BENCHMARK REPORT                 │");
        let _ = writeln!(out, "├────────────────────────────────────────────────────────┤");
        let _ = writeln!(out, "│ CPU Integer (ChaCha/ARX) : {} {:>6} │", b_int, self.scores.cpu_int);
        let _ = writeln!(out, "│ CPU SHA-256 Digest       : {} {:>6} │", b_sha, self.scores.sha256);
        let _ = writeln!(out, "│ FP Tiled Matrix GEMM     : {} {:>6} │", b_gemm, self.scores.gemm_fp);
        let _ = writeln!(out, "│ Vectorizable Audio DSP   : {} {:>6} │", b_dsp, self.scores.dsp_fp);
        let _ = writeln!(out, "│ Memory STREAM Bandwidth  : {} {:>6} │", b_mem, self.scores.memory_bandwidth);
        let _ = writeln!(out, "│ Cache/TLB Pointer Chase  : {} {:>6} │", b_lat, self.scores.pointer_latency);
        let _ = writeln!(out, "├────────────────────────────────────────────────────────┤");
        let _ = writeln!(out, "│ Composite Climax Score   : {:<27} │", self.scores.composite);
        let _ = writeln!(out, "│ Throughput p50 / p95     : {:<6.0} / {:<6.0} ops/sec  │", p50, p95);
        let _ = writeln!(out, "│ Throughput p99 (Peak)    : {:<6.0} ops/sec           │", p99);
        let _ = writeln!(out, "│ Sustained Stability      : {:>5.1}% (3DMark Style)   │", self.thermal.stability_percent);
        let _ = writeln!(out, "│ Thermal Throttle State   : {:<27} │", throttle_desc);
        let _ = writeln!(out, "│ Hardware Self-Check      : {:<27} │", corruption_str);
        let _ = writeln!(out, "└────────────────────────────────────────────────────────┘");

        out
    }
}
