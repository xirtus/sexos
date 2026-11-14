//! Round trips: fixture → edits → incremental save → our reader, lopdf, hayro (and pypdf if present).

mod fixtures;

use fixtures::{form, simple, Style};
use vellum_edit::*;

fn render(pdf: &[u8]) -> (usize, usize, Vec<u8>) {
    let doc = hayro::hayro_syntax::Pdf::new(pdf.to_vec()).expect("hayro parses");
    let pages = doc.pages();
    let page = &pages[0];
    let cache = hayro::RenderCache::new();
    let pm = hayro::render(page, &cache, &Default::default(), &Default::default(),
        &hayro::PixmapSettings { x_scale: 1.0, y_scale: 1.0, bg_color: hayro::vello_cpu::color::palette::css::WHITE });
    (pm.width() as usize, pm.height() as usize, pm.data_as_u8_slice().to_vec())
}

/// RGB at PDF user-space point (x, y) on a 200x200 page rendered at scale 1.
fn px(img: &(usize, usize, Vec<u8>), x: f64, y: f64) -> [u8; 3] {
    let (w, h, d) = img;
    let (cx, cy) = (x as usize, h - 1 - y as usize);
    let i = (cy * w + cx) * 4;
    [d[i], d[i + 1], d[i + 2]]
}

fn region_diff(a: &(usize, usize, Vec<u8>), b: &(usize, usize, Vec<u8>), r: [f64; 4]) -> usize {
    let mut n = 0;
    let mut y = r[1] + 0.5;
    while y < r[3] {
        let mut x = r[0] + 0.5;
        while x < r[2] {
            if px(a, x, y) != px(b, x, y) { n += 1; }
            x += 1.0;
        }
        y += 1.0;
    }
    n
}

fn style() -> AnnotStyle {
    AnnotStyle {
        author: Some("Zoë 李".into()),
        contents: Some("Check this (and that)".into()),
        date: Some(PdfDate { year: 2026, month: 11, day: 14, hour: 9, minute: 30, second: 5, utc_offset_minutes: 60 }),
        ..Default::default()
    }
}

fn assert_incremental(orig: &[u8], saved: &[u8]) {
    assert!(saved.len() > orig.len());
    assert_eq!(&saved[..orig.len()], orig, "original bytes are an untouched prefix");
    let tail = &saved[orig.len()..];
    let s = String::from_utf8_lossy(tail);
    assert!(s.contains("/Prev "), "update points back with /Prev");
    assert!(s.trim_end().ends_with("%%EOF"));
}

fn lopdf_annots(saved: &[u8]) -> Vec<lopdf::Dictionary> {
    let doc = lopdf::Document::load_mem(saved).expect("lopdf loads the update");
    let pages = doc.get_pages();
    let pid = *pages.get(&1).unwrap();
    doc.get_page_annotations(pid).unwrap().into_iter().cloned().collect()
}

