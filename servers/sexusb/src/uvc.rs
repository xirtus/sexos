// SPDX-License-Identifier: GPL-3.0-only
//! USB Video Class (UVC) webcam streaming transport for `servers/sexuvc`.
//!
//! When `uvc_scan::scan` finds a UVC camera (such as the Alienware x17 R1's `0c45:6d18`),
//! SexUSB publishes its metadata into the `sex_pdx::usb_uvc` shared region and notifies
//! `sexuvc` via `OP_LINK`. When `sexuvc` requests streaming (`CMD_START`), SexUSB runs
//! `VS_PROBE` / `VS_COMMIT`, selects the matching alternate setting (`SET_INTERFACE`),
//! configures the xHCI isochronous (or bulk) IN endpoint, strips 2–12 byte UVC payload
//! headers via `uvc_stream::UvcReassembler`, and pushes frame payload chunks into the ring.

use sex_pdx::serial_record as log;
use sex_pdx::usb_uvc::{self, CameraInfo, FormatMeta, Shared};
use super::{Usb, Failure, Ring, xhci, trb, page, isoch, uvc_scan, uvc_stream, CC_SHORT_PACKET, CC_SUCCESS};

/// Transfer ring positions (31 data TRBs + 1 link TRB) and 4 KiB DMA buffer per position.
const UVC_TRBS: u32 = 32;
const UVC_BUF_SLOT: u64 = 4096;
const UVC_IN_FLIGHT: u32 = 8;

/// Completion codes an isochronous IN endpoint reports without being broken.
const CC_RING_UNDERRUN: u32 = 14;
const CC_RING_OVERRUN: u32 = 21;
const CC_MISSED_SERVICE: u32 = 23;
const CC_STOPPED: u32 = 26;
const CC_STOPPED_LENGTH: u32 = 27;

/// Single-threaded reassembler staging buffer (kept off `Usb`'s stack frame).
static mut UVC_REASSEMBLER: uvc_stream::UvcReassembler = uvc_stream::UvcReassembler::new();

#[derive(Clone, Copy)]
pub(super) struct Uvc {
    pub(super) dev: usize,
    speed: u32,
    cam: uvc_scan::Camera,
    running: bool,
    dci: u32,
    ep: u8,
    alt: u8,
    isoch: bool,
    ep_params: isoch::Endpoint,
    td_len: u32,
    ring: Ring,
    packets: u64,
    errors: u32,
}

fn notify(up: bool) {
    let (status, _) = sex_pdx::pdx_call(usb_uvc::NOTIFY_SLOT, usb_uvc::OP_LINK, up as u64, 0, 0);
    log!("[sexusb.uvc.notify] up={} sent={}", up as u8, (status == 0) as u8);
}

fn format_kind(f: uvc_scan::Format) -> u8 {
    match f {
        uvc_scan::Format::Mjpeg => usb_uvc::FMT_MJPEG,
        uvc_scan::Format::Yuy2 => usb_uvc::FMT_YUY2,
        uvc_scan::Format::Nv12 => usb_uvc::FMT_NV12,
        uvc_scan::Format::Other => usb_uvc::FMT_OTHER,
    }
}

impl Usb {
    /// Publish a detected UVC webcam to `sexuvc` and keep it ready for `CMD_START`.
    pub(super) fn try_uvc(&mut self, i: usize, cam: uvc_scan::Camera, config: u8, speed: u32) -> Result<bool, Failure> {
        if self.uvc.is_some() {
            return Ok(false);
        }
        let slot = self.dev[i].slot;
        let (vendor, product) = (self.dev[i].vendor, self.dev[i].product);
        // Set configuration (alt 0 on VideoStreaming uses zero bus bandwidth until started).
        self.set(i, 0x00, 9, config as u16, 0)?;

        let mut info = CameraInfo {
            vendor,
            product,
            generation: 0,
            control_iface: cam.control_iface,
            stream_iface: cam.stream_iface,
            alt_count: cam.alt_count as u8,
            bulk: cam.bulk(),
            max_payload: cam.max_payload(speed),
            format_count: 0,
            formats: [FormatMeta::default(); usb_uvc::MAX_FORMATS],
        };
        for f in cam.formats() {
            let idx = info.format_count as usize;
            if idx < usb_uvc::MAX_FORMATS {
                info.formats[idx] = FormatMeta {
                    index: f.index,
                    kind: format_kind(f.format),
                    frames: f.frames,
                    max_w: f.max_w,
                    max_h: f.max_h,
                };
                info.format_count += 1;
            }
        }

        self.uvc = Some(Uvc {
            dev: i,
            speed,
            cam,
            running: false,
            dci: 0,
            ep: 0,
            alt: 0,
            isoch: !cam.bulk(),
            ep_params: isoch::Endpoint::default(),
            td_len: 0,
            ring: Ring::empty(),
            packets: 0,
            errors: 0,
        });

        let route = self.uvc_region != 0;
        log!(
            "[sexusb.uvc.published] slot={} vendor={:#06x} product={:#06x} route={} formats={} max_payload={}",
            slot, vendor, product, route as u8, info.format_count, info.max_payload
        );
        if route {
            unsafe { Shared::at(self.uvc_region as *mut u8) }.set_camera(Some(&info));
            notify(true);
        }
        Ok(true)
    }

