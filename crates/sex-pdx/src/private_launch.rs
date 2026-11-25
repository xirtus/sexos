//! Recognize a private Kaleidoscope command before terminal history captures it.

fn browser(word: &[u8]) -> bool {
    let name = word.rsplit(|&b| b == b'/').next().unwrap_or(word);
    matches!(name, b"kaleidoscope" | b"web" | b"browser")
}

pub fn is_private_browser_target(target: &[u8], args: &[u8]) -> bool {
    browser(target) && args.split(|&b| b == b' ').any(|word| word == b"--private")
}

fn launch_target(line: &[u8]) -> (Option<&[u8]>, impl Iterator<Item = &[u8]>) {
    let mut words = line.split(|&b| b == b' ').filter(|w| !w.is_empty());
    let target = if words.next() == Some(b"run".as_slice()) {
        let mut next = words.next();
        while matches!(next, Some(b"+net" | b"+pkg")) { next = words.next(); }
        next
    } else { line.split(|&b| b == b' ').find(|w| !w.is_empty()) };
    (target, words)
}

pub fn is_browser_launch_candidate(line: &[u8]) -> bool {
    launch_target(line).0.is_some_and(browser)
}

pub fn is_private_browser_launch(line: &[u8]) -> bool {
    let (target, words) = launch_target(line);
    target.is_some_and(browser) && words.into_iter().any(|word| word == b"--private")
}

#[cfg(test)]
mod tests {
    use super::{is_browser_launch_candidate as candidate, is_private_browser_launch as private};

    #[test]
    fn exact_private_browser_commands_only() {
        assert!(private(b"kaleidoscope --private https://private.invalid/x"));
        assert!(private(b"browser https://private.invalid/x --private"));
        assert!(private(b"run +net +pkg /bin/kaleidoscope --private https://private.invalid/x"));
        assert!(!private(b"run +net kaleidoscope --private-ish https://private.invalid/x"));
        assert!(!private(b"echo run +net kaleidoscope --private"));
        assert!(!private(b"run +net another-app --private https://private.invalid/x"));
        assert!(!private(b"run +net kaleidoscope https://private.invalid/x"));
        assert!(!private(b"run +net kaleidoscope '--private https://private.invalid/x'"));
        assert!(candidate(b"run +net kaleidoscope https://private.invalid/x --pri"));
        assert!(candidate(b"browser https://private.invalid/x --pri"));
        assert!(!candidate(b"echo browser https://private.invalid/x --pri"));
    }
}
