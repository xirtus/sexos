// SPDX-License-Identifier: GPL-3.0-only
//! sexgpu-nv: SexOS's NVIDIA GPU service, a ring-3 server (no nouveau, no Linux bridge).
//!
//! Stage N4b of docs/handoff/NVIDIA_GSP_BRINGUP_2026_11_19.md: boot NVIDIA's GSP-RM
//! firmware on the GPU's RISC-V management core, with the logic in `crates/sex-gpu-nv`.
//! BOOT starts this service only from the "SexOS with NVIDIA GPU" boot entry; it then
//! leaves the GPU powered and hands over (`sex_pdx::nvidia_gpu`):
//! - the GPU's lease: all 16 MiB of BAR0, the memory and bus-master command bits, and
//!   the BAR addresses GSP-RM must be told;
//! - a 64 MiB DMA pool, inside the service's IOMMU domain;
//! - the firmware pack (`nvidia-gsp.pack`, the four pinned 535.113.01 files).
//!
//! Boot log, one line per step so a run on the x17 says how far it got:
//! `[sexgpu-nv.start]`, `[nvidia.gpu] vendor= device= bar0= bar1= bar3= bus=`,
//! `[nvidia.firmware] file= bytes= sha256=ok`, `[nvidia.gsp] step=<name> ...` for each of
//! chip, vbios, layout, prepare, fwsec_frts, riscv, booter, init_done,
//! then `[nvidia.gsp.ready] ...` or `[nvidia.gsp.failed] step=<name> reason=...`.
//! After that (stage N4c) it asks GSP-RM what it found and opens its own RM objects:
//! `[nvidia.rm] name="..." vram_mib= ...`, `[nvidia.rm.device]`, `[nvidia.rm.display] heads= ...`,
//! `[nvidia.rm.ready]`, or `[nvidia.rm.failed] step= reason=`.
//! Without a GPU it logs `[sexgpu-nv.idle] gpu=none`. Afterwards it sleeps in
//! `sys_wait_self` and answers messages with "not yet" (display is stage N5).
#![no_std]
#![no_main]

use sex_gpu_nv::boot::{self, Config, Firmware, Memory, Report, Sizes, Step};
use sex_gpu_nv::pack::{self, Pack};
use sex_gpu_nv::rm::{self, Session};
use sex_gpu_nv::{chip::Chip, rpc::SystemInfo, Clock, Mmio};
use sex_pdx::memory_region::{DMA_ADDRESS, QUERY_VIEW, READ, WRITE};
use sex_pdx::nvidia_gpu as n;
use sex_pdx::pci_command as c;
use sex_pdx::serial_record as log;
use core::sync::atomic::{AtomicU64, Ordering};

const HEAP_BYTES: usize = 64 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct HeapBytes([u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

/// The VBIOS copy (the PROM window is 1 MiB) and the RPC scratch (two messages).
static mut VBIOS: [u8; 1 << 20] = [0; 1 << 20];
static mut SCRATCH: [u8; 2 * sex_gpu_nv::msgq::MAX_MSG] = [0; 2 * sex_gpu_nv::msgq::MAX_MSG];

/// GSP-RM 535 is waited for this long after Booter starts it (nouveau allows seconds).
const INIT_POLLS: u32 = 500_000; // x 10 us = 5 s
/// `GSP_SET_SYSTEM_INFO.maxUserVa`: the top of a 47-bit user address space, as nouveau.
const MAX_USER_VA: u64 = 0x7fff_ffff_f000;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log!("[sexgpu-nv.panic] {}", info);
    park()
}

/// Sleep for good: only messages wake us, and each is answered "not yet".
fn park() -> ! {
    loop {
        while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
            if m.type_id <= 1 { continue; }
            let _ = sex_pdx::pdx_reply(m.caller_pd, u64::MAX);
        }
        sex_pdx::sys_wait_self();
    }
}

// ── Time: the TSC, measured against the kernel's millisecond tick ───────────

