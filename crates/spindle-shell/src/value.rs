// SPDX-License-Identifier: GPL-3.0-only
//! The values that flow through pipelines.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::error::{ShellError, Span};
use crate::units;

/// A value. A table is a `List` of `Record`s.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nothing,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Bytes.
    Filesize(i64),
    /// Nanoseconds.
    Duration(i64),
    /// Unix seconds, UTC.
    Date(i64),
    String(String),
    Binary(Vec<u8>),
    List(Vec<Value>),
    Record(Record),
    /// A failure kept as a value, such as one row that could not be read.
    Error(Box<ShellError>),
}

/// Named fields in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    cols: Vec<String>,
    vals: Vec<Value>,
}

impl Record {
    pub fn new() -> Record { Record::default() }
    /// Sets a field, replacing one of the same name in place.
    pub fn insert(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        if let Some(i) = self.cols.iter().position(|c| *c == key) { self.vals[i] = value; }
        else { self.cols.push(key); self.vals.push(value); }
    }
    pub fn with(mut self, key: impl Into<String>, value: Value) -> Record {
        self.insert(key, value);
        self
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.cols.iter().position(|c| c == key).map(|i| &self.vals[i])
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.cols.iter().position(|c| c == key).map(move |i| &mut self.vals[i])
    }
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.cols.iter().position(|c| c == key)?;
        self.cols.remove(i);
        Some(self.vals.remove(i))
    }
    pub fn contains(&self, key: &str) -> bool { self.cols.iter().any(|c| c == key) }
    pub fn columns(&self) -> &[String] { &self.cols }
    pub fn values(&self) -> &[Value] { &self.vals }
    pub fn len(&self) -> usize { self.cols.len() }
    pub fn is_empty(&self) -> bool { self.cols.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> { self.cols.iter().zip(self.vals.iter()) }
    pub fn into_iter_pairs(self) -> impl Iterator<Item = (String, Value)> { self.cols.into_iter().zip(self.vals) }
}

impl FromIterator<(String, Value)> for Record {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Record {
        let mut r = Record::new();
        for (k, v) in iter { r.insert(k, v); }
        r
    }
}

/// One step of a cell path: `name` or `0`.
#[derive(Clone, Debug, PartialEq)]
pub enum PathMember {
    Field(String, Span),
    Index(usize, Span),
}

impl PathMember {
    pub fn span(&self) -> Span {
        match self { PathMember::Field(_, s) | PathMember::Index(_, s) => *s }
    }
    /// Parses `name.0.field` into members, with spans starting at `offset`.
    pub fn parse_path(text: &str, offset: usize) -> Vec<PathMember> {
        let mut out = Vec::new();
        let mut at = offset;
        for part in text.split('.') {
            let span = Span::new(at, at + part.len());
            at += part.len() + 1;
            if part.is_empty() { continue; }
            match part.parse::<usize>() {
                Ok(i) => out.push(PathMember::Index(i, span)),
                Err(_) => out.push(PathMember::Field(part.to_string(), span)),
            }
        }
        out
    }
}

