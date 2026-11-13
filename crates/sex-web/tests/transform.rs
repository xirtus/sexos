//! CSS rotate(), scale() and skew(): a transformed box paints through its map.
use sex_web::{Fonts, BAR};

fn shot(html: &str) -> (Vec<u32>, i32) {
    let mut fonts = Fonts::new().unwrap();
    let (w, h) = (400, 300);
    let page = sex_web::sex_html::render(html, w, h - BAR, &fonts);
    let mut px = vec![0u32; (w * h) as usize];
    sex_web::frame(&page, &mut fonts, &mut px, w, h, 0, None, "x", false);
    (px, w)
}
fn at(s: &(Vec<u32>, i32), x: i32, y: i32) -> u32 { s.0[((y + BAR) * s.1 + x) as usize] & 0xFF_FFFF }
const RED: u32 = 0xFF0000;
const WHITE: u32 = 0xFFFFFF;

#[test]
fn rotate_turns_a_bar_upright() {
    let box_ = "margin:100px 0 0 100px;width:200px;height:20px;background:#f00";
    let flat = shot(&format!("<body style='margin:0;background:#fff'><div style='{box_}'></div>"));
    assert_eq!(at(&flat, 120, 110), RED);
    assert_eq!(at(&flat, 200, 40), WHITE);
    // A quarter turn about its centre (200, 110): 20 wide, 200 tall.
    let turned = shot(&format!("<body style='margin:0;background:#fff'><div style='{box_};transform:rotate(90deg)'></div>"));
    assert_eq!(at(&turned, 120, 110), WHITE);
    assert_eq!(at(&turned, 200, 40), RED);
    assert_eq!(at(&turned, 200, 190), RED);
    assert_eq!(at(&turned, 220, 110), WHITE);
    // The rotate property and a turn unit do the same.
    let prop = shot(&format!("<body style='margin:0;background:#fff'><div style='{box_};rotate:.25turn'></div>"));
    assert_eq!(at(&prop, 200, 40), RED);
}

#[test]
fn scale_grows_about_the_origin() {
    let box_ = "margin:100px 0 0 100px;width:100px;height:20px;background:#f00";
    let s = shot(&format!("<body style='margin:0;background:#fff'><div style='{box_};transform:scale(2)'></div>"));
    // About the centre (150, 110): 50..250 by 90..130.
    assert_eq!(at(&s, 60, 110), RED);
    assert_eq!(at(&s, 240, 125), RED);
    assert_eq!(at(&s, 40, 110), WHITE);
    assert_eq!(at(&s, 150, 85), WHITE);
    // From the top left corner: 100..300 by 100..140.
    let o = shot(&format!("<body style='margin:0;background:#fff'><div style='{box_};transform:scale(2);transform-origin:0 0'></div>"));
    assert_eq!(at(&o, 290, 135), RED);
    assert_eq!(at(&o, 95, 105), WHITE);
}

#[test]
fn text_inside_a_rotated_box_turns_too() {
    let flat = shot("<body style='margin:0;background:#fff;color:#000'><div style='margin:100px 0 0 100px;width:200px;font-size:20px'>HHHHHHHH</div>");
    let turned = shot("<body style='margin:0;background:#fff;color:#000'><div style='margin:100px 0 0 100px;width:200px;font-size:20px;transform:rotate(-90deg)'>HHHHHHHH</div>");
    let ink = |s: &(Vec<u32>, i32)| {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in 0..270 { for x in 0..400 { if at(s, x, y) & 0xFF < 0x80 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); } } }
        (x1 - x0, y1 - y0)
    };
    let (fw, fh) = ink(&flat);
    let (tw, th) = ink(&turned);
    assert!(fw > 2 * fh, "flat text is wide: {fw}x{fh}");
    assert!(th > 2 * tw, "turned text is tall: {tw}x{th}");
}

#[test]
fn find_matches_a_phrase_across_a_line_wrap() {
    let mut fonts = Fonts::new().unwrap();
    // Narrow enough that "inter-process communication" wraps between its words.
    let page = sex_web::sex_html::render("<body style='margin:0'><p style='width:150px;font-size:16px'>The kernel does inter-process communication well. <b>Micro</b>kernel.</p>", 400, 300, &fonts);
    let marks = sex_web::find_marks(&page, &mut fonts, "inter-process   communication");
    assert_eq!(marks.iter().map(|m| m[4]).max(), Some(0), "one match: {marks:?}");
    assert!(marks.len() >= 2 && marks[0][1] != marks[marks.len() - 1][1], "a mark on each line: {marks:?}");
    // Across runs on one line.
    let m = sex_web::find_marks(&page, &mut fonts, "microkernel");
    assert_eq!(m.iter().map(|m| m[4]).max(), Some(0), "{m:?}");
    assert!(sex_web::find_marks(&page, &mut fonts, "kernel").iter().map(|m| m[4]).max() == Some(1));
}

#[test]
fn a_web_font_from_font_face_is_used_by_its_family() {
    let mut fonts = Fonts::new().unwrap();
    let html = "<style>@font-face{font-family:'Gate Face';src:url(x.woff2) format('woff2'),url(fonts/geist-mono.woff) format('woff')}\
        p{font-family:\"Gate Face\",sans-serif;font-size:20px;margin:0}</style><body style='margin:0'><p id=a><span id=i>iiiiiiii</span></p><p><span id=m>mmmmmmmm</span></p>";
    let dom = sex_web::sex_html::parse::parse(html);
    let faces = sex_web::sex_html::page_font_faces(&dom, 400);
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].family, "Gate Face");
    assert_eq!(faces[0].sources[1], ("fonts/geist-mono.woff".into(), "woff".into()));
    let width = |fonts: &Fonts| {
        let page = sex_web::sex_html::render(html, 400, 300, fonts);
        let w = |id: &str| { let n = dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap(); page.boxes[n].unwrap()[2] };
        (w("i"), w("m"))
    };
    // Without the font: proportional, i is narrow.
    sex_web::sex_html::style::set_web_fonts(Vec::new());
    let (i, m) = width(&fonts);
    assert!(i * 2 < m, "{i} {m}");
    // With it: every glyph as wide as the next.
    let woff = include_bytes!("../../../scripts/web_site/fonts/geist-mono.woff");
    let face = fonts.add_face(sex_fetch::woff::to_sfnt(woff).unwrap()).unwrap();
    sex_web::sex_html::style::set_web_fonts(vec![("gate face".into(), false, false, face)]);
    let (i, m) = width(&fonts);
    assert_eq!(i, m);
    sex_web::sex_html::style::set_web_fonts(Vec::new());
}
