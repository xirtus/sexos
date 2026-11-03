// SPDX-License-Identifier: GPL-3.0-only
//! Bounded HTML-to-readable-text conversion for the Browser reader.
//!
//! This is a reading view, not a web engine: there is no DOM, CSS, layout,
//! network fetch or form. Markup becomes plain UTF-8 text:
//! headings and block elements start new lines, list items get a bullet,
//! links keep their text plus a numbered reference listed at the end, and
//! images show their alt text, inline scripts run only through the bounded
//! subset in `script.rs` (their writes become text), and `style`, `head`,
//! `template` and comments contribute nothing. The
//! output never exceeds the caller's buffer; a cut is marked, never silent.

pub const MAX_LINKS: usize = 16;
const TRUNCATED: &[u8] = b"\n[page truncated]";

pub struct Output<'a> {
    buf: &'a mut [u8],
    len: usize,
    truncated: bool,
    /// The last emitted byte was whitespace or nothing has been emitted.
    space: bool,
    newlines: u8,
    /// Inside `<pre>`: whitespace is kept as written (tabs become spaces).
    pre: bool,
    /// `<blockquote>` nesting; each line inside starts with `> ` per level.
    quote: u8,
    /// Only a list marker is on the current line (a block opening here stays on it).
    marker: bool,
}

impl<'a> Output<'a> {
    fn new(buf: &'a mut [u8]) -> Self { Self { buf, len: 0, truncated: false, space: true, newlines: 2, pre: false, quote: 0, marker: false } }
    fn room(&self) -> usize { self.buf.len().saturating_sub(TRUNCATED.len()) }
    fn byte(&mut self, b: u8) {
        if self.len < self.room() { self.buf[self.len] = b; self.len += 1; } else { self.truncated = true; }
    }
    fn text(&mut self, b: u8) {
        if self.pre {
            match b {
                b'\r' => {}
                b'\n' => { self.byte(b'\n'); self.newlines = self.newlines.saturating_add(1); }
                b'\t' | 0x0c => { self.byte(b' '); self.newlines = 0; }
                _ => { self.byte(b); self.newlines = 0; }
            }
            self.space = false;
            return;
        }
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c) {
            self.space = true;
            return;
        }
        if self.space && self.newlines == 0 { self.byte(b' '); }
        if self.newlines != 0 { for _ in 0..self.quote { self.byte(b'>'); self.byte(b' '); } }
        self.space = false;
        self.newlines = 0;
        self.marker = false;
        self.byte(b);
    }
    fn bytes(&mut self, s: &[u8]) { for &b in s { self.text(b); } }
    /// Literal bytes at the start of a line (indentation, list markers).
    fn raw(&mut self, s: &[u8]) {
        if self.newlines != 0 { for _ in 0..self.quote { self.byte(b'>'); self.byte(b' '); } }
        for &b in s { self.byte(b); }
        self.newlines = 0;
        self.space = false;
    }
    /// End the current line; `blank` asks for an empty line before the next block.
    fn line(&mut self, blank: bool) {
        let want = if blank { 2 } else { 1 };
        while self.newlines < want { self.byte(b'\n'); self.newlines += 1; }
        self.space = true;
    }
    /// Pad the ` | `-separated cells written since `start` into columns.
    /// Skipped if the padding would not fit; widths count characters.
    fn align_columns(&mut self, start: usize) {
        if self.truncated || start >= self.len { return; }
        let text = &self.buf[start..self.len];
        let width = |cell: &[u8]| cell.iter().filter(|b| **b & 0xc0 != 0x80).count();
        let mut widths = [0usize; 16];
        for line in text.split(|b| *b == b'\n') {
            let cells = split_cells(line);
            for (k, cell) in cells.iter().enumerate().take(16) { widths[k] = widths[k].max(width(cell)); }
        }
        let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        for (n, line) in text.split(|b| *b == b'\n').enumerate() {
            if n > 0 { out.push(b'\n'); }
            let cells = split_cells(line);
            let last = cells.len().saturating_sub(1);
            for (k, cell) in cells.iter().enumerate() {
                if k > 0 { out.extend_from_slice(b" | "); }
                out.extend_from_slice(cell);
                if k < last && k < 16 { for _ in width(cell)..widths[k] { out.push(b' '); } }
            }
        }
        if start + out.len() > self.room() { return; }
        self.buf[start..start + out.len()].copy_from_slice(&out);
        self.len = start + out.len();
    }
    fn finish(mut self) -> (usize, bool) {
        while self.len > 0 && self.buf[self.len - 1] == b'\n' { self.len -= 1; }
        if self.truncated {
            for &b in TRUNCATED { self.buf[self.len] = b; self.len += 1; }
        }
        (self.len, self.truncated)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Converted {
    pub len: usize, pub links: usize, pub truncated: bool,
    /// Output ranges of each listed link target; empty when the list was cut.
    pub targets: [(usize, usize); MAX_LINKS],
    /// Inline scripts run by the bounded subset, and scripts refused
    /// (external `src`, other types, or outside the subset).
    pub scripts_run: u8,
    pub scripts_refused: u8,
}

fn lower(b: u8) -> u8 { b.to_ascii_lowercase() }
fn name_is(name: &[u8], want: &[u8]) -> bool { name.len() == want.len() && name.iter().zip(want).all(|(a, b)| lower(*a) == *b) }

/// Value of attribute `want` inside a tag's attribute bytes, unquoted.
fn attribute<'b>(attrs: &'b [u8], want: &[u8]) -> Option<&'b [u8]> {
    let mut i = 0;
    while i < attrs.len() {
        while i < attrs.len() && attrs[i].is_ascii_whitespace() { i += 1; }
        let start = i;
        while i < attrs.len() && !attrs[i].is_ascii_whitespace() && attrs[i] != b'=' { i += 1; }
        let name = &attrs[start..i];
        while i < attrs.len() && attrs[i].is_ascii_whitespace() { i += 1; }
        if i < attrs.len() && attrs[i] == b'=' {
            i += 1;
            while i < attrs.len() && attrs[i].is_ascii_whitespace() { i += 1; }
            let (value, next) = match attrs.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let end = attrs[i + 1..].iter().position(|&b| b == q).map_or(attrs.len(), |p| i + 1 + p);
                    (&attrs[(i + 1).min(attrs.len())..end], end + 1)
                }
                _ => {
                    let end = attrs[i..].iter().position(|b| b.is_ascii_whitespace()).map_or(attrs.len(), |p| i + p);
                    (&attrs[i..end], end)
                }
            };
            if name_is(name, want) { return Some(value); }
            i = next;
        } else if name.is_empty() {
            i += 1;
        }
    }
    None
}