    /// True while a UVC camera is attached (so SexUSB checks stream commands promptly).
    pub fn uvc_active(&self) -> bool {
        self.uvc.is_some()
    }

    /// Poll `sex_pdx::usb_uvc` for start/stop commands from `sexuvc`.
    pub fn uvc_pump(&mut self) -> bool {
        if self.uvc_region == 0 {
            return false;
        }
        let Some(u) = self.uvc else { return false };
        let shared = unsafe { Shared::at(self.uvc_region as *mut u8) };
        let Some(cmd) = shared.poll_command() else { return false };
        match cmd.op {
            usb_uvc::CMD_START => {
                match self.uvc_start(u, cmd) {
                    Ok((alt, ep, td_len)) => {
                        shared.ack_command(cmd.seq, usb_uvc::STATE_STREAMING, alt, ep, td_len.min(u16::MAX as u32) as u16);
                    }
                    Err(e) => {
                        log!("[sexusb.uvc.start.fail] slot={} reason={:?}", self.dev[u.dev].slot, e);
                        shared.ack_command(cmd.seq, usb_uvc::STATE_ERROR, 0, 0, 0);
                    }
                }
                true
            }
            usb_uvc::CMD_STOP => {
                self.uvc_stop(u);
                shared.ack_command(cmd.seq, usb_uvc::STATE_STOPPED, 0, 0, 0);
                true
            }
            _ => {
                shared.ack_command(cmd.seq, if u.running { usb_uvc::STATE_STREAMING } else { usb_uvc::STATE_STOPPED }, u.alt, u.ep, u.td_len as u16);
                true
            }
        }
    }

