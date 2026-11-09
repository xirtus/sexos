//! SVG pictures: shapes, paths, transforms and sizes.
use sex_html::dom::Dom;

fn draw(src: &str, w: u32, h: u32) -> sex_image::Image {
    let dom = sex_html::parse::parse(src);
    let svg = dom.descendants(Dom::ROOT).into_iter().find(|&n| dom.tag(n) == Some("svg")).unwrap();
    sex_web::svg::render(&dom, svg, w, h, 0xFF00_00FF).unwrap()
}

fn at(img: &sex_image::Image, x: usize, y: usize) -> u32 { img.pixels[y * img.width + x] }

#[test]
fn shapes_fill_and_stroke_in_the_view_box() {
    // viewBox 0 0 10 10 drawn at 100 x 100: everything scales by ten.
    let img = draw("<svg viewBox='0 0 10 10'><rect x=1 y=1 width=3 height=3 fill='#ff0000'/><circle cx=7 cy=7 r=2 fill='currentColor'/>\
<path d='M1 9h3' stroke='#00ff00' stroke-width='1'/></svg>", 100, 100);
    assert_eq!(at(&img, 25, 25), 0xFFFF_0000);
    assert_eq!(at(&img, 70, 70), 0xFF00_00FF);
    assert_eq!(at(&img, 25, 90), 0xFF00_FF00);
    assert_eq!(at(&img, 50, 50), 0);
}

#[test]
fn paths_take_relative_curves_arcs_and_transforms() {
    // A filled half-disc from an arc, moved by a group transform, and a packed-number path.
    let img = draw("<svg viewBox='0 0 100 100'><g transform='translate(50 0)'><path d='M-40 50a40 40 0 0 1 80 0z' fill='#000'/></g>\
<path d='M0,90l10-0,0,10-10,0z' fill='#ff0000'/></svg>", 100, 100);
    assert_eq!(at(&img, 50, 30) >> 24, 0xFF);
    assert_eq!(at(&img, 50, 70), 0);
    assert_eq!(at(&img, 5, 95), 0xFFFF_0000);
}

#[test]
fn svg_files_have_their_own_size() {
    let img = sex_web::decode_image(b"<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg' width='40' height='20'><rect width='40' height='20' fill='red'/></svg>", 1000, 1000).unwrap();
    assert_eq!((img.width, img.height), (40, 20));
    assert_eq!(img.pixels[0], 0xFFFF_0000);
}
