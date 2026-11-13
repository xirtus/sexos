//! Page zoom: absolute lengths and default font sizes grow with it, so the page
//! reflows; percentages keep following their container.
use sex_html::{layout::{self, FontKey, Host, Item}, parse};
struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.len() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
}
fn page(html: &str) -> (sex_html::dom::Dom, layout::Page) {
    let dom = parse::parse(html);
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = layout::layout_in(&dom, &styles, 400, 300, &Fixed);
    (dom, page)
}
fn size_of(p: &layout::Page, word: &str) -> i32 {
    p.items.iter().find_map(|i| match i { Item::Text { text, font, .. } if text.contains(word) => Some(font.size), _ => None }).unwrap()
}
// One test: the zoom is process-wide.
#[test]
fn zoom_scales_absolute_lengths_and_reflows() {
    let html = "<body style='margin:0'><p>plain words that wrap when they get bigger and bigger</p><div id=b style='width:100px;height:20px'></div><div id=w style='width:50%;height:1pt'></div><h1>Head</h1></body>";
    let (d, at100) = page(html);
    let node = |id: &str| d.descendants(0).into_iter().find(|&n| d.attr(n, "id") == Some(id)).unwrap();
    sex_html::style::set_zoom(200);
    let (_, at200) = page(html);
    sex_html::style::set_zoom(100);
    assert_eq!(size_of(&at100, "plain"), 16);
    assert_eq!(size_of(&at200, "plain"), 32);
    assert_eq!(size_of(&at200, "Head"), 64);
    assert_eq!(at100.boxes[node("b")].unwrap()[2], 100);
    assert_eq!(at200.boxes[node("b")].unwrap()[2], 200);
    // A percentage stays a share of the same viewport.
    assert_eq!(at200.boxes[node("w")].unwrap()[2], 200);
    assert!(at200.height > at100.height * 3 / 2, "{} vs {}", at200.height, at100.height);
}
