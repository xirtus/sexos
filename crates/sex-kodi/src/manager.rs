// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's Kodi addon folder, kept the way Kodi keeps it: every installed addon
//! (repositories too) unpacked under `addons/<id>/`, and `installed.txt` listing
//! `id<TAB>version`. The repositories are the installed addons that are repository
//! addons; their `addons.xml` files make the [`Catalogue`].
//!
//! The caller moves the bytes: [`Net`] fetches an address, [`Store`] reads and writes
//! files inside the addon folder Qupid was given. Everything here is tested on the host.
//!
//! - [`Manager::add_repository`]: a repository addon zip, by address (what Kodi calls
//!   "Install from zip file"). It must hold a repository addon.
//! - [`Manager::refresh`]: every repository's `addons.xml`, checked against its
//!   published checksum, into the catalogue.
//! - [`Manager::install`]: an addon and what it needs (`<requires>`), each zip checked
//!   against its published hash when the repository offers one, unpacked and checked
//!   by [`crate::package`].

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sex_archive::{Archive, EntryKind};

use crate::addon::Addon;
use crate::catalogue::{Catalogue, PlanError};
use crate::check;
use crate::package;
use crate::repo::Dir;

/// The largest `addons.xml` read (repositories with thousands of addons run to a few MB).
pub const MAX_INDEX: usize = 32 << 20;
/// The largest addon zip downloaded.
pub const MAX_ZIP: usize = 64 << 20;
/// The largest checksum file read.
const MAX_CHECKSUM: usize = 4096;

/// Fetches `url`; at most `max` bytes. The error is a short reason for the person.
pub trait Net {
    fn get(&mut self, url: &str, max: usize) -> Result<Vec<u8>, String>;
}

/// Files inside the addon folder. Paths are relative and use `/`.
pub trait Store {
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Writes the whole file, making its folders as needed.
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The address could not be fetched.
    Fetch { url: String, why: String },
    /// A zip that is not an addon, or one that fails its checks.
    Package(package::Error),
    /// The zip holds an addon, but not a repository.
    NotRepository(String),
    /// A download does not match the checksum the repository published.
    Checksum(String),
    /// `addons.xml` could not be read.
    Index(String),
    Plan(PlanError),
    /// The repository serves loose files instead of zips.
    NoZip(String),
    Store(String),
}

/// One `<dir>` of an installed repository addon: where its addons come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source { pub repository: String, pub dir: Dir }

/// What [`Manager::refresh`] read from one source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refreshed { pub source: usize, pub addons: Result<usize, Error> }

#[derive(Clone, Debug, Default)]
pub struct Manager {
    /// (id, version) of every installed addon, repositories included.
    pub installed: Vec<(String, String)>,
    pub sources: Vec<Source>,
    pub catalogue: Catalogue,
}

pub const INSTALLED: &str = "installed.txt";

fn addon_folder(id: &str) -> String { alloc::format!("addons/{id}") }

/// The one top folder of a zip that holds `<folder>/addon.xml`.
fn top_folder(zip: &[u8]) -> Option<String> {
    let archive = Archive::open(zip.to_vec()).ok()?;
    archive.entries().iter()
        .filter(|e| matches!(e.kind, EntryKind::File))
        .find_map(|e| e.path.strip_suffix("/addon.xml").filter(|f| !f.is_empty() && !f.contains('/')).map(String::from))
}

/// Undoes gzip around `addons.xml.gz`.
fn gunzip(data: Vec<u8>) -> Result<Vec<u8>, Error> {
    let archive = Archive::open(data).map_err(|e| Error::Index(alloc::format!("{e:?}")))?;
    let mut out = Vec::new();
    if archive.entries().is_empty() { return Err(Error::Index("empty gzip".to_string())); }
    archive.extract(0, &mut out, &mut sex_archive::no_progress).map_err(|e| Error::Index(alloc::format!("{e:?}")))?;
    if out.len() > MAX_INDEX { return Err(Error::Index("too large".to_string())); }
    Ok(out)
}

fn fetch(net: &mut dyn Net, url: &str, max: usize) -> Result<Vec<u8>, Error> {
    net.get(url, max).map_err(|why| Error::Fetch { url: url.to_string(), why })
}

impl Manager {
    /// The folder as it is: installed addons and the repositories among them. The
    /// catalogue stays empty until [`Manager::refresh`].
    pub fn load(store: &mut dyn Store) -> Manager {
        let mut m = Manager::default();
        if let Some(text) = store.read(INSTALLED) {
            for line in String::from_utf8_lossy(&text).lines() {
                let mut parts = line.split('\t');
                if let (Some(id), Some(v)) = (parts.next(), parts.next()) {
                    if !id.is_empty() { m.installed.push((id.to_string(), v.to_string())); }
                }
            }
        }
        for (id, _) in m.installed.clone() {
            let Some(xml) = store.read(&alloc::format!("{}/addon.xml", addon_folder(&id))) else { continue };
            let Ok(addon) = Addon::parse(&String::from_utf8_lossy(&xml)) else { continue };
            m.add_sources(&addon);
        }
        m
    }

