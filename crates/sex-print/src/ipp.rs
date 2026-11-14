// SPDX-License-Identifier: GPL-3.0-only
//! IPP/1.1 and IPP/2.0 messages in their binary encoding (RFC 8010), and the
//! requests Needle sends (RFC 8011, PWG 5100.14 IPP Everywhere).
//!
//! A message is a version, an operation (request) or status (response) code, a
//! request id, attribute groups, and whatever document data follows the
//! end-of-attributes tag. Collections (`media-col`) are decoded into nested members
//! so a printer's whole attribute set can be read, not just its flat attributes.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Delimiter tags: the start of each attribute group.
pub mod group {
    pub const OPERATION: u8 = 0x01;
    pub const JOB: u8 = 0x02;
    pub const END: u8 = 0x03;
    pub const PRINTER: u8 = 0x04;
    pub const UNSUPPORTED: u8 = 0x05;
}

/// Value tags.
pub mod tag {
    pub const UNSUPPORTED: u8 = 0x10;
    pub const UNKNOWN: u8 = 0x12;
    pub const NO_VALUE: u8 = 0x13;
    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const OCTET_STRING: u8 = 0x30;
    pub const DATE_TIME: u8 = 0x31;
    pub const RESOLUTION: u8 = 0x32;
    pub const RANGE: u8 = 0x33;
    pub const BEG_COLLECTION: u8 = 0x34;
    pub const TEXT_WITH_LANGUAGE: u8 = 0x35;
    pub const NAME_WITH_LANGUAGE: u8 = 0x36;
    pub const END_COLLECTION: u8 = 0x37;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const URI_SCHEME: u8 = 0x46;
    pub const CHARSET: u8 = 0x47;
    pub const LANGUAGE: u8 = 0x48;
    pub const MIME: u8 = 0x49;
    pub const MEMBER_NAME: u8 = 0x4A;
}

/// Operation codes Needle uses.
pub mod op {
    pub const PRINT_JOB: u16 = 0x0002;
    pub const VALIDATE_JOB: u16 = 0x0004;
    pub const CANCEL_JOB: u16 = 0x0008;
    pub const GET_JOB_ATTRIBUTES: u16 = 0x0009;
    pub const GET_JOBS: u16 = 0x000A;
    pub const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
}

/// Status codes (the common ones; anything below 0x0100 is success).
pub mod status {
    pub const OK: u16 = 0x0000;
    pub const OK_IGNORED_OR_SUBSTITUTED: u16 = 0x0001;
    pub const OK_CONFLICTING: u16 = 0x0002;
    pub const BAD_REQUEST: u16 = 0x0400;
    pub const FORBIDDEN: u16 = 0x0401;
    pub const NOT_FOUND: u16 = 0x0406;
    pub const NOT_POSSIBLE: u16 = 0x0404;
    pub const DOCUMENT_FORMAT_NOT_SUPPORTED: u16 = 0x040A;
    pub const ATTRIBUTES_OR_VALUES_NOT_SUPPORTED: u16 = 0x040B;
    pub const INTERNAL_ERROR: u16 = 0x0500;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x0501;
    pub const BUSY: u16 = 0x0507;

    pub fn name(code: u16) -> &'static str {
        match code {
            OK => "successful-ok",
            OK_IGNORED_OR_SUBSTITUTED => "successful-ok-ignored-or-substituted-attributes",
            OK_CONFLICTING => "successful-ok-conflicting-attributes",
            BAD_REQUEST => "client-error-bad-request",
            FORBIDDEN => "client-error-forbidden",
            NOT_POSSIBLE => "client-error-not-possible",
            NOT_FOUND => "client-error-not-found",
            DOCUMENT_FORMAT_NOT_SUPPORTED => "client-error-document-format-not-supported",
            ATTRIBUTES_OR_VALUES_NOT_SUPPORTED => "client-error-attributes-or-values-not-supported",
            INTERNAL_ERROR => "server-error-internal-error",
            OPERATION_NOT_SUPPORTED => "server-error-operation-not-supported",
            BUSY => "server-error-busy",
            c if c < 0x0100 => "successful-ok",
            c if c < 0x0500 => "client-error",
            _ => "server-error",
        }
    }
}

