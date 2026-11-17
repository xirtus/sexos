//! Pure-Rust ports of **spectralizer** and **waveform** audio visualizers for Cabaret.
//!
//! Includes a pure-Rust radix-2 Hann-windowed FFT analyzer with logarithmic frequency binning
//! and gravity peak-cap physics, rendering [`BarSpectrumConfig`] (with stereo mirror),
//! [`CircularSpectrumConfig`], and a Stereo Phase Goniometer / Lissajous Scope directly to
//! [`cabaret_core::compositor::FrameBuffer`].

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::{FRAC_1_SQRT_2, PI};

use cabaret_core::compositor::{CompositorError, FrameBuffer, LinearRgba};

use crate::util::{
    clamp01, cosf, deg_to_rad, draw_line, fill_circle, fill_rect, fill_rounded_rect, floorf,
    lerp, log10f, powf, sinf, sqrtf,
};

/// Pure-Rust in-place radix-2 Cooley-Tukey FFT (`no_std`).
pub fn fft_radix2(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len().min(im.len());
    if n <= 1 || !n.is_power_of_two() {
        return;
    }

    let mut j = 0_usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while (j & bit) != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let sign = if inverse { 1.0_f32 } else { -1.0_f32 };
    let mut len = 2_usize;
    while len <= n {
        let ang = sign * 2.0 * PI / len as f32;
        let wlen_r = cosf(ang);
        let wlen_i = sinf(ang);
        let half = len >> 1;

        for start in (0..n).step_by(len) {
            let mut wr = 1.0_f32;
            let mut wi = 0.0_f32;
            for k in 0..half {
                let i0 = start + k;
                let i1 = i0 + half;
                let tr = re[i1] * wr - im[i1] * wi;
                let ti = re[i1] * wi + im[i1] * wr;
                re[i1] = re[i0] - tr;
                im[i1] = im[i0] - ti;
                re[i0] += tr;
                im[i0] += ti;

                let next_wr = wr * wlen_r - wi * wlen_i;
                wi = wr * wlen_i + wi * wlen_r;
                wr = next_wr;
            }
        }
        len <<= 1;
    }

    if inverse {
        let inv_n = 1.0 / n as f32;
        for i in 0..n {
            re[i] *= inv_n;
            im[i] *= inv_n;
        }
    }
}

/// Generates a symmetric Hann window of length `n`.
pub fn hann_window(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.5 * (1.0 - cosf(2.0 * PI * (i as f32) / denom)))
        .collect()
}

/// Configuration for [`SpectrumAnalyzer`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpectrumConfig {
    pub sample_rate_hz: u32,
    pub fft_size: usize,
    pub num_bins: usize,
    pub min_freq_hz: f32,
    pub max_freq_hz: f32,
    pub min_db: f32,
    pub max_db: f32,
    pub smoothing: f32,
    /// Peak-cap downward gravity acceleration in normalized height/sec^2.
    pub gravity: f32,
    pub peak_hold_ms: f32,
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 48_000,
            fft_size: 1024,
            num_bins: 32,
            min_freq_hz: 30.0,
            max_freq_hz: 18_000.0,
            min_db: -72.0,
            max_db: 0.0,
            smoothing: 0.5,
            gravity: 2.0,
            peak_hold_ms: 250.0,
        }
    }
}

/// Stateful Hann-windowed radix-2 FFT logarithmic spectrum analyzer with gravity peak caps.
#[derive(Clone, Debug)]
pub struct SpectrumAnalyzer {
    pub config: SpectrumConfig,
    window: Vec<f32>,
    window_sum: f32,
    pub levels_mono: Vec<f32>,
    pub levels_left: Vec<f32>,
    pub levels_right: Vec<f32>,
    pub peak_caps: Vec<f32>,
    peak_velocities: Vec<f32>,
    peak_hold_timers: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(mut config: SpectrumConfig) -> Self {
        let fft_size = config
            .fft_size
            .clamp(64, 4096)
            .next_power_of_two()
            .min(4096);
        config.fft_size = fft_size;
        config.num_bins = config.num_bins.max(4);
        config.sample_rate_hz = config.sample_rate_hz.max(8000);

        let window = hann_window(fft_size);
        let window_sum = window.iter().sum::<f32>().max(1e-6);
        let n = config.num_bins;

        Self {
            config,
            window,
            window_sum,
            levels_mono: vec![0.0; n],
            levels_left: vec![0.0; n],
            levels_right: vec![0.0; n],
            peak_caps: vec![0.0; n],
            peak_velocities: vec![0.0; n],
            peak_hold_timers: vec![0.0; n],
        }
    }

