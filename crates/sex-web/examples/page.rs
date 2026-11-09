//! Renders an HTML file to a binary PPM the way the Web program draws it:
//! `cargo run -p sex-web --example page -- in.html out.ppm [width height scroll focus bar-text]`
//! With a bar text the whole window is drawn (bar included), as the program does.
//! Scripts run as in the program; `actions` (after the bar text) is a comma list of
//! `click<link index>` and `key<character>`, each followed by running timers until none are left.
use sex_web::{paint, sex_html, Fonts, Target};

fn clock() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

/// Answers the page's `fetch` requests from files beside it.
fn serve(script: &mut sex_js::Script<sex_js::Document>, base: &std::path::Path) -> bool {
    let asked = std::mem::take(&mut script.page_mut().fetches);
    for ask in &asked {
        let (id, url) = (ask.id, &ask.url);
        let (status, body) = match std::fs::read(base.join(url.trim_start_matches("./"))) { Ok(b) => (200, b), Err(_) => (404, Vec::new()) };
        let n = body.len();
        script.page_mut().keep_body(id, status, body);
        script.dispatch(&format!("fetched\t{id}\t{status}\t{n}")).unwrap();
    }
    !asked.is_empty()
}

/// Runs timers, animation frames and fetches until the page has none left.
fn settle(script: &mut sex_js::Script<sex_js::Document>, canvases: &mut sex_web::canvas::Canvases, fonts: &mut Fonts, base: &std::path::Path) {
    for _ in 0..1000 {
        while serve(script, base) {}
        let queue = std::mem::take(&mut script.page_mut().canvas_queue);
        canvases.run_queue(fonts, &script.page().dom, queue);
        sex_web::canvas::present_gl(fonts, &mut script.page_mut().gl);
        let next = script.dispatch("tick").unwrap();
        if next < 0 && script.page().fetches.is_empty() {
            let queue = std::mem::take(&mut script.page_mut().canvas_queue);
            canvases.run_queue(fonts, &script.page().dom, queue);
            sex_web::canvas::present_gl(fonts, &mut script.page_mut().gl);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(next.max(0) as u64));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let html = std::fs::read_to_string(&args[0]).unwrap();
    let out = args.get(1).cloned().unwrap_or_else(|| "page.ppm".into());
    let num = |i: usize, d: i32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (w, h, scroll) = (num(2, 624), num(3, 356), num(4, 0));
    let mut fonts = Fonts::new().unwrap();
    let base = std::path::Path::new(&args[0]).parent().unwrap().to_path_buf();
    let dom = sex_html::parse::parse(&html);
    for src in sex_html::image_sources(&dom) {
        if let Ok(bytes) = std::fs::read(base.join(src)) {
            fonts.add_picture(src, &bytes, (w - 16) as usize, 2000);
        }
    }
    // Style sheets and pictures come from files beside the page, as the program fetches them.
    let sheets: Vec<String> = sex_html::stylesheet_links(&dom).iter().filter_map(|l| std::fs::read_to_string(base.join(l)).ok()).collect();
    let sheet_refs: Vec<&str> = sheets.iter().map(|s| s.as_str()).collect();
    let bar_h = if args.len() > 6 { sex_web::BAR } else { 0 };
    let scripts: Vec<String> = dom.descendants(0).into_iter().filter(|&n| dom.tag(n) == Some("script"))
        .map(|n| match dom.attr(n, "src") { Some(s) => std::fs::read_to_string(base.join(s)).unwrap_or_default(), None => dom.text_content(n) }).collect();
    let mut doc = sex_js::Document::new(dom, "", (w, h - bar_h), clock);
    doc.measure = sex_web::canvas::measure_css;
    doc.pictures = fonts.images.clone();
    let mut canvases = sex_web::canvas::Canvases::new();
    let lay = |doc: &sex_js::Document, fonts: &mut Fonts| {
        let at_ms = std::env::var("PAGE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let styles = sex_html::styles_at(&doc.dom, &sheet_refs, w, h - bar_h, at_ms).0;
        let page = sex_html::layout::layout_in(&doc.dom, &styles, w, h - bar_h, fonts);
        sex_web::svg::draw_inline(&page, &doc.dom, &styles, fonts);
        page
    };
    let mut page = lay(&doc, &mut fonts);
    doc.set_boxes(&page.boxes); doc.set_layers(&page.layers);
    if !scripts.is_empty() {
        let mut limits = sex_js::Limits::default();
        if let Ok(ms) = std::env::var("SEX_WEB_BUDGET_MS") { limits.budget_ms = ms.parse().unwrap(); }
        let mut script = sex_js::Script::new(doc, limits).unwrap();
        for (i, s) in scripts.iter().enumerate() { let _ = script.eval(s, &format!("script{i}")); }
        script.dispatch("ready").unwrap();
        settle(&mut script, &mut canvases, &mut fonts, &base);
        page = lay(script.page(), &mut fonts);
        script.page_mut().set_boxes(&page.boxes); script.page_mut().set_layers(&page.layers);
        for action in args.get(7).map(|a| a.split(',').filter(|x| !x.is_empty()).collect::<Vec<_>>()).unwrap_or_default() {
            if let Some(i) = action.strip_prefix("click") {
                let node = page.links[i.parse::<usize>().unwrap()].node;
                script.dispatch(&format!("click\t{node}")).unwrap();
            } else if let Some(k) = action.strip_prefix("key") {
                script.dispatch(&format!("key\tkeydown\t{k}\t\t0")).unwrap();
                script.dispatch(&format!("key\tkeyup\t{k}\t\t0")).unwrap();
            }
            settle(&mut script, &mut canvases, &mut fonts, &base);
            page = lay(script.page(), &mut fonts);
            script.page_mut().set_boxes(&page.boxes); script.page_mut().set_layers(&page.layers);
        }
        for (l, t) in script.page_mut().log.drain(..) { eprintln!("console {l}: {t}"); }
    }
    let mut buf = vec![0u32; (w * h) as usize];
    let focus = args.get(5).and_then(|v| v.parse::<i64>().ok()).filter(|f| *f >= 0).map(|f| f as usize);
    match args.get(6) {
        Some(bar) => sex_web::frame(&page, &mut fonts, &mut buf, w, h, scroll, focus, bar, false),
        None => paint(&page, &mut fonts, &mut Target { pixels: &mut buf, width: w, height: h, scroll, top: 0, premul: false, clip: None }, focus),
    }
    let mut bytes = format!("P6 {w} {h} 255\n").into_bytes();
    for p in &buf { bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
    std::fs::write(&out, bytes).unwrap();
    eprintln!("{} items, {} links, height {}", page.items.len(), page.links.len(), page.height);
}
