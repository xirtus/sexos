// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Fuzz hooks: run a driver against the most hostile device possible.
//!
//! The same harness the in-tree drivers use (`fuzz/drivers`, see its README),
//! packaged for drivers that live outside the tree. A target is a function
//! `fn(&[u8])` that builds the driver over [`FuzzRegs`] (every register read is
//! the next word of the input) and drives it; it passes when it neither panics
//! nor reads the device more than [`READ_BUDGET`] times for one input (a driver
//! that polls forever on a dead device). Two ways to run it:
//!
//! * built-in mutator, any toolchain: a binary whose `main` is
//!   [`fuzz_main!`]`(run, seeds)`; `cargo run --release --bin NAME` runs it,
//!   `NAME FILE...` replays inputs, `NAME --seeds DIR` writes the seed corpus.
//!   `HARNESS_FUZZ_SECONDS`, `HARNESS_FUZZ_ITERS`, `HARNESS_FUZZ_SEED` and
//!   `HARNESS_FUZZ_OUT` steer it (the `DRIVER_FUZZ_*` names work too).
//! * libFuzzer (nightly, coverage guided): in a cargo-fuzz target,
//!   `fuzz_target!(|d: &[u8]| harness::fuzz::one(run, d));`.

use std::cell::Cell;

use crate::error::Error;
use crate::irq::Irq;
use crate::regs::Regs;

/// Device reads (and clock looks) one input may cause before the driver counts
/// as stuck. Every wait in Harness is bounded by `wait::MAX_POLLS` (1M looks);
/// a driver loop that is not, shows up here.
pub const READ_BUDGET: u64 = 4_000_000;

thread_local! { static READS: Cell<u64> = const { Cell::new(0) }; }

/// Counts one device read; panics (`DRIVER_HANG`) when the run looks endless.
/// [`crate::mock::ModelRegs`], [`crate::mock::MockClock`] and [`FuzzRegs`] call it.
pub fn tick() {
    READS.with(|r| {
        let n = r.get() + 1;
        r.set(n);
        if n > READ_BUDGET { panic!("DRIVER_HANG: more than {READ_BUDGET} device reads for one input"); }
    });
}
pub fn reads() -> u64 { READS.with(|r| r.get()) }
pub fn reset_reads() { READS.with(|r| r.set(0)); }

/// The input as a stream of device answers; past its end it reads zeros.
pub struct Src<'a> { data: &'a [u8], at: usize }
impl<'a> Src<'a> {
    pub fn new(data: &'a [u8]) -> Self { Src { data, at: 0 } }
    pub fn is_empty(&self) -> bool { self.at >= self.data.len() }
    pub fn u8(&mut self) -> u8 { let v = self.data.get(self.at).copied().unwrap_or(0); self.at = self.at.saturating_add(1); v }
    pub fn u16(&mut self) -> u16 { u16::from_le_bytes([self.u8(), self.u8()]) }
    pub fn u32(&mut self) -> u32 { u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()]) }
    pub fn u64(&mut self) -> u64 { self.u32() as u64 | (self.u32() as u64) << 32 }
    pub fn bool(&mut self) -> bool { self.u8() & 1 != 0 }
    /// A value in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize { if n == 0 { 0 } else { self.u32() as usize % n } }
    /// Up to `n` bytes (fewer at the end of the input).
    pub fn take(&mut self, n: usize) -> &'a [u8] {
        let s = self.at.min(self.data.len());
        let e = s.saturating_add(n).min(self.data.len());
        self.at = s.saturating_add(n);
        &self.data[s..e]
    }
    /// A u16-length-prefixed chunk, so one input can carry several blobs.
    pub fn chunk(&mut self) -> &'a [u8] { let n = self.u16() as usize; self.take(n) }
    pub fn rest(&mut self) -> &'a [u8] { let s = self.at.min(self.data.len()); self.at = self.data.len(); &self.data[s..] }
}

