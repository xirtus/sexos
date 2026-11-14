// SPDX-License-Identifier: GPL-3.0-only
//! Needle: driverless printing and scanning. No drivers, no CUPS, no SANE: printers
//! that speak IPP Everywhere / AirPrint and scanners that speak eSCL (AirScan).
//!
//! ```text
//! run +net needle discover [HOST[:PORT]]        DNS-SD browse (unicast to HOST, else mDNS multicast)
//! run +net needle printers [URI...]             what each printer can do (saved list if none given)
//! run +net needle print URI FILE|--test-page [--format MIME] [--copies N] [--sides KW] [--color] [--media NAME]
//! run +net needle jobs URI                      the printer's jobs
//! run +net needle cancel URI JOB-ID
//! run +net needle scan ESCL-URL OUT.png|.jpg|.pdf|.pgm [--color] [--dpi N]
//! ```
//! Lines starting `[needle.` go to the serial record too, for gates.
use sex_print::caps::{self, PrinterCaps};
use sex_print::dnssd::{self, Packet, Service};
use sex_print::escl::{self, Capabilities, ScanSettings};
use sex_print::ipp::{self, Client, JobOptions, Message};
use sex_print::raster::{self, Color, Page, PwgOptions};
use sex_print::{http, pdf, url::Url};
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::time::{Duration, Instant};

#[cfg(target_os = "hermit")]
use sex_std as _;

const SAVED: &str = "printers";

fn rec(line: &str) {
    println!("{line}");
    #[cfg(target_os = "hermit")]
    sex_std::record(line);
}
fn fail(what: &str) -> ! {
    rec(&format!("[needle.failed] {what}"));
    std::process::exit(1)
}
fn usage() -> ! {
    println!("usage: needle discover [HOST[:PORT]] | printers [URI...] | print URI FILE|--test-page [--format MIME] [--copies N] [--sides KW] [--color] [--media NAME]");
    println!("       needle jobs URI | cancel URI JOB-ID | scan ESCL-URL OUT.png|.jpg|.pdf|.pgm [--color] [--dpi N]");
    std::process::exit(2)
}

// --- transport -----------------------------------------------------------------

fn fetch(url: &Url, method: &str, content_type: Option<&str>, body: &[u8]) -> Result<http::Response, String> {
    if url.tls() { return Err(format!("{}:// needs TLS, which Needle doesn't speak yet; use the plain ipp:// or http:// address", url.scheme)); }
    let mut s = TcpStream::connect((url.host.as_str(), url.port)).map_err(|e| format!("can't reach {}: {e}", url.host_header()))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    s.write_all(&http::request(method, url, content_type, body)).map_err(|e| format!("send: {e}"))?;
    let mut data = Vec::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let n = match s.read(&mut buf) {
            Ok(n) => n,
            Err(e) => return Err(format!("read: {e}")),
        };
        if n > 0 { data.extend_from_slice(&buf[..n]); }
        match http::parse_response(&data, n == 0) {
            Ok(Some(r)) => return Ok(r),
            Ok(None) if n == 0 => return Err("connection closed early".into()),
            Ok(None) => {}
            Err(e) => return Err(format!("bad HTTP answer: {e:?}")),
        }
    }
}

fn ipp_call(url: &Url, m: &Message) -> Result<Message, String> {
    let r = fetch(url, "POST", Some("application/ipp"), &m.encode())?;
    if r.status != 200 { return Err(format!("HTTP {}", r.status)); }
    Message::decode(&r.body).map_err(|e| format!("bad IPP answer: {e:?}"))
}

fn printer_url(s: &str) -> Url { Url::parse(s).unwrap_or_else(|| fail(&format!("not a printer address: {s}"))) }

fn get_caps(url: &Url, client: &mut Client) -> PrinterCaps {
    let m = ipp_call(url, &client.get_printer_attributes(caps::REQUESTED)).unwrap_or_else(|e| fail(&format!("printer {url}: {e}")));
    if !m.is_success() { fail(&format!("printer {url}: {}", ipp::status::name(m.code))); }
    PrinterCaps::from_response(&m)
}

// --- discover --------------------------------------------------------------------

