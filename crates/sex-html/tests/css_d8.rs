//! calc() with mixed units, min()/max()/clamp(), and position: sticky.
use sex_html::{layout::{self, FontKey, Host}, parse};
struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.len() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
}
fn page(html: &str) -> (sex_html::dom::Dom, layout::Page) {
    let dom = parse::parse(html);
    let styles = sex_html::styles(&dom, &[], 800, 600);
    let page = layout::layout_in(&dom, &styles, 800, 600, &Fixed);
    (dom, page)
}
fn rect(d: &sex_html::dom::Dom, p: &layout::Page, id: &str) -> [i32; 4] {
    let n = d.descendants(0).into_iter().find(|&n| d.attr(n, "id") == Some(id)).unwrap();
    p.boxes[n].unwrap()
}
#[test]
fn calc_mixes_units() {
    let (d, p) = page("<body style='margin:0'>\
        <div id=a style='width:calc(50% - 20px);height:10px'></div>\
        <div id=b style='width:calc((100% - 2 * 50px) / 2);height:calc(1em + 4px)'></div>\
        <div id=c style='width:min(300px, 200px, 500px);height:max(10px, 2em)'></div>\
        <div id=e style='width:clamp(100px, 50px, 400px);height:calc(10px*3)'></div>\
        <div id=f style='width:calc(100% - (2 * 10%))'></div></body>");
    assert_eq!(rect(&d, &p, "a")[2], 380);
    assert_eq!(rect(&d, &p, "b")[2], 350);
    assert_eq!(rect(&d, &p, "b")[3], 20);
    assert_eq!(rect(&d, &p, "c")[2], 200);
    assert_eq!(rect(&d, &p, "c")[3], 32);
    assert_eq!(rect(&d, &p, "e")[2], 100);
    assert_eq!(rect(&d, &p, "e")[3], 30);
    assert_eq!(rect(&d, &p, "f")[2], 640);
}
#[test]
fn sticky_header_follows_the_scroll_within_its_parent() {
    let (d, p) = page("<body style='margin:0'><div id=box style='height:2000px'>\
        <div style='height:100px'></div>\
        <div id=s style='position:sticky;top:10px;height:50px;background:red'>head</div>\
        </div><div style='height:1000px'></div></body>");
    let s = rect(&d, &p, "s");
    assert_eq!(s[1], 100);
    let i = p.items.iter().position(|it| matches!(it, sex_html::Item::Rect { color, .. } if *color == 0xFFFF_0000)).expect("sticky background");
    assert_eq!(p.sticky_shift(i, 0), 0);
    assert_eq!(p.sticky_shift(i, 90), 0);
    assert_eq!(p.sticky_shift(i, 500), 410);
    // It stops at the bottom of its containing block (2000).
    assert_eq!(p.sticky_shift(i, 5000), 2000 - 100 - 50);
}
#[test]
fn matrix_and_mixed_translations_move_the_box() {
    let (_, p) = page("<body style='margin:0'><div style='width:100px;height:20px;background:red;transform:matrix(1,0,0,1,30,40) translateX(10%)'></div></body>");
    let r = p.items.iter().find_map(|it| match it { sex_html::Item::Rect { x, y, color, .. } if *color == 0xFFFF_0000 => Some((*x, *y)), _ => None }).unwrap();
    assert_eq!(r, (40, 40));
}
