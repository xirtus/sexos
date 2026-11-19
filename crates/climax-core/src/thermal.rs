//! Thermal throttling analyzer and sustained stability scoring engine.

use crate::stress::Sample;

/// Thermal throttling state of the system during test execution.
#[derive(Clone, Debug, PartialEq)]
pub enum ThrottleState {
    /// Insufficient measurement steps to determine throttling state.
    Warmup,
    /// Throughput sustained within tolerance throughout the test run.
    Sustained,
    /// Thermal throttling detected with drop percentage and onset time.
    Throttled {
        /// Percentage drop relative to baseline boost throughput (e.g. 18.5%).
        drop_percent: f64,
        /// Elapsed time in milliseconds at which throttling began.
        onset_ms: u64,
    },
}

/// Comprehensive analysis of thermal and performance stability.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalAnalysis {
    /// Initial boost phase baseline throughput in ops/sec.
    pub baseline_ops_per_sec: f64,
    /// Minimum rolling window throughput observed.
    pub rolling_min_ops_per_sec: f64,
    /// Maximum rolling window throughput observed.
    pub rolling_max_ops_per_sec: f64,
    /// Sustained stability score (0.0..=100.0%).
    pub stability_percent: f64,
    /// Verdict on thermal throttling.
    pub throttle_state: ThrottleState,
    /// Peak temperature recorded in degrees Celsius.
    pub peak_temp_celsius: Option<i16>,
    /// Average temperature across all measured steps in degrees Celsius.
    pub avg_temp_celsius: Option<f32>,
    /// Temperature recorded at the onset of thermal throttling.
    pub throttle_onset_temp: Option<i16>,
    /// Whether any bit-exact data corruption was detected during the run.
    pub corruption_detected: bool,
}

/// Analyzes time-series samples to detect thermal throttling and sustained stability.
pub struct ThermalAnalyzer {
    /// Percentage drop from baseline considered thermal throttling (default 5.0%).
    pub throttle_threshold_percent: f64,
}

impl Default for ThermalAnalyzer {
    fn default() -> Self {
        Self {
            throttle_threshold_percent: 5.0,
        }
    }
}

impl ThermalAnalyzer {
    /// Creates a new thermal analyzer with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyzes a slice of time-series samples captured by the torture runner.
    pub fn analyze(&self, samples: &[Sample]) -> ThermalAnalysis {
        if samples.len() < 3 {
            let baseline = samples.first().map(|s| s.ops_per_sec).unwrap_or(0.0);
            let baseline = if baseline.is_finite() && baseline >= 0.0 {
                baseline
            } else {
                0.0
            };
            return ThermalAnalysis {
                baseline_ops_per_sec: baseline,
                rolling_min_ops_per_sec: 0.0,
                rolling_max_ops_per_sec: 0.0,
                stability_percent: 100.0,
                throttle_state: ThrottleState::Warmup,
                peak_temp_celsius: None,
                avg_temp_celsius: None,
                throttle_onset_temp: None,
                corruption_detected: samples.iter().any(|s| s.corruption_detected),
            };
        }

        // Check for bit-exact corruption across any step
        let corruption_detected = samples.iter().any(|s| s.corruption_detected);

        // 1. Compute baseline boost throughput from the first 20% of samples (at least 2 samples)
        let baseline_sample_count = (samples.len() / 5).max(2).min(samples.len() - 1);
        let baseline_sum: f64 = samples[..baseline_sample_count]
            .iter()
            .map(|s| {
                if s.ops_per_sec.is_finite() && s.ops_per_sec >= 0.0 {
                    s.ops_per_sec
                } else {
                    0.0
                }
            })
            .sum();
        let baseline = if baseline_sample_count > 0 {
            baseline_sum / (baseline_sample_count as f64)
        } else {
            0.0
        };

        // 2. Rolling window throughput evaluation
        let window_size = (samples.len() / 4).max(2);
        let mut rolling_min = f64::MAX;
        let mut rolling_max = 0.0f64;

        let mut throttled_onset_ms = None;
        let mut throttled_onset_temp = None;
        let mut max_drop_percent = 0.0f64;

        for i in 0..=samples.len() - window_size {
            let window = &samples[i..i + window_size];
            let avg: f64 = if window_size > 0 {
                window
                    .iter()
                    .map(|s| {
                        if s.ops_per_sec.is_finite() && s.ops_per_sec >= 0.0 {
                            s.ops_per_sec
                        } else {
                            0.0
                        }
                    })
                    .sum::<f64>()
                    / (window_size as f64)
            } else {
                0.0
            };

            if avg < rolling_min {
                rolling_min = avg;
            }
            if avg > rolling_max {
                rolling_max = avg;
            }

            // Check if drop relative to baseline exceeds throttle threshold
            if baseline > 0.0 && avg < baseline {
                let drop_pct = ((baseline - avg) / baseline) * 100.0;
                if drop_pct >= self.throttle_threshold_percent {
                    if throttled_onset_ms.is_none() && i >= baseline_sample_count {
                        throttled_onset_ms = Some(window[0].elapsed_ms);
                        throttled_onset_temp = window[0].temp_celsius;
                    }
                    if drop_pct > max_drop_percent {
                        max_drop_percent = drop_pct;
                    }
                }
            }
        }

        let throttle_state = if let Some(onset_ms) = throttled_onset_ms {
            ThrottleState::Throttled {
                drop_percent: max_drop_percent,
                onset_ms,
            }
        } else {
            ThrottleState::Sustained
        };

        // 3. Compute stability percentage (0.0..=100.0%)
        let raw_stability = if baseline > 0.0 && rolling_min.is_finite() && rolling_min < f64::MAX {
            (rolling_min / baseline) * 100.0
        } else {
            100.0
        };
        let mut stability_percent = if !raw_stability.is_finite() || raw_stability < 0.0 {
            0.0
        } else if raw_stability > 100.0 {
            100.0
        } else {
            raw_stability
        };

        // Hardware corruption drops stability score to zero immediately!
        if corruption_detected {
            stability_percent = 0.0;
        }

        // 4. Temperature metrics
        let mut peak_temp = None;
        let mut temp_sum = 0i64;
        let mut temp_count = 0;

        for s in samples {
            if let Some(t) = s.temp_celsius {
                peak_temp = Some(peak_temp.map_or(t, |p: i16| p.max(t)));
                temp_sum += t as i64;
                temp_count += 1;
            }
        }

        let avg_temp = if temp_count > 0 {
            Some((temp_sum as f32) / (temp_count as f32))
        } else {
            None
        };

        ThermalAnalysis {
            baseline_ops_per_sec: baseline,
            rolling_min_ops_per_sec: if rolling_min < f64::MAX { rolling_min } else { 0.0 },
            rolling_max_ops_per_sec: rolling_max,
            stability_percent,
            throttle_state,
            peak_temp_celsius: peak_temp,
            avg_temp_celsius: avg_temp,
            throttle_onset_temp: throttled_onset_temp,
            corruption_detected,
        }
    }
}
