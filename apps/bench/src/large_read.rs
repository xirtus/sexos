//! Large reads qualified by successful device I/O, without changing queue policy.
use alloc::{string::String, vec};
use core::fmt::Write;
use sex_app::{files, Window};

const CHUNK: usize = 64 * 1024;
const TOTAL: usize = 16 * 1024 * 1024;
const PATH: &[u8] = b"bench-large.dat";
fn byte(at: usize) -> u8 {
    (at as u8).wrapping_mul(37).wrapping_add(((at >> 12) ^ (at >> 20)) as u8).wrapping_add(11)
}
fn cycles() -> u64 {
    unsafe { core::arch::x86_64::_mm_lfence(); core::arch::x86_64::_rdtsc() }
}
pub fn run(window: &mut Window) -> Result<String, &'static str> {
    let mut data = vec![0; CHUNK];
    sex_pdx::serial_record!("[bench.large.prepare] bytes={} chunk={}", TOTAL, CHUNK);
    for offset in (0..TOTAL).step_by(CHUNK) {
        for (i, value) in data.iter_mut().enumerate() { *value = byte(offset + i); }
        if offset == 0 { files::write(window, PATH, &data).map_err(|_| "large create failed")?; }
        else if files::write_at(window, PATH, offset as u64, &data) != Ok(CHUNK) { return Err("large write failed"); }
    }
    files::sync(window).map_err(|_| "large sync failed")?;
    if files::size(window, PATH) != Ok(TOTAL as u64) { return Err("wrong large file size"); }
    let initial = files::read_io_stats(window).map_err(|_| "read I/O counters unavailable")?;
    if initial != sex_pdx::fs::ReadIoStats::ZERO { return Err("read I/O counters not private to new run"); }
    sex_pdx::serial_record!("[bench.large.baseline] requests=0 device_calls=0 device_bytes=0");
    let mut report = String::from("Large device-backed reads. F11 closes.\n");
    for pass in 0..3 {
        let before = files::read_io_stats(window).map_err(|_| "read I/O counters unavailable")?;
        let cpu0 = cpu_split(window.target);
        let start = sex_pdx::get_ticks();
        let mut read_cycles = 0u64;
        for offset in (0..TOTAL).step_by(CHUNK) {
            let begin = cycles();
            let got = files::read_at(window, PATH, offset as u64, &mut data);
            read_cycles = read_cycles.checked_add(cycles().checked_sub(begin).ok_or("TSC reversed")?).ok_or("cycle overflow")?;
            if got != Ok(CHUNK) { return Err("short large read"); }
            if data.iter().enumerate().any(|(i, &got)| got != byte(offset + i)) { return Err("large data mismatch"); }
        }
        let elapsed = sex_pdx::get_ticks().checked_sub(start).ok_or("ticks reversed")?;
        let io = files::read_io_stats(window).map_err(|_| "read I/O counters unavailable")?
            .since(before).ok_or("read I/O counters reset")?;
        // Payload accounting excludes metadata. Every pass must physically
        // submit the complete file payload to the driver; cache hits cannot pass.
        if io.requests != (TOTAL / CHUNK) as u64 || io.device_calls == 0 || io.device_bytes != TOTAL as u64 {
            return Err("large read was not fully device backed");
        }
        let cpu1 = cpu_split(window.target);
        sex_pdx::serial_record!("[bench.large.cpu] pass={} drive={} files={} bench={} idle={}", pass,
            cpu1[0].wrapping_sub(cpu0[0]), cpu1[1].wrapping_sub(cpu0[1]), cpu1[2].wrapping_sub(cpu0[2]), cpu1[3].wrapping_sub(cpu0[3]));
        sex_pdx::serial_record!("[bench.large.read] pass={} bytes={} requests={} device_calls={} device_bytes={} read_cycles={} elapsed_ticks={} verified=1",
            pass, TOTAL, io.requests, io.device_calls, io.device_bytes, read_cycles, elapsed);
        let _ = writeln!(report, "Pass {}: {} cycles/MiB, {} device reads", pass + 1, read_cycles / 16, io.device_calls);
    }
    Ok(report)
}

/// CPU cycles so far of Drive, Files, this program and all cores' idle time (syscall 87).
fn cpu_split(me: u32) -> [u64; 4] {
    use sex_pdx::system_stats as st;
    let c = |pd| st::domain(pd).map_or(0, |d| d.cycles);
    [c(2), c(11), c(me), st::overview().map_or(0, |o| o.idle_cycles)]
}
