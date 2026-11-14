// SPDX-License-Identifier: GPL-3.0-only
//! Climax benchmark, stress torture, thermal throttling and report engine UI on Weft.

#![cfg_attr(feature = "sexos", no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use climax_core::kernels::{
    bench_crypto_arx, bench_dsp_f32, bench_pointer_chase_ladder, bench_sha256,
    bench_stream, bench_tiled_gemm_f32, LatencyStep, StreamResult,
};
use climax_core::report::SubsystemScores;
use climax_core::stress::{Sample, StressConfig, TortureRunner, WorkloadType};
use climax_core::thermal::{ThermalAnalysis, ThermalAnalyzer, ThrottleState};
pub mod ui_backend;
pub use ui_backend as weft;

use weft::widget::{Id, TextStyle};
use weft::{Response, Ui};

/// Active mode view in the Climax window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimaxMode {
    /// Deterministic microbenchmarks suite.
    Benchmarks,
    /// Sustained thermal stress and throttling torture.
    Thermal,
}

/// Recorded results from executing the full benchmark suite.
#[derive(Clone, Debug, PartialEq)]
pub struct SuiteResult {
    pub arx_ops: u64,
    pub arx_elapsed_ms: u64,
    pub arx_ops_per_sec: f64,
    pub arx_checksum: u64,

    pub sha_ops: u64,
    pub sha_elapsed_ms: u64,
    pub sha_ops_per_sec: f64,
    pub sha_checksum: u64,

    pub gemm_ops: u64,
    pub gemm_elapsed_ms: u64,
    pub gemm_ops_per_sec: f64,
    pub gemm_checksum: u64,

    pub dsp_ops: u64,
    pub dsp_elapsed_ms: u64,
    pub dsp_ops_per_sec: f64,
    pub dsp_checksum: u64,

    pub stream_res: StreamResult,
    pub stream_elapsed_ms: u64,
    pub stream_bytes_per_sec: f64,

    pub ladder_steps: Vec<LatencyStep>,
    pub ladder_elapsed_ms: u64,
    pub ladder_hops: u64,
    pub ladder_hops_per_sec: f64,
    pub ladder_checksum: u64,

    pub scores: SubsystemScores,
    pub json_payload: String,
}

/// Climax application state and Weft widget hierarchy.
pub struct Climax {
    pub mode: ClimaxMode,
    pub suite_ran: bool,
    pub suite_result: Option<SuiteResult>,
    pub torture_runner: TortureRunner,
    pub torture_analysis: ThermalAnalysis,

    // UI Widgets
    pub choices: [Id; 2],
    pub run_suite: Id,
    pub start_step: Id,
    pub run_burst: Id,
    pub reset: Id,
    pub status: Id,
    pub display_text: Id,
    pub instructions: Id,
}

impl Climax {
    /// Builds the Climax UI widgets and initializes stress runner state.
    pub fn build(ui: &mut Ui) -> Self {
        let root = ui.root();
        ui.text(root, "Climax Hardware Benchmark & Thermal Stress Suite", TextStyle::Title);

        let toolbar_mode = ui.row(root);
        let choices = [
            ui.button(toolbar_mode, "Benchmarks"),
            ui.button(toolbar_mode, "Thermal Torture"),
        ];

        let toolbar_actions = ui.row(root);
        let run_suite = ui.button(toolbar_actions, "Run Suite");
        let start_step = ui.button(toolbar_actions, "Start Step");
        let run_burst = ui.button(toolbar_actions, "Run Burst (5x)");
        let reset = ui.button(toolbar_actions, "Reset");

        let status = ui.label(root, "Status: Ready");
        let instructions = ui.paragraph(root, "");
        let display_text = ui.paragraph(root, "");

        let torture_config = StressConfig {
            workload: WorkloadType::BalancedMix,
            step_duration_ms: 100,
            total_steps: 20,
            memory_quota_bytes: 4 * 1024 * 1024,
            iters_per_slice: 100,
        };
        let torture_runner = TortureRunner::new(torture_config).expect("valid stress config");
        let torture_analysis = ThermalAnalyzer::new().analyze(torture_runner.samples());

        let mut app = Self {
            mode: ClimaxMode::Benchmarks,
            suite_ran: false,
            suite_result: None,
            torture_runner,
            torture_analysis,
            choices,
            run_suite,
            start_step,
            run_burst,
            reset,
            status,
            display_text,
            instructions,
        };

        app.refresh(ui);
        app
    }

