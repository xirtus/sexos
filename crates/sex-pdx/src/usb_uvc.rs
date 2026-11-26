// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! USB Video Class (UVC) webcam transport between SexUSB (xHCI transport) and SexUVC
//! (userspace UVC driver server): camera metadata, start/stop stream commands, and a
//! compact chunked frame payload ring in one 128 KiB shared region.
//!
//! BOOT starts SexUVC as domain [`SEXUVC_PD`] (Senses organ, key [`SEXUVC_KEY`]) and pairs
//! this region between SexUSB and SexUVC (`kernel/src/boot_memory.rs::init_usb_uvc`).
//! SexUSB also holds a notify route ([`NOTIFY_SLOT`]) to SexUVC and sends [`OP_LINK`]
//! when a physical UVC camera arrives or leaves, so an idle SexUVC sleeps without polling.

use core::sync::atomic::{AtomicU32, Ordering};

/// Shared memory region slot, paired in SexUSB and SexUVC.
pub const SLOT: u64 = 1010;
/// SexUSB's notify route capability slot to SexUVC.
pub const NOTIFY_SLOT: u64 = 1011;
/// Region ID for the SexUSB <-> SexUVC shared region.
pub const REGION_ID: u32 = 0x4407;
/// Total shared region size (128 KiB — compact on 512 MiB QEMU).
pub const BYTES: u64 = 128 * 1024;
/// SexUVC BOOT protection domain identity.
pub const SEXUVC_PD: u32 = 55;
/// Senses organ PKU key (shared with SexUSB, Input, Cufflinks, Saddle).
pub const SEXUVC_KEY: u8 = 4;
/// SexUSB -> SexUVC: camera link changed (`arg0 = 1` attached, `0` detached). SexUVC replies `0`.
pub const OP_LINK: u64 = 0x2CC;

/// Maximum advertised formats in the shared header.
pub const MAX_FORMATS: usize = 6;

/// Format kind tags in [`FormatMeta::kind`].
pub const FMT_OTHER: u8 = 0;
pub const FMT_MJPEG: u8 = 1;
pub const FMT_YUY2: u8 = 2;
pub const FMT_NV12: u8 = 3;

/// Stream control commands (`sexuvc` -> `sexusb`).
pub const CMD_NONE: u32 = 0;
pub const CMD_START: u32 = 1;
pub const CMD_STOP: u32 = 2;

/// Stream state reported by `sexusb`.
pub const STATE_STOPPED: u32 = 0;
pub const STATE_STREAMING: u32 = 1;
pub const STATE_ERROR: u32 = 2;

/// Ring geometry: 4 KiB header + 30 x 4 KiB chunk slots = 124 KiB <= 128 KiB.
pub const SLOTS: u32 = 30;
pub const SLOT_BYTES: usize = 4096;
pub const CHUNK_HEADER: usize = 8;
pub const MAX_CHUNK: usize = SLOT_BYTES - CHUNK_HEADER;

/// Chunk flags in each ring slot header.
pub const FLAG_SOF: u16 = 1 << 0;
pub const FLAG_EOF: u16 = 1 << 1;
pub const FLAG_ERR: u16 = 1 << 2;
pub const FLAG_FID: u16 = 1 << 3;

const MAGIC: u32 = 0x5556_4331; // "UVC1"
const HEADER: usize = 4096;
const RING_BASE: usize = HEADER;
const _: () = assert!(RING_BASE + SLOTS as usize * SLOT_BYTES <= BYTES as usize);

// Header 32-bit word byte offsets.
const W_MAGIC: usize = 0;
const W_LINK: usize = 4;
const W_IDS: usize = 8;
const W_GENERATION: usize = 12;
const W_IFACES: usize = 16;
const W_MAX_PAYLOAD: usize = 20;
const W_FORMAT_COUNT: usize = 24;
const W_CMD_SEQ: usize = 28;
const W_CMD_ACK: usize = 32;
const W_CMD_OP: usize = 36;
const W_CMD_PARAM0: usize = 40;
const W_CMD_PARAM1: usize = 44;
const W_STREAM_STATE: usize = 48;
const W_ACTIVE_ALT: usize = 52;
const W_RING_PROD: usize = 56;
const W_RING_CONS: usize = 60;
const W_FRAMES_DONE: usize = 64;
const W_FRAMES_DROPPED: usize = 68;
const W_PACKETS_IN: usize = 72;
const W_BYTES_IN: usize = 76;

