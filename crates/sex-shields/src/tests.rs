// SPDX-License-Identifier: GPL-3.0-only
//! Filter matching goldens. `SEXOS_SHIELDS_LISTS=dir` also loads that folder's
//! easylist.txt and easyprivacy.txt (the real lists) and checks them.
extern crate std;

use super::*;
use alloc::vec;

fn engine(list: &str) -> Engine { let mut e = Engine::new(); e.add_list(list); e }
fn blocked(e: &Engine, url: &str, page: &str, kind: Kind) -> bool { e.check(&Request { url, page, kind }).blocked() }

#[test]
fn host_anchor_and_separator() {
    let e = engine("||ads.example.com^");
    let page = "https://news.site/";
    assert!(blocked(&e, "https://ads.example.com/x.js", page, Kind::Script));
    assert!(blocked(&e, "http://cdn.ads.example.com:8080/x", page, Kind::Image));
    assert!(blocked(&e, "https://ads.example.com", page, Kind::Image));
    assert!(!blocked(&e, "https://badads.example.com/x", page, Kind::Image));
    assert!(!blocked(&e, "https://ads.example.community/x", page, Kind::Image));
    assert!(!blocked(&e, "https://site/?u=ads.example.com", page, Kind::Image));
}

#[test]
fn plain_wildcards_and_anchors() {
    let e = engine("/banner/*/img^\n|https://evil.\nswf|\n&adid=*&");
    let p = "https://a.b/";
    assert!(blocked(&e, "https://x.com/banner/foo/img?x", p, Kind::Image));
    assert!(blocked(&e, "https://x.com/banner/foo/img", p, Kind::Image));
    assert!(!blocked(&e, "https://x.com/banner/foo/imgs", p, Kind::Image));
    assert!(blocked(&e, "https://evil.net/", p, Kind::Other));
    assert!(!blocked(&e, "http://x.com/?https://evil.", p, Kind::Other));
    assert!(blocked(&e, "https://x.com/a.swf", p, Kind::Object));
    assert!(!blocked(&e, "https://x.com/a.swf?x", p, Kind::Object));
    assert!(blocked(&e, "https://x.com/?a=1&adid=9&b", p, Kind::Xhr));
}

#[test]
fn types_and_party() {
    let e = engine("||track.net^$script,third-party\n/pixel.$image,~third-party\n||cdn.net^$~script");
    assert!(blocked(&e, "https://track.net/t.js", "https://shop.com/", Kind::Script));
    assert!(!blocked(&e, "https://track.net/t.png", "https://shop.com/", Kind::Image));
    assert!(!blocked(&e, "https://track.net/t.js", "https://www.track.net/", Kind::Script), "first-party");
    assert!(blocked(&e, "https://shop.com/pixel.gif", "https://www.shop.com/", Kind::Image));
    assert!(!blocked(&e, "https://other.com/pixel.gif", "https://www.shop.com/", Kind::Image));
    assert!(blocked(&e, "https://cdn.net/a.png", "https://x.org/", Kind::Image));
    assert!(!blocked(&e, "https://cdn.net/a.js", "https://x.org/", Kind::Script));
    // co.uk is a public suffix: these two are different sites.
    let e = engine("||a.co.uk^$third-party");
    assert!(blocked(&e, "https://a.co.uk/x", "https://b.co.uk/", Kind::Image));
    assert!(!blocked(&e, "https://a.co.uk/x", "https://www.a.co.uk/", Kind::Image));
}

#[test]
fn exceptions_and_important() {
    let e = engine("||ads.net^\n@@||ads.net/ok/$image\n||bad.net^$important\n@@||bad.net^");
    let p = "https://site.org/";
    assert!(blocked(&e, "https://ads.net/x.png", p, Kind::Image));
    assert!(matches!(e.check(&Request { url: "https://ads.net/ok/x.png", page: p, kind: Kind::Image }), Verdict::Excepted(_)));
    assert!(blocked(&e, "https://ads.net/ok/x.js", p, Kind::Script));
    assert!(blocked(&e, "https://bad.net/x", p, Kind::Script), "important beats exceptions");
    match e.check(&Request { url: "https://ads.net/x.png", page: p, kind: Kind::Image }) {
        Verdict::Block(id) => assert_eq!(e.filter_text(id), "||ads.net^"),
        v => panic!("{:?}", v),
    }
}