/// `job-state` values (RFC 8011 5.3.7).
pub fn job_state_name(state: i32) -> &'static str {
    match state {
        3 => "pending",
        4 => "pending-held",
        5 => "processing",
        6 => "processing-stopped",
        7 => "canceled",
        8 => "aborted",
        9 => "completed",
        _ => "unknown",
    }
}
/// `printer-state` values.
pub fn printer_state_name(state: i32) -> &'static str {
    match state { 3 => "idle", 4 => "processing", 5 => "stopped", _ => "unknown" }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Integer(i32),
    Enum(i32),
    Boolean(bool),
    /// Text, name, keyword, uri, uriScheme, charset, naturalLanguage, mimeMediaType.
    Str(u8, String),
    /// octetString, dateTime, the with-language forms, and tags this code doesn't know.
    Octets(u8, Vec<u8>),
    Resolution { x: i32, y: i32, units: u8 },
    Range(i32, i32),
    Collection(Vec<Attribute>),
    /// unsupported, unknown, no-value.
    OutOfBand(u8),
}

impl Value {
    pub fn keyword(s: &str) -> Self { Value::Str(tag::KEYWORD, s.to_string()) }
    pub fn text(s: &str) -> Self { Value::Str(tag::TEXT, s.to_string()) }
    pub fn name(s: &str) -> Self { Value::Str(tag::NAME, s.to_string()) }
    pub fn uri(s: &str) -> Self { Value::Str(tag::URI, s.to_string()) }
    pub fn mime(s: &str) -> Self { Value::Str(tag::MIME, s.to_string()) }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(_, s) => Some(s),
            // textWithLanguage: u16 language length, language, u16 text length, text.
            Value::Octets(t, b) if *t == tag::TEXT_WITH_LANGUAGE || *t == tag::NAME_WITH_LANGUAGE => {
                let l = u16::from_be_bytes([*b.first()?, *b.get(1)?]) as usize;
                let rest = b.get(2 + l..)?;
                let n = u16::from_be_bytes([*rest.first()?, *rest.get(1)?]) as usize;
                core::str::from_utf8(rest.get(2..2 + n)?).ok()
            }
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i32> {
        match self { Value::Integer(v) | Value::Enum(v) => Some(*v), _ => None }
    }
    pub fn as_bool(&self) -> Option<bool> { if let Value::Boolean(b) = self { Some(*b) } else { None } }

