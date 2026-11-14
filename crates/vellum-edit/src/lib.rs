//! Vellum's editing engine: annotations, AcroForm filling and signature inspection, saved as an
//! incremental update so the original bytes (and any signatures over them) stay intact.
//!
//! Pure bytes in, bytes out: [`Editor::open`] takes the file's bytes, edits accumulate in memory,
//! and [`Editor::save_incremental`] returns the original bytes followed by one appended update
//! section (changed and new objects, a cross-reference section in the same style as the file's
//! newest one, and a trailer whose `/Prev` points at the previous section). No threads, no
//! filesystem, no clock: dates are passed in as [`PdfDate`].
//!
//! Coordinates are PDF user space of the page (points, origin bottom-left, before `/Rotate`).
//! Encrypted files are refused ([`Error::Encrypted`]).

mod annot;
pub mod cos;
mod doc;
mod filters;
mod form;
mod parse;
mod text;
mod write;

use std::collections::{BTreeMap, HashSet};

pub use annot::{AnnotInfo, AnnotKind, AnnotStyle, NewAnnot, NoteIcon};
pub use cos::{Dict, ObjId, Object, Stream};
pub use doc::XrefStyle;
pub use form::{FieldInfo, FieldKind, FieldValue, SignatureInfo};
pub use text::PdfDate;