#[test]
fn domain_option() {
    let e = engine("/promo.js$domain=news.com|~sports.news.com\n||cdn.x^$domain=google.*");
    assert!(blocked(&e, "https://cdn.y/promo.js", "https://www.news.com/a", Kind::Script));
    assert!(!blocked(&e, "https://cdn.y/promo.js", "https://sports.news.com/a", Kind::Script));
    assert!(!blocked(&e, "https://cdn.y/promo.js", "https://other.com/", Kind::Script));
    assert!(blocked(&e, "https://cdn.x/a", "https://www.google.co.uk/", Kind::Script));
    assert!(blocked(&e, "https://cdn.x/a", "https://google.de/", Kind::Script));
    assert!(!blocked(&e, "https://cdn.x/a", "https://notgoogle.de/", Kind::Script));
}

#[test]
fn page_exceptions_and_documents() {
    let e = engine("||ads.net^\n||shop.com^$image\n@@||trusted.org^$document\n@@||shop.com^$generichide\n||site.com^$document");
    assert_eq!(e.page_exceptions("https://www.trusted.org/x"), (true, true, false));
    assert_eq!(e.page_exceptions("https://shop.com/"), (false, false, true));
    assert_eq!(e.page_exceptions("https://other.com/"), (false, false, false));
    assert!(!blocked(&e, "https://ads.net/x", "https://trusted.org/", Kind::Script));
    // `$generichide` is about hiding only: requests from and to the site are judged as usual.
    assert!(blocked(&e, "https://ads.net/x", "https://shop.com/", Kind::Script));
    assert!(!blocked(&e, "https://shop.com/x", "https://a.b/", Kind::Script));
    // `$document` blocking filters are left out: tabs are never blocked.
    assert_eq!(e.counts().skipped, 1);
}

#[test]
fn unsupported_filters_are_skipped() {
    let mut e = Engine::new();
    for f in ["/ads[0-9]+\\.js/", "||x.com^$csp=script-src 'none'", "||y.com^$removeparam=utm", "example.com##+js(nobab)", "x.com#?#div:has-text(Ad)", "##div:has(> .ad)", "||z.com^$popup", "x.com#$#body { overflow: auto }"] {
        assert!(!e.add_filter(f), "{}", f);
    }
    assert_eq!(e.counts(), Counts { network: 0, cosmetic: 0, skipped: 8 });
    assert!(e.add_filter("! comment") == false && e.counts().skipped == 8);
}

#[test]
fn hosts_and_badfilter() {
    let e = engine("0.0.0.0 tracker.io\n127.0.0.1 localhost\n||ads.net^$script\n||ads.net^$script,badfilter");
    assert!(blocked(&e, "https://tracker.io/x", "https://a.b/", Kind::Image));
    assert!(!blocked(&e, "https://ads.net/x.js", "https://a.b/", Kind::Script));
    assert_eq!(e.counts().network, 1);
}

#[test]
fn cosmetic() {
    let e = engine("##.ad-box\n##div[id^=\"gpt-\"]\n##.promo > a.x\n##.rare\nnews.com##.paywall\n~shop.news.com##.subscribe\nnews.com#@#.ad-box\ngoogle.*##.g-ad\n#@#.rare\n@@||clean.org^$elemhide");
    let classes = ["ad-box", "promo", "rare", "subscribe", "g-ad"];
    let has_class = |c: &str| classes.contains(&c);
    let no_id = |_: &str| false;
    let h = e.hidden("https://www.news.com/a", &has_class, &no_id);
    assert_eq!(h, vec![".paywall", ".promo > a.x", ".subscribe", "div[id^=\"gpt-\"]"]);
    let h = e.hidden("https://shop.news.com/", &has_class, &no_id);
    assert_eq!(h, vec![".paywall", ".promo > a.x", "div[id^=\"gpt-\"]"]);
    let h = e.hidden("https://other.net/", &|_: &str| false, &no_id);
    assert_eq!(h, vec![".subscribe", "div[id^=\"gpt-\"]"]);
    let h = e.hidden("https://images.google.fr/", &has_class, &no_id);
    assert!(h.contains(&".g-ad") && h.contains(&".ad-box"));
    assert!(e.hidden("https://clean.org/", &has_class, &no_id).is_empty());
    let css = e.css("https://other.net/", &has_class, &no_id);
    assert!(css.contains(".ad-box{display:none!important}\n"));
}