    /// Computes normalized logarithmic spectrum levels and updates gravity peak caps.
    pub fn analyze(&mut self, stereo_samples: &[[f32; 2]], dt_ms: f32) {
        let n_fft = self.config.fft_size;
        let mut mono_in = vec![0.0_f32; n_fft];
        let mut left_in = vec![0.0_f32; n_fft];
        let mut right_in = vec![0.0_f32; n_fft];

        let take = stereo_samples.len().min(n_fft);
        let offset = n_fft - take;
        for i in 0..take {
            let [l, r] = stereo_samples[stereo_samples.len() - take + i];
            left_in[offset + i] = l;
            right_in[offset + i] = r;
            mono_in[offset + i] = 0.5 * (l + r);
        }

        let raw_mono = self.compute_log_bands(&mono_in);
        let raw_left = self.compute_log_bands(&left_in);
        let raw_right = self.compute_log_bands(&right_in);

        let sm = self.config.smoothing.clamp(0.0, 0.96);
        let dt_s = dt_ms.max(0.0) / 1000.0;

        for i in 0..self.config.num_bins {
            self.levels_mono[i] = lerp(raw_mono[i], self.levels_mono[i], sm);
            self.levels_left[i] = lerp(raw_left[i], self.levels_left[i], sm);
            self.levels_right[i] = lerp(raw_right[i], self.levels_right[i], sm);

            let lvl = self.levels_mono[i];
            if lvl >= self.peak_caps[i] {
                self.peak_caps[i] = lvl;
                self.peak_velocities[i] = 0.0;
                self.peak_hold_timers[i] = self.config.peak_hold_ms;
            } else if self.peak_hold_timers[i] > 0.0 {
                self.peak_hold_timers[i] = (self.peak_hold_timers[i] - dt_ms.max(0.0)).max(0.0);
            } else {
                self.peak_velocities[i] += self.config.gravity * dt_s;
                self.peak_caps[i] = (self.peak_caps[i] - self.peak_velocities[i] * dt_s).max(lvl);
            }
        }
    }

    /// Returns the center frequency in Hz for logarithmic bin `bin_idx`.
    pub fn bin_center_freq_hz(&self, bin_idx: usize) -> f32 {
        let f_min = self.config.min_freq_hz.max(10.0);
        let nyquist = self.config.sample_rate_hz as f32 * 0.49;
        let f_max = self.config.max_freq_hz.clamp(f_min + 10.0, nyquist);
        let ratio = f_max / f_min;
        let n = self.config.num_bins.max(1) as f32;
        let t0 = (bin_idx.min(self.config.num_bins - 1) as f32) / n;
        let t1 = ((bin_idx.min(self.config.num_bins - 1) + 1) as f32) / n;
        f_min * powf(ratio, 0.5 * (t0 + t1))
    }

