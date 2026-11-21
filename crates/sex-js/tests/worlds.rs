//! Isolated worlds: a second script context over the same document, as an extension's
//! content scripts get in Chrome. Globals are apart; nodes, events and mutation records
//! are shared.
use sex_js::{Document, Limits, Script};

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(html: &str) -> Script<Document> {
    let doc = Document::new(sex_html::parse::parse(html), "https://a.example/", (410, 330), clock);
    Script::new(doc, Limits::default()).expect("engine")
}

fn logs(s: &mut Script<Document>) -> Vec<String> { s.page_mut().log.drain(..).map(|(_, t)| t).collect() }

fn in_world(s: &mut Script<Document>, w: u8, src: &str) -> bool {
    s.in_world(w, |_| {}, |s, made| { s.eval(src, "content.js").unwrap(); made }).unwrap()
}

#[test]
fn worlds_have_their_own_globals_over_one_document() {
    let mut s = page("<body><p id=a>hi</p></body>");
    s.eval("window.secret = 42; Array.prototype.evil = 1; document.getElementById('a').mark = 'page';", "page.js").unwrap();
    assert!(in_world(&mut s, 1, "console.log(typeof secret, [].evil, document.getElementById('a').mark, document.getElementById('a').textContent); document.body.setAttribute('data-ext', 'on'); window.mine = 1;"));
    assert_eq!(logs(&mut s), ["undefined undefined undefined hi"]);
    // The second time, the world is already there and keeps its globals.
    assert!(!in_world(&mut s, 1, "console.log(mine)"));
    s.eval("console.log(document.body.getAttribute('data-ext'), typeof mine)", "page.js").unwrap();
    assert_eq!(logs(&mut s), ["1", "on undefined"]);
}

#[test]
fn events_reach_every_world_and_any_may_cancel() {
    let mut s = page("<body><button id=b>go</button></body>");
    s.eval("document.getElementById('b').addEventListener('click', () => console.log('page heard'));", "page.js").unwrap();
    in_world(&mut s, 2, "document.getElementById('b').addEventListener('click', e => { console.log('extension heard'); e.preventDefault(); });");
    // The button's node id, as the browser names it.
    s.eval("console.log(String(document.getElementById('b')._id))", "page.js").unwrap();
    let node = logs(&mut s).pop().unwrap();
    let r = s.dispatch(&format!("click\t{node}")).unwrap();
    assert_eq!(logs(&mut s), ["page heard", "extension heard"]);
    assert_eq!(r, 1, "the extension's world cancelled the click");
}

#[test]
fn mutation_observers_hear_changes_made_in_other_worlds() {
    let mut s = page("<body><ul id=l></ul></body>");
    in_world(&mut s, 1, "new MutationObserver(rs => { for (const r of rs) console.log('ext saw', r.type, r.addedNodes.length ? r.addedNodes[0].localName : '-', r.attributeName); }).observe(document.body, { childList: true, subtree: true, attributes: true });");
    s.eval("new MutationObserver(rs => console.log('page saw', rs.length, rs[0].target.id)).observe(document.getElementById('l'), { attributes: true });", "page.js").unwrap();
    s.eval("document.getElementById('l').appendChild(document.createElement('li'));", "page.js").unwrap();
    s.dispatch("tick").unwrap();
    assert_eq!(logs(&mut s), ["ext saw childList li null"]);
    in_world(&mut s, 1, "document.getElementById('l').setAttribute('data-x', '1');");
    s.dispatch("tick").unwrap();
    let mut l = logs(&mut s);
    l.sort();
    assert_eq!(l, ["ext saw attributes - data-x", "page saw 1 l"]);
}

#[test]
fn timers_run_in_their_own_world() {
    let mut s = page("<body></body>");
    in_world(&mut s, 3, "setTimeout(() => console.log('ext timer', typeof secret), 0);");
    s.eval("window.secret = 1; setTimeout(() => console.log('page timer'), 0);", "page.js").unwrap();
    for _ in 0..20 {
        s.dispatch("tick").unwrap();
        if s.page().log.len() >= 2 { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut l = logs(&mut s);
    l.sort();
    assert_eq!(l, ["ext timer undefined", "page timer"]);
}
