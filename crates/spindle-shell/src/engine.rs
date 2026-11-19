// SPDX-License-Identifier: GPL-3.0-only
//! The engine: runs a line statement by statement and stage by stage, and stops at each
//! program so the terminal can run it without blocking its event loop.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::{Block, Element, Statement};
use crate::builtins;
use crate::error::{ShellError, Span};
use crate::eval::{self, Ctx};
use crate::host::{Host, JobId};
use crate::parser;
use crate::path;
use crate::render;
use crate::signature::COMMANDS;
use crate::value::Value;

/// A program for the terminal to run.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalRequest {
    pub name: String,
    pub args: Vec<String>,
    /// Bytes for its standard input; `None` when nothing is piped in.
    pub input: Option<Vec<u8>>,
    /// The current folder, absolute.
    pub cwd: String,
    /// The engine needs the output (it feeds a later stage, a variable or a file).
    /// When false the terminal may show the output as it comes and resume with an
    /// empty `output`.
    pub capture: bool,
    /// Where the program's name is on the line.
    pub span: Span,
}

/// What a program did.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExternalResult {
    pub output: Vec<u8>,
    pub exit_code: i32,
}

/// What the line produced once it finished.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Text to show: rendered tables, values, errors (ANSI colours when the engine's
    /// `color` is on). Ends with a newline unless empty.
    pub text: String,
    /// The last statement's value.
    pub value: Value,
    /// The error that stopped the line, if one did; it is also in `text`.
    pub error: Option<ShellError>,
    /// The last program's exit code (0 when none ran).
    pub exit_code: i32,
}

/// Where the engine is with a line.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The line is finished.
    Done(Output),
    /// Show `print` (what earlier statements produced), run the program, then call
    /// [`Engine::resume`] with its result.
    RunExternal { print: String, request: ExternalRequest },
    /// `fg`: show `print`, wait for background job `job` to end in front, then call
    /// [`Engine::resume`] with what it printed (empty if shown directly) and its code.
    WaitJob { print: String, job: JobId, capture: bool },
}

struct Exec {
    line: String,
    block: Block,
    stmt: usize,
    elem: usize,
    input: Value,
    out: String,
    cols: usize,
    capture: bool,
    last_value: Value,
}

/// Spindle's shell. Keeps variables between lines.
pub struct Engine {
    vars: BTreeMap<String, Value>,
    pending: Option<Box<Exec>>,
    last_exit: i32,
    /// ANSI colours in rendered output and errors.
    pub color: bool,
}

impl Default for Engine {
    fn default() -> Engine { Engine::new() }
}

impl Engine {
    pub fn new() -> Engine {
        Engine { vars: BTreeMap::new(), pending: None, last_exit: 0, color: true }
    }

    /// A variable set with `let`.
    pub fn var(&self, name: &str) -> Option<&Value> { self.vars.get(name) }
    pub fn set_var(&mut self, name: &str, value: Value) { self.vars.insert(name.to_string(), value); }
    pub fn var_names(&self) -> impl Iterator<Item = &str> { self.vars.keys().map(|k| k.as_str()) }
    pub(crate) fn vars(&self) -> &BTreeMap<String, Value> { &self.vars }

    /// Whether a line is waiting on [`Engine::resume`].
    pub fn is_waiting(&self) -> bool { self.pending.is_some() }

    /// Drops a line that is waiting on a program (Ctrl+C).
    pub fn cancel(&mut self) -> Option<Output> {
        let exec = self.pending.take()?;
        Some(Output { text: exec.out, value: Value::Nothing, error: None, exit_code: self.last_exit })
    }

    /// Starts a line. `cols` is the terminal width in cells for tables (0: unlimited).
    /// A line still waiting is dropped.
    pub fn run(&mut self, line: &str, host: &mut dyn Host, cols: usize) -> Step {
        self.pending = None;
        let block = match parser::parse(line) {
            Ok(b) => b,
            Err(e) => {
                let text = e.render(line, self.color);
                return Step::Done(Output { text, value: Value::Nothing, error: Some(e), exit_code: self.last_exit });
            }
        };
        let exec = Exec { line: line.to_string(), block, stmt: 0, elem: 0, input: Value::Nothing, out: String::new(), cols, capture: false, last_value: Value::Nothing };
        self.drive(Box::new(exec), host)
    }

    /// Continues the line after the program from [`Step::RunExternal`] or the job from
    /// [`Step::WaitJob`] ended.
    pub fn resume(&mut self, result: ExternalResult, host: &mut dyn Host) -> Step {
        let Some(mut exec) = self.pending.take() else {
            return Step::Done(Output { text: String::new(), value: Value::Nothing, error: None, exit_code: self.last_exit });
        };
        self.last_exit = result.exit_code;
        if exec.capture {
            exec.input = match String::from_utf8(result.output) {
                Ok(s) => Value::String(s),
                Err(e) => Value::Binary(e.into_bytes()),
            };
        } else {
            if !result.output.is_empty() {
                exec.out.push_str(&String::from_utf8_lossy(&result.output));
                if !exec.out.ends_with('\n') { exec.out.push('\n'); }
            }
            exec.input = Value::Nothing;
        }
        self.drive(exec, host)
    }

