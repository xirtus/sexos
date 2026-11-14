//! Markup annotations (12.5.6): highlight, underline, strikeout, squiggly, sticky note, free
//! text and ink, each with a generated normal appearance stream (`/AP /N`).

use crate::cos::{fmt_num as n, Dict, ObjId, Object, Stream};
use crate::text::{content_string, decode_text, encode_text, wrap, PdfDate};
use crate::{rect_obj, rect_of, Editor, Error};

/// Colour, opacity and the markup metadata shared by every kind.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotStyle {
    /// RGB, 0..=1.
    pub color: [f32; 3],
    /// 0..=1 (`/CA`).
    pub opacity: f32,
    /// `/T`: the author shown by viewers.
    pub author: Option<String>,
    /// `/Contents`: the comment text (for free text, the text itself when `None`).
    pub contents: Option<String>,
    /// `/CreationDate` and `/M`.
    pub date: Option<PdfDate>,
}

impl Default for AnnotStyle {
    fn default() -> Self {
        AnnotStyle { color: [1.0, 0.92, 0.0], opacity: 1.0, author: None, contents: None, date: None }
    }
}

/// The icon of a sticky note (`/Name`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoteIcon {
    #[default]
    Comment,
    Note,
    Help,
    Insert,
    Key,
    NewParagraph,
    Paragraph,
}

impl NoteIcon {
    fn name(self) -> &'static str {
        match self {
            NoteIcon::Comment => "Comment",
            NoteIcon::Note => "Note",
            NoteIcon::Help => "Help",
            NoteIcon::Insert => "Insert",
            NoteIcon::Key => "Key",
            NoteIcon::NewParagraph => "NewParagraph",
            NoteIcon::Paragraph => "Paragraph",
        }
    }
}

/// What to draw. Rectangles are `[x0, y0, x1, y1]` in page user space.
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotKind {
    /// Text highlight over each rectangle (one per text line), drawn with Multiply blending.
    Highlight { rects: Vec<[f64; 4]> },
    Underline { rects: Vec<[f64; 4]> },
    StrikeOut { rects: Vec<[f64; 4]> },
    Squiggly { rects: Vec<[f64; 4]> },
    /// A sticky note (`/Text`): a 20x20 icon whose top-left corner is `at`.
    Note { at: (f64, f64), icon: NoteIcon },
    /// Text set directly on the page in Helvetica, wrapped to `rect`.
    FreeText { rect: [f64; 4], text: String, font_size: f64 },
    /// Freehand strokes; each stroke is a polyline of points.
    Ink { strokes: Vec<Vec<(f64, f64)>>, width: f64 },
}

/// An annotation to add.
#[derive(Clone, Debug, PartialEq)]
pub struct NewAnnot {
    pub kind: AnnotKind,
    pub style: AnnotStyle,
}

/// An existing annotation, as listed by [`Editor::annotations`].
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotInfo {
    /// Position in the page's `/Annots` array (what [`Editor::delete_annotation`] takes).
    pub index: usize,
    /// The object, when the annotation is an indirect object (it nearly always is).
    pub id: Option<ObjId>,
    /// `/Subtype`, e.g. `Highlight`, `Text`, `Widget`, `Link`, `Popup`.
    pub subtype: String,
    pub rect: [f64; 4],
    pub contents: Option<String>,
    pub author: Option<String>,
    pub color: Option<[f32; 3]>,
    /// `/M`, raw.
    pub modified: Option<String>,
    /// `/CreationDate`, raw.
    pub created: Option<String>,
    /// `/NM`.
    pub name: Option<String>,
    /// `/QuadPoints` (8 numbers per quad) for text markup.
    pub quad_points: Vec<f64>,
    /// `/InkList` for ink.
    pub ink: Vec<Vec<(f64, f64)>>,
    pub has_appearance: bool,
    /// Hidden flag (`/F` bit 2) set.
    pub hidden: bool,
}

const MARKUP_FLAGS: i64 = 4; // Print
const NOTE_FLAGS: i64 = 4 | 8 | 16; // Print, NoZoom, NoRotate

