// SPDX-License-Identifier: GPL-3.0-only
//! Generic named attributes, as Blender's `CustomData`/`AttributeStorage`:
//! a typed array per name on one domain (point, edge, face, corner, curve,
//! instance). [`Val`] is one element of any type, with Blender's implicit
//! conversions between socket types.
use crate::math::{EulerOrder, Mat3, Mat4, Quat, Vec3};

/// Where an attribute lives (Blender's `AttrDomain`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Domain {
    #[default]
    Point,
    Edge,
    Face,
    Corner,
    /// A whole curve (spline).
    Curve,
    Instance,
    Layer,
}

impl Domain {
    pub const ALL: [Domain; 6] = [Domain::Point, Domain::Edge, Domain::Face, Domain::Corner, Domain::Curve, Domain::Instance];

    /// RNA identifier ("POINT", "FACE", ...).
    pub fn name(self) -> &'static str {
        match self {
            Domain::Point => "POINT",
            Domain::Edge => "EDGE",
            Domain::Face => "FACE",
            Domain::Corner => "CORNER",
            Domain::Curve => "CURVE",
            Domain::Instance => "INSTANCE",
            Domain::Layer => "LAYER",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Domain::Point => "Point",
            Domain::Edge => "Edge",
            Domain::Face => "Face",
            Domain::Corner => "Face Corner",
            Domain::Curve => "Spline",
            Domain::Instance => "Instance",
            Domain::Layer => "Layer",
        }
    }

    pub fn from_name(s: &str) -> Option<Domain> {
        Some(match s.to_ascii_uppercase().as_str() {
            "POINT" => Domain::Point,
            "EDGE" => Domain::Edge,
            "FACE" => Domain::Face,
            "CORNER" => Domain::Corner,
            "CURVE" | "SPLINE" => Domain::Curve,
            "INSTANCE" => Domain::Instance,
            "LAYER" => Domain::Layer,
            _ => return None,
        })
    }

    /// Blender's `eAttrDomain` numbering in files.
    pub fn from_file(v: i64) -> Domain {
        match v {
            1 => Domain::Edge,
            2 => Domain::Face,
            3 => Domain::Corner,
            4 => Domain::Curve,
            5 => Domain::Instance,
            6 => Domain::Layer,
            _ => Domain::Point,
        }
    }
}

/// Attribute data types (Blender's `eCustomDataType` as RNA names them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AttrType {
    #[default]
    Float,
    Int,
    Bool,
    Float2,
    Float3,
    Color,
    Quaternion,
    Float4x4,
}

impl AttrType {
    pub fn name(self) -> &'static str {
        match self {
            AttrType::Float => "FLOAT",
            AttrType::Int => "INT",
            AttrType::Bool => "BOOLEAN",
            AttrType::Float2 => "FLOAT2",
            AttrType::Float3 => "FLOAT_VECTOR",
            AttrType::Color => "FLOAT_COLOR",
            AttrType::Quaternion => "QUATERNION",
            AttrType::Float4x4 => "FLOAT4X4",
        }
    }

    pub fn from_name(s: &str) -> Option<AttrType> {
        Some(match s.to_ascii_uppercase().as_str() {
            "FLOAT" | "VALUE" => AttrType::Float,
            "INT" | "INT8" | "INT32" => AttrType::Int,
            "BOOLEAN" | "BOOL" => AttrType::Bool,
            "FLOAT2" | "INT32_2D" => AttrType::Float2,
            "FLOAT_VECTOR" | "VECTOR" | "FLOAT3" => AttrType::Float3,
            "FLOAT_COLOR" | "BYTE_COLOR" | "RGBA" | "COLOR" => AttrType::Color,
            "QUATERNION" | "ROTATION" => AttrType::Quaternion,
            "FLOAT4X4" | "MATRIX" => AttrType::Float4x4,
            _ => return None,
        })
    }

    /// The type a value of this kind evaluates to in fields.
    pub fn of_val(v: &Val) -> AttrType {
        match v {
            Val::Float(_) => AttrType::Float,
            Val::Int(_) => AttrType::Int,
            Val::Bool(_) => AttrType::Bool,
            Val::Vector(_) => AttrType::Float3,
            Val::Color(_) => AttrType::Color,
            Val::Rotation(_) => AttrType::Quaternion,
            Val::Matrix(_) => AttrType::Float4x4,
        }
    }

    pub fn default_val(self) -> Val {
        match self {
            AttrType::Float => Val::Float(0.0),
            AttrType::Int => Val::Int(0),
            AttrType::Bool => Val::Bool(false),
            AttrType::Float2 | AttrType::Float3 => Val::Vector(Vec3::ZERO),
            AttrType::Color => Val::Color([0.0; 4]),
            AttrType::Quaternion => Val::Rotation(Quat::IDENTITY),
            AttrType::Float4x4 => Val::Matrix(Mat4::IDENTITY),
        }
    }

    /// Converts `v` to this type.
    pub fn convert(self, v: Val) -> Val {
        match self {
            AttrType::Float => Val::Float(v.f()),
            AttrType::Int => Val::Int(v.i()),
            AttrType::Bool => Val::Bool(v.b()),
            AttrType::Float2 => {
                let x = v.v();
                Val::Vector(Vec3::new(x.x, x.y, 0.0))
            }
            AttrType::Float3 => Val::Vector(v.v()),
            AttrType::Color => Val::Color(v.c()),
            AttrType::Quaternion => Val::Rotation(v.q()),
            AttrType::Float4x4 => Val::Matrix(v.m()),
        }
    }
}

