// SPDX-License-Identifier: GPL-3.0-only
//! Flash: USB UVC Photo Booth & Capture App (Live Filters, Timer, Burst, Video & Library).
//!
//! Features:
//! - 9 Live Photo Booth filters (Normal, Sepia, Noir, Thermal, XRay, PopArt, Mirror, Bulge, Comic)
//! - 3x3 Live Effects Grid matrix mode ('E' key / Effects button)
//! - Single Shot ('Space' / Shutter button) with white screen flash
//! - 3-Second Countdown Timer ('T' key / Timer button) with bold badge & amber pips
//! - 4-Shot Burst ('B' key / Burst button) + 2x2 Photo Booth Contact Sheet composition
//! - WebM video recording ('V'/'R' key / Video button) via Splicy Exporter
//! - Persistent Photo Library filmstrip with thumbnail preview, delete ('Delete' key)
//! - Direct launch into Gaze ('G' key / Open in Gaze button)
//! - Serial control logging for deterministic automated QEMU interactions.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use sex_app::net::direct::{Socket, State};
use sex_app::net::IpAddr;
use sex_app::{files, Event, Window};
use sex_media::export::{Config, Exporter, Format};
use sex_media::recording::Drain;
use sex_pdx::serial_record as log;
use sex_uvc::control::VideoProbeCommitControl;
use sex_uvc::descriptors::{parse_descriptors, FormatKind};
use sex_uvc::fixture::{SyntheticUvcDevice, WireMessage};
use sex_uvc::photobooth::{
    apply_filter, compose_burst_contact_sheet, draw_text_8x8, render_effects_grid, Filter,
};
use sex_uvc::pixel::decode_frame;
use sex_uvc::stream::UvcStreamAssembler;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log!("[flash.panic] {}", info);
    sex_pdx::task_finish(1)
}

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

const MAX_FRAME_DIM: usize = 4096;
const MAX_RX_BUF: usize = 4 * 1024 * 1024;
const MAX_VIDEO_BYTES: usize = 64 * 1024 * 1024;

fn valid_uvc_dims(w: usize, h: usize) -> bool {
    (16..=MAX_FRAME_DIM).contains(&w)
        && (16..=MAX_FRAME_DIM).contains(&h)
        && (w & 1) == 0
        && (h & 1) == 0
}

fn push_rx_bounded(rx_buf: &mut Vec<u8>, chunk: &[u8]) -> bool {
    if rx_buf.len().saturating_add(chunk.len()) > MAX_RX_BUF {
        rx_buf.clear();
        return false;
    }
    if rx_buf.try_reserve(chunk.len()).is_err() {
        rx_buf.clear();
        return false;
    }
    rx_buf.extend_from_slice(chunk);
    true
}

fn pop_wire_msg(rx_buf: &mut Vec<u8>) -> Result<Option<WireMessage>, ()> {
    match WireMessage::decode_one(rx_buf) {
        Ok(Some((msg, len))) => {
            rx_buf.drain(..len);
            Ok(Some(msg))
        }
        Ok(None) => Ok(None),
        Err(_) => {
            rx_buf.clear();
            Err(())
        }
    }
}

fn drain_exporter(exp: &mut Exporter, out: &mut Vec<u8>) {
    loop {
        match exp.output() {
            Ok(Drain::Write(w)) => {
                let token = w.token;
                let bytes = w.bytes;
                let len = bytes.len();
                if let Some(end) = usize::try_from(w.offset).ok().and_then(|off| off.checked_add(len)) {
                    if end <= MAX_VIDEO_BYTES {
                        if end > out.len() {
                            if out.try_reserve(end - out.len()).is_ok() {
                                out.resize(end, 0);
                            } else {
                                let _ = exp.acknowledge(token, len);
                                continue;
                            }
                        }
                        let off = end - len;
                        out[off..end].copy_from_slice(bytes);
                    }
                }
                let _ = exp.acknowledge(token, len);
            }
            _ => break,
        }
    }
}

fn encode_png(pixels: &[u32], width: usize, height: usize) -> Result<Vec<u8>, ()> {
    let mut encoder = sex_graphics::png_encode::Encoder::new(width, height).map_err(|_| ())?;
    let mut out = vec![0u8; encoder.total_bytes()];
    let mut written = 0;
    while written < out.len() {
        let n = encoder.fill(pixels, &mut out[written..]).map_err(|_| ())?;
        if n == 0 {
            break;
        }
        written += n;
    }
    out.truncate(written);
    Ok(out)
}