    /// Updates widget labels, button states, and display text based on current state.
    pub fn refresh(&mut self, ui: &mut Ui) {
        let is_bench = self.mode == ClimaxMode::Benchmarks;
        ui.set_visible(self.run_suite, is_bench);
        ui.set_visible(self.start_step, !is_bench);
        ui.set_visible(self.run_burst, !is_bench);
        ui.set_visible(self.reset, !is_bench);

        match self.mode {
            ClimaxMode::Benchmarks => {
                ui.set_text(
                    self.instructions,
                    "Mode: Microbenchmarks | 6 Subsystems: Integer ARX, SHA-256, GEMM FP32, DSP, STREAM, Sattolo Ladder",
                );
                if let Some(res) = &self.suite_result {
                    let mut s = String::new();
                    let _ = writeln!(s, "=== BENCHMARK SUITE COMPLETED ===");
                    let _ = writeln!(s, "• Integer ALU (ChaCha/ARX) : {:.1} Mops/s ({} ops in {}ms, ck={:#x})",
                        res.arx_ops_per_sec / 1e6, res.arx_ops, res.arx_elapsed_ms, res.arx_checksum);
                    let _ = writeln!(s, "• SHA-256 Digest Rounds    : {:.1} Kops/s ({} ops in {}ms, ck={:#x})",
                        res.sha_ops_per_sec / 1e3, res.sha_ops, res.sha_elapsed_ms, res.sha_checksum);
                    let _ = writeln!(s, "• Matrix GEMM 64x64 FP32   : {:.1} ops/s ({} ops in {}ms, ck={:#x})",
                        res.gemm_ops_per_sec, res.gemm_ops, res.gemm_elapsed_ms, res.gemm_checksum);
                    let _ = writeln!(s, "• Audio Biquad DSP Filter  : {:.1} Kops/s ({} ops in {}ms, ck={:#x})",
                        res.dsp_ops_per_sec / 1e3, res.dsp_ops, res.dsp_elapsed_ms, res.dsp_checksum);
                    let _ = writeln!(s, "• Memory STREAM Bandwidth  : {:.2} MB/s ({} bytes in {}ms)",
                        res.stream_bytes_per_sec / (1024.0 * 1024.0), res.stream_res.bytes_processed, res.stream_elapsed_ms);
                    let _ = writeln!(s, "• Cache/TLB Sattolo Ladder : {:.1} Khops/s ({} hops across {} tiers in {}ms)",
                        res.ladder_hops_per_sec / 1e3, res.ladder_hops, res.ladder_steps.len(), res.ladder_elapsed_ms);
                    let _ = writeln!(s, "--------------------------------------------------------");
                    let _ = writeln!(s, "Composite Score: {} | CPU: {} | SHA: {} | GEMM: {} | DSP: {} | Mem: {} | Lat: {}",
                        res.scores.composite, res.scores.cpu_int, res.scores.sha256, res.scores.gemm_fp, res.scores.dsp_fp,
                        res.scores.memory_bandwidth, res.scores.pointer_latency);
                    ui.set_text(self.display_text, &s);
                    ui.set_text(self.status, &format!("Status: Benchmarks complete — Composite Score {}", res.scores.composite));
                } else {
                    ui.set_text(
                        self.display_text,
                        "Press 'Run Suite' to benchmark Integer ARX, SHA-256, GEMM FP32, DSP, STREAM Triad, and Cache Ladder.",
                    );
                    ui.set_text(self.status, "Status: Ready to run benchmark suite");
                }
            }
            ClimaxMode::Thermal => {
                ui.set_text(
                    self.instructions,
                    "Mode: Thermal Torture | Sustained cyclic load measuring throttle onset, stability % and bit-exact integrity.",
                );
                let completed = self.torture_runner.steps_completed();
                let total = 20;
                let corruption_str = if self.torture_analysis.corruption_detected {
                    "FAIL (Bit corruption detected!)"
                } else {
                    "PASS (Bit-exact golden match)"
                };

                let throttle_str = match &self.torture_analysis.throttle_state {
                    ThrottleState::Warmup => String::from("Warmup (gathering baseline)"),
                    ThrottleState::Sustained => String::from("Sustained (no thermal drop)"),
                    ThrottleState::Throttled { drop_percent, onset_ms } => {
                        format!("THROTTLED (-{:.1}% at {}ms)", drop_percent, onset_ms)
                    }
                };

                let mut s = String::new();
                let _ = writeln!(s, "=== THERMAL TORTURE STATUS ===");
                let _ = writeln!(s, "• Progress            : Step {} / {}", completed, total);
                let _ = writeln!(s, "• Throttle State      : {}", throttle_str);
                let _ = writeln!(s, "• Sustained Stability : {:.1}% (3DMark Style)", self.torture_analysis.stability_percent);
                let _ = writeln!(s, "• Baseline Boost      : {:.0} ops/sec", self.torture_analysis.baseline_ops_per_sec);
                let _ = writeln!(s, "• Rolling Window Min  : {:.0} ops/sec", self.torture_analysis.rolling_min_ops_per_sec);
                let _ = writeln!(s, "• Rolling Window Max  : {:.0} ops/sec", self.torture_analysis.rolling_max_ops_per_sec);
                let _ = writeln!(s, "• Hardware Integrity  : {}", corruption_str);

                if !self.torture_runner.samples().is_empty() {
                    let _ = writeln!(s, "--------------------------------------------------------");
                    let _ = writeln!(s, "Recent Samples (last 5):");
                    for (idx, sample) in self.torture_runner.samples().iter().rev().take(5).enumerate() {
                        let _ = writeln!(
                            s,
                            "  [step -{}] {}ms: {:.0} ops/sec, corrupt={}",
                            idx, sample.elapsed_ms, sample.ops_per_sec, sample.corruption_detected
                        );
                    }
                }

                ui.set_text(self.display_text, &s);
                ui.set_text(
                    self.status,
                    &format!("Status: Torture Step {}/{} — Stability {:.1}%", completed, total, self.torture_analysis.stability_percent),
                );
            }
        }
    }

