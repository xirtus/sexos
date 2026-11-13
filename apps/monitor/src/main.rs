// SPDX-License-Identifier: GPL-3.0-only
//! Monitor: what the machine is doing. `run monitor` lists every service and program
//! that has used the CPU, busiest first, with its share of one core over the last
//! second and the memory it holds, and the whole machine's load and free memory.
//! The numbers come from the kernel's read-only statistics (syscall 87). Up and Down
//! choose a line; Home goes back to the top.
#![no_std]
#![no_main]

use sex_app::{nav, Event, Nav, Window};
use sex_pdx::system_stats as stats;

const MAX: usize = 96;
const SHOWN: usize = 18;
const PERIOD_MS: u64 = 1000;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[monitor.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, Default)]
struct Row { pd: u32, cycles: u64, tenths: u64, pages: u64, task: u32, syscalls: u64, rate: u64 }

struct Sample { rows: [Row; MAX], count: usize, tsc: u64, idle: u64, cores: u64, free_pages: u64, ms: u64, span_ms: u64 }

fn sample(previous: &Sample) -> Sample {
    let ms = sex_pdx::get_ticks();
    let mut s = Sample { rows: [Row::default(); MAX], count: 0, tsc: 0, idle: 0, cores: 1, free_pages: 0, ms, span_ms: 0 };
    if let Some(o) = stats::overview() { s.tsc = o.tsc; s.idle = o.idle_cycles; s.cores = o.cores.max(1); s.free_pages = o.free_pages; }
    let span_ms = ms.saturating_sub(previous.ms);
    s.span_ms = span_ms;
    let mut after = 0;
    while let Some(pd) = stats::next(after) {
        after = pd;
        if s.count == MAX { break; }
        let Some(d) = stats::domain(pd) else { continue };
        let before = previous.rows[..previous.count].iter().find(|r| r.pd == pd);
        let tenths = before.map_or(0, |b| stats::tenths_of_core(b.cycles, d.cycles, previous.tsc, s.tsc));
        let rate = before.map_or(0, |b| d.syscalls.saturating_sub(b.syscalls) * 1000 / span_ms.max(1));
        s.rows[s.count] = Row { pd, cycles: d.cycles, tenths, pages: d.pages, task: d.task, syscalls: d.syscalls, rate };
        s.count += 1;
    }
    // Busiest first; ties by domain so the list does not jump about.
    s.rows[..s.count].sort_unstable_by(|a, b| b.tenths.cmp(&a.tenths).then(a.pd.cmp(&b.pd)));
    s
}

struct Text { buf: [u8; 2048], len: usize }
impl Text {
    fn put(&mut self, b: &[u8]) { for &c in b { if self.len < self.buf.len() { self.buf[self.len] = c; self.len += 1; } } }
    fn num(&mut self, mut n: u64, width: usize) {
        let mut d = [0u8; 20]; let mut i = d.len();
        loop { i -= 1; d[i] = b'0' + (n % 10) as u8; n /= 10; if n == 0 { break; } }
        for _ in (d.len() - i)..width { self.put(b" "); }
        self.put(&d[i..]);
    }
    fn tenths(&mut self, t: u64) { self.num(t / 10, 3); self.put(b"."); self.num(t % 10, 1); self.put(b"%"); }
    fn bytes(&mut self, pages: u64) {
        if pages == stats::UNKNOWN { self.put(b"        -"); return; }
        let kib = pages * 4;
        if kib >= 10 * 1024 { self.num(kib / 1024, 6); self.put(b" MiB"); } else { self.num(kib, 6); self.put(b" KiB"); }
    }
    fn name(&mut self, pd: u32) {
        let n = stats::name(pd);
        let len = n.iter().position(|&c| c == 0).unwrap_or(16);
        let start = self.len;
        if len > 0 { self.put(&n[..len]); }
        else if (1..=15).contains(&pd) { self.put(sex_pdx::crash_log::domain_name(pd).as_bytes()); }
        else { self.put(b"domain "); self.num(pd as u64, 1); }
        while self.len < start + 16 { self.put(b" "); }
    }
}

fn render(window: &mut Window, s: &Sample, prev: &Sample, top: usize) -> usize {
    let mut t = Text { buf: [0; 2048], len: 0 };
    let span = s.tsc.saturating_sub(prev.tsc).max(1);
    let all = span.saturating_mul(s.cores);
    let busy = 1000u64.saturating_sub((s.idle.saturating_sub(prev.idle) as u128 * 1000 / all as u128).min(1000) as u64);
    t.put(b"CPU "); t.tenths(busy); t.put(b" of "); t.num(s.cores, 1); t.put(b" cores   free memory");
    t.bytes(s.free_pages); t.put(b"\n\nNAME             DOMAIN    CPU      MEMORY  CALLS/S\n");
    let top = top.min(s.count.saturating_sub(1));
    for r in s.rows[top..s.count].iter().take(SHOWN) {
        t.name(r.pd); t.num(r.pd as u64, 6); t.put(b"  "); t.tenths(r.tenths); t.put(b" "); t.bytes(r.pages); t.num(r.rate, 9); t.put(b"\n");
    }
    window.show(&t.buf[..t.len]);
    if let Some(r) = s.rows[..s.count].first() {
        sex_pdx::serial_record!("[monitor.sample] rows={} busy_tenths={} top_pd={} top_tenths={} top_task={}", s.count, busy, r.pd, r.tenths, r.task);
        for r in s.rows[..s.count].iter().take(4) {
            sex_pdx::serial_record!("[monitor.row] pd={} tenths={} pages={} calls_per_s={}", r.pd, r.tenths, r.pages, r.rate);
        }
    }
    top
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Monitor");
    let mut prev = sample(&Sample { rows: [Row::default(); MAX], count: 0, tsc: 0, idle: 0, cores: 1, free_pages: 0, ms: 0, span_ms: 0 });
    let mut top = 0usize;
    sex_pdx::serial_record!("[monitor.ready] target={} task={}", window.target, window.task);
    loop {
        match window.next_within(PERIOD_MS) {
            Some(Event::Close) => window.close(),
            Some(Event::Key(key)) => match nav(key) {
                Some(Nav::Up) => top = top.saturating_sub(1),
                Some(Nav::Down) => top += 1,
                Some(Nav::Home) => top = 0,
                _ => {}
            },
            Some(_) => {}
            None => {
                let now = sample(&prev);
                top = render(&mut window, &now, &prev, top);
                prev = now;
            }
        }
    }
}