const FORMATS_AT: usize = 128;
const FORMAT_RECORD: usize = 16;

/// One video format advertised by the attached USB camera.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatMeta {
    pub index: u8,
    pub kind: u8,
    pub frames: u8,
    pub max_w: u16,
    pub max_h: u16,
}

impl FormatMeta {
    pub fn kind_name(self) -> &'static str {
        match self.kind {
            FMT_MJPEG => "mjpeg",
            FMT_YUY2 => "yuy2",
            FMT_NV12 => "nv12",
            _ => "other",
        }
    }
}

/// Metadata for the currently attached physical USB UVC camera.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CameraInfo {
    pub vendor: u16,
    pub product: u16,
    pub generation: u32,
    pub control_iface: u8,
    pub stream_iface: u8,
    pub alt_count: u8,
    pub bulk: bool,
    pub max_payload: u32,
    pub format_count: u8,
    pub formats: [FormatMeta; MAX_FORMATS],
}

impl CameraInfo {
    pub fn formats_slice(&self) -> &[FormatMeta] {
        &self.formats[..(self.format_count as usize).min(MAX_FORMATS)]
    }

    /// Largest advertised frame dimensions across all formats.
    pub fn max_dimensions(&self) -> (u16, u16) {
        let mut best = (0u16, 0u16);
        for f in self.formats_slice() {
            if f.max_w as u32 * f.max_h as u32 > best.0 as u32 * best.1 as u32 {
                best = (f.max_w, f.max_h);
            }
        }
        best
    }
}

/// Start/stop stream command sent by `sexuvc` to `sexusb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamCommand {
    pub seq: u32,
    pub op: u32,
    pub format_index: u8,
    pub frame_index: u8,
    pub fps: u16,
    pub width: u16,
    pub height: u16,
}

/// Metadata for one popped frame payload chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkMeta {
    pub len: usize,
    pub flags: u16,
    pub frame_seq: u32,
}

/// View over the 128 KiB shared memory region. `base` must be [`BYTES`] long and 4-byte aligned.
#[derive(Clone, Copy)]
pub struct Shared {
    base: *mut u8,
}

impl Shared {
    /// # Safety
    /// `base` must point to a valid, 4-byte-aligned mapping of at least [`BYTES`] bytes.
    pub unsafe fn at(base: *mut u8) -> Self {
        Self { base }
    }

    fn word(&self, at: usize) -> &AtomicU32 {
        unsafe { &*(self.base.add(at) as *const AtomicU32) }
    }
    fn load(&self, at: usize) -> u32 {
        self.word(at).load(Ordering::Acquire)
    }
    fn store(&self, at: usize, v: u32) {
        self.word(at).store(v, Ordering::Release)
    }