    fn compute_log_bands(&self, time_domain: &[f32]) -> Vec<f32> {
        let n = self.config.fft_size;
        let mut re = vec![0.0_f32; n];
        let mut im = vec![0.0_f32; n];
        for i in 0..n {
            re[i] = time_domain[i] * self.window[i];
        }
        fft_radix2(&mut re, &mut im, false);

        let half = n / 2;
        let bin_hz = self.config.sample_rate_hz as f32 / n as f32;
        let f_min = self.config.min_freq_hz.max(10.0);
        let nyquist = self.config.sample_rate_hz as f32 * 0.49;
        let f_max = self.config.max_freq_hz.clamp(f_min + 10.0, nyquist);
        let ratio = f_max / f_min;
        let db_span = (self.config.max_db - self.config.min_db).max(1.0);

        let mut bands = vec![0.0_f32; self.config.num_bins];
        for b in 0..self.config.num_bins {
            let t0 = b as f32 / self.config.num_bins as f32;
            let t1 = (b + 1) as f32 / self.config.num_bins as f32;
            let flo = f_min * powf(ratio, t0);
            let fhi = f_min * powf(ratio, t1);
            let k_lo_f = flo / bin_hz;
            let k_hi_f = fhi / bin_hz;

            let k_lo = (floorf(k_lo_f) as usize).clamp(1, half.saturating_sub(1));
            let k_hi = (floorf(k_hi_f) as usize).clamp(k_lo, half.saturating_sub(1));

            let mut max_mag = 0.0_f32;
            for k in k_lo..=k_hi {
                let mag = 2.0 * sqrtf(re[k] * re[k] + im[k] * im[k]) / self.window_sum;
                max_mag = max_mag.max(mag);
            }
            // Linear interpolation if band sits between two adjacent bins.
            if k_lo == k_hi && k_lo + 1 < half {
                let frac = k_lo_f - floorf(k_lo_f);
                let mag0 = 2.0 * sqrtf(re[k_lo] * re[k_lo] + im[k_lo] * im[k_lo]) / self.window_sum;
                let mag1 =
                    2.0 * sqrtf(re[k_lo + 1] * re[k_lo + 1] + im[k_lo + 1] * im[k_lo + 1])
                        / self.window_sum;
                max_mag = max_mag.max(lerp(mag0, mag1, clamp01(frac)));
            }

            let db = 20.0 * log10f(max_mag.max(1e-7));
            bands[b] = clamp01((db - self.config.min_db) / db_span);
        }
        bands
    }
}

/// Configuration for rendering a logarithmic bar spectrum (`BarSpectrum`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarSpectrumConfig {
    pub width: u32,
    pub height: u32,
    pub bg_color: LinearRgba,
    pub bar_color_low: LinearRgba,
    pub bar_color_high: LinearRgba,
    pub peak_cap_color: LinearRgba,
    pub bar_gap_px: u32,
    pub corner_radius_px: f32,
    pub show_peak_caps: bool,
    pub stereo_mirror: bool,
}

impl Default for BarSpectrumConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 200,
            bg_color: [0.02, 0.02, 0.04, 0.85],
            bar_color_low: [0.05, 0.78, 0.95, 1.0],
            bar_color_high: [0.98, 0.25, 0.55, 1.0],
            peak_cap_color: [1.0, 0.95, 0.85, 1.0],
            bar_gap_px: 2,
            corner_radius_px: 2.0,
            show_peak_caps: true,
            stereo_mirror: false,
        }
    }
}

/// Renders a logarithmic frequency bar spectrum with gravity peak-caps and optional stereo mirror.
pub fn render_bar_spectrum(
    analyzer: &SpectrumAnalyzer,
    cfg: &BarSpectrumConfig,
) -> Result<FrameBuffer, CompositorError> {
    let mut fb = FrameBuffer::filled(cfg.width, cfg.height, cfg.bg_color)?;
    let n = analyzer.levels_mono.len();
    if n == 0 {
        return Ok(fb);
    }

    let gap = cfg.bar_gap_px as f32;
    let total_gap = gap * (n.saturating_sub(1)) as f32;
    let bar_w = ((cfg.width as f32 - total_gap) / n as f32).max(1.0);
    let hf = cfg.height as f32;

    for i in 0..n {
        let bx = i as f32 * (bar_w + gap);
        let t_bin = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 };
        let col = [
            lerp(cfg.bar_color_low[0], cfg.bar_color_high[0], t_bin),
            lerp(cfg.bar_color_low[1], cfg.bar_color_high[1], t_bin),
            lerp(cfg.bar_color_low[2], cfg.bar_color_high[2], t_bin),
            lerp(cfg.bar_color_low[3], cfg.bar_color_high[3], t_bin),
        ];

        if cfg.stereo_mirror {
            let mid = hf * 0.5;
            let lh = clamp01(analyzer.levels_left[i]) * (mid - 2.0);
            let rh = clamp01(analyzer.levels_right[i]) * (mid - 2.0);
            if lh > 0.5 {
                fill_rounded_rect(&mut fb, bx, mid - lh, bar_w, lh, cfg.corner_radius_px, col);
            }
            if rh > 0.5 {
                fill_rounded_rect(&mut fb, bx, mid, bar_w, rh, cfg.corner_radius_px, col);
            }
            if cfg.show_peak_caps {
                let p = clamp01(analyzer.peak_caps[i]) * (mid - 2.0);
                fill_rect(&mut fb, bx as i32, (mid - p - 2.0) as i32, bar_w.max(1.0) as i32, 2, cfg.peak_cap_color);
                fill_rect(&mut fb, bx as i32, (mid + p) as i32, bar_w.max(1.0) as i32, 2, cfg.peak_cap_color);
            }
        } else {
            let h_bar = clamp01(analyzer.levels_mono[i]) * (hf - 4.0);
            if h_bar > 0.5 {
                fill_rounded_rect(
                    &mut fb,
                    bx,
                    hf - h_bar,
                    bar_w,
                    h_bar,
                    cfg.corner_radius_px,
                    col,
                );
            }
            if cfg.show_peak_caps {
                let py = (hf - clamp01(analyzer.peak_caps[i]) * (hf - 4.0) - 2.0).clamp(0.0, hf - 2.0);
                fill_rect(
                    &mut fb,
                    bx as i32,
                    py as i32,
                    bar_w.max(1.0) as i32,
                    2,
                    cfg.peak_cap_color,
                );
            }
        }
    }

    Ok(fb)
}

