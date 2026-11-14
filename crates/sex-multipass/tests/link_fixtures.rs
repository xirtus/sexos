// Host gate for slice Y1: fixture servers for Mastodon, Matrix, the fediverse and Bluesky
// link accounts, tokens stay in Shibari, apps act through grants and never get a token.
use sex_multipass::address::{query_param, Provider};
use sex_multipass::flows::{Env, Keep};
use sex_multipass::http::{Request, Response, Transport};
use sex_multipass::service::{Act, App, Multipass, Scope};
use sex_multipass::{ApiError, Result};
use shibari::{Keeper, Owner};
use std::collections::HashMap;

type Handler = Box<dyn FnMut(&Request) -> Response>;

struct Net { routes: HashMap<String, Handler>, log: Vec<Request> }

impl Transport for Net {
    fn send(&mut self, r: &Request) -> Result<Response> {
        self.log.push(r.clone());
        let key = format!("{} {}", r.method, r.url.split('?').next().unwrap());
        match self.routes.get_mut(&key) {
            Some(h) => Ok(h(r)),
            None => Ok(Response { status: 404, headers: vec![], body: b"{}".to_vec() }),
        }
    }
}

struct World { net: Net, now: u64, seed: u8 }
impl Env for World {
    fn transport(&mut self) -> &mut dyn Transport { &mut self.net }
    fn random(&mut self, out: &mut [u8]) { for b in out { self.seed = self.seed.wrapping_mul(31).wrapping_add(7); *b = self.seed; } }
    fn now(&self) -> u64 { self.now }
}

fn ok(body: &str) -> Response { Response { status: 200, headers: vec![], body: body.as_bytes().to_vec() } }
fn body(r: &Request) -> String { String::from_utf8(r.body.clone()).unwrap() }
fn form_get(r: &Request, k: &str) -> Option<String> { query_param(&format!("?{}", body(r)), k) }

