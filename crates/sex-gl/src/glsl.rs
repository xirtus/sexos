// SPDX-License-Identifier: GPL-3.0-only
//! GLSL ES (WebGL's `#version 100` and `#version 300 es`) to the Vulkan-flavoured GLSL 450
//! that naga's front end reads. The source is preprocessed first (pp-rs), then top-level
//! declarations are rewritten:
//! - loose uniforms go into one-member uniform blocks, so their names stay global;
//! - `sampler2D x` becomes a texture and a sampler, and each use of `x` becomes
//!   `sampler2D(x_sxt, x_sxs)`;
//! - attributes, varyings and fragment outputs get explicit locations (varyings are
//!   matched between the two stages by name);
//! - ES 1.00 built-ins (`texture2D`, `gl_FragColor`, ...) get their GLSL 4 spellings.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use pp_rs::pp::Preprocessor;
use pp_rs::token::{Punct, TokenValue};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Vertex,
    Fragment,
}

/// What a sampler uniform samples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SamplerKind {
    D2,
    Cube,
    D3,
    D2Array,
    D2Shadow,
}

#[derive(Clone, Debug)]
pub struct SamplerDecl {
    pub name: String,
    pub kind: SamplerKind,
    /// Array length, 1 for a single sampler.
    pub count: u32,
}

/// A translated shader and the interface the rewrite found.
#[derive(Clone, Debug, Default)]
pub struct Translated {
    pub source: String,
    /// Vertex attributes: name and location.
    pub attributes: Vec<(String, u32)>,
    pub samplers: Vec<SamplerDecl>,
    /// Fragment outputs: name and location.
    pub outputs: Vec<(String, u32)>,
    /// `true` for `#version 300 es`.
    pub es3: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Int(u64, bool),
    Float(f32),
    P(Punct),
}

fn punct_str(p: Punct) -> &'static str {
    use Punct::*;
    match p {
        AddAssign => "+=", SubAssign => "-=", MulAssign => "*=", DivAssign => "/=", ModAssign => "%=",
        LeftShiftAssign => "<<=", RightShiftAssign => ">>=", AndAssign => "&=", XorAssign => "^=", OrAssign => "|=",
        Increment => "++", Decrement => "--", LogicalAnd => "&&", LogicalOr => "||", LogicalXor => "^^",
        LessEqual => "<=", GreaterEqual => ">=", EqualEqual => "==", NotEqual => "!=", LeftShift => "<<",
        RightShift => ">>", LeftBrace => "{", RightBrace => "}", LeftParen => "(", RightParen => ")",
        LeftBracket => "[", RightBracket => "]", LeftAngle => "<", RightAngle => ">", Semicolon => ";",
        Comma => ",", Colon => ":", Dot => ".", Equal => "=", Bang => "!", Minus => "-", Tilde => "~",
        Plus => "+", Star => "*", Slash => "/", Percent => "%", Pipe => "|", Caret => "^", Ampersand => "&",
        Question => "?",
    }
}

fn float_str(v: f32) -> String {
    let s = format!("{v:?}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") { s } else { s + ".0" }
}

/// Words GLSL 4.50 reserves that ES programs may use as names.
const RESERVED: &[&str] = &[
    "texture", "sample", "buffer", "shared", "coherent", "volatile", "restrict", "readonly", "writeonly", "patch",
    "subroutine", "common", "partition", "active", "filter", "resource", "noperspective", "superp", "input", "output",
    "image1D", "image2D", "image3D", "atomic_uint", "double", "dvec2", "dvec3", "dvec4", "precise", "sampler",
    "texture2D", "texture3D", "textureCube", "attribute", "varying", "unsigned", "half", "fixed", "hvec2", "hvec3",
    "hvec4", "fvec2", "fvec3", "fvec4", "cast", "namespace", "using", "public", "static", "extern", "external",
    "interface", "long", "short", "template", "this", "typedef", "union", "goto", "inline", "noinline", "class",
    "asm", "enum", "sizeof", "packed", "row_major", "lowp", "mediump", "highp", "precision", "invariant",
];