/// Configuration for rendering a radial [`CircularSpectrumConfig`] visualizer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircularSpectrumConfig {
    pub width: u32,
    pub height: u32,
    pub bg_color: LinearRgba,
    pub bar_color: LinearRgba,
    pub peak_color: LinearRgba,
    pub inner_radius_px: f32,
    pub max_bar_length_px: f32,
    pub bar_thickness_px: f32,
    pub rotation_deg: f32,
    pub show_peak_caps: bool,
}

impl Default for CircularSpectrumConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 320,
            bg_color: [0.01, 0.01, 0.03, 0.85],
            bar_color: [0.15, 0.85, 1.0, 1.0],
            peak_color: [1.0, 0.4, 0.8, 1.0],
            inner_radius_px: 52.0,
            max_bar_length_px: 88.0,
            bar_thickness_px: 4.0,
            rotation_deg: -90.0,
            show_peak_caps: true,
        }
    }
}

/// Renders a radial circular spectrum visualizer around the canvas center.
pub fn render_circular_spectrum(
    analyzer: &SpectrumAnalyzer,
    cfg: &CircularSpectrumConfig,
) -> Result<FrameBuffer, CompositorError> {
    let mut fb = FrameBuffer::filled(cfg.width, cfg.height, cfg.bg_color)?;
    let n = analyzer.levels_mono.len();
    if n == 0 {
        return Ok(fb);
    }

    let cx = cfg.width as f32 * 0.5;
    let cy = cfg.height as f32 * 0.5;
    let rot = deg_to_rad(cfg.rotation_deg);

    for i in 0..n {
        let ang = rot + (2.0 * PI * i as f32) / n as f32;
        let dx = cosf(ang);
        let dy = sinf(ang);
        let r0 = cfg.inner_radius_px.max(2.0);
        let len = clamp01(analyzer.levels_mono[i]) * cfg.max_bar_length_px.max(2.0);
        let r1 = r0 + len.max(1.5);
        draw_line(
            &mut fb,
            cx + dx * r0,
            cy + dy * r0,
            cx + dx * r1,
            cy + dy * r1,
            cfg.bar_thickness_px,
            cfg.bar_color,
        );

        if cfg.show_peak_caps {
            let rp = r0 + clamp01(analyzer.peak_caps[i]) * cfg.max_bar_length_px.max(2.0) + 2.0;
            fill_circle(
                &mut fb,
                cx + dx * rp,
                cy + dy * rp,
                (cfg.bar_thickness_px * 0.55).max(1.5),
                cfg.peak_color,
            );
        }
    }

    Ok(fb)
}

/// Configuration for the Stereo Phase Goniometer / Lissajous vectorscope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoniometerConfig {
    pub width: u32,
    pub height: u32,
    pub bg_color: LinearRgba,
    pub trace_color: LinearRgba,
    pub reticle_color: LinearRgba,
    pub trace_thickness_px: f32,
    pub show_reticle: bool,
    pub persistence_decay: f32,
}

impl Default for GoniometerConfig {
    fn default() -> Self {
        Self {
            width: 256,
            height: 256,
            bg_color: [0.02, 0.03, 0.03, 0.92],
            trace_color: [0.1, 1.0, 0.55, 0.95],
            reticle_color: [0.22, 0.28, 0.28, 0.6],
            trace_thickness_px: 1.6,
            show_reticle: true,
            persistence_decay: 0.75,
        }
    }
}