    fn add_sources(&mut self, addon: &Addon) {
        let Some(repo) = addon.repository.as_ref() else { return };
        self.sources.retain(|s| s.repository != addon.id);
        for dir in repo.dirs_for_us() { self.sources.push(Source { repository: addon.id.clone(), dir: dir.clone() }); }
    }

    fn save(&self, store: &mut dyn Store) -> Result<(), Error> {
        let mut text = String::new();
        for (id, v) in &self.installed { text.push_str(id); text.push('\t'); text.push_str(v); text.push('\n'); }
        store.write(INSTALLED, text.as_bytes()).map_err(Error::Store)
    }

    fn put(&mut self, store: &mut dyn Store, pkg: &package::Package) -> Result<(), Error> {
        let folder = addon_folder(&pkg.addon.id);
        for (rel, data) in &pkg.files { store.write(&alloc::format!("{folder}/{rel}"), data).map_err(Error::Store)?; }
        match self.installed.iter_mut().find(|(id, _)| *id == pkg.addon.id) {
            Some(e) => e.1 = pkg.addon.version.clone(),
            None => self.installed.push((pkg.addon.id.clone(), pkg.addon.version.clone())),
        }
        self.add_sources(&pkg.addon);
        self.save(store)
    }

    /// Installs the repository addon zip at `url`. Returns the repository's addon.
    pub fn add_repository(&mut self, net: &mut dyn Net, store: &mut dyn Store, url: &str) -> Result<Addon, Error> {
        let zip = fetch(net, url, MAX_ZIP)?;
        let id = top_folder(&zip).ok_or(Error::Package(package::Error::NoAddonXml))?;
        let pkg = package::unpack(zip, &id, None).map_err(Error::Package)?;
        if pkg.addon.repository.as_ref().is_none_or(|r| r.dirs_for_us().next().is_none()) {
            return Err(Error::NotRepository(id));
        }
        self.put(store, &pkg)?;
        Ok(pkg.addon)
    }

    /// Reads every repository's `addons.xml` into a fresh catalogue. A source that
    /// fails is reported and left out; the others still count.
    pub fn refresh(&mut self, net: &mut dyn Net) -> Vec<Refreshed> {
        self.catalogue = Catalogue::new();
        let mut report = Vec::new();
        for k in 0..self.sources.len() {
            let addons = self.read_index(net, k).map(|addons| { let n = addons.len(); self.catalogue.add(k, addons); n });
            report.push(Refreshed { source: k, addons });
        }
        report
    }

    fn read_index(&self, net: &mut dyn Net, k: usize) -> Result<Vec<Addon>, Error> {
        let dir = &self.sources[k].dir;
        let mut data = fetch(net, &dir.info, MAX_INDEX)?;
        if dir.compressed { data = gunzip(data)?; }
        if let Some(url) = dir.checksum.as_deref() {
            let sum = fetch(net, url, MAX_CHECKSUM)?;
            // Kodi checks the uncompressed addons.xml.
            if !check::verify(&data, dir.checksum_hash, &String::from_utf8_lossy(&sum)) { return Err(Error::Checksum(url.to_string())); }
        }
        let text = core::str::from_utf8(&data).map_err(|_| Error::Index("not UTF-8".to_string()))?;
        Addon::parse_list(text).map_err(|e| Error::Index(alloc::format!("{e:?}")))
    }

    /// Installs `id` and what it needs, dependencies first. Returns the addons
    /// installed, in order (none when everything was already there).
    pub fn install(&mut self, net: &mut dyn Net, store: &mut dyn Store, id: &str) -> Result<Vec<(String, String)>, Error> {
        let plan: Vec<(Addon, usize)> = self.catalogue.plan(id, &self.installed).map_err(Error::Plan)?
            .into_iter().map(|e| (e.addon.clone(), e.source)).collect();
        let mut done = Vec::new();
        for (addon, source) in plan {
            let dir = self.sources[source].dir.clone();
            let url = dir.zip_url(&addon).ok_or_else(|| Error::NoZip(addon.id.clone()))?;
            let zip = fetch(net, &url, MAX_ZIP)?;
            if let Some((hash_url, hash)) = dir.zip_hash_url(&addon) {
                let sum = fetch(net, &hash_url, MAX_CHECKSUM)?;
                if !check::verify(&zip, hash, &String::from_utf8_lossy(&sum)) { return Err(Error::Checksum(hash_url)); }
            }
            let pkg = package::unpack(zip, &addon.id, Some(&addon.version)).map_err(Error::Package)?;
            self.put(store, &pkg)?;
            done.push((addon.id, addon.version));
        }
        Ok(done)
    }

    /// The installed version of `id`.
    pub fn version_of(&self, id: &str) -> Option<&str> {
        self.installed.iter().find(|(i, _)| i == id).map(|(_, v)| v.as_str())
    }
}