    /// SexUSB: publish an attached camera (`Some`) or clear on disconnect (`None`).
    pub fn set_camera(&self, info: Option<&CameraInfo>) {
        self.store(W_LINK, 0);
        for w in [
            W_CMD_SEQ,
            W_CMD_ACK,
            W_CMD_OP,
            W_CMD_PARAM0,
            W_CMD_PARAM1,
            W_STREAM_STATE,
            W_ACTIVE_ALT,
            W_RING_PROD,
            W_RING_CONS,
            W_FRAMES_DONE,
            W_FRAMES_DROPPED,
            W_PACKETS_IN,
            W_BYTES_IN,
        ] {
            self.store(w, 0);
        }
        if let Some(c) = info {
            self.store(W_IDS, c.vendor as u32 | ((c.product as u32) << 16));
            let ifaces = c.control_iface as u32
                | ((c.stream_iface as u32) << 8)
                | ((c.alt_count as u32) << 16)
                | ((c.bulk as u32) << 24);
            self.store(W_IFACES, ifaces);
            self.store(W_MAX_PAYLOAD, c.max_payload);
            let n = (c.format_count as usize).min(MAX_FORMATS);
            self.store(W_FORMAT_COUNT, n as u32);
            for (k, f) in c.formats[..n].iter().enumerate() {
                let off = FORMATS_AT + k * FORMAT_RECORD;
                self.store(off, f.index as u32 | ((f.kind as u32) << 8) | ((f.frames as u32) << 16));
                self.store(off + 4, f.max_w as u32 | ((f.max_h as u32) << 16));
                self.store(off + 8, 0);
                self.store(off + 12, 0);
            }
            self.store(W_GENERATION, self.load(W_GENERATION).wrapping_add(1));
            self.store(W_MAGIC, MAGIC);
            self.store(W_LINK, 1);
        }
    }

    /// Either side: read the currently attached camera metadata, if any.
    pub fn camera(&self) -> Option<CameraInfo> {
        if self.load(W_MAGIC) != MAGIC || self.load(W_LINK) != 1 {
            return None;
        }
        let ids = self.load(W_IDS);
        let ifaces = self.load(W_IFACES);
        let n = (self.load(W_FORMAT_COUNT) as usize).min(MAX_FORMATS);
        let mut formats = [FormatMeta::default(); MAX_FORMATS];
        for (k, slot) in formats[..n].iter_mut().enumerate() {
            let off = FORMATS_AT + k * FORMAT_RECORD;
            let w0 = self.load(off);
            let w1 = self.load(off + 4);
            *slot = FormatMeta {
                index: w0 as u8,
                kind: (w0 >> 8) as u8,
                frames: (w0 >> 16) as u8,
                max_w: w1 as u16,
                max_h: (w1 >> 16) as u16,
            };
        }
        Some(CameraInfo {
            vendor: ids as u16,
            product: (ids >> 16) as u16,
            generation: self.load(W_GENERATION),
            control_iface: ifaces as u8,
            stream_iface: (ifaces >> 8) as u8,
            alt_count: (ifaces >> 16) as u8,
            bulk: ((ifaces >> 24) & 1) != 0,
            max_payload: self.load(W_MAX_PAYLOAD),
            format_count: n as u8,
            formats,
        })
    }

    /// SexUVC: request SexUSB to start streaming the selected format/frame.
    pub fn request_start(&self, format_index: u8, frame_index: u8, fps: u16, width: u16, height: u16) -> u32 {
        let p0 = format_index as u32 | ((frame_index as u32) << 8) | ((fps as u32) << 16);
        let p1 = width as u32 | ((height as u32) << 16);
        self.store(W_CMD_PARAM0, p0);
        self.store(W_CMD_PARAM1, p1);
        self.store(W_CMD_OP, CMD_START);
        let seq = self.load(W_CMD_SEQ).wrapping_add(1);
        self.store(W_CMD_SEQ, seq);
        seq
    }

    /// SexUVC: request SexUSB to stop streaming.
    pub fn request_stop(&self) -> u32 {
        self.store(W_CMD_OP, CMD_STOP);
        let seq = self.load(W_CMD_SEQ).wrapping_add(1);
        self.store(W_CMD_SEQ, seq);
        seq
    }

    /// SexUSB: check whether SexUVC has issued a new command.
    pub fn poll_command(&self) -> Option<StreamCommand> {
        if self.load(W_MAGIC) != MAGIC || self.load(W_LINK) != 1 {
            return None;
        }
        let seq = self.load(W_CMD_SEQ);
        if seq == self.load(W_CMD_ACK) {
            return None;
        }
        let op = self.load(W_CMD_OP);
        let p0 = self.load(W_CMD_PARAM0);
        let p1 = self.load(W_CMD_PARAM1);
        Some(StreamCommand {
            seq,
            op,
            format_index: p0 as u8,
            frame_index: (p0 >> 8) as u8,
            fps: (p0 >> 16) as u16,
            width: p1 as u16,
            height: (p1 >> 16) as u16,
        })
    }

