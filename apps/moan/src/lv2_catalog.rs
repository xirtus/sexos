//! LV2 bundle discovery for Moan's Instruments/Plugins pane: pure-Rust Turtle
//! manifest parsing, no native plugin code is loaded.
use alloc::{format, string::String, vec::Vec};
pub use moan_plugin::lv2::{
    Error as Lv2Error, Lv2Manifest, Lv2PluginInfo, Lv2Port, Lv2PortKind, Lv2Preset,
};

/// UI-friendly descriptor for one LV2 control input port.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Lv2ControlUi {
    pub index: u32,
    pub symbol: String,
    pub name: String,
    pub default_value: f32,
    pub min_value: Option<f32>,
    pub max_value: Option<f32>,
    pub is_integer: bool,
    pub is_toggled: bool,
    pub is_logarithmic: bool,
}

impl From<&Lv2Port> for Lv2ControlUi {
    fn from(port: &Lv2Port) -> Self {
        Self {
            index: port.index,
            symbol: port.symbol.clone(),
            name: port.name.clone(),
            default_value: port.default_value,
            min_value: port.min_value,
            max_value: port.max_value,
            is_integer: port.is_integer,
            is_toggled: port.is_toggled,
            is_logarithmic: port.is_logarithmic,
        }
    }
}

/// Scanned LV2 bundle entry backed by pure-Rust Turtle (`.ttl`) manifest parsing.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Lv2DiscoveredBundle {
    pub bundle_path: String,
    pub manifest: Lv2Manifest,
}

/// Safe pure-Rust LV2 bundle manifest discovery catalog for Moan's Instruments/Plugins pane.
/// Parsing `.ttl` manifests never loads or executes any native plugin shared object.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lv2Catalog {
    pub bundles: Vec<Lv2DiscoveredBundle>,
}

#[allow(dead_code)]
impl Lv2Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `manifest.ttl` and optional `rdfs:seeAlso` `.ttl` documents for `bundle_path`,
    /// replacing any previous scan for the same bundle path. Returns discovered plugin count.
    pub fn discover_ttl_bundle(
        &mut self,
        bundle_path: &str,
        manifest_ttl: &str,
        extra_ttls: &[&str],
    ) -> Result<usize, Lv2Error> {
        let manifest = Lv2Manifest::parse_bundle(manifest_ttl, extra_ttls)?;
        let count = manifest.plugins.len();
        if let Some(existing) = self
            .bundles
            .iter_mut()
            .find(|b| b.bundle_path == bundle_path)
        {
            existing.manifest = manifest;
        } else {
            self.bundles.push(Lv2DiscoveredBundle {
                bundle_path: String::from(bundle_path),
                manifest,
            });
        }
        Ok(count)
    }

    /// Parses a single combined `.ttl` document for `bundle_path`.
    pub fn discover_single_ttl(
        &mut self,
        bundle_path: &str,
        ttl: &str,
    ) -> Result<usize, Lv2Error> {
        self.discover_ttl_bundle(bundle_path, ttl, &[])
    }

    pub fn plugins(&self) -> Vec<(&str, &Lv2PluginInfo)> {
        let mut out = Vec::new();
        for bundle in &self.bundles {
            for plugin in &bundle.manifest.plugins {
                out.push((bundle.bundle_path.as_str(), plugin));
            }
        }
        out
    }

    pub fn stereo_plugins(&self) -> Vec<(&str, &Lv2PluginInfo)> {
        self.plugins()
            .into_iter()
            .filter(|(_, info)| info.is_stereo_capable())
            .collect()
    }

    pub fn find_plugin(&self, uri: &str) -> Option<(&str, &Lv2PluginInfo)> {
        self.plugins()
            .into_iter()
            .find(|(_, info)| info.uri == uri)
    }

    pub fn plugin_count(&self) -> usize {
        self.bundles.iter().map(|b| b.manifest.plugins.len()).sum()
    }

    pub fn control_ui(info: &Lv2PluginInfo) -> Vec<Lv2ControlUi> {
        info.control_inputs().map(Lv2ControlUi::from).collect()
    }

    pub fn summary_text(&self) -> String {
        let total = self.plugin_count();
        if total == 0 {
            return String::from("LV2: no bundles scanned");
        }
        let mut names: Vec<&str> = Vec::new();
        for (_, info) in self.plugins().into_iter().take(3) {
            names.push(info.name.as_str());
        }
        let suffix = if total > 3 { ", ..." } else { "" };
        format!(
            "LV2: {} bundle(s) · {} plugin(s) ({}{})",
            self.bundles.len(),
            total,
            names.join(", "),
            suffix
        )
    }
}
