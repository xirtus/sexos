// SPDX-License-Identifier: GPL-3.0-only
//! Compiled scripts kept across runs, so a site's libraries are not compiled again
//! each time Kaleidoscope starts (see `sex_js::code_cache`).
//!
//! Files `Kaleidoscope/code/<site>/` holds one top-level origin's entries: one file per
//! script, named by its key, and `seen` (classic scripts met once, which are written on
//! their next sighting). `<site>` is a hash of the origin, so the folder list is not a
//! readable history. Entries are partitioned by the top-level origin in memory too, so
//! one site cannot time another site's cached scripts. Each file is checked on reading
//! (engine build, key, checksum) and anything else is ignored. Only http(s) pages keep
//! entries; a future private tab must leave `persist` off. Forgetting a site removes its
//! folder.

use alloc::string::String;
use alloc::vec::Vec;
use sex_app::{files, Window};
use sex_js::code_cache::{self, CodeCache};

const DIR: &str = "Kaleidoscope/code";
/// Bytes one site may keep on disk.
const SITE_BYTES: u64 = 24 << 20;
/// Sites kept; beyond this, one is dropped to make room.
const MAX_SITES: usize = 64;
/// Classic-script keys remembered as seen, per site.
const MAX_SEEN: usize = 4096;
/// New entries wait this long (ticks, ms) so a page's burst is written once.
const SAVE_AFTER: u64 = 2000;

#[derive(Default)]
pub struct CodeStore {
    /// The site folder whose entries are in memory and whose new entries are pending.
    site: Option<String>,
    /// When the pending entries go to Files.
    pub due: Option<u64>,
}

fn read_all(window: &mut Window, path: &[u8], most: u64) -> Option<Vec<u8>> {
    let size = files::size(window, path).ok()?;
    if size == 0 || size > most { return None; }
    let mut buf = alloc::vec![0u8; size as usize];
    let n = files::read(window, path, &mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

impl CodeStore {
    /// A top-level page of `url` is about to run its scripts: key new entries by its
    /// origin and bring that site's saved entries into memory.
    pub fn open(&mut self, window: &mut Window, cache: &mut CodeCache, url: &str) {
        let origin = crate::origin_of(url).filter(|_| url.starts_with("http"));
        let site = origin.as_deref().map(code_cache::partition_name);
        if site == self.site { return; }
        // What the previous site wrote is saved under that site, before switching.
        self.save(window, cache);
        cache.set_partition(origin.as_deref().unwrap_or("").as_bytes());
        cache.persist = site.is_some();
        self.site = site;
        let Some(site) = self.site.clone() else { return };
        let dir = alloc::format!("{DIR}/{site}");
        let Ok(list) = files::list(window, dir.as_bytes()) else { return };
        let (mut loaded, mut bytes) = (0usize, 0u64);
        for e in &list {
            if e.dir { continue; }
            if e.name == b"seen" {
                if let Some(b) = read_all(window, alloc::format!("{dir}/seen").as_bytes(), (MAX_SEEN * 32) as u64) {
                    for k in b.chunks_exact(32) { cache.insert_seen(k.try_into().unwrap()); }
                }
                continue;
            }
            let Some(key) = code_cache::key_from_name(&e.name) else { continue };
            if bytes + e.size > SITE_BYTES { continue; }
            let path = alloc::format!("{dir}/{}", code_cache::key_name(&key));
            let Some(saved) = read_all(window, path.as_bytes(), SITE_BYTES) else { continue };
            match code_cache::decode(&key, &saved) {
                Some(code) => { bytes += e.size; loaded += 1; cache.insert_loaded(key, code); }
                // Another engine build's (or a damaged) entry: it can never be used.
                None => { let _ = files::remove(window, path.as_bytes()); }
            }
        }
        web_record!("[web.code_cache.loaded] site={} entries={} bytes={}", site, loaded, bytes);
    }

    /// New entries are waiting: save them a moment from now.
    pub fn later(&mut self, cache: &CodeCache) {
        if cache.has_fresh() && self.due.is_none() { self.due = Some(sex_pdx::get_ticks() + SAVE_AFTER); }
    }

    /// Writes the current site's new entries.
    pub fn save(&mut self, window: &mut Window, cache: &mut CodeCache) {
        self.due = None;
        let (fresh, seen) = (cache.take_fresh(), cache.take_seen());
        let Some(site) = self.site.clone() else { return };
        if fresh.is_empty() && seen.is_empty() { return; }
        let _ = files::mkdir(window, b"Kaleidoscope");
        let _ = files::mkdir(window, DIR.as_bytes());
        let dir = alloc::format!("{DIR}/{site}");
        let mut used = 0u64;
        match files::list(window, dir.as_bytes()) {
            Ok(list) => used = list.iter().filter(|e| !e.dir).map(|e| e.size).sum(),
            Err(_) => {
                self.make_room(window, &site);
                if files::mkdir(window, dir.as_bytes()).is_err() { return; }
            }
        }
        let mut wrote = 0usize;
        for (key, code) in fresh {
            let saved = code_cache::encode(&key, &code);
            if used + saved.len() as u64 > SITE_BYTES { break; }
            let path = alloc::format!("{dir}/{}", code_cache::key_name(&key));
            if files::write(window, path.as_bytes(), &saved).is_err() { break; }
            used += saved.len() as u64;
            wrote += 1;
        }
        if !seen.is_empty() {
            let path = alloc::format!("{dir}/seen");
            let mut all = read_all(window, path.as_bytes(), (MAX_SEEN * 32) as u64).unwrap_or_default();
            all.truncate(all.len() / 32 * 32);
            for k in &seen { all.extend_from_slice(k); }
            let keep = all.len().saturating_sub(MAX_SEEN * 32);
            let _ = files::write(window, path.as_bytes(), &all[keep..]);
        }
        let _ = files::sync(window);
        web_record!("[web.code_cache.saved] site={} entries={} seen={} bytes={}", site, wrote, seen.len(), used);
    }

    /// Before a new site's folder: drop one other site when there are too many.
    fn make_room(&mut self, window: &mut Window, keep: &str) {
        let Ok(sites) = files::list(window, DIR.as_bytes()) else { return };
        let others: Vec<&files::Entry> = sites.iter().filter(|e| e.dir && e.name != keep.as_bytes()).collect();
        if others.len() + 1 <= MAX_SITES { return; }
        // Folders carry no age here; the first one listed goes.
        let name = String::from_utf8_lossy(&others[0].name).into_owned();
        Self::remove_site(window, &name);
    }

    fn remove_site(window: &mut Window, site: &str) {
        let dir = alloc::format!("{DIR}/{site}");
        if let Ok(list) = files::list(window, dir.as_bytes()) {
            for e in list.iter().filter(|e| !e.dir) {
                let _ = files::remove(window, alloc::format!("{dir}/{}", String::from_utf8_lossy(&e.name)).as_bytes());
            }
        }
        let _ = files::remove(window, dir.as_bytes());
    }

    /// The person forgot the site at `url`: its saved scripts go too.
    pub fn forget(&mut self, window: &mut Window, cache: &mut CodeCache, url: &str) {
        let Some(origin) = crate::origin_of(url) else { return };
        let site = code_cache::partition_name(&origin);
        Self::remove_site(window, &site);
        // Memory holds every site's entries under their own keys; start afresh.
        cache.clear();
        if self.site.as_deref() == Some(site.as_str()) { self.site = None; }
    }
}
