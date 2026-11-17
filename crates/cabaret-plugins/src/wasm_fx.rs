//! Safe adapter around [`sex_wasm::frame_fx::FrameFxHost`] converting between
//! [`cabaret_core::compositor::FrameBuffer`] and 8-bit RGBA bytes while running
//! fuel-metered `.wasm` video frame-effect plugins (`DEFAULT_FRAME_FUEL`).

use alloc::vec;
use alloc::vec::Vec;

use cabaret_core::compositor::{CompositorError, FrameBuffer};
pub use sex_color::{ColorPrimaries, LinearFrame, RgbaF32};
pub use sex_wasm::frame_fx::{
    FrameFxHost, FxCallReport, FxError, FxLimits, DEFAULT_FRAME_FUEL, DEFAULT_MAX_MEMORY_BYTES,
};

use crate::util::{clamp01, roundf};

/// Color transfer used when marshaling [`FrameBuffer`] FP32 pixels <-> 8-bit RGBA guest buffers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rgba8Transfer {
    /// Standard IEC 61966-2-1 sRGB encoded RGBA8 (matches `FrameBuffer::to_srgb_rgba8` / `from_srgb_rgba8`).
    #[default]
    Srgb,
    /// Direct linear-light `[0.0..=1.0] <-> [0..=255]` quantization without gamma curve.
    Linear,
}

/// Errors emitted by [`WasmFxBridge`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WasmFxError {
    Wasm(FxError),
    Compositor(CompositorError),
}

impl From<FxError> for WasmFxError {
    fn from(e: FxError) -> Self {
        Self::Wasm(e)
    }
}

impl From<CompositorError> for WasmFxError {
    fn from(e: CompositorError) -> Self {
        Self::Compositor(e)
    }
}

/// Processed output frame + deterministic execution report from a `.wasm` video filter.
#[derive(Clone, Debug, PartialEq)]
pub struct WasmFxOutput {
    pub frame: FrameBuffer,
    pub report: FxCallReport,
}

/// Converts a `cabaret_core::compositor::FrameBuffer` into a `sex_color::LinearFrame`.
pub fn framebuffer_to_linear_frame(frame: &FrameBuffer, primaries: ColorPrimaries) -> LinearFrame {
    LinearFrame {
        width: frame.width as usize,
        height: frame.height as usize,
        primaries,
        pixels: frame.pixels.iter().map(|&p| RgbaF32(p)).collect(),
    }
}

/// Converts a `sex_color::LinearFrame` into a `cabaret_core::compositor::FrameBuffer`.
pub fn linear_frame_to_framebuffer(lf: &LinearFrame) -> Result<FrameBuffer, WasmFxError> {
    let pixels = lf.pixels.iter().map(|p| p.0).collect();
    Ok(FrameBuffer::from_pixels(
        lf.width as u32,
        lf.height as u32,
        pixels,
    )?)
}

/// Converts a linear-light [`FrameBuffer`] to packed 8-bit RGBA bytes under `transfer`.
pub fn framebuffer_to_rgba8(frame: &FrameBuffer, transfer: Rgba8Transfer) -> Vec<u8> {
    match transfer {
        Rgba8Transfer::Srgb => frame.to_srgb_rgba8(),
        Rgba8Transfer::Linear => {
            let mut out = Vec::with_capacity(frame.pixels.len() * 4);
            for &p in &frame.pixels {
                out.push(roundf(clamp01(p[0]) * 255.0) as u8);
                out.push(roundf(clamp01(p[1]) * 255.0) as u8);
                out.push(roundf(clamp01(p[2]) * 255.0) as u8);
                out.push(roundf(clamp01(p[3]) * 255.0) as u8);
            }
            out
        }
    }
}