    /// SexUSB: acknowledge a processed command and update stream state.
    pub fn ack_command(&self, seq: u32, state: u32, alt: u8, ep: u8, mps: u16) {
        self.store(W_STREAM_STATE, state);
        self.store(W_ACTIVE_ALT, alt as u32 | ((ep as u32) << 8) | ((mps as u32) << 16));
        self.store(W_CMD_ACK, seq);
    }

    pub fn stream_state(&self) -> u32 {
        self.load(W_STREAM_STATE)
    }

    pub fn active_alt(&self) -> (u8, u8, u16) {
        let w = self.load(W_ACTIVE_ALT);
        (w as u8, (w >> 8) as u8, (w >> 16) as u16)
    }

    fn slot_ptr(&self, index: u32) -> *mut u8 {
        unsafe { self.base.add(RING_BASE + (index % SLOTS) as usize * SLOT_BYTES) }
    }

    /// SexUSB: push one stripped frame payload chunk into the shared ring.
    pub fn push_chunk(&self, flags: u16, frame_seq: u32, payload: &[u8]) -> bool {
        if payload.len() > MAX_CHUNK {
            self.store(W_FRAMES_DROPPED, self.load(W_FRAMES_DROPPED).wrapping_add(1));
            return false;
        }
        let (p, c) = (self.load(W_RING_PROD), self.load(W_RING_CONS));
        if p.wrapping_sub(c) >= SLOTS {
            self.store(W_FRAMES_DROPPED, self.load(W_FRAMES_DROPPED).wrapping_add(1));
            return false;
        }
        let s = self.slot_ptr(p);
        let hdr0 = (payload.len() as u32) | ((flags as u32) << 16);
        unsafe {
            (s as *mut u32).write_volatile(hdr0);
            (s.add(4) as *mut u32).write_volatile(frame_seq);
            if !payload.is_empty() {
                core::ptr::copy_nonoverlapping(payload.as_ptr(), s.add(CHUNK_HEADER), payload.len());
            }
        }
        self.store(W_RING_PROD, p.wrapping_add(1));
        if (flags & FLAG_EOF) != 0 && (flags & FLAG_ERR) == 0 {
            self.store(W_FRAMES_DONE, self.load(W_FRAMES_DONE).wrapping_add(1));
        }
        if (flags & FLAG_ERR) != 0 {
            self.store(W_FRAMES_DROPPED, self.load(W_FRAMES_DROPPED).wrapping_add(1));
        }
        true
    }

    /// SexUVC: pop the next frame payload chunk into `out`.
    pub fn pop_chunk(&self, out: &mut [u8]) -> Option<ChunkMeta> {
        let (p, c) = (self.load(W_RING_PROD), self.load(W_RING_CONS));
        if p == c {
            return None;
        }
        let s = self.slot_ptr(c);
        let hdr0 = unsafe { (s as *const u32).read_volatile() };
        let frame_seq = unsafe { (s.add(4) as *const u32).read_volatile() };
        let len = (hdr0 as u16) as usize;
        let flags = (hdr0 >> 16) as u16;
        let n = len.min(MAX_CHUNK).min(out.len());
        if n > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(s.add(CHUNK_HEADER), out.as_mut_ptr(), n);
            }
        }
        self.store(W_RING_CONS, c.wrapping_add(1));
        Some(ChunkMeta { len: n, flags, frame_seq })
    }

    /// SexUSB: record raw packet/byte counters.
    pub fn add_packet_stats(&self, packets: u32, bytes: u32) {
        self.store(W_PACKETS_IN, self.load(W_PACKETS_IN).wrapping_add(packets));
        self.store(W_BYTES_IN, self.load(W_BYTES_IN).wrapping_add(bytes));
    }

    pub fn frames_done(&self) -> u32 {
        self.load(W_FRAMES_DONE)
    }

    pub fn frames_dropped(&self) -> u32 {
        self.load(W_FRAMES_DROPPED)
    }

    pub fn packets_in(&self) -> u32 {
        self.load(W_PACKETS_IN)
    }
}

