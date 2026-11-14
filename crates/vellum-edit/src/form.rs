//! Interactive forms (12.7): listing fields, setting values with regenerated appearance
//! streams, and a read-only view of signature fields.

use crate::cos::{fmt_num as n, Dict, ObjId, Object, Stream};
use crate::text::{content_string, decode_text, encode_text, text_width, wrap};
use crate::{rect_of, Editor, Error};

/// The kind of a terminal field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text { multiline: bool, password: bool, comb: bool, max_len: Option<usize> },
    Checkbox,
    Radio,
    PushButton,
    ComboBox { editable: bool },
    ListBox { multi_select: bool },
    Signature,
    Unknown,
}

/// A field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    /// No `/V`.
    Empty,
    /// Text fields, and single selections of choice fields.
    Text(String),
    /// Checkbox and radio states: an on-state name such as `Yes`, or `Off`.
    State(String),
    /// Multiple selections of a list box.
    Choices(Vec<String>),
}

/// A terminal form field.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldInfo {
    /// Fully qualified name (`parent.child`).
    pub name: String,
    pub id: ObjId,
    pub kind: FieldKind,
    pub value: FieldValue,
    /// Choice options as (export value, display text).
    pub options: Vec<(String, String)>,
    /// Checkbox/radio on-states found in the widgets' appearances (`Off` excluded).
    pub states: Vec<String>,
    pub read_only: bool,
    pub required: bool,
    /// Widget annotations: (object, page index if known, rectangle).
    pub widgets: Vec<(ObjId, Option<usize>, [f64; 4])>,
}

/// A signature field, as stored. **Nothing here is cryptographically verified**: `signed` only
/// means a signature dictionary is present; the PKCS#7/CMS blob in `/Contents` is not checked.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureInfo {
    pub field_name: String,
    pub signed: bool,
    /// `/Name` of the signature dictionary (the signer as written by the signing tool).
    pub signer_name: Option<String>,
    /// `/M`, raw (`D:...`).
    pub date: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub sub_filter: Option<String>,
    /// `/ByteRange` pairs (offset, length).
    pub byte_range: Vec<(u64, u64)>,
    /// Whether the byte range ends exactly at the end of the original file we opened, i.e. the
    /// signature covered the whole file as it was before this session's edits.
    pub covers_whole_file: bool,
    /// DocMDP permission level (1 = no changes allowed, 2 = form filling and signing,
    /// 3 = also annotations), when the signature is a certification signature.
    pub mdp_permissions: Option<i64>,
}

#[derive(Clone, Default)]
struct Inherit {
    ft: Option<Vec<u8>>,
    ff: i64,
    v: Option<Object>,
    da: Option<Vec<u8>>,
    q: i64,
    opt: Option<Object>,
    max_len: Option<i64>,
}

struct Node {
    id: ObjId,
    name: String,
    inh: Inherit,
    widgets: Vec<ObjId>,
}

const FF_READONLY: i64 = 1;
const FF_REQUIRED: i64 = 2;
const FF_MULTILINE: i64 = 1 << 12;
const FF_PASSWORD: i64 = 1 << 13;
const FF_RADIO: i64 = 1 << 15;
const FF_PUSHBUTTON: i64 = 1 << 16;
const FF_COMBO: i64 = 1 << 17;
const FF_EDIT: i64 = 1 << 18;
const FF_MULTISELECT: i64 = 1 << 21;
const FF_COMB: i64 = 1 << 24;

impl Editor {
    /// Every terminal field of the AcroForm, in tree order.
    pub fn fields(&self) -> Result<Vec<FieldInfo>, Error> {
        Ok(self.field_nodes().iter().map(|n| self.field_info(n)).collect())
    }

