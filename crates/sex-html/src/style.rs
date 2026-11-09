// SPDX-License-Identifier: GPL-3.0-only
//! The cascade: which declarations apply to each element, in which order, and the
//! computed values that layout reads. Lengths stay relative (`%`, `auto`) where only
//! layout can resolve them; `em`/`rem`/`pt` become pixels here.
use crate::css::{self, Declaration, Sheet};
use crate::dom::{Dom, Kind, NodeId};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display { None, Block, Inline, InlineBlock, ListItem, Table, TableRow, TableCell, Flex, Grid, Contents }

/// One `grid-template-columns` track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track { Len(Len), /// hundredths of an `fr`
    Fr(i32), Auto, /// `minmax(min, max)` whose max is a flexible (`fr`, hundredths) or, with 0, `auto` size
    MinMax(Len, i32) }

/// Where a grid item goes along the columns: lines are 1-based, negative ones count from
/// the end, 0 is `auto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GridPlace { pub start: i32, pub end: i32, pub span: i32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Len { Auto, Px(i32), /// hundredths of a percent
    Pct(i32) }

impl Len {
    pub fn resolve(self, base: i32) -> Option<i32> {
        match self { Len::Auto => None, Len::Px(p) => Some(p), Len::Pct(p) => Some((base as i64 * p as i64 / 10000) as i32) }
    }
    pub fn or_zero(self, base: i32) -> i32 { self.resolve(base).unwrap_or(0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align { Left, Center, Right, Justify }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhiteSpace { Normal, NoWrap, Pre, PreWrap, PreLine }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListStyle { None, Disc, Circle, Square, Decimal, LowerAlpha, UpperAlpha, LowerRoman, UpperRoman }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform { None, Upper, Lower, Capitalize }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position { Static, Relative, Absolute, Fixed, Sticky }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify { Start, Center, End, SpaceBetween, SpaceAround, SpaceEvenly }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Float { None, Left, Right }

/// Colours are 0xAARRGGBB (straight alpha).
#[derive(Clone, Debug)]
pub struct Style {
    pub display: Display,
    pub color: u32,
    pub background: u32,
    pub font_size: i32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// Pixels; 0 means `normal`.
    pub line_height: i32,
    pub align: Align,
    pub white_space: WhiteSpace,
    pub underline: bool,
    pub strike: bool,
    pub transform: Transform,
    pub list_style: ListStyle,
    pub hidden: bool,
    pub margin: [Len; 4],
    pub padding: [Len; 4],
    pub border: [i32; 4],
    pub border_color: [u32; 4],
    pub radius: i32,
    pub width: Len,
    pub height: Len,
    pub min_width: Len,
    pub max_width: Len,
    pub min_height: Len,
    pub position: Position,
    /// `top`, `right`, `bottom`, `left` for positioned boxes.
    pub inset: [Len; 4],
    /// `z-index`; `None` is `auto`.
    pub z_index: Option<i32>,
    pub float: Float,
    /// `clear`: bit 0 left, bit 1 right.
    pub clear: u8,
    /// `overflow` other than `visible`: the box contains its floats.
    pub overflow_clip: bool,
    pub flex_column: bool,
    pub flex_wrap: bool,
    pub flex_grow: i32,
    /// `flex-basis`; `Auto` means the content size.
    pub flex_basis: Len,
    pub border_collapse: bool,
    /// `border-spacing` in pixels, when set.
    pub border_spacing: Option<i32>,
    pub justify: Justify,
    pub align_center: bool,
    pub gap: i32,
    pub row_gap: i32,
    pub grid_columns: Vec<Track>,
    /// `repeat(auto-fill | auto-fit, track)`: the track, and whether empty tracks collapse.
    pub grid_auto: Option<(Track, bool)>,
    pub grid_column: GridPlace,
    pub box_sizing_border: bool,
    pub letter_spacing: i32,
    pub vertical_super: i8,
    /// `transform: translate(..)` (percentages are of the box's own size).
    pub translate: Option<(Len, Len)>,
    /// The first outer `box-shadow`: x, y, blur, spread and colour.
    pub shadow: Option<(i32, i32, i32, i32, u32)>,
    /// `opacity`, 0 to 255.
    pub opacity: u8,
    /// Custom properties (`--name: value`), inherited; shared until an element sets one.
    pub vars: alloc::rc::Rc<BTreeMap<String, String>>,
}

pub const TOP: usize = 0;
pub const RIGHT: usize = 1;
pub const BOTTOM: usize = 2;
pub const LEFT: usize = 3;

impl Style {
    pub fn initial() -> Self {
        Style {
            display: Display::Inline,
            color: 0xFF20_2124,
            background: 0,
            font_size: 16,
            bold: false,
            italic: false,
            mono: false,
            line_height: 0,
            align: Align::Left,
            white_space: WhiteSpace::Normal,
            underline: false,
            strike: false,
            transform: Transform::None,
            list_style: ListStyle::Disc,
            hidden: false,
            margin: [Len::Px(0); 4],
            padding: [Len::Px(0); 4],
            border: [0; 4],
            border_color: [0xFF00_0000; 4],
            radius: 0,
            width: Len::Auto,
            height: Len::Auto,
            min_width: Len::Auto,
            max_width: Len::Auto,
            min_height: Len::Auto,
            position: Position::Static,
            inset: [Len::Auto; 4],
            z_index: None,
            float: Float::None,
            clear: 0,
            overflow_clip: false,
            flex_column: false,
            flex_wrap: false,
            flex_grow: 0,
            flex_basis: Len::Auto,
            border_collapse: false,
            border_spacing: None,
            justify: Justify::Start,
            align_center: false,
            gap: 0,
            row_gap: 0,
            grid_columns: Vec::new(),
            grid_auto: None,
            grid_column: GridPlace::default(),
            box_sizing_border: false,
            letter_spacing: 0,
            vars: Default::default(),
            opacity: 255,
            shadow: None,
            translate: None,
            vertical_super: 0,
        }
    }

    /// The inherited properties of `parent`, the rest initial.
    pub fn inherit(parent: &Style) -> Self {
        let mut s = Style::initial();
        s.color = parent.color;
        s.font_size = parent.font_size;
        s.bold = parent.bold;
        s.italic = parent.italic;
        s.mono = parent.mono;
        s.line_height = parent.line_height;
        s.align = parent.align;
        s.white_space = parent.white_space;
        s.underline = parent.underline;
        s.strike = parent.strike;
        s.transform = parent.transform;
        s.list_style = parent.list_style;
        s.hidden = parent.hidden;
        s.letter_spacing = parent.letter_spacing;
        s.border_collapse = parent.border_collapse;
        s.border_spacing = parent.border_spacing;
        s.vars = parent.vars.clone();
        s
    }

    pub fn line_height(&self) -> i32 {
        if self.line_height > 0 { self.line_height } else { (self.font_size * 5 + 2) / 4 }
    }

    pub fn is_block_level(&self) -> bool {
        matches!(self.display, Display::Block | Display::ListItem | Display::Table | Display::TableRow | Display::TableCell | Display::Flex | Display::Grid)
    }
}

/// The browser's own style sheet: what elements look like before a page says otherwise.
pub const USER_AGENT_CSS: &str = r#"
html, address, blockquote, body, center, dd, details, dialog, dir, div, dl, dt, fieldset, figcaption, figure,
footer, form, header, hgroup, hr, legend, main, menu, nav, ol, p, pre, section, summary, ul, article, aside,
h1, h2, h3, h4, h5, h6, li, table, caption, thead, tbody, tfoot, tr, td, th, optgroup, picture, search { display: block }
head, script, style, link, meta, title, template, noscript, datalist, param, base, area, map, [hidden], input[type=hidden],
dialog:not([open]), noembed, noframes, rp { display: none }
li { display: list-item }
table { display: table }
tr { display: table-row }
td, th { display: table-cell; padding: 1px 3px }
th { font-weight: bold; text-align: center }
caption { text-align: center }
body { margin: 8px; line-height: 1.4 }
p, dl, figure, blockquote { margin: 1em 0 }
blockquote, figure { margin-left: 40px; margin-right: 40px }
blockquote { color: #555; border-left: 3px solid #ccc; padding-left: 12px; margin-left: 16px }
h1 { font-size: 2em; margin: 0.67em 0; font-weight: bold; line-height: 1.2 }
h2 { font-size: 1.5em; margin: 0.83em 0; font-weight: bold; line-height: 1.25 }
h3 { font-size: 1.17em; margin: 1em 0; font-weight: bold }
h4 { margin: 1.33em 0; font-weight: bold }
h5 { font-size: 0.83em; margin: 1.67em 0; font-weight: bold }
h6 { font-size: 0.67em; margin: 2.33em 0; font-weight: bold }
ul, ol, menu, dir { margin: 1em 0; padding-left: 40px }
ol { list-style-type: decimal }
ul ul, ol ul { list-style-type: circle }
ul ul ul, ol ul ul, ul ol ul { list-style-type: square }
ul ul, ul ol, ol ol, ol ul { margin-top: 0; margin-bottom: 0 }
dd { margin-left: 40px }
dt { font-weight: bold }
b, strong { font-weight: bold }
i, em, cite, var, dfn, address { font-style: italic }
u, ins { text-decoration: underline }
s, strike, del { text-decoration: line-through }
code, kbd, samp, tt, pre, xmp, listing, plaintext { font-family: monospace }
code, kbd, samp, tt { font-size: 0.9em; background: #f1f3f4; border-radius: 3px }
pre { white-space: pre; margin: 1em 0; background: #f6f8fa; padding: 8px; font-size: 0.9em }
pre code { background: transparent }
small { font-size: 0.83em }
big { font-size: 1.17em }
sub { font-size: 0.83em; vertical-align: sub }
sup { font-size: 0.83em; vertical-align: super }
mark { background: #fff59d; color: black }
a:link { color: #0b57d0; text-decoration: underline }
hr { border: 0; border-top: 1px solid #c4c7c5; margin: 0.5em 0; height: 0 }
center { text-align: center }
img { display: inline-block }
button, input, select, textarea { display: inline-block; border: 1px solid #8e918f; padding: 2px 6px; background: #f1f3f4; border-radius: 4px }
fieldset { border: 1px solid #c4c7c5; padding: 0.35em 0.75em 0.625em; margin: 0 2px }
summary { font-weight: bold }
"#;

#[derive(Clone, Copy)]
struct Applied { key: u64, rule: u32, decl: u32 }

/// Every applicable declaration, ordered: user agent, then author (by specificity,
/// then order), then `!important` author, then the element's `style` attribute.
pub struct Cascade<'a> {
    sheets: Vec<&'a Sheet>,
    by_id: BTreeMap<&'a str, Vec<(u16, u32, u16)>>,
    by_class: BTreeMap<&'a str, Vec<(u16, u32, u16)>>,
    by_tag: BTreeMap<&'a str, Vec<(u16, u32, u16)>>,
    universal: Vec<(u16, u32, u16)>,
}

impl<'a> Cascade<'a> {
    /// `sheets[0]` is the user-agent sheet; the rest are the page's, in document order.
    pub fn new(sheets: Vec<&'a Sheet>) -> Self {
        let mut c = Cascade { sheets, by_id: BTreeMap::new(), by_class: BTreeMap::new(), by_tag: BTreeMap::new(), universal: Vec::new() };
        for (si, sheet) in c.sheets.iter().enumerate() {
            for (ri, rule) in sheet.rules.iter().enumerate() {
                for (xi, sel) in rule.selectors.iter().enumerate() {
                    let key = (si as u16, ri as u32, xi as u16);
                    let last = sel.parts.last().unwrap();
                    if let Some(id) = &last.id { c.by_id.entry(id.as_str()).or_default().push(key); }
                    else if let Some(class) = last.classes.first() { c.by_class.entry(class.as_str()).or_default().push(key); }
                    else if let Some(tag) = &last.tag { c.by_tag.entry(tag.as_str()).or_default().push(key); }
                    else { c.universal.push(key); }
                }
            }
        }
        c
    }

    /// (normal, important) declarations in cascade order.
    fn declarations(&self, dom: &Dom, node: NodeId) -> (Vec<&'a Declaration>, Vec<&'a Declaration>) {
        let mut found: Vec<Applied> = Vec::new();
        let mut consider = |list: Option<&Vec<(u16, u32, u16)>>| {
            for &(si, ri, xi) in list.into_iter().flatten() {
                let sel = &self.sheets[si as usize].rules[ri as usize].selectors[xi as usize];
                if css::matches(dom, node, sel) {
                    let origin = if si == 0 { 0u64 } else { 1 };
                    found.push(Applied { key: (origin << 62) | ((sel.specificity as u64) << 32) | ((si as u64) << 24), rule: ri, decl: si as u32 });
                }
            }
        };
        if let Some(id) = dom.attr(node, "id") { consider(self.by_id.get(id)); }
        if let Some(classes) = dom.attr(node, "class") {
            let mut seen: Vec<&str> = Vec::new();
            for class in classes.split_ascii_whitespace() {
                if seen.contains(&class) { continue; }
                seen.push(class);
                consider(self.by_class.get(class));
            }
        }
        if let Some(tag) = dom.tag(node) { consider(self.by_tag.get(tag)); }
        consider(Some(&self.universal));
        // Same selector matched through two buckets counts once; order is stable.
        found.sort_by(|a, b| a.key.cmp(&b.key).then(a.decl.cmp(&b.decl)).then(a.rule.cmp(&b.rule)));
        found.dedup_by(|a, b| a.key == b.key && a.rule == b.rule && a.decl == b.decl);
        let mut normal = Vec::new();
        let mut important = Vec::new();
        for a in &found {
            for d in &self.sheets[a.decl as usize].rules[a.rule as usize].declarations {
                if d.important { important.push(d) } else { normal.push(d) }
            }
        }
        (normal, important)
    }
}

pub fn compute(dom: &Dom, cascade: &Cascade, node: NodeId, parent: &Style, root_font: i32, viewport: (i32, i32)) -> Style {
    let mut style = Style::inherit(parent);
    let Kind::Element { .. } = &dom.nodes[node].kind else { return style };
    let (normal, important) = cascade.declarations(dom, node);
    let inline = dom.attr(node, "style").map(css::parse_declarations).unwrap_or_default();
    let ctx = Ctx { parent, root_font, viewport };
    presentational_hints(dom, node, &mut style, &ctx);
    let ordered: Vec<&Declaration> = normal.iter().copied()
        .chain(inline.iter().filter(|d| !d.important))
        .chain(important.iter().copied())
        .chain(inline.iter().filter(|d| d.important))
        .collect();
    // Custom properties first, then var() references in every other value.
    if ordered.iter().any(|d| d.name.starts_with("--")) {
        let mut vars = (*style.vars).clone();
        for d in &ordered {
            if let Some(name) = d.name.strip_prefix("--") {
                match substitute_vars(&d.value, &vars, 0) { Some(v) => { vars.insert(String::from(name), v); } None => { vars.remove(name); } }
            }
        }
        style.vars = alloc::rc::Rc::new(vars);
    }
    let substituted: Vec<Declaration>;
    let ordered: Vec<&Declaration> = if ordered.iter().any(|d| d.value.contains("var(")) {
        substituted = ordered.iter().filter(|d| !d.name.starts_with("--")).filter_map(|d| {
            if !d.value.contains("var(") { return Some((*d).clone()); }
            substitute_vars(&d.value, &style.vars, 0).map(|value| Declaration { name: d.name.clone(), value, important: d.important })
        }).collect();
        substituted.iter().collect()
    } else { ordered };
    // font-size first: other `em` lengths depend on it.
    for d in &ordered { if d.name == "font-size" || d.name == "font" { apply(&mut style, d, &ctx, true); } }
    for d in &ordered { apply(&mut style, d, &ctx, false); }
    if style.position == Position::Absolute || style.position == Position::Fixed {
        if style.display == Display::Inline { style.display = Display::Block; }
    }
    if style.float != Float::None && style.display == Display::Inline { style.display = Display::Block; }
    style
}

struct Ctx<'p> { parent: &'p Style, root_font: i32, viewport: (i32, i32) }

/// Old HTML attributes that still style pages (`bgcolor`, `width`, `align`, `<font>`).
fn presentational_hints(dom: &Dom, node: NodeId, s: &mut Style, ctx: &Ctx) {
    let tag = dom.tag(node).unwrap_or("");
    if let Some(c) = dom.attr(node, "bgcolor").and_then(parse_color) { s.background = c; }
    if matches!(tag, "td" | "th" | "table" | "img" | "hr" | "div" | "col" | "iframe" | "canvas" | "video") {
        if let Some(w) = dom.attr(node, "width").and_then(|v| parse_len(v, s, ctx)) { s.width = w; }
        if let Some(h) = dom.attr(node, "height").and_then(|v| parse_len(v, s, ctx)) { s.height = h; }
    }
    if let Some(a) = dom.attr(node, "align") {
        match a.to_ascii_lowercase().as_str() {
            "center" | "middle" if tag != "img" => {
                if tag == "table" { s.margin[LEFT] = Len::Auto; s.margin[RIGHT] = Len::Auto; } else { s.align = Align::Center; }
            }
            "right" if tag != "img" => s.align = Align::Right,
            _ => {}
        }
    }
    if tag == "font" {
        if let Some(c) = dom.attr(node, "color").and_then(parse_color) { s.color = c; }
        if let Some(size) = dom.attr(node, "size") {
            let n: i32 = size.trim_start_matches(['+', '-']).parse().unwrap_or(3);
            let n = if size.starts_with('+') { 3 + n } else if size.starts_with('-') { 3 - n } else { n };
            s.font_size = [10, 13, 16, 18, 24, 32, 48][n.clamp(1, 7) as usize - 1];
        }
        if let Some(face) = dom.attr(node, "face") { s.mono = is_mono(face); }
    }
    if tag == "body" {
        if let Some(c) = dom.attr(node, "text").and_then(parse_color) { s.color = c; }
    }
    if tag == "table" {
        if let Some(b) = dom.attr(node, "border").and_then(|v| v.trim().parse::<i32>().ok()) {
            if b > 0 { s.border = [b; 4]; s.border_color = [0xFF80_8080; 4]; }
        }
    }
    if matches!(tag, "td" | "th") {
        if let Some(p) = table_attr(dom, node, "cellpadding").and_then(|v| v.trim().parse::<i32>().ok()) { s.padding = [Len::Px(p); 4]; }
        if let Some(b) = table_attr(dom, node, "border").and_then(|v| v.trim().parse::<i32>().ok()) {
            if b > 0 { s.border = [1; 4]; s.border_color = [0xFF80_8080; 4]; }
        }
        if dom.attr(node, "nowrap").is_some() { s.white_space = WhiteSpace::NoWrap; }
        if let Some(v) = dom.attr(node, "valign") { let _ = v; }
    }
    if tag == "ol" {
        if let Some(t) = dom.attr(node, "type") {
            s.list_style = match t { "a" => ListStyle::LowerAlpha, "A" => ListStyle::UpperAlpha, "i" => ListStyle::LowerRoman, "I" => ListStyle::UpperRoman, _ => ListStyle::Decimal };
        }
    }
}

fn table_attr<'d>(dom: &'d Dom, mut node: NodeId, name: &str) -> Option<&'d str> {
    while let Some(p) = dom.nodes[node].parent {
        if dom.tag(p) == Some("table") { return dom.attr(p, name); }
        node = p;
    }
    None
}

fn is_mono(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    let first = f.split(',').next().unwrap_or("");
    first.contains("mono") || first.contains("courier") || first.contains("consol") || first.contains("menlo") || first.contains("code")
        || (f.contains("monospace") && !f.contains("sans") && !f.contains("serif") && !f.contains("system"))
}

fn parse_len(v: &str, s: &Style, ctx: &Ctx) -> Option<Len> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("auto") || v.eq_ignore_ascii_case("none") || v == "initial" || v == "fit-content" || v == "max-content" || v == "min-content" { return Some(Len::Auto); }
    if let Some(inner) = v.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) { return parse_calc(inner, s, ctx); }
    if let Some(inner) = v.strip_prefix("min(").or(v.strip_prefix("max(")).or(v.strip_prefix("clamp(")).and_then(|r| r.strip_suffix(')')) {
        // Pick the first plain length: good enough for most responsive sizing.
        for part in inner.split(',') { if let Some(l) = parse_len(part, s, ctx) { if l != Len::Auto { return Some(l); } } }
        return None;
    }
    if let Some(inner) = v.strip_prefix("var(") { let _ = inner; return None; }
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(v.len());
    let n = css::parse_number(&v[..end])?;
    let unit = v[end..].to_ascii_lowercase();
    let px = |hundredths: i64| Some(Len::Px((hundredths / 100) as i32));
    match unit.as_str() {
        "px" => px(n as i64),
        "" => if n == 0 { Some(Len::Px(0)) } else { px(n as i64) },
        "em" => px(n as i64 * s.font_size as i64),
        "rem" => px(n as i64 * ctx.root_font as i64),
        "ex" | "ch" => px(n as i64 * s.font_size as i64 / 2),
        "pt" => px(n as i64 * 4 / 3),
        "pc" => px(n as i64 * 16),
        "in" => px(n as i64 * 96),
        "cm" => px(n as i64 * 96 * 100 / 254),
        "mm" => px(n as i64 * 96 * 10 / 254),
        "vw" => px(n as i64 * ctx.viewport.0 as i64 / 100),
        "vh" => px(n as i64 * ctx.viewport.1 as i64 / 100),
        "vmin" => px(n as i64 * ctx.viewport.0.min(ctx.viewport.1) as i64 / 100),
        "vmax" => px(n as i64 * ctx.viewport.0.max(ctx.viewport.1) as i64 / 100),
        "%" => Some(Len::Pct(n)),
        _ => None,
    }
}

/// `calc()` with one `+`/`-` between a percentage and a length is approximated by
/// the percentage alone when present, else the sum of pixel terms.
fn parse_calc(inner: &str, s: &Style, ctx: &Ctx) -> Option<Len> {
    let mut px = 0;
    let mut pct = None;
    let mut sign = 1;
    for tok in inner.split_ascii_whitespace() {
        match tok {
            "+" => sign = 1,
            "-" => sign = -1,
            t => match parse_len(t, s, ctx)? {
                Len::Px(p) => px += sign * p,
                Len::Pct(p) => pct = Some(sign * p),
                Len::Auto => return None,
            },
        }
    }
    Some(match pct { Some(p) if px == 0 => Len::Pct(p), Some(p) => Len::Pct(p), None => Len::Px(px) })
}

fn font_size(v: &str, s: &Style, ctx: &Ctx) -> Option<i32> {
    let parent = ctx.parent.font_size;
    Some(match v.trim().to_ascii_lowercase().as_str() {
        "xx-small" => 9, "x-small" => 10, "small" => 13, "medium" => 16, "large" => 18, "x-large" => 24, "xx-large" => 32, "xxx-large" => 48,
        "smaller" => parent * 5 / 6, "larger" => parent * 6 / 5,
        other => {
            let mut probe = s.clone();
            probe.font_size = parent;
            match parse_len(other, &probe, ctx)? {
                Len::Px(p) => p,
                Len::Pct(p) => parent * p / 10000,
                Len::Auto => return None,
            }
        }
    }.clamp(1, 400))
}

fn sides<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    Some(match values.len() {
        1 => [values[0]; 4],
        2 => [values[0], values[1], values[0], values[1]],
        3 => [values[0], values[1], values[2], values[1]],
        4 => [values[0], values[1], values[2], values[3]],
        _ => return None,
    })
}

/// Splits a value on top-level whitespace (keeping `rgb(1, 2, 3)` whole).
/// `grid-column-start`/`-end` value: a line number or `span N`.
fn grid_line(v: &str, g: &mut GridPlace, start: bool) {
    let v = v.trim();
    if let Some(n) = v.strip_prefix("span").and_then(|n| n.trim().parse::<i32>().ok()) { g.span = n.clamp(1, 1000); return; }
    if let Ok(n) = v.parse::<i32>() { if n != 0 { if start { g.start = n } else { g.end = n } } }
}

/// `grid-template-columns`: explicit tracks, and an auto-repeated one.
fn grid_tracks(v: &str, s: &Style, ctx: &Ctx) -> (Vec<Track>, Option<(Track, bool)>) {
    fn one(w: &str, s: &Style, ctx: &Ctx) -> Option<Track> {
        let w = w.trim();
        if let Some(f) = w.strip_suffix("fr") { return css::parse_number(f).map(|f| Track::Fr(f.max(0))); }
        if let Some(inner) = w.strip_prefix("minmax(").and_then(|r| r.strip_suffix(')')) {
            let (a, b) = inner.split_once(',')?;
            let min = parse_len(a, s, ctx).unwrap_or(Len::Auto);
            let b = b.trim();
            return Some(match b.strip_suffix("fr").and_then(css::parse_number) {
                Some(f) => Track::MinMax(min, f.max(1)),
                None => match parse_len(b, s, ctx) { Some(Len::Auto) | None => Track::MinMax(min, 0), Some(l) => Track::Len(l) },
            });
        }
        if w.starts_with("fit-content(") { return Some(Track::Auto); }
        match parse_len(w, s, ctx)? { Len::Auto => Some(Track::Auto), l => Some(Track::Len(l)) }
    }
    let mut tracks = Vec::new();
    let mut auto = None;
    for w in words(v) {
        if w.starts_with('[') { continue; }
        if let Some(inner) = w.strip_prefix("repeat(").and_then(|r| r.strip_suffix(')')) {
            let Some((count, rest)) = inner.split_once(',') else { continue };
            let inner: Vec<Track> = words(rest).into_iter().filter(|w| !w.starts_with('[')).filter_map(|w| one(w, s, ctx)).collect();
            match count.trim() {
                "auto-fill" | "auto-fit" => if let Some(t) = inner.first() { auto = Some((*t, count.trim() == "auto-fit")) },
                n => if let Ok(n) = n.parse::<usize>() { for _ in 0..n.min(1000) { tracks.extend_from_slice(&inner); } },
            }
            continue;
        }
        if let Some(t) = one(w, s, ctx) { tracks.push(t); }
    }
    (tracks, auto)
}

/// Replaces each `var(--name, fallback)` in `v`; None when one has no value and no
/// fallback (the declaration then does not apply).
fn substitute_vars(v: &str, vars: &BTreeMap<String, String>, depth: u32) -> Option<String> {
    if !v.contains("var(") { return Some(String::from(v)); }
    if depth > 16 { return None; }
    let mut out = String::new();
    let mut rest = v;
    while let Some(at) = rest.find("var(") {
        out.push_str(&rest[..at]);
        let body = &rest[at + 4..];
        let mut level = 1;
        let mut end = None;
        for (i, c) in body.char_indices() {
            match c { '(' => level += 1, ')' => { level -= 1; if level == 0 { end = Some(i); break; } } _ => {} }
        }
        let end = end?;
        let inner = &body[..end];
        let (name, fallback) = match inner.split_once(',') { Some((n, f)) => (n.trim(), Some(f.trim())), None => (inner.trim(), None) };
        let value = match vars.get(name.strip_prefix("--").unwrap_or(name).to_ascii_lowercase().as_str()) {
            Some(v) => Some(v.clone()),
            None => fallback.map(String::from),
        }?;
        out.push_str(&substitute_vars(&value, vars, depth + 1)?);
        rest = &body[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn words(v: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = None;
    for (i, c) in v.char_indices() {
        match c {
            '(' => { depth += 1; if start.is_none() { start = Some(i); } }
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => { if let Some(s) = start.take() { out.push(&v[s..i]); } }
            _ => { if start.is_none() { start = Some(i); } }
        }
    }
    if let Some(s) = start { out.push(&v[s..]); }
    out
}

fn border_width(w: &str, s: &Style, ctx: &Ctx) -> Option<i32> {
    match w { "thin" => Some(1), "medium" => Some(3), "thick" => Some(5), _ => match parse_len(w, s, ctx)? { Len::Px(p) => Some(p.max(0)), _ => None } }
}

/// `border`/`border-top` shorthand: width, style and colour in any order.
fn border_shorthand(v: &str, s: &Style, ctx: &Ctx) -> (i32, u32) {
    let mut width = 3;
    let mut color = s.color;
    let mut visible = true;
    for w in words(v) {
        let lw = w.to_ascii_lowercase();
        if matches!(lw.as_str(), "none" | "hidden") { visible = false; }
        else if matches!(lw.as_str(), "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset") {}
        else if let Some(b) = border_width(&lw, s, ctx) { width = b; }
        else if let Some(c) = parse_color_with(&lw, s.color) { color = c; }
    }
    if v.trim() == "0" { visible = false; }
    (if visible { width } else { 0 }, color)
}

fn apply(s: &mut Style, d: &Declaration, ctx: &Ctx, font_only: bool) {
    let v = d.value.as_str();
    let lv = v.to_ascii_lowercase();
    if lv == "inherit" {
        inherit_one(s, &d.name, ctx.parent);
        return;
    }
    if font_only {
        match d.name.as_str() {
            "font-size" => if let Some(px) = font_size(v, s, ctx) { s.font_size = px },
            "font" => font_shorthand(s, v, ctx),
            _ => {}
        }
        return;
    }
    let len = |s: &Style| parse_len(v, s, ctx);
    match d.name.as_str() {
        "display" => {
            let first = lv.split_ascii_whitespace().next().unwrap_or("");
            // flow-root contains its floats, as overflow other than visible does.
            if first == "flow-root" { s.overflow_clip = true; }
            s.display = match first {
                "none" => Display::None,
                "block" | "flow-root" | "table-caption" | "table-header-group" | "table-row-group" | "table-footer-group" | "-webkit-box" => Display::Block,
                "grid" => Display::Grid,
                "inline" | "ruby" => Display::Inline,
                "inline-block" | "inline-flex" | "inline-grid" | "inline-table" => Display::InlineBlock,
                "list-item" => Display::ListItem,
                "table" => Display::Table,
                "table-row" => Display::TableRow,
                "table-cell" => Display::TableCell,
                "flex" | "-webkit-flex" => Display::Flex,
                "contents" => Display::Contents,
                _ => s.display,
            };
            if lv.contains("list-item") { s.display = Display::ListItem; }
        }
        "color" => if let Some(c) = parse_color_with(&lv, ctx.parent.color) { s.color = c },
        "background-color" => if let Some(c) = parse_color_with(&lv, s.color) { s.background = c },
        "background" => {
            if lv == "none" || lv == "transparent" { s.background = 0; }
            for w in words(&lv) {
                if let Some(c) = parse_color_with(w, s.color) { s.background = c; }
                else if let Some(g) = w.strip_prefix("linear-gradient(").or(w.strip_prefix("radial-gradient(")) {
                    // A gradient becomes its first colour stop.
                    if let Some(c) = g.split(',').filter_map(|p| p.split_ascii_whitespace().next().and_then(|c| parse_color_with(c, s.color))).next() { s.background = c; }
                }
            }
        }
        "font-weight" => s.bold = match lv.as_str() { "bold" | "bolder" => true, "normal" | "lighter" => false, n => n.parse::<i32>().map_or(s.bold, |w| w >= 600) },
        "font-style" => s.italic = lv.starts_with("italic") || lv.starts_with("oblique"),
        "font-family" => s.mono = is_mono(&lv),
        "line-height" => {
            s.line_height = if lv == "normal" { 0 } else if let Some(n) = css::parse_number(&lv) {
                s.font_size * n / 100
            } else {
                match len(s) { Some(Len::Px(p)) => p, Some(Len::Pct(p)) => s.font_size * p / 10000, _ => s.line_height }
            }
        }
        "text-align" => s.align = match lv.as_str() { "center" | "-webkit-center" => Align::Center, "right" | "end" | "-webkit-right" => Align::Right, "justify" => Align::Justify, "left" | "start" => Align::Left, _ => s.align },
        "white-space" | "white-space-collapse" | "text-wrap" | "text-wrap-mode" => s.white_space = match lv.as_str() {
            "pre" => WhiteSpace::Pre, "nowrap" => WhiteSpace::NoWrap, "pre-wrap" | "break-spaces" | "preserve" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine, "normal" | "wrap" | "collapse" => WhiteSpace::Normal, _ => s.white_space,
        },
        "text-decoration" | "text-decoration-line" => {
            if lv.contains("none") { s.underline = false; s.strike = false; }
            if lv.contains("underline") { s.underline = true; }
            if lv.contains("line-through") { s.strike = true; }
        }
        "text-transform" => s.transform = match lv.as_str() { "uppercase" => Transform::Upper, "lowercase" => Transform::Lower, "capitalize" => Transform::Capitalize, _ => Transform::None },
        "list-style-type" | "list-style" => {
            for w in lv.split_ascii_whitespace() {
                s.list_style = match w {
                    "none" => ListStyle::None, "disc" => ListStyle::Disc, "circle" => ListStyle::Circle, "square" => ListStyle::Square,
                    "decimal" | "decimal-leading-zero" => ListStyle::Decimal, "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
                    "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha, "lower-roman" => ListStyle::LowerRoman, "upper-roman" => ListStyle::UpperRoman,
                    _ => continue,
                };
            }
        }
        "visibility" => s.hidden = lv == "hidden" || lv == "collapse",
        "margin" | "padding" => {
            let vals: Option<Vec<Len>> = words(v).iter().map(|w| parse_len(w, s, ctx)).collect();
            if let Some(sides) = vals.as_deref().and_then(sides) {
                if d.name == "margin" { s.margin = sides } else { s.padding = sides.map(|l| if l == Len::Auto { Len::Px(0) } else { l }) }
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "margin-block-start" | "margin-block-end" | "margin-inline-start" | "margin-inline-end"
        | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "padding-block-start" | "padding-block-end" | "padding-inline-start" | "padding-inline-end" => {
            if let Some(l) = len(s) {
                let side = side_index(&d.name);
                if d.name.starts_with("margin") { s.margin[side] = l } else { s.padding[side] = if l == Len::Auto { Len::Px(0) } else { l } }
            }
        }
        "margin-block" | "margin-inline" | "padding-block" | "padding-inline" => {
            let vals: Option<Vec<Len>> = words(v).iter().map(|w| parse_len(w, s, ctx)).collect();
            if let Some(vals) = vals.filter(|v| !v.is_empty()) {
                let (a, b) = (vals[0], *vals.get(1).unwrap_or(&vals[0]));
                let (i, j) = if d.name.ends_with("block") { (TOP, BOTTOM) } else { (LEFT, RIGHT) };
                let target = if d.name.starts_with("margin") { &mut s.margin } else { &mut s.padding };
                target[i] = a; target[j] = b;
            }
        }
        "border" => { let (w, c) = border_shorthand(v, s, ctx); s.border = [w; 4]; s.border_color = [c; 4]; }
        "border-top" | "border-right" | "border-bottom" | "border-left" | "border-block-start" | "border-block-end" | "border-inline-start" | "border-inline-end" => {
            let (w, c) = border_shorthand(v, s, ctx);
            let i = side_index(&d.name);
            s.border[i] = w; s.border_color[i] = c;
        }
        "border-block" | "border-inline" => {
            let (w, c) = border_shorthand(v, s, ctx);
            let (i, j) = if d.name == "border-block" { (TOP, BOTTOM) } else { (LEFT, RIGHT) };
            s.border[i] = w; s.border[j] = w; s.border_color[i] = c; s.border_color[j] = c;
        }
        "border-width" => {
            let vals: Option<Vec<i32>> = words(&lv).iter().map(|w| border_width(w, s, ctx)).collect();
            if let Some(sd) = vals.as_deref().and_then(sides) { s.border = sd; }
        }
        "border-color" => {
            let vals: Option<Vec<u32>> = words(&lv).iter().map(|w| parse_color_with(w, s.color)).collect();
            if let Some(sd) = vals.as_deref().and_then(sides) { s.border_color = sd; }
        }
        "border-style" => if lv == "none" || lv == "hidden" { s.border = [0; 4] },
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            if let Some(w) = border_width(&lv, s, ctx) { s.border[side_index(&d.name)] = w; }
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            if let Some(c) = parse_color_with(&lv, s.color) { s.border_color[side_index(&d.name)] = c; }
        }
        "border-radius" => if let Some(Len::Px(p)) = words(v).first().and_then(|w| parse_len(w, s, ctx)) { s.radius = p.max(0) },
        "width" => if let Some(l) = len(s) { s.width = l },
        "height" => if let Some(l) = len(s) { s.height = l },
        "min-width" => if let Some(l) = len(s) { s.min_width = l },
        "max-width" => if let Some(l) = len(s) { s.max_width = l },
        "min-height" => if let Some(l) = len(s) { s.min_height = l },
        "position" => s.position = match lv.as_str() { "relative" => Position::Relative, "absolute" => Position::Absolute, "fixed" => Position::Fixed, "sticky" | "-webkit-sticky" => Position::Sticky, _ => Position::Static },
        "top" | "right" | "bottom" | "left" => if let Some(l) = len(s) { s.inset[side_index(&d.name)] = l },
        "inset" => {
            let ls: Vec<Len> = words(v).iter().filter_map(|w| if w.eq_ignore_ascii_case("auto") { Some(Len::Auto) } else { parse_len(w, s, ctx) }).collect();
            let get = |i: usize| ls.get(i).copied();
            if let Some(t) = get(0) {
                let r = get(1).unwrap_or(t);
                s.inset = [t, r, get(2).unwrap_or(t), get(3).unwrap_or(r)];
            }
        }
        "z-index" => s.z_index = lv.trim().parse().ok(),
        "clear" => s.clear = match lv.as_str() { "left" | "inline-start" => 1, "right" | "inline-end" => 2, "both" => 3, _ => 0 },
        "overflow" | "overflow-x" | "overflow-y" => s.overflow_clip = lv.split_ascii_whitespace().any(|w| matches!(w, "hidden" | "auto" | "scroll" | "clip")),
        "float" => s.float = match lv.as_str() { "left" => Float::Left, "right" => Float::Right, _ => Float::None },
        "flex-direction" => s.flex_column = lv.starts_with("column"),
        "flex-wrap" => s.flex_wrap = lv == "wrap" || lv == "wrap-reverse",
        "flex-flow" => { s.flex_column = lv.contains("column"); s.flex_wrap = lv.contains("wrap") && !lv.contains("nowrap"); }
        "flex" => {
            match lv.as_str() {
                "none" | "initial" => { s.flex_grow = 0; s.flex_basis = Len::Auto; }
                "auto" => { s.flex_grow = 100; s.flex_basis = Len::Auto; }
                _ => {
                    let ws = words(&lv);
                    s.flex_grow = ws.first().and_then(|w| css::parse_number(w)).unwrap_or(0);
                    // `flex: 1` means a zero basis; a length among the values is the basis.
                    s.flex_basis = Len::Px(0);
                    for w in ws.iter().skip(1) {
                        if css::parse_number(w).is_none() { if let Some(l) = parse_len(w, s, ctx) { s.flex_basis = l; } }
                    }
                }
            }
        }
        "flex-basis" => if let Some(l) = len(s) { s.flex_basis = l },
        "border-collapse" => s.border_collapse = lv == "collapse",
        "border-spacing" => if let Some(Len::Px(p)) = words(v).first().and_then(|w| parse_len(w, s, ctx)) { s.border_spacing = Some(p) },
        "flex-grow" => s.flex_grow = css::parse_number(&lv).unwrap_or(0),
        "justify-content" => s.justify = match lv.as_str() {
            "center" => Justify::Center, "flex-end" | "end" | "right" => Justify::End, "space-between" => Justify::SpaceBetween,
            "space-around" => Justify::SpaceAround, "space-evenly" => Justify::SpaceEvenly, _ => Justify::Start,
        },
        "align-items" => s.align_center = lv == "center",
        "gap" | "column-gap" | "grid-gap" | "grid-column-gap" => {
            let w = words(v);
            if let Some(Len::Px(p)) = w.last().and_then(|w| parse_len(w, s, ctx)) { s.gap = p }
            if matches!(d.name.as_str(), "gap" | "grid-gap") { if let Some(Len::Px(p)) = w.first().and_then(|w| parse_len(w, s, ctx)) { s.row_gap = p } }
        }
        "transform" | "translate" => {
            // Translations add up; other transforms are not drawn.
            let (mut x, mut y) = (Len::Px(0), Len::Px(0));
            let add = |a: Len, b: Option<Len>| match (a, b) { (Len::Px(p), Some(Len::Px(q))) => Len::Px(p + q), (a, Some(Len::Px(0))) => a, (Len::Px(0), Some(b)) => b, (Len::Pct(p), Some(Len::Pct(q))) => Len::Pct(p + q), (a, _) => a };
            if d.name == "translate" {
                let w = words(v);
                x = w.first().and_then(|w| parse_len(w, s, ctx)).unwrap_or(Len::Px(0));
                y = w.get(1).and_then(|w| parse_len(w, s, ctx)).unwrap_or(Len::Px(0));
            } else {
                for f in words(&lv) {
                    let Some((name, args)) = f.strip_suffix(')').and_then(|f| f.split_once('(')) else { continue };
                    let a: Vec<Option<Len>> = args.split(',').map(|a| parse_len(a.trim(), s, ctx)).collect();
                    match name {
                        "translate" | "translate3d" => { x = add(x, a.first().copied().flatten()); y = add(y, a.get(1).copied().flatten()); }
                        "translatex" => x = add(x, a.first().copied().flatten()),
                        "translatey" => y = add(y, a.first().copied().flatten()),
                        _ => {}
                    }
                }
            }
            s.translate = (x != Len::Px(0) || y != Len::Px(0)).then_some((x, y));
        }
        "box-shadow" => {
            s.shadow = None;
            if let Some(first) = css::split_top(&lv, ',').into_iter().find(|p| !p.contains("inset")) {
                let mut lens = Vec::new();
                let mut color = s.color;
                for w in words(first) {
                    match parse_len(w, s, ctx) { Some(Len::Px(p)) => lens.push(p), _ => if let Some(c) = parse_color_with(w, s.color) { color = c } }
                }
                if lens.len() >= 2 && color >> 24 != 0 {
                    s.shadow = Some((lens[0], lens[1], lens.get(2).copied().unwrap_or(0).clamp(0, 200), lens.get(3).copied().unwrap_or(0).clamp(-200, 200), color));
                }
            }
        }
        "opacity" => {
            let n = match lv.strip_suffix('%') { Some(p) => css::parse_number(p).map(|p| p / 100), None => css::parse_number(&lv) };
            if let Some(n) = n { s.opacity = (n.clamp(0, 100) * 255 / 100) as u8; }
        }
        "row-gap" | "grid-row-gap" => if let Some(Len::Px(p)) = parse_len(v, s, ctx) { s.row_gap = p },
        "grid-template-columns" => { let (t, a) = grid_tracks(v, s, ctx); s.grid_columns = t; s.grid_auto = a; }
        "grid-template" => if let Some((_, cols)) = v.split_once('/') { let (t, a) = grid_tracks(cols, s, ctx); s.grid_columns = t; s.grid_auto = a; },
        "grid-column" | "grid-area" => {
            // grid-area is row-start / column-start / row-end / column-end.
            let parts: Vec<&str> = v.split('/').map(str::trim).collect();
            let (a, b) = if d.name == "grid-area" { (parts.get(1).copied().unwrap_or(""), parts.get(3).copied().unwrap_or("")) } else { (parts[0], parts.get(1).copied().unwrap_or("")) };
            let mut g = GridPlace::default();
            grid_line(a, &mut g, true);
            grid_line(b, &mut g, false);
            s.grid_column = g;
        }
        "grid-column-start" => grid_line(&lv, &mut s.grid_column, true),
        "grid-column-end" => grid_line(&lv, &mut s.grid_column, false),
        "box-sizing" => s.box_sizing_border = lv == "border-box",
        "letter-spacing" => s.letter_spacing = match len(s) { Some(Len::Px(p)) => p, _ => 0 },
        "vertical-align" => s.vertical_super = match lv.as_str() { "super" | "text-top" => 1, "sub" | "text-bottom" => -1, _ => 0 },
        _ => {}
    }
}

fn side_index(name: &str) -> usize {
    if name.contains("top") || name.contains("block-start") { TOP }
    else if name.contains("right") || name.contains("inline-end") { RIGHT }
    else if name.contains("bottom") || name.contains("block-end") { BOTTOM }
    else { LEFT }
}

fn inherit_one(s: &mut Style, name: &str, p: &Style) {
    match name {
        "color" => s.color = p.color,
        "background-color" | "background" => s.background = p.background,
        "font-size" => s.font_size = p.font_size,
        "font-weight" => s.bold = p.bold,
        "font-family" => s.mono = p.mono,
        "display" => s.display = p.display,
        "width" => s.width = p.width,
        "text-align" => s.align = p.align,
        _ => {}
    }
}

/// `font: italic bold 14px/1.5 Geist, sans-serif`.
fn font_shorthand(s: &mut Style, v: &str, ctx: &Ctx) {
    let lv = v.to_ascii_lowercase();
    let ws = words(&lv);
    for (i, w) in ws.iter().enumerate() {
        if w.starts_with(|c: char| c.is_ascii_digit() || c == '.') || matches!(*w, "small" | "medium" | "large" | "x-large" | "xx-large" | "x-small" | "smaller" | "larger") {
            let (size, lh) = w.split_once('/').unwrap_or((w, ""));
            if let Some(px) = font_size(size, s, ctx) {
                s.font_size = px;
                if !lh.is_empty() {
                    s.line_height = css::parse_number(lh).map(|n| px * n / 100).filter(|_| lh.bytes().all(|c| c.is_ascii_digit() || c == b'.')).unwrap_or_else(|| {
                        match parse_len(lh, s, ctx) { Some(Len::Px(p)) => p, _ => 0 }
                    });
                }
                s.mono = is_mono(&ws[i + 1..].join(" "));
                for pre in &ws[..i] {
                    match *pre { "bold" | "bolder" | "600" | "700" | "800" | "900" => s.bold = true, "italic" | "oblique" => s.italic = true, _ => {} }
                }
                return;
            }
        }
    }
}

pub fn parse_color(v: &str) -> Option<u32> { parse_color_with(&v.trim().to_ascii_lowercase(), 0xFF00_0000) }

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`, `hsl()`, named colours,
/// `transparent` and `currentcolor`; `v` must be lower case.
pub fn parse_color_with(v: &str, current: u32) -> Option<u32> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let d: Option<Vec<u32>> = hex.chars().map(|c| c.to_digit(16)).collect();
        let d = d?;
        return Some(match d.len() {
            3 => 0xFF00_0000 | (d[0] * 17) << 16 | (d[1] * 17) << 8 | d[2] * 17,
            4 => (d[3] * 17) << 24 | (d[0] * 17) << 16 | (d[1] * 17) << 8 | d[2] * 17,
            6 => 0xFF00_0000 | (d[0] << 20 | d[1] << 16 | d[2] << 12 | d[3] << 8 | d[4] << 4 | d[5]),
            8 => (d[6] << 4 | d[7]) << 24 | (d[0] << 20 | d[1] << 16 | d[2] << 12 | d[3] << 8 | d[4] << 4 | d[5]),
            _ => return None,
        });
    }
    if let Some(args) = v.strip_prefix("rgba(").or(v.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 { return None; }
        let chan = |p: &str| -> Option<u32> {
            Some(if let Some(pct) = p.strip_suffix('%') { (css::parse_number(pct)? * 255 / 10000).clamp(0, 255) as u32 } else { (css::parse_number(p)? / 100).clamp(0, 255) as u32 })
        };
        let alpha = match parts.get(3) {
            Some(a) => if let Some(pct) = a.strip_suffix('%') { (css::parse_number(pct)? * 255 / 10000).clamp(0, 255) as u32 } else { (css::parse_number(a)? * 255 / 100).clamp(0, 255) as u32 },
            None => 255,
        };
        return Some(alpha << 24 | chan(parts[0])? << 16 | chan(parts[1])? << 8 | chan(parts[2])?);
    }
    if let Some(args) = v.strip_prefix("hsla(").or(v.strip_prefix("hsl(")).and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 { return None; }
        let h = css::parse_number(parts[0].trim_end_matches("deg"))? / 100;
        let s = css::parse_number(parts[1].trim_end_matches('%'))? / 100;
        let l = css::parse_number(parts[2].trim_end_matches('%'))? / 100;
        let alpha = parts.get(3).and_then(|a| if let Some(p) = a.strip_suffix('%') { css::parse_number(p).map(|n| n * 255 / 10000) } else { css::parse_number(a).map(|n| n * 255 / 100) }).unwrap_or(255).clamp(0, 255) as u32;
        let (r, g, b) = hsl(h.rem_euclid(360), s.clamp(0, 100), l.clamp(0, 100));
        return Some(alpha << 24 | r << 16 | g << 8 | b);
    }
    Some(match v {
        "transparent" => 0,
        "currentcolor" => current,
        _ => 0xFF00_0000 | named_color(v)?,
    })
}

fn hsl(h: i32, s: i32, l: i32) -> (u32, u32, u32) {
    // Integer HSL to RGB, all in thousandths.
    let c = (1000 - (2 * l * 10 - 1000).abs()) * s / 100;
    let hp = h * 1000 / 60;
    let x = c * (1000 - ((hp % 2000) - 1000).abs()) / 1000;
    let (r, g, b) = match h / 60 { 0 => (c, x, 0), 1 => (x, c, 0), 2 => (0, c, x), 3 => (0, x, c), 4 => (x, 0, c), _ => (c, 0, x) };
    let m = l * 10 - c / 2;
    let to = |v: i32| (((v + m) * 255 + 500) / 1000).clamp(0, 255) as u32;
    (to(r), to(g), to(b))
}

fn named_color(v: &str) -> Option<u32> {
    Some(match v {
        "black" => 0x000000, "white" => 0xFFFFFF, "red" => 0xFF0000, "green" => 0x008000, "blue" => 0x0000FF,
        "yellow" => 0xFFFF00, "orange" => 0xFFA500, "purple" => 0x800080, "gray" | "grey" => 0x808080,
        "silver" => 0xC0C0C0, "maroon" => 0x800000, "olive" => 0x808000, "lime" => 0x00FF00, "aqua" | "cyan" => 0x00FFFF,
        "teal" => 0x008080, "navy" => 0x000080, "fuchsia" | "magenta" => 0xFF00FF, "pink" => 0xFFC0CB,
        "brown" => 0xA52A2A, "gold" => 0xFFD700, "indigo" => 0x4B0082, "violet" => 0xEE82EE, "coral" => 0xFF7F50,
        "salmon" => 0xFA8072, "tomato" => 0xFF6347, "crimson" => 0xDC143C, "khaki" => 0xF0E68C, "beige" => 0xF5F5DC,
        "ivory" => 0xFFFFF0, "lavender" => 0xE6E6FA, "plum" => 0xDDA0DD, "orchid" => 0xDA70D6, "tan" => 0xD2B48C,
        "chocolate" => 0xD2691E, "sienna" => 0xA0522D, "peru" => 0xCD853F, "wheat" => 0xF5DEB3, "linen" => 0xFAF0E6,
        "snow" => 0xFFFAFA, "honeydew" => 0xF0FFF0, "azure" => 0xF0FFFF, "aliceblue" => 0xF0F8FF, "ghostwhite" => 0xF8F8FF,
        "whitesmoke" => 0xF5F5F5, "gainsboro" => 0xDCDCDC, "lightgray" | "lightgrey" => 0xD3D3D3, "darkgray" | "darkgrey" => 0xA9A9A9,
        "dimgray" | "dimgrey" => 0x696969, "lightslategray" | "lightslategrey" => 0x778899, "slategray" | "slategrey" => 0x708090,
        "darkslategray" | "darkslategrey" => 0x2F4F4F, "darkred" => 0x8B0000, "darkgreen" => 0x006400, "darkblue" => 0x00008B,
        "darkorange" => 0xFF8C00, "darkviolet" => 0x9400D3, "darkcyan" => 0x008B8B, "darkmagenta" => 0x8B008B,
        "lightblue" => 0xADD8E6, "lightgreen" => 0x90EE90, "lightyellow" => 0xFFFFE0, "lightpink" => 0xFFB6C1,
        "lightcyan" => 0xE0FFFF, "skyblue" => 0x87CEEB, "steelblue" => 0x4682B4, "royalblue" => 0x4169E1,
        "dodgerblue" => 0x1E90FF, "deepskyblue" => 0x00BFFF, "cornflowerblue" => 0x6495ED, "midnightblue" => 0x191970,
        "slateblue" => 0x6A5ACD, "mediumblue" => 0x0000CD, "cadetblue" => 0x5F9EA0, "powderblue" => 0xB0E0E6,
        "forestgreen" => 0x228B22, "seagreen" => 0x2E8B57, "limegreen" => 0x32CD32, "springgreen" => 0x00FF7F,
        "olivedrab" => 0x6B8E23, "yellowgreen" => 0x9ACD32, "lawngreen" => 0x7CFC00, "mediumseagreen" => 0x3CB371,
        "turquoise" => 0x40E0D0, "aquamarine" => 0x7FFFD4, "firebrick" => 0xB22222, "indianred" => 0xCD5C5C,
        "hotpink" => 0xFF69B4, "deeppink" => 0xFF1493, "orangered" => 0xFF4500, "goldenrod" => 0xDAA520,
        "darkgoldenrod" => 0xB8860B, "saddlebrown" => 0x8B4513, "rosybrown" => 0xBC8F8F, "mintcream" => 0xF5FFFA,
        "seashell" => 0xFFF5EE, "oldlace" => 0xFDF5E6, "floralwhite" => 0xFFFAF0, "cornsilk" => 0xFFF8DC,
        "lemonchiffon" => 0xFFFACD, "papayawhip" => 0xFFEFD5, "blanchedalmond" => 0xFFEBCD, "bisque" => 0xFFE4C4,
        "moccasin" => 0xFFE4B5, "navajowhite" => 0xFFDEAD, "peachpuff" => 0xFFDAB9, "mistyrose" => 0xFFE4E1,
        "lavenderblush" => 0xFFF0F5, "thistle" => 0xD8BFD8, "rebeccapurple" => 0x663399, "mediumpurple" => 0x9370DB,
        "blueviolet" => 0x8A2BE2, "darkorchid" => 0x9932CC, "darkslateblue" => 0x483D8B, "darkolivegreen" => 0x556B2F,
        "darkseagreen" => 0x8FBC8F, "darkkhaki" => 0xBDB76B, "darksalmon" => 0xE9967A, "lightsalmon" => 0xFFA07A,
        "lightcoral" => 0xF08080, "palegreen" => 0x98FB98, "paleturquoise" => 0xAFEEEE, "palevioletred" => 0xDB7093,
        "palegoldenrod" => 0xEEE8AA, "lightsteelblue" => 0xB0C4DE, "lightseagreen" => 0x20B2AA, "darkturquoise" => 0x00CED1,
        "mediumturquoise" => 0x48D1CC, "mediumaquamarine" => 0x66CDAA, "mediumspringgreen" => 0x00FA9A, "chartreuse" => 0x7FFF00,
        "greenyellow" => 0xADFF2F, "burlywood" => 0xDEB887, "sandybrown" => 0xF4A460, "lightgoldenrodyellow" => 0xFAFAD2,
        "mediumorchid" => 0xBA55D3, "mediumvioletred" => 0xC71585, "mediumslateblue" => 0x7B68EE,
        _ => return None,
    })
}

/// The `<style>` sheets of a document, in document order.
pub fn page_css(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    for id in 0..dom.nodes.len() {
        if dom.tag(id) == Some("style") {
            let media = dom.attr(id, "media").unwrap_or("");
            let _ = media;
            out.push(dom.text_content(id));
        }
    }
    out
}