    fn tag(&self) -> u8 {
        match self {
            Value::Integer(_) => tag::INTEGER,
            Value::Enum(_) => tag::ENUM,
            Value::Boolean(_) => tag::BOOLEAN,
            Value::Str(t, _) | Value::Octets(t, _) => *t,
            Value::Resolution { .. } => tag::RESOLUTION,
            Value::Range(..) => tag::RANGE,
            Value::Collection(_) => tag::BEG_COLLECTION,
            Value::OutOfBand(t) => *t,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub values: Vec<Value>,
}
impl Attribute {
    pub fn new(name: &str, value: Value) -> Self { Self { name: name.to_string(), values: alloc::vec![value] } }
    pub fn many(name: &str, values: Vec<Value>) -> Self { Self { name: name.to_string(), values } }
    pub fn first(&self) -> Option<&Value> { self.values.first() }
    /// Every value that reads as a string (keywords, names, mime types, uris...).
    pub fn strings(&self) -> Vec<&str> { self.values.iter().filter_map(Value::as_str).collect() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub tag: u8,
    pub attributes: Vec<Attribute>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// (major, minor): 1.1 or 2.0.
    pub version: (u8, u8),
    /// Operation id in a request, status code in a response.
    pub code: u16,
    pub request_id: u32,
    pub groups: Vec<Group>,
    /// Document bytes after the attributes (Print-Job).
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    /// An attribute before any group, a value with no name to belong to, or a
    /// collection that never ends.
    Malformed,
    BadUtf8,
}

impl Message {
    pub fn new(code: u16, request_id: u32) -> Self {
        Self { version: (2, 0), code, request_id, groups: Vec::new(), data: Vec::new() }
    }

    pub fn group(&self, tag: u8) -> Option<&Group> { self.groups.iter().find(|g| g.tag == tag) }
    pub fn groups_of(&self, tag: u8) -> impl Iterator<Item = &Group> { self.groups.iter().filter(move |g| g.tag == tag) }
    fn group_mut(&mut self, tag: u8) -> &mut Group {
        if let Some(i) = self.groups.iter().position(|g| g.tag == tag) { return &mut self.groups[i]; }
        self.groups.push(Group { tag, attributes: Vec::new() });
        self.groups.last_mut().unwrap()
    }
    pub fn push(&mut self, group: u8, attr: Attribute) -> &mut Self { self.group_mut(group).attributes.push(attr); self }

    /// The first attribute of that name in any group.
    pub fn attr(&self, name: &str) -> Option<&Attribute> {
        self.groups.iter().flat_map(|g| g.attributes.iter()).find(|a| a.name == name)
    }
    pub fn is_success(&self) -> bool { self.code < 0x0100 }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[self.version.0, self.version.1]);
        out.extend_from_slice(&self.code.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());
        for g in &self.groups {
            out.push(g.tag);
            for a in &g.attributes { encode_attr(&mut out, &a.name, &a.values); }
        }
        out.push(group::END);
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let head = bytes.get(..8).ok_or(Error::Truncated)?;
        let mut m = Message {
            version: (head[0], head[1]),
            code: u16::from_be_bytes([head[2], head[3]]),
            request_id: u32::from_be_bytes([head[4], head[5], head[6], head[7]]),
            groups: Vec::new(),
            data: Vec::new(),
        };
        let mut r = Reader { b: bytes, at: 8 };
        loop {
            let t = r.u8()?;
            if t == group::END { break; }
            if t < 0x10 {
                m.groups.push(Group { tag: t, attributes: Vec::new() });
                continue;
            }
            let (name, value) = r.attr_head(t)?;
            let g = m.groups.last_mut().ok_or(Error::Malformed)?;
            let value = if t == tag::BEG_COLLECTION { r.collection()? } else { value };
            if name.is_empty() {
                g.attributes.last_mut().ok_or(Error::Malformed)?.values.push(value);
            } else {
                g.attributes.push(Attribute { name, values: alloc::vec![value] });
            }
        }
        m.data = bytes[r.at..].to_vec();
        Ok(m)
    }
}

fn put_str(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(b);
}

fn encode_attr(out: &mut Vec<u8>, name: &str, values: &[Value]) {
    for (i, v) in values.iter().enumerate() {
        out.push(v.tag());
        put_str(out, if i == 0 { name.as_bytes() } else { b"" });
        match v {
            Value::Integer(n) | Value::Enum(n) => put_str(out, &n.to_be_bytes()),
            Value::Boolean(b) => put_str(out, &[*b as u8]),
            Value::Str(_, s) => put_str(out, s.as_bytes()),
            Value::Octets(_, b) => put_str(out, b),
            Value::Resolution { x, y, units } => {
                let mut b = [0u8; 9];
                b[..4].copy_from_slice(&x.to_be_bytes());
                b[4..8].copy_from_slice(&y.to_be_bytes());
                b[8] = *units;
                put_str(out, &b);
            }
            Value::Range(lo, hi) => {
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&lo.to_be_bytes());
                b[4..].copy_from_slice(&hi.to_be_bytes());
                put_str(out, &b);
            }
            Value::OutOfBand(_) => put_str(out, b""),
            Value::Collection(members) => {
                put_str(out, b"");
                for m in members {
                    out.push(tag::MEMBER_NAME);
                    put_str(out, b"");
                    put_str(out, m.name.as_bytes());
                    // Member values carry no name; they follow the member name.
                    encode_attr(out, "", &m.values);
                }
                out.push(tag::END_COLLECTION);
                put_str(out, b"");
                put_str(out, b"");
            }
        }
    }
}

struct Reader<'a> { b: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, Error> { let v = *self.b.get(self.at).ok_or(Error::Truncated)?; self.at += 1; Ok(v) }
    fn chunk(&mut self) -> Result<&'a [u8], Error> {
        let l = self.b.get(self.at..self.at + 2).ok_or(Error::Truncated)?;
        let n = u16::from_be_bytes([l[0], l[1]]) as usize;
        let s = self.b.get(self.at + 2..self.at + 2 + n).ok_or(Error::Truncated)?;
        self.at += 2 + n;
        Ok(s)
    }
    /// Name and value of one attribute-with-one-value whose tag was just read.
    fn attr_head(&mut self, t: u8) -> Result<(String, Value), Error> {
        let name = core::str::from_utf8(self.chunk()?).map_err(|_| Error::BadUtf8)?.to_string();
        let v = self.chunk()?;
        Ok((name, value_of(t, v)?))
    }
    fn collection(&mut self) -> Result<Value, Error> {
        let mut members: Vec<Attribute> = Vec::new();
        loop {
            let t = self.u8()?;
            if t < 0x10 { return Err(Error::Malformed); }
            let _name = self.chunk()?;
            let v = self.chunk()?;
            match t {
                tag::END_COLLECTION => return Ok(Value::Collection(members)),
                tag::MEMBER_NAME => {
                    let n = core::str::from_utf8(v).map_err(|_| Error::BadUtf8)?.to_string();
                    members.push(Attribute { name: n, values: Vec::new() });
                }
                tag::BEG_COLLECTION => {
                    let inner = self.collection()?;
                    members.last_mut().ok_or(Error::Malformed)?.values.push(inner);
                }
                _ => members.last_mut().ok_or(Error::Malformed)?.values.push(value_of(t, v)?),
            }
        }
    }
}