#[test]
fn highlight_on_table_file_renders_and_round_trips() {
    let orig = simple(Style::Table);
    let mut ed = Editor::open(orig.clone()).unwrap();
    assert_eq!(ed.xref_style(), XrefStyle::Table);
    assert_eq!(ed.page_count(), 1);
    assert_eq!(ed.page_info(0).unwrap().media_box, [0.0, 0.0, 200.0, 200.0]);
    assert!(ed.annotations(0).unwrap().is_empty());
    let hl = [18.0, 145.0, 140.0, 170.0];
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Highlight { rects: vec![hl] }, style: style() }).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert_incremental(&orig, &saved);
    // A table file gets a table update.
    assert!(String::from_utf8_lossy(&saved[orig.len()..]).contains("\nxref\n"));

    let re = Editor::open(saved.clone()).unwrap();
    let a = re.annotations(0).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].subtype, "Highlight");
    assert_eq!(a[0].author.as_deref(), Some("Zoë 李"));
    assert_eq!(a[0].contents.as_deref(), Some("Check this (and that)"));
    assert_eq!(a[0].modified.as_deref(), Some("D:20261114093005+01'00'"));
    assert_eq!(a[0].quad_points, vec![18.0, 170.0, 140.0, 170.0, 18.0, 145.0, 140.0, 145.0]);
    assert!(a[0].has_appearance);

    let l = lopdf_annots(&saved);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].get(b"Subtype").unwrap().as_name().unwrap(), b"Highlight");
    assert!(l[0].get(b"AP").is_ok());

    let before = render(&orig);
    let after = render(&saved);
    // White paper inside the highlight turns yellow; text stays dark (multiply).
    assert_eq!(px(&before, 135.0, 165.0), [255, 255, 255]);
    let y = px(&after, 135.0, 165.0);
    assert!(y[0] > 200 && y[1] > 180 && y[2] < 80, "highlight colour {y:?}");
    assert!(region_diff(&before, &after, hl) > 1500);
    // Outside the annotation nothing changed.
    assert_eq!(region_diff(&before, &after, [0.0, 0.0, 200.0, 140.0]), 0);
}

#[test]
fn every_kind_on_xref_stream_file() {
    let orig = simple(Style::XrefStream);
    let mut ed = Editor::open(orig.clone()).unwrap();
    assert_eq!(ed.xref_style(), XrefStyle::Stream);
    let st = style();
    let red = AnnotStyle { color: [1.0, 0.0, 0.0], ..st.clone() };
    let kinds = vec![
        AnnotKind::Underline { rects: vec![[20.0, 145.0, 130.0, 168.0]] },
        AnnotKind::StrikeOut { rects: vec![[20.0, 145.0, 130.0, 168.0]] },
        AnnotKind::Squiggly { rects: vec![[20.0, 40.0, 100.0, 55.0]] },
        AnnotKind::Note { at: (170.0, 195.0), icon: NoteIcon::Comment },
        AnnotKind::FreeText { rect: [20.0, 60.0, 180.0, 100.0], text: "Free text that wraps onto a second line".into(), font_size: 11.0 },
        AnnotKind::Ink { strokes: vec![vec![(150.0, 120.0), (190.0, 140.0), (150.0, 135.0)], vec![(160.0, 110.0)]], width: 3.0 },
    ];
    for k in &kinds {
        ed.add_annotation(0, &NewAnnot { kind: k.clone(), style: red.clone() }).unwrap();
    }
    let saved = ed.save_incremental().unwrap();
    assert_incremental(&orig, &saved);
    let tail = String::from_utf8_lossy(&saved[orig.len()..]).into_owned();
    assert!(tail.contains("/Type /XRef") && !tail.contains("\nxref\n"), "stream file gets a stream update");

    let re = Editor::open(saved.clone()).unwrap();
    let subs: Vec<String> = re.annotations(0).unwrap().into_iter().map(|a| a.subtype).collect();
    assert_eq!(subs, ["Underline", "StrikeOut", "Squiggly", "Text", "FreeText", "Ink"]);
    let ink = &re.annotations(0).unwrap()[5];
    assert_eq!(ink.ink.len(), 2);
    assert_eq!(ink.ink[0][1], (190.0, 140.0));
    assert_eq!(lopdf_annots(&saved).len(), 6);

    let before = render(&orig);
    let after = render(&saved);
    let redish = |p: [u8; 3]| p[0] > 180 && p[1] < 90 && p[2] < 90;
    assert!(redish(px(&after, 170.0, 130.0)), "ink stroke drawn: {:?}", px(&after, 170.0, 130.0));
    assert!(region_diff(&before, &after, [20.0, 60.0, 180.0, 100.0]) > 100, "free text drawn");
    assert!(region_diff(&before, &after, [170.0, 175.0, 190.0, 195.0]) > 100, "note icon drawn");
}

