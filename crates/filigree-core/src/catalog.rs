// SPDX-License-Identifier: GPL-3.0-only
//! Font catalog, legacy family folding, duplicate detection, smart collections,
//! and non-destructive `/system/fonts/active.toml` activation manifest.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use sex_font::{inspect_font, FontClass, FontMetadata, UnicodeCoverage};

use crate::validator::{encode_hex, sha256_digest};

/// Default filesystem path for the non-destructive activation manifest.
pub const DEFAULT_MANIFEST_PATH: &str = "/system/fonts/active.toml";

/// One cataloged font face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontEntry {
    pub id: String,
    pub path: String,
    pub face_index: u32,
    pub sha256: [u8; 32],
    pub sha256_hex: String,
    pub metadata: FontMetadata,
    pub active: bool,
    /// User rating (`0..=5`).
    pub stars: u8,
    pub tags: Vec<String>,
    pub note: Option<String>,
}

impl FontEntry {
    /// Inspect `bytes` at `face_index` and build a catalog entry.
    pub fn from_bytes(path: impl Into<String>, face_index: u32, bytes: &[u8]) -> Option<Self> {
        let metadata = inspect_font(bytes, face_index)?;
        let sha256 = sha256_digest(bytes);
        Some(Self::from_metadata(path, face_index, sha256, metadata))
    }

    /// Build a catalog entry from pre-computed digest and metadata.
    pub fn from_metadata(
        path: impl Into<String>,
        face_index: u32,
        sha256: [u8; 32],
        metadata: FontMetadata,
    ) -> Self {
        let sha256_hex = encode_hex(&sha256);
        let id = format!("{sha256_hex}#{face_index}");
        Self {
            id,
            path: path.into(),
            face_index,
            sha256,
            sha256_hex,
            metadata,
            active: true,
            stars: 0,
            tags: Vec::new(),
            note: None,
        }
    }

    /// Canonical family name with weight-split legacy suffixes folded (e.g. `"Foo SemiBold"` -> `"Foo"`).
    pub fn canonical_family(&self) -> String {
        canonical_family_name(&self.metadata.family)
    }

    /// Parse numeric version tuple from `metadata.version` for collision comparison.
    pub fn parsed_version(&self) -> Option<FontVersion> {
        self.metadata
            .version
            .as_deref()
            .and_then(parse_font_version)
    }

    pub fn set_stars(&mut self, stars: u8) {
        self.stars = stars.min(5);
    }

    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let t = tag.into();
        let trimmed = t.trim();
        if trimmed.is_empty() {
            return;
        }
        if !self.tags.iter().any(|existing| existing.eq_ignore_ascii_case(trimmed)) {
            self.tags.push(String::from(trimmed));
        }
    }

    pub fn remove_tag(&mut self, tag: &str) -> bool {
        let before = self.tags.len();
        self.tags.retain(|t| !t.eq_ignore_ascii_case(tag));
        self.tags.len() != before
    }

    pub fn set_note(&mut self, note: Option<String>) {
        self.note = note.and_then(|n| {
            let t = n.trim();
            if t.is_empty() {
                None
            } else {
                Some(String::from(t))
            }
        });
    }
}

/// Ordered style/weight suffixes folded out of legacy `name` ID 1 family strings.
/// Slant tokens are listed first so compound names like `"Foo Extra Bold Italic"`
/// strip `"Italic"` and then `"Extra Bold"`.
const LEGACY_STYLE_SUFFIXES: &[&str] = &[
    "italic",
    "oblique",
    "extra light",
    "extralight",
    "ultra light",
    "ultralight",
    "semi bold",
    "semibold",
    "demi bold",
    "demibold",
    "extra bold",
    "extrabold",
    "ultra bold",
    "ultrabold",
    "thin",
    "hairline",
    "light",
    "book",
    "regular",
    "normal",
    "medium",
    "bold",
    "heavy",
    "black",
];

