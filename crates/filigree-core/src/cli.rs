// SPDX-License-Identifier: GPL-3.0-only
//! Headless `OmaFont`-parity CLI engine for Filigree (`no_std` + `alloc`).
//!
//! Operates purely on an in-memory [`FontCatalog`] and caller-provided byte slices
//! so both `apps/filigree` and kernel/test harnesses can drive every command.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_font::FontClass;

use crate::catalog::{FontCatalog, FontEntry, SmartFilter, UnifiedFamily};
use crate::specimen::{
    build_comparison, spindle_font_config, spindle_font_config_entry, ComparisonBoard,
    MAX_COMPARE_SLOTS,
};
use crate::validator::{validate_input, ValidatedFontAsset};

/// Structured output returned by [`FiligreeCli::run`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Emitted `/system/fonts/active.toml` when `activate`/`deactivate` modified state.
    pub manifest_toml: Option<String>,
    /// Emitted `/system/settings.toml` when `spindle-hand-off` succeeded.
    pub settings_toml: Option<String>,
}

impl CliOutput {
    fn ok(stdout: String) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            manifest_toml: None,
            settings_toml: None,
        }
    }

    fn err(code: i32, msg: impl Into<String>) -> Self {
        Self {
            exit_code: code,
            stdout: String::new(),
            stderr: msg.into(),
            manifest_toml: None,
            settings_toml: None,
        }
    }
}

/// Headless CLI engine supporting `list`, `info`, `install-check`, `activate`,
/// `deactivate`, `dupes`, `compare`, and `spindle-hand-off` with `--json`.
pub struct FiligreeCli<'a> {
    pub catalog: &'a mut FontCatalog,
    pub font_blobs: BTreeMap<String, &'a [u8]>,
    pub settings_toml: &'a str,
}

impl<'a> FiligreeCli<'a> {
    pub fn new(catalog: &'a mut FontCatalog) -> Self {
        Self {
            catalog,
            font_blobs: BTreeMap::new(),
            settings_toml: "",
        }
    }

    pub fn with_blob(mut self, path_or_id: impl Into<String>, bytes: &'a [u8]) -> Self {
        self.font_blobs.insert(path_or_id.into(), bytes);
        self
    }

    pub fn with_settings_toml(mut self, toml: &'a str) -> Self {
        self.settings_toml = toml;
        self
    }

    /// Execute one CLI invocation. `args` should begin with the subcommand (no argv[0]).
    pub fn run(&mut self, args: &[&str], install_bytes: Option<&[u8]>) -> CliOutput {
        let Some((&cmd, rest)) = args.split_first() else {
            return CliOutput::err(2, "usage: filigree <list|info|install-check|activate|deactivate|dupes|compare|spindle-hand-off> [--json]");
        };

        match cmd {
            "list" => self.cmd_list(rest),
            "info" => self.cmd_info(rest),
            "install-check" => self.cmd_install_check(rest, install_bytes),
            "activate" => self.cmd_set_active(rest, true),
            "deactivate" => self.cmd_set_active(rest, false),
            "dupes" => self.cmd_dupes(rest),
            "compare" => self.cmd_compare(rest),
            "spindle-hand-off" => self.cmd_spindle_handoff(rest),
            other => CliOutput::err(2, format!("unknown subcommand '{other}'")),
        }
    }