const ES1_BUILTINS: &[(&str, &str)] = &[
    ("texture2D", "texture"), ("textureCube", "texture"), ("texture2DProj", "textureProj"),
    ("texture2DLod", "textureLod"), ("texture2DLodEXT", "textureLod"), ("textureCubeLod", "textureLod"),
    ("textureCubeLodEXT", "textureLod"), ("texture2DProjLod", "textureProjLod"), ("texture2DProjLodEXT", "textureProjLod"),
    ("texture2DGradEXT", "textureGrad"), ("textureCubeGradEXT", "textureGrad"),
    ("shadow2D", "texture"), ("shadow2DEXT", "texture"),
];

fn sampler_kind(t: &str) -> Option<SamplerKind> {
    Some(match t {
        "sampler2D" => SamplerKind::D2,
        "samplerCube" => SamplerKind::Cube,
        "sampler3D" => SamplerKind::D3,
        "sampler2DArray" => SamplerKind::D2Array,
        "sampler2DShadow" => SamplerKind::D2Shadow,
        _ => return None,
    })
}

fn sampler_glsl(kind: SamplerKind) -> (&'static str, &'static str, &'static str) {
    match kind {
        SamplerKind::D2 => ("texture2D", "sampler", "sampler2D"),
        SamplerKind::Cube => ("textureCube", "sampler", "samplerCube"),
        SamplerKind::D3 => ("texture3D", "sampler", "sampler3D"),
        SamplerKind::D2Array => ("texture2DArray", "sampler", "sampler2DArray"),
        SamplerKind::D2Shadow => ("texture2D", "samplerShadow", "sampler2DShadow"),
    }
}

fn tokens(src: &str, es3: &mut bool) -> Result<Vec<Tok>, String> {
    let mut pp = Preprocessor::new(src);
    let _ = pp.add_define("GL_ES", "1");
    let _ = pp.add_define("GL_FRAGMENT_PRECISION_HIGH", "1");
    let mut out = Vec::new();
    for item in pp {
        let tok = item.map_err(|(e, loc)| format!("ERROR: 0:{}: preprocessor: {e:?}", loc.start))?;
        match tok.value {
            TokenValue::Ident(s) => out.push(Tok::Id(s)),
            TokenValue::Integer(i) => out.push(Tok::Int(i.value, i.signed)),
            TokenValue::Float(f) => out.push(Tok::Float(f.value)),
            TokenValue::Punct(p) => out.push(Tok::P(p)),
            TokenValue::Version(v) => {
                *es3 = v.tokens.iter().any(|t| matches!(&t.value, TokenValue::Integer(i) if i.value >= 300));
            }
            TokenValue::Extension(_) | TokenValue::Pragma(_) => {}
        }
    }
    Ok(out)
}

fn text(toks: &[Tok], out: &mut String) {
    for t in toks {
        match t {
            Tok::Id(s) => out.push_str(s),
            Tok::Int(v, signed) => { out.push_str(&v.to_string()); if !signed { out.push('u'); } }
            Tok::Float(v) => out.push_str(&float_str(*v)),
            Tok::P(p) => out.push_str(punct_str(*p)),
        }
        match t {
            Tok::P(Punct::Semicolon) | Tok::P(Punct::LeftBrace) | Tok::P(Punct::RightBrace) => out.push('\n'),
            _ => out.push(' '),
        }
    }
}

/// Splits top-level statements: declarations end at `;`, function definitions at their
/// closing brace.
fn statements(toks: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut paren_before_brace) = (0i32, 0usize, false);
    let mut seen_brace = false;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::P(Punct::LeftParen) if depth == 0 && !seen_brace => paren_before_brace = true,
            Tok::P(Punct::LeftBrace) => { if depth == 0 { seen_brace = true; } depth += 1; }
            Tok::P(Punct::RightBrace) => {
                depth -= 1;
                if depth == 0 && paren_before_brace {
                    out.push(&toks[start..=i]);
                    start = i + 1;
                    paren_before_brace = false;
                    seen_brace = false;
                }
            }
            Tok::P(Punct::Semicolon) if depth == 0 => {
                out.push(&toks[start..=i]);
                start = i + 1;
                paren_before_brace = false;
                seen_brace = false;
            }
            _ => {}
        }
    }
    if start < toks.len() { out.push(&toks[start..]); }
    out
}

