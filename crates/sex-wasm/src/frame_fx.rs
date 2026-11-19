// SPDX-License-Identifier: GPL-3.0-only
//! Deterministic, fuel-metered, zero-ambient-authority WASM video frame-effect plugin host.
//!
//! Adapted from Rook's `crates/rook-plugin-host/src/wasm_host.rs` (GPL-3.0) for SexOS's
//! zero-ambient-authority `wasmi 2.0` sandbox.
//!
//! Each guest module exports:
//! - `memory`: linear memory. For each call the host adds pages after the guest's own
//!   memory and puts the input frame, output frame and parameters there, so they never
//!   overlap the guest's data, stack or heap. A guest takes more memory only through
//!   `memory.grow` (as every wasm allocator does), never by treating `memory.size` as its own.
//! - `process_frame(in_ptr: i32, out_ptr: i32, width: i32, height: i32, params_ptr: i32, params_len: i32) -> i32`
//!
//! The host exposes strictly two pure imports under `"env"`:
//! - `env::log(ptr: i32, len: i32)`: writes bounded UTF-8 logs into [`FxCallReport::log`].
//! - `env::get_time_us() -> i64`: returns the deterministic `timestamp_us` for bit-reproducible renders.

use alloc::{
    collections::BTreeMap,
    string::String,
    vec::Vec,
};
use wasmi::{
    Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode,
};

/// Default fuel budget per frame call (~10 ms CPU budget on modern hardware, matching Rook).
pub const DEFAULT_FRAME_FUEL: u64 = 50_000_000;

/// Default maximum linear memory limit (64 MiB cap on SexOS; configurable up to 256 MiB).
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Maximum allowed log bytes collected per frame call.
pub const MAX_LOG_BYTES: usize = 4096;

/// Resource limits for video frame effect execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxLimits {
    /// Maximum fuel consumed per `process_frame` invocation.
    pub fuel_per_frame: u64,
    /// Maximum linear memory bytes allowed for guest instance.
    pub max_memory_bytes: usize,
}

impl Default for FxLimits {
    fn default() -> Self {
        Self {
            fuel_per_frame: DEFAULT_FRAME_FUEL,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        }
    }
}

/// Errors that can occur during video frame effect execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FxError {
    /// WebAssembly module is invalid or failed validation/compilation.
    InvalidModule,
    /// Guest module lacks the required `memory` export.
    MissingMemory,
    /// Guest module lacks the required `process_frame` export.
    MissingProcessFrame,
    /// Frame dimensions or scratch buffer layout exceed allowed memory limits or buffer sizes.
    FrameTooLarge,
    /// Out-of-bounds linear memory access or allocation error.
    MemoryOutOfBounds,
    /// Instruction fuel budget exhausted during frame execution.
    FuelExhausted,
    /// Guest execution encountered a WebAssembly trap.
    Trapped,
    /// Guest rejected the frame call by returning a non-zero exit code.
    GuestRejected(i32),
}

impl core::fmt::Display for FxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidModule => write!(f, "invalid WASM module"),
            Self::MissingMemory => write!(f, "module missing 'memory' export"),
            Self::MissingProcessFrame => write!(f, "module missing 'process_frame' export"),
            Self::FrameTooLarge => write!(f, "frame dimensions or scratch buffers exceed memory limits"),
            Self::MemoryOutOfBounds => write!(f, "out of bounds memory access or allocation failure"),
            Self::FuelExhausted => write!(f, "execution fuel budget exhausted"),
            Self::Trapped => write!(f, "WASM execution trapped"),
            Self::GuestRejected(code) => write!(f, "guest rejected frame with code {code}"),
        }
    }
}

/// Report returned on successful frame processing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FxCallReport {
    /// Fuel units consumed by the guest during this frame call.
    pub fuel_consumed: u64,
    /// Text logged by the guest via `env::log`.
    pub log: String,
}

/// Host state attached to the wasmi `Store`.
struct HostState {
    timestamp_us: i64,
    log_buf: String,
    store_limits: StoreLimits,
}

