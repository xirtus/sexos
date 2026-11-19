// SPDX-License-Identifier: GPL-3.0-only
//! Settings described once, used three ways: a type lists its fields through
//! a [`Visit`]or, and the visitor writes them out (polyops options), reads
//! them back (from a command's options) or draws them (Properties panels).
//! Grease Pencil modifiers, visual effects, layers, materials and sequencer
//! strips use it, so a new setting appears in the file format and the UI at
//! once.

/// The range and step of a number field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// Shown as a 0..1 slider.
    pub slider: bool,
    /// Degrees in the UI (stored in radians).
    pub angle: bool,
}

impl Range {
    pub const ANY: Range = Range { min: f32::NEG_INFINITY, max: f32::INFINITY, step: 0.1, slider: false, angle: false };
    pub const POSITIVE: Range = Range { min: 0.0, max: f32::INFINITY, step: 0.1, slider: false, angle: false };
    pub const FACTOR: Range = Range { min: 0.0, max: 1.0, step: 0.05, slider: true, angle: false };
    pub const ANGLE: Range = Range { min: f32::NEG_INFINITY, max: f32::INFINITY, step: 0.1, slider: false, angle: true };
    pub const fn new(min: f32, max: f32) -> Range {
        Range { min, max, step: 0.1, slider: false, angle: false }
    }
    pub const fn slider(min: f32, max: f32) -> Range {
        Range { min, max, step: 0.05, slider: true, angle: false }
    }
}

/// What a name field refers to (the UI offers a list of these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Text,
    Object,
    Collection,
    /// A layer of the data-block being edited.
    Layer,
    /// A material of the data-block being edited.
    Material,
    VertexGroup,
    Image,
    /// A file path.
    File,
}

/// Something that walks over settings. Every method gets the field's
/// polyops key, its UI label and the value to read or change.
pub trait Visit {
    fn float(&mut self, key: &str, label: &str, v: &mut f32, r: Range);
    fn int(&mut self, key: &str, label: &str, v: &mut i32, min: i32, max: i32);
    fn boolean(&mut self, key: &str, label: &str, v: &mut bool);
    /// A vector or colour of 2 to 4 components.
    fn floats(&mut self, key: &str, label: &str, v: &mut [f32], r: Range, color: bool);
    fn name(&mut self, key: &str, label: &str, v: &mut String, kind: NameKind);
    /// One of `items` (identifier, label); `v` indexes `items`.
    fn choice(&mut self, key: &str, label: &str, items: &[(&str, &str)], v: &mut usize);
    /// A heading (the UI starts a group; others ignore it).
    fn heading(&mut self, _label: &str) {}

    fn uint(&mut self, key: &str, label: &str, v: &mut u32, min: u32, max: u32) {
        let mut i = (*v).min(i32::MAX as u32) as i32;
        self.int(key, label, &mut i, min.min(i32::MAX as u32) as i32, max.min(i32::MAX as u32) as i32);
        *v = i.clamp(min.min(i32::MAX as u32) as i32, max.min(i32::MAX as u32) as i32).max(0) as u32;
    }
    /// An optional index (`-1` for none).
    fn index(&mut self, key: &str, label: &str, v: &mut Option<usize>) {
        let mut i = v.map_or(-1, |x| x as i32);
        self.int(key, label, &mut i, -1, i32::MAX);
        *v = (i >= 0).then_some(i as usize);
    }
    fn byte(&mut self, key: &str, label: &str, v: &mut u8, max: u8) {
        let mut i = *v as i32;
        self.int(key, label, &mut i, 0, max as i32);
        *v = i.clamp(0, max as i32) as u8;
    }
    fn uints(&mut self, key: &str, label: &str, v: &mut [u32]) {
        let mut f: Vec<f32> = v.iter().map(|&x| x as f32).collect();
        self.floats(key, label, &mut f, Range { step: 1.0, ..Range::POSITIVE }, false);
        for (d, s) in v.iter_mut().zip(f) {
            *d = s.max(0.0).round() as u32;
        }
    }
    fn ints(&mut self, key: &str, label: &str, v: &mut [i32]) {
        let mut f: Vec<f32> = v.iter().map(|&x| x as f32).collect();
        self.floats(key, label, &mut f, Range { step: 1.0, ..Range::ANY }, false);
        for (d, s) in v.iter_mut().zip(f) {
            *d = s.round() as i32;
        }
    }
    fn bools(&mut self, key: &str, label: &str, v: &mut [bool]) {
        let mut f: Vec<f32> = v.iter().map(|&x| x as u8 as f32).collect();
        self.floats(key, label, &mut f, Range { step: 1.0, ..Range::new(0.0, 1.0) }, false);
        for (d, s) in v.iter_mut().zip(f) {
            *d = s > 0.5;
        }
    }
}