/// Decode one character reference starting after `&`; returns the bytes to
/// emit and how many input bytes (after `&`) it consumed.
fn entity(input: &[u8], out: &mut [u8; 4]) -> Option<(usize, usize)> {
    let end = input.iter().take(12).position(|&b| b == b';')?;
    let name = &input[..end];
    let code: u32 = if let Some(num) = name.strip_prefix(b"#") {
        let (digits, radix) = match num.first() { Some(b'x' | b'X') => (&num[1..], 16), _ => (num, 10) };
        if digits.is_empty() { return None; }
        let mut v: u32 = 0;
        for &d in digits { v = v.checked_mul(radix)?.checked_add((d as char).to_digit(radix)?)?; }
        v
    } else {
        match name {
            b"amp" => b'&' as u32, b"lt" => b'<' as u32, b"gt" => b'>' as u32, b"quot" => b'"' as u32,
            b"apos" => b'\'' as u32, b"nbsp" => b' ' as u32, b"copy" => 0xA9, b"mdash" => 0x2014,
            b"ndash" => 0x2013, b"hellip" => 0x2026, b"lsquo" => 0x2018, b"rsquo" => 0x2019,
            b"ldquo" => 0x201c, b"rdquo" => 0x201d, b"bull" => 0x2022, b"middot" => 0xb7, b"times" => 0xd7,
            b"divide" => 0xf7, b"deg" => 0xb0, b"euro" => 0x20ac, b"pound" => 0xa3, b"yen" => 0xa5,
            b"cent" => 0xa2, b"sect" => 0xa7, b"para" => 0xb6, b"reg" => 0xae, b"trade" => 0x2122,
            b"larr" => 0x2190, b"rarr" => 0x2192, b"uarr" => 0x2191, b"darr" => 0x2193, b"plusmn" => 0xb1,
            b"frac12" => 0xbd, b"laquo" => 0xab, b"raquo" => 0xbb, b"iexcl" => 0xa1, b"iquest" => 0xbf,
            b"shy" | b"zwnj" | b"zwj" | b"lrm" | b"rlm" => return Some((0, end + 1)),
            _ => named(name)?,
        }
    };
    // Controls and surrogates are not readable text.
    let ch = char::from_u32(code).filter(|c| !c.is_control() || matches!(*c, '\n' | '\t'))?;
    Some((ch.encode_utf8(out).len(), end + 1))
}

