//! AVIF pictures: recognised by their `ftyp` brand and handed to the program's decoder.

const STILL: &[u8] = include_bytes!("../../../scripts/web_site/still.avif");

#[test]
fn avif_is_recognised_by_its_brand() {
    assert!(sex_web::is_avif(STILL));
    // An MP4 video is not a picture.
    let mp4 = b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2";
    assert!(!sex_web::is_avif(mp4));
    // A compatible brand is enough.
    let compatible = b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif";
    assert!(sex_web::is_avif(compatible));
    assert!(!sex_web::is_avif(b"\x89PNG\r\n\x1a\n"));
}

fn solid(_: &[u8], w: usize, h: usize) -> Option<sex_web::Image> {
    let (w, h) = (w.min(96), h.min(48));
    Some(sex_web::Image { width: w, height: h, pixels: vec![0xFFE0_7020; w * h] })
}

#[test]
fn avif_goes_to_the_decoder_the_program_set() {
    sex_web::set_avif_decoder(solid);
    let img = sex_web::decode_image(STILL, 50, 50).unwrap();
    assert_eq!((img.width, img.height), (50, 48));
    assert_eq!(img.pixels[0], 0xFFE0_7020);
}