/// One element of a field or attribute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vector(Vec3),
    /// Linear RGBA.
    Color([f32; 4]),
    Rotation(Quat),
    Matrix(Mat4),
}

impl Default for Val {
    fn default() -> Self {
        Val::Float(0.0)
    }
}

fn luminance(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl Val {
    /// As a float (Blender's implicit conversions: vectors average, colours
    /// give their luminance).
    pub fn f(&self) -> f32 {
        match *self {
            Val::Float(f) => f,
            Val::Int(i) => i as f32,
            Val::Bool(b) => b as i32 as f32,
            Val::Vector(v) => (v.x + v.y + v.z) / 3.0,
            Val::Color(c) => luminance(c),
            Val::Rotation(_) | Val::Matrix(_) => 0.0,
        }
    }

    pub fn i(&self) -> i32 {
        match *self {
            Val::Int(i) => i,
            Val::Bool(b) => b as i32,
            _ => {
                let f = self.f();
                if f.is_finite() {
                    f as i32
                } else {
                    0
                }
            }
        }
    }

    pub fn b(&self) -> bool {
        match *self {
            Val::Bool(b) => b,
            Val::Int(i) => i > 0,
            Val::Rotation(q) => q != Quat::IDENTITY,
            Val::Matrix(m) => m != Mat4::IDENTITY,
            _ => self.f() > 0.0,
        }
    }

    pub fn v(&self) -> Vec3 {
        match *self {
            Val::Vector(v) => v,
            Val::Color(c) => Vec3::new(c[0], c[1], c[2]),
            Val::Rotation(q) => Vec3::from_array(q.to_euler(EulerOrder::XYZ)),
            Val::Matrix(m) => m.translation_part(),
            _ => Vec3::splat(self.f()),
        }
    }

    pub fn c(&self) -> [f32; 4] {
        match *self {
            Val::Color(c) => c,
            Val::Vector(v) => [v.x, v.y, v.z, 1.0],
            Val::Rotation(_) | Val::Matrix(_) => [0.0, 0.0, 0.0, 1.0],
            _ => {
                let f = self.f();
                [f, f, f, 1.0]
            }
        }
    }

    pub fn q(&self) -> Quat {
        match *self {
            Val::Rotation(q) => q,
            Val::Vector(v) => Quat::from_euler(v.to_array(), EulerOrder::XYZ),
            Val::Matrix(m) => m.mat3().to_quat(),
            _ => Quat::IDENTITY,
        }
    }

    pub fn m(&self) -> Mat4 {
        match *self {
            Val::Matrix(m) => m,
            Val::Rotation(q) => Mat3::from_quat(q).to_mat4(),
            _ => Mat4::IDENTITY,
        }
    }

    /// Converts to the type of `like`.
    pub fn like(self, like: &Val) -> Val {
        AttrType::of_val(like).convert(self)
    }
}

/// Mix many values into one, as Blender's attribute mixers do when moving a
/// value to another domain: numbers, vectors and colours average, integers
/// round, booleans are true when any (or with `all`, every) value is.
pub fn mix(vals: impl IntoIterator<Item = Val>, all: bool) -> Option<Val> {
    let mut it = vals.into_iter();
    let first = it.next()?;
    let mut n = 1.0f32;
    Some(match first {
        Val::Float(mut s) => {
            for v in it {
                s += v.f();
                n += 1.0;
            }
            Val::Float(s / n)
        }
        Val::Int(i) => {
            let mut s = i as f64;
            for v in it {
                s += v.i() as f64;
                n += 1.0;
            }
            Val::Int((s / n as f64).round() as i32)
        }
        Val::Bool(b) => {
            let mut acc = b;
            for v in it {
                if all {
                    acc &= v.b();
                } else {
                    acc |= v.b();
                }
            }
            Val::Bool(acc)
        }
        Val::Vector(mut s) => {
            for v in it {
                s += v.v();
                n += 1.0;
            }
            Val::Vector(s / n)
        }
        Val::Color(mut s) => {
            for v in it {
                let c = v.c();
                for k in 0..4 {
                    s[k] += c[k];
                }
                n += 1.0;
            }
            Val::Color(s.map(|x| x / n))
        }
        Val::Rotation(q) => {
            let mut s = q;
            for v in it {
                let mut o = v.q();
                if o.dot(q) < 0.0 {
                    o = Quat { x: -o.x, y: -o.y, z: -o.z, w: -o.w };
                }
                s = Quat { x: s.x + o.x, y: s.y + o.y, z: s.z + o.z, w: s.w + o.w };
            }
            Val::Rotation(s.normalize())
        }
        Val::Matrix(m) => {
            let mut s = m.to_cols_array();
            for v in it {
                for (a, b) in s.iter_mut().zip(v.m().to_cols_array()) {
                    *a += b;
                }
                n += 1.0;
            }
            Val::Matrix(Mat4::from_cols_array(s.map(|x| x / n)))
        }
    })
}

/// Weighted interpolation (barycentric or along curves): numbers, vectors
/// and colours blend; other types take the heaviest value.
pub fn interp(vals: &[(Val, f32)]) -> Val {
    let Some(&(first, _)) = vals.first() else { return Val::Float(0.0) };
    match first {
        Val::Float(_) => Val::Float(vals.iter().map(|(v, w)| v.f() * w).sum()),
        Val::Vector(_) => Val::Vector(vals.iter().fold(Vec3::ZERO, |a, (v, w)| a + v.v() * *w)),
        Val::Color(_) => {
            let mut c = [0.0; 4];
            for (v, w) in vals {
                let x = v.c();
                for k in 0..4 {
                    c[k] += x[k] * w;
                }
            }
            Val::Color(c)
        }
        Val::Int(_) => Val::Int(vals.iter().map(|(v, w)| v.f() * w).sum::<f32>().round() as i32),
        _ => vals.iter().fold((first, f32::MIN), |a, &(v, w)| if w > a.1 { (v, w) } else { a }).0,
    }
}

/// A typed array.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrData {
    Float(Vec<f32>),
    Int(Vec<i32>),
    Bool(Vec<bool>),
    Float2(Vec<[f32; 2]>),
    Float3(Vec<Vec3>),
    Color(Vec<[f32; 4]>),
    Quaternion(Vec<Quat>),
    Float4x4(Vec<Mat4>),
}

