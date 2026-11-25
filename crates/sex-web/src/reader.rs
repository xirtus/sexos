// SPDX-License-Identifier: GPL-3.0-only
//! Reader view: the article of a page without its menus, side bars, comments and
//! scripts, as a plain page of its own (Kaleidoscope N5, Peek folded in).
//!
//! Like other browsers' reader modes, it scores the containers of the page's
//! paragraphs: each paragraph of some length gives its parent points for its length
//! and commas, and half as many to the grandparent; class and id names such as
//! `article` or `content` add points and `comment`, `sidebar` or `nav` take them away.
//! The best container's paragraphs, headings, lists, quotes, code and pictures make
//! the reader page.
use alloc::string::String;
use alloc::vec::Vec;
use sex_html::dom::{Dom, Kind, NodeId};

/// What a reader page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    pub title: String,
    /// The article's HTML (no scripts, styles or attributes beyond picture and link addresses).
    pub html: String,
    pub words: usize,
}

const GOOD: &[&str] = &["article", "body", "content", "entry", "main", "page", "post", "text", "blog", "story"];
const BAD: &[&str] = &["comment", "meta", "footer", "footnote", "sidebar", "side-bar", "nav", "menu", "share", "social", "promo", "related", "advert", "banner", "cookie", "newsletter", "subscribe", "popup", "masthead"];
/// Elements that never hold article text.
const SKIP: &[&str] = &["script", "style", "noscript", "nav", "aside", "footer", "header", "form", "button", "iframe", "svg", "template", "select", "input", "textarea"];

fn names(dom: &Dom, n: NodeId) -> String {
    let mut s = String::from(dom.attr(n, "class").unwrap_or(""));
    s.push(' ');
    s.push_str(dom.attr(n, "id").unwrap_or(""));
    s.to_ascii_lowercase()
}

fn weight(dom: &Dom, n: NodeId) -> i64 {
    let s = names(dom, n);
    let mut w = 0;
    if GOOD.iter().any(|g| s.contains(g)) { w += 25; }
    if BAD.iter().any(|b| s.contains(b)) { w -= 25; }
    match dom.tag(n) { Some("article") | Some("main") => w += 30, Some("div") => w += 5, Some("form") | Some("ul") | Some("ol") => w -= 3, _ => {} }
    w
}

/// Hidden or left out: skipped elements, ones marked hidden, and those whose names
/// say they are not the article.
fn unwanted(dom: &Dom, n: NodeId) -> bool {
    let Some(tag) = dom.tag(n) else { return false };
    if SKIP.contains(&tag) || dom.attr(n, "hidden").is_some() || dom.attr(n, "aria-hidden") == Some("true") { return true; }
    let style = dom.attr(n, "style").unwrap_or("").replace(' ', "");
    if style.contains("display:none") { return true; }
    let s = names(dom, n);
    BAD.iter().any(|b| s.contains(b)) && !GOOD.iter().any(|g| s.contains(g)) && !matches!(tag, "body" | "article" | "main")
}

fn parent(dom: &Dom, n: NodeId) -> Option<NodeId> { dom.nodes[n].parent.filter(|p| matches!(dom.nodes[*p].kind, Kind::Element { .. })) }

fn inside_unwanted(dom: &Dom, mut n: NodeId) -> bool {
    loop {
        if unwanted(dom, n) { return true; }
        match parent(dom, n) { Some(p) => n = p, None => return false }
    }
}

fn clean(text: &str) -> String {
    let mut out = String::new();
    for w in text.split_whitespace() { if !out.is_empty() { out.push(' '); } out.push_str(w); }
    out
}

fn visible_words(dom: &Dom, n: NodeId) -> usize {
    fn collect(dom: &Dom, n: NodeId, text: &mut String) {
        if unwanted(dom, n) { return; }
        match &dom.nodes[n].kind {
            Kind::Text(value) => text.push_str(value),
            _ => for &child in &dom.nodes[n].children { collect(dom, child, text); },
        }
    }
    let mut text = String::new();
    collect(dom, n, &mut text);
    clean(&text).split_whitespace().count()
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }

/// The page's article, or `None` when it has too little running text to be one.
pub fn article(dom: &Dom, base: &str) -> Option<Article> {
    let all = dom.descendants(Dom::ROOT);
    let mut scores: Vec<(NodeId, i64)> = Vec::new();
    let add = |n: NodeId, s: i64, scores: &mut Vec<(NodeId, i64)>| match scores.iter_mut().find(|(m, _)| *m == n) {
        Some((_, v)) => *v += s,
        None => scores.push((n, s + weight(dom, n))),
    };
    for &n in &all {
        if !matches!(dom.tag(n), Some("p" | "pre" | "td" | "blockquote")) { continue; }
        if inside_unwanted(dom, n) { continue; }
        let text = clean(&dom.text_content(n));
        if text.len() < 25 { continue; }
        let s = 1 + text.matches(',').count() as i64 + (text.len() as i64 / 100).min(3);
        if let Some(p) = parent(dom, n) {
            add(p, s, &mut scores);
            if let Some(g) = parent(dom, p) { add(g, s / 2, &mut scores); }
        }
    }
    // Fewer points for containers that are mostly links (lists of other articles).
    for (n, s) in scores.iter_mut() {
        let text = clean(&dom.text_content(*n)).len().max(1) as i64;
        let linked: i64 = dom.descendants(*n).into_iter().filter(|d| dom.tag(*d) == Some("a")).map(|a| clean(&dom.text_content(a)).len() as i64).sum();
        *s = *s * (text - linked.min(text)) / text;
    }
    let (top, best) = scores.iter().copied().max_by_key(|(_, s)| *s)?;
    if best < 5 { return None; }
    // Siblings that score well too (an article split over several boxes) come along.
    let mut parts = alloc::vec![top];
    if let Some(p) = parent(dom, top) {
        for sib in dom.element_children(p) {
            if sib == top { continue; }
            if scores.iter().any(|(m, s)| *m == sib && *s * 5 >= best) { parts.push(sib); }
        }
        let order: Vec<NodeId> = dom.element_children(p).collect();
        parts.sort_by_key(|n| order.iter().position(|m| m == n));
    }
    let mut html = String::new();
    let mut words = 0;
    for part in parts { emit(dom, part, base, &mut html, &mut words); }
    if words < 50 { return None; }
    let heading = all.iter().copied().find(|&n| dom.tag(n) == Some("h1") && !inside_unwanted(dom, n)).map(|n| clean(&dom.text_content(n)));
    let titled = all.iter().copied().find(|&n| dom.tag(n) == Some("title")).map(|n| clean(&dom.text_content(n)));
    let title = heading.filter(|h| !h.is_empty()).or(titled).unwrap_or_default();
    Some(Article { title, html, words })
}

/// Writes what of `n` belongs in the article.
fn emit(dom: &Dom, n: NodeId, base: &str, out: &mut String, words: &mut usize) {
    match &dom.nodes[n].kind {
        Kind::Element { tag, .. } => {
            if unwanted(dom, n) { return; }
            let tag = tag.as_str();
            match tag {
                "p" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote" | "figcaption" | "dt" | "dd" => {
                    let mut inner = String::new();
                    inline(dom, n, base, &mut inner);
                    let inner = inner.trim();
                    if inner.is_empty() { return; }
                    *words += inner.split_whitespace().count();
                    out.push_str(&alloc::format!("<{tag}>{inner}</{tag}>\n"));
                }
                "pre" => {
                    let text = dom.text_content(n);
                    *words += text.split_whitespace().count();
                    out.push_str(&alloc::format!("<pre>{}</pre>\n", esc(&text)));
                }
                "table" => table(dom, n, base, out, words),
                "img" => picture(dom, n, base, out),
                "ul" | "ol" | "dl" | "figure" => {
                    out.push_str(&alloc::format!("<{tag}>"));
                    for c in dom.nodes[n].children.clone() { emit(dom, c, base, out, words); }
                    out.push_str(&alloc::format!("</{tag}>\n"));
                }
                // The article's own title is shown above it.
                "h1" => {}
                _ => for c in dom.nodes[n].children.clone() { emit(dom, c, base, out, words); },
            }
        }
        Kind::Text(t) => {
            // Loose text in a container (no paragraph around it).
            let t = clean(t);
            if t.len() >= 25 { *words += t.split_whitespace().count(); out.push_str(&alloc::format!("<p>{}</p>\n", esc(&t))); }
        }
        _ => {}
    }
}

/// Keep data tables in reading order. Only the table structure and bounded
/// column and row spans survive; page styling, event handlers and other attributes do
/// not cross into reader view.
fn table(dom: &Dom, n: NodeId, base: &str, out: &mut String, words: &mut usize) {
    let mut rows = String::new();
    table_rows(dom, n, base, &mut rows, words);
    if !rows.is_empty() {
        out.push_str("<table>");
        if let Some(caption) = dom.nodes[n].children.iter().copied()
            .find(|child| dom.tag(*child) == Some("caption") && !unwanted(dom, *child)) {
            let mut text = String::new();
            inline(dom, caption, base, &mut text);
            if !text.trim().is_empty() {
                *words += visible_words(dom, caption);
                out.push_str(&alloc::format!("<caption>{}</caption>", text.trim()));
            }
        }
        out.push_str(&rows);
        out.push_str("</table>\n");
    }
}