/// Fuel-metered, zero-authority host for WASM video frame-effect plugins.
pub struct FrameFxHost {
    engine: Engine,
    limits: FxLimits,
    modules: BTreeMap<(String, u64), Module>,
}

impl Default for FrameFxHost {
    fn default() -> Self {
        Self::new(FxLimits::default())
    }
}

impl FrameFxHost {
    /// Create a new host with configured resource limits and fuel metering enabled.
    pub fn new(limits: FxLimits) -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        Self {
            engine,
            limits,
            modules: BTreeMap::new(),
        }
    }

    /// Current resource limits.
    pub fn limits(&self) -> &FxLimits {
        &self.limits
    }

    /// Update resource limits.
    pub fn set_limits(&mut self, limits: FxLimits) {
        self.limits = limits;
    }

    /// Compiles and caches a plugin module under `(plugin_id, version_tag)`.
    ///
    /// Any older `version_tag`s for this `plugin_id` are evicted from the cache.
    pub fn register_wasm(&mut self, plugin_id: &str, version_tag: u64, wasm_bytes: &[u8]) -> Result<(), FxError> {
        let module = Module::new(&self.engine, wasm_bytes).map_err(|_| FxError::InvalidModule)?;

        // Evict older versions for this plugin_id
        let to_remove: Vec<(String, u64)> = self
            .modules
            .keys()
            .filter(|(id, ver)| id == plugin_id && *ver != version_tag)
            .cloned()
            .collect();
        for k in to_remove {
            self.modules.remove(&k);
        }

        self.modules.insert((String::from(plugin_id), version_tag), module);
        Ok(())
    }

    /// Returns `true` if the exact `(plugin_id, version_tag)` pair is present in the cache.
    pub fn is_cached(&self, plugin_id: &str, version_tag: u64) -> bool {
        self.modules
            .iter()
            .any(|((id, ver), _)| id == plugin_id && *ver == version_tag)
    }

    /// Process a single RGBA/BGRA video frame through a registered plugin.
    ///
    /// - `width`, `height`: frame dimensions in pixels.
    /// - `frame_in`, `frame_out`: must both have length `width * height * 4`.
    /// - `params_f32`: parameter array passed to guest at `params_ptr`.
    /// - `timestamp_us`: deterministic frame timestamp in microseconds passed to `env::get_time_us`.
    pub fn process_frame(
        &mut self,
        plugin_id: &str,
        width: u32,
        height: u32,
        frame_in: &[u8],
        frame_out: &mut [u8],
        params_f32: &[f32],
        timestamp_us: i64,
    ) -> Result<FxCallReport, FxError> {
        // Find latest registered module for plugin_id
        let module = self
            .modules
            .iter()
            .rev()
            .find(|((id, _), _)| id == plugin_id)
            .map(|(_, m)| m.clone())
            .ok_or(FxError::InvalidModule)?;

        // Calculate frame size
        let frame_size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or(FxError::FrameTooLarge)?;

        if frame_in.len() != frame_size || frame_out.len() != frame_size {
            return Err(FxError::FrameTooLarge);
        }
        if width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(FxError::FrameTooLarge);
        }

        let params_len = params_f32.len();
        if params_len > (i32::MAX as usize / 4) {
            return Err(FxError::FrameTooLarge);
        }
        let params_bytes_len = params_len * 4;

        // Set up Store with StoreLimits and fuel
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.max_memory_bytes)
            .memories(1)
            .tables(4)
            .table_elements(100_000)
            .instances(1)
            .build();

        let state = HostState {
            timestamp_us,
            log_buf: String::new(),
            store_limits,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.store_limits);
        store
            .set_fuel(self.limits.fuel_per_frame)
            .map_err(|_| FxError::FuelExhausted)?;

        // Link ONLY the two pure zero-ambient-authority imports: env::log and env::get_time_us
        let mut linker = Linker::<HostState>::new(&self.engine);

        linker
            .func_wrap("env", "log", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                if ptr < 0 || len <= 0 {
                    return;
                }
                let (ptr, len) = (ptr as usize, len as usize);
                // Cap extraction length to MAX_LOG_BYTES to avoid large allocations
                let take_len = len.min(MAX_LOG_BYTES);
                let raw_bytes: Option<Vec<u8>> = (|| {
                    let memory = caller.get_export("memory")?.into_memory()?;
                    let data = memory.data(&caller);
                    let end = ptr.checked_add(take_len)?;
                    if ptr >= data.len() {
                        return None;
                    }
                    Some(data[ptr..end.min(data.len())].to_vec())
                })();
                let Some(bytes) = raw_bytes else {
                    return;
                };
                let valid_str = match core::str::from_utf8(&bytes) {
                    Ok(s) => s,
                    Err(e) if e.valid_up_to() > 0 => {
                        // SAFETY: from_utf8 verified validity up to e.valid_up_to()
                        unsafe { core::str::from_utf8_unchecked(&bytes[..e.valid_up_to()]) }
                    }
                    _ => return,
                };
                let current_len = caller.data().log_buf.len();
                if current_len < MAX_LOG_BYTES {
                    let available = MAX_LOG_BYTES - current_len;
                    let take = take_utf8_prefix(valid_str, available);
                    caller.data_mut().log_buf.push_str(take);
                }
            })
            .map_err(|_| FxError::InvalidModule)?;

        linker
            .func_wrap("env", "get_time_us", |caller: Caller<'_, HostState>| -> i64 {
                caller.data().timestamp_us
            })
            .map_err(|_| FxError::InvalidModule)?;

        // Instantiate guest
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| match e.as_trap_code() {
                Some(TrapCode::OutOfFuel) => FxError::FuelExhausted,
                Some(TrapCode::MemoryOutOfBounds) => FxError::MemoryOutOfBounds,
                Some(_) => FxError::Trapped,
                None => FxError::InvalidModule,
            })?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or(FxError::MissingMemory)?;

        let process_frame_func = instance
            .get_typed_func::<(i32, i32, i32, i32, i32, i32), i32>(&store, "process_frame")
            .map_err(|_| FxError::MissingProcessFrame)?;

        // Guest buffer placement: frames and parameters go in pages the host adds after
        // the guest's own memory, never inside it. A guest's allocator (dlmalloc, talc,
        // wee_alloc) only hands out memory it obtained with `memory.grow` itself, so its
        // heap, stack and data never overlap these buffers, and anything the guest
        // allocates during the call lands in pages after them.
        let current_bytes = memory.data(&store).len();
        let in_ptr = current_bytes;
        let out_ptr = in_ptr.checked_add(frame_size).ok_or(FxError::FrameTooLarge)?;
        let params_ptr = out_ptr.checked_add(frame_size).ok_or(FxError::FrameTooLarge)?;
        let total_needed = params_ptr.checked_add(params_bytes_len).ok_or(FxError::FrameTooLarge)?;

        if total_needed > i32::MAX as usize || total_needed > self.limits.max_memory_bytes {
            return Err(FxError::FrameTooLarge);
        }

        let pages_needed = (total_needed - current_bytes).div_ceil(65536).max(1);
        memory
            .grow(&mut store, pages_needed as u64)
            .map_err(|_| FxError::MemoryOutOfBounds)?;

        // Write input frame and params into linear memory
        {
            let mem_data = memory.data_mut(&mut store);
            if in_ptr + frame_size > mem_data.len() || params_ptr + params_bytes_len > mem_data.len() {
                return Err(FxError::MemoryOutOfBounds);
            }
            mem_data[in_ptr..in_ptr + frame_size].copy_from_slice(frame_in);
            for (i, &param) in params_f32.iter().enumerate() {
                let off = params_ptr + i * 4;
                mem_data[off..off + 4].copy_from_slice(&param.to_le_bytes());
            }
        }

        // Execute process_frame
        let status = process_frame_func
            .call(
                &mut store,
                (
                    in_ptr as i32,
                    out_ptr as i32,
                    width as i32,
                    height as i32,
                    params_ptr as i32,
                    params_len as i32,
                ),
            )
            .map_err(|e| match e.as_trap_code() {
                Some(TrapCode::OutOfFuel) => FxError::FuelExhausted,
                Some(TrapCode::MemoryOutOfBounds) => FxError::MemoryOutOfBounds,
                Some(_) => FxError::Trapped,
                None => FxError::Trapped,
            })?;

        if status != 0 {
            return Err(FxError::GuestRejected(status));
        }

        // Copy output frame back
        {
            let mem_data = memory.data(&store);
            if out_ptr + frame_size > mem_data.len() {
                return Err(FxError::MemoryOutOfBounds);
            }
            frame_out.copy_from_slice(&mem_data[out_ptr..out_ptr + frame_size]);
        }

        let remaining_fuel = store.get_fuel().unwrap_or(0);
        let fuel_consumed = self.limits.fuel_per_frame.saturating_sub(remaining_fuel);
        let log = store.data().log_buf.clone();

        Ok(FxCallReport {
            fuel_consumed,
            log,
        })
    }

    /// Convenience wrapper for 32-bit pixel buffers (e.g. RGBA8888 or BGRA8888).
    pub fn process_frame_u32(
        &mut self,
        plugin_id: &str,
        width: u32,
        height: u32,
        pixels_in: &[u32],
        pixels_out: &mut [u32],
        params_f32: &[f32],
        timestamp_us: i64,
    ) -> Result<FxCallReport, FxError> {
        let in_bytes_len = pixels_in.len().checked_mul(4).ok_or(FxError::FrameTooLarge)?;
        let out_bytes_len = pixels_out.len().checked_mul(4).ok_or(FxError::FrameTooLarge)?;

        // SAFETY: u32 has 4-byte alignment (greater than u8's 1-byte alignment).
        // Any u32 bit pattern is valid u8, and slice lengths in bytes match accurately.
        let bytes_in = unsafe { core::slice::from_raw_parts(pixels_in.as_ptr() as *const u8, in_bytes_len) };
        let bytes_out = unsafe { core::slice::from_raw_parts_mut(pixels_out.as_mut_ptr() as *mut u8, out_bytes_len) };

        self.process_frame(
            plugin_id,
            width,
            height,
            bytes_in,
            bytes_out,
            params_f32,
            timestamp_us,
        )
    }
}

