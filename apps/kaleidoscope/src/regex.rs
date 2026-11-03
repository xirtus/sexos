// SPDX-License-Identifier: GPL-3.0-only
//! Bounded regular expressions for the Browser's script subset.
//!
//! Bytes, not Unicode code points. Supported: literals and escapes (`\d \w \s
//! \D \W \S \b \B \n \t \r \.` ...), `.`, classes `[a-z_]` / `[^...]`, anchors
//! `^ $`, groups `( )` (capturing, up to 9) and `(?: )`, alternation `|`,
//! quantifiers `* + ? {n} {n,} {n,m}` with lazy `?`, and the `i` flag
//! (ASCII case). Matching is backtracking with an explicit stack and a step
//! budget, so a pathological pattern refuses instead of hanging.
use alloc::boxed::Box;
use alloc::vec::Vec;

pub const MAX_PATTERN: usize = 256;
pub const CAPTURES: usize = 10;
const MAX_INSTS: usize = 2048;
const STEPS: u32 = 200_000;
const STACK_FRAMES: usize = 512;
const MAX_REPEAT: u32 = 100;
/// Unbounded loops, each with a register that stops empty iterations.
const LOOPS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Syntax, Limit }

#[derive(Clone, Debug)]
enum Ast {
    Empty,
    Byte(u8),
    Any,
    Class(Box<[bool; 256]>),
    Start,
    End,
    WordBoundary(bool),
    Group(Option<u8>, Box<Ast>),
    Concat(Vec<Ast>),
    Alt(Vec<Ast>),
    Repeat(Box<Ast>, u32, Option<u32>, bool),
}

#[derive(Clone, Debug)]
enum Inst { Byte(u8), Any, Class(Box<[bool; 256]>), Start, End, Boundary(bool), Split(usize, usize), Jmp(usize), Save(usize), Mark(usize), Progress(usize), Match }

pub struct Regex { prog: Vec<Inst>, pub groups: usize, icase: bool }

struct Parser<'a> { p: &'a [u8], at: usize, groups: u8, depth: u8, icase: bool }

