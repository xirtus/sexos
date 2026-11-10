use sex_html::{layout::{self, FontKey, Host, Item}, parse};
struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.len() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
}
fn page(html: &str) -> (sex_html::dom::Dom, layout::Page) {
    let dom=parse::parse(html);let styles=sex_html::styles(&dom,&[],720,405);
    let page=layout::layout_in(&dom,&styles,720,405,&Fixed);(dom,page)
}
fn rect(dom:&sex_html::dom::Dom,page:&layout::Page,id:&str)->[i32;4] {
    let node=dom.descendants(0).into_iter().find(|&n|dom.attr(n,"id")==Some(id)).unwrap();
    page.boxes[node].expect("positioned descendant must have a layout box")
}
#[test]
fn fixed_controls_inside_absolute_body_use_viewport() {
    let(d,p)=page("<body style='position:absolute;left:80px;top:60px;width:200px;height:100px;margin:0;overflow:hidden'><div id=controls style='position:fixed;inset:0;background:red'>Play</div></body>");
    assert_eq!(rect(&d,&p,"controls"),[0,0,720,405]);
    assert!(p.items.iter().any(|i|matches!(i,Item::Text{text,..} if text=="Play")));
}
#[test]
fn newly_discovered_fixed_descendants_preserve_tree_paint_order() {
    let(_,p)=page("<body style='margin:0'><div style='position:absolute;inset:0'><div style='position:fixed;inset:0;background:red'></div></div><div style='position:fixed;inset:0;background:blue'></div></body>");
    let colors:Vec<_>=p.items.iter().filter_map(|i|match i{Item::Rect{color,w,h,..}if *w==720&&*h==405&&(*color==0xffff0000||*color==0xff0000ff)=>Some(*color),_=>None}).collect();
    assert_eq!(colors,vec![0xffff0000,0xff0000ff]);
}
#[test]
fn nested_fixed_and_absolute_descendants_keep_distinct_containing_blocks() {
    let(d,p)=page("<body style='position:absolute;left:80px;top:60px;width:200px;height:100px;margin:0'><div id=f style='position:fixed;right:10px;bottom:5px;width:100px;height:50px'><div id=a style='position:absolute;right:3px;bottom:2px;width:10px;height:7px'></div><div id=g style='position:fixed;left:9px;top:8px;width:20px;height:10px'></div></div></body>");
    assert_eq!(rect(&d,&p,"f"),[610,350,100,50]);
    assert_eq!(rect(&d,&p,"a"),[697,391,10,7]);
    assert_eq!(rect(&d,&p,"g"),[9,8,20,10]);
}