    /// Runs a line to the end, running programs with `run` (scripts, tests, hosts that
    /// can block). `fg` waits are passed to `run` as a request named `fg` with the job
    /// number as its argument.
    pub fn run_sync(&mut self, line: &str, host: &mut dyn Host, cols: usize, run: &mut dyn FnMut(&mut dyn Host, &ExternalRequest) -> ExternalResult) -> Output {
        let mut shown = String::new();
        let mut step = self.run(line, host, cols);
        loop {
            match step {
                Step::Done(mut out) => {
                    shown.push_str(&out.text);
                    out.text = shown;
                    return out;
                }
                Step::RunExternal { print, request } => {
                    shown.push_str(&print);
                    let r = run(host, &request);
                    if !request.capture && !r.output.is_empty() {
                        shown.push_str(&String::from_utf8_lossy(&r.output));
                        if !shown.ends_with('\n') { shown.push('\n'); }
                    }
                    let r = if request.capture { r } else { ExternalResult { output: Vec::new(), exit_code: r.exit_code } };
                    step = self.resume(r, host);
                }
                Step::WaitJob { print, job, capture } => {
                    shown.push_str(&print);
                    let req = ExternalRequest { name: String::from("fg"), args: alloc::vec![job.to_string()], input: None, cwd: host.cwd(), capture, span: Span::default() };
                    let r = run(host, &req);
                    step = self.resume(r, host);
                }
            }
        }
    }

    fn fail(&mut self, exec: Box<Exec>, err: ShellError) -> Step {
        let mut text = exec.out;
        text.push_str(&err.render(&exec.line, self.color));
        Step::Done(Output { text, value: Value::Nothing, error: Some(err), exit_code: self.last_exit })
    }