/// [`Visit::choice`] over a list of (value, identifier, label).
pub fn pick<T: Copy + PartialEq>(v: &mut dyn Visit, key: &str, label: &str, all: &[(T, &str, &str)], cur: &mut T) {
    let items: Vec<(&str, &str)> = all.iter().map(|x| (x.1, x.2)).collect();
    let mut i = all.iter().position(|x| x.0 == *cur).unwrap_or(0);
    v.choice(key, label, &items, &mut i);
    if let Some(x) = all.get(i) {
        *cur = x.0;
    }
}

/// Collects `key=value` pairs (values as text, lists comma-separated).
#[derive(Default)]
pub struct Pairs {
    pub pairs: Vec<(String, String)>,
}

pub fn fmt_num(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

impl Visit for Pairs {
    fn float(&mut self, key: &str, _: &str, v: &mut f32, _: Range) {
        self.pairs.push((key.into(), fmt_num(*v)));
    }
    fn int(&mut self, key: &str, _: &str, v: &mut i32, _: i32, _: i32) {
        self.pairs.push((key.into(), v.to_string()));
    }
    fn boolean(&mut self, key: &str, _: &str, v: &mut bool) {
        self.pairs.push((key.into(), (*v as u8).to_string()));
    }
    fn floats(&mut self, key: &str, _: &str, v: &mut [f32], _: Range, _: bool) {
        self.pairs.push((key.into(), v.iter().map(|x| fmt_num(*x)).collect::<Vec<_>>().join(",")));
    }
    fn name(&mut self, key: &str, _: &str, v: &mut String, _: NameKind) {
        self.pairs.push((key.into(), v.clone()));
    }
    fn choice(&mut self, key: &str, _: &str, items: &[(&str, &str)], v: &mut usize) {
        self.pairs.push((key.into(), items.get(*v).map_or("", |x| x.0).to_ascii_lowercase()));
    }
}

/// Sets fields from `key=value` text pairs; unknown keys stay in `rest`,
/// bad values in `errors`.
pub struct Apply {
    pub rest: Vec<(String, String)>,
    pub errors: Vec<String>,
}

impl Apply {
    pub fn new(pairs: Vec<(String, String)>) -> Apply {
        Apply { rest: pairs, errors: Vec::new() }
    }
    fn take(&mut self, key: &str) -> Option<String> {
        let i = self.rest.iter().position(|(k, _)| k == key)?;
        Some(self.rest.remove(i).1)
    }
    fn num(&mut self, key: &str, s: &str) -> Option<f32> {
        match s.trim().parse::<f32>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.errors.push(format!("{key}: '{s}' is not a number"));
                None
            }
        }
    }
}

impl Visit for Apply {
    fn float(&mut self, key: &str, _: &str, v: &mut f32, _: Range) {
        if let Some(s) = self.take(key) {
            if let Some(x) = self.num(key, &s) {
                *v = x;
            }
        }
    }
    fn int(&mut self, key: &str, _: &str, v: &mut i32, min: i32, max: i32) {
        if let Some(s) = self.take(key) {
            if let Some(x) = self.num(key, &s) {
                *v = (x.round() as i32).clamp(min, max);
            }
        }
    }
    fn boolean(&mut self, key: &str, _: &str, v: &mut bool) {
        if let Some(s) = self.take(key) {
            match s.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => *v = true,
                "0" | "false" | "no" | "off" => *v = false,
                _ => self.errors.push(format!("{key}: '{s}' is not a boolean")),
            }
        }
    }
    fn floats(&mut self, key: &str, _: &str, v: &mut [f32], _: Range, _: bool) {
        if let Some(s) = self.take(key) {
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() != v.len() && parts.len() != 1 {
                self.errors.push(format!("{key}: expected {} values", v.len()));
                return;
            }
            for i in 0..v.len() {
                if let Some(x) = self.num(key, parts[i.min(parts.len() - 1)]) {
                    v[i] = x;
                }
            }
        }
    }
    fn name(&mut self, key: &str, _: &str, v: &mut String, _: NameKind) {
        if let Some(s) = self.take(key) {
            *v = s;
        }
    }
    fn choice(&mut self, key: &str, _: &str, items: &[(&str, &str)], v: &mut usize) {
        if let Some(s) = self.take(key) {
            match items.iter().position(|x| x.0.eq_ignore_ascii_case(&s) || x.1.eq_ignore_ascii_case(&s)) {
                Some(i) => *v = i,
                None => {
                    let all: Vec<String> = items.iter().map(|x| x.0.to_ascii_lowercase()).collect();
                    self.errors.push(format!("{key}: '{s}' is not one of {}", all.join(", ")));
                }
            }
        }
    }
}

/// Write a value's fields as pairs.
pub fn to_pairs(f: impl FnOnce(&mut dyn Visit)) -> Vec<(String, String)> {
    let mut p = Pairs::default();
    f(&mut p);
    p.pairs
}

/// Apply pairs; returns the unused pairs or the first error.
pub fn apply_pairs(pairs: Vec<(String, String)>, f: impl FnOnce(&mut dyn Visit)) -> Result<Vec<(String, String)>, String> {
    let mut a = Apply::new(pairs);
    f(&mut a);
    match a.errors.into_iter().next() {
        Some(e) => Err(e),
        None => Ok(a.rest),
    }
}
