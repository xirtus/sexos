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