    fn cmd_list(&self, args: &[&str]) -> CliOutput {
        let mut json = false;
        let mut filter = SmartFilter::new("cli-list");
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--json" => json = true,
                "--mono" => filter.monospace_only = true,
                "--starred" => filter.starred_only = true,
                "--inactive" => filter.deactivated_only = Some(true),
                "--active" => filter.deactivated_only = Some(false),
                "--class" => {
                    i += 1;
                    let Some(val) = args.get(i) else {
                        return CliOutput::err(2, "missing value for --class");
                    };
                    let Some(cls) = parse_font_class(val) else {
                        return CliOutput::err(2, format!("unknown font class '{val}'"));
                    };
                    filter.classification = Some(cls);
                }
                other => return CliOutput::err(2, format!("unknown flag '{other}'")),
            }
            i += 1;
        }

        let matched: Vec<FontEntry> = self
            .catalog
            .eval_smart(&filter)
            .into_iter()
            .cloned()
            .collect();
        let families = crate::catalog::unify_families(&matched);

        if json {
            CliOutput::ok(format_families_json(&families))
        } else {
            CliOutput::ok(format_families_text(&families))
        }
    }

    fn cmd_info(&self, args: &[&str]) -> CliOutput {
        let mut json = false;
        let mut query: Option<&str> = None;
        let mut face_idx: Option<u32> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--json" => json = true,
                "--face" => {
                    i += 1;
                    let Some(val) = args.get(i).and_then(|s| s.parse::<u32>().ok()) else {
                        return CliOutput::err(2, "invalid --face index");
                    };
                    face_idx = Some(val);
                }
                arg if !arg.starts_with('-') && query.is_none() => query = Some(arg),
                other => return CliOutput::err(2, format!("unexpected argument '{other}'")),
            }
            i += 1;
        }
        let Some(q) = query else {
            return CliOutput::err(2, "usage: filigree info <family|id|path> [--face <n>] [--json]");
        };

        let found = self.catalog.entries.iter().find(|e| {
            let id_match = e.id == q
                || e.sha256_hex == q
                || e.path == q
                || e.canonical_family().eq_ignore_ascii_case(q)
                || e.metadata.family.eq_ignore_ascii_case(q);
            id_match && face_idx.is_none_or(|idx| e.face_index == idx)
        });

        let Some(entry) = found else {
            return CliOutput::err(1, format!("font not found: {q}"));
        };

        if json {
            CliOutput::ok(format_entry_info_json(entry))
        } else {
            CliOutput::ok(format_entry_info_text(entry))
        }
    }

    fn cmd_install_check(&self, args: &[&str], install_bytes: Option<&[u8]>) -> CliOutput {
        let mut json = false;
        let mut source_name: Option<&str> = None;
        for &arg in args {
            if arg == "--json" {
                json = true;
            } else if !arg.starts_with('-') && source_name.is_none() {
                source_name = Some(arg);
            } else {
                return CliOutput::err(2, format!("unexpected argument '{arg}'"));
            }
        }
        let name = source_name.unwrap_or("input.ttf");
        let bytes = match install_bytes.or_else(|| self.font_blobs.get(name).copied()) {
            Some(b) => b,
            None => return CliOutput::err(1, format!("no input bytes provided for '{name}'")),
        };

        match validate_input(bytes, name) {
            Ok(assets) => {
                let out = if json {
                    format_install_check_json(&assets)
                } else {
                    format_install_check_text(&assets)
                };
                CliOutput::ok(out)
            }
            Err(e) => {
                if json {
                    let mut j = String::from("{\"ok\":false,\"error\":");
                    push_json_str(&mut j, &format!("{e}"));
                    j.push('}');
                    CliOutput {
                        exit_code: 1,
                        stdout: j,
                        stderr: format!("{e}"),
                        manifest_toml: None,
                        settings_toml: None,
                    }
                } else {
                    CliOutput::err(1, format!("{e}"))
                }
            }
        }
    }

    fn cmd_set_active(&mut self, args: &[&str], activate: bool) -> CliOutput {
        let mut json = false;
        let mut force_face = false;
        let mut target: Option<&str> = None;
        for &arg in args {
            match arg {
                "--json" => json = true,
                "--face" => force_face = true,
                a if !a.starts_with('-') && target.is_none() => target = Some(a),
                other => return CliOutput::err(2, format!("unexpected argument '{other}'")),
            }
        }
        let Some(q) = target else {
            return CliOutput::err(2, "usage: filigree <activate|deactivate> <family|face-id> [--face] [--json]");
        };

        let is_face_ref = force_face
            || self
                .catalog
                .entries
                .iter()
                .any(|e| e.id == q || e.sha256_hex == q || e.path == q);

        if is_face_ref {
            if activate {
                self.catalog.manifest.reactivate_face(q);
            } else {
                self.catalog.manifest.deactivate_face(q);
            }
        } else if activate {
            self.catalog.manifest.reactivate_family(q);
        } else {
            self.catalog.manifest.deactivate_family(q);
        }

        self.catalog.apply_manifest();
        let toml = self.catalog.manifest.to_toml();
        let verb = if activate { "activated" } else { "deactivated" };
        let scope = if is_face_ref { "face" } else { "family" };

        let stdout = if json {
            let mut j = String::from("{\"ok\":true,\"action\":");
            push_json_str(&mut j, verb);
            j.push_str(",\"scope\":");
            push_json_str(&mut j, scope);
            j.push_str(",\"target\":");
            push_json_str(&mut j, q);
            j.push_str(",\"active_count\":");
            j.push_str(&format!(
                "{}",
                self.catalog.entries.iter().filter(|e| e.active).count()
            ));
            j.push('}');
            j
        } else {
            format!("{verb} {scope} '{q}'\n")
        };

        CliOutput {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            manifest_toml: Some(toml),
            settings_toml: None,
        }
    }

    fn cmd_dupes(&self, args: &[&str]) -> CliOutput {
        let mut json = false;
        for &arg in args {
            if arg == "--json" {
                json = true;
            } else {
                return CliOutput::err(2, format!("unknown flag '{arg}'"));
            }
        }
        let dupes = self.catalog.find_duplicates();
        if json {
            let mut j = String::from("{\"groups\":[");
            for (idx, g) in dupes.iter().enumerate() {
                if idx > 0 {
                    j.push(',');
                }
                j.push_str("{\"kind\":");
                push_json_str(
                    &mut j,
                    match g.kind {
                        crate::catalog::DuplicateKind::ExactSha256 => "exact_sha256",
                        crate::catalog::DuplicateKind::VersionShadowed => "version_shadowed",
                    },
                );
                j.push_str(",\"family\":");
                push_json_str(&mut j, &g.family);
                j.push_str(",\"subfamily\":");
                push_json_str(&mut j, &g.subfamily);
                j.push_str(",\"keeper\":");
                push_json_str(&mut j, &g.keeper_path);
                j.push_str(",\"shadowed\":[");
                for (si, sp) in g.shadowed_paths.iter().enumerate() {
                    if si > 0 {
                        j.push(',');
                    }
                    push_json_str(&mut j, sp);
                }
                j.push_str("],\"tidy_plan\":[");
                for (ti, act) in g.tidy_plan.iter().enumerate() {
                    if ti > 0 {
                        j.push(',');
                    }
                    j.push_str("{\"action\":");
                    push_json_str(&mut j, act.action.as_str());
                    j.push_str(",\"path\":");
                    push_json_str(&mut j, &act.path);
                    j.push_str(",\"face_index\":");
                    j.push_str(&format!("{}", act.face_index));
                    j.push_str(",\"reason\":");
                    push_json_str(&mut j, &act.reason);
                    j.push('}');
                }
                j.push_str("]}");
            }
            j.push_str("]}");
            CliOutput::ok(j)
        } else if dupes.is_empty() {
            CliOutput::ok(String::from("no duplicates detected\n"))
        } else {
            let mut s = String::new();
            for g in &dupes {
                s.push_str(&format!(
                    "[{:?}] {} ({}) keeper={}\n",
                    g.kind, g.family, g.subfamily, g.keeper_path
                ));
                for act in &g.tidy_plan {
                    s.push_str(&format!(
                        "  - {} {}#{} ({})\n",
                        act.action.as_str(),
                        act.path,
                        act.face_index,
                        act.reason
                    ));
                }
            }
            CliOutput::ok(s)
        }
    }

    fn cmd_compare(&self, args: &[&str]) -> CliOutput {
        let mut json = false;
        let mut word = "Hamburgefonstiv";
        let mut size_px = 48u32;
        let mut queries: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--json" => json = true,
                "--word" => {
                    i += 1;
                    let Some(w) = args.get(i) else {
                        return CliOutput::err(2, "missing value for --word");
                    };
                    word = w;
                }
                "--size" => {
                    i += 1;
                    let Some(sz) = args.get(i).and_then(|s| s.parse::<u32>().ok()) else {
                        return CliOutput::err(2, "invalid --size value");
                    };
                    size_px = sz.max(1);
                }
                arg if !arg.starts_with('-') => queries.push(arg),
                other => return CliOutput::err(2, format!("unknown flag '{other}'")),
            }
            i += 1;
        }

        if queries.is_empty() {
            return CliOutput::err(2, "usage: filigree compare <family...> [--word <str>] [--size <px>] [--json]");
        }
        if queries.len() > MAX_COMPARE_SLOTS {
            return CliOutput::err(
                2,
                format!("maximum {MAX_COMPARE_SLOTS} families can be compared at once"),
            );
        }

        let mut resolved: Vec<(&[u8], u32)> = Vec::with_capacity(queries.len());
        for q in queries {
            let Some(entry) = self.catalog.find_entry(q) else {
                return CliOutput::err(1, format!("font not found: {q}"));
            };
            let Some(&blob) = self
                .font_blobs
                .get(&entry.path)
                .or_else(|| self.font_blobs.get(&entry.id))
            else {
                return CliOutput::err(1, format!("no font bytes loaded for '{}'", entry.path));
            };
            resolved.push((blob, entry.face_index));
        }

        let board = match build_comparison(&resolved, word, size_px) {
            Ok(b) => b,
            Err(e) => return CliOutput::err(1, format!("{e}")),
        };

        if json {
            CliOutput::ok(format_compare_json(&board))
        } else {
            CliOutput::ok(format_compare_text(&board))
        }
    }

    fn cmd_spindle_handoff(&self, args: &[&str]) -> CliOutput {
        let mut json = false;
        let mut query: Option<&str> = None;
        for &arg in args {
            match arg {
                "--json" => json = true,
                a if !a.starts_with('-') && query.is_none() => query = Some(a),
                other => return CliOutput::err(2, format!("unexpected argument '{other}'")),
            }
        }
        let Some(q) = query else {
            return CliOutput::err(2, "usage: filigree spindle-hand-off <family|id|path> [--json]");
        };
        let Some(entry) = self.catalog.find_entry(q) else {
            return CliOutput::err(1, format!("font not found: {q}"));
        };

        let cfg = match self
            .font_blobs
            .get(&entry.path)
            .or_else(|| self.font_blobs.get(&entry.id))
        {
            Some(&bytes) => spindle_font_config(entry, bytes),
            None => spindle_font_config_entry(entry),
        };

        let cfg = match cfg {
            Ok(c) => c,
            Err(e) => return CliOutput::err(1, format!("{e}")),
        };

        let updated_settings = cfg.apply_to_settings_toml(self.settings_toml);
        let stdout = if json {
            let mut j = String::from("{\"ok\":true,\"family\":");
            push_json_str(&mut j, &cfg.family);
            j.push_str(",\"path\":");
            push_json_str(&mut j, &cfg.path);
            j.push_str(",\"face_index\":");
            j.push_str(&format!("{}", cfg.face_index));
            j.push_str(",\"units_per_em\":");
            j.push_str(&format!("{}", cfg.units_per_em));
            j.push_str(",\"advance_units\":");
            match cfg.advance_units {
                Some(a) => j.push_str(&format!("{a}")),
                None => j.push_str("null"),
            }
            j.push_str(",\"toml\":");
            push_json_str(&mut j, &updated_settings);
            j.push('}');
            j
        } else {
            updated_settings.clone()
        };

        CliOutput {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            manifest_toml: None,
            settings_toml: Some(updated_settings),
        }
    }
}