impl Editor {
    /// Lists every annotation on the page, in `/Annots` order (including links, popups, widgets).
    pub fn annotations(&self, page: usize) -> Result<Vec<AnnotInfo>, Error> {
        let pid = self.page_id(page)?;
        let (arr, _) = self.annots_array(pid);
        let mut out = Vec::new();
        for (index, entry) in arr.iter().enumerate() {
            let Some(d) = self.dict_of(entry) else { continue };
            let s = |k: &[u8]| self.lookup(&d, k).and_then(|o| o.as_str().map(decode_text));
            let nums = |k: &[u8]| -> Vec<f64> { self.lookup(&d, k).and_then(|o| o.as_array().map(|a| a.iter().filter_map(|x| self.resolve(x).as_f64()).collect())).unwrap_or_default() };
            let color = self.lookup(&d, b"C").and_then(|o| {
                let a: Vec<f32> = o.as_array()?.iter().filter_map(|x| x.as_f64()).map(|v| v as f32).collect();
                match a.len() {
                    1 => Some([a[0]; 3]),
                    3 => Some([a[0], a[1], a[2]]),
                    4 => Some([(1.0 - a[0]) * (1.0 - a[3]), (1.0 - a[1]) * (1.0 - a[3]), (1.0 - a[2]) * (1.0 - a[3])]),
                    _ => None,
                }
            });
            let ink = self.lookup(&d, b"InkList").and_then(|o| o.as_array().cloned()).unwrap_or_default().iter().map(|stroke| {
                let v: Vec<f64> = self.resolve(stroke).as_array().map(|a| a.iter().filter_map(Object::as_f64).collect()).unwrap_or_default();
                v.chunks_exact(2).map(|p| (p[0], p[1])).collect()
            }).collect();
            out.push(AnnotInfo {
                index,
                id: entry.as_ref(),
                subtype: self.lookup(&d, b"Subtype").and_then(|o| o.as_name().map(|n| String::from_utf8_lossy(n).into_owned())).unwrap_or_default(),
                rect: self.lookup(&d, b"Rect").and_then(|o| rect_of(&o)).unwrap_or_default(),
                contents: s(b"Contents"),
                author: s(b"T"),
                color,
                modified: s(b"M"),
                created: s(b"CreationDate"),
                name: s(b"NM"),
                quad_points: nums(b"QuadPoints"),
                ink,
                has_appearance: self.lookup(&d, b"AP").is_some(),
                hidden: self.lookup(&d, b"F").and_then(|o| o.as_int()).unwrap_or(0) & 2 != 0,
            });
        }
        Ok(out)
    }

