// SPDX-License-Identifier: GPL-3.0-only
//! polyops: the line-based text command language that Polyamour's Python API
//! (`apps/polyamour/python/polyamour`) speaks, and the scene text format.
//!
//! A Python script cannot link into the Polyamour process, so the `polyamour`
//! (and `bpy`) package turns API calls into polyops lines; Polyamour starts
//! `python` with the script, reads the stream (a file today, an IPC channel
//! later) and executes it here against a [`Scene`]. The full description is in
//! `docs/handoff/POLYAMOUR_PYTHON_2026_11_14.md`; in short:
//!
//! * UTF-8 lines. The first line that is not blank or a comment is `polyops 1`.
//!   A token starting with `#` (unquoted) ends the line as a comment.
//! * A line is a command word then tokens separated by blanks. A token is bare
//!   text or `"quoted text"` (escapes `\" \\ \n \t \r`), or a mix (`a"b c"`).
//!   `key=value` (key an identifier, `=` outside quotes) is an option; the rest
//!   are positional arguments. Lists are comma-separated (`location=1,2,3`).
//! * Objects, meshes, materials and collections are named by their names, or by
//!   a handle: `as=$7` on a creating command binds the unquoted token `$7` to
//!   what it created, whatever unique name it got (`Cube.001`). Quoted text is
//!   never a handle, never `none`, never an option.
//! * `@ID` before the command word asks for a reply line `@ID ok ...`; queries
//!   (`get`, `list`, `exists`, `ping`) always reply. Replies are collected in
//!   [`RunReport::replies`], so a one-way stream works today and the same lines
//!   can travel back over a channel later.
//! * `end` marks a complete stream, `abort MESSAGE` fails the whole run (the
//!   Python side writes it when the script raised).
//!
//! [`run`] is atomic: it executes on a copy and only replaces the scene when every
//! line succeeded, so the caller records one undo step per script (push the
//! scene on the [`crate::undo::UndoStack`] before calling). [`Session`] is the
//! line-at-a-time engine underneath, for a live channel. [`scene_to_polyops`]
//! writes a scene back out as commands; running that text on [`Scene::new`]
//! rebuilds an equal scene (images and textures excepted).
use crate::math::{Quat, Vec2, Vec3};
use crate::mesh::{Face, Mesh};
use crate::modifiers::Modifier;
use crate::scene::{unique_name, Camera, Light, LightKind, Material, Object, ObjectData, Scene, Transform};
use base64::Engine;
use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Write as _};

/// The language version this module reads and writes (`polyops 1`).
pub const VERSION: u32 = 1;

/// Meshes with more vertices than this are written with `b64:` arrays.
const BASE64_ABOVE: usize = 64;

/// A failed line: 1-based line number and what went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ScriptError {}

/// What a successful run did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunReport {
    /// The version named by the `polyops` header.
    pub version: u32,
    /// Commands executed (header, blank and comment lines not counted).
    pub commands: usize,
    /// Names of the objects created, in order (deleted ones included).
    pub created: Vec<String>,
    /// Reply lines for queries and `@ID` requests, in order.
    pub replies: Vec<String>,
    /// `render key=value` settings, stored for the caller (the scene has no
    /// render settings of its own yet).
    pub render: BTreeMap<String, String>,
    /// True when the stream ended with `end`. A Python-produced stream that is
    /// not complete was cut short (the interpreter died) and should not be trusted.
    pub complete: bool,
}

/// Run a polyops program against `scene`, all or nothing: on error the scene is
/// left untouched and the error names the line.
pub fn run(scene: &mut Scene, src: &str) -> Result<RunReport, ScriptError> {
    let mut work = scene.clone();
    let mut session = Session::new();
    for line in src.lines() {
        session.exec_line(&mut work, line)?;
    }
    let report = session.finish()?;
    *scene = work;
    Ok(report)
}

/// Quote `s` as a polyops token when it is not a safe bare word.
pub fn quote(s: &str) -> String {
    let bare = !s.is_empty()
        && s != "none"
        && !s.starts_with(['$', '#', '@'])
        && s.chars().all(|c| c.is_alphanumeric() || "_-.+:/!?%&*<>^~'[]()".contains(c));
    if bare {
        return s.to_string();
    }
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ---------------------------------------------------------------------------
// Tokens

/// One token of a line.
#[derive(Clone, Debug, PartialEq)]
struct Token {
    /// `Some(key)` for `key=value` options.
    key: Option<String>,
    /// The (unescaped) value.
    text: String,
    /// Any part of the value was quoted: never a handle, keyword or option.
    quoted: bool,
}

impl Token {
    fn is_word(&self, w: &str) -> bool {
        !self.quoted && self.text == w
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_') && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn tokenize(line: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut it = line.chars().peekable();
    loop {
        while it.peek().is_some_and(|c| c.is_whitespace()) {
            it.next();
        }
        match it.peek() {
            None | Some('#') => break,
            _ => {}
        }
        let mut tok = Token { key: None, text: String::new(), quoted: false };
        while let Some(&c) = it.peek() {
            if c.is_whitespace() {
                break;
            }
            it.next();
            match c {
                '"' => {
                    tok.quoted = true;
                    loop {
                        match it.next() {
                            None => return Err("unterminated quoted string".into()),
                            Some('"') => break,
                            Some('\\') => match it.next() {
                                Some('"') => tok.text.push('"'),
                                Some('\\') => tok.text.push('\\'),
                                Some('n') => tok.text.push('\n'),
                                Some('t') => tok.text.push('\t'),
                                Some('r') => tok.text.push('\r'),
                                Some(e) => return Err(format!("unknown escape \\{e} in a quoted string")),
                                None => return Err("unterminated quoted string".into()),
                            },
                            Some(ch) => tok.text.push(ch),
                        }
                    }
                }
                '=' if tok.key.is_none() && !tok.quoted && is_ident(&tok.text) => tok.key = Some(std::mem::take(&mut tok.text)),
                c => tok.text.push(c),
            }
        }
        out.push(tok);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Value parsing

type Res<T> = Result<T, String>;

fn num(t: &str) -> Res<f32> {
    match t.trim().parse::<f32>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => Err(format!("'{t}' is not a finite number")),
    }
}

fn uint(t: &str) -> Res<u32> {
    t.trim().parse::<u32>().map_err(|_| format!("'{t}' is not a whole number"))
}

fn boolean(t: &str) -> Res<bool> {
    match t.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("'{t}' is not a boolean (1/0, true/false)")),
    }
}

/// A comma-separated float list, or `b64:` little-endian f32s.
fn floats(t: &str) -> Res<Vec<f32>> {
    if let Some(b) = t.strip_prefix("b64:") {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b.trim()).map_err(|e| format!("bad base64: {e}"))?;
        if bytes.len() % 4 != 0 {
            return Err("base64 float array length is not a multiple of 4".into());
        }
        let v: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if v.iter().any(|x| !x.is_finite()) {
            return Err("base64 float array holds a non-finite number".into());
        }
        return Ok(v);
    }
    if t.trim().is_empty() {
        return Ok(Vec::new());
    }
    t.split(',').map(num).collect()
}

/// A comma-separated index list, or `b64:` little-endian u32s.
fn uints(t: &str) -> Res<Vec<u32>> {
    if let Some(b) = t.strip_prefix("b64:") {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b.trim()).map_err(|e| format!("bad base64: {e}"))?;
        if bytes.len() % 4 != 0 {
            return Err("base64 index array length is not a multiple of 4".into());
        }
        return Ok(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
    }
    if t.trim().is_empty() {
        return Ok(Vec::new());
    }
    t.split(',').map(uint).collect()
}

