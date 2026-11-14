extern crate alloc;

use alloc::vec;

use climax_core::kernels::{
    bench_crypto_arx, bench_dsp_f32, bench_pointer_chase_ladder, bench_sha256,
    bench_stream, bench_tiled_gemm_f32, generate_sattolo_cycle,
};
use climax_core::report::{compute_percentiles, BenchmarkReport, SubsystemScores};
use climax_core::stress::{Sample, StressConfig, TortureRunner, WorkloadType};
use climax_core::thermal::{ThermalAnalyzer, ThrottleState};

#[test]
fn test_crypto_arx_deterministic() {
    let (ops1, ck1) = bench_crypto_arx(100, 0x1234_5678);
    let (ops2, ck2) = bench_crypto_arx(100, 0x1234_5678);
    assert_eq!(ops1, ops2);
    assert_eq!(ck1, ck2);
    assert!(ops1 > 0);
    assert!(ck1 != 0);

    // Different seed produces different checksum
    let (_, ck3) = bench_crypto_arx(100, 0x8765_4321);
    assert_ne!(ck1, ck3);
}

#[test]
fn test_sha256_deterministic() {
    let (ops1, ck1) = bench_sha256(50, 0xdead_beef);
    let (ops2, ck2) = bench_sha256(50, 0xdead_beef);
    assert_eq!(ops1, ops2);
    assert_eq!(ck1, ck2);
    assert!(ops1 > 0);
}

#[test]
fn test_gemm_and_dsp_floating_point() {
    let (ops_gemm, ck_gemm) = bench_tiled_gemm_f32(16, 5);
    assert!(ops_gemm > 0);
    assert!(ck_gemm != 0);

    let (ops_dsp, ck_dsp) = bench_dsp_f32(512, 10);
    assert!(ops_dsp > 0);
    assert!(ck_dsp != 0);
}

#[test]
fn test_stream_memory_bandwidth() {
    let res = bench_stream(2048, 10);
    assert!(res.operations > 0);
    assert!(res.bytes_processed > 0);
    assert!(res.checksum != 0);
}

#[test]
fn test_sattolo_single_cycle_permutation() {
    let n = 256;
    let cycle = generate_sattolo_cycle(n, 0xcafe_babe);
    assert_eq!(cycle.len(), n);

    // Verify Sattolo single-cycle invariant:
    // Starting from 0 and following cycle[i], exactly n hops visit all n distinct nodes!
    let mut visited = vec![false; n];
    let mut cur = 0;
    for _ in 0..n {
        assert!(!visited[cur], "Premature subcycle detected in Sattolo permutation!");
        visited[cur] = true;
        cur = cycle[cur];
    }
    assert_eq!(cur, 0, "Sattolo cycle must return to start after exactly n hops");
    assert!(visited.iter().all(|&v| v), "Every node must be visited in a single cycle");
}

#[test]
fn test_pointer_chase_ladder_memory_quota() {
    // Quota of 512 KiB ensures ladder tests up to 256 KiB and skips 1 MiB+
    let ladder = bench_pointer_chase_ladder(512 * 1024, 1000);
    assert!(!ladder.is_empty());
    for step in &ladder {
        assert!(step.working_set_bytes <= 512 * 1024);
        assert_eq!(step.hops, 1000);
        assert!(step.checksum > 0);
    }
}

#[test]
fn test_torture_runner_and_corruption_detection() {
    let config = StressConfig {
        workload: WorkloadType::BalancedMix,
        step_duration_ms: 10,
        total_steps: 6,
        memory_quota_bytes: 1024 * 1024,
        iters_per_slice: 50,
    };

    let mut runner = TortureRunner::new(config).unwrap();
    assert_eq!(runner.steps_completed(), 0);

    while !runner.is_finished() {
        let sample = runner.run_step(Some(65), Some(25000)).unwrap();
        assert!(!sample.corruption_detected, "Clean test should not detect corruption");
        assert!(sample.ops_per_sec > 0.0);
    }

    assert_eq!(runner.samples().len(), 6);
}

