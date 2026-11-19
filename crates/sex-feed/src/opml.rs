// SPDX-License-Identifier: GPL-3.0-only
//! OPML subscription lists: every `outline` with an `xmlUrl`, at any depth (folders
//! are flattened), and an OPML 2.0 file written back from a list.
use crate::store::Subscription;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_xml::{escape, parse, Node};

pub fn import(text: &str) -> Vec<Subscription> {
    let doc = parse(text);
    let mut outlines = Vec::new();
    collect(&doc, &mut outlines);
    let mut subs: Vec<Subscription> = Vec::new();
    for o in outlines {
        let Some(url) = o.attr("xmlUrl").map(str::trim).filter(|u| !u.is_empty()) else { continue };
        if subs.iter().any(|s| s.url == url) { continue; }
        let title = o.attr("title").or(o.attr("text")).unwrap_or("").trim();
        subs.push(Subscription { url: String::from(url), title: String::from(title) });
    }
    subs
}

fn collect<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    for e in n.elements() {
        if e.is("outline") { out.push(e); }
        collect(e, out);
    }
}

pub fn export(subs: &[Subscription]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head><title>Gossip subscriptions</title></head>\n  <body>\n");
    for f in subs {
        let title = escape(if f.title.is_empty() { &f.url } else { &f.title });
        s.push_str(&format!("    <outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{}\"/>\n", escape(&f.url)));
    }
    s.push_str("  </body>\n</opml>\n");
    s
}
