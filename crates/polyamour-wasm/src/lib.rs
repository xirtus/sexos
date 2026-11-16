// SPDX-License-Identifier: GPL-3.0-only
//! `polyamour-wasm`: capability-confined WebAssembly add-on host and interactive
//! `polyops 1` session runner for Polyamour (`crates/sex-3d` + `crates/sex-wasm`).
//!
//! Keeps Polyamour's existing out-of-process Python `import bpy` support intact
//! while executing compute-heavy or third-party pure-Rust `.wasm` add-ons in-process
//! on `wasmi` with:
//! - Deterministic WASM instruction fuel limits and linear memory caps (`AddonConfig`).
//! - Per-run host-side limits (`SessionLimits`: command count, created datablocks,
//!   cumulative reply bytes, and pre/post-modifier mesh vertex/face caps) plus
//!   diagnostic log byte caps so host calls cannot bypass WASM fuel/memory limits.
//! - Zero real filesystem or network grants (`net: false`, virtualized in-RAM
//!   `polyamour/{scene,ops,replies}` channels + WASI `stdout`/`stdin` streaming).
//! - Atomic undoable scene transactions: any trap, fuel exhaustion, host limit
//!   breach, non-zero exit, incomplete stream, or `ScriptError` leaves `Scene` untouched.

use polyamour_sdk::{Client, SdkError, Transport};
use sex_3d::scene::Scene;
use sex_3d::script::{scene_to_polyops, RunReport, ScriptError, Session, SessionLimits};
use sex_wasm::{Grants, Guest, Host, Limits, Outcome, Refusal};
use std::collections::VecDeque;
use std::fmt;

/// Resource limits for a sandboxed Polyamour `.wasm` add-on execution, covering
/// both guest WASM execution and host-side `sex-3d` work.
#[derive(Clone, Debug)]
pub struct AddonConfig {
    /// Guest name (`argv[0]`).
    pub name: String,
    /// Optional command-line arguments (`key=value` panel overrides, etc.).
    pub args: Vec<String>,
    /// Maximum WASM linear memory in bytes (default: 8 MiB).
    pub memory_bytes: usize,
    /// Maximum `wasmi` fuel before terminating with [`AddonError::OutOfFuel`]
    /// (default: `50_000_000` instructions).
    pub fuel: Option<u64>,
    /// Maximum single host read/write buffer size (default: 2 MiB).
    pub io_bytes: usize,
    /// Host-side caps on commands, created datablocks, reply bytes, and mesh sizes.
    pub session_limits: SessionLimits,
    /// Maximum cumulative diagnostic `sex.log` bytes retained per run (default: 256 KiB).
    pub max_log_bytes: usize,
}

impl Default for AddonConfig {
    fn default() -> Self {
        Self {
            name: "polyamour-addon".into(),
            args: Vec::new(),
            memory_bytes: 8 << 20,
            fuel: Some(50_000_000),
            io_bytes: 2 << 20,
            session_limits: SessionLimits::sandboxed(),
            max_log_bytes: 256 << 10,
        }
    }
}

/// Successful execution summary of a `.wasm` or native Rust add-on.
#[derive(Clone, Debug, PartialEq)]
pub struct AddonRun {
    pub report: RunReport,
    pub logs: Vec<String>,
}

/// Failure modes of an add-on run. In every error case, the caller's [`Scene`]
/// is guaranteed to remain completely unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddonError {
    Script(ScriptError),
    Sdk(SdkError),
    IncompleteStream,
    ExitedNonZero(i32),
    OutOfFuel,
    Trapped(String),
    Rejected(String),
}

impl fmt::Display for AddonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Script(e) => write!(f, "polyops error at {e}"),
            Self::Sdk(e) => write!(f, "sdk error: {e}"),
            Self::IncompleteStream => f.write_str("add-on exited without closing polyops stream with 'end'"),
            Self::ExitedNonZero(code) => write!(f, "add-on exited with status {code}"),
            Self::OutOfFuel => f.write_str("add-on exceeded its instruction fuel budget"),
            Self::Trapped(why) => write!(f, "add-on trapped: {why}"),
            Self::Rejected(why) => write!(f, "add-on module rejected: {why}"),
        }
    }
}

