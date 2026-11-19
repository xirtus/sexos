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

/// A ZIP of (name, method, declared size, payload) entries, for hostile layouts.
fn zip(entries: &[(&str, u16, u32, &[u8])]) -> std::vec::Vec<u8> {
    let (mut out, mut cd) = (std::vec::Vec::new(), std::vec::Vec::new());
    for (name, method, size, data) in entries {
        let offset = out.len() as u32;
        let head = |v: &mut std::vec::Vec<u8>| {
            v.extend_from_slice(&method.to_le_bytes());
            v.extend_from_slice(&[0; 8]); // time, date, crc
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&size.to_le_bytes());
            v.extend_from_slice(&(name.len() as u16).to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04\x14\x00\x00\x00");
        head(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        cd.extend_from_slice(b"PK\x01\x02\x14\x00\x14\x00\x00\x00");
        head(&mut cd);
        cd.extend_from_slice(&[0; 10]); // comment, disk, attributes
        cd.extend_from_slice(&offset.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let at = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(b"PK\x05\x06\x00\x00\x00\x00");
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Issue #319: a 64 MiB declared size over a 2-byte stream is refused, and a
/// folder entry with trailing blanks never becomes a file with an empty path.
#[test]
fn hostile_packages() {
    let manifest = br#"{"manifest_version":3,"name":"x","version":"1"}"#;
    let bomb = zip(&[("manifest.json", 0, manifest.len() as u32, manifest), ("big.js", 8, 64 << 20, b"\x03\x00")]);
    assert!(package::unpack(&bomb).is_err());
    let wrap = zip(&[("wrap/ ", 0, 1, b"x"), ("wrap/manifest.json", 0, manifest.len() as u32, manifest)]);
    let files = package::unpack(&wrap).unwrap();
    assert_eq!(files.iter().map(|(p, _)| p.as_str()).collect::<std::vec::Vec<_>>(), ["manifest.json"]);
    for p in ["a//b", "wrap/", "a/./b", "a/../b", " "] { assert_eq!(clean_path(p), None, "{p:?}"); }
    assert_eq!(clean_path("./js/a.js").as_deref(), Some("js/a.js"));
}

/// Issue #320: `default_locale` can't climb out of `_locales/`.
#[test]
fn locale_cannot_traverse() {
    let m = Manifest::parse(include_str!("../../../fuzz/repros/round4/kaleidoscope-ext-locale-path-traversal.bin")).unwrap();
    assert_eq!(m.default_locale, None);
    assert_eq!(m.locale_file().as_deref(), Some("_locales/en/messages.json"));
    let m = Manifest::parse(r#"{"manifest_version":3,"name":"__MSG_n__","default_locale":"pt_BR"}"#).unwrap();
    assert_eq!(m.locale_file().as_deref(), Some("_locales/pt_BR/messages.json"));
}

/// Issue #321: DNR rules can't smuggle cosmetic syntax or extra Shields options.
#[test]
fn dnr_cannot_smuggle_filter_syntax() {
    for cond in [
        r#"{"urlFilter":"example.com#@#.ad-banner"}"#,
        r#"{"urlFilter":"example.com##.x"}"#,
        r#"{"urlFilter":"a.com\nb.com##.x"}"#,
        r#"{"urlFilter":"ads","initiatorDomains":["evil.com,badfilter"]}"#,
        r#"{"urlFilter":"ads","excludedDomains":["site.com,important"]}"#,
        r#"{"requestDomains":["a.com$badfilter"]}"#,
        r#"{"urlFilter":"ads","excludedRequestDomains":["a.com|b"]}"#,
    ] {
        let rule: serde_json::Value = serde_json::from_str(&alloc::format!(r#"{{"action":{{"type":"block"}},"condition":{cond}}}"#)).unwrap();
        assert_eq!(dnr::rule(&rule), None, "{cond}");
    }
    let rule: serde_json::Value = serde_json::from_str(r#"{"action":{"type":"block"},"condition":{"urlFilter":"||ads.","initiatorDomains":["news.example-site.co.uk"]}}"#).unwrap();
    assert_eq!(dnr::rule(&rule).unwrap(), ["||ads.$domain=news.example-site.co.uk"]);
    let smuggle = include_str!("../../../fuzz/repros/round4/kaleidoscope-ext-dnr-filter-smuggling.bin");
    if let Ok(c) = dnr::convert(smuggle) {
        let mut engine = Engine::new();
        for f in &c.filters { assert!(!f.contains('#'), "{f}"); let _ = engine.add_filter(f); }
        assert_eq!(engine.counts().cosmetic, 0);
    }
}
