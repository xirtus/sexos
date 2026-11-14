// SPDX-License-Identifier: GPL-3.0-only
//! Byte-exact goldens. The .bin fixtures come from an independent Python encoder
//! (tests/fixtures/make_fixtures.py over scripts/needle_fakes.py).
use sex_print::caps::{PrinterCaps, REQUESTED};
use sex_print::dnssd::{self, Packet};
use sex_print::escl::{Capabilities, ScanSettings};
use sex_print::ipp::{self, Client, JobOptions, Message};
use sex_print::raster::{self, Color, Page, PwgOptions};
use sex_print::{pdf, url::Url};

const URI: &str = "ipp://10.0.2.2:8631/ipp/print";
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn ipp_requests_encode() {
    let mut c = Client::new(URI, "needle");
    assert_eq!(c.get_printer_attributes(&["printer-name", "document-format-supported"]).encode(), fixture("ipp_gpa_request.bin"));

    let mut c = Client::new(URI, "needle");
    let o = JobOptions {
        job_name: "hello.txt".into(), format: "image/pwg-raster".into(), copies: Some(2),
        sides: Some("two-sided-long-edge".into()), color_mode: Some("monochrome".into()),
        media: Some("iso_a4_210x297mm".into()), resolution: Some((300, 300)),
    };
    let pj = c.print_job(&o, b"RaS2".to_vec()).encode();
    assert_eq!(pj, fixture("ipp_print_job_request.bin"));
    // Decoding our own request gives it back, document bytes included.
    let back = Message::decode(&pj).unwrap();
    assert_eq!(back.data, b"RaS2");
    assert_eq!(back.encode(), pj);

    let mut c = Client::new(URI, "needle");
    let mut cancel = c.cancel_job(42);
    cancel.version = (1, 1);
    assert_eq!(cancel.encode(), fixture("ipp_cancel_job_request.bin"));
}

#[test]
fn ipp_printer_attributes_decode() {
    let bytes = fixture("ipp_gpa_response.bin");
    let m = Message::decode(&bytes).unwrap();
    assert_eq!((m.version, m.code, m.request_id), ((2, 0), ipp::status::OK, 7));
    // Re-encoding a decoded response is byte-exact too.
    assert_eq!(m.encode(), bytes);
    let c = PrinterCaps::from_response(&m);
    assert_eq!(c.name, "needle-test");
    assert_eq!(c.make_and_model, "SexOS Fake IPP Everywhere");
    assert_eq!(c.state_name(), "idle");
    assert_eq!(c.formats, ["application/pdf", "image/pwg-raster", "image/urf", "application/octet-stream"]);
    assert_eq!(c.media, ["iso_a4_210x297mm", "na_letter_8.5x11in"]);
    assert_eq!(c.media_choice(), "iso_a4_210x297mm");
    assert!(c.duplex() && c.color);
    assert_eq!(c.color_modes, ["monochrome", "color"]);
    assert_eq!(c.resolutions, [(300, 300), (600, 600)]);
    assert_eq!(c.raster_resolutions, [(75, 75), (300, 300)]);
    assert_eq!(c.raster_types, ["sgray_8", "srgb_8"]);
    assert_eq!(c.urf, ["V1.4", "CP1", "W8", "SRGB24", "RS300"]);
    assert_eq!(c.copies_max, 99);
    assert_eq!(c.operations, [2, 4, 8, 9, 10, 11]);
    assert_eq!(c.raster_format(), Some("image/pwg-raster"));
    assert_eq!(c.raster_dpi(), 75);
    assert!(c.raster_color());
    assert!(REQUESTED.contains(&"pwg-raster-document-type-supported"));
}

#[test]
fn ipp_get_jobs_decode() {
    let m = Message::decode(&fixture("ipp_get_jobs_response.bin")).unwrap();
    let jobs = ipp::jobs(&m);
    assert_eq!(jobs.len(), 2);
    assert_eq!((jobs[0].id, jobs[0].state_name(), jobs[0].name.as_str()), (1, "completed", "one"));
    assert_eq!((jobs[1].id, jobs[1].state_name(), jobs[1].finished()), (2, "processing", false));
    assert_eq!(jobs[0].uri, format!("{URI}/1"));
    assert_eq!(jobs[0].reasons, ["job-completed-successfully"]);
}

