use sex_html::{layout::{FontKey, Host}, parse};

struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.chars().count() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
    fn image_size(&self, src: &str) -> Option<(i32, i32)> { (src == "cat.png").then_some((200, 100)) }
}
fn size(html: &str, id: &str, viewport: (i32, i32)) -> (i32, i32) {
    let dom = parse::parse(html);
    let styles = sex_html::styles(&dom, &[], viewport.0, viewport.1);
    let page = sex_html::layout::layout_in(&dom, &styles, viewport.0, viewport.1, &Fixed);
    let node = dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap();
    let rect = page.boxes[node].expect("replaced element has a real layout box");
    (rect[2], rect[3])
}
#[test]
fn absolute_frame_percentage_tracks_its_real_viewport() {
    let html = "<body style='margin:0'><iframe id=f style='position:absolute;left:20px;top:40px;width:50%;height:120px;border:0'></iframe>";
    assert_eq!(size(html, "f", (400,300)), (200,120));
    assert_eq!(size(html, "f", (1264,676)), (632,120));
}
#[test]
fn inline_image_percentage_uses_its_container_and_intrinsic_aspect() {
    let html = "<body style='margin:0'><div style='width:240px'><span><img id=i src=cat.png style='width:50%'></span></div>";
    assert_eq!(size(html, "i", (800,600)), (120,60));
}
#[test]
fn block_replaced_percentages_use_definite_container_dimensions() {
    let html = "<body style='margin:0'><div style='width:240px;height:160px'><iframe id=f style='display:block;width:50%;height:50%;border:0'></iframe></div>";
    assert_eq!(size(html, "f", (800,600)), (120,80));
}
#[test]
fn indefinite_percentage_height_uses_intrinsic_image_or_frame_default() {
    let html = "<body style='margin:0'><div style='width:240px'><img id=i src=cat.png style='height:50%'><iframe id=f style='width:50%;height:50%;border:0'></iframe></div>";
    assert_eq!(size(html, "i", (800,600)), (200,100));
    assert_eq!(size(html, "f", (800,600)), (120,150));
}
#[test]
fn intrinsic_shrink_to_fit_does_not_turn_percentage_height_into_zero_width() {
    let html = "<body style='margin:0'><span id=p style='display:inline-block'><img id=i src=cat.png style='height:50%'></span>";
    assert_eq!(size(html, "i", (800,600)), (200,100));
    assert_eq!(size(html, "p", (800,600)).0, 200);
}
#[test]
fn fixed_and_unspecified_replaced_sizes_keep_their_existing_defaults() {
    let html = "<body style='margin:0'><img id=i src=cat.png width=100><iframe id=f style='border:0'></iframe>";
    assert_eq!(size(html, "i", (800,600)), (100,50));
    assert_eq!(size(html, "f", (800,600)), (300,150));
}
#[test]
fn zero_percentage_width_stays_zero_instead_of_an_intrinsic_fallback() {
    let html = "<body style='margin:0'><iframe id=f style='width:0%;height:120px;border:0'></iframe>";
    assert_eq!(size(html, "f", (800,600)), (0,120));
}