fn world() -> World { World { net: Net { routes: HashMap::new(), log: vec![] }, now: 1_700_000_000, seed: 1 } }
fn route(w: &mut World, key: &str, h: impl FnMut(&Request) -> Response + 'static) { w.net.routes.insert(key.into(), Box::new(h)); }
fn service() -> Multipass { Multipass::new(Owner(42), Keeper::new(0xfeed, 64)) }

fn mastodon(w: &mut World) {
    let s = "https://social.example";
    route(w, &format!("POST {s}/api/v1/apps"), |r| {
        assert_eq!(form_get(r, "redirect_uris").as_deref(), Some("multipass://callback/mastodon"));
        ok(r#"{"client_id":"cid","client_secret":"csecret"}"#)
    });
    route(w, &format!("POST {s}/oauth/token"), |r| {
        assert_eq!(form_get(r, "code").as_deref(), Some("the-code"));
        assert_eq!(form_get(r, "client_secret").as_deref(), Some("csecret"));
        assert!(form_get(r, "code_verifier").unwrap().len() >= 43);
        ok(r#"{"access_token":"SECRET-MASTODON-TOKEN","token_type":"Bearer"}"#)
    });
    route(w, &format!("GET {s}/api/v1/accounts/verify_credentials"), |r| {
        assert_eq!(r.header_value("authorization"), Some("Bearer SECRET-MASTODON-TOKEN"));
        ok(r#"{"id":"109","username":"alice","acct":"alice","url":"https://social.example/@alice"}"#)
    });
    route(w, &format!("POST {s}/oauth/revoke"), |_| ok("{}"));
    route(w, &format!("GET {s}/api/v1/timelines/home"), |r| {
        if r.header_value("authorization") != Some("Bearer SECRET-MASTODON-TOKEN") { return Response { status: 401, headers: vec![], body: vec![] }; }
        Response { status: 200, headers: vec![("set-cookie".into(), "s=1".into())], body: br#"[{"id":"1","content":"hello"}]"#.to_vec() }
    });
}

fn approve(url: &str, extra: &str) -> String {
    let state = query_param(url, "state").unwrap();
    format!("multipass://callback/mastodon?code=the-code&state={}{extra}", sex_multipass::address::form_encode(&state))
}

#[test]
fn mastodon_fixture_links_an_account_and_apps_never_get_the_token() {
    let mut w = world(); mastodon(&mut w);
    let mut mp = service();
    let url = mp.connect(&mut w, "me", Provider::Mastodon, "@alice@social.example", Keep::ForApps).unwrap();
    assert!(url.starts_with("https://social.example/oauth/authorize?"));
    assert_eq!(query_param(&url, "code_challenge_method").as_deref(), Some("S256"));
    assert!(query_param(&url, "scope").unwrap().contains("write:statuses"));

    let cb = approve(&url, "");
    // Someone else cannot finish my link.
    assert_eq!(mp.finish(&mut w, "intruder", &cb).unwrap_err().code, "link_mismatch");
    let id = mp.finish(&mut w, "me", &cb).unwrap();
    let stamp = mp.book.get(id).unwrap();
    assert!(stamp.verified());
    assert_eq!(stamp.subject.as_deref(), Some("109"));
    assert!(mp.signed_in(id));
    // The callback is single-use.
    assert_eq!(mp.finish(&mut w, "me", &cb).unwrap_err().code, "link_expired");
    // Nothing persisted mentions the token.
    assert!(!mp.book.to_text().contains("SECRET"));

    let qupid = App(9);
    let get = Act { method: "GET", path: "/api/v1/timelines/home", content_type: None, body: b"", headers: &[] };
    assert_eq!(mp.act(&mut w, qupid, id, &get).unwrap_err().code, "no_grant");
    assert_eq!(mp.grant("someone-else", qupid, id, Scope::Read).unwrap_err().code, "unknown_stamp");
    mp.grant("me", qupid, id, Scope::Read).unwrap();
    let r = mp.act(&mut w, qupid, id, &get).unwrap();
    assert_eq!(r.status, 200);
    assert!(String::from_utf8(r.body.clone()).unwrap().contains("hello"));
    assert!(r.header("set-cookie").is_none());
    // A program asking for the raw token is refused.
    assert_eq!(mp.token(qupid, id).unwrap_err().code, "token_never_leaves");
    // Read grant does not allow writes; tokens never go to another server.
    let post = Act { method: "POST", path: "/api/v1/statuses", content_type: Some("application/json"), body: b"{}", headers: &[] };
    assert_eq!(mp.act(&mut w, qupid, id, &post).unwrap_err().code, "no_grant");
    mp.grant("me", qupid, id, Scope::Write).unwrap();
    for path in ["//evil.example/steal", "https://evil.example/", "@evil.example/x", "/x y", "/a\\b"] {
        let a = Act { method: "GET", path, content_type: None, body: b"", headers: &[] };
        let e = mp.act(&mut w, qupid, id, &a).unwrap_err();
        assert!(e.code == "act_path" || e.code == "invalid_network", "{path}: {e:?}");
    }
    assert!(w.net.log.iter().all(|r| r.url.starts_with("https://social.example/")));
    // Disconnecting forgets the token and the grants.
    assert!(mp.disconnect(&w, "me", id));
    assert_eq!(mp.act(&mut w, qupid, id, &get).unwrap_err().code, "no_grant");
}

#[test]
fn verify_only_signs_out_and_keeps_no_session() {
    let mut w = world(); mastodon(&mut w);
    let mut mp = service();
    let url = mp.connect(&mut w, "me", Provider::Mastodon, "https://social.example/@alice", Keep::VerifyOnly).unwrap();
    assert_eq!(query_param(&url, "scope").as_deref(), Some("read:accounts"));
    let id = mp.finish(&mut w, "me", &approve(&url, "")).unwrap();
    assert!(mp.book.get(id).unwrap().verified() && !mp.signed_in(id));
    assert!(w.net.log.iter().any(|r| r.url == "https://social.example/oauth/revoke"));
    // Declined and expired links.
    let url = mp.connect(&mut w, "me", Provider::Mastodon, "@alice@social.example", Keep::VerifyOnly).unwrap();
    assert_eq!(mp.finish(&mut w, "me", &approve(&url, "&error=access_denied")).unwrap_err().code, "link_declined");
    let url = mp.connect(&mut w, "me", Provider::Mastodon, "@alice@social.example", Keep::VerifyOnly).unwrap();
    w.now += 1200;
    assert_eq!(mp.finish(&mut w, "me", &approve(&url, "")).unwrap_err().code, "link_expired");
}

#[test]
fn matrix_sso_links_the_right_account_only() {
    let mut w = world();
    route(&mut w, "GET https://example.org/.well-known/matrix/client", |_| ok(r#"{"m.homeserver":{"base_url":"https://matrix.example.org"}}"#));
    route(&mut w, "GET https://matrix.example.org/_matrix/client/v3/login", |_| ok(r#"{"flows":[{"type":"m.login.password"},{"type":"m.login.sso"}]}"#));
    route(&mut w, "POST https://matrix.example.org/_matrix/client/v3/login", |r| {
        assert!(body(r).contains("m.login.token"));
        ok(r#"{"access_token":"MXTOKEN","user_id":"@alice:example.org"}"#)
    });
    let who = std::rc::Rc::new(std::cell::Cell::new("@alice:example.org"));
    let w2 = who.clone();
    route(&mut w, "GET https://matrix.example.org/_matrix/client/v3/account/whoami", move |_| ok(&format!(r#"{{"user_id":"{}"}}"#, w2.get())));
    route(&mut w, "POST https://matrix.example.org/_matrix/client/v3/logout", |_| ok("{}"));
    let mut mp = service();
    let url = mp.connect(&mut w, "me", Provider::Matrix, "@alice:example.org", Keep::ForApps).unwrap();
    let redirect = query_param(&url, "redirectUrl").unwrap();
    assert!(redirect.starts_with("multipass://callback/matrix?state="));
    let id = mp.finish(&mut w, "me", &format!("{redirect}&loginToken=lt")).unwrap();
    assert!(mp.signed_in(id));
    assert!(!w.net.log.iter().any(|r| r.url.ends_with("/logout")));

    who.set("@mallory:example.org");
    let url = mp.connect(&mut w, "me", Provider::Matrix, "@alice:example.org", Keep::ForApps).unwrap();
    let redirect = query_param(&url, "redirectUrl").unwrap();
    assert_eq!(mp.finish(&mut w, "me", &format!("{redirect}&loginToken=lt")).unwrap_err().code, "link_wrong_account");
    assert!(w.net.log.iter().any(|r| r.url.ends_with("/logout")), "a wrong-account session is signed out");
}

#[test]
fn matrix_without_sso_is_refused_without_asking_for_a_password() {
    let mut w = world();
    route(&mut w, "GET https://m.example/_matrix/client/v3/login", |_| ok(r#"{"flows":[{"type":"m.login.password"}]}"#));
    let mut mp = service();
    assert_eq!(mp.connect(&mut w, "me", Provider::Matrix, "@a:m.example", Keep::VerifyOnly).unwrap_err().code, "matrix_sso_unavailable");
}

#[test]
fn fediverse_bio_proof() {
    let mut w = world();
    route(&mut w, "GET https://threads.example/.well-known/webfinger", |r| {
        assert_eq!(query_param(&r.url, "resource").as_deref(), Some("acct:alice@threads.example"));
        ok(r#"{"subject":"acct:alice@threads.example","links":[{"rel":"self","type":"application/activity+json","href":"https://threads.example/ap/users/17"},{"rel":"http://webfinger.net/rel/profile-page","href":"https://threads.example/@alice"}]}"#)
    });
    let bio = std::rc::Rc::new(std::cell::RefCell::new(String::from("just me")));
    let b2 = bio.clone();
    route(&mut w, "GET https://threads.example/ap/users/17", move |_| ok(&format!(r#"{{"id":"https://threads.example/ap/users/17","summary":"{}"}}"#, b2.borrow())));
    let mut mp = service();
    let c = mp.proof_challenge(&mut w, "me", "@alice@threads.example").unwrap();
    assert!(c.code.starts_with("multipass-"));
    assert_eq!(mp.book.list("me").count(), 1, "the address is saved as a dashed stamp");
    assert_eq!(mp.proof_verify(&mut w, "me", &c.id, None).unwrap_err().code, "proof_missing");
    assert_eq!(mp.proof_verify(&mut w, "you", &c.id, None).unwrap_err().code, "proof_expired");
    *bio.borrow_mut() = format!("hi {}", c.code);
    let id = mp.proof_verify(&mut w, "me", &c.id, None).unwrap();
    let s = mp.book.get(id).unwrap();
    assert!(s.verified() && s.subject.as_deref() == Some("https://threads.example/ap/users/17"));
    assert_eq!(mp.book.list("me").count(), 1);
    assert_eq!(mp.proof_verify(&mut w, "me", &c.id, None).unwrap_err().code, "proof_expired", "proofs are single-use");
}

#[test]
fn bluesky_oauth_with_pkce_par_and_dpop_nonce_retry() {
    let mut w = world();
    route(&mut w, "GET https://public.api.bsky.app/xrpc/com.atproto.identity.resolveHandle", |_| ok(r#"{"did":"did:plc:abc123"}"#));
    route(&mut w, "GET https://plc.directory/did:plc:abc123", |_| ok(r##"{"id":"did:plc:abc123","service":[{"id":"#atproto_pds","type":"AtprotoPersonalDataServer","serviceEndpoint":"https://pds.example"}]}"##));
    route(&mut w, "GET https://pds.example/.well-known/oauth-protected-resource", |_| ok(r#"{"authorization_servers":["https://auth.example"]}"#));
    route(&mut w, "GET https://auth.example/.well-known/oauth-authorization-server", |_| ok(r#"{"issuer":"https://auth.example","authorization_endpoint":"https://auth.example/oauth/authorize","token_endpoint":"https://auth.example/oauth/token","pushed_authorization_request_endpoint":"https://auth.example/oauth/par"}"#));
    let mut par_calls = 0;
    route(&mut w, "POST https://auth.example/oauth/par", move |r| {
        assert!(r.header_value("dpop").is_some());
        par_calls += 1;
        if par_calls == 1 { return Response { status: 400, headers: vec![("DPoP-Nonce".into(), "n1".into())], body: br#"{"error":"use_dpop_nonce"}"#.to_vec() }; }
        assert_eq!(form_get(r, "login_hint").as_deref(), Some("alice.example.social"));
        ok(r#"{"request_uri":"urn:ietf:params:oauth:request_uri:xyz","expires_in":60}"#)
    });
    route(&mut w, "POST https://auth.example/oauth/token", |r| {
        assert!(r.header_value("dpop").is_some());
        ok(r#"{"access_token":"BSKYTOKEN","token_type":"DPoP","sub":"did:plc:abc123","scope":"atproto"}"#)
    });
    route(&mut w, "GET https://pds.example/xrpc/app.bsky.actor.getPreferences", |r| {
        assert_eq!(r.header_value("authorization"), Some("DPoP BSKYTOKEN"));
        assert!(r.header_value("dpop").is_some());
        ok(r#"{"preferences":[]}"#)
    });
    let mut mp = service();
    let url = mp.connect(&mut w, "me", Provider::Bluesky, "alice.example.social", Keep::ForApps).unwrap();
    assert!(url.starts_with("https://auth.example/oauth/authorize?"));
    assert_eq!(query_param(&url, "request_uri").as_deref(), Some("urn:ietf:params:oauth:request_uri:xyz"));
    // The PAR body carried the state; recover it from the logged request.
    let par = w.net.log.iter().rev().find(|r| r.url.ends_with("/oauth/par")).unwrap().clone();
    let state = form_get(&par, "state").unwrap();
    let enc = sex_multipass::address::form_encode(&state);
    assert_eq!(mp.finish(&mut w, "me", &format!("multipass://callback/bluesky?code=c&state={enc}&iss=https%3A%2F%2Fevil.example")).unwrap_err().code, "link_issuer");
    let url = mp.connect(&mut w, "me", Provider::Bluesky, "alice.example.social", Keep::ForApps).unwrap();
    let _ = url;
    let par = w.net.log.iter().rev().find(|r| r.url.ends_with("/oauth/par")).unwrap().clone();
    let enc = sex_multipass::address::form_encode(&form_get(&par, "state").unwrap());
    let id = mp.finish(&mut w, "me", &format!("multipass://callback/bluesky?code=c&state={enc}&iss=https%3A%2F%2Fauth.example")).unwrap();
    let s = mp.book.get(id).unwrap();
    assert_eq!(s.subject.as_deref(), Some("did:plc:abc123"));
    assert_eq!(s.account.profile_url, "https://bsky.app/profile/did:plc:abc123");
    mp.grant("me", App(3), id, Scope::Read).unwrap();
    let r = mp.act(&mut w, App(3), id, &Act { method: "GET", path: "/xrpc/app.bsky.actor.getPreferences", content_type: None, body: b"", headers: &[] }).unwrap();
    assert_eq!(r.status, 200);
}

#[test]
fn dav_server_account_for_date_and_crush() {
    let mut w = world();
    route(&mut w, "PROPFIND https://cloud.example/remote.php/dav/", |r| {
        // alice:app-pass
        if r.header_value("authorization") == Some("Basic YWxpY2U6YXBwLXBhc3M=") { Response { status: 207, headers: vec![], body: b"<multistatus/>".to_vec() } }
        else { Response { status: 401, headers: vec![], body: vec![] } }
    });
    route(&mut w, "REPORT https://cloud.example/remote.php/dav/calendars/alice/home/", |r| {
        assert_eq!(r.header_value("authorization"), Some("Basic YWxpY2U6YXBwLXBhc3M="));
        assert_eq!(r.header_value("depth"), Some("1"));
        Response { status: 207, headers: vec![], body: b"<multistatus>events</multistatus>".to_vec() }
    });
    let mut mp = service();
    let e = mp.add_server_account(&mut w, "me", Provider::Dav, "https://cloud.example/remote.php/dav", "alice", "wrong").unwrap_err();
    assert_eq!(e.code, "bad_password");
    let id = mp.add_server_account(&mut w, "me", Provider::Dav, "https://cloud.example/remote.php/dav", "alice", "app-pass").unwrap();
    assert!(mp.signed_in(id) && !mp.book.get(id).unwrap().verified());
    assert!(!mp.book.to_text().contains("app-pass"));
    let date = App(11);
    mp.grant("me", date, id, Scope::Read).unwrap();
    let report = Act { method: "REPORT", path: "/remote.php/dav/calendars/alice/home/", content_type: Some("application/xml"), body: b"<c:calendar-query/>", headers: &[("Depth", "1")] };
    assert_eq!(mp.act(&mut w, date, id, &report).unwrap().status, 207);
    let sneaky = Act { headers: &[("Authorization", "Basic x")], ..report };
    assert_eq!(mp.act(&mut w, date, id, &sneaky).unwrap_err().code, "act_header");
    let put = Act { method: "PUT", path: "/remote.php/dav/calendars/alice/home/e.ics", content_type: Some("text/calendar"), body: b"BEGIN:VCALENDAR", headers: &[] };
    assert_eq!(mp.act(&mut w, date, id, &put).unwrap_err().code, "no_grant", "a read grant cannot write events");
}

#[test]
fn errors_are_api_errors() {
    let e: ApiError = ApiError::new("x", "y");
    assert_eq!(e.code, "x");
}
