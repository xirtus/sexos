//! Times a saved web page's scripts the way the Web program runs them, repeatably:
//! `cargo run --release -p sex-web --example bench -- ROOT URL [page-seconds]`.
//!
//! Every address is read from a snapshot under ROOT (see `file_for`). With
//! `SEXWEB_FETCH=1` a missing address is downloaded once with curl and kept, so the
//! first run records the page and later runs replay it offline. Scripts run in the
//! program's order: parser scripts, then deferred and module scripts, then scripts
//! that the page inserts (`serve_classic_scripts` in apps/web), fetches and timers.
//! Page time is virtual: instead of sleeping until the next timer, the clock jumps
//! there, so a run does the same work every time and measures only CPU.
//!
//! Prints one `bench` line with the time spent in each phase, in milliseconds.
use sex_web::sex_html;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static SKIPPED_NS: AtomicU64 = AtomicU64::new(0);

/// Page time, independent of how fast this machine or engine is: each reading moves
/// it on by a microsecond (so loops that wait on the clock end), and the main loop
/// jumps it to the next timer. Host durations are measured with `Instant` instead.
fn clock() -> u64 { SKIPPED_NS.fetch_add(1_000, Ordering::Relaxed) + 1_000 }

/// ROOT/HOST/PATH, `index.html` for a path ending in `/`, and the query (when there
/// is one) folded into the file name, since load.php-style addresses differ only there.
fn file_for(root: &Path, url: &str) -> Option<PathBuf> {
    let rest = url.split_once("://")?.1;
    let rest = rest.split('#').next()?;
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut p = root.join(host).join(path);
    if path.is_empty() || path.ends_with('/') { p = p.join("index.html"); }
    if !query.is_empty() {
        // FNV-1a keeps long queries short and stable.
        let h = query.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        let mut name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.push_str(&format!("@{h:016x}"));
        p.set_file_name(name);
    }
    Some(p)
}

fn load(root: &Path, url: &str) -> Option<Vec<u8>> {
    let path = file_for(root, url)?;
    if let Ok(b) = std::fs::read(&path) { return Some(b); }
    let missing = path.with_extension("missing");
    if missing.exists() || std::env::var("SEXWEB_FETCH").ok().as_deref() != Some("1") || !url.starts_with("http") { return None; }
    std::fs::create_dir_all(path.parent()?).ok()?;
    let out = std::process::Command::new("curl")
        .args(["-sSL", "--compressed", "-m", "30", "-A", "Mozilla/5.0 (X11; SexOS x86_64) Kaleidoscope", "-o"]).arg(&path)
        .args(["-w", "%{http_code}", url]).output().ok()?;
    let code = String::from_utf8_lossy(&out.stdout).into_owned();
    eprintln!("downloaded {code} {url}");
    if code.starts_with('2') { std::fs::read(&path).ok() } else { let _ = std::fs::remove_file(&path); let _ = std::fs::write(missing, code); None }
}

fn text(b: Vec<u8>) -> String { String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()) }

#[derive(Default)]
struct Phases { parse_ns: u64, scripts_ns: u64, inserted_ns: u64, fetched_ns: u64, ticks_ns: u64, ticks: u32, inserted: u32, fetches: u32 }

fn ms(ns: u64) -> f64 { ns as f64 / 1e6 }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    let url = args[1].clone();
    let page_seconds: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(30);
    // SEXWEB_BENCH_VISITS=N loads the page N times, as the browser would on revisits,
    // sharing one compiled-script cache (SEXWEB_BENCH_NO_CACHE=1 leaves it out).
    let visits: usize = std::env::var("SEXWEB_BENCH_VISITS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let cache = std::rc::Rc::new(std::cell::RefCell::new(sex_js::CodeCache::new(64 << 20)));
    for _ in 0..visits {
        let cache = std::env::var("SEXWEB_BENCH_NO_CACHE").is_err().then(|| cache.clone());
        visit(&root, &url, page_seconds, cache);
    }
}

