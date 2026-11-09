use sex_html::layout::{FontKey, Host, Item};
use sex_html::{css, parse, render, style};

/// Every character is half the font size wide; ascent 4/5, descent 1/5.
struct Fixed;
impl Host for Fixed {
    fn text_width(&self, text: &str, f: FontKey) -> i32 { text.chars().count() as i32 * f.size / 2 }
    fn metrics(&self, f: FontKey) -> (i32, i32) { (f.size * 4 / 5, f.size / 5) }
    fn image_size(&self, src: &str) -> Option<(i32, i32)> { (src == "cat.png").then_some((200, 100)) }
}

fn texts(page: &sex_html::Page) -> Vec<(i32, i32, String)> {
    page.items.iter().filter_map(|i| match i { Item::Text { x, baseline, text, .. } => Some((*x, *baseline, text.clone())), _ => None }).collect()
}

#[test]
fn parses_implied_structure_and_entities() {
    let dom = parse::parse("<!doctype html><title>T &amp; U</title><p>one<p>two &lt;3 &#x41;&copy<li>x<li>y");
    let body = dom.find("body").unwrap();
    let kids: Vec<_> = dom.element_children(body).map(|c| dom.tag(c).unwrap().to_string()).collect();
    assert_eq!(kids, ["p", "p", "li", "li"]);
    assert_eq!(dom.text_content(dom.find("title").unwrap()), "T & U");
    let p2 = dom.element_children(body).nth(1).unwrap();
    assert_eq!(dom.text_content(p2), "two <3 A©");
}

#[test]
fn raw_text_and_tables() {
    let dom = parse::parse("<style>p > b { x: '</p>' }</style><table><td>a<td>b<tr><td>c</table><script>if (a<b) {}</script>");
    assert!(dom.text_content(dom.find("style").unwrap()).contains("p > b"));
    assert_eq!(dom.text_content(dom.find("script").unwrap()), "if (a<b) {}");
    let table = dom.find("table").unwrap();
    let tbody = dom.element_children(table).next().unwrap();
    assert_eq!(dom.tag(tbody), Some("tbody"));
    assert_eq!(dom.element_children(tbody).count(), 2);
}

