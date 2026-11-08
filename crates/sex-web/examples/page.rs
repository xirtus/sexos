//! Renders an HTML file to a binary PPM the way the Web program draws it:
//! `cargo run -p sex-web --example page -- in.html out.ppm [width height scroll focus bar-text]`
//! With a bar text the whole window is drawn (bar included), as the program does.
use sex_web::{paint, sex_html, Fonts, Target};

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
            if let Some(img) = sex_web::decode_image(&bytes, (w - 16) as usize, 2000) { fonts.images.push((src.to_string(), img)); }
        }
    }
    // Style sheets and pictures come from files beside the page, as the program fetches them.
    let sheets: Vec<String> = sex_html::stylesheet_links(&dom).iter().filter_map(|l| std::fs::read_to_string(base.join(l)).ok()).collect();
    let sheet_refs: Vec<&str> = sheets.iter().map(|s| s.as_str()).collect();
    let bar_h = if args.len() > 6 { sex_web::BAR } else { 0 };
    let styles = sex_html::styles(&dom, &sheet_refs, w, h - bar_h);
    let page = sex_html::layout::layout(&dom, &styles, w, &fonts);
    let mut buf = vec![0u32; (w * h) as usize];
    let focus = args.get(5).and_then(|v| v.parse::<i64>().ok()).filter(|f| *f >= 0).map(|f| f as usize);
    match args.get(6) {
        Some(bar) => sex_web::frame(&page, &mut fonts, &mut buf, w, h, scroll, focus, bar, false),
        None => paint(&page, &mut fonts, &mut Target { pixels: &mut buf, width: w, height: h, scroll, top: 0 }, focus),
    }
    let mut bytes = format!("P6 {w} {h} 255\n").into_bytes();
    for p in &buf { bytes.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]); }
    std::fs::write(&out, bytes).unwrap();
    eprintln!("{} items, {} links, height {}", page.items.len(), page.links.len(), page.height);
}
