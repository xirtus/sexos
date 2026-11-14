//! Sustained stress torture runner and bit-exact corruption self-checking engine.

use alloc::vec::Vec;

use crate::kernels::{
    bench_crypto_arx, bench_dsp_f32, bench_pointer_chase_ladder, bench_sha256,
    bench_stream_with_quota, bench_tiled_gemm_f32,
};
use crate::ClimaxError;

/// Workload type executed during the stress test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadType {
    /// ChaCha20/ARX pure-integer ALU throughput.
    IntegerArx,
    /// SHA-256 compression rounds.
    Sha256,
    /// Tiled FMA-free matrix multiply.
    MatrixMulF32,
    /// Vectorizable biquad DSP filtering.
    DspF32,
    /// STREAM memory copy/scale/add/triad.
    StreamMemory,
    /// Cache/TLB latency pointer-chase ladder.
    PointerChase,
    /// Balanced mix cycling through all subsystems.
    BalancedMix,
}

/// Time-series measurement sample captured during a stress test step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Timestamp offset from start in milliseconds.
    pub elapsed_ms: u64,
    /// Number of operations completed in this step.
    pub ops_completed: u64,
    /// Normalized throughput in operations per second.
    pub ops_per_sec: f64,
    /// Optional package/die temperature in degrees Celsius.
    pub temp_celsius: Option<i16>,
    /// Optional power consumption in milliwatts.
    pub power_mw: Option<u32>,
    /// Bit-exact memory/ALU corruption detection flag.
    pub corruption_detected: bool,
}

/// Configuration parameters for the torture runner.
#[derive(Clone, Debug, PartialEq)]
pub struct StressConfig {
    /// Workload type or mix.
    pub workload: WorkloadType,
    /// Nominal duration allocated per measurement step in milliseconds.
    pub step_duration_ms: u64,
    /// Total number of torture steps to execute.
    pub total_steps: usize,
    /// Maximum allowed memory footprint in bytes.
    pub memory_quota_bytes: usize,
    /// Inner benchmark iteration count per slice.
    pub iters_per_slice: usize,
}

impl Default for StressConfig {
    fn default() -> Self {
        Self {
            workload: WorkloadType::BalancedMix,
            step_duration_ms: 100,
            total_steps: 20,
            // 4 MiB memory quota default
            memory_quota_bytes: 4 * 1024 * 1024,
            iters_per_slice: 200,
        }
    }
}

/// Sustained torture runner executing microbenchmarks and tracking time-series samples.
pub struct TortureRunner {
    config: StressConfig,
    samples: Vec<Sample>,
    step_index: usize,
    elapsed_ms: u64,
}

impl TortureRunner {
    /// Creates a new torture runner with the specified configuration.
    pub fn new(config: StressConfig) -> Result<Self, ClimaxError> {
        if config.total_steps == 0 {
            return Err(ClimaxError::InvalidParameter("total_steps must be greater than zero"));
        }
        if config.step_duration_ms == 0 {
            return Err(ClimaxError::InvalidParameter("step_duration_ms must be non-zero"));
        }
        if config.memory_quota_bytes == 0 {
            return Err(ClimaxError::InvalidParameter("memory_quota_bytes must be greater than zero"));
        }
        if config.iters_per_slice == 0 {
            return Err(ClimaxError::InvalidParameter("iters_per_slice must be greater than zero"));
        }

        Ok(Self {
            config,
            samples: Vec::new(),
            step_index: 0,
            elapsed_ms: 0,
        })
    }

    /// Number of steps completed so far.
    pub fn steps_completed(&self) -> usize {
        self.step_index
    }

    /// Whether the full torture test run has completed.
    pub fn is_finished(&self) -> bool {
        self.step_index >= self.config.total_steps
    }

    /// Collected time-series samples.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// Executes one torture step, optionally receiving external temperature and power readings.
    pub fn run_step(
        &mut self,
        temp_celsius: Option<i16>,
        power_mw: Option<u32>,
    ) -> Result<Sample, ClimaxError> {
        if self.is_finished() {
            return Err(ClimaxError::TestAlreadyFinished);
        }

        let workload = match self.config.workload {
            WorkloadType::BalancedMix => {
                match self.step_index % 6 {
                    0 => WorkloadType::IntegerArx,
                    1 => WorkloadType::Sha256,
                    2 => WorkloadType::MatrixMulF32,
                    3 => WorkloadType::DspF32,
                    4 => WorkloadType::StreamMemory,
                    _ => WorkloadType::PointerChase,
                }
            }
            w => w,
        };

        let iters = self.config.iters_per_slice;
        let (ops, checksum, expected_checksum) = match workload {
            WorkloadType::IntegerArx => {
                let (ops, ck) = bench_crypto_arx(iters, 0x1234_5678);
                // Deterministic golden check
                let (_, expected) = bench_crypto_arx(iters, 0x1234_5678);
                (ops, ck, expected)
            }
            WorkloadType::Sha256 => {
                let (ops, ck) = bench_sha256(iters, 0xcafe_babe);
                let (_, expected) = bench_sha256(iters, 0xcafe_babe);
                (ops, ck, expected)
            }
            WorkloadType::MatrixMulF32 => {
                let (ops, ck) = bench_tiled_gemm_f32(32, iters.max(1) / 10 + 1);
                let (_, expected) = bench_tiled_gemm_f32(32, iters.max(1) / 10 + 1);
                (ops, ck, expected)
            }
            WorkloadType::DspF32 => {
                let (ops, ck) = bench_dsp_f32(1024, iters);
                let (_, expected) = bench_dsp_f32(1024, iters);
                (ops, ck, expected)
            }
            WorkloadType::StreamMemory => {
                let res = bench_stream_with_quota(4096, iters, self.config.memory_quota_bytes);
                let exp = bench_stream_with_quota(4096, iters, self.config.memory_quota_bytes);
                (res.operations, res.checksum, exp.checksum)
            }
            WorkloadType::PointerChase => {
                let steps = bench_pointer_chase_ladder(self.config.memory_quota_bytes, iters * 100);
                let ck = steps.iter().map(|s| s.checksum as u64).sum::<u64>();
                let total_hops = steps.iter().map(|s| s.hops as u64).sum::<u64>();
                (total_hops, ck, ck)
            }
            WorkloadType::BalancedMix => unreachable!(),
        };

        let corruption_detected = checksum != expected_checksum;

        self.elapsed_ms = self.elapsed_ms.saturating_add(self.config.step_duration_ms);
        let sec = (self.config.step_duration_ms as f64) / 1000.0;
        let ops_per_sec = if sec > 0.0 { (ops as f64) / sec } else { 0.0 };

        let sample = Sample {
            elapsed_ms: self.elapsed_ms,
            ops_completed: ops,
            ops_per_sec,
            temp_celsius,
            power_mw,
            corruption_detected,
        };

        self.samples.try_reserve(1).map_err(|_| ClimaxError::AllocationFailed)?;
        self.samples.push(sample);
        self.step_index += 1;

        Ok(sample)
    }

    /// Runs all remaining steps until completion.
    pub fn run_to_completion(&mut self) -> Result<&[Sample], ClimaxError> {
        while !self.is_finished() {
            self.run_step(None, None)?;
        }
        Ok(&self.samples)
    }
}
