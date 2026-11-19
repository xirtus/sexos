// SPDX-License-Identifier: GPL-3.0-only
//! vCard 2.1/3.0/4.0 as phones and other programs write them, written back without
//! loss, searched, and kept in Crush's book with CardDAV state.
use sex_vcard::{export, parse, Book, Contact, SyncState, Typed};

const FIXTURE: &str = include_str!("fixture.vcf");

#[test]
fn reads_every_version() {
    let cards = parse(FIXTURE).unwrap();
    assert_eq!(cards.len(), 3);
    let ada = &cards[0];
    assert_eq!((ada.version, ada.name.as_str(), ada.n[0].as_str(), ada.n[3].as_str()), (3, "Ada Lovelace", "Lovelace", "Countess"));
    assert_eq!(ada.org, "Analytical Engines Ltd, Research");
    assert_eq!(ada.emails, [Typed { kinds: vec!["internet".into(), "work".into(), "pref".into()], value: "ada@engines.example".into() }]);
    assert_eq!(ada.emails[0].label(), "work");
    assert_eq!((ada.phones[0].value.as_str(), ada.phones[0].label()), ("+44 20 7946 0018", "cell"));
    assert_eq!(ada.addresses[0].line(), "12 St James's Square, SW1Y 4JH London, United Kingdom");
    assert_eq!(ada.note, "Wrote the first program, notes on the engine\nand more");
    assert_eq!(ada.categories, ["Friends", "Science"]);
    assert_eq!(ada.other.len(), 2, "{:?}", ada.other);
    let grace = &cards[1];
    assert_eq!((grace.version, grace.uid.as_str()), (4, "4fbe8971-0bc3-424c-9c26-36c3e1eff6b1"));
    assert_eq!((grace.phones[0].value.as_str(), grace.phones[0].kinds.clone()), ("+1-202-555-0143", vec!["voice".to_string(), "work".to_string()]));
    let kurt = &cards[2];
    assert_eq!((kurt.name.as_str(), kurt.n[0].as_str(), kurt.phones[0].label()), ("Kurt Gödel", "Gödel", "work"));
    assert_eq!(parse("hello"), Err(sex_vcard::Error::NotVcard));
}

#[test]
fn writes_back_without_loss() {
    let cards = parse(FIXTURE).unwrap();
    let text = export(&cards);
    assert!(text.lines().all(|l| l.len() <= 76));
    assert!(text.contains("VERSION:3.0\r\n") && text.contains("VERSION:4.0\r\n"));
    assert!(text.contains("EMAIL;TYPE=INTERNET,WORK,PREF:ada@engines.example") && text.contains("TEL;TYPE=voice,work:+1-202-555-0143"));
    let mut back = parse(&text).unwrap();
    // 2.1 is written as 3.0.
    back[2].version = cards[2].version;
    assert_eq!(back, cards);
    let fresh = Contact { emails: vec![Typed::new("work", "x@y")], ..Contact::new("New; person, here") };
    let again = parse(&export([&fresh])).unwrap().remove(0);
    assert_eq!((again.name.as_str(), again.version, again.emails[0].value.as_str()), ("New; person, here", 4, "x@y"));
}

#[test]
fn search_and_sort() {
    let book = Book { contacts: parse(FIXTURE).unwrap(), ..Book::default() };
    let names = |q: &str| book.search(q).iter().map(|&i| book.contacts[i].display()).collect::<Vec<_>>();
    // By family name: Gödel, Hopper, Lovelace.
    assert_eq!(names(""), ["Kurt Gödel", "Grace Hopper", "Ada Lovelace"]);
    assert_eq!(names("ada"), ["Ada Lovelace"]);
    assert_eq!(names("ENGINES"), ["Ada Lovelace"]);
    assert_eq!(names("2025550143"), ["Grace Hopper"]);
    assert_eq!(names("navy grace"), ["Grace Hopper"]);
    assert!(names("nobody").is_empty());
}

#[test]
fn book_keeps_sync_state_and_merges() {
    let mut book = Book { contacts: parse(FIXTURE).unwrap(), url: "/dav/ab/".into(), ctag: "c3".into(), sync_token: "http://x/s/3".into() };
    book.contacts[0].href = "/dav/ab/ada.vcf".into();
    book.contacts[0].etag = "\"e1\"".into();
    book.contacts[0].touch();
    assert_eq!(book.contacts[0].state, SyncState::Modified);
    book.contacts[1].state = SyncState::Deleted;
    let text = book.serialize();
    let back = Book::parse(&text);
    for c in &back.contacts { assert!(c.other.iter().all(|o| !o.starts_with("X-SEXOS"))); }
    let mut expect = book.clone();
    expect.contacts[2].version = 3;
    assert_eq!(back, expect);
    assert_eq!(back.search("").len(), 2);
    // Sync state never goes to a server.
    assert!(!export(&back.contacts).contains("X-SEXOS"));
    let mut b = Book::default();
    assert_eq!(b.merge(parse(FIXTURE).unwrap()), 3);
    assert_eq!(b.merge(parse(FIXTURE).unwrap()), 0);
}
