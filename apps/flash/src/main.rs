// SPDX-License-Identifier: GPL-3.0-only
//! Flash: USB UVC Webcam Capture App (Photo PNG + Splicy WebM Video).
//!
//! Connects to a UVC stream (via 10.0.2.2:PORT over TCP, or in-process Synthetic camera),
//! performs descriptor discovery, Probe/Commit negotiation, and multi-packet stream
//! reassembly. Displays live viewfinder, saves photos as PNG via sex-graphics/sex-image,
//! and records WebM (VP9+Opus) video through Splicy's sex_media::export::Exporter.

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

fn drain_exporter(exp: &mut Exporter, out: &mut Vec<u8>) {
    loop {
        match exp.output() {
            Ok(Drain::Write(w)) => {
                let token = w.token;
                let offset = w.offset as usize;
                let bytes = w.bytes;
                if offset + bytes.len() > out.len() {
                    out.resize(offset + bytes.len(), 0);
                }
                out[offset..offset + bytes.len()].copy_from_slice(bytes);
                let _ = exp.acknowledge(token, bytes.len());
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
    let s = text.strip_prefix(b"uvc://")
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

#[no_mangle]
pub extern "C" fn _start() -> ! {
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
    let mut auto_record_frames: Option<u64> = None;
    let mut max_frames: Option<u64> = None;
    let mut net_endpoint: Option<(IpAddr, u16)> = None;
    let mut force_synthetic = false;

    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w == b"--photo" {
            auto_photo = true;
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
        } else if let Some(ep) = parse_endpoint(w) {
            net_endpoint = Some(ep);
        }
        i += 1;
    }

    if auto_record_frames.is_some() && max_frames.is_none() {
        max_frames = Some(auto_record_frames.unwrap() + 5);
    }

    let mut socket_conn: Option<Socket> = None;
    let mut rx_buf = Vec::new();

    let mut frame_w = 640usize;
    let mut frame_h = 480usize;
    let mut frame_fps = 30u32;
    let mut frame_format = FormatKind::Yuy2;

    let mut synthetic_cam: Option<SyntheticUvcDevice> = None;

    if let (Some((ip, port)), false) = (net_endpoint, force_synthetic) {
        log!("[flash.connect] target={}:{}", ip, port);
        match Socket::connect(&ip, port) {
            Ok(sock) => {
                let start = sex_pdx::get_ticks();
                let mut connected = false;
                while sex_pdx::get_ticks().saturating_sub(start) < 5000 {
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
                }
            }
            Err(_) => {
                log!("[flash.connect.failed] error=socket");
            }
        }
    }

    // Handshake or synthetic fallback
    if let Some(sock) = socket_conn.as_ref() {
        // Wait for Descriptors
        let mut got_desc = false;
        let start = sex_pdx::get_ticks();
        while !got_desc && sex_pdx::get_ticks().saturating_sub(start) < 10000 {
            while let Ok(Some((msg, len))) = WireMessage::decode_one(&rx_buf) {
                rx_buf.drain(..len);
                if let WireMessage::Descriptors(data) = msg {
                    if let Ok(dev) = parse_descriptors(&data) {
                        if let Some(stream) = dev.streaming.first() {
                            if let Some(fmt) = stream.formats.first() {
                                frame_format = fmt.kind;
                                if let Some(fr) = fmt.frames.first() {
                                    frame_w = fr.width as usize;
                                    frame_h = fr.height as usize;
                                    frame_fps = fr.default_fps().max(1);
                                }
                            }
                        }
                    }
                    got_desc = true;
                    break;
                }
            }
            if !got_desc {
                let mut tmp = [0u8; 4096];
                if let Ok(n) = sock.receive(&mut tmp) {
                    if n > 0 {
                        rx_buf.extend_from_slice(&tmp[..n]);
                    } else {
                        let _ = sex_pdx::sys_wait_self_ticks(2);
                    }
                }
            }
        }

        if got_desc {
            let mut ctrl = VideoProbeCommitControl::default();
            ctrl.set_fps(frame_fps);
            let _ = send_wire(sock, &WireMessage::ProbeReq(ctrl));

            // Wait for ProbeResp
            let start = sex_pdx::get_ticks();
            let mut got_probe_resp = false;
            while !got_probe_resp && sex_pdx::get_ticks().saturating_sub(start) < 5000 {
                while let Ok(Some((msg, len))) = WireMessage::decode_one(&rx_buf) {
                    rx_buf.drain(..len);
                    if let WireMessage::ProbeResp(resp) = msg {
                        ctrl = resp;
                        got_probe_resp = true;
                        break;
                    }
                }
                if !got_probe_resp {
                    let mut tmp = [0u8; 1024];
                    if let Ok(n) = sock.receive(&mut tmp) {
                        if n > 0 {
                            rx_buf.extend_from_slice(&tmp[..n]);
                        } else {
                            let _ = sex_pdx::sys_wait_self_ticks(2);
                        }
                    }
                }
            }

            let _ = send_wire(sock, &WireMessage::CommitReq(ctrl));
            // Wait for CommitAck
            let start = sex_pdx::get_ticks();
            let mut got_commit_ack = false;
            while !got_commit_ack && sex_pdx::get_ticks().saturating_sub(start) < 5000 {
                while let Ok(Some((msg, len))) = WireMessage::decode_one(&rx_buf) {
                    rx_buf.drain(..len);
                    if let WireMessage::CommitAck(_) = msg {
                        got_commit_ack = true;
                        break;
                    }
                }
                if !got_commit_ack {
                    let mut tmp = [0u8; 1024];
                    if let Ok(n) = sock.receive(&mut tmp) {
                        if n > 0 {
                            rx_buf.extend_from_slice(&tmp[..n]);
                        } else {
                            let _ = sex_pdx::sys_wait_self_ticks(2);
                        }
                    }
                }
            }

            let _ = send_wire(sock, &WireMessage::StartStream);
        } else {
            // Fallback to synthetic camera if handshake failed
            socket_conn = None;
        }
    }

    if socket_conn.is_none() {
        let syn = SyntheticUvcDevice::new(640, 480, 30);
        let desc_bytes = syn.binary_descriptors();
        if let Ok(dev) = parse_descriptors(&desc_bytes) {
            if let Some(stream) = dev.streaming.first() {
                if let Some(fmt) = stream.formats.first() {
                    frame_format = fmt.kind;
                    if let Some(fr) = fmt.frames.first() {
                        frame_w = fr.width as usize;
                        frame_h = fr.height as usize;
                        frame_fps = fr.default_fps().max(1);
                    }
                }
            }
        }
        synthetic_cam = Some(syn);
    }

    log!(
        "[flash.uvc.negotiated] width={} height={} format={:?}",
        frame_w,
        frame_h,
        frame_format
    );

    let mut assembler = UvcStreamAssembler::new(4 * 1024 * 1024);
    let mut argb_pixels = vec![0u32; frame_w * frame_h];

    let mut frame_count = 0u64;
    let mut photo_taken = false;
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

    let mut photo_trigger = auto_photo;
    let mut running = true;

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
                        running = false;
                        break;
                    } else if k.code == 25 || k.code == 0x70 || k.code == 0x50 {
                        // 'P' key -> Photo shutter
                        photo_trigger = true;
                    } else if k.code == 19 || k.code == 0x72 || k.code == 0x52 {
                        // 'R' key -> Toggle record
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
                        } else {
                            // Finish recording
                            if let Some(mut exp) = video_exporter.take() {
                                let _ = exp.finish();
                                drain_exporter(&mut exp, &mut video_bytes);
                                let _ = files::mkdir(&mut window, b"data");
                                let _ = files::mkdir(&mut window, b"data/flash");
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
                Event::Pointer(p) => {
                    if p.buttons != 0 {
                        // Click -> Shutter
                        photo_trigger = true;
                    }
                }
                _ => {}
            }
        }

        if !running {
            break;
        }

        // Get UVC packets
        let mut completed_frame = None;

        if let Some(sock) = socket_conn.as_ref() {
            let mut read_buf = [0u8; 8192];
            if let Ok(n) = sock.receive(&mut read_buf) {
                if n > 0 {
                    rx_buf.extend_from_slice(&read_buf[..n]);
                }
            }

            while let Ok(Some((msg, len))) = WireMessage::decode_one(&rx_buf) {
                rx_buf.drain(..len);
                if let WireMessage::UvcPacket(pkt) = msg {
                    if let Ok(Some(cf)) = assembler.push_packet(&pkt) {
                        completed_frame = Some(cf);
                        break;
                    }
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

        if let Some(frame) = completed_frame {
            frame_count += 1;
            log!("[flash.frame] n={}", frame_count);

            // Decode to ARGB
            if let Ok((w, h)) = decode_frame(frame_format, &frame.data, frame_w, frame_h, &mut argb_pixels) {
                let win_pixels = window.pixels();
                let count = (w * h).min(win_pixels.len());
                win_pixels[..count].copy_from_slice(&argb_pixels[..count]);
                window.show_pixels(w, h);
            }

            // Photo Capture
            if photo_trigger && !photo_taken {
                photo_trigger = false;
                if let Ok(png_data) = encode_png(&argb_pixels, frame_w, frame_h) {
                    let _ = files::mkdir(&mut window, b"data");
                    let _ = files::mkdir(&mut window, b"data/flash");
                    let _ = files::write(&mut window, b"data/flash/photo-0001.png", &png_data);
                    let _ = files::write(&mut window, b"photo-0001.png", &png_data);
                    log!(
                        "[flash.photo] path=/data/flash/photo-0001.png bytes={}",
                        png_data.len()
                    );
                    photo_taken = true;
                }
            }

            // Video Recording
            if recording {
                if let Some(exp) = video_exporter.as_mut() {
                    let _ = exp.picture(&argb_pixels, record_frames_done);
                    drain_exporter(exp, &mut video_bytes);
                    record_frames_done += 1;

                    if let Some(target) = auto_record_frames {
                        if record_frames_done >= target {
                            if let Some(mut exp) = video_exporter.take() {
                                let _ = exp.finish();
                                drain_exporter(&mut exp, &mut video_bytes);
                                let _ = files::mkdir(&mut window, b"data");
                                let _ = files::mkdir(&mut window, b"data/flash");
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

            if let Some(max_f) = max_frames {
                if frame_count >= max_f {
                    running = false;
                }
            }
        } else {
            let _ = sex_pdx::sys_wait_self_ticks(1);
        }
    }

    // Flush any remaining active recording
    if let Some(mut exp) = video_exporter.take() {
        let _ = exp.finish();
        drain_exporter(&mut exp, &mut video_bytes);
        let _ = files::mkdir(&mut window, b"data");
        let _ = files::mkdir(&mut window, b"data/flash");
        let _ = files::write(&mut window, b"data/flash/video-0001.webm", &video_bytes);
        let _ = files::write(&mut window, b"video-0001.webm", &video_bytes);
        log!(
            "[flash.video] path=/data/flash/video-0001.webm bytes={} frames={}",
            video_bytes.len(),
            record_frames_done
        );
    }

    sex_pdx::task_finish(0)
}