fn send_wire(socket: &Socket, msg: &WireMessage) -> Result<(), ()> {
    let encoded = msg.encode();
    let mut at = 0;
    while at < encoded.len() {
        match socket.send(&encoded[at..]) {
            Ok(n) if n > 0 => at += n,
            Ok(_) => {
                let _ = sex_pdx::sys_wait_self_ticks(1);
            }
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

fn parse_endpoint(text: &[u8]) -> Option<(IpAddr, u16)> {
    let s = text
        .strip_prefix(b"uvc://")
        .or_else(|| text.strip_prefix(b"tcp://"))
        .or_else(|| text.strip_prefix(b"http://"))
        .unwrap_or(text);

    let colon = s.iter().position(|b| *b == b':')?;
    let ip_str = &s[..colon];
    let port_str = &s[colon + 1..];

    let ip = IpAddr::parse(ip_str)?;
    let port: u16 = core::str::from_utf8(port_str).ok()?.parse().ok()?;
    Some((ip, port))
}

fn format_id(id: u32, buf: &mut [u8; 4]) {
    let mut val = id % 10000;
    buf[3] = b'0' + (val % 10) as u8;
    val /= 10;
    buf[2] = b'0' + (val % 10) as u8;
    val /= 10;
    buf[1] = b'0' + (val % 10) as u8;
    val /= 10;
    buf[0] = b'0' + (val % 10) as u8;
}

#[derive(Clone)]
struct Thumbnail {
    name: Vec<u8>,
    path: Vec<u8>,
    w: usize,
    h: usize,
    pixels: Vec<u32>,
    #[allow(dead_code)]
    is_burst: bool,
}

#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

impl Rect {
    const fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Self { x, y, w, h }
    }
    fn contains(&self, px: usize, py: usize) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

fn fill_rect(dst: &mut [u32], dst_w: usize, dst_h: usize, r: Rect, col: u32) {
    let x1 = (r.x + r.w).min(dst_w);
    let y1 = (r.y + r.h).min(dst_h);
    let x0 = r.x.min(dst_w);
    let y0 = r.y.min(dst_h);
    for y in y0..y1 {
        let row = y * dst_w;
        dst[row + x0..row + x1].fill(col);
    }
}

fn draw_rect_outline(dst: &mut [u32], dst_w: usize, dst_h: usize, r: Rect, col: u32, thick: usize) {
    if r.w <= thick * 2 || r.h <= thick * 2 {
        fill_rect(dst, dst_w, dst_h, r, col);
        return;
    }
    fill_rect(dst, dst_w, dst_h, Rect::new(r.x, r.y, r.w, thick), col);
    fill_rect(dst, dst_w, dst_h, Rect::new(r.x, r.y + r.h - thick, r.w, thick), col);
    fill_rect(dst, dst_w, dst_h, Rect::new(r.x, r.y, thick, r.h), col);
    fill_rect(dst, dst_w, dst_h, Rect::new(r.x + r.w - thick, r.y, thick, r.h), col);
}

fn draw_circle(dst: &mut [u32], dst_w: usize, dst_h: usize, cx: usize, cy: usize, radius: usize, col: u32) {
    let r_sq = (radius * radius) as i32;
    let y0 = cy.saturating_sub(radius);
    let y1 = (cy + radius + 1).min(dst_h);
    let x0 = cx.saturating_sub(radius);
    let x1 = (cx + radius + 1).min(dst_w);

    for y in y0..y1 {
        let dy = y as i32 - cy as i32;
        let row = y * dst_w;
        for x in x0..x1 {
            let dx = x as i32 - cx as i32;
            if dx * dx + dy * dy <= r_sq {
                dst[row + x] = col;
            }
        }
    }
}

fn draw_big_digit(dst: &mut [u32], dst_w: usize, dst_h: usize, cx: usize, cy: usize, digit: u8, scale: usize, col: u32) {
    // 8x8 glyph scaled up
    let glyph: &[u8; 8] = match digit {
        b'1' => &[0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00],
        b'2' => &[0x3C, 0x66, 0x06, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        b'3' => &[0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00],
        _ => return,
    };
    let top_x = cx.saturating_sub(4 * scale);
    let top_y = cy.saturating_sub(4 * scale);

    for row in 0..8 {
        let byte = glyph[row];
        for col_idx in 0..8 {
            let bit = (byte >> (7 - col_idx)) & 1;
            if bit != 0 {
                let px0 = top_x + col_idx * scale;
                let py0 = top_y + row * scale;
                fill_rect(dst, dst_w, dst_h, Rect::new(px0, py0, scale, scale), col);
            }
        }
    }
}

fn scale_blit(src: &[u32], src_w: usize, src_h: usize, dst: &mut [u32], dst_w: usize, dst_h: usize, r: Rect) {
    let dw = r.w.min(dst_w.saturating_sub(r.x));
    let dh = r.h.min(dst_h.saturating_sub(r.y));
    if dw == 0 || dh == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for dy in 0..dh {
        let sy = (dy * src_h) / dh;
        let sy_off = sy * src_w;
        let dst_row = (r.y + dy) * dst_w + r.x;
        for dx in 0..dw {
            let sx = (dx * src_w) / dw;
            dst[dst_row + dx] = src[sy_off + sx];
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_app::with_stack(4 << 20, flash_main)
}

extern "C" fn flash_main() -> ! {
    let mut window = Window::open();
    window.title(b"Flash");
    log!("[flash.ready]");

    let mut args_buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let args_given = files::args(&mut window, &mut args_buf);
    let words: Vec<&[u8]> = args_given
        .split(|b| *b == b' ')
        .filter(|w| !w.is_empty())
        .collect();

    let mut auto_photo = false;
    let mut auto_timer = false;
    let mut auto_burst = false;
    let mut auto_filter: Option<Filter> = None;
    let mut auto_record_frames: Option<u64> = None;
    let mut max_frames: Option<u64> = None;
    let mut net_endpoint: Option<(IpAddr, u16)> = None;
    let mut force_synthetic = false;
    let mut library_grant: Option<Vec<u8>> = None;

    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w == b"--photo" {
            auto_photo = true;
        } else if w == b"--timer" {
            auto_timer = true;
        } else if w == b"--burst" {
            auto_burst = true;
        } else if w == b"--filter" {
            if let Some(next) = words.get(i + 1) {
                if let Ok(name_str) = core::str::from_utf8(next) {
                    auto_filter = Filter::from_name(name_str);
                    i += 1;
                }
            }
        } else if w.starts_with(b"--filter=") {
            if let Ok(name_str) = core::str::from_utf8(&w[9..]) {
                auto_filter = Filter::from_name(name_str);
            }
        } else if w == b"--record-frames" {
            if let Some(next) = words.get(i + 1) {
                if let Ok(num) = core::str::from_utf8(next).unwrap_or("").parse::<u64>() {
                    auto_record_frames = Some(num);
                    i += 1;
                }
            }
        } else if w.starts_with(b"--record-frames=") {
            if let Ok(num) = core::str::from_utf8(&w[16..]).unwrap_or("").parse::<u64>() {
                auto_record_frames = Some(num);
            }
        } else if w == b"--frames" {
            if let Some(next) = words.get(i + 1) {
                if let Ok(num) = core::str::from_utf8(next).unwrap_or("").parse::<u64>() {
                    max_frames = Some(num);
                    i += 1;
                }
            }
        } else if w == b"--synthetic" {
            force_synthetic = true;
        } else if w.starts_with(b"@/") {
            library_grant = Some(w.to_vec());
        } else if let Some(ep) = parse_endpoint(w) {
            net_endpoint = Some(ep);
        }
        i += 1;
    }

    if auto_record_frames.is_some() && max_frames.is_none() {
        max_frames = Some(auto_record_frames.unwrap() + 5);
    }

    // Ensure storage folders exist
    let _ = files::mkdir(&mut window, b"data");
    let _ = files::mkdir(&mut window, b"data/flash");
    let _ = files::mkdir(&mut window, b"library");
    let _ = files::mkdir(&mut window, b"data/flash/library");
    if let Some(ref grant) = library_grant {
        if grant.starts_with(b"@/") {
            let _ = files::mkdir(&mut window, &grant[1..]);
        }
        let _ = files::mkdir(&mut window, grant);
    }

    let mut socket_conn: Option<Socket> = None;
    let mut rx_buf = Vec::new();

    let mut frame_w = 640usize;
    let mut frame_h = 480usize;
    let mut frame_fps = 30u32;
    let mut frame_format = FormatKind::Yuy2;

    let mut synthetic_cam: Option<SyntheticUvcDevice> = None;

    if let (Some((ip, port)), false) = (net_endpoint, force_synthetic) {
        log!("[flash.connect] port={}", port);
        let up_start = sex_pdx::get_ticks();
        while sex_pdx::get_ticks().saturating_sub(up_start) < 25_000 {
            match sex_app::net::direct::up() {
                Ok(true) => break,
                Ok(false) => {
                    let _ = sex_pdx::sys_wait_self_ticks(5);
                }
                Err(e) => {
                    log!("[flash.connect.failed] up_error={:?}", e);
                    break;
                }
            }
        }

        let conn_start = sex_pdx::get_ticks();
        while socket_conn.is_none() && sex_pdx::get_ticks().saturating_sub(conn_start) < 15_000 {
            match Socket::connect(&ip, port) {
                Ok(sock) => {
                    let start = sex_pdx::get_ticks();
                    let mut connected = false;
                    while sex_pdx::get_ticks().saturating_sub(start) < 15_000 {
                        match sock.state() {
                            Ok(State::Open) => {
                                connected = true;
                                break;
                            }
                            Ok(State::Opening) => {
                                let _ = sex_pdx::sys_wait_self_ticks(2);
                            }
                            _ => break,
                        }
                    }

                    if connected {
                        socket_conn = Some(sock);
                    } else {
                        log!("[flash.connect.failed] timeout=1");
                        break;
                    }
                }
                Err(sex_app::net::Error::Offline) => {
                    let _ = sex_pdx::sys_wait_self_ticks(5);
                }
                Err(e) => {
                    log!("[flash.connect.failed] error={:?}", e);
                    break;
                }
            }
        }
    }

    // UVC Handshake or synthetic fallback
    if let Some(sock) = socket_conn.as_ref() {
        let mut got_desc = false;
        let mut wire_ok = true;
        let start = sex_pdx::get_ticks();
        while wire_ok && !got_desc && sex_pdx::get_ticks().saturating_sub(start) < 10000 {
            loop {
                match pop_wire_msg(&mut rx_buf) {
                    Ok(Some(WireMessage::Descriptors(data))) => {
                        if let Ok(dev) = parse_descriptors(&data) {
                            if let Some(stream) = dev.streaming.first() {
                                if let Some(fmt) = stream.formats.first() {
                                    if let Some(fr) = fmt.frames.first() {
                                        let w = fr.width as usize;
                                        let h = fr.height as usize;
                                        if valid_uvc_dims(w, h) {
                                            frame_format = fmt.kind;
                                            frame_w = w;
                                            frame_h = h;
                                            frame_fps = fr.default_fps().clamp(1, 120);
                                            got_desc = true;
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(()) => {
                        wire_ok = false;
                        break;
                    }
                }
            }
            if wire_ok && !got_desc {
                let mut tmp = [0u8; 4096];
                if let Ok(n) = sock.receive(&mut tmp) {
                    if n > 0 {
                        if !push_rx_bounded(&mut rx_buf, &tmp[..n]) {
                            wire_ok = false;
                        }
                    } else {
                        let _ = sex_pdx::sys_wait_self_ticks(2);
                    }
                }
            }
        }

        if wire_ok && got_desc {
            let mut ctrl = VideoProbeCommitControl::default();
            ctrl.set_fps(frame_fps);
            let _ = send_wire(sock, &WireMessage::ProbeReq(ctrl));

            let start = sex_pdx::get_ticks();
            let mut got_probe_resp = false;
            while wire_ok && !got_probe_resp && sex_pdx::get_ticks().saturating_sub(start) < 5000 {
                loop {
                    match pop_wire_msg(&mut rx_buf) {
                        Ok(Some(WireMessage::ProbeResp(resp))) => {
                            ctrl = resp;
                            got_probe_resp = true;
                            break;
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(()) => {
                            wire_ok = false;
                            break;
                        }
                    }
                }
                if wire_ok && !got_probe_resp {
                    let mut tmp = [0u8; 1024];
                    if let Ok(n) = sock.receive(&mut tmp) {
                        if n > 0 {
                            if !push_rx_bounded(&mut rx_buf, &tmp[..n]) {
                                wire_ok = false;
                            }
                        } else {
                            let _ = sex_pdx::sys_wait_self_ticks(2);
                        }
                    }
                }
            }

            let _ = send_wire(sock, &WireMessage::CommitReq(ctrl));
            let start = sex_pdx::get_ticks();
            let mut got_commit_ack = false;
            while wire_ok && !got_commit_ack && sex_pdx::get_ticks().saturating_sub(start) < 5000 {
                loop {
                    match pop_wire_msg(&mut rx_buf) {
                        Ok(Some(WireMessage::CommitAck(_))) => {
                            got_commit_ack = true;
                            break;
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(()) => {
                            wire_ok = false;
                            break;
                        }
                    }
                }
                if wire_ok && !got_commit_ack {
                    let mut tmp = [0u8; 1024];
                    if let Ok(n) = sock.receive(&mut tmp) {
                        if n > 0 {
                            if !push_rx_bounded(&mut rx_buf, &tmp[..n]) {
                                wire_ok = false;
                            }
                        } else {
                            let _ = sex_pdx::sys_wait_self_ticks(2);
                        }
                    }
                }
            }

            if wire_ok {
                let _ = send_wire(sock, &WireMessage::StartStream);
            } else {
                socket_conn = None;
            }
        } else {
            socket_conn = None;
        }
    }

    let usb_bank = if socket_conn.is_none() && !force_synthetic && net_endpoint.is_none() {
        sex_pdx::usb_uvc::FrameBank::open_reader().filter(|b| b.is_active())
    } else {
        None
    };
    let mut usb_raw_buf: Vec<u8> = Vec::new();
    let mut usb_last_seq: u32 = 0;
    let mut usb_decode_errors: u32 = 0;
    let mut usb_vendor: u16 = 0;
    let mut usb_product: u16 = 0;

    if let Some(bank) = usb_bank {
        if let Some(info) = bank.info() {
            let w = (info.width as usize).clamp(16, MAX_FRAME_DIM);
            let h = (info.height as usize).clamp(16, MAX_FRAME_DIM);
            frame_w = w;
            frame_h = h;
            frame_fps = (info.fps as u32).clamp(1, 120);
            frame_format = match info.format {
                sex_pdx::usb_uvc::FMT_MJPEG => FormatKind::Mjpeg,
                sex_pdx::usb_uvc::FMT_YUY2 => FormatKind::Yuy2,
                sex_pdx::usb_uvc::FMT_NV12 => FormatKind::Nv12,
                _ => FormatKind::Mjpeg,
            };
            usb_vendor = info.vendor;
            usb_product = info.product;
        }
        usb_raw_buf = vec![0u8; sex_pdx::usb_uvc::FRAME_SLOT_BYTES];
    } else if socket_conn.is_none() {
        if net_endpoint.is_some() && !force_synthetic {
            log!("[flash.failed] uvc_tcp_unreachable=1");
            sex_pdx::task_finish(1);
        }
        let syn = SyntheticUvcDevice::new(640, 480, 30);
        let desc_bytes = syn.binary_descriptors();
        if let Ok(dev) = parse_descriptors(&desc_bytes) {
            if let Some(stream) = dev.streaming.first() {
                if let Some(fmt) = stream.formats.first() {
                    if let Some(fr) = fmt.frames.first() {
                        let w = fr.width as usize;
                        let h = fr.height as usize;
                        if valid_uvc_dims(w, h) {
                            frame_format = fmt.kind;
                            frame_w = w;
                            frame_h = h;
                            frame_fps = fr.default_fps().clamp(1, 120);
                        }
                    }
                }
            }
        }
        synthetic_cam = Some(syn);
    }

    if usb_bank.is_some() {
        log!(
            "[flash.uvc.source] kind=usb vendor={:#06x} product={:#06x}",
            usb_vendor,
            usb_product
        );
    } else {
        log!(
            "[flash.uvc.source] kind={}",
            if socket_conn.is_some() { "tcp" } else { "synthetic" }
        );
    }
    log!(
        "[flash.uvc.negotiated] width={} height={} format={:?}",
        frame_w,
        frame_h,
        frame_format
    );

    let mut assembler = UvcStreamAssembler::new(4 * 1024 * 1024);
    let pixel_len = frame_w.saturating_mul(frame_h);
    let mut argb_pixels = vec![0u32; pixel_len];
    let mut filtered_pixels = vec![0u32; pixel_len];

    let mut active_filter = auto_filter.unwrap_or(Filter::Normal);
    log!("[flash.filter] mode=live filter={}", active_filter.name());

    let mut grid_mode = false;
    let mut grid_pixels = vec![0u32; pixel_len];

    let mut fps_window_start = sex_pdx::get_ticks();
    let mut fps_window_frames = 0u32;
    let mut measured_fps = 0u32;

    // Library State
    let mut next_photo_id = 1u32;
    let mut next_burst_id = 1u32;
    let mut library_thumbs: Vec<Thumbnail> = Vec::new();
    let mut preview_photo: Option<usize> = None;
    let mut gaze_cookie = 100u32;

    // Scan existing library photos
    let list_target = if let Some(ref grant) = library_grant {
        grant.as_slice()
    } else {
        b"library"
    };
    if let Ok(entries) = files::list(&mut window, list_target) {
        for e in entries {
            if e.dir || !e.name.ends_with(b".png") {
                continue;
            }
            if e.name.starts_with(b"photo-") && e.name.len() >= 10 {
                if let Ok(s) = core::str::from_utf8(&e.name[6..10]) {
                    if let Ok(num) = s.parse::<u32>() {
                        next_photo_id = next_photo_id.max(num + 1);
                    }
                }
            } else if e.name.starts_with(b"burst-") && e.name.len() >= 10 {
                if let Ok(s) = core::str::from_utf8(&e.name[6..10]) {
                    if let Ok(num) = s.parse::<u32>() {
                        next_burst_id = next_burst_id.max(num + 1);
                    }
                }
            }
        }
    }

    let mut frame_count = 0u64;
    let mut single_shot_trigger = auto_photo;
    let mut countdown_step = if auto_timer { 3u8 } else { 0u8 };
    let mut step_due_ticks = if auto_timer { sex_pdx::get_ticks() + 900 } else { 0u64 };
    if auto_timer {
        log!("[flash.timer] countdown=3");
    }

    let mut burst_active = auto_burst;
    let mut burst_shots_taken = 0usize;
    let mut burst_frames: Vec<Vec<u32>> = Vec::new();
    let mut next_burst_ticks = if auto_burst { sex_pdx::get_ticks() } else { 0u64 };

    let mut shutter_flash_frames = 0u8;

    let mut recording = auto_record_frames.is_some();
    let mut record_frames_done = 0u64;
    let mut video_exporter: Option<Exporter> = None;
    let mut video_bytes = Vec::new();

    if recording {
        match Exporter::new(Config {
            format: Format::WebmVp9Opus,
            width: frame_w as u32,
            height: frame_h as u32,
            fps_num: frame_fps,
            fps_den: 1,
            sound: false,
            quality: 32,
        }) {
            Ok(exp) => video_exporter = Some(exp),
            Err(e) => {
                log!("[flash.video.unavailable] error={:?}", e);
                recording = false;
            }
        }
    }

    let mut running = true;
    let mut last_logged_layout = 0u32;
    let mut splash_drawn = false;
    let mut prev_mouse_down = false;

    while running {
        // Poll window events
        while let Some(ev) = window.poll() {
            match ev {
                Event::Close => {
                    running = false;
                    break;
                }
                Event::Key(k) => {
                    if k.code == 0x01 || k.code == 0x57 {
                        // Escape or F11
                        if preview_photo.is_some() {
                            preview_photo = None;
                        } else if grid_mode {
                            grid_mode = false;
                            log!("[flash.filter] mode=live filter={}", active_filter.name());
                        } else {
                            running = false;
                            break;
                        }
                    } else if k.code == 0x53 || k.code == 0x0E {
                        // Delete / Backspace: remove previewed photo
                        if let Some(idx) = preview_photo {
                            if idx < library_thumbs.len() {
                                let path = library_thumbs[idx].path.clone();
                                let _ = files::remove(&mut window, &path);
                                let name = library_thumbs[idx].name.clone();
                                let mut rel = Vec::new();
                                rel.extend_from_slice(b"library/");
                                rel.extend_from_slice(&name);
                                let _ = files::remove(&mut window, &rel);
                                if let Some(ref grant) = library_grant {
                                    let mut g_path = grant.clone();
                                    g_path.push(b'/');
                                    g_path.extend_from_slice(&name);
                                    let _ = files::remove(&mut window, &g_path);
                                }
                                library_thumbs.remove(idx);
                                log!("[flash.library.deleted] name={}", core::str::from_utf8(&name).unwrap_or("?"));
                            }
                            preview_photo = None;
                        }
                    } else if let Some(ch) = k.char().map(|c| c.to_ascii_lowercase()) {
                        match ch {
                            ' ' => {
                                if preview_photo.is_some() {
                                    preview_photo = None;
                                } else {
                                    single_shot_trigger = true;
                                }
                            }
                            'e' => {
                                grid_mode = !grid_mode;
                                preview_photo = None;
                                if grid_mode {
                                    log!("[flash.filter] mode=grid filter={}", active_filter.name());
                                } else {
                                    log!("[flash.filter] mode=live filter={}", active_filter.name());
                                }
                            }
                            'f' => {
                                active_filter = active_filter.next();
                                grid_mode = false;
                                preview_photo = None;
                                log!("[flash.filter] mode=live filter={}", active_filter.name());
                            }
                            't' => {
                                if countdown_step == 0 && !burst_active {
                                    countdown_step = 3;
                                    step_due_ticks = sex_pdx::get_ticks() + 900;
                                    preview_photo = None;
                                    log!("[flash.timer] countdown=3");
                                }
                            }
                            'b' => {
                                if !burst_active && countdown_step == 0 {
                                    burst_active = true;
                                    burst_shots_taken = 0;
                                    burst_frames.clear();
                                    next_burst_ticks = sex_pdx::get_ticks();
                                    preview_photo = None;
                                }
                            }
                            'v' | 'r' => {
                                if !recording {
                                    match Exporter::new(Config {
                                        format: Format::WebmVp9Opus,
                                        width: frame_w as u32,
                                        height: frame_h as u32,
                                        fps_num: frame_fps,
                                        fps_den: 1,
                                        sound: false,
                                        quality: 32,
                                    }) {
                                        Ok(exp) => {
                                            video_exporter = Some(exp);
                                            video_bytes.clear();
                                            record_frames_done = 0;
                                            recording = true;
                                        }
                                        Err(e) => {
                                            log!("[flash.video.unavailable] error={:?}", e);
                                        }
                                    }
                                } else if let Some(mut exp) = video_exporter.take() {
                                    let _ = exp.finish();
                                    drain_exporter(&mut exp, &mut video_bytes);
                                    let _ = files::write(&mut window, b"library/video-0001.webm", &video_bytes);
                                    let _ = files::write(&mut window, b"data/flash/library/video-0001.webm", &video_bytes);
                                    let _ = files::write(&mut window, b"data/flash/video-0001.webm", &video_bytes);
                                    let _ = files::write(&mut window, b"video-0001.webm", &video_bytes);
                                    log!(
                                        "[flash.video] path=/data/flash/video-0001.webm bytes={} frames={}",
                                        video_bytes.len(),
                                        record_frames_done
                                    );
                                    recording = false;
                                }
                            }
                            'g' => {
                                // Open in Gaze
                                if let Some(ref grant) = library_grant {
                                    gaze_cookie = gaze_cookie.wrapping_add(1);
                                    let abs_path = if grant.starts_with(b"@/") {
                                        &grant[1..]
                                    } else {
                                        grant.as_slice()
                                    };
                                    let mut open_buf = [0u8; sex_pdx::fs::ARGS_MAX];
                                    if let Some(n) = sex_pdx::launch::open_args(abs_path, &mut open_buf) {
                                        let res = sex_app::launch::run(b"gaze", &open_buf[..n], false, gaze_cookie)
                                            .or_else(|_| sex_app::launch::run(b"view", &open_buf[..n], false, gaze_cookie));
                                        match res {
                                            Ok(_) => log!("[flash.gaze.launched] path={}", core::str::from_utf8(abs_path).unwrap_or("?")),
                                            Err(e) => log!("[flash.gaze.failed] error={:?}", e),
                                        }
                                    }
                                } else {
                                    log!("[flash.gaze.hint] cmd=run gaze @/data/flash/library");
                                }
                            }
                            '1'..='9' => {
                                let idx = (ch as usize) - ('1' as usize);
                                if idx < Filter::ALL.len() {
                                    active_filter = Filter::ALL[idx];
                                    grid_mode = false;
                                    preview_photo = None;
                                    log!("[flash.filter] mode=live filter={}", active_filter.name());
                                }
                            }
                            _ => {}
                        }
                    } else if k.code >= 0x02 && k.code <= 0x0A {
                        // Numeric row 1..9
                        let idx = (k.code - 0x02) as usize;
                        if idx < Filter::ALL.len() {
                            active_filter = Filter::ALL[idx];
                            grid_mode = false;
                            preview_photo = None;
                            log!("[flash.filter] mode=live filter={}", active_filter.name());
                        }
                    }
                }
                Event::Pointer(p) => {
                    let is_click = p.buttons != 0 && !prev_mouse_down;
                    prev_mouse_down = p.buttons != 0;

                    if is_click {
                        let px = p.x.max(0) as usize;
                        let py = p.y.max(0) as usize;
                        let (win_w, win_h) = window.size();
                        let (win_w, win_h) = (win_w.max(640), win_h.max(480));

                        let top_h = 36usize;
                        let film_h = 64usize;
                        let ctrl_h = 42usize;
                        let vp_y = top_h;
                        let vp_h = win_h.saturating_sub(top_h + ctrl_h + film_h);
                        let vp_x = 0usize;
                        let vp_w = win_w;

                        if grid_mode && px >= vp_x && px < vp_x + vp_w && py >= vp_y && py < vp_y + vp_h {
                            // Clicked 3x3 grid tile
                            let tile_w = vp_w / 3;
                            let tile_h = vp_h / 3;
                            if tile_w > 0 && tile_h > 0 {
                                let col = ((px - vp_x) / tile_w).min(2);
                                let row = ((py - vp_y) / tile_h).min(2);
                                let idx = row * 3 + col;
                                if idx < Filter::ALL.len() {
                                    active_filter = Filter::ALL[idx];
                                    grid_mode = false;
                                    log!("[flash.filter] mode=live filter={}", active_filter.name());
                                }
                            }
                        } else if py >= win_h - film_h {
                            // Clicked bottom filmstrip
                            let thumb_y = win_h - film_h + 10;
                            let card_w = 60usize;
                            let gap = 8usize;
                            let start_x = 16usize;

                            for (i, _) in library_thumbs.iter().enumerate() {
                                let card_x = start_x + i * (card_w + gap);
                                if px >= card_x && px < card_x + card_w && py >= thumb_y && py < thumb_y + 44 {
                                    preview_photo = Some(i);
                                    grid_mode = false;
                                    break;
                                }
                            }
                        } else {
                            // Control Bar clicks
                            let ctrl_y = win_h - film_h - ctrl_h;
                            let btn_w_shutter = 104usize;
                            let btn_w_other = 88usize;
                            let btn_gap = 8usize;
                            let total_w = btn_w_other * 5 + btn_w_shutter + 110 + btn_gap * 6;
                            let start_bx = (win_w.saturating_sub(total_w)) / 2;

                            let mut bx = start_bx;
                            let btn_h = 30usize;
                            let by = ctrl_y + 6;

                            // 1. Effects
                            if Rect::new(bx, by, btn_w_other, btn_h).contains(px, py) {
                                grid_mode = !grid_mode;
                                preview_photo = None;
                                if grid_mode {
                                    log!("[flash.filter] mode=grid filter={}", active_filter.name());
                                } else {
                                    log!("[flash.filter] mode=live filter={}", active_filter.name());
                                }
                            }
                            bx += btn_w_other + btn_gap;

                            // 2. Filter
                            if Rect::new(bx, by, btn_w_other, btn_h).contains(px, py) {
                                active_filter = active_filter.next();
                                grid_mode = false;
                                preview_photo = None;
                                log!("[flash.filter] mode=live filter={}", active_filter.name());
                            }
                            bx += btn_w_other + btn_gap;

                            // 3. Shutter
                            if Rect::new(bx, by, btn_w_shutter, btn_h).contains(px, py) {
                                if preview_photo.is_some() {
                                    preview_photo = None;
                                } else {
                                    single_shot_trigger = true;
                                }
                            }
                            bx += btn_w_shutter + btn_gap;

                            // 4. Timer 3s
                            if Rect::new(bx, by, btn_w_other, btn_h).contains(px, py) {
                                if countdown_step == 0 && !burst_active {
                                    countdown_step = 3;
                                    step_due_ticks = sex_pdx::get_ticks() + 900;
                                    preview_photo = None;
                                    log!("[flash.timer] countdown=3");
                                }
                            }
                            bx += btn_w_other + btn_gap;

                            // 5. Burst x4
                            if Rect::new(bx, by, btn_w_other, btn_h).contains(px, py) {
                                if !burst_active && countdown_step == 0 {
                                    burst_active = true;
                                    burst_shots_taken = 0;
                                    burst_frames.clear();
                                    next_burst_ticks = sex_pdx::get_ticks();
                                    preview_photo = None;
                                }
                            }
                            bx += btn_w_other + btn_gap;

                            // 6. Video
                            if Rect::new(bx, by, btn_w_other, btn_h).contains(px, py) {
                                if !recording {
                                    if let Ok(exp) = Exporter::new(Config {
                                        format: Format::WebmVp9Opus,
                                        width: frame_w as u32,
                                        height: frame_h as u32,
                                        fps_num: frame_fps,
                                        fps_den: 1,
                                        sound: false,
                                        quality: 32,
                                    }) {
                                        video_exporter = Some(exp);
                                        video_bytes.clear();
                                        record_frames_done = 0;
                                        recording = true;
                                    }
                                } else if let Some(mut exp) = video_exporter.take() {
                                    let _ = exp.finish();
                                    drain_exporter(&mut exp, &mut video_bytes);
                                    let _ = files::write(&mut window, b"library/video-0001.webm", &video_bytes);
                                    let _ = files::write(&mut window, b"data/flash/library/video-0001.webm", &video_bytes);
                                    let _ = files::write(&mut window, b"data/flash/video-0001.webm", &video_bytes);
                                    let _ = files::write(&mut window, b"video-0001.webm", &video_bytes);
                                    log!(
                                        "[flash.video] path=/data/flash/video-0001.webm bytes={} frames={}",
                                        video_bytes.len(),
                                        record_frames_done
                                    );
                                    recording = false;
                                }
                            }
                            bx += btn_w_other + btn_gap;

                            // 7. Open in Gaze
                            if Rect::new(bx, by, 110, btn_h).contains(px, py) {
                                if let Some(ref grant) = library_grant {
                                    gaze_cookie = gaze_cookie.wrapping_add(1);
                                    let abs_path = if grant.starts_with(b"@/") {
                                        &grant[1..]
                                    } else {
                                        grant.as_slice()
                                    };
                                    let mut open_buf = [0u8; sex_pdx::fs::ARGS_MAX];
                                    if let Some(n) = sex_pdx::launch::open_args(abs_path, &mut open_buf) {
                                        let res = sex_app::launch::run(b"gaze", &open_buf[..n], false, gaze_cookie)
                                            .or_else(|_| sex_app::launch::run(b"view", &open_buf[..n], false, gaze_cookie));
                                        match res {
                                            Ok(_) => log!("[flash.gaze.launched] path={}", core::str::from_utf8(abs_path).unwrap_or("?")),
                                            Err(e) => log!("[flash.gaze.failed] error={:?}", e),
                                        }
                                    }
                                } else {
                                    log!("[flash.gaze.hint] cmd=run gaze @/data/flash/library");
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if !running {
            break;
        }

        // Receive UVC frame (USB FrameBank, TCP socket, or synthetic fallback)
        let mut completed_frame = None;
        let mut usb_frame_decoded = false;

        if let Some(bank) = usb_bank {
            if let Some(meta) = bank.read_frame(usb_last_seq, &mut usb_raw_buf) {
                usb_last_seq = meta.seq;
                let fmt = match meta.format {
                    sex_pdx::usb_uvc::FMT_MJPEG => FormatKind::Mjpeg,
                    sex_pdx::usb_uvc::FMT_YUY2 => FormatKind::Yuy2,
                    sex_pdx::usb_uvc::FMT_NV12 => FormatKind::Nv12,
                    _ => FormatKind::Mjpeg,
                };
                let raw = &usb_raw_buf[..meta.len];
                let (dec_w, dec_h) = if fmt == FormatKind::Mjpeg {
                    match sex_image::dimensions(raw) {
                        Ok((_, w, h)) if w >= 16 && h >= 16 && w <= MAX_FRAME_DIM && h <= MAX_FRAME_DIM => (w, h),
                        _ => (meta.width as usize, meta.height as usize),
                    }
                } else {
                    (meta.width as usize, meta.height as usize)
                };
                let needed = dec_w.saturating_mul(dec_h);
                if needed > 0 && needed <= MAX_FRAME_DIM * MAX_FRAME_DIM {
                    if argb_pixels.len() != needed {
                        argb_pixels.resize(needed, 0);
                        filtered_pixels.resize(needed, 0);
                    }
                    match decode_frame(fmt, raw, dec_w, dec_h, &mut argb_pixels) {
                        Ok((w, h)) => {
                            frame_w = w;
                            frame_h = h;
                            frame_format = fmt;
                            usb_frame_decoded = true;
                        }
                        Err(e) => {
                            usb_decode_errors = usb_decode_errors.saturating_add(1);
                            if usb_decode_errors <= 5 || usb_decode_errors.is_power_of_two() {
                                log!(
                                    "[flash.usb.decode_err] seq={} len={} fmt={:?} err={:?}",
                                    meta.seq,
                                    meta.len,
                                    fmt,
                                    e
                                );
                            }
                        }
                    }
                }
            }
        } else if let Some(sock) = socket_conn.as_ref() {
            let mut read_buf = [0u8; 16384];
            for _ in 0..96 {
                loop {
                    match pop_wire_msg(&mut rx_buf) {
                        Ok(Some(WireMessage::UvcPacket(pkt))) => {
                            if let Ok(Some(cf)) = assembler.push_packet(&pkt) {
                                completed_frame = Some(cf);
                                break;
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(()) => {
                            running = false;
                            break;
                        }
                    }
                }
                if completed_frame.is_some() || !running {
                    break;
                }
                match sock.receive(&mut read_buf) {
                    Ok(n) if n > 0 => {
                        if !push_rx_bounded(&mut rx_buf, &read_buf[..n]) {
                            running = false;
                            break;
                        }
                    }
                    _ => break,
                }
            }
        } else if let Some(syn) = synthetic_cam.as_mut() {
            let packets = syn.next_frame_packets();
            for pkt in &packets {
                if let Ok(Some(cf)) = assembler.push_packet(pkt) {
                    completed_frame = Some(cf);
                    break;
                }
            }
        }

        if let Some(ref frame) = completed_frame {
            // Decode TCP/synthetic UVC frame into argb_pixels
            let _ = decode_frame(frame_format, &frame.data, frame_w, frame_h, &mut argb_pixels);
        }

        if usb_frame_decoded || completed_frame.is_some() {
            frame_count += 1;
            if usb_bank.is_none() || frame_count <= 8 || frame_count.is_power_of_two() {
                log!("[flash.frame] n={}", frame_count);
            }

            let now = sex_pdx::get_ticks();
            fps_window_frames = fps_window_frames.saturating_add(1);
            let elapsed = now.saturating_sub(fps_window_start);
            if elapsed >= 1000 {
                measured_fps = (fps_window_frames as u64 * 1000 / elapsed.max(1)) as u32;
                fps_window_frames = 0;
                fps_window_start = now;
            }

            // Apply active filter to camera frame
            apply_filter(active_filter, &argb_pixels, &mut filtered_pixels, frame_w, frame_h);

            // 1. Countdown Timer State Machine
            if countdown_step > 0 && now >= step_due_ticks {
                if countdown_step == 3 {
                    countdown_step = 2;
                    step_due_ticks = now + 900;
                    log!("[flash.timer] countdown=2");
                } else if countdown_step == 2 {
                    countdown_step = 1;
                    step_due_ticks = now + 900;
                    log!("[flash.timer] countdown=1");
                } else if countdown_step == 1 {
                    countdown_step = 0;
                    shutter_flash_frames = 2;

                    // Save timer photo
                    let mut id_buf = [0u8; 4];
                    format_id(next_photo_id, &mut id_buf);
                    next_photo_id += 1;

                    let mut filename = Vec::new();
                    filename.extend_from_slice(b"photo-");
                    filename.extend_from_slice(&id_buf);
                    filename.extend_from_slice(b".png");

                    if let Ok(png_data) = encode_png(&filtered_pixels, frame_w, frame_h) {
                        let mut lib_path = Vec::new();
                        lib_path.extend_from_slice(b"library/");
                        lib_path.extend_from_slice(&filename);
                        let _ = files::write(&mut window, &lib_path, &png_data);

                        let mut full_path = Vec::new();
                        full_path.extend_from_slice(b"data/flash/library/");
                        full_path.extend_from_slice(&filename);
                        let _ = files::write(&mut window, &full_path, &png_data);
                        let _ = files::write(&mut window, b"data/flash/photo-0001.png", &png_data);
                        let _ = files::write(&mut window, b"photo-0001.png", &png_data);

                        if let Some(ref grant) = library_grant {
                            let mut g_path = grant.clone();
                            g_path.push(b'/');
                            g_path.extend_from_slice(&filename);
                            let _ = files::write(&mut window, &g_path, &png_data);
                        }

                        log!(
                            "[flash.photo] mode=timer filter={} path=/data/flash/library/{} bytes={}",
                            active_filter.name(),
                            core::str::from_utf8(&filename).unwrap_or("?"),
                            png_data.len()
                        );

                        // Downsample thumbnail
                        let (tw, th) = (52, 38);
                        let mut thumb_px = vec![0u32; tw * th];
                        for ty in 0..th {
                            let sy = (ty * frame_h) / th;
                            for tx in 0..tw {
                                let sx = (tx * frame_w) / tw;
                                thumb_px[ty * tw + tx] = filtered_pixels[sy * frame_w + sx];
                            }
                        }
                        library_thumbs.insert(0, Thumbnail {
                            name: filename,
                            path: lib_path,
                            w: tw,
                            h: th,
                            pixels: thumb_px,
                            is_burst: false,
                        });
                    }
                }
            }

            // 2. 4-Shot Burst State Machine
            if burst_active && now >= next_burst_ticks {
                burst_shots_taken += 1;
                shutter_flash_frames = 2;
                burst_frames.push(filtered_pixels.clone());

                let mut id_buf = [0u8; 4];
                format_id(next_photo_id, &mut id_buf);
                next_photo_id += 1;

                let mut filename = Vec::new();
                filename.extend_from_slice(b"photo-");
                filename.extend_from_slice(&id_buf);
                filename.extend_from_slice(b".png");

                if let Ok(png_data) = encode_png(&filtered_pixels, frame_w, frame_h) {
                    let mut lib_path = Vec::new();
                    lib_path.extend_from_slice(b"library/");
                    lib_path.extend_from_slice(&filename);
                    let _ = files::write(&mut window, &lib_path, &png_data);

                    let mut full_path = Vec::new();
                    full_path.extend_from_slice(b"data/flash/library/");
                    full_path.extend_from_slice(&filename);
                    let _ = files::write(&mut window, &full_path, &png_data);

                    if let Some(ref grant) = library_grant {
                        let mut g_path = grant.clone();
                        g_path.push(b'/');
                        g_path.extend_from_slice(&filename);
                        let _ = files::write(&mut window, &g_path, &png_data);
                    }

                    log!(
                        "[flash.photo] mode=burst filter={} path=/data/flash/library/{} bytes={}",
                        active_filter.name(),
                        core::str::from_utf8(&filename).unwrap_or("?"),
                        png_data.len()
                    );
                }

                if burst_shots_taken < 4 {
                    next_burst_ticks = now + 150;
                } else {
                    burst_active = false;
                    // Compose 2x2 Contact Sheet
                    let (cw, ch, sheet) = compose_burst_contact_sheet(
                        [&burst_frames[0], &burst_frames[1], &burst_frames[2], &burst_frames[3]],
                        frame_w,
                        frame_h,
                    );

                    let mut c_id_buf = [0u8; 4];
                    format_id(next_burst_id, &mut c_id_buf);
                    next_burst_id += 1;

                    let mut c_name = Vec::new();
                    c_name.extend_from_slice(b"burst-");
                    c_name.extend_from_slice(&c_id_buf);
                    c_name.extend_from_slice(b".png");

                    if let Ok(png_sheet) = encode_png(&sheet, cw, ch) {
                        let mut lib_path = Vec::new();
                        lib_path.extend_from_slice(b"library/");
                        lib_path.extend_from_slice(&c_name);
                        let _ = files::write(&mut window, &lib_path, &png_sheet);

                        let mut full_path = Vec::new();
                        full_path.extend_from_slice(b"data/flash/library/");
                        full_path.extend_from_slice(&c_name);
                        let _ = files::write(&mut window, &full_path, &png_sheet);

                        if let Some(ref grant) = library_grant {
                            let mut g_path = grant.clone();
                            g_path.push(b'/');
                            g_path.extend_from_slice(&c_name);
                            let _ = files::write(&mut window, &g_path, &png_sheet);
                        }

                        log!(
                            "[flash.burst] count=4 contact=/data/flash/library/{} bytes={}",
                            core::str::from_utf8(&c_name).unwrap_or("?"),
                            png_sheet.len()
                        );

                        // Thumbnail for contact sheet
                        let (tw, th) = (52, 38);
                        let mut thumb_px = vec![0u32; tw * th];
                        for ty in 0..th {
                            let sy = (ty * ch) / th;
                            for tx in 0..tw {
                                let sx = (tx * cw) / tw;
                                thumb_px[ty * tw + tx] = sheet[sy * cw + sx];
                            }
                        }
                        library_thumbs.insert(0, Thumbnail {
                            name: c_name,
                            path: lib_path,
                            w: tw,
                            h: th,
                            pixels: thumb_px,
                            is_burst: true,
                        });
                    }
                }
            }

            // 3. Single Shot Photo Capture
            if single_shot_trigger {
                single_shot_trigger = false;
                shutter_flash_frames = 2;

                let mut id_buf = [0u8; 4];
                format_id(next_photo_id, &mut id_buf);
                next_photo_id += 1;

                let mut filename = Vec::new();
                filename.extend_from_slice(b"photo-");
                filename.extend_from_slice(&id_buf);
                filename.extend_from_slice(b".png");

                if let Ok(png_data) = encode_png(&filtered_pixels, frame_w, frame_h) {
                    let mut lib_path = Vec::new();
                    lib_path.extend_from_slice(b"library/");
                    lib_path.extend_from_slice(&filename);
                    let _ = files::write(&mut window, &lib_path, &png_data);

                    let mut full_path = Vec::new();
                    full_path.extend_from_slice(b"data/flash/library/");
                    full_path.extend_from_slice(&filename);
                    let _ = files::write(&mut window, &full_path, &png_data);
                    let _ = files::write(&mut window, b"data/flash/photo-0001.png", &png_data);
                    let _ = files::write(&mut window, b"photo-0001.png", &png_data);

                    if let Some(ref grant) = library_grant {
                        let mut g_path = grant.clone();
                        g_path.push(b'/');
                        g_path.extend_from_slice(&filename);
                        let _ = files::write(&mut window, &g_path, &png_data);
                    }

                    log!(
                        "[flash.photo] mode=single filter={} path=/data/flash/library/{} bytes={}",
                        active_filter.name(),
                        core::str::from_utf8(&filename).unwrap_or("?"),
                        png_data.len()
                    );
                    log!(
                        "[flash.photo] path=/data/flash/photo-0001.png bytes={}",
                        png_data.len()
                    );

                    let (tw, th) = (52, 38);
                    let mut thumb_px = vec![0u32; tw * th];
                    for ty in 0..th {
                        let sy = (ty * frame_h) / th;
                        for tx in 0..tw {
                            let sx = (tx * frame_w) / tw;
                            thumb_px[ty * tw + tx] = filtered_pixels[sy * frame_w + sx];
                        }
                    }
                    library_thumbs.insert(0, Thumbnail {
                        name: filename,
                        path: lib_path,
                        w: tw,
                        h: th,
                        pixels: thumb_px,
                        is_burst: false,
                    });
                }
            }

            // 4. Video Recording with active filter
            if recording {
                if let Some(exp) = video_exporter.as_mut() {
                    let _ = exp.picture(&filtered_pixels, record_frames_done);
                    drain_exporter(exp, &mut video_bytes);
                    record_frames_done += 1;

                    if let Some(target) = auto_record_frames {
                        if record_frames_done >= target {
                            if let Some(mut exp) = video_exporter.take() {
                                let _ = exp.finish();
                                drain_exporter(&mut exp, &mut video_bytes);
                                let _ = files::write(&mut window, b"library/video-0001.webm", &video_bytes);
                                let _ = files::write(&mut window, b"data/flash/library/video-0001.webm", &video_bytes);
                                let _ = files::write(&mut window, b"data/flash/video-0001.webm", &video_bytes);
                                let _ = files::write(&mut window, b"video-0001.webm", &video_bytes);
                                log!(
                                    "[flash.video] path=/data/flash/video-0001.webm bytes={} frames={}",
                                    video_bytes.len(),
                                    record_frames_done
                                );
                            }
                            recording = false;
                        }
                    }
                }
            }

            // 5. Window UI Presentation
            let (win_w, win_h) = window.size();
            let (win_w, win_h) = (win_w.max(640), win_h.max(480));
            let win_pixels = window.pixels();
            let total_pixels = win_w * win_h;

            if win_pixels.len() >= total_pixels {
                // Background dark matte
                let bg_col = 0x0014161E;
                win_pixels[..total_pixels].fill(bg_col);

                let top_h = 36usize;
                let film_h = 64usize;
                let ctrl_h = 42usize;

                // Viewport area
                let vp_x = 0usize;
                let vp_y = top_h;
                let vp_w = win_w;
                let vp_h = win_h.saturating_sub(top_h + ctrl_h + film_h);

                use sex_graphics::theme::candy_ui::{
                    button_text_color, draw_candy_badge, draw_candy_button, draw_candy_panel,
                    draw_candy_toolbar, ButtonState, ButtonStyle, CandyPalette,
                };
                let pal = CandyPalette::dark();

                // Top Toolbar
                draw_candy_toolbar(win_pixels, win_w, win_h, 0, 0, win_w as i32, top_h as i32, &pal);
                // Title
                draw_text_8x8(win_pixels, win_w, win_h, 16, 14, b"FLASH PHOTO BOOTH", pal.text, None);
                // Camera status dot & live telemetry overlay (source + resolution + FPS)
                let is_live = usb_bank.is_some() || socket_conn.is_some();
                let dot_col = if is_live { pal.mint } else { pal.accent };
                draw_circle(win_pixels, win_w, win_h, 172, 18, 4, dot_col);
                let src_label = if usb_bank.is_some() {
                    "LIVE USB"
                } else if socket_conn.is_some() {
                    "LIVE TCP"
                } else {
                    "SYNTHETIC"
                };
                let fps_show = if measured_fps > 0 { measured_fps } else { frame_fps };
                let status_line = alloc::format!("{} {}x{} {}fps", src_label, frame_w, frame_h, fps_show);
                draw_text_8x8(
                    win_pixels,
                    win_w,
                    win_h,
                    182,
                    14,
                    status_line.as_bytes(),
                    if is_live { pal.mint } else { pal.text_dim },
                    None,
                );

                // Filter badge pill on right
                let filter_str = active_filter.name().as_bytes();
                let pill_w = filter_str.len() * 8 + 20;
                let pill_x = win_w.saturating_sub(pill_w + 16);
                draw_candy_badge(win_pixels, win_w, win_h, pill_x as i32, 8, pill_w as i32, 20, pal.control, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, pill_x + 10, 14, filter_str, pal.text, None);

                // Viewport rendering
                if let Some(idx) = preview_photo {
                    // Previewing an inspected library photo
                    if idx < library_thumbs.len() {
                        let thumb = &library_thumbs[idx];
                        // Centered display
                        let fit_w = (vp_w - 32).min(frame_w);
                        let fit_h = (vp_h - 32).min(frame_h);
                        let bx = vp_x + (vp_w.saturating_sub(fit_w)) / 2;
                        let by = vp_y + (vp_h.saturating_sub(fit_h)) / 2;
                        scale_blit(&thumb.pixels, thumb.w, thumb.h, win_pixels, win_w, win_h, Rect::new(bx, by, fit_w, fit_h));
                        draw_rect_outline(win_pixels, win_w, win_h, Rect::new(bx, by, fit_w, fit_h), 0x003B82F6, 2);

                        // Banner hint
                        draw_text_8x8(
                            win_pixels,
                            win_w,
                            win_h,
                            bx + 12,
                            by + 12,
                            b"INSPECTING PHOTO - [Del] Delete, [Esc] Return to Camera",
                            0x00FFFFFF,
                            Some(0x000F172A),
                        );
                    }
                } else if grid_mode {
                    // 3x3 Live Effects Grid
                    if grid_pixels.len() != vp_w * vp_h {
                        grid_pixels.resize(vp_w * vp_h, 0);
                    }
                    render_effects_grid(&argb_pixels, frame_w, frame_h, &mut grid_pixels, vp_w, vp_h);
                    for cy in 0..vp_h {
                        let dst_row = (vp_y + cy) * win_w + vp_x;
                        let src_row = cy * vp_w;
                        win_pixels[dst_row..dst_row + vp_w].copy_from_slice(&grid_pixels[src_row..src_row + vp_w]);
                    }
                } else {
                    // Live Camera Viewfinder with active filter (aspect-ratio preserving letterbox)
                    let (fit_w, fit_h) = if frame_w > 0 && frame_h > 0 && vp_w > 0 && vp_h > 0 {
                        if frame_w.saturating_mul(vp_h) >= frame_h.saturating_mul(vp_w) {
                            (vp_w, (frame_h.saturating_mul(vp_w) / frame_w).max(1).min(vp_h))
                        } else {
                            ((frame_w.saturating_mul(vp_h) / frame_h).max(1).min(vp_w), vp_h)
                        }
                    } else {
                        (vp_w, vp_h)
                    };
                    let fit_x = vp_x + (vp_w.saturating_sub(fit_w)) / 2;
                    let fit_y = vp_y + (vp_h.saturating_sub(fit_h)) / 2;
                    scale_blit(&filtered_pixels, frame_w, frame_h, win_pixels, win_w, win_h, Rect::new(fit_x, fit_y, fit_w, fit_h));

                    // Shutter flash effect
                    if shutter_flash_frames > 0 {
                        fill_rect(win_pixels, win_w, win_h, Rect::new(vp_x, vp_y, vp_w, vp_h), 0x00FFFFFF);
                        shutter_flash_frames -= 1;
                    }

                    // Countdown Timer Badge Overlay
                    if countdown_step > 0 {
                        let cx = vp_x + vp_w / 2;
                        let cy = vp_y + vp_h / 2;
                        draw_circle(win_pixels, win_w, win_h, cx, cy, 48, 0x000F172A);
                        draw_big_digit(
                            win_pixels,
                            win_w,
                            win_h,
                            cx,
                            cy - 4,
                            b'0' + countdown_step,
                            5,
                            0x00FFFFFF,
                        );

                        // Amber indicator pips
                        let pip_y = cy + 28;
                        let amber = 0x00F59E0B;
                        let dim = 0x00475569;
                        draw_circle(win_pixels, win_w, win_h, cx - 18, pip_y, 4, if countdown_step >= 1 { amber } else { dim });
                        draw_circle(win_pixels, win_w, win_h, cx, pip_y, 4, if countdown_step >= 2 { amber } else { dim });
                        draw_circle(win_pixels, win_w, win_h, cx + 18, pip_y, 4, if countdown_step >= 3 { amber } else { dim });
                    }
                }

                // Control Bar
                let ctrl_y = win_h - film_h - ctrl_h;
                draw_candy_toolbar(win_pixels, win_w, win_h, 0, ctrl_y as i32, win_w as i32, ctrl_h as i32, &pal);

                let btn_w_shutter = 104usize;
                let btn_w_other = 88usize;
                let btn_gap = 8usize;
                let total_btn_w = btn_w_other * 5 + btn_w_shutter + 110 + btn_gap * 6;
                let start_bx = (win_w.saturating_sub(total_btn_w)) / 2;
                let btn_h = 30usize;
                let by = ctrl_y + 6;
                let btn_r = 10i32;

                let mut cur_bx = start_bx;

                // Log controls once or on layout change
                let current_layout_hash = (win_w as u32) ^ ((win_h as u32) << 16) ^ (if grid_mode { 1 } else { 0 });
                let should_log_controls = last_logged_layout != current_layout_hash;
                if should_log_controls {
                    last_logged_layout = current_layout_hash;
                }

                // 1. Effects
                let fx_style = if grid_mode { ButtonStyle::Active } else { ButtonStyle::Normal };
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, btn_w_other as i32, btn_h as i32, btn_r, fx_style, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 10, by + 11, b"Effects (E)", button_text_color(fx_style, &pal), None);
                if should_log_controls {
                    log!("[flash.control] name=effects x={} y={} w={} h={}", cur_bx, by, btn_w_other, btn_h);
                }
                cur_bx += btn_w_other + btn_gap;

                // 2. Filter
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, btn_w_other as i32, btn_h as i32, btn_r, ButtonStyle::Normal, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 14, by + 11, b"Filter (F)", pal.text, None);
                if should_log_controls {
                    log!("[flash.control] name=filter x={} y={} w={} h={}", cur_bx, by, btn_w_other, btn_h);
                }
                cur_bx += btn_w_other + btn_gap;

                // 3. Shutter
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, btn_w_shutter as i32, btn_h as i32, (btn_h / 2) as i32, ButtonStyle::Danger, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 12, by + 11, b"Shutter [Spc]", button_text_color(ButtonStyle::Danger, &pal), None);
                if should_log_controls {
                    log!("[flash.control] name=shutter x={} y={} w={} h={}", cur_bx, by, btn_w_shutter, btn_h);
                }
                cur_bx += btn_w_shutter + btn_gap;

                // 4. Timer
                let timer_style = if countdown_step > 0 { ButtonStyle::Active } else { ButtonStyle::Normal };
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, btn_w_other as i32, btn_h as i32, btn_r, timer_style, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 10, by + 11, b"Timer 3s(T)", button_text_color(timer_style, &pal), None);
                if should_log_controls {
                    log!("[flash.control] name=timer x={} y={} w={} h={}", cur_bx, by, btn_w_other, btn_h);
                }
                cur_bx += btn_w_other + btn_gap;

                // 5. Burst
                let burst_style = if burst_active { ButtonStyle::Active } else { ButtonStyle::Normal };
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, btn_w_other as i32, btn_h as i32, btn_r, burst_style, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 12, by + 11, b"Burst x4(B)", button_text_color(burst_style, &pal), None);
                if should_log_controls {
                    log!("[flash.control] name=burst x={} y={} w={} h={}", cur_bx, by, btn_w_other, btn_h);
                }
                cur_bx += btn_w_other + btn_gap;

                // 6. Video
                let vid_style = if recording { ButtonStyle::Danger } else { ButtonStyle::Normal };
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, btn_w_other as i32, btn_h as i32, btn_r, vid_style, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 16, by + 11, if recording { b"REC..." } else { b"Video (V)" }, button_text_color(vid_style, &pal), None);
                if should_log_controls {
                    log!("[flash.control] name=video x={} y={} w={} h={}", cur_bx, by, btn_w_other, btn_h);
                }
                cur_bx += btn_w_other + btn_gap;

                // 7. Open in Gaze
                draw_candy_button(win_pixels, win_w, win_h, cur_bx as i32, by as i32, 110, btn_h as i32, btn_r, ButtonStyle::Primary, ButtonState::Rest, &pal);
                draw_text_8x8(win_pixels, win_w, win_h, cur_bx + 10, by + 11, b"Open in Gaze(G)", button_text_color(ButtonStyle::Primary, &pal), None);
                if should_log_controls {
                    log!("[flash.control] name=open_gaze x={} y={} w={} h={}", cur_bx, by, 110, btn_h);
                }

                // If in grid mode, log tile controls for automated clicking
                if grid_mode && should_log_controls {
                    let tile_w = vp_w / 3;
                    let tile_h = vp_h / 3;
                    for r_i in 0..3 {
                        for c_i in 0..3 {
                            let idx = r_i * 3 + c_i + 1;
                            let tx = vp_x + c_i * tile_w;
                            let ty = vp_y + r_i * tile_h;
                            log!("[flash.control] name=filter_{} x={} y={} w={} h={}", idx, tx, ty, tile_w, tile_h);
                        }
                    }
                }

                // Bottom Filmstrip
                let film_y = win_h - film_h;
                fill_rect(win_pixels, win_w, win_h, Rect::new(0, film_y, win_w, film_h), pal.bg);
                // Divider line
                fill_rect(win_pixels, win_w, win_h, Rect::new(0, film_y, win_w, 1), pal.hairline);

                let card_w = 60usize;
                let card_h = 44usize;
                let thumb_y = film_y + 10;
                let start_x = 16usize;

                for (i, thumb) in library_thumbs.iter().enumerate() {
                    let card_x = start_x + i * (card_w + 8);
                    if card_x + card_w >= win_w {
                        break;
                    }
                    let is_sel = preview_photo == Some(i);
                    // Rounded Candy card background
                    if is_sel {
                        draw_candy_button(win_pixels, win_w, win_h, card_x as i32, thumb_y as i32, card_w as i32, card_h as i32, 8, ButtonStyle::Active, ButtonState::Rest, &pal);
                    } else {
                        draw_candy_panel(win_pixels, win_w, win_h, card_x as i32, thumb_y as i32, card_w as i32, card_h as i32, 8, &pal);
                    }
                    // Blit thumbnail
                    let img_x = card_x + 4;
                    let img_y = thumb_y + 3;
                    scale_blit(&thumb.pixels, thumb.w, thumb.h, win_pixels, win_w, win_h, Rect::new(img_x, img_y, card_w - 8, card_h - 6));

                    if should_log_controls {
                        log!("[flash.control] name=thumb_{} x={} y={} w={} h={}", i, card_x, thumb_y, card_w, card_h);
                    }
                }

                window.show_pixels(win_w, win_h);
            }

            if let Some(max_f) = max_frames {
                if frame_count >= max_f {
                    running = false;
                }
            }
        } else {
            if frame_count == 0 && !splash_drawn && window.ready() {
                let (win_w, win_h) = window.size();
                if win_w >= 64 && win_h >= 64 {
                    let pal = sex_theme::candy_ui::CandyPalette::dark();
                    let win_pixels = window.frame(win_w, win_h);
                    win_pixels.fill(pal.bg);
                    let title = b"FLASH PHOTO BOOTH";
                    let sub = if usb_bank.is_some() {
                        b"CONNECTING TO USB WEBCAM..." as &[u8]
                    } else {
                        b"INITIALIZING CAMERA..." as &[u8]
                    };
                    let tx = win_w.saturating_sub(title.len() * 8) / 2;
                    let sx = win_w.saturating_sub(sub.len() * 8) / 2;
                    let cy = win_h / 2;
                    draw_text_8x8(win_pixels, win_w, win_h, tx, cy.saturating_sub(14), title, pal.accent);
                    draw_text_8x8(win_pixels, win_w, win_h, sx, cy + 8, sub, pal.text_dim);
                    window.show_pixels(win_w, win_h);
                    splash_drawn = true;
                }
            }
            let _ = sex_pdx::sys_wait_self_ticks(1);
        }
    }

    // Flush any remaining active recording
    if let Some(mut exp) = video_exporter.take() {
        let _ = exp.finish();
        drain_exporter(&mut exp, &mut video_bytes);
        let _ = files::write(&mut window, b"library/video-0001.webm", &video_bytes);
        let _ = files::write(&mut window, b"data/flash/library/video-0001.webm", &video_bytes);
        let _ = files::write(&mut window, b"data/flash/video-0001.webm", &video_bytes);
        let _ = files::write(&mut window, b"video-0001.webm", &video_bytes);
        log!(
            "[flash.video] path=/data/flash/video-0001.webm bytes={} frames={}",
            video_bytes.len(),
            record_frames_done
        );
    }

    log!("[flash.done] total_frames={}", frame_count);
    sex_pdx::task_finish(0)
}
