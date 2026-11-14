// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_image::{Animation, Limits};

fuzz_target!(|data: &[u8]| {
    // 1. sex-image base decoder with SURFACE limits
    let _ = sex_image::decode(data, &Limits::SURFACE);

    // 2. Animated image parser (GIF/WebP frames)
    if let Ok(mut anim) = Animation::open(data, &Limits::SURFACE) {
        let _ = anim.frames();
        let _ = anim.loops();
        let pixel_count = anim.width.saturating_mul(anim.height);
        if pixel_count > 0 && pixel_count <= Limits::SURFACE.max_pixels {
            let mut canvas = vec![0u32; pixel_count];
            let _ = anim.next_into(&mut canvas);
        }
    }

    // 3. gaze-codecs extended formats: sniff, exif, raw preview, and decode
    let _ = gaze_codecs::sniff(data);
    let _ = gaze_codecs::find_exif(data);
    let _ = gaze_codecs::extract_preview(data);
    let _ = gaze_codecs::decode(data, &Limits::SURFACE);
});
