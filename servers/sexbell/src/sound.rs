// SPDX-License-Identifier: GPL-3.0-only
//! Sound: the Bell service drives a virtio sound card, when BOOT leased it one.
//! The device protocol is the `virtio-drivers` crate (MIT); this file supplies its
//! two platform halves from SexOS capabilities: a transport over the one BAR view
//! BOOT leased, and DMA memory from the service's own DMA pool. Notes play from a
//! short queue, fed a period at a time while the service waits for messages.
use core::ptr::NonNull;
use sex_pdx::{memory_region as m, pci_command as c, virtio as v};
use virtio_drivers::device::sound::{PcmFeatures, PcmFormat, PcmRate, VirtIOSound};
use virtio_drivers::transport::{DeviceStatus, DeviceType, InterruptStatus, Transport};
use virtio_drivers::{BufferDirection, Error, Hal, PhysAddr};

// ── Heap ──────────────────────────────────────────────────────────────
// The driver keeps its tables and in-flight periods in `alloc` collections.
const HEAP_BYTES: usize = 192 * 1024;
#[repr(align(4096))]
struct HeapBytes(#[allow(dead_code)] [u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

// ── DMA pool ──────────────────────────────────────────────────────────
// Byte granules over the BOOT pool. Queues take whole aligned pages; shared
// buffers bounce through granules and are copied back for device writes.
const GRANULE: usize = 256;
const GRANULES: usize = v::DMA_BYTES as usize / GRANULE;
const PAGE_GRANULES: usize = 4096 / GRANULE;
struct Pool { va: usize, pa: u64, used: [u64; GRANULES / 64] }
static mut POOL: Pool = Pool { va: 0, pa: 0, used: [0; GRANULES / 64] };

fn pool() -> &'static mut Pool { unsafe { &mut *core::ptr::addr_of_mut!(POOL) } }
impl Pool {
    fn taken(&self, i: usize) -> bool { self.used[i / 64] & 1 << (i % 64) != 0 }
    fn mark(&mut self, start: usize, count: usize, on: bool) {
        for i in start..start + count {
            if on { self.used[i / 64] |= 1 << (i % 64) } else { self.used[i / 64] &= !(1 << (i % 64)) }
        }
    }
    fn take(&mut self, bytes: usize, align: usize) -> Option<usize> {
        let count = bytes.div_ceil(GRANULE).max(1);
        let mut start = 0;
        while start + count <= GRANULES {
            match (start..start + count).find(|&i| self.taken(i)) {
                Some(busy) => start = (busy + 1).next_multiple_of(align),
                None => { self.mark(start, count, true); return Some(start); }
            }
        }
        None
    }
    fn give(&mut self, pa: u64, bytes: usize) {
        let start = (pa - self.pa) as usize / GRANULE;
        self.mark(start, bytes.div_ceil(GRANULE).max(1), false);
    }
    fn at(&self, granule: usize) -> (u64, usize) { (self.pa + (granule * GRANULE) as u64, self.va + granule * GRANULE) }
    fn virt(&self, pa: u64) -> usize {
        assert!(pa >= self.pa && pa < self.pa + v::DMA_BYTES, "SOUND_DMA_OUTSIDE_POOL");
        self.va + (pa - self.pa) as usize
    }
}

pub struct PoolHal;
unsafe impl Hal for PoolHal {
    fn dma_alloc(pages: usize, _: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let granule = pool().take(pages * 4096, PAGE_GRANULES).expect("SOUND_DMA_EXHAUSTED");
        let (pa, va) = pool().at(granule);
        unsafe { core::ptr::write_bytes(va as *mut u8, 0, pages * 4096) };
        (pa, NonNull::new(va as *mut u8).unwrap())
    }
    unsafe fn dma_dealloc(pa: PhysAddr, _: NonNull<u8>, pages: usize) -> i32 { pool().give(pa, pages * 4096); 0 }
    unsafe fn mmio_phys_to_virt(_: PhysAddr, _: usize) -> NonNull<u8> {
        // Only the PCI transport asks; this service's transport never names an address.
        panic!("SOUND_NO_PHYSICAL_MMIO")
    }
    unsafe fn share(buffer: NonNull<[u8]>, direction: BufferDirection) -> PhysAddr {
        let bytes = buffer.len();
        let granule = pool().take(bytes, 1).expect("SOUND_DMA_EXHAUSTED");
        let (pa, va) = pool().at(granule);
        if direction != BufferDirection::DeviceToDriver {
            unsafe { core::ptr::copy_nonoverlapping(buffer.as_ptr() as *const u8, va as *mut u8, bytes) };
        }
        pa
    }
    unsafe fn unshare(pa: PhysAddr, buffer: NonNull<[u8]>, direction: BufferDirection) {
        let bytes = buffer.len();
        if direction != BufferDirection::DriverToDevice {
            unsafe { core::ptr::copy_nonoverlapping(pool().virt(pa) as *const u8, buffer.as_ptr() as *mut u8, bytes) };
        }
        pool().give(pa, bytes);
    }
}

// ── Transport over the leased BAR view ────────────────────────────────
// virtio_pci_common_cfg field offsets (virtio 1.x, 4.1.4.3).
const DEVICE_FEATURE_SELECT: u32 = 0x00;
const DEVICE_FEATURE: u32 = 0x04;
const DRIVER_FEATURE_SELECT: u32 = 0x08;
const DRIVER_FEATURE: u32 = 0x0c;
const DEVICE_STATUS: u32 = 0x14;
const CONFIG_GENERATION: u32 = 0x15;
const QUEUE_SELECT: u32 = 0x16;
const QUEUE_SIZE: u32 = 0x18;
const QUEUE_ENABLE: u32 = 0x1c;
const QUEUE_NOTIFY_OFF: u32 = 0x1e;
const QUEUE_DESC: u32 = 0x20;
const QUEUE_DRIVER: u32 = 0x28;
const QUEUE_DEVICE: u32 = 0x30;

pub struct LeaseTransport { base: usize, layout: v::Layout, kind: DeviceType }
impl LeaseTransport {
    fn at<T>(&self, window: v::Window, offset: u32) -> *mut T {
        assert!(offset as usize + size_of::<T>() <= window.length as usize, "SOUND_MMIO_OUTSIDE_WINDOW");
        (self.base + window.offset as usize + offset as usize) as *mut T
    }
    fn read<T: Copy>(&self, offset: u32) -> T { unsafe { self.at::<T>(self.layout.common, offset).read_volatile() } }
    fn write<T>(&mut self, offset: u32, value: T) { unsafe { self.at::<T>(self.layout.common, offset).write_volatile(value) } }
}
impl Transport for LeaseTransport {
    fn device_type(&self) -> DeviceType { self.kind }
    fn read_device_features(&mut self) -> u64 {
        self.write(DEVICE_FEATURE_SELECT, 0u32);
        let low: u32 = self.read(DEVICE_FEATURE);
        self.write(DEVICE_FEATURE_SELECT, 1u32);
        let high: u32 = self.read(DEVICE_FEATURE);
        low as u64 | (high as u64) << 32
    }
    fn write_driver_features(&mut self, features: u64) {
        self.write(DRIVER_FEATURE_SELECT, 0u32);
        self.write(DRIVER_FEATURE, features as u32);
        self.write(DRIVER_FEATURE_SELECT, 1u32);
        self.write(DRIVER_FEATURE, (features >> 32) as u32);
    }
    fn max_queue_size(&mut self, queue: u16) -> u32 {
        self.write(QUEUE_SELECT, queue);
        self.read::<u16>(QUEUE_SIZE) as u32
    }
    fn notify(&mut self, queue: u16) {
        self.write(QUEUE_SELECT, queue);
        let off: u16 = self.read(QUEUE_NOTIFY_OFF);
        let at = off as u32 * self.layout.notify_multiplier;
        unsafe { self.at::<u16>(self.layout.notify, at).write_volatile(queue) }
    }
    fn get_status(&self) -> DeviceStatus { DeviceStatus::from_bits_truncate(self.read::<u8>(DEVICE_STATUS) as u32) }
    fn set_status(&mut self, status: DeviceStatus) { self.write(DEVICE_STATUS, status.bits() as u8) }
    fn set_guest_page_size(&mut self, _: u32) {}
    fn requires_legacy_layout(&self) -> bool { false }
    fn queue_set(&mut self, queue: u16, size: u32, descriptors: PhysAddr, driver: PhysAddr, device: PhysAddr) {
        self.write(QUEUE_SELECT, queue);
        self.write(QUEUE_SIZE, size as u16);
        self.write(QUEUE_DESC, descriptors);
        self.write(QUEUE_DRIVER, driver);
        self.write(QUEUE_DEVICE, device);
        self.write(QUEUE_ENABLE, 1u16);
    }
    fn queue_unset(&mut self, _: u16) {}
    fn queue_used(&mut self, queue: u16) -> bool {
        self.write(QUEUE_SELECT, queue);
        self.read::<u16>(QUEUE_ENABLE) == 1
    }
    fn ack_interrupt(&mut self) -> InterruptStatus {
        InterruptStatus::from_bits_retain(unsafe { self.at::<u8>(self.layout.isr, 0).read_volatile() } as u32)
    }
    fn read_config_generation(&self) -> u32 { self.read::<u8>(CONFIG_GENERATION) as u32 }
    fn read_config_space<T: zerocopy::FromBytes + zerocopy::IntoBytes>(&self, offset: usize) -> Result<T, Error> {
        if offset % align_of::<T>() != 0 || align_of::<T>() > 4 { return Err(Error::InvalidParam) }
        if offset + size_of::<T>() > self.layout.device.length as usize { return Err(Error::ConfigSpaceTooSmall) }
        Ok(unsafe { self.at::<T>(self.layout.device, offset as u32).read_volatile() })
    }
    fn write_config_space<T: zerocopy::IntoBytes + zerocopy::Immutable>(&mut self, offset: usize, value: T) -> Result<(), Error> {
        if offset % align_of::<T>() != 0 || align_of::<T>() > 4 { return Err(Error::InvalidParam) }
        if offset + size_of::<T>() > self.layout.device.length as usize { return Err(Error::ConfigSpaceTooSmall) }
        unsafe { self.at::<T>(self.layout.device, offset as u32).write_volatile(value) };
        Ok(())
    }
}

// ── Notes ─────────────────────────────────────────────────────────────
pub const RATE: u32 = 48_000;
const CHANNELS: u8 = 2;
const FRAME_BYTES: usize = 4; // S16 stereo
/// 8 KiB = 2048 frames ≈ 43 ms. Two descriptors each, so 16 fit the 32-entry ring.
const PERIOD_BYTES: usize = 8192;
const IN_FLIGHT: usize = 12;
const NOTES: usize = 16;
pub const MAX_MS: u64 = 5000;
const PERIOD_FRAMES: usize = PERIOD_BYTES / FRAME_BYTES;
/// One stream per program row: the sound ring BOOT shares with that row.
const STREAMS: usize = 16;
use sex_pdx::sound_ring as ring;

#[derive(Clone, Copy)]
struct Stream { ring: ring::Ring, active: bool, pd: u32 }

#[derive(Clone, Copy)]
struct Note { hz: u32, frames: u32, volume: u16 }

struct Player {
    card: VirtIOSound<PoolHal, LeaseTransport>,
    stream: u32,
    started: bool,
    notes: [Note; NOTES],
    head: usize,
    len: usize,
    done: u32,     // frames of the current note already rendered
    phase: u32,    // 0..2^32 is one cycle
    tokens: [u16; IN_FLIGHT],
    first: usize,
    flying: usize,
    period: [u8; PERIOD_BYTES],
    played: u64,
    streams: [Option<Stream>; STREAMS],
    /// Frames each stream gave each in-flight period, credited to PLAYED when the
    /// card hands that period back.
    took: [[u16; STREAMS]; IN_FLIGHT],
}
/// The period being mixed, kept off the service's small stack.
static mut MIX: [[i32; 2]; PERIOD_FRAMES] = [[0; 2]; PERIOD_FRAMES];
fn mix() -> &'static mut [[i32; 2]; PERIOD_FRAMES] { unsafe { &mut *core::ptr::addr_of_mut!(MIX) } }
static mut PLAYER: Option<Player> = None;
fn player() -> Option<&'static mut Player> { unsafe { (*core::ptr::addr_of_mut!(PLAYER)).as_mut() } }

/// sin(2πx) for x in [0,1), Bhaskara I's approximation (error < 0.2%); no libm here.
fn sine(phase: u32) -> f32 {
    let half = phase >= 1 << 31;
    let x = (phase & 0x7fff_ffff) as f32 / 2147483648.0 * 180.0; // degrees in [0,180)
    let p = x * (180.0 - x);
    let s = 4.0 * p / (40500.0 - p);
    if half { -s } else { s }
}

impl Player {
    fn streaming(&self) -> bool { self.streams.iter().flatten().any(|st| st.active) }

    /// Mixes the next period of notes and streams into `period`, noting how many
    /// frames each stream gave in `took[slot]`. False when there is nothing to play.
    fn render(&mut self, slot: usize) -> bool {
        if self.len == 0 && !self.streaming() { return false; }
        let mix = mix();
        *mix = [[0; 2]; PERIOD_FRAMES];
        for frame in 0..PERIOD_FRAMES {
            if self.len == 0 { break; }
            let note = self.notes[self.head];
            // 5 ms ramps at both ends keep notes from clicking.
            let ramp = (RATE / 200).min(note.frames / 2).max(1);
            let edge = self.done.min(note.frames - 1 - self.done.min(note.frames - 1)).min(ramp);
            let gain = note.volume as f32 / 100.0 * 0.5 * 32767.0 * edge as f32 / ramp as f32;
            let s = if note.hz == 0 { 0.0 } else { sine(self.phase) * gain };
            self.phase = self.phase.wrapping_add(((note.hz as u64) << 32).wrapping_div(RATE as u64) as u32);
            self.done += 1;
            if self.done >= note.frames {
                self.head = (self.head + 1) % NOTES; self.len -= 1; self.done = 0; self.phase = 0;
            }
            mix[frame] = [s as i32, s as i32];
        }
        for (i, entry) in self.streams.iter_mut().enumerate() {
            self.took[slot][i] = 0;
            let Some(st) = entry.as_mut() else { continue };
            if !st.active { continue; }
            let r = st.ring;
            let (written, taken) = (r.load(ring::WRITTEN), r.load(ring::TAKEN));
            // A program owns only its own ring: a counter it bent is clamped, not trusted.
            let have = written.wrapping_sub(taken).min(ring::FRAMES) as usize;
            let n = have.min(PERIOD_FRAMES);
            // A muted program's sound is taken and counted as played, so its clock
            // runs on, but not heard.
            let volume = if crate::sound_muted(st.pd) { 0 } else { r.load(ring::VOLUME).min(100) as i32 };
            let frames = r.frames();
            for f in 0..n {
                let at = ((taken + f as u64) % ring::FRAMES) as usize;
                let [l, rr] = unsafe { core::ptr::read_volatile(frames.add(at)) };
                mix[f][0] += l as i32 * volume / 100;
                mix[f][1] += rr as i32 * volume / 100;
            }
            r.store(ring::TAKEN, taken + n as u64);
            self.took[slot][i] = n as u16;
            if n == 0 && r.load(ring::OPEN) == 0 {
                st.active = false;
                sex_pdx::serial_record!("[bell.sound.stream.closed] row={} taken={}", i, taken);
            }
        }
        for (frame, [l, r]) in mix.iter().enumerate() {
            let (l, r) = ((*l).clamp(-32768, 32767) as i16, (*r).clamp(-32768, 32767) as i16);
            let at = frame * FRAME_BYTES;
            self.period[at..at + 2].copy_from_slice(&l.to_le_bytes());
            self.period[at + 2..at + 4].copy_from_slice(&r.to_le_bytes());
        }
        true
    }

    fn pump(&mut self) {
        // Reap finished periods in order; the device returns them as it plays.
        while self.flying > 0 {
            if self.card.pcm_xfer_ok(self.tokens[self.first]).is_err() { break; }
            for (i, st) in self.streams.iter().enumerate() {
                let n = self.took[self.first][i];
                if let (Some(st), true) = (st, n > 0) { st.ring.store(ring::PLAYED, st.ring.load(ring::PLAYED) + n as u64); }
            }
            self.first = (self.first + 1) % IN_FLIGHT; self.flying -= 1; self.played += 1;
        }
        while self.flying < IN_FLIGHT && self.render((self.first + self.flying) % IN_FLIGHT) {
            let period = self.period;
            match self.card.pcm_xfer_nb(self.stream, &period) {
                Ok(token) => { self.tokens[(self.first + self.flying) % IN_FLIGHT] = token; self.flying += 1; }
                Err(e) => { sex_pdx::serial_record!("[bell.sound.xfer.error] {:?}", e); self.len = 0; break; }
            }
        }
        if !self.started && self.flying > 0 {
            self.started = self.card.pcm_start(self.stream).is_ok();
            sex_pdx::serial_record!("[bell.sound.started] stream={} ok={}", self.stream, self.started as u8);
        }
    }
}

/// Take the BOOT lease, if there is one, and ready the card's first output stream.
pub fn init() {
    match open() {
        Ok(p) => {
            sex_pdx::serial_record!("[bell.sound.ready] stream={} rate={} channels={} period_bytes={}", p.stream, RATE, CHANNELS, PERIOD_BYTES);
            unsafe { *core::ptr::addr_of_mut!(PLAYER) = Some(p) };
        }
        Err(reason) => sex_pdx::serial_record!("[bell.sound.absent] reason={}", reason),
    }
}

fn open() -> Result<Player, &'static str> {
    let info = sex_pdx::memory_region_info(v::SLOT_SOUND).map_err(|_| "no_lease")?;
    if info.rights != m::QUERY_VIEW | m::READ | m::WRITE || info.peer_pd.is_some() { return Err("lease_rights"); }
    let view = sex_pdx::memory_region_full_view(v::SLOT_SOUND, m::QUERY_VIEW | m::READ | m::WRITE).map_err(|_| "no_view")?;
    let identity = sex_pdx::device_identity(v::SLOT_SOUND).map_err(|_| "no_identity")?;
    if (identity.vendor, identity.product) != (v::VENDOR, v::SOUND) { return Err("not_sound"); }
    let layout = v::layout(v::SLOT_SOUND).map_err(|_| "no_layout")?;
    if !layout.valid(view.length) || view.dma_address.is_some() { return Err("layout"); }

    let dma_info = sex_pdx::memory_region_info(sex_pdx::SLOT_DMA_REGION).map_err(|_| "no_dma")?;
    if dma_info.region_id != v::DMA_REGION_ID || dma_info.length != v::DMA_BYTES { return Err("dma_region"); }
    let dma = sex_pdx::memory_region_full_view(sex_pdx::SLOT_DMA_REGION, dma_info.rights).map_err(|_| "no_dma_view")?;
    let pa = dma.dma_address.ok_or("no_dma_address")?;
    unsafe {
        *core::ptr::addr_of_mut!(POOL) = Pool { va: dma.virtual_address as usize, pa, used: [0; GRANULES / 64] };
        ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES);
    }
    sex_pdx::device_command(v::SLOT_SOUND, c::NIC_BITS as u64, 0).map_err(|_| "command")?;

    let transport = LeaseTransport { base: view.virtual_address as usize, layout, kind: DeviceType::Sound };
    let mut card = VirtIOSound::<PoolHal, _>::new(transport).map_err(|_| "init")?;
    let stream = *card.output_streams().map_err(|_| "streams")?.first().ok_or("no_output")?;
    card.pcm_set_params(stream, (PERIOD_BYTES * IN_FLIGHT) as u32, PERIOD_BYTES as u32,
        PcmFeatures::empty(), CHANNELS, PcmFormat::S16, PcmRate::Rate48000).map_err(|_| "params")?;
    card.pcm_prepare(stream).map_err(|_| "prepare")?;
    Ok(Player { card, stream, started: false, notes: [Note { hz: 0, frames: 0, volume: 0 }; NOTES],
        head: 0, len: 0, done: 0, phase: 0, tokens: [0; IN_FLIGHT], first: 0, flying: 0,
        period: [0; PERIOD_BYTES], played: 0, streams: [None; STREAMS], took: [[0; STREAMS]; IN_FLIGHT] })
}

