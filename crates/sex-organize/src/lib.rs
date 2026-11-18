// SPDX-License-Identifier: GPL-3.0-only
//! Pure, allocation-free placement decisions for the SexOS Organizer.
//!
//! This crate never opens files or grants authority. The caller supplies file metadata
//! and authorized rules, then asks SexFiles to perform a checked move separately.
#![no_std]

/// A project rule outranks a specific rule, which outranks a general rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    General,
    Specific,
    Project,
}

/// Borrowed rule data. `destination` names a directory, not a constructed path.
/// The Organizer must separately prove authority for source and destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule<'a> {
    pub id: u32,
    pub class: Class,
    pub priority: i16,
    pub source: &'a [u8],
    pub name_prefix: Option<&'a [u8]>,
    /// Extension without the dot; ASCII case-insensitive.
    pub extension: Option<&'a [u8]>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub destination: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub path: &'a [u8],
    pub size: u64,
    pub pinned: bool,
    /// An explicit user choice outranks every rule. `None` means no override.
    pub override_destination: Option<&'a [u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stay {
    Pinned,
    NoMatch,
    AlreadyPlaced,
    InvalidSource,
}

/// A plan, never permission to move. `destination` and `name` must be joined
/// into a path and passed to a separately authorized SexFiles operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision<'a> {
    Stay(Stay),
    Move { destination: &'a [u8], name: &'a [u8], rule_id: Option<u32> },
    Conflict { first: u32, second: u32 },
    InvalidDestination { rule_id: Option<u32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError { Invalid, TooLong }

/// Accept only canonical absolute paths with no empty, dot, dot-dot or NUL segment.
/// The root is a valid directory but cannot be a candidate file.
pub fn valid_path(path: &[u8]) -> bool {
    if path.first() != Some(&b'/') || path.contains(&0) { return false; }
    if path == b"/" { return true; }
    if path.last() == Some(&b'/') { return false; }
    path[1..].split(|&b| b == b'/').all(|part| !part.is_empty() && part != b"." && part != b"..")
}

fn name(path: &[u8]) -> &[u8] {
    path.rsplit(|&b| b == b'/').next().unwrap_or(path)
}

fn in_directory(path: &[u8], dir: &[u8]) -> bool {
    dir == b"/" || (path.starts_with(dir) && path.get(dir.len()) == Some(&b'/'))
}

fn same_directory(path: &[u8], dir: &[u8]) -> bool {
    let parent_len = path.len() - name(path).len();
    if dir == b"/" { parent_len == 1 } else { parent_len == dir.len() + 1 && &path[..dir.len()] == dir }
}

fn ascii_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

fn extension(file_name: &[u8]) -> Option<&[u8]> {
    let dot = file_name.iter().rposition(|&b| b == b'.')?;
    if dot == 0 || dot + 1 == file_name.len() { None } else { Some(&file_name[dot + 1..]) }
}

fn matches(rule: &Rule<'_>, candidate: &Candidate<'_>, file_name: &[u8]) -> bool {
    if !valid_path(rule.source) || !in_directory(candidate.path, rule.source) { return false; }
    if let Some(prefix) = rule.name_prefix {
        if prefix.is_empty() || !file_name.starts_with(prefix) { return false; }
    }
    if let Some(ext) = rule.extension {
        if ext.is_empty() || ext.contains(&b'.') || !extension(file_name).is_some_and(|actual| ascii_eq(actual, ext)) { return false; }
    }
    if rule.min_size.is_some_and(|min| candidate.size < min) { return false; }
    if rule.max_size.is_some_and(|max| candidate.size > max) { return false; }
    true
}

fn specificity(rule: &Rule<'_>) -> u8 {
    u8::from(rule.name_prefix.is_some())
        + u8::from(rule.extension.is_some())
        + u8::from(rule.min_size.is_some())
        + u8::from(rule.max_size.is_some())
}

/// Selects a deterministic plan independent of rule order. Tied rules pointing to
/// different destinations produce a conflict for review, never an arbitrary move.
pub fn decide<'a>(candidate: Candidate<'a>, rules: &'a [Rule<'a>]) -> Decision<'a> {
    if !valid_path(candidate.path) || candidate.path == b"/" { return Decision::Stay(Stay::InvalidSource); }
    if candidate.pinned { return Decision::Stay(Stay::Pinned); }
    let file_name = name(candidate.path);
    if let Some(destination) = candidate.override_destination {
        if !valid_path(destination) { return Decision::InvalidDestination { rule_id: None }; }
        return if same_directory(candidate.path, destination) {
            Decision::Stay(Stay::AlreadyPlaced)
        } else {
            Decision::Move { destination, name: file_name, rule_id: None }
        };
    }

    let mut best: Option<(&Rule<'a>, (Class, i16, u8))> = None;
    for rule in rules {
        if !matches(rule, &candidate, file_name) { continue; }
        let rank = (rule.class, rule.priority, specificity(rule));
        match best {
            None => best = Some((rule, rank)),
            Some((_, previous)) if rank > previous => best = Some((rule, rank)),
            Some((winner, previous)) if rank == previous => {
                if rule.id < winner.id { best = Some((rule, rank)); }
            }
            _ => {}
        }
    }
    let Some((winner, rank)) = best else { return Decision::Stay(Stay::NoMatch); };
    if let Some(other) = rules.iter()
        .filter(|rule| matches(rule, &candidate, file_name)
            && (rule.class, rule.priority, specificity(rule)) == rank
            && rule.destination != winner.destination)
        .map(|rule| rule.id).min() {
        return Decision::Conflict { first: winner.id, second: other };
    }
    if !valid_path(winner.destination) { return Decision::InvalidDestination { rule_id: Some(winner.id) }; }
    if same_directory(candidate.path, winner.destination) { return Decision::Stay(Stay::AlreadyPlaced); }
    Decision::Move { destination: winner.destination, name: file_name, rule_id: Some(winner.id) }
}