/// Decodes packed 8-bit RGBA bytes into a linear-light [`FrameBuffer`] under `transfer`.
pub fn rgba8_to_framebuffer(
    width: u32,
    height: u32,
    bytes: &[u8],
    transfer: Rgba8Transfer,
) -> Result<FrameBuffer, WasmFxError> {
    match transfer {
        Rgba8Transfer::Srgb => Ok(FrameBuffer::from_srgb_rgba8(width, height, bytes)?),
        Rgba8Transfer::Linear => {
            if width == 0 || height == 0 {
                return Err(WasmFxError::Compositor(CompositorError::ZeroDimensions));
            }
            let count = (width as usize)
                .checked_mul(height as usize)
                .ok_or(WasmFxError::Compositor(
                    CompositorError::DimensionsExceedLimit { width, height },
                ))?;
            let expected = count * 4;
            if bytes.len() != expected {
                return Err(WasmFxError::Compositor(
                    CompositorError::ByteLengthMismatch {
                        expected,
                        actual: bytes.len(),
                    },
                ));
            }
            let mut pixels = Vec::with_capacity(count);
            for chunk in bytes.chunks_exact(4) {
                pixels.push([
                    chunk[0] as f32 / 255.0,
                    chunk[1] as f32 / 255.0,
                    chunk[2] as f32 / 255.0,
                    chunk[3] as f32 / 255.0,
                ]);
            }
            Ok(FrameBuffer::from_pixels(width, height, pixels)?)
        }
    }
}

/// Safe adapter around [`sex_wasm::frame_fx::FrameFxHost`] bridging [`FrameBuffer`] video
/// frames into deterministic, fuel-metered `.wasm` video filters.
pub struct WasmFxBridge {
    host: FrameFxHost,
    pub transfer: Rgba8Transfer,
}

impl Default for WasmFxBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmFxBridge {
    /// Creates a bridge initialized with [`DEFAULT_FRAME_FUEL`] and [`DEFAULT_MAX_MEMORY_BYTES`].
    pub fn new() -> Self {
        Self::with_limits(FxLimits {
            fuel_per_frame: DEFAULT_FRAME_FUEL,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        })
    }

    pub fn with_limits(limits: FxLimits) -> Self {
        Self {
            host: FrameFxHost::new(limits),
            transfer: Rgba8Transfer::Srgb,
        }
    }

    pub fn with_transfer(mut self, transfer: Rgba8Transfer) -> Self {
        self.transfer = transfer;
        self
    }

    pub fn limits(&self) -> &FxLimits {
        self.host.limits()
    }

    pub fn set_limits(&mut self, limits: FxLimits) {
        self.host.set_limits(limits);
    }

    pub fn host(&self) -> &FrameFxHost {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut FrameFxHost {
        &mut self.host
    }

    /// Compiles and caches a `.wasm` video frame filter under `(plugin_id, version_tag)`.
    pub fn register_wasm(
        &mut self,
        plugin_id: &str,
        version_tag: u64,
        wasm_bytes: &[u8],
    ) -> Result<(), WasmFxError> {
        self.host
            .register_wasm(plugin_id, version_tag, wasm_bytes)?;
        Ok(())
    }

    pub fn is_cached(&self, plugin_id: &str, version_tag: u64) -> bool {
        self.host.is_cached(plugin_id, version_tag)
    }

    /// Runs a registered `.wasm` video filter against `input` using the bridge's configured transfer.
    pub fn process_frame(
        &mut self,
        plugin_id: &str,
        input: &FrameBuffer,
        params_f32: &[f32],
        timestamp_us: i64,
    ) -> Result<WasmFxOutput, WasmFxError> {
        self.process_frame_with_transfer(plugin_id, input, params_f32, timestamp_us, self.transfer)
    }

    /// Runs a registered `.wasm` video filter against `input` with an explicit [`Rgba8Transfer`].
    pub fn process_frame_with_transfer(
        &mut self,
        plugin_id: &str,
        input: &FrameBuffer,
        params_f32: &[f32],
        timestamp_us: i64,
        transfer: Rgba8Transfer,
    ) -> Result<WasmFxOutput, WasmFxError> {
        let in_bytes = framebuffer_to_rgba8(input, transfer);
        let mut out_bytes = vec![0u8; in_bytes.len()];
        let report = self.host.process_frame(
            plugin_id,
            input.width,
            input.height,
            &in_bytes,
            &mut out_bytes,
            params_f32,
            timestamp_us,
        )?;
        let frame = rgba8_to_framebuffer(input.width, input.height, &out_bytes, transfer)?;
        Ok(WasmFxOutput { frame, report })
    }

    /// Convenience in-place wrapper replacing `frame` on success.
    pub fn process_frame_in_place(
        &mut self,
        plugin_id: &str,
        frame: &mut FrameBuffer,
        params_f32: &[f32],
        timestamp_us: i64,
    ) -> Result<FxCallReport, WasmFxError> {
        let out = self.process_frame(plugin_id, frame, params_f32, timestamp_us)?;
        *frame = out.frame;
        Ok(out.report)
    }
}