/// Latin-1 letters U+00C0..U+00FF by entity name (the two operators too).
const LATIN1: [&[u8]; 64] = [b"Agrave", b"Aacute", b"Acirc", b"Atilde", b"Auml", b"Aring", b"AElig", b"Ccedil", b"Egrave", b"Eacute", b"Ecirc", b"Euml", b"Igrave", b"Iacute", b"Icirc", b"Iuml", b"ETH", b"Ntilde", b"Ograve", b"Oacute", b"Ocirc", b"Otilde", b"Ouml", b"times", b"Oslash", b"Ugrave", b"Uacute", b"Ucirc", b"Uuml", b"Yacute", b"THORN", b"szlig", b"agrave", b"aacute", b"acirc", b"atilde", b"auml", b"aring", b"aelig", b"ccedil", b"egrave", b"eacute", b"ecirc", b"euml", b"igrave", b"iacute", b"icirc", b"iuml", b"eth", b"ntilde", b"ograve", b"oacute", b"ocirc", b"otilde", b"ouml", b"divide", b"oslash", b"ugrave", b"uacute", b"ucirc", b"uuml", b"yacute", b"thorn", b"yuml"];
/// Greek lower case U+03B1..U+03C9 (`sigmaf` is U+03C2); capitalised names are
/// U+0391.. (there is no capital final sigma).
const GREEK: [&[u8]; 25] = [b"alpha", b"beta", b"gamma", b"delta", b"epsilon", b"zeta", b"eta", b"theta", b"iota", b"kappa", b"lambda", b"mu", b"nu", b"xi", b"omicron", b"pi", b"rho", b"sigmaf", b"sigma", b"tau", b"upsilon", b"phi", b"chi", b"psi", b"omega"];
/// Other common named references.
const NAMED: [(&[u8], u32); 89] = [(b"le", 0x2264), (b"ge", 0x2265), (b"ne", 0x2260), (b"micro", 0xb5), (b"check", 0x2713), (b"infin", 0x221e), (b"minus", 0x2212), (b"asymp", 0x2248), (b"equiv", 0x2261), (b"sum", 0x2211), (b"prod", 0x220f), (b"radic", 0x221a), (b"part", 0x2202), (b"nabla", 0x2207), (b"isin", 0x2208), (b"notin", 0x2209), (b"cap", 0x2229), (b"cup", 0x222a), (b"forall", 0x2200), (b"exist", 0x2203), (b"empty", 0x2205), (b"harr", 0x2194), (b"rArr", 0x21d2), (b"lArr", 0x21d0), (b"hArr", 0x21d4), (b"uArr", 0x21d1), (b"dArr", 0x21d3), (b"crarr", 0x21b5), (b"sup1", 0xb9), (b"sup2", 0xb2), (b"sup3", 0xb3), (b"frac14", 0xbc), (b"frac34", 0xbe), (b"ensp", 0x2002), (b"emsp", 0x2003), (b"thinsp", 0x2009), (b"dagger", 0x2020), (b"Dagger", 0x2021), (b"permil", 0x2030), (b"prime", 0x2032), (b"Prime", 0x2033), (b"lsaquo", 0x2039), (b"rsaquo", 0x203a), (b"sbquo", 0x201a), (b"bdquo", 0x201e), (b"loz", 0x25ca), (b"spades", 0x2660), (b"clubs", 0x2663), (b"hearts", 0x2665), (b"diams", 0x2666), (b"curren", 0xa4), (b"brvbar", 0xa6), (b"uml", 0xa8), (b"ordf", 0xaa), (b"ordm", 0xba), (b"macr", 0xaf), (b"acute", 0xb4), (b"cedil", 0xb8), (b"oline", 0x203e), (b"frasl", 0x2044), (b"lowast", 0x2217), (b"sim", 0x223c), (b"cong", 0x2245), (b"sub", 0x2282), (b"sup", 0x2283), (b"perp", 0x22a5), (b"sdot", 0x22c5), (b"lceil", 0x2308), (b"rceil", 0x2309), (b"lfloor", 0x230a), (b"rfloor", 0x230b), (b"lang", 0x27e8), (b"rang", 0x27e9), (b"OElig", 0x152), (b"oelig", 0x153), (b"Scaron", 0x160), (b"scaron", 0x161), (b"Yuml", 0x178), (b"fnof", 0x192), (b"circ", 0x2c6), (b"tilde", 0x2dc), (b"star", 0x2606), (b"starf", 0x2605), (b"cross", 0x2717), (b"hyphen", 0x2010), (b"ang", 0x2220), (b"there4", 0x2234), (b"prop", 0x221d), (b"int", 0x222b)];
fn named(name: &[u8]) -> Option<u32> {
    if let Some(i) = LATIN1.iter().position(|n| *n == name) { return Some(0xC0 + i as u32); }
    if let Some(i) = GREEK.iter().position(|n| *n == name) { return Some(0x3B1 + i as u32); }
    if let Some(i) = GREEK.iter().position(|n| n.len() == name.len() && n[0].to_ascii_uppercase() == name[0] && n[1..] == name[1..]) { return (i != 17).then_some(0x391 + i as u32); }
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

const INLINE: [(&[u8], u8); 7] = [(b"code", b'`'), (b"kbd", b'`'), (b"strong", b'*'), (b"b", b'*'), (b"em", b'_'), (b"i", b'_'), (b"cite", b'_')];
/// Two-character markers: highlighted, deleted and inserted text.
const INLINE2: [(&[u8], &[u8]); 6] = [(b"mark", b"=="), (b"del", b"~~"), (b"s", b"~~"), (b"strike", b"~~"), (b"ins", b"++"), (b"u", b"__")];
const MEDIA: [&[u8]; 3] = [b"video", b"audio", b"iframe"];
const SKIP: [&[u8]; 5] = [b"style", b"head", b"template", b"noscript", b"svg"];
const BLOCK: [&[u8]; 19] = [b"p", b"div", b"section", b"article", b"header", b"footer", b"main", b"nav", b"ul", b"ol", b"table", b"blockquote", b"pre", b"hr", b"dl", b"form", b"details", b"fieldset", b"figure"];

/// Convert `html` into readable text in `out`. Link targets are appended as a
/// numbered list; at most MAX_LINKS are kept.
/// Windows-1252 (a superset of ISO-8859-1's printable range) to Unicode.
fn cp1252(b: u8) -> char {
    const HIGH: [u16; 32] = [0x20ac, 0xfffd, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0xfffd, 0x017d, 0xfffd,
        0xfffd, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0xfffd, 0x017e, 0x0178];
    let code = if (0x80..0xa0).contains(&b) { HIGH[(b - 0x80) as usize] as u32 } else { b as u32 };
    char::from_u32(code).unwrap_or('\u{fffd}')
}

/// Whether the document declares a single-byte Latin charset in a `<meta>`
/// near its start (pages without one are read as UTF-8).
fn declares_latin(html: &[u8]) -> bool {
    let head = &html[..html.len().min(1024)];
    let Some(at) = head.windows(8).position(|w| w.eq_ignore_ascii_case(b"charset=")) else { return false };
    let rest = &head[at + 8..];
    let rest = rest.strip_prefix(b"\"").or(rest.strip_prefix(b"'")).unwrap_or(rest);
    [&b"windows-1252"[..], b"iso-8859-1", b"latin1", b"iso8859-1", b"cp1252"].iter().any(|c| rest.len() >= c.len() && rest[..c.len()].eq_ignore_ascii_case(c))
}

pub fn convert(html: &[u8], out: &mut [u8]) -> Converted {
    let latin = declares_latin(html);
    let mut o = Output::new(out);
    let mut links: [(usize, usize); MAX_LINKS] = [(0, 0); MAX_LINKS];
    let mut link_count = 0usize;
    let mut open_link: Option<usize> = None;
    let mut skip_until: Option<&[u8]> = None;
    let (mut scripts_run, mut scripts_refused) = (0u8, 0u8);
    // Open lists: 0 = bulleted, n = next number of an ordered list.
    let mut lists = [0u16; 4];
    // Marker style per open ordered list: b'1', b'a', b'A', b'i' or b'I'.
    let mut styles = [b'1'; 4];
    // Title of the open <abbr>, shown after its text.
    let mut abbr: Option<(usize, usize)> = None;
    // Output offset where the open (outermost) table's rows begin.
    let mut table: Option<usize> = None;
    let mut depth = 0usize;
    if let Some(title) = title(html) {
        o.bytes(b"Title:");
        o.space = true;
        let mut buf = [0u8; 4];
        let mut k = 0;
        while k < title.len() {
            if title[k] == b'&' { if let Some((n, used)) = entity(&title[k + 1..], &mut buf) { for &c in &buf[..n] { o.text(c); } k += 1 + used; continue; } }
            o.text(title[k]); k += 1;
        }
        o.line(true);
    }
    let mut i = 0;
    while i < html.len() {
        let b = html[i];
        if b == b'<' {
            if html[i..].starts_with(b"<!--") {
                i = html[i + 4..].windows(3).position(|w| w == b"-->").map_or(html.len(), |p| i + 4 + p + 3);
                continue;
            }
            let Some(close) = html[i..].iter().position(|&c| c == b'>') else { break };
            let tag = &html[i + 1..i + close];
            i += close + 1;
            let (end_tag, body) = match tag.first() { Some(b'/') => (true, &tag[1..]), _ => (false, tag) };
            let name_end = body.iter().position(|c| c.is_ascii_whitespace() || *c == b'/').unwrap_or(body.len());
            let name = &body[..name_end];
            if let Some(skip) = skip_until {
                if end_tag && name_is(name, skip) { skip_until = None; }
                continue;
            }
            if !end_tag && name_is(name, b"script") {
                // Find the raw end tag; script text is never markup.
                let rest = &html[i..];
                let close = rest.windows(9).position(|w| name_is(w, b"</script>"));
                let source = &rest[..close.unwrap_or(rest.len())];
                i += close.map_or(rest.len(), |c| c + 9);
                let attrs = &body[name_end..];
                let plain = attribute(attrs, b"src").is_none()
                    && attribute(attrs, b"type").is_none_or(|t| name_is(t, b"text/javascript"));
                match plain.then(|| crate::script::run(source)) {
                    Some(Ok(written)) => { scripts_run = scripts_run.saturating_add(1); for &c in &written { o.text(c); } }
                    _ => scripts_refused = scripts_refused.saturating_add(1),
                }
                continue;
            }
            if !end_tag {
                if let Some(s) = SKIP.iter().find(|s| name_is(name, s)) {
                    if !body.ends_with(b"/") { skip_until = Some(s); }
                    continue;
                }
            }
            let h = name.len() == 2 && lower(name[0]) == b'h' && (b'1'..=b'6').contains(&name[1]);
            if h {
                o.line(true);
                if !end_tag { o.bytes(if name[1] <= b'2' { b"# " } else { b"## " }); }
            } else if name_is(name, b"br") {
                o.line(false);
            } else if name_is(name, b"li") && !end_tag {
                o.line(false);
                for _ in 1..depth.max(1) { o.raw(b"  "); }
                match lists.get(depth.wrapping_sub(1)).copied().filter(|n| *n > 0) {
                    Some(n) => {
                        let mut digits = [0u8; 16];
                        let len = list_marker(n as usize, styles[depth - 1], &mut digits);
                        o.raw(&digits[..len]);
                        o.raw(b". ");
                        lists[depth - 1] = n.saturating_add(1);
                    }
                    None => o.raw(b"- "),
                }
                o.marker = true;
            } else if (name_is(name, b"dt") || name_is(name, b"dd")) && !end_tag {
                o.line(false);
                if name_is(name, b"dd") { o.raw(b"    "); }
            } else if name_is(name, b"tr") {
                // A row ends at `</tr>` or at the next `<tr>` (the end tag is optional).
                if end_tag || o.newlines == 0 { o.line(false); }
            } else if name_is(name, b"caption") {
                // The caption is its own line above the rows (not a column).
                o.line(false);
                if end_tag && table.is_some() { table = Some(o.len); }
            } else if (name_is(name, b"td") || name_is(name, b"th")) && !end_tag {
                if o.newlines == 0 { o.bytes(b" |"); o.space = true; }
            } else if BLOCK.iter().any(|n| name_is(name, n)) {
                o.pre = false;
                let list = name_is(name, b"ol") || name_is(name, b"ul");
                // A list inside a list item continues that item's lines; a
                // paragraph opening right after a list marker stays on its
                // line, and blocks inside lists are not spaced apart.
                if !(o.marker && !end_tag && !list) { o.line(!(depth > usize::from(end_tag && list))); }
                if name_is(name, b"table") {
                    if !end_tag { table = Some(o.len); } else if let Some(start) = table.take() { o.align_columns(start); }
                } else if name_is(name, b"ol") || name_is(name, b"ul") {
                    if end_tag { depth = depth.saturating_sub(1); }
                    else if depth < lists.len() {
                        let start = attribute(&body[name_end..], b"start")
                            .and_then(|v| core::str::from_utf8(v).ok()?.trim().parse::<u16>().ok()).unwrap_or(1).max(1);
                        lists[depth] = if name_is(name, b"ol") { start } else { 0 };
                        styles[depth] = match attribute(&body[name_end..], b"type") { Some([c @ (b'a' | b'A' | b'i' | b'I')]) => *c, _ => b'1' };
                        depth += 1;
                    } else { depth += 1; }
                } else if name_is(name, b"blockquote") {
                    o.quote = if end_tag { o.quote.saturating_sub(1) } else { (o.quote + 1).min(4) };
                } else if name_is(name, b"hr") {
                    o.bytes(b"----");
                    o.line(true);
                }
                if name_is(name, b"pre") && !end_tag {
                    o.pre = true;
                    // A newline right after <pre> is not content.
                    if html.get(i) == Some(&b'\n') { i += 1; } else if html[i..].starts_with(b"\r\n") { i += 2; }
                }
            } else if let Some(mark) = INLINE2.iter().find(|(n, _)| name_is(name, n)).map(|(_, m)| *m) {
                if end_tag { o.space = false; o.bytes(mark); } else { o.bytes(mark); o.space = false; }
            } else if name_is(name, b"figcaption") {
                o.line(false);
                if !end_tag { o.bytes(b"Figure:"); o.space = true; }
            } else if o.pre && (name_is(name, b"code") || name_is(name, b"kbd")) {
                // Preformatted code is already set apart: no inline markers.
            } else if MEDIA.iter().any(|n| name_is(name, n)) && !end_tag {
                // Media and frames are not played or loaded; a placeholder
                // stands for them and their fallback content is skipped.
                let label = MEDIA.iter().find(|n| name_is(name, n)).copied().unwrap_or(b"media");
                o.text(b'[');
                o.space = false;
                for &c in if name_is(name, b"iframe") { &b"frame"[..] } else { label } { o.text(c); }
                if let Some(t) = attribute(&body[name_end..], b"title").filter(|t| !t.is_empty()) { o.text(b':'); o.space = true; for &c in t { o.text(c); } }
                o.text(b']');
                if !body.ends_with(b"/") { skip_until = MEDIA.iter().find(|n| name_is(name, n)).copied(); }
            } else if let Some(mark) = INLINE.iter().find(|(n, _)| name_is(name, n)).map(|(_, m)| *m) {
                // Inline emphasis and code keep a plain-text marker, Markdown style.
                if end_tag { o.space = false; o.text(mark); } else { o.text(mark); o.space = false; }
            } else if name_is(name, b"input") && !end_tag {
                // Forms are shown, never submitted: there is no form route.
                let attrs = &body[name_end..];
                let kind = attribute(attrs, b"type").unwrap_or(b"text");
                let checked = attribute(attrs, b"checked").is_some() || attrs.windows(7).any(|w| name_is(w, b"checked"));
                if name_is(kind, b"hidden") {
                } else if name_is(kind, b"checkbox") {
                    o.bytes(if checked { b"[x]" } else { b"[ ]" });
                } else if name_is(kind, b"radio") {
                    o.bytes(if checked { b"(*)" } else { b"( )" });
                } else if name_is(kind, b"submit") || name_is(kind, b"button") || name_is(kind, b"reset") {
                    o.bytes(b"[button:");
                    o.space = true;
                    let label: &[u8] = attribute(attrs, b"value").unwrap_or(if name_is(kind, b"reset") { b"Reset" } else { b"Submit" });
                    for &c in label { o.text(c); }
                    o.text(b']');
                } else {
                    o.bytes(b"[input");
                    if let Some(hint) = attribute(attrs, b"value").or(attribute(attrs, b"placeholder")).filter(|v| !v.is_empty()) {
                        o.text(b':');
                        o.space = true;
                        for &c in hint { o.text(c); }
                    }
                    o.text(b']');
                }
            } else if (name_is(name, b"button") || name_is(name, b"textarea") || name_is(name, b"select")) && !end_tag {
                o.bytes(if name_is(name, b"button") { b"[button:" } else if name_is(name, b"select") { b"[select:" } else { b"[text:" });
                o.space = true;
            } else if (name_is(name, b"button") || name_is(name, b"textarea") || name_is(name, b"select")) && end_tag {
                o.space = false;
                o.text(b']');
            } else if name_is(name, b"option") && !end_tag {
                // Options after the first are separated inside [select: ...].
                if o.len > 0 && o.buf[o.len - 1] != b':' && !(o.len > 1 && &o.buf[o.len - 2..o.len] == b": ") { o.space = false; o.text(b' '); o.text(b'|'); }
                o.space = true;
            } else if name_is(name, b"sup") || name_is(name, b"sub") {
                if !end_tag { o.space = false; o.text(if name_is(name, b"sup") { b'^' } else { b'_' }); o.space = false; }
            } else if name_is(name, b"q") {
                if end_tag { o.space = false; }
                o.text(b'"');
                if !end_tag { o.space = false; }
            } else if name_is(name, b"summary") && !end_tag {
                o.line(false);
                o.bytes(b"> ");
            } else if name_is(name, b"summary") && end_tag {
                o.line(false);
            } else if name_is(name, b"abbr") && !end_tag {
                abbr = attribute(&body[name_end..], b"title").map(|t| (t.as_ptr() as usize - html.as_ptr() as usize, t.len()));
            } else if name_is(name, b"abbr") && end_tag {
                if let Some((start, len)) = abbr.take() {
                    o.text(b'(');
                    o.space = false;
                    for &c in &html[start..start + len] { o.text(c); }
                    o.space = false;
                    o.text(b')');
                }
            } else if name_is(name, b"img") && !end_tag {
                // Images are not fetched; their alternative text is shown.
                o.bytes(b"[image");
                if let Some(alt) = attribute(&body[name_end..], b"alt").filter(|a| a.iter().any(|b| !b.is_ascii_whitespace())) {
                    o.text(b':');
                    o.space = true;
                    for &c in alt { o.text(c); }
                }
                o.text(b']');
            } else if name_is(name, b"a") {
                if !end_tag {
                    if let Some(href) = attribute(&body[name_end..], b"href") {
                        if link_count < MAX_LINKS {
                            let start = (href.as_ptr() as usize) - (html.as_ptr() as usize);
                            links[link_count] = (start, href.len());
                            open_link = Some(link_count);
                            link_count += 1;
                        }
                    }
                } else if let Some(n) = open_link.take() {
                    let mut tagbuf = [0u8; 6];
                    let len = number(n + 1, &mut tagbuf);
                    o.text(b'[');
                    o.space = false;
                    for &d in &tagbuf[..len] { o.byte(d); }
                    o.byte(b']');
                }
            }
            continue;
        }
        if skip_until.is_some() { i += 1; continue; }
        if b == b'&' {
            let mut buf = [0u8; 4];
            if let Some((n, used)) = entity(&html[i + 1..], &mut buf) {
                for &c in &buf[..n] { o.text(c); }
                i += 1 + used;
                continue;
            }
        }
        if latin && b >= 0x80 {
            // Declared Windows-1252 / ISO-8859-1 text becomes UTF-8.
            let mut buf = [0u8; 4];
            for &c in cp1252(b).encode_utf8(&mut buf).as_bytes() { o.text(c); }
        } else {
            o.text(b);
        }
        i += 1;
    }
    let mut targets = [(0, 0); MAX_LINKS];
    o.pre = false;
    if link_count > 0 {
        o.line(true);
        o.bytes(b"Links:");
        for (n, &(start, len)) in links[..link_count].iter().enumerate() {
            o.line(false);
            let mut tagbuf = [0u8; 6];
            let digits = number(n + 1, &mut tagbuf);
            o.byte(b'[');
            for &d in &tagbuf[..digits] { o.byte(d); }
            o.byte(b']');
            o.byte(b' ');
            o.space = false;
            o.newlines = 0;
            let from = o.len;
            for &c in &html[start..start + len] { o.text(c); }
            if !o.truncated { targets[n] = (from, o.len); }
        }
    }
    let (len, truncated) = o.finish();
    Converted { len, links: link_count, truncated, targets, scripts_run, scripts_refused }
}

fn split_cells(line: &[u8]) -> alloc::vec::Vec<&[u8]> {
    let mut cells = alloc::vec::Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i + 3 <= line.len() {
        if &line[i..i + 3] == b" | " { cells.push(&line[from..i]); i += 3; from = i; } else { i += 1; }
    }
    cells.push(&line[from..]);
    cells
}

/// Text of the first `<title>` element, if any (it sits in the skipped head).
fn title(html: &[u8]) -> Option<&[u8]> {
    let open = html.windows(6).position(|w| name_is(&w[..6], b"<title"))?;
    let start = open + html[open..].iter().position(|&b| b == b'>')? + 1;
    let end = start + html[start..].windows(8).position(|w| name_is(w, b"</title>"))?;
    let t = &html[start..end];
    t.iter().any(|b| !b.is_ascii_whitespace()).then_some(t)
}

/// An ordered-list marker: decimal, letters (`a`..`z`, `aa`..) or roman.
fn list_marker(n: usize, style: u8, out: &mut [u8; 16]) -> usize {
    match style {
        b'a' | b'A' => {
            let (mut n, mut tmp, mut len) = (n, [0u8; 16], 0);
            while n > 0 && len < 16 { n -= 1; tmp[len] = style + (n % 26) as u8; len += 1; n /= 26; }
            for i in 0..len { out[i] = tmp[len - 1 - i]; }
            len
        }
        b'i' | b'I' if (1..4000).contains(&n) => {
            let mut len = 0;
            let mut n = n;
            for (v, r) in [(1000, &b"m"[..]), (900, b"cm"), (500, b"d"), (400, b"cd"), (100, b"c"), (90, b"xc"), (50, b"l"), (40, b"xl"), (10, b"x"), (9, b"ix"), (5, b"v"), (4, b"iv"), (1, b"i")] {
                while n >= v { for &c in r { if len < 16 { out[len] = if style == b'I' { c.to_ascii_uppercase() } else { c }; len += 1; } } n -= v; }
            }
            len
        }
        _ => { let mut d = [0u8; 6]; let len = number(n, &mut d); out[..len].copy_from_slice(&d[..len]); len }
    }
}

fn number(mut n: usize, out: &mut [u8; 6]) -> usize {
    let mut tmp = [0u8; 6];
    let mut len = 0;
    loop { tmp[len] = b'0' + (n % 10) as u8; len += 1; n /= 10; if n == 0 || len == 6 { break; } }
    for i in 0..len { out[i] = tmp[len - 1 - i]; }
    len
}

/// Whether bytes declare themselves as an HTML document.
pub fn looks_like_html(bytes: &[u8]) -> bool {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(bytes.len());
    let head = &bytes[start..bytes.len().min(start + 15)];
    head.len() >= 5 && (name_is(&head[..head.len().min(14)], b"<!doctype html") || name_is(&head[..5], b"<html"))
}