pub fn present() -> bool { player().is_some() }

/// Queue a note: `hz` 0 is a rest. False when there's no card or the queue is full.
pub fn tone(hz: u64, ms: u64, volume: u64) -> bool {
    let Some(p) = player() else { return false };
    if hz > 20_000 || ms == 0 || ms > MAX_MS || volume > 100 || p.len == NOTES { return false; }
    let frames = (RATE as u64 * ms / 1000).max(1) as u32;
    p.notes[(p.head + p.len) % NOTES] = Note { hz: hz as u32, frames, volume: volume as u16 };
    p.len += 1;
    p.pump();
    true
}

/// Two rising notes: something needs the person now.
pub fn chime() -> bool { tone(1319, 90, 30) && tone(1760, 140, 30) }

/// Notes still to play, streams playing, or periods still with the device.
pub fn busy() -> bool { player().is_some_and(|p| p.len > 0 || p.flying > 0 || p.streaming()) }

/// A program row says its ring has sound: play it from where Bell last took.
/// False when there's no card or the caller has no ring.
pub fn stream(caller: u32) -> bool {
    let Some(p) = player() else { return false };
    let Some(k) = ring::index_of(caller).filter(|k| *k < STREAMS) else { return false };
    if p.streams[k].is_none() {
        match sex_pdx::sound_ring_bell(k) {
            Ok(r) => p.streams[k] = Some(Stream { ring: r, active: false, pd: caller }),
            Err(e) => { sex_pdx::serial_record!("[bell.sound.stream.denied] row={} err={}", k, e); return false; }
        }
    }
    let st = p.streams[k].as_mut().unwrap();
    st.pd = caller;
    if !st.active {
        st.active = true;
        sex_pdx::serial_record!("[bell.sound.stream.open] row={} caller={} written={} taken={}",
            k, caller, st.ring.load(ring::WRITTEN), st.ring.load(ring::TAKEN));
    }
    p.pump();
    true
}

/// A program ended, closed or crashed: stop its stream, whatever its ring says.
pub fn retire(pd: u32) -> bool {
    let Some(p) = player() else { return false };
    let Some(st) = p.streams.iter_mut().flatten().find(|st| st.active && st.pd == pd) else { return false };
    st.active = false;
    sex_pdx::serial_record!("[bell.sound.stream.retired] pd={} taken={}", pd, st.ring.load(ring::TAKEN));
    true
}

pub fn pump() {
    if let Some(p) = player() {
        let before = p.played;
        p.pump();
        if p.len == 0 && p.flying == 0 && p.played != before {
            sex_pdx::serial_record!("[bell.sound.drained] periods={}", p.played);
        }
    }
}