    /// Executes the full benchmark suite using the provided monotonic millisecond clock function.
    pub fn run_bench_suite<F>(&mut self, now_ms: F) -> &SuiteResult
    where
        F: Fn() -> u64,
    {
        // 1. Integer ARX
        let t0 = now_ms();
        let (arx_ops, arx_ck) = bench_crypto_arx(10_000, 0x1234_5678);
        let t1 = now_ms();
        let arx_elapsed_ms = t1.saturating_sub(t0);
        let arx_ops_per_sec = (arx_ops as f64) / ((arx_elapsed_ms.max(1) as f64) / 1000.0);

        // 2. SHA-256
        let t0 = now_ms();
        let (sha_ops, sha_ck) = bench_sha256(2_000, 0xcafe_babe);
        let t1 = now_ms();
        let sha_elapsed_ms = t1.saturating_sub(t0);
        let sha_ops_per_sec = (sha_ops as f64) / ((sha_elapsed_ms.max(1) as f64) / 1000.0);

        // 3. GEMM FP32 (tiled 64x64)
        let t0 = now_ms();
        let (gemm_ops, gemm_ck) = bench_tiled_gemm_f32(64, 10);
        let t1 = now_ms();
        let gemm_elapsed_ms = t1.saturating_sub(t0);
        let gemm_ops_per_sec = (gemm_ops as f64) / ((gemm_elapsed_ms.max(1) as f64) / 1000.0);

        // 4. DSP Audio Filter
        let t0 = now_ms();
        let (dsp_ops, dsp_ck) = bench_dsp_f32(1024, 500);
        let t1 = now_ms();
        let dsp_elapsed_ms = t1.saturating_sub(t0);
        let dsp_ops_per_sec = (dsp_ops as f64) / ((dsp_elapsed_ms.max(1) as f64) / 1000.0);

        // 5. STREAM Memory Bandwidth
        let t0 = now_ms();
        let stream_res = bench_stream(4096, 100);
        let t1 = now_ms();
        let stream_elapsed_ms = t1.saturating_sub(t0);
        let stream_bytes_per_sec = (stream_res.bytes_processed as f64) / ((stream_elapsed_ms.max(1) as f64) / 1000.0);

        // 6. Sattolo Cache/TLB Pointer-Chase Ladder
        let t0 = now_ms();
        let ladder_steps = bench_pointer_chase_ladder(16 * 1024 * 1024, 20_000);
        let t1 = now_ms();
        let ladder_elapsed_ms = t1.saturating_sub(t0);
        let ladder_hops: u64 = ladder_steps.iter().map(|s| s.hops as u64).sum();
        let ladder_ck: u64 = ladder_steps.iter().map(|s| s.checksum as u64).sum();
        let ladder_hops_per_sec = (ladder_hops as f64) / ((ladder_elapsed_ms.max(1) as f64) / 1000.0);

        let scores = SubsystemScores::compute(
            arx_ops,
            sha_ops,
            gemm_ops,
            dsp_ops,
            stream_res.bytes_processed,
            ladder_hops,
        );

        let json_payload = format!(
            "{{\"target\":\"sexos\",\"arx\":{{\"ops\":{},\"elapsed_ms\":{},\"ops_per_sec\":{:.1},\"checksum\":{}}},\"sha256\":{{\"ops\":{},\"elapsed_ms\":{},\"ops_per_sec\":{:.1},\"checksum\":{}}},\"gemm\":{{\"dim\":64,\"ops\":{},\"elapsed_ms\":{},\"ops_per_sec\":{:.1},\"checksum\":{}}},\"dsp\":{{\"samples\":1024,\"ops\":{},\"elapsed_ms\":{},\"ops_per_sec\":{:.1},\"checksum\":{}}},\"stream\":{{\"len\":4096,\"bytes_processed\":{},\"elapsed_ms\":{},\"bytes_per_sec\":{:.1},\"checksum\":{}}},\"ladder\":{{\"steps\":{},\"hops\":{},\"elapsed_ms\":{},\"hops_per_sec\":{:.1},\"checksum\":{}}},\"scores\":{{\"composite\":{},\"cpu_int\":{},\"sha256\":{},\"gemm_fp\":{},\"dsp_fp\":{},\"memory_bandwidth\":{},\"pointer_latency\":{}}}}}",
            arx_ops, arx_elapsed_ms, arx_ops_per_sec, arx_ck,
            sha_ops, sha_elapsed_ms, sha_ops_per_sec, sha_ck,
            gemm_ops, gemm_elapsed_ms, gemm_ops_per_sec, gemm_ck,
            dsp_ops, dsp_elapsed_ms, dsp_ops_per_sec, dsp_ck,
            stream_res.bytes_processed, stream_elapsed_ms, stream_bytes_per_sec, stream_res.checksum,
            ladder_steps.len(), ladder_hops, ladder_elapsed_ms, ladder_hops_per_sec, ladder_ck,
            scores.composite, scores.cpu_int, scores.sha256, scores.gemm_fp, scores.dsp_fp, scores.memory_bandwidth, scores.pointer_latency
        );

        let res = SuiteResult {
            arx_ops,
            arx_elapsed_ms,
            arx_ops_per_sec,
            arx_checksum: arx_ck,
            sha_ops,
            sha_elapsed_ms,
            sha_ops_per_sec,
            sha_checksum: sha_ck,
            gemm_ops,
            gemm_elapsed_ms,
            gemm_ops_per_sec,
            gemm_checksum: gemm_ck,
            dsp_ops,
            dsp_elapsed_ms,
            dsp_ops_per_sec,
            dsp_checksum: dsp_ck,
            stream_res,
            stream_elapsed_ms,
            stream_bytes_per_sec,
            ladder_steps,
            ladder_elapsed_ms,
            ladder_hops,
            ladder_hops_per_sec,
            ladder_checksum: ladder_ck,
            scores,
            json_payload,
        };

        self.suite_ran = true;
        self.suite_result = Some(res);
        self.suite_result.as_ref().unwrap()
    }

