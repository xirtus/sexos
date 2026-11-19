// SPDX-License-Identifier: GPL-3.0-only
//! One description per settings struct, used four ways: the Properties
//! editor's panels, polyops options (`.poly` files), the point cache's
//! "outdated" signature and the `.blend` reader's defaults.
//!
//! A settings struct implements [`Props::visit`], calling the visitor for
//! each panel (Blender's panel layout and titles) and each property (its
//! Python identifier, its UI label and a mutable reference). The helpers
//! here collect the properties into a flat list of [`Value`]s, set one by
//! identifier, and hash them.
use std::hash::{Hash, Hasher};

/// A Blender enum property: every value with its identifier (Blender's,
/// upper case) and UI label.
pub trait PropEnum: Copy + PartialEq + 'static {
    const ITEMS: &'static [(Self, &'static str, &'static str)];
    fn ident(self) -> &'static str {
        Self::ITEMS.iter().find(|i| i.0 == self).map_or("", |i| i.1)
    }
    fn label(self) -> &'static str {
        Self::ITEMS.iter().find(|i| i.0 == self).map_or("", |i| i.2)
    }
    /// By identifier or label, any case, spaces or underscores.
    fn parse(s: &str) -> Option<Self> {
        let k = norm(s);
        Self::ITEMS.iter().find(|i| norm(i.1) == k || norm(i.2) == k).map(|i| i.0)
    }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect::<String>().to_ascii_lowercase()
}

/// Object-safe access to an enum field.
pub trait EnumSlot {
    fn index(&self) -> usize;
    fn set_index(&mut self, i: usize);
    fn items(&self) -> Vec<(&'static str, &'static str)>;
}

impl<T: PropEnum> EnumSlot for T {
    fn index(&self) -> usize {
        T::ITEMS.iter().position(|i| i.0 == *self).unwrap_or(0)
    }
    fn set_index(&mut self, i: usize) {
        if let Some(it) = T::ITEMS.get(i) {
            *self = it.0;
        }
    }
    fn items(&self) -> Vec<(&'static str, &'static str)> {
        T::ITEMS.iter().map(|i| (i.1, i.2)).collect()
    }
}

/// Define a [`PropEnum`] with a default.
#[macro_export]
#[doc(hidden)]
macro_rules! prop_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = ($id:literal, $label:literal)),* $(,)? } default $d:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vm])* $v),* }
        impl $crate::physics::props::PropEnum for $name {
            const ITEMS: &'static [(Self, &'static str, &'static str)] = &[$(($name::$v, $id, $label)),*];
        }
        impl Default for $name {
            fn default() -> Self { $name::$d }
        }
    };
}

/// How a number is shown and limited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub unit: &'static str,
    pub slider: bool,
}

impl Range {
    pub const fn new(min: f32, max: f32, step: f32) -> Range {
        Range { min, max, step, unit: "", slider: false }
    }
    pub const fn unit(self, unit: &'static str) -> Range {
        Range { unit, ..self }
    }
    pub const fn slider(self) -> Range {
        Range { slider: true, ..self }
    }
}

/// What a property refers to, for the UI's pickers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Object,
    Collection,
    VertexGroup,
    Text,
}

