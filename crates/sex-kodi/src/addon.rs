// SPDX-License-Identifier: GPL-3.0-only
//! One addon, from its `<addon>` element: an addon's own `addon.xml`, or one entry of a
//! repository's `addons.xml`.

use alloc::string::String;
use alloc::vec::Vec;

use crate::repo::Repository;
use crate::xml::{self, Element};

/// A `<requires><import addon=".." version=".." optional=".."/>` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Require {
    pub id: String,
    /// The lowest version that will do; empty when any will.
    pub version: String,
    pub optional: bool,
}

/// What a plugin offers, from `<provides>`.
pub mod content {
    pub const VIDEO: u8 = 1;
    pub const AUDIO: u8 = 2;
    pub const IMAGE: u8 = 4;
    pub const PROGRAM: u8 = 8;
}

/// What an addon is, from its extension points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `xbmc.python.pluginsource`: browsable menus that end in a playable URL.
    /// `content` bits say for what, `entry` is the Python file Kodi would run.
    Plugin { content: u8, entry: String },
    /// `xbmc.python.script`: a program of its own.
    Script { entry: String },
    /// `xbmc.python.module`: a library other addons import (`script.module.*`).
    /// `lib` is the folder that goes on Python's path.
    Module { lib: String },
    /// `xbmc.addon.repository`: where more addons come from.
    Repository,
    /// `xbmc.service`: runs in the background in Kodi; not run by Qupid.
    Service,
    /// `kodi.resource.*`: images, languages, sounds.
    Resource,
    /// Anything else (skins, scrapers, binary addons...): listed, not installed.
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addon {
    pub id: String,
    pub name: String,
    pub version: String,
    pub provider: String,
    pub requires: Vec<Require>,
    pub kinds: Vec<Kind>,
    /// Present for a repository addon.
    pub repository: Option<Repository>,
    pub summary: String,
    pub description: String,
    /// Paths inside the addon's folder (or repository art folder).
    pub icon: Option<String>,
    pub fanart: Option<String>,
    /// Space-separated platforms (`all`, `linux`, `android`...); empty means all.
    pub platform: String,
    /// Why the author marked it broken or deprecated, when they did.
    pub broken: Option<String>,
    /// Where the zip lives, relative to the repository's data folder, when the
    /// repository says (`<path>` in the metadata); otherwise `id/id-version.zip`.
    pub path: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Xml(xml::Error),
    /// Not an `<addon>`, or one without an id or version.
    NotAnAddon,
}

/// Text in the language Qupid shows: English first, then text with no language, then
/// whatever there is.
fn localized(meta: &Element, name: &str) -> String {
    let all: Vec<&Element> = meta.children_named(name).collect();
    for want in ["en_GB", "en_US", "en"] {
        if let Some(e) = all.iter().find(|e| e.attr("lang") == Some(want)) { return e.text(); }
    }
    all.iter().find(|e| e.attr("lang").is_none()).or(all.first()).map(|e| e.text()).unwrap_or_default()
}

fn kind(ext: &Element) -> Option<Kind> {
    let point = ext.attr("point")?;
    let library = || String::from(ext.attr("library").unwrap_or("default.py"));
    Some(match point {
        "xbmc.python.pluginsource" => {
            let mut content = 0;
            if let Some(p) = ext.child("provides") {
                for word in p.text().split_whitespace() {
                    content |= match word { "video" => content::VIDEO, "audio" => content::AUDIO, "image" => content::IMAGE, "executable" | "game" => content::PROGRAM, _ => 0 };
                }
            }
            if content == 0 { content = content::PROGRAM; }
            Kind::Plugin { content, entry: library() }
        }
        "xbmc.python.script" => Kind::Script { entry: library() },
        "xbmc.python.module" => Kind::Module { lib: String::from(ext.attr("library").unwrap_or("lib")) },
        "xbmc.addon.repository" => Kind::Repository,
        "xbmc.service" => Kind::Service,
        "xbmc.addon.metadata" | "kodi.addon.metadata" => return None,
        p if p.starts_with("kodi.resource") => Kind::Resource,
        p => Kind::Other(String::from(p)),
    })
}