/// Fold weight/slant-split legacy family names (e.g. `"Foo Light"`, `"Foo SemiBold"`)
/// into their canonical family name (`"Foo"`).
pub fn canonical_family_name(raw_family: &str) -> String {
    let mut cur = raw_family.trim();
    loop {
        let lower = cur.to_ascii_lowercase();
        let mut stripped = None;
        for suffix in LEGACY_STYLE_SUFFIXES {
            if lower.ends_with(suffix) && cur.len() > suffix.len() {
                let split_at = cur.len() - suffix.len();
                let prefix = &cur[..split_at];
                if prefix.ends_with([' ', '-', '_']) {
                    let candidate = prefix.trim_end_matches([' ', '-', '_']).trim();
                    if !candidate.is_empty() {
                        stripped = Some(candidate);
                        break;
                    }
                }
            }
        }
        match stripped {
            Some(next) => cur = next,
            None => break,
        }
    }
    String::from(cur)
}

/// A unified font family grouping all weight/width/italic variants under one name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedFamily {
    pub name: String,
    pub faces: Vec<FontEntry>,
}

impl UnifiedFamily {
    pub fn active_count(&self) -> usize {
        self.faces.iter().filter(|f| f.active).count()
    }

    pub fn is_any_active(&self) -> bool {
        self.faces.iter().any(|f| f.active)
    }

    /// Preferred representative face (Regular 400 upright if present, else closest weight).
    pub fn primary_face(&self) -> Option<&FontEntry> {
        self.faces.iter().min_by_key(|f| {
            let slant_penalty = if f.metadata.is_italic || f.metadata.is_oblique {
                1000u32
            } else {
                0u32
            };
            let weight_diff = (f.metadata.weight as i32 - 400).unsigned_abs();
            (slant_penalty + weight_diff, f.metadata.width_class)
        })
    }

    pub fn classification(&self) -> FontClass {
        self.primary_face()
            .map(|f| f.metadata.classification())
            .unwrap_or(FontClass::Unknown)
    }
}

/// Group entries into unified families, folding weight-split legacy names and sorting faces.
pub fn unify_families(entries: &[FontEntry]) -> Vec<UnifiedFamily> {
    let mut groups: BTreeMap<String, (String, Vec<FontEntry>)> = BTreeMap::new();
    for entry in entries {
        let canon = entry.canonical_family();
        let key = canon.to_ascii_lowercase();
        let slot = groups.entry(key).or_insert_with(|| (canon, Vec::new()));
        slot.1.push(entry.clone());
    }

    let mut families: Vec<UnifiedFamily> = groups
        .into_values()
        .map(|(name, mut faces)| {
            faces.sort_by(|a, b| {
                (
                    a.metadata.weight,
                    a.metadata.is_italic || a.metadata.is_oblique,
                    a.metadata.width_class,
                    a.metadata.subfamily.as_str(),
                    a.path.as_str(),
                )
                    .cmp(&(
                        b.metadata.weight,
                        b.metadata.is_italic || b.metadata.is_oblique,
                        b.metadata.width_class,
                        b.metadata.subfamily.as_str(),
                        b.path.as_str(),
                    ))
            });
            UnifiedFamily { name, faces }
        })
        .collect();

    families.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    families
}

/// Parsed numeric version components for comparing shadowed font releases.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontVersion {
    pub parts: Vec<u32>,
}

impl Ord for FontVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let max_len = self.parts.len().max(other.parts.len());
        for i in 0..max_len {
            let a = self.parts.get(i).copied().unwrap_or(0);
            let b = other.parts.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for FontVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parse the primary numeric version tuple from an OpenType `name` ID 5 string.
pub fn parse_font_version(raw: &str) -> Option<FontVersion> {
    let bytes = raw.as_bytes();
    let mut start = None;
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            start = Some(i);
            break;
        }
    }
    let start = start?;
    let mut parts = Vec::new();
    let mut idx = start;
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_digit() {
            break;
        }
        let mut val: u32 = 0;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            val = val.saturating_mul(10).saturating_add((bytes[idx] - b'0') as u32);
            idx += 1;
        }
        parts.push(val);
        if idx < bytes.len() && bytes[idx] == b'.' {
            idx += 1;
        } else {
            break;
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(FontVersion { parts })
    }
}

/// Category of duplicate collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateKind {
    ExactSha256,
    VersionShadowed,
}

/// Recommended tidy-up action for one entry in a [`DuplicateGroup`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TidyVerb {
    Keep,
    Deactivate,
    RemoveDuplicate,
}