    /// Negotiate VS_PROBE / VS_COMMIT, select alternate setting, configure endpoint, and arm TRBs.
    fn uvc_start(&mut self, mut u: Uvc, cmd: usb_uvc::StreamCommand) -> Result<(u8, u8, u32), Failure> {
        let i = u.dev;
        self.select(self.dev[i].ctl);
        let slot = self.dev[i].slot;
        let buf = self.dev[i].mem.unwrap().buf;

        // 1. VS_PROBE SET_CUR -> GET_CUR -> VS_COMMIT SET_CUR (26-byte UVC 1.0/1.1 probe/commit).
        let probe = uvc_stream::build_probe_commit(cmd.format_index, cmd.frame_index, cmd.fps);
        unsafe { core::ptr::copy_nonoverlapping(probe.as_ptr(), buf.1 as *mut u8, probe.len()); }
        let iface = u.cam.stream_iface as u16;
        let _ = self.get(i, uvc_stream::BM_REQ_SET_IFACE, uvc_stream::REQ_SET_CUR, uvc_stream::VS_PROBE_CONTROL, iface, buf.0, 26);
        let mut need_payload = u.cam.max_payload(u.speed);
        if let Ok(got) = self.get(i, uvc_stream::BM_REQ_GET_IFACE, uvc_stream::REQ_GET_CUR, uvc_stream::VS_PROBE_CONTROL, iface, buf.0, 26) {
            let reply = unsafe { core::slice::from_raw_parts(buf.1 as *const u8, (got as usize).min(26)) };
            if let Some((_, max_payload)) = uvc_stream::parse_probe_response(reply) {
                if max_payload > 0 {
                    need_payload = max_payload;
                }
            }
            let _ = self.get(i, uvc_stream::BM_REQ_SET_IFACE, uvc_stream::REQ_SET_CUR, uvc_stream::VS_COMMIT_CONTROL, iface, buf.0, 26);
        } else {
            unsafe { core::ptr::copy_nonoverlapping(probe.as_ptr(), buf.1 as *mut u8, probe.len()); }
            let _ = self.get(i, uvc_stream::BM_REQ_SET_IFACE, uvc_stream::REQ_SET_CUR, uvc_stream::VS_COMMIT_CONTROL, iface, buf.0, 26);
        }

        // 2. Pick the smallest alt-setting whose ESIT payload covers `need_payload`.
        let mut alt_table = [(0u8, 0u32); uvc_scan::MAX_ALTS];
        let mut alt_n = 0usize;
        for a in u.cam.alts() {
            alt_table[alt_n] = (a.alt, a.endpoint(u.speed).esit_payload());
            alt_n += 1;
        }
        let chosen_alt_num = isoch::pick_alt(&alt_table[..alt_n], need_payload).ok_or(Failure::Code(0))?;
        let chosen = *u.cam.alts().iter().find(|a| a.alt == chosen_alt_num).ok_or(Failure::Code(0))?;
        let ep_params = chosen.endpoint(u.speed);
        let td_len = isoch::td_bytes(&ep_params).clamp(1, UVC_BUF_SLOT as u32);
        let dci = (chosen.ep & 0x0f) as u32 * 2 + 1;

        // 3. Lazily allocate DMA ring (1 page) and TRB buffers (32 x 4 KiB = 32 pages).
        if self.uvc_ring.0 == 0 {
            let pool = &mut self.x.pool;
            self.uvc_ring = page(pool, 1)?;
            self.uvc_buffers = page(pool, (UVC_TRBS as u64 * UVC_BUF_SLOT).div_ceil(4096))?;
        }
        u.ring.init(self.uvc_ring.0, self.uvc_ring.1, UVC_TRBS);

        // 4. Configure xHCI endpoint context (Isoch IN type 5 with CErr=0, or Bulk IN type 6).
        self.input_context(i, 0, 0);
        if chosen.isoch {
            let m = self.dev[i].mem.unwrap();
            let e = self.ctx(m.in_ctx.1, 1 + dci as u64);
            let deq = u.ring.dequeue_pointer();
            let esit = ep_params.esit_payload();
            unsafe {
                core::ptr::write_volatile(e, ((ep_params.mult as u32) << 8) | (ep_params.interval << 16) | ((esit >> 16) << 24));
                core::ptr::write_volatile(e.add(1), (isoch::EP_ISOCH_IN << 3) | ((ep_params.burst as u32) << 8) | ((ep_params.mps as u32) << 16));
                core::ptr::write_volatile(e.add(2), deq as u32);
                core::ptr::write_volatile(e.add(3), (deq >> 32) as u32);
                core::ptr::write_volatile(e.add(4), td_len | ((esit & 0xFFFF) << 16));
            }
        } else {
            self.endpoint_context_burst(i, dci, 6, ep_params.mps, ep_params.burst, u.ring.dequeue_pointer());
        }
        self.input_flags(i, 1 | (1 << dci));
        self.slot_context(i, dci);
        let mem = self.dev[i].mem.unwrap();
        self.x.command_ok(mem.in_ctx.0 as u32, (mem.in_ctx.0 >> 32) as u32, 0, (trb::CONFIGURE_ENDPOINT << 10) | (slot << 24))?;

        // 5. Select the streaming alternate setting (for isoch endpoints).
        if chosen.isoch {
            self.set(i, 0x01, 0x0B, chosen.alt as u16, iface)?;
        }

        // 6. Reset reassembler, queue initial IN TRBs, and ring the doorbell.
        unsafe { (*core::ptr::addr_of_mut!(UVC_REASSEMBLER)).reset(); }
        u.dci = dci;
        u.ep = chosen.ep;
        u.alt = chosen.alt;
        u.isoch = chosen.isoch;
        u.ep_params = ep_params;
        u.td_len = td_len;
        u.running = true;
        for _ in 0..UVC_IN_FLIGHT {
            self.uvc_queue(&mut u);
        }
        self.x.doorbell(slot, dci);
        self.uvc = Some(u);
        log!(
            "[sexusb.uvc.stream] on=1 slot={} iface={} alt={} ep={:#04x} td={} isoch={} fmt={} frame={} fps={}",
            slot, u.cam.stream_iface, chosen.alt, chosen.ep, td_len, chosen.isoch as u8, cmd.format_index, cmd.frame_index, cmd.fps
        );
        Ok((chosen.alt, chosen.ep, td_len))
    }