/// Registers whose every read is the next input word: values change between
/// reads and bits that "never" set do. Past the end of the input the device
/// either fell off the bus (all ones) or hung (zeros); the first input byte's
/// top bit picks which, so both endless-wait shapes meet a device that never
/// answers. `pin` fixes chosen registers (an identity register, so bring-up
/// gets past the check most of the time).
pub struct FuzzRegs<'a> { pub src: Src<'a>, pub writes: u64, pub tail: u32, pins: Vec<(u32, u32)> }
impl<'a> FuzzRegs<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let tail = if data.first().is_some_and(|b| b & 0x80 != 0) { 0 } else { 0xffff_ffff };
        Self::with_tail(data, tail)
    }
    pub fn with_tail(data: &'a [u8], tail: u32) -> Self { FuzzRegs { src: Src::new(data), writes: 0, tail, pins: Vec::new() } }
    /// `offset` always reads `value`.
    pub fn pin(mut self, offset: u32, value: u32) -> Self { self.pins.push((offset, value)); self }
}
impl Regs for FuzzRegs<'_> {
    fn read32(&mut self, offset: u32) -> u32 {
        tick();
        if let Some(&(_, v)) = self.pins.iter().find(|(o, _)| *o == offset) { return v; }
        if self.src.is_empty() { self.tail } else { self.src.u32() }
    }
    fn write32(&mut self, _offset: u32, _value: u32) { self.writes += 1; }
}

/// Interrupt bits from the input: any vector may be pending at any take, masked
/// or not (a device that interrupts when told not to).
pub struct FuzzIrq { pub bits: u64 }
impl Irq for FuzzIrq {
    fn unmask(&mut self, _vector: u8) -> Result<(), Error> { Ok(()) }
    fn mask(&mut self, _vector: u8) -> Result<(), Error> { Ok(()) }
    fn take(&mut self) -> u64 {
        tick();
        let b = self.bits;
        self.bits = self.bits.rotate_left(7) ^ 0x9e37_79b9_7f4a_7c15;
        b
    }
}

/// Little xorshift for the built-in mutator (deterministic per seed).
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1) }
    #[allow(clippy::should_implement_trait)]
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
            5 if n > 1 => { let a = rng.below(n); let b = rng.below(n); let c = (1 + rng.below(32)).min(n - a).min(n - b); let t = input[a..a + c].to_vec(); input[b..b + c].copy_from_slice(&t); }
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

/// Runs one input with a fresh hang budget; `Err` holds the panic message.
pub fn try_one(run: fn(&[u8]), input: &[u8]) -> Result<(), String> {
    reset_reads();
    match std::panic::catch_unwind(|| run(input)) {
        Ok(()) => Ok(()),
        Err(e) => Err(e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()),
    }
}

/// One libFuzzer input: fresh hang budget, then the target (a panic is the finding).
pub fn one(run: fn(&[u8]), input: &[u8]) { reset_reads(); run(input) }

fn env_u64(names: &[&str], default: u64) -> u64 {
    names.iter().find_map(|n| std::env::var(n).ok().and_then(|v| v.parse().ok())).unwrap_or(default)
}

fn hash(d: &[u8]) -> u64 { d.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3)) }

fn report(name: &str, input: &[u8], why: &str) -> ! {
    let dir = std::env::var("HARNESS_FUZZ_OUT").or_else(|_| std::env::var("DRIVER_FUZZ_OUT")).unwrap_or_else(|_| "crashes".into());
    let _ = std::fs::create_dir_all(&dir);
    let path = format!("{dir}/{name}-{:016x}.bin", hash(input));
    let _ = std::fs::write(&path, input);
    let hex: String = input.iter().take(256).map(|b| format!("{b:02x}")).collect();
    eprintln!("[harness.fuzz.fail] target={name} len={} reason={why:?}\n  input(hex, first 256 B)={hex}\n  replay: cargo run --release --bin {name} -- {path}", input.len());
    std::process::exit(1)
}