fn is(t: &Tok, s: &str) -> bool { matches!(t, Tok::Id(x) if x == s) }

const QUALIFIERS: &[&str] = &["attribute", "varying", "uniform", "in", "out", "flat", "smooth", "centroid", "invariant", "const", "inout"];

/// One declarator: its name and the tokens after the name (array suffix, initializer).
struct Declarator<'a> {
    name: String,
    rest: &'a [Tok],
}

/// A top-level declaration: qualifiers, a layout (if any), its type tokens and declarators.
struct Decl<'a> {
    quals: Vec<String>,
    layout: Option<&'a [Tok]>,
    ty: &'a [Tok],
    decls: Vec<Declarator<'a>>,
}

fn parse_decl(stmt: &[Tok]) -> Option<Decl<'_>> {
    let mut i = 0;
    let mut quals = Vec::new();
    let mut layout = None;
    loop {
        match stmt.get(i)? {
            Tok::Id(s) if s == "layout" => {
                let start = i;
                let mut depth = 0;
                loop {
                    i += 1;
                    match stmt.get(i)? {
                        Tok::P(Punct::LeftParen) => depth += 1,
                        Tok::P(Punct::RightParen) => { depth -= 1; if depth == 0 { break; } }
                        _ => {}
                    }
                }
                layout = Some(&stmt[start..=i]);
                i += 1;
            }
            Tok::Id(s) if QUALIFIERS.contains(&s.as_str()) => { quals.push(s.clone()); i += 1; }
            Tok::Id(s) if matches!(s.as_str(), "lowp" | "mediump" | "highp") => i += 1,
            _ => break,
        }
    }
    if quals.is_empty() { return None; }
    let ty_start = i;
    // The type: one identifier, or `struct ... { ... }`.
    if is(stmt.get(i)?, "struct") { return None; }
    i += 1;
    // An interface block: `uniform Name { ... } inst;`
    if matches!(stmt.get(i), Some(Tok::P(Punct::LeftBrace))) { return Some(Decl { quals, layout, ty: &stmt[ty_start..], decls: Vec::new() }); }
    // Array types (`float[3] x`) keep their brackets in the type.
    while matches!(stmt.get(i), Some(Tok::P(Punct::LeftBracket))) {
        while !matches!(stmt.get(i)?, Tok::P(Punct::RightBracket)) { i += 1; }
        i += 1;
    }
    let ty = &stmt[ty_start..i];
    let mut decls = Vec::new();
    let body = &stmt[i..stmt.len().saturating_sub(1)];
    let mut j = 0;
    while j < body.len() {
        let Tok::Id(name) = &body[j] else { return None };
        let start = j + 1;
        let mut depth = 0;
        let mut k = start;
        while k < body.len() {
            match &body[k] {
                Tok::P(Punct::LeftParen) | Tok::P(Punct::LeftBracket) => depth += 1,
                Tok::P(Punct::RightParen) | Tok::P(Punct::RightBracket) => depth -= 1,
                Tok::P(Punct::Comma) if depth == 0 => break,
                _ => {}
            }
            k += 1;
        }
        decls.push(Declarator { name: name.clone(), rest: &body[start..k] });
        j = k + 1;
    }
    Some(Decl { quals, layout, ty, decls })
}

fn array_len(rest: &[Tok]) -> Option<u32> {
    match rest {
        [Tok::P(Punct::LeftBracket), Tok::Int(n, _), Tok::P(Punct::RightBracket), ..] => Some(*n as u32),
        _ => None,
    }
}

