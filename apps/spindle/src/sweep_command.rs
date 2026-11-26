// SPDX-License-Identifier: GPL-3.0-only
//! Real-hardware microkernel & NVMe sweep benchmarks (`sweep sched|ipc|nvme|all`).
//!
//! All scratch buffers (sample arrays, path buffer) live on the **stack** only while
//! the command runs, with zero static BSS buffers and zero heap allocations (Spindle
//! runs with `DummyAllocator` and a bounded memory budget).

use crate::sweep_math::{
    compute_percentiles, cycles_to_ns, parse_sweep_args, throughput_kib_s, throughput_mib_tenths,
    Percentiles, SweepKind,
};
use crate::Scrollback;
use sex_pdx::endpoint::{self as ep, OP_CREATE, OP_DESTROY, OP_POLL, OP_SIGNAL, OP_STATS};
use sex_pdx::fs as w;
use sex_pdx::ipc_recv::pdx_call_recv;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

const WARMUP: usize = 8;
const SAMPLE_CAP: usize = 256;
const FS_TIMEOUT_MS: u64 = 5_000;

struct Text {
    buf: [u8; 160],
    len: usize,
}
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len.min(110)]);
    sex_pdx::serial_record!("[spindle.sweep.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}

#[inline]
fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

#[inline]
fn null_syscall() {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 40u64 => _,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
}

/// Estimates TSC cycles per millisecond (kHz) against `get_ticks()` (~1 ms/tick).
fn calibrate_khz() -> u64 {
    let t0 = sex_pdx::get_ticks();
    let mut spins = 0u32;
    let mut start = t0;
    while start == t0 && spins < 5_000_000 {
        start = sex_pdx::get_ticks();
        spins += 1;
    }
    let c0 = rdtsc();
    let target = start.saturating_add(20);
    spins = 0;
    while sex_pdx::get_ticks() < target && spins < 50_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
    let c1 = rdtsc();
    let dt = sex_pdx::get_ticks().saturating_sub(start);
    if dt == 0 || c1 <= c0 {
        0
    } else {
        (c1 - c0) / dt
    }
}

fn measure<F: FnMut()>(n: usize, mut f: F) -> Percentiles {
    let mut samples = [0u64; SAMPLE_CAP];
    let stride = (n / SAMPLE_CAP).max(1);
    let mut count = 0usize;
    for _ in 0..WARMUP {
        f();
    }
    for i in 0..n {
        let t0 = rdtsc();
        f();
        let t1 = rdtsc();
        if i % stride == 0 && count < SAMPLE_CAP {
            samples[count] = t1.saturating_sub(t0);
            count += 1;
        }
    }
    compute_percentiles(&mut samples[..count])
}

fn show_stats(sb: &mut Scrollback, label: &str, s: Percentiles, khz: u64) {
    let p50_ns = cycles_to_ns(s.p50, khz);
    let p99_ns = cycles_to_ns(s.p99, khz);
    say!(
        sb,
        "  {:<10} p50={:>6} ({:>5}ns) p95={:>6} p99={:>6} ({:>5}ns) max={:>7}",
        label,
        s.p50,
        p50_ns,
        s.p95,
        s.p99,
        p99_ns,
        s.max
    );
}

#[derive(Clone, Copy, Default)]
struct SchedSummary {
    null_p50: u64,
    yield_p50: u64,
}

fn run_sched(n: usize, khz: u64, sb: &mut Scrollback) -> SchedSummary {
    say!(sb, "sweep sched (N={}, tsc~{} MHz):", n, khz / 1000);
    let null_s = measure(n, null_syscall);
    show_stats(sb, "null_sys", null_s, khz);
    let tick_s = measure(n, || {
        let _ = core::hint::black_box(sex_pdx::get_ticks());
    });
    show_stats(sb, "get_ticks", tick_s, khz);
    let yield_s = measure(n, sex_pdx::sys_yield);
    show_stats(sb, "sys_yield", yield_s, khz);
    sex_pdx::serial_record!(
        "[spindle.sweep.sched] n={} khz={} null_p50={} null_p99={} yield_p50={} yield_p99={}",
        n,
        khz,
        null_s.p50,
        null_s.p99,
        yield_s.p50,
        yield_s.p99
    );
    SchedSummary {
        null_p50: null_s.p50,
        yield_p50: yield_s.p50,
    }
}

fn fs_region() -> Option<&'static mut [u8]> {
    let view = sex_pdx::memory_region_view(
        w::CLIENT_SLOT,
        0,
        w::REGION_BYTES as u64,
        QUERY_VIEW | READ | WRITE,
    )
    .ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) })
}