#[test]
fn selectors_and_specificity() {
    let dom = parse::parse(r#"<div id=a class="x y"><p class=z>hi</p><p>there</p></div>"#);
    let p = dom.find("p").unwrap();
    for (sel, want) in [("div p", true), ("#a > .z", true), (".x.y p:first-child", true), ("p + p", false), ("div:not(.q) p", true), ("p:hover", false), ("[class~=y] p", true), ("p:nth-child(2n+1)", true)] {
        let s = css::parse_selector(sel).unwrap_or_else(|| panic!("{sel}"));
        assert_eq!(css::matches(&dom, p, &s), want, "{sel}");
    }
    assert!(css::parse_selector("#a").unwrap().specificity > css::parse_selector(".x.y.z p").unwrap().specificity);
}

#[test]
fn cascade_orders_by_specificity_and_importance() {
    let html = r#"<style>p { color: red } .k { color: #00ff00 } p { font-size: 2em !important } @media (max-width: 100px) { p { color: blue } }</style>
                <body style="font-size: 10px"><p class=k style="font-size: 3px">t</p>"#;
    let dom = parse::parse(html);
    let st = sex_html::styles(&dom, &[], 800, 600);
    let p = dom.find("p").unwrap();
    let s = st[p].as_ref().unwrap();
    assert_eq!(s.color, 0xFF00FF00);
    assert_eq!(s.font_size, 20);
    assert_eq!(style::parse_color("rgba(255, 0, 0, 0.5)"), Some(0x7FFF0000));
    assert_eq!(style::parse_color("hsl(120, 100%, 50%)"), Some(0xFF00FF00));
}

#[test]
fn blocks_stack_with_collapsed_margins_and_lines_wrap() {
    let html = "<body style='margin:0'><p style='margin:10px 0'>aaaa bbbb cccc</p><p style='margin:20px 0'>dd</p>";
    // 16px font: 8px per char; width 80 holds "aaaa bbbb" (72) but not "+ cccc".
    let page = render(html, 80, 600, &Fixed);
    let t = texts(&page);
    assert_eq!(t.len(), 3, "{t:?}");
    assert_eq!(t[0].2, "aaaa bbbb");
    assert_eq!(t[1].2, "cccc");
    let line = t[1].1 - t[0].1;
    assert!(line >= 16, "line height {line}");
    // Second paragraph: 10px top + 2 lines + max(10, 20) collapsed gap.
    assert_eq!(t[2].1 - t[0].1, 2 * line + 20);
}

#[test]
fn inline_styles_links_and_alignment() {
    let html = "<body style='margin:0'><p style='text-align:center'>go <a href='/x'>there</a> <b>now</b></p>";
    let page = render(html, 200, 600, &Fixed);
    let t = texts(&page);
    let total: i32 = "go there now".len() as i32 * 8;
    assert_eq!(t[0].0, (200 - total) / 2);
    assert_eq!(page.links.len(), 1);
    assert_eq!(page.links[0].href, "/x");
    assert_eq!(page.links[0].x, t[0].0 + 3 * 8);
    assert!(page.items.iter().any(|i| matches!(i, Item::Text { font, text, .. } if font.bold && text == "now")));
}

#[test]
fn lists_get_markers() {
    let page = render("<ol start=3><li>a<li>b</ol><ul><li>c</ul>", 300, 600, &Fixed);
    let t = texts(&page);
    assert!(t.iter().any(|x| x.2 == "3."), "{t:?}");
    assert!(t.iter().any(|x| x.2 == "4."));
    assert!(page.items.iter().any(|i| matches!(i, Item::Bullet { .. })));
}

#[test]
fn tables_lay_cells_side_by_side() {
    let page = render("<table><tr><td>aa<td>bbbb</tr><tr><td>c<td>d</table>", 400, 600, &Fixed);
    let t = texts(&page);
    let a = t.iter().find(|x| x.2 == "aa").unwrap();
    let b = t.iter().find(|x| x.2 == "bbbb").unwrap();
    let d = t.iter().find(|x| x.2 == "d").unwrap();
    assert_eq!(a.1, b.1);
    assert!(b.0 > a.0 + 16);
    assert_eq!(b.0, d.0);
}

#[test]
fn flex_rows_and_centering() {
    let html = "<body style='margin:0'><div style='display:flex;justify-content:space-between'><span>ab</span><span>cd</span></div><div style='width:100px;margin:0 auto;background:#eee'>x</div>";
    let page = render(html, 300, 600, &Fixed);
    let t = texts(&page);
    assert_eq!(t[0].0, 0);
    assert_eq!(t[1].0, 300 - 16);
    assert!(page.items.iter().any(|i| matches!(i, Item::Rect { x: 100, w: 100, .. })));
}

#[test]
fn images_scale_and_alt_text_stands_in() {
    let page = render("<img src=cat.png width=100><img src=missing.png alt=dog>", 400, 600, &Fixed);
    assert!(page.items.iter().any(|i| matches!(i, Item::Image { w: 100, h: 50, .. })));
    assert!(texts(&page).iter().any(|t| t.2 == "[dog]"));
}

#[test]
fn pre_keeps_whitespace() {
    let page = render("<pre>a  b\n\tc</pre>", 400, 600, &Fixed);
    let t = texts(&page);
    assert_eq!(t[0].2, "a  b");
    assert_eq!(t[1].2, "        c");
}

#[test]
fn hidden_things_take_no_space() {
    let page = render("<div style=display:none>x</div><script>y</script><p hidden>z</p>w", 400, 600, &Fixed);
    let t = texts(&page);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].2, "w");
}

#[test]
fn survives_garbage() {
    for src in ["<", "<<<>>>", "</p></div>", "<a href='", "<table><td><table><td>x", "<p style='color:'>", "&#xFFFFFFFF;&#0;", "<ul><li><ul><li><ul><li>deep"] {
        let _ = render(src, 100, 100, &Fixed);
    }
}

