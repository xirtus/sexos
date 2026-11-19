// SPDX-License-Identifier: GPL-3.0-only
//! A repository addon's `xbmc.addon.repository` extension: one or more `<dir>`s, each
//! with the address of its `addons.xml`, its checksum and the folder of zips.
//!
//! ```xml
//! <extension point="xbmc.addon.repository">
//!   <dir minversion="19.0.0">
//!     <info compressed="true">https://example.org/omega/addons.xml</info>
//!     <checksum>https://example.org/omega/addons.xml.md5</checksum>
//!     <datadir zip="true">https://example.org/omega/zips/</datadir>
//!     <hashes>sha256</hashes>
//!   </dir>
//! </extension>
//! ```
//!
//! Older repositories put `<info>`, `<checksum>` and `<datadir>` straight in the
//! extension; that is one dir.

use alloc::string::String;
use alloc::vec::Vec;

use crate::addon::Addon;
use crate::check::Hash;
use crate::version;
use crate::xml::Element;

/// The Kodi release whose addon API Qupid's helper offers (Omega, Python 3).
pub const KODI_VERSION: &str = "21.0.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    /// Where `addons.xml` is (already with `.gz` when the repository compresses it).
    pub info: String,
    pub compressed: bool,
    /// Where the checksum of `addons.xml` is, and which hash it holds.
    pub checksum: Option<String>,
    pub checksum_hash: Hash,
    /// The folder of addon zips.
    pub datadir: String,
    pub zip: bool,
    /// The hash every zip has beside it (`<zip>.md5`, `<zip>.sha256`), if any.
    pub hashes: Option<Hash>,
    /// Kodi versions this dir is for; empty when any.
    pub min: String,
    pub max: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Repository {
    pub dirs: Vec<Dir>,
}

fn dir_from(e: &Element) -> Option<Dir> {
    let info_el = e.child("info")?;
    let mut info = info_el.text();
    let compressed = info_el.attr("compressed") == Some("true");
    if compressed && !info.ends_with(".gz") { info.push_str(".gz"); }
    let checksum_el = e.child("checksum");
    let checksum = checksum_el.map(|c| c.text()).filter(|s| !s.is_empty());
    let checksum_hash = checksum_el.and_then(|c| c.attr("verify")).and_then(Hash::from_name).unwrap_or(Hash::Md5);
    let datadir_el = e.child("datadir")?;
    let hashes = match e.child("hashes").map(|h| h.text()) {
        None => None,
        Some(t) if t == "true" => Some(Hash::Md5),
        Some(t) => Hash::from_name(&t),
    };
    Some(Dir {
        info, compressed, checksum, checksum_hash,
        datadir: datadir_el.text(),
        zip: datadir_el.attr("zip") == Some("true"),
        hashes,
        min: String::from(e.attr("minversion").unwrap_or("")),
        max: String::from(e.attr("maxversion").unwrap_or("")),
    })
}

fn join(base: &str, path: &str) -> String {
    let mut s = String::from(base.trim_end_matches('/'));
    s.push('/');
    s.push_str(path.trim_start_matches('/'));
    s
}

impl Repository {
    pub fn from_extension(ext: &Element) -> Repository {
        let mut dirs: Vec<Dir> = ext.children_named("dir").filter_map(dir_from).collect();
        if dirs.is_empty() { dirs.extend(dir_from(ext)); }
        Repository { dirs }
    }

    /// The dirs meant for the Kodi version Qupid offers.
    pub fn dirs_for_us(&self) -> impl Iterator<Item = &Dir> {
        self.dirs.iter().filter(|d| d.suits(KODI_VERSION))
    }
}

impl Dir {
    pub fn suits(&self, kodi: &str) -> bool {
        (self.min.is_empty() || version::at_least(kodi, &self.min)) && (self.max.is_empty() || version::at_least(&self.max, kodi))
    }

    /// Where an addon of this dir's `addons.xml` is downloaded from. None when the
    /// repository serves loose files instead of zips, which Qupid does not install.
    pub fn zip_url(&self, addon: &Addon) -> Option<String> {
        self.zip.then(|| join(&self.datadir, &addon.zip_path()))
    }

    /// Where the zip's checksum is, and which hash, when the repository offers one.
    pub fn zip_hash_url(&self, addon: &Addon) -> Option<(String, Hash)> {
        let h = self.hashes?;
        let mut url = self.zip_url(addon)?;
        url.push('.');
        url.push_str(h.name());
        Some((url, h))
    }

    /// An addon's file from the art folder (icon, fanart) for browsing before install.
    pub fn art_url(&self, addon: &Addon, file: &str) -> String {
        join(&self.datadir, &alloc::format!("{}/{}", addon.id, file))
    }
}
