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

#[test]
fn new_signature_field_round_trips_and_can_receive_detached_cms() {
    let original = simple(Style::Table);
    let mut editor = Editor::open(original.clone()).unwrap();
    editor.add_signature_field(0, "Vellum signature 1", [20.0, 20.0, 160.0, 50.0]).unwrap();
    let field = editor.fields().unwrap().into_iter().find(|f| f.name == "Vellum signature 1").unwrap();
    assert_eq!(field.kind, FieldKind::Signature);
    assert_eq!(field.widgets.len(), 1);
    assert_eq!(field.widgets[0].1, Some(0));
    assert_eq!(field.widgets[0].2, [20.0, 20.0, 160.0, 50.0]);
    assert!(!editor.signatures().unwrap()[0].signed);
    let saved = editor.save_incremental().unwrap();
    assert_incremental(&original, &saved);
    assert!(region_diff(&render(&original), &render(&saved), [19.0, 19.0, 162.0, 52.0]) > 0,
        "the empty signature field must be visible");
    let independent = lopdf::Document::load_mem(&saved).unwrap();
    let catalog = independent.catalog().unwrap();
    let form = catalog.get(b"AcroForm").unwrap().as_dict().unwrap();
    assert_eq!(form.get(b"SigFlags").unwrap().as_i64().unwrap(), 3);
    let reopened = Editor::open(saved).unwrap();
    let signed = reopened.embed_detached_signature_with("Vellum signature 1", 1024, |a, b| {
        assert!(!a.is_empty() && !b.is_empty());
        Ok(vec![1, 2, 3])
    }).unwrap();
    assert!(Editor::open(signed).unwrap().signatures().unwrap()[0].signed);
}

#[test]
fn new_signature_field_rejects_bad_placement_and_signed_document() {
    let mut editor = Editor::open(form(Style::Table, false)).unwrap();
    assert!(editor.add_signature_field(0, "person.name", [20.0, 20.0, 80.0, 50.0]).is_err());
    assert!(editor.add_signature_field(0, "bad.name", [20.0, 20.0, 80.0, 50.0]).is_err());
    assert!(editor.add_signature_field(0, "New", [20.0, 20.0, 220.0, 50.0]).is_err());
    assert!(!editor.is_modified());
    let mut signed = Editor::open(form(Style::Table, true)).unwrap();
    assert!(signed.add_signature_field(0, "New", [20.0, 20.0, 80.0, 50.0]).is_err());
    assert!(!signed.is_modified());
}

#[test]
fn signed_pdf_refuses_annotation_add_and_remove_without_changes() {
    let mut editor = Editor::open(form(Style::Table, true)).unwrap();
    assert!(editor.signatures().unwrap().iter().any(|signature| signature.signed));
    let highlight = NewAnnot {
        kind: AnnotKind::Highlight { rects: vec![[18.0, 145.0, 140.0, 170.0]] },
        style: AnnotStyle::default(),
    };
    let expected = Error::BadValue("cannot edit annotations in a signed PDF".into());
    assert_eq!(editor.add_annotation(0, &highlight).unwrap_err(), expected);
    assert_eq!(editor.set_annotation_text(0, 0, "No change").unwrap_err(), expected);
    assert_eq!(editor.remove_annotation(0, 0).unwrap_err(), expected);
    assert!(!editor.is_modified());
}

#[test]
fn catalog_docmdp_without_signature_field_refuses_annotations() {
    let pdf = fixtures::build(&[
        (1, "<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 6 0 R >> >>".into()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into()),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".into()),
        (6, "<< /Type /Sig /Filter /Adobe.PPKLite /ByteRange [0 1 2 3] /Contents <0000> >>".into()),
    ], Style::Table, 1);
    let mut editor = Editor::open(pdf).unwrap();
    assert!(editor.signatures().unwrap().is_empty());
    let highlight = NewAnnot {
        kind: AnnotKind::Highlight { rects: vec![[18.0, 145.0, 140.0, 170.0]] },
        style: AnnotStyle::default(),
    };
    let expected = Error::BadValue("cannot edit annotations in a signed PDF".into());
    assert_eq!(editor.add_annotation(0, &highlight).unwrap_err(), expected);
    assert_eq!(editor.set_annotation_text(0, 0, "No change").unwrap_err(), expected);
    assert_eq!(editor.remove_annotation(0, 0).unwrap_err(), expected);
    assert!(!editor.is_modified());
}