    /// Signature fields with their stored signer data (not cryptographically verified).
    pub fn signatures(&self) -> Result<Vec<SignatureInfo>, Error> {
        let mut out = Vec::new();
        for node in self.field_nodes() {
            if node.inh.ft.as_deref() != Some(b"Sig") { continue; }
            let v = node.inh.v.as_ref().and_then(|v| self.dict_of(v));
            let s = |k: &[u8]| v.as_ref().and_then(|d| self.lookup(d, k)).and_then(|o| o.as_str().map(decode_text));
            let byte_range: Vec<(u64, u64)> = v.as_ref().and_then(|d| self.lookup(d, b"ByteRange")).and_then(|o| o.as_array().cloned())
                .map(|a| a.iter().filter_map(Object::as_int).collect::<Vec<_>>().chunks_exact(2).map(|p| (p[0].max(0) as u64, p[1].max(0) as u64)).collect())
                .unwrap_or_default();
            let end = byte_range.last().map(|(o, l)| o + l);
            let mdp = v.as_ref().and_then(|d| self.lookup(d, b"Reference")).and_then(|r| r.as_array().cloned()).and_then(|refs| {
                refs.iter().filter_map(|r| self.dict_of(r)).find_map(|r| {
                    if self.lookup(&r, b"TransformMethod")?.as_name()? != b"DocMDP" { return None; }
                    Some(self.lookup(&r, b"TransformParams").and_then(|p| p.as_dict().and_then(|p| self.lookup(p, b"P")?.as_int())).unwrap_or(2))
                })
            });
            out.push(SignatureInfo {
                field_name: node.name.clone(),
                signed: v.is_some(),
                signer_name: s(b"Name"),
                date: s(b"M"),
                reason: s(b"Reason"),
                location: s(b"Location"),
                sub_filter: v.as_ref().and_then(|d| self.lookup(d, b"SubFilter")).and_then(|o| o.as_name().map(|n| String::from_utf8_lossy(n).into_owned())),
                covers_whole_file: end == Some(self.original().len() as u64)
                    || end.is_some_and(|e| self.original().get(e as usize..).is_some_and(|rest| rest.iter().all(|b| crate::parse::is_white(*b)))),
                byte_range,
                mdp_permissions: mdp,
            });
        }
        Ok(out)
    }