/// A mutable property handed to a visitor.
pub enum Prop<'a> {
    Float(&'a mut f32, Range),
    Int(&'a mut i32, i32, i32),
    Bool(&'a mut bool),
    Enum(&'a mut dyn EnumSlot),
    Vec3(&'a mut [f32; 3], Range),
    Color(&'a mut [f32; 3]),
    Ref(&'a mut String, RefKind),
}

/// A property value, detached from its struct.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    F(f32),
    I(i32),
    B(bool),
    /// Enum index (and its identifier, for files).
    E(usize, &'static str),
    V3([f32; 3]),
    S(String),
}

impl Value {
    /// polyops text.
    pub fn to_text(&self) -> String {
        match self {
            Value::F(f) => fmt_f(*f),
            Value::I(i) => i.to_string(),
            Value::B(b) => (*b as u8).to_string(),
            Value::E(_, id) => id.to_ascii_lowercase(),
            Value::V3(v) => format!("{},{},{}", fmt_f(v[0]), fmt_f(v[1]), fmt_f(v[2])),
            Value::S(s) => crate::script::quote(s),
        }
    }
}

pub(crate) fn fmt_f(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e7 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

pub trait Visitor {
    /// Start a panel (closed by [`Visitor::end`]). `toggle` is a checkbox in
    /// its header (Blender's `use_*` panels such as Self Collisions).
    fn panel(&mut self, key: &'static str, title: &'static str, open: bool, toggle: Option<(&'static str, &mut bool)>);
    fn end(&mut self);
    fn prop(&mut self, id: &'static str, label: &'static str, p: Prop<'_>);
    /// Grey out (or not) the properties that follow, up to the panel's end.
    fn active(&mut self, _on: bool) {}
}

pub trait Props {
    fn visit(&mut self, v: &mut dyn Visitor);
}

/// One collected property.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: &'static str,
    pub label: &'static str,
    pub value: Value,
    pub range: Option<Range>,
    pub reference: Option<RefKind>,
    pub items: Vec<(&'static str, &'static str)>,
    pub active: bool,
}

/// A collected panel tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Panel { key: &'static str, title: &'static str, open: bool, toggle: Option<(&'static str, bool)>, children: Vec<Node> },
    Prop(Entry),
}

struct Collect {
    stack: Vec<(Vec<Node>, Option<(&'static str, &'static str, bool, Option<(&'static str, bool)>)>, bool)>,
    active: bool,
}

impl Visitor for Collect {
    fn panel(&mut self, key: &'static str, title: &'static str, open: bool, toggle: Option<(&'static str, &mut bool)>) {
        let t = toggle.map(|(id, b)| (id, *b));
        self.stack.push((Vec::new(), Some((key, title, open, t)), self.active));
        if let Some((_, on)) = t {
            self.active = self.active && on;
        }
    }
    fn end(&mut self) {
        if self.stack.len() < 2 {
            return;
        }
        let (children, head, was) = self.stack.pop().unwrap();
        self.active = was;
        if let Some((key, title, open, toggle)) = head {
            self.stack.last_mut().unwrap().0.push(Node::Panel { key, title, open, toggle, children });
        }
    }
    fn prop(&mut self, id: &'static str, label: &'static str, p: Prop<'_>) {
        let (value, range, reference, items) = match p {
            Prop::Float(v, r) => (Value::F(*v), Some(r), None, Vec::new()),
            Prop::Int(v, lo, hi) => (Value::I(*v), Some(Range::new(lo as f32, hi as f32, 1.0)), None, Vec::new()),
            Prop::Bool(v) => (Value::B(*v), None, None, Vec::new()),
            Prop::Enum(e) => {
                let items = e.items();
                let i = e.index();
                (Value::E(i, items.get(i).map_or("", |x| x.0)), None, None, items)
            }
            Prop::Vec3(v, r) => (Value::V3(*v), Some(r), None, Vec::new()),
            Prop::Color(v) => (Value::V3(*v), Some(Range::new(0.0, 1.0, 0.01)), None, Vec::new()),
            Prop::Ref(s, k) => (Value::S(s.clone()), None, Some(k), Vec::new()),
        };
        self.stack.last_mut().unwrap().0.push(Node::Prop(Entry { id, label, value, range, reference, items, active: self.active }));
    }
    fn active(&mut self, on: bool) {
        let parent = self.stack.last().map_or(true, |s| s.2);
        self.active = parent && on;
    }
}

/// The panel tree of `p`.
pub fn tree(p: &mut dyn Props) -> Vec<Node> {
    let mut c = Collect { stack: vec![(Vec::new(), None, true)], active: true };
    p.visit(&mut c);
    while c.stack.len() > 1 {
        c.end();
    }
    c.stack.pop().map(|s| s.0).unwrap_or_default()
}

/// Every property of `p` in order (header toggles included, as booleans).
pub fn entries(p: &mut dyn Props) -> Vec<Entry> {
    fn walk(nodes: &[Node], out: &mut Vec<Entry>) {
        for n in nodes {
            match n {
                Node::Prop(e) => out.push(e.clone()),
                Node::Panel { toggle, children, .. } => {
                    if let Some((id, b)) = toggle {
                        out.push(Entry { id, label: id, value: Value::B(*b), range: None, reference: None, items: Vec::new(), active: true });
                    }
                    walk(children, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&tree(p), &mut out);
    out
}

struct Setter<'a> {
    id: &'a str,
    text: Option<&'a str>,
    value: Option<&'a Value>,
    done: Result<bool, String>,
}

impl Setter<'_> {
    fn hit(&self, id: &str) -> bool {
        self.done == Ok(false) && id == self.id
    }
}

fn parse_f(s: &str) -> Result<f32, String> {
    s.trim().parse::<f32>().map_err(|_| format!("'{s}' is not a number"))
}

impl Visitor for Setter<'_> {
    fn panel(&mut self, _key: &'static str, _title: &'static str, _open: bool, toggle: Option<(&'static str, &mut bool)>) {
        if let Some((id, b)) = toggle {
            self.prop(id, id, Prop::Bool(b));
        }
    }
    fn end(&mut self) {}
    fn prop(&mut self, id: &'static str, _label: &'static str, p: Prop<'_>) {
        if !self.hit(id) {
            return;
        }
        let r: Result<(), String> = (|| {
            match p {
                Prop::Float(v, r) => {
                    let x = match (self.value, self.text) {
                        (Some(Value::F(f)), _) => *f,
                        (Some(Value::I(i)), _) => *i as f32,
                        (_, Some(t)) => parse_f(t)?,
                        _ => return Err("expected a number".into()),
                    };
                    *v = if r.min < r.max { x.clamp(r.min, r.max) } else { x };
                }
                Prop::Int(v, lo, hi) => {
                    let x = match (self.value, self.text) {
                        (Some(Value::I(i)), _) => *i,
                        (Some(Value::F(f)), _) => f.round() as i32,
                        (_, Some(t)) => t.trim().parse::<f64>().map_err(|_| format!("'{t}' is not an integer"))?.round() as i32,
                        _ => return Err("expected an integer".into()),
                    };
                    *v = x.clamp(lo, hi);
                }
                Prop::Bool(v) => {
                    *v = match (self.value, self.text) {
                        (Some(Value::B(b)), _) => *b,
                        (_, Some(t)) => match t.trim().to_ascii_lowercase().as_str() {
                            "1" | "true" | "yes" | "on" => true,
                            "0" | "false" | "no" | "off" => false,
                            _ => return Err(format!("'{t}' is not a boolean")),
                        },
                        _ => return Err("expected a boolean".into()),
                    };
                }
                Prop::Enum(e) => {
                    let items = e.items();
                    let i = match (self.value, self.text) {
                        (Some(Value::E(i, _)), _) => *i,
                        (Some(Value::I(i)), _) => (*i).max(0) as usize,
                        (_, Some(t)) => {
                            let k = norm(t);
                            items.iter().position(|it| norm(it.0) == k || norm(it.1) == k).ok_or_else(|| {
                                format!("'{t}' is not one of {}", items.iter().map(|i| i.0.to_ascii_lowercase()).collect::<Vec<_>>().join(", "))
                            })?
                        }
                        _ => return Err("expected an enum value".into()),
                    };
                    e.set_index(i);
                }
                Prop::Vec3(v, _) | Prop::Color(v) => {
                    *v = match (self.value, self.text) {
                        (Some(Value::V3(x)), _) => *x,
                        (_, Some(t)) => {
                            let parts: Vec<f32> = t.split(',').map(parse_f).collect::<Result<_, _>>()?;
                            match parts.len() {
                                1 => [parts[0]; 3],
                                3 => [parts[0], parts[1], parts[2]],
                                _ => return Err("expected 3 numbers".into()),
                            }
                        }
                        _ => return Err("expected a vector".into()),
                    };
                }
                Prop::Ref(s, _) => {
                    *s = match (self.value, self.text) {
                        (Some(Value::S(x)), _) => x.clone(),
                        (_, Some(t)) => {
                            if t == "none" {
                                String::new()
                            } else {
                                t.to_string()
                            }
                        }
                        _ => return Err("expected a name".into()),
                    };
                }
            }
            Ok(())
        })();
        self.done = r.map(|_| true).map_err(|e| format!("{id}: {e}"));
    }
}

/// Set property `id` from polyops text. `Ok(false)`: no such property.
pub fn set_text(p: &mut dyn Props, id: &str, text: &str) -> Result<bool, String> {
    let mut s = Setter { id, text: Some(text), value: None, done: Ok(false) };
    p.visit(&mut s);
    s.done
}

/// Set property `id` to `value`. Returns false when there is no such property.
pub fn set_value(p: &mut dyn Props, id: &str, value: &Value) -> bool {
    let mut s = Setter { id, text: None, value: Some(value), done: Ok(false) };
    p.visit(&mut s);
    matches!(s.done, Ok(true))
}

/// The value of property `id`.
pub fn get(p: &mut dyn Props, id: &str) -> Option<Value> {
    entries(p).into_iter().find(|e| e.id == id).map(|e| e.value)
}

/// ` key=value` for every property that differs from `default`'s.
pub fn diff_options(p: &mut dyn Props, default: &mut dyn Props) -> String {
    let d = entries(default);
    let mut out = String::new();
    for e in entries(p) {
        if d.iter().any(|x| x.id == e.id && x.value == e.value) {
            continue;
        }
        out.push(' ');
        out.push_str(e.id);
        out.push('=');
        out.push_str(&e.value.to_text());
    }
    out
}

/// Feed every property into a hasher (the point cache's signature).
pub fn hash_into(p: &mut dyn Props, h: &mut impl Hasher) {
    for e in entries(p) {
        e.id.hash(h);
        match e.value {
            Value::F(f) => f.to_bits().hash(h),
            Value::I(i) => i.hash(h),
            Value::B(b) => b.hash(h),
            Value::E(i, _) => i.hash(h),
            Value::V3(v) => v.map(f32::to_bits).hash(h),
            Value::S(s) => s.hash(h),
        }
    }
}
