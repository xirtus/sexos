// SPDX-License-Identifier: GPL-3.0-only
use sex_icons::maya;

#[test]
fn every_day_sign_draws_with_open_holes() {
    const S: usize = 96;
    let mut acc = vec![0i32; sex_font::raster::scratch_len(S, S)];
    let (mut mask, mut out) = (vec![0u8; S * S], vec![0u32; S * S]);
    for seal in 1..=20u8 {
        let (back, ink) = maya::sign(seal).expect("sign");
        assert!(sex_icons::walk(back, &mut Count(0)) && sex_icons::walk(ink, &mut Count(0)), "seal {seal} parses");
        assert!(maya::draw(seal, S, 0xD8453E, 0x1C1A1B, &mut acc, &mut mask, &mut out), "seal {seal}");
        let solid = |p: u32| p >> 24 == 255;
        let colour = out.iter().filter(|&&p| solid(p) && p & 0xFF_FFFF == 0xD8453E).count();
        let ink_px = out.iter().filter(|&&p| solid(p) && p & 0xFF_FFFF == 0x1C1A1B).count();
        // Mostly cartouche colour with the drawing on it: the holes in the ink stay open.
        assert!(colour > S * S / 4, "seal {seal}: {colour} colour pixels");
        assert!(ink_px > S * S / 30 && ink_px < colour, "seal {seal}: {ink_px} ink pixels");
        // The corners stay clear (the silhouette is not a square).
        assert_eq!(out[0] >> 24, 0, "seal {seal}");
    }
    assert!(maya::sign(0).is_none() && maya::sign(21).is_none());
}

struct Count(usize);
impl sex_icons::Sink for Count {
    fn begin(&mut self, _: f32, _: f32, _: bool) { self.0 += 1; }
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn cubic_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
    fn circle(&mut self, _: f32, _: f32, _: f32, _: bool) {}
}