impl TidyVerb {
    pub const fn as_str(self) -> &'static str {
        match self {
            TidyVerb::Keep => "keep",
            TidyVerb::Deactivate => "deactivate",
            TidyVerb::RemoveDuplicate => "remove_duplicate",
        }
    }
}

/// Actionable item inside a [`DuplicateGroup`] tidy-up plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TidyAction {
    pub action: TidyVerb,
    pub path: String,
    pub face_index: u32,
    pub reason: String,
}

/// A detected exact or version-shadowed duplicate cluster with a tidy-up plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub kind: DuplicateKind,
    pub family: String,
    pub subfamily: String,
    pub sha256_hex: Option<String>,
    pub keeper_path: String,
    pub shadowed_paths: Vec<String>,
    pub tidy_plan: Vec<TidyAction>,
}

/// Detect exact SHA-256 duplicates and version-shadowed family/style collisions.
pub fn find_duplicates(entries: &[FontEntry]) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();

    // 1. Exact SHA-256 + face_index duplicates.
    let mut by_sha: BTreeMap<(&str, u32), Vec<&FontEntry>> = BTreeMap::new();
    for e in entries {
        by_sha
            .entry((e.sha256_hex.as_str(), e.face_index))
            .or_default()
            .push(e);
    }

    let mut canonical_sha_keepers: Vec<&FontEntry> = Vec::new();
    for ((sha_hex, face_index), mut cluster) in by_sha {
        cluster.sort_by(|a, b| keeper_preference(a, b));
        canonical_sha_keepers.push(cluster[0]);

        if cluster.len() > 1 {
            let keeper = cluster[0];
            let mut shadowed_paths = Vec::with_capacity(cluster.len() - 1);
            let mut tidy_plan = Vec::with_capacity(cluster.len());
            tidy_plan.push(TidyAction {
                action: TidyVerb::Keep,
                path: keeper.path.clone(),
                face_index,
                reason: format!("primary copy for sha256:{sha_hex}"),
            });
            for dup in &cluster[1..] {
                shadowed_paths.push(dup.path.clone());
                tidy_plan.push(TidyAction {
                    action: TidyVerb::RemoveDuplicate,
                    path: dup.path.clone(),
                    face_index,
                    reason: format!("byte-identical duplicate of {}", keeper.path),
                });
            }
            groups.push(DuplicateGroup {
                kind: DuplicateKind::ExactSha256,
                family: keeper.canonical_family(),
                subfamily: keeper.metadata.subfamily.clone(),
                sha256_hex: Some(String::from(sha_hex)),
                keeper_path: keeper.path.clone(),
                shadowed_paths,
                tidy_plan,
            });
        }
    }

    // 2. Version-shadowed family/style collisions among distinct SHA-256 faces.
    let mut by_style: BTreeMap<(String, u16, bool, u8), Vec<&FontEntry>> = BTreeMap::new();
    for e in canonical_sha_keepers {
        let key = (
            e.canonical_family().to_ascii_lowercase(),
            e.metadata.weight,
            e.metadata.is_italic || e.metadata.is_oblique,
            e.metadata.width_class,
        );
        by_style.entry(key).or_default().push(e);
    }

    for (_key, mut cluster) in by_style {
        if cluster.len() <= 1 {
            continue;
        }
        cluster.sort_by(|a, b| version_preference(a, b));
        let keeper = cluster[0];
        let keeper_ver = keeper
            .metadata
            .version
            .as_deref()
            .unwrap_or("unknown");

        let mut shadowed_paths = Vec::with_capacity(cluster.len() - 1);
        let mut tidy_plan = Vec::with_capacity(cluster.len());
        tidy_plan.push(TidyAction {
            action: TidyVerb::Keep,
            path: keeper.path.clone(),
            face_index: keeper.face_index,
            reason: format!("highest version ({keeper_ver})"),
        });
        for shadowed in &cluster[1..] {
            let old_ver = shadowed
                .metadata
                .version
                .as_deref()
                .unwrap_or("unknown");
            shadowed_paths.push(shadowed.path.clone());
            tidy_plan.push(TidyAction {
                action: TidyVerb::Deactivate,
                path: shadowed.path.clone(),
                face_index: shadowed.face_index,
                reason: format!("version {old_ver} shadowed by {keeper_ver} at {}", keeper.path),
            });
        }

        groups.push(DuplicateGroup {
            kind: DuplicateKind::VersionShadowed,
            family: keeper.canonical_family(),
            subfamily: keeper.metadata.subfamily.clone(),
            sha256_hex: None,
            keeper_path: keeper.path.clone(),
            shadowed_paths,
            tidy_plan,
        });
    }

    groups
}