#[test]
fn guest_signed_pdf_refuses_annotation_edits_when_supplied() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_SIGNED_PDF") else { return };
    let mut editor = Editor::open(std::fs::read(path).unwrap()).unwrap();
    let highlight = NewAnnot {
        kind: AnnotKind::Highlight { rects: vec![[18.0, 145.0, 140.0, 170.0]] },
        style: AnnotStyle::default(),
    };
    assert!(editor.signatures().unwrap().iter().any(|signature| signature.signed));
    assert!(matches!(editor.add_annotation(0, &highlight), Err(Error::BadValue(_))));
    assert!(!editor.is_modified());
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
fn colored_highlight_renders_and_survives_incremental_save() {
    let original = simple(Style::Table);
    let mut editor = Editor::open(original.clone()).unwrap();
    let color = [0.45, 0.9, 0.45];
    editor.add_annotation(0, &NewAnnot {
        kind: AnnotKind::Highlight { rects: vec![[18.0, 145.0, 140.0, 170.0]] },
        style: AnnotStyle { color, ..Default::default() },
    }).unwrap();
    let saved = editor.save_incremental().unwrap();
    assert_incremental(&original, &saved);
    let reopened = Editor::open(saved.clone()).unwrap();
    assert_eq!(reopened.annotations(0).unwrap()[0].color, Some(color));
    let highlighted = px(&render(&saved), 135.0, 165.0);
    assert!(highlighted[1] > highlighted[0] && highlighted[1] > highlighted[2],
        "green highlight renders green: {highlighted:?}");
}

#[test]
fn highlight_comment_edits_without_changing_appearance() {
    let original = simple(Style::Table);
    let mut editor = Editor::open(original.clone()).unwrap();
    editor.add_annotation(0, &NewAnnot {
        kind: AnnotKind::Highlight { rects: vec![[18.0, 145.0, 140.0, 170.0]] },
        style: AnnotStyle::default(),
    }).unwrap();
    let before_comment = editor.save_incremental().unwrap();
    let before = editor.annotations(0).unwrap().remove(0);
    assert_eq!(editor.set_annotation_text(0, before.index, "Review this paragraph").unwrap(), None);
    let after = editor.annotations(0).unwrap().remove(0);
    assert_eq!(after.contents.as_deref(), Some("Review this paragraph"));
    assert_eq!(after.quad_points, before.quad_points);
    assert_eq!(after.color, before.color);
    assert!(after.has_appearance);
    let saved = editor.save_incremental().unwrap();
    assert_incremental(&original, &saved);
    assert_eq!(Editor::open(saved.clone()).unwrap().annotations(0).unwrap()[0].contents.as_deref(), Some("Review this paragraph"));
    let l = lopdf_annots(&saved);
    assert_eq!(l[0].get(b"Contents").unwrap().as_str().unwrap(), b"Review this paragraph");
    assert_eq!(render(&saved), render(&before_comment));
    let mut reopened = Editor::open(saved).unwrap();
    assert_eq!(reopened.set_annotation_text(0, before.index, "").unwrap().as_deref(), Some("Review this paragraph"));
    assert_eq!(Editor::open(reopened.save_incremental().unwrap()).unwrap().annotations(0).unwrap()[0].contents, None);
}

#[test]
fn guest_highlight_comment_survives_guest_save() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_COMMENT_PDF") else { return };
    let editor = Editor::open(std::fs::read(path).unwrap()).unwrap();
    let highlight = editor.annotations(0).unwrap().into_iter().find(|a| a.subtype == "Highlight").unwrap();
    assert_eq!(highlight.contents.as_deref(), Some("Review green"));
}

