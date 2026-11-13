// SPDX-License-Identifier: GPL-3.0-only
//! Intel Gen12 (Tiger Lake) display engine: first boot receipt (slice L3).
//!
//! BOOT leases Display a view of the GPU's display registers. This reads what
//! firmware (UEFI GOP) lit, without changing it: every pipe's transcoder, timings
//! and primary plane, PSR state, which ports have a monitor, the backlight, and
//! the panel's EDID and DisplayPort capabilities over the AUX channel. The
//! receipt is what the x17 R1 bare-metal test reads to plan modesetting; the
//! register logic is `sex-gpu-intel`, host-tested against simulated registers.

use sex_gpu_intel::regs::{self, PORTS};
use sex_gpu_intel::{aux, backlight, hotplug, readout, Mmio};
use sex_pdx::{display_frames as df, memory_region as m};

struct Bar { va: u64, len: u64 }
impl Mmio for Bar {
    fn read(&mut self, off: u32) -> u32 {
        if off as u64 + 4 > self.len { return 0; }
        unsafe { ((self.va + off as u64) as *const u32).read_volatile() }
    }
    fn write(&mut self, off: u32, v: u32) {
        if off as u64 + 4 > self.len { return; }
        unsafe { ((self.va + off as u64) as *mut u32).write_volatile(v) }
    }
}

fn mode_name(m: u32) -> &'static str {
    match m { regs::FUNC_MODE_HDMI => "hdmi", regs::FUNC_MODE_DVI => "dvi", regs::FUNC_MODE_DP_SST => "dp", regs::FUNC_MODE_DP_MST => "dp_mst", _ => "?" }
}

/// Print the receipt. Returns false when no Intel display engine was leased.
pub fn report() -> bool {
    let Ok(id) = sex_pdx::device_identity(df::INTEL_SLOT) else { return false };
    if !sex_gpu_intel::supported(id.vendor, id.product) { return false; }
    let Ok(view) = sex_pdx::memory_region_full_view(df::INTEL_SLOT, m::QUERY_VIEW | m::READ | m::WRITE) else {
        sex_pdx::serial_record!("[display.intel] device={:04x} view=0", id.product);
        return false;
    };
    let mut bar = Bar { va: view.virtual_address, len: view.length };
    let mut edp: Option<u32> = None;
    let mut lit = 0;
    for p in 0..regs::PIPES {
        let r = readout::read_pipe(&mut bar, p);
        if !r.enabled { continue; }
        lit += 1;
        let port = r.port.unwrap_or(u32::MAX);
        if port == 0 && r.mode_select == regs::FUNC_MODE_DP_SST { edp = Some(0); }
        let f0 = bar.read(regs::pipe(p, regs::FRMCOUNT));
        sex_pdx::serial_record!("[display.intel.pipe] pipe={} port={} mode={} bpc={} h={},{},{},{} v={},{},{},{} src={}x{} plane={} linear_xrgb={} stride={} size={}x{} surf={:#x} psr={} psr2={} frame={}",
            (b'A' + p as u8) as char, regs::port_name(port), mode_name(r.mode_select), r.bpc,
            r.h[0], r.h[1], r.h[2], r.h[3], r.v[0], r.v[1], r.v[2], r.v[3], r.src_w, r.src_h,
            r.plane.enabled as u8, r.plane.linear_xrgb() as u8, r.plane.stride_bytes, r.plane.width, r.plane.height, r.plane.surf,
            r.psr as u8, r.psr2 as u8, f0);
    }
    let live = hotplug::scan(&mut bar);
    let mut names = [0u8; 64];
    let mut n = 0;
    for (i, &p) in PORTS.iter().enumerate() {
        if live & (1 << i) == 0 { continue; }
        for &b in regs::port_name(p).as_bytes().iter().chain(b",") { if n < names.len() { names[n] = b; n += 1; } }
    }
    sex_pdx::serial_record!("[display.intel.ports] live={:#x} names={}", live, core::str::from_utf8(&names[..n.saturating_sub(1)]).unwrap_or(""));
    let b = backlight::read(&mut bar);
    sex_pdx::serial_record!("[display.intel.backlight] enabled={} period={} duty={} percent={}", b.enabled as u8, b.period, b.duty, b.percent());
    if let Some(port) = edp {
        match aux::caps(&mut bar, port, true) {
            Ok(c) => sex_pdx::serial_record!("[display.intel.dpcd] port={} rev={:#x} max_link_khz={} lanes={} psr={:#x} edp_rev={:#x}",
                regs::port_name(port), c.rev, c.max_link_khz, c.max_lanes, c.psr, c.edp_rev),
            Err(e) => sex_pdx::serial_record!("[display.intel.dpcd] port={} error={:?}", regs::port_name(port), e),
        }
        let mut edid = [0u8; 256];
        match aux::read_edid(&mut bar, port, &mut edid) {
            Ok(len) => {
                let (w, h) = sex_pdx::display_boot::edid_size_mm(&edid[..len]).unwrap_or((0, 0));
                let id = u16::from_be_bytes([edid[8], edid[9]]);
                let l = |s: u16| (b'A' - 1 + (s & 0x1f) as u8) as char;
                sex_pdx::serial_record!("[display.intel.edid] port={} bytes={} vendor={}{}{} product={:#06x} size_mm={}x{} dtd={:02x?}",
                    regs::port_name(port), len, l(id >> 10), l(id >> 5), l(id), u16::from_le_bytes([edid[10], edid[11]]), w, h, &edid[54..72]);
            }
            Err(e) => sex_pdx::serial_record!("[display.intel.edid] port={} error={:?}", regs::port_name(port), e),
        }
    }
    sex_pdx::serial_record!("[display.intel] device={:04x} pipes_lit={} edp={} registers={} writes=aux_only",
        id.product, lit, edp.is_some() as u8, view.length);
    true
}
