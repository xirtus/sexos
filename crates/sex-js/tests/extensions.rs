//! `chrome.*` for extensions (extensions.js): two realms, a page with a content script
//! and an extension's background, with the browser's routing played here.
use sex_js::{storage::WebStorage, Document, Limits, Script};
use std::{cell::RefCell, rc::Rc};

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

const KEY: &str = "k3y";

fn realm(url: &str, store: &Rc<RefCell<WebStorage>>, key: &str) -> Script<Document> {
    let mut doc = Document::new(sex_html::parse::parse(""), url, (410, 330), clock);
    doc.storage = Some(store.clone());
    doc.ext_key = String::from(key);
    doc.ext_tab = String::from(r#"{"id":1,"url":"https://a.example/","active":true}"#);
    Script::new(doc, Limits::default()).expect("engine")
}

fn api(where_: &str) -> String {
    format!("const chrome = __kaleidoscopeExt('demo', '{KEY}', {{name:'Demo', version:'1', manifest_version:3}}, {{hello:{{message:'Hi $1'}}}}, '{where_}');")
}

/// Delivers what each realm posted until both are quiet, the way the browser does.
fn route(page: &mut Script<Document>, bg: &mut Script<Document>) {
    for _ in 0..50 {
        let from_page: Vec<String> = page.page_mut().ext_outbox.drain(..).collect();
        let from_bg: Vec<String> = bg.page_mut().ext_outbox.drain(..).collect();
        if from_page.is_empty() && from_bg.is_empty() {
            page.dispatch("tick").unwrap();
            bg.dispatch("tick").unwrap();
            if page.page().ext_outbox.is_empty() && bg.page().ext_outbox.is_empty() { return; }
            continue;
        }
        for (text, from_bg) in from_page.into_iter().map(|t| (t, false)).chain(from_bg.into_iter().map(|t| (t, true))) {
            let m: serde_json::Value = serde_json::from_str(&text).unwrap();
            let to_page = |v: serde_json::Value, page: &mut Script<Document>| { page.dispatch(&format!("ext\t{v}")).unwrap(); };
            match m["t"].as_str().unwrap() {
                "send" => {
                    let e = serde_json::json!({"k":"msg","id":m["id"],"n":m["n"],"m":m["m"],"to":"page","sender":{"id":"demo","url":"https://a.example/"}});
                    bg.dispatch(&format!("ext\t{e}")).unwrap();
                }
                "reply" => {
                    let e = serde_json::json!({"k":"reply","n":m["n"],"r":m["r"]});
                    if m["to"] == "page" { to_page(e, page) } else { bg.dispatch(&format!("ext\t{e}")).unwrap(); }
                }
                "changed" => {
                    let e = serde_json::json!({"k":"changed","id":m["id"],"area":m["area"],"changes":m["changes"]});
                    if from_bg { to_page(e, page) } else { bg.dispatch(&format!("ext\t{e}")).unwrap(); }
                }
                other => panic!("unexpected {other}"),
            }
        }
    }
}

fn logs(s: &mut Script<Document>) -> Vec<String> { s.page_mut().log.drain(..).map(|(_, t)| t).collect() }

#[test]
fn messages_storage_and_events_cross_realms() {
    let store = Rc::new(RefCell::new(WebStorage::default()));
    let mut page = realm("https://a.example/", &store, KEY);
    let mut bg = realm("kaleidoscope-extension://demo/_background", &store, KEY);
    bg.eval(&format!("{}
        chrome.runtime.onMessage.addListener((m, sender, reply) => {{
          if (m.kind === 'add') {{ setTimeout(() => reply({{sum: m.a + m.b, from: sender.url}}), 1); return true; }}
          if (m.kind === 'save') {{ chrome.storage.local.set({{saved: m.value}}).then(() => reply('ok')); return true; }}
        }});
        chrome.storage.onChanged.addListener((c, area) => console.log('bg saw', area, JSON.stringify(c)));", api("bg")), "bg.js").unwrap();
    page.eval(&format!("(function(){{{}
        chrome.storage.onChanged.addListener((c, area) => console.log('page saw', area, c.saved.newValue));
        chrome.runtime.sendMessage({{kind:'add', a:2, b:3}}).then(r => console.log('sum', r.sum, r.from));
        chrome.runtime.sendMessage({{kind:'save', value:[1,2]}}, r => console.log('save', r));
        chrome.storage.local.set({{mine: true}}, () => console.log('set done', chrome.runtime.lastError === undefined));
        console.log('i18n', chrome.i18n.getMessage('hello', 'there'), chrome.runtime.getURL('/a.js'));
        chrome.tabs.query({{active:true}}).then(t => console.log('tab', t[0].url));
    }})()", api("content")), "content.js").unwrap();
    route(&mut page, &mut bg);
    let p = logs(&mut page);
    let b = logs(&mut bg);
    assert!(p.contains(&String::from("sum 5 https://a.example/")), "{p:?}");
    assert!(p.contains(&String::from("save ok")), "{p:?}");
    assert!(p.contains(&String::from("set done true")), "{p:?}");
    assert!(p.contains(&String::from("i18n Hi there kaleidoscope-extension://demo/a.js")), "{p:?}");
    assert!(p.contains(&String::from("tab https://a.example/")), "{p:?}");
    assert!(p.contains(&String::from("page saw local [1,2]")), "{p:?}");
    assert!(b.iter().any(|l| l.starts_with("bg saw local") && l.contains("\"mine\"")), "{b:?}");
    // Both realms read the same storage.
    bg.eval("chrome.storage.local.get(null).then(all => console.log('all', JSON.stringify(all)))", "get.js").unwrap();
    route(&mut page, &mut bg);
    assert_eq!(logs(&mut bg), ["all {\"mine\":true,\"saved\":[1,2]}"]);
}

#[test]
fn only_the_browser_key_opens_the_api() {
    let store = Rc::new(RefCell::new(WebStorage::default()));
    let mut page = realm("https://a.example/", &store, KEY);
    page.eval("console.log('guess', typeof __kaleidoscopeExt('demo', 'nope'), typeof __kaleidoscopeExt('demo', ''))", "page.js").unwrap();
    assert_eq!(logs(&mut page), ["guess undefined undefined"]);
    // A page without extensions has no key at all.
    let mut plain = realm("https://a.example/", &store, "");
    plain.eval("console.log('plain', typeof __kaleidoscopeExt === 'function' ? typeof __kaleidoscopeExt('demo', '') : 'none')", "p.js").unwrap();
    assert_eq!(logs(&mut plain), ["plain none"]);
    // Nothing stored under the extension's origin is a page's localStorage.
    let mut ext = realm("https://a.example/", &store, KEY);
    ext.eval(&format!("{} chrome.storage.local.set({{x:1}}); console.log('ls', localStorage.length)", api("content")), "x.js").unwrap();
    assert_eq!(logs(&mut ext), ["ls 0"]);
}