/// Takes the longest UTF-8 prefix of `s` that does not exceed `max_bytes` without slicing chars.
fn take_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_1_rgba_channel_invert_and_brightness_boost() {
        // Plugin that:
        // - Calls env::get_time_us
        // - Logs via env::log
        // - Reads params_f32 (boost offset)
        // - Inverts color and adds boost: out[i] = (255 - in[i]) + boost
        let wat = r#"(module
            (import "env" "log" (func $log (param i32 i32)))
            (import "env" "get_time_us" (func $get_time_us (result i64)))
            (memory (export "memory") 1)
            (data (i32.const 128) "filter applied successfully")
            (func (export "process_frame")
                (param $in_ptr i32)
                (param $out_ptr i32)
                (param $width i32)
                (param $height i32)
                (param $params_ptr i32)
                (param $params_len i32)
                (result i32)
                (local $i i32)
                (local $total i32)
                (local $boost i32)
                (local $val i32)

                ;; Read timestamp and verify deterministic matching
                (if (i64.ne (call $get_time_us) (i64.const 1234567890))
                    (then (return (i32.const 99)))
                )

                ;; Emit log message
                (call $log (i32.const 128) (i32.const 27))

                ;; Read boost param (f32 -> i32)
                (if (i32.gt_s (local.get $params_len) (i32.const 0))
                    (then
                        (local.set $boost (i32.trunc_f32_s (f32.load (local.get $params_ptr))))
                    )
                )

                ;; total = width * height * 4
                (local.set $total (i32.mul (i32.mul (local.get $width) (local.get $height)) (i32.const 4)))

                (local.set $i (i32.const 0))
                (loop $loop
                    (if (i32.lt_s (local.get $i) (local.get $total))
                        (then
                            (local.set $val
                                (i32.sub (i32.const 255) (i32.load8_u (i32.add (local.get $in_ptr) (local.get $i))))
                            )
                            (local.set $val (i32.add (local.get $val) (local.get $boost)))
                            (i32.store8
                                (i32.add (local.get $out_ptr) (local.get $i))
                                (local.get $val)
                            )
                            (local.set $i (i32.add (local.get $i) (i32.const 1)))
                            (br $loop)
                        )
                    )
                )
                (i32.const 0)
            )
        )"#;

        let wasm = wat::parse_str(wat).expect("valid wat");
        let mut host = FrameFxHost::new(FxLimits::default());
        host.register_wasm("invert_boost", 1, &wasm).expect("registered");

        let width = 2u32;
        let height = 2u32;
        // 4 pixels (16 bytes)
        let frame_in: [u8; 16] = [
            10, 20, 30, 255,
            50, 60, 70, 255,
            100, 110, 120, 255,
            200, 210, 220, 255,
        ];
        let mut frame_out = [0u8; 16];
        let params = [5.0f32]; // boost = 5
        let timestamp_us = 1_234_567_890i64;

        let report = host
            .process_frame("invert_boost", width, height, &frame_in, &mut frame_out, &params, timestamp_us)
            .expect("processed");

        assert!(report.fuel_consumed > 0, "fuel consumed should be positive: {}", report.fuel_consumed);
        assert_eq!(report.log, "filter applied successfully");

        // Expected: (255 - in[i]) + 5
        for i in 0..16 {
            let expected = (255 - frame_in[i]) + 5;
            assert_eq!(frame_out[i], expected, "pixel byte mismatch at index {i}");
        }

        // Test convenience u32 wrapper
        let pixels_in = [0x11223344u32, 0x55667788u32];
        let mut pixels_out = [0u32; 2];
        let report_u32 = host
            .process_frame_u32("invert_boost", 2, 1, &pixels_in, &mut pixels_out, &params, timestamp_us)
            .expect("processed u32");
        assert!(report_u32.fuel_consumed > 0);
    }

    #[test]
    fn test_2_runaway_infinite_loop_terminates_with_fuel_exhausted() {
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "process_frame")
                (param i32 i32 i32 i32 i32 i32)
                (result i32)
                (loop (br 0))
                (i32.const 0)
            )
        )"#;

        let wasm = wat::parse_str(wat).expect("valid wat");
        let mut host = FrameFxHost::new(FxLimits {
            fuel_per_frame: 50_000, // Small fuel budget to speed up test
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        });
        host.register_wasm("infinite_loop", 1, &wasm).expect("registered");

        let frame_in = [0u8; 4];
        let mut frame_out = [0u8; 4];
        let res = host.process_frame("infinite_loop", 1, 1, &frame_in, &mut frame_out, &[], 0);

        assert_eq!(res, Err(FxError::FuelExhausted));
    }

    #[test]
    fn test_3_memory_bomb_fails_cleanly_without_crashing() {
        // Plugin attempting to grow memory past max_memory_bytes and write to it
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "process_frame")
                (param i32 i32 i32 i32 i32 i32)
                (result i32)
                ;; Attempt to grow by 2000 pages (125 MiB > 64 MiB limit)
                (local.set 0 (memory.grow (i32.const 2000)))
                ;; memory.grow returns -1 when limiter rejects it
                (if (i32.eq (local.get 0) (i32.const -1))
                    (then
                        ;; Writing to out-of-bounds page triggers MemoryOutOfBounds trap
                        (i32.store (i32.const 70000000) (i32.const 42))
                    )
                )
                (i32.const 0)
            )
        )"#;

        let wasm = wat::parse_str(wat).expect("valid wat");
        let mut host = FrameFxHost::new(FxLimits::default());
        host.register_wasm("memory_bomb", 1, &wasm).expect("registered");

        let frame_in = [0u8; 4];
        let mut frame_out = [0u8; 4];
        let res = host.process_frame("memory_bomb", 1, 1, &frame_in, &mut frame_out, &[], 0);

        assert_eq!(res, Err(FxError::MemoryOutOfBounds));

        // Also test host-side frame size exceeding max_memory_bytes
        let mut small_host = FrameFxHost::new(FxLimits {
            fuel_per_frame: DEFAULT_FRAME_FUEL,
            max_memory_bytes: 65536, // 1 page only
        });
        small_host.register_wasm("memory_bomb", 1, &wasm).expect("registered");
        // A 256x256 frame requires 262144 bytes for frame_in alone, exceeding 65536
        let large_in = alloc::vec![0u8; 256 * 256 * 4];
        let mut large_out = alloc::vec![0u8; 256 * 256 * 4];
        let res_large = small_host.process_frame("memory_bomb", 256, 256, &large_in, &mut large_out, &[], 0);
        assert_eq!(res_large, Err(FxError::FrameTooLarge));
    }

    #[test]
    fn test_4_version_tag_hot_reload() {
        let wat_v1 = r#"(module
            (memory (export "memory") 1)
            (func (export "process_frame")
                (param $in i32) (param $out i32) (param $w i32) (param $h i32) (param $p i32) (param $plen i32)
                (result i32)
                (i32.store8 (local.get $out) (i32.const 11))
                (i32.const 0)
            )
        )"#;

        let wat_v2 = r#"(module
            (memory (export "memory") 1)
            (func (export "process_frame")
                (param $in i32) (param $out i32) (param $w i32) (param $h i32) (param $p i32) (param $plen i32)
                (result i32)
                (i32.store8 (local.get $out) (i32.const 22))
                (i32.const 0)
            )
        )"#;

        let wasm_v1 = wat::parse_str(wat_v1).expect("wat v1");
        let wasm_v2 = wat::parse_str(wat_v2).expect("wat v2");

        let mut host = FrameFxHost::new(FxLimits::default());
        let plugin_id = "hot_reload_plugin";

        host.register_wasm(plugin_id, 10, &wasm_v1).expect("registered v1");
        assert!(host.is_cached(plugin_id, 10));
        assert!(!host.is_cached(plugin_id, 20));

        let frame_in = [0u8; 4];
        let mut frame_out = [0u8; 4];
        host.process_frame(plugin_id, 1, 1, &frame_in, &mut frame_out, &[], 0).expect("v1 run");
        assert_eq!(frame_out[0], 11);

        // Hot reload with new version tag
        host.register_wasm(plugin_id, 20, &wasm_v2).expect("registered v2");
        assert!(!host.is_cached(plugin_id, 10), "old version tag must be evicted");
        assert!(host.is_cached(plugin_id, 20), "new version tag must be cached");

        host.process_frame(plugin_id, 1, 1, &frame_in, &mut frame_out, &[], 0).expect("v2 run");
        assert_eq!(frame_out[0], 22, "v2 code must have executed");
    }

    #[test]
    fn test_5_guest_returning_nonzero_code_yields_guest_rejected() {
        let wat = r#"(module
            (memory (export "memory") 1)
            (func (export "process_frame")
                (param i32 i32 i32 i32 i32 i32)
                (result i32)
                (i32.const 42)
            )
        )"#;

        let wasm = wat::parse_str(wat).expect("valid wat");
        let mut host = FrameFxHost::new(FxLimits::default());
        host.register_wasm("rejecting_plugin", 1, &wasm).expect("registered");

        let frame_in = [0u8; 4];
        let mut frame_out = [0u8; 4];
        let res = host.process_frame("rejecting_plugin", 1, 1, &frame_in, &mut frame_out, &[], 0);

        assert_eq!(res, Err(FxError::GuestRejected(42)));
    }

    #[test]
    fn buffers_sit_after_the_guests_own_memory() {
        // A guest whose heap starts at __heap_base (8192) writes all over its heap during
        // the call, as an allocating plugin would. The frame it was given must be intact,
        // so it can still copy it out, and the buffers must start after its one page.
        let wat = r#"(module
            (global (export "__heap_base") i32 (i32.const 8192))
            (memory (export "memory") 1)
            (func (export "process_frame")
                (param $in i32) (param $out i32) (param $w i32) (param $h i32) (param $p i32) (param $plen i32)
                (result i32)
                (if (i32.lt_u (local.get $in) (i32.const 65536)) (then (return (i32.const 101))))
                (if (i32.ne (local.get $out) (i32.add (local.get $in) (i32.const 4))) (then (return (i32.const 102))))
                ;; Scribble over the guest's whole heap.
                (memory.fill (i32.const 8192) (i32.const 0xAA) (i32.const 57344))
                ;; Memory the guest grows for itself comes after the host's buffers.
                (if (i32.lt_u (i32.mul (memory.grow (i32.const 1)) (i32.const 65536))
                              (i32.add (local.get $p) (i32.mul (local.get $plen) (i32.const 4))))
                    (then (return (i32.const 103))))
                (i32.store (local.get $out) (i32.load (local.get $in)))
                (i32.const 0)
            )
        )"#;

        let wasm = wat::parse_str(wat).expect("valid wat");
        let mut host = FrameFxHost::new(FxLimits::default());
        host.register_wasm("placement", 1, &wasm).expect("registered");

        let frame_in = [1, 2, 3, 4];
        let mut frame_out = [0u8; 4];
        let res = host.process_frame("placement", 1, 1, &frame_in, &mut frame_out, &[0.5], 0);
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(frame_out, [1, 2, 3, 4]);
    }

    #[test]
    fn test_missing_memory_and_missing_process_frame() {
        let mut host = FrameFxHost::new(FxLimits::default());

        // Missing memory
        let wat_no_mem = r#"(module
            (func (export "process_frame") (param i32 i32 i32 i32 i32 i32) (result i32) (i32.const 0))
        )"#;
        let wasm_no_mem = wat::parse_str(wat_no_mem).expect("valid wat");
        host.register_wasm("no_mem", 1, &wasm_no_mem).expect("registered");
        let res = host.process_frame("no_mem", 1, 1, &[0; 4], &mut [0; 4], &[], 0);
        assert_eq!(res, Err(FxError::MissingMemory));

        // Missing process_frame
        let wat_no_func = r#"(module (memory (export "memory") 1))"#;
        let wasm_no_func = wat::parse_str(wat_no_func).expect("valid wat");
        host.register_wasm("no_func", 1, &wasm_no_func).expect("registered");
        let res2 = host.process_frame("no_func", 1, 1, &[0; 4], &mut [0; 4], &[], 0);
        assert_eq!(res2, Err(FxError::MissingProcessFrame));
    }

    #[test]
    fn test_log_bounds_and_truncation() {
        // Module attempting to log 10,000 bytes; host bounds log to MAX_LOG_BYTES (4096)
        let wat = r#"(module
            (import "env" "log" (func $log (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "A")
            (func (export "process_frame")
                (param i32 i32 i32 i32 i32 i32)
                (result i32)
                (local $i i32)
                ;; Call log 5000 times with "A"
                (local.set $i (i32.const 0))
                (loop $loop
                    (if (i32.lt_s (local.get $i) (i32.const 5000))
                        (then
                            (call $log (i32.const 100) (i32.const 1))
                            (local.set $i (i32.add (local.get $i) (i32.const 1)))
                            (br $loop)
                        )
                    )
                )
                (i32.const 0)
            )
        )"#;

        let wasm = wat::parse_str(wat).expect("valid wat");
        let mut host = FrameFxHost::new(FxLimits::default());
        host.register_wasm("flood_log", 1, &wasm).expect("registered");

        let report = host
            .process_frame("flood_log", 1, 1, &[0; 4], &mut [0; 4], &[], 0)
            .expect("processed");

        assert_eq!(report.log.len(), MAX_LOG_BYTES);
    }
}

