// SPDX-License-Identifier: GPL-3.0-only
//! Python in Polyamour: one long-running kernel per document
//! (`apps/polyamour/python/polyamour/kernel.py`, driven through
//! `sex_3d::script::live`). The Text Editor's Run Script, the Python Console,
//! add-ons (Preferences > Add-ons), script operators in F3 search, script
//! panels in the 3D View sidebar and the Properties editor, frame-change
//! handlers and `bpy.app.timers` all go through it.
//!
//! Every call that may change the scene runs on a copy: when it succeeds and
//! the scene changed, the copy replaces the scene as one undo step; when it
//! fails, nothing changes and the error (with the script's line) goes to the
//! Info editor and the status bar.
//!
//! Where programs cannot be connected by pipes (SexOS today) the kernel cannot
//! start; scripts then run on the batch path (`live::batch_*`): Python runs the
//! script on its own, reading the scene as it was when the script started, and
//! its commands are applied when it ends. The console, add-ons, panels and
//! handlers need the live kernel and say so.
use crate::anim::{self, ReportKind};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::theme as t;
use crate::ui::{id, MenuItem, NumOpts, Op, Ui};
pub use sex_3d::script::live::Printed;
use sex_3d::script::live::{self, json::Value, CallError, CallResult, Kernel, KernelConfig, NoHost};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A panel a script registered (`bpy.types.Panel`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelInfo {
    pub id: String,
    pub label: String,
    pub space: String,
    pub region: String,
    pub category: String,
    pub context: String,
    pub parent: String,
    pub options: Vec<String>,
}

/// An operator a script registered (`bpy.types.Operator`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperatorInfo {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// An add-on found in the add-ons folder (its `bl_info`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddonInfo {
    pub module: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: String,
    pub author: String,
    pub location: String,
    pub file: String,
}

#[derive(Default)]
pub struct Python {
    kernel: Option<Kernel>,
    /// Why the live kernel is not available here (then scripts run in batch).
    pub unavailable: Option<String>,
    told_batch: bool,
    /// The kernel's greeting (`hello polyops 1 python=3.13.1`).
    pub hello: String,
    pub panels: Vec<PanelInfo>,
    pub operators: Vec<OperatorInfo>,
    /// Built-in menus scripts appended to (`VIEW3D_MT_object.append`).
    pub extended_menus: Vec<String>,
    handlers: Vec<String>,
    timers: usize,
    timer_due: Option<Instant>,
    frame_seen: Option<i32>,
    /// Drawn layouts by panel id, with the state key they were drawn for.
    layouts: HashMap<String, (u64, Value)>,
    layout_gen: u64,
    /// The script tab shown in the 3D View sidebar (None: a built-in tab).
    pub side_tab: Option<String>,
    /// Script panels the user closed (or opened, for DEFAULT_CLOSED ones).
    toggled: HashSet<String>,
    pub addons: Vec<AddonInfo>,
    pub enabled: BTreeSet<String>,
    addons_loaded: bool,
    /// Another data folder than [`data_dir`] (tests).
    pub dir: Option<PathBuf>,
}

impl Python {
    /// Timers are waiting: the window should keep sending ticks.
    pub fn wants_tick(&self) -> bool {
        self.timers > 0 && self.kernel.is_some()
    }

    /// The data folder: add-ons and the enabled list live here.
    pub fn data(&self) -> PathBuf {
        self.dir.clone().unwrap_or_else(data_dir)
    }

    /// Why the live kernel cannot run here (empty while it can).
    pub fn unavailable_reason(&self) -> &str {
        self.unavailable.as_deref().unwrap_or("")
    }

    /// Whether the live kernel runs (or has not been tried yet).
    pub fn live(&self) -> bool {
        self.unavailable.is_none()
    }

    /// Sidebar categories of script panels, in registration order.
    pub fn sidebar_categories(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.panels.iter().filter(|p| p.space == "VIEW_3D" && p.region == "UI" && p.parent.is_empty()) {
            let c = if p.category.is_empty() { "Misc".to_string() } else { p.category.clone() };
            if !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }
}

// ---------------------------------------------------------------- places

/// Polyamour's own data folder: `/data/polyamour` on SexOS, the user's data
/// folder elsewhere.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "hermit")]
    {
        PathBuf::from("/data/polyamour")
    }
    #[cfg(not(target_os = "hermit"))]
    {
        if let Ok(p) = std::env::var("POLYAMOUR_DATA") {
            return PathBuf::from(p);
        }
        if let Ok(x) = std::env::var("XDG_DATA_HOME") {
            if !x.is_empty() {
                return PathBuf::from(x).join("polyamour");
            }
        }
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => PathBuf::from(h).join(".local/share/polyamour"),
            _ => std::env::temp_dir().join("polyamour"),
        }
    }
}

/// Where add-ons are installed (each a `.py` module or a package folder).
pub fn addons_dir(d: &Doc) -> PathBuf {
    d.python.data().join("addons")
}

/// The list of enabled add-ons, one module name per line.
fn enabled_file(d: &Doc) -> PathBuf {
    d.python.data().join("addons_enabled.txt")
}

fn config() -> KernelConfig {
    #[cfg(target_os = "hermit")]
    {
        // The package is frozen into SexOS's python.
        KernelConfig { program: "python".into(), paths: Vec::new(), addons: None, env: Vec::new() }
    }
    #[cfg(not(target_os = "hermit"))]
    {
        let mut c = KernelConfig::host();
        if std::env::var("POLYAMOUR_PYTHON_PATH").is_err() {
            c.paths = vec![concat!(env!("CARGO_MANIFEST_DIR"), "/python").to_string()];
        }
        c
    }
}

// ---------------------------------------------------------------- calls

/// How a call's scene changes are kept.
#[derive(Clone, Copy)]
enum Apply<'a> {
    /// As one undo step with this label.
    Undo(&'a str),
    /// Without an undo step (handlers, timers: Blender records none either).
    Quiet,
    /// The call only reads; it runs on the scene itself.
    Read,
}

