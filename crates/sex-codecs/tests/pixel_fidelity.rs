// SPDX-License-Identifier: GPL-3.0-only
//! Mandatory pixel fidelity audit test for WebP, GIF, HEIC, and AVIF decoders.
//! Verifies that each codec decodes real compressed bitstreams into accurate RGB pixels.

use sex_codecs::{decode, Limits};
use sex_codecs::image::unpack_argb;

#[test]
fn pixel_fidelity_quadrant_all_four_formats() {
    let formats: &[(&str, &[u8])] = &[
        ("webp", include_bytes!("fixtures/quadrant.webp")),
        ("gif", include_bytes!("fixtures/quadrant.gif")),
        ("heic", include_bytes!("fixtures/quadrant.heic")),
        ("avif", include_bytes!("fixtures/quadrant.avif")),
    ];

    let limits = Limits::PHOTO;
    for (name, data) in formats {
        let img = decode(data, &limits).unwrap_or_else(|e| panic!("Failed to decode {}: {:?}", name, e));
        assert_eq!(img.width, 64, "{name} width mismatch");
        assert_eq!(img.height, 64, "{name} height mismatch");
        assert_eq!(img.pixels.len(), 64 * 64, "{name} pixel buffer length mismatch");

        // TL center pixel (16, 16): Red
        let px_tl = img.pixels[16 * 64 + 16];
        let [_a, r_tl, g_tl, b_tl] = unpack_argb(px_tl);
        println!("{name} TL (16,16): R={r_tl}, G={g_tl}, B={b_tl}");
        assert!(
            r_tl > 200 && g_tl < 55 && b_tl < 55,
            "{name} TL failed fidelity: R={r_tl}, G={g_tl}, B={b_tl} (expected R>200, G<55, B<55)"
        );

        // TR center pixel (48, 16): Green
        let px_tr = img.pixels[16 * 64 + 48];
        let [_a, r_tr, g_tr, b_tr] = unpack_argb(px_tr);
        println!("{name} TR (48,16): R={r_tr}, G={g_tr}, B={b_tr}");
        assert!(
            g_tr > 200 && r_tr < 55 && b_tr < 55,
            "{name} TR failed fidelity: R={r_tr}, G={g_tr}, B={b_tr} (expected G>200, R<55, B<55)"
        );

        // BL center pixel (16, 48): Blue
        let px_bl = img.pixels[48 * 64 + 16];
        let [_a, r_bl, g_bl, b_bl] = unpack_argb(px_bl);
        println!("{name} BL (16,48): R={r_bl}, G={g_bl}, B={b_bl}");
        assert!(
            b_bl > 200 && r_bl < 55 && g_bl < 55,
            "{name} BL failed fidelity: R={r_bl}, G={g_bl}, B={b_bl} (expected B>200, R<55, G<55)"
        );

        // BR center pixel (48, 48): White
        let px_br = img.pixels[48 * 64 + 48];
        let [_a, r_br, g_br, b_br] = unpack_argb(px_br);
        println!("{name} BR (48,48): R={r_br}, G={g_br}, B={b_br}");
        assert!(
            r_br > 200 && g_br > 200 && b_br > 200,
            "{name} BR failed fidelity: R={r_br}, G={g_br}, B={b_br} (expected R>200, G>200, B>200)"
        );
    }
}