    /// Sets a field's value and regenerates its widgets' appearances.
    pub fn set_field(&mut self, name: &str, value: FieldValue) -> Result<(), Error> {
        let nodes = self.field_nodes();
        let node = nodes.iter().find(|n| n.name == name).ok_or_else(|| Error::NoSuchField(name.into()))?;
        let info = self.field_info(node);
        if info.read_only || info.kind == FieldKind::Signature {
            return Err(Error::ReadOnly(name.into()));
        }
        let mut field = self.get_dict(node.id).ok_or(Error::Malformed("field vanished"))?;
        let helv = self.form_font();
        match &info.kind {
            FieldKind::Text { max_len, .. } => {
                let s = match value {
                    FieldValue::Text(s) => s,
                    FieldValue::Empty => String::new(),
                    _ => return Err(Error::BadValue("text field takes text".into())),
                };
                if let Some(m) = max_len {
                    if s.chars().count() > *m { return Err(Error::BadValue(format!("longer than MaxLen {m}"))); }
                }
                field.set(b"V", Object::Str(encode_text(&s)));
                self.set_dict(node.id, field);
                for w in &node.widgets {
                    self.text_appearance(*w, node, &info.kind, &[s.clone()], helv);
                }
            }
            FieldKind::Checkbox | FieldKind::Radio => {
                let s = match value {
                    FieldValue::State(s) | FieldValue::Text(s) => s,
                    FieldValue::Empty => "Off".into(),
                    _ => return Err(Error::BadValue("checkbox takes a state".into())),
                };
                if s != "Off" && !info.states.is_empty() && !info.states.contains(&s) {
                    return Err(Error::BadValue(format!("{name} has no state {s}")));
                }
                field.set(b"V", Object::Name(s.clone().into_bytes()));
                self.set_dict(node.id, field);
                for w in &node.widgets {
                    let mut wd = self.get_dict(*w).unwrap_or_default();
                    let mut states = self.widget_states(&wd);
                    if states.is_empty() {
                        // No appearances to switch between: draw them.
                        let on = if s == "Off" { "Yes".to_string() } else { s.clone() };
                        self.check_appearance(*w, &on, info.kind == FieldKind::Radio);
                        wd = self.get_dict(*w).unwrap_or_default();
                        states = vec![on];
                    }
                    let as_ = if states.contains(&s) { s.clone() } else { "Off".into() };
                    wd.set(b"AS", Object::Name(as_.into_bytes()));
                    self.set_dict(*w, wd);
                }
            }
            FieldKind::ComboBox { .. } | FieldKind::ListBox { .. } => {
                let editable = matches!(info.kind, FieldKind::ComboBox { editable: true });
                let multi = matches!(info.kind, FieldKind::ListBox { multi_select: true });
                let picks = match value {
                    FieldValue::Text(s) => vec![s],
                    FieldValue::Choices(v) => v,
                    FieldValue::Empty => vec![],
                    FieldValue::State(s) => vec![s],
                };
                if picks.len() > 1 && !multi { return Err(Error::BadValue("one choice only".into())); }
                let mut indices = Vec::new();
                for p in &picks {
                    match info.options.iter().position(|(e, d)| e == p || d == p) {
                        Some(i) => indices.push(Object::Int(i as i64)),
                        None if editable => {}
                        None => return Err(Error::BadValue(format!("{name} has no option {p}"))),
                    }
                }
                // Store export values.
                let exports: Vec<String> = picks.iter().map(|p| info.options.iter().find(|(e, d)| e == p || d == p).map(|(e, _)| e.clone()).unwrap_or_else(|| p.clone())).collect();
                match exports.len() {
                    0 => { field.remove(b"V"); }
                    1 => field.set(b"V", Object::Str(encode_text(&exports[0]))),
                    _ => field.set(b"V", Object::Array(exports.iter().map(|e| Object::Str(encode_text(e))).collect())),
                }
                if indices.is_empty() { field.remove(b"I"); } else { field.set(b"I", Object::Array(indices)); }
                self.set_dict(node.id, field);
                let shown: Vec<String> = exports.iter().map(|e| info.options.iter().find(|(x, _)| x == e).map(|(_, d)| d.clone()).unwrap_or_else(|| e.clone())).collect();
                for w in &node.widgets {
                    if let FieldKind::ListBox { .. } = info.kind {
                        self.list_appearance(*w, node, &info.options, &shown, helv);
                    } else {
                        self.text_appearance(*w, node, &info.kind, &shown, helv);
                    }
                }
            }
            _ => return Err(Error::BadValue(format!("{name} cannot hold a value"))),
        }
        self.mark_need_appearances(helv)?;
        Ok(())
    }

    fn field_nodes(&self) -> Vec<Node> {
        let mut out = Vec::new();
        let Ok((_, cat)) = self.catalog() else { return out };
        let Some(form) = self.lookup(&cat, b"AcroForm").and_then(|o| o.as_dict().cloned()) else { return out };
        let fields = self.lookup(&form, b"Fields").and_then(|o| o.as_array().cloned()).unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        for f in fields.iter().filter_map(Object::as_ref) {
            self.walk(f, "", &Inherit::default(), &mut out, &mut seen, 0);
        }
        out
    }