    fn uvc_stop(&mut self, mut u: Uvc) {
        let i = u.dev;
        self.select(self.dev[i].ctl);
        let slot = self.dev[i].slot;
        u.running = false;
        if u.isoch {
            let _ = self.set(i, 0x01, 0x0B, 0, u.cam.stream_iface as u16);
        }
        if u.dci != 0 {
            self.x.recover(slot, u.dci, &mut u.ring);
        }
        unsafe { (*core::ptr::addr_of_mut!(UVC_REASSEMBLER)).reset(); }
        self.uvc = Some(u);
        log!("[sexusb.uvc.stream] on=0 slot={} packets={} errors={}", slot, u.packets, u.errors);
    }

    fn uvc_queue(&mut self, u: &mut Uvc) {
        let pos = u.ring.idx as u64;
        let phys = self.uvc_buffers.0 + pos * UVC_BUF_SLOT;
        if u.isoch {
            let t = isoch::trb(phys, u.td_len, &u.ep_params, None, true, 0);
            u.ring.push(t[0], t[1], t[2], t[3]);
        } else {
            u.ring.push(phys as u32, (phys >> 32) as u32, u.td_len, (trb::NORMAL << 10) | trb::IOC | trb::ISP);
        }
    }

    /// Handle a transfer event on the UVC streaming endpoint. Returns false if not ours.
    pub(super) fn uvc_event(&mut self, i: usize, e: xhci::Event) -> bool {
        let Some(mut u) = self.uvc.filter(|u| u.dev == i && u.dci != 0 && u.dci == e.endpoint()) else {
            return false;
        };
        let slot = self.dev[i].slot;
        match e.code() {
            CC_STOPPED | CC_STOPPED_LENGTH => {
                self.uvc = Some(u);
                return true;
            }
            CC_SUCCESS | CC_SHORT_PACKET if u.running => {
                let got = u.td_len.saturating_sub(e.residual()) as usize;
                if let Some(pos) = u.ring.index_of(e.pointer()) {
                    let raw = unsafe {
                        core::slice::from_raw_parts(
                            (self.uvc_buffers.1 + pos as u64 * UVC_BUF_SLOT) as *const u8,
                            got.min(UVC_BUF_SLOT as usize),
                        )
                    };
                    if self.uvc_region != 0 && !raw.is_empty() {
                        let shared = unsafe { Shared::at(self.uvc_region as *mut u8) };
                        shared.add_packet_stats(1, raw.len() as u32);
                        let reasm = unsafe { &mut *core::ptr::addr_of_mut!(UVC_REASSEMBLER) };
                        reasm.feed(raw, |flags, seq, chunk| shared.push_chunk(flags, seq, chunk));
                    }
                    u.packets += 1;
                }
            }
            CC_MISSED_SERVICE | CC_RING_UNDERRUN | CC_RING_OVERRUN => {}
            _ if !u.running => {
                self.uvc = Some(u);
                return true;
            }
            code => {
                u.errors += 1;
                if u.errors <= 8 || u.errors.is_power_of_two() {
                    log!("[sexusb.uvc.error] slot={} dci={} code={} errors={}", slot, u.dci, code, u.errors);
                }
                self.x.recover(slot, u.dci, &mut u.ring);
                for _ in 0..UVC_IN_FLIGHT - 1 {
                    self.uvc_queue(&mut u);
                }
            }
        }
        if u.running {
            self.uvc_queue(&mut u);
            self.x.doorbell(slot, u.dci);
        }
        self.uvc = Some(u);
        true
    }

    /// The webcam device disconnected.
    pub(super) fn uvc_gone(&mut self, i: usize) {
        if let Some(u) = self.uvc.filter(|u| u.dev == i) {
            self.uvc = None;
            unsafe { (*core::ptr::addr_of_mut!(UVC_REASSEMBLER)).reset(); }
            if self.uvc_region != 0 {
                unsafe { Shared::at(self.uvc_region as *mut u8) }.set_camera(None);
                notify(false);
            }
            log!("[sexusb.uvc.gone] slot={} packets={} errors={}", self.dev[i].slot, u.packets, u.errors);
        }
    }
}