/// Polygons: `0,1,2;2,3,0` or `b64:` u32 runs of `count, v0 .. v(count-1)`.
fn polygons(t: &str) -> Res<Vec<Vec<u32>>> {
    if t.starts_with("b64:") {
        let flat = uints(t)?;
        let mut out = Vec::new();
        let mut i = 0;
        while i < flat.len() {
            let n = flat[i] as usize;
            let end = i + 1 + n;
            if end > flat.len() {
                return Err("base64 face array ends inside a face".into());
            }
            out.push(flat[i + 1..end].to_vec());
            i = end;
        }
        return Ok(out);
    }
    t.split(';').filter(|f| !f.trim().is_empty()).map(uints).collect()
}

fn vec3_of(v: &[f32], what: &str) -> Res<Vec3> {
    match v {
        [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
        _ => Err(format!("{what} needs 3 numbers, got {}", v.len())),
    }
}

/// Values of a property: positional tokens, each possibly comma-separated.
fn values(toks: &[Token]) -> Res<Vec<f32>> {
    let mut v = Vec::new();
    for t in toks {
        v.extend(floats(&t.text)?);
    }
    Ok(v)
}

fn fmt_f(v: f32) -> String {
    // Rust prints the shortest text that reads back to the same f32.
    format!("{v}")
}

fn fmt_list(v: &[f32]) -> String {
    v.iter().map(|x| fmt_f(*x)).collect::<Vec<_>>().join(",")
}

fn b64_f32(v: &[f32]) -> String {
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    format!("b64:{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn b64_u32(v: &[u32]) -> String {
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    format!("b64:{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Blender XYZ Euler angles (radians) of a rotation.
pub fn quat_to_euler_xyz(q: Quat) -> [f32; 3] {
    let q = q.normalize();
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let r00 = 1.0 - 2.0 * (y * y + z * z);
    let r10 = 2.0 * (x * y + w * z);
    let r20 = 2.0 * (x * z - w * y);
    let r21 = 2.0 * (y * z + w * x);
    let r22 = 1.0 - 2.0 * (x * x + y * y);
    let sy = (-r20).clamp(-1.0, 1.0);
    if sy.abs() > 0.99999 {
        // Gimbal lock: fold Z into X.
        let r01 = 2.0 * (x * y - w * z);
        let r11 = 1.0 - 2.0 * (x * x + z * z);
        return [(sy.signum() * r01).atan2(r11), sy.asin(), 0.0];
    }
    [r21.atan2(r22), sy.asin(), r10.atan2(r00)]
}

/// Vertical field of view for a lens in millimetres on Blender's 36 mm sensor
/// (sensor fit along the 16:9 frame's width, as [`Camera::default`]).
fn lens_to_fov(lens: f32) -> f32 {
    2.0 * (18.0 / lens * 9.0 / 16.0).atan()
}

fn fov_to_lens(fov: f32) -> f32 {
    18.0 * 9.0 / 16.0 / (fov * 0.5).tan()
}

// ---------------------------------------------------------------------------
// Commands

/// A parsed command line: positional arguments and options, with every option
/// required to be used (so a typo is an error, not silently ignored).
struct Cmd {
    name: String,
    pos: Vec<Token>,
    opts: Vec<(String, Token, bool)>,
}

impl Cmd {
    fn opt(&mut self, key: &str) -> Option<Token> {
        let o = self.opts.iter_mut().find(|o| o.0 == key)?;
        o.2 = true;
        Some(o.1.clone())
    }
    fn f(&mut self, key: &str) -> Res<Option<f32>> {
        self.opt(key).map(|t| num(&t.text).map_err(|e| format!("{key}: {e}"))).transpose()
    }
    fn u(&mut self, key: &str) -> Res<Option<u32>> {
        self.opt(key).map(|t| uint(&t.text).map_err(|e| format!("{key}: {e}"))).transpose()
    }
    fn b(&mut self, key: &str) -> Res<Option<bool>> {
        self.opt(key).map(|t| boolean(&t.text).map_err(|e| format!("{key}: {e}"))).transpose()
    }
    fn v3(&mut self, key: &str) -> Res<Option<Vec3>> {
        self.opt(key).map(|t| floats(&t.text).and_then(|v| vec3_of(&v, key))).transpose()
    }
    fn list(&mut self, key: &str) -> Res<Option<Vec<f32>>> {
        self.opt(key).map(|t| floats(&t.text).map_err(|e| format!("{key}: {e}"))).transpose()
    }
    fn arg(&self, i: usize, what: &str) -> Res<&Token> {
        self.pos.get(i).ok_or_else(|| format!("{} needs {what}", self.name))
    }
    fn max_args(&self, n: usize) -> Res<()> {
        if self.pos.len() > n {
            return Err(format!("{} takes {n} argument(s), got {}", self.name, self.pos.len()));
        }
        Ok(())
    }
    /// Error on options nobody asked for.
    fn done(&self) -> Res<()> {
        match self.opts.iter().find(|o| !o.2) {
            Some(o) => Err(format!("unknown option '{}' for {}", o.0, self.name)),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
enum Handle {
    Object(String),
    Mesh(usize),
    Material(usize),
    Collection(usize),
}

/// The line-at-a-time interpreter behind [`run`]. Feed it lines with
/// [`Session::exec_line`]; it keeps handles and the header state between lines.
#[derive(Debug, Default)]
pub struct Session {
    line: usize,
    handles: HashMap<String, Handle>,
    report: RunReport,
    header: bool,
    ended: bool,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Execute one line. Errors carry this line's number.
    pub fn exec_line(&mut self, scene: &mut Scene, text: &str) -> Result<(), ScriptError> {
        self.line += 1;
        let line = self.line;
        self.exec(scene, text).map_err(|message| ScriptError { line, message })
    }

    /// Reply lines produced so far, taken out of the report (for a live channel).
    pub fn take_replies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.report.replies)
    }

    /// The report; fails when no `polyops` header was seen.
    pub fn finish(self) -> Result<RunReport, ScriptError> {
        if !self.header {
            return Err(ScriptError { line: self.line.max(1), message: "missing 'polyops 1' header".into() });
        }
        Ok(self.report)
    }

    fn exec(&mut self, scene: &mut Scene, text: &str) -> Res<()> {
        let mut toks = tokenize(text)?;
        if toks.is_empty() {
            return Ok(());
        }
        if self.ended {
            return Err("text after 'end'".into());
        }
        let id = match toks.first() {
            Some(t) if !t.quoted && t.key.is_none() && t.text.starts_with('@') => {
                let id = toks.remove(0).text[1..].to_string();
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                    return Err("a request id is @ then letters, digits, '_' or '-'".into());
                }
                Some(id)
            }
            _ => None,
        };
        let head = toks.first().ok_or("a request id needs a command")?;
        if head.quoted || head.key.is_some() {
            return Err("a line starts with a command word".into());
        }
        let name = toks.remove(0).text;
        if !self.header {
            if name != "polyops" {
                return Err("the first command must be 'polyops 1'".into());
            }
            let v = match toks.as_slice() {
                [t] if t.key.is_none() => uint(&t.text)?,
                _ => return Err("expected 'polyops 1'".into()),
            };
            if v == 0 || v > VERSION {
                return Err(format!("polyops version {v} is not supported (this Polyamour reads version {VERSION})"));
            }
            self.header = true;
            self.report.version = v;
            return Ok(());
        }
        let mut cmd = Cmd { name, pos: Vec::new(), opts: Vec::new() };
        for t in toks {
            match t.key.clone() {
                Some(k) => {
                    if cmd.opts.iter().any(|o| o.0 == k) {
                        return Err(format!("option '{k}' given twice"));
                    }
                    cmd.opts.push((k, Token { key: None, ..t }, false));
                }
                None => cmd.pos.push(t),
            }
        }
        let query = matches!(cmd.name.as_str(), "get" | "list" | "exists" | "ping");
        let reply = self.command(scene, &mut cmd)?;
        cmd.done()?;
        self.report.commands += 1;
        let prefix = id.as_ref().map(|i| format!("@{i} ")).unwrap_or_default();
        // Queries always answer; other commands only when asked with @ID (a
        // creating command then answers with the unique name it gave).
        if query || id.is_some() {
            match reply {
                Some(r) if !r.is_empty() => self.report.replies.push(format!("{prefix}ok {r}")),
                _ => self.report.replies.push(format!("{prefix}ok")),
            }
        }
        Ok(())
    }

    // --- name resolution ------------------------------------------------------

    fn handle(&self, t: &Token) -> Res<Option<&Handle>> {
        if t.quoted || !t.text.starts_with('$') {
            return Ok(None);
        }
        self.handles.get(&t.text).map(Some).ok_or_else(|| format!("handle {} is not bound", t.text))
    }

    fn object(&self, scene: &Scene, t: &Token) -> Res<usize> {
        let name = match self.handle(t)? {
            Some(Handle::Object(n)) => n.as_str(),
            Some(_) => return Err(format!("{} is not an object", t.text)),
            None => t.text.as_str(),
        };
        scene.find_object(name).ok_or_else(|| format!("no object named '{name}'"))
    }

    fn mesh(&self, scene: &Scene, t: &Token) -> Res<usize> {
        match self.handle(t)? {
            Some(Handle::Mesh(i)) => return Ok(*i),
            Some(Handle::Object(_)) => {}
            Some(_) => return Err(format!("{} is not a mesh", t.text)),
            None => {}
        }
        if let Ok(o) = self.object(scene, t) {
            return match scene.objects[o].data {
                ObjectData::Mesh(m) => Ok(m),
                _ => Err(format!("object '{}' has no mesh", scene.objects[o].name)),
            };
        }
        scene.mesh_names.iter().position(|n| *n == t.text).ok_or_else(|| format!("no object or mesh named '{}'", t.text))
    }

    fn material(&self, scene: &Scene, t: &Token) -> Res<usize> {
        match self.handle(t)? {
            Some(Handle::Material(i)) => Ok(*i),
            Some(_) => Err(format!("{} is not a material", t.text)),
            None => scene.materials.iter().position(|m| m.name == t.text).ok_or_else(|| format!("no material named '{}'", t.text)),
        }
    }

    fn collection(&self, scene: &Scene, t: &Token) -> Res<usize> {
        match self.handle(t)? {
            Some(Handle::Collection(i)) => Ok(*i),
            Some(_) => Err(format!("{} is not a collection", t.text)),
            None => scene.collections.iter().position(|c| c.name == t.text).ok_or_else(|| format!("no collection named '{}'", t.text)),
        }
    }

    /// Bind `as=$H` if given.
    fn bind(&mut self, cmd: &mut Cmd, h: Handle) -> Res<()> {
        if let Some(t) = cmd.opt("as") {
            if t.quoted || !t.text.starts_with('$') || t.text.len() < 2 {
                return Err("as= takes a handle like $1".into());
            }
            self.handles.insert(t.text, h);
        }
        Ok(())
    }

    fn rename_handles(&mut self, old: &str, new: &str) {
        for h in self.handles.values_mut() {
            if matches!(h, Handle::Object(n) if n == old) {
                *h = Handle::Object(new.to_string());
            }
        }
    }

    // --- dispatch ----------------------------------------------------------------

    fn command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        match cmd.name.as_str() {
            "polyops" => Err("the header appears twice".into()),
            "end" => {
                cmd.max_args(0)?;
                self.ended = true;
                self.report.complete = true;
                Ok(None)
            }
            "abort" => {
                let msg: Vec<&str> = cmd.pos.iter().map(|t| t.text.as_str()).collect();
                Err(format!("script aborted: {}", if msg.is_empty() { "no reason given".into() } else { msg.join(" ") }))
            }
            "ping" => {
                cmd.max_args(0)?;
                Ok(Some(format!("polyops {VERSION}")))
            }
            "scene" => {
                cmd.max_args(0)?;
                if let Some(t) = cmd.opt("name") {
                    scene.name = t.text;
                }
                if let Some(v) = cmd.opt("frame_start") {
                    scene.frame_start = v.text.parse().map_err(|_| "frame_start: not an integer")?;
                }
                if let Some(v) = cmd.opt("frame_end") {
                    scene.frame_end = v.text.parse().map_err(|_| "frame_end: not an integer")?;
                }
                if let Some(v) = cmd.f("fps")? {
                    scene.fps = v;
                }
                Ok(None)
            }
            "world" => {
                cmd.max_args(0)?;
                if let Some(c) = cmd.v3("color")? {
                    scene.world.color = c;
                }
                if let Some(s) = cmd.f("strength")? {
                    scene.world.strength = s;
                }
                Ok(None)
            }
            "render" => {
                cmd.max_args(0)?;
                for o in &mut cmd.opts {
                    o.2 = true;
                    self.report.render.insert(o.0.clone(), o.1.text.clone());
                }
                Ok(None)
            }
            "add_primitive" => self.add_primitive(scene, cmd),
            "mesh_new" => {
                cmd.max_args(1)?;
                let name = cmd.arg(0, "a mesh name")?.text.clone();
                let m = parse_mesh(cmd, Mesh::new())?;
                let i = scene.add_mesh(&name, m);
                self.bind(cmd, Handle::Mesh(i))?;
                Ok(Some(quote(&scene.mesh_names[i])))
            }
            "add_mesh" => {
                cmd.max_args(1)?;
                let name = given_name(cmd, true).unwrap_or_else(|| "Mesh".into());
                let m = parse_mesh(cmd, Mesh::new())?;
                let i = scene.add_mesh_object(&name, m);
                self.created(scene, cmd, i).map(Some)
            }
            "add_object" | "add_empty" => {
                cmd.max_args(1)?;
                let name = given_name(cmd, true).unwrap_or_else(|| "Empty".into());
                let data = match cmd.opt("data") {
                    Some(t) if t.is_word("none") => ObjectData::Empty,
                    Some(t) if cmd.name == "add_object" => ObjectData::Mesh(self.mesh(scene, &t)?),
                    Some(_) => return Err("add_empty takes no data=".into()),
                    None => ObjectData::Empty,
                };
                let i = scene.add_object(Object::new(&name, data));
                self.created(scene, cmd, i).map(Some)
            }
            "light_add" => {
                cmd.max_args(1)?;
                let kind = cmd.pos.first().map(|t| t.text.to_ascii_lowercase()).unwrap_or_else(|| "point".into());
                let (kind, default_name, power) = match kind.as_str() {
                    "point" => (LightKind::Point, "Point", 1000.0),
                    "sun" => (LightKind::Sun, "Sun", 1.0),
                    "spot" => (LightKind::Spot { angle: 45f32.to_radians(), blend: 0.15 }, "Spot", 1000.0),
                    "area" => (LightKind::Area { size: 1.0 }, "Area", 1000.0),
                    k => return Err(format!("unknown light type '{k}' (point, sun, spot, area)")),
                };
                let name = given_name(cmd, false).unwrap_or_else(|| default_name.into());
                let mut light = Light { kind, power, ..Light::default() };
                set_light_opts(cmd, &mut light)?;
                let i = scene.add_object(Object::new(&name, ObjectData::Light(light)));
                self.created(scene, cmd, i).map(Some)
            }
            "camera_add" => {
                cmd.max_args(0)?;
                let name = given_name(cmd, false).unwrap_or_else(|| "Camera".into());
                let mut cam = Camera::default();
                if let Some(v) = cmd.f("fov")? {
                    cam.fov_y = v;
                }
                if let Some(v) = cmd.f("lens")? {
                    cam.fov_y = lens_to_fov(v);
                }
                if let Some(v) = cmd.f("ortho_scale")? {
                    cam.ortho_scale = Some(v);
                }
                if let Some(v) = cmd.f("near")? {
                    cam.near = v;
                }
                if let Some(v) = cmd.f("far")? {
                    cam.far = v;
                }
                let i = scene.add_object(Object::new(&name, ObjectData::Camera(cam)));
                let make_active = cmd.b("active")?;
                let r = self.created(scene, cmd, i)?;
                if make_active.unwrap_or(scene.active_camera.is_none()) {
                    scene.active_camera = Some(i);
                }
                Ok(Some(r))
            }
            "mesh_set" => {
                cmd.max_args(1)?;
                let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
                let m = parse_mesh(cmd, Mesh::new())?;
                scene.meshes[mi] = m;
                Ok(None)
            }
            "mesh_edit" => self.mesh_edit(scene, cmd),
            "set" => self.set(scene, cmd),
            "get" => self.get(scene, cmd).map(Some),
            "parent" => {
                cmd.max_args(2)?;
                let child = self.object(scene, cmd.arg(0, "a child object")?)?;
                let p = cmd.arg(1, "a parent object or none")?.clone();
                let keep = cmd.b("keep_transform")?.unwrap_or(false);
                self.set_parent(scene, child, &p, keep)?;
                Ok(None)
            }
            "material_new" => {
                cmd.max_args(1)?;
                let base = cmd.arg(0, "a material name")?.text.clone();
                let name = unique_name(&base, |n| scene.materials.iter().any(|m| m.name == n));
                let mut m = Material { name, ..Material::default() };
                set_material_opts(cmd, &mut m)?;
                let i = scene.add_material(m);
                self.bind(cmd, Handle::Material(i))?;
                Ok(Some(quote(&scene.materials[i].name)))
            }
            "material_set" => {
                cmd.max_args(1)?;
                let i = self.material(scene, cmd.arg(0, "a material")?)?;
                if let Some(n) = cmd.opt("name") {
                    let taken = |x: &str| scene.materials.iter().enumerate().any(|(j, m)| j != i && m.name == x);
                    scene.materials[i].name = unique_name(&n.text, taken);
                }
                set_material_opts(cmd, &mut scene.materials[i])?;
                Ok(None)
            }
            "material_assign" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                let m = self.material(scene, cmd.arg(1, "a material")?)?;
                let slots = &mut scene.objects[o].materials;
                match cmd.u("slot")? {
                    Some(s) => {
                        let s = s as usize;
                        if s > 255 {
                            return Err("slot must be below 256".into());
                        }
                        if slots.len() <= s {
                            slots.resize(s + 1, m);
                        }
                        slots[s] = m;
                    }
                    None => slots.push(m),
                }
                Ok(None)
            }
            "material_clear" => {
                cmd.max_args(1)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                scene.objects[o].materials.clear();
                Ok(None)
            }
            "modifier_add" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                let ty = cmd.arg(1, "a modifier type")?.text.clone();
                let m = modifier_opts(cmd, default_modifier(&ty)?)?;
                scene.objects[o].modifiers.push(m);
                Ok(Some((scene.objects[o].modifiers.len() - 1).to_string()))
            }
            "modifier_set" | "modifier_remove" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                let idx = uint(&cmd.arg(1, "a modifier index")?.text)? as usize;
                let n = scene.objects[o].modifiers.len();
                if idx >= n {
                    return Err(format!("modifier {idx} does not exist ('{}' has {n})", scene.objects[o].name));
                }
                if cmd.name == "modifier_remove" {
                    scene.objects[o].modifiers.remove(idx);
                } else {
                    let m = modifier_opts(cmd, scene.objects[o].modifiers[idx].clone())?;
                    scene.objects[o].modifiers[idx] = m;
                }
                Ok(None)
            }
            "modifier_clear" => {
                cmd.max_args(1)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                scene.objects[o].modifiers.clear();
                Ok(None)
            }
            "select" | "select_add" | "deselect" => {
                let objs = cmd.pos.iter().map(|t| self.object(scene, t)).collect::<Res<Vec<_>>>()?;
                match cmd.name.as_str() {
                    "select" => scene.selected.clear(),
                    "deselect" if objs.is_empty() => scene.selected.clear(),
                    _ => {}
                }
                for o in objs {
                    if cmd.name == "deselect" {
                        scene.selected.retain(|&s| s != o);
                    } else if !scene.selected.contains(&o) {
                        scene.selected.push(o);
                    }
                }
                Ok(None)
            }
            "select_all" => {
                cmd.max_args(0)?;
                scene.selected = (0..scene.objects.len()).collect();
                Ok(None)
            }
            "active" | "active_camera" => {
                cmd.max_args(1)?;
                let t = cmd.arg(0, "an object or none")?.clone();
                let o = if t.is_word("none") { None } else { Some(self.object(scene, &t)?) };
                if cmd.name == "active" {
                    scene.active_object = o;
                } else {
                    if let Some(i) = o {
                        if !matches!(scene.objects[i].data, ObjectData::Camera(_)) {
                            return Err(format!("'{}' is not a camera", scene.objects[i].name));
                        }
                    }
                    scene.active_camera = o;
                }
                Ok(None)
            }
            "delete" => {
                if cmd.pos.is_empty() {
                    return Err("delete needs objects".into());
                }
                let mut objs = cmd.pos.iter().map(|t| self.object(scene, t)).collect::<Res<Vec<_>>>()?;
                objs.sort_unstable();
                objs.dedup();
                for &o in objs.iter().rev() {
                    let gone = scene.remove_object(o).name;
                    self.handles.retain(|_, h| !matches!(h, Handle::Object(n) if *n == gone));
                }
                Ok(None)
            }
            "collection_new" => {
                cmd.max_args(1)?;
                let name = cmd.arg(0, "a collection name")?.text.clone();
                let parent = cmd.opt("parent").map(|t| self.collection(scene, &t)).transpose()?;
                let i = scene.add_collection(&name, parent);
                if let Some(v) = cmd.b("visible")? {
                    scene.collections[i].visible = v;
                }
                self.bind(cmd, Handle::Collection(i))?;
                Ok(Some(quote(&scene.collections[i].name)))
            }
            "collection_set" => {
                cmd.max_args(1)?;
                let i = self.collection(scene, cmd.arg(0, "a collection")?)?;
                if let Some(t) = cmd.opt("parent") {
                    let p = if t.is_word("none") { None } else { Some(self.collection(scene, &t)?) };
                    let mut c = p;
                    while let Some(ci) = c {
                        if ci == i {
                            return Err("collection parenting would make a cycle".into());
                        }
                        c = scene.collections[ci].parent;
                    }
                    scene.collections[i].parent = p;
                }
                if let Some(n) = cmd.opt("name") {
                    let taken = |x: &str| scene.collections.iter().enumerate().any(|(j, c)| j != i && c.name == x);
                    scene.collections[i].name = unique_name(&n.text, taken);
                }
                if let Some(v) = cmd.b("visible")? {
                    scene.collections[i].visible = v;
                }
                Ok(None)
            }
            "link" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                let c = self.collection(scene, cmd.arg(1, "a collection")?)?;
                scene.objects[o].collection = c;
                Ok(None)
            }
            "list" => {
                cmd.max_args(1)?;
                let what = cmd.arg(0, "objects, meshes, materials, collections or selected")?.text.clone();
                let names: Vec<String> = match what.as_str() {
                    "objects" => scene.objects.iter().map(|o| quote(&o.name)).collect(),
                    "meshes" => scene.mesh_names.iter().map(|n| quote(n)).collect(),
                    "materials" => scene.materials.iter().map(|m| quote(&m.name)).collect(),
                    "collections" => scene.collections.iter().map(|c| quote(&c.name)).collect(),
                    "selected" => scene.selected.iter().map(|&i| quote(&scene.objects[i].name)).collect(),
                    w => return Err(format!("cannot list '{w}'")),
                };
                Ok(Some(names.join(" ")))
            }
            "exists" => {
                cmd.max_args(1)?;
                let t = cmd.arg(0, "an object")?;
                Ok(Some(if self.object(scene, t).is_ok() { "1" } else { "0" }.into()))
            }
            other => Err(format!("unknown command '{other}'")),
        }
    }

    /// Placement options shared by every creating command, then Blender's
    /// add behaviour: the new object is the active and only selected one.
    fn created(&mut self, scene: &mut Scene, cmd: &mut Cmd, i: usize) -> Res<String> {
        let o = &mut scene.objects[i];
        if let Some(v) = cmd.v3("location")? {
            o.transform.location = v;
        }
        if let Some(r) = cmd.v3("rotation")? {
            o.transform.rotation = Quat::from_euler_xyz(r.x, r.y, r.z);
        }
        if let Some(q) = cmd.list("quaternion")? {
            o.transform.rotation = quat_of(&q)?;
        }
        if let Some(s) = cmd.list("scale")? {
            o.transform.scale = scale_of(&s)?;
        }
        if let Some(v) = cmd.b("visible")? {
            scene.objects[i].visible = v;
        }
        if let Some(t) = cmd.opt("collection") {
            scene.objects[i].collection = self.collection(scene, &t)?;
        }
        let name = scene.objects[i].name.clone();
        self.bind(cmd, Handle::Object(name.clone()))?;
        scene.active_object = Some(i);
        scene.selected = vec![i];
        self.report.created.push(name.clone());
        Ok(quote(&name))
    }

    fn add_primitive(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        use crate::primitives as p;
        cmd.max_args(1)?;
        let kind = cmd.arg(0, "a primitive type")?.text.to_ascii_lowercase();
        let (mesh, default_name) = match kind.as_str() {
            "cube" => (p::cube(cmd.f("size")?.unwrap_or(2.0)), "Cube"),
            "plane" => (p::plane(cmd.f("size")?.unwrap_or(2.0)), "Plane"),
            "grid" => {
                let (x, y) = (cmd.u("x_subdivisions")?.unwrap_or(10), cmd.u("y_subdivisions")?.unwrap_or(10));
                (p::grid(x.min(4096), y.min(4096), cmd.f("size")?.unwrap_or(2.0)), "Grid")
            }
            "uv_sphere" | "sphere" => {
                let (s, r) = (cmd.u("segments")?.unwrap_or(32), cmd.u("rings")?.unwrap_or(16));
                (p::uv_sphere(s.min(4096), r.min(4096), cmd.f("radius")?.unwrap_or(1.0)), "Sphere")
            }
            "ico_sphere" | "icosphere" => {
                let sub = cmd.u("subdivisions")?.unwrap_or(2);
                if sub > 8 {
                    return Err("subdivisions must be 8 or fewer".into());
                }
                (p::ico_sphere(sub, cmd.f("radius")?.unwrap_or(1.0)), "Icosphere")
            }
            "cylinder" => {
                let s = cmd.u("segments")?.unwrap_or(32).min(4096);
                (p::cylinder(s, cmd.f("radius")?.unwrap_or(1.0), cmd.f("depth")?.unwrap_or(2.0)), "Cylinder")
            }
            "cone" => {
                let s = cmd.u("segments")?.unwrap_or(32).min(4096);
                let (r1, r2) = (cmd.f("radius1")?.unwrap_or(1.0), cmd.f("radius2")?.unwrap_or(0.0));
                (p::cone(s, r1, r2, cmd.f("depth")?.unwrap_or(2.0)), "Cone")
            }
            "torus" => {
                let (a, b) = (cmd.u("major_segments")?.unwrap_or(48).min(4096), cmd.u("minor_segments")?.unwrap_or(12).min(4096));
                (p::torus(a, b, cmd.f("major_radius")?.unwrap_or(1.0), cmd.f("minor_radius")?.unwrap_or(0.25)), "Torus")
            }
            k => return Err(format!("unknown primitive '{k}' (cube, plane, grid, uv_sphere, ico_sphere, cylinder, cone, torus)")),
        };
        let name = given_name(cmd, false).unwrap_or_else(|| default_name.into());
        let i = scene.add_mesh_object(&name, mesh);
        self.created(scene, cmd, i).map(Some)
    }

    fn mesh_edit(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
        let op = cmd.arg(1, "an operation")?.text.clone();
        let rest = values(&cmd.pos[2..]);
        let m = &mut scene.meshes[mi];
        let n = m.positions.len();
        let picked: Vec<usize> = match cmd.opt("indices") {
            Some(t) => {
                let idx = uints(&t.text)?;
                if let Some(&bad) = idx.iter().find(|&&v| v as usize >= n) {
                    return Err(format!("vertex {bad} does not exist (the mesh has {n})"));
                }
                idx.into_iter().map(|v| v as usize).collect()
            }
            None => (0..n).collect(),
        };
        let center = cmd.v3("center")?.unwrap_or(Vec3::ZERO);
        match op.as_str() {
            "translate_verts" => {
                let d = vec3_of(&rest?, "translate_verts")?;
                for &i in &picked {
                    m.positions[i] += d;
                }
            }
            "scale_verts" => {
                let s = scale_of(&rest?)?;
                for &i in &picked {
                    m.positions[i] = center + (m.positions[i] - center).mul_elem(s);
                }
            }
            "rotate_verts" => {
                let r = vec3_of(&rest?, "rotate_verts")?;
                let q = Quat::from_euler_xyz(r.x, r.y, r.z);
                for &i in &picked {
                    m.positions[i] = center + q.rotate(m.positions[i] - center);
                }
            }
            "set_verts" => {
                let p = cmd.list("verts")?.ok_or("set_verts needs verts=")?;
                if p.len() != picked.len() * 3 {
                    return Err(format!("set_verts: {} numbers for {} vertices", p.len(), picked.len()));
                }
                for (k, &i) in picked.iter().enumerate() {
                    m.positions[i] = Vec3::new(p[3 * k], p[3 * k + 1], p[3 * k + 2]);
                }
            }
            "flip_normals" => m.flip_normals(),
            "shade_smooth" | "shade_flat" => {
                for f in &mut m.faces {
                    f.smooth = op == "shade_smooth";
                }
            }
            "weld" => {
                let d = rest?.first().copied().unwrap_or(1e-4);
                m.weld(d);
            }
            "set_material" => {
                let slot = rest?.first().copied().ok_or("set_material needs a slot")?;
                if slot < 0.0 || slot > 255.0 || slot.fract() != 0.0 {
                    return Err("slot must be a whole number below 256".into());
                }
                let faces = match cmd.opt("faces") {
                    Some(t) => uints(&t.text)?.into_iter().map(|f| f as usize).collect(),
                    None => (0..m.faces.len()).collect::<Vec<_>>(),
                };
                for f in faces {
                    m.faces.get_mut(f).ok_or_else(|| format!("face {f} does not exist"))?.material = slot as u16;
                }
            }
            o => return Err(format!("unknown mesh_edit operation '{o}'")),
        }
        if !matches!(op.as_str(), "translate_verts" | "scale_verts" | "rotate_verts" | "weld" | "set_material") {
            cmd.max_args(2)?;
        }
        Ok(None)
    }

    fn set_parent(&mut self, scene: &mut Scene, child: usize, p: &Token, keep: bool) -> Res<()> {
        let parent = if p.is_word("none") { None } else { Some(self.object(scene, p)?) };
        let mut c = parent;
        let mut guard = 0;
        while let Some(ci) = c {
            if ci == child {
                return Err("parenting would make a cycle".into());
            }
            c = scene.objects[ci].parent;
            guard += 1;
            if guard > scene.objects.len() {
                break;
            }
        }
        let world = scene.world_matrix(child);
        scene.objects[child].parent = parent;
        if keep {
            let local = match parent {
                Some(pi) => scene.world_matrix(pi).inverse() * world,
                None => world,
            };
            scene.objects[child].transform = Transform::from_matrix(&local);
        }
        Ok(())
    }

    /// Split `Obj.prop` at its last dot (property names have none).
    fn target(&self, scene: &Scene, cmd: &Cmd) -> Res<(usize, String)> {
        let t = cmd.arg(0, "OBJECT.property")?;
        let (obj, prop) = t.text.rsplit_once('.').ok_or_else(|| format!("'{}' is not OBJECT.property", t.text))?;
        let tok = Token { key: None, text: obj.to_string(), quoted: t.quoted };
        Ok((self.object(scene, &tok)?, prop.to_string()))
    }

    fn set(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        let (i, prop) = self.target(scene, cmd)?;
        let args = cmd.pos[1..].to_vec();
        let one = || -> Res<&Token> {
            match args.as_slice() {
                [t] => Ok(t),
                _ => Err(format!("{prop} takes one value")),
            }
        };
        let v = || values(&args);
        let o = &mut scene.objects[i];
        match prop.as_str() {
            "location" => o.transform.location = vec3_of(&v()?, "location")?,
            "rotation_euler" => {
                let r = vec3_of(&v()?, "rotation_euler")?;
                o.transform.rotation = Quat::from_euler_xyz(r.x, r.y, r.z);
            }
            "rotation_quaternion" => o.transform.rotation = quat_of(&v()?)?,
            "scale" => o.transform.scale = scale_of(&v()?)?,
            "visible" => o.visible = boolean(&one()?.text)?,
            "name" => {
                let old = o.name.clone();
                let new = one()?.text.clone();
                if new.is_empty() {
                    return Err("a name cannot be empty".into());
                }
                let new = unique_name(&new, |n| scene.objects.iter().enumerate().any(|(j, x)| j != i && x.name == n));
                scene.objects[i].name = new.clone();
                self.rename_handles(&old, &new);
            }
            "collection" => {
                let c = self.collection(scene, one()?)?;
                scene.objects[i].collection = c;
            }
            "parent" => {
                let p = one()?.clone();
                self.set_parent(scene, i, &p, false)?;
            }
            _ => match &mut o.data {
                ObjectData::Light(l) => set_light_prop(l, &prop, &args)?,
                ObjectData::Camera(c) => set_camera_prop(c, &prop, &args)?,
                _ => return Err(format!("unknown property '{prop}' for '{}'", o.name)),
            },
        }
        Ok(None)
    }

    fn get(&self, scene: &Scene, cmd: &mut Cmd) -> Res<String> {
        cmd.max_args(1)?;
        let (i, prop) = self.target(scene, cmd)?;
        let o = &scene.objects[i];
        let t = &o.transform;
        Ok(match prop.as_str() {
            "location" => fmt_vals(&t.location.to_array()),
            "rotation_euler" => fmt_vals(&quat_to_euler_xyz(t.rotation)),
            "rotation_quaternion" => fmt_vals(&[t.rotation.w, t.rotation.x, t.rotation.y, t.rotation.z]),
            "scale" => fmt_vals(&t.scale.to_array()),
            "matrix_world" => fmt_vals(&scene.world_matrix(i).to_cols_array()),
            "visible" => (o.visible as u8).to_string(),
            "name" => quote(&o.name),
            "collection" => quote(&scene.collections[o.collection].name),
            "parent" => o.parent.map(|p| quote(&scene.objects[p].name)).unwrap_or_else(|| "none".into()),
            "type" => match o.data {
                ObjectData::Empty => "EMPTY",
                ObjectData::Mesh(_) => "MESH",
                ObjectData::Camera(_) => "CAMERA",
                ObjectData::Light(_) => "LIGHT",
            }
            .into(),
            "data" => match o.data {
                ObjectData::Mesh(m) => quote(&scene.mesh_names[m]),
                _ => "none".into(),
            },
            "vertex_count" | "face_count" => match o.data {
                ObjectData::Mesh(m) => {
                    let m = &scene.meshes[m];
                    (if prop == "vertex_count" { m.vertex_count() } else { m.face_count() }).to_string()
                }
                _ => "0".into(),
            },
            "materials" => o.materials.iter().map(|&m| quote(&scene.materials[m].name)).collect::<Vec<_>>().join(" "),
            "modifiers" => o.modifiers.iter().map(|m| m.name()).collect::<Vec<_>>().join(" "),
            _ => match &o.data {
                ObjectData::Light(l) => match prop.as_str() {
                    "power" => fmt_f(l.power),
                    "color" => fmt_vals(&l.color.to_array()),
                    "radius" => fmt_f(l.radius),
                    _ => return Err(format!("unknown property '{prop}' for light '{}'", o.name)),
                },
                ObjectData::Camera(c) => match prop.as_str() {
                    "fov" => fmt_f(c.fov_y),
                    "lens" => fmt_f(fov_to_lens(c.fov_y)),
                    "near" => fmt_f(c.near),
                    "far" => fmt_f(c.far),
                    "ortho_scale" => c.ortho_scale.map(fmt_f).unwrap_or_else(|| "none".into()),
                    _ => return Err(format!("unknown property '{prop}' for camera '{}'", o.name)),
                },
                _ => return Err(format!("unknown property '{prop}' for '{}'", o.name)),
            },
        })
    }
}