static TSC_PER_US: AtomicU64 = AtomicU64::new(3000);
fn rdtsc() -> u64 { unsafe { core::arch::x86_64::_rdtsc() } }
fn calibrate() {
    let start = sex_pdx::get_ticks();
    let mut t = start;
    while t == start { core::hint::spin_loop(); t = sex_pdx::get_ticks(); }
    let (t0, c0) = (t, rdtsc());
    while sex_pdx::get_ticks() < t0 + 20 { core::hint::spin_loop(); }
    TSC_PER_US.store(((rdtsc() - c0) / 20_000).clamp(100, 10_000), Ordering::Relaxed);
}
struct Tsc;
impl Clock for Tsc {
    fn delay_us(&mut self, us: u32) {
        let end = rdtsc().saturating_add(us as u64 * TSC_PER_US.load(Ordering::Relaxed));
        while rdtsc() < end { core::hint::spin_loop(); }
    }
}

// ── Registers: the leased BAR0 view ─────────────────────────────────────────

struct Bar0 { base: u64, length: u64 }
impl Mmio for Bar0 {
    fn read(&mut self, offset: u32) -> u32 {
        if offset as u64 + 4 > self.length || offset % 4 != 0 { return u32::MAX; }
        unsafe { core::ptr::read_volatile((self.base + offset as u64) as *const u32) }
    }
    fn write(&mut self, offset: u32, value: u32) {
        if offset as u64 + 4 > self.length || offset % 4 != 0 { return; }
        unsafe { core::ptr::write_volatile((self.base + offset as u64) as *mut u32, value) }
    }
}

fn region(slot: u64, rights: u8) -> Option<sex_pdx::memory_region::RegionView> {
    let info = sex_pdx::memory_region_info(slot).ok()?;
    if info.rights & rights != rights { return None; }
    sex_pdx::memory_region_full_view(slot, rights).ok().filter(|v| v.virtual_address != 0 && v.length != 0)
}

fn hex8(b: &[u8; 32]) -> u32 { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) }

/// A pinned file of the pack whose bytes really hash to the pin.
fn firmware<'a>(p: &Pack<'a>, name: &str) -> Option<&'a [u8]> {
    use sha2::Digest;
    let f = match p.pinned(name) {
        Ok(f) => f,
        Err(e) => { log!("[nvidia.gsp.failed] step=firmware file={} reason={:?} hint=run_scripts/fetch_nvidia_gsp_firmware.sh_before_building", name, e); return None; }
    };
    let digest: [u8; 32] = sha2::Sha256::digest(f.data).into();
    if digest != f.sha256 {
        log!("[nvidia.gsp.failed] step=firmware file={} reason=sha256_mismatch have={:08x} want={:08x}", name, hex8(&digest), hex8(&f.sha256));
        return None;
    }
    log!("[nvidia.firmware] file={} bytes={} sha256=ok", name, f.data.len());
    Some(f.data)
}

