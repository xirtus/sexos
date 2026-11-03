// SPDX-License-Identifier: GPL-3.0-only
//! Host tests of the Browser's bounded HTML reading view.
extern crate alloc;
#[path="../apps/kaleidoscope/src/html_text.rs"] mod html_text;
#[path="../apps/kaleidoscope/src/regex.rs"] mod regex;
#[path="../apps/kaleidoscope/src/fmath.rs"] mod fmath;
#[path="../apps/kaleidoscope/src/script.rs"] mod script;
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
    assert_eq!(t, "Title: t\n\n# SexOS News\n\nFirst paragraph.\n\nSecond\nline\n\n- one\n- two\n\n## Small\n\nend");
    assert!(!c.truncated);
    assert_eq!(c.links, 0);
}

#[test]
fn script_style_and_comments_contribute_nothing() {
    let page = "<p>a<script>document.write('<p>x</p>')</script>b<!-- hidden <p>c</p> -->d</p>\
        <noscript>ns</noscript><template><p>tp</p></template><SCRIPT type=x>y</SCRIPT><script src=/x.js></script><script>while(1){}</script>e";
    let (t, c) = text(page, 512);
    assert_eq!(t, "a<p>x</p>bd\n\ne");
    assert_eq!((c.scripts_run, c.scripts_refused), (1, 3));
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
    let (t, _) = text("<table><tr><th>name</th><th>size</th><th>kind</th></tr><tr><td>Quil</td><td>12 KiB</td><td>editor</td></tr><tr><td>Spindle shell</td><td>8</td></tr></table><p>after</p>", 512);
    assert_eq!(t, "name          | size   | kind\nQuil          | 12 KiB | editor\nSpindle shell | 8\n\nafter");
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

#[test]
fn ordered_nested_and_definition_lists() {
    let page = "<ol><li>one<li>two<ul><li>a</li><li>b</li></ul></li><li>three</li></ol><ol start=7><li>seven</li></ol><dl><dt>term</dt><dd>meaning</dd></dl>";
    let (t, _) = text(page, 512);
    assert_eq!(t, "1. one\n2. two\n  - a\n  - b\n3. three\n\n7. seven\n\nterm\n    meaning");
}

#[test]
fn inline_emphasis_and_code_keep_markers() {
    let (t, _) = text("<p>Press <kbd>Ctrl</kbd>+<kbd>S</kbd> to <strong>save</strong>, see <code>quil.rs</code> and <em>notes</em>.</p>", 256);
    assert_eq!(t, "Press `Ctrl`+`S` to *save*, see `quil.rs` and _notes_.");
}

#[test]
fn common_named_entities() {
    let (t, _) = text("<p>&ldquo;Held&rdquo; &ndash; it&rsquo;s 5&deg; &times; 2 &rarr; &euro;10 &bull; soft&shy;hyphen &nosuch;</p>", 256);
    assert_eq!(t, "\u{201c}Held\u{201d} \u{2013} it\u{2019}s 5\u{b0} \u{d7} 2 \u{2192} \u{20ac}10 \u{2022} softhyphen &nosuch;");
}

#[test]
fn forms_and_inline_extras_read_as_text() {
    let page = "<form><label>Name <input name=n placeholder='Your name'></label><input type=hidden value=secret>\
        <label><input type=checkbox checked> Remember</label> <input type=radio> A <input type=radio checked> B\
        <select><option>One<option selected>Two</select><textarea>hi there</textarea>\
        <button>Go now</button><input type=submit value=Send></form>\
        <p>E = mc<sup>2</sup>, H<sub>2</sub>O, <abbr title=\"capability\">cap</abbr>, <q>held</q>.</p>\
        <details><summary>More</summary><p>Hidden by default.</p></details>";
    let (t, _) = text(page, 1024);
    assert_eq!(t, "Name [input: Your name][x] Remember ( ) A (*) B[select: One | Two][text: hi there][button: Go now][button: Send]\n\nE = mc^2, H_2O, cap(capability), \"held\".\n\n> More\n\nHidden by default.");
}

#[test]
fn highlight_edit_and_figure_markers() {
    let (t, _) = text("<p>Use <mark>held</mark> routes, not <del>ambient</del><ins>granted</ins> ones; see <cite>ARCHITECTURE</cite> <s>old</s> <u>u</u>.</p><figure><img alt=Map><figcaption>The SASOS map</figcaption></figure>", 512);
    assert_eq!(t, "Use ==held== routes, not ~~ambient~~++granted++ ones; see _ARCHITECTURE_ ~~old~~ __u__.\n\n[image: Map]\nFigure: The SASOS map");
}

#[test]
fn declared_latin_charset_becomes_utf8() {
    let page = b"<html><head><meta charset=\"windows-1252\"></head><body><p>caf\xe9 \x93q\x94 \x80</p></body></html>";
    let mut out = vec![0u8; 256];
    let c = convert(page, &mut out);
    assert_eq!(std::str::from_utf8(&out[..c.len]).unwrap(), "caf\u{e9} \u{201c}q\u{201d} \u{20ac}");
    let utf8 = "<p>caf\u{e9}</p>";
    let c = convert(utf8.as_bytes(), &mut out);
    assert_eq!(std::str::from_utf8(&out[..c.len]).unwrap(), "caf\u{e9}", "undeclared pages are UTF-8");
}

#[test]
fn latin1_greek_and_symbol_entities() {
    let (t, _) = text("<p>&eacute;&Eacute;&ntilde;&szlig;&AElig;&yuml;&ETH; &le;&ge;&ne; &micro; &alpha;&pi;&Omega;&sigmaf; &check;&infin;&frac14;&sup2;&dagger;&hearts; a&shy;b &bogus; &Sigmaf;</p>", 512);
    assert_eq!(t.trim(), "éÉñßÆÿÐ ≤≥≠ µ απΩς ✓∞¼²†♥ ab &bogus; &Sigmaf;");
}

#[test]
fn list_paragraphs_marker_styles_media_and_preformatted_code() {
    let (t, _) = text("<ul><li><p>para in li</p></li><li>two</li></ul><p>after</p><ol type=a><li>x<li>y</ol><ol type=I start=4><li>x<li>y</ol>", 512);
    assert_eq!(t.trim(), "- para in li\n- two\n\nafter\n\na. x\nb. y\n\nIV. x\nV. y");
    let (t, _) = text("<video src=v.mp4 title='Demo clip'>Your browser cannot play this.</video><p>next</p><iframe src=x>fallback</iframe><p>tab&#9;stop</p><pre><code>fn main() {\n  x\n}</code></pre>", 512);
    assert_eq!(t.trim(), "[video: Demo clip]\n\nnext\n\n[frame]\n\ntab stop\n\nfn main() {\n  x\n}");
}

#[test]
fn table_captions_and_implicit_row_ends() {
    let (t, _) = text("<table><caption>Prices</caption><thead><tr><th>Item</th><th>Cost</th></tr></thead><tbody><tr><td>Tea</td><td>2</td></tr><tr><td>Coffee</td><td>3</td></tr></tbody></table><table><tr><td>a<td>b<tr><td>longer cell<td>x</table>", 512);
    assert_eq!(t.trim(), "Prices\nItem   | Cost\nTea    | 2\nCoffee | 3\n\na           | b\nlonger cell | x");
}