/// Summary of a [`run`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome { pub inputs: u64, pub pool: usize, pub max_reads: u64 }

/// The built-in mutator: every seed, then `iters` mutated inputs (or `seconds`
/// of them when non-zero). Returns the first failing input and why.
pub fn run(run: fn(&[u8]), seeds: &[Vec<u8>], seed: u64, iters: u64, seconds: u64, max_len: usize) -> Result<Outcome, (Vec<u8>, String)> {
    let start = std::time::Instant::now();
    let mut pool: Vec<Vec<u8>> = seeds.to_vec();
    pool.push(Vec::new());
    for s in &pool { try_one(run, s).map_err(|w| (s.clone(), w))?; }
    let mut rng = Rng::new(seed);
    let (mut n, mut max_reads) = (0u64, 0u64);
    loop {
        if seconds > 0 { if start.elapsed().as_secs() >= seconds { break; } } else if n >= iters { break; }
        let mut input = if rng.below(8) == 0 {
            (0..rng.below(max_len.min(1024) + 1)).map(|_| rng.next() as u8).collect()
        } else {
            pool[rng.below(pool.len())].clone()
        };
        mutate(&mut rng, &mut input, &pool, max_len);
        try_one(run, &input).map_err(|w| (input.clone(), w))?;
        max_reads = max_reads.max(reads());
        // Inputs that went deep (many device reads) become parents.
        if reads() * 4 > max_reads * 3 && pool.len() < 256 { pool.push(input); }
        n += 1;
    }
    Ok(Outcome { inputs: n, pool: pool.len(), max_reads })
}

/// The `main` of a built-in-mutator fuzz binary (see [`fuzz_main!`]).
pub fn main(name: &str, target: fn(&[u8]), seeds: fn() -> Vec<Vec<u8>>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--seeds") {
        let dir = args.get(1).expect("--seeds DIR");
        std::fs::create_dir_all(dir).expect("seed dir");
        for (i, s) in seeds().iter().enumerate() { std::fs::write(format!("{dir}/seed-{i:03}"), s).expect("seed"); }
        return;
    }
    std::panic::set_hook(Box::new(|_| {}));
    if !args.is_empty() {
        for f in &args {
            let d = std::fs::read(f).unwrap_or_else(|e| panic!("{f}: {e}"));
            if let Err(why) = try_one(target, &d) { report(name, &d, &why); }
            println!("[harness.fuzz.replay] target={name} file={f} ok reads={}", reads());
        }
        return;
    }
    let iters = env_u64(&["HARNESS_FUZZ_ITERS", "DRIVER_FUZZ_ITERS"], 3000);
    let seconds = env_u64(&["HARNESS_FUZZ_SECONDS", "DRIVER_FUZZ_SECONDS"], 0);
    let seed = env_u64(&["HARNESS_FUZZ_SEED", "DRIVER_FUZZ_SEED"], 1);
    let max_len = env_u64(&["HARNESS_FUZZ_MAX_LEN", "DRIVER_FUZZ_MAX_LEN"], 4096) as usize;
    let start = std::time::Instant::now();
    match run(target, &seeds(), seed ^ hash(name.as_bytes()), iters, seconds, max_len) {
        Ok(o) => println!("[harness.fuzz.ok] target={name} inputs={} seed={seed} pool={} max_reads={} secs={}", o.inputs, o.pool, o.max_reads, start.elapsed().as_secs()),
        Err((input, why)) => report(name, &input, &why),
    }
}

/// `harness::fuzz_main!(run, seeds);` makes a binary's `main` run the built-in mutator.
#[macro_export]
macro_rules! fuzz_main {
    ($run:path, $seeds:path) => {
        fn main() { $crate::fuzz::main(env!("CARGO_BIN_NAME"), $run, $seeds) }
    };
}