impl std::error::Error for AddonError {}

/// Direct in-process [`Transport`] connecting [`polyamour_sdk::Client`] straight
/// to a working [`Scene`] and [`Session`] with zero intermediate files.
pub struct SessionTransport {
    work_scene: Scene,
    initial_snapshot: Option<String>,
    session: Session,
    replies: VecDeque<String>,
    all_replies: Vec<String>,
    logs: Vec<String>,
}

impl SessionTransport {
    pub fn new(scene: &Scene) -> Self {
        Self::with_limits(scene, SessionLimits::sandboxed())
    }

    pub fn with_limits(scene: &Scene, limits: SessionLimits) -> Self {
        Self {
            work_scene: scene.clone(),
            initial_snapshot: None,
            session: Session::with_limits(limits),
            replies: VecDeque::new(),
            all_replies: Vec::new(),
            logs: Vec::new(),
        }
    }

    fn finish_into(self, target: &mut Scene) -> Result<AddonRun, AddonError> {
        let mut report = self.session.finish().map_err(AddonError::Script)?;
        if !report.complete {
            return Err(AddonError::IncompleteStream);
        }
        report.replies = self.all_replies;
        *target = self.work_scene;
        Ok(AddonRun { report, logs: self.logs })
    }
}

impl Transport for SessionTransport {
    fn write_line(&mut self, line: &str) -> Result<(), SdkError> {
        self.session
            .exec_line(&mut self.work_scene, line)
            .map_err(|e| SdkError::Protocol(e.to_string()))?;
        for r in self.session.take_replies() {
            self.all_replies.push(r.clone());
            self.replies.push_back(r);
        }
        Ok(())
    }

    fn read_reply(&mut self) -> Result<String, SdkError> {
        self.replies
            .pop_front()
            .ok_or_else(|| SdkError::Transport("no reply produced by Session".into()))
    }

    fn read_scene_snapshot(&mut self) -> Result<Option<String>, SdkError> {
        let snap = self
            .initial_snapshot
            .get_or_insert_with(|| scene_to_polyops(&self.work_scene));
        Ok(Some(snap.clone()))
    }

    fn log(&mut self, message: &str) {
        self.logs.push(message.to_string());
    }
}

/// Run a pure-Rust procedural closure built on [`polyamour_sdk::Client`] against
/// `scene` atomically: commits on `Ok(())` and rolls back on any error.
pub fn run_rust_addon<F>(scene: &mut Scene, f: F) -> Result<AddonRun, AddonError>
where
    F: FnOnce(&mut Client<SessionTransport>) -> Result<(), SdkError>,
{
    let transport = SessionTransport::new(scene);
    let mut client = Client::open(transport).map_err(AddonError::Sdk)?;
    match f(&mut client) {
        Ok(()) => {
            let transport = client.finish().map_err(AddonError::Sdk)?;
            transport.finish_into(scene)
        }
        Err(err) => {
            let _ = client.abort(&err.to_string());
            Err(AddonError::Sdk(err))
        }
    }
}

// ---------------------------------------------------------------------------
// Sandboxed WASM Host (`sex-wasm` / `wasmi`)

const VIRTUAL_SCENE: &[u8] = b"polyamour/scene";
const VIRTUAL_OPS: &[u8] = b"polyamour/ops";
const VIRTUAL_REPLIES: &[u8] = b"polyamour/replies";

struct PolyopsWasmHost {
    initial_scene: Scene,
    scene_bytes: Option<Vec<u8>>,
    scene_cursor: usize,
    work_scene: Scene,
    session: Session,
    ops_line_buf: Vec<u8>,
    reply_stream: VecDeque<u8>,
    all_replies: Vec<String>,
    logs: Vec<String>,
    log_bytes: usize,
    max_log_bytes: usize,
    max_line_bytes: usize,
    script_error: Option<ScriptError>,
}