/// Sends one synchronous Files request using `pdx_call_recv` (syscall 102) with a bounded timeout.
fn fs_sync_call(seq: u32, kind: u8, path_len: usize, offset: u64, length: u64, sb: &mut Scrollback) -> Option<(u8, u32)> {
    let req = w::Request {
        seq,
        kind,
        path_len: path_len as u16,
        offset,
        length,
    };
    let a = req.args();
    let deadline = sex_pdx::get_ticks().saturating_add(FS_TIMEOUT_MS);
    let first = match pdx_call_recv(sex_pdx::SLOT_STORAGE, w::OP, a[0], a[1], a[2]) {
        Ok(m) => m,
        Err(_) => return None,
    };
    let mut cur = first;
    loop {
        if let Some(msg) = cur {
            if msg.type_id == 1 && msg.caller_pd == 11 {
                if let Some((rseq, status, value)) = w::decode_reply(msg.arg0) {
                    if rseq == seq {
                        return Some((status, value));
                    }
                }
            } else if msg.type_id == 1 && msg.caller_pd == 13 {
                unsafe {
                    (*core::ptr::addr_of_mut!(crate::tether_command::TETHER))
                        .receive(msg.caller_pd, msg.arg0, sb);
                }
            }
        }
        if sex_pdx::get_ticks() > deadline {
            return None;
        }
        cur = match sex_pdx::pdx_try_listen_raw(0) {
            Some(m) => Some(m),
            None => {
                sex_pdx::sys_yield();
                None
            }
        };
    }
}

#[derive(Clone, Copy, Default)]
struct IpcSummary {
    notify_p50: u64,
    endpoint_p50: u64,
    pdx_p50: u64,
}

fn run_ipc(n: usize, khz: u64, sb: &mut Scrollback) -> IpcSummary {
    say!(sb, "sweep ipc (N={}, tsc~{} MHz):", n, khz / 1000);
    let mut out = IpcSummary::default();

    // 1. IPC v2 SYS_NOTIFY (106): signal + poll round-trip
    #[cfg(target_arch = "x86_64")]
    {
        match ep::notify(OP_CREATE, 0, 0) {
            Ok(slot) => {
                let note = slot as u32;
                let s = measure(n, || {
                    let _ = ep::notify(OP_SIGNAL, note, 1);
                    let _ = ep::notify(OP_POLL, note, 0);
                });
                show_stats(sb, "notify106", s, khz);
                out.notify_p50 = s.p50;
                let _ = ep::notify(OP_DESTROY, note, 0);
            }
            Err(e) => say!(sb, "  notify106  unavailable ({:#x})", e),
        }

        // 2. IPC v2 SYS_ENDPOINT (105): create + stats query
        match ep::create_endpoint() {
            Ok(ep_slot) => {
                let s = measure(n, || {
                    let _ = ep::endpoint(OP_STATS, 0, [0; 4]);
                });
                show_stats(sb, "endpt105", s, khz);
                out.endpoint_p50 = s.p50;
                let _ = ep::destroy(ep_slot);
            }
            Err(e) => say!(sb, "  endpt105   unavailable ({:#x})", e),
        }
    }

    // 3. Cross-domain pdx_call_recv (102) round-trip to Files (PING)
    if unsafe { crate::fs_commands::FS.busy() } {
        sb.push(b"  pdx102     skipped (disk busy)");
    } else {
        let mut samples = [0u64; SAMPLE_CAP];
        let stride = (n / SAMPLE_CAP).max(1);
        let mut count = 0usize;
        let mut seq = 0x00E0_0000u32;
        let mut ok = true;
        for _ in 0..WARMUP {
            seq = (seq + 1) & 0x00FF_FFFF;
            if fs_sync_call(seq, w::PING, 0, 0, 0, sb).is_none() {
                ok = false;
                break;
            }
        }
        if ok {
            for i in 0..n {
                seq = (seq + 1) & 0x00FF_FFFF;
                let t0 = rdtsc();
                let res = fs_sync_call(seq, w::PING, 0, 0, 0, sb);
                let t1 = rdtsc();
                if res.is_none() {
                    ok = false;
                    break;
                }
                if i % stride == 0 && count < SAMPLE_CAP {
                    samples[count] = t1.saturating_sub(t0);
                    count += 1;
                }
            }
        }
        if ok && count > 0 {
            let s = compute_percentiles(&mut samples[..count]);
            show_stats(sb, "pdx102", s, khz);
            out.pdx_p50 = s.p50;
        } else {
            sb.push(b"  pdx102     Files PING timed out or refused");
        }
    }

    sex_pdx::serial_record!(
        "[spindle.sweep.ipc] n={} notify_p50={} endpoint_p50={} pdx_p50={}",
        n,
        out.notify_p50,
        out.endpoint_p50,
        out.pdx_p50
    );
    out
}