    fn walk(&self, id: ObjId, prefix: &str, parent: &Inherit, out: &mut Vec<Node>, seen: &mut std::collections::HashSet<u32>, depth: usize) {
        if depth > 32 || !seen.insert(id.0) { return; }
        let Some(d) = self.get_dict(id) else { return };
        let mut inh = parent.clone();
        if let Some(ft) = self.lookup(&d, b"FT").and_then(|o| o.as_name().map(<[u8]>::to_vec)) { inh.ft = Some(ft); }
        if let Some(ff) = self.lookup(&d, b"Ff").and_then(|o| o.as_int()) { inh.ff = ff; }
        if let Some(v) = d.get(b"V") { inh.v = Some(v.clone()); }
        if let Some(da) = self.lookup(&d, b"DA").and_then(|o| o.as_str().map(<[u8]>::to_vec)) { inh.da = Some(da); }
        if let Some(q) = self.lookup(&d, b"Q").and_then(|o| o.as_int()) { inh.q = q; }
        if let Some(o) = self.lookup(&d, b"Opt") { inh.opt = Some(o); }
        if let Some(m) = self.lookup(&d, b"MaxLen").and_then(|o| o.as_int()) { inh.max_len = Some(m); }
        let partial = self.lookup(&d, b"T").and_then(|o| o.as_str().map(decode_text));
        let name = match (&partial, prefix.is_empty()) {
            (Some(t), true) => t.clone(),
            (Some(t), false) => format!("{prefix}.{t}"),
            (None, _) => prefix.to_string(),
        };
        let kids: Vec<ObjId> = self.lookup(&d, b"Kids").and_then(|o| o.as_array().map(|a| a.iter().filter_map(Object::as_ref).collect())).unwrap_or_default();
        let kid_fields: Vec<ObjId> = kids.iter().copied().filter(|k| self.get_dict(*k).is_some_and(|kd| kd.get(b"T").is_some())).collect();
        if kid_fields.is_empty() {
            let mut widgets: Vec<ObjId> = kids.clone();
            if widgets.is_empty() && d.get(b"Subtype").and_then(Object::as_name) == Some(b"Widget") { widgets.push(id); }
            if widgets.is_empty() && kids.is_empty() && d.get(b"Rect").is_some() { widgets.push(id); }
            out.push(Node { id, name, inh, widgets });
        } else {
            for k in kids { self.walk(k, &name, &inh, out, seen, depth + 1); }
        }
    }

    fn widget_states(&self, w: &Dict) -> Vec<String> {
        let mut states = Vec::new();
        if let Some(ap) = w.get(b"AP").and_then(|o| self.dict_of(o)) {
            for key in [&b"N"[..], b"D"] {
                if let Some(nd) = ap.get(key).and_then(|o| self.dict_of(o)) {
                    if matches!(self.resolve(ap.get(key).unwrap()), Object::Stream(_)) { continue; }
                    for (k, _) in &nd.0 {
                        let s = String::from_utf8_lossy(k).into_owned();
                        if s != "Off" && !states.contains(&s) { states.push(s); }
                    }
                }
            }
        }
        states
    }

