mod common;

use common::*;
use gaze_codecs::{apply_orientation, decode, decode_with, orient_image, Image, Limits, Options};

/// Goldens for the 3x2 picture [10 20 30 / 40 50 60]; each was checked
/// against ImageMagick (`convert -orient <EXIF name> -auto-orient`).
const GOLDEN: [(u8, usize, usize, [u8; 6]); 8] = [
    (1, 3, 2, [10, 20, 30, 40, 50, 60]),
    (2, 3, 2, [30, 20, 10, 60, 50, 40]),
    (3, 3, 2, [60, 50, 40, 30, 20, 10]),
    (4, 3, 2, [40, 50, 60, 10, 20, 30]),
    (5, 2, 3, [10, 40, 20, 50, 30, 60]),
    (6, 2, 3, [40, 10, 50, 20, 60, 30]),
    (7, 2, 3, [60, 30, 50, 20, 40, 10]),
    (8, 2, 3, [30, 60, 20, 50, 10, 40]),
];

#[test]
fn all_eight_orientations() {
    let src = [10u8, 20, 30, 40, 50, 60];
    for (o, w, h, want) in GOLDEN {
        let (got, gw, gh) = apply_orientation(&src, 3, 2, o).unwrap();
        assert_eq!((gw, gh, &got[..]), (w, h, &want[..]), "orientation {o}");
        // Same with RGBA pixels.
        let px: Vec<[u8; 4]> = src.iter().map(|&v| [v, v + 1, v + 2, 255]).collect();
        let (got, _, _) = apply_orientation(&px, 3, 2, o).unwrap();
        assert_eq!(got.iter().map(|p| p[0]).collect::<Vec<_>>(), want, "rgba {o}");
    }
    // Out of range behaves as 1.
    assert_eq!(apply_orientation(&src, 3, 2, 0).unwrap().0, src);
    assert_eq!(apply_orientation(&src, 3, 2, 9).unwrap().0, src);
    // Too few pixels is refused, empty is fine.
    assert!(apply_orientation(&src, 4, 2, 6).is_err());
    assert_eq!(apply_orientation::<u8>(&[], 0, 0, 6).unwrap(), (vec![], 0, 0));
}

#[test]
fn orientations_compose_like_the_dihedral_group() {
    // Four 90-degree turns restore the picture; 6 then 8 too; 2, 4, 5, 7 are their own inverse.
    let w = 5;
    let h = 3;
    let src: Vec<u32> = (0..15).collect();
    let turn = |p: &[u32], w, h, o| apply_orientation(p, w, h, o).unwrap();
    let (a, aw, ah) = turn(&src, w, h, 6);
    let (b, bw, bh) = turn(&a, aw, ah, 6);
    assert_eq!(b, turn(&src, w, h, 3).0);
    let (c, cw, ch) = turn(&b, bw, bh, 6);
    let (d, _, _) = turn(&c, cw, ch, 6);
    assert_eq!(d, src);
    assert_eq!(turn(&a, aw, ah, 8).0, src);
    for o in [2, 3, 4, 5, 7] {
        let (x, xw, xh) = turn(&src, w, h, o);
        assert_eq!(turn(&x, xw, xh, o).0, src, "{o}");
    }
}

#[test]
fn rotated_exif_jpeg_shows_upright() {
    let plain = sex_image::decode(SMALL_JPEG, &Limits::PHOTO).unwrap();
    assert_eq!((plain.width, plain.height), (64, 48));
    for o in 1..=8u8 {
        let file = jpeg_with_exif(SMALL_JPEG, &orientation_tiff(o as u16));
        let got = decode(&file, &Limits::PHOTO).unwrap();
        let want = orient_image(plain.clone(), o).unwrap();
        assert_eq!(got, want, "orientation {o}");
        if o >= 5 {
            assert_eq!((got.width, got.height), (48, 64));
        }
        // auto_orient off leaves it as stored.
        let raw = decode_with(&file, &Options { auto_orient: false, ..Options::default() }).unwrap();
        assert_eq!(raw, plain);
    }
}

#[test]
fn png_exif_orientation() {
    // 2x1 red, blue; orientation 6 makes it 1x2 red over blue.
    let file = png(&[[255, 0, 0, 255], [0, 0, 255, 255]], 2, 1, Some(&orientation_tiff(6)));
    let img = decode(&file, &Limits::PHOTO).unwrap();
    assert_eq!(img, Image { width: 1, height: 2, pixels: vec![0xffff_0000, 0xff00_00ff] });
    assert_eq!(rgba(img.pixels[1]), [0, 0, 255, 255]);
}