/// Start the kernel if needed. False when it cannot run here.
fn ensure(d: &mut Doc) -> bool {
    if let Some(k) = &d.python.kernel {
        if k.is_alive() {
            return true;
        }
        d.python.kernel = None;
        d.python.addons_loaded = false;
        anim::report(d, ReportKind::Warning, "The Python kernel stopped; starting a new one");
    }
    if d.python.unavailable.is_some() {
        return false;
    }
    match Kernel::spawn(&config()) {
        Ok(k) => {
            d.python.hello = k.hello.clone();
            d.python.kernel = Some(k);
            load_addons(d);
            true
        }
        Err(e) => {
            let why = if e.kind() == std::io::ErrorKind::Unsupported {
                "live Python needs pipes between programs, which this system does not have yet".to_string()
            } else {
                format!("cannot start Python ({e})")
            };
            d.python.unavailable = Some(why);
            false
        }
    }
}

fn call(d: &mut Doc, apply: Apply, kind: &str, args: &[&str]) -> Option<CallResult> {
    if !ensure(d) {
        return None;
    }
    let (r, changed, reports, alive) = {
        let Doc { python, scene, .. } = &mut *d;
        let k = python.kernel.as_mut()?;
        let (r, changed) = match apply {
            Apply::Read => (k.call(scene, &mut NoHost, kind, args), None),
            _ => {
                let mut work = scene.clone();
                let r = k.call(&mut work, &mut NoHost, kind, args);
                let changed = (r.ok() && work != *scene).then_some(work);
                (r, changed)
            }
        };
        (r, changed, std::mem::take(&mut k.session.bpy.reports), k.is_alive())
    };
    if let Some(work) = changed {
        if let Apply::Undo(label) = apply {
            d.push_undo(label);
        }
        d.scene = work;
        crate::edit::sanitize(d);
        d.touch();
        d.anim.dirty = true;
    }
    if !matches!(apply, Apply::Read) {
        d.python.layout_gen += 1;
    }
    for (level, text) in reports {
        let kind = if level.starts_with("ERROR") {
            ReportKind::Error
        } else if level == "WARNING" {
            ReportKind::Warning
        } else {
            ReportKind::Info
        };
        anim::report(d, kind, &text);
    }
    if !alive {
        d.python.kernel = None;
        d.python.addons_loaded = false;
    }
    Some(r)
}