    fn field_info(&self, node: &Node) -> FieldInfo {
        let ff = node.inh.ff;
        let kind = match node.inh.ft.as_deref() {
            Some(b"Tx") => FieldKind::Text {
                multiline: ff & FF_MULTILINE != 0,
                password: ff & FF_PASSWORD != 0,
                comb: ff & FF_COMB != 0,
                max_len: node.inh.max_len.map(|m| m.max(0) as usize),
            },
            Some(b"Btn") if ff & FF_PUSHBUTTON != 0 => FieldKind::PushButton,
            Some(b"Btn") if ff & FF_RADIO != 0 => FieldKind::Radio,
            Some(b"Btn") => FieldKind::Checkbox,
            Some(b"Ch") if ff & FF_COMBO != 0 => FieldKind::ComboBox { editable: ff & FF_EDIT != 0 },
            Some(b"Ch") => FieldKind::ListBox { multi_select: ff & FF_MULTISELECT != 0 },
            Some(b"Sig") => FieldKind::Signature,
            _ => FieldKind::Unknown,
        };
        let options: Vec<(String, String)> = node.inh.opt.as_ref().and_then(|o| o.as_array().cloned()).unwrap_or_default().iter().filter_map(|o| {
            match self.resolve(o) {
                Object::Str(s) => { let t = decode_text(&s); Some((t.clone(), t)) }
                Object::Array(p) if p.len() >= 2 => {
                    let e = self.resolve(&p[0]).as_str().map(decode_text)?;
                    let d = self.resolve(&p[1]).as_str().map(decode_text)?;
                    Some((e, d))
                }
                _ => None,
            }
        }).collect();
        let mut states = Vec::new();
        let mut widgets = Vec::new();
        for w in &node.widgets {
            let wd = self.get_dict(*w).unwrap_or_default();
            for s in self.widget_states(&wd) { if !states.contains(&s) { states.push(s); } }
            let page = wd.get(b"P").and_then(Object::as_ref).and_then(|p| self.page_index(p))
                .or_else(|| (0..self.page_count()).find(|&i| self.annotations(i).map(|a| a.iter().any(|x| x.id.map(|i| i.0) == Some(w.0))).unwrap_or(false)));
            widgets.push((*w, page, self.lookup(&wd, b"Rect").and_then(|r| rect_of(&r)).unwrap_or_default()));
        }
        let v = node.inh.v.as_ref().map(|v| self.resolve(v));
        let value = match (&kind, v) {
            (_, None) => FieldValue::Empty,
            (FieldKind::Signature, Some(_)) => FieldValue::Empty,
            (_, Some(Object::Name(nm))) => FieldValue::State(String::from_utf8_lossy(&nm).into_owned()),
            (_, Some(Object::Str(s))) => FieldValue::Text(decode_text(&s)),
            (_, Some(Object::Array(a))) => FieldValue::Choices(a.iter().filter_map(|o| self.resolve(o).as_str().map(decode_text)).collect()),
            _ => FieldValue::Empty,
        };
        FieldInfo {
            name: node.name.clone(),
            id: node.id,
            kind,
            value,
            options,
            states,
            read_only: ff & FF_READONLY != 0,
            required: ff & FF_REQUIRED != 0,
            widgets,
        }
    }

    /// The AcroForm's `/Helv` font from `/DR` if present, else our own Helvetica.
    fn form_font(&mut self) -> ObjId {
        let dr_helv = self.catalog().ok()
            .and_then(|(_, c)| self.lookup(&c, b"AcroForm"))
            .and_then(|f| f.as_dict().and_then(|f| self.lookup(f, b"DR")))
            .and_then(|dr| dr.as_dict().and_then(|dr| self.lookup(dr, b"Font")))
            .and_then(|fd| fd.as_dict().and_then(|fd| fd.get(b"Helv").and_then(Object::as_ref)));
        dr_helv.unwrap_or_else(|| self.helvetica())
    }

    /// Sets `/NeedAppearances true` (unless a signature is present: it would make viewers
    /// rewrite signed appearances) and makes sure `/DR` has `/Helv`.
    fn mark_need_appearances(&mut self, helv: ObjId) -> Result<(), Error> {
        let signed = self.signatures()?.iter().any(|s| s.signed);
        let (cat_id, mut cat) = self.catalog()?;
        let (holder, mut form) = match cat.get(b"AcroForm") {
            Some(Object::Ref(r)) => (Some(*r), self.get_dict(*r).unwrap_or_default()),
            Some(o) => (None, o.as_dict().cloned().unwrap_or_default()),
            None => return Ok(()),
        };
        let mut changed = false;
        if !signed && form.get(b"NeedAppearances") != Some(&Object::Bool(true)) {
            form.set(b"NeedAppearances", true);
            changed = true;
        }
        // /DR /Font /Helv, resolving indirect levels.
        let dr_obj = form.get(b"DR").cloned();
        let mut dr = dr_obj.as_ref().and_then(|o| self.dict_of(o)).unwrap_or_default();
        let font_obj = dr.get(b"Font").cloned();
        let mut fonts = font_obj.as_ref().and_then(|o| self.dict_of(o)).unwrap_or_default();
        if fonts.get(b"Helv").is_none() {
            fonts.set(b"Helv", helv);
            match font_obj { Some(Object::Ref(r)) => self.set(r, fonts), _ => dr.set(b"Font", fonts) }
            match dr_obj { Some(Object::Ref(r)) => self.set(r, dr), _ => { form.set(b"DR", dr); changed = true; } }
            if form.get(b"DA").is_none() { form.set(b"DA", Object::Str(b"/Helv 0 Tf 0 g".to_vec())); changed = true; }
        }
        if changed {
            match holder {
                Some(h) => self.set(h, form),
                None => { cat.set(b"AcroForm", form); self.set(cat_id, cat); }
            }
        }
        Ok(())
    }

