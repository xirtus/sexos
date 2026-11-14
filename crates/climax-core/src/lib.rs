//! Climax Core — High-performance `#![no_std] + alloc` benchmark and stress test engine:
//! deterministic CPU/memory microbenchmarks, sustained torture runner, thermal throttling
//! detector, and Spindle/Weft visual report formatter.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod kernels;
pub mod report;
pub mod stress;
pub mod thermal;

pub use kernels::{
    bench_crypto_arx, bench_dsp_f32, bench_pointer_chase_ladder, bench_sha256,
    bench_stream, bench_stream_with_quota, bench_tiled_gemm_f32, generate_sattolo_cycle,
    LatencyStep, StreamResult,
};
pub use report::{compute_percentiles, BenchmarkReport, SubsystemScores};
pub use stress::{Sample, StressConfig, TortureRunner, WorkloadType};
pub use thermal::{ThermalAnalysis, ThermalAnalyzer, ThrottleState};

/// Top-level error type for Climax benchmark operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClimaxError {
    /// Invalid configuration parameter provided.
    InvalidParameter(&'static str),
    /// Attempted to run more steps when the test has already concluded.
    TestAlreadyFinished,
    /// Memory allocation cap exceeded during benchmark execution.
    AllocationFailed,
}