fn discover(target: Option<&str>) {
    let sock = UdpSocket::bind("0.0.0.0:0").unwrap_or_else(|e| fail(&format!("udp: {e}")));
    let _ = sock.set_read_timeout(Some(Duration::from_millis(700)));
    let dest = match target {
        Some(t) if t.contains(':') => t.to_string(),
        Some(t) => format!("{t}:{}", dnssd::MDNS_PORT),
        None => format!("{}.{}.{}.{}:{}", 224, 0, 0, 251, dnssd::MDNS_PORT),
    };
    let mut found: Vec<Service> = Vec::new();
    let mut buf = vec![0u8; 9000];
    let mut sent_ok = false;
    // A few sends: the first datagram may be lost while the route is learned.
    for round in 0..4u16 {
        if sock.send_to(&dnssd::browse_query(0x4e64 + round, true), &*dest).is_ok() { sent_ok = true; }
        let until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < until {
            let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
            let Ok(p) = Packet::parse(&buf[..n]) else { continue };
            if !p.is_response() { continue; }
            for s in dnssd::services(&p) {
                if !found.iter().any(|f| f.instance.eq_ignore_ascii_case(&s.instance)) { found.push(s); }
            }
        }
        if !found.is_empty() { break; }
    }
    if !sent_ok {
        fail(if target.is_none() { "mDNS multicast is not available here; give the host to ask: needle discover HOST" } else { "could not send the query" });
    }
    // Instances whose SRV/TXT weren't in the answer: ask for them.
    for s in found.iter_mut().filter(|s| s.port == 0) {
        let q = dnssd::query(1, &[(s.instance.as_str(), dnssd::rtype::SRV), (s.instance.as_str(), dnssd::rtype::TXT)], true);
        for _ in 0..3 {
            let _ = sock.send_to(&q, &*dest);
            if let Ok((n, _)) = sock.recv_from(&mut buf) {
                if let Ok(p) = Packet::parse(&buf[..n]) { dnssd::fill(s, &p); }
            }
            if s.port != 0 { break; }
        }
    }
    let mut saved = String::new();
    for s in &found {
        // Without an address record the host name is used; .local names need mDNS, so
        // fall back to the host that answered.
        let mut s = s.clone();
        if s.v4.is_empty() && s.v6.is_empty() {
            if let Some(h) = target { s.host = h.split(':').next().unwrap_or(h).to_string(); }
        }
        let kind = if s.is_printer() { "printer" } else { "scanner" };
        let mut line = format!("[needle.{kind}] name={} uri={}", s.name().replace(' ', "_"), s.uri());
        if s.is_printer() {
            line.push_str(&format!(" formats={} color={} duplex={}", s.txt("pdl").unwrap_or("?"), s.txt("Color").unwrap_or("?"), s.txt("Duplex").unwrap_or("?")));
            if let Some(u) = s.txt("URF") { line.push_str(&format!(" urf={u}")); }
            if !s.uri().starts_with("ipps") { saved.push_str(&format!("{}\n", s.uri())); }
        }
        if let Some(u) = s.txt("UUID").or(s.txt("uuid")) { line.push_str(&format!(" uuid={u}")); }
        rec(&line);
    }
    if !saved.is_empty() { let _ = std::fs::write(SAVED, &saved); }
    rec(&format!("[needle.discover] found={}", found.len()));
}

// --- printers ---------------------------------------------------------------------

fn printers(uris: &[String]) {
    let list: Vec<String> = if uris.is_empty() {
        std::fs::read_to_string(SAVED).unwrap_or_default().lines().map(String::from).filter(|l| !l.is_empty()).collect()
    } else {
        uris.to_vec()
    };
    if list.is_empty() { fail("no printers known: run `needle discover HOST` or give a printer address"); }
    for u in &list {
        let url = printer_url(u);
        let c = get_caps(&url, &mut Client::new(u, "needle"));
        rec(&format!(
            "[needle.printer] name={} uri={u} state={} formats={} media={} sides={} color={} raster={}dpi",
            c.name.replace(' ', "_"), c.state_name(), c.formats.join(","), c.media_choice(), c.sides.join(","),
            c.color_modes.join(","), c.raster_dpi()
        ));
        if !c.make_and_model.is_empty() { println!("  {} ({})", c.make_and_model, c.info); }
    }
}