impl PolyopsWasmHost {
    fn new(scene: &Scene, config: &AddonConfig) -> Self {
        Self {
            initial_scene: scene.clone(),
            scene_bytes: None,
            scene_cursor: 0,
            work_scene: scene.clone(),
            session: Session::with_limits(config.session_limits),
            ops_line_buf: Vec::new(),
            reply_stream: VecDeque::new(),
            all_replies: Vec::new(),
            logs: Vec::new(),
            log_bytes: 0,
            max_log_bytes: config.max_log_bytes,
            max_line_bytes: config.io_bytes.max(4096),
            script_error: None,
        }
    }

    fn feed_ops_bytes(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        if self.script_error.is_some() {
            return Err(Refusal::Failed);
        }
        if self.ops_line_buf.len().saturating_add(bytes.len()) > self.max_line_bytes {
            self.script_error = Some(ScriptError {
                line: 0,
                message: format!("polyops command line exceeds {} bytes", self.max_line_bytes),
            });
            return Err(Refusal::TooLarge);
        }
        self.ops_line_buf.extend_from_slice(bytes);
        while let Some(nl) = self.ops_line_buf.iter().position(|&b| b == b'\n') {
            let raw_line: Vec<u8> = self.ops_line_buf.drain(..=nl).collect();
            let without_nl = &raw_line[..raw_line.len() - 1];
            let line = std::str::from_utf8(without_nl)
                .map_err(|_| {
                    self.script_error = Some(ScriptError {
                        line: 0,
                        message: "polyops stream contains invalid UTF-8".into(),
                    });
                    Refusal::Invalid
                })?
                .trim_end_matches('\r');
            if let Err(err) = self.session.exec_line(&mut self.work_scene, line) {
                self.script_error = Some(err);
                return Err(Refusal::Failed);
            }
            for reply in self.session.take_replies() {
                self.reply_stream.extend(reply.as_bytes());
                self.reply_stream.push_back(b'\n');
                self.all_replies.push(reply);
            }
        }
        Ok(())
    }

    fn flush_trailing_line(&mut self) {
        if self.script_error.is_none() && !self.ops_line_buf.is_empty() {
            let trailing = std::mem::take(&mut self.ops_line_buf);
            let _ = self.feed_ops_bytes(&[trailing.as_slice(), b"\n"].concat());
        }
    }

    fn drain_replies(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.reply_stream.len());
        for slot in &mut out[..count] {
            *slot = self.reply_stream.pop_front().unwrap();
        }
        count
    }
}

impl Host for PolyopsWasmHost {
    fn show(&mut self, _text: &[u8]) {}

    fn log(&mut self, line: &[u8]) {
        if self.log_bytes >= self.max_log_bytes {
            return;
        }
        let room = self.max_log_bytes - self.log_bytes;
        let slice = &line[..line.len().min(room)];
        self.log_bytes += slice.len();
        self.logs.push(String::from_utf8_lossy(slice).into_owned());
    }

    /// Intercepts WASI `stdout` (`fd == 1`) so plain `wasm32-wasip1` plugins can
    /// emit `polyops 1` lines directly to standard output.
    fn console(&mut self, bytes: &[u8]) -> bool {
        let _ = self.feed_ops_bytes(bytes);
        true
    }

    /// Supplies `@ID ok ...` replies to WASI `stdin` (`fd == 0`) when queued.
    fn input(&mut self, out: &mut [u8]) -> Option<usize> {
        Some(self.drain_replies(out))
    }

    fn read(&mut self, path: &[u8], out: &mut [u8]) -> Result<usize, Refusal> {
        match path {
            VIRTUAL_SCENE => {
                let data = self
                    .scene_bytes
                    .get_or_insert_with(|| scene_to_polyops(&self.initial_scene).into_bytes());
                if self.scene_cursor >= data.len() {
                    return Ok(0);
                }
                let rem = &data[self.scene_cursor..];
                let n = out.len().min(rem.len());
                out[..n].copy_from_slice(&rem[..n]);
                self.scene_cursor += n;
                Ok(n)
            }
            VIRTUAL_REPLIES => Ok(self.drain_replies(out)),
            _ => Err(Refusal::NotFound),
        }
    }

