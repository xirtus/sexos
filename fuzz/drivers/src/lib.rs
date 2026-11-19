// SPDX-License-Identifier: GPL-3.0-only
//! Host-side fuzzing of what devices hand our drivers: register values, DMA
//! descriptors and rings, firmware files and tables, and the bytes a USB, HID,
//! Wi-Fi or ACPI device reports. Each `fuzz_targets/drv_*.rs` builds a real
//! driver source (crate or `#[path]` module) against a device model fed by the
//! fuzz input, and runs two ways:
//!
//! * `cargo fuzz run drv_x --features libfuzzer` (nightly, coverage guided);
//! * `cargo run --release --bin drv_x` (any toolchain): the built-in seeded
//!   mutator below, used by pull requests. `drv_x FILE...` replays inputs and
//!   `drv_x --seeds DIR` writes the target's seed corpus.
//!
//! A driver must never panic or spin forever on what a device says. Every device
//! model read calls [`tick`]; a run that reads more than [`READ_BUDGET`] times is a
//! hang and fails like a panic.
use std::cell::Cell;

#[cfg(feature = "libfuzzer")]
pub use libfuzzer_sys;

/// Device reads one input may cause before the driver counts as stuck. Every
/// driver wait is bounded by a poll budget or the (model) clock; budgets above
/// this are a finding of their own (a real device that never answers would hold
/// the driver for minutes).
pub const READ_BUDGET: u64 = 4_000_000;

thread_local! { static READS: Cell<u64> = const { Cell::new(0) }; }

/// Counts one device read; panics when the run looks like an endless poll.
pub fn tick() {
    READS.with(|r| {
        let n = r.get() + 1;
        r.set(n);
        if n > READ_BUDGET { panic!("DRIVER_HANG: more than {READ_BUDGET} device reads for one input"); }
    });
}
pub fn reads() -> u64 { READS.with(|r| r.get()) }
fn reset_reads() { READS.with(|r| r.set(0)); }

/// The fuzz input as a stream of device answers. Past the end it reads zeros,
/// as an absent device mostly does (the all-ones case is left to the input).
pub struct Src<'a> { data: &'a [u8], at: usize }
impl<'a> Src<'a> {
    pub fn new(data: &'a [u8]) -> Self { Src { data, at: 0 } }
    pub fn is_empty(&self) -> bool { self.at >= self.data.len() }
    pub fn u8(&mut self) -> u8 { let v = self.data.get(self.at).copied().unwrap_or(0); self.at += 1; v }
    pub fn u16(&mut self) -> u16 { u16::from_le_bytes([self.u8(), self.u8()]) }
    pub fn u32(&mut self) -> u32 { u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()]) }
    pub fn u64(&mut self) -> u64 { self.u32() as u64 | (self.u32() as u64) << 32 }
    pub fn bool(&mut self) -> bool { self.u8() & 1 != 0 }
    /// A value in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize { if n == 0 { 0 } else { self.u32() as usize % n } }
    /// Up to `n` bytes (fewer at the end of the input).
    pub fn take(&mut self, n: usize) -> &'a [u8] {
        let s = self.at.min(self.data.len());
        let e = (s + n).min(self.data.len());
        self.at = s + n;
        &self.data[s..e]
    }
    /// A length-prefixed chunk (u16 length), so one input can carry several blobs.
    pub fn chunk(&mut self) -> &'a [u8] { let n = self.u16() as usize; self.take(n) }
    pub fn rest(&mut self) -> &'a [u8] { let s = self.at.min(self.data.len()); self.at = self.data.len(); &self.data[s..] }
}

