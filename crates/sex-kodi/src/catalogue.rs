// SPDX-License-Identifier: GPL-3.0-only
//! Every repository's addons in one list, and what installing one takes.
//!
//! Each id keeps its newest version that Qupid can run: one whose `xbmc.*`/`kodi.*`
//! imports Qupid provides. [`Catalogue::plan`] then walks `<requires>` (the
//! `script.module.*` libraries most plugins pull in) and returns what to download,
//! dependencies before the addons that import them, leaving out what is already
//! installed at a good enough version.

use alloc::string::String;
use alloc::vec::Vec;

use crate::addon::Addon;
use crate::version;

/// An addon id Qupid itself answers for, as Kodi does for its built-in `xbmc.*` ones.
/// An import is satisfied when `min <= wanted <= version` (Kodi's backwards range).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provided {
    pub id: &'static str,
    pub version: &'static str,
    pub min: &'static str,
}

/// The APIs the Python helper gives addons: Kodi 21's, with Python 3 only. An addon
/// importing `xbmc.python` 2.x is a Python 2 addon and cannot run.
pub const PROVIDED: &[Provided] = &[
    Provided { id: "xbmc.python", version: "3.0.1", min: "3.0.0" },
    Provided { id: "xbmc.gui", version: "5.17.0", min: "5.14.0" },
    Provided { id: "xbmc.addon", version: "21.0.0", min: "12.0.0" },
    Provided { id: "xbmc.core", version: "0.1.0", min: "0.1.0" },
    Provided { id: "xbmc.json", version: "13.0.0", min: "6.0.0" },
    Provided { id: "xbmc.metadata", version: "2.1.0", min: "2.1.0" },
    Provided { id: "kodi.resource", version: "1.0.0", min: "1.0.0" },
];

/// Ids that belong to Kodi itself: never downloaded from a repository.
pub fn is_system(id: &str) -> bool {
    id.starts_with("xbmc.") || id.starts_with("kodi.")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Nothing in any repository has this id.
    Missing { id: String, needed_by: String },
    /// The repositories' newest is older than an import asks for.
    TooOld { id: String, wanted: String, have: String, needed_by: String },
    /// An import Qupid does not offer (a Kodi API at an unsupported version, or one
    /// it lacks), so the addon cannot run here.
    Unsupported { id: String, wanted: String, needed_by: String },
    /// A binary addon, skin or other kind Qupid does not install, or one for
    /// another platform.
    NotInstallable { id: String },
    /// Marked broken by its author.
    Broken { id: String, why: String },
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub addon: Addon,
    /// Which repository (the caller's index) it came from.
    pub source: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    pub entries: Vec<Entry>,
}

fn provided_ok(id: &str, wanted: &str) -> Option<bool> {
    let p = PROVIDED.iter().find(|p| p.id == id)?;
    Some(wanted.is_empty() || (version::at_least(wanted, p.min) && version::at_least(p.version, wanted)))
}

/// Whether every `xbmc.*`/`kodi.*` import the addon needs is offered.
pub fn runs_here(addon: &Addon) -> bool {
    addon.requires.iter().filter(|r| !r.optional && is_system(&r.id)).all(|r| {
        // A built-in Qupid does not list is one it lacks.
        provided_ok(&r.id, &r.version).unwrap_or(false)
    })
}

impl Catalogue {
    pub fn new() -> Catalogue { Catalogue::default() }

    /// Adds one repository's `addons.xml`. For each id the newest version that runs
    /// here wins (or, when none runs here, the newest, so it can be shown as such).
    pub fn add(&mut self, source: usize, addons: Vec<Addon>) {
        for addon in addons {
            match self.entries.iter_mut().find(|e| e.addon.id == addon.id) {
                Some(e) => {
                    let (old_ok, new_ok) = (runs_here(&e.addon), runs_here(&addon));
                    let newer = version::compare(&addon.version, &e.addon.version) == core::cmp::Ordering::Greater;
                    if (new_ok && !old_ok) || (new_ok == old_ok && newer) { *e = Entry { addon, source }; }
                }
                None => self.entries.push(Entry { addon, source }),
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.addon.id == id)
    }

    /// Plugins offering any of the `content` bits, by name.
    pub fn plugins(&self, content: u8) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self.entries.iter().filter(|e| e.addon.content() & content != 0).collect();
        v.sort_by(|a, b| a.addon.name.to_lowercase().cmp(&b.addon.name.to_lowercase()));
        v
    }

    /// Addons whose name, id or summary holds every word of `query` (any case).
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        self.entries.iter().filter(|e| {
            let hay = alloc::format!("{} {} {}", e.addon.name, e.addon.id, e.addon.summary).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        }).collect()
    }

    /// Installed addons (id, version) that a repository has a newer version of.
    pub fn updates<'a>(&'a self, installed: &[(String, String)]) -> Vec<&'a Entry> {
        installed.iter().filter_map(|(id, have)| {
            let e = self.get(id)?;
            (runs_here(&e.addon) && version::compare(&e.addon.version, have) == core::cmp::Ordering::Greater).then_some(e)
        }).collect()
    }

    /// What to download to install `id`, dependencies first. `installed` lists
    /// (id, version) already in the addon folder; those are left out when new enough.
    pub fn plan<'a>(&'a self, id: &str, installed: &[(String, String)]) -> Result<Vec<&'a Entry>, PlanError> {
        let mut out: Vec<&Entry> = Vec::new();
        let mut visiting: Vec<String> = Vec::new();
        self.visit(id, "", "", false, installed, &mut out, &mut visiting)?;
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn visit<'a>(&'a self, id: &str, wanted: &str, by: &str, optional: bool, installed: &[(String, String)], out: &mut Vec<&'a Entry>, visiting: &mut Vec<String>) -> Result<(), PlanError> {
        let err_by = || String::from(by);
        if is_system(id) {
            return match provided_ok(id, wanted) {
                Some(true) => Ok(()),
                _ if optional => Ok(()),
                _ => Err(PlanError::Unsupported { id: id.into(), wanted: wanted.into(), needed_by: err_by() }),
            };
        }
        if let Some((_, have)) = installed.iter().find(|(i, _)| i == id) {
            if wanted.is_empty() || version::at_least(have, wanted) { return Ok(()); }
        }
        // Already planned, or an import cycle (some repositories have them; Kodi
        // installs both): nothing more to add.
        if out.iter().any(|e| e.addon.id == id) || visiting.iter().any(|v| v == id) { return Ok(()); }
        let Some(entry) = self.get(id) else {
            return if optional { Ok(()) } else { Err(PlanError::Missing { id: id.into(), needed_by: err_by() }) };
        };
        let a = &entry.addon;
        if !wanted.is_empty() && !version::at_least(&a.version, wanted) {
            if optional { return Ok(()); }
            return Err(PlanError::TooOld { id: id.into(), wanted: wanted.into(), have: a.version.clone(), needed_by: err_by() });
        }
        if let Some(why) = &a.broken {
            if optional { return Ok(()); }
            return Err(PlanError::Broken { id: id.into(), why: why.clone() });
        }
        if !a.installable() {
            if optional { return Ok(()); }
            return Err(PlanError::NotInstallable { id: id.into() });
        }
        visiting.push(String::from(id));
        for r in &a.requires {
            self.visit(&r.id, &r.version, id, r.optional, installed, out, visiting)?;
        }
        visiting.pop();
        out.push(entry);
        Ok(())
    }
}
