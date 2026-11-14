#![cfg(feature = "svg")]

mod common;

use common::rgba;
use gaze_codecs::{decode, decode_with, render_svg, sniff, svg_fit, svg_size, Error, Format, Limits, Options};

/// Geist (SIL OFL 1.1), the font sex-font bundles.
const GEIST: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Regular.ttf");

/// 200x100: left half red, right half a blue circle on a transparent ground,
/// a half-transparent green bar across the bottom quarter.
const SHAPES: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 20 10">
  <rect x="0" y="0" width="10" height="10" fill="#ff0000"/>
  <circle cx="15" cy="5" r="4" fill="#0000ff"/>
  <rect x="0" y="7.5" width="20" height="2.5" fill="#00ff00" fill-opacity="0.5"/>
</svg>"##;

#[test]
fn shapes_at_known_spots() {
    assert_eq!(sniff(SHAPES), Some(Format::Svg));
    assert_eq!(svg_size(SHAPES), Ok((200.0, 100.0)));
    let img = render_svg(SHAPES, 200, 100, &[]).unwrap();
    assert_eq!((img.width, img.height), (200, 100));
    assert_eq!(rgba(img.pixel(50, 30).unwrap()), [255, 0, 0, 255]);
    assert_eq!(rgba(img.pixel(150, 50).unwrap()), [0, 0, 255, 255]);
    // Outside the circle, above the bar: transparent.
    assert_eq!(img.pixel(198, 2), Some(0));
    // Half-transparent green over transparent: straight alpha, green stays full.
    let [r, g, b, a] = rgba(img.pixel(199, 95).unwrap());
    assert_eq!((r, g, b), (0, 255, 0));
    assert!((126..=129).contains(&a), "{a}");
    // Over red: about half red, half green, opaque.
    let [r, g, b, a] = rgba(img.pixel(20, 95).unwrap());
    assert_eq!((b, a), (0, 255));
    assert!((126..=129).contains(&r) && (126..=129).contains(&g), "{r} {g}");
}

#[test]
fn scales_to_fit_and_centres() {
    // 100x100 canvas: the 2:1 drawing becomes 100x50, centred at y 25..75.
    let img = render_svg(SHAPES, 100, 100, &[]).unwrap();
    assert_eq!(img.pixel(25, 10), Some(0));
    assert_eq!(rgba(img.pixel(25, 30).unwrap()), [255, 0, 0, 255]);
    assert_eq!(rgba(img.pixel(75, 50).unwrap()), [0, 0, 255, 255]);
    assert_eq!(img.pixel(25, 90), Some(0));
    assert_eq!(svg_fit(SHAPES, 100, 100), Ok((100, 50)));
    assert_eq!(svg_fit(SHAPES, 1000, 1000), Ok((1000, 500)));
    // decode() draws at intrinsic size, scaled down only to fit the limits.
    assert_eq!(decode(SHAPES, &Limits::PHOTO).map(|i| (i.width, i.height)), Ok((200, 100)));
    assert_eq!(decode(SHAPES, &Limits::SURFACE).map(|i| (i.width, i.height)), Ok((200, 100)));
    let small = Limits { max_width: 50, max_height: 50, max_pixels: 2500, max_input_bytes: 1 << 20 };
    assert_eq!(decode(SHAPES, &small).map(|i| (i.width, i.height)), Ok((50, 25)));
}

#[test]
fn text_needs_a_font() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40">
      <text x="4" y="30" font-family="sans-serif" font-size="28" fill="#000">Gaze</text>
    </svg>"##;
    let ink = |img: &gaze_codecs::Image| img.pixels.iter().filter(|&&p| p >> 24 > 128).count();
    let without = render_svg(svg, 120, 40, &[]).unwrap();
    assert_eq!(ink(&without), 0, "no fonts: text is left out");
    let with = render_svg(svg, 120, 40, &[GEIST]).unwrap();
    assert!(ink(&with) > 150, "{}", ink(&with));
    // Glyphs sit on the baseline at y 30, not above the cap height.
    assert!(with.pixels[..120 * 5].iter().all(|&p| p >> 24 == 0));
    let via_decode = decode_with(svg, &Options { fonts: &[GEIST], ..Options::default() }).unwrap();
    assert_eq!(via_decode, with);
}

#[test]
fn refuses_bad_input_and_files() {
    assert_eq!(render_svg(b"<svg", 10, 10, &[]), Err(Error::Svg));
    assert_eq!(render_svg(SHAPES, 0, 10, &[]), Err(Error::TooLarge));
    assert_eq!(render_svg(SHAPES, 9000, 10, &[]), Err(Error::TooLarge));
    // An <image> naming a file on disk is never opened: nothing is drawn.
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
      <image width="10" height="10" xlink:href="/etc/passwd"/></svg>"#;
    assert!(render_svg(svg, 10, 10, &[]).unwrap().pixels.iter().all(|&p| p == 0));
}