fn word(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' }

impl Parser<'_> {
    fn peek(&self) -> Option<u8> { self.p.get(self.at).copied() }
    fn alt(&mut self) -> Result<Ast, Error> {
        self.depth += 1;
        if self.depth > 32 { return Err(Error::Limit); }
        let mut arms = alloc::vec![self.seq()?];
        while self.peek() == Some(b'|') { self.at += 1; arms.push(self.seq()?); }
        self.depth -= 1;
        Ok(if arms.len() == 1 { arms.pop().unwrap() } else { Ast::Alt(arms) })
    }
    fn seq(&mut self) -> Result<Ast, Error> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == b'|' || c == b')' { break; }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(match items.len() { 0 => Ast::Empty, 1 => items.pop().unwrap(), _ => Ast::Concat(items) })
    }
    fn number(&mut self) -> Option<u32> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) { self.at += 1; }
        core::str::from_utf8(&self.p[start..self.at]).ok()?.parse().ok()
    }
    fn quantified(&mut self, atom: Ast) -> Result<Ast, Error> {
        let (min, max) = match self.peek() {
            Some(b'*') => { self.at += 1; (0, None) }
            Some(b'+') => { self.at += 1; (1, None) }
            Some(b'?') => { self.at += 1; (0, Some(1)) }
            Some(b'{') if self.p.get(self.at + 1).is_some_and(|c| c.is_ascii_digit()) => {
                self.at += 1;
                let min = self.number().ok_or(Error::Syntax)?;
                let max = if self.peek() == Some(b',') { self.at += 1; if self.peek() == Some(b'}') { None } else { Some(self.number().ok_or(Error::Syntax)?) } } else { Some(min) };
                if self.peek() != Some(b'}') { return Err(Error::Syntax); }
                self.at += 1;
                if min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT || m < min) { return Err(Error::Limit); }
                (min, max)
            }
            _ => return Ok(atom),
        };
        if matches!(atom, Ast::Start | Ast::End | Ast::WordBoundary(_)) { return Err(Error::Syntax); }
        let lazy = self.peek() == Some(b'?');
        if lazy { self.at += 1; }
        Ok(Ast::Repeat(Box::new(atom), min, max, lazy))
    }
    fn escape_class(c: u8) -> Option<[bool; 256]> {
        let mut set = [false; 256];
        let f: fn(u8) -> bool = match c.to_ascii_lowercase() {
            b'd' => |b| b.is_ascii_digit(),
            b'w' => word,
            b's' => |b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c),
            _ => return None,
        };
        for b in 0..=255u8 { set[b as usize] = f(b) != c.is_ascii_uppercase(); }
        Some(set)
    }
    fn escaped_byte(c: u8) -> u8 { match c { b'n' => b'\n', b't' => b'\t', b'r' => b'\r', b'0' => 0, b'f' => 0x0c, b'v' => 0x0b, other => other } }
    fn atom(&mut self) -> Result<Ast, Error> {
        let c = self.peek().ok_or(Error::Syntax)?;
        self.at += 1;
        Ok(match c {
            b'.' => Ast::Any,
            b'^' => Ast::Start,
            b'$' => Ast::End,
            b'(' => {
                let capture = if self.p[self.at..].starts_with(b"?:") { self.at += 2; None } else {
                    if self.groups as usize + 1 >= CAPTURES { return Err(Error::Limit); }
                    self.groups += 1; Some(self.groups)
                };
                let inner = self.alt()?;
                if self.peek() != Some(b')') { return Err(Error::Syntax); }
                self.at += 1;
                Ast::Group(capture, Box::new(inner))
            }
            b'[' => self.class()?,
            b'\\' => {
                let e = self.peek().ok_or(Error::Syntax)?;
                self.at += 1;
                if let Some(set) = Self::escape_class(e) { Ast::Class(Box::new(set)) }
                else if e == b'b' || e == b'B' { Ast::WordBoundary(e == b'b') }
                else { Ast::Byte(Self::escaped_byte(e)) }
            }
            b'*' | b'+' | b'?' | b')' => return Err(Error::Syntax),
            b'{' if self.p.get(self.at).is_some_and(|c| c.is_ascii_digit()) => return Err(Error::Syntax),
            other => Ast::Byte(other),
        })
    }
    fn class(&mut self) -> Result<Ast, Error> {
        let mut set = [false; 256];
        let negated = self.peek() == Some(b'^');
        if negated { self.at += 1; }
        let mut first = true;
        loop {
            let c = self.peek().ok_or(Error::Syntax)?;
            if c == b']' && !first { self.at += 1; break; }
            first = false;
            self.at += 1;
            let lo = if c == b'\\' {
                let e = self.peek().ok_or(Error::Syntax)?;
                self.at += 1;
                if let Some(s) = Self::escape_class(e) { for b in 0..256 { set[b] |= s[b]; } continue; }
                Self::escaped_byte(e)
            } else { c };
            if self.peek() == Some(b'-') && self.p.get(self.at + 1).is_some_and(|&n| n != b']') {
                self.at += 1;
                let mut hi = self.peek().ok_or(Error::Syntax)?;
                self.at += 1;
                if hi == b'\\' { hi = Self::escaped_byte(self.peek().ok_or(Error::Syntax)?); self.at += 1; }
                if hi < lo { return Err(Error::Syntax); }
                for b in lo..=hi { set[b as usize] = true; }
            } else { set[lo as usize] = true; }
        }
        if self.icase { for b in b'a'..=b'z' { let u = b.to_ascii_uppercase() as usize; let l = b as usize; let any = set[u] || set[l]; set[u] = any; set[l] = any; } }
        if negated { for b in set.iter_mut() { *b = !*b; } }
        Ok(Ast::Class(Box::new(set)))
    }
}

