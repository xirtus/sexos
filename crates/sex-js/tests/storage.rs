//! localStorage and IndexedDB over the browser's lasting storage: what one page keeps,
//! a later page of the same origin (a new engine, as after a reboot) finds again.
use sex_js::{storage::WebStorage, Document, Limits, Script};
use std::{cell::RefCell, rc::Rc};

fn clock() -> u64 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn page(url: &str, store: &Rc<RefCell<WebStorage>>) -> Script<Document> {
    let mut doc = Document::new(sex_html::parse::parse(""), url, (410, 330), clock);
    doc.storage = Some(store.clone());
    Script::new(doc, Limits::default()).expect("engine")
}

fn run(s: &mut Script<Document>, src: &str, until: &str) -> Vec<String> {
    s.eval(src, "t.js").unwrap();
    let mut out = Vec::new();
    for _ in 0..500 {
        out.extend(s.page_mut().log.drain(..).map(|(l, t)| format!("{l}:{t}")));
        if out.iter().any(|l| l.contains(until)) { return out; }
        s.dispatch("tick").unwrap();
    }
    panic!("never logged {until}: {out:?}");
}

#[test]
fn local_storage_lasts_and_stays_in_its_origin() {
    let store = Rc::new(RefCell::new(WebStorage::default()));
    let mut a = page("https://a.example/x", &store);
    run(&mut a, "localStorage.setItem('n', 1); localStorage.k = 'v'; console.log('set', localStorage.length)", "set 2");
    let saved = WebStorage::load(&store.borrow().save());
    let store = Rc::new(RefCell::new(saved));
    let mut again = page("https://a.example/y", &store);
    let l = run(&mut again, "console.log('got', localStorage.getItem('n'), localStorage.k, 'k' in localStorage, Object.keys(localStorage).join())", "got");
    assert_eq!(l, ["0:got 1 v true k,n"]);
    let mut other = page("https://b.example/", &store);
    assert_eq!(run(&mut other, "console.log('other', localStorage.length, localStorage.getItem('n'))", "other"), ["0:other 0 null"]);
}

#[test]
fn indexeddb_stores_indexes_cursors_and_lasts() {
    let store = Rc::new(RefCell::new(WebStorage::default()));
    let mut s = page("https://a.example/app", &store);
    let first = r#"
      const r = indexedDB.open('notes', 2);
      r.onupgradeneeded = e => {
        const db = r.result;
        const os = db.createObjectStore('notes', { keyPath: 'id', autoIncrement: true });
        os.createIndex('by_tag', 'tag');
        os.createIndex('by_title', 'title', { unique: true });
        console.log('upgrade', e.oldVersion, e.newVersion);
      };
      r.onsuccess = () => {
        const db = r.result;
        const tx = db.transaction('notes', 'readwrite');
        const os = tx.objectStore('notes');
        for (let i = 0; i < 10; i++) os.add({ title: 't' + i, tag: i % 2 ? 'odd' : 'even', when: new Date(1000 * i), bytes: new Uint8Array([i]) });
        os.put({ id: 3, title: 'three', tag: 'odd', when: new Date(0), bytes: new Uint8Array([3]) });
        const dup = os.add({ title: 't5', tag: 'x' });
        dup.onerror = e => { console.log('dup', dup.error.name); e.preventDefault(); };
        os.delete(10);
        tx.oncomplete = () => console.log('stored', db.version, Array.from(db.objectStoreNames).join());
      };"#;
    let l = run(&mut s, first, "stored");
    assert!(l.contains(&"0:upgrade 0 2".to_string()), "{l:?}");
    assert!(l.contains(&"0:dup ConstraintError".to_string()), "{l:?}");
    assert!(l.contains(&"0:stored 2 notes".to_string()), "{l:?}");
    // A new engine on the saved storage: as after a reboot.
    let store = Rc::new(RefCell::new(WebStorage::load(&store.borrow().save())));
    let mut s = page("https://a.example/other", &store);
    let second = r#"
      const r = indexedDB.open('notes');
      r.onupgradeneeded = () => console.log('no upgrade expected');
      r.onsuccess = () => {
        const db = r.result;
        const tx = db.transaction(['notes']);
        const os = tx.objectStore('notes');
        const out = [];
        os.count().onsuccess = e => out.push('count ' + e.target.result);
        os.get(3).onsuccess = e => out.push('get ' + e.target.result.title + ' ' + (e.target.result.when instanceof Date) + ' ' + e.target.result.bytes[0]);
        os.index('by_tag').getAll('odd').onsuccess = e => out.push('odd ' + e.target.result.map(n => n.id).join());
        os.index('by_title').getKey('t4').onsuccess = e => out.push('t4 ' + e.target.result);
        os.getAllKeys(IDBKeyRange.bound(2, 5, false, true)).onsuccess = e => out.push('range ' + e.target.result.join());
        const keys = [];
        os.openCursor(null, 'prev').onsuccess = e => { const c = e.target.result; if (c) { keys.push(c.key); c.continue(); } else out.push('cursor ' + keys.join()); };
        tx.oncomplete = () => console.log('read ' + out.join('; '));
      };"#;
    let l = run(&mut s, second, "read");
    assert_eq!(l, ["0:read count 9; get three true 3; odd 2,3,4,6,8; t4 5; range 2,3,4; cursor 9,8,7,6,5,4,3,2,1"]);
    let l = run(&mut s, "indexedDB.databases().then(d => console.log('dbs', JSON.stringify(d)))", "dbs");
    assert_eq!(l, [r#"0:dbs [{"name":"notes","version":2}]"#]);
    let mut other = page("https://b.example/", &store);
    let l = run(&mut other, "indexedDB.databases().then(d => console.log('b dbs', d.length))", "b dbs");
    assert_eq!(l, ["0:b dbs 0"]);
}