    fn widget_box(&self, w: &Dict) -> (f64, f64, Option<[f64; 3]>, Option<[f64; 3]>) {
        let r = self.lookup(w, b"Rect").and_then(|o| rect_of(&o)).unwrap_or([0.0, 0.0, 100.0, 20.0]);
        let mk = self.lookup(w, b"MK").and_then(|o| o.as_dict().cloned()).unwrap_or_default();
        let rot = self.lookup(&mk, b"R").and_then(|o| o.as_int()).unwrap_or(0).rem_euclid(360);
        let (mut wd, mut ht) = (r[2] - r[0], r[3] - r[1]);
        if rot == 90 || rot == 270 { std::mem::swap(&mut wd, &mut ht); }
        let col = |k: &[u8]| -> Option<[f64; 3]> {
            let a: Vec<f64> = self.lookup(&mk, k)?.as_array()?.iter().filter_map(Object::as_f64).collect();
            match a.len() { 1 => Some([a[0]; 3]), 3 => Some([a[0], a[1], a[2]]), 4 => Some([(1.0 - a[0]) * (1.0 - a[3]), (1.0 - a[1]) * (1.0 - a[3]), (1.0 - a[2]) * (1.0 - a[3])]), _ => None }
        };
        (wd.max(1.0), ht.max(1.0), col(b"BG"), col(b"BC"))
    }

    fn frame(w: f64, h: f64, bg: Option<[f64; 3]>, bc: Option<[f64; 3]>) -> String {
        let mut s = String::new();
        if let Some(c) = bg { s.push_str(&format!("{} {} {} rg 0 0 {} {} re f\n", n(c[0]), n(c[1]), n(c[2]), n(w), n(h))); }
        if let Some(c) = bc { s.push_str(&format!("{} {} {} RG 1 w 0.5 0.5 {} {} re S\n", n(c[0]), n(c[1]), n(c[2]), n(w - 1.0), n(h - 1.0))); }
        s
    }

    fn put_appearance(&mut self, wid: ObjId, w: f64, h: f64, ops: String, helv: Option<ObjId>) -> ObjId {
        let mut res = Dict::new();
        if let Some(f) = helv { res.set(b"Font", Dict::new().with(b"Helv", f)); }
        let wd = self.get_dict(wid).unwrap_or_default();
        let mut d = Dict::new()
            .with(b"Type", Object::name("XObject"))
            .with(b"Subtype", Object::name("Form"))
            .with(b"BBox", Object::Array(vec![Object::Int(0), Object::Int(0), Object::Real(w), Object::Real(h)]))
            .with(b"Resources", res);
        // Rotated widgets (/MK /R) get the matching matrix.
        let rot = self.lookup(&wd, b"MK").and_then(|m| m.as_dict().and_then(|m| self.lookup(m, b"R")?.as_int())).unwrap_or(0).rem_euclid(360);
        let m = match rot { 90 => Some([0.0, 1.0, -1.0, 0.0, h, 0.0]), 180 => Some([-1.0, 0.0, 0.0, -1.0, w, h]), 270 => Some([0.0, -1.0, 1.0, 0.0, 0.0, w]), _ => None };
        if let Some(m) = m { d.set(b"Matrix", Object::Array(m.iter().map(|v| Object::Real(*v)).collect())); }
        self.add(Stream { dict: d, data: ops.into_bytes() })
    }