/// SexUVC write-capable slot for the live webcam frame bank.
pub const FRAME_WRITER_SLOT: u64 = 1012;
/// Program-row read-only slot for the live webcam frame bank (`< 1024` for `program_tenancy`).
pub const FRAME_CLIENT_SLOT: u64 = 263;
/// Region ID for the SexUVC -> Program live webcam frame bank.
pub const FRAME_REGION_ID: u32 = 0x4408;
/// Total live frame bank size (512 KiB, one physical allocation shared read-only across program rows).
pub const FRAME_BYTES: u64 = 512 * 1024;
/// Frame bank header size (4 KiB).
pub const FRAME_HEADER: usize = 4096;
/// Number of ping-pong frame slots in the live frame bank.
pub const FRAME_SLOTS: usize = 2;
/// Maximum payload bytes per published frame (254 KiB; 4 KiB + 2 * 254 KiB == 512 KiB).
pub const FRAME_SLOT_BYTES: usize = 254 * 1024;
const _: () = assert!(FRAME_HEADER + FRAME_SLOTS * FRAME_SLOT_BYTES <= FRAME_BYTES as usize);

const FB_MAGIC_VAL: u32 = 0x5556_4346; // "UVCF"
const FB_MAGIC: usize = 0;
const FB_ACTIVE: usize = 4;
const FB_ACTIVE_SLOT: usize = 8;
const FB_SEQ: usize = 12;
const FB_WIDTH: usize = 16;
const FB_HEIGHT: usize = 20;
const FB_FORMAT: usize = 24;
const FB_LEN: usize = 28;
const FB_FPS: usize = 32;
const FB_VENDOR_PRODUCT: usize = 36;
const FB_DROPPED: usize = 40;

const FB_SLOT_META_BASE: usize = 64;
const FB_SLOT_META_STRIDE: usize = 16;

/// Metadata for a live frame published by `sexuvc` into [`FrameBank`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveFrameMeta {
    pub seq: u32,
    pub width: u16,
    pub height: u16,
    pub format: u8,
    pub fps: u16,
    pub len: usize,
    pub vendor: u16,
    pub product: u16,
    pub dropped: u32,
}

/// Lock-free double-buffered live frame bank: `sexuvc` writes into the inactive slot and
/// flips `FB_ACTIVE_SLOT`/`FB_SEQ`; any program row maps the region read-only and performs
/// tear-free reads verified against per-slot even/odd seqlock words.
#[derive(Clone, Copy)]
pub struct FrameBank {
    base: *mut u8,
}

impl FrameBank {
    /// # Safety
    /// `base` must point to a valid, 4-byte-aligned mapping of at least [`FRAME_BYTES`] bytes.
    pub unsafe fn at(base: *mut u8) -> Self {
        Self { base }
    }

    /// SexUVC: open the writable view of the live frame bank.
    #[cfg(target_os = "none")]
    pub fn open_writer() -> Option<Self> {
        crate::memory_region_full_view(
            FRAME_WRITER_SLOT,
            crate::memory_region::QUERY_VIEW
                | crate::memory_region::READ
                | crate::memory_region::WRITE,
        )
        .ok()
        .filter(|v| v.length >= FRAME_BYTES && v.virtual_address != 0)
        .map(|v| unsafe { Self::at(v.virtual_address as *mut u8) })
    }

    /// Program row (e.g. Flash): open the read-only view of the live frame bank.
    #[cfg(target_os = "none")]
    pub fn open_reader() -> Option<Self> {
        crate::memory_region_full_view(
            FRAME_CLIENT_SLOT,
            crate::memory_region::QUERY_VIEW | crate::memory_region::READ,
        )
        .ok()
        .filter(|v| v.length >= FRAME_BYTES && v.virtual_address != 0)
        .map(|v| unsafe { Self::at(v.virtual_address as *mut u8) })
    }

