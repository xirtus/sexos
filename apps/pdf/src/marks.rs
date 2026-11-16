// SPDX-License-Identifier: GPL-3.0-only
//! The arithmetic behind annotating and form filling, tested on the host.
//!
//! The viewer works in "shown" coordinates: points from the page's top left as it is
//! shown, rotation applied (what hayro draws and what [`crate::text`] measures).
//! vellum-edit works in PDF user space: points from the bottom left of the unrotated
//! page. These convert between the two, merge a found word's character boxes into one
//! rectangle per line, order form fields as the eye reads them, and date annotations.
use vellum_edit::{FieldInfo, FieldKind, PageInfo, PdfDate};

/// A shown point in user space.
pub fn to_user(p: &PageInfo, (x, y): (f64, f64)) -> (f64, f64) {
    let [x0, y0, x1, y1] = p.crop_box;
    match p.rotate {
        90 => (x0 + y, y0 + x),
        180 => (x1 - x, y0 + y),
        270 => (x1 - y, y1 - x),
        _ => (x0 + x, y1 - y),
    }
}

/// A user-space point as shown.
pub fn to_shown(p: &PageInfo, (ux, uy): (f64, f64)) -> (f64, f64) {
    let [x0, y0, x1, y1] = p.crop_box;
    match p.rotate {
        90 => (uy - y0, ux - x0),
        180 => (x1 - ux, uy - y0),
        270 => (y1 - uy, x1 - ux),
        _ => (ux - x0, y1 - uy),
    }
}

/// A shown rectangle `[left, top, right, bottom]` in user space `[x0, y0, x1, y1]`.
pub fn rect_to_user(p: &PageInfo, r: [f64; 4]) -> [f64; 4] {
    let (ax, ay) = to_user(p, (r[0], r[1]));
    let (bx, by) = to_user(p, (r[2], r[3]));
    [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]
}

/// A user-space rectangle as shown, `[left, top, right, bottom]`.
pub fn rect_to_shown(p: &PageInfo, r: [f64; 4]) -> [f64; 4] {
    let (ax, ay) = to_shown(p, (r[0], r[1]));
    let (bx, by) = to_shown(p, (r[2], r[3]));
    [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)]
}

/// Character boxes (shown, `[left, top, right, bottom]`) merged into one rectangle per
/// line: a box whose middle lies outside the line so far starts a new one.
pub fn line_rects(boxes: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let mut lines: Vec<[f64; 4]> = Vec::new();
    for b in boxes {
        let mid = (b[1] + b[3]) / 2.0;
        match lines.last_mut() {
            Some(l) if mid >= l[1] && mid <= l[3] => {
                *l = [l[0].min(b[0]), l[1].min(b[1]), l[2].max(b[2]), l[3].max(b[3])];
            }
            _ => lines.push(*b),
        }
    }
    lines
}

/// Fields the keyboard can fill: text fields and checkboxes that are not read-only and
/// have a widget, in reading order (page, then top to bottom, then left to right).
pub fn fillable(fields: &[FieldInfo]) -> Vec<(FieldInfo, usize, [f64; 4])> {
    let mut out: Vec<(FieldInfo, usize, [f64; 4])> = fields.iter()
        .filter(|f| !f.read_only && matches!(f.kind, FieldKind::Text { password: false, .. } | FieldKind::Checkbox))
        .filter_map(|f| f.widgets.iter().find_map(|(_, page, rect)| page.map(|p| (f.clone(), p, *rect))))
        .collect();
    // Rows within a few points of each other read left to right.
    out.sort_by(|a, b| a.1.cmp(&b.1)
        .then(((b.2[3] / 4.0).round() as i64).cmp(&((a.2[3] / 4.0).round() as i64)))
        .then(a.2[0].total_cmp(&b.2[0])));
    out
}

/// The state a checkbox moves to: on (its own on-state, else `Yes`) from off, and back.
pub fn toggled(f: &FieldInfo) -> String {
    let on = matches!(&f.value, vellum_edit::FieldValue::State(s) | vellum_edit::FieldValue::Text(s) if s != "Off");
    if on { "Off".into() } else { f.states.first().cloned().unwrap_or_else(|| "Yes".into()) }
}