#[test]
fn sticky_note_text_edits_without_moving_its_icon() {
    let original = simple(Style::Table);
    let mut editor = Editor::open(original.clone()).unwrap();
    editor.add_annotation(0, &NewAnnot {
        kind: AnnotKind::Note { at: (50.0, 100.0), icon: NoteIcon::Comment },
        style: AnnotStyle { contents: Some("check this".into()), ..Default::default() },
    }).unwrap();
    let before_pdf = editor.save_incremental().unwrap();
    let before = editor.annotations(0).unwrap().remove(0);
    assert_eq!(editor.set_annotation_text(0, before.index, "check this updated").unwrap().as_deref(), Some("check this"));
    let saved = editor.save_incremental().unwrap();
    assert_incremental(&original, &saved);
    let after = Editor::open(saved.clone()).unwrap().annotations(0).unwrap().remove(0);
    assert_eq!(after.contents.as_deref(), Some("check this updated"));
    assert_eq!(after.rect, before.rect);
    assert!(after.has_appearance);
    assert_eq!(render(&saved), render(&before_pdf));
    assert_eq!(lopdf_annots(&saved)[0].get(b"Contents").unwrap().as_str().unwrap(), b"check this updated");
}

#[test]
fn guest_note_edit_survives_guest_save() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_NOTE_EDIT_PDF") else { return };
    let editor = Editor::open(std::fs::read(path).unwrap()).unwrap();
    let note = editor.annotations(0).unwrap().into_iter().find(|a| a.subtype == "Text").unwrap();
    assert_eq!(note.contents.as_deref(), Some("check this updated"));
}

#[test]
fn translucent_highlight_renders_and_survives_incremental_save() {
    let original = simple(Style::Table);
    let mut opaque = Editor::open(original.clone()).unwrap();
    let mut translucent = Editor::open(original).unwrap();
    let rect = [18.0, 145.0, 140.0, 170.0];
    for (editor, opacity) in [(&mut opaque, 1.0), (&mut translucent, 0.5)] {
        editor.add_annotation(0, &NewAnnot {
            kind: AnnotKind::Highlight { rects: vec![rect] },
            style: AnnotStyle { color: [0.45, 0.9, 0.45], opacity, ..Default::default() },
        }).unwrap();
    }
    let opaque = opaque.save_incremental().unwrap();
    let saved = translucent.save_incremental().unwrap();
    assert_eq!(Editor::open(saved.clone()).unwrap().annotations(0).unwrap()[0].opacity, Some(0.5));
    let solid = px(&render(&opaque), 135.0, 165.0);
    let light = px(&render(&saved), 135.0, 165.0);
    assert!(light[0] > solid[0] && light[0] < 255,
        "50% green must be lighter than opaque green: {solid:?} vs {light:?}");
}

#[test]
fn guest_highlight_opacity_survives_guest_save() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_OPACITY_PDF") else { return };
    let editor = Editor::open(std::fs::read(path).unwrap()).unwrap();
    let highlights: Vec<_> = editor.annotations(0).unwrap().into_iter()
        .filter(|annot| annot.subtype == "Highlight").collect();
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].opacity, Some(0.75));
}