fn is_system_path(path: &str) -> bool {
    path.starts_with("/system/")
}

fn keeper_preference(a: &FontEntry, b: &FontEntry) -> Ordering {
    // Prefer system fonts, then active entries, then shorter path, then lexicographical.
    (!is_system_path(&a.path), !a.active, a.path.len(), a.path.as_str()).cmp(&(
        !is_system_path(&b.path),
        !b.active,
        b.path.len(),
        b.path.as_str(),
    ))
}

fn version_preference(a: &FontEntry, b: &FontEntry) -> Ordering {
    // Higher parsed version first, then keeper_preference.
    let va = a.parsed_version().unwrap_or_default();
    let vb = b.parsed_version().unwrap_or_default();
    vb.cmp(&va).then_with(|| keeper_preference(a, b))
}

/// Parse error for `/system/fonts/active.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "active.toml line {}: {}", self.line, self.message)
    }
}

/// Non-destructive activation manifest (`/system/fonts/active.toml`).
///
/// Tracks deactivated families and face IDs without deleting font files or
/// requiring root privileges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveManifest {
    pub version: u32,
    pub deactivated_families: BTreeSet<String>,
    pub deactivated_faces: BTreeSet<String>,
}

impl Default for ActiveManifest {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveManifest {
    pub fn new() -> Self {
        Self {
            version: 1,
            deactivated_families: BTreeSet::new(),
            deactivated_faces: BTreeSet::new(),
        }
    }

    /// Deactivate all faces belonging to `family` (case-insensitive canonical matching).
    pub fn deactivate_family(&mut self, family: &str) {
        let canon = canonical_family_name(family);
        if !canon.is_empty() {
            self.deactivated_families.insert(canon);
        }
    }

    /// Reactivate `family`. Returns `true` if it was previously deactivated.
    pub fn reactivate_family(&mut self, family: &str) -> bool {
        let canon = canonical_family_name(family);
        let key = self
            .deactivated_families
            .iter()
            .find(|f| f.eq_ignore_ascii_case(&canon) || f.eq_ignore_ascii_case(family))
            .cloned();
        if let Some(k) = key {
            self.deactivated_families.remove(&k)
        } else {
            false
        }
    }

    /// Deactivate a specific face identifier (`id`, `sha256_hex`, or `path`).
    pub fn deactivate_face(&mut self, face_ref: &str) {
        let trimmed = face_ref.trim();
        if !trimmed.is_empty() {
            self.deactivated_faces.insert(String::from(trimmed));
        }
    }

    /// Reactivate a specific face identifier. Returns `true` if it was deactivated.
    pub fn reactivate_face(&mut self, face_ref: &str) -> bool {
        self.deactivated_faces.remove(face_ref.trim())
    }

    /// Whether `entry` is currently active under this manifest.
    pub fn is_entry_active(&self, entry: &FontEntry) -> bool {
        let canon = entry.canonical_family();
        let family_off = self.deactivated_families.iter().any(|f| {
            f.eq_ignore_ascii_case(&canon) || f.eq_ignore_ascii_case(&entry.metadata.family)
        });
        if family_off {
            return false;
        }
        let face_off = self.deactivated_faces.contains(&entry.id)
            || self.deactivated_faces.contains(&entry.sha256_hex)
            || self.deactivated_faces.contains(&entry.path);
        !face_off
    }

    /// Apply activation states to `entries` in place without mutating files.
    pub fn apply(&self, entries: &mut [FontEntry]) {
        for entry in entries {
            entry.active = self.is_entry_active(entry);
        }
    }

    /// Emit deterministic TOML suitable for `/system/fonts/active.toml`.
    pub fn to_toml(&self) -> String {
        let mut out = String::from("# Filigree non-destructive font activation manifest\n");
        out.push_str(&format!("version = {}\n\n", self.version));
        emit_toml_array(&mut out, "deactivated_families", &self.deactivated_families);
        out.push('\n');
        emit_toml_array(&mut out, "deactivated_faces", &self.deactivated_faces);
        out
    }