fn visit(root: &Path, url: &str, page_seconds: u64, cache: Option<std::rc::Rc<std::cell::RefCell<sex_js::CodeCache>>>) {
    let url = String::from(url);
    let root = root.to_path_buf();
    let verbose = std::env::var("SEXWEB_BENCH_VERBOSE").is_ok();
    let (w, h) = (1280, 800);
    let mut ph = Phases::default();
    let all = Instant::now();
    let t = Instant::now();
    let html = text(load(&root, &url).expect("page"));
    let dom = sex_html::parse::parse(&html);
    ph.parse_ns = t.elapsed().as_nanos() as u64;
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
                match load(&root, &abs) { Some(b) => (text(b), abs, module, n, true), None => { eprintln!("script missing: {abs}"); continue } }
            }
            None => (dom.text_content(n), format!("{}#script{}", url, now.len() + later.len() + 1), module, n, false),
        };
        if deferred { later.push(item) } else { now.push(item) }
    }
    now.extend(later);
    let mut doc = sex_js::Document::new(dom, &url, (w, h), clock);
    doc.measure = sex_web::canvas::measure_css;
    doc.import_map = map;
    doc.code_cache = cache.clone();
    let mut script = sex_js::Script::new(doc, sex_js::Limits::default()).unwrap();
    let serve_modules = |script: &mut sex_js::Script<sex_js::Document>, list: Vec<String>| {
        for u in list {
            let body = load(&root, &u);
            if body.is_none() { eprintln!("module missing: {u}"); }
            let failed = sex_js::document::failed_module(&u);
            script.page_mut().modules.insert(u, body.map(text).unwrap_or(failed));
        }
    };
    let t = Instant::now();
    for (src, name, module, node, external) in &now {
        let ts = Instant::now();
        let r = if *module {
            for _ in 0..64 {
                let need = script.module_needs(src, name).unwrap();
                if need.is_empty() { break; }
                serve_modules(&mut script, need);
            }
            if verbose { eprintln!("module graph found in {:.1} ms", ms(ts.elapsed().as_nanos() as u64)); }
            script.eval_module(src, name)
        } else {
            // As apps/web run_scripts: currentScript while it runs, then its load event.
            let _ = script.dispatch(&format!("script\t{node}"));
            let r = script.eval(src, name);
            let _ = script.dispatch("script\t");
            if *external && !matches!(&r, Err(sex_js::Error::Trapped(_))) { let _ = script.dispatch(&format!("scriptload\t{node}")); }
            r
        };
        if verbose { eprintln!("ran {:?} in {:.1} ms: {}", r.err(), ms(ts.elapsed().as_nanos() as u64), name.chars().take(120).collect::<String>()); }
    }
    script.dispatch("ready").unwrap();
    ph.scripts_ns = t.elapsed().as_nanos() as u64;
    let end = clock() + page_seconds * 1_000_000_000;
    for _ in 0..20_000 {
        // Inserted classic scripts (apps/web serve_classic_scripts).
        let asked = std::mem::take(&mut script.page_mut().classic_scripts);
        for ask in asked {
            if !script.page().classic_script_current(&ask) { continue; }
            let Some(abs) = sex_html::url::resolve(&url, &ask.url) else { continue };
            let body = load(&root, &abs);
            let t = Instant::now();
            let ok = body.is_some();
            if let Some(b) = body {
                let _ = script.dispatch(&format!("script\t{}", ask.node));
                let r = script.eval_resource(&text(b), &abs, false);
                let _ = script.dispatch("script\t");
                if verbose { eprintln!("inserted {:?} in {:.1} ms: {}", r.err(), ms(t.elapsed().as_nanos() as u64), abs.chars().take(160).collect::<String>()); }
            } else { eprintln!("inserted script missing: {abs}"); }
            let _ = script.dispatch(&format!("scriptend\t{}\t{}", ask.node, ok as u8));
            ph.inserted_ns += t.elapsed().as_nanos() as u64;
            ph.inserted += 1;
        }
        let asked = std::mem::take(&mut script.page_mut().fetches);
        for ask in &asked {
            let abs = sex_html::url::resolve(&url, &ask.url).unwrap_or(ask.url.clone());
            let get = ask.method.is_empty() || ask.method == "GET";
            let (status, body) = match get.then(|| load(&root, &abs)).flatten() { Some(b) => (200, b), None => (404, Vec::new()) };
            if verbose || status != 200 { eprintln!("fetch {status} {} {abs}", ask.method); }
            let n = body.len();
            let t = Instant::now();
            script.page_mut().keep_body(ask.id, status, body);
            script.dispatch(&format!("fetched\t{}\t{status}\t{n}", ask.id)).unwrap();
            ph.fetched_ns += t.elapsed().as_nanos() as u64;
            ph.fetches += 1;
        }
        let wanted = script.modules_wanted();
        if !wanted.is_empty() {
            serve_modules(&mut script, wanted);
            let t = Instant::now();
            script.modules_ready().unwrap();
            ph.fetched_ns += t.elapsed().as_nanos() as u64;
        }
        let t = Instant::now();
        let next = script.dispatch("tick").unwrap();
        if verbose { eprintln!("tick {:.1} ms next={next}", ms(t.elapsed().as_nanos() as u64)); }
        ph.ticks_ns += t.elapsed().as_nanos() as u64;
        ph.ticks += 1;
        let pending = !script.page().fetches.is_empty() || !script.page().modules_wanted.is_empty() || !script.page().classic_scripts.is_empty();
        if pending { continue; }
        if next < 0 || clock() >= end { break; }
        SKIPPED_NS.fetch_add(next as u64 * 1_000_000, Ordering::Relaxed);
    }
    let total = all.elapsed().as_nanos() as u64;
    let errors = script.page().log.iter().filter(|(l, _)| *l >= 2).count();
    if verbose { for (l, t) in script.page_mut().log.drain(..) { eprintln!("console {l}: {}", t.lines().take(3).collect::<Vec<_>>().join(" | ")); } }
    // `--features profile`: host operations and the costliest selectors.
    #[cfg(feature = "profile")]
    {
        let p = script.execution_profile();
        let mut ops: Vec<usize> = (0..128).filter(|&i| p.calls[i] > 0).collect();
        ops.sort_by_key(|&i| std::cmp::Reverse(p.host_ns[i]));
        for i in ops.into_iter().take(12) { eprintln!("op {i:3} calls={:7} {:8.1} ms", p.calls[i], ms(p.host_ns[i])); }
        let mut q = script.page().query_profile.clone();
        q.sort_by_key(|e| std::cmp::Reverse(e.2));
        for (s, calls, ns) in q.into_iter().take(16) { eprintln!("query calls={calls:6} {:8.1} ms {}", ms(ns), s.chars().take(100).collect::<String>()); }
    }
    let d = &script.page().dom;
    let elements = d.descendants(0).into_iter().filter(|&n| d.tag(n).is_some()).count();
    println!(
        "bench{} total={:.1} parse={:.1} scripts={:.1} inserted={:.1}({}) fetched={:.1}({}) ticks={:.1}({}) elements={} console_errors={}",
        cache.as_ref().map(|c| { let c = c.borrow(); format!(" code_cache={}/{}({}KiB)", c.hits, c.hits + c.misses, c.bytes() >> 10) }).unwrap_or_default(),
        ms(total), ms(ph.parse_ns), ms(ph.scripts_ns), ms(ph.inserted_ns), ph.inserted, ms(ph.fetched_ns), ph.fetches, ms(ph.ticks_ns), ph.ticks, elements, errors
    );
}
