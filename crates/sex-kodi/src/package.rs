// SPDX-License-Identifier: GPL-3.0-only
//! An addon zip, checked and unpacked in memory. A zip from a repository must hold
//! only `<id>/...` files (no absolute paths, `..` or links), and its `<id>/addon.xml`
//! must name the id and version the repository promised. Limits keep a hostile zip
//! from filling memory.

use alloc::string::String;
use alloc::vec::Vec;

use sex_archive::{Archive, EntryKind};

use crate::addon::Addon;

/// Most bytes an addon may unpack to.
pub const MAX_BYTES: u64 = 256 << 20;
/// Most files an addon may hold.
pub const MAX_FILES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a zip, or a damaged one.
    Archive(String),
    /// A member outside `<id>/`, a link, or an unsafe path.
    Outside(String),
    Encrypted(String),
    TooBig,
    /// No `<id>/addon.xml`, or one that does not parse.
    NoAddonXml,
    /// `addon.xml` names another id or version than the repository did.
    NotPromised { id: String, version: String },
}

/// An unpacked addon: its own description and its files, paths relative to its folder.
#[derive(Clone, Debug)]
pub struct Package {
    pub addon: Addon,
    pub files: Vec<(String, Vec<u8>)>,
}

fn archive_error(e: sex_archive::Error) -> Error {
    Error::Archive(alloc::format!("{e:?}"))
}

/// Opens and checks the zip of addon `id`. With `version`, the zip must be that version.
pub fn unpack(zip: Vec<u8>, id: &str, version: Option<&str>) -> Result<Package, Error> {
    let archive = Archive::open(zip).map_err(archive_error)?;
    let prefix = alloc::format!("{id}/");
    let mut total = 0u64;
    let mut files = Vec::new();
    let entries = archive.entries();
    if entries.len() > MAX_FILES { return Err(Error::TooBig); }
    for (index, e) in entries.iter().enumerate() {
        if !e.safe || !(e.path == id || e.path.starts_with(&prefix)) { return Err(Error::Outside(e.raw_path.clone())); }
        match e.kind {
            EntryKind::Dir => continue,
            EntryKind::File => {}
            _ => return Err(Error::Outside(e.raw_path.clone())),
        }
        if e.encrypted { return Err(Error::Encrypted(e.path.clone())); }
        total = total.saturating_add(e.size);
        if total > MAX_BYTES { return Err(Error::TooBig); }
        let mut data = Vec::with_capacity(e.size.min(MAX_BYTES) as usize);
        archive.extract(index, &mut data, &mut sex_archive::no_progress).map_err(archive_error)?;
        files.push((String::from(&e.path[prefix.len()..]), data));
    }
    let xml = files.iter().find(|(p, _)| p == "addon.xml").ok_or(Error::NoAddonXml)?;
    let text = core::str::from_utf8(&xml.1).map_err(|_| Error::NoAddonXml)?;
    let addon = Addon::parse(text).map_err(|_| Error::NoAddonXml)?;
    if addon.id != id || version.is_some_and(|v| v != addon.version) {
        return Err(Error::NotPromised { id: addon.id, version: addon.version });
    }
    Ok(Package { addon, files })
}
