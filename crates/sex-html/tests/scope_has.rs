// SPDX-License-Identifier: GPL-3.0-only
//! `:scope` and `:has()` as uBlock Origin Lite's dashboard uses them.
use sex_html::{css, dom::Dom, parse};

fn by_id(dom: &Dom, id: &str) -> usize { dom.descendants(0).into_iter().find(|&n| dom.attr(n, "id") == Some(id)).unwrap() }
fn ids(dom: &Dom, found: Vec<usize>) -> Vec<String> { found.into_iter().map(|n| dom.attr(n, "id").unwrap_or("?").to_string()).collect() }
fn q(dom: &Dom, root: usize, sel: &str) -> Vec<String> {
    let list = css::parse_selector_list(sel);
    assert!(!list.is_empty(), "{sel} should parse");
    ids(dom, dom.query(root, &list, true))
}

const PAGE: &str = "<div id=pane>\
    <div id=bar class=detailbar><input id=top></div>\
    <div id=lists class=listEntries><div id=e1 class=listEntry data-rulesetid=x><div class=detailbar><input id=deep></div></div><div id=e2 class=listEntry></div></div>\
    <ul id=u1><li id=l1 class=selector></li></ul><ul id=u2><li id=l2></li></ul>\
    <p id=p1></p><span id=s1></span><p id=p2></p>\
    </div>";

#[test]
fn scope_is_the_element_queried_from() {
    let dom = parse::parse(PAGE);
    let pane = by_id(&dom, "pane");
    // Only the pane's own detail bar, not the one inside a list entry.
    assert_eq!(q(&dom, pane, ":scope > .detailbar input"), ["top"]);
    assert_eq!(q(&dom, pane, ":scope > .listEntries .listEntry[data-rulesetid]"), ["e1"]);
    let lists = by_id(&dom, "lists");
    assert_eq!(q(&dom, lists, ":scope > .listEntry"), ["e1", "e2"]);
    assert!(q(&dom, lists, ":scope > .detailbar").is_empty());
    // In a style sheet (no query), :scope is the root element.
    let html = parse::parse("<html id=h><body id=b></body></html>");
    let s = css::parse_selector(":scope").unwrap();
    let root = by_id(&html, "h");
    assert!(css::matches(&html, root, &s) && !css::matches(&html, by_id(&html, "b"), &s));
    // element.matches(':scope') is true of the element itself.
    assert!(css::matches_scoped(&dom, pane, &s, pane) && !css::matches_scoped(&dom, lists, &s, pane));
}

#[test]
fn has_looks_down_and_along() {
    let dom = parse::parse(PAGE);
    assert_eq!(q(&dom, 0, "ul:has(li.selector)"), ["u1"]);
    assert_eq!(q(&dom, 0, "ul:has(> li)"), ["u1", "u2"]);
    assert_eq!(q(&dom, 0, ".listEntry:has(input)"), ["e1"]);
    assert_eq!(q(&dom, 0, "div:has(> .listEntry[data-rulesetid])"), ["lists"]);
    // Siblings: `+` the next one only, `~` any later one.
    assert_eq!(q(&dom, 0, "p:has(+ span)"), ["p1"]);
    assert_eq!(q(&dom, 0, "p:has(~ p)"), ["p1"]);
    assert_eq!(q(&dom, 0, "ul:not(:has(.selector))"), ["u2"]);
    assert_eq!(q(&dom, 0, "li:has(.nothing), #s1"), ["s1"]);
    // As in the dashboard: a scope-relative :has.
    let pane = by_id(&dom, "pane");
    assert_eq!(q(&dom, pane, ":scope > :has(li.selector)"), ["u1"]);
    // :has() can't nest.
    assert!(css::parse_selector("div:has(:has(p))").is_none());
    // Specificity counts the most specific argument, like :is().
    assert_eq!(css::parse_selector("ul:has(#l1)").unwrap().specificity, (1 << 16) + 1);
}