impl Addon {
    /// Reads an `<addon>` element.
    pub fn from_element(el: &Element) -> Result<Addon, Error> {
        if el.name != "addon" { return Err(Error::NotAnAddon); }
        let id = String::from(el.attr("id").unwrap_or("").trim());
        let version = String::from(el.attr("version").unwrap_or("").trim());
        if id.is_empty() || version.is_empty() { return Err(Error::NotAnAddon); }
        let mut a = Addon {
            name: String::from(el.attr("name").unwrap_or(&id)),
            provider: String::from(el.attr("provider-name").unwrap_or("")),
            id, version,
            requires: Vec::new(), kinds: Vec::new(), repository: None,
            summary: String::new(), description: String::new(), icon: None, fanart: None,
            platform: String::new(), broken: None, path: None, size: None,
        };
        // Old addons list imports straight under <addon>, newer ones under <requires>.
        let imports = el.children_named("requires").flat_map(|r| r.children_named("import")).chain(el.children_named("import"));
        for imp in imports {
            let Some(id) = imp.attr("addon") else { continue };
            a.requires.push(Require {
                id: String::from(id.trim()),
                version: String::from(imp.attr("version").unwrap_or("").trim()),
                optional: imp.attr("optional") == Some("true"),
            });
        }
        for ext in el.children_named("extension") {
            match ext.attr("point") {
                Some("xbmc.addon.metadata") | Some("kodi.addon.metadata") => {
                    a.summary = localized(ext, "summary");
                    a.description = localized(ext, "description");
                    a.platform = ext.child("platform").map(|p| p.text()).unwrap_or_default();
                    if let Some(assets) = ext.child("assets") {
                        a.icon = assets.child("icon").map(|e| e.text()).filter(|s| !s.is_empty());
                        a.fanart = assets.child("fanart").map(|e| e.text()).filter(|s| !s.is_empty());
                    }
                    if let Some(b) = ext.child("broken") { a.broken = Some(b.text()); }
                    if let Some(l) = ext.child("lifecyclestate") {
                        if matches!(l.attr("type"), Some("broken") | Some("deprecated")) {
                            let why = localized(ext, "lifecyclestate");
                            a.broken = Some(if why.is_empty() { String::from(l.attr("type").unwrap_or("broken")) } else { why });
                        }
                    }
                    a.path = ext.child("path").map(|e| e.text()).filter(|s| !s.is_empty());
                    a.size = ext.child("size").and_then(|e| e.text().parse().ok());
                }
                _ => {
                    if let Some(k) = kind(ext) {
                        if k == Kind::Repository { a.repository = Some(Repository::from_extension(ext)); }
                        a.kinds.push(k);
                    }
                }
            }
        }
        Ok(a)
    }

    /// Reads one addon's `addon.xml`.
    pub fn parse(doc: &str) -> Result<Addon, Error> {
        Addon::from_element(&xml::parse(doc).map_err(Error::Xml)?)
    }

    /// Every addon in a repository's `addons.xml`. Entries that are not addons are
    /// left out rather than failing the whole list.
    pub fn parse_list(doc: &str) -> Result<Vec<Addon>, Error> {
        let root = xml::parse(doc).map_err(Error::Xml)?;
        if root.name == "addon" { return Ok(alloc::vec![Addon::from_element(&root)?]); }
        Ok(root.children_named("addon").filter_map(|e| Addon::from_element(e).ok()).collect())
    }

    pub fn is_plugin(&self) -> bool { self.kinds.iter().any(|k| matches!(k, Kind::Plugin { .. })) }
    pub fn is_module(&self) -> bool { self.kinds.iter().any(|k| matches!(k, Kind::Module { .. })) }

    /// What a plugin offers (`content` bits), 0 for other addons.
    pub fn content(&self) -> u8 {
        self.kinds.iter().map(|k| if let Kind::Plugin { content, .. } = k { *content } else { 0 }).fold(0, |a, b| a | b)
    }

    /// Whether Qupid can install it: pure Python or data, and for this platform.
    /// Binary addons and skins are Kodi's own and are not.
    pub fn installable(&self) -> bool {
        let platform_ok = self.platform.split_whitespace().next().is_none()
            || self.platform.split_whitespace().any(|p| p == "all" || p.starts_with("linux"));
        let kinds_ok = !self.kinds.is_empty() && self.kinds.iter().all(|k| !matches!(k, Kind::Other(_)));
        platform_ok && kinds_ok
    }

    /// The zip's path below the repository's data folder.
    pub fn zip_path(&self) -> String {
        match &self.path {
            Some(p) => String::from(p.trim_start_matches('/')),
            None => alloc::format!("{0}/{0}-{1}.zip", self.id, self.version),
        }
    }
}
