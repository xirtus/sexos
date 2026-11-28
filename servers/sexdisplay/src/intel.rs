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

/// Width, height and refresh (millihertz) of one EDID detailed timing.
fn dtd(d: &[u8]) -> Option<(u32, u32, u32)> {
    let clock = u16::from_le_bytes([d[0], d[1]]) as u64 * 10_000;
    if clock == 0 { return None; }
    let w = d[2] as u32 | ((d[4] >> 4) as u32) << 8;
    let hb = d[3] as u32 | ((d[4] & 0xf) as u32) << 8;
    let h = d[5] as u32 | ((d[7] >> 4) as u32) << 8;
    let vb = d[6] as u32 | ((d[7] & 0xf) as u32) << 8;
    let total = (w + hb) as u64 * (h + vb) as u64;
    if total == 0 { return None; }
    Some((w, h, (clock * 1000 / total) as u32))
}

/// The refresh the pipe really runs at: frames counted over a quarter second
/// of timer ticks (1 ms), starting on a frame edge. Tells 60 from 165 Hz on
/// the first x17 boot without trusting firmware's tables.
fn measure_refresh(bar: &mut Bar, p: u32) -> u32 {
    let frame = |b: &mut Bar| b.read(regs::pipe(p, regs::FRMCOUNT));
    let start = frame(bar);
    let mut spins = 0u32;
    while frame(bar) == start { spins += 1; if spins > 2_000_000 { return 0; } }
    let (f0, t0) = (frame(bar), sex_pdx::get_ticks());
    let _ = sex_pdx::sys_wait_self_ticks(250);
    let (f1, t1) = (frame(bar), sex_pdx::get_ticks());
    let ms = t1.saturating_sub(t0);
    if ms == 0 { 0 } else { (f1.wrapping_sub(f0) as u64 * 1_000_000 / ms) as u32 }
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
        let refresh_mhz = measure_refresh(&mut bar, p);
        sex_pdx::serial_record!("[display.intel.pipe] pipe={} port={} mode={} bpc={} h={},{},{},{} v={},{},{},{} src={}x{} plane={} linear_xrgb={} stride={} size={}x{} surf={:#x} psr={} psr2={} frame={} refresh_mhz={}",
            (b'A' + p as u8) as char, regs::port_name(port), mode_name(r.mode_select), r.bpc,
            r.h[0], r.h[1], r.h[2], r.h[3], r.v[0], r.v[1], r.v[2], r.v[3], r.src_w, r.src_h,
            r.plane.enabled as u8, r.plane.linear_xrgb() as u8, r.plane.stride_bytes, r.plane.width, r.plane.height, r.plane.surf,
            r.psr as u8, r.psr2 as u8, f0, refresh_mhz);
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
                // Every detailed timing the panel lists: high-refresh panels put
                // their 120/144/165/240/360 Hz mode here, beside a 60 Hz one.
                for (i, d) in edid[54..126].chunks(18).enumerate() {
                    if let Some((w, h, mhz)) = dtd(d) {
                        sex_pdx::serial_record!("[display.intel.edid.mode] port={} index={} width={} height={} refresh_mhz={} clock_khz={}",
                            regs::port_name(port), i, w, h, mhz, u16::from_le_bytes([d[0], d[1]]) as u32 * 10);
                    }
                }
            }
            Err(e) => sex_pdx::serial_record!("[display.intel.edid] port={} error={:?}", regs::port_name(port), e),
        }
    }
    sex_pdx::serial_record!("[display.intel] device={:04x} pipes_lit={} edp={} registers={} writes=aux_only",
        id.product, lit, edp.is_some() as u8, view.length);
    true
}

// ---- Takeover: our own buffers, flipped at vblank (slice L3 on hardware) ----

use sex_gpu_intel::takeover::{Chain, Cursor};
use sex_pdx::display_frames::IntelScanout;

/// Register reads a single wait may spend before sleeping a tick.
const LATCH_READS: u32 = 64;
/// Ticks (1 ms) a flip may take to latch: a panel leaving PSR needs a few
/// frames; a dead pipe costs this once, then Display falls back to firmware.
const LATCH_TICKS: u32 = 200;
/// The takeover's own wait spins (once, at boot): roughly the same span.
const TAKE_READS: u32 = 400_000;

pub struct Intel {
    bar: Bar,
    chain: Chain,
    cursor: Option<Cursor>,
    cursor_scale: usize,
    va: u64,
    layout: IntelScanout,
    width: usize,
    height: usize,
    /// Rows the back buffer lacks: what the last flip changed on the front.
    behind: (usize, usize),
}

/// Write back cache lines so the display engine, which does not snoop the
/// CPU caches, reads what was drawn.
fn flush(start: u64, bytes: u64) {
    let mut line = start & !63;
    while line < start + bytes {
        #[cfg(target_arch = "x86_64")]
        unsafe { core::arch::asm!("clflush [{}]", in(reg) line, options(nostack, preserves_flags)); }
        line += 64;
    }
    #[cfg(target_arch = "x86_64")]
    unsafe { core::arch::asm!("mfence", options(nostack, preserves_flags)); }
}