/// Probes a single file via `STAT`, returning its size if it is a non-empty regular file.
fn stat_regular_file(path: &[u8], sb: &mut Scrollback, seq: &mut u32) -> Option<u64> {
    if path.is_empty() || path.len() > w::PATH_BYTES {
        return None;
    }
    let region = fs_region()?;
    region[..path.len()].copy_from_slice(path);
    *seq = (*seq + 1) & 0x00FF_FFFF;
    let (status, _) = fs_sync_call(*seq, w::STAT, path.len(), 0, 0, sb)?;
    if status != w::OK {
        return None;
    }
    let data = &region[w::PATH_BYTES..];
    let is_dir = u64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8])) == 1;
    let sz = u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
    if is_dir || sz == 0 {
        None
    } else {
        Some(sz)
    }
}

/// Finds a readable file on `/media/Ventoy`, `/media/windows`, or `/` when no path was supplied.
fn auto_pick_file(sb: &mut Scrollback, seq: &mut u32) -> Option<([u8; w::PATH_BYTES], usize, u64)> {
    let dirs: &[&[u8]] = &[b"/media/Ventoy", b"/media/windows", b"/"];
    let mut best_path = [0u8; w::PATH_BYTES];
    let mut best_len: usize = 0;
    let mut best_size: u64 = 0;

    for &dir in dirs {
        let Some(region) = fs_region() else { return None };
        region[..dir.len()].copy_from_slice(dir);
        *seq = (*seq + 1) & 0x00FF_FFFF;
        let Some((status, value)) = fs_sync_call(*seq, w::LIST, dir.len(), 0, 0, sb) else {
            continue;
        };
        if status != w::OK {
            continue;
        }
        let data = &region[w::PATH_BYTES..];
        let mut at = w::LIST_HEAD;
        while at + w::ENTRY_HEAD <= value as usize {
            let is_dir = data[at] == 1;
            let len = data[at + 1] as usize;
            let size = u64::from_le_bytes(data[at + 2..at + 10].try_into().unwrap_or([0; 8]));
            if !is_dir && len > 0 && at + w::ENTRY_HEAD + len <= data.len() && size > best_size {
                let name = &data[at + w::ENTRY_HEAD..at + w::ENTRY_HEAD + len];
                let sep = usize::from(dir != b"/");
                let need = dir.len() + sep + name.len();
                if need <= w::PATH_BYTES {
                    best_path[..dir.len()].copy_from_slice(dir);
                    if sep == 1 {
                        best_path[dir.len()] = b'/';
                    }
                    best_path[dir.len() + sep..need].copy_from_slice(name);
                    best_len = need;
                    best_size = size;
                }
            }
            at += w::ENTRY_HEAD + len;
        }
        if best_size >= 1024 * 1024 {
            break;
        }
    }

    if best_size == 0 {
        let fallbacks: &[&[u8]] = &[
            b"/media/Ventoy/sexos-x17-c28152a0.iso",
            b"/media/Ventoy/wifi-sweep.txt",
            b"/media/windows/Windows/win.ini",
        ];
        for &candidate in fallbacks {
            if let Some(sz) = stat_regular_file(candidate, sb, seq) {
                best_path[..candidate.len()].copy_from_slice(candidate);
                best_len = candidate.len();
                best_size = sz;
                break;
            }
        }
    }

    if best_size > 0 && best_len > 0 {
        Some((best_path, best_len, best_size))
    } else {
        None
    }
}