fn table_rows(dom: &Dom, n: NodeId, base: &str, out: &mut String, words: &mut usize) {
    for &child in &dom.nodes[n].children {
        if unwanted(dom, child) { continue; }
        match dom.tag(child) {
            Some("thead" | "tbody" | "tfoot") => table_rows(dom, child, base, out, words),
            Some("tr") => {
                let mut cells = String::new();
                for &cell in &dom.nodes[child].children {
                    if unwanted(dom, cell) { continue; }
                    let Some(tag @ ("th" | "td")) = dom.tag(cell) else { continue };
                    let mut content = String::new();
                    inline(dom, cell, base, &mut content);
                    let content = content.trim();
                    // Empty cells still occupy a column. Dropping one shifts
                    // every following value under the wrong heading.
                    *words += visible_words(dom, cell);
                    let span = |name| dom.attr(cell, name).and_then(|v| v.trim().parse::<usize>().ok())
                        .filter(|v| (2..=64).contains(v));
                    let colspan = span("colspan").map_or(String::new(), |v| alloc::format!(" colspan=\"{v}\""));
                    let rowspan = span("rowspan").map_or(String::new(), |v| alloc::format!(" rowspan=\"{v}\""));
                    cells.push_str(&alloc::format!("<{tag}{colspan}{rowspan}>{content}</{tag}>"));
                }
                if !cells.is_empty() { out.push_str("<tr>"); out.push_str(&cells); out.push_str("</tr>"); }
            }
            _ => {},
        }
    }
}

fn picture(dom: &Dom, n: NodeId, base: &str, out: &mut String) {
    let src = dom.attr(n, "src").or_else(|| dom.attr(n, "data-src")).unwrap_or("");
    if src.is_empty() || src.starts_with("data:") { return; }
    let Some(abs) = sex_html::url::resolve(base, src) else { return };
    out.push_str(&alloc::format!("<p><img src=\"{}\" alt=\"{}\"></p>\n", esc(&abs), esc(dom.attr(n, "alt").unwrap_or(""))));
}

/// Text with links, emphasis and code kept.
fn inline(dom: &Dom, n: NodeId, base: &str, out: &mut String) {
    for c in dom.nodes[n].children.clone() {
        match &dom.nodes[c].kind {
            Kind::Text(t) => {
                let t = t.replace(['\n', '\r', '\t'], " ");
                if out.ends_with(' ') { out.push_str(&esc(t.trim_start())); } else { out.push_str(&esc(&t)); }
            }
            Kind::Element { tag, .. } => {
                if unwanted(dom, c) { continue; }
                match tag.as_str() {
                    "a" => {
                        let mut inner = String::new();
                        inline(dom, c, base, &mut inner);
                        match dom.attr(c, "href").and_then(|h| sex_html::url::resolve(base, h)) {
                            Some(href) if !href.starts_with("javascript:") => out.push_str(&alloc::format!("<a href=\"{}\">{}</a>", esc(&href), inner)),
                            _ => out.push_str(&inner),
                        }
                    }
                    t @ ("em" | "i" | "strong" | "b" | "code" | "sub" | "sup") => {
                        out.push_str(&alloc::format!("<{t}>"));
                        inline(dom, c, base, out);
                        out.push_str(&alloc::format!("</{t}>"));
                    }
                    "br" => out.push_str("<br>"),
                    "table" => {
                        // A table inside a data cell is still a table. The outer
                        // cell already accounts for its visible words.
                        let mut nested_words = 0;
                        table(dom, c, base, out, &mut nested_words);
                    }
                    "img" => {}
                    _ => inline(dom, c, base, out),
                }
            }
            _ => {}
        }
    }
}

/// The reader page for `article` from `url`: readable width, larger type, a link back.
pub fn page(article: &Article, url: &str) -> String {
    let minutes = (article.words + 229) / 230;
    alloc::format!(
        "<!doctype html><title>{t}</title>\
         <style>body{{margin:0;background:#fbf8f1;color:#1f1d1a;font-family:sans-serif}}\
         main{{max-width:680px;margin:0 auto;padding:28px 24px 64px;font-size:19px;line-height:1.6}}\
         h1{{font-size:32px;line-height:1.2;margin:8px 0 6px}} .meta{{color:#6b665d;font-size:14px;margin-bottom:26px}}\
         .meta a{{color:#6b665d}} img{{max-width:100%}} pre{{background:#efeadf;padding:12px;overflow:auto;font-size:15px}}\
         blockquote{{border-left:3px solid #d6cfbf;margin-left:0;padding-left:16px;color:#47433c}} a{{color:#8a3b12}}\
         table{{border-collapse:collapse;width:100%;margin:18px 0}} th,td{{border:1px solid #d6cfbf;padding:7px 10px;text-align:left}}\
         th{{background:#efeadf}}</style>\
         <main><h1>{t}</h1><div class=\"meta\">{m} min read · <a href=\"{u}\">{h}</a></div>\n{body}</main>",
        t = esc(&article.title), m = minutes, u = esc(url), h = esc(host(url)), body = article.html)
}

/// The host part of `url`, for the line under the title.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}