    /// Adds an annotation with its appearance stream; returns the new annotation's object id.
    pub fn add_annotation(&mut self, page: usize, a: &NewAnnot) -> Result<ObjId, Error> {
        let pid = self.page_id(page)?;
        let st = &a.style;
        let [r, g, b] = st.color.map(|c| c.clamp(0.0, 1.0) as f64);
        let op = st.opacity.clamp(0.0, 1.0) as f64;
        let color_arr = Object::Array(vec![Object::Real(r), Object::Real(g), Object::Real(b)]);
        let mut d = Dict::new().with(b"Type", Object::name("Annot"));
        let mut res = Dict::new();
        let mut gs = Dict::new().with(b"Type", Object::name("ExtGState")).with(b"CA", op).with(b"ca", op);
        let mut ops = String::new();
        let rect: [f64; 4];
        let mut flags = MARKUP_FLAGS;

        match &a.kind {
            AnnotKind::Highlight { rects } | AnnotKind::Underline { rects } | AnnotKind::StrikeOut { rects } | AnnotKind::Squiggly { rects } => {
                let rects: Vec<[f64; 4]> = rects.iter().map(|q| norm(*q)).filter(|q| q[2] > q[0] && q[3] > q[1]).collect();
                if rects.is_empty() { return Err(Error::BadValue("no rectangles".into())); }
                let (sub, kind) = match &a.kind {
                    AnnotKind::Highlight { .. } => ("Highlight", 0),
                    AnnotKind::Underline { .. } => ("Underline", 1),
                    AnnotKind::StrikeOut { .. } => ("StrikeOut", 2),
                    _ => ("Squiggly", 3),
                };
                d.set(b"Subtype", Object::name(sub));
                let mut quads = Vec::new();
                for q in &rects {
                    // Acrobat's order: upper-left, upper-right, lower-left, lower-right.
                    for v in [q[0], q[3], q[2], q[3], q[0], q[1], q[2], q[1]] { quads.push(Object::Real(v)); }
                }
                d.set(b"QuadPoints", Object::Array(quads));
                rect = bbox(&rects, 1.0);
                ops.push_str("/GS0 gs\n");
                if kind == 0 {
                    gs.set(b"BM", Object::name("Multiply"));
                    ops.push_str(&format!("{} {} {} rg\n", n(r), n(g), n(b)));
                    for q in &rects { ops.push_str(&format!("{} {} {} {} re f\n", n(q[0]), n(q[1]), n(q[2] - q[0]), n(q[3] - q[1]))); }
                } else {
                    ops.push_str(&format!("{} {} {} RG {} {} {} rg\n", n(r), n(g), n(b), n(r), n(g), n(b)));
                    for q in &rects {
                        let h = q[3] - q[1];
                        let t = (h / 14.0).max(0.6);
                        match kind {
                            1 => ops.push_str(&format!("{} {} {} {} re f\n", n(q[0]), n(q[1] + h * 0.04), n(q[2] - q[0]), n(t))),
                            2 => ops.push_str(&format!("{} {} {} {} re f\n", n(q[0]), n(q[1] + h * 0.42), n(q[2] - q[0]), n(t))),
                            _ => {
                                let (amp, step) = ((h / 16.0).max(0.8), (h / 8.0).max(2.0));
                                let y = q[1] + h * 0.06 + amp;
                                ops.push_str(&format!("{} w {} {} m\n", n(t), n(q[0]), n(y)));
                                let mut x = q[0];
                                let mut up = true;
                                while x < q[2] {
                                    x = (x + step).min(q[2]);
                                    ops.push_str(&format!("{} {} l\n", n(x), n(if up { y + amp } else { y - amp })));
                                    up = !up;
                                }
                                ops.push_str("S\n");
                            }
                        }
                    }
                }
            }
            AnnotKind::Note { at, icon } => {
                flags = NOTE_FLAGS;
                d.set(b"Subtype", Object::name("Text"));
                d.set(b"Name", Object::name(icon.name()));
                d.set(b"Open", false);
                let (x, y) = *at;
                rect = [x, y - 20.0, x + 20.0, y];
                let (x0, y0) = (rect[0], rect[1]);
                ops.push_str(&format!(
                    "/GS0 gs {r} {g} {b} rg 0 G 0.8 w\n{} {} m {} {} l {} {} l {} {} l {} {} l {} {} l {} {} l h B\n",
                    n(x0 + 1.0), n(y0 + 5.0), n(x0 + 5.0), n(y0 + 5.0), n(x0 + 4.0), n(y0 + 1.0), n(x0 + 9.0), n(y0 + 5.0),
                    n(x0 + 19.0), n(y0 + 5.0), n(x0 + 19.0), n(y0 + 19.0), n(x0 + 1.0), n(y0 + 19.0),
                    r = n(r), g = n(g), b = n(b)
                ));
                ops.push_str("0.6 w\n");
                for k in 0..3 {
                    let ly = y0 + 15.5 - 3.5 * k as f64;
                    ops.push_str(&format!("{} {} m {} {} l S\n", n(x0 + 4.0), n(ly), n(x0 + 16.0), n(ly)));
                }
            }
            AnnotKind::FreeText { rect: fr, text, font_size } => {
                let fr = norm(*fr);
                if fr[2] <= fr[0] || fr[3] <= fr[1] { return Err(Error::BadValue("empty rectangle".into())); }
                rect = fr;
                let size = if *font_size > 0.0 { *font_size } else { 12.0 };
                d.set(b"Subtype", Object::name("FreeText"));
                let da = format!("/Helv {} Tf {} {} {} rg", n(size), n(r), n(g), n(b));
                d.set(b"DA", Object::Str(da.clone().into_bytes()));
                d.set(b"Q", Object::Int(0));
                if st.contents.is_none() { d.set(b"Contents", Object::Str(encode_text(text))); }
                let helv = self.helvetica();
                res.set(b"Font", Dict::new().with(b"Helv", helv));
                let (w, h) = (fr[2] - fr[0], fr[3] - fr[1]);
                ops.push_str(&format!("/GS0 gs {} {} {} RG 1 w {} {} {} {} re S\n", n(r), n(g), n(b), n(fr[0] + 0.5), n(fr[1] + 0.5), n(w - 1.0), n(h - 1.0)));
                ops.push_str(&format!("{} {} {} {} re W n\nBT {}\n", n(fr[0] + 1.0), n(fr[1] + 1.0), n(w - 2.0), n(h - 2.0), da));
                let lead = size * 1.2;
                for (i, line) in wrap(text, size, w - 4.0).iter().enumerate() {
                    let y = fr[3] - 2.0 - size * 0.9 - lead * i as f64;
                    if y < fr[1] - size { break; }
                    ops.push_str(&format!("1 0 0 1 {} {} Tm ", n(fr[0] + 2.0), n(y)));
                    ops.push_str(&String::from_utf8_lossy(&content_string(line)));
                    ops.push_str(" Tj\n");
                }
                ops.push_str("ET\n");
            }
            AnnotKind::Ink { strokes, width } => {
                let width = if *width > 0.0 { *width } else { 1.0 };
                let strokes: Vec<&Vec<(f64, f64)>> = strokes.iter().filter(|s| !s.is_empty()).collect();
                if strokes.is_empty() { return Err(Error::BadValue("no strokes".into())); }
                d.set(b"Subtype", Object::name("Ink"));
                d.set(b"InkList", Object::Array(strokes.iter().map(|s| Object::Array(s.iter().flat_map(|p| [Object::Real(p.0), Object::Real(p.1)]).collect())).collect()));
                d.set(b"BS", Dict::new().with(b"W", width));
                let pts: Vec<[f64; 4]> = strokes.iter().flat_map(|s| s.iter().map(|p| [p.0, p.1, p.0, p.1])).collect();
                rect = bbox(&pts, width / 2.0 + 1.0);
                ops.push_str(&format!("/GS0 gs {} {} {} RG {} w 1 J 1 j\n", n(r), n(g), n(b), n(width)));
                for s in strokes {
                    ops.push_str(&format!("{} {} m\n", n(s[0].0), n(s[0].1)));
                    if s.len() == 1 { ops.push_str(&format!("{} {} l\n", n(s[0].0), n(s[0].1))); }
                    for p in &s[1..] { ops.push_str(&format!("{} {} l\n", n(p.0), n(p.1))); }
                    ops.push_str("S\n");
                }
            }
        }

        res.set(b"ExtGState", Dict::new().with(b"GS0", gs));
        let ap = Stream {
            dict: Dict::new()
                .with(b"Type", Object::name("XObject"))
                .with(b"Subtype", Object::name("Form"))
                .with(b"BBox", rect_obj(rect))
                .with(b"Resources", res),
            data: ops.into_bytes(),
        };
        let ap_id = self.add(ap);
        d.set(b"Rect", rect_obj(rect));
        d.set(b"P", pid);
        d.set(b"F", Object::Int(flags));
        d.set(b"C", color_arr);
        if op < 1.0 { d.set(b"CA", op); }
        if let Some(t) = &st.author { d.set(b"T", Object::Str(encode_text(t))); }
        if let Some(c) = &st.contents { d.set(b"Contents", Object::Str(encode_text(c))); }
        if let Some(dt) = &st.date {
            let s = Object::Str(dt.to_pdf_string().into_bytes());
            d.set(b"M", s.clone());
            d.set(b"CreationDate", s);
        }
        d.set(b"AP", Dict::new().with(b"N", ap_id));
        let id = self.add(Object::Null);
        d.set(b"NM", Object::Str(format!("vellum-{}-{}", self.original().len(), id.0).into_bytes()));
        self.set(id, d);
        self.push_annot(pid, Object::Ref(id));
        Ok(id)
    }

