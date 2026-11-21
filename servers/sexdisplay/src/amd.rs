// SPDX-License-Identifier: GPL-3.0-only
//! AMD Radeon display engine (DCN): first boot receipt (slice A2).
//!
//! BOOT leases Display the first `AMD_REGISTERS` bytes of the Radeon's register
//! BAR. This reads what firmware (UEFI GOP) lit, without changing it: the chip,
//! every lit OTG's timing and the HUBP surface feeding it, where VRAM or the
//! APU's carve-out sits in the display's address space, and on an APU the panel's
//! DisplayPort capabilities and EDID over AUX channel 0 (the eDP panel on every
//! Ryzen laptop board in the plan). The receipt is what a Ryzen laptop's first
//! boot reads to plan takeover (A3); the register logic is `sex-gpu-amd`,
//! host-tested against a simulated DCN.

use sex_gpu_amd::{aux, readout, Mmio};
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

/// Print the receipt. Returns false when no known AMD display engine was leased.
pub fn report() -> bool {
    let Ok(id) = sex_pdx::device_identity(df::AMD_SLOT) else { return false };
    let Some(asic) = sex_gpu_amd::identify(id.vendor as u16, id.product as u16) else {
        sex_pdx::serial_record!("[display.amd] device={:04x} known=0 picture=firmware", id.product);
        return false;
    };
    let Ok(view) = sex_pdx::memory_region_full_view(df::AMD_SLOT, m::QUERY_VIEW | m::READ | m::WRITE) else {
        sex_pdx::serial_record!("[display.amd] device={:04x} view=0", id.product);
        return false;
    };
    let mut bar = Bar { va: view.virtual_address, len: view.length };
    sex_pdx::serial_record!("[display.amd.chip] device={:04x} name={} dcn={}.{}.{} apu={} otgs={} pipes={} auxes={} confirmed={}",
        asic.device, asic.name, asic.dcn.0, asic.dcn.1, asic.dcn.2, asic.apu as u8, asic.otgs, asic.pipes, asic.auxes, asic.confirmed as u8);
    let fb = readout::read_aperture(&mut bar, &asic);
    let mut lit = 0;
    readout::for_each_head(&mut bar, &asic, |h| {
        lit += 1;
        let o = h.otg;
        let (hh, vv) = (o.h().unwrap_or([0; 4]), o.v().unwrap_or([0; 4]));
        sex_pdx::serial_record!("[display.amd.otg] otg={} h={},{},{},{} v={},{},{},{} flags={:#x} frame={}",
            o.otg, hh[0], hh[1], hh[2], hh[3], vv[0], vv[1], vv[2], vv[3], o.flags, o.frame_count);
        match h.hubp {
            Some(p) => sex_pdx::serial_record!("[display.amd.surface] otg={} pipe={} format={:?} linear={} linear_xrgb={} stride={} at={},{} size={}x{} address={:#x} vram_offset={:#x}",
                o.otg, p.pipe, p.format, p.linear as u8, p.linear_xrgb() as u8, p.stride_bytes(), p.x, p.y, p.width, p.height, p.address,
                fb.vram_offset(p.address).unwrap_or(u64::MAX)),
            None => sex_pdx::serial_record!("[display.amd.surface] otg={} pipe=none", o.otg),
        }
    });
    sex_pdx::serial_record!("[display.amd.aperture] base={:#x} top={:#x} offset={:#x}", fb.base, fb.top, fb.offset);
    if asic.apu {
        match aux::caps(&mut bar, 0) {
            Ok(c) => sex_pdx::serial_record!("[display.amd.dpcd] aux=0 rev={:#x} max_link_khz={} lanes={} psr={:#x} edp_rev={:#x}",
                c.rev, c.max_link_khz, c.max_lanes, c.psr, c.edp_rev),
            Err(e) => sex_pdx::serial_record!("[display.amd.dpcd] aux=0 error={:?}", e),
        }
        let mut edid = [0u8; 256];
        match aux::read_edid(&mut bar, 0, &mut edid) {
            Ok(len) => {
                let (w, h) = sex_pdx::display_boot::edid_size_mm(&edid[..len]).unwrap_or((0, 0));
                let id = u16::from_be_bytes([edid[8], edid[9]]);
                let l = |s: u16| (b'A' - 1 + (s & 0x1f) as u8) as char;
                sex_pdx::serial_record!("[display.amd.edid] aux=0 bytes={} vendor={}{}{} product={:#06x} size_mm={}x{} dtd={:02x?}",
                    len, l(id >> 10), l(id >> 5), l(id), u16::from_le_bytes([edid[10], edid[11]]), w, h, &edid[54..72]);
            }
            Err(e) => sex_pdx::serial_record!("[display.amd.edid] aux=0 error={:?}", e),
        }
    }
    sex_pdx::serial_record!("[display.amd] device={:04x} otgs_lit={} registers={} writes={}",
        id.product, lit, view.length, if asic.apu { "aux_only" } else { "none" });
    true
}
