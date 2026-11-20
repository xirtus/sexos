// SPDX-License-Identifier: GPL-3.0-only
//! The About page's lines: what this computer is, from the kernel's numbers
//! (`sex_pdx::system_stats` and `mesh_stats`, syscall 87). A number the kernel did
//! not give is left out, never guessed. Host-tested (`scripts/settings_host_gate.sh`).

/// What the kernel told this run; None where it did not answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub cores: Option<u64>,
    /// Pages program runs may hold in all, hold now, and the machine's free frames (4 KiB each).
    pub program_limit: Option<u64>,
    pub program_lent: Option<u64>,
    pub free_frames: Option<u64>,
    /// Bytes the kernel's own heap uses now.
    pub kernel_heap: Option<u64>,
    /// Domains (services and program runs) that ran or have a name.
    pub domains: u32,
    /// Seconds since start, from the tick count (ms).
    pub uptime_s: u64,
}

struct Out<'a> { buf: &'a mut [u8], at: usize }
impl Out<'_> {
    fn put(&mut self, s: &str) { for b in s.bytes() { if self.at < self.buf.len() { self.buf[self.at] = b; self.at += 1; } } }
    fn num(&mut self, mut v: u64) {
        let mut d = [0u8; 20];
        let mut i = d.len();
        loop { i -= 1; d[i] = b'0' + (v % 10) as u8; v /= 10; if v == 0 { break; } }
        self.put(core::str::from_utf8(&d[i..]).unwrap_or("?"));
    }
    /// Pages of 4 KiB as MiB.
    fn mib(&mut self, pages: u64) { self.num(pages / 256); self.put(" MiB"); }
}

/// The lines, each starting with two spaces; returns their length.
pub fn render(f: &Facts, version: &str, out: &mut [u8]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    w.put("  SexOS, a single-address-space microkernel system. Settings (Tatami) ");
    w.put(version);
    w.put("\n");
    if let Some(n) = f.cores { w.put("  Processor cores: "); w.num(n); w.put("\n"); }
    if let Some(p) = f.program_limit {
        w.put("  Memory for programs: "); w.mib(p);
        if let Some(l) = f.program_lent { w.put(", "); w.mib(l); w.put(" in use"); }
        w.put("\n");
    }
    if let Some(p) = f.free_frames { w.put("  Free memory: "); w.mib(p); w.put("\n"); }
    if let Some(b) = f.kernel_heap { w.put("  Kernel heap: "); w.num(b / 1024); w.put(" KiB\n"); }
    w.put("  Running services and programs: "); w.num(f.domains as u64); w.put("\n");
    w.put("  Up for "); w.num(f.uptime_s / 3600); w.put(" h "); w.num(f.uptime_s / 60 % 60); w.put(" min\n");
    w.at
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leaves_out_what_the_kernel_did_not_say() {
        let mut buf = [0u8; 1024];
        let f = Facts { cores: Some(4), program_limit: Some(256 * 1024), program_lent: Some(2560), free_frames: None, kernel_heap: Some(3 << 20), domains: 31, uptime_s: 3725 };
        let n = render(&f, "0.1.0", &mut buf);
        let t = core::str::from_utf8(&buf[..n]).unwrap();
        assert!(t.contains("Processor cores: 4\n"), "{t}");
        assert!(t.contains("Memory for programs: 1024 MiB, 10 MiB in use\n"), "{t}");
        assert!(!t.contains("Free memory"), "{t}");
        assert!(t.contains("Kernel heap: 3072 KiB\n") && t.contains("services and programs: 31\n") && t.contains("Up for 1 h 2 min\n"), "{t}");
        let n = render(&Facts::default(), "0.1.0", &mut buf[..40]);
        assert_eq!(n, 40, "a small buffer is cut, never overrun");
    }
}