/// A register file whose every read is the next fuzz word: the most hostile
/// device possible (values change between reads, bits that "never" set do).
/// Past the end of the input the device either fell off the bus (reads all
/// ones, as a surprise-removed or powered-down PCI device does) or hung (reads
/// zeros); the input's first byte picks which, so both endless-wait shapes
/// (for a bit to set, for a bit to clear) meet a device that never answers.
pub struct FuzzRegs<'a> { pub src: Src<'a>, pub writes: u64, pub tail: u32 }
impl<'a> FuzzRegs<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let tail = if data.first().is_some_and(|b| b & 0x80 != 0) { 0 } else { 0xffff_ffff };
        Self::with_tail(data, tail)
    }
    pub fn with_tail(data: &'a [u8], tail: u32) -> Self { FuzzRegs { src: Src::new(data), writes: 0, tail } }
    pub fn read32(&mut self) -> u32 { tick(); if self.src.is_empty() { self.tail } else { self.src.u32() } }
    pub fn write32(&mut self) { self.writes += 1; }
}

/// Little xorshift for the built-in mutator (deterministic per seed).
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1) }
    pub fn next(&mut self) -> u64 { let mut x = self.0; x ^= x << 13; x ^= x >> 7; x ^= x << 17; self.0 = x; x }
    pub fn below(&mut self, n: usize) -> usize { if n == 0 { 0 } else { (self.next() % n as u64) as usize } }
}

const INTERESTING: [u32; 12] = [0, 1, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000, 0xffff, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff];

/// One mutation pass in the style of libFuzzer's default mutators.
pub fn mutate(rng: &mut Rng, input: &mut Vec<u8>, pool: &[Vec<u8>], max_len: usize) {
    let rounds = 1 + rng.below(8);
    for _ in 0..rounds {
        let n = input.len();
        match rng.below(10) {
            0 if n > 0 => { let i = rng.below(n); input[i] ^= 1 << rng.below(8); }
            1 if n > 0 => { let i = rng.below(n); input[i] = rng.next() as u8; }
            2 if n > 0 => {
                let v = INTERESTING[rng.below(INTERESTING.len())];
                let w = [1, 2, 4][rng.below(3)];
                let i = rng.below(n);
                for k in 0..w { if i + k < n { input[i + k] = (v >> (8 * k)) as u8; } }
            }
            3 if n < max_len => { let i = rng.below(n + 1); let c = 1 + rng.below(16); for _ in 0..c { input.insert(i, rng.next() as u8); } }
            4 if n > 0 => { let i = rng.below(n); let c = (1 + rng.below(16)).min(n - i); input.drain(i..i + c); }
            5 if n > 1 => { let a = rng.below(n); let b = rng.below(n); let c = (1 + rng.below(32)).min(n - a).min(n - b); let tmp = input[a..a + c].to_vec(); input[b..b + c].copy_from_slice(&tmp); }
            6 if !pool.is_empty() => {
                let other = &pool[rng.below(pool.len())];
                if !other.is_empty() { let s = rng.below(other.len()); let c = (1 + rng.below(64)).min(other.len() - s); let i = rng.below(n + 1); input.splice(i..i, other[s..s + c].iter().copied()); }
            }
            7 if n > 0 => { let i = rng.below(n); let c = (1 + rng.below(8)).min(n - i); let b = [0u8, 0xff][rng.below(2)]; input[i..i + c].fill(b); }
            8 if n > 0 => { let c = rng.below(n); input.truncate(c.max(1)); }
            _ => { let c = 1 + rng.below(8); for _ in 0..c { if input.len() < max_len { input.push(rng.next() as u8); } } }
        }
        input.truncate(max_len);
    }
}

fn hex(d: &[u8]) -> String { d.iter().take(256).map(|b| format!("{b:02x}")).collect() }

fn env_u64(name: &str, default: u64) -> u64 { std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default) }

fn one(run: fn(&[u8]), input: &[u8]) -> Result<(), String> {
    reset_reads();
    sex_pdx::set_hook(tick);
    sex_pdx::reset_clock();
    match std::panic::catch_unwind(|| run(input)) {
        Ok(()) => Ok(()),
        Err(e) => Err(e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()),
    }
}