macro_rules! each {
    ($s:expr, $v:ident => $e:expr) => {
        match $s {
            AttrData::Float($v) => $e,
            AttrData::Int($v) => $e,
            AttrData::Bool($v) => $e,
            AttrData::Float2($v) => $e,
            AttrData::Float3($v) => $e,
            AttrData::Color($v) => $e,
            AttrData::Quaternion($v) => $e,
            AttrData::Float4x4($v) => $e,
        }
    };
}

macro_rules! map_each {
    ($s:expr, $v:ident => $e:expr) => {
        match $s {
            AttrData::Float($v) => AttrData::Float($e),
            AttrData::Int($v) => AttrData::Int($e),
            AttrData::Bool($v) => AttrData::Bool($e),
            AttrData::Float2($v) => AttrData::Float2($e),
            AttrData::Float3($v) => AttrData::Float3($e),
            AttrData::Color($v) => AttrData::Color($e),
            AttrData::Quaternion($v) => AttrData::Quaternion($e),
            AttrData::Float4x4($v) => AttrData::Float4x4($e),
        }
    };
}

impl AttrData {
    pub fn new(ty: AttrType, n: usize) -> AttrData {
        match ty {
            AttrType::Float => AttrData::Float(vec![0.0; n]),
            AttrType::Int => AttrData::Int(vec![0; n]),
            AttrType::Bool => AttrData::Bool(vec![false; n]),
            AttrType::Float2 => AttrData::Float2(vec![[0.0; 2]; n]),
            AttrType::Float3 => AttrData::Float3(vec![Vec3::ZERO; n]),
            AttrType::Color => AttrData::Color(vec![[0.0; 4]; n]),
            AttrType::Quaternion => AttrData::Quaternion(vec![Quat::IDENTITY; n]),
            AttrType::Float4x4 => AttrData::Float4x4(vec![Mat4::IDENTITY; n]),
        }
    }

