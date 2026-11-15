// SPDX-License-Identifier: GPL-3.0-only
//! The live channel: a long-running Python process (the *kernel*,
//! `apps/polyamour/python/polyamour/kernel.py`) that runs scripts, console
//! lines, add-on code, panel `draw()`s and handlers, and asks Polyamour for
//! everything it reads or changes with polyops requests, one line each way.
//!
//! ```text
//! host   -> python   call 7 exec "script.py" "import bpy\n..."
//! python -> host     @1 op mesh.primitive_cube_add size=2
//! host   -> python   @1 ok FINISHED
//! python -> host     @2 ids
//! host   -> python   @2 ok =json {"objects":["Cube"],...}
//! python -> host     out "made Cube\n"
//! python -> host     done 7 ok
//! ```
//!
//! * A **call** (`call ID KIND ARGS...`) is the host asking Python to do
//!   something; Python answers with `done ID ok [ARGS...]`, `done ID json
//!   {...}` or `done ID error MESSAGE [line=N] [traceback=TEXT]`. Kinds: `exec
//!   NAME SOURCE`, `console SOURCE`, `complete TEXT`, `draw PANEL`,
//!   `operator IDNAME invoke|execute|poll PROPS-JSON`, `handler NAME [ARGS]`,
//!   `timers`, `addon_enable MODULE`, `addon_disable MODULE`, `addons DIR`,
//!   `set_prop PATH VALUE`, `menu MENU`, `reset`.
//! * While a call runs, Python sends **requests** (`@N` + any polyops
//!   command); each gets exactly one reply (`@N ok ...` or `@N err MESSAGE`)
//!   before Python continues, so a script reads back what it just did.
//! * `out TEXT` / `err TEXT` carry what the script printed (standard output
//!   and standard error), in order with the requests.
//!
//! The request lines are executed by one [`Session`] that lives as long as the
//! kernel, so handles and the [`super::bpy::State`] persist across calls; a
//! [`Host`] gets the first look at each request (the app overrides file and
//! render operators). Undo is the caller's: snapshot before [`Kernel::call`],
//! push one step when the scene changed.
//!
//! The transport is any [`Wire`]: on a host, pipes to a `python3` child. On
//! SexOS `std::process` has no pipes yet, so [`Kernel::spawn`] fails with
//! `Unsupported` there and Polyamour falls back to the one-way batch path.
use super::{quote, tokenize, Scene, ScriptError, Session};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

/// A line transport to the kernel.
pub trait Wire {
    fn send(&mut self, line: &str) -> io::Result<()>;
    /// The next line, without its newline; `None` at end of stream.
    fn recv(&mut self) -> io::Result<Option<String>>;
    /// What the other side wrote outside the protocol (a crash report).
    fn diagnostics(&mut self) -> String {
        String::new()
    }
    fn close(&mut self) {}
}

/// Pipes to a child process: requests on its stdout, calls on its stdin.
pub struct PipeWire {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    stderr: Arc<Mutex<String>>,
}

impl Wire for PipeWire {
    fn send(&mut self, line: &str) -> io::Result<()> {
        let w = self.stdin.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "the kernel is closed"))?;
        w.write_all(line.as_bytes())?;
        w.write_all(b"\n")?;
        w.flush()
    }
    fn recv(&mut self) -> io::Result<Option<String>> {
        let mut s = String::new();
        if self.stdout.read_line(&mut s)? == 0 {
            return Ok(None);
        }
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        Ok(Some(s))
    }
    fn diagnostics(&mut self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }
    fn close(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        self.close();
    }
}

/// How to start the kernel.
#[derive(Clone, Debug)]
pub struct KernelConfig {
    pub program: String,
    /// Folders put first on `sys.path` (the folder holding `polyamour`, `bpy`...).
    pub paths: Vec<String>,
    /// Add-on folder scanned by `addons`.
    pub addons: Option<String>,
    pub env: Vec<(String, String)>,
}

impl KernelConfig {
    /// `python3` with the package from this source tree (host builds, tests).
    pub fn host() -> KernelConfig {
        let pkg = std::env::var("POLYAMOUR_PYTHON_PATH").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/polyamour/python").to_string());
        KernelConfig { program: std::env::var("POLYAMOUR_PYTHON").unwrap_or_else(|_| "python3".into()), paths: vec![pkg], addons: None, env: Vec::new() }
    }
}