/// Writes the proposed full path into `out`. The caller still checks collision,
/// identity and capability before committing a move.
pub fn join_destination(dir: &[u8], file_name: &[u8], out: &mut [u8]) -> Result<usize, PathError> {
    if !valid_path(dir) || file_name.is_empty() || file_name == b"." || file_name == b".."
        || file_name.contains(&b'/') || file_name.contains(&0) { return Err(PathError::Invalid); }
    let len = dir.len() + usize::from(dir != b"/") + file_name.len();
    if len > out.len() { return Err(PathError::TooLong); }
    out[..dir.len()].copy_from_slice(dir);
    let mut at = dir.len();
    if dir != b"/" { out[at] = b'/'; at += 1; }
    out[at..len].copy_from_slice(file_name);
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rule(id: u32, class: Class, priority: i16, ext: Option<&'static [u8]>, dest: &'static [u8]) -> Rule<'static> {
        Rule { id, class, priority, source: b"/Inbox", name_prefix: None, extension: ext,
            min_size: None, max_size: None, destination: dest }
    }

    #[test]
    fn routes_matching_file_and_preserves_unmatched() {
        let rules = [rule(7, Class::Specific, 0, Some(b"pdf"), b"/Documents")];
        assert_eq!(decide(Candidate { path: b"/Inbox/Invoice.PDF", size: 500, pinned: false, override_destination: None }, &rules),
            Decision::Move { destination: b"/Documents", name: b"Invoice.PDF", rule_id: Some(7) });
        assert_eq!(decide(Candidate { path: b"/Inbox/photo.png", size: 500, pinned: false, override_destination: None }, &rules), Decision::Stay(Stay::NoMatch));
    }

    #[test]
    fn explicit_choice_and_pin_outrank_rules() {
        let rules = [rule(1, Class::Project, 10, Some(b"pdf"), b"/Projects")];
        let mut candidate = Candidate { path: b"/Inbox/a.pdf", size: 1, pinned: false, override_destination: Some(b"/Documents") };
        assert!(matches!(decide(candidate, &rules), Decision::Move { destination: b"/Documents", rule_id: None, .. }));
        candidate.pinned = true;
        assert_eq!(decide(candidate, &rules), Decision::Stay(Stay::Pinned));
    }

    #[test]
    fn project_and_specificity_win_without_rule_order_dependence() {
        let general = rule(1, Class::General, 100, None, b"/Documents");
        let specific = rule(2, Class::Specific, 0, Some(b"pdf"), b"/Receipts");
        let project = rule(3, Class::Project, -100, Some(b"pdf"), b"/Projects/Paper");
        let candidate = Candidate { path: b"/Inbox/a.pdf", size: 1, pinned: false, override_destination: None };
        for rules in [[general, specific, project], [project, general, specific]] {
            assert!(matches!(decide(candidate, &rules), Decision::Move { rule_id: Some(3), .. }));
        }
    }

    #[test]
    fn conflicting_tied_rules_never_move() {
        let a = rule(4, Class::Specific, 0, Some(b"pdf"), b"/Documents");
        let b = rule(2, Class::Specific, 0, Some(b"pdf"), b"/Projects");
        let c = Candidate { path: b"/Inbox/a.pdf", size: 1, pinned: false, override_destination: None };
        assert_eq!(decide(c, &[a, b]), Decision::Conflict { first: 2, second: 4 });
        assert_eq!(decide(c, &[b, a]), Decision::Conflict { first: 2, second: 4 });
    }

    #[test]
    fn rejects_escaping_destinations_and_bad_names() {
        let c = Candidate { path: b"/Inbox/a.pdf", size: 1, pinned: false, override_destination: None };
        assert_eq!(decide(c, &[rule(5, Class::Specific, 0, Some(b"pdf"), b"/Documents/../bin")]),
            Decision::InvalidDestination { rule_id: Some(5) });
        assert_eq!(decide(Candidate { path: b"/Inbox/../a.pdf", ..c }, &[]), Decision::Stay(Stay::InvalidSource));
        let mut out = [0; 64];
        assert_eq!(join_destination(b"/Documents", b"a.pdf", &mut out), Ok(16));
        assert_eq!(&out[..16], b"/Documents/a.pdf");
        assert_eq!(join_destination(b"/Documents", b"../a.pdf", &mut out), Err(PathError::Invalid));
    }

    #[test]
    fn scoped_and_repeatable() {
        let rules = [rule(1, Class::General, 0, None, b"/Documents")];
        let c = Candidate { path: b"/Inboxed/a.pdf", size: 1, pinned: false, override_destination: None };
        assert_eq!(decide(c, &rules), Decision::Stay(Stay::NoMatch));
        let c = Candidate { path: b"/Documents/a.pdf", override_destination: Some(b"/Documents"), ..c };
        assert_eq!(decide(c, &rules), Decision::Stay(Stay::AlreadyPlaced));
    }

    #[test]
    fn size_and_name_predicates_narrow_a_rule() {
        let rules = [Rule { id: 9, class: Class::Specific, priority: 0,
            source: b"/Inbox", name_prefix: Some(b"scan-"), extension: Some(b"pdf"),
            min_size: Some(100), max_size: Some(1000), destination: b"/Documents/Scans" }];
        let candidate = Candidate { path: b"/Inbox/scan-12.PDF", size: 100,
            pinned: false, override_destination: None };
        assert!(matches!(decide(candidate, &rules), Decision::Move { rule_id: Some(9), .. }));
        assert_eq!(decide(Candidate { size: 99, ..candidate }, &rules), Decision::Stay(Stay::NoMatch));
        assert_eq!(decide(Candidate { path: b"/Inbox/scan.PDF", ..candidate }, &rules), Decision::Stay(Stay::NoMatch));
    }
}