/// The new object's name: `name=`, or (`positional`) the first argument; an
/// empty name counts as none given.
fn given_name(cmd: &mut Cmd, positional: bool) -> Option<String> {
    let n = cmd.opt("name").map(|t| t.text).or_else(|| if positional { cmd.pos.first().map(|t| t.text.clone()) } else { None });
    n.filter(|n| !n.is_empty())
}

fn fmt_vals(v: &[f32]) -> String {
    // `+ 0.0` turns -0 into 0 for readers.
    v.iter().map(|x| fmt_f(*x + 0.0)).collect::<Vec<_>>().join(" ")
}

/// `w,x,y,z` (Blender's order) to a unit quaternion.
fn quat_of(v: &[f32]) -> Res<Quat> {
    match v {
        [w, x, y, z] => {
            let q = Quat { x: *x, y: *y, z: *z, w: *w };
            if (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w) < 1e-12 {
                return Err("a rotation quaternion cannot be zero".into());
            }
            Ok(q)
        }
        _ => Err(format!("a quaternion needs 4 numbers (w x y z), got {}", v.len())),
    }
}

/// One number (uniform) or three.
fn scale_of(v: &[f32]) -> Res<Vec3> {
    match v {
        [s] => Ok(Vec3::splat(*s)),
        _ => vec3_of(v, "scale"),
    }
}

