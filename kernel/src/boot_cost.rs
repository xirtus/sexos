//! Where the kernel's own boot time goes, in TSC cycles, from the start of `init::init`
//! to the hand-over to services. One `[boot.kernel.cost]` line at the hand-over; the
//! boot time gate (`scripts/boot_time_gate.py`) compares it with host wall time, so a
//! cost that only shows under nested KVM (VM exits, waking halted cores) is named
//! instead of guessed.
use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub enum Bucket { Spawn = 0, Zero = 1, Serial = 2, PciWalk = 3 }
const NAMES: [&str; 4] = ["spawn", "zero", "serial", "pci_walk"];
static CYCLES: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static UNITS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static START: AtomicU64 = AtomicU64::new(0);
static OPEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[inline] pub fn now() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }
pub fn begin() { START.store(now(), Ordering::Relaxed); OPEN.store(true, Ordering::Release); }
/// Adds the cycles since `since` and `units` (pages, bytes, calls) to a bucket while
/// the window is open; costs nothing after the hand-over.
#[inline] pub fn add(bucket: Bucket, since: u64, units: u64) {
    if !OPEN.load(Ordering::Relaxed) { return; }
    CYCLES[bucket as usize].fetch_add(now().saturating_sub(since), Ordering::Relaxed);
    UNITS[bucket as usize].fetch_add(units, Ordering::Relaxed);
}
pub fn report() {
    if !OPEN.swap(false, Ordering::AcqRel) { return; }
    let window = now().saturating_sub(START.load(Ordering::Relaxed));
    let mut line = crate::console::Text::<320>::new();
    use core::fmt::Write;
    let _ = write!(line, "[boot.kernel.cost] window_cycles={} shootdowns={}", window, crate::smp::tlb::sent());
    for (i, name) in NAMES.iter().enumerate() {
        let _ = write!(line, " {}_cycles={} {}_units={}", name, CYCLES[i].load(Ordering::Relaxed), name, UNITS[i].load(Ordering::Relaxed));
    }
    crate::serial_println!("{}", core::str::from_utf8(line.as_bytes()).unwrap_or("[boot.kernel.cost] utf8"));
}