// --- print ------------------------------------------------------------------------

/// Text on raster pages: Geist Mono, 10 pt type on 12 pt lines, half-inch margins.
fn text_pages(text: &str, media: &str, dpi: u32) -> Vec<Page> {
    let font = sex_font::Font::new(sex_font::GEIST_MONO).unwrap_or_else(|| fail("font"));
    let blank = Page::for_media(media, dpi, Color::Gray8).unwrap_or_else(|| Page::for_media("iso_a4_210x297mm", dpi, Color::Gray8).unwrap());
    let (cw, ch) = ((dpi / 12).max(4) as usize, (dpi / 6).max(6) as usize); // 6 pt (0.6 em of 10 pt) by 12 pt
    let margin = (dpi / 2) as usize;
    let cols = ((blank.width as usize).saturating_sub(2 * margin) / cw).max(1);
    let rows = ((blank.height as usize).saturating_sub(2 * margin) / ch).max(1);
    let mut lines: Vec<Vec<char>> = Vec::new();
    for raw in text.split('\n') {
        let l: Vec<char> = raw.trim_end_matches('\r').replace('\t', "    ").chars().collect();
        if l.is_empty() { lines.push(Vec::new()); }
        for chunk in l.chunks(cols) { lines.push(chunk.to_vec()); }
    }
    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) { lines.pop(); }
    let mut acc = vec![0i32; sex_font::raster::scratch_len(cw, ch)];
    let mut cov = vec![0u8; cw * ch];
    let mut pages = Vec::new();
    for chunk in lines.chunks(rows) {
        let mut p = blank.clone();
        for (r, line) in chunk.iter().enumerate() {
            for (c, &chr) in line.iter().enumerate() {
                if chr == ' ' { continue; }
                cov.fill(0);
                if !sex_font::draw_cell(&font, chr as u32, cw, ch, &mut acc, &mut cov) {
                    sex_font::draw_cell(&font, '?' as u32, cw, ch, &mut acc, &mut cov);
                }
                let (x0, y0) = (margin + c * cw, margin + r * ch);
                for y in 0..ch {
                    let row = (y0 + y) * p.width as usize;
                    for x in 0..cw {
                        let v = cov[y * cw + x];
                        if v > 0 { let px = &mut p.pixels[row + x0 + x]; *px = (*px).min(255 - v); }
                    }
                }
            }
        }
        pages.push(p);
    }
    if pages.is_empty() { pages.push(blank); }
    pages
}

/// A picture fitted (aspect kept, centred) inside half-inch margins.
fn image_page(width: usize, height: usize, argb: &dyn Fn(usize, usize) -> u32, media: &str, dpi: u32, color: Color) -> Page {
    let mut p = Page::for_media(media, dpi, color).unwrap_or_else(|| Page::for_media("iso_a4_210x297mm", dpi, color).unwrap());
    let margin = (dpi / 2) as usize;
    let (aw, ah) = ((p.width as usize).saturating_sub(2 * margin).max(1), (p.height as usize).saturating_sub(2 * margin).max(1));
    // Scale = min(aw/width, ah/height), as a fraction num/den.
    let (num, den) = if aw * height <= ah * width { (aw, width) } else { (ah, height) };
    let (w, h) = ((width * num / den).max(1), (height * num / den).max(1));
    let (x0, y0) = (margin + (aw - w.min(aw)) / 2, margin + (ah - h.min(ah)) / 2);
    let bpp = color.bytes_per_pixel();
    for y in 0..h.min(ah) {
        for x in 0..w.min(aw) {
            let v = argb(x * den / num, y * den / num);
            let (r, g, b) = ((v >> 16) as u8, (v >> 8) as u8, v as u8);
            let at = ((y0 + y) * p.width as usize + x0 + x) * bpp;
            match color {
                Color::Gray8 => p.pixels[at] = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8,
                Color::Rgb8 => p.pixels[at..at + 3].copy_from_slice(&[r, g, b]),
            }
        }
    }
    p
}