fn set_light_opts(cmd: &mut Cmd, l: &mut Light) -> Res<()> {
    for key in ["power", "color", "radius", "angle", "blend", "size"] {
        if let Some(t) = cmd.opt(key) {
            set_light_prop(l, key, std::slice::from_ref(&t))?;
        }
    }
    Ok(())
}

fn set_light_prop(l: &mut Light, prop: &str, args: &[Token]) -> Res<()> {
    let v = values(args)?;
    let one = || match v.as_slice() {
        [x] => Ok(*x),
        _ => Err(format!("{prop} takes one number")),
    };
    match (prop, &mut l.kind) {
        ("power", _) => l.power = one()?,
        ("color", _) => l.color = vec3_of(&v, "color")?,
        ("radius", _) => l.radius = one()?,
        ("angle", LightKind::Spot { angle, .. }) => *angle = one()?,
        ("blend", LightKind::Spot { blend, .. }) => *blend = one()?,
        ("size", LightKind::Area { size }) => *size = one()?,
        ("angle" | "blend", _) => return Err(format!("{prop} is only for spot lights")),
        ("size", _) => return Err("size is only for area lights".into()),
        _ => return Err(format!("unknown light property '{prop}'")),
    }
    Ok(())
}

fn set_camera_prop(c: &mut Camera, prop: &str, args: &[Token]) -> Res<()> {
    if prop == "ortho_scale" && matches!(args, [t] if t.is_word("none")) {
        c.ortho_scale = None;
        return Ok(());
    }
    let v = match values(args)?.as_slice() {
        [x] => *x,
        _ => return Err(format!("{prop} takes one number")),
    };
    match prop {
        "fov" => c.fov_y = v,
        "lens" => c.fov_y = lens_to_fov(v),
        "near" => c.near = v,
        "far" => c.far = v,
        "ortho_scale" => c.ortho_scale = Some(v),
        _ => return Err(format!("unknown camera property '{prop}'")),
    }
    Ok(())
}