    /// Removes the annotation at `index` of the page's `/Annots` (and its popup).
    /// Form widgets are refused: change them through fields.
    pub fn delete_annotation(&mut self, page: usize, index: usize) -> Result<(), Error> {
        let pid = self.page_id(page)?;
        let (mut arr, holder) = self.annots_array(pid);
        let entry = arr.get(index).cloned().ok_or(Error::NoSuchAnnotation(index))?;
        let d = self.dict_of(&entry).unwrap_or_default();
        if d.get(b"Subtype").and_then(Object::as_name) == Some(b"Widget") {
            return Err(Error::Unsupported("deleting form widgets"));
        }
        let popup = d.get(b"Popup").and_then(Object::as_ref);
        arr.remove(index);
        if let Some(p) = popup {
            arr.retain(|o| o.as_ref().map(|r| r.0) != Some(p.0));
        }
        self.store_annots(pid, holder, arr);
        Ok(())
    }

    /// The page's annotation array and, if it is an indirect object, its id.
    fn annots_array(&self, pid: ObjId) -> (Vec<Object>, Option<ObjId>) {
        let page = self.get_dict(pid).unwrap_or_default();
        match page.get(b"Annots") {
            Some(Object::Ref(r)) => (self.get(*r).and_then(|o| o.as_array().cloned()).unwrap_or_default(), Some(*r)),
            Some(Object::Array(a)) => (a.clone(), None),
            _ => (Vec::new(), None),
        }
    }

    fn store_annots(&mut self, pid: ObjId, holder: Option<ObjId>, arr: Vec<Object>) {
        match holder {
            Some(h) => self.set(h, Object::Array(arr)),
            None => {
                let mut page = self.get_dict(pid).unwrap_or_default();
                page.set(b"Annots", Object::Array(arr));
                self.set_dict(pid, page);
            }
        }
    }

    pub(crate) fn push_annot(&mut self, pid: ObjId, annot: Object) {
        let (mut arr, holder) = self.annots_array(pid);
        arr.push(annot);
        self.store_annots(pid, holder, arr);
    }
}

fn norm(r: [f64; 4]) -> [f64; 4] {
    [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]
}

fn bbox(rects: &[[f64; 4]], pad: f64) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for r in rects {
        b[0] = b[0].min(r[0]);
        b[1] = b[1].min(r[1]);
        b[2] = b[2].max(r[2]);
        b[3] = b[3].max(r[3]);
    }
    [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad]
}
