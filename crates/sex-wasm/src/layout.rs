// SPDX-License-Identifier: GPL-3.0-only
//! Window layout plugins (slice S4 follow-up): a module that exports `memory` and
//! `layout(n: i32, w: i32, h: i32) -> i32`. The call returns a pointer into the
//! guest's memory where it wrote `n` rectangles, sixteen bytes each: `x`, `y` (`i32`),
//! `w`, `h` (`u32`), little-endian, relative to the work area's top-left corner.
//!
//! A plugin is a pure function: it imports nothing at all, so it can reach no file,
//! window, clock or network. Each call runs in a fresh instance with its own fuel, so
//! one that loops forever ends with [`LayoutError::OutOfFuel`] and the next call
//! starts clean, and a trap leaves nothing behind. The `wasm` program runs plugins in
//! a run of their own for the Shell (`sex_pdx::wasm_layout`); this module never
//! decides whether the rectangles are sensible, the Shell does.
use alloc::{format, string::String, vec::Vec};
use wasmi::{Config, Engine, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode};

/// The bytes one rectangle takes in the guest's memory.
pub const RECT_BYTES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutLimits {
    /// Fuel for one `layout` call, its instantiation included.
    pub fuel_per_call: u64,
    /// The largest linear memory.
    pub memory_bytes: usize,
    /// The most rectangles one call may be asked for.
    pub max_rects: u32,
}
impl Default for LayoutLimits {
    fn default() -> Self { Self { fuel_per_call: 2_000_000, memory_bytes: 1 << 20, max_rects: 24 } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// Not a valid module, or it imports something, or lacks `memory` or `layout`.
    Module(String),
    OutOfFuel,
    Trapped(String),
    /// A negative return, or a pointer whose rectangles leave the guest's memory.
    BadResult,
    /// Asked for no rectangles, or more than the limit.
    BadCount,
}

/// A compiled plugin, ready to be asked for layouts.
pub struct LayoutPlugin { engine: Engine, module: Module, limits: LayoutLimits }

impl LayoutPlugin {
    /// Validates and compiles `wasm`. Refuses any import: plugins get no host calls.
    pub fn new(wasm: &[u8], limits: LayoutLimits) -> Result<Self, LayoutError> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| LayoutError::Module(format!("not a valid Wasm module: {e}")))?;
        if let Some(i) = module.imports().next() {
            return Err(LayoutError::Module(format!("imports {}.{}; layout plugins get no imports", i.module(), i.name())));
        }
        let has = |name: &str| module.exports().any(|e| e.name() == name);
        if !has("memory") { return Err(LayoutError::Module("exports no `memory`".into())); }
        if !has("layout") { return Err(LayoutError::Module("exports no `layout` function".into())); }
        Ok(Self { engine, module, limits })
    }

    /// `n` rectangles for a work area `w` by `h`, from a fresh instance.
    pub fn layout(&self, n: u32, w: u32, h: u32) -> Result<Vec<[i64; 4]>, LayoutError> {
        if n == 0 || n > self.limits.max_rects { return Err(LayoutError::BadCount); }
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .memories(1)
            .tables(1)
            .table_elements(1024)
            .instances(1)
            .build();
        let mut store: Store<StoreLimits> = Store::new(&self.engine, store_limits);
        store.limiter(|s| s);
        store.set_fuel(self.limits.fuel_per_call).map_err(|_| LayoutError::OutOfFuel)?;
        let linker = wasmi::Linker::<StoreLimits>::new(&self.engine);
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(trap)?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| LayoutError::Module("`memory` is not a memory".into()))?;
        let func = instance
            .get_typed_func::<(i32, i32, i32), i32>(&store, "layout")
            .map_err(|_| LayoutError::Module("`layout` is not (i32, i32, i32) -> i32".into()))?;
        let clamp = |v: u32| v.min(i32::MAX as u32) as i32;
        let ptr = func.call(&mut store, (n as i32, clamp(w), clamp(h))).map_err(trap)?;
        if ptr < 0 { return Err(LayoutError::BadResult); }
        let data = memory.data(&store);
        let start = ptr as usize;
        let end = start.checked_add(n as usize * RECT_BYTES).ok_or(LayoutError::BadResult)?;
        let bytes = data.get(start..end).ok_or(LayoutError::BadResult)?;
        let word = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        Ok((0..n as usize)
            .map(|i| {
                let at = i * RECT_BYTES;
                [word(at) as i32 as i64, word(at + 4) as i32 as i64, word(at + 8) as i64, word(at + 12) as i64]
            })
            .collect())
    }
}

fn trap(e: wasmi::Error) -> LayoutError {
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => LayoutError::OutOfFuel,
        Some(code) => LayoutError::Trapped(format!("{code}")),
        None => LayoutError::Trapped(format!("{e}")),
    }
}
