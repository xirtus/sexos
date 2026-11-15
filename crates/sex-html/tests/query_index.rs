use sex_html::{css, dom::{Dom, Kind}, parse};
fn check(dom: &Dom, roots: &[usize]) {
    for &root in roots {
        for text in ["#dup", ".a", "span", "div > .a", "#root span", ".a + span", ".a ~ span", "span, #dup, .a", ":is(.a, #dup)", "span:not(.b)", "*:first-child", "*:last-child", ":hover", ":focus", ":empty", "#absent", "div .a:not(:is(.b, #absent))"] {
            let selectors = css::parse_selector_list(text);
            let expected: Vec<_> = dom.descendants(root).into_iter().skip(1)
                .filter(|&n| selectors.iter().any(|s| css::matches(dom, n, s))).collect();
            assert_eq!(dom.query(root, &selectors, true), expected, "{text} root={root}");
            assert_eq!(dom.query(root, &selectors, false), expected.into_iter().take(1).collect::<Vec<_>>(), "first {text}");
        }
    }
}
#[test]
fn indexed_queries_match_scan_across_mutations_and_detached_roots() {
    let mut dom = parse::parse("<div id=root><span id=dup class='a b a'></span><span id=dup class=a></span><div class=a><span></span></div></div>");
    let root = dom.descendants(0).into_iter().find(|&n| dom.attr(n,"id")==Some("root")).unwrap();
    let children=dom.nodes[root].children.clone(); let (first,second)=(children[0],children[1]);
    check(&dom,&[0,root]);
    dom.insert(root,second,Some(first)); check(&dom,&[0,root]);
    assert_eq!(dom.query(root,&css::parse_selector_list("#dup"),false),[second]);
    dom.set_attr(second,"class","b"); dom.remove_attr(first,"id"); check(&dom,&[0,root]);
    dom.hover=Some(first); dom.caret=Some((second,0)); check(&dom,&[0,root]);
    dom.detach(root); check(&dom,&[0,root]);
    let new=dom.create(Kind::Element{tag:"span".into(),attrs:vec![("class".into(),"a".into())]});
    dom.insert(root,new,Some(second)); check(&dom,&[0,root]);
    dom.set_text(children[2],"text"); check(&dom,&[root]);
    dom.clear(root); parse::parse_into(&mut dom,root,"<span class=a id=dup></span><span class=b></span>"); check(&dom,&[0,root]);
    dom.insert(0,root,None); check(&dom,&[0,root]);
}
#[test]
fn oversized_index_falls_back_and_can_rebuild_after_mutation() {
    let mut dom=Dom::new();
    let n=dom.add(0,Kind::Element{tag:"span".into(),attrs:vec![("id".into(),"x".repeat(2*1024*1024+1))]});
    assert_eq!(dom.query(0,&css::parse_selector_list("span"),true),[n]);
    dom.set_attr(n,"id","dup");
    check(&dom,&[0]);
}
#[test]
fn indexes_follow_many_small_mutations() {
    // Script start-up interleaves attribute changes, new nodes and moves with queries;
    // the indexes are updated in place and must keep agreeing with a plain scan.
    let mut dom = parse::parse("<div id=root><p class='a b'>x</p><p id=dup></p><section><span class=b></span></section></div>");
    let mut seed = 0x9e3779b9u32;
    let mut next = |m: usize| { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed as usize % m };
    let names = ["id", "class", "title"];
    let values = ["a", "b", "a b", "dup", "", "c a"];
    for step in 0..400 {
        let len = dom.nodes.len();
        let n = next(len);
        match next(7) {
            0 => dom.set_attr(n, names[next(3)], values[next(values.len())]),
            1 => dom.remove_attr(n, names[next(3)]),
            2 => { let tag = ["span", "p", "div"][next(3)]; let c = dom.create(Kind::Element { tag: tag.into(), attrs: vec![("class".into(), values[next(values.len())].into())] }); if step % 2 == 0 { dom.insert(n, c, None); } }
            3 => { let m = next(len); let before = dom.nodes[n].children.first().copied(); dom.insert(n, m, before); }
            4 => dom.detach(n),
            5 => { dom.add(n, Kind::Element { tag: "p".into(), attrs: vec![("id".into(), "dup".into())] }); }
            _ => {}
        }
        if step % 5 == 0 {
            let roots = [0, n, next(dom.nodes.len())];
            check(&dom, &roots);
        }
    }
}
