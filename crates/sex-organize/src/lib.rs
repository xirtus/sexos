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

/// Folders where new files arrive and may be organized: the Inbox, the person's
/// Downloads and Desktop, and a program's own `Downloads` folder (`/data/NAME/Downloads`,
/// where Kaleidoscope saves). Only files directly inside one count; subfolders are the
/// person's own arrangement and stay out of scope.
pub fn ingress(path: &[u8]) -> bool {
    if !valid_path(path) || path == b"/" { return false; }
    let parent = &path[..path.len() - name(path).len() - 1];
    if matches!(parent, b"/Inbox" | b"/Downloads" | b"/Desktop") { return true; }
    let Some(rest) = parent.strip_prefix(b"/data/") else { return false };
    let mut parts = rest.split(|&b| b == b'/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(n), Some(b"Downloads"), None) if !n.is_empty())
}

/// A name still being written or meant to stay hidden: dot files (Loom's `.NAME.part`
/// copies) and the usual partial-download suffixes. These never produce a placement.
pub fn unsettled(file_name: &[u8]) -> bool {
    file_name.is_empty() || file_name[0] == b'.'
        || [&b".part"[..], b".partial", b".crdownload", b".tmp", b".download"].iter().any(|s| {
            file_name.len() > s.len() && ascii_eq(&file_name[file_name.len() - s.len()..], s)
        })
}

/// Where an organizer may put files: a canonical folder in the person's own tree. Never
/// hidden folders, programs' private folders, system folders or other disks.
pub fn destination_allowed(dir: &[u8]) -> bool {
    if !valid_path(dir) || dir == b"/" || dir.len() > 200 { return false; }
    let first = dir[1..].split(|&b| b == b'/').next().unwrap_or(b"");
    if first.first() == Some(&b'.') { return false; }
    !matches!(first, b"data" | b"media" | b"bin" | b"servers" | b"programs" | b"home" | b"boot" | b"system" | b"Inbox")
        && dir[1..].split(|&b| b == b'/').all(|p| p.first() != Some(&b'.'))
}

/// The built-in placements, by extension. Rule ids 1..=4 name the category.
pub const DOCUMENTS: &[&[u8]] = &[b"pdf", b"txt", b"md", b"odt", b"ods", b"odp", b"doc", b"docx", b"xls", b"xlsx", b"ppt", b"pptx", b"rtf", b"epub", b"csv"];
pub const PICTURES: &[&[u8]] = &[b"png", b"jpg", b"jpeg", b"gif", b"webp", b"bmp", b"qoi", b"svg", b"avif", b"tif", b"tiff", b"heic"];
pub const MUSIC: &[&[u8]] = &[b"mp3", b"flac", b"ogg", b"opus", b"wav", b"m4a", b"aac"];
pub const VIDEOS: &[&[u8]] = &[b"mp4", b"mkv", b"webm", b"mov", b"avi", b"m4v"];
pub const BUILTIN: [(u32, &[u8], &[&[u8]]); 4] = [
    (1, b"/Documents", DOCUMENTS), (2, b"/Pictures", PICTURES), (3, b"/Music", MUSIC), (4, b"/Videos", VIDEOS),
];
/// User rules are numbered from here, in the order they were added.
pub const USER_RULE_BASE: u32 = 100;
pub const USER_RULES_MAX: usize = 32;

/// One user rule line: `PATTERN DESTINATION`. PATTERN is `*.ext`, `prefix*` or
/// `prefix*.ext`; DESTINATION a folder `destination_allowed` accepts.
pub fn parse_rule(line: &[u8]) -> Option<(Option<&[u8]>, Option<&[u8]>, &[u8])> {
    let space = line.iter().position(|&b| b == b' ')?;
    let (pattern, dest) = (&line[..space], &line[space + 1..]);
    if !destination_allowed(dest) || pattern.contains(&b'/') || pattern.contains(&0) { return None; }
    let star = pattern.iter().position(|&b| b == b'*')?;
    if pattern[star + 1..].contains(&b'*') { return None; }
    let (prefix, tail) = (&pattern[..star], &pattern[star + 1..]);
    let ext = match tail {
        b"" => None,
        t if t.len() > 1 && t[0] == b'.' && !t[1..].contains(&b'.') => Some(&t[1..]),
        _ => return None,
    };
    if prefix.is_empty() && ext.is_none() { return None; }
    Some(((!prefix.is_empty()).then_some(prefix), ext, dest))
}

/// What the organizer would do with one arrived file, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan<'a> {
    Move { destination: &'a [u8], rule_id: u32 },
    Stay(Stay),
    Conflict { first: u32, second: u32 },
    /// Outside the organized folders, or a name still being written.
    Ignore,
}

/// Plans one file: pins first, then the person's rules (newline-separated `rules`, which
/// outrank the built-ins), then the built-in placements; anything else stays.
pub fn plan<'a>(path: &'a [u8], size: u64, pinned: bool, rules: &'a [u8]) -> Plan<'a> {
    if !ingress(path) || unsettled(name(path)) { return Plan::Ignore; }
    if pinned { return Plan::Stay(Stay::Pinned); }
    let parent = &path[..path.len() - name(path).len() - 1];
    let candidate = Candidate { path, size, pinned: false, override_destination: None };
    let blank = Rule { id: 0, class: Class::Project, priority: 0, source: parent, name_prefix: None, extension: None, min_size: None, max_size: None, destination: b"/" };
    let mut list: [Rule<'a>; USER_RULES_MAX] = [blank; USER_RULES_MAX];
    let mut n = 0;
    for (i, line) in rules.split(|&b| b == b'\n').filter(|l| !l.is_empty()).take(USER_RULES_MAX).enumerate() {
        if let Some((prefix, ext, dest)) = parse_rule(line) {
            list[n] = Rule { id: USER_RULE_BASE + i as u32, name_prefix: prefix, extension: ext, destination: dest, ..blank };
            n += 1;
        }
    }
    let chosen = match decide(candidate, &list[..n]) {
        Decision::Move { rule_id, .. } => rule_id,
        Decision::Conflict { first, second } => return Plan::Conflict { first, second },
        Decision::Stay(Stay::AlreadyPlaced) => return Plan::Stay(Stay::AlreadyPlaced),
        _ => None,
    };
    if let Some(id) = chosen {
        let destination = list[..n].iter().find(|r| r.id == id).map(|r| r.destination).unwrap_or(b"/");
        return Plan::Move { destination, rule_id: id };
    }
    let Some(ext) = extension(name(path)) else { return Plan::Stay(Stay::NoMatch) };
    for (id, dest, exts) in BUILTIN {
        if exts.iter().any(|e| ascii_eq(e, ext)) { return Plan::Move { destination: dest, rule_id: id }; }
    }
    Plan::Stay(Stay::NoMatch)
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

    #[test]
    fn ingress_folders_and_unsettled_names() {
        assert!(ingress(b"/Inbox/a.pdf") && ingress(b"/Downloads/a") && ingress(b"/Desktop/x.png"));
        assert!(ingress(b"/data/kaleidoscope/Downloads/song.mp3"));
        assert!(!ingress(b"/Inbox/sub/a.pdf") && !ingress(b"/data/Downloads/a") && !ingress(b"/data/k/Other/a"));
        assert!(!ingress(b"/Documents/a.pdf") && !ingress(b"/Inbox") && !ingress(b"/Inbox/../a"));
        assert!(unsettled(b".a.pdf.part") && unsettled(b"big.iso.PART") && unsettled(b"x.crdownload") && !unsettled(b"a.pdf"));
    }

    #[test]
    fn destinations_stay_in_the_persons_tree() {
        assert!(destination_allowed(b"/Documents") && destination_allowed(b"/Documents/Taxes 2026"));
        for bad in [&b"/"[..], b"/data/x", b"/media/STICK", b"/.Trash", b"/Documents/.hidden", b"/bin", b"/Inbox", b"/a/../b", b"rel"] {
            assert!(!destination_allowed(bad), "{:?}", core::str::from_utf8(bad));
        }
    }

    #[test]
    fn plans_builtins_rules_pins_and_conflicts() {
        assert_eq!(plan(b"/Inbox/a.PDF", 1, false, b""), Plan::Move { destination: b"/Documents", rule_id: 1 });
        assert_eq!(plan(b"/Downloads/b.jpeg", 1, false, b""), Plan::Move { destination: b"/Pictures", rule_id: 2 });
        assert_eq!(plan(b"/Desktop/c.flac", 1, false, b""), Plan::Move { destination: b"/Music", rule_id: 3 });
        assert_eq!(plan(b"/data/web/Downloads/d.webm", 1, false, b""), Plan::Move { destination: b"/Videos", rule_id: 4 });
        assert_eq!(plan(b"/Inbox/e.zip", 1, false, b""), Plan::Stay(Stay::NoMatch));
        assert_eq!(plan(b"/Inbox/noext", 1, false, b""), Plan::Stay(Stay::NoMatch));
        assert_eq!(plan(b"/Inbox/a.pdf", 1, true, b""), Plan::Stay(Stay::Pinned));
        assert_eq!(plan(b"/Inbox/.a.pdf.part", 1, false, b""), Plan::Ignore);
        assert_eq!(plan(b"/Documents/a.pdf", 1, false, b""), Plan::Ignore);
        let rules = b"invoice-* /Documents/Invoices\n*.zip /Archives\n";
        assert_eq!(plan(b"/Inbox/invoice-7.pdf", 1, false, rules), Plan::Move { destination: b"/Documents/Invoices", rule_id: 100 });
        assert_eq!(plan(b"/Downloads/x.ZIP", 1, false, rules), Plan::Move { destination: b"/Archives", rule_id: 101 });
        let tied = b"*.pdf /A\n*.pdf /B\n";
        assert_eq!(plan(b"/Inbox/a.pdf", 1, false, tied), Plan::Conflict { first: 100, second: 101 });
        // Bad lines are skipped, never widened.
        assert_eq!(plan(b"/Inbox/a.pdf", 1, false, b"* /X\n*.pdf /data/x\n"), Plan::Move { destination: b"/Documents", rule_id: 1 });
    }

    #[test]
    fn rule_lines_parse_strictly() {
        assert_eq!(parse_rule(b"*.pdf /Documents"), Some((None, Some(&b"pdf"[..]), &b"/Documents"[..])));
        assert_eq!(parse_rule(b"scan-*.pdf /Scans"), Some((Some(&b"scan-"[..]), Some(&b"pdf"[..]), &b"/Scans"[..])));
        for bad in [&b"*"[..], b"*.pdf", b"a.pdf /X", b"*.p.df /X", b"**.pdf /X", b"a/*.pdf /X", b"*.pdf /media/S"] {
            assert_eq!(parse_rule(bad), None, "{:?}", core::str::from_utf8(bad));
        }
    }
}
