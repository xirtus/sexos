//! Runs a saved web page the way the Web program loads it, for finding what breaks on
//! real sites: `cargo run -p sex-web --example site -- ROOT URL [out.ppm width height]`.
//! Every address is read from ROOT/HOST/PATH (the query left out; `index.html` for a
//! path ending in `/`); a missing file is a failed load. Scripts run in the program's
//! order (classic, then deferred and module scripts, with their imports), timers run
//! until the page is quiet, and the console, the modules loaded and the layout are printed.
use sex_web::{sex_html, Fonts};
use std::path::{Path, PathBuf};

fn clock() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

fn file_for(root: &Path, url: &str) -> Option<PathBuf> {
    let rest = url.split_once("://")?.1;
    let rest = rest.split(['?', '#']).next()?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut p = root.join(host).join(path);
    if path.is_empty() || path.ends_with('/') { p = p.join("index.html"); }
    Some(p)
}

fn load(root: &Path, url: &str) -> Option<Vec<u8>> { std::fs::read(file_for(root, url)?).ok() }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    let url = args[1].clone();
    let num = |i: usize, d: i32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (w, h) = (num(3, 410), num(4, 330));
    let html = String::from_utf8_lossy(&load(&root, &url).expect("page")).into_owned();
    let dom = sex_html::parse::parse(&html);
    let mut fonts = Fonts::new().unwrap();
    let sheets: Vec<String> = sex_html::stylesheet_links(&dom).iter()
        .filter_map(|l| sex_html::url::resolve(&url, l)).filter_map(|u| load(&root, &u)).map(|b| String::from_utf8_lossy(&b).into_owned()).collect();
    for src in sex_html::image_sources(&dom) {
        if let Some(bytes) = sex_html::url::resolve(&url, src).and_then(|u| load(&root, &u)) { fonts.add_picture(src, &bytes, (w - 16) as usize, 2000); }
    }
    // The program's script order: classic now, then deferred (defer and modules).
    let (mut now, mut later) = (Vec::new(), Vec::new());
    let mut map: Vec<(String, String)> = Vec::new();
    for n in dom.descendants(0) {
        if dom.tag(n) != Some("script") { continue; }
        let ty = dom.attr(n, "type").unwrap_or("").trim().to_ascii_lowercase();
        let module = ty == "module";
        if ty == "importmap" { map.extend(sex_js::document::import_map(&url, &dom.text_content(n))); continue; }
        if !(ty.is_empty() || ty.contains("javascript") || module || ty.contains("ecmascript")) { continue; }
        if !module && dom.attr(n, "nomodule").is_some() { continue; }
        let deferred = module || (dom.attr(n, "defer").is_some() && dom.attr(n, "src").is_some());
        let item = match dom.attr(n, "src") {
            Some(src) => {
                let Some(abs) = sex_html::url::resolve(&url, src) else { continue };
                match load(&root, &abs) { Some(b) => (String::from_utf8_lossy(&b).into_owned(), abs, module), None => { eprintln!("script missing: {abs}"); continue } }
            }
            None => (dom.text_content(n), format!("{}#script{}", url, now.len() + later.len() + 1), module),
        };
        if deferred { later.push(item) } else { now.push(item) }
    }
    now.extend(later);
    let mut doc = sex_js::Document::new(dom, &url, (w, h), clock);
    doc.measure = sex_web::canvas::measure_css;
    doc.pictures = fonts.images.clone();
    doc.import_map = map;
    let mut script = sex_js::Script::new(doc, sex_js::Limits::default()).unwrap();
    let started = std::time::Instant::now();
    for (src, name, module) in &now {
        let t = std::time::Instant::now();
        let r = if *module {
            for _ in 0..64 {
                let need = script.module_needs(src, name).unwrap();
                if need.is_empty() { break; }
                for u in need {
                    let body = load(&root, &u);
                    eprintln!("module {} {u}", if body.is_some() { "ok" } else { "MISSING" });
                    let failed = sex_js::document::failed_module(&u);
                    script.page_mut().modules.insert(u, body.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or(failed));
                }
            }
            script.eval_module(src, name)
        } else { script.eval(src, name) };
        eprintln!("ran {} {:?} in {} ms: {}", if *module { "module" } else { "script" }, r.err(), t.elapsed().as_millis(), name.chars().take(120).collect::<String>());
    }
    script.dispatch("ready").unwrap();
    // Timers, frames and fetches (answered from ROOT) for up to two seconds of page time.
    for _ in 0..200 {
        let asked = std::mem::take(&mut script.page_mut().fetches);
        for ask in &asked {
            let abs = sex_html::url::resolve(&url, &ask.url).unwrap_or(ask.url.clone());
            let (status, body) = match load(&root, &abs) { Some(b) => (200, b), None => (404, Vec::new()) };
            eprintln!("fetch {status} {abs}");
            let n = body.len();
            script.page_mut().keep_body(ask.id, status, body);
            script.dispatch(&format!("fetched\t{}\t{status}\t{n}", ask.id)).unwrap();
        }
        // Modules that import() calls wait for.
        let wanted = script.modules_wanted();
        if !wanted.is_empty() {
            for u in wanted {
                let body = load(&root, &u);
                eprintln!("import() module {} {u}", if body.is_some() { "ok" } else { "MISSING" });
                let failed = sex_js::document::failed_module(&u);
                    script.page_mut().modules.insert(u, body.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or(failed));
            }
            script.modules_ready().unwrap();
        }
        let next = script.dispatch("tick").unwrap();
        if next < 0 && script.page().fetches.is_empty() && script.page().modules_wanted.is_empty() { break; }
        std::thread::sleep(std::time::Duration::from_millis(next.clamp(0, 10) as u64));
    }
    eprintln!("scripts took {} ms", started.elapsed().as_millis());
    for (l, t) in script.page_mut().log.drain(..) { eprintln!("console {l}: {}", t.lines().take(4).collect::<Vec<_>>().join(" | ")); }
    let sheet_refs: Vec<&str> = sheets.iter().map(|s| s.as_str()).collect();
    let styles = sex_html::styles_at(&script.page().dom, &sheet_refs, w, h, 0, &mut sex_html::Timeline::starting_at(0)).0;
    let page = sex_html::layout::layout_in(&script.page().dom, &styles, w, h, &mut fonts);
    eprintln!("{} items, {} links, height {}, {} sheets", page.items.len(), page.links.len(), page.height, sheets.len());
    if let Some(out) = args.get(2) {
        let mut buf = vec![0u32; (w * h) as usize];
        sex_web::paint(&page, &mut fonts, &mut sex_web::Target { pixels: &mut buf, width: w, height: h, scroll: 0, top: 0, premul: false, clip: None }, None);
        let mut bytes = format!("P6 {w} {h} 255\n").into_bytes();
        for p in &buf { bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
        std::fs::write(out, bytes).unwrap();
    }
}