/// Repoint the firmware's pipe at Display's own two buffers. The boot picture
/// is copied into both first, so the switch shows no change on the panel.
pub fn take_over(width: u32, height: u32, boot_fb: u64, boot_pitch_px: u32) -> Result<Intel, &'static str> {
    let id = sex_pdx::device_identity(df::INTEL_SLOT).map_err(|_| "no_lease")?;
    if !sex_gpu_intel::supported(id.vendor, id.product) { return Err("not_supported"); }
    let regs = sex_pdx::memory_region_full_view(df::INTEL_SLOT, m::QUERY_VIEW | m::READ | m::WRITE).map_err(|_| "no_registers")?;
    let info = sex_pdx::memory_region_info(df::INTEL_SCANOUT_SLOT).map_err(|_| "no_scanout")?;
    if info.region_id != df::INTEL_SCANOUT_REGION_ID { return Err("scanout_region"); }
    let so = sex_pdx::memory_region_full_view(df::INTEL_SCANOUT_SLOT, info.rights).map_err(|_| "no_scanout_view")?;
    let layout = IntelScanout::new(width as u64, height as u64).ok_or("geometry")?;
    if so.length != layout.total || so.dma_address.is_none() { return Err("scanout_size"); }
    let (w, h, stride) = (width as usize, height as usize, layout.stride as usize / 4);
    for buffer in 0..2u64 {
        let base = so.virtual_address + buffer * layout.span;
        for y in 0..h {
            unsafe {
                core::ptr::copy_nonoverlapping((boot_fb + (y * boot_pitch_px as usize * 4) as u64) as *const u32,
                    (base + (y * stride * 4) as u64) as *mut u32, w);
            }
        }
        flush(base, layout.stride * height as u64);
    }
    unsafe { core::ptr::write_bytes((so.virtual_address + layout.cursor) as *mut u8, 0, df::INTEL_CURSOR_BYTES as usize); }
    let mut bar = Bar { va: regs.virtual_address, len: regs.length };
    let buffers = [layout.ggtt(0) as u32, layout.ggtt(1) as u32];
    let chain = Chain::take(&mut bar, width, height, buffers, layout.stride as u32, TAKE_READS).map_err(|r| r.name())?;
    let cursor = Cursor::new(chain.pipe, 64, layout.ggtt(2) as u32);
    sex_pdx::serial_record!("[display.intel.takeover] takeover=1 pipe={} width={} height={} stride={} buffers={:#x},{:#x} firmware_surf={:#x} cursor_plane={} source=boot_copy modeset=0",
        (b'A' + chain.pipe as u8) as char, width, height, layout.stride, buffers[0], buffers[1], chain.saved.surf, cursor.is_some() as u8);
    Ok(Intel { bar, chain, cursor, cursor_scale: 0, va: so.virtual_address, layout, width: w, height: h, behind: (0, 0) })
}

impl Intel {
    fn buffer(&self, index: usize) -> u64 { self.va + index as u64 * self.layout.span }
    /// Pixels per row of the scanout buffers.
    pub fn pitch(&self) -> usize { self.layout.stride as usize / 4 }
    pub fn front_address(&self) -> u64 { self.buffer(self.chain.front) }

