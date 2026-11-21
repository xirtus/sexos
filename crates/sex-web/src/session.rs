//! Session restore's file, as Chrome's "Continue where you left off": a window's tabs,
//! their groups and the shown one, one line each, tab-separated:
//! `shown <n>`, `group <colour hex> <folded 0|1> <name>`, and
//! `tab <group or -> <own|1..4> <url> <title>`.

use crate::TabGroup;
use alloc::string::String;
use alloc::vec::Vec;

/// Most tabs and groups a session keeps.
pub const MAX_TABS: usize = 24;
pub const MAX_GROUPS: usize = 8;

/// Tabs and newlines would break a line apart.
fn clean(s: &str) -> String { s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect() }

/// A kept tab: its group, box, address and title.
#[derive(Debug, PartialEq)]
pub struct Kept {
    pub group: Option<usize>,
    /// 0 for the person's own cookies, 1..=4 a container.
    pub container: u8,
    pub url: String,
    pub title: String,
}

/// A kept window: its tabs, groups and the shown tab.
#[derive(Debug, PartialEq, Default)]
pub struct Session { pub tabs: Vec<Kept>, pub groups: Vec<TabGroup>, pub shown: usize }

impl Session {
    pub fn text(&self) -> String {
        let mut out = alloc::format!("shown\t{}\n", self.shown);
        for g in &self.groups { out.push_str(&alloc::format!("group\t{:08x}\t{}\t{}\n", g.colour, g.folded as u8, clean(&g.name))); }
        for t in &self.tabs {
            let group = t.group.map_or(String::from("-"), |g| alloc::format!("{g}"));
            let container = if t.container == 0 { String::from("own") } else { alloc::format!("{}", t.container) };
            out.push_str(&alloc::format!("tab\t{}\t{}\t{}\t{}\n", group, container, clean(&t.url), clean(&t.title)));
        }
        out
    }

    /// Reads a session back, leaving out what doesn't make sense (a group that isn't
    /// there, a private box, a shown tab past the end).
    pub fn parse(text: &str) -> Session {
        let mut s = Session::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["shown", n] => s.shown = n.parse().unwrap_or(0),
                ["group", colour, folded, name] if s.groups.len() < MAX_GROUPS => {
                    let Ok(colour) = u32::from_str_radix(colour, 16) else { continue };
                    s.groups.push(TabGroup { colour, folded: *folded == "1", name: String::from(*name) });
                }
                ["tab", group, jarbox, url, title] if s.tabs.len() < MAX_TABS && !url.is_empty() => {
                    let container = match *jarbox { "own" => 0, n => match n.parse::<u8>() { Ok(n @ 1..=4) => n, _ => continue } };
                    s.tabs.push(Kept { group: group.parse().ok(), container, url: String::from(*url), title: String::from(*title) });
                }
                _ => {}
            }
        }
        for t in s.tabs.iter_mut() { if t.group.is_some_and(|g| g >= s.groups.len()) { t.group = None; } }
        if s.shown >= s.tabs.len() { s.shown = 0; }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    #[test]
    fn a_session_reads_back_as_written() {
        let s = Session {
            tabs: alloc::vec![
                Kept { group: None, container: 0, url: String::from("https://a.example/"), title: String::from("A\ttab") },
                Kept { group: Some(0), container: 2, url: String::from("https://b.example/x?y=1"), title: String::new() },
            ],
            groups: alloc::vec![TabGroup { colour: 0xFF1A_73E8, name: String::from("news"), folded: true }],
            shown: 1,
        };
        let back = Session::parse(&s.text());
        assert_eq!(back.tabs[0].title, "A tab");
        assert_eq!((back.tabs[1].group, back.tabs[1].container, back.shown), (Some(0), 2, 1));
        assert_eq!(back.groups, s.groups);
    }

    #[test]
    fn nonsense_is_left_out() {
        let back = Session::parse("shown\t9\ntab\t3\town\thttps://a/\tA\ntab\t-\tprivate\thttps://p/\tP\ntab\t-\t7\thttps://c/\tC\ngroup\tzz\t0\tx\n");
        assert_eq!(back.tabs.len(), 1);
        assert_eq!((back.tabs[0].group, back.shown), (None, 0));
        assert!(back.groups.is_empty());
    }
}