    fn word(&self, at: usize) -> &AtomicU32 {
        unsafe { &*(self.base.add(at) as *const AtomicU32) }
    }
    fn load(&self, at: usize) -> u32 {
        self.word(at).load(Ordering::Acquire)
    }
    fn store(&self, at: usize, v: u32) {
        self.word(at).store(v, Ordering::Release)
    }
    fn slot_meta(slot: usize) -> usize {
        FB_SLOT_META_BASE + (slot & 1) * FB_SLOT_META_STRIDE
    }
    fn slot_ptr(&self, slot: usize) -> *mut u8 {
        unsafe { self.base.add(FRAME_HEADER + (slot & 1) * FRAME_SLOT_BYTES) }
    }

    /// SexUVC: announce that a physical camera is attached and configured.
    pub fn set_active(&self, vendor: u16, product: u16, width: u16, height: u16, format: u8, fps: u16) {
        self.store(FB_VENDOR_PRODUCT, vendor as u32 | ((product as u32) << 16));
        self.store(FB_WIDTH, width as u32);
        self.store(FB_HEIGHT, height as u32);
        self.store(FB_FORMAT, format as u32);
        self.store(FB_FPS, fps as u32);
        self.store(FB_MAGIC, FB_MAGIC_VAL);
        self.store(FB_ACTIVE, 1);
    }

    /// SexUVC: mark the live camera inactive (e.g. on USB detach).
    pub fn set_inactive(&self) {
        self.store(FB_ACTIVE, 0);
    }

    /// Reader: true if a physical USB camera is currently active in `sexuvc`.
    pub fn is_active(&self) -> bool {
        self.load(FB_MAGIC) == FB_MAGIC_VAL && self.load(FB_ACTIVE) == 1
    }

    /// Reader: current monotonic published frame sequence (`0` before first frame).
    pub fn seq(&self) -> u32 {
        if !self.is_active() {
            return 0;
        }
        self.load(FB_SEQ)
    }

    /// Reader: snapshot of current camera metadata if active.
    pub fn info(&self) -> Option<LiveFrameMeta> {
        if !self.is_active() {
            return None;
        }
        let vp = self.load(FB_VENDOR_PRODUCT);
        Some(LiveFrameMeta {
            seq: self.load(FB_SEQ),
            width: self.load(FB_WIDTH) as u16,
            height: self.load(FB_HEIGHT) as u16,
            format: self.load(FB_FORMAT) as u8,
            fps: self.load(FB_FPS) as u16,
            len: self.load(FB_LEN) as usize,
            vendor: vp as u16,
            product: (vp >> 16) as u16,
            dropped: self.load(FB_DROPPED),
        })
    }

    /// SexUVC: publish one complete frame into the inactive double-buffer slot.
    pub fn publish(
        &self,
        width: u16,
        height: u16,
        format: u8,
        fps: u16,
        vendor: u16,
        product: u16,
        dropped: u32,
        payload: &[u8],
    ) -> Option<u32> {
        if payload.is_empty() || payload.len() > FRAME_SLOT_BYTES {
            return None;
        }
        let next_slot = ((self.load(FB_ACTIVE_SLOT) ^ 1) & 1) as usize;
        let next_seq = self.load(FB_SEQ).wrapping_add(1).max(1);
        let meta = Self::slot_meta(next_slot);
        let odd_gen = next_seq.wrapping_mul(2).wrapping_sub(1);
        let even_gen = next_seq.wrapping_mul(2);

        self.store(meta, odd_gen);
        core::sync::atomic::fence(Ordering::Release);
        unsafe {
            core::ptr::copy_nonoverlapping(payload.as_ptr(), self.slot_ptr(next_slot), payload.len());
        }
        self.store(meta + 4, payload.len() as u32);
        self.store(meta + 8, width as u32 | ((height as u32) << 16));
        self.store(meta + 12, format as u32 | ((fps as u32) << 8));
        self.store(meta, even_gen);

        self.store(FB_WIDTH, width as u32);
        self.store(FB_HEIGHT, height as u32);
        self.store(FB_FORMAT, format as u32);
        self.store(FB_LEN, payload.len() as u32);
        self.store(FB_FPS, fps as u32);
        self.store(FB_VENDOR_PRODUCT, vendor as u32 | ((product as u32) << 16));
        self.store(FB_DROPPED, dropped);
        self.store(FB_ACTIVE_SLOT, next_slot as u32);
        self.store(FB_MAGIC, FB_MAGIC_VAL);
        self.store(FB_ACTIVE, 1);
        self.store(FB_SEQ, next_seq);
        Some(next_seq)
    }