    /// The buffer the next publication writes, brought up to date with the
    /// front where the last flip changed it. Waits (bounded) for that flip to
    /// latch, which also holds the desktop to one frame per refresh.
    pub fn back(&mut self) -> Result<u64, &'static str> {
        // Sleep a tick at a time instead of spinning while the engine scans.
        let mut back = Err("flip_never_latched");
        for _ in 0..LATCH_TICKS {
            if let Ok(b) = self.chain.back(&mut self.bar, LATCH_READS) { back = Ok(b); break; }
            let _ = sex_pdx::sys_wait_self_ticks(1);
        }
        let back = back?;
        let (first, end) = self.behind;
        if end > first {
            let row = self.layout.stride;
            let (from, to) = (self.buffer(1 - back) + first as u64 * row, self.buffer(back) + first as u64 * row);
            unsafe { core::ptr::copy_nonoverlapping(from as *const u8, to as *mut u8, ((end - first) as u64 * row) as usize); }
            flush(to, (end - first) as u64 * row);
            self.behind = (0, 0);
        }
        Ok(self.buffer(back))
    }

    /// Rows `first..end` of the back buffer are drawn: make them visible to
    /// the engine and flip at the next vblank.
    pub fn present(&mut self, first: usize, end: usize) -> Result<(), &'static str> {
        let back = 1 - self.chain.front;
        let end = end.min(self.height);
        if end > first {
            flush(self.buffer(back) + first as u64 * self.layout.stride, (end - first) as u64 * self.layout.stride);
        }
        self.chain.flip(&mut self.bar).map_err(|_| "flip")?;
        self.behind = (first, end);
        let n = self.chain.flips;
        if n <= 4 || n % 256 == 0 {
            sex_pdx::serial_record!("[display.intel.flip] sequence={} rows={}..{} buffer={} vblank_waits={} frame={}",
                n, first, end, back, self.chain.waits, sex_gpu_intel::plane::frame_count(&mut self.bar, self.chain.pipe));
        }
        Ok(())
    }

    /// Hardware cursor: draw the arrow once per scale into the cursor image,
    /// then every move is two register writes and no pixels.
    pub fn cursor(&mut self, x: usize, y: usize, scale: usize, bitmap: &[u8; 16], color: u32) -> bool {
        let Some(cur) = self.cursor.as_mut() else { return false };
        if scale == 0 || scale * 16 > 64 { return false; }
        if scale != self.cursor_scale {
            let image = (self.va + self.layout.cursor) as *mut u32;
            for row in 0..64 {
                for col in 0..64 {
                    let on = row < 16 * scale && col < 8 * scale && bitmap[row / scale] & (0x80 >> (col / scale)) != 0;
                    unsafe { image.add(row * 64 + col).write_volatile(if on { 0xff00_0000 | color } else { 0 }); }
                }
            }
            flush(self.va + self.layout.cursor, 64 * 64 * 4);
            self.cursor_scale = scale;
        }
        let (x, y) = (x.min(self.width) as i32, y.min(self.height) as i32);
        let first = !cur.shown;
        if cur.show(&mut self.bar, x, y).is_err() { return false; }
        if first { sex_pdx::serial_record!("[display.intel.cursor] plane=1 size=64 scale={} x={} y={}", scale, x, y); }
        true
    }

    pub fn hide_cursor(&mut self) {
        if let Some(cur) = self.cursor.as_mut() { cur.hide(&mut self.bar); }
    }

    /// Give the panel back to the firmware framebuffer.
    pub fn release(&mut self, reason: &str) {
        self.chain.release(&mut self.bar);
        sex_pdx::serial_record!("[display.intel.fallback] reason={} flips={} firmware_surf={:#x}", reason, self.chain.flips, self.chain.saved.surf);
    }
}

/// Backlight as it was before the machine slept.
/// One brightness step on the eDP panel's PWM: 10 % (5 % below 10 %), never below 5 %.
/// None when no Intel display engine is leased or firmware left the PWM off.
pub fn backlight_step(up: bool) -> Option<u32> {
    let id = sex_pdx::device_identity(df::INTEL_SLOT).ok()?;
    if !sex_gpu_intel::supported(id.vendor, id.product) { return None; }
    let view = sex_pdx::memory_region_full_view(df::INTEL_SLOT, m::QUERY_VIEW | m::READ | m::WRITE).ok()?;
    let mut bar = Bar { va: view.virtual_address, len: view.length };
    let now = backlight::read(&mut bar);
    let cur = now.percent();
    let step = if cur <= 10 && !up || cur < 10 && up { 5 } else { 10 };
    let next = if up { (cur + step).min(100) } else { cur.saturating_sub(step).max(5) };
    let set = backlight::set_percent(&mut bar, next);
    sex_pdx::serial_record!("[display.intel.backlight.step] up={} from={} to={} enabled={} period={} ok={}",
        up as u8, cur, next, now.enabled as u8, now.period, set.is_some() as u8);
    set.map(|b| b.percent())
}

static mut BEFORE_SLEEP: Option<backlight::Backlight> = None;

/// The machine is going to sleep (true) or woke: the eDP panel's backlight goes off
/// and the panel to DPCD D3 (eDP 1.4 SET_POWER, 0x600), and back. False when no
/// Intel display engine was leased.
pub fn device_sleep(asleep: bool) -> bool {
    let Ok(id) = sex_pdx::device_identity(df::INTEL_SLOT) else { return false };
    if !sex_gpu_intel::supported(id.vendor, id.product) { return false; }
    let Ok(view) = sex_pdx::memory_region_full_view(df::INTEL_SLOT, m::QUERY_VIEW | m::READ | m::WRITE) else { return false };
    let mut bar = Bar { va: view.virtual_address, len: view.length };
    let edp = (0..regs::PIPES).map(|p| readout::read_pipe(&mut bar, p))
        .any(|r| r.enabled && r.port == Some(0) && r.mode_select == regs::FUNC_MODE_DP_SST);
    let saved = unsafe { &mut *core::ptr::addr_of_mut!(BEFORE_SLEEP) };
    let mut lit = saved.is_some();
    let panel = if asleep {
        *saved = backlight::sleep(&mut bar);
        lit = saved.is_some();
        edp && aux::dpcd_write(&mut bar, 0, 0x600, &[2]).is_ok()
    } else {
        // The sink needs up to 1 ms to wake; the AUX layer retries a deferring sink.
        let ok = edp && aux::dpcd_write(&mut bar, 0, 0x600, &[1]).is_ok();
        if let Some(b) = saved.take() { backlight::wake(&mut bar, b); }
        ok
    };
    sex_pdx::serial_record!("[display.intel.sleep] asleep={} edp={} panel_power={} backlight={}",
        asleep as u8, edp as u8, panel as u8, lit as u8);
    true
}
