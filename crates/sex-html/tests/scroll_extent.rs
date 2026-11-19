use sex_html::{layout::{self, FontKey, Host}, parse};
struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.len() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
}
fn height(body: &str) -> i32 {
    let dom = parse::parse(&format!("<html style='height:100%'><body style='margin:0;height:100%'>{body}</body></html>"));
    let styles = sex_html::styles(&dom, &[], 720, 405);
    layout::layout_in(&dom, &styles, 720, 405, &Fixed).height
}
#[test]
fn viewport_height_root_keeps_article_overflow_scrollable() {
    assert_eq!(height("<header style='height:82px'></header><main style='height:2000px'></main>"), 2082);
}
#[test]
fn clipped_and_fixed_descendants_do_not_extend_document() {
    for overflow in ["hidden", "auto", "scroll"] {
        assert_eq!(height(&format!("<div style='height:100px;overflow:{overflow}'><div style='height:2000px'></div></div>")), 405);
    }
    assert_eq!(height("<div style='position:fixed;top:1000px;height:100px'><div style='height:2000px'></div></div>"),405);
}
#[test]
fn positioned_visible_overflow_extends_document() {
    assert_eq!(height("<div style='position:relative;height:10px'><div style='position:absolute;top:500px;height:100px'></div></div>"),600);
}