fn report(name: &str, input: &[u8], why: &str) -> ! {
    let dir = std::env::var("DRIVER_FUZZ_OUT").unwrap_or_else(|_| "crashes".into());
    let _ = std::fs::create_dir_all(&dir);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input { h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3); }
    let path = format!("{dir}/{name}-{h:016x}.bin");
    let _ = std::fs::write(&path, input);
    eprintln!("[driver_fuzz.fail] target={name} len={} reason={why:?}\n  input(hex, first 256 B)={}\n  replay: cargo run --release --bin {name} -- {path}", input.len(), hex(input));
    std::process::exit(1)
}

/// One libFuzzer input: fresh hang budget and model clock, then the target.
pub fn fuzz_one(run: fn(&[u8]), input: &[u8]) {
    reset_reads();
    sex_pdx::set_hook(tick);
    sex_pdx::reset_clock();
    run(input)
}

/// The entry point of every target when not built by cargo-fuzz.
pub fn main(name: &str, run: fn(&[u8]), seeds: fn() -> Vec<Vec<u8>>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--seeds") {
        let dir = args.get(1).expect("--seeds DIR");
        std::fs::create_dir_all(dir).unwrap();
        for (i, s) in seeds().iter().enumerate() { std::fs::write(format!("{dir}/seed-{i:03}"), s).unwrap(); }
        return;
    }
    if !args.is_empty() {
        for f in &args {
            let d = std::fs::read(f).unwrap_or_else(|e| panic!("{f}: {e}"));
            if let Err(why) = one(run, &d) { report(name, &d, &why); }
            println!("[driver_fuzz.replay] target={name} file={f} ok reads={}", reads());
        }
        return;
    }
    // Quiet panics: a failing input is reported once, by `report`.
    std::panic::set_hook(Box::new(|_| {}));
    let iters = env_u64("DRIVER_FUZZ_ITERS", 3000);
    let seconds = env_u64("DRIVER_FUZZ_SECONDS", 0);
    let seed = env_u64("DRIVER_FUZZ_SEED", 1);
    let max_len = env_u64("DRIVER_FUZZ_MAX_LEN", 4096) as usize;
    let start = std::time::Instant::now();
    let mut pool = seeds();
    pool.push(Vec::new());
    for s in pool.clone() { if let Err(why) = one(run, &s) { let _ = std::panic::take_hook(); report(name, &s, &why); } }
    let mut rng = Rng::new(seed ^ name.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64)));
    let mut n = 0u64;
    let mut max_reads = 0u64;
    loop {
        if seconds > 0 { if start.elapsed().as_secs() >= seconds { break; } } else if n >= iters { break; }
        let mut input = if rng.below(8) == 0 {
            (0..rng.below(max_len.min(1024) + 1)).map(|_| rng.next() as u8).collect()
        } else {
            pool[rng.below(pool.len())].clone()
        };
        mutate(&mut rng, &mut input, &pool, max_len);
        if let Err(why) = one(run, &input) { report(name, &input, &why); }
        max_reads = max_reads.max(reads());
        // Keep a few inputs that went deep (many device reads) as new parents.
        if reads() * 4 > max_reads * 3 && pool.len() < 256 { pool.push(input); }
        n += 1;
    }
    println!("[driver_fuzz.ok] target={name} inputs={n} seed={seed} pool={} max_reads={max_reads} secs={}", pool.len(), start.elapsed().as_secs());
}

/// `driver_target!(run, seeds)` at the end of a target makes both entry points.
#[macro_export]
macro_rules! driver_target {
    ($run:path, $seeds:path) => {
        #[cfg(all(fuzzing, feature = "libfuzzer"))]
        $crate::libfuzzer_sys::fuzz_target!(|d: &[u8]| $crate::fuzz_one($run, d));
        #[cfg(not(all(fuzzing, feature = "libfuzzer")))]
        fn main() { $crate::main(env!("CARGO_BIN_NAME"), $run, $seeds) }
    };
}
