// Host contract: Valentine can use authenticated flows without seeing a secret.
use sex_mail::fixture::{FixtureConn, Store};
use sex_mail::{imap, smtp, Auth};

#[test]
fn imap_and_smtp_resume_after_account_service_authenticates() {
    let store = Store::shared();
    store.borrow_mut().add_user("alice@example.org", "secret-only-in-account-service");

    // The account service owns credentials and completes SASL before handoff.
    let mut imap_auth = imap::Client::connect(FixtureConn::imap(&store)).unwrap();
    imap_auth.authenticate(&Auth::Plain { user: "alice@example.org", password: "secret-only-in-account-service" }).unwrap();
    let mut inbox = imap::Client::resume_authenticated(imap_auth.into_conn());
    assert!(inbox.authenticated);
    assert!(inbox.list().unwrap().iter().any(|b| b.name == "INBOX"));
    assert_eq!(inbox.select("INBOX").unwrap().exists, 0);

    let mut smtp_auth = smtp::Client::connect(FixtureConn::smtp(&store), "multipass.sexos").unwrap();
    smtp_auth.authenticate(&Auth::Plain { user: "alice@example.org", password: "secret-only-in-account-service" }).unwrap();
    let mut submit = smtp::Client::resume_authenticated(smtp_auth.into_conn());
    submit.send("alice@example.org", &["alice@example.org"], b"From: alice@example.org\r\nTo: alice@example.org\r\nSubject: Handoff\r\n\r\nHello\r\n").unwrap();
    assert_eq!(store.borrow().messages("alice@example.org", "INBOX").len(), 1);
    assert_eq!(inbox.uid_search("ALL").unwrap(), [1]);
}