fn set_material_opts(cmd: &mut Cmd, m: &mut Material) -> Res<()> {
    if let Some(c) = cmd.list("base_color")? {
        m.base_color = match c.as_slice() {
            [r, g, b] => [*r, *g, *b, 1.0],
            [r, g, b, a] => [*r, *g, *b, *a],
            _ => return Err("base_color needs 3 or 4 numbers".into()),
        };
    }
    if let Some(v) = cmd.f("metallic")? {
        m.metallic = v;
    }
    if let Some(v) = cmd.f("roughness")? {
        m.roughness = v;
    }
    if let Some(v) = cmd.v3("emission")? {
        m.emission = v;
    }
    if let Some(v) = cmd.f("emission_strength")? {
        m.emission_strength = v;
    }
    if let Some(v) = cmd.f("ior")? {
        m.ior = v;
    }
    if let Some(v) = cmd.f("transmission")? {
        m.transmission = v;
    }
    if let Some(v) = cmd.b("double_sided")? {
        m.double_sided = v;
    }
    Ok(())
}

/// A modifier with Blender's defaults, by polyops or Blender type name.
fn default_modifier(ty: &str) -> Res<Modifier> {
    Ok(match ty.to_ascii_lowercase().as_str() {
        "mirror" => Modifier::Mirror { x: true, y: false, z: false, merge: true, merge_distance: 0.001 },
        "subdivision" | "subsurf" | "subdivision_surface" => Modifier::Subdivision { levels: 1, simple: false },
        "array" => Modifier::Array { count: 2, relative: [1.0, 0.0, 0.0], constant: [0.0; 3], merge: false },
        "solidify" => Modifier::Solidify { thickness: 0.01, offset: -1.0 },
        "bevel" => Modifier::Bevel { width: 0.1, segments: 1, angle: 30f32.to_radians() },
        "triangulate" => Modifier::Triangulate,
        "weld" => Modifier::Weld { distance: 0.001 },
        "displace" => Modifier::Displace { strength: 1.0 },
        "smooth" => Modifier::Smooth { factor: 0.5, iterations: 1 },
        _ => return Err(format!("unknown modifier type '{ty}' (Mirror, Subdivision, Array, Solidify, Bevel, Triangulate, Weld, Displace, Smooth)")),
    })
}

