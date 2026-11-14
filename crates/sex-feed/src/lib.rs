// SPDX-License-Identifier: GPL-3.0-only
//! The feed model behind Gossip, with no I/O so all of it is tested on the host.
//!
//! * [`parse`] reads RSS 2.0 (and the RSS 0.9x/1.0 shapes close to it) and Atom into a
//!   [`Feed`] of [`Item`]s, each with a stable id, title, link, date, an HTML summary
//!   (shown as text through [`sex_xml::html`]) and a podcast enclosure if it has one.
//! * [`opml`] imports and exports subscription lists.
//! * [`store`] keeps the subscriptions and which items were read, as text files.
#![no_std]

extern crate alloc;

pub mod opml;
pub mod store;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_xml::{html, parse as parse_xml, text_of, Node};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Rss, Atom }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enclosure { pub url: String, pub mime: String, pub length: u64 }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    /// The guid or Atom id; else the link; else the title and date.
    pub id: String,
    pub title: String,
    pub link: String,
    pub date: String,
    /// HTML, as the feed gave it.
    pub summary: String,
    pub enclosure: Option<Enclosure>,
}

impl Item {
    /// The summary as plain text in paragraphs.
    pub fn text(&self) -> String { html::to_text(&self.summary) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    pub kind: Kind,
    pub title: String,
    pub link: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Neither an RSS channel nor an Atom feed.
    NotFeed,
}

/// Reads a feed's bytes (UTF-8, UTF-16 or a declared Latin-1/windows-1251).
pub fn parse_bytes(bytes: &[u8]) -> Result<Feed, Error> { parse(&text_of(bytes)) }

pub fn parse(text: &str) -> Result<Feed, Error> {
    let doc = parse_xml(text);
    if let Some(feed) = doc.child("feed") { return Ok(atom(feed)); }
    let root = doc.child("rss").or_else(|| doc.child("RDF")).ok_or(Error::NotFeed)?;
    let channel = root.child("channel").ok_or(Error::NotFeed)?;
    // RSS 1.0 puts its items beside the channel, 2.0 inside it.
    let items = channel.children_named("item").chain(root.children_named("item")).map(rss_item).collect();
    Ok(Feed { kind: Kind::Rss, title: channel.child_text("title"), link: rss_link(channel), items })
}

/// The RSS `link` with text (an `atom:link` beside it has only an href).
fn rss_link(n: &Node) -> String {
    n.children_named("link").map(Node::text_trim).find(|t| !t.is_empty()).unwrap_or_default()
}

fn rss_item(n: &Node) -> Item {
    let title = n.child_text("title");
    let link = rss_link(n);
    let date = [n.child_text("pubDate"), n.child_text("date")].into_iter().find(|d| !d.is_empty()).unwrap_or_default();
    // content:encoded is the full text; description is often a teaser.
    let summary = n.child("encoded").or_else(|| n.child("description")).map(|d| String::from(d.text().trim())).unwrap_or_default();
    let enclosure = n.child("enclosure").and_then(|e| {
        let url = e.attr("url")?.trim();
        (!url.is_empty()).then(|| Enclosure { url: String::from(url), mime: String::from(e.attr("type").unwrap_or("")), length: e.attr("length").and_then(|l| l.trim().parse().ok()).unwrap_or(0) })
    });
    let guid = n.child_text("guid");
    let id = first_nonempty(&[&guid, &link]).unwrap_or_else(|| format!("{title}|{date}"));
    Item { id, title, link, date, summary, enclosure }
}

fn atom(feed: &Node) -> Feed {
    let items = feed.children_named("entry").map(|e| {
        let title = e.child("title").map(atom_text).unwrap_or_default();
        let link = atom_link(e, "alternate").unwrap_or_default();
        let date = [e.child_text("updated"), e.child_text("published")].into_iter().find(|d| !d.is_empty()).unwrap_or_default();
        let summary = e.child("content").or_else(|| e.child("summary")).map(atom_html).unwrap_or_default();
        let enclosure = e.children_named("link").find(|l| l.attr("rel") == Some("enclosure")).and_then(|l| {
            Some(Enclosure { url: String::from(l.attr("href")?), mime: String::from(l.attr("type").unwrap_or("")), length: l.attr("length").and_then(|v| v.parse().ok()).unwrap_or(0) })
        });
        let id = e.child_text("id");
        let id = first_nonempty(&[&id, &link]).unwrap_or_else(|| format!("{title}|{date}"));
        Item { id, title, link, date, summary, enclosure }
    }).collect();
    Feed { kind: Kind::Atom, title: feed.child("title").map(atom_text).unwrap_or_default(), link: atom_link(feed, "alternate").unwrap_or_default(), items }
}

fn first_nonempty(v: &[&String]) -> Option<String> { v.iter().find(|s| !s.is_empty()).map(|s| (*s).clone()) }

/// An Atom link by its rel (a link with no rel is `alternate`).
fn atom_link(n: &Node, rel: &str) -> Option<String> {
    n.children_named("link").find(|l| l.attr("rel").unwrap_or("alternate") == rel).and_then(|l| l.attr("href")).map(String::from)
}

/// An Atom text construct as plain text.
fn atom_text(n: &Node) -> String {
    match n.attr("type") {
        Some("html") => sex_xml::collapse(&html::to_text(&n.text())),
        _ => n.text_trim(),
    }
}

/// An Atom text construct as HTML.
fn atom_html(n: &Node) -> String {
    match n.attr("type") {
        Some("html") => String::from(n.text().trim()),
        // XHTML: the elements are there already; keep them as text the HTML reader takes.
        Some("xhtml") => {
            let blocks = html::blocks_of(n);
            blocks.iter().map(|b| format!("<p>{}</p>", sex_xml::escape(&b.text))).collect()
        }
        _ => sex_xml::escape(n.text().trim()),
    }
}

/// A short name for a file kept for `url`: FNV-1a of the address in hex.
pub fn hash(url: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() { h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3); }
    format!("{h:016x}")
}

/// Where a feed's last reply is kept, so the list shows without the network.
pub fn cache_name(url: &str) -> String { format!("feed-{}.xml", hash(url)) }

/// The file name for a downloaded enclosure: the address's last path part, made safe,
/// else one made from its hash.
pub fn enclosure_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let path = path.split_once("://").map_or(path, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    let last = path.rsplit('/').next().unwrap_or("");
    let safe: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).take(60).collect();
    if safe.trim_matches('.').is_empty() { format!("episode-{}", &hash(url)[..8]) } else { safe }
}
