// SPDX-License-Identifier: GPL-3.0-only
//! Exact-origin JavaScript choices made in Kaleidoscope's site settings.

use alloc::string::String;
use alloc::vec::Vec;

const MAX_ORIGINS: usize = 128;

fn origin(url: &str) -> Option<String> {
    sex_fetch::Url::parse(url).ok().map(|parsed| parsed.origin())
}

#[derive(Clone, Debug, Default)]
pub struct Policy { off: Vec<String> }

impl Policy {
    pub fn from_text(text: &str) -> Self {
        let mut off = Vec::new();
        for line in text.lines().take(MAX_ORIGINS) {
            if let Some(value) = origin(line.trim()) {
                if value == line.trim() && !off.contains(&value) { off.push(value); }
            }
        }
        Self { off }
    }
    pub fn enabled(&self, url: &str) -> bool { origin(url).is_none_or(|site| !self.off.contains(&site)) }
    pub fn toggle(&mut self, url: &str) -> Option<bool> {
        let site = origin(url)?;
        if let Some(i) = self.off.iter().position(|value| *value == site) {
            self.off.remove(i);
            Some(true)
        } else if self.off.len() < MAX_ORIGINS {
            self.off.push(site);
            Some(false)
        } else { None }
    }
    pub fn text(&self) -> String {
        let mut out = String::new();
        for site in &self.off { out.push_str(site); out.push('\n'); }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Policy;

    #[test]
    fn exact_origin_toggle_and_persistent_round_trip() {
        let mut policy = Policy::default();
        assert_eq!(policy.toggle("https://example.test:443/page"), Some(false));
        assert!(!policy.enabled("https://example.test:443/other"));
        assert!(policy.enabled("http://example.test:443/page"));
        assert!(policy.enabled("https://other.example.test:443/page"));
        let saved = Policy::from_text(&policy.text());
        assert!(!saved.enabled("https://example.test:443/again"));
        assert_eq!(policy.toggle("https://example.test:443/page"), Some(true));
        assert!(policy.enabled("https://example.test:443/again"));
        assert!(Policy::from_text("broken\nabout:welcome\n").enabled("https://example.test"));
    }
}