#[test]
fn pwg_raster_golden() {
    // 5x3 gray: two equal lines, then a ramp.
    let mut p = Page::blank(5, 3, 75, Color::Gray8);
    p.pixels.copy_from_slice(&[255, 255, 255, 0, 10, 255, 255, 255, 0, 10, 1, 2, 3, 4, 5]);
    assert_eq!(raster::compress(&p), [0x01, 0x02, 0xFF, 0xFF, 0x00, 0x0A, 0x00, 0xFC, 1, 2, 3, 4, 5]);
    // Long runs split at 128; lone pixels are runs of one.
    let mut q = Page::blank(300, 1, 75, Color::Rgb8);
    q.pixels[299 * 3..].copy_from_slice(&[1, 2, 3]);
    assert_eq!(raster::compress(&q), [0x00, 127, 255, 255, 255, 127, 255, 255, 255, 42, 255, 255, 255, 0, 1, 2, 3]);

    let o = PwgOptions { media: "iso_a4_210x297mm".into(), ..Default::default() };
    let doc = raster::pwg_document(&[p.clone()], &o);
    assert_eq!(&doc[..4], b"RaS2");
    let h = &doc[4..4 + raster::PWG_HEADER_LEN];
    let u = |at: usize| u32::from_be_bytes(h[at..at + 4].try_into().unwrap());
    assert_eq!(&h[..10], b"PwgRaster\0");
    assert_eq!((u(276), u(280)), (75, 75));
    assert_eq!((u(352), u(356)), (595, 842));
    assert_eq!((u(372), u(376), u(384), u(388), u(392), u(400), u(420), u(452)), (5, 3, 8, 8, 5, 18, 1, 1));
    assert_eq!(&h[1732..1748], b"iso_a4_210x297mm");
    assert_eq!(&doc[4 + raster::PWG_HEADER_LEN..], raster::compress(&p).as_slice());
    assert_eq!(doc.len(), 4 + 1796 + 13);

    let urf = raster::urf_document(&[p], false, 0);
    assert_eq!(&urf[..12], b"UNIRAST\0\0\0\0\x01");
    assert_eq!(&urf[12..36], &[8, 0, 1, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 3, 0, 0, 0, 75]);
}

#[test]
fn a4_page_size() {
    let p = Page::for_media("iso_a4_210x297mm", 300, Color::Gray8).unwrap();
    assert_eq!((p.width, p.height), (2480, 3508));
    // A blank page compresses to a few bytes per 256 lines.
    assert!(raster::compress(&p).len() < 14 * 45); // 20 runs of 128 per 256 lines
}

#[test]
fn mdns_query_golden() {
    assert_eq!(dnssd::browse_query(0x4e64, true), fixture("dnssd_browse_query.bin"));
    let q = Packet::parse(&fixture("dnssd_browse_query.bin")).unwrap();
    assert_eq!(q.questions[2], ("_uscan._tcp.local".to_string(), dnssd::rtype::PTR));
    assert!(!q.is_response());
}

#[test]
fn mdns_response_parse() {
    let p = Packet::parse(&fixture("dnssd_response.bin")).unwrap();
    assert!(p.is_response());
    assert_eq!(p.id, 0x4e64);
    assert_eq!(p.records.len(), 7);
    let s = dnssd::services(&p);
    assert_eq!(s.len(), 2);
    let printer = s.iter().find(|s| s.is_printer()).unwrap();
    assert_eq!(printer.name(), "Needle Test");
    assert_eq!((printer.host.as_str(), printer.port), ("needle-test.local", 8631));
    assert_eq!(printer.v4, [[10, 0, 2, 2]]);
    assert_eq!(printer.txt("rp"), Some("ipp/print"));
    assert_eq!(printer.txt("pdl"), Some("application/pdf,image/pwg-raster,image/urf"));
    assert_eq!(printer.txt("urf"), Some("V1.4,CP1,W8,SRGB24,RS300"));
    assert_eq!((printer.txt("Color"), printer.txt("Duplex")), (Some("T"), Some("T")));
    assert_eq!(printer.txt("UUID"), Some("4e65656c-6465-4000-8000-000000000001"));
    assert_eq!(printer.uri(), "ipp://10.0.2.2:8631/ipp/print");
    let scanner = s.iter().find(|s| s.is_scanner()).unwrap();
    assert_eq!(scanner.uri(), "http://10.0.2.2:8080/eSCL");
}