    /// Parse `/system/fonts/active.toml`.
    pub fn parse_toml(input: &str) -> Result<Self, ManifestError> {
        let mut manifest = Self::new();
        let mut active_array_key: Option<String> = None;
        let mut array_buf = String::new();
        let mut array_start_line = 0usize;

        for (idx, raw_line) in input.lines().enumerate() {
            let line_no = idx + 1;
            let stripped = strip_toml_comment(raw_line).trim();
            if stripped.is_empty() {
                continue;
            }

            if let Some(key) = active_array_key.as_ref() {
                array_buf.push(' ');
                array_buf.push_str(stripped);
                if contains_unquoted_close_bracket(stripped) {
                    let items = parse_toml_string_array(&array_buf, array_start_line)?;
                    assign_manifest_array(&mut manifest, key, items);
                    active_array_key = None;
                    array_buf.clear();
                }
                continue;
            }

            if stripped.starts_with('[') && stripped.ends_with(']') {
                // Optional [activation] section header is tolerated.
                continue;
            }

            let Some((raw_key, raw_val)) = stripped.split_once('=') else {
                return Err(ManifestError {
                    line: line_no,
                    message: format!("expected 'key = value', got '{stripped}'"),
                });
            };
            let key = raw_key.trim();
            let val = raw_val.trim();

            if key == "version" {
                manifest.version = val.parse::<u32>().map_err(|_| ManifestError {
                    line: line_no,
                    message: format!("invalid version integer '{val}'"),
                })?;
            } else if val.starts_with('[') {
                if contains_unquoted_close_bracket(val) {
                    let items = parse_toml_string_array(val, line_no)?;
                    assign_manifest_array(&mut manifest, key, items);
                } else {
                    active_array_key = Some(String::from(key));
                    array_buf.clear();
                    array_buf.push_str(val);
                    array_start_line = line_no;
                }
            } else {
                return Err(ManifestError {
                    line: line_no,
                    message: format!("unsupported value for '{key}'"),
                });
            }
        }

        if active_array_key.is_some() {
            return Err(ManifestError {
                line: array_start_line,
                message: String::from("unterminated TOML array"),
            });
        }

        Ok(manifest)
    }
}

fn assign_manifest_array(manifest: &mut ActiveManifest, key: &str, items: Vec<String>) {
    match key {
        "deactivated_families" => {
            manifest.deactivated_families.extend(items);
        }
        "deactivated_faces" => {
            manifest.deactivated_faces.extend(items);
        }
        _ => {}
    }
}

fn emit_toml_array(out: &mut String, key: &str, items: &BTreeSet<String>) {
    out.push_str(key);
    if items.is_empty() {
        out.push_str(" = []\n");
        return;
    }
    out.push_str(" = [\n");
    for item in items {
        out.push_str("  \"");
        for ch in item.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push_str("\",\n");
    }
    out.push_str("]\n");
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_quotes = false;
    let mut escaped = false;
    for (idx, b) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_quotes {
            if b == '\\' {
                escaped = true;
            } else if b == '"' {
                in_quotes = false;
            }
        } else if b == '"' {
            in_quotes = true;
        } else if b == '#' {
            return &line[..idx];
        }
    }
    line
}

fn contains_unquoted_close_bracket(s: &str) -> bool {
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_quotes {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_quotes = false;
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == ']' {
            return true;
        }
    }
    false
}

fn parse_toml_string_array(raw: &str, line: usize) -> Result<Vec<String>, ManifestError> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| ManifestError {
            line,
            message: String::from("malformed TOML array"),
        })?
        .trim();

    let mut items = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_whitespace() || c == ',' {
            chars.next();
            continue;
        }
        if c != '"' {
            return Err(ManifestError {
                line,
                message: format!("expected quoted string in array near '{c}'"),
            });
        }
        chars.next();
        let mut s = String::new();
        let mut closed = false;
        while let Some(ch) = chars.next() {
            if ch == '"' {
                closed = true;
                break;
            }
            if ch == '\\' {
                let esc = chars.next().ok_or_else(|| ManifestError {
                    line,
                    message: String::from("unterminated string escape"),
                })?;
                match esc {
                    '\\' => s.push('\\'),
                    '"' => s.push('"'),
                    'n' => s.push('\n'),
                    'r' => s.push('\r'),
                    't' => s.push('\t'),
                    other => s.push(other),
                }
            } else {
                s.push(ch);
            }
        }
        if !closed {
            return Err(ManifestError {
                line,
                message: String::from("unterminated string literal"),
            });
        }
        items.push(s);
    }
    Ok(items)
}