#[derive(Clone, Copy, Default)]
struct NvmeSummary {
    kib_s_4k: u64,
    kib_s_16k: u64,
    kib_s_64k: u64,
}

fn bench_read_chunk(
    path: &[u8],
    file_size: u64,
    chunk: usize,
    total_cap: u64,
    khz: u64,
    seq: &mut u32,
    sb: &mut Scrollback,
    label: &str,
) -> u64 {
    let target = file_size.min(total_cap);
    if target == 0 {
        return 0;
    }

    let mut latencies = [0u64; SAMPLE_CAP];
    let mut lat_count = 0usize;

    let mut offset = 0u64;
    let mut total_bytes = 0u64;
    let t_start = sex_pdx::get_ticks();
    let c_start = rdtsc();

    while offset < target {
        let want = (chunk as u64).min(target - offset);
        let op_t0 = rdtsc();
        let mut sub_done = 0u64;
        while sub_done < want {
            let step = ((want - sub_done) as usize).min(w::IO_MAX).min(64 * 1024);
            let Some(region) = fs_region() else {
                return 0;
            };
            region[..path.len()].copy_from_slice(path);
            *seq = (*seq + 1) & 0x00FF_FFFF;
            let Some((status, got)) = fs_sync_call(*seq, w::READ, path.len(), offset + sub_done, step as u64, sb) else {
                say!(sb, "  {:<8} read timed out at offset {}", label, offset + sub_done);
                return 0;
            };
            if status != w::OK || got == 0 {
                break;
            }
            let n = (got as usize).min(region.len().saturating_sub(w::PATH_BYTES));
            // Touch one byte per 4 KiB page directly in the Files shared region (zero heap copy).
            let data = &region[w::PATH_BYTES..w::PATH_BYTES + n];
            let mut p = 0usize;
            while p < data.len() {
                core::hint::black_box(data[p]);
                p += 4096;
            }
            sub_done += got as u64;
            if (got as usize) < step {
                break;
            }
        }
        let op_t1 = rdtsc();
        if sub_done == 0 {
            break;
        }
        if lat_count < SAMPLE_CAP {
            latencies[lat_count] = op_t1.saturating_sub(op_t0);
            lat_count += 1;
        }
        offset += sub_done;
        total_bytes += sub_done;
    }

    let c_elapsed = rdtsc().saturating_sub(c_start);
    let mut ms = sex_pdx::get_ticks().saturating_sub(t_start);
    if ms == 0 && khz > 0 {
        ms = (c_elapsed / khz).max(1);
    }
    let kib_s = throughput_kib_s(total_bytes, ms);
    let (mib_w, mib_t) = throughput_mib_tenths(total_bytes, ms);
    let stats = compute_percentiles(&mut latencies[..lat_count]);
    let p50_us = cycles_to_ns(stats.p50, khz) / 1000;
    let p99_us = cycles_to_ns(stats.p99, khz) / 1000;
    say!(
        sb,
        "  {:<8} {:>6} KiB in {:>4}ms -> {:>7} KiB/s ({}.{} MiB/s) p50={}us p99={}us",
        label,
        total_bytes / 1024,
        ms,
        kib_s,
        mib_w,
        mib_t,
        p50_us,
        p99_us
    );
    kib_s
}