#[test]
fn delete_and_second_revision() {
    let orig = simple(Style::Table);
    let mut ed = Editor::open(orig.clone()).unwrap();
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Highlight { rects: vec![[10.0, 10.0, 50.0, 30.0]] }, style: style() }).unwrap();
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Note { at: (100.0, 100.0), icon: NoteIcon::Key }, style: style() }).unwrap();
    ed.commit().unwrap();
    let rev1 = ed.original().to_vec();
    assert!(!ed.is_modified());
    assert_eq!(ed.annotations(0).unwrap().len(), 2);
    ed.delete_annotation(0, 0).unwrap();
    assert!(matches!(ed.delete_annotation(0, 5), Err(Error::NoSuchAnnotation(5))));
    let rev2 = ed.save_incremental().unwrap();
    assert_incremental(&rev1, &rev2);
    assert_eq!(&rev2[..orig.len()], &orig[..]);
    let re = Editor::open(rev2.clone()).unwrap();
    let a = re.annotations(0).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].subtype, "Text");
    assert_eq!(lopdf_annots(&rev2).len(), 1);
    // Two %%EOF updates on top of the original.
    assert_eq!(String::from_utf8_lossy(&rev2).matches("%%EOF").count(), 3);
}

#[test]
fn no_edits_means_identical_bytes() {
    for s in [Style::Table, Style::XrefStream] {
        let orig = form(s, true);
        let ed = Editor::open(orig.clone()).unwrap();
        assert_eq!(ed.save_incremental().unwrap(), orig);
    }
}

fn check_form(style: Style) {
    let orig = form(style, false);
    let mut ed = Editor::open(orig.clone()).unwrap();
    let fields = ed.fields().unwrap();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["person.name", "person.age", "agree", "color", "size", "notes"]);
    let get = |fs: &[FieldInfo], n: &str| fs.iter().find(|f| f.name == n).unwrap().clone();
    assert!(matches!(get(&fields, "person.name").kind, FieldKind::Text { multiline: false, .. }));
    assert_eq!(get(&fields, "person.age").value, FieldValue::Text("42".into()));
    assert!(get(&fields, "person.age").read_only);
    assert_eq!(get(&fields, "agree").kind, FieldKind::Checkbox);
    assert_eq!(get(&fields, "agree").states, ["Yes"]);
    assert_eq!(get(&fields, "color").kind, FieldKind::Radio);
    assert_eq!(get(&fields, "color").states, ["Red", "Blue"]);
    assert_eq!(get(&fields, "color").value, FieldValue::State("Red".into()));
    assert_eq!(get(&fields, "size").kind, FieldKind::ComboBox { editable: false });
    assert_eq!(get(&fields, "size").options, [("S".into(), "S".into()), ("M".into(), "Medium".into()), ("L".into(), "L".into())]);
    assert_eq!(get(&fields, "person.name").widgets[0].1, Some(0));

    assert!(matches!(ed.set_field("person.age", FieldValue::Text("1".into())), Err(Error::ReadOnly(_))));
    assert!(matches!(ed.set_field("size", FieldValue::Text("XL".into())), Err(Error::BadValue(_))));
    assert!(matches!(ed.set_field("agree", FieldValue::State("Maybe".into())), Err(Error::BadValue(_))));
    assert!(matches!(ed.set_field("nope", FieldValue::Empty), Err(Error::NoSuchField(_))));

    ed.set_field("person.name", FieldValue::Text("Ada Lovelace".into())).unwrap();
    ed.set_field("agree", FieldValue::State("Yes".into())).unwrap();
    ed.set_field("color", FieldValue::State("Blue".into())).unwrap();
    ed.set_field("size", FieldValue::Text("Medium".into())).unwrap();
    ed.set_field("notes", FieldValue::Text("Line one\nLine two is long enough to wrap around".into())).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert_incremental(&orig, &saved);

    let re = Editor::open(saved.clone()).unwrap();
    let f = re.fields().unwrap();
    assert_eq!(get(&f, "person.name").value, FieldValue::Text("Ada Lovelace".into()));
    assert_eq!(get(&f, "agree").value, FieldValue::State("Yes".into()));
    assert_eq!(get(&f, "color").value, FieldValue::State("Blue".into()));
    assert_eq!(get(&f, "size").value, FieldValue::Text("M".into()));

    // lopdf sees the values, the radio states and NeedAppearances.
    let doc = lopdf::Document::load_mem(&saved).unwrap();
    let mut found = 0;
    for (_, obj) in doc.objects.iter() {
        let Ok(d) = obj.as_dict() else { continue };
        match d.get(b"T").ok().and_then(|t| t.as_str().ok()) {
            Some(b"name") => { assert_eq!(d.get(b"V").unwrap().as_str().unwrap(), b"Ada Lovelace"); found += 1; }
            Some(b"agree") => { assert_eq!(d.get(b"AS").unwrap().as_name().unwrap(), b"Yes"); found += 1; }
            _ => {}
        }
        if d.get(b"Parent").is_ok() && d.get(b"AS").is_ok() && d.get(b"Rect").unwrap().as_array().unwrap()[0].as_float().unwrap() == 90.0 {
            assert_eq!(d.get(b"AS").unwrap().as_name().unwrap(), b"Blue");
            found += 1;
        }
    }
    assert_eq!(found, 3);
    let cat = doc.catalog().unwrap();
    let af = match cat.get(b"AcroForm").unwrap() { lopdf::Object::Reference(r) => doc.get_dictionary(*r).unwrap(), o => o.as_dict().unwrap() };
    assert!(af.get(b"NeedAppearances").unwrap().as_bool().unwrap());

    let before = render(&orig);
    let after = render(&saved);
    assert!(region_diff(&before, &after, [22.0, 152.0, 178.0, 168.0]) > 150, "name text drawn");
    assert!(region_diff(&before, &after, [20.0, 95.0, 40.0, 115.0]) > 50, "checkbox checked");
    assert!(region_diff(&before, &after, [60.0, 95.0, 80.0, 115.0]) > 50, "radio Red cleared");
    assert!(region_diff(&before, &after, [90.0, 95.0, 110.0, 115.0]) > 50, "radio Blue set");
    assert!(region_diff(&before, &after, [20.0, 20.0, 180.0, 80.0]) > 100, "notes drawn");
    // Blue text from the field's /DA.
    let blue = (152..168).flat_map(|y| (22..178).map(move |x| (x, y))).any(|(x, y)| { let p = px(&after, x as f64, y as f64); p[2] > 150 && p[0] < 100 });
    assert!(blue);
}