impl Value {
    pub fn string(s: impl Into<String>) -> Value { Value::String(s.into()) }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nothing => "nothing",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Filesize(_) => "filesize",
            Value::Duration(_) => "duration",
            Value::Date(_) => "date",
            Value::String(_) => "string",
            Value::Binary(_) => "binary",
            Value::List(_) if self.is_table() => "table",
            Value::List(_) => "list",
            Value::Record(_) => "record",
            Value::Error(_) => "error",
        }
    }

    /// A non-empty list whose items are all records.
    pub fn is_table(&self) -> bool {
        matches!(self, Value::List(items) if !items.is_empty() && items.iter().all(|v| matches!(v, Value::Record(_))))
    }

    /// The columns of a table (every column any row has, in first-seen order) or a record.
    pub fn columns(&self) -> Vec<String> {
        let mut cols: Vec<String> = Vec::new();
        match self {
            Value::Record(r) => cols.extend(r.columns().iter().cloned()),
            Value::List(items) => for item in items {
                if let Value::Record(r) = item {
                    for c in r.columns() { if !cols.contains(c) { cols.push(c.clone()); } }
                }
            },
            _ => {}
        }
        cols
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self { Some(s) } else { None }
    }

    /// The value in a table cell or a one-line summary: nested values abbreviated.
    pub fn to_abbreviated(&self) -> String {
        match self {
            Value::List(items) if self.is_table() => format!("[table {} row{}]", items.len(), plural(items.len())),
            Value::List(items) => format!("[list {} item{}]", items.len(), plural(items.len())),
            Value::Record(r) => format!("{{record {} field{}}}", r.len(), plural(r.len())),
            Value::Binary(b) => format!("[binary {}]", units::format_filesize(b.len() as i64)),
            _ => self.to_text(),
        }
    }

    /// Plain text for strings, programs and files: scalars as shown, lists one item a
    /// line, records `key: value` a line.
    pub fn to_text(&self) -> String {
        match self {
            Value::Nothing => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => units::format_float(*f),
            Value::Filesize(b) => units::format_filesize(*b),
            Value::Duration(d) => units::format_duration(*d),
            Value::Date(d) => units::format_date(*d),
            Value::String(s) => s.clone(),
            Value::Binary(b) => String::from_utf8_lossy(b).into_owned(),
            Value::List(items) if self.is_table() => crate::render::render_plain(self),
            Value::List(items) => items.iter().map(|v| v.to_abbreviated()).collect::<Vec<_>>().join("\n"),
            Value::Record(r) => r.iter().map(|(k, v)| format!("{}: {}", k, v.to_abbreviated())).collect::<Vec<_>>().join("\n"),
            Value::Error(e) => format!("error: {}", e.message),
        }
    }

    /// Follows a cell path: a field of a record, a row of a list, or a column of a table.
    pub fn follow(&self, path: &[PathMember]) -> Result<Value, ShellError> {
        let mut cur = self.clone();
        for m in path {
            cur = match (&cur, m) {
                (Value::Record(r), PathMember::Field(name, span)) => match r.get(name) {
                    Some(v) => v.clone(),
                    None => return Err(no_column(name, *span, &cur)),
                },
                (Value::List(items), PathMember::Index(i, span)) => match items.get(*i) {
                    Some(v) => v.clone(),
                    None => return Err(ShellError::at(format!("row {} is past the end", i), *span, format!("there {} {} row{}", if items.len() == 1 { "is" } else { "are" }, items.len(), plural(items.len())))),
                },
                (Value::List(items), PathMember::Field(name, span)) => {
                    let mut col = Vec::new();
                    for item in items {
                        match item {
                            Value::Record(r) => match r.get(name) {
                                Some(v) => col.push(v.clone()),
                                None => return Err(no_column(name, *span, &cur)),
                            },
                            other => return Err(ShellError::at(format!("can't take column `{}` of a {}", name, other.type_name()), *span, "not a table")),
                        }
                    }
                    Value::List(col)
                }
                (Value::Record(r), PathMember::Index(i, span)) => match r.values().get(*i) {
                    Some(v) => v.clone(),
                    None => return Err(ShellError::at(format!("field {} is past the end", i), *span, "no such field")),
                },
                (other, m) => return Err(ShellError::at(format!("can't follow a cell path into a {}", other.type_name()), m.span(), "nothing to look into here")),
            };
        }
        Ok(cur)
    }

    /// Ordering of comparable values (numbers with numbers, strings with strings, ...).
    pub fn compare(&self, other: &Value) -> Option<Ordering> {
        use Value::*;
        match (self, other) {
            (Int(a), Int(b)) => Some(a.cmp(b)),
            (Float(a), Float(b)) => a.partial_cmp(b),
            (Int(a), Float(b)) => (*a as f64).partial_cmp(b),
            (Float(a), Int(b)) => a.partial_cmp(&(*b as f64)),
            (Filesize(a), Filesize(b)) | (Duration(a), Duration(b)) | (Date(a), Date(b)) => Some(a.cmp(b)),
            (Filesize(a), Int(b)) | (Int(a), Filesize(b)) => Some(a.cmp(b)),
            (String(a), String(b)) => Some(a.cmp(b)),
            (Bool(a), Bool(b)) => Some(a.cmp(b)),
            (Nothing, Nothing) => Some(Ordering::Equal),
            (Binary(a), Binary(b)) => Some(a.cmp(b)),
            (List(a), List(b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    match x.compare(y)? { Ordering::Equal => continue, o => return Some(o) }
                }
                Some(a.len().cmp(&b.len()))
            }
            _ => None,
        }
    }

    /// A total order for sorting: comparable values by value, others by type.
    pub fn sort_cmp(&self, other: &Value) -> Ordering {
        self.compare(other).unwrap_or_else(|| rank(self).cmp(&rank(other)))
    }

    /// Equality as `==` sees it: `1 == 1.0`.
    pub fn loose_eq(&self, other: &Value) -> bool {
        match self.compare(other) {
            Some(o) => o == Ordering::Equal,
            None => self == other,
        }
    }
}

fn rank(v: &Value) -> u8 {
    match v {
        Value::Bool(_) => 0,
        Value::Int(_) | Value::Float(_) => 1,
        Value::Filesize(_) => 2,
        Value::Duration(_) => 3,
        Value::Date(_) => 4,
        Value::String(_) => 5,
        Value::Binary(_) => 6,
        Value::List(_) => 7,
        Value::Record(_) => 8,
        Value::Error(_) => 9,
        Value::Nothing => 10,
    }
}

pub(crate) fn plural(n: usize) -> &'static str { if n == 1 { "" } else { "s" } }

pub(crate) fn no_column(name: &str, span: Span, value: &Value) -> ShellError {
    let cols = value.columns();
    let err = ShellError::at(format!("no column `{}`", name), span, "not found");
    match crate::suggest::closest(name, cols.iter().map(|s| s.as_str())) {
        Some(best) => err.help(format!("did you mean `{}`?", best)),
        None if !cols.is_empty() => err.help(format!("the columns are {}", cols.join(", "))),
        None => err,
    }
}