struct Compiler { prog: Vec<Inst>, loops: usize }
impl Compiler {
    fn emit(&mut self, i: Inst) -> Result<usize, Error> {
        if self.prog.len() >= MAX_INSTS { return Err(Error::Limit); }
        self.prog.push(i);
        Ok(self.prog.len() - 1)
    }
    fn node(&mut self, a: &Ast) -> Result<(), Error> {
        match a {
            Ast::Empty => {}
            Ast::Byte(b) => { self.emit(Inst::Byte(*b))?; }
            Ast::Any => { self.emit(Inst::Any)?; }
            Ast::Class(s) => { self.emit(Inst::Class(s.clone()))?; }
            Ast::Start => { self.emit(Inst::Start)?; }
            Ast::End => { self.emit(Inst::End)?; }
            Ast::WordBoundary(b) => { self.emit(Inst::Boundary(*b))?; }
            Ast::Group(cap, inner) => {
                if let Some(n) = cap { self.emit(Inst::Save(*n as usize * 2))?; }
                self.node(inner)?;
                if let Some(n) = cap { self.emit(Inst::Save(*n as usize * 2 + 1))?; }
            }
            Ast::Concat(items) => for i in items { self.node(i)?; },
            Ast::Alt(arms) => {
                let mut jumps = Vec::new();
                for (k, arm) in arms.iter().enumerate() {
                    if k + 1 < arms.len() {
                        let split = self.emit(Inst::Split(0, 0))?;
                        self.node(arm)?;
                        jumps.push(self.emit(Inst::Jmp(0))?);
                        let next = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, next);
                    } else { self.node(arm)?; }
                }
                let end = self.prog.len();
                for j in jumps { self.prog[j] = Inst::Jmp(end); }
            }
            Ast::Repeat(inner, min, max, lazy) => {
                for _ in 0..*min { self.node(inner)?; }
                match max {
                    None => {
                        // L: split body, out; mark; body; progress; jmp L
                        // (an iteration that consumed nothing fails, so `(a*)*`
                        // cannot spin).
                        if self.loops == LOOPS { return Err(Error::Limit); }
                        let r = self.loops;
                        self.loops += 1;
                        let split = self.emit(Inst::Split(0, 0))?;
                        self.emit(Inst::Mark(r))?;
                        self.node(inner)?;
                        self.emit(Inst::Progress(r))?;
                        self.emit(Inst::Jmp(split))?;
                        let out = self.prog.len();
                        self.prog[split] = if *lazy { Inst::Split(out, split + 1) } else { Inst::Split(split + 1, out) };
                    }
                    Some(m) => {
                        let mut splits = Vec::new();
                        for _ in *min..*m {
                            splits.push(self.emit(Inst::Split(0, 0))?);
                            self.node(inner)?;
                        }
                        let out = self.prog.len();
                        for s in splits { self.prog[s] = if *lazy { Inst::Split(out, s + 1) } else { Inst::Split(s + 1, out) }; }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Regex {
    pub fn new(pattern: &[u8], flags: &[u8]) -> Result<Self, Error> {
        if pattern.len() > MAX_PATTERN { return Err(Error::Limit); }
        let icase = flags.contains(&b'i');
        if flags.iter().any(|f| !matches!(f, b'i' | b'g')) { return Err(Error::Syntax); }
        let mut p = Parser { p: pattern, at: 0, groups: 0, depth: 0, icase };
        let ast = p.alt()?;
        if p.at != pattern.len() { return Err(Error::Syntax); }
        let mut c = Compiler { prog: Vec::new(), loops: 0 };
        c.emit(Inst::Save(0))?;
        c.node(&ast)?;
        c.emit(Inst::Save(1))?;
        c.emit(Inst::Match)?;
        Ok(Regex { prog: c.prog, groups: p.groups as usize, icase })
    }

    /// First match at or after `from`: capture spans (0 = whole match).
    pub fn find(&self, text: &[u8], from: usize) -> Result<Option<[Option<(usize, usize)>; CAPTURES]>, Error> {
        let mut steps = 0u32;
        for start in from..=text.len() {
            if let Some(slots) = self.run(text, start, &mut steps)? {
                let mut caps = [None; CAPTURES];
                for (n, cap) in caps.iter_mut().enumerate().take(self.groups + 1) {
                    if let (Some(a), Some(b)) = (slots[n * 2], slots[n * 2 + 1]) { *cap = Some((a, b)); }
                }
                return Ok(Some(caps));
            }
        }
        Ok(None)
    }

    fn run(&self, text: &[u8], start: usize, steps: &mut u32) -> Result<Option<[Option<usize>; CAPTURES * 2]>, Error> {
        enum Frame { Try(usize, usize), Restore(usize, Option<usize>), Unmark(usize, usize) }
        let mut slots = [None; CAPTURES * 2];
        let mut marks = [usize::MAX; LOOPS];
        // Backtracking state lives on the (small, shared) heap: bounded and
        // grown fallibly, so a long greedy match refuses instead of aborting.
        fn push<T>(stack: &mut Vec<T>, f: T) -> Result<(), Error> {
            if stack.len() >= STACK_FRAMES { return Err(Error::Limit); }
            if stack.len() == stack.capacity() { stack.try_reserve(stack.len().max(16)).map_err(|_| Error::Limit)?; }
            stack.push(f);
            Ok(())
        }
        let mut stack: Vec<Frame> = Vec::new();
        push(&mut stack, Frame::Try(0, start))?;
        let eq = |a: u8, b: u8| if self.icase { a.eq_ignore_ascii_case(&b) } else { a == b };
        while let Some(frame) = stack.pop() {
            let (mut pc, mut pos) = match frame { Frame::Restore(s, v) => { slots[s] = v; continue } Frame::Unmark(r, v) => { marks[r] = v; continue } Frame::Try(pc, pos) => (pc, pos) };
            loop {
                *steps += 1;
                if *steps > STEPS { return Err(Error::Limit); }
                match &self.prog[pc] {
                    Inst::Byte(b) => { if pos < text.len() && eq(text[pos], *b) { pc += 1; pos += 1; } else { break } }
                    Inst::Any => { if pos < text.len() && text[pos] != b'\n' { pc += 1; pos += 1; } else { break } }
                    Inst::Class(set) => { if pos < text.len() && set[text[pos] as usize] { pc += 1; pos += 1; } else { break } }
                    Inst::Start => { if pos == 0 { pc += 1 } else { break } }
                    Inst::End => { if pos == text.len() { pc += 1 } else { break } }
                    Inst::Boundary(want) => {
                        let before = pos > 0 && word(text[pos - 1]);
                        let after = pos < text.len() && word(text[pos]);
                        if (before != after) == *want { pc += 1 } else { break }
                    }
                    Inst::Split(a, b) => { push(&mut stack, Frame::Try(*b, pos))?; pc = *a; }
                    Inst::Jmp(a) => pc = *a,
                    Inst::Save(s) => { push(&mut stack, Frame::Restore(*s, slots[*s]))?; slots[*s] = Some(pos); pc += 1; }
                    Inst::Mark(r) => { push(&mut stack, Frame::Unmark(*r, marks[*r]))?; marks[*r] = pos; pc += 1; }
                    Inst::Progress(r) => { if pos != marks[*r] { pc += 1 } else { break } }
                    Inst::Match => return Ok(Some(slots)),
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m(p: &str, f: &str, t: &str) -> Option<(usize, usize)> { Regex::new(p.as_bytes(), f.as_bytes()).unwrap().find(t.as_bytes(), 0).unwrap().and_then(|c| c[0]) }
    fn groups(p: &str, t: &str) -> Vec<Option<&'static str>> {
        let t: &'static str = Box::leak(t.to_string().into_boxed_str());
        let r = Regex::new(p.as_bytes(), b"").unwrap();
        let c = r.find(t.as_bytes(), 0).unwrap().unwrap();
        (0..=r.groups).map(|n| c[n].map(|(a, b)| &t[a..b])).collect()
    }
    #[test] fn basics() {
        assert_eq!(m("b+", "", "aabbbc"), Some((2, 5)));
        assert_eq!(m("^a.c$", "", "abc"), Some((0, 3)));
        assert_eq!(m("^a.c$", "", "abcd"), None);
        assert_eq!(m("colou?r", "", "my color"), Some((3, 8)));
        assert_eq!(m("\\d{2,3}", "", "a1234"), Some((1, 4)));
        assert_eq!(m("\\d{2,3}?", "", "a1234"), Some((1, 3)));
        assert_eq!(m("[^a-c ]+", "", "abc xyz"), Some((4, 7)));
        assert_eq!(m("HELLO", "i", "say hello"), Some((4, 9)));
        assert_eq!(m("[a-z]+", "i", "123ABC"), Some((3, 6)));
        assert_eq!(m("cat|dog", "", "hotdog"), Some((3, 6)));
        assert_eq!(m("\\bis\\b", "", "this is it"), Some((5, 7)));
        assert_eq!(m("a.*b", "", "axxbyyb"), Some((0, 7)));
        assert_eq!(m("a.*?b", "", "axxbyyb"), Some((0, 4)));
        assert_eq!(m("x*", "", "abc"), Some((0, 0)));
    }
    #[test] fn captures() {
        assert_eq!(groups("(\\w+)@(\\w+)\\.com", "mail ann@sex.com now"), vec![Some("ann@sex.com"), Some("ann"), Some("sex")]);
        assert_eq!(groups("(a)|(b)", "b"), vec![Some("b"), None, Some("b")]);
        assert_eq!(groups("(?:ab)+(c)", "ababc"), vec![Some("ababc"), Some("c")]);
        assert_eq!(groups("(a*)+b", "aab"), vec![Some("aab"), Some("aa")], "an empty iteration is rejected, as in JavaScript");
    }
    #[test] fn refuses_bad_patterns_and_runaway_matches() {
        for p in ["(", "a)", "*a", "[a-", "a{3,2}", "[z-a]", "a{1000}"] { assert!(Regex::new(p.as_bytes(), b"").is_err(), "{p}"); }
        assert!(Regex::new(b"a", b"x").is_err());
        let r = Regex::new(b"(a*)*b", b"").unwrap();
        assert_eq!(r.find(&[b'a'; 40], 0), Err(Error::Limit));
    }
}