#[test]
fn form_fill_table() { check_form(Style::Table); }

#[test]
fn form_fill_xref_stream() { check_form(Style::XrefStream); }

#[test]
fn signatures_are_reported_and_survive() {
    let orig = form(Style::Table, true);
    let mut ed = Editor::open(orig.clone()).unwrap();
    let sigs = ed.signatures().unwrap();
    assert_eq!(sigs.len(), 1);
    let s = &sigs[0];
    assert!(s.signed);
    assert_eq!(s.field_name, "sig");
    assert_eq!(s.signer_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(s.date.as_deref(), Some("D:20261114093000+01'00'"));
    assert_eq!(PdfDate::parse(s.date.as_ref().unwrap()).unwrap().utc_offset_minutes, 60);
    assert_eq!(s.reason.as_deref(), Some("Approved"));
    assert_eq!(s.sub_filter.as_deref(), Some("adbe.pkcs7.detached"));
    assert!(s.covers_whole_file, "{:?}", s.byte_range);
    assert!(ed.fields().unwrap().iter().any(|f| f.name == "sig" && f.kind == FieldKind::Signature));
    assert!(matches!(ed.set_field("sig", FieldValue::Empty), Err(Error::ReadOnly(_))));

    ed.set_field("person.name", FieldValue::Text("Grace".into())).unwrap();
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Highlight { rects: vec![[10.0, 180.0, 60.0, 195.0]] }, style: style() }).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert_incremental(&orig, &saved);
    // The signed byte ranges are byte-identical in the new file.
    for (o, l) in &s.byte_range {
        let r = *o as usize..(*o + *l) as usize;
        assert_eq!(&saved[r.clone()], &orig[r]);
    }
    // With a signature present we do not ask viewers to regenerate appearances.
    assert!(!String::from_utf8_lossy(&saved[orig.len()..]).contains("NeedAppearances"));
    let re = Editor::open(saved).unwrap();
    let s2 = &re.signatures().unwrap()[0];
    assert!(s2.signed && !s2.covers_whole_file, "the signature now covers an earlier revision only");
}