/// User-curated manual collection of font families or face IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub description: Option<String>,
    pub members: BTreeSet<String>,
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            members: BTreeSet::new(),
        }
    }

    pub fn add_member(&mut self, member: impl Into<String>) {
        self.members.insert(member.into());
    }

    pub fn remove_member(&mut self, member: &str) -> bool {
        self.members.remove(member)
    }

    pub fn contains_entry(&self, entry: &FontEntry) -> bool {
        self.members.contains(&entry.id)
            || self.members.contains(&entry.sha256_hex)
            || self.members.contains(&entry.path)
            || self.members.contains(&entry.canonical_family())
            || self.members.contains(&entry.metadata.family)
    }
}

/// Predicate for dynamic smart collections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmartFilter {
    pub name: String,
    pub classification: Option<FontClass>,
    pub weight_min: Option<u16>,
    pub weight_max: Option<u16>,
    pub monospace_only: bool,
    pub required_scripts: UnicodeCoverage,
    pub min_stars: Option<u8>,
    pub starred_only: bool,
    /// `Some(true)` = deactivated only; `Some(false)` = active only; `None` = all.
    pub deactivated_only: Option<bool>,
    pub required_tag: Option<String>,
}

impl SmartFilter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// True if `entry` satisfies every active filter criterion.
    pub fn matches(&self, entry: &FontEntry) -> bool {
        if let Some(class) = self.classification {
            if entry.metadata.classification() != class {
                return false;
            }
        }
        if let Some(min_w) = self.weight_min {
            if entry.metadata.weight < min_w {
                return false;
            }
        }
        if let Some(max_w) = self.weight_max {
            if entry.metadata.weight > max_w {
                return false;
            }
        }
        if self.monospace_only && !entry.metadata.is_monospace {
            return false;
        }
        if !entry.metadata.coverage.contains_all(self.required_scripts) {
            return false;
        }
        if self.starred_only && entry.stars == 0 {
            return false;
        }
        if let Some(min_s) = self.min_stars {
            if entry.stars < min_s {
                return false;
            }
        }
        if let Some(deactivated) = self.deactivated_only {
            if deactivated && entry.active {
                return false;
            }
            if !deactivated && !entry.active {
                return false;
            }
        }
        if let Some(ref req_tag) = self.required_tag {
            if !entry.tags.iter().any(|t| t.eq_ignore_ascii_case(req_tag)) {
                return false;
            }
        }
        true
    }

    pub fn filter<'a>(&self, entries: &'a [FontEntry]) -> Vec<&'a FontEntry> {
        entries.iter().filter(|e| self.matches(e)).collect()
    }
}

/// Complete stateful Filigree catalog combining faces, manifest, and collections.
#[derive(Clone, Debug, Default)]
pub struct FontCatalog {
    pub entries: Vec<FontEntry>,
    pub manifest: ActiveManifest,
    pub collections: Vec<Collection>,
    pub smart_collections: Vec<SmartFilter>,
}

