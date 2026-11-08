// SPDX-License-Identifier: GPL-3.0-only
//! Tab completion of Spindle command names (the first word of the line).
//! A pure lookup over the fixed command table: no history, files or
//! services are consulted, so completion cannot reveal anything new.

/// Every command `dispatch` recognises, sorted.
pub const COMMANDS: &[&[u8]] = &[
    b"about",
    b"app-info",
    b"app-state",
    b"app-status",
    b"append",
    b"apps",
    b"atlas",
    b"authority-status",
    b"bell",
    b"bell-launch-events",
    b"bell-status",
    b"bell-test",
    b"blockers",
    b"browser",
    b"browser-localdoc",
    b"browser-localdoc-status",
    b"browser-roadmap",
    b"browser-status",
    b"browser-surface",
    b"calc",
    b"cat",
    b"catdoc",
    b"catdocx",
    b"cd",
    b"clear",
    b"close",
    b"collar-grants",
    b"collar-status",
    b"daily",
    b"df",
    b"disk",
    b"disk-stat",
    b"echo",
    b"edit",
    b"edit-help",
    b"edit-status",
    b"editor",
    b"events",
    b"faults",
    b"fetch",
    b"files",
    b"filldoc",
    b"filldocx",
    b"focus-help",
    b"frame-chrome",
    b"frame-lights",
    b"frame-lights-keys",
    b"frame-rim",
    b"help",
    b"history",
    b"history-cancel",
    b"history-retry",
    b"history-status",
    b"input",
    b"install",
    b"keys",
    b"launch",
    b"launch-outcomes",
    b"lifecycle",
    b"linen-list",
    b"linen-open",
    b"linen-search",
    b"linen-status",
    b"load",
    b"ls",
    b"mesh-graph",
    b"mesh-graph-status",
    b"mkdir",
    b"mkdoc",
    b"mv",
    b"mvdoc",
    b"net-cancel",
    b"net-probe",
    b"net-retry",
    b"net-status",
    b"notify",
    b"object-new",
    b"object-search",
    b"object-tag",
    b"pd",
    b"probe-cancel",
    b"probe-help",
    b"probe-retry",
    b"probe-status",
    b"project-scene-link",
    b"project-scene-status",
    b"proof",
    b"pwd",
    b"quil",
    b"rm",
    b"rmdoc",
    b"route",
    b"save",
    b"scene-keys",
    b"scene-lifecycle",
    b"scene-lifecycle-status",
    b"scene-status",
    b"scene-switch-status",
    b"search",
    b"servers",
    b"session",
    b"sextime",
    b"status",
    b"touch",
    b"truncdoc",
    b"url",
    b"url-status",
    b"web",
    b"window-keys",
    b"windows",
    b"winreset",
    b"write",
];

/// For a typed first word: the bytes every matching command shares beyond
/// it, and how many commands match. One match means the word is complete.
pub fn complete(word: &[u8]) -> (&'static [u8], usize) {
    let mut first: Option<&'static [u8]> = None;
    let mut common = 0;
    let mut n = 0;
    for &c in COMMANDS.iter().filter(|c| c.len() > word.len() && c.starts_with(word)) {
        n += 1;
        match first {
            None => { first = Some(c); common = c.len(); }
            Some(f) => common = common.min(f.iter().zip(c.iter()).take_while(|(a, b)| a == b).count()),
        }
    }
    match first { Some(f) => (&f[word.len()..common], n), None => (b"", 0) }
}

/// The commands that start with `word` (for listing an ambiguous Tab).
pub fn candidates(word: &[u8]) -> impl Iterator<Item = &'static [u8]> + '_ {
    COMMANDS.iter().copied().filter(move |c| c.len() > word.len() && c.starts_with(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn completes_unique_and_common_prefixes() {
        assert_eq!(complete(b"sext"), (&b"ime"[..], 1));
        assert_eq!(complete(b"his"), (&b"tory"[..], 4));
        assert_eq!(complete(b"history"), (&b"-"[..], 3), "the exact word is not a match of its own");
        assert_eq!(complete(b"zzz"), (&b""[..], 0));
        assert_eq!(complete(b"calc"), (&b""[..], 0));
        assert_eq!(candidates(b"history-").count(), 3);
        let mut sorted = COMMANDS.to_vec(); sorted.sort(); assert_eq!(sorted, COMMANDS, "table stays sorted");
    }
}