fn modifier_opts(cmd: &mut Cmd, mut m: Modifier) -> Res<Modifier> {
    fn arr3(cmd: &mut Cmd, key: &str, into: &mut [f32; 3]) -> Res<()> {
        if let Some(v) = cmd.v3(key)? {
            *into = v.to_array();
        }
        Ok(())
    }
    match &mut m {
        Modifier::Mirror { x, y, z, merge, merge_distance } => {
            *x = cmd.b("x")?.unwrap_or(*x);
            *y = cmd.b("y")?.unwrap_or(*y);
            *z = cmd.b("z")?.unwrap_or(*z);
            *merge = cmd.b("merge")?.unwrap_or(*merge);
            *merge_distance = cmd.f("merge_distance")?.unwrap_or(*merge_distance);
        }
        Modifier::Subdivision { levels, simple } => {
            *levels = cmd.u("levels")?.unwrap_or(*levels);
            if *levels > 6 {
                return Err("subdivision levels must be 6 or fewer".into());
            }
            *simple = cmd.b("simple")?.unwrap_or(*simple);
        }
        Modifier::Array { count, relative, constant, merge } => {
            *count = cmd.u("count")?.unwrap_or(*count);
            if *count > 10_000 {
                return Err("array count must be 10000 or fewer".into());
            }
            arr3(cmd, "relative", relative)?;
            arr3(cmd, "constant", constant)?;
            *merge = cmd.b("merge")?.unwrap_or(*merge);
        }
        Modifier::Solidify { thickness, offset } => {
            *thickness = cmd.f("thickness")?.unwrap_or(*thickness);
            *offset = cmd.f("offset")?.unwrap_or(*offset);
        }
        Modifier::Bevel { width, segments, angle } => {
            *width = cmd.f("width")?.unwrap_or(*width);
            *segments = cmd.u("segments")?.unwrap_or(*segments);
            *angle = cmd.f("angle")?.unwrap_or(*angle);
        }
        Modifier::Triangulate => {}
        Modifier::Weld { distance } => *distance = cmd.f("distance")?.unwrap_or(*distance),
        Modifier::Displace { strength } => *strength = cmd.f("strength")?.unwrap_or(*strength),
        Modifier::Smooth { factor, iterations } => {
            *factor = cmd.f("factor")?.unwrap_or(*factor);
            *iterations = cmd.u("iterations")?.unwrap_or(*iterations);
        }
    }
    Ok(m)
}