#[test]
fn guest_highlight_color_survives_guest_save() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_COLOR_PDF") else { return };
    let saved = std::fs::read(path).unwrap();
    let editor = Editor::open(saved).unwrap();
    let highlights: Vec<_> = editor.annotations(0).unwrap().into_iter()
        .filter(|annot| annot.subtype == "Highlight").collect();
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].color, Some([0.45, 0.9, 0.45]));
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
fn restore_existing_annotation_keeps_original_object_and_appearance() {
    let orig = simple(Style::Table);
    let mut ed = Editor::open(orig).unwrap();
    ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Note { at: (100.0, 100.0), icon: NoteIcon::Key }, style: style() }).unwrap();
    let saved = ed.save_incremental().unwrap();
    let mut reopened = Editor::open(saved).unwrap();
    let original = reopened.annotations(0).unwrap();
    let removed = reopened.remove_annotation(0, 0).unwrap();
    assert!(reopened.annotations(0).unwrap().is_empty());
    reopened.restore_annotation(&removed).unwrap();
    assert_eq!(reopened.annotations(0).unwrap(), original);
    assert!(reopened.restore_annotation(&removed).is_err(), "duplicate restore must fail");
    reopened.remove_restored_annotation(&removed).unwrap();
    assert!(reopened.annotations(0).unwrap().is_empty());
    reopened.restore_annotation(&removed).unwrap();
    assert_eq!(Editor::open(reopened.save_incremental().unwrap()).unwrap().annotations(0).unwrap(), original);
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
fn real_combo_and_list_pdf_keep_export_values_after_incremental_save() {
    const CHOICE: &[u8] = include_bytes!("../../../tests/fixtures/pdf/choice.pdf");
    let mut ed = Editor::open(CHOICE.to_vec()).unwrap();
    let fields = ed.fields().unwrap();
    let size = fields.iter().find(|f| f.name == "size").unwrap();
    let team = fields.iter().find(|f| f.name == "team").unwrap();
    assert_eq!(size.kind, FieldKind::ComboBox { editable: false });
    assert_eq!(team.kind, FieldKind::ListBox { multi_select: false });
    assert_eq!(size.options[1], ("M".into(), "Medium".into()));
    assert_eq!(team.options[1], ("E".into(), "Engineering".into()));
    ed.set_field("size", FieldValue::Text("M".into())).unwrap();
    ed.set_field("team", FieldValue::Text("E".into())).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert!(saved.starts_with(CHOICE));
    let reopened = Editor::open(saved).unwrap();
    let after = reopened.fields().unwrap();
    assert_eq!(after.iter().find(|f| f.name == "size").unwrap().value, FieldValue::Text("M".into()));
    assert_eq!(after.iter().find(|f| f.name == "team").unwrap().value, FieldValue::Text("E".into()));
}

#[test]
fn editable_combo_custom_text_round_trips_real_pdf() {
    const PDF: &[u8] = include_bytes!("../../../tests/fixtures/pdf/advanced-choice.pdf");
    let mut ed = Editor::open(PDF.to_vec()).unwrap();
    let fields = ed.fields().unwrap();
    let project = fields.iter().find(|f| f.name == "project").unwrap();
    assert_eq!(project.kind, FieldKind::ComboBox { editable: true });
    assert_eq!(project.value, FieldValue::Text("Custom".into()));
    ed.set_field("project", FieldValue::Text("New Project".into())).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert!(saved.starts_with(PDF));
    let reopened = Editor::open(saved).unwrap();
    let after = reopened.fields().unwrap();
    assert_eq!(after.iter().find(|f| f.name == "project").unwrap().value, FieldValue::Text("New Project".into()));
}

#[test]
fn multi_list_export_values_round_trip_real_pdf() {
    const PDF: &[u8] = include_bytes!("../../../tests/fixtures/pdf/advanced-choice.pdf");
    let mut ed = Editor::open(PDF.to_vec()).unwrap();
    let fields = ed.fields().unwrap();
    let teams = fields.iter().find(|f| f.name == "teams").unwrap();
    assert_eq!(teams.kind, FieldKind::ListBox { multi_select: true });
    assert_eq!(teams.value, FieldValue::Choices(vec!["A".into(), "D".into()]));
    ed.set_field("teams", FieldValue::Choices(vec!["E".into(), "D".into()])).unwrap();
    let saved = ed.save_incremental().unwrap();
    assert!(saved.starts_with(PDF));
    let after = Editor::open(saved).unwrap().fields().unwrap();
    assert_eq!(after.iter().find(|f| f.name == "teams").unwrap().value, FieldValue::Choices(vec!["E".into(), "D".into()]));
}

#[test]
fn guest_advanced_choices_survive_guest_save() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_ADVANCED_PDF") else { return };
    const PDF: &[u8] = include_bytes!("../../../tests/fixtures/pdf/advanced-choice.pdf");
    let saved = std::fs::read(path).unwrap();
    assert!(saved.starts_with(PDF));
    let fields = Editor::open(saved).unwrap().fields().unwrap();
    assert_eq!(fields.iter().find(|f| f.name == "project").unwrap().value, FieldValue::Text("Orbit".into()));
    assert_eq!(fields.iter().find(|f| f.name == "teams").unwrap().value, FieldValue::Choices(vec!["E".into(), "D".into()]));
}