fn run_nvme(path_arg: Option<&[u8]>, khz: u64, sb: &mut Scrollback) -> NvmeSummary {
    if unsafe { crate::fs_commands::FS.busy() } {
        sb.push(b"sweep nvme: disk is busy with another command.");
        return NvmeSummary::default();
    }
    let mut seq = 0x00D0_0000u32;
    let (path_buf, path_len, size) = match path_arg {
        Some(p) => {
            let mut full = [0u8; w::PATH_BYTES];
            let Some(n) = (unsafe { crate::fs_commands::FS.join(p, &mut full) }) else {
                sb.push(b"sweep nvme: path too long.");
                return NvmeSummary::default();
            };
            let Some(region) = fs_region() else {
                sb.push(b"sweep nvme: Files region unavailable.");
                return NvmeSummary::default();
            };
            region[..n].copy_from_slice(&full[..n]);
            seq = (seq + 1) & 0x00FF_FFFF;
            let Some((status, _)) = fs_sync_call(seq, w::STAT, n, 0, 0, sb) else {
                sb.push(b"sweep nvme: stat timed out.");
                return NvmeSummary::default();
            };
            if status != w::OK {
                say!(sb, "sweep nvme: {}", core::str::from_utf8(w::message(status)).unwrap_or("error"));
                return NvmeSummary::default();
            }
            let data = &region[w::PATH_BYTES..];
            let is_dir = u64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8])) == 1;
            let sz = u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
            if is_dir || sz == 0 {
                sb.push(b"sweep nvme: give a non-empty file path (not a directory).");
                return NvmeSummary::default();
            }
            (full, n, sz)
        }
        None => match auto_pick_file(sb, &mut seq) {
            Some(found) => found,
            None => {
                sb.push(b"sweep nvme: no readable file found on /media/Ventoy, /media/windows, or /.");
                sb.push(b"  Pass an explicit path: sweep nvme /media/Ventoy/<file>");
                return NvmeSummary::default();
            }
        },
    };
    let path = &path_buf[..path_len];

    say!(
        sb,
        "sweep nvme: {} ({} KiB, read-only):",
        core::str::from_utf8(path).unwrap_or("?"),
        size / 1024
    );
    let kib_s_4k = bench_read_chunk(path, size, 4 * 1024, 256 * 1024, khz, &mut seq, sb, "4KiB");
    let kib_s_16k = bench_read_chunk(path, size, 16 * 1024, 1024 * 1024, khz, &mut seq, sb, "16KiB");
    let kib_s_64k = bench_read_chunk(path, size, 64 * 1024, 4 * 1024 * 1024, khz, &mut seq, sb, "64KiB");

    sex_pdx::serial_record!(
        "[spindle.sweep.nvme] kib_s_4k={} kib_s_16k={} kib_s_64k={}",
        kib_s_4k,
        kib_s_16k,
        kib_s_64k
    );
    NvmeSummary {
        kib_s_4k,
        kib_s_16k,
        kib_s_64k,
    }
}

pub fn command(args: &[u8], sb: &mut Scrollback) {
    let Some(kind) = parse_sweep_args(args) else {
        sb.push(b"Usage: sweep sched [N] | sweep ipc [N] | sweep nvme [PATH] | sweep all [PATH]");
        sb.push(b"  sched: null syscall + get_ticks + sys_yield cycles/ns (N: 1..10000, default 64)");
        sb.push(b"  ipc:   IPC v2 notify(106) + endpoint(105) + pdx_call_recv(102) latency");
        sb.push(b"  nvme:  read-only sequential 4KiB / 16KiB / 64KiB throughput");
        return;
    };
    let khz = calibrate_khz();
    match kind {
        SweepKind::Sched(n) => {
            run_sched(n, khz, sb);
        }
        SweepKind::Ipc(n) => {
            run_ipc(n, khz, sb);
        }
        SweepKind::Nvme(path) => {
            run_nvme(path, khz, sb);
        }
        SweepKind::All(path) => {
            let s = run_sched(crate::sweep_math::DEFAULT_ITERS, khz, sb);
            let i = run_ipc(crate::sweep_math::DEFAULT_ITERS, khz, sb);
            let n = run_nvme(path, khz, sb);
            sb.push(b"--- sweep all summary ---");
            say!(
                sb,
                "  sched: null_p50={} cyc  yield_p50={} cyc (tsc~{} MHz)",
                s.null_p50,
                s.yield_p50,
                khz / 1000
            );
            say!(
                sb,
                "  ipc:   notify106={} cyc  endpt105={} cyc  pdx102={} cyc",
                i.notify_p50,
                i.endpoint_p50,
                i.pdx_p50
            );
            say!(
                sb,
                "  nvme:  4KiB={} KiB/s  16KiB={} KiB/s  64KiB={} KiB/s",
                n.kib_s_4k,
                n.kib_s_16k,
                n.kib_s_64k
            );
            sex_pdx::serial_record!(
                "[spindle.sweep.all] null_p50={} yield_p50={} notify_p50={} endpt_p50={} pdx_p50={} nvme_4k={} nvme_16k={} nvme_64k={}",
                s.null_p50,
                s.yield_p50,
                i.notify_p50,
                i.endpoint_p50,
                i.pdx_p50,
                n.kib_s_4k,
                n.kib_s_16k,
                n.kib_s_64k
            );
        }
    }
}
