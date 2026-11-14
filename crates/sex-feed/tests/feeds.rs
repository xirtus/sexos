// SPDX-License-Identifier: GPL-3.0-only
use sex_feed::opml;
use sex_feed::store::{ReadState, Subscription, Subscriptions};
use sex_feed::{cache_name, enclosure_name, parse, parse_bytes, Kind};

const RSS: &str = include_str!("fixture.rss");

#[test]
fn rss_channel_items_and_enclosure() {
    let f = parse(RSS).unwrap();
    assert_eq!(f.kind, Kind::Rss);
    assert_eq!(f.title, "Gossip Fixture Radio");
    assert_eq!(f.link, "http://example.org/");
    assert_eq!(f.items.len(), 3);
    let e = &f.items[0];
    assert_eq!((e.id.as_str(), e.title.as_str()), ("fixture-3", "Episode 3: Lanterns & Ferries"));
    assert_eq!(e.date, "Sat, 14 Nov 2026 09:00:00 GMT");
    assert_eq!(e.text(), "The third episode.\n\nSecond paragraph.");
    let enc = e.enclosure.as_ref().unwrap();
    assert_eq!((enc.url.as_str(), enc.mime.as_str(), enc.length), ("http://10.0.2.2/episode3.mp3", "audio/mpeg", 16));
    // content:encoded wins over the teaser; dc:date stands in for pubDate.
    let full = &f.items[1];
    assert_eq!(full.text(), "Full text\n\nEverything, with a link.");
    assert_eq!(full.date, "2026-11-13T08:00:00Z");
    assert!(full.enclosure.is_none());
    // Without a guid the link is the id.
    assert_eq!(f.items[2].id, "http://example.org/1");
}

#[test]
fn atom_entries() {
    let f = parse(r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title type="html">Atom &lt;b&gt;Fixture&lt;/b&gt;</title>
  <link rel="self" href="http://example.org/atom.xml"/><link href="http://example.org/"/>
  <entry>
    <title>First entry</title><id>urn:uuid:1</id><updated>2026-11-14T10:00:00Z</updated>
    <link rel="alternate" href="http://example.org/e1"/>
    <link rel="enclosure" type="audio/ogg" length="99" href="http://example.org/e1.ogg"/>
    <summary type="html">&lt;p&gt;Hello &lt;i&gt;there&lt;/i&gt;&lt;/p&gt;</summary>
  </entry>
  <entry>
    <title>Second</title><id>urn:uuid:2</id><published>2026-11-13T10:00:00Z</published>
    <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml"><p>One &amp; two</p><p>Three</p></div></content>
  </entry>
  <entry><title>Third</title><id>urn:uuid:3</id><summary>Plain &lt;not a tag&gt;</summary></entry>
</feed>"#).unwrap();
    assert_eq!(f.kind, Kind::Atom);
    assert_eq!(f.title, "Atom Fixture");
    assert_eq!(f.link, "http://example.org/");
    assert_eq!(f.items.len(), 3);
    assert_eq!(f.items[0].link, "http://example.org/e1");
    assert_eq!(f.items[0].text(), "Hello there");
    assert_eq!(f.items[0].enclosure.as_ref().unwrap().url, "http://example.org/e1.ogg");
    assert_eq!(f.items[1].date, "2026-11-13T10:00:00Z");
    assert_eq!(f.items[1].text(), "One & two\n\nThree");
    assert_eq!(f.items[2].text(), "Plain <not a tag>");
}

#[test]
fn not_a_feed_and_latin1() {
    assert!(parse("<html><body>hi</body></html>").is_err());
    let f = parse_bytes(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><rss><channel><title>Caf\xE9</title></channel></rss>").unwrap();
    assert_eq!(f.title, "Caf\u{E9}");
    assert!(f.items.is_empty());
}

#[test]
fn opml_round_trip() {
    let subs = opml::import(r#"<?xml version="1.0"?><opml version="1.0"><head/><body>
      <outline text="News"><outline text="One" xmlUrl="http://a/1.xml"/><outline title="Two &amp; more" text="x" xmlUrl="http://a/2.xml"/></outline>
      <outline text="Dup" xmlUrl="http://a/1.xml"/><outline text="No url"/></body></opml>"#);
    assert_eq!(subs, [Subscription { url: "http://a/1.xml".into(), title: "One".into() }, Subscription { url: "http://a/2.xml".into(), title: "Two & more".into() }]);
    let text = opml::export(&subs);
    assert!(text.contains("title=\"Two &amp; more\""));
    assert_eq!(opml::import(&text), subs);
}

#[test]
fn subscriptions_and_read_state() {
    let mut s = Subscriptions::default();
    assert!(s.add("http://a/1.xml", ""));
    assert!(!s.add("http://a/1.xml", "again"));
    assert_eq!(s.merge(&[Subscription { url: "http://a/2.xml".into(), title: "Two".into() }, Subscription { url: "http://a/1.xml".into(), title: "".into() }]), 1);
    assert!(s.retitle("http://a/1.xml", "One"));
    let again = Subscriptions::parse(&s.serialize());
    assert_eq!(again, s);
    assert_eq!(again.feeds[0].title, "One");
    s.remove("http://a/1.xml");
    assert_eq!(s.feeds.len(), 1);

    let mut r = ReadState::default();
    assert!(r.set("http://a/1.xml", "fixture-3", true));
    assert!(!r.set("http://a/1.xml", "fixture-3", true));
    r.set("http://a/2.xml", "x", true);
    let again = ReadState::parse(&r.serialize());
    assert!(again.is_read("http://a/1.xml", "fixture-3"));
    assert!(!again.is_read("http://a/2.xml", "fixture-3"));
    r.set("http://a/1.xml", "fixture-3", false);
    assert!(!r.is_read("http://a/1.xml", "fixture-3"));
    r.forget("http://a/2.xml");
    assert!(r.is_empty());
}

#[test]
fn file_names() {
    assert_eq!(enclosure_name("http://10.0.2.2:8000/pod/episode3.mp3?x=1"), "episode3.mp3");
    assert!(enclosure_name("http://example.org/").starts_with("episode-"));
    assert!(enclosure_name("http://example.org/../..").starts_with("episode-"));
    assert_eq!(cache_name("http://a"), cache_name("http://a"));
    assert_ne!(cache_name("http://a"), cache_name("http://b"));
}