#[test]
fn guest_pointer_choice_survives_guest_save() {
    let Ok(path) = std::env::var("SEXOS_VELLUM_GUEST_POINTER_PDF") else { return };
    const PDF: &[u8] = include_bytes!("../../../tests/fixtures/pdf/choice.pdf");
    let saved = std::fs::read(path).unwrap();
    assert!(saved.starts_with(PDF));
    let fields = Editor::open(saved).unwrap().fields().unwrap();
    assert_eq!(fields.iter().find(|f| f.name == "size").unwrap().value, FieldValue::Text("L".into()));
    assert_eq!(fields.iter().find(|f| f.name == "team").unwrap().value, FieldValue::Text("E".into()));
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
    assert!(s.byte_range_well_formed);
    assert_eq!(ed.signature_content_integrity("sig").unwrap(), ContentIntegrity::Unsupported);
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
fn detached_signature_slot_covers_exact_final_bytes() {
    let mut source = form(Style::Table, true);
    let old = b"/V 19 0 R";
    let at = source.windows(old.len()).position(|w| w == old).unwrap();
    source[at..at + old.len()].copy_from_slice(b"/V null  ");
    let editor = Editor::open(source.clone()).unwrap();
    assert!(!editor.signatures().unwrap()[0].signed);
    let mut covered = Vec::new();
    let signed = editor.embed_detached_signature_with("sig", 1024, |first, second| {
        covered.extend_from_slice(first);
        covered.extend_from_slice(second);
        Ok(vec![0x30, 0x03, 0x02, 0x01, 0x01])
    }).unwrap();
    assert_eq!(&signed[..source.len()], source.as_slice(), "incremental signing must keep original bytes");
    let parsed = Editor::open(signed.clone()).unwrap();
    let sig = &parsed.signatures().unwrap()[0];
    assert!(sig.signed && sig.byte_range_well_formed && sig.covers_whole_file, "{sig:?}");
    let mut actual = Vec::new();
    for (offset, len) in &sig.byte_range { actual.extend_from_slice(&signed[*offset as usize..(*offset + *len) as usize]); }
    assert_eq!(actual, covered, "the callback must see exactly the bytes the PDF covers");
    let gap = sig.byte_range[0].1 as usize..sig.byte_range[1].0 as usize;
    assert!(signed[gap.clone()].starts_with(b"<3003020101"));
    assert_eq!(gap.end - gap.start, 2050);
    let external = lopdf::Document::load_mem(&signed).expect("independent parser reopens signed PDF");
    let catalog = external.catalog().unwrap();
    let form = catalog.get(b"AcroForm").unwrap().as_dict().unwrap();
    let fields = form.get(b"Fields").unwrap().as_array().unwrap();
    let sig_field = external.get_dictionary(fields.last().unwrap().as_reference().unwrap()).unwrap();
    let sig_ref = sig_field.get(b"V").unwrap().as_reference().unwrap();
    let sig_dict = external.get_dictionary(sig_ref).unwrap();
    assert_eq!(sig_dict.get(b"SubFilter").unwrap().as_name().unwrap(), b"adbe.pkcs7.detached");
    assert_eq!(sig_dict.get(b"ByteRange").unwrap().as_array().unwrap().len(), 4);
    assert!(!editor.is_modified(), "signing output does not mutate the editor on success");
    let mut tampered = signed;
    tampered[source.len() + 1] ^= 1;
    let mut changed = Vec::new();
    for (offset, len) in &sig.byte_range { changed.extend_from_slice(&tampered[*offset as usize..(*offset + *len) as usize]); }
    assert_ne!(changed, covered, "a covered byte change must alter the callback's message");
}

#[test]
fn detached_cms_in_pdf_verifies_over_its_byte_range() {
    use shibari::{Keeper, Owner};
    use std::{fs, process::Command};
    const SCALAR: [u8; 32] = [
        0x15, 0xb6, 0xeb, 0x1d, 0x2f, 0xa7, 0xec, 0x84,
        0xfc, 0x8d, 0x80, 0xef, 0x31, 0xbc, 0x5f, 0x6c,
        0x37, 0xdb, 0x2e, 0x88, 0x87, 0x94, 0x59, 0x32,
        0xee, 0x00, 0xc7, 0x32, 0x2c, 0x42, 0xa7, 0xce,
    ];
    let cert = include_bytes!("fixtures/cms-test-cert.der");
    let owner = Owner(88);
    let mut keeper = Keeper::new(0x9183, 1);
    let knot = keeper.tie(owner, "PDF test key", &SCALAR).unwrap();
    let point = keeper.p256_public_key(owner, knot).unwrap();
    let mut source = form(Style::Table, true);
    let old = b"/V 19 0 R";
    let at = source.windows(old.len()).position(|w| w == old).unwrap();
    source[at..at + old.len()].copy_from_slice(b"/V null  ");
    let editor = Editor::open(source).unwrap();
    let mut cms = Vec::new();
    let signed = editor.embed_detached_signature_with("sig", 2048, |first, second| {
        cms = vellum_edit::cms::detached_p256_sha256(first, second, cert, &point, |attrs| {
            keeper.sign_p256_sha256(owner, knot, attrs)
                .map_err(|_| vellum_edit::cms::CmsError::SignerRefused)
        }).map_err(|e| Error::BadValue(format!("CMS: {e:?}")))?;
        Ok(cms.clone())
    }).unwrap();
    assert_eq!(Editor::open(signed.clone()).unwrap().signature_content_integrity("sig").unwrap(), ContentIntegrity::Intact);
    let mut tampered = signed.clone();
    let old_value = b"/V null  ";
    let at = tampered.windows(old_value.len()).position(|w| w == old_value).unwrap();
    tampered[at + 7] = b'\t'; // PDF whitespace remains valid, but the covered bytes changed.
    assert_eq!(Editor::open(tampered).unwrap().signature_content_integrity("sig").unwrap(), ContentIntegrity::Changed);
    let mut appended = signed.clone();
    appended.extend_from_slice(b"\n%later revision\n");
    assert_eq!(Editor::open(appended).unwrap().signature_content_integrity("sig").unwrap(), ContentIntegrity::Unsupported);
    if let Some(destination) = std::env::var_os("SEXOS_VELLUM_CMS_PDF_FIXTURE") {
        fs::write(destination, &signed).unwrap();
    }
    let independent = lopdf::Document::load_mem(&signed).unwrap();
    let acroform = independent.catalog().unwrap().get(b"AcroForm").unwrap().as_dict().unwrap();
    let field = independent.get_dictionary(acroform.get(b"Fields").unwrap().as_array().unwrap()
        .last().unwrap().as_reference().unwrap()).unwrap();
    let signature = independent.get_dictionary(field.get(b"V").unwrap().as_reference().unwrap()).unwrap();
    let contents = signature.get(b"Contents").unwrap().as_str().unwrap();
    assert!(contents.starts_with(&cms));
    assert!(contents[cms.len()..].iter().all(|b| *b == 0));
    let range = &Editor::open(signed.clone()).unwrap().signatures().unwrap()[0].byte_range;
    let mut covered = Vec::new();
    for (offset, len) in range { covered.extend_from_slice(&signed[*offset as usize..(*offset + *len) as usize]); }
    let path = std::env::temp_dir().join(format!("sexos-vellum-pdf-cms-{}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("cms.der"), &cms).unwrap();
    fs::write(path.join("covered.bin"), &covered).unwrap();
    let verified = Command::new("openssl").args(["cms", "-verify", "-inform", "DER", "-binary", "-noverify", "-in"])
        .arg(path.join("cms.der")).arg("-content").arg(path.join("covered.bin"))
        .arg("-out").arg(path.join("verified.bin")).output().unwrap();
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));
    assert_eq!(fs::read(path.join("verified.bin")).unwrap(), covered);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn guest_signed_pdf_verifies_independently_when_supplied() {
    use std::{fs, process::Command};
    let Some(input) = std::env::var_os("SEXOS_VELLUM_GUEST_SIGNED_PDF") else { return; };
    let bytes = fs::read(input).unwrap();
    let editor = Editor::open(bytes.clone()).unwrap();
    assert_eq!(editor.signature_content_integrity("Vellum signature 1").unwrap(), ContentIntegrity::Intact);
    let signatures = editor.signatures().unwrap();
    assert_eq!(signatures.len(), 1);
    let signature = &signatures[0];
    assert!(signature.signed && signature.byte_range_well_formed && signature.covers_whole_file);
    let mut covered = Vec::new();
    for (offset, len) in &signature.byte_range {
        covered.extend_from_slice(&bytes[*offset as usize..(*offset + *len) as usize]);
    }
    let independent = lopdf::Document::load_mem(&bytes).unwrap();
    let acroform = independent.catalog().unwrap().get(b"AcroForm").unwrap().as_dict().unwrap();
    let field = independent.get_dictionary(acroform.get(b"Fields").unwrap().as_array().unwrap()
        .last().unwrap().as_reference().unwrap()).unwrap();
    let signed = independent.get_dictionary(field.get(b"V").unwrap().as_reference().unwrap()).unwrap();
    let contents = signed.get(b"Contents").unwrap().as_str().unwrap();
    assert_eq!(contents[0], 0x30);
    let (header, body) = if contents[1] & 0x80 == 0 {
        (2, contents[1] as usize)
    } else {
        let n = (contents[1] & 0x7f) as usize;
        assert!((1..=4).contains(&n) && contents.len() >= n + 2);
        (n + 2, contents[2..n + 2].iter().fold(0usize, |v, b| (v << 8) | *b as usize))
    };
    assert!(header + body <= contents.len());
    assert!(contents[header + body..].iter().all(|b| *b == 0));
    let dir = std::env::temp_dir().join(format!("sexos-vellum-guest-cms-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("cms.der"), &contents[..header + body]).unwrap();
    fs::write(dir.join("covered.bin"), &covered).unwrap();
    let verified = Command::new("openssl").args(["cms", "-verify", "-inform", "DER", "-binary", "-noverify", "-in"])
        .arg(dir.join("cms.der")).arg("-content").arg(dir.join("covered.bin"))
        .arg("-out").arg(dir.join("verified.bin")).output().unwrap();
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));
    assert_eq!(fs::read(dir.join("verified.bin")).unwrap(), covered);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn detached_signature_slot_refuses_invalid_requests_without_editing() {
    let mut source = form(Style::Table, true);
    let old = b"/V 19 0 R";
    let at = source.windows(old.len()).position(|w| w == old).unwrap();
    source[at..at + old.len()].copy_from_slice(b"/V null  ");
    let editor = Editor::open(source.clone()).unwrap();
    assert!(editor.embed_detached_signature_with("sig", 1023, |_, _| Ok(vec![1])).is_err());
    assert!(editor.embed_detached_signature_with("name", 1024, |_, _| Ok(vec![1])).is_err());
    assert!(editor.embed_detached_signature_with("sig", 1024, |_, _| Ok(vec![1; 1025])).is_err());
    assert!(editor.embed_detached_signature_with("sig", 1024, |_, _| Err(Error::BadValue("signer failed".into()))).is_err());
    assert!(!editor.is_modified());
    assert_eq!(editor.original(), source);
}

#[test]
fn malformed_signature_range_is_not_presented_as_covered() {
    let mut pdf = form(Style::Table, true);
    let at = pdf.windows(b"/ByteRange [0 ".len()).position(|w| w == b"/ByteRange [0 ").unwrap() + b"/ByteRange [0 ".len();
    pdf[at..at + 10].copy_from_slice(b"9999999999");
    let signature = Editor::open(pdf).unwrap().signatures().unwrap().remove(0);
    assert!(signature.signed);
    assert!(!signature.byte_range_well_formed);
    assert!(!signature.covers_whole_file);
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