#[test]
fn selector_keys() {
    assert_eq!(selector_key(".ad").as_deref(), Some(".ad"));
    assert_eq!(selector_key("div#top-ad > span").as_deref(), Some("#top-ad"));
    assert_eq!(selector_key("a[href*=\".x\"]"), None);
    assert_eq!(selector_key("div:not(.a)"), None);
    assert_eq!(selector_key("[data-x] .b").as_deref(), Some(".b"));
}

#[test]
fn guesses() {
    assert_eq!(Kind::guess("https://a/b/c.JS?x=1"), Kind::Script);
    assert_eq!(Kind::guess("https://a/b.png#f"), Kind::Image);
    assert_eq!(Kind::guess("https://a/collect"), Kind::Other);
    assert_eq!(Kind::guess("https://a.com/"), Kind::Other);
}

#[test]
fn starter_list_blocks_the_usual_trackers() {
    let e = engine(STARTER_LIST);
    assert_eq!(e.counts().skipped, 0);
    let page = "https://news.example/";
    for u in ["https://www.google-analytics.com/analytics.js", "https://www.googletagmanager.com/gtag/js?id=G-1", "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
              "https://connect.facebook.net/en_US/fbevents.js", "https://static.hotjar.com/c/hotjar-1.js", "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js"] {
        assert!(blocked(&e, u, page, Kind::Script), "{}", u);
    }
    assert!(!blocked(&e, "https://news.example/app.js", page, Kind::Script));
    assert!(!blocked(&e, "https://fonts.googleapis.com/css2?family=Inter", page, Kind::Stylesheet));
}

#[test]
fn real_lists() {
    let Ok(dir) = std::env::var("SEXOS_SHIELDS_LISTS") else { return };
    let mut e = Engine::new();
    let t0 = std::time::Instant::now();
    for name in ["easylist.txt", "easyprivacy.txt"] {
        let text = std::fs::read_to_string(std::format!("{}/{}", dir, name)).unwrap();
        e.add_list(&text);
    }
    let parse = t0.elapsed();
    let c = e.counts();
    std::println!("real lists: {:?} parsed in {:?}", c, parse);
    assert!(c.network > 30_000 && c.cosmetic > 10_000);
    let page = "https://www.example-news.com/article";
    let t1 = std::time::Instant::now();
    let trackers = ["https://www.google-analytics.com/analytics.js", "https://www.googletagmanager.com/gtm.js?id=GTM-X", "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
        "https://connect.facebook.net/en_US/fbevents.js", "https://static.hotjar.com/c/hotjar-1.js", "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
        "https://sb.scorecardresearch.com/beacon.js", "https://cdn.taboola.com/libtrc/x/loader.js", "https://c.amazon-adsystem.com/aax2/apstag.js",
        "https://bat.bing.com/bat.js", "https://www.clarity.ms/tag/abc", "https://cdn.segment.com/analytics.js/v1/x/analytics.min.js"];
    for u in trackers { assert!(blocked(&e, u, page, Kind::Script), "{}", u); }
    let fine = ["https://www.example-news.com/static/app.js", "https://fonts.googleapis.com/css2?family=Inter", "https://cdn.jsdelivr.net/npm/react@18.3.1/umd/react.production.min.js",
        "https://upload.wikimedia.org/wikipedia/commons/a/a9/Example.jpg", "https://github.githubassets.com/assets/app.js"];
    for u in fine { assert!(!blocked(&e, u, page, Kind::guess(u)), "{}", u); }
    let mut n = 0;
    for _ in 0..1000 { for u in trackers.iter().chain(fine.iter()) { n += e.check(&Request { url: u, page, kind: Kind::Script }).blocked() as usize; } }
    std::println!("17000 checks in {:?} ({} blocked)", t1.elapsed(), n);
    let h = e.hidden(page, &|c: &str| c == "adsbygoogle" || c == "ad-slot", &|_: &str| false);
    assert!(h.iter().any(|s| s.contains("adsbygoogle")), "{:?}", &h[..h.len().min(20)]);
    std::println!("generic always-on selectors: {}", e.generic_always.len());
}
