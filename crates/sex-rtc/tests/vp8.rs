// SPDX-License-Identifier: GPL-3.0-only
// The VP8 encoder against real decoders: image-webp for a key frame, and the ffmpeg
// binary (libvpx's or FFmpeg's own decoder) for a moving sequence, which must decode to
// the encoder's own reconstruction bit for bit.
use sex_rtc::vp8::{Encoder, Planes};
use std::process::Command;

fn pattern(w: usize, h: usize, t: usize) -> Vec<u32> {
    let mut px = vec![0u32; w * h];
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w) as u32;
            let g = (y * 255 / h) as u32;
            let b = ((x + y) * 3 % 256) as u32;
            px[y * w + x] = 0xff00_0000 | r << 16 | g << 8 | b;
        }
    }
    // A white box that moves, as the fake camera's does.
    let x0 = (t * 6) % (w - 24);
    for y in 10..34 { for x in x0..x0 + 24 { px[y * w + x] = 0xffff_ffff; } }
    px
}

fn ivf(frames: &[Vec<u8>], w: usize, h: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"DKIF");
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(b"VP80");
    v.extend_from_slice(&(w as u16).to_le_bytes());
    v.extend_from_slice(&(h as u16).to_le_bytes());
    v.extend_from_slice(&30u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        v.extend_from_slice(&(f.len() as u32).to_le_bytes());
        v.extend_from_slice(&(i as u64).to_le_bytes());
        v.extend_from_slice(f);
    }
    v
}

fn crop(p: &Planes, w: usize, h: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h { v.extend_from_slice(&p.y[y * p.w..y * p.w + w]); }
    for plane in [&p.u, &p.v] { for y in 0..h / 2 { v.extend_from_slice(&plane[y * p.w / 2..y * p.w / 2 + w / 2]); } }
    v
}

#[test]
fn key_frame_decodes_with_image_webp() {
    let (w, h) = (100, 60);
    let mut e = Encoder::new(w, h);
    let f = e.encode_argb(&pattern(w, h, 0), w, h);
    assert!(f.keyframe);
    let mut webp = Vec::new();
    webp.extend_from_slice(b"RIFF");
    webp.extend_from_slice(&((4 + 8 + f.data.len() + (f.data.len() & 1)) as u32).to_le_bytes());
    webp.extend_from_slice(b"WEBPVP8 ");
    webp.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
    webp.extend_from_slice(&f.data);
    if f.data.len() & 1 == 1 { webp.push(0); }
    let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(webp)).unwrap();
    assert_eq!(d.dimensions(), (w as u32, h as u32));
    let mut rgb = vec![0u8; w * h * 3];
    d.read_image(&mut rgb).unwrap();
    // Close to the source (lossy), and the box is there.
    let src = pattern(w, h, 0);
    let mut err = 0u64;
    for i in 0..w * h {
        let s = src[i];
        for (k, sh) in [16, 8, 0].into_iter().enumerate() { err += (((s >> sh) & 255) as i64 - rgb[i * 3 + k] as i64).unsigned_abs(); }
    }
    let mean = err as f64 / (w * h * 3) as f64;
    assert!(mean < 8.0, "mean error {mean}");
    assert!(rgb[(20 * w + 5) * 3] > 220);
}

#[test]
fn moving_sequence_matches_ffmpeg_bit_for_bit() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let (w, h) = (168, 120); // not macroblock aligned
    let mut e = Encoder::new(w, h);
    e.set_rate(300_000, 15);
    let mut frames = Vec::new();
    let mut recon = Vec::new();
    let mut keys = 0;
    for t in 0..24 {
        if t == 12 { e.request_keyframe(); }
        let f = e.encode_argb(&pattern(w, h, t), w, h);
        keys += f.keyframe as usize;
        frames.push(f.data);
        recon.extend(crop(e.reconstruction(), w, h));
    }
    assert_eq!(keys, 2);
    // Inter frames of a mostly still picture are small.
    assert!(frames[5].len() * 3 < frames[0].len(), "{} vs {}", frames[5].len(), frames[0].len());
    let dir = std::env::temp_dir().join(format!("sex-rtc-vp8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, out) = (dir.join("in.ivf"), dir.join("out.yuv"));
    std::fs::write(&src, ivf(&frames, w, h)).unwrap();
    // FFmpeg's own decoder always; libvpx's too when this ffmpeg has it.
    for decoder in ["vp8", "libvpx"] {
        let st = Command::new("ffmpeg").args(["-v", "error", "-y", "-c:v", decoder, "-i"]).arg(&src)
            .args(["-f", "rawvideo", "-pix_fmt", "yuv420p"]).arg(&out).status().unwrap();
        if !st.success() && decoder == "libvpx" { eprintln!("no libvpx decoder"); continue; }
        assert!(st.success());
        let got = std::fs::read(&out).unwrap();
        assert_eq!(got.len(), recon.len());
        let frame = w * h * 3 / 2;
        for t in 0..24 {
            let (a, b) = (&got[t * frame..(t + 1) * frame], &recon[t * frame..(t + 1) * frame]);
            let diff = a.iter().zip(b).filter(|(x, y)| x != y).count();
            assert_eq!(diff, 0, "{decoder} frame {t}: {diff} bytes differ");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