/// One printed chunk: standard output or standard error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Printed {
    Out(String),
    Err(String),
}

/// A failed call: the Python exception (or a dead kernel).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallError {
    pub message: String,
    /// The line of the script that raised, when the error came from one.
    pub line: Option<usize>,
    pub traceback: String,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// What a call returned.
#[derive(Clone, Debug, PartialEq)]
pub struct CallResult {
    pub printed: Vec<Printed>,
    /// `done ID ok ...` tokens, or `done ID json` as one JSON text.
    pub result: Result<Vec<String>, CallError>,
    /// `json` answers.
    pub json: Option<serde_json::Value>,
    /// Requests the call made.
    pub requests: usize,
}

impl Default for CallError {
    fn default() -> Self {
        CallError { message: String::new(), line: None, traceback: String::new() }
    }
}

impl Default for CallResult {
    fn default() -> Self {
        CallResult { printed: Vec::new(), result: Ok(Vec::new()), json: None, requests: 0 }
    }
}

impl CallResult {
    /// Printed text joined (standard output and error together, in order).
    pub fn text(&self) -> String {
        self.printed.iter().map(|p| match p {
            Printed::Out(s) | Printed::Err(s) => s.as_str(),
        }).collect()
    }
    pub fn ok(&self) -> bool {
        self.result.is_ok()
    }
}

/// The app's hook into requests: return `Some` to answer a request yourself
/// (`Ok(reply)` becomes `@N ok reply`), `None` to let the [`Session`] run it.
pub trait Host {
    fn request(&mut self, _scene: &mut Scene, _session: &mut Session, _command: &str, _line: &str) -> Option<Result<Option<String>, String>> {
        None
    }
}

/// A host that lets the session run everything.
pub struct NoHost;
impl Host for NoHost {}

/// The kernel process and the session its requests run in.
pub struct Kernel {
    wire: Box<dyn Wire>,
    pub session: Session,
    next: u64,
    /// The kernel's greeting (`hello polyops 1 python=3.13.18 ...`).
    pub hello: String,
    alive: bool,
}