    /// Advances the torture runner by one step and re-analyzes thermal stability.
    pub fn step_torture(&mut self) -> Option<Sample> {
        if !self.torture_runner.is_finished() {
            let sample = self.torture_runner.run_step(None, None).ok()?;
            self.torture_analysis = ThermalAnalyzer::new().analyze(self.torture_runner.samples());
            Some(sample)
        } else {
            None
        }
    }

    /// Advances the torture runner by up to `count` steps.
    pub fn burst_torture(&mut self, count: usize) -> usize {
        let mut ran = 0;
        for _ in 0..count {
            if self.step_torture().is_some() {
                ran += 1;
            } else {
                break;
            }
        }
        ran
    }

    /// Resets the torture runner to initial zeroed state.
    pub fn reset_torture(&mut self) {
        let torture_config = StressConfig {
            workload: WorkloadType::BalancedMix,
            step_duration_ms: 100,
            total_steps: 20,
            memory_quota_bytes: 4 * 1024 * 1024,
            iters_per_slice: 100,
        };
        self.torture_runner = TortureRunner::new(torture_config).expect("valid stress config");
        self.torture_analysis = ThermalAnalyzer::new().analyze(self.torture_runner.samples());
    }

    /// Handles a Weft user interface response event. Returns true if UI changed.
    pub fn on<F>(&mut self, ui: &mut Ui, response: &Response, now_ms: F) -> bool
    where
        F: Fn() -> u64,
    {
        let mut command = None;
        if let Response::Clicked(id) = response {
            if *id == self.choices[0] {
                command = Some("climax.benchmarks");
            } else if *id == self.choices[1] {
                command = Some("climax.torture");
            } else if *id == self.run_suite {
                command = Some("climax.run_suite");
            } else if *id == self.start_step {
                command = Some("climax.start_step");
            } else if *id == self.run_burst {
                command = Some("climax.run_burst");
            } else if *id == self.reset {
                command = Some("climax.reset");
            }
        }

        if let Response::Command(cmd) = response {
            command = Some(cmd.as_str());
        }

        let Some(cmd) = command else {
            return false;
        };

        match cmd {
            "climax.benchmarks" => {
                self.mode = ClimaxMode::Benchmarks;
                self.refresh(ui);
                true
            }
            "climax.torture" => {
                self.mode = ClimaxMode::Thermal;
                self.refresh(ui);
                true
            }
            "climax.run_suite" => {
                self.run_bench_suite(now_ms);
                self.refresh(ui);
                true
            }
            "climax.start_step" => {
                self.step_torture();
                self.refresh(ui);
                true
            }
            "climax.run_burst" => {
                self.burst_torture(5);
                self.refresh(ui);
                true
            }
            "climax.reset" => {
                self.reset_torture();
                self.refresh(ui);
                true
            }
            _ => false,
        }
    }
}