fn parse_font_class(s: &str) -> Option<FontClass> {
    match s.to_ascii_lowercase().as_str() {
        "serif" => Some(FontClass::Serif),
        "sans" => Some(FontClass::Sans),
        "script" => Some(FontClass::Script),
        "decorative" => Some(FontClass::Decorative),
        "monospace" | "mono" => Some(FontClass::Monospace),
        _ => None,
    }
}

fn format_families_text(families: &[UnifiedFamily]) -> String {
    let mut out = String::new();
    for fam in families {
        out.push_str(&format!(
            "{} [{}] ({}/{} active)\n",
            fam.name,
            fam.classification().as_str(),
            fam.active_count(),
            fam.faces.len()
        ));
        for face in &fam.faces {
            out.push_str(&format!(
                "  - {} (w={}, {}, stars={}, {})\n",
                face.metadata.subfamily,
                face.metadata.weight,
                if face.active { "active" } else { "inactive" },
                face.stars,
                face.path
            ));
        }
    }
    out
}

fn format_families_json(families: &[UnifiedFamily]) -> String {
    let mut j = String::from("{\"families\":[");
    for (fi, fam) in families.iter().enumerate() {
        if fi > 0 {
            j.push(',');
        }
        j.push_str("{\"name\":");
        push_json_str(&mut j, &fam.name);
        j.push_str(",\"class\":");
        push_json_str(&mut j, fam.classification().as_str());
        j.push_str(",\"faces\":[");
        for (i, face) in fam.faces.iter().enumerate() {
            if i > 0 {
                j.push(',');
            }
            j.push_str("{\"id\":");
            push_json_str(&mut j, &face.id);
            j.push_str(",\"path\":");
            push_json_str(&mut j, &face.path);
            j.push_str(",\"subfamily\":");
            push_json_str(&mut j, &face.metadata.subfamily);
            j.push_str(&format!(
                ",\"weight\":{},\"active\":{},\"stars\":{},\"monospace\":{}}}",
                face.metadata.weight, face.active, face.stars, face.metadata.is_monospace
            ));
        }
        j.push_str("]}");
    }
    j.push_str("]}");
    j
}

