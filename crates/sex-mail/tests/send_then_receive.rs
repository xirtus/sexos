// Host gate for slice Y3: Valentine's mail core against the in-memory IMAP/SMTP server.
// Send through SMTP, get pushed by IMAP IDLE, fetch, parse, thread, flag, file a sent copy.
use sex_mail::fixture::{FixtureConn, Store};
use sex_mail::message::{compose, parse, Draft};
use sex_mail::thread::{thread, Item};
use sex_mail::{imap, smtp, Auth, Error};

#[test]
fn send_then_receive_with_idle_push() {
    let store = Store::shared();
    store.borrow_mut().add_user("alice@example.org", "alice-app-password");
    store.borrow_mut().add_user("bob@example.org", "bob-oauth-token");

    // Bob's Valentine opens his inbox and idles (Multipass authenticated the flow).
    let mut bob = imap::Client::connect(FixtureConn::imap(&store)).unwrap();
    assert!(matches!(bob.authenticate(&Auth::XOAuth2 { user: "bob@example.org", token: "wrong" }), Err(Error::Refused(_))));
    bob.authenticate(&Auth::XOAuth2 { user: "bob@example.org", token: "bob-oauth-token" }).unwrap();
    let boxes = bob.list().unwrap();
    assert!(boxes.iter().any(|b| b.name == "Sent" && b.flags.iter().any(|f| f == "\\Sent")));
    let sel = bob.select("INBOX").unwrap();
    assert_eq!((sel.exists, sel.uid_validity, sel.uid_next), (0, 1_700_000_000, 1));
    let idle = bob.idle_start().unwrap();

    // Alice sends two messages, the second a reply in the same conversation.
    let mut out = smtp::Client::connect(FixtureConn::smtp(&store), "valentine.sexos").unwrap();
    assert!(out.extensions.iter().any(|e| e.starts_with("AUTH")));
    assert!(out.send("alice@example.org", &["bob@example.org"], b"x").is_err(), "unauthenticated submission is refused");
    out.authenticate(&Auth::Plain { user: "alice@example.org", password: "alice-app-password" }).unwrap();
    let first = compose(&Draft { from: "Alice <alice@example.org>", to: &["Bob <bob@example.org>"], subject: "Dinner on Friday?", body: "Shall we?\n.\nA line that is only a dot survives.", message_id: "1@example.org", date: "Sat, 14 Nov 2026 02:00:00 +0000", in_reply_to: None, references: &[] });
    out.send("alice@example.org", &["bob@example.org"], &first).unwrap();
    assert!(out.send("alice@example.org", &["nobody@example.org"], &first).is_err(), "unknown recipients are refused");

    // IDLE pushes the new mail to Bob without polling.
    assert_eq!(bob.idle_wait().unwrap(), imap::Event::Exists(1));
    bob.idle_done(&idle).unwrap();

    let second = compose(&Draft { from: "Alice <alice@example.org>", to: &["bob@example.org"], subject: "Re: Dinner on Friday?", body: "7pm?", message_id: "2@example.org", date: "Sat, 14 Nov 2026 02:05:00 +0000", in_reply_to: Some("1@example.org"), references: &["1@example.org"] });
    out.send("alice@example.org", &["bob@example.org"], &second).unwrap();
    out.quit().unwrap();

    let unseen = bob.uid_search("UNSEEN").unwrap();
    assert_eq!(unseen, [1, 2]);
    let fetched = bob.uid_fetch("1:*").unwrap();
    assert_eq!(fetched.len(), 2);
    let msgs: Vec<_> = fetched.iter().map(|f| parse(&f.body)).collect();
    assert_eq!(msgs[0].subject(), "Dinner on Friday?");
    assert_eq!(msgs[0].text.as_deref(), Some("Shall we?\r\n.\r\nA line that is only a dot survives.\r\n"));
    assert_eq!(fetched[0].size as usize, fetched[0].body.len());
    let ids: Vec<Option<String>> = msgs.iter().map(|m| m.message_id()).collect();
    let refs: Vec<Vec<String>> = msgs.iter().map(|m| m.references()).collect();
    let subjects: Vec<String> = msgs.iter().map(|m| m.subject()).collect();
    let items: Vec<Item> = (0..2).map(|i| Item { id: ids[i].as_deref(), parents: refs[i].iter().map(String::as_str).collect(), subject: &subjects[i] }).collect();
    let t = thread(&items);
    assert_eq!(t[0], t[1], "the reply joins the conversation");

    // Reading marks seen; the reply is filed in Sent.
    bob.uid_store("1", "+FLAGS", &["\\Seen"]).unwrap();
    assert_eq!(bob.uid_search("UNSEEN").unwrap(), [2]);
    let reply = compose(&Draft { from: "bob@example.org", to: &["alice@example.org"], subject: "Re: Dinner on Friday?", body: "7pm works.", message_id: "3@example.org", date: "Sat, 14 Nov 2026 02:10:00 +0000", in_reply_to: Some("2@example.org"), references: &["1@example.org", "2@example.org"] });
    bob.append("Sent", &["\\Seen"], &reply).unwrap();
    assert_eq!(store.borrow().messages("bob@example.org", "Sent").len(), 1);
    assert!(matches!(bob.append("Nope", &[], b"x"), Err(Error::Refused(_))));
    bob.logout().unwrap();
}

#[test]
fn literal_and_quoted_parsing() {
    assert_eq!(imap::literal_len(b"* 1 FETCH (BODY[] {42}"), Some(42));
    assert_eq!(imap::literal_len(b"* OK no literal"), None);
    assert_eq!(imap::quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
}