#[test]
fn resolves_links() {
    use sex_html::url::resolve;
    let base = "https://a.org/docs/guide/intro.html?x=1#top";
    for (href, want) in [
        ("next.html", "https://a.org/docs/guide/next.html"),
        ("../api/", "https://a.org/docs/api/"),
        ("/root", "https://a.org/root"),
        ("//cdn.org/x.png", "https://cdn.org/x.png"),
        ("?q=2", "https://a.org/docs/guide/intro.html?q=2"),
        ("#s", "https://a.org/docs/guide/intro.html?x=1#s"),
        ("http://b.org", "http://b.org"),
        ("./a/./b/../c", "https://a.org/docs/guide/a/c"),
    ] {
        assert_eq!(resolve(base, href).as_deref(), Some(want), "{href}");
    }
    assert_eq!(resolve(base, "mailto:x@y"), None);
    assert_eq!(resolve("https://a.org", "b").as_deref(), Some("https://a.org/b"));
}

#[test]
fn block_images_inline_anchors_and_cell_alignment() {
    // A block-level picture keeps its size and pushes the next line down.
    let page = render("<body style=margin:0><img src=cat.png style='display:block;width:100px'><p style=margin:0>after</p>", 300, 600, &Fixed);
    assert!(page.items.iter().any(|i| matches!(i, Item::Image { x: 0, y: 0, w: 100, h: 50, .. })));
    assert!(texts(&page)[0].1 > 50);
    // Fragment targets inside lines are recorded where their line is.
    let page = render("<body style=margin:0><p style=margin:0>one</p><p style=margin:0>two <span id=t>x</span><a name=n></a></p>", 300, 600, &Fixed);
    let y = |id: &str| page.anchors.iter().find(|(a, _)| a == id).map(|(_, y)| *y);
    assert!(y("t").unwrap() > 0, "{:?}", page.anchors);
    assert_eq!(y("t"), y("n"));
    // Centring a short cell must not move its taller neighbour.
    let page = render("<table><tr><td>short<td>a<br>b<br>c</table>", 300, 600, &Fixed);
    let t = texts(&page);
    let a = t.iter().find(|x| x.2 == "a").unwrap().1;
    let c = t.iter().find(|x| x.2 == "c").unwrap().1;
    let short = t.iter().find(|x| x.2 == "short").unwrap().1;
    assert_eq!(short, t.iter().find(|x| x.2 == "b").unwrap().1);
    assert!(a < short && short < c);
}

