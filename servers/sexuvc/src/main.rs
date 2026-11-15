// SPDX-License-Identifier: GPL-3.0-only
//! SexUVC: SexOS native Userspace USB Video Class (UVC) Webcam Driver Server.
//!
//! Modeled after `servers/cufflinks`: `#![no_std]`, `#![no_main]`, `GPL-3.0-only`.
//! Wraps `sex_uvc`: enumerates UVC interface descriptors, negotiates Probe/Commit,
//! enforces per-client camera capability grants, reassembles incoming USB packets via
//! `UvcStreamAssembler`, and logs progress to the serial record.

#![no_std]
#![no_main]

extern crate alloc;

use sex_pdx::serial_record as log;
use sex_uvc::control::VideoProbeCommitControl;
use sex_uvc::descriptors::{parse_descriptors, UvcDeviceDescriptors};
use sex_uvc::stream::UvcStreamAssembler;
use sex_uvc::SyntheticUvcDevice;

const HEAP_BYTES: usize = 2 * 1024 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct HeapBytes([u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);

#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

#[no_mangle]
#[allow(suspicious_runtime_symbol_definitions)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    for i in 0..n {
        *dest.add(i) = *src.add(i);
    }
    dest
}

#[no_mangle]
#[allow(suspicious_runtime_symbol_definitions)]
pub unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    for i in 0..n {
        *dest.add(i) = c as u8;
    }
    dest
}

#[no_mangle]
#[allow(suspicious_runtime_symbol_definitions)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) <= (src as usize) {
        for i in 0..n {
            *dest.add(i) = *src.add(i);
        }
    } else {
        for i in (0..n).rev() {
            *dest.add(i) = *src.add(i);
        }
    }
    dest
}

#[no_mangle]
#[allow(suspicious_runtime_symbol_definitions)]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let diff = *a.add(i) as i32 - *b.add(i) as i32;
        if diff != 0 {
            return diff;
        }
    }
    0
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log!("[sexuvc.panic] {}", info);
    park()
}

fn park() -> ! {
    loop {
        let _ = sex_pdx::sys_wait_self_ticks(100);
    }
}

/// Camera capability grant for a client task or protection domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraGrant {
    pub client_id: u32,
    pub max_width: u16,
    pub max_height: u16,
    pub max_fps: u32,
    pub active: bool,
}

pub struct CameraGrantTable {
    grants: [Option<CameraGrant>; 16],
}

impl CameraGrantTable {
    pub const fn new() -> Self {
        Self {
            grants: [None; 16],
        }
    }

    pub fn grant(&mut self, client_id: u32, max_width: u16, max_height: u16, max_fps: u32) -> bool {
        for slot in self.grants.iter_mut() {
            if slot.is_none() || slot.as_ref().map_or(false, |g| g.client_id == client_id) {
                *slot = Some(CameraGrant {
                    client_id,
                    max_width,
                    max_height,
                    max_fps,
                    active: true,
                });
                return true;
            }
        }
        false
    }

    pub fn check(&self, client_id: u32, width: u16, height: u16, fps: u32) -> bool {
        for slot in self.grants.iter().flatten() {
            if slot.client_id == client_id && slot.active {
                if width <= slot.max_width && height <= slot.max_height && fps <= slot.max_fps {
                    return true;
                }
            }
        }
        false
    }

    #[allow(dead_code)]
    pub fn revoke(&mut self, client_id: u32) {
        for slot in self.grants.iter_mut() {
            if let Some(g) = slot {
                if g.client_id == client_id {
                    g.active = false;
                }
            }
        }
    }
}

/// The core UVC userspace driver server engine.
pub struct UvcDriverServer {
    grant_table: CameraGrantTable,
    assembler: UvcStreamAssembler,
    negotiated_ctrl: Option<VideoProbeCommitControl>,
    frame_count: u64,
}