/// A binary PGM (P5) or PPM (P6), 8-bit, as width, height, ARGB.
fn read_pnm(b: &[u8]) -> Option<(usize, usize, Vec<u32>)> {
    let kind = b.get(..2)?;
    let mut fields = Vec::new();
    let mut at = 2;
    while fields.len() < 3 {
        while at < b.len() && (b[at] as char).is_ascii_whitespace() { at += 1; }
        if b.get(at) == Some(&b'#') { while at < b.len() && b[at] != b'\n' { at += 1; } continue; }
        let start = at;
        while at < b.len() && b[at].is_ascii_digit() { at += 1; }
        fields.push(std::str::from_utf8(&b[start..at]).ok()?.parse::<usize>().ok()?);
    }
    at += 1;
    let (w, h, max) = (fields[0], fields[1], fields[2]);
    if max != 255 { return None; }
    let px = match kind { b"P5" => 1, b"P6" => 3, _ => return None };
    let data = b.get(at..at + w * h * px)?;
    Some((w, h, data.chunks(px).map(|c| if px == 1 { 0xFF00_0000 | (c[0] as u32) * 0x01_01_01 } else { 0xFF00_0000 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32 }).collect()))
}

const TEST_PAGE: &str = "Needle test page\n\nPrinted from SexOS without a driver: IPP Everywhere, PWG raster.\n\nThe quick brown fox jumps over the lazy dog. 0123456789\n";

fn print(args: &[String]) {
    let (mut uri, mut file, mut format, mut copies, mut sides, mut color, mut media) = (None, None, None, None, None, false, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--format" => format = it.next().cloned(),
            "--copies" => copies = it.next().and_then(|n| n.parse::<i32>().ok()),
            "--sides" => sides = it.next().cloned(),
            "--media" => media = it.next().cloned(),
            "--color" => color = true,
            _ if uri.is_none() => uri = Some(a.clone()),
            _ if file.is_none() => file = Some(a.clone()),
            _ => usage(),
        }
    }
    let (Some(uri), Some(file)) = (uri, file) else { usage() };
    let url = printer_url(&uri);
    let mut client = Client::new(&uri, "needle");
    let c = get_caps(&url, &mut client);
    let media = media.unwrap_or_else(|| c.media_choice().to_string());
    let (name, bytes) = if file == "--test-page" {
        ("Needle test page".to_string(), TEST_PAGE.as_bytes().to_vec())
    } else {
        (file.rsplit('/').next().unwrap_or(&file).to_string(), std::fs::read(&file).unwrap_or_else(|e| fail(&format!("can't read {file}: {e}"))))
    };
    let duplex = sides.as_deref().is_some_and(|s| s.starts_with("two-sided"));
    let raster_color = color && c.raster_color();
    let pcolor = if raster_color { Color::Rgb8 } else { Color::Gray8 };
    let encode_raster = |pages: Vec<Page>, fmt: &str| -> Vec<u8> {
        if fmt == "image/urf" { raster::urf_document(&pages, duplex, 0) } else {
            raster::pwg_document(&pages, &PwgOptions { media: media.clone(), duplex, tumble: sides.as_deref() == Some("two-sided-short-edge"), quality: 0 })
        }
    };
    let dpi = c.raster_dpi();
    let (fmt, doc, pages): (String, Vec<u8>, usize) = if let Some(f) = format {
        (f, bytes, 0)
    } else if pdf::is_pdf(&bytes) {
        let f = ["application/pdf", "application/octet-stream"].into_iter().find(|f| c.supports(f))
            .unwrap_or_else(|| fail("this printer takes no PDF; rendering PDF to raster needs Vellum's renderer (follow-up)"));
        (f.into(), bytes, 0)
    } else if let Some(kind) = sex_image::sniff(&bytes) {
        if kind == sex_image::Format::Jpeg && c.supports("image/jpeg") {
            ("image/jpeg".into(), bytes, 1)
        } else {
            let rf = c.raster_format().unwrap_or_else(|| fail("this printer takes no raster pages for pictures"));
            let img = sex_image::decode(&bytes, &sex_image::Limits::PHOTO).unwrap_or_else(|e| fail(&format!("picture: {e:?}")));
            let page = image_page(img.width, img.height, &|x, y| img.pixels[y * img.width + x], &media, dpi, pcolor);
            (rf.into(), encode_raster(vec![page], rf), 1)
        }
    } else if let Some((w, h, px)) = read_pnm(&bytes) {
        let rf = c.raster_format().unwrap_or_else(|| fail("this printer takes no raster pages for pictures"));
        let page = image_page(w, h, &|x, y| px[y * w + x], &media, dpi, pcolor);
        (rf.into(), encode_raster(vec![page], rf), 1)
    } else {
        let text = String::from_utf8_lossy(&bytes);
        if let Some(rf) = c.raster_format() {
            let pages = text_pages(&text, &media, dpi);
            let n = pages.len();
            (rf.into(), encode_raster(pages, rf), n)
        } else if c.supports("application/pdf") {
            ("application/pdf".into(), pdf::text_to_pdf(&text, &media), 0)
        } else if c.supports("text/plain") {
            ("text/plain".into(), bytes, 0)
        } else {
            ("application/octet-stream".into(), bytes, 0)
        }
    };
    let o = JobOptions {
        job_name: name, format: fmt.clone(), copies, sides,
        color_mode: if color { Some("color".into()) } else if c.color_modes.iter().any(|m| m == "monochrome") { Some("monochrome".into()) } else { None },
        media: Some(media.clone()), resolution: None,
    };
    if c.operations.is_empty() || c.operations.contains(&(ipp::op::VALIDATE_JOB as i32)) {
        match ipp_call(&url, &client.validate_job(&o)) {
            Ok(m) if !m.is_success() => fail(&format!("printer refused the job: {} {}", ipp::status::name(m.code), ipp::status_message(&m).unwrap_or(""))),
            Err(e) => fail(&e),
            Ok(_) => {}
        }
    }
    let size = doc.len();
    let m = ipp_call(&url, &client.print_job(&o, doc)).unwrap_or_else(|e| fail(&e));
    if !m.is_success() { fail(&format!("Print-Job: {} {}", ipp::status::name(m.code), ipp::status_message(&m).unwrap_or(""))); }
    let mut job = ipp::jobs(&m).into_iter().next().unwrap_or_else(|| fail("the printer gave no job id"));
    rec(&format!("[needle.sent] id={} format={fmt} bytes={size} pages={pages} media={media} dpi={dpi}", job.id));
    let until = Instant::now() + Duration::from_secs(120);
    while !job.finished() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(r) = ipp_call(&url, &client.get_job_attributes(job.id)) {
            if let Some(j) = ipp::jobs(&r).into_iter().next() { job = j; }
        }
    }
    rec(&format!("[needle.job] id={} state={} reasons={}", job.id, job.state_name(), job.reasons.join(",")));
    if job.state != 9 && job.finished() { std::process::exit(1); }
}