/// Computes normalized stereo phase correlation `rho in [-1.0, +1.0]` (`+1` = mono, `0` = uncorrelated, `-1` = anti-phase).
pub fn compute_stereo_correlation(stereo_samples: &[[f32; 2]]) -> f32 {
    let mut sum_lr = 0.0_f64;
    let mut sum_l2 = 0.0_f64;
    let mut sum_r2 = 0.0_f64;
    for &[l, r] in stereo_samples {
        let lf = l as f64;
        let rf = r as f64;
        sum_lr += lf * rf;
        sum_l2 += lf * lf;
        sum_r2 += rf * rf;
    }
    let denom = libm::sqrt(sum_l2 * sum_r2);
    if denom <= 1e-12 {
        0.0
    } else {
        (sum_lr / denom).clamp(-1.0, 1.0) as f32
    }
}

/// Renders a 45-degree rotated Stereo Phase Goniometer / Lissajous vectorscope directly to [`FrameBuffer`].
///
/// Mapping:
/// - Vertical axis (`+Y` up): `(L + R) / sqrt(2)` (Mono in-phase)
/// - Horizontal axis (`+X` right): `(R - L) / sqrt(2)` (Stereo side / anti-phase)
/// - `-45 deg` diagonal = Left channel only; `+45 deg` diagonal = Right channel only.
pub fn render_goniometer(
    stereo_samples: &[[f32; 2]],
    cfg: &GoniometerConfig,
    prev_frame: Option<&FrameBuffer>,
) -> Result<FrameBuffer, CompositorError> {
    let mut fb = if let Some(prev) = prev_frame {
        if prev.width == cfg.width && prev.height == cfg.height {
            let mut decayed = prev.clone();
            let d = clamp01(cfg.persistence_decay);
            for (dst, &bg) in decayed.pixels.iter_mut().zip(core::iter::repeat(&cfg.bg_color)) {
                dst[0] = lerp(bg[0], dst[0], d);
                dst[1] = lerp(bg[1], dst[1], d);
                dst[2] = lerp(bg[2], dst[2], d);
            }
            decayed
        } else {
            FrameBuffer::filled(cfg.width, cfg.height, cfg.bg_color)?
        }
    } else {
        FrameBuffer::filled(cfg.width, cfg.height, cfg.bg_color)?
    };

    let cx = cfg.width as f32 * 0.5;
    let cy = cfg.height as f32 * 0.5;
    let rad = (cfg.width.min(cfg.height) as f32) * 0.42;

    if cfg.show_reticle {
        // Horizontal S axis and Vertical M axis.
        draw_line(&mut fb, cx - rad, cy, cx + rad, cy, 1.0, cfg.reticle_color);
        draw_line(&mut fb, cx, cy - rad, cx, cy + rad, 1.0, cfg.reticle_color);
        // L (-45 deg) and R (+45 deg) stereo axes.
        let d = rad * FRAC_1_SQRT_2;
        draw_line(&mut fb, cx - d, cy - d, cx + d, cy + d, 1.0, cfg.reticle_color);
        draw_line(&mut fb, cx + d, cy - d, cx - d, cy + d, 1.0, cfg.reticle_color);
    }

    let project = |l: f32, r: f32| -> (f32, f32) {
        let sx = ((r - l) * FRAC_1_SQRT_2).clamp(-1.0, 1.0);
        let sy = ((l + r) * FRAC_1_SQRT_2).clamp(-1.0, 1.0);
        (cx + sx * rad, cy - sy * rad)
    };

    if stereo_samples.len() == 1 {
        let (px, py) = project(stereo_samples[0][0], stereo_samples[0][1]);
        fill_circle(&mut fb, px, py, cfg.trace_thickness_px, cfg.trace_color);
    } else if stereo_samples.len() >= 2 {
        let (mut prev_x, mut prev_y) = project(stereo_samples[0][0], stereo_samples[0][1]);
        for &[l, r] in &stereo_samples[1..] {
            let (nx, ny) = project(l, r);
            draw_line(
                &mut fb,
                prev_x,
                prev_y,
                nx,
                ny,
                cfg.trace_thickness_px,
                cfg.trace_color,
            );
            prev_x = nx;
            prev_y = ny;
        }
    }

    Ok(fb)
}