    pub fn ty(&self) -> AttrType {
        match self {
            AttrData::Float(_) => AttrType::Float,
            AttrData::Int(_) => AttrType::Int,
            AttrData::Bool(_) => AttrType::Bool,
            AttrData::Float2(_) => AttrType::Float2,
            AttrData::Float3(_) => AttrType::Float3,
            AttrData::Color(_) => AttrType::Color,
            AttrData::Quaternion(_) => AttrType::Quaternion,
            AttrData::Float4x4(_) => AttrType::Float4x4,
        }
    }

    pub fn len(&self) -> usize {
        each!(self, v => v.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, i: usize) -> Val {
        match self {
            AttrData::Float(v) => Val::Float(v[i]),
            AttrData::Int(v) => Val::Int(v[i]),
            AttrData::Bool(v) => Val::Bool(v[i]),
            AttrData::Float2(v) => Val::Vector(Vec3::new(v[i][0], v[i][1], 0.0)),
            AttrData::Float3(v) => Val::Vector(v[i]),
            AttrData::Color(v) => Val::Color(v[i]),
            AttrData::Quaternion(v) => Val::Rotation(v[i]),
            AttrData::Float4x4(v) => Val::Matrix(v[i]),
        }
    }

    pub fn set(&mut self, i: usize, x: Val) {
        match self {
            AttrData::Float(v) => v[i] = x.f(),
            AttrData::Int(v) => v[i] = x.i(),
            AttrData::Bool(v) => v[i] = x.b(),
            AttrData::Float2(v) => {
                let p = x.v();
                v[i] = [p.x, p.y];
            }
            AttrData::Float3(v) => v[i] = x.v(),
            AttrData::Color(v) => v[i] = x.c(),
            AttrData::Quaternion(v) => v[i] = x.q(),
            AttrData::Float4x4(v) => v[i] = x.m(),
        }
    }

    pub fn push(&mut self, x: Val) {
        let n = self.len();
        self.resize(n + 1);
        self.set(n, x);
    }

    pub fn from_vals(ty: AttrType, vals: &[Val]) -> AttrData {
        let mut d = AttrData::new(ty, vals.len());
        for (i, v) in vals.iter().enumerate() {
            d.set(i, *v);
        }
        d
    }

    pub fn to_vals(&self) -> Vec<Val> {
        (0..self.len()).map(|i| self.get(i)).collect()
    }

    /// New data with element `map[i]` at `i` (out-of-range indices give the
    /// type's default).
    pub fn gather(&self, map: &[usize]) -> AttrData {
        if self.is_empty() {
            return AttrData::new(self.ty(), map.len());
        }
        map_each!(self, v => map.iter().map(|&i| v.get(i).copied().unwrap_or_default_like(v)).collect())
    }

    /// Grow with defaults or shrink to `n`.
    pub fn resize(&mut self, n: usize) {
        let d = AttrData::new(self.ty(), 1);
        match (self, d) {
            (AttrData::Float(v), AttrData::Float(d)) => v.resize(n, d[0]),
            (AttrData::Int(v), AttrData::Int(d)) => v.resize(n, d[0]),
            (AttrData::Bool(v), AttrData::Bool(d)) => v.resize(n, d[0]),
            (AttrData::Float2(v), AttrData::Float2(d)) => v.resize(n, d[0]),
            (AttrData::Float3(v), AttrData::Float3(d)) => v.resize(n, d[0]),
            (AttrData::Color(v), AttrData::Color(d)) => v.resize(n, d[0]),
            (AttrData::Quaternion(v), AttrData::Quaternion(d)) => v.resize(n, d[0]),
            (AttrData::Float4x4(v), AttrData::Float4x4(d)) => v.resize(n, d[0]),
            _ => {}
        }
    }

    /// Append `o` (converted to this type).
    pub fn extend(&mut self, o: &AttrData) {
        let n = self.len();
        self.resize(n + o.len());
        for i in 0..o.len() {
            self.set(n + i, o.get(i));
        }
    }

    pub fn converted(&self, ty: AttrType) -> AttrData {
        if ty == self.ty() {
            return self.clone();
        }
        let mut d = AttrData::new(ty, self.len());
        for i in 0..self.len() {
            d.set(i, self.get(i));
        }
        d
    }
}

trait DefaultLike<T> {
    fn unwrap_or_default_like(self, v: &[T]) -> T;
}

impl<T: Copy> DefaultLike<T> for Option<T> {
    fn unwrap_or_default_like(self, v: &[T]) -> T {
        match self {
            Some(x) => x,
            None => v[0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub domain: Domain,
    pub data: AttrData,
}

/// The named attributes of one geometry component.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes {
    pub list: Vec<Attribute>,
}

impl Attributes {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Attribute> {
        self.list.iter().find(|a| a.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Attribute> {
        self.list.iter_mut().find(|a| a.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Store `data` as `name` on `domain`, replacing any attribute of that name.
    pub fn set(&mut self, name: &str, domain: Domain, data: AttrData) {
        match self.get_mut(name) {
            Some(a) => {
                a.domain = domain;
                a.data = data;
            }
            None => self.list.push(Attribute { name: name.into(), domain, data }),
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.list.len();
        self.list.retain(|a| a.name != name);
        n != self.list.len()
    }

    pub fn names(&self) -> Vec<String> {
        self.list.iter().map(|a| a.name.clone()).collect()
    }

    /// Keep only attributes whose length matches their domain's size.
    pub fn retain_valid(&mut self, size: impl Fn(Domain) -> usize) {
        self.list.retain(|a| a.data.len() == size(a.domain));
    }

    /// Rebuild the attributes of `domain` from source indices (`map[i]` is
    /// the source of new element `i`). Other domains are untouched.
    pub fn gather_domain(&mut self, domain: Domain, map: &[usize]) {
        for a in self.list.iter_mut().filter(|a| a.domain == domain) {
            if a.data.is_empty() {
                a.data = AttrData::new(a.data.ty(), map.len());
            } else {
                a.data = a.data.gather(map);
            }
        }
    }

    /// Like [`Attributes::gather_domain`] but each new element mixes several
    /// weighted sources (new vertices on edges, face centres, ...).
    pub fn interp_domain(&mut self, domain: Domain, map: &[Vec<(usize, f32)>]) {
        for a in self.list.iter_mut().filter(|a| a.domain == domain) {
            let ty = a.data.ty();
            let old = &a.data;
            let vals: Vec<Val> = map
                .iter()
                .map(|src| {
                    let v: Vec<(Val, f32)> = src.iter().filter(|(i, _)| *i < old.len()).map(|&(i, w)| (old.get(i), w)).collect();
                    if v.is_empty() {
                        ty.default_val()
                    } else {
                        ty.convert(interp(&v))
                    }
                })
                .collect();
            a.data = AttrData::from_vals(ty, &vals);
        }
    }

    /// Append `o`'s attributes: names present on both are joined; names on
    /// only one side are padded with defaults. `sizes` gives (this, other)
    /// sizes of each domain.
    pub fn join(&mut self, o: &Attributes, sizes: impl Fn(Domain) -> (usize, usize)) {
        for a in &o.list {
            if !self.contains(&a.name) {
                let (n, _) = sizes(a.domain);
                self.list.push(Attribute { name: a.name.clone(), domain: a.domain, data: AttrData::new(a.data.ty(), n) });
            }
        }
        for a in self.list.iter_mut() {
            let (n, m) = sizes(a.domain);
            a.data.resize(n);
            match o.get(&a.name) {
                Some(b) if b.domain == a.domain && b.data.len() == m => a.data.extend(&b.data),
                Some(b) if b.data.len() == m => {
                    // Same name, other domain: keep ours, pad.
                    let _ = b;
                    a.data.resize(n + m);
                }
                _ => a.data.resize(n + m),
            }
        }
    }
}