    fn write(&mut self, path: &[u8], data: &[u8]) -> Result<(), Refusal> {
        match path {
            VIRTUAL_OPS => self.feed_ops_bytes(data),
            _ => Err(Refusal::NotAllowed),
        }
    }
}

/// Validate and run a compiled `.wasm` Polyamour add-on inside `wasmi` (`sex-wasm`)
/// against `scene` as a single atomic, fuel- and host-resource-bounded step.
pub fn run_wasm_addon(scene: &mut Scene, wasm: &[u8], config: &AddonConfig) -> Result<AddonRun, AddonError> {
    let arg_slices: Vec<&[u8]> = config.args.iter().map(|s| s.as_bytes()).collect();
    let guest = Guest {
        name: config.name.as_bytes(),
        args: &arg_slices,
        grants: Grants {
            folder: Some(b"polyamour".to_vec()),
            net: false,
        },
        limits: Limits {
            memory_bytes: config.memory_bytes,
            fuel: config.fuel,
            io_bytes: config.io_bytes,
            console_bytes: 4096,
        },
    };

    let (outcome, mut host) = sex_wasm::run(wasm, &guest, PolyopsWasmHost::new(scene, config));
    host.flush_trailing_line();

    if let Some(err) = host.script_error {
        return Err(AddonError::Script(err));
    }

    match outcome {
        Outcome::Exited(0) => {}
        Outcome::Exited(code) => return Err(AddonError::ExitedNonZero(code)),
        Outcome::OutOfFuel => return Err(AddonError::OutOfFuel),
        Outcome::Trapped(why) => return Err(AddonError::Trapped(why)),
        Outcome::Rejected(why) => return Err(AddonError::Rejected(why)),
        Outcome::Closed => return Err(AddonError::IncompleteStream),
    }

    let mut report = host.session.finish().map_err(AddonError::Script)?;
    if !report.complete {
        return Err(AddonError::IncompleteStream);
    }
    report.replies = host.all_replies;
    *scene = host.work_scene;
    Ok(AddonRun {
        report,
        logs: host.logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use polyamour_sdk::{
        Color, Euler, LightSpec, MaterialParams, MeshData, ModifierSpec, Placement, Primitive,
        UiPropKind, UiPropSpec, Vec3,
    };
    use sex_3d::scene::ObjectData;
    use sex_3d::script::AddonPropKind;

    /// Real `#![no_std]` Rust guest compiled for `wasm32-unknown-unknown` from
    /// `crates/polyamour-wasm/guests/spiral_rust`.
    const SPIRAL_RUST_WASM: &[u8] = include_bytes!("../tests/spiral_rust.wasm");

    #[test]
    fn compiled_rust_wasm_guest_runs_and_rebuilds_from_panel_args() {
        let mut scene = Scene::startup();

        // 1. Default run (`steps=12`, `radius=1.5`, `rise=0.2`)
        let run1 = run_wasm_addon(&mut scene, SPIRAL_RUST_WASM, &AddonConfig::default())
            .expect("compiled spiral_rust.wasm should run cleanly");
        assert!(run1.report.complete);
        assert_eq!(run1.report.panels.len(), 1);
        assert_eq!(run1.report.panels[0].id, "spiral_rust");
        assert_eq!(run1.report.panels[0].category, "WASM");
        assert_eq!(run1.report.panels[0].props.len(), 4);
        assert_eq!(
            run1.logs,
            vec!["spiral_rust: steps=12 radius=1.5 step0_eval_faces=24 rail_verts=73"]
        );
        assert!(scene.find_object("WasmPole").is_some());
        assert!(scene.find_object("WasmStep11").is_some());
        let rail = scene.find_object("WasmHandrail").expect("handrail created");
        let ObjectData::Mesh(mi) = scene.objects[rail].data else { panic!("expected mesh") };
        assert_eq!(scene.meshes[mi].positions.len(), 73);

        // 2. Re-run with sidebar UI overrides (`steps=6`, `radius=2.2`): replaces `WasmPole`
        let cfg2 = AddonConfig {
            args: vec!["steps=6".into(), "radius=2.2".into()],
            ..AddonConfig::default()
        };
        let run2 = run_wasm_addon(&mut scene, SPIRAL_RUST_WASM, &cfg2)
            .expect("spiral_rust.wasm rebuild with args should succeed");
        assert_eq!(
            run2.logs,
            vec!["spiral_rust: steps=6 radius=2.2 step0_eval_faces=24 rail_verts=37"]
        );
        assert!(scene.find_object("WasmPole").is_some());
    }

    #[test]
    fn host_compute_and_memory_limits_stop_resource_exhaustion() {
        let mut scene = Scene::startup();
        let before = scene.clone();

        // 1. WASM loop spamming `mesh_get Cube evaluated=1` (draining replies each iteration!)
        //    must hit `max_reply_bytes` and leave Scene untouched.
        let spam_mesh_get_wat = r#"
        (module
            (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (import "sex" "read"  (func $read  (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 2)
            (data (i32.const 0)  "ops")
            (data (i32.const 8)  "replies")
            (data (i32.const 32) "polyops 1\nmodifier_add Cube Subdivision levels=2\n")
            (data (i32.const 96) "@q mesh_get Cube evaluated=1\n")
            (func (export "_start")
                (local $i i32)
                (drop (call $write (i32.const 0) (i32.const 3) (i32.const 32) (i32.const 49)))
                (loop $lp
                    (if (i32.lt_s (call $write (i32.const 0) (i32.const 3) (i32.const 96) (i32.const 29)) (i32.const 0))
                        (then return)
                    )
                    ;; Drain reply buffer so reply_stream doesn't grow, testing cumulative reply_bytes!
                    (drop (call $read (i32.const 8) (i32.const 7) (i32.const 512) (i32.const 8192)))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $lp (i32.lt_u (local.get $i) (i32.const 200)))
                )
            )
        )
        "#;
        let spam_wasm = wat::parse_str(spam_mesh_get_wat).unwrap();
        let tight_reply_cfg = AddonConfig {
            session_limits: SessionLimits {
                max_reply_bytes: Some(8 * 1024),
                ..SessionLimits::sandboxed()
            },
            ..AddonConfig::default()
        };
        let err = run_wasm_addon(&mut scene, &spam_wasm, &tight_reply_cfg).unwrap_err();
        assert!(
            matches!(&err, AddonError::Script(e) if e.message.contains("reply output limit exceeded")),
            "expected reply output cap error, got {err:?}"
        );
        assert_eq!(scene, before, "failed run must roll back Scene");

        // 2. Stacked Subdivision modifiers (`6 * 4^6 * 4^6` = 100M faces) must be refused
        //    BEFORE `scene.evaluated_mesh` runs!
        let bomb_wat = r#"
        (module
            (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0)  "ops")
            (data (i32.const 16) "polyops 1\nmodifier_add Cube Subdivision levels=6\nmodifier_add Cube Subdivision levels=6\n@q mesh_get Cube evaluated=1\nend\n")
            (func (export "_start")
                (drop (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 121)))
            )
        )
        "#;
        let bomb_wasm = wat::parse_str(bomb_wat).unwrap();
        let err = run_wasm_addon(&mut scene, &bomb_wasm, &AddonConfig::default()).unwrap_err();
        assert!(
            matches!(&err, AddonError::Script(e) if e.message.contains("exceeding limit")),
            "expected mesh limit error, got {err:?}"
        );
        assert_eq!(scene, before);

        // 3. Loop creating objects must hit `max_created_objects` and roll back Scene.
        let create_loop_wat = r#"
        (module
            (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0)  "ops")
            (data (i32.const 16) "polyops 1\n")
            (data (i32.const 32) "add_empty E\n")
            (func (export "_start")
                (local $i i32)
                (drop (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 10)))
                (loop $lp
                    (if (i32.lt_s (call $write (i32.const 0) (i32.const 3) (i32.const 32) (i32.const 12)) (i32.const 0))
                        (then return)
                    )
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $lp (i32.lt_u (local.get $i) (i32.const 100)))
                )
            )
        )
        "#;
        let create_wasm = wat::parse_str(create_loop_wat).unwrap();
        let tight_obj_cfg = AddonConfig {
            session_limits: SessionLimits {
                max_created_objects: Some(10),
                ..SessionLimits::sandboxed()
            },
            ..AddonConfig::default()
        };
        let err = run_wasm_addon(&mut scene, &create_wasm, &tight_obj_cfg).unwrap_err();
        assert!(
            matches!(&err, AddonError::Script(e) if e.message.contains("created datablock limit exceeded")),
            "expected datablock cap error, got {err:?}"
        );
        assert_eq!(scene, before);

        // 4. Command spam must hit `max_commands` and roll back Scene.
        let cmd_loop_wat = r#"
        (module
            (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0)  "ops")
            (data (i32.const 16) "polyops 1\n")
            (data (i32.const 32) "select_all\n")
            (func (export "_start")
                (local $i i32)
                (drop (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 10)))
                (loop $lp
                    (if (i32.lt_s (call $write (i32.const 0) (i32.const 3) (i32.const 32) (i32.const 11)) (i32.const 0))
                        (then return)
                    )
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $lp (i32.lt_u (local.get $i) (i32.const 100)))
                )
            )
        )
        "#;
        let cmd_wasm = wat::parse_str(cmd_loop_wat).unwrap();
        let tight_cmd_cfg = AddonConfig {
            session_limits: SessionLimits {
                max_commands: Some(15),
                ..SessionLimits::sandboxed()
            },
            ..AddonConfig::default()
        };
        let err = run_wasm_addon(&mut scene, &cmd_wasm, &tight_cmd_cfg).unwrap_err();
        assert!(
            matches!(&err, AddonError::Script(e) if e.message.contains("command limit exceeded")),
            "expected command cap error, got {err:?}"
        );
        assert_eq!(scene, before);
    }

    #[test]
    fn rust_sdk_spiral_staircase_and_live_evaluated_mesh_query() {
        let mut scene = Scene::new();
        let run = run_rust_addon(&mut scene, |pa| {
            pa.ui_panel("spiral_stairs", "Spiral Staircase (Rust)", "Addons")?;
            pa.ui_prop(
                "spiral_stairs",
                &UiPropSpec {
                    key: "steps".into(),
                    kind: UiPropKind::Int,
                    label: "Steps".into(),
                    default: Some("12".into()),
                    min: Some(3.0),
                    max: Some(128.0),
                },
            )?;

            let stairs = pa.collection_new("Staircase", None)?;
            let brass = pa.material_new(
                "Brass",
                &MaterialParams {
                    base_color: Color::rgba(0.9, 0.7, 0.3, 1.0),
                    metallic: 1.0,
                    roughness: 0.2,
                    ..MaterialParams::default()
                },
            )?;

            let pole = pa.add_primitive(
                Primitive::Cylinder { segments: 24, radius: 0.15, depth: 3.0 },
                &Placement::at(Vec3::new(0.0, 0.0, 1.5))
                    .named("Pole")
                    .in_collection(&stairs),
            )?;
            pa.material_assign(&pole, &brass, None)?;

            let step0 = pa.add_primitive(
                Primitive::Cube { size: 1.0 },
                &Placement::at(Vec3::new(1.5, 0.0, 0.2))
                    .named("Step00")
                    .with_rotation(Euler::new(0.0, 0.0, 0.0))
                    .with_scale(Vec3::new(1.2, 0.4, 0.08))
                    .in_collection(&stairs),
            )?;
            pa.set_parent(&step0, Some(&pole), true)?;
            pa.modifier_add(&step0, ModifierSpec::Subdivision { levels: 2, simple: false })?;

            let base_mesh = pa.mesh_get(&step0, false)?;
            let eval_mesh = pa.mesh_get(&step0, true)?;
            assert_eq!(base_mesh.vertex_count(), 8);
            assert_eq!(base_mesh.face_count(), 6);
            assert_eq!(eval_mesh.face_count(), 96);

            let verts: Vec<Vec3> = (0..97)
                .map(|k| {
                    let a = k as f32 * (std::f32::consts::TAU / 48.0);
                    Vec3::new(2.05 * a.cos(), 2.05 * a.sin(), k as f32 * 0.05 + 0.9)
                })
                .collect();
            let edges: Vec<[u32; 2]> = (0..96).map(|k| [k, k + 1]).collect();
            let rail_data = MeshData::from_pydata(&verts, &edges, &[]).map_err(SdkError::Wire)?;
            let rail = pa.add_mesh_object("Handrail", &rail_data, &Placement::default().in_collection(&stairs))?;
            pa.material_assign(&rail, &brass, None)?;

            pa.add_light(
                LightSpec::Area { power: 500.0, color: Color::WHITE, size: 3.0 },
                &Placement::at(Vec3::new(0.0, 0.0, 6.0)).named("KeyLight"),
            )?;
            pa.add_camera(None, true, &Placement::at(Vec3::new(8.0, -8.0, 5.0)).named("MainCam"))?;
            pa.log("built spiral staircase in pure Rust");
            Ok(())
        })
        .expect("rust addon succeeded");

        assert!(run.report.complete);
        assert_eq!(run.report.panels.len(), 1);
        assert_eq!(run.report.panels[0].props[0].kind, AddonPropKind::Int);
        assert_eq!(run.logs, vec!["built spiral staircase in pure Rust"]);
        assert!(scene.find_object("Pole").is_some());
        assert!(scene.find_object("Step00").is_some());
        let rail_idx = scene.find_object("Handrail").expect("Handrail exists");
        let ObjectData::Mesh(mi) = scene.objects[rail_idx].data else { panic!("expected mesh") };
        assert_eq!(scene.meshes[mi].positions.len(), 97);
        assert_eq!(scene.meshes[mi].loose_edges.len(), 96);
    }

    #[test]
    fn wasm_trap_and_out_of_fuel_leave_scene_untouched() {
        let mut scene = Scene::startup();
        let before = scene.clone();

        let trap_wat = r#"
        (module
            (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0)  "ops")
            (data (i32.const 16) "polyops 1\ndelete Cube\n")
            (func (export "_start")
                (drop (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 22)))
                unreachable
            )
        )
        "#;
        let trap_wasm = wat::parse_str(trap_wat).unwrap();
        let err = run_wasm_addon(&mut scene, &trap_wasm, &AddonConfig::default()).unwrap_err();
        assert!(matches!(err, AddonError::Trapped(_)), "expected trap, got {err:?}");
        assert_eq!(scene, before, "trapped WASM add-on must not mutate Scene");

        let loop_wat = r#"
        (module
            (import "sex" "write" (func $write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0)  "ops")
            (data (i32.const 16) "polyops 1\ndelete Cube\n")
            (func (export "_start")
                (drop (call $write (i32.const 0) (i32.const 3) (i32.const 16) (i32.const 22)))
                (loop $inf br $inf)
            )
        )
        "#;
        let loop_wasm = wat::parse_str(loop_wat).unwrap();
        let low_fuel = AddonConfig { fuel: Some(10_000), ..AddonConfig::default() };
        let err = run_wasm_addon(&mut scene, &loop_wasm, &low_fuel).unwrap_err();
        assert_eq!(err, AddonError::OutOfFuel);
        assert_eq!(scene, before, "out-of-fuel WASM add-on must not mutate Scene");
    }
}
