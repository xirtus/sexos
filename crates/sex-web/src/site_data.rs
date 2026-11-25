// SPDX-License-Identifier: GPL-3.0-only
//! Browser-owned site data actions. The caller supplies the shown tab's jar and
//! storage, so a container or private tab cannot clear another box by accident.

use sex_fetch::{Jar, Url};
use sex_js::storage::WebStorage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part { Cookies, Storage }

/// Clear one kind of data for the exact site shown in browser site settings.
/// Cookies include parent-domain cookies that could reach the host; storage is
/// scoped to its full origin (scheme, host and port).
pub fn clear(jar: &mut Jar, storage: &mut WebStorage, url: &str, part: Part) -> Option<usize> {
    let site = Url::parse(url).ok()?;
    Some(match part {
        Part::Cookies => {
            let before = jar.count_site(&site.host);
            jar.clear_site(&site.host);
            before - jar.count_site(&site.host)
        }
        Part::Storage => {
            let origin = site.origin();
            let before = storage.len(&origin);
            storage.clear(&origin);
            before - storage.len(&origin)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selective_clear_keeps_other_data_sites_and_boxes() {
        let target = Url::parse("https://news.example.test:8443/read").unwrap();
        let other = Url::parse("https://other.example.test/").unwrap();
        let mut own = Jar::new();
        own.store(&target, "session=own; Domain=example.test; Path=/", 1);
        own.store(&other, "other=keep; Path=/", 1);
        let mut container = Jar::new();
        container.store(&target, "session=container; Path=/", 1);
        let mut container_storage = WebStorage::default();
        container_storage.set(&target.origin(), "container-draft", "keep");
        let mut storage = WebStorage::default();
        storage.set(&target.origin(), "draft", "keep until storage clear");
        storage.set("https://news.example.test", "other-port", "keep");
        storage.set(&other.origin(), "other-site", "keep");
        assert_eq!(clear(&mut own, &mut storage, "https://news.example.test:8443/read", Part::Cookies), Some(1));
        assert_eq!(own.count_site(&target.host), 0);
        assert_eq!(own.count_site(&other.host), 1);
        assert_eq!(container.count_site(&target.host), 1);
        assert_eq!(storage.get(&target.origin(), "draft"), Some("keep until storage clear"));
        assert_eq!(clear(&mut own, &mut storage, "https://news.example.test:8443/read", Part::Storage), Some(1));
        assert_eq!(storage.len(&target.origin()), 0);
        assert_eq!(storage.get("https://news.example.test", "other-port"), Some("keep"));
        assert_eq!(storage.get(&other.origin(), "other-site"), Some("keep"));
        assert_eq!(container_storage.get(&target.origin(), "container-draft"), Some("keep"));
        assert_eq!(clear(&mut own, &mut storage, "about:downloads", Part::Cookies), None);
    }
}