/// Mesh data options: `verts= faces= edges= uvs= smooth= mat=`.
fn parse_mesh(cmd: &mut Cmd, mut m: Mesh) -> Res<Mesh> {
    if let Some(v) = cmd.list("verts")? {
        if v.len() % 3 != 0 {
            return Err(format!("verts= holds {} numbers, not a multiple of 3", v.len()));
        }
        m.positions = v.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
    }
    if let Some(t) = cmd.opt("faces") {
        m.faces = polygons(&t.text).map_err(|e| format!("faces: {e}"))?.into_iter().map(Face::new).collect();
    }
    if let Some(t) = cmd.opt("edges") {
        let e = if t.text.starts_with("b64:") {
            uints(&t.text)
        } else {
            t.text.split(';').filter(|s| !s.trim().is_empty()).map(uints).collect::<Res<Vec<_>>>().map(|v| v.concat())
        }
        .map_err(|e| format!("edges: {e}"))?;
        if e.len() % 2 != 0 {
            return Err("edges= needs pairs of vertex indices".into());
        }
        m.loose_edges = e.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    }
    if let Some(uv) = cmd.list("uvs")? {
        let corners: usize = m.faces.iter().map(|f| f.verts.len()).sum();
        if uv.len() != corners * 2 {
            return Err(format!("uvs= holds {} numbers for {corners} face corners (2 each)", uv.len()));
        }
        let mut k = 0;
        for f in &mut m.faces {
            f.uvs = (0..f.verts.len()).map(|c| Vec2::new(uv[k + 2 * c], uv[k + 2 * c + 1])).collect();
            k += 2 * f.verts.len();
        }
    }
    if let Some(t) = cmd.opt("smooth") {
        let s = t.text.trim();
        if s.len() == 1 {
            let on = boolean(s)?;
            m.faces.iter_mut().for_each(|f| f.smooth = on);
        } else {
            if s.len() != m.faces.len() || !s.bytes().all(|b| b == b'0' || b == b'1') {
                return Err(format!("smooth= is one 0/1 per face ({} faces) or a single 0/1", m.faces.len()));
            }
            for (f, b) in m.faces.iter_mut().zip(s.bytes()) {
                f.smooth = b == b'1';
            }
        }
    }
    if let Some(t) = cmd.opt("mat") {
        let mat = uints(&t.text)?;
        if mat.len() != m.faces.len() {
            return Err(format!("mat= has {} entries for {} faces", mat.len(), m.faces.len()));
        }
        for (f, s) in m.faces.iter_mut().zip(mat) {
            f.material = u16::try_from(s).ok().filter(|s| *s < 256).ok_or("mat= slots must be below 256")?;
        }
    }
    m.validate()?;
    Ok(m)
}

// ---------------------------------------------------------------------------
// Writing scenes