    fn da_parts(node: &Node) -> (f64, String) {
        let da = node.inh.da.as_deref().map(|d| String::from_utf8_lossy(d).into_owned()).unwrap_or_else(|| "/Helv 0 Tf 0 g".into());
        let toks: Vec<&str> = da.split_whitespace().collect();
        let mut size = 0.0;
        let mut color = "0 g".to_string();
        for (i, t) in toks.iter().enumerate() {
            match *t {
                "Tf" if i >= 1 => size = toks[i - 1].parse().unwrap_or(0.0),
                "g" | "G" if i >= 1 => color = format!("{} g", toks[i - 1]),
                "rg" | "RG" if i >= 3 => color = format!("{} {} {} rg", toks[i - 3], toks[i - 2], toks[i - 1]),
                "k" | "K" if i >= 4 => color = format!("{} {} {} {} k", toks[i - 4], toks[i - 3], toks[i - 2], toks[i - 1]),
                _ => {}
            }
        }
        (size, color)
    }

    fn text_appearance(&mut self, wid: ObjId, node: &Node, kind: &FieldKind, lines_in: &[String], helv: ObjId) {
        let wd = self.get_dict(wid).unwrap_or_default();
        let (w, h, bg, bc) = self.widget_box(&wd);
        let (mut size, color) = Self::da_parts(node);
        let q = self.lookup(&wd, b"Q").and_then(|o| o.as_int()).unwrap_or(node.inh.q);
        let (multiline, password, comb, max_len) = match kind {
            FieldKind::Text { multiline, password, comb, max_len } => (*multiline, *password, *comb, *max_len),
            _ => (false, false, false, None),
        };
        let mut text = lines_in.join(", ");
        if password { text = "*".repeat(text.chars().count()); }
        let mut ops = Self::frame(w, h, bg, bc);
        ops.push_str("/Tx BMC\nq 1 1 ");
        ops.push_str(&format!("{} {} re W n\nBT /Helv ", n(w - 2.0), n(h - 2.0)));
        if multiline {
            if size <= 0.0 { size = 10.0f64.min(h - 4.0).max(4.0); }
            ops.push_str(&format!("{} Tf {}\n", n(size), color));
            let lead = size * 1.15;
            for (i, line) in wrap(&text, size, w - 4.0).iter().enumerate() {
                let y = h - 2.0 - size * 0.9 - lead * i as f64;
                if y < -size { break; }
                let x = align(q, w, text_width(line, size));
                ops.push_str(&format!("1 0 0 1 {} {} Tm {} Tj\n", n(x), n(y), String::from_utf8_lossy(&content_string(line))));
            }
        } else {
            if size <= 0.0 {
                size = ((h - 4.0) / 1.15).clamp(4.0, 12.0);
                let tw = text_width(&text, size);
                if tw > w - 4.0 && tw > 0.0 { size = (size * (w - 4.0) / tw).max(4.0); }
            }
            ops.push_str(&format!("{} Tf {}\n", n(size), color));
            let y = (h - size) / 2.0 + size * 0.22;
            match (comb, max_len) {
                (true, Some(m)) if m > 0 => {
                    let cell = w / m as f64;
                    for (i, c) in text.chars().enumerate() {
                        let cs = c.to_string();
                        let x = cell * i as f64 + (cell - text_width(&cs, size)) / 2.0;
                        ops.push_str(&format!("1 0 0 1 {} {} Tm {} Tj\n", n(x), n(y), String::from_utf8_lossy(&content_string(&cs))));
                    }
                }
                _ => {
                    let x = align(q, w, text_width(&text, size));
                    ops.push_str(&format!("1 0 0 1 {} {} Tm {} Tj\n", n(x), n(y), String::from_utf8_lossy(&content_string(&text))));
                }
            }
        }
        ops.push_str("ET Q\nEMC\n");
        let ap = self.put_appearance(wid, w, h, ops, Some(helv));
        let mut wd = self.get_dict(wid).unwrap_or_default();
        wd.set(b"AP", Dict::new().with(b"N", ap));
        self.set_dict(wid, wd);
    }