fn be32(v: &[u8]) -> i32 { i32::from_be_bytes([v[0], v[1], v[2], v[3]]) }

fn value_of(t: u8, v: &[u8]) -> Result<Value, Error> {
    Ok(match t {
        tag::INTEGER | tag::ENUM if v.len() == 4 => if t == tag::INTEGER { Value::Integer(be32(v)) } else { Value::Enum(be32(v)) },
        tag::BOOLEAN if v.len() == 1 => Value::Boolean(v[0] != 0),
        tag::RESOLUTION if v.len() == 9 => Value::Resolution { x: be32(v), y: be32(&v[4..]), units: v[8] },
        tag::RANGE if v.len() == 8 => Value::Range(be32(v), be32(&v[4..])),
        tag::UNSUPPORTED..=0x1F => Value::OutOfBand(t),
        tag::BEG_COLLECTION => Value::Collection(Vec::new()),
        tag::TEXT | tag::NAME | tag::KEYWORD | tag::URI | tag::URI_SCHEME | tag::CHARSET | tag::LANGUAGE | tag::MIME | tag::MEMBER_NAME
        | 0x40 | 0x43 => Value::Str(t, core::str::from_utf8(v).map_err(|_| Error::BadUtf8)?.to_string()),
        _ => Value::Octets(t, v.to_vec()),
    })
}

/// Options for Print-Job and Validate-Job; `None` leaves the printer's default.
#[derive(Clone, Debug, Default)]
pub struct JobOptions {
    pub job_name: String,
    pub format: String,
    pub copies: Option<i32>,
    pub sides: Option<String>,
    pub color_mode: Option<String>,
    pub media: Option<String>,
    pub resolution: Option<(i32, i32)>,
}

/// Builds the requests Needle sends; request ids count up from 1.
pub struct Client {
    pub printer_uri: String,
    pub user: String,
    next_id: u32,
}

impl Client {
    pub fn new(printer_uri: &str, user: &str) -> Self {
        Self { printer_uri: printer_uri.to_string(), user: user.to_string(), next_id: 1 }
    }

    /// A request with the operation attributes every request starts with (RFC 8011 4.1.4).
    pub fn request(&mut self, op: u16) -> Message {
        let mut m = Message::new(op, self.next_id);
        self.next_id += 1;
        m.push(group::OPERATION, Attribute::new("attributes-charset", Value::Str(tag::CHARSET, "utf-8".into())));
        m.push(group::OPERATION, Attribute::new("attributes-natural-language", Value::Str(tag::LANGUAGE, "en".into())));
        m.push(group::OPERATION, Attribute::new("printer-uri", Value::uri(&self.printer_uri)));
        m.push(group::OPERATION, Attribute::new("requesting-user-name", Value::name(&self.user)));
        m
    }

    pub fn get_printer_attributes(&mut self, requested: &[&str]) -> Message {
        let mut m = self.request(op::GET_PRINTER_ATTRIBUTES);
        if !requested.is_empty() {
            m.push(group::OPERATION, Attribute::many("requested-attributes", requested.iter().map(|s| Value::keyword(s)).collect()));
        }
        m
    }

    fn job_request(&mut self, operation: u16, o: &JobOptions) -> Message {
        let mut m = self.request(operation);
        m.push(group::OPERATION, Attribute::new("job-name", Value::name(&o.job_name)));
        m.push(group::OPERATION, Attribute::new("document-format", Value::mime(&o.format)));
        let mut job = Vec::new();
        if let Some(c) = o.copies { job.push(Attribute::new("copies", Value::Integer(c))); }
        if let Some(s) = &o.sides { job.push(Attribute::new("sides", Value::keyword(s))); }
        if let Some(c) = &o.color_mode { job.push(Attribute::new("print-color-mode", Value::keyword(c))); }
        if let Some(md) = &o.media { job.push(Attribute::new("media", Value::keyword(md))); }
        if let Some((x, y)) = o.resolution { job.push(Attribute::new("printer-resolution", Value::Resolution { x, y, units: 3 })); }
        if !job.is_empty() { m.groups.push(Group { tag: group::JOB, attributes: job }); }
        m
    }