impl FontCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_entry(&mut self, mut entry: FontEntry) {
        entry.active = self.manifest.is_entry_active(&entry);
        self.entries.push(entry);
    }

    pub fn add_from_bytes(
        &mut self,
        path: impl Into<String>,
        face_index: u32,
        bytes: &[u8],
    ) -> Option<&FontEntry> {
        let entry = FontEntry::from_bytes(path, face_index, bytes)?;
        self.add_entry(entry);
        self.entries.last()
    }

    pub fn apply_manifest(&mut self) {
        self.manifest.apply(&mut self.entries);
    }

    pub fn families(&self) -> Vec<UnifiedFamily> {
        unify_families(&self.entries)
    }

    pub fn find_duplicates(&self) -> Vec<DuplicateGroup> {
        find_duplicates(&self.entries)
    }

    pub fn find_entry_mut(&mut self, query: &str) -> Option<&mut FontEntry> {
        self.entries.iter_mut().find(|e| {
            e.id == query
                || e.sha256_hex == query
                || e.path == query
                || e.canonical_family().eq_ignore_ascii_case(query)
                || e.metadata.family.eq_ignore_ascii_case(query)
        })
    }

    pub fn find_entry(&self, query: &str) -> Option<&FontEntry> {
        self.entries.iter().find(|e| {
            e.id == query
                || e.sha256_hex == query
                || e.path == query
                || e.canonical_family().eq_ignore_ascii_case(query)
                || e.metadata.family.eq_ignore_ascii_case(query)
        })
    }

    pub fn eval_smart(&self, filter: &SmartFilter) -> Vec<&FontEntry> {
        filter.filter(&self.entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sex_font::fallback::NOTO_ARABIC;
    use sex_font::{Panose, ScriptRange, GEIST_MONO, GEIST_SANS, GEIST_SANS_BOLD};

    fn synthetic_entry(
        path: &str,
        family: &str,
        subfamily: &str,
        weight: u16,
        italic: bool,
        version: Option<&str>,
        sha_seed: u8,
    ) -> FontEntry {
        let mut sha = [0u8; 32];
        sha[0] = sha_seed;
        let meta = FontMetadata {
            face_index: 0,
            family: String::from(family),
            subfamily: String::from(subfamily),
            legacy_family: String::from(family),
            legacy_subfamily: String::from(subfamily),
            postscript_name: None,
            designer: None,
            manufacturer: None,
            license: None,
            version: version.map(String::from),
            weight,
            width_class: 5,
            is_italic: italic,
            is_oblique: false,
            is_bold: weight >= 700,
            is_monospace: false,
            is_variable: false,
            panose: Panose::new([2, 11, 5, 3, 0, 0, 0, 0, 0, 0]),
            glyph_count: 100,
            units_per_em: 1000,
            ascender: 800,
            descender: -200,
            line_gap: 0,
            x_height: Some(500),
            cap_height: Some(700),
            coverage: UnicodeCoverage::from_bits(UnicodeCoverage::BASIC_LATIN),
        };
        FontEntry::from_metadata(path, 0, sha, meta)
    }

    #[test]
    fn folds_legacy_weight_split_families_and_sorts_faces() {
        assert_eq!(canonical_family_name("Foo Light"), "Foo");
        assert_eq!(canonical_family_name("Foo SemiBold"), "Foo");
        assert_eq!(canonical_family_name("Foo Extra Bold Italic"), "Foo");
        assert_eq!(canonical_family_name("Geist Mono"), "Geist Mono");
        assert_eq!(canonical_family_name("Light"), "Light");

        let entries = alloc::vec![
            synthetic_entry("/f/Foo-BoldItalic.ttf", "Foo Bold Italic", "Regular", 700, true, None, 1),
            synthetic_entry("/f/Foo-SemiBold.ttf", "Foo SemiBold", "Regular", 600, false, None, 2),
            synthetic_entry("/f/Foo-Light.ttf", "Foo Light", "Regular", 300, false, None, 3),
            synthetic_entry("/f/Foo-Regular.ttf", "Foo", "Regular", 400, false, None, 4),
        ];
        let families = unify_families(&entries);
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name, "Foo");
        let weights: Vec<u16> = families[0].faces.iter().map(|f| f.metadata.weight).collect();
        assert_eq!(weights, alloc::vec![300, 400, 600, 700]);
    }

    #[test]
    fn detects_exact_sha256_duplicates_with_tidy_plan() {
        let e1 = FontEntry::from_bytes("/system/fonts/Geist-Regular.ttf", 0, GEIST_SANS).unwrap();
        let e2 = FontEntry::from_bytes("/home/user/fonts/Geist-Copy.ttf", 0, GEIST_SANS).unwrap();
        let dupes = find_duplicates(&[e2, e1]);
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].kind, DuplicateKind::ExactSha256);
        assert_eq!(dupes[0].keeper_path, "/system/fonts/Geist-Regular.ttf");
        assert_eq!(dupes[0].shadowed_paths, alloc::vec!["/home/user/fonts/Geist-Copy.ttf"]);
        assert_eq!(dupes[0].tidy_plan[0].action, TidyVerb::Keep);
        assert_eq!(dupes[0].tidy_plan[1].action, TidyVerb::RemoveDuplicate);
    }

    #[test]
    fn detects_version_shadowed_duplicates_with_tidy_plan() {
        let old = synthetic_entry(
            "/home/user/fonts/Inter-Regular.ttf",
            "Inter",
            "Regular",
            400,
            false,
            Some("Version 1.8"),
            10,
        );
        let new = synthetic_entry(
            "/system/fonts/Inter-Regular.ttf",
            "Inter",
            "Regular",
            400,
            false,
            Some("Version 2.15"),
            20,
        );
        let dupes = find_duplicates(&[old, new]);
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].kind, DuplicateKind::VersionShadowed);
        assert_eq!(dupes[0].keeper_path, "/system/fonts/Inter-Regular.ttf");
        assert_eq!(dupes[0].tidy_plan[1].action, TidyVerb::Deactivate);
    }

    #[test]
    fn active_manifest_toml_roundtrip_and_non_destructive_toggle() {
        let mut cat = FontCatalog::new();
        cat.add_from_bytes("/system/fonts/Geist-Regular.ttf", 0, GEIST_SANS);
        cat.add_from_bytes("/system/fonts/Geist-Bold.ttf", 0, GEIST_SANS_BOLD);
        cat.add_from_bytes("/system/fonts/GeistMono-Regular.ttf", 0, GEIST_MONO);

        cat.manifest.deactivate_family("Geist");
        let mono_id = cat.entries[2].id.clone();
        cat.manifest.deactivate_face(&mono_id);
        cat.apply_manifest();

        assert!(!cat.entries[0].active);
        assert!(!cat.entries[1].active);
        assert!(!cat.entries[2].active);

        let toml = cat.manifest.to_toml();
        let parsed = ActiveManifest::parse_toml(&toml).expect("parse emitted TOML");
        assert_eq!(parsed, cat.manifest);

        cat.manifest = parsed;
        assert!(cat.manifest.reactivate_family("Geist"));
        cat.apply_manifest();
        assert!(cat.entries[0].active);
        assert!(cat.entries[1].active);
        assert!(!cat.entries[2].active);
    }

    #[test]
    fn user_metadata_manual_collections_and_smart_filters() {
        let mut cat = FontCatalog::new();
        cat.add_from_bytes("/system/fonts/Geist-Regular.ttf", 0, GEIST_SANS);
        cat.add_from_bytes("/system/fonts/Geist-Bold.ttf", 0, GEIST_SANS_BOLD);
        cat.add_from_bytes("/system/fonts/GeistMono-Regular.ttf", 0, GEIST_MONO);
        cat.add_from_bytes("/system/fonts/NotoArabic.ttf", 0, NOTO_ARABIC);

        cat.entries[2].set_stars(9); // clamped to 5
        assert_eq!(cat.entries[2].stars, 5);
        cat.entries[2].add_tag("terminal");
        cat.entries[2].set_note(Some(String::from("Preferred Spindle face")));
        cat.manifest.deactivate_family("Noto Sans Arabic");
        cat.apply_manifest();

        let mut col = Collection::new("Coding");
        col.add_member("Geist Mono");
        assert!(col.contains_entry(&cat.entries[2]));
        assert!(!col.contains_entry(&cat.entries[0]));

        let mut mono_starred = SmartFilter::new("Starred Mono");
        mono_starred.monospace_only = true;
        mono_starred.starred_only = true;
        mono_starred.classification = Some(FontClass::Monospace);
        mono_starred.required_tag = Some(String::from("TERMINAL"));
        let hits = cat.eval_smart(&mono_starred);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/system/fonts/GeistMono-Regular.ttf");

        let mut arabic_inactive = SmartFilter::new("Deactivated Arabic");
        arabic_inactive.required_scripts.insert(ScriptRange::Arabic);
        arabic_inactive.deactivated_only = Some(true);
        let ar_hits = cat.eval_smart(&arabic_inactive);
        assert_eq!(ar_hits.len(), 1);
        assert_eq!(ar_hits[0].path, "/system/fonts/NotoArabic.ttf");

        let mut heavy = SmartFilter::new("Heavy");
        heavy.weight_min = Some(650);
        heavy.weight_max = Some(800);
        assert_eq!(cat.eval_smart(&heavy).len(), 1);
    }
}
