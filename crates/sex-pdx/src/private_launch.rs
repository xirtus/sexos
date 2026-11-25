//! Recognize a private Kaleidoscope command before terminal history captures it.

fn browser(word: &[u8]) -> bool {
    let name = word.rsplit(|&b| b == b'/').next().unwrap_or(word);
    matches!(name, b"kaleidoscope" | b"web" | b"browser")
}

pub fn is_private_browser_target(target: &[u8], args: &[u8]) -> bool {
    browser(target) && args.split(|&b| b == b' ').any(|word| word == b"--private")
}

pub fn is_private_browser_launch(line: &[u8]) -> bool {
    let mut words = line.split(|&b| b == b' ').filter(|w| !w.is_empty());
    let Some(first) = words.next() else { return false };
    let target = if first == b"run" {
        let mut next = words.next();
        while matches!(next, Some(b"+net" | b"+pkg")) { next = words.next(); }
        next
    } else { Some(first) };
    target.is_some_and(browser) && words.any(|word| word == b"--private")
}

#[cfg(test)]
mod tests {
    use super::is_private_browser_launch as private;

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
    }
}