/// Translates one stage. `varyings` is shared by the two stages of a program and gives each
/// varying name its location.
pub fn translate(src: &str, stage: Stage, varyings: &mut Vec<String>) -> Result<Translated, String> {
    let mut es3 = false;
    let toks = tokens(src, &mut es3)?;
    let mut out = Translated { es3, ..Translated::default() };
    let mut head = String::from("#version 450\n");
    let mut body: Vec<Tok> = Vec::new();
    let mut binding = 0u32;
    let mut samplers: BTreeMap<String, SamplerKind> = BTreeMap::new();
    let mut bools: BTreeMap<String, &'static str> = BTreeMap::new();
    let mut attr_loc = 0u32;
    let mut out_loc = 0u32;
    let mut uses_frag_color = false;
    let mut uses_frag_data = false;
    let mut uses_modf = false;

    let rename = |t: &Tok| -> Tok {
        match t {
            Tok::Id(s) if !es3 => {
                if let Some((_, to)) = ES1_BUILTINS.iter().find(|(from, _)| from == s) { return Tok::Id((*to).to_string()); }
                if RESERVED.contains(&s.as_str()) && !matches!(s.as_str(), "attribute" | "varying" | "lowp" | "mediump" | "highp" | "precision" | "invariant") {
                    return Tok::Id(format!("{s}_sx"));
                }
                t.clone()
            }
            Tok::Id(s) if es3 && RESERVED.contains(&s.as_str()) && !matches!(s.as_str(), "texture" | "lowp" | "mediump" | "highp" | "precision" | "invariant") => {
                Tok::Id(format!("{s}_sx"))
            }
            _ => t.clone(),
        }
    };

    for stmt in statements(&toks) {
        if stmt.is_empty() { continue; }
        if is(&stmt[0], "precision") { continue; }
        if is(&stmt[0], "invariant") && stmt.len() <= 3 { continue; }
        let Some(d) = parse_decl(stmt) else {
            body.extend(stmt.iter().filter(|t| !matches!(t, Tok::Id(s) if s == "lowp" || s == "mediump" || s == "highp")).map(rename));
            continue;
        };
        let has = |q: &str| d.quals.iter().any(|x| x == q);
        let ty_toks: Vec<Tok> = d.ty.iter().filter(|t| !matches!(t, Tok::Id(s) if s == "lowp" || s == "mediump" || s == "highp")).map(rename).collect();
        let mut ty_text = String::new();
        text(&ty_toks, &mut ty_text);
        let interp = if has("flat") { "flat " } else { "" };
        if has("uniform") {
            if d.decls.is_empty() {
                // An interface block: give it a binding, keep the rest.
                let mut t = format!("layout(set=0, binding={binding}) uniform ");
                binding += 1;
                text(&d.ty.iter().map(rename).collect::<Vec<_>>(), &mut t);
                t.push('\n');
                body.push(Tok::Id(t));
                continue;
            }
            if let Some(kind) = d.ty.first().and_then(|t| if let Tok::Id(s) = t { sampler_kind(s) } else { None }) {
                let (tex, smp, _) = sampler_glsl(kind);
                for dc in &d.decls {
                    let count = array_len(dc.rest).unwrap_or(1);
                    let suffix = if array_len(dc.rest).is_some() { format!("[{count}]") } else { String::new() };
                    body.push(Tok::Id(format!("layout(set=0, binding={binding}) uniform {tex} {}_sxt{suffix};\n", dc.name)));
                    body.push(Tok::Id(format!("layout(set=0, binding={}) uniform {smp} {}_sxs{suffix};\n", binding + 1, dc.name)));
                    binding += 2;
                    samplers.insert(dc.name.clone(), kind);
                    out.samplers.push(SamplerDecl { name: dc.name.clone(), kind, count });
                }
                continue;
            }
            // Booleans can't live in a uniform block: keep them as uint, and turn each use
            // back into a boolean.
            let as_bool = match ty_text.trim() { "bool" => Some(("uint", "bool")), "bvec2" => Some(("uvec2", "bvec2")), "bvec3" => Some(("uvec3", "bvec3")), "bvec4" => Some(("uvec4", "bvec4")), _ => None };
            for dc in &d.decls {
                let mut rest = String::new();
                text(dc.rest, &mut rest);
                if let Some((stored, ctor)) = as_bool {
                    body.push(Tok::Id(format!("layout(set=0, binding={binding}) uniform _sxu{binding} {{ {stored} {}_sxb {rest}; }};\n", dc.name)));
                    bools.insert(dc.name.clone(), ctor);
                    binding += 1;
                    continue;
                }
                body.push(Tok::Id(format!("layout(set=0, binding={binding}) uniform _sxu{binding} {{ {ty_text} {} {rest}; }};\n", dc.name)));
                binding += 1;
            }
            continue;
        }
        let is_in = has("in") || has("attribute") || (has("varying") && stage == Stage::Fragment);
        let is_out = has("out") || (has("varying") && stage == Stage::Vertex);
        if stage == Stage::Vertex && is_in {
            for dc in &d.decls {
                let loc = explicit_location(d.layout).unwrap_or(attr_loc);
                attr_loc = attr_loc.max(loc + 1);
                body.push(Tok::Id(format!("layout(location={loc}) in {ty_text} {};\n", dc.name)));
                out.attributes.push((dc.name.clone(), loc));
            }
            continue;
        }
        if (stage == Stage::Vertex && is_out) || (stage == Stage::Fragment && is_in) {
            let dir = if is_out { "out" } else { "in" };
            for dc in &d.decls {
                let loc = match varyings.iter().position(|v| *v == dc.name) {
                    Some(i) => i,
                    None => { varyings.push(dc.name.clone()); varyings.len() - 1 },
                };
                let mut rest = String::new();
                text(dc.rest, &mut rest);
                body.push(Tok::Id(format!("layout(location={loc}) {interp}{dir} {ty_text} {} {rest};\n", dc.name)));
            }
            continue;
        }
        if stage == Stage::Fragment && is_out {
            for dc in &d.decls {
                let loc = explicit_location(d.layout).unwrap_or(out_loc);
                out_loc = out_loc.max(loc + 1);
                body.push(Tok::Id(format!("layout(location={loc}) out {ty_text} {};\n", dc.name)));
                out.outputs.push((dc.name.clone(), loc));
            }
            continue;
        }
        // Constants and other globals stay as they are.
        body.extend(stmt.iter().filter(|t| !matches!(t, Tok::Id(s) if s == "lowp" || s == "mediump" || s == "highp")).map(rename));
    }

    // Uses of samplers, gl_FragColor and gl_FragData.
    let mut fixed: Vec<Tok> = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        match &body[i] {
            Tok::Id(s) if samplers.contains_key(s) && !matches!(fixed.last(), Some(Tok::P(Punct::Dot))) => {
                let kind = samplers[s];
                let (_, _, combined) = sampler_glsl(kind);
                // `name[i]` indexes both halves.
                let mut index: Vec<Tok> = Vec::new();
                let mut j = i + 1;
                if matches!(body.get(j), Some(Tok::P(Punct::LeftBracket))) {
                    let mut depth = 0;
                    loop {
                        let t = body.get(j).cloned().unwrap_or(Tok::P(Punct::RightBracket));
                        match t { Tok::P(Punct::LeftBracket) => depth += 1, Tok::P(Punct::RightBracket) => depth -= 1, _ => {} }
                        index.push(t);
                        j += 1;
                        if depth == 0 { break; }
                    }
                }
                fixed.push(Tok::Id(combined.into()));
                fixed.push(Tok::P(Punct::LeftParen));
                fixed.push(Tok::Id(format!("{s}_sxt")));
                fixed.extend(index.iter().cloned());
                fixed.push(Tok::P(Punct::Comma));
                fixed.push(Tok::Id(format!("{s}_sxs")));
                fixed.extend(index);
                fixed.push(Tok::P(Punct::RightParen));
                i = j;
                continue;
            }
            Tok::Id(s) if bools.contains_key(s) && !matches!(fixed.last(), Some(Tok::P(Punct::Dot))) => {
                let mut j = i + 1;
                let mut index: Vec<Tok> = Vec::new();
                if matches!(body.get(j), Some(Tok::P(Punct::LeftBracket))) {
                    let mut depth = 0;
                    loop {
                        let t = body.get(j).cloned().unwrap_or(Tok::P(Punct::RightBracket));
                        match t { Tok::P(Punct::LeftBracket) => depth += 1, Tok::P(Punct::RightBracket) => depth -= 1, _ => {} }
                        index.push(t);
                        j += 1;
                        if depth == 0 { break; }
                    }
                }
                fixed.push(Tok::Id(bools[s].into()));
                fixed.push(Tok::P(Punct::LeftParen));
                fixed.push(Tok::Id(format!("{s}_sxb")));
                fixed.extend(index);
                fixed.push(Tok::P(Punct::RightParen));
                i = j;
                continue;
            }
            // `const` on a parameter (`f(const in float x)`): naga takes none.
            Tok::Id(s) if s == "const" && matches!(fixed.last(), Some(Tok::P(Punct::LeftParen)) | Some(Tok::P(Punct::Comma))) => {}
            Tok::Id(s) if s == "modf" => { uses_modf = true; fixed.push(Tok::Id("sx_modf".into())); }
            Tok::Id(s) if s == "gl_FragColor" => { uses_frag_color = true; fixed.push(Tok::Id("sx_FragColor".into())); }
            Tok::Id(s) if s == "gl_FragData" => { uses_frag_data = true; fixed.push(Tok::Id("sx_FragData".into())); }
            t => fixed.push(t.clone()),
        }
        i += 1;
    }
    // Built-ins naga's GLSL front end lacks.
    if uses_modf {
        for t in ["float", "vec2", "vec3", "vec4"] {
            head.push_str(&format!("{t} sx_modf({t} x, out {t} i) {{ i = trunc(x); return x - i; }}\n"));
        }
    }
    if stage == Stage::Fragment && uses_frag_color {
        head.push_str("layout(location=0) out vec4 sx_FragColor;\n");
        out.outputs.push(("gl_FragColor".into(), 0));
    }
    if stage == Stage::Fragment && uses_frag_data {
        head.push_str("layout(location=0) out vec4 sx_FragData0;\n");
        out.outputs.push(("gl_FragData".into(), 0));
        // Only gl_FragData[0] (no draw buffers): `sx_FragData [ 0 ]` -> `sx_FragData0`.
        let mut k = 0;
        let mut f2 = Vec::with_capacity(fixed.len());
        while k < fixed.len() {
            if matches!(&fixed[k], Tok::Id(s) if s == "sx_FragData") {
                if let (Some(Tok::P(Punct::LeftBracket)), Some(Tok::Int(0, _)), Some(Tok::P(Punct::RightBracket))) = (fixed.get(k + 1), fixed.get(k + 2), fixed.get(k + 3)) {
                    f2.push(Tok::Id("sx_FragData0".into()));
                    k += 4;
                    continue;
                }
                return Err("ERROR: gl_FragData beyond index 0 is not supported".into());
            }
            f2.push(fixed[k].clone());
            k += 1;
        }
        fixed = f2;
    }
    let mut src = head;
    text(&fixed, &mut src);
    out.source = src;
    Ok(out)
}

fn explicit_location(layout: Option<&[Tok]>) -> Option<u32> {
    let l = layout?;
    let i = l.iter().position(|t| is(t, "location"))?;
    match (l.get(i + 1), l.get(i + 2)) {
        (Some(Tok::P(Punct::Equal)), Some(Tok::Int(n, _))) => Some(*n as u32),
        _ => None,
    }
}
