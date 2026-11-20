// SPDX-License-Identifier: GPL-3.0-only
//! USB webcams (sexuvc, the x17's IR/RGB camera): the class descriptors the camera
//! reports, its probe/commit answer, the isochronous payload stream (headers,
//! frame toggles, errors) and the pixels of each frame (YUY2, NV12, MJPEG).
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_uvc::{decode_frame, parse_descriptors, FormatKind, SyntheticUvcDevice, UvcStreamAssembler, VideoProbeCommitControl};
use sexos_driver_fuzz::Src;

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let desc = s.chunk();
    if let Ok(dev) = parse_descriptors(desc) {
        for st in &dev.streaming {
            for f in &st.formats { for fr in &f.frames { let _ = (fr.width, fr.height, f.kind.clone()); } }
        }
    }
    let _ = VideoProbeCommitControl::decode(s.chunk());
    let (w, h) = (1 + s.below(64), 1 + s.below(48));
    let kind = match s.below(3) { 0 => FormatKind::Yuy2, 1 => FormatKind::Nv12, _ => FormatKind::Mjpeg };
    let mut asm = UvcStreamAssembler::new(1 + s.below(64 * 1024));
    let mut out = vec![0u32; w * h];
    while !s.is_empty() {
        let packet = s.chunk();
        if let Ok(Some(frame)) = asm.push_packet(packet) {
            let _ = decode_frame(kind.clone(), &frame.data, w, h, &mut out);
        }
    }
}

fn seeds() -> Vec<Vec<u8>> {
    let cam = SyntheticUvcDevice::new(32, 24, 30);
    let desc = cam.binary_descriptors();
    let mut v = (desc.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&desc);
    let probe = VideoProbeCommitControl::default().encode(true);
    v.extend_from_slice(&(probe.len() as u16).to_le_bytes());
    v.extend_from_slice(&probe);
    v.extend_from_slice(&[31, 0, 0, 0, 23, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0]);
    let frame = cam.generate_raw_yuy2_frame(1);
    for (i, part) in frame.chunks(512).enumerate() {
        let last = (i + 1) * 512 >= frame.len();
        let mut p = vec![2u8, 0x80 | if last { 0x02 } else { 0 }];
        p.extend_from_slice(part);
        v.extend_from_slice(&(p.len() as u16).to_le_bytes());
        v.extend_from_slice(&p);
    }
    vec![v]
}

sexos_driver_fuzz::driver_target!(run, seeds);