fn step_line(s: Step, r: &Report) {
    match s {
        Step::Chip => log!("[nvidia.gsp] step=chip boot0={:#010x} chipset={:#x} vram_mib={}", r.boot0, r.chipset, r.fb_size >> 20),
        Step::Vbios => log!("[nvidia.gsp] step=vbios bytes={} version={:02x}.{:02x}.{:02x}.{:02x}.{:02x}", r.vbios_len,
            r.vbios_version[0], r.vbios_version[1], r.vbios_version[2], r.vbios_version[3], r.vbios_version[4]),
        Step::Layout => log!("[nvidia.gsp] step=layout frts={:#x} wpr2={:#x}+{:#x} heap_mib={} elf={:#x}", r.layout.frts.addr,
            r.layout.wpr2.addr, r.layout.wpr2.size, r.layout.heap.size >> 20, r.layout.elf.addr),
        Step::Fwsec => log!("[nvidia.gsp] step=fwsec_frts signature={} wpr2_lo={:#x} wpr2_hi={:#x}", r.fwsec_sig, r.wpr2_lo, r.wpr2_hi),
        Step::Booter => log!("[nvidia.gsp] step=booter signature={} app_version={:#x}", r.booter_sig, r.app_version),
        Step::InitDone => log!("[nvidia.gsp] step=init_done events={} sequencer_runs={}", r.events, r.sequencer_runs),
        other => log!("[nvidia.gsp] step={}", other.name()),
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe { ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES); }
    log!("[sexgpu-nv.start] firmware={}", boot::FW_VERSION);
    let Ok(id) = sex_pdx::device_identity(n::SLOT) else { log!("[sexgpu-nv.idle] gpu=none"); park() };
    let bars = n::bars(n::SLOT);
    let Some(view) = region(n::SLOT, QUERY_VIEW | READ | WRITE).filter(|v| v.dma_address.is_none() && v.length >= n::REGISTER_BYTES) else {
        log!("[nvidia.gsp.failed] step=lease reason=register_view_missing"); park()
    };
    let Ok(bars) = bars else { log!("[nvidia.gsp.failed] step=lease reason=bars_unreadable"); park() };
    log!("[nvidia.gpu] vendor={:#06x} device={:#06x} bar0={:#x} bar1={:#x} bar3={:#x} bus={:02x}:{:02x}.{} view_bytes={}",
        id.vendor, id.product, bars.bar0, bars.bar1, bars.bar3, bars.bus, bars.devfn >> 3, bars.devfn & 7, view.length);
    let mut bar0 = Bar0 { base: view.virtual_address, length: view.length };
    let boot0 = bar0.read(sex_gpu_nv::regs::PMC_BOOT_0);
    if boot0 == u32::MAX {
        log!("[nvidia.gsp.failed] step=chip reason=reads_all_ones hint=gpu_powered_off_or_link_down"); park()
    }
    let Some(chip) = Chip::decode(boot0).filter(|c| c.gsp_boot_supported()) else {
        log!("[nvidia.gsp.failed] step=chip reason=unsupported boot0={:#010x}", boot0); park()
    };

    // Firmware: the pack BOOT loaded, each file checked against its pin.
    let Some(pack_view) = region(n::SLOT_FIRMWARE, QUERY_VIEW | READ) else {
        log!("[nvidia.gsp.failed] step=firmware reason=no_pack module={}", n::FIRMWARE_MODULE); park()
    };
    let bytes: &'static [u8] = unsafe { core::slice::from_raw_parts(pack_view.virtual_address as *const u8, pack_view.length as usize) };
    let p = match Pack::parse(bytes) {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => { log!("[nvidia.gsp.failed] step=firmware reason=pack_empty hint=run_scripts/fetch_nvidia_gsp_firmware.sh_before_building"); park() }
        Err(e) => { log!("[nvidia.gsp.failed] step=firmware reason={:?}", e); park() }
    };
    let (Some(booter_load), Some(bootloader), Some(gsp)) =
        (firmware(&p, pack::BOOTER_LOAD), firmware(&p, pack::BOOTLOADER), firmware(&p, pack::GSP_RM)) else { park() };
    let f = Firmware { booter_load, bootloader, gsp };
    let sizes = match Sizes::for_firmware(&chip, &f) {
        Ok(s) => s,
        Err(e) => { log!("[nvidia.gsp.failed] step=firmware reason={:?}", e); park() }
    };

    // Memory: the DMA pool, cut into the boot's buffers.
    let pool = region(n::SLOT_DMA, QUERY_VIEW | READ | WRITE | DMA_ADDRESS)
        .filter(|v| v.length == n::DMA_BYTES).and_then(|v| v.dma_address.map(|bus| (v, bus)));
    let Some((pool, bus)) = pool else { log!("[nvidia.gsp.failed] step=dma reason=no_pool"); park() };
    let pool_bytes: &'static mut [u8] = unsafe { core::slice::from_raw_parts_mut(pool.virtual_address as *mut u8, pool.length as usize) };
    let scratch: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };
    let mut mem = match Memory::carve(pool_bytes, bus, &sizes, scratch) {
        Ok(m) => m,
        Err(e) => { log!("[nvidia.gsp.failed] step=dma reason={:?} pool={} need={}", e, pool.length, sizes.total()); park() }
    };
    log!("[nvidia.dma] pool_bytes={} need={} bus={:#x}", pool.length, sizes.total(), bus);

    // Memory decode and bus mastering: GSP-RM and the falcons DMA from the pool.
    match sex_pdx::device_command(n::SLOT, c::NIC_BITS as u64, 0) {
        Ok(r) if r.after & c::NIC_BITS == c::NIC_BITS => {}
        other => { log!("[nvidia.gsp.failed] step=command ok={}", other.is_ok() as u8); park() }
    }

    calibrate();
    let cfg = Config {
        system: SystemInfo { bar0: bars.bar0, bar1: bars.bar1, bar_inst: bars.bar3, bus: bars.bus, devfn: bars.devfn, max_user_va: MAX_USER_VA },
        init_polls: INIT_POLLS,
    };
    let vbios: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(VBIOS) };
    let started = sex_pdx::get_ticks();
    let report = match boot::boot(&mut bar0, &mut Tsc, &cfg, &f, vbios, &mut mem, &mut |s: Step, r: &Report| step_line(s, r)) {
        Ok(r) => {
            log!("[nvidia.gsp.ready] chipset={:#x} vram_mib={} wpr2_lo={:#x} wpr2_hi={:#x} events={} ms={}", r.chipset, r.fb_size >> 20,
                r.wpr2_lo, r.wpr2_hi, r.events, sex_pdx::get_ticks() - started);
            r
        }
        Err(e) => { log!("[nvidia.gsp.failed] step={} reason={:?} ms={}", e.step.name(), e.error, sex_pdx::get_ticks() - started); park() }
    };
    rm_bringup(&mut bar0, &mut mem, &report);
    park()
}