impl UvcDriverServer {
    pub fn new() -> Self {
        let mut grant_table = CameraGrantTable::new();
        // Default permission grants for shell and flash apps
        grant_table.grant(0, 3840, 2160, 60);
        grant_table.grant(1, 1920, 1080, 60);
        grant_table.grant(2, 1280, 720, 60);

        Self {
            grant_table,
            assembler: UvcStreamAssembler::new(4 * 1024 * 1024),
            negotiated_ctrl: None,
            frame_count: 0,
        }
    }

    pub fn enumerate_device(&mut self, raw_descriptors: &[u8], vendor: u16, product: u16, name: &str) -> Option<UvcDeviceDescriptors> {
        match parse_descriptors(raw_descriptors) {
            Ok(desc) => {
                let stream_iface = desc.streaming.first()?;
                let iface_num = stream_iface.interface_number;
                let num_formats = stream_iface.formats.len();
                log!(
                    "[sexuvc.device] vendor={:#06x} product={:#06x} iface={} formats={} name=\"{}\"",
                    vendor,
                    product,
                    iface_num,
                    num_formats,
                    name
                );
                Some(desc)
            }
            Err(e) => {
                log!("[sexuvc.error] descriptor_parse={:?}", e);
                None
            }
        }
    }

    pub fn negotiate_probe_commit(&mut self, client_id: u32, dev_desc: &UvcDeviceDescriptors, target_w: u16, target_h: u16, target_fps: u32) -> bool {
        if !self.grant_table.check(client_id, target_w, target_h, target_fps) {
            log!("[sexuvc.denied] client={} w={} h={} fps={}", client_id, target_w, target_h, target_fps);
            return false;
        }

        let stream_iface = match dev_desc.streaming.first() {
            Some(s) => s,
            None => return false,
        };

        let format = match stream_iface.formats.first() {
            Some(f) => f,
            None => return false,
        };

        let frame = format.frames.iter().find(|fr| fr.width == target_w && fr.height == target_h)
            .or_else(|| format.frames.first());

        let (w, h, fps) = if let Some(fr) = frame {
            (fr.width, fr.height, fr.default_fps().max(target_fps))
        } else {
            (target_w, target_h, target_fps)
        };

        let mut ctrl = VideoProbeCommitControl::default();
        ctrl.b_format_index = format.format_index;
        ctrl.b_frame_index = frame.map_or(1, |f| f.frame_index);
        ctrl.set_fps(fps);
        ctrl.dw_max_video_frame_size = (w as u32) * (h as u32) * (format.bits_per_pixel as u32 / 8).max(2);

        self.negotiated_ctrl = Some(ctrl);
        log!("[sexuvc.commit] format={:?} width={} height={} fps={}", format.kind, w, h, fps);
        true
    }

    pub fn process_packet(&mut self, packet: &[u8]) {
        match self.assembler.push_packet(packet) {
            Ok(Some(frame)) => {
                self.frame_count = self.frame_count.wrapping_add(1);
                let dropped = self.assembler.frames_corrupted;
                log!("[sexuvc.frame] n={} bytes={} dropped={}", self.frame_count, frame.data.len(), dropped);
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES);
    }
    log!("[sexuvc.start]");

    let mut server = UvcDriverServer::new();

    // Initialize camera device (hardware or deterministic synthetic camera loopback)
    let mut synthetic = SyntheticUvcDevice::new(640, 480, 30);
    let descriptors = synthetic.binary_descriptors();

    if let Some(desc) = server.enumerate_device(&descriptors, 0x1d6b, 0x0002, "SexOS UVC Webcam") {
        if server.negotiate_probe_commit(0, &desc, 640, 480, 30) {
            // Process initial verification frames
            for _ in 0..3 {
                let packets = synthetic.next_frame_packets();
                for pkt in &packets {
                    server.process_packet(pkt);
                }
            }
        }
    }

    loop {
        let _ = sex_pdx::sys_wait_self_ticks(100);
    }
}
