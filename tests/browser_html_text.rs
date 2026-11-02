// SPDX-License-Identifier: GPL-3.0-only
//! Host tests of the Browser's bounded HTML reading view.
#[path="../apps/kaleidoscope/src/html_text.rs"] mod html_text;
use html_text::{convert, looks_like_html, MAX_LINKS};

fn text(html: &str, cap: usize) -> (String, html_text::Converted) {
    let mut out = vec![0u8; cap];
    let c = convert(html.as_bytes(), &mut out);
    (String::from_utf8(out[..c.len].to_vec()).unwrap(), c)
}

#[test]
fn page_becomes_readable_blocks() {
    let page = "<!DOCTYPE html><html><head><title>t</title><style>p{color:red}</style></head>\
        <body><h1>SexOS   News</h1><p>First\n  paragraph.</p><p>Second<br>line</p>\
        <ul><li>one</li><li>two</li></ul><h3>Small</h3><p>end</p></body></html>";
    let (t, c) = text(page, 4096);
    assert_eq!(t, "Title: t\n\n# SexOS News\n\nFirst paragraph.\n\nSecond\nline\n\n- one\n- two\n\nSmall\n\nend");
    assert!(!c.truncated);
    assert_eq!(c.links, 0);
}

#[test]
fn script_style_and_comments_contribute_nothing() {
    let page = "<p>a<script>document.write('<p>x</p>')</script>b<!-- hidden <p>c</p> -->d</p>\
        <noscript>ns</noscript><template><p>tp</p></template><SCRIPT type=x>y</SCRIPT>e";
    let (t, _) = text(page, 512);
    assert_eq!(t, "abd\n\ne");
}

#[test]
fn entities_decode_and_unknown_ones_stay_literal() {
    let (t, _) = text("<p>a &amp; b &lt;c&gt; &#65;&#x42; &copy; &bogus; &#0; &mdash;</p>", 512);
    assert_eq!(t, "a & b <c> AB \u{a9} &bogus; &#0; \u{2014}");
}

#[test]
fn links_keep_text_and_list_targets() {
    let page = "<p>Read <a href=\"/docs\">the docs</a> or <A class=x HREF='https://sexos.example/a?b=1'>this</A>.</p><a>bare</a>";
    let (t, c) = text(page, 1024);
    assert_eq!(c.links, 2);
    assert_eq!(t, "Read the docs[1] or this[2].\n\nbare\n\nLinks:\n[1] /docs\n[2] https://sexos.example/a?b=1");
}

#[test]
fn link_list_is_bounded() {
    let mut page = String::new();
    for n in 0..MAX_LINKS + 4 { page += &format!("<a href=\"/{n}\">l{n}</a> "); }
    let (_, c) = text(&page, 8192);
    assert_eq!(c.links, MAX_LINKS);
}

#[test]
fn output_never_exceeds_buffer_and_marks_cut() {
    let page = "<p>".to_string() + &"word ".repeat(2000) + "</p>";
    for cap in [17usize, 64, 1000] {
        let mut out = vec![0u8; cap];
        let c = convert(page.as_bytes(), &mut out);
        assert!(c.len <= cap && c.truncated);
        assert!(out[..c.len].ends_with(b"[page truncated]"));
    }
}

#[test]
fn malformed_markup_terminates() {
    for page in ["<p", "<!-- open", "<script>never closed", "&#xFFFFFFFFFF;", "<a href=\"/x>t</a>", "<<<>>>", "&#xD800;"] {
        let (_, c) = text(page, 256);
        assert!(c.len <= 256);
    }
}

#[test]
fn table_cells_are_separated() {
    let (t, _) = text("<table><tr><th>k</th><th>v</th></tr><tr><td>a</td><td>1</td></tr></table>", 256);
    assert_eq!(t, "k | v\na | 1");
}

#[test]
fn html_detection() {
    assert!(looks_like_html(b"  <!DOCTYPE html><p>"));
    assert!(looks_like_html(b"<HTML>"));
    assert!(!looks_like_html(b"plain text <p>"));
    assert!(!looks_like_html(b""));
}

#[test]
fn link_targets_are_exact_output_ranges() {
    let page = "<a href='/a/b'>x</a> <a href=\"/guide\">y</a>";
    let mut out = vec![0u8; 256];
    let c = convert(page.as_bytes(), &mut out);
    assert_eq!(c.links, 2);
    let t = |n: usize| &out[c.targets[n].0..c.targets[n].1];
    assert_eq!(t(0), b"/a/b");
    assert_eq!(t(1), b"/guide");
    assert_eq!(c.targets[2], (0, 0));
}

#[test]
fn cut_link_list_publishes_no_partial_target() {
    let page = "<p>".to_string() + &"w ".repeat(40) + "</p><a href=\"/a-long-target-path\">x</a>";
    let mut out = vec![0u8; 120];
    let c = convert(page.as_bytes(), &mut out);
    assert!(c.truncated);
    for &(a, b) in &c.targets { assert!(b <= c.len && (a, b) == (0, 0) || &out[a..b] == b"/a-long-target-path"); }
}

#[test]
fn pre_keeps_its_layout_and_other_blocks_do_not() {
    let page = "<p>a   b</p><pre>\nfn main() {\n    x\t= 1;\n}\n</pre><p>c   d</p>";
    let (t, _) = text(page, 512);
    assert_eq!(t, "a b\n\nfn main() {\n    x = 1;\n}\n\nc d");
}

#[test]
fn unterminated_pre_ends_before_the_link_list() {
    let (t, _) = text("<pre>  x  <a href=\"/y\">y</a>", 256);
    assert_eq!(t, "  x  y[1]\n\nLinks:\n[1] /y");
}

#[test]
fn images_show_alt_text_and_are_never_fetched() {
    let (t, c) = text("<p>Logo <img src=\"/art.qoi\" alt=\"SexOS mark\"> and <IMG src=x.png> and <img alt=' '/></p>", 256);
    assert_eq!(t, "Logo [image: SexOS mark] and [image] and [image]");
    assert_eq!(c.links, 0);
}

#[test]
fn title_quotes_and_rules() {
    let page = "<html><head><TITLE> Field &amp; notes </TITLE></head><body><p>a</p><blockquote><p>q1</p>q2<blockquote>deep</blockquote></blockquote><hr><p>b</p></body></html>";
    let (t, _) = text(page, 512);
    assert_eq!(t, "Title: Field & notes\n\na\n\n> q1\n\n> q2\n\n> > deep\n\n----\n\nb");
}