#[test]
fn mdns_compression_and_aaaa() {
    // A hand-made answer using name compression pointers and an AAAA record.
    let mut b = vec![0, 0, 0x84, 0, 0, 0, 0, 3, 0, 0, 0, 0];
    let ptr_owner = b.len();
    for l in ["_ipp", "_tcp", "local"] { b.push(l.len() as u8); b.extend_from_slice(l.as_bytes()); }
    b.push(0);
    b.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 120]);
    let inst = b"\x07Lab.One"; // a dot inside the instance label
    b.extend_from_slice(&((inst.len() + 2) as u16).to_be_bytes());
    let inst_at = b.len();
    b.extend_from_slice(inst);
    b.extend_from_slice(&[0xC0, ptr_owner as u8]);
    // SRV owned by the instance (pointer), target "lab.local" with compressed "local".
    b.extend_from_slice(&[0xC0, inst_at as u8, 0, 33, 0x80, 1, 0, 0, 0, 120]);
    let host = [3, b'l', b'a', b'b', 0xC0, (ptr_owner + 10) as u8];
    b.extend_from_slice(&((6 + host.len()) as u16).to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 0, 0x02, 0x77]);
    let host_at = b.len();
    b.extend_from_slice(&host);
    b.extend_from_slice(&[0xC0, host_at as u8, 0, 28, 0x80, 1, 0, 0, 0, 120, 0, 16]);
    b.extend_from_slice(&[0xfd, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    let p = Packet::parse(&b).unwrap();
    let s = dnssd::services(&p);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].instance, "Lab\\.One._ipp._tcp.local");
    assert_eq!(s[0].name(), "Lab.One");
    assert_eq!((s[0].host.as_str(), s[0].port), ("lab.local", 631));
    assert_eq!(s[0].uri(), "ipp://[fd00:0:0:0:0:0:0:1]:631/ipp/print");
    // A pointer loop is refused rather than followed forever.
    let mut bad = b.clone();
    bad[ptr_owner] = 0xC0;
    bad[ptr_owner + 1] = ptr_owner as u8;
    assert!(Packet::parse(&bad).is_err());
}

#[test]
fn escl_capabilities_fixtures() {
    let c = Capabilities::parse(&String::from_utf8(fixture("escl_capabilities.xml")).unwrap()).unwrap();
    assert_eq!((c.version.as_str(), c.make_and_model.as_str()), ("2.63", "SexOS Fake eSCL"));
    let p = c.platen.as_ref().unwrap();
    assert_eq!((p.max_width, p.max_height), (2550, 3508));
    assert_eq!(p.color_modes, ["BlackAndWhite1", "Grayscale8", "RGB24"]);
    assert_eq!(p.formats, ["image/jpeg", "image/png", "application/pdf"]);
    assert_eq!(p.resolutions, [(75, 75), (300, 300)]);
    assert!(c.adf.is_none());
    let s = ScanSettings::choose(&c, "image/png", true, 75).unwrap();
    assert_eq!((s.input_source.as_str(), s.color_mode.as_str(), s.format.as_str(), s.resolution), ("Platen", "Grayscale8", "image/png", 75));
    let xml = s.to_xml();
    let back = sex_print::xml::parse(&xml).unwrap();
    assert_eq!(back.name, "ScanSettings");
    assert_eq!(back.text_of("ColorMode"), Some("Grayscale8"));
    assert_eq!(back.path(&["ScanRegions", "ScanRegion", "Height"]).map(|e| e.text.as_str()), Some("3508"));
    assert!(xml.contains("<pwg:InputSource>Platen</pwg:InputSource>"));

    let a = Capabilities::parse(&String::from_utf8(fixture("escl_capabilities_adf.xml")).unwrap()).unwrap();
    assert_eq!(a.make_and_model, "Office Jet & Scan 5000");
    assert!(a.platen.is_none());
    let f = a.adf.as_ref().unwrap();
    assert_eq!(f.color_modes, ["RGB24", "Grayscale8"]);
    assert_eq!(f.resolutions, [(200, 200), (600, 600)]);
    let s = ScanSettings::choose(&a, "image/png", false, 300).unwrap();
    assert_eq!((s.input_source.as_str(), s.format.as_str(), s.color_mode.as_str(), s.resolution), ("Feeder", "image/jpeg", "RGB24", 600));
}

#[test]
fn text_pdf_is_well_formed() {
    let doc = pdf::text_to_pdf("Hello (Needle)\\\nline two\n", "na_letter_8.5x11in");
    assert!(pdf::is_pdf(&doc));
    let s = String::from_utf8(doc.clone()).unwrap();
    assert!(s.contains("/MediaBox [0 0 612 792]"));
    assert!(s.contains("(Hello \\(Needle\\)\\\\) Tj"));
    // startxref points at the xref table, and each offset at its object.
    let xref: usize = s.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
    assert!(s[xref..].starts_with("xref\n"));
    for (i, line) in s[xref..].lines().skip(3).take(5).enumerate() {
        let off: usize = line[..10].parse().unwrap();
        assert!(s[off..].starts_with(&format!("{} 0 obj", i + 1)), "object {}", i + 1);
    }
    // 200 lines on Letter at 12 pt leading: four pages.
    let long: String = (0..200).map(|i| format!("{i}\n")).collect();
    assert!(String::from_utf8(pdf::text_to_pdf(&long, "na_letter_8.5x11in")).unwrap().contains("/Count 4"));
}

#[test]
fn url_forms() {
    assert_eq!(Url::parse(URI).unwrap().to_string(), URI);
}