/// Stage N4c: ask GSP-RM what it found, then open our own client, device and subdevice
/// and read the display engine's shape (heads), which N5 builds on.
fn rm_bringup(bar0: &mut Bar0, mem: &mut Memory, report: &Report) {
    let mut s = Session::new(mem, report.cmdq_seq, report.app_version);
    let info = match s.static_info(bar0, &mut Tsc) {
        Ok(i) => i,
        Err(e) => { log!("[nvidia.rm.failed] step=static_info reason={:?}", e); return }
    };
    log!("[nvidia.rm] name=\"{}\" chip={} vram_mib={} usable_mib={} bus_bits={} ram_type={} l2_kib={} gpcs={} tpcs={} mobile={} vbios_valid={} subsystem={:04x}:{:04x}",
        info.name_str(), info.short_name_str(), info.fb_length >> 20, info.fb_usable >> 20, info.fb_bus_width, info.fb_ram_type,
        info.l2_cache_size >> 10, info.gpcs, info.tpcs, info.mobile as u8, info.vbios_valid as u8, info.vbios_sub_vendor, info.vbios_sub_device);
    if let Err(e) = s.open_device(bar0, &mut Tsc) {
        log!("[nvidia.rm.failed] step=device reason={:?}", e); return
    }
    log!("[nvidia.rm.device] client={:#x} device={:#x} subdevice={:#x}", rm::CLIENT, rm::DEVICE, rm::SUBDEVICE);
    match s.display_static_info(bar0, &mut Tsc, &info) {
        Ok(d) => log!("[nvidia.rm.display] heads={} windows={:#x} primary_vga={} internal_active={:#x} i2c_port={}",
            d.heads, d.window_mask, d.primary_vga as u8, d.internal_active_mask, d.i2c_port),
        Err(e) => log!("[nvidia.rm.failed] step=display reason={:?}", e),
    }
    log!("[nvidia.rm.ready] other_events={}", s.other_events);
}
