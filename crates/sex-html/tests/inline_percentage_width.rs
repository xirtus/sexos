use sex_html::{layout::{self, FontKey, Host}, parse};
struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.len() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
}
fn width(html: &str, id: &str) -> i32 {
    let dom=parse::parse(html);let styles=sex_html::styles(&dom,&[],720,405);
    let page=layout::layout_in(&dom,&styles,720,405,&Fixed);
    let node=dom.descendants(0).into_iter().find(|&n|dom.attr(n,"id")==Some(id)).unwrap();
    page.boxes[node].expect("inline element must have a layout box")[2]
}
#[test]
fn inline_block_percentage_uses_definite_container_width() {
    assert_eq!(width("<body style='margin:0'><div style='width:360px'><span id=x style='display:inline-block;width:50%;height:24px'></span></div></body>","x"),180);
}
#[test]
fn percentage_button_keeps_clickable_border_box_width() {
    assert_eq!(width("<body style='margin:0'><div style='width:360px'><button id=x style='width:50%;height:24px;box-sizing:border-box;padding:4px;border:2px solid black'>Play</button></div></body>","x"),180);
}
#[test]
fn nested_percentage_inline_blocks_use_their_own_containers() {
    assert_eq!(width("<body style='margin:0'><div style='width:360px'><span style='display:inline-block;width:50%'><span id=x style='display:inline-block;width:50%;height:24px'></span></span></div></body>","x"),90);
}