fn format_entry_info_text(e: &FontEntry) -> String {
    let m = &e.metadata;
    let scripts = m.coverage.script_names().join(", ");
    format!(
        "Family: {}\nSubfamily: {}\nPath: {}#{}\nSHA-256: {}\nWeight: {} Width: {} Class: {}\nGlyphs: {} UPEM: {}\nActive: {} Stars: {}\nScripts: {}\n",
        m.family,
        m.subfamily,
        e.path,
        e.face_index,
        e.sha256_hex,
        m.weight,
        m.width_class,
        m.classification().as_str(),
        m.glyph_count,
        m.units_per_em,
        e.active,
        e.stars,
        scripts
    )
}

fn format_entry_info_json(e: &FontEntry) -> String {
    let m = &e.metadata;
    let mut j = String::from("{");
    j.push_str("\"id\":");
    push_json_str(&mut j, &e.id);
    j.push_str(",\"family\":");
    push_json_str(&mut j, &m.family);
    j.push_str(",\"subfamily\":");
    push_json_str(&mut j, &m.subfamily);
    j.push_str(",\"path\":");
    push_json_str(&mut j, &e.path);
    j.push_str(",\"sha256\":");
    push_json_str(&mut j, &e.sha256_hex);
    j.push_str(",\"class\":");
    push_json_str(&mut j, m.classification().as_str());
    j.push_str(&format!(
        ",\"weight\":{},\"width_class\":{},\"glyph_count\":{},\"units_per_em\":{},\"is_italic\":{},\"is_bold\":{},\"is_monospace\":{},\"is_variable\":{},\"active\":{},\"stars\":{}",
        m.weight,
        m.width_class,
        m.glyph_count,
        m.units_per_em,
        m.is_italic,
        m.is_bold,
        m.is_monospace,
        m.is_variable,
        e.active,
        e.stars
    ));
    j.push_str(",\"scripts\":[");
    for (idx, name) in m.coverage.script_names().iter().enumerate() {
        if idx > 0 {
            j.push(',');
        }
        push_json_str(&mut j, name);
    }
    j.push_str("]}");
    j
}