/// Mesh data options for `mesh_new` / `add_mesh` / `mesh_set`.
pub fn mesh_to_options(m: &Mesh) -> String {
    let b64 = m.positions.len() > BASE64_ABOVE;
    let mut o = String::new();
    let pos: Vec<f32> = m.positions.iter().flat_map(|p| p.to_array()).collect();
    let _ = write!(o, " verts={}", if b64 { b64_f32(&pos) } else { fmt_list(&pos) });
    if !m.faces.is_empty() {
        let faces = if b64 {
            b64_u32(&m.faces.iter().flat_map(|f| std::iter::once(f.verts.len() as u32).chain(f.verts.iter().copied())).collect::<Vec<_>>())
        } else {
            m.faces.iter().map(|f| f.verts.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join(";")
        };
        let _ = write!(o, " faces={faces}");
    }
    if !m.loose_edges.is_empty() {
        let flat: Vec<u32> = m.loose_edges.iter().flat_map(|e| e.iter().copied()).collect();
        let edges = if b64 { b64_u32(&flat) } else { m.loose_edges.iter().map(|e| format!("{},{}", e[0], e[1])).collect::<Vec<_>>().join(";") };
        let _ = write!(o, " edges={edges}");
    }
    if m.has_uvs() {
        let uv: Vec<f32> = m.faces.iter().flat_map(|f| f.uvs.iter().flat_map(|u| [u.x, u.y])).collect();
        let _ = write!(o, " uvs={}", if b64 { b64_f32(&uv) } else { fmt_list(&uv) });
    }
    if m.faces.iter().any(|f| f.smooth) {
        if m.faces.iter().all(|f| f.smooth) {
            o.push_str(" smooth=1");
        } else {
            let s: String = m.faces.iter().map(|f| if f.smooth { '1' } else { '0' }).collect();
            let _ = write!(o, " smooth={s}");
        }
    }
    if m.faces.iter().any(|f| f.material != 0) {
        let _ = write!(o, " mat={}", m.faces.iter().map(|f| f.material.to_string()).collect::<Vec<_>>().join(","));
    }
    o
}

fn modifier_to_options(m: &Modifier) -> String {
    let b = |v: bool| v as u8;
    match m {
        Modifier::Mirror { x, y, z, merge, merge_distance } => format!(" x={} y={} z={} merge={} merge_distance={}", b(*x), b(*y), b(*z), b(*merge), fmt_f(*merge_distance)),
        Modifier::Subdivision { levels, simple } => format!(" levels={levels} simple={}", b(*simple)),
        Modifier::Array { count, relative, constant, merge } => format!(" count={count} relative={} constant={} merge={}", fmt_list(relative), fmt_list(constant), b(*merge)),
        Modifier::Solidify { thickness, offset } => format!(" thickness={} offset={}", fmt_f(*thickness), fmt_f(*offset)),
        Modifier::Bevel { width, segments, angle } => format!(" width={} segments={segments} angle={}", fmt_f(*width), fmt_f(*angle)),
        Modifier::Triangulate => String::new(),
        Modifier::Weld { distance } => format!(" distance={}", fmt_f(*distance)),
        Modifier::Displace { strength } => format!(" strength={}", fmt_f(*strength)),
        Modifier::Smooth { factor, iterations } => format!(" factor={} iterations={iterations}", fmt_f(*factor)),
    }
}

/// Serialise `scene` as a polyops program. Running it on [`Scene::new`] gives a
/// scene equal to `scene`, except that images, material textures and the world
/// HDRI are not carried, and duplicate material names come back made unique.
pub fn scene_to_polyops(scene: &Scene) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "polyops {VERSION}");
    let _ = writeln!(o, "# Polyamour scene: {} objects, {} meshes, {} materials", scene.objects.len(), scene.meshes.len(), scene.materials.len());
    let _ = writeln!(o, "scene name={} frame_start={} frame_end={} fps={}", quote(&scene.name), scene.frame_start, scene.frame_end, fmt_f(scene.fps));
    let w = &scene.world;
    let _ = writeln!(o, "world color={} strength={}", fmt_list(&w.color.to_array()), fmt_f(w.strength));
    // Collection 0 exists in every scene; create the others, then fix parents
    // (a parent may come later in the list).
    for (i, c) in scene.collections.iter().enumerate() {
        if i == 0 {
            if c.name != "Collection" || !c.visible {
                let _ = writeln!(o, "collection_set Collection name={} visible={}", quote(&c.name), c.visible as u8);
            }
        } else {
            let _ = writeln!(o, "collection_new {}{}", quote(&c.name), if c.visible { "" } else { " visible=0" });
        }
    }
    for c in &scene.collections {
        if let Some(p) = c.parent {
            let _ = writeln!(o, "collection_set {} parent={}", quote(&c.name), quote(&scene.collections[p].name));
        }
    }
    for m in &scene.materials {
        let c = m.base_color;
        let _ = writeln!(
            o,
            "material_new {} base_color={} metallic={} roughness={} emission={} emission_strength={} ior={} transmission={} double_sided={}",
            quote(&m.name),
            fmt_list(&c),
            fmt_f(m.metallic),
            fmt_f(m.roughness),
            fmt_list(&m.emission.to_array()),
            fmt_f(m.emission_strength),
            fmt_f(m.ior),
            fmt_f(m.transmission),
            m.double_sided as u8
        );
    }
    for (m, n) in scene.meshes.iter().zip(&scene.mesh_names) {
        let _ = writeln!(o, "mesh_new {}{}", quote(n), mesh_to_options(m));
    }
    for ob in &scene.objects {
        let t = &ob.transform;
        let place = format!(
            " location={} quaternion={} scale={} collection={}{}",
            fmt_list(&t.location.to_array()),
            fmt_list(&[t.rotation.w, t.rotation.x, t.rotation.y, t.rotation.z]),
            fmt_list(&t.scale.to_array()),
            quote(&scene.collections.get(ob.collection).map(|c| c.name.as_str()).unwrap_or("Collection")),
            if ob.visible { "" } else { " visible=0" }
        );
        let name = quote(&ob.name);
        let _ = match &ob.data {
            ObjectData::Empty => writeln!(o, "add_empty {name}{place}"),
            ObjectData::Mesh(m) => writeln!(o, "add_object {name} data={}{place}", quote(&scene.mesh_names[*m])),
            ObjectData::Light(l) => {
                let (kind, extra) = match l.kind {
                    LightKind::Point => ("point", String::new()),
                    LightKind::Sun => ("sun", String::new()),
                    LightKind::Spot { angle, blend } => ("spot", format!(" angle={} blend={}", fmt_f(angle), fmt_f(blend))),
                    LightKind::Area { size } => ("area", format!(" size={}", fmt_f(size))),
                };
                writeln!(o, "light_add {kind} name={name} power={} color={} radius={}{extra}{place}", fmt_f(l.power), fmt_list(&l.color.to_array()), fmt_f(l.radius))
            }
            ObjectData::Camera(c) => {
                let ortho = c.ortho_scale.map(|s| format!(" ortho_scale={}", fmt_f(s))).unwrap_or_default();
                writeln!(o, "camera_add name={name} fov={} near={} far={}{ortho} active=0{place}", fmt_f(c.fov_y), fmt_f(c.near), fmt_f(c.far))
            }
        };
    }
    for ob in &scene.objects {
        let name = quote(&ob.name);
        if let Some(p) = ob.parent {
            let _ = writeln!(o, "parent {name} {}", quote(&scene.objects[p].name));
        }
        for (slot, &m) in ob.materials.iter().enumerate() {
            let _ = writeln!(o, "material_assign {name} {} slot={slot}", quote(&scene.materials[m].name));
        }
        for m in &ob.modifiers {
            let _ = writeln!(o, "modifier_add {name} {}{}", m.name(), modifier_to_options(m));
        }
    }
    let names = |v: &[usize]| v.iter().map(|&i| format!(" {}", quote(&scene.objects[i].name))).collect::<String>();
    let _ = writeln!(o, "select{}", names(&scene.selected));
    let one = |i: Option<usize>| i.map(|i| quote(&scene.objects[i].name)).unwrap_or_else(|| "none".into());
    let _ = writeln!(o, "active {}", one(scene.active_object));
    let _ = writeln!(o, "active_camera {}", one(scene.active_camera));
    o.push_str("end\n");
    o
}