#[test]
fn test_thermal_analyzer_sustained_vs_throttled() {
    let analyzer = ThermalAnalyzer::new();

    // 1. Sustained workload (steady throughput ~10,000 ops/s)
    let sustained_samples = vec![
        Sample { elapsed_ms: 100, ops_completed: 1000, ops_per_sec: 10000.0, temp_celsius: Some(50), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 200, ops_completed: 1000, ops_per_sec: 10050.0, temp_celsius: Some(55), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 300, ops_completed: 1000, ops_per_sec: 9980.0,  temp_celsius: Some(60), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 400, ops_completed: 1000, ops_per_sec: 10020.0, temp_celsius: Some(62), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 500, ops_completed: 1000, ops_per_sec: 9990.0,  temp_celsius: Some(63), power_mw: None, corruption_detected: false },
    ];
    let sustained_res = analyzer.analyze(&sustained_samples);
    assert_eq!(sustained_res.throttle_state, ThrottleState::Sustained);
    assert!(sustained_res.stability_percent >= 98.0);
    assert!(!sustained_res.corruption_detected);

    // 2. Throttled workload (boost at 10,000, drops to 6,000 at 95°C)
    let throttled_samples = vec![
        Sample { elapsed_ms: 100, ops_completed: 1000, ops_per_sec: 10000.0, temp_celsius: Some(70), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 200, ops_completed: 1000, ops_per_sec: 10000.0, temp_celsius: Some(85), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 300, ops_completed: 600,  ops_per_sec: 6000.0,  temp_celsius: Some(96), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 400, ops_completed: 600,  ops_per_sec: 6000.0,  temp_celsius: Some(97), power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 500, ops_completed: 600,  ops_per_sec: 5900.0,  temp_celsius: Some(98), power_mw: None, corruption_detected: false },
    ];
    let throttled_res = analyzer.analyze(&throttled_samples);
    match throttled_res.throttle_state {
        ThrottleState::Throttled { drop_percent, onset_ms } => {
            assert!(drop_percent > 30.0, "Expected drop percent > 30%, got {}", drop_percent);
            assert!(onset_ms > 0);
        }
        _ => panic!("Expected Throttled state"),
    }
    assert!(throttled_res.stability_percent < 70.0);

    // 3. Corruption test: hardware fault forces stability to 0.0%
    let mut corrupt_samples = sustained_samples.clone();
    corrupt_samples[2].corruption_detected = true;
    let corrupt_res = analyzer.analyze(&corrupt_samples);
    assert!(corrupt_res.corruption_detected);
    assert_eq!(corrupt_res.stability_percent, 0.0);
}

#[test]
fn test_report_and_percentiles() {
    let throughputs = vec![1000.0, 5000.0, 2000.0, 8000.0, 10000.0];
    let (p50, p95, p99) = compute_percentiles(throughputs);
    assert_eq!(p50, 5000.0);
    assert_eq!(p95, 10000.0);
    assert_eq!(p99, 10000.0);

    let scores = SubsystemScores::compute(100000, 80000, 120000, 90000, 50000000, 20000);
    assert!(scores.composite > 0);
    assert!(scores.cpu_int > 0);

    let thermal = ThermalAnalyzer::new().analyze(&[
        Sample { elapsed_ms: 100, ops_completed: 100, ops_per_sec: 1000.0, temp_celsius: Some(45), power_mw: Some(15000), corruption_detected: false },
        Sample { elapsed_ms: 200, ops_completed: 100, ops_per_sec: 1000.0, temp_celsius: Some(46), power_mw: Some(15000), corruption_detected: false },
        Sample { elapsed_ms: 300, ops_completed: 100, ops_per_sec: 1000.0, temp_celsius: Some(46), power_mw: Some(15000), corruption_detected: false },
    ]);

    let report = BenchmarkReport {
        scores,
        percentiles: (1000.0, 1000.0, 1000.0),
        thermal,
        peak_memory_bytes: 4 * 1024 * 1024,
    };

    let rendered = report.render_terminal_report();
    assert!(rendered.contains("CLIMAX BENCHMARK REPORT"));
    assert!(rendered.contains("Composite Climax Score"));
    assert!(rendered.contains("PASS (0 bit flips)"));
}