    pub fn validate_job(&mut self, o: &JobOptions) -> Message { self.job_request(op::VALIDATE_JOB, o) }

    pub fn print_job(&mut self, o: &JobOptions, document: Vec<u8>) -> Message {
        let mut m = self.job_request(op::PRINT_JOB, o);
        m.data = document;
        m
    }

    /// `which` is "completed", "not-completed" or "all" (IPP/2.0).
    pub fn get_jobs(&mut self, which: &str, requested: &[&str]) -> Message {
        let mut m = self.request(op::GET_JOBS);
        m.push(group::OPERATION, Attribute::new("which-jobs", Value::keyword(which)));
        if !requested.is_empty() {
            m.push(group::OPERATION, Attribute::many("requested-attributes", requested.iter().map(|s| Value::keyword(s)).collect()));
        }
        m
    }

    pub fn get_job_attributes(&mut self, job_id: i32) -> Message {
        let mut m = self.request(op::GET_JOB_ATTRIBUTES);
        m.push(group::OPERATION, Attribute::new("job-id", Value::Integer(job_id)));
        m
    }

    pub fn cancel_job(&mut self, job_id: i32) -> Message {
        let mut m = self.request(op::CANCEL_JOB);
        m.push(group::OPERATION, Attribute::new("job-id", Value::Integer(job_id)));
        m
    }
}

/// One job from a Get-Jobs or Get-Job-Attributes response (or a Print-Job's answer).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Job {
    pub id: i32,
    pub state: i32,
    pub name: String,
    pub uri: String,
    pub reasons: Vec<String>,
}
impl Job {
    pub fn state_name(&self) -> &'static str { job_state_name(self.state) }
    pub fn finished(&self) -> bool { matches!(self.state, 7..=9) }
    fn from_group(g: &Group) -> Self {
        let mut j = Job::default();
        for a in &g.attributes {
            match a.name.as_str() {
                "job-id" => j.id = a.first().and_then(Value::as_int).unwrap_or(0),
                "job-state" => j.state = a.first().and_then(Value::as_int).unwrap_or(0),
                "job-name" => j.name = a.first().and_then(Value::as_str).unwrap_or("").to_string(),
                "job-uri" => j.uri = a.first().and_then(Value::as_str).unwrap_or("").to_string(),
                "job-state-reasons" => j.reasons = a.strings().into_iter().map(String::from).collect(),
                _ => {}
            }
        }
        j
    }
}
/// Every job group in a response, in order.
pub fn jobs(m: &Message) -> Vec<Job> { m.groups_of(group::JOB).map(Job::from_group).collect() }

/// The `status-message` operation attribute, if the printer gave one.
pub fn status_message(m: &Message) -> Option<&str> {
    m.group(group::OPERATION)?.attributes.iter().find(|a| a.name == "status-message")?.first()?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_round_trip() {
        let size = Value::Collection(alloc::vec![
            Attribute::new("x-dimension", Value::Integer(21000)),
            Attribute::new("y-dimension", Value::Integer(29700)),
        ]);
        let col = Value::Collection(alloc::vec![
            Attribute::new("media-size", size),
            Attribute::new("media-source", Value::keyword("main")),
        ]);
        let mut m = Message::new(op::PRINT_JOB, 7);
        m.push(group::JOB, Attribute::many("media-col", alloc::vec![col.clone(), col]));
        let back = Message::decode(&m.encode()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn text_with_language() {
        let v = Value::Octets(tag::TEXT_WITH_LANGUAGE, [&[0, 2][..], b"en", &[0, 3], b"hey"].concat());
        assert_eq!(v.as_str(), Some("hey"));
    }

    #[test]
    fn truncated_is_an_error() {
        let mut c = Client::new("ipp://p/ipp/print", "u");
        let b = c.get_printer_attributes(&[]).encode();
        for n in 0..b.len() - 1 { assert!(Message::decode(&b[..n]).is_err(), "cut at {n}"); }
    }
}
