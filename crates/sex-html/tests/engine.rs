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