/// What went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No `%PDF-` header.
    NotPdf,
    /// The file is encrypted; this engine does not decrypt.
    Encrypted,
    /// The structure is broken beyond repair.
    Malformed(&'static str),
    /// A valid feature this engine does not handle.
    Unsupported(&'static str),
    /// A page index past the end.
    NoSuchPage(usize),
    /// No annotation at that index on the page.
    NoSuchAnnotation(usize),
    /// No form field with that fully qualified name.
    NoSuchField(String),
    /// The field is read-only (`/Ff` bit 1) or a signature field.
    ReadOnly(String),
    /// The value does not suit the field (wrong kind, unknown option or state).
    BadValue(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotPdf => write!(f, "not a PDF file"),
            Error::Encrypted => write!(f, "the PDF is encrypted"),
            Error::Malformed(m) => write!(f, "malformed PDF: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::NoSuchPage(p) => write!(f, "no page {p}"),
            Error::NoSuchAnnotation(i) => write!(f, "no annotation {i}"),
            Error::NoSuchField(n) => write!(f, "no field {n}"),
            Error::ReadOnly(n) => write!(f, "field {n} is read-only"),
            Error::BadValue(m) => write!(f, "bad value: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// A page's boxes, for mapping screen positions to user space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageInfo {
    pub media_box: [f64; 4],
    /// The visible region (CropBox, defaulting to MediaBox).
    pub crop_box: [f64; 4],
    /// Clockwise rotation in degrees: 0, 90, 180 or 270.
    pub rotate: i32,
}

/// An open document with pending edits.
pub struct Editor {
    doc: doc::Document,
    changes: BTreeMap<u32, (u16, Object)>,
    next_num: u32,
    helv: Option<ObjId>,
    pages: Vec<ObjId>,
}

impl Editor {
    /// Parses the file. The bytes are kept and become the prefix of every incremental save.
    pub fn open(bytes: Vec<u8>) -> Result<Editor, Error> {
        let doc = doc::Document::load(bytes)?;
        let next_num = doc.size.max(1);
        let mut ed = Editor { doc, changes: BTreeMap::new(), next_num, helv: None, pages: Vec::new() };
        ed.pages = ed.collect_pages()?;
        Ok(ed)
    }

    /// The original file bytes.
    pub fn original(&self) -> &[u8] { &self.doc.data }

    /// The style of the newest cross-reference section, which a save matches.
    pub fn xref_style(&self) -> XrefStyle { self.doc.style }

    /// Whether there are unsaved edits.
    pub fn is_modified(&self) -> bool { !self.changes.is_empty() }

    pub fn page_count(&self) -> usize { self.pages.len() }

    /// The page's boxes and rotation.
    pub fn page_info(&self, page: usize) -> Result<PageInfo, Error> {
        let id = self.page_id(page)?;
        let rect = |k: &[u8]| self.inherited(id, k).and_then(|o| rect_of(&self.resolve(&o)));
        let media_box = rect(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let crop_box = rect(b"CropBox").unwrap_or(media_box);
        let rotate = self.inherited(id, b"Rotate").and_then(|o| self.resolve(&o).as_int()).unwrap_or(0);
        Ok(PageInfo { media_box, crop_box, rotate: (rotate.rem_euclid(360) / 90 * 90) as i32 })
    }

    /// Returns the bytes of the original file plus one incremental update with every edit.
    /// Without edits it returns the original bytes unchanged.
    pub fn save_incremental(&self) -> Result<Vec<u8>, Error> {
        write::incremental(&self.doc, &self.changes, self.next_num)
    }

    /// Makes the saved result the new base, so later edits append another update.
    pub fn commit(&mut self) -> Result<(), Error> {
        let bytes = self.save_incremental()?;
        *self = Editor::open(bytes)?;
        Ok(())
    }

    // ---- object access (crate-internal) ----

    pub(crate) fn get(&self, id: ObjId) -> Option<Object> {
        match self.changes.get(&id.0) {
            Some((_, o)) => Some(o.clone()),
            None => self.doc.get(id),
        }
    }

    /// Follows references (bounded).
    pub(crate) fn resolve(&self, o: &Object) -> Object {
        let mut cur = o.clone();
        for _ in 0..32 {
            match cur {
                Object::Ref(r) => cur = self.get(r).unwrap_or(Object::Null),
                _ => return cur,
            }
        }
        Object::Null
    }

    pub(crate) fn dict_of(&self, o: &Object) -> Option<Dict> {
        self.resolve(o).as_dict().cloned()
    }

    pub(crate) fn get_dict(&self, id: ObjId) -> Option<Dict> {
        self.get(id).and_then(|o| o.as_dict().cloned())
    }

    /// Looks a key up in a dict and resolves it.
    pub(crate) fn lookup(&self, d: &Dict, key: &[u8]) -> Option<Object> {
        d.get(key).map(|o| self.resolve(o)).filter(|o| *o != Object::Null)
    }

    /// Adds a new object; returns its id.
    pub(crate) fn add(&mut self, obj: impl Into<Object>) -> ObjId {
        let id = ObjId(self.next_num, 0);
        self.next_num += 1;
        self.changes.insert(id.0, (0, obj.into()));
        id
    }

    /// Replaces an object (existing objects keep their generation).
    pub(crate) fn set(&mut self, id: ObjId, obj: impl Into<Object>) {
        let gen = self.changes.get(&id.0).map(|c| c.0).unwrap_or_else(|| self.doc.generation(id.0));
        self.changes.insert(id.0, (gen, obj.into()));
    }

    /// Replaces the dictionary part of an object, keeping stream data if it is a stream.
    pub(crate) fn set_dict(&mut self, id: ObjId, d: Dict) {
        let new = match self.get(id) {
            Some(Object::Stream(s)) => Object::Stream(Stream { dict: d, data: s.data }),
            _ => Object::Dict(d),
        };
        self.set(id, new);
    }

    pub(crate) fn catalog(&self) -> Result<(ObjId, Dict), Error> {
        let id = self.doc.trailer.get(b"Root").and_then(Object::as_ref).ok_or(Error::Malformed("no /Root"))?;
        Ok((id, self.get_dict(id).ok_or(Error::Malformed("bad catalog"))?))
    }

    pub(crate) fn page_id(&self, page: usize) -> Result<ObjId, Error> {
        self.pages.get(page).copied().ok_or(Error::NoSuchPage(page))
    }

    pub(crate) fn page_index(&self, id: ObjId) -> Option<usize> {
        self.pages.iter().position(|p| p.0 == id.0)
    }

    /// An inheritable page attribute (7.7.3.4).
    pub(crate) fn inherited(&self, page: ObjId, key: &[u8]) -> Option<Object> {
        let mut node = self.get_dict(page)?;
        for _ in 0..64 {
            if let Some(v) = node.get(key) { return Some(v.clone()); }
            node = self.dict_of(node.get(b"Parent")?)?;
        }
        None
    }

    fn collect_pages(&self) -> Result<Vec<ObjId>, Error> {
        let (_, cat) = self.catalog()?;
        let root = cat.get(b"Pages").and_then(Object::as_ref).ok_or(Error::Malformed("no /Pages"))?;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id.0) { continue; }
            let Some(d) = self.get_dict(id) else { continue };
            let is_tree = d.get(b"Type").and_then(Object::as_name) == Some(b"Pages") || d.get(b"Kids").is_some();
            if is_tree {
                if let Some(kids) = self.lookup(&d, b"Kids") {
                    let refs: Vec<ObjId> = kids.as_array().map(|a| a.iter().filter_map(Object::as_ref).collect()).unwrap_or_default();
                    stack.extend(refs.into_iter().rev());
                }
            } else {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// A shared simple Helvetica font object (WinAnsi), created on first use.
    pub(crate) fn helvetica(&mut self) -> ObjId {
        if let Some(h) = self.helv { return h; }
        let d = Dict::new()
            .with(b"Type", Object::name("Font"))
            .with(b"Subtype", Object::name("Type1"))
            .with(b"BaseFont", Object::name("Helvetica"))
            .with(b"Encoding", Object::name("WinAnsiEncoding"));
        let id = self.add(d);
        self.helv = Some(id);
        id
    }
}

pub(crate) fn rect_of(o: &Object) -> Option<[f64; 4]> {
    let a = o.as_array()?;
    if a.len() < 4 { return None; }
    let v: Vec<f64> = a.iter().take(4).map(|x| x.as_f64().unwrap_or(0.0)).collect();
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

pub(crate) fn rect_obj(r: [f64; 4]) -> Object {
    Object::Array(r.iter().map(|&v| Object::Real(v)).collect())
}