    /// Reader: copy the latest frame into `out` if its sequence is newer than `last_seq`.
    /// Returns `None` if inactive, no new frame is available, `out` is too small, or a
    /// concurrent write invalidated all retry attempts.
    pub fn read_frame(&self, last_seq: u32, out: &mut [u8]) -> Option<LiveFrameMeta> {
        if !self.is_active() {
            return None;
        }
        let seq = self.load(FB_SEQ);
        if seq == 0 || seq == last_seq {
            return None;
        }
        for _ in 0..4 {
            let slot = (self.load(FB_ACTIVE_SLOT) & 1) as usize;
            let meta = Self::slot_meta(slot);
            let s0 = self.load(meta);
            if s0 == 0 || (s0 & 1) != 0 {
                continue;
            }
            let len = self.load(meta + 4) as usize;
            let dims = self.load(meta + 8);
            let fmt_fps = self.load(meta + 12);
            let vp = self.load(FB_VENDOR_PRODUCT);
            let dropped = self.load(FB_DROPPED);
            if len == 0 || len > FRAME_SLOT_BYTES || len > out.len() {
                return None;
            }
            unsafe {
                core::ptr::copy_nonoverlapping(self.slot_ptr(slot), out.as_mut_ptr(), len);
            }
            core::sync::atomic::fence(Ordering::Acquire);
            let s1 = self.load(meta);
            if s0 == s1 {
                let frame_seq = s0 / 2;
                if frame_seq == last_seq {
                    return None;
                }
                return Some(LiveFrameMeta {
                    seq: frame_seq,
                    width: dims as u16,
                    height: (dims >> 16) as u16,
                    format: fmt_fps as u8,
                    fps: (fmt_fps >> 8) as u16,
                    len,
                    vendor: vp as u16,
                    product: (vp >> 16) as u16,
                    dropped,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_metadata_commands_and_chunk_ring() {
        let mut mem = vec![0u32; BYTES as usize / 4];
        let s = unsafe { Shared::at(mem.as_mut_ptr() as *mut u8) };
        assert!(s.camera().is_none());

        let mut info = CameraInfo {
            vendor: 0x0c45,
            product: 0x6d18,
            generation: 0,
            control_iface: 0,
            stream_iface: 1,
            alt_count: 6,
            bulk: false,
            max_payload: 3072,
            format_count: 2,
            formats: [FormatMeta::default(); MAX_FORMATS],
        };
        info.formats[0] = FormatMeta { index: 1, kind: FMT_MJPEG, frames: 5, max_w: 1280, max_h: 720 };
        info.formats[1] = FormatMeta { index: 2, kind: FMT_YUY2, frames: 4, max_w: 1280, max_h: 720 };
        s.set_camera(Some(&info));

        let got = s.camera().expect("camera published");
        assert_eq!((got.vendor, got.product, got.generation), (0x0c45, 0x6d18, 1));
        assert_eq!((got.control_iface, got.stream_iface, got.alt_count, got.max_payload), (0, 1, 6, 3072));
        assert_eq!(got.formats_slice().len(), 2);
        assert_eq!(got.max_dimensions(), (1280, 720));

        // Command roundtrip
        assert!(s.poll_command().is_none());
        let seq = s.request_start(1, 1, 30, 1280, 720);
        let cmd = s.poll_command().expect("pending start");
        assert_eq!((cmd.seq, cmd.op, cmd.format_index, cmd.frame_index, cmd.fps, cmd.width, cmd.height),
                   (seq, CMD_START, 1, 1, 30, 1280, 720));
        s.ack_command(cmd.seq, STATE_STREAMING, 6, 0x81, 3072);
        assert!(s.poll_command().is_none());
        assert_eq!(s.stream_state(), STATE_STREAMING);
        assert_eq!(s.active_alt(), (6, 0x81, 3072));

        // Chunk ring push/pop
        assert!(s.push_chunk(FLAG_SOF, 1, b"hello "));
        assert!(s.push_chunk(FLAG_EOF, 1, b"world"));
        assert_eq!(s.frames_done(), 1);
        let mut buf = [0u8; MAX_CHUNK];
        let c1 = s.pop_chunk(&mut buf).unwrap();
        assert_eq!((c1.len, c1.flags, c1.frame_seq), (6, FLAG_SOF, 1));
        assert_eq!(&buf[..c1.len], b"hello ");
        let c2 = s.pop_chunk(&mut buf).unwrap();
        assert_eq!((c2.len, c2.flags, c2.frame_seq), (5, FLAG_EOF, 1));
        assert_eq!(&buf[..c2.len], b"world");
        assert!(s.pop_chunk(&mut buf).is_none());

        // Disconnect clears camera and increments generation on re-attach
        s.set_camera(None);
        assert!(s.camera().is_none());
        s.set_camera(Some(&info));
        assert_eq!(s.camera().unwrap().generation, 2);
    }

    #[test]
    fn frame_bank_double_buffer_publish_and_read() {
        let mut mem = vec![0u32; FRAME_BYTES as usize / 4];
        let bank = unsafe { FrameBank::at(mem.as_mut_ptr() as *mut u8) };
        assert!(!bank.is_active());
        assert!(bank.info().is_none());

        bank.set_active(0x0c45, 0x6d18, 1280, 720, FMT_MJPEG, 30);
        assert!(bank.is_active());
        let mut out = vec![0u8; 64];
        assert!(bank.read_frame(0, &mut out).is_none());

        let f1 = [0xFFu8, 0xD8, 0xFF, 0xE0, 0x01, 0x02, 0xFF, 0xD9];
        assert_eq!(bank.publish(1280, 720, FMT_MJPEG, 30, 0x0c45, 0x6d18, 0, &f1), Some(1));
        let m1 = bank.read_frame(0, &mut out).expect("frame 1");
        assert_eq!((m1.seq, m1.width, m1.height, m1.format, m1.fps, m1.len), (1, 1280, 720, FMT_MJPEG, 30, f1.len()));
        assert_eq!((m1.vendor, m1.product, m1.dropped), (0x0c45, 0x6d18, 0));
        assert_eq!(&out[..m1.len], &f1);
        // Same seq returns None
        assert!(bank.read_frame(m1.seq, &mut out).is_none());

        // Second frame flips to the other slot
        let f2 = [0xFFu8, 0xD8, 0xFF, 0xDB, 0xAA, 0xBB, 0xCC, 0xFF, 0xD9];
        assert_eq!(bank.publish(1280, 720, FMT_MJPEG, 30, 0x0c45, 0x6d18, 2, &f2), Some(2));
        let m2 = bank.read_frame(m1.seq, &mut out).expect("frame 2");
        assert_eq!((m2.seq, m2.len, m2.dropped), (2, f2.len(), 2));
        assert_eq!(&out[..m2.len], &f2);

        // Detach marks inactive
        bank.set_inactive();
        assert!(!bank.is_active());
        assert!(bank.read_frame(0, &mut out).is_none());
    }
}
