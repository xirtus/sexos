// SPDX-License-Identifier: GPL-3.0-only
//! Zero-allocation ASCII `fastfetch`/`neofetch`, `bars`, and `btop` snapshot commands for
//! the boot Spindle console (using stack buffers only, preserving DummyAllocator).

use core::fmt::{self, Write};
use sex_pdx::{mesh_stats, system_stats};

struct LineBuf {
    buf: [u8; 108],
    len: usize,
}

impl LineBuf {
    const fn new() -> Self {
        Self { buf: [0; 108], len: 0 }
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let avail = self.buf.len().saturating_sub(self.len);
        let take = bytes.len().min(avail);
        self.buf[self.len..self.len + take].copy_from_slice(&bytes[..take]);
        self.len += take;
        Ok(())
    }
}

fn push_fmt(push: &mut impl FnMut(&[u8]), args: fmt::Arguments<'_>) {
    let mut lb = LineBuf::new();
    let _ = lb.write_fmt(args);
    push(lb.as_bytes());
}

fn fill_bar(buf: &mut [u8; 22], pct: usize) -> &str {
    buf[0] = b'[';
    let filled = (pct.min(100) * 20) / 100;
    for i in 0..20 {
        buf[1 + i] = if i < filled { b'#' } else { b'.' };
    }
    buf[21] = b']';
    core::str::from_utf8(buf).unwrap_or("[....................]")
}

pub fn run_fetch(mut push: impl FnMut(&[u8])) {
    let ov = system_stats::overview().unwrap_or_default();
    let mem = mesh_stats::memory().unwrap_or_default();
    let kern = system_stats::kernel().unwrap_or_default();
    let up_s = sex_pdx::get_ticks() / 1000;
    let mem_used_mib = (mem.lent_pages * 4096) >> 20;
    let mem_cap_mib = (mem.limit_pages * 4096) >> 20;
    let heap_kib = kern.heap_used >> 10;

    push(b"   .d888b.  .d888b.     root@sexos-noir");
    push_fmt(&mut push, format_args!("  d8888888bd8888888b    OS:     SexOS 0.1.0-noir x86_64"));
    push_fmt(&mut push, format_args!("  88888\"\"88888\"\"88888   Kernel: SexOS Microkernel (PKU + PDX)"));
    push_fmt(&mut push, format_args!("   \"88\"  \"888\"  \"88\"    Uptime: {}m {}s", up_s / 60, up_s % 60));
    push_fmt(&mut push, format_args!("    `888888888888'     CPUs:   {} cores ({} free pgs)", ov.cores.max(1), ov.free_pages));
    push_fmt(&mut push, format_args!("      `88888888'       Memory: {} / {} MiB (heap {} KiB)", mem_used_mib, mem_cap_mib, heap_kib));
    push(b"         \"\"\"\"           Term:   Boot Console -> 'run spindle' for P10k+TUI");
}

pub fn run_bars(mut push: impl FnMut(&[u8])) {
    let kern = system_stats::kernel().unwrap_or_default();
    let mem = mesh_stats::memory().unwrap_or_default();
    let mem_pct = if mem.limit_pages > 0 { ((mem.lent_pages * 100) / mem.limit_pages) as usize } else { 24 };
    let heap_pct = if kern.heap_mapped > 0 { ((kern.heap_used * 100) / kern.heap_mapped) as usize } else { 42 };
    let mut b1 = [0u8; 22];
    let mut b2 = [0u8; 22];
    let mut b3 = [0u8; 22];
    let mut b4 = [0u8; 22];

    push(b"+-- SexOS Subsystem Status ----------------------------+");
    push_fmt(&mut push, format_args!("| Kernel PKU / PDX   {} {:>3}% |", fill_bar(&mut b1, 100), 100));
    push_fmt(&mut push, format_args!("| Kernel Heap        {} {:>3}% |", fill_bar(&mut b2, heap_pct), heap_pct));
    push_fmt(&mut push, format_args!("| Program Pool       {} {:>3}% |", fill_bar(&mut b3, mem_pct), mem_pct));
    push_fmt(&mut push, format_args!("| Silk Display Bus   {} {:>3}% |", fill_bar(&mut b4, 100), 100));
    push(b"+-- Type 'run spindle' for animated TrueColor bars ----+");
}

pub fn run_btop(mut push: impl FnMut(&[u8])) {
    let ov = system_stats::overview().unwrap_or_default();
    let mem = mesh_stats::memory().unwrap_or_default();
    let kern = system_stats::kernel().unwrap_or_default();
    let mem_pct = if mem.limit_pages > 0 { ((mem.lent_pages * 100) / mem.limit_pages) as usize } else { 0 };
    let heap_pct = if kern.heap_mapped > 0 { ((kern.heap_used * 100) / kern.heap_mapped) as usize } else { 0 };
    let mut b1 = [0u8; 22];
    let mut b2 = [0u8; 22];

    push_fmt(&mut push, format_args!("+-- SexOS btop Snapshot ({} cores, {} free frames) ------+", ov.cores.max(1), kern.free_frames));
    push_fmt(&mut push, format_args!("| Prog Memory {} {:>3}% ({}/{} pgs) |", fill_bar(&mut b1, mem_pct), mem_pct, mem.lent_pages, mem.limit_pages));
    push_fmt(&mut push, format_args!("| Kernel Heap {} {:>3}% ({}/{} KiB) |", fill_bar(&mut b2, heap_pct), heap_pct, kern.heap_used >> 10, kern.heap_mapped >> 10));
    push(b"+-- Active Domains ------------------------------------+");
    let mut after = 0u32;
    for _ in 0..8 {
        let Some(pd) = system_stats::next(after) else { break };
        after = pd;
        let raw = system_stats::name(pd);
        let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let name = core::str::from_utf8(&raw[..len]).unwrap_or("domain");
        let pages = system_stats::domain(pd).map(|d| d.pages).unwrap_or(0);
        if pages == system_stats::UNKNOWN {
            push_fmt(&mut push, format_args!("| PD {:<3}  {:<16}  service                  |", pd, name));
        } else {
            push_fmt(&mut push, format_args!("| PD {:<3}  {:<16}  {:>6} KiB               |", pd, name, pages * 4));
        }
    }
    push(b"+-- Type 'run spindle' then 'btop' for live dashboard -+");
}
