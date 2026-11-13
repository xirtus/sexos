// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The per-program firewall: what each program the person let onto the network may
//! reach. A program run without the network grant reaches nothing, whatever a rule
//! says; these rules only narrow a grant (or, with `allow`, document it). Rules are
//! text a person can read and Settings writes, one per line:
//!
//! ```text
//! # program  action  direction  target
//! web        allow   out        any:80,443
//! talk       allow   listen     7070
//! *          deny    out        10.0.0.0/8
//! *          deny    out        lan
//! ```
//!
//! `direction` is `out` (connect or send), `listen` (accept or bind) or `any`.
//! `target` is `any`, `lan` (private and link-local addresses), an IPv4 or IPv6
//! prefix, optionally `:PORTS` (`80`, `8000-8080`, `80,443`), or for `listen` just
//! ports. A program's own rules are read before `*` rules, top to bottom; the first
//! that matches decides. With no match, the grant stands: allowed.

pub const MAX_RULES: usize = 256;
pub const MAX_NAME: usize = 32;
const MAX_PORT_RANGES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Out, Listen, Any }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address { V4([u8; 4]), V6([u8; 16]) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target { Any, Lan, V4([u8; 4], u8), V6([u8; 16], u8) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    name: [u8; MAX_NAME],
    name_len: usize,
    pub allow: bool,
    pub direction: Direction,
    target: Target,
    ports: [(u16, u16); MAX_PORT_RANGES],
    port_count: usize,
    /// The line the rule came from, to show which rule decided.
    pub line: u16,
}

impl Rule {
    pub fn program(&self) -> &[u8] { &self.name[..self.name_len] }
    fn for_all(&self) -> bool { self.program() == b"*" }
    fn matches(&self, direction: Direction, address: Option<Address>, port: u16) -> bool {
        let dir = self.direction == Direction::Any || direction == Direction::Any || self.direction == direction;
        let ports = self.port_count == 0 || self.ports[..self.port_count].iter().any(|(a, b)| (*a..=*b).contains(&port));
        let target = match (self.target, address) {
            (Target::Any, _) => true,
            (_, None) => direction == Direction::Listen,
            (Target::Lan, Some(a)) => local(a),
            (Target::V4(p, n), Some(Address::V4(a))) => prefix(&p, &a, n),
            (Target::V6(p, n), Some(Address::V6(a))) => prefix(&p, &a, n),
            _ => false,
        };
        dir && ports && target
    }
}

fn prefix(p: &[u8], a: &[u8], bits: u8) -> bool {
    let (whole, rest) = ((bits / 8) as usize, bits % 8);
    p[..whole] == a[..whole] && (rest == 0 || (p[whole] ^ a[whole]) >> (8 - rest) == 0)
}

/// Private, loopback and link-local addresses: the local network.
pub fn local(a: Address) -> bool {
    match a {
        Address::V4(a) => a[0] == 10 || a[0] == 127 || (a[0] == 172 && a[1] & 0xf0 == 16) || (a[0] == 192 && a[1] == 168) || (a[0] == 169 && a[1] == 254),
        Address::V6(a) => a == LOOPBACK6 || a[0] & 0xfe == 0xfc || (a[0] == 0xfe && a[1] & 0xc0 == 0x80),
    }
}
const LOOPBACK6: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError { pub line: u16 }

fn parse_v4(s: &str) -> Option<[u8; 4]> {
    let mut o = [0u8; 4];
    let mut it = s.split('.');
    for b in o.iter_mut() { *b = it.next()?.parse().ok()?; }
    it.next().is_none().then_some(o)
}
fn parse_v6(s: &str) -> Option<[u8; 16]> {
    let (head, tail) = match s.split_once("::") { Some((h, t)) => (h, Some(t)), None => (s, None) };
    let words = |p: &str| -> Option<([u16; 8], usize)> {
        let mut w = [0u16; 8];
        let mut n = 0;
        if p.is_empty() { return Some((w, 0)); }
        for g in p.split(':') { if n == 8 || g.is_empty() || g.len() > 4 { return None; } w[n] = u16::from_str_radix(g, 16).ok()?; n += 1; }
        Some((w, n))
    };
    let (h, hn) = words(head)?;
    let mut all = [0u16; 8];
    all[..hn].copy_from_slice(&h[..hn]);
    match tail {
        None if hn == 8 => {}
        None => return None,
        Some(t) => { let (t, tn) = words(t)?; if hn + tn > 7 { return None; } all[8 - tn..].copy_from_slice(&t[..tn]); }
    }
    let mut o = [0u8; 16];
    for (i, w) in all.iter().enumerate() { o[i * 2..i * 2 + 2].copy_from_slice(&w.to_be_bytes()); }
    Some(o)
}
fn parse_ports(s: &str, out: &mut [(u16, u16); MAX_PORT_RANGES]) -> Option<usize> {
    let mut n = 0;
    for part in s.split(',') {
        if n == MAX_PORT_RANGES { return None; }
        let (a, b) = match part.split_once('-') { Some((a, b)) => (a.parse().ok()?, b.parse().ok()?), None => { let p = part.parse().ok()?; (p, p) } };
        if a == 0 || a > b { return None; }
        out[n] = (a, b);
        n += 1;
    }
    Some(n)
}

fn parse_line(line: &str, number: u16) -> Result<Option<Rule>, ParseError> {
    let err = ParseError { line: number };
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() { return Ok(None); }
    let mut w = line.split_ascii_whitespace();
    let (Some(name), Some(action), Some(direction), Some(target), None) = (w.next(), w.next(), w.next(), w.next(), w.next()) else { return Err(err) };
    if name.len() > MAX_NAME || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.*".contains(&b)) || (name.contains('*') && name != "*") { return Err(err); }
    let allow = match action { "allow" => true, "deny" => false, _ => return Err(err) };
    let direction = match direction { "out" => Direction::Out, "listen" => Direction::Listen, "any" => Direction::Any, _ => return Err(err) };
    let mut rule = Rule { name: [0; MAX_NAME], name_len: name.len(), allow, direction, target: Target::Any, ports: [(0, 0); MAX_PORT_RANGES], port_count: 0, line: number };
    rule.name[..name.len()].copy_from_slice(name.as_bytes());
    // Ports alone (listen 7070), or TARGET[:PORTS] with IPv6 in brackets.
    if target.bytes().all(|b| b.is_ascii_digit() || b == b',' || b == b'-') {
        rule.port_count = parse_ports(target, &mut rule.ports).ok_or(err)?;
        return Ok(Some(rule));
    }
    let (addr, ports) = if let Some(rest) = target.strip_prefix('[') {
        let (a, p) = rest.split_once(']').ok_or(err)?;
        (a, p.strip_prefix(':'))
    } else if target.matches(':').count() == 1 || !target.contains(':') {
        match target.split_once(':') { Some((a, p)) => (a, Some(p)), None => (target, None) }
    } else { (target, None) };
    if let Some(p) = ports { rule.port_count = parse_ports(p, &mut rule.ports).ok_or(err)?; }
    rule.target = match addr {
        "any" => Target::Any,
        "lan" => Target::Lan,
        a => {
            let (ip, bits) = match a.split_once('/') { Some((i, b)) => (i, Some(b.parse::<u8>().map_err(|_| err)?)), None => (a, None) };
            if let Some(v4) = parse_v4(ip) { let b = bits.unwrap_or(32); if b > 32 { return Err(err); } Target::V4(v4, b) }
            else if let Some(v6) = parse_v6(ip) { let b = bits.unwrap_or(128); if b > 128 { return Err(err); } Target::V6(v6, b) }
            else { return Err(err); }
        }
    };
    Ok(Some(rule))
}

/// What the firewall said, and which rule said it (None: no rule, the grant stands).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict { pub allowed: bool, pub line: Option<u16> }

pub struct Firewall { rules: [Option<Rule>; MAX_RULES], count: usize }

impl Firewall {
    pub const fn empty() -> Self { Self { rules: [None; MAX_RULES], count: 0 } }
    /// Reads rules; a bad line refuses the whole text (so a typo never opens a hole).
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut f = Self::empty();
        for (i, line) in text.lines().enumerate() {
            let number = u16::try_from(i + 1).map_err(|_| ParseError { line: u16::MAX })?;
            if let Some(rule) = parse_line(line, number)? {
                if f.count == MAX_RULES { return Err(ParseError { line: number }); }
                f.rules[f.count] = Some(rule);
                f.count += 1;
            }
        }
        Ok(f)
    }
    pub fn rules(&self) -> impl Iterator<Item = &Rule> { self.rules[..self.count].iter().flatten() }
    /// May `program` (granted the network) do this? `address` is None for a listen.
    pub fn check(&self, program: &[u8], direction: Direction, address: Option<Address>, port: u16) -> Verdict {
        let own = self.rules().filter(|r| r.program() == program);
        let all = self.rules().filter(|r| r.for_all());
        for r in own.chain(all) {
            if r.matches(direction, address, port) { return Verdict { allowed: r.allow, line: Some(r.line) }; }
        }
        Verdict { allowed: true, line: None }
    }
    /// The rules that name `program` or everyone, for its row in Settings.
    pub fn for_program<'a>(&'a self, program: &'a [u8]) -> impl Iterator<Item = &'a Rule> + 'a {
        self.rules().filter(move |r| r.program() == program || r.for_all())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RULES: &str = "# the person's rules\nweb allow out any:80,443\nweb deny out any\ntalk allow listen 7070\ntalk deny listen any\n* deny out 10.0.0.0/8\n* deny out [fd00::/8]:22\nmail allow out lan:993 # imap at home\n* deny out lan\n";
    #[test]
    fn a_programs_own_rules_come_first_then_everyones_then_the_grant() {
        let f = Firewall::parse(RULES).unwrap();
        let v4 = |a, b, c, d| Some(Address::V4([a, b, c, d]));
        assert_eq!(f.check(b"web", Direction::Out, v4(93, 184, 215, 14), 443), Verdict { allowed: true, line: Some(2) });
        assert_eq!(f.check(b"web", Direction::Out, v4(93, 184, 215, 14), 25), Verdict { allowed: false, line: Some(3) });
        assert!(f.check(b"talk", Direction::Listen, None, 7070).allowed);
        assert_eq!(f.check(b"talk", Direction::Listen, None, 8080).line, Some(5));
        assert!(!f.check(b"peek", Direction::Out, v4(10, 1, 2, 3), 80).allowed);
        assert!(!f.check(b"peek", Direction::Out, v4(192, 168, 1, 1), 80).allowed, "lan");
        assert!(f.check(b"mail", Direction::Out, v4(192, 168, 1, 5), 993).allowed);
        assert_eq!(f.check(b"peek", Direction::Out, v4(1, 1, 1, 1), 53), Verdict { allowed: true, line: None });
        let six = |s| Some(Address::V6(parse_v6(s).unwrap()));
        assert!(!f.check(b"ssh", Direction::Out, six("fd12::1"), 22).allowed);
        assert!(f.check(b"ssh", Direction::Out, six("2001:db8::1"), 22).allowed);
        assert!(!f.check(b"ssh", Direction::Out, six("fe80::1"), 443).allowed, "link-local is lan");
        assert_eq!(f.for_program(b"web").count(), 5);
    }
    #[test]
    fn a_bad_line_refuses_the_whole_text() {
        for (bad, line) in [("web allow out any:0\n", 1), ("ok allow out any\nweb permit out any\n", 2), ("web allow out 300.1.1.1\n", 1), ("we*b allow out any\n", 1), ("web allow out 10.0.0.0/33\n", 1), ("web allow out\n", 1)] {
            assert_eq!(Firewall::parse(bad).err(), Some(ParseError { line }), "{bad}");
        }
        assert_eq!(parse_v6("2001:db8::1"), Some([0x20, 1, 0xd, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]));
        assert_eq!(parse_v6("::1").map(|a| a[15]), Some(1));
        assert_eq!(parse_v6("1:2:3:4:5:6:7:8:9"), None);
    }
}