fn format_install_check_text(assets: &[ValidatedFontAsset]) -> String {
    let mut out = String::new();
    for asset in assets {
        let p = &asset.preview;
        out.push_str(&format!(
            "OK {} [{}] {} bytes sha256={} ({} faces)\n",
            p.source_name,
            p.format.as_str(),
            p.byte_len,
            p.sha256_hex,
            p.faces.len()
        ));
        for f in &p.faces {
            out.push_str(&format!(
                "  face #{}: {} - {} (w={}, glyphs={})\n",
                f.face_index,
                f.metadata.family,
                f.metadata.subfamily,
                f.metadata.weight,
                f.metadata.glyph_count
            ));
        }
    }
    out
}

fn format_install_check_json(assets: &[ValidatedFontAsset]) -> String {
    let mut j = String::from("{\"ok\":true,\"assets\":[");
    for (ai, asset) in assets.iter().enumerate() {
        if ai > 0 {
            j.push(',');
        }
        let p = &asset.preview;
        j.push_str("{\"source_name\":");
        push_json_str(&mut j, &p.source_name);
        j.push_str(",\"format\":");
        push_json_str(&mut j, p.format.as_str());
        j.push_str(",\"sha256\":");
        push_json_str(&mut j, &p.sha256_hex);
        j.push_str(&format!(",\"byte_len\":{},\"faces\":[", p.byte_len));
        for (fi, f) in p.faces.iter().enumerate() {
            if fi > 0 {
                j.push(',');
            }
            j.push_str(&format!("{{\"face_index\":{},\"family\":", f.face_index));
            push_json_str(&mut j, &f.metadata.family);
            j.push_str(",\"subfamily\":");
            push_json_str(&mut j, &f.metadata.subfamily);
            j.push_str(&format!(
                ",\"weight\":{},\"glyph_count\":{}}}",
                f.metadata.weight, f.metadata.glyph_count
            ));
        }
        j.push_str("]}");
    }
    j.push_str("]}");
    j
}