    fn list_appearance(&mut self, wid: ObjId, node: &Node, options: &[(String, String)], selected: &[String], helv: ObjId) {
        let wd = self.get_dict(wid).unwrap_or_default();
        let (w, h, bg, bc) = self.widget_box(&wd);
        let (mut size, color) = Self::da_parts(node);
        if size <= 0.0 { size = 10.0; }
        let lead = size * 1.15;
        let mut ops = Self::frame(w, h, bg, bc);
        ops.push_str(&format!("/Tx BMC\nq 1 1 {} {} re W n\n", n(w - 2.0), n(h - 2.0)));
        for (i, (_, disp)) in options.iter().enumerate() {
            let top = h - 1.0 - lead * i as f64;
            if top < 0.0 { break; }
            let sel = selected.contains(disp);
            if sel { ops.push_str(&format!("0.6 0.75 0.95 rg 1 {} {} {} re f\n", n(top - lead), n(w - 2.0), n(lead))); }
            ops.push_str(&format!("BT /Helv {} Tf {} 1 0 0 1 2 {} Tm {} Tj ET\n", n(size), color, n(top - size * 0.95), String::from_utf8_lossy(&content_string(disp))));
        }
        ops.push_str("Q\nEMC\n");
        let ap = self.put_appearance(wid, w, h, ops, Some(helv));
        let mut wd = self.get_dict(wid).unwrap_or_default();
        wd.set(b"AP", Dict::new().with(b"N", ap));
        self.set_dict(wid, wd);
    }

    /// Draws on/off appearances for a checkbox or radio widget that had none.
    fn check_appearance(&mut self, wid: ObjId, on: &str, radio: bool) {
        let wd = self.get_dict(wid).unwrap_or_default();
        let (w, h, bg, bc) = self.widget_box(&wd);
        let frame = Self::frame(w, h, bg, bc);
        let mark = if radio {
            let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) * 0.3);
            let k = r * 0.5523;
            format!("0 g {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c f\n",
                n(cx + r), n(cy),
                n(cx + r), n(cy + k), n(cx + k), n(cy + r), n(cx), n(cy + r),
                n(cx - k), n(cy + r), n(cx - r), n(cy + k), n(cx - r), n(cy),
                n(cx - r), n(cy - k), n(cx - k), n(cy - r), n(cx), n(cy - r),
                n(cx + k), n(cy - r), n(cx + r), n(cy - k), n(cx + r), n(cy))
        } else {
            format!("0 G {} w 1 J 1 j {} {} m {} {} l {} {} l S\n", n(w.min(h) * 0.12), n(w * 0.2), n(h * 0.5), n(w * 0.42), n(h * 0.25), n(w * 0.8), n(h * 0.8))
        };
        let on_id = self.put_appearance(wid, w, h, format!("{frame}{mark}"), None);
        let off_id = self.put_appearance(wid, w, h, frame, None);
        let mut wd = self.get_dict(wid).unwrap_or_default();
        let states = Dict::new().with(on.as_bytes(), on_id).with(b"Off", off_id);
        wd.set(b"AP", Dict::new().with(b"N", states));
        self.set_dict(wid, wd);
    }
}

fn align(q: i64, w: f64, tw: f64) -> f64 {
    match q {
        1 => ((w - tw) / 2.0).max(2.0),
        2 => (w - 2.0 - tw).max(2.0),
        _ => 2.0,
    }
}