#[test]
fn test_sattolo_empty_and_single_and_various_sizes() {
    // n == 0: must return empty vector without panicking or % 0
    assert!(generate_sattolo_cycle(0, 42).is_empty());

    // n == 1: must return [0]
    assert_eq!(generate_sattolo_cycle(1, 42), vec![0]);

    // Test multiple small and prime sizes for single-cycle property
    for &n in &[2, 3, 5, 7, 16, 31, 64] {
        let cycle = generate_sattolo_cycle(n, 0x1234_5678_9abc_def0);
        assert_eq!(cycle.len(), n);
        let mut visited = vec![false; n];
        let mut cur = 0;
        for _ in 0..n {
            assert!(!visited[cur]);
            visited[cur] = true;
            cur = cycle[cur];
        }
        assert_eq!(cur, 0);
        assert!(visited.iter().all(|&v| v));
    }
}

#[test]
fn test_stream_memory_quota_and_bounds() {
    // Zero length
    let res0 = climax_core::bench_stream_with_quota(0, 10, 1024);
    assert_eq!(res0.operations, 0);
    assert_eq!(res0.bytes_processed, 0);

    // Zero quota
    let res_no_quota = climax_core::bench_stream_with_quota(1024, 10, 0);
    assert_eq!(res_no_quota.operations, 0);

    // Quota smaller than requested len: caps len to fit quota
    let res_capped = climax_core::bench_stream_with_quota(10000, 5, 1200); // 1200 bytes / 12 = 100 elements
    assert_eq!(res_capped.operations, 5 * 100 * 4);
    assert_eq!(res_capped.bytes_processed, 5 * 100 * 40);
}

#[test]
fn test_math_safety_zero_durations_and_empty_samples() {
    let analyzer = ThermalAnalyzer::new();

    // 1. Empty sample vector
    let empty_res = analyzer.analyze(&[]);
    assert_eq!(empty_res.baseline_ops_per_sec, 0.0);
    assert_eq!(empty_res.stability_percent, 100.0);
    assert_eq!(empty_res.throttle_state, ThrottleState::Warmup);

    // 2. Samples with 0 ops_per_sec (baseline == 0.0)
    let zero_samples = vec![
        Sample { elapsed_ms: 100, ops_completed: 0, ops_per_sec: 0.0, temp_celsius: None, power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 200, ops_completed: 0, ops_per_sec: 0.0, temp_celsius: None, power_mw: None, corruption_detected: false },
        Sample { elapsed_ms: 300, ops_completed: 0, ops_per_sec: 0.0, temp_celsius: None, power_mw: None, corruption_detected: false },
    ];
    let zero_res = analyzer.analyze(&zero_samples);
    assert_eq!(zero_res.baseline_ops_per_sec, 0.0);
    assert!(!zero_res.stability_percent.is_nan());
    assert_eq!(zero_res.stability_percent, 100.0);

    // 3. Subsystem scores compute on zero inputs: geometric mean must not be NaN
    let zero_scores = SubsystemScores::compute(0, 0, 0, 0, 0, 0);
    assert_eq!(zero_scores.composite, 1);
    assert_eq!(zero_scores.cpu_int, 1);

    // 4. Bar chart rendering with extreme scores (u64::MAX)
    let extreme_scores = SubsystemScores {
        cpu_int: u64::MAX,
        sha256: 1,
        gemm_fp: 1,
        dsp_fp: 1,
        memory_bandwidth: 1,
        pointer_latency: 1,
        composite: 1000,
    };
    let extreme_report = BenchmarkReport {
        scores: extreme_scores,
        percentiles: (0.0, 0.0, 0.0),
        thermal: empty_res,
        peak_memory_bytes: 1024,
    };
    let rendered = extreme_report.render_terminal_report();
    assert!(rendered.contains("CLIMAX BENCHMARK REPORT"));
}