fn format_compare_text(board: &ComparisonBoard) -> String {
    let mut out = format!(
        "Compare '{}' at {}px (shared baseline y={}px, height={}px)\n",
        board.sample_word, board.size_px, board.shared_baseline_y_px, board.total_height_px
    );
    for s in &board.slots {
        out.push_str(&format!(
            "  [{}] {} ({}) xh={}px cap={}px asc={}px desc={}px width={}px missing={}\n",
            s.slot_index,
            s.family,
            s.subfamily,
            s.metrics.x_height_px,
            s.metrics.cap_height_px,
            s.metrics.ascender_px,
            s.metrics.descender_px,
            s.metrics.sample_width_px,
            s.metrics.missing_chars
        ));
    }
    out
}

fn format_compare_json(board: &ComparisonBoard) -> String {
    let mut j = String::from("{\"sample_word\":");
    push_json_str(&mut j, &board.sample_word);
    j.push_str(&format!(
        ",\"size_px\":{},\"shared_baseline_y_px\":{},\"total_height_px\":{},\"slots\":[",
        board.size_px, board.shared_baseline_y_px, board.total_height_px
    ));
    for (i, s) in board.slots.iter().enumerate() {
        if i > 0 {
            j.push(',');
        }
        j.push_str(&format!("{{\"slot_index\":{},\"family\":", s.slot_index));
        push_json_str(&mut j, &s.family);
        j.push_str(",\"subfamily\":");
        push_json_str(&mut j, &s.subfamily);
        j.push_str(&format!(
            ",\"weight\":{},\"baseline_y_px\":{},\"x_height_y_px\":{},\"cap_height_y_px\":{},\"ascender_y_px\":{},\"descender_y_px\":{},\"sample_width_px\":{},\"missing_chars\":{}}}",
            s.weight,
            s.baseline_y_px,
            s.x_height_y_px,
            s.cap_height_y_px,
            s.ascender_y_px,
            s.descender_y_px,
            s.metrics.sample_width_px,
            s.metrics.missing_chars
        ));
    }
    j.push_str("]}");
    j
}

fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use sex_font::{GEIST_MONO, GEIST_SANS, JETBRAINS_MONO};

    fn seeded_catalog() -> FontCatalog {
        let mut cat = FontCatalog::new();
        cat.add_from_bytes("/system/fonts/Geist-Regular.ttf", 0, GEIST_SANS);
        cat.add_from_bytes("/system/fonts/GeistMono-Regular.ttf", 0, GEIST_MONO);
        cat.add_from_bytes("/system/fonts/JetBrainsMono-Regular.ttf", 0, JETBRAINS_MONO);
        cat
    }

    #[test]
    fn cli_list_info_and_json_output() {
        let mut cat = seeded_catalog();
        let mut cli = FiligreeCli::new(&mut cat);

        let out = cli.run(&["list", "--mono", "--json"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("\"Geist Mono\""));
        assert!(out.stdout.contains("\"JetBrains Mono\""));
        assert!(!out.stdout.contains("\"Geist\""));

        let info = cli.run(&["info", "Geist Mono", "--json"], None);
        assert_eq!(info.exit_code, 0);
        assert!(info.stdout.contains("\"family\":\"Geist Mono\""));
        assert!(info.stdout.contains("\"is_monospace\":true"));
    }

    #[test]
    fn cli_activate_deactivate_emits_manifest_toml() {
        let mut cat = seeded_catalog();
        let mut cli = FiligreeCli::new(&mut cat);

        let deact = cli.run(&["deactivate", "Geist", "--json"], None);
        assert_eq!(deact.exit_code, 0);
        let toml = deact.manifest_toml.expect("manifest_toml");
        assert!(toml.contains("\"Geist\""));
        assert!(!cli.catalog.entries[0].active);

        let act = cli.run(&["activate", "Geist", "--json"], None);
        assert_eq!(act.exit_code, 0);
        assert!(cli.catalog.entries[0].active);
    }

    #[test]
    fn cli_install_check_dupes_compare_and_spindle_handoff() {
        let mut cat = seeded_catalog();
        cat.add_from_bytes("/home/user/GeistMono-Copy.ttf", 0, GEIST_MONO);

        let mut cli = FiligreeCli::new(&mut cat)
            .with_blob("/system/fonts/Geist-Regular.ttf", GEIST_SANS)
            .with_blob("/system/fonts/GeistMono-Regular.ttf", GEIST_MONO)
            .with_blob("/system/fonts/JetBrainsMono-Regular.ttf", JETBRAINS_MONO)
            .with_settings_toml("[appearance]\naccent = \"#3d7eff\"\n");

        let check = cli.run(&["install-check", "Geist.ttf", "--json"], Some(GEIST_SANS));
        assert_eq!(check.exit_code, 0);
        assert!(check.stdout.contains("\"ok\":true"));

        let dupes = cli.run(&["dupes", "--json"], None);
        assert_eq!(dupes.exit_code, 0);
        assert!(dupes.stdout.contains("\"exact_sha256\""));
        assert!(dupes.stdout.contains("\"remove_duplicate\""));

        let cmp = cli.run(
            &["compare", "Geist", "Geist Mono", "JetBrains Mono", "--word", "SexOS", "--json"],
            None,
        );
        assert_eq!(cmp.exit_code, 0);
        assert!(cmp.stdout.contains("\"sample_word\":\"SexOS\""));
        assert!(cmp.stdout.contains("\"shared_baseline_y_px\""));

        let handoff = cli.run(&["spindle-hand-off", "JetBrains Mono", "--json"], None);
        assert_eq!(handoff.exit_code, 0);
        let settings = handoff.settings_toml.expect("settings_toml");
        assert!(settings.contains("[spindle]"));
        assert!(settings.contains("monospace_font = \"JetBrains Mono\""));
        assert!(settings.contains("[appearance]"));
    }
}
