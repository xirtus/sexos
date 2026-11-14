// SPDX-License-Identifier: GPL-3.0-only
extern crate std;

use super::*;
use alloc::vec;
use sex_shields::{Engine, Kind, Request};

const CRX: &[u8] = include_bytes!("../fixtures/sample.crx");
const ZIP: &[u8] = include_bytes!("../fixtures/sample.zip");

#[test]
fn patterns() {
    let p = |s: &str| Pattern::parse(s).unwrap();
    assert!(p("<all_urls>").matches("https://a.b/c?d"));
    assert!(p("<all_urls>").matches("file:///x"));
    assert!(!p("*://*/*").matches("file:///x"));
    assert!(p("*://*.example.com/*").matches("http://example.com/"));
    assert!(p("*://*.example.com/*").matches("https://a.b.example.com:8080/x"));
    assert!(!p("*://*.example.com/*").matches("https://badexample.com/"));
    assert!(p("https://example.com/foo*bar").matches("https://example.com/foo/x/bar"));
    assert!(!p("https://example.com/foo*bar").matches("https://example.com/foo/x/baz"));
    assert!(!p("https://example.com/*").matches("http://example.com/"));
    assert!(p("*://example.com/*").matches("https://example.com"));
    assert_eq!(Pattern::parse("*://exa*mple.com/*"), None);
    assert_eq!(Pattern::parse("chrome://x/*"), None);
}

#[test]
fn packages_unpack() {
    for bytes in [CRX, ZIP] {
        let files = package::unpack(bytes).unwrap();
        let names: std::vec::Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(names.contains(&"manifest.json") && names.contains(&"_locales/en/messages.json") && names.contains(&"rules.json"), "{:?}", names);
        let css = &files.iter().find(|(p, _)| p == "recolour.css").unwrap().1;
        assert!(css.starts_with(b"body{background:#ffe0f0"));
    }
    assert!(package::unpack(b"Cr24\x09\0\0\0").is_err());
    assert!(package::unpack(b"not a zip").is_err());
    assert_eq!(clean_path("../x"), None);
    assert_eq!(clean_path("a/../../x"), None);
    assert_eq!(clean_path("./a/b.js").as_deref(), Some("a/b.js"));
}

#[test]
fn manifest_and_locale() {
    let files = package::unpack(CRX).unwrap();
    let get = |n: &str| core::str::from_utf8(&files.iter().find(|(p, _)| p == n).unwrap().1).unwrap();
    let mut m = Manifest::parse(get("manifest.json")).unwrap();
    assert_eq!(m.version, 3);
    let loc = m.locale_file().unwrap();
    m.localize(get(&loc));
    assert_eq!(m.name, "Recolour Sample");
    assert_eq!(m.rulesets, vec![Ruleset { id: "block".into(), path: "rules.json".into(), enabled: true }]);
    let cs = &m.content_scripts[0];
    assert_eq!((cs.css.clone(), cs.js.clone(), cs.run_at), (vec![String::from("recolour.css")], vec![String::from("mark.js")], RunAt::End));
    assert!(cs.applies("https://news.site/a", false) && !cs.applies("https://news.site/a", true));
    assert!(m.host_permissions[0].matches("https://x.y/"));
    // MV2: hosts among the permissions, background scripts, browser_action.
    let m2 = Manifest::parse(r#"{"manifest_version":2,"name":"Two","version":"1","permissions":["storage","*://*.a.com/*","webRequest"],
        "background":{"scripts":["bg.js","../evil.js"]},"browser_action":{"default_popup":"popup.html"}}"#).unwrap();
    assert_eq!(m2.permissions, vec![String::from("storage"), String::from("webRequest")]);
    assert_eq!(m2.background, vec![String::from("bg.js")]);
    assert_eq!(m2.popup.as_deref(), Some("popup.html"));
    assert!(m2.host_permissions[0].matches("https://b.a.com/"));
    assert_eq!(Manifest::parse(r#"{"manifest_version":4,"name":"x"}"#).unwrap_err(), Error::Version);
}

#[test]
fn dnr_rules_block_through_shields() {
    let json = r#"[
      {"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||ads.example^","resourceTypes":["script","image"]}},
      {"id":2,"action":{"type":"block"},"condition":{"requestDomains":["t1.net","t2.net"],"domainType":"thirdParty"}},
      {"id":3,"action":{"type":"allow"},"condition":{"urlFilter":"||ads.example/ok/"}},
      {"id":4,"action":{"type":"block"},"condition":{"regexFilter":"^https?://x"}},
      {"id":5,"action":{"type":"modifyHeaders","responseHeaders":[]},"condition":{"urlFilter":"x"}},
      {"id":6,"action":{"type":"block"},"condition":{"urlFilter":"/pixel","initiatorDomains":["shop.com"],"excludedInitiatorDomains":["a.shop.com"],"excludedResourceTypes":["main_frame"]}},
      {"id":7,"action":{"type":"allowAllRequests"},"condition":{"urlFilter":"||trusted.org^","resourceTypes":["main_frame"]}},
      {"id":8,"action":{"type":"block"},"condition":{"urlFilter":"/beacon","requestDomains":["cdn.io"]}}
    ]"#;
    let c = dnr::convert(json).unwrap();
    assert_eq!(c.skipped, 2);
    let mut e = Engine::new();
    for f in &c.filters { assert!(e.add_filter(f), "{}", f); }
    let b = |url: &str, page: &str, kind| e.check(&Request { url, page, kind }).blocked();
    assert!(b("https://ads.example/a.js", "https://x.y/", Kind::Script));
    assert!(!b("https://ads.example/a.css", "https://x.y/", Kind::Stylesheet));
    assert!(!b("https://ads.example/ok/a.js", "https://x.y/", Kind::Script));
    assert!(b("https://t2.net/x", "https://x.y/", Kind::Image));
    assert!(!b("https://t2.net/x", "https://www.t2.net/", Kind::Image));
    assert!(b("https://cdn.z/pixel.gif", "https://shop.com/", Kind::Image));
    assert!(!b("https://cdn.z/pixel.gif", "https://a.shop.com/", Kind::Image));
    assert!(!b("https://ads.example/a.js", "https://trusted.org/", Kind::Script));
    assert!(b("https://cdn.io/beacon", "https://x.y/", Kind::Ping));
    assert!(!b("https://cdn2.io/beacon", "https://x.y/", Kind::Ping));
}