/// A Unix time (seconds, UTC) as a PDF date.
pub fn date(unix: u64) -> PdfDate {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + (month <= 2) as i64) as u16;
    PdfDate { year, month, day, hour: (secs / 3600) as u8, minute: (secs / 60 % 60) as u8, second: (secs % 60) as u8, utc_offset_minutes: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_edit::{Editor, FieldValue};

    fn page(rotate: i32) -> PageInfo {
        PageInfo { media_box: [0.0, 0.0, 612.0, 792.0], crop_box: [10.0, 20.0, 610.0, 780.0], rotate }
    }

    #[test]
    fn shown_and_user_space_round_trip() {
        for rotate in [0, 90, 180, 270] {
            let p = page(rotate);
            for pt in [(0.0, 0.0), (100.0, 40.0), (300.5, 700.25)] {
                let back = to_shown(&p, to_user(&p, pt));
                assert!((back.0 - pt.0).abs() < 1e-9 && (back.1 - pt.1).abs() < 1e-9, "{rotate} {pt:?} {back:?}");
            }
        }
        // Unrotated: the shown top left is the crop box's top left.
        assert_eq!(to_user(&page(0), (0.0, 0.0)), (10.0, 780.0));
        assert_eq!(rect_to_user(&page(0), [72.0, 100.0, 172.0, 120.0]), [82.0, 660.0, 182.0, 680.0]);
        // A quarter turn: the shown top left is the crop box's bottom left.
        assert_eq!(to_user(&page(90), (0.0, 0.0)), (10.0, 20.0));
        assert_eq!(rect_to_shown(&page(0), [82.0, 660.0, 182.0, 680.0]), [72.0, 100.0, 172.0, 120.0]);
    }

    #[test]
    fn character_boxes_become_line_rectangles() {
        let boxes = [[10.0, 100.0, 16.0, 114.0], [16.0, 101.0, 22.0, 114.0], [22.0, 100.0, 30.0, 115.0], [5.0, 120.0, 11.0, 134.0], [11.0, 120.0, 17.0, 134.0]];
        assert_eq!(line_rects(&boxes), vec![[10.0, 100.0, 30.0, 115.0], [5.0, 120.0, 17.0, 134.0]]);
        assert!(line_rects(&[]).is_empty());
    }

    #[test]
    fn dates() {
        let d = date(0);
        assert_eq!((d.year, d.month, d.day, d.hour), (1970, 1, 1, 0));
        // 2026-11-14 09:30:05 UTC.
        let d = date(1_794_648_605);
        assert_eq!((d.year, d.month, d.day, d.hour, d.minute, d.second), (2026, 11, 14, 9, 30, 5));
        let d = date(951_782_400); // 2000-02-29
        assert_eq!((d.year, d.month, d.day), (2000, 2, 29));
    }

    /// A one-page form: two text fields on one row, a checkbox below, and a read-only field.
    pub fn form() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R 6 0 R 7 0 R] /DA (/Helv 0 Tf 0 g) >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [5 0 R 4 0 R 6 0 R 7 0 R] >>",
            "<< /Type /Annot /Subtype /Widget /T (last) /FT /Tx /DA (/Helv 12 Tf 0 g) /Rect [160 250 280 270] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Widget /T (first) /FT /Tx /DA (/Helv 12 Tf 0 g) /Rect [20 252 140 272] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Widget /T (agree) /FT /Btn /V /Off /AS /Off /Rect [20 200 40 220] /P 3 0 R >>",
            "<< /Type /Annot /Subtype /Widget /T (id) /FT /Tx /Ff 1 /V (7) /Rect [20 150 80 170] /P 3 0 R >>",
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut at = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            at.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for a in at { out.extend_from_slice(format!("{a:010} 00000 n \n").as_bytes()); }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    /// What the viewer does for Ctrl+F PARAKEET, Ctrl+H, Ctrl+N, Ctrl+E and Ctrl+S, without
    /// the window: the word's boxes become a highlight, a note goes beside it, both
    /// fields are filled, and the saved bytes reopen with all of it.
    #[test]
    fn highlight_note_and_fields_survive_a_save() {
        use hayro::hayro_syntax::Pdf;
        use vellum_edit::{AnnotKind, AnnotStyle, NewAnnot, NoteIcon};
        const FORM: &[u8] = include_bytes!("../../../tests/fixtures/pdf/form1.pdf");
        let pdf = Pdf::new(FORM.to_vec()).unwrap();
        let text = crate::text::page_text(&pdf.pages()[0]);
        let chars: Vec<Vec<char>> = vec![text.chars.clone()];
        let hit = crate::search::hits(&chars, "parakeet")[0];
        let boxes: Vec<[f64; 4]> = text.boxes[hit.at..hit.at + 8].iter().map(|b| [b.x0, b.y0, b.x1, b.y1]).collect();
        let lines = line_rects(&boxes);
        assert_eq!(lines.len(), 1);
        let mut ed = Editor::open(FORM.to_vec()).unwrap();
        let info = ed.page_info(0).unwrap();
        let rect = rect_to_user(&info, lines[0]);
        // The word sits on the 700 pt baseline, right of 72 pt.
        assert!(rect[0] > 72.0 && rect[2] < 560.0 && rect[1] < 700.0 && rect[3] > 700.0, "{rect:?}");
        let style = AnnotStyle { date: Some(date(1_794_648_605)), ..AnnotStyle::default() };
        ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Highlight { rects: vec![rect] }, style: style.clone() }).unwrap();
        let at = to_user(&info, (lines[0][2] + 2.0, lines[0][1] - 4.0));
        ed.add_annotation(0, &NewAnnot { kind: AnnotKind::Note { at, icon: NoteIcon::Comment }, style: AnnotStyle { contents: Some("check this".into()), ..style } }).unwrap();
        let fields = fillable(&ed.fields().unwrap());
        assert_eq!(fields.iter().map(|f| f.0.name.as_str()).collect::<Vec<_>>(), ["name", "agree"]);
        ed.set_field("name", FieldValue::Text("Ada Lovelace".into())).unwrap();
        ed.set_field("agree", FieldValue::State(toggled(&fields[1].0))).unwrap();
        let saved = ed.save_incremental().unwrap();
        assert!(saved.starts_with(FORM));
        let again = Editor::open(saved.clone()).unwrap();
        let kinds: Vec<String> = again.annotations(0).unwrap().into_iter().map(|a| a.subtype).collect();
        assert_eq!(kinds.iter().filter(|k| *k == "Highlight").count(), 1, "{kinds:?}");
        assert_eq!(kinds.iter().filter(|k| *k == "Text").count(), 1, "{kinds:?}");
        let values: Vec<FieldValue> = again.fields().unwrap().into_iter().map(|f| f.value).collect();
        assert_eq!(values, [FieldValue::Text("Ada Lovelace".into()), FieldValue::State("Yes".into())]);
        // hayro draws the saved file: the highlight turns the paper under the word yellow.
        let pdf = Pdf::new(saved).unwrap();
        let pages = pdf.pages();
        let cache = hayro::RenderCache::new();
        let pm = hayro::render(&pages[0], &cache, &Default::default(), &Default::default(),
            &hayro::PixmapSettings { x_scale: 1.0, y_scale: 1.0, bg_color: hayro::vello_cpu::color::palette::css::WHITE });
        let w = pm.width() as usize;
        let d = pm.data_as_u8_slice();
        let r = lines[0].map(|v| v as usize);
        let yellow = (r[1]..r[3]).flat_map(|y| (r[0]..r[2]).map(move |x| (x, y)))
            .filter(|(x, y)| { let p = &d[(y * w + x) * 4..(y * w + x) * 4 + 3]; p[0] > 200 && p[1] > 180 && p[2] < 100 }).count();
        let area = (r[2] - r[0]) * (r[3] - r[1]);
        assert!(yellow * 3 > area, "{yellow} yellow of {area}");
    }

    #[test]
    fn fields_in_reading_order_and_checkbox_toggles() {
        let mut ed = Editor::open(form()).unwrap();
        let fields = fillable(&ed.fields().unwrap());
        let names: Vec<&str> = fields.iter().map(|f| f.0.name.as_str()).collect();
        assert_eq!(names, ["first", "last", "agree"]);
        assert_eq!(toggled(&fields[2].0), "Yes");
        ed.set_field("agree", FieldValue::State("Yes".into())).unwrap();
        ed.set_field("first", FieldValue::Text("Ada".into())).unwrap();
        let again = fillable(&ed.fields().unwrap());
        assert_eq!(toggled(&again[2].0), "Off");
        assert_eq!(again[0].0.value, FieldValue::Text("Ada".into()));
        let saved = ed.save_incremental().unwrap();
        assert!(saved.starts_with(&form()));
    }
}