#[test]
fn every_laid_out_element_has_its_box() {
    let dom = parse::parse("<body style='margin:0'><div id=d style='padding:4px'><canvas id=c width=100 height=50></canvas> <a href=x id=a>link</a></div><p id=h style='display:none'>x</p></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = sex_html::layout::layout(&dom, &styles, 400, &Fixed);
    let by_id = |id: &str| dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap();
    let d = page.boxes[by_id("d")].unwrap();
    assert_eq!((d[0], d[1], d[2]), (0, 0, 400));
    let c = page.boxes[by_id("c")].unwrap();
    assert_eq!((c[0], c[2], c[3]), (4, 100, 50));
    assert!(c[1] >= 4 && c[1] + c[3] <= d[1] + d[3]);
    let a = page.boxes[by_id("a")].unwrap();
    assert!(a[0] > c[0] + c[2] && a[2] > 0);
    assert_eq!(page.boxes[by_id("h")], None);
}

#[test]
fn inline_elements_have_boxes_on_every_line() {
    let dom = parse::parse("<body style='margin:0'><p id=p style='margin:0'>a <span id=s>hover me please across lines</span> <em id=e></em>b</p></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let narrow = sex_html::layout::layout(&dom, &styles, 60, &Fixed);
    let wide = sex_html::layout::layout(&dom, &styles, 400, &Fixed);
    let by_id = |id: &str| dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap();
    let s = wide.boxes[by_id("s")].unwrap();
    let p = wide.boxes[by_id("p")].unwrap();
    assert!(s[0] > 0 && s[2] > 0 && s[1] >= p[1] && s[1] + s[3] <= p[1] + p[3]);
    // Wrapped, the span covers several lines.
    let sn = narrow.boxes[by_id("s")].unwrap();
    assert!(sn[3] > s[3]);
    // An empty inline element has no area to hit.
    assert_eq!(wide.boxes[by_id("e")], None);
    assert_eq!(narrow.height, sex_html::layout::layout(&dom, &styles, 60, &Fixed).height);
}

#[test]
fn positioned_boxes_place_and_layer() {
    let dom = parse::parse("<body style='margin:0'><div id=s style='position:relative;width:300px;height:100px'>\
        <div id=a style='position:absolute;right:10px;bottom:5px;width:50px;height:20px;background:red'></div>\
        <div id=f style='position:absolute;left:0;right:0;top:30px;height:10px'></div></div>\
        <p id=r style='position:relative;left:20px;top:5px;margin:0'>x</p><div id=z style='position:absolute;top:0;left:0;width:10px;height:10px;z-index:-1;background:blue'></div></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = sex_html::layout::layout_in(&dom, &styles, 400, 300, &Fixed);
    let by_id = |id: &str| dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap();
    assert_eq!(page.boxes[by_id("a")], Some([240, 75, 50, 20]));
    assert_eq!(page.boxes[by_id("f")], Some([0, 30, 300, 10]));
    // Out of flow: the paragraph follows the 100 px box directly, then moves by its offsets.
    let r = page.boxes[by_id("r")].unwrap();
    assert_eq!((r[0], r[1]), (20, 105));
    // Positioned boxes paint above normal flow; a negative z-index paints under it.
    let body = page.layers[dom.find("body").unwrap()];
    assert!(page.layers[by_id("s")] > body && page.layers[by_id("a")] > page.layers[by_id("s")]);
    assert!(page.layers[by_id("z")] < body);
    let reds: Vec<usize> = page.items.iter().enumerate().filter(|(_, i)| matches!(i, sex_html::layout::Item::Rect { color: 0xFFFF_0000, .. })).map(|(i, _)| i).collect();
    let blue = page.items.iter().position(|i| matches!(i, sex_html::layout::Item::Rect { color: 0xFF00_00FF, .. })).unwrap();
    assert!(blue < reds[0]);
    assert!(!page.items.iter().any(|i| matches!(i, sex_html::layout::Item::Rect { w, .. } if *w < -1_000_000)));
}

#[test]
fn text_wraps_around_floats_and_clear_goes_below() {
    let dom = parse::parse("<body style='margin:0'><div id=f style='float:right;width:100px;height:60px'></div>\
        <div id=g style='float:left;width:50px;height:30px'></div><p id=p style='margin:0'>words words words words words words words words words words words words</p>\
        <div id=c style='clear:both'>x</div><div id=o style='overflow:hidden'><div style='float:left;width:10px;height:90px'></div></div></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = sex_html::layout::layout(&dom, &styles, 400, &Fixed);
    let by_id = |id: &str| dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap();
    assert_eq!(page.boxes[by_id("f")], Some([300, 0, 100, 60]));
    assert_eq!(page.boxes[by_id("g")], Some([0, 0, 50, 30]));
    // The first line starts after the left float and stops before the right one.
    let first = page.items.iter().find_map(|i| match i { sex_html::layout::Item::Text { x, baseline, text, .. } if text.starts_with("words") => Some((*x, *baseline, text.len())), _ => None }).unwrap();
    assert_eq!(first.0, 50);
    assert!(first.1 < 30);
    let c = page.boxes[by_id("c")].unwrap();
    assert!(c[1] >= 60, "{c:?}");
    // overflow:hidden holds its float.
    assert_eq!(page.boxes[by_id("o")].unwrap()[3], 90);
}

#[test]
fn overflow_hidden_clips_its_content() {
    let dom = parse::parse("<body style='margin:0'><div style='width:100px;height:20px;overflow:hidden;border:1px solid red'><div style='width:300px;height:50px;background:blue'></div></div><p style='background:green'>x</p></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = sex_html::layout::layout(&dom, &styles, 400, &Fixed);
    let blue = page.items.iter().position(|i| matches!(i, sex_html::layout::Item::Rect { color: 0xFF00_00FF, .. })).unwrap();
    assert_eq!(page.clip_of(blue), Some([1, 1, 100, 20]));
    let green = page.items.iter().position(|i| matches!(i, sex_html::layout::Item::Rect { color: 0xFF00_8000, .. })).unwrap();
    assert_eq!(page.clip_of(green), None);
    assert!(!page.items.iter().any(|i| matches!(i, sex_html::layout::Item::Rect { w, .. } if *w < -1_000_000)));
}

#[test]
fn grid_places_items_in_tracks() {
    let rect = |page: &sex_html::layout::Page, c: u32| page.items.iter().find_map(|i| match i { sex_html::layout::Item::Rect { x, y, w, h, color, .. } if *color == c => Some((*x, *y, *w, *h)), _ => None }).unwrap();
    // Fixed + fr tracks, a gap, a full-width item and stretch to the tallest in the row.
    let dom = parse::parse("<body style='margin:0'><div style='display:grid;grid-template-columns:100px 1fr 2fr;gap:10px 20px;width:400px'>\
<div style='background:#ff0000;height:30px'></div><div style='background:#00ff00'>x</div><div style='background:#0000ff;height:50px'></div>\
<div style='grid-column:1 / -1;background:#ffff00;height:5px'></div></div></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = sex_html::layout::layout(&dom, &styles, 400, &Fixed);
    assert_eq!(rect(&page, 0xFFFF_0000), (0, 0, 100, 30));
    assert_eq!(rect(&page, 0xFF00_FF00), (120, 0, 86, 50));
    assert_eq!(rect(&page, 0xFF00_00FF), (226, 0, 174, 50));
    assert_eq!(rect(&page, 0xFFFF_FF00), (0, 60, 400, 5));
    // repeat(auto-fill, minmax(120px, 1fr)) fits three columns into 400px.
    let dom = parse::parse("<body style='margin:0'><div style='display:grid;grid-template-columns:repeat(auto-fill, minmax(120px, 1fr));width:400px'>\
<div style='background:#ff0000;height:10px'></div><div style='background:#00ff00;height:10px'></div><div style='background:#0000ff;height:10px'></div><div style='background:#ffff00;height:10px'></div></div></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let page = sex_html::layout::layout(&dom, &styles, 400, &Fixed);
    assert_eq!(rect(&page, 0xFF00_00FF), (266, 0, 134, 10));
    assert_eq!(rect(&page, 0xFFFF_FF00), (0, 10, 133, 10));
}

#[test]
fn css_variables_inherit_and_substitute() {
    let dom = parse::parse("<style>:root{--accent:#ff0000;--pad:7px} .card{--accent:#00ff00} div{background:var(--accent);padding:var(--pad);margin:var(--missing, 3px)} p{color:var(--nope)}</style>\
<body><div id=a></div><section class=card><div id=b></div></section><p id=p>x</p></body>");
    let styles = sex_html::styles(&dom, &[], 400, 300);
    let by_id = |id: &str| styles[dom.descendants(sex_html::dom::Dom::ROOT).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap()].as_ref().unwrap();
    assert_eq!(by_id("a").background, 0xFFFF_0000);
    assert_eq!(by_id("b").background, 0xFF00_FF00);
    assert_eq!(by_id("a").padding[0], sex_html::style::Len::Px(7));
    assert_eq!(by_id("a").margin[0], sex_html::style::Len::Px(3));
    assert_eq!(by_id("p").color, sex_html::style::Style::initial().color);
}