fn jobs(uri: &str) {
    let url = printer_url(uri);
    let mut client = Client::new(uri, "needle");
    let want = ["job-id", "job-state", "job-name", "job-state-reasons", "job-uri"];
    let mut m = ipp_call(&url, &client.get_jobs("all", &want)).unwrap_or_else(|e| fail(&e));
    if !m.is_success() { m = ipp_call(&url, &client.get_jobs("completed", &want)).unwrap_or_else(|e| fail(&e)); }
    let js = ipp::jobs(&m);
    for j in &js { rec(&format!("[needle.job] id={} state={} name={}", j.id, j.state_name(), j.name.replace(' ', "_"))); }
    rec(&format!("[needle.jobs] count={}", js.len()));
}

fn cancel(uri: &str, id: &str) {
    let url = printer_url(uri);
    let id: i32 = id.parse().unwrap_or_else(|_| usage());
    let m = ipp_call(&url, &Client::new(uri, "needle").cancel_job(id)).unwrap_or_else(|e| fail(&e));
    rec(&format!("[needle.cancel] id={id} status={}", ipp::status::name(m.code)));
    if !m.is_success() { std::process::exit(1); }
}

// --- scan -------------------------------------------------------------------------

fn scan(args: &[String]) {
    let (mut root, mut out, mut color, mut dpi) = (None, None, false, 150u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--color" => color = true,
            "--dpi" => dpi = it.next().and_then(|n| n.parse().ok()).unwrap_or_else(|| usage()),
            _ if root.is_none() => root = Some(a.clone()),
            _ if out.is_none() => out = Some(a.clone()),
            _ => usage(),
        }
    }
    let (Some(root), Some(out)) = (root, out) else { usage() };
    let base = Url::parse(&root).unwrap_or_else(|| fail(&format!("not a scanner address: {root}")));
    let ext = out.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let want = match ext.as_str() { "jpg" | "jpeg" => "image/jpeg", "pdf" => "application/pdf", _ => "image/png" };
    let at = |path: String| Url { path, ..base.clone() };
    let r = fetch(&at(escl::capabilities_path(&base.path)), "GET", None, b"").unwrap_or_else(|e| fail(&e));
    if r.status != 200 { fail(&format!("ScannerCapabilities: HTTP {}", r.status)); }
    let caps = Capabilities::parse(&String::from_utf8_lossy(&r.body)).unwrap_or_else(|_| fail("ScannerCapabilities is not readable XML"));
    let s = ScanSettings::choose(&caps, want, !color, dpi).unwrap_or_else(|| fail("the scanner offers no input"));
    println!("Scanning on {} ({}, {} dpi, {})", caps.make_and_model, s.input_source, s.resolution, s.format);
    let r = fetch(&at(escl::jobs_path(&base.path)), "POST", Some("text/xml"), s.to_xml().as_bytes()).unwrap_or_else(|e| fail(&e));
    if r.status != 201 && r.status != 200 { fail(&format!("ScanJobs: HTTP {}", r.status)); }
    let job = r.header("location").and_then(|l| base.join(l)).unwrap_or_else(|| fail("the scanner gave no job location"));
    let mut pages: Vec<(String, Vec<u8>)> = Vec::new();
    let mut busy = 0;
    loop {
        let r = fetch(&Url { path: escl::next_document_path(&job.path), ..job.clone() }, "GET", None, b"").unwrap_or_else(|e| fail(&e));
        match r.status {
            200 => pages.push((r.header("content-type").unwrap_or(&s.format).to_string(), r.body)),
            404 | 410 => break,
            503 if busy < 60 => { busy += 1; std::thread::sleep(Duration::from_millis(500)); continue; }
            other => fail(&format!("NextDocument: HTTP {other}")),
        }
        if s.input_source == "Platen" || pages.len() >= 500 { break; }
    }
    let _ = fetch(&job, "DELETE", None, b"");
    if pages.is_empty() { fail("the scanner returned no pages"); }
    let mut total = 0;
    for (i, (_ctype, data)) in pages.iter().enumerate() {
        let name = if i == 0 { out.clone() } else { match out.rsplit_once('.') { Some((b, e)) => format!("{b}-{}.{e}", i + 1), None => format!("{out}-{}", i + 1) } };
        let bytes = if ext == "pgm" {
            let img = sex_image::decode(data, &sex_image::Limits::PHOTO).unwrap_or_else(|e| fail(&format!("scanned page: {e:?}")));
            let mut b = format!("P5\n{} {}\n255\n", img.width, img.height).into_bytes();
            b.extend(img.pixels.iter().map(|v| (((v >> 16) & 255) * 77 + ((v >> 8) & 255) * 150 + (v & 255) * 29 >> 8) as u8));
            b
        } else {
            data.clone()
        };
        std::fs::write(&name, &bytes).unwrap_or_else(|e| fail(&format!("can't write {name}: {e}")));
        total += bytes.len();
        println!("wrote {name}");
    }
    rec(&format!("[needle.scan] pages={} bytes={total} format={} out={out}", pages.len(), s.format));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { usage() };
    let rest = &args[1..];
    match cmd.as_str() {
        "discover" => discover(rest.first().map(|s| s.as_str())),
        "printers" => printers(rest),
        "print" => print(rest),
        "jobs" => jobs(rest.first().unwrap_or_else(|| usage())),
        "cancel" if rest.len() == 2 => cancel(&rest[0], &rest[1]),
        "scan" => scan(rest),
        _ => usage(),
    }
    let _ = std::io::stdout().flush();
}