impl Kernel {
    /// Start a kernel process. Fails with `Unsupported` where programs cannot
    /// be connected by pipes (SexOS today).
    pub fn spawn(cfg: &KernelConfig) -> io::Result<Kernel> {
        let mut boot = String::from("import sys\nsys.path[:0] = ");
        boot += &format!("{:?}", cfg.paths);
        boot += "\nimport polyamour.kernel as k\nk.main()\n";
        let mut cmd = Command::new(&cfg.program);
        cmd.arg("-u").arg("-I").arg("-c").arg(&boot).env("POLYAMOUR_LIVE", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(a) = &cfg.addons {
            cmd.env("POLYAMOUR_ADDONS", a);
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?);
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut e) = child.stderr.take() {
            let sink = stderr.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = e.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut s) = sink.lock() {
                        s.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if s.len() > 65536 {
                            let cut = s.len() - 32768;
                            let cut = (cut..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(s.len());
                            s.drain(..cut);
                        }
                    }
                }
            });
        }
        Kernel::with_wire(Box::new(PipeWire { child, stdin, stdout, stderr }))
    }

    /// A kernel over any transport; reads the greeting.
    pub fn with_wire(mut wire: Box<dyn Wire>) -> io::Result<Kernel> {
        let mut session = Session::new();
        let mut scratch = Scene::new();
        session.exec_line(&mut scratch, "polyops 1").map_err(|e| io::Error::other(e.to_string()))?;
        let hello = loop {
            match wire.recv()? {
                Some(l) if l.starts_with("hello ") => break l,
                Some(_) => continue,
                None => return Err(io::Error::other(format!("the Python kernel did not start: {}", wire.diagnostics().trim()))),
            }
        };
        Ok(Kernel { wire, session, next: 0, hello, alive: true })
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Ask the kernel to do `kind` with `args` (each sent as one token) and
    /// serve its requests against `scene` until it is done.
    pub fn call(&mut self, scene: &mut Scene, host: &mut dyn Host, kind: &str, args: &[&str]) -> CallResult {
        let mut res = CallResult { result: Ok(Vec::new()), ..Default::default() };
        if !self.alive {
            res.result = Err(CallError { message: "the Python kernel has stopped".into(), ..Default::default() });
            return res;
        }
        self.next += 1;
        let id = self.next;
        let mut line = format!("call {id} {kind}");
        for a in args {
            line.push(' ');
            line.push_str(&quote(a));
        }
        self.session.bpy.reports.clear();
        if let Err(e) = self.wire.send(&line) {
            return self.dead(res, &format!("cannot reach the Python kernel: {e}"));
        }
        loop {
            let l = match self.wire.recv() {
                Ok(Some(l)) => l,
                Ok(None) => return self.dead(res, "the Python kernel stopped"),
                Err(e) => return self.dead(res, &format!("the Python kernel stopped: {e}")),
            };
            if let Some(rest) = l.strip_prefix('@') {
                res.requests += 1;
                let (rid, body) = rest.split_once(' ').unwrap_or((rest, ""));
                let reply = self.request(scene, host, rid, body);
                if let Err(e) = self.wire.send(&reply) {
                    return self.dead(res, &format!("cannot reach the Python kernel: {e}"));
                }
                continue;
            }
            let toks = match tokenize(&l) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let Some(head) = toks.first() else { continue };
            match head.text.as_str() {
                "out" | "err" => {
                    let text = toks.get(1).map(|t| t.text.clone()).unwrap_or_default();
                    res.printed.push(if head.text == "out" { Printed::Out(text) } else { Printed::Err(text) });
                }
                "done" => {
                    if toks.get(1).map(|t| t.text.as_str()) != Some(&id.to_string()) {
                        continue;
                    }
                    match toks.get(2).map(|t| t.text.as_str()) {
                        Some("ok") => res.result = Ok(toks[3..].iter().map(|t| t.text.clone()).collect()),
                        Some("json") => {
                            let raw = l.splitn(4, ' ').nth(3).unwrap_or("null");
                            res.json = serde_json::from_str(raw).ok();
                            res.result = Ok(Vec::new());
                        }
                        _ => {
                            let mut e = CallError { message: toks.get(3).map(|t| t.text.clone()).unwrap_or_default(), ..Default::default() };
                            for t in &toks[3.min(toks.len())..] {
                                match t.key.as_deref() {
                                    Some("line") => e.line = t.text.parse().ok(),
                                    Some("traceback") => e.traceback = t.text.clone(),
                                    _ => {}
                                }
                            }
                            res.result = Err(e);
                        }
                    }
                    return res;
                }
                _ => {}
            }
        }
    }

    fn request(&mut self, scene: &mut Scene, host: &mut dyn Host, rid: &str, body: &str) -> String {
        let word = body.split_whitespace().next().unwrap_or("");
        if let Some(r) = host.request(scene, &mut self.session, word, body) {
            return match r {
                Ok(Some(v)) if !v.is_empty() => format!("@{rid} ok {v}"),
                Ok(_) => format!("@{rid} ok"),
                Err(e) => format!("@{rid} err {}", quote(&e)),
            };
        }
        self.session.take_replies();
        match self.session.exec_line(scene, &format!("@{rid} {body}")) {
            Ok(()) => self.session.take_replies().pop().unwrap_or_else(|| format!("@{rid} ok")),
            Err(ScriptError { message, .. }) => {
                self.session.take_replies();
                format!("@{rid} err {}", quote(&message))
            }
        }
    }

    fn dead(&mut self, mut res: CallResult, why: &str) -> CallResult {
        self.alive = false;
        let diag = self.wire.diagnostics();
        let tail: Vec<&str> = diag.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
        res.result = Err(CallError { message: why.to_string(), line: None, traceback: tail.join("\n") });
        res
    }

    /// End the process.
    pub fn shutdown(&mut self) {
        if self.alive {
            let _ = self.wire.send("quit");
        }
        self.alive = false;
        self.wire.close();
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Run one script on `scene` in a fresh kernel, all or nothing: on error the
/// scene is left as it was. The convenience behind tests and one-shot tools.
pub fn run_script(scene: &mut Scene, name: &str, source: &str) -> io::Result<CallResult> {
    let mut k = Kernel::spawn(&KernelConfig::host())?;
    let mut work = scene.clone();
    let r = k.call(&mut work, &mut NoHost, "exec", &[name, source]);
    if r.ok() {
        *scene = work;
    }
    Ok(r)
}

// ---------------------------------------------------------------- batch

/// The one-way fallback where programs cannot be connected by pipes (SexOS
/// today): Python runs the script on its own, writes the commands to a file,
/// and Polyamour applies them when the process has ended. Requests get no
/// replies, so reads come from a snapshot taken before the run: the names
/// (`ids`), the scene settings, the operator list and every object and
/// material dump, as they were when the script started.
///
/// Nothing is passed in the environment (SexOS programs have none): the boot
/// code gets the command file, the snapshot and the script as arguments.
///
/// ```text
/// python -I -c BOOT OPS-FILE SNAPSHOT-FILE SCRIPT.py
/// ```
pub fn batch_snapshot(scene: &Scene) -> String {
    /// Bigger meshes are left out of the snapshot (reading them fails).
    const BATCH_MESH_LIMIT: usize = 100_000;
    let mut work = scene.clone();
    let mut session = Session::new();
    let _ = session.exec_line(&mut work, "polyops 1");
    let mut keys: Vec<Vec<String>> = vec![vec!["ids".into()], vec!["dump_scene".into()], vec!["op_list".into()]];
    for o in &scene.objects {
        keys.push(vec!["dump_object".into(), o.name.clone()]);
    }
    for m in &scene.materials {
        keys.push(vec!["dump_material".into(), m.name.clone()]);
        keys.push(vec!["dump_tree".into(), "material".into(), m.name.clone()]);
    }
    for (m, name) in scene.meshes.iter().zip(&scene.mesh_names) {
        if m.vertex_count() <= BATCH_MESH_LIMIT {
            keys.push(vec!["dump_mesh".into(), name.clone()]);
        }
    }
    for name in &scene.armature_names {
        keys.push(vec!["dump_armature".into(), name.clone()]);
    }
    let mut out = Vec::new();
    for key in keys {
        let line = format!("@1 {}", key.iter().map(|k| quote(k)).collect::<Vec<_>>().join(" "));
        if session.exec_line(&mut work, &line).is_err() {
            continue;
        }
        for reply in session.take_replies() {
            let Some(rest) = reply.strip_prefix("@1 ok") else { continue };
            let rest = rest.trim_start();
            let value = match rest.strip_prefix("=json ") {
                Some(j) => serde_json::from_str(j).unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::from(tokenize(rest).map(|t| t.into_iter().map(|t| t.text).collect::<Vec<_>>()).unwrap_or_default()),
            };
            out.push(serde_json::json!([key, value]));
        }
    }
    serde_json::Value::from(out).to_string()
}

/// The `-c` boot code of a batch run; arguments: OPS-FILE SNAPSHOT-FILE
/// SCRIPT [SCRIPT-ARGS...]. `paths` go in front of `sys.path` (empty on SexOS,
/// where the package is frozen into `python`).
pub fn batch_boot(paths: &[String]) -> String {
    format!("import sys\nsys.path[:0] = {paths:?}\nfrom polyamour import polyops\npolyops.run_batch(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:])\n")
}

/// A `Command` for a batch run of `script` (see [`batch_snapshot`]).
pub fn batch_command(cfg: &KernelConfig, script: &std::path::Path, ops: &std::path::Path, snapshot: &std::path::Path) -> Command {
    let mut cmd = Command::new(&cfg.program);
    cmd.arg("-I").arg("-c").arg(batch_boot(&cfg.paths)).arg(ops).arg(snapshot).arg(script);
    cmd
}

/// Apply what a batch run wrote, all or nothing (`script::run`). The stream
/// ends with `end`, or with `abort "line N: Kind: message"` when the script
/// raised; a stream with neither was cut short and is refused.
pub fn batch_apply(scene: &mut Scene, ops: &str) -> Result<super::RunReport, ScriptError> {
    let mut work = scene.clone();
    let rep = super::run(&mut work, ops)?;
    if !rep.complete {
        return Err(ScriptError { line: 0, message: "the script stopped before it finished (no 'end')".into() });
    }
    *scene = work;
    Ok(rep)
}