#[test]
fn broken_xref_is_repaired_and_saved_with_full_table() {
    let mut orig = simple(Style::Table);
    // Point startxref at garbage.
    let at = orig.windows(9).rposition(|w| w == b"startxref").unwrap();
    let end = orig.len();
    orig.truncate(at);
    orig.extend_from_slice(b"startxref\n12\n%%EOF\n");
    let _ = end;
    let mut ed = Editor::open(orig.clone()).unwrap();
    assert_eq!(ed.xref_style(), XrefStyle::Repaired);
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Highlight { rects: vec![[18.0, 145.0, 140.0, 170.0]] }, style: style() }).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert_eq!(&saved[..orig.len()], &orig[..]);
    assert!(!String::from_utf8_lossy(&saved[orig.len()..]).contains("/Prev"));
    assert_eq!(Editor::open(saved.clone()).unwrap().annotations(0).unwrap().len(), 1);
    assert_eq!(lopdf_annots(&saved).len(), 1);
    let after = render(&saved);
    let y = px(&after, 135.0, 165.0);
    assert!(y[2] < 80, "{y:?}");
}

#[test]
fn rejects_non_pdf_and_encrypted() {
    assert_eq!(Editor::open(b"hello".to_vec()).err(), Some(Error::NotPdf));
    let mut enc = simple(Style::Table);
    let t = enc.windows(7).rposition(|w| w == b"/Root 1").unwrap();
    enc.splice(t..t, b"/Encrypt << /Filter /Standard >> ".iter().copied());
    assert_eq!(Editor::open(enc).err(), Some(Error::Encrypted));
}

#[test]
fn dates_round_trip() {
    let d = PdfDate { year: 2026, month: 1, day: 2, hour: 3, minute: 4, second: 5, utc_offset_minutes: -330 };
    assert_eq!(d.to_pdf_string(), "D:20260102030405-05'30'");
    assert_eq!(PdfDate::parse(&d.to_pdf_string()), Some(d));
    assert_eq!(PdfDate::parse("D:2026").unwrap().month, 1);
    assert_eq!(PdfDate { utc_offset_minutes: 0, ..d }.to_pdf_string(), "D:20260102030405Z");
}

/// Optional: pypdf, if the host has it, reads the annotations and field values.
#[test]
fn pypdf_reads_our_output_if_available() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let ok = Command::new("python3").args(["-c", "import pypdf"]).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if !ok { eprintln!("pypdf not available; skipped"); return; }
    let orig = form(Style::XrefStream, false);
    let mut ed = Editor::open(orig).unwrap();
    ed.set_field("person.name", FieldValue::Text("Ada".into())).unwrap();
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Ink { strokes: vec![vec![(1.0, 1.0), (5.0, 5.0)]], width: 1.0 }, style: style() }).unwrap();
    let saved = ed.save_incremental().unwrap();
    let script = "import sys,io,pypdf\nr=pypdf.PdfReader(io.BytesIO(sys.stdin.buffer.read()),strict=True)\nf=r.get_fields()\nprint(len(r.pages[0]['/Annots']), f['person.name'].get('/V'))";
    let mut child = Command::new("python3").args(["-I", "-c", script]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&saved).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "pypdf failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(text.starts_with("8 Ada"), "{text}");
}