/// Ask the kernel what scripts registered (after anything that may register).
fn refresh_registry(d: &mut Doc) {
    let Some(r) = call(d, Apply::Read, "panels", &[]) else { return };
    let Some(j) = r.json else { return };
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let py = &mut d.python;
    py.panels = j["panels"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| PanelInfo {
                    id: s(&p["id"]),
                    label: s(&p["label"]),
                    space: s(&p["space"]),
                    region: s(&p["region"]),
                    category: s(&p["category"]),
                    context: s(&p["context"]),
                    parent: s(&p["parent"]),
                    options: p["options"].as_array().map(|o| o.iter().map(s).collect()).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    py.operators = j["operators"]
        .as_array()
        .map(|a| a.iter().map(|o| OperatorInfo { id: s(&o["id"]), label: s(&o["label"]), description: s(&o["description"]) }).collect())
        .unwrap_or_default();
    py.extended_menus = j["extended_menus"].as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default();
    py.handlers = j["handlers"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    py.timers = j["timers"].as_u64().unwrap_or(0) as usize;
    if py.timers > 0 && py.timer_due.is_none() {
        py.timer_due = Some(Instant::now());
    }
    if let Some(tab) = &py.side_tab {
        if !py.sidebar_categories().contains(tab) {
            py.side_tab = None;
        }
    }
    py.layouts.clear();
}

fn printed_to_info(d: &mut Doc, printed: &[Printed]) {
    let text: String = printed.iter().map(|p| match p {
        Printed::Out(s) | Printed::Err(s) => s.as_str(),
    }).collect();
    for l in text.lines() {
        d.info.reports.push(anim::Report { kind: ReportKind::Output, text: l.to_string() });
    }
}

fn report_error(d: &mut Doc, what: &str, e: &CallError) {
    for l in e.traceback.lines() {
        d.info.reports.push(anim::Report { kind: ReportKind::Output, text: l.to_string() });
    }
    anim::report(d, ReportKind::Error, &format!("{what}: {e}"));
}

// ---------------------------------------------------------------- scripts

/// Run a Python text (Text Editor > Run Script, Alt+P).
pub fn run_script(d: &mut Doc, name: &str, src: &str) {
    if ensure(d) {
        let Some(r) = call(d, Apply::Undo("Run Script"), "exec", &[name, src]) else { return };
        printed_to_info(d, &r.printed);
        match &r.result {
            Ok(_) => anim::report(d, ReportKind::Info, &format!("Ran {name}")),
            Err(e) => report_error(d, name, e),
        }
        refresh_registry(d);
    } else {
        run_batch(d, name, src);
    }
}

/// The one-way path for systems without pipes: see `live::batch_snapshot`.
fn run_batch(d: &mut Doc, name: &str, src: &str) {
    if !d.python.told_batch {
        d.python.told_batch = true;
        let why = d.python.unavailable.clone().unwrap_or_default();
        anim::report(d, ReportKind::Warning, &format!("Python runs scripts in batch mode: {why}; a script reads the scene as it was when it started"));
    }
    match batch(d, name, src) {
        Ok(()) => {}
        Err(e) => anim::report(d, ReportKind::Error, &format!("{name}: {e}")),
    }
}

fn batch(d: &mut Doc, name: &str, src: &str) -> Result<(), String> {
    let dir = d.python.data().join("run");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let file: String = name.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect();
    let script = dir.join(if file.ends_with(".py") { file } else { format!("{file}.py") });
    let ops = dir.join("ops.polyops");
    let snap = dir.join("snapshot.json");
    let _ = std::fs::remove_file(&ops);
    std::fs::write(&script, src).map_err(|e| format!("cannot write {}: {e}", script.display()))?;
    std::fs::write(&snap, live::batch_snapshot(&d.scene)).map_err(|e| format!("cannot write {}: {e}", snap.display()))?;
    // The child works in the run folder (on SexOS, Files hands it that folder),
    // so the files go by their names.
    let name_of = |p: &std::path::Path| PathBuf::from(p.file_name().unwrap_or_default());
    let mut cmd = live::batch_command(&config(), &name_of(&script), &name_of(&ops), &name_of(&snap));
    cmd.current_dir(&dir);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let status = cmd.status().map_err(|e| format!("cannot run python: {e}"))?;
    let text = std::fs::read_to_string(&ops).unwrap_or_default();
    printed_to_info(d, &live::batch_output(&text));
    if text.is_empty() {
        return Err(format!("python ended ({status}) without running the script"));
    }
    let mut work = d.scene.clone();
    let rep = live::batch_apply(&mut work, &text).map_err(|e| e.message)?;
    if work != d.scene {
        d.push_undo("Run Script");
        d.scene = work;
        crate::edit::sanitize(d);
        d.touch();
        d.anim.dirty = true;
    }
    anim::report(d, ReportKind::Info, &format!("Ran {name}: {} command(s)", rep.commands));
    Ok(())
}

// ---------------------------------------------------------------- console

/// What a console line did.
pub enum ConsoleResult {
    /// It ran (or failed); the printed text, and whether it raised.
    Done { printed: Vec<Printed>, error: bool },
    /// The block goes on (`for ...:`): prompt with `... `.
    More,
}

/// Run one console line in the kernel's console namespace (`C`, `D`, `bpy`,
/// mathutils and math names). None when Python cannot run live here.
pub fn console(d: &mut Doc, line: &str) -> Option<ConsoleResult> {
    let r = call(d, Apply::Undo("Console"), "console", &[line])?;
    let out = match &r.result {
        Ok(v) if v.first().map(String::as_str) == Some("more") => ConsoleResult::More,
        Ok(v) => ConsoleResult::Done { printed: r.printed.clone(), error: v.first().map(String::as_str) == Some("error") },
        Err(e) => ConsoleResult::Done { printed: vec![Printed::Err(format!("{e}\n"))], error: true },
    };
    if matches!(out, ConsoleResult::Done { .. }) {
        refresh_registry(d);
    }
    Some(out)
}

/// Completions of `text` (the input up to the caret): whole replacement
/// texts, sorted. None when Python cannot run live here.
pub fn complete(d: &mut Doc, text: &str) -> Option<Vec<String>> {
    let r = call(d, Apply::Read, "complete", &[text])?;
    let j = r.json?;
    Some(j["completions"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default())
}

/// The console banner, Blender's with our names.
pub fn banner(d: &mut Doc) -> Vec<String> {
    if !ensure(d) {
        let why = d.python.unavailable.clone().unwrap_or_default();
        return vec![
            "POLYAMOUR INTERACTIVE CONSOLE (polyops 1)".into(),
            String::new(),
            format!("Python is not live here: {why}."),
            "Each line runs as a polyops command on the open scene, for example:".into(),
            "    add_primitive uv_sphere location=0,0,2".into(),
            "Python scripts run from the Text Editor (Alt P), in batch mode.".into(),
            String::new(),
        ];
    }
    let version = d.python.hello.split_whitespace().find_map(|w| w.strip_prefix("python=")).unwrap_or("3").to_string();
    vec![
        format!("PYTHON INTERACTIVE CONSOLE {version}"),
        String::new(),
        "Builtin Modules:       bpy, bpy.data, bpy.ops, bpy.props, bpy.types, bpy.context, bpy.utils, bmesh, mathutils".into(),
        "Convenience Imports:   from mathutils import *; from math import *".into(),
        "Convenience Variables: C = bpy.context, D = bpy.data".into(),
        String::new(),
    ]
}

// ---------------------------------------------------------------- operators

/// Script operators for F3 search: (label, operator to run).
pub fn search_items(d: &Doc) -> Vec<MenuItem> {
    d.python.operators.iter().map(|o| MenuItem::op(&if o.label.is_empty() { o.id.clone() } else { o.label.clone() }, "", Op::with("script.operator", &o.id))).collect()
}

/// Operators this module runs (`Doc::run` passes them here first).
pub fn run(d: &mut Doc, op: &Op) -> bool {
    match op.id.as_str() {
        "script.operator" => {
            // The arg: an idname, or {"id": ..., "props": {...}} from a button.
            let (idname, props) = match live::json::from_str::<Value>(&op.arg) {
                Ok(j) if j.is_object() => (j["id"].as_str().unwrap_or("").to_string(), j["props"].to_string()),
                _ => (op.arg.clone(), "{}".to_string()),
            };
            let label = d.python.operators.iter().find(|o| o.id == idname).map(|o| o.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| idname.clone());
            let Some(r) = call(d, Apply::Undo(&label), "operator", &[&idname, "invoke", &props]) else {
                anim::report(d, ReportKind::Error, &format!("{idname}: script operators need live Python"));
                return true;
            };
            printed_to_info(d, &r.printed);
            if let Err(e) = &r.result {
                report_error(d, &idname, e);
            }
            refresh_registry(d);
            true
        }
        "script.set_prop" => {
            let Ok(j) = live::json::from_str::<Value>(&op.arg) else { return true };
            let (panel, rf) = (j["panel"].as_str().unwrap_or(""), j["ref"].to_string());
            let value = j["value"].to_string();
            if let Some(r) = call(d, Apply::Undo("Change Property"), "set_prop", &[panel, &rf, &value]) {
                if let Err(e) = &r.result {
                    report_error(d, panel, e);
                }
            }
            true
        }
        "script.addon_toggle" => {
            toggle_addon(d, &op.arg);
            true
        }
        "script.addons_refresh" => {
            scan_addons(d);
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- add-ons

fn read_enabled(d: &Doc) -> BTreeSet<String> {
    std::fs::read_to_string(enabled_file(d)).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect()).unwrap_or_default()
}

fn save_enabled(d: &Doc) -> Result<(), String> {
    let set = &d.python.enabled;
    let f = enabled_file(d);
    if let Some(p) = f.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut text = String::from("# Add-ons Polyamour enables at start (Preferences > Add-ons)\n");
    for m in set {
        text.push_str(m);
        text.push('\n');
    }
    std::fs::write(&f, text).map_err(|e| e.to_string())
}

/// Called once the kernel runs: point it at the add-ons folder and enable
/// the add-ons the user enabled before.
fn load_addons(d: &mut Doc) {
    if d.python.addons_loaded {
        return;
    }
    d.python.addons_loaded = true;
    scan_addons(d);
    let wanted = read_enabled(d);
    for m in &wanted {
        if d.python.addons.iter().any(|a| &a.module == m) {
            if let Some(r) = call(d, Apply::Quiet, "addon_enable", &[m]) {
                if let Err(e) = &r.result {
                    report_error(d, m, e);
                }
            }
        }
    }
    d.python.enabled = wanted;
    refresh_registry(d);
}

/// Read the add-ons folder (bl_info of each add-on).
pub fn scan_addons(d: &mut Doc) {
    let dir = addons_dir(d);
    let _ = std::fs::create_dir_all(&dir);
    let dir = dir.to_string_lossy().to_string();
    let Some(r) = call(d, Apply::Read, "addons", &[&dir]) else { return };
    let Some(j) = r.json else { return };
    let s = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    };
    d.python.addons = j["addons"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| AddonInfo {
                    module: s(&x["module"]),
                    name: s(&x["name"]),
                    description: s(&x["description"]),
                    category: s(&x["category"]),
                    version: s(&x["version"]),
                    author: s(&x["author"]),
                    location: s(&x["location"]),
                    file: s(&x["file"]),
                })
                .collect()
        })
        .unwrap_or_default();
}

/// Enable or disable an add-on and remember the choice.
pub fn toggle_addon(d: &mut Doc, module: &str) {
    let on = !d.python.enabled.contains(module);
    let kind = if on { "addon_enable" } else { "addon_disable" };
    let Some(r) = call(d, Apply::Undo(if on { "Enable Add-on" } else { "Disable Add-on" }), kind, &[module]) else {
        anim::report(d, ReportKind::Error, "Add-ons need live Python");
        return;
    };
    printed_to_info(d, &r.printed);
    match &r.result {
        Ok(_) => {
            if on {
                d.python.enabled.insert(module.to_string());
            } else {
                d.python.enabled.remove(module);
            }
            let name = d.python.addons.iter().find(|a| a.module == module).map(|a| a.name.clone()).unwrap_or_else(|| module.to_string());
            anim::report(d, ReportKind::Info, &format!("{} add-on \"{name}\"", if on { "Enabled" } else { "Disabled" }));
            if let Err(e) = save_enabled(d) {
                anim::report(d, ReportKind::Warning, &format!("Cannot save the add-on list: {e}"));
            }
        }
        Err(e) => report_error(d, module, e),
    }
    refresh_registry(d);
}

// ---------------------------------------------------------------- handlers

/// Once per window frame: frame-change handlers when the frame moved, due
/// timers, and add-ons enabled in an earlier session. True when the scene
/// changed (the window draws again).
pub fn tick(d: &mut Doc) -> bool {
    let gen = d.generation;
    if !d.python.addons_loaded && d.python.unavailable.is_none() && d.python.kernel.is_none() && enabled_file(d).exists() && !read_enabled(d).is_empty() {
        ensure(d);
    }
    if d.python.kernel.is_none() {
        return d.generation != gen;
    }
    let frame = d.frame;
    if d.python.frame_seen != Some(frame) {
        let first = d.python.frame_seen.is_none();
        d.python.frame_seen = Some(frame);
        if !first && d.python.handlers.iter().any(|h| h.starts_with("frame_change")) {
            let f = frame.to_string();
            if let Some(r) = call(d, Apply::Quiet, "frame", &[&f]) {
                printed_to_info(d, &r.printed);
                if let Err(e) = &r.result {
                    report_error(d, "frame_change handler", e);
                }
            }
        }
    }
    if d.python.timers > 0 && d.python.timer_due.is_some_and(|t| Instant::now() >= t) {
        if let Some(r) = call(d, Apply::Quiet, "timers", &[]) {
            printed_to_info(d, &r.printed);
            let next = r.json.as_ref().and_then(|j| j["next"].as_f64());
            d.python.timer_due = next.map(|s| Instant::now() + Duration::from_secs_f64(s.max(0.0)));
            refresh_registry(d);
        }
    }
    d.generation != gen
}

// ---------------------------------------------------------------- layouts

const ROW: i32 = 20;

/// A drawn panel (`draw` call), cached until the scene or a property changes.
fn layout(d: &mut Doc, panel: &str) -> Option<Value> {
    let key = d.generation.wrapping_mul(0x9e37_79b9) ^ d.python.layout_gen.wrapping_shl(20) ^ d.scene.active_object.map_or(u64::MAX, |a| a as u64).wrapping_shl(40) ^ d.scene.selected.len() as u64;
    if let Some((k, v)) = d.python.layouts.get(panel) {
        if *k == key {
            return Some(v.clone());
        }
    }
    let r = call(d, Apply::Read, "draw", &[panel])?;
    let v = match (&r.result, r.json) {
        (Ok(_), Some(j)) => j,
        (Err(e), _) => {
            let e = e.clone();
            report_error(d, panel, &e);
            live::json::json!({"id": panel, "poll": true, "layout": {"type": "column", "items": [{"type": "label", "text": format!("Error: {}", e.message), "alert": true}]}})
        }
        _ => return None,
    };
    d.python.layouts.insert(panel.to_string(), (key, v.clone()));
    Some(v)
}

fn height(n: &Value) -> i32 {
    match n["type"].as_str().unwrap_or("") {
        "row" | "split" | "pie" => n["items"].as_array().map_or(0, |a| a.iter().map(height).max().unwrap_or(0)),
        "column" | "box" | "grid" => {
            let inner: i32 = n["items"].as_array().map_or(0, |a| a.iter().map(height).sum());
            inner + if n["type"] == "box" { 8 } else { 0 }
        }
        "separator" => (8.0 * n["factor"].as_f64().unwrap_or(1.0)) as i32,
        "spacer" => 0,
        "prop" if n["value"].is_array() && n["expand"] != true && n["kind"] != "enum" => n["value"].as_array().map_or(1, |a| a.len().max(1)) as i32 * ROW + 2,
        "list" => n["items"].as_array().map_or(1, |a| a.len().clamp(1, n["rows"].as_u64().unwrap_or(5) as usize)) as i32 * ROW + 4,
        _ => ROW + 2,
    }
}

fn text_of(n: &Value, key: &str) -> String {
    n[key].as_str().unwrap_or("").to_string()
}

/// Draw a layout node (from `UILayout.to_json`) at (x, *y) in width `w`.
/// Changes become `script.*` operators in `ops`.
#[allow(clippy::too_many_arguments)]
fn draw_node(ui: &mut Ui, d: &mut Doc, panel: &str, n: &Value, x: i32, w: i32, y: &mut i32, ops: &mut Vec<Op>) {
    let enabled = n["enabled"] != false;
    match n["type"].as_str().unwrap_or("") {
        "row" | "split" | "pie" => {
            let items: Vec<&Value> = n["items"].as_array().map(|a| a.iter().filter(|c| c["type"] != "separator" && c["type"] != "spacer").collect()).unwrap_or_default();
            let k = items.len().max(1) as i32;
            let gap = if n["align"] == true { 0 } else { 4 };
            let factor = n["factor"].as_f64().unwrap_or(0.0);
            let h = height(n);
            for (i, c) in items.iter().enumerate() {
                let (cx, cw) = if n["type"] == "split" && factor > 0.0 && k == 2 {
                    let lw = (w as f64 * factor) as i32;
                    if i == 0 { (x, lw - gap) } else { (x + lw, w - lw) }
                } else {
                    let cw = (w - gap * (k - 1)) / k;
                    (x + i as i32 * (cw + gap), cw)
                };
                let mut cy = *y;
                draw_node(ui, d, panel, c, cx, cw, &mut cy, ops);
            }
            *y += h;
        }
        "column" | "box" | "grid" => {
            let boxed = n["type"] == "box";
            if boxed {
                let h = height(n);
                ui.p.round_rect(Rect::new(x - 2, *y, w + 4, h - 2), 4, 0x2b2b2b);
                *y += 4;
            }
            let (bx, bw) = if boxed { (x + 4, w - 8) } else { (x, w) };
            for c in n["items"].as_array().into_iter().flatten() {
                draw_node(ui, d, panel, c, bx, bw, y, ops);
            }
            if boxed {
                *y += 4;
            }
        }
        "separator" | "spacer" => *y += height(n),
        "label" => {
            let col = if n["alert"] == true { 0xff6060 } else if enabled { t::TEXT } else { t::TEXT_DIM };
            ui.p.label(Rect::new(x, *y, w, ROW), &text_of(n, "text"), col, Align::Left);
            *y += ROW + 2;
        }
        "operator" | "operator_menu_enum" => {
            let r = Rect::new(x, *y, w, ROW);
            let label = text_of(n, "text");
            if ui.button(r, &label) && enabled {
                let arg = live::json::json!({"id": n["operator"], "props": n["properties"]});
                ops.push(Op::with("script.operator", &arg.to_string()));
            }
            *y += ROW + 2;
        }
        "menu" => {
            let r = Rect::new(x, *y, w, ROW);
            if ui.dropdown(r, &text_of(n, "text")) {
                let menu = text_of(n, "menu");
                let items = menu_items(d, &menu);
                ui.open_menu(r, &text_of(n, "text"), items);
            }
            *y += ROW + 2;
        }
        "prop" => draw_prop(ui, panel, n, x, w, y, ops),
        "list" => {
            let names: Vec<String> = n["items"].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
            let h = height(n);
            ui.p.round_rect(Rect::new(x, *y, w, h - 2), 4, 0x282828);
            let active = n["active"].as_i64().unwrap_or(-1);
            for (i, name) in names.iter().take(((h - 4) / ROW) as usize).enumerate() {
                let rr = Rect::new(x + 2, *y + 2 + i as i32 * ROW, w - 4, ROW);
                if i as i64 == active {
                    ui.p.round_rect(rr, 3, t::SELECTED);
                }
                ui.p.label(rr.shrink(4), name, t::TEXT, Align::Left);
            }
            *y += h;
        }
        _ => {
            let label = text_of(n, "text");
            if !label.is_empty() {
                ui.p.label(Rect::new(x, *y, w, ROW), &label, t::TEXT_DIM, Align::Left);
                *y += ROW + 2;
            }
        }
    }
}

fn set_prop_op(panel: &str, rf: &Value, value: Value) -> Op {
    Op::with("script.set_prop", &live::json::json!({"panel": panel, "ref": rf, "value": value}).to_string())
}

fn draw_prop(ui: &mut Ui, panel: &str, n: &Value, x: i32, w: i32, y: &mut i32, ops: &mut Vec<Op>) {
    let text = text_of(n, "text");
    let rf = &n["ref"];
    let wid_key = format!("py/{panel}/{rf}");
    let kind = n["kind"].as_str().unwrap_or("");
    let value = &n["value"];
    let r = Rect::new(x, *y, w, ROW);
    match kind {
        "bool" if !value.is_array() => {
            let mut v = value.as_bool().unwrap_or(false);
            if ui.checkbox(r, &text, &mut v) {
                ops.push(set_prop_op(panel, rf, Value::Bool(v)));
            }
            *y += ROW + 2;
        }
        "enum" => {
            let items: Vec<(String, String)> = n["items"].as_array().map(|a| a.iter().map(|i| (i[0].as_str().unwrap_or("").to_string(), i[1].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
            let cur = value.as_str().unwrap_or("");
            let cur_label = items.iter().find(|i| i.0 == cur).map(|i| i.1.clone()).unwrap_or_else(|| cur.to_string());
            if n["expand"] == true {
                let mut sel = items.iter().position(|i| i.0 == cur).unwrap_or(usize::MAX);
                let labels: Vec<&str> = items.iter().map(|i| i.1.as_str()).collect();
                if ui.segmented(r, &labels, &mut sel) {
                    if let Some(i) = items.get(sel) {
                        ops.push(set_prop_op(panel, rf, Value::String(i.0.clone())));
                    }
                }
            } else {
                let (lr, fr) = split(r, !text.is_empty());
                ui.p.label(lr, &text, t::TEXT, Align::Right);
                if ui.dropdown(fr, &cur_label) {
                    let menu = items.iter().map(|i| MenuItem::op(&i.1, "", set_prop_op(panel, rf, Value::String(i.0.clone())))).collect();
                    ui.open_menu(fr, &text, menu);
                }
            }
            *y += ROW + 2;
        }
        "string" => {
            let (lr, fr) = split(r, !text.is_empty());
            ui.p.label(lr, &text, t::TEXT, Align::Right);
            let mut s = value.as_str().unwrap_or("").to_string();
            if ui.text_field(id(&wid_key), fr, &mut s) {
                ops.push(set_prop_op(panel, rf, Value::String(s)));
            }
            *y += ROW + 2;
        }
        "int" | "float" | "" if value.is_number() => {
            let int = kind == "int" || (value.is_i64() && kind.is_empty());
            let mut v = value.as_f64().unwrap_or(0.0) as f32;
            let o = NumOpts {
                step: if int { 1.0 } else { n["step"].as_f64().map_or(0.1, |s| s / 100.0) as f32 },
                unit: "",
                min: n["min"].as_f64().map_or(f32::NEG_INFINITY, |m| m as f32),
                max: n["max"].as_f64().map_or(f32::INFINITY, |m| m as f32),
                slider: n["slider"] == true,
                int,
            };
            if ui.number_ex(id(&wid_key), r, &text, &mut v, o) {
                let nv = if int { Value::from(v.round() as i64) } else { Value::from(v as f64) };
                ops.push(set_prop_op(panel, rf, nv));
            }
            *y += ROW + 2;
        }
        _ if value.is_array() => {
            // A vector or colour: one field per component, like Blender's column.
            if !text.is_empty() {
                ui.p.label(Rect::new(x, *y, w, ROW), &format!("{text}:"), t::TEXT, Align::Left);
                *y += ROW;
            }
            let comps: Vec<f64> = value.as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default();
            let names = ["X", "Y", "Z", "W"];
            let is_bool = value.as_array().is_some_and(|a| a.iter().all(Value::is_boolean));
            for (i, c) in comps.iter().enumerate() {
                let fr = Rect::new(x, *y, w, ROW - 1);
                let mut v = *c as f32;
                if !is_bool && ui.number(id(&format!("{wid_key}/{i}")), fr, names.get(i).copied().unwrap_or(""), &mut v, 0.1, "") {
                    let mut all = comps.clone();
                    all[i] = v as f64;
                    ops.push(set_prop_op(panel, rf, Value::from(all)));
                }
                *y += ROW;
            }
            *y += 2;
        }
        _ => {
            let shown = match value {
                Value::String(s) => s.clone(),
                Value::Null => "None".into(),
                v => v.to_string(),
            };
            let (lr, fr) = split(r, !text.is_empty());
            ui.p.label(lr, &text, t::TEXT, Align::Right);
            ui.p.round_rect(fr, 4, t::BUTTON_DARK);
            ui.p.label(fr.shrink(4), &shown, t::TEXT, Align::Left);
            *y += ROW + 2;
        }
    }
}

/// Blender's property split: label column (40%) and field.
fn split(r: Rect, labelled: bool) -> (Rect, Rect) {
    if !labelled {
        return (Rect::new(r.x, r.y, 0, r.h), r);
    }
    let lw = (r.w as f32 * 0.4) as i32;
    (Rect::new(r.x, r.y, lw - 8, r.h), Rect::new(r.x + lw, r.y, r.w - lw, r.h))
}

/// A script menu's entries (`menu` call) as popup items.
fn menu_items(d: &mut Doc, menu: &str) -> Vec<MenuItem> {
    let Some(r) = call(d, Apply::Read, "menu", &[menu]) else { return vec![MenuItem::Heading("Python is not live here".into())] };
    let Some(j) = r.json else { return Vec::new() };
    let mut out = Vec::new();
    collect_menu(&j["layout"], &mut out);
    if out.is_empty() {
        out.push(MenuItem::Heading("Empty".into()));
    }
    out
}

fn collect_menu(n: &Value, out: &mut Vec<MenuItem>) {
    match n["type"].as_str().unwrap_or("") {
        "operator" => {
            let arg = live::json::json!({"id": n["operator"], "props": n["properties"]});
            out.push(MenuItem::op(&text_of(n, "text"), "", Op::with("script.operator", &arg.to_string())));
        }
        "separator" => out.push(MenuItem::Separator),
        "label" => out.push(MenuItem::Heading(text_of(n, "text"))),
        _ => {
            for c in n["items"].as_array().into_iter().flatten() {
                collect_menu(c, out);
            }
        }
    }
}

/// Entries scripts appended to the built-in menu `menu`
/// (`bpy.types.VIEW3D_MT_object.append(fn)`), for the app's own menus.
pub fn appended_menu_items(d: &mut Doc, menu: &str) -> Vec<MenuItem> {
    if !d.python.extended_menus.iter().any(|m| m == menu) {
        return Vec::new();
    }
    let items = menu_items(d, menu);
    if items.iter().all(|i| matches!(i, MenuItem::Heading(_))) {
        return Vec::new();
    }
    items
}

/// Draw one script panel (header and body) at (x, *y); returns false when its
/// poll() said no (nothing drawn).
fn draw_panel(ui: &mut Ui, d: &mut Doc, p: &PanelInfo, x: i32, w: i32, y: &mut i32) -> bool {
    let Some(j) = layout(d, &p.id) else { return false };
    if j["poll"] == false {
        return false;
    }
    let mut ops = Vec::new();
    let hide_header = p.options.iter().any(|o| o == "HIDE_HEADER");
    let default_closed = p.options.iter().any(|o| o == "DEFAULT_CLOSED");
    let open = default_closed == d.python.toggled.contains(&p.id);
    if !hide_header {
        let hr = Rect::new(x - 6, *y, w + 12, 24);
        ui.p.round_rect_alpha(hr, 4, t::PANEL_HEADER, 245);
        ui.p.arrow((hr.x + 12) as f32, (hr.y + hr.h / 2) as f32, 8.0, open, t::TEXT_DIM);
        let label = j["label"].as_str().map(str::to_string).unwrap_or_else(|| p.label.clone());
        ui.p.label(Rect::new(hr.x + 22, hr.y, hr.w - 28, hr.h), &label, t::TEXT, Align::Left);
        if ui.clicked(hr) && !d.python.toggled.remove(&p.id) {
            d.python.toggled.insert(p.id.clone());
        }
        *y += 28;
    }
    if open || hide_header {
        draw_node(ui, d, &p.id, &j["layout"], x, w, y, &mut ops);
        *y += 6;
    }
    d.queue.extend(ops);
    true
}

/// The 3D View sidebar's script tab `category`: its panels, then each one's
/// sub-panels.
pub fn draw_sidebar(ui: &mut Ui, d: &mut Doc, category: &str, x: i32, w: i32, y: &mut i32) {
    let tops: Vec<PanelInfo> = d
        .python
        .panels
        .iter()
        .filter(|p| p.space == "VIEW_3D" && p.region == "UI" && p.parent.is_empty() && (if p.category.is_empty() { "Misc" } else { p.category.as_str() }) == category)
        .cloned()
        .collect();
    for p in tops {
        if draw_panel(ui, d, &p, x, w, y) {
            let subs: Vec<PanelInfo> = d.python.panels.iter().filter(|c| c.parent == p.id).cloned().collect();
            for s in subs {
                draw_panel(ui, d, &s, x + 8, w - 8, y);
            }
        }
    }
}

/// Script panels for a Properties tab (`bl_context`: object, scene, render,
/// material, data, ...), drawn below the tab's own panels.
pub fn properties_panels(l: &mut crate::editors::properties::layout::L, d: &mut Doc, context: &str) {
    if l.dry {
        return;
    }
    let tops: Vec<PanelInfo> = d.python.panels.iter().filter(|p| p.space == "PROPERTIES" && p.context == context && p.parent.is_empty()).cloned().collect();
    for p in tops {
        let (x, w) = l.inner();
        let mut y = l.y;
        if draw_panel(l.ui, d, &p, x, w, &mut y) {
            let subs: Vec<PanelInfo> = d.python.panels.iter().filter(|c| c.parent == p.id).cloned().collect();
            for s in subs {
                draw_panel(l.ui, d, &s, x + 8, w - 8, &mut y);
            }
            l.y = y + 4;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_python() -> bool {
        std::process::Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn script_undo_console_and_errors() {
        if !have_python() {
            return;
        }
        let mut d = Doc::default();
        let n = d.scene.objects.len();
        run_script(&mut d, "gen.py", "import bpy\nfor i in range(3):\n    bpy.ops.mesh.primitive_cube_add(size=0.5, location=(i, 0, 0))\nprint(len(bpy.data.objects))\n");
        assert_eq!(d.scene.objects.len(), n + 3, "{:?}", d.info.reports);
        assert!(d.info.reports.iter().any(|r| r.kind == ReportKind::Output && r.text == (n + 3).to_string()));
        // One script, one undo step.
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.objects.len(), n);
        // An error names the line and changes nothing.
        run_script(&mut d, "bad.py", "import bpy\nbpy.ops.mesh.primitive_cube_add()\nbpy.data.objects['Missing']\n");
        assert_eq!(d.scene.objects.len(), n);
        let last = d.info.reports.last().unwrap();
        assert_eq!(last.kind, ReportKind::Error);
        assert!(last.text.contains("bad.py: line 3: KeyError"), "{}", last.text);
        assert!(d.report.contains("line 3"));
        // The console keeps its namespace and reads what it did.
        assert!(matches!(console(&mut d, "x = C.active_object.name"), Some(ConsoleResult::Done { error: false, .. })));
        match console(&mut d, "x") {
            Some(ConsoleResult::Done { printed, .. }) => assert_eq!(printed, vec![Printed::Out("'Cube'".into()), Printed::Out("\n".into())]),
            _ => panic!("no result"),
        }
        assert!(matches!(console(&mut d, "if True:"), Some(ConsoleResult::More)));
        let c = complete(&mut d, "bpy.ops.mesh.primitive_cu").unwrap();
        assert!(c.iter().any(|s| s.starts_with("bpy.ops.mesh.primitive_cube_add")), "{c:?}");
    }

    #[test]
    fn every_python_template_runs() {
        if !have_python() {
            return;
        }
        for (label, file, src) in crate::editors::text::TEMPLATES.iter().filter(|t| t.1.ends_with(".py")) {
            let mut d = Doc::default();
            d.python.dir = Some(std::env::temp_dir().join(format!("polyamour-tpl-{}", std::process::id())));
            run_script(&mut d, file, src);
            let errors: Vec<&str> = d.info.reports.iter().filter(|r| r.kind == ReportKind::Error).map(|r| r.text.as_str()).collect();
            assert!(errors.is_empty(), "template {label}: {errors:?}\n{:?}", d.info.reports);
        }
    }

    #[test]
    fn script_panel_in_the_sidebar_and_properties() {
        if !have_python() {
            return;
        }
        use crate::app::App;
        use crate::editors::{Editor, EditorKind};
        use crate::input::{Input, Key};
        let mut app = App::new(1600, 900);
        app.doc.python.dir = Some(std::env::temp_dir().join(format!("polyamour-side-{}", std::process::id())));
        let src = crate::editors::text::TEMPLATES.iter().find(|t| t.1 == "ui_panel_simple.py").unwrap().2;
        run_script(&mut app.doc, "ui_panel_simple.py", src);
        assert_eq!(app.doc.python.sidebar_categories(), ["Tool Kit"]);
        // Open the sidebar of the 3D View and its script tab.
        let ws = &mut app.workspaces[app.active_ws];
        let area = ws.areas_mut().into_iter().find(|a| a.editor.kind() == EditorKind::View3D).unwrap();
        let r = area.rect;
        if let Editor::View3D(s) = &mut area.editor {
            s.show_sidebar = true;
        }
        app.doc.python.side_tab = Some("Tool Kit".into());
        app.frame(Input::Move { x: r.x + r.w / 2, y: r.y + r.h / 2 });
        let j = app.doc.python.layouts.get("VIEW3D_PT_tool_kit").map(|l| l.1.clone()).expect("the sidebar drew the panel");
        assert_eq!(j["layout"]["items"][0]["items"][0]["value"], 5);
        // A property change from the panel, then its button.
        let rf = j["layout"]["items"][0]["items"][0]["ref"].clone();
        app.doc.queue.push(set_prop_op("VIEW3D_PT_tool_kit", &rf, Value::from(3)));
        app.frame(Input::Tick);
        let n = app.doc.scene.objects.len();
        app.doc.queue.push(Op::with("script.operator", r#"{"id": "object.tool_kit_ring", "props": {}}"#));
        app.frame(Input::Tick);
        assert_eq!(app.doc.scene.objects.len(), n + 3, "{:?}", app.doc.info.reports);
        app.frame(Input::key_mods(Key::Char('z'), crate::input::Mods::CTRL));
        assert_eq!(app.doc.scene.objects.len(), n);
        // The Properties panel for the object tab.
        assert!(app.doc.python.panels.iter().any(|p| p.space == "PROPERTIES" && p.context == "object"));
    }

    #[test]
    fn addon_operator_reaches_search_panels_and_preferences() {
        if !have_python() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("polyamour-data-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("addons")).unwrap();
        std::fs::write(
            dir.join("addons/nudge.py"),
            "bl_info = {'name': 'Nudge', 'category': 'Object', 'version': (0, 1), 'blender': (4, 2, 0)}\n\
             import bpy\n\
             class OBJECT_OT_nudge(bpy.types.Operator):\n    \"\"\"Move the active object up\"\"\"\n    bl_idname = 'object.nudge'\n    bl_label = 'Nudge Up'\n    step: bpy.props.FloatProperty(default=1.0)\n    def execute(self, context):\n        context.active_object.location.z += self.step\n        return {'FINISHED'}\n\
             class VIEW3D_PT_nudge(bpy.types.Panel):\n    bl_space_type = 'VIEW_3D'\n    bl_region_type = 'UI'\n    bl_category = 'Nudge'\n    bl_label = 'Nudge'\n    def draw(self, context):\n        self.layout.operator('object.nudge')\n\
             def register():\n    bpy.utils.register_class(OBJECT_OT_nudge)\n    bpy.utils.register_class(VIEW3D_PT_nudge)\n\
             def unregister():\n    bpy.utils.unregister_class(VIEW3D_PT_nudge)\n    bpy.utils.unregister_class(OBJECT_OT_nudge)\n",
        )
        .unwrap();
        let mut d = Doc::default();
        d.python.dir = Some(dir.clone());
        scan_addons(&mut d);
        assert_eq!(d.python.addons.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Nudge"]);
        d.run(&Op::with("script.addon_toggle", "nudge"));
        assert!(d.python.enabled.contains("nudge"));
        assert!(std::fs::read_to_string(dir.join("addons_enabled.txt")).unwrap().contains("nudge"));
        assert_eq!(d.python.sidebar_categories(), ["Nudge"]);
        let items = search_items(&d);
        assert!(matches!(&items[..], [MenuItem::Op(label, _, _)] if label == "Nudge Up"), "{items:?}");
        let z = d.scene.objects[d.scene.find_object("Cube").unwrap()].transform.location.z;
        d.run(&Op::with("script.operator", "object.nudge"));
        let cube = &d.scene.objects[d.scene.find_object("Cube").unwrap()];
        assert!((cube.transform.location.z - z - 1.0).abs() < 1e-5, "{:?}", d.info.reports);
        let j = layout(&mut d, "VIEW3D_PT_nudge").unwrap();
        assert_eq!(j["layout"]["items"][0]["operator"], "object.nudge");
        // A new session enables it again from the saved list.
        let mut d2 = Doc::default();
        d2.python.dir = Some(dir.clone());
        tick(&mut d2);
        assert!(d2.python.operators.iter().any(|o| o.id == "object.nudge"));
        d2.run(&Op::with("script.addon_toggle", "nudge"));
        assert!(d2.python.operators.is_empty());
        assert!(!std::fs::read_to_string(dir.join("addons_enabled.txt")).unwrap().contains("\nnudge"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