    fn drive(&mut self, mut exec: Box<Exec>, host: &mut dyn Host) -> Step {
        loop {
            if exec.stmt >= exec.block.stmts.len() {
                return Step::Done(Output { text: exec.out, value: exec.last_value, error: None, exit_code: self.last_exit });
            }
            let (pipeline, bind) = match &exec.block.stmts[exec.stmt] {
                Statement::Let { name, pipeline, .. } => (pipeline.clone(), Some(name.clone())),
                Statement::Pipeline(p) => (p.clone(), None),
            };
            let mut job_started: Option<(JobId, String)> = None;
            while exec.elem < pipeline.elements.len() {
                let is_last = exec.elem + 1 == pipeline.elements.len();
                let input = core::mem::replace(&mut exec.input, Value::Nothing);
                let el = &pipeline.elements[exec.elem];
                let mut ctx = Ctx { vars: &self.vars, host, cols: exec.cols, color: self.color };
                let result = match el {
                    Element::Expr(e) => eval::eval(&mut ctx, e, &input, None),
                    Element::Call(c) if c.builtin.is_some() && !c.forced_external && c.name == "fg" => {
                        let id = match c.args.first() {
                            Some(crate::ast::Arg::Positional(e)) => match eval::eval(&mut ctx, e, &input, None) {
                                Ok(Value::Int(i)) if i >= 0 => Ok(i as JobId),
                                Ok(Value::String(s)) if s.trim_start_matches('%').parse::<JobId>().is_ok() => Ok(s.trim_start_matches('%').parse().unwrap()),
                                Ok(_) => Err(ShellError::at("expected a job number", e.span, "see `jobs`")),
                                Err(e) => Err(e),
                            },
                            _ => match host.jobs().iter().rev().find(|j| j.state != crate::host::JobState::Done(0) && !matches!(j.state, crate::host::JobState::Done(_))) {
                                Some(j) => Ok(j.id),
                                None => Err(ShellError::at("there are no background jobs", c.head_span, "")),
                            },
                        };
                        let id = match id { Ok(id) => id, Err(e) => return self.fail(exec, e) };
                        if let Err(e) = host.job_foreground(id) {
                            return self.fail(exec, ShellError::at(format!("can't bring job {} to the front: {}", id, e), c.span, e));
                        }
                        exec.capture = !is_last || bind.is_some() || pipeline.redirect.is_some();
                        let capture = exec.capture;
                        exec.elem += 1;
                        let print = core::mem::take(&mut exec.out);
                        self.pending = Some(exec);
                        return Step::WaitJob { print, job: id, capture };
                    }
                    Element::Call(c) if c.builtin.is_some() && !c.forced_external => builtins::run(&mut ctx, c, input),
                    Element::Call(c) => {
                        if !c.forced_external && !host.is_external(&c.name) {
                            let err = unknown_command(&c.name, c.head_span, host);
                            return self.fail(exec, err);
                        }
                        let mut ctx = Ctx { vars: &self.vars, host, cols: exec.cols, color: self.color };
                        let mut args = Vec::new();
                        for a in &c.args {
                            match a {
                                crate::ast::Arg::Positional(e) => match eval::eval(&mut ctx, e, &input, None) {
                                    Ok(Value::List(items)) => args.extend(items.iter().map(|v| v.to_text())),
                                    Ok(v) => args.push(v.to_text()),
                                    Err(e) => return self.fail(exec, e),
                                },
                                crate::ast::Arg::Flag { name, .. } => args.push(format!("--{}", name)),
                            }
                        }
                        let request = ExternalRequest {
                            name: c.name.clone(),
                            args,
                            input: to_bytes(&input),
                            cwd: host.cwd(),
                            capture: !is_last || bind.is_some() || pipeline.redirect.is_some(),
                            span: c.head_span,
                        };
                        if pipeline.background && is_last {
                            match host.spawn_job(&request) {
                                Ok(id) => {
                                    job_started = Some((id, c.name.clone()));
                                    Ok(Value::Int(id as i64))
                                }
                                Err(e) => Err(ShellError::at(format!("can't start `{}` in the background: {}", c.name, e), c.head_span, e)),
                            }
                        } else {
                            exec.capture = request.capture;
                            exec.elem += 1;
                            let print = core::mem::take(&mut exec.out);
                            self.pending = Some(exec);
                            return Step::RunExternal { print, request };
                        }
                    }
                };
                match result {
                    Ok(v) => exec.input = v,
                    Err(e) => return self.fail(exec, e),
                }
                exec.elem += 1;
            }
            if pipeline.background && job_started.is_none() {
                let err = ShellError::at("`&` runs a program in the background, and this pipeline ends in a built-in command", pipeline.span, "built-ins finish at once")
                    .help("drop the `&`");
                return self.fail(exec, err);
            }
            // The pipeline is done: redirect, bind or show its value.
            let value = core::mem::replace(&mut exec.input, Value::Nothing);
            if let Some((id, name)) = &job_started {
                exec.out.push_str(&format!("[{}] {}\n", id, name));
                exec.last_value = value;
            } else if let Some(r) = &pipeline.redirect {
                let mut ctx = Ctx { vars: &self.vars, host, cols: exec.cols, color: self.color };
                let target = match eval::eval(&mut ctx, &r.path, &Value::Nothing, None) {
                    Ok(Value::String(s)) => s,
                    Ok(v @ (Value::Int(_) | Value::Float(_))) => v.to_text(),
                    Ok(v) => return self.fail(exec, ShellError::at(format!("expected a file name, got a {}", v.type_name()), r.path.span, "")),
                    Err(e) => return self.fail(exec, e),
                };
                let full = path::resolve(&host.cwd(), &host.home(), &target);
                let data = match builtins::encode(&value, &path::extension(&target), false) {
                    Ok(d) => d,
                    Err(e) => return self.fail(exec, ShellError::at(e, r.span, "")),
                };
                if let Err(e) = host.write_file(&full, &data, r.append) {
                    return self.fail(exec, ShellError::at(format!("can't write `{}`: {}", target, e), r.path.span, e));
                }
                exec.last_value = Value::Nothing;
            } else if let Some(name) = bind {
                self.vars.insert(name, value.clone());
                exec.last_value = value;
            } else {
                exec.out.push_str(&render::render(&value, exec.cols, self.color));
                exec.last_value = value;
            }
            exec.stmt += 1;
            exec.elem = 0;
        }
    }
}

/// What a program reads when a value is piped into it: text as is, values as plain text.
pub fn to_bytes(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Nothing => None,
        Value::String(s) => Some(s.as_bytes().to_vec()),
        Value::Binary(b) => Some(b.clone()),
        other => {
            let mut t = other.to_text();
            if !t.ends_with('\n') { t.push('\n'); }
            Some(t.into_bytes())
        }
    }
}

fn unknown_command(name: &str, span: Span, host: &dyn Host) -> ShellError {
    let err = ShellError::at(format!("unknown command `{}`", name), span, "not a built-in command or a program");
    let externals = host.external_names();
    let candidates = COMMANDS.iter().map(|s| s.name).chain(externals.iter().map(|s| s.as_str()));
    match crate::suggest::closest(name, candidates) {
        Some(best) => err.help(format!("did you mean `{}`?", best)),
        None => err.help("`help` lists the built-in commands"),
    }
}
