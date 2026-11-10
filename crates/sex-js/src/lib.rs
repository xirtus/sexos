// SPDX-License-Identifier: GPL-3.0-only
//! JavaScript for web pages. The engine is QuickJS-NG (MIT), compiled to WebAssembly
//! (`quickjs.wasm`, built by `scripts/build_quickjs_wasm.sh` from `guest/sexjs.c`) and
//! run on wasmi, so scripts are confined by the interpreter: they reach nothing but
//! their own linear memory and the page operations below. That also keeps every
//! floating point operation in software, which suits soft-float SexOS programs.
//!
//! The guest exposes one native function, `__host(op, ...strings)`. `prelude.js` builds
//! the DOM API (`document`, elements, events, timers, `console`) on top of it, and a
//! [`Page`] answers each operation. [`Document`] is the [`Page`] for a
//! [`sex_html::dom::Dom`].
#![no_std]
extern crate alloc;

#[cfg(feature = "baseline-jit")]
pub mod baseline;
pub mod document;
pub mod webgl;
#[cfg(sexjs_native)]
mod native;
pub use document::{Document, FetchAsk};

use alloc::{format, string::String, vec::Vec};
use wasmi::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// The engine, as built by `scripts/build_quickjs_wasm.sh`.
pub static QUICKJS_WASM: &[u8] = include_bytes!("../quickjs.wasm");
/// The DOM and browser API, written in JavaScript over `__host`.
pub static PRELUDE: &str = include_str!("prelude.js");

/// Page operations, `__host`'s first argument. Node ids travel as decimal strings.
pub mod op {
    /// (level 0..=2, text): console output.
    pub const LOG: u32 = 1;
    pub const BY_ID: u32 = 2;
    /// (root, selector, all): matching element ids joined by commas.
    pub const QUERY: u32 = 3;
    /// (node): "type,TAG" with DOM node types 1, 3 and 9.
    pub const INFO: u32 = 4;
    pub const GET_ATTR: u32 = 5;
    pub const SET_ATTR: u32 = 6;
    pub const REMOVE_ATTR: u32 = 7;
    pub const GET_TEXT: u32 = 8;
    pub const SET_TEXT: u32 = 9;
    /// (node, outer "1" or "0").
    pub const GET_HTML: u32 = 10;
    pub const SET_HTML: u32 = 11;
    pub const CREATE_ELEMENT: u32 = 12;
    pub const CREATE_TEXT: u32 = 13;
    /// (parent, child, before or "").
    pub const INSERT: u32 = 14;
    pub const DETACH: u32 = 15;
    pub const PARENT: u32 = 16;
    pub const CHILDREN: u32 = 17;
    /// (node): attribute names joined by newlines.
    pub const ATTR_NAMES: u32 = 18;
    /// (): "width,height" of the viewport.
    pub const VIEWPORT: u32 = 19;
    /// (url): go to another page once the script returns.
    pub const NAVIGATE: u32 = 20;
    /// (node): the element now has a click listener.
    pub const CLICKABLE: u32 = 21;
    /// (node): "x,y,w,h" of its box in the last layout.
    pub const RECT: u32 = 22;
    /// (): the page's address.
    pub const LOCATION: u32 = 23;
    /// (canvas, commands): Canvas 2D drawing, queued for the browser to run.
    pub const CANVAS: u32 = 24;
    /// (css font, text): width in pixels.
    pub const MEASURE: u32 = 25;
    /// (x, y in page pixels): the deepest laid-out element there, or nothing.
    pub const HIT: u32 = 26;
    /// (control): its current value; (control, text): sets it.
    pub const VALUE: u32 = 27;
    /// (control, caret in characters): focuses a text control and places its caret;
    /// (): no text control has focus.
    pub const CARET: u32 = 28;
    /// (video or audio element, verb, argument): play, pause, seek SECONDS, load,
    /// volume 0..1, muted 0/1, loop 0/1; queued for the browser. The browser answers
    /// with `media` dispatches carrying the element's state and its events.
    pub const MEDIA: u32 = 48;
    /// (video or audio element, SourceBuffer index, bytes): a Media Source append.
    /// The browser answers `mse\tNODE\tINDEX\tOK\tRANGES` once it has taken them.
    pub const MEDIA_APPEND: u32 = 70;
    /// (base, specifier): the module's address (asked by the engine, not the prelude).
    pub const MODULE_RESOLVE: u32 = 29;
    /// (address): the module's source, or nothing when the browser has not loaded it.
    pub const MODULE_SOURCE: u32 = 30;
    /// WebGL batch for canvas NODE: commands, then the batch's bytes (an ArrayBuffer).
    pub const GL: u32 = 64;
    /// WebGL query for canvas NODE: what, then its arguments; answers text or bytes.
    pub const GL_QUERY: u32 = 65;
    /// (url): asks the browser to load it; answers the request's id. The browser loads it
    /// once the script returns and dispatches `fetched\tID\tSTATUS\tLENGTH` (status 0:
    /// it could not be loaded).
    /// Arguments after the url (all optional): method, request headers as
    /// `name: value` lines, and the body's bytes.
    pub const FETCH: u32 = 66;
    /// (id): the loaded bytes, once (text or, with bit 16, an ArrayBuffer).
    pub const FETCH_BODY: u32 = 67;
    /// (id): the reply's final address, a newline, then its headers as `name: value`
    /// lines (Set-Cookie left out).
    pub const FETCH_HEAD: u32 = 68;
    /// (): what `document.cookie` reads; (text): a `document.cookie = text` assignment.
    pub const COOKIE: u32 = 69;
    /// (): `document.referrer`, a tab, then the origin of the page around this one
    /// (empty for a top-level page).
    pub const FRAME: u32 = 31;
    /// (to, data as JSON, target origin): a `postMessage` to the page around this one
    /// (to -1) or into iframe element `to`. The browser delivers it as
    /// `message\tFROM\tORIGIN\tDATA` (FROM `parent`, or the iframe element it came from).
    pub const POST_MESSAGE: u32 = 32;
}

/// What scripts act on. A `None` result is JavaScript `null`.
pub trait Page {
    fn call(&mut self, op: u32, args: &[&str]) -> Option<String>;
    /// Ops from [`BYTE_OPS`] up take raw byte arguments (ArrayBuffers) and may answer
    /// with bytes (an ArrayBuffer when the script asks for one).
    fn call_bytes(&mut self, op: u32, args: &[&[u8]]) -> Option<Vec<u8>> { let _ = (op, args); None }
    /// Nanoseconds on a monotonic clock, for `Date.now`, timers and the time budget.
    fn now_ns(&mut self) -> u64;
    /// Module addresses the engine asked for ([`op::MODULE_SOURCE`]) and did not get,
    /// since the last call.
    fn take_wanted_modules(&mut self) -> Vec<String> { Vec::new() }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest linear memory for the engine, in bytes.
    pub memory_bytes: usize,
    /// QuickJS heap limit, inside that memory.
    pub heap_bytes: usize,
    /// Longest a single script or event handler may run before it is stopped.
    pub budget_ms: u32,
    /// Longest a page's own script (`eval`) may run: loading a large library on a slow
    /// machine takes far longer than any event handler should.
    pub load_budget_ms: u32,
}
impl Default for Limits {
    // Libraries like three.js take about a second to load on a fast host and need room
    // for their scenes (a whole game, such as a Quake port with its data, needs most of a
    // gigabyte); memory grows only as it is used. Browsers let a script run for about
    // ten seconds before they ask.
    fn default() -> Self { Self { memory_bytes: 1 << 30, heap_bytes: 960 << 20, budget_ms: 10_000, load_budget_ms: 120_000 } }
}

/// The first op whose arguments are bytes rather than text.
pub const BYTE_OPS: u32 = 64;

struct State<P> {
    page: P,
    result: Vec<u8>,
    limits: StoreLimits,
    #[cfg(feature = "baseline-jit")]
    jit: baseline::Jit,
}

/// One page's script engine.
pub struct Script<P: Page + 'static> {
    vm: Vm<P>,
    budget_ms: i32,
    load_budget_ms: i32,
}

/// The engine's two tiers: machine code (`cfg(sexjs_native)`, see build.rs) or the
/// wasmi interpreter. Both run the same quickjs.wasm guest.
enum Vm<P: Page + 'static> {
    Wasmi(Wasmi<P>),
    #[cfg(sexjs_native)]
    Native(native::Native<P>),
}

struct Wasmi<P: Page + 'static> {
    store: Store<State<P>>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<i32, ()>,
    eval: TypedFunc<(i32, i32, i32, i32, i32), i32>,
    module: TypedFunc<(i32, i32, i32, i32, i32, i32), i32>,
    ready: TypedFunc<i32, i32>,
    dispatch: TypedFunc<(i32, i32, i32), i32>,
}

fn trapped(x: wasmi::Error) -> Error { Error::Trapped(format!("{x}")) }

impl<P: Page + 'static> Vm<P> {
    fn page(&self) -> &P { match self { Vm::Wasmi(w) => &w.store.data().page, #[cfg(sexjs_native)] Vm::Native(n) => n.page() } }
    fn page_mut(&mut self) -> &mut P { match self { Vm::Wasmi(w) => &mut w.store.data_mut().page, #[cfg(sexjs_native)] Vm::Native(n) => n.page_mut() } }
    fn alloc(&mut self, n: i32) -> Result<i32, Error> { match self { Vm::Wasmi(w) => w.alloc.call(&mut w.store, n).map_err(trapped), #[cfg(sexjs_native)] Vm::Native(v) => v.alloc(n) } }
    fn free(&mut self, p: i32) { match self { Vm::Wasmi(w) => { let _ = w.free.call(&mut w.store, p); } #[cfg(sexjs_native)] Vm::Native(v) => v.free(p) } }
    fn write(&mut self, at: usize, bytes: &[u8]) -> Result<(), Error> {
        match self {
            Vm::Wasmi(w) => w.memory.write(&mut w.store, at, bytes).map_err(|_| Error::Trapped(String::from("write"))),
            #[cfg(sexjs_native)]
            Vm::Native(v) => v.write(at, bytes),
        }
    }
    fn eval(&mut self, a: (i32, i32, i32, i32, i32)) -> Result<i32, Error> { match self { Vm::Wasmi(w) => w.eval.call(&mut w.store, a).map_err(trapped), #[cfg(sexjs_native)] Vm::Native(v) => v.eval(a) } }
    fn module(&mut self, a: (i32, i32, i32, i32, i32, i32)) -> Result<i32, Error> { match self { Vm::Wasmi(w) => w.module.call(&mut w.store, a).map_err(trapped), #[cfg(sexjs_native)] Vm::Native(v) => v.module(a) } }
    fn ready(&mut self, a: i32) -> Result<i32, Error> { match self { Vm::Wasmi(w) => w.ready.call(&mut w.store, a).map_err(trapped), #[cfg(sexjs_native)] Vm::Native(v) => v.ready(a) } }
    fn dispatch(&mut self, a: (i32, i32, i32)) -> Result<i32, Error> { match self { Vm::Wasmi(w) => w.dispatch.call(&mut w.store, a).map_err(trapped), #[cfg(sexjs_native)] Vm::Native(v) => v.dispatch(a) } }
    fn memory_bytes(&self) -> usize { match self { Vm::Wasmi(w) => w.memory.data(&w.store).len(), #[cfg(sexjs_native)] Vm::Native(v) => v.memory_bytes() } }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The engine could not be loaded or started.
    Engine(String),
    /// The script threw; the message went to the page's log.
    Thrown,
    /// The engine itself trapped (out of memory, stack): it is unusable now.
    Trapped(String),
}

fn args_of(data: &[u8], ptr: i32, len: i32) -> Option<Vec<&[u8]>> {
    let start = ptr as u32 as usize;
    let end = start.checked_add(len as u32 as usize)?;
    let mut bytes = data.get(start..end)?;
    let mut out = Vec::new();
    while bytes.len() >= 4 {
        let n = u32::from_le_bytes(bytes[..4].try_into().ok()?) as usize;
        out.push(bytes.get(4..4 + n)?);
        bytes = &bytes[4 + n..];
    }
    Some(out)
}

impl<P: Page + 'static> Script<P> {
    /// Loads the engine and runs the prelude.
    /// Loads the engine (machine code where this build has it) and runs the prelude.
    pub fn new(page: P, limits: Limits) -> Result<Self, Error> {
        #[cfg(sexjs_native)]
        {
            let mut vm = native::Native::new(page, limits.memory_bytes)?;
            if vm.init(limits.heap_bytes as i32)? != 0 { return Err(Error::Engine(String::from("init"))); }
            return Self::start(Vm::Native(vm), limits);
        }
        #[allow(unreachable_code)]
        Self::with_engine(QUICKJS_WASM, page, limits)
    }

    /// Whether this build runs scripts as machine code rather than on the interpreter.
    pub const NATIVE: bool = cfg!(sexjs_native);

    fn start(vm: Vm<P>, limits: Limits) -> Result<Self, Error> {
        let mut script = Self { vm, budget_ms: 0, load_budget_ms: 0 };
        // The prelude is ours: no time budget, which a slow machine could exceed.
        #[cfg(feature = "baseline-jit")]
        script.set_jit_enabled(false);
        script.eval(PRELUDE, "prelude.js")?;
        #[cfg(feature = "baseline-jit")]
        script.set_jit_enabled(true);
        script.budget_ms = limits.budget_ms as i32;
        script.load_budget_ms = limits.load_budget_ms as i32;
        Ok(script)
    }

    /// Loads `wasm` (a quickjs.wasm build) on the wasmi interpreter.

    pub fn with_engine(wasm: &[u8], page: P, limits: Limits) -> Result<Self, Error> {
        let e = |m: &str| Error::Engine(String::from(m));
        let mut config = Config::default();
        // Translate each function when it first runs: a page uses a small part of the engine.
        config.compilation_mode(wasmi::CompilationMode::Lazy);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|x| Error::Engine(format!("{x}")))?;
        let store_limits = StoreLimitsBuilder::new().memory_size(limits.memory_bytes).memories(1).tables(4).table_elements(100_000).instances(1).build();
        let mut store = Store::new(&engine, State { page, result: Vec::new(), limits: store_limits, #[cfg(feature = "baseline-jit")] jit: baseline::Jit::default() });
        store.limiter(|s| &mut s.limits);
        let mut linker = Linker::<State<P>>::new(&engine);
        link(&mut linker).map_err(|_| e("link"))?;
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(|x| Error::Engine(format!("{x}")))?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| e("no memory"))?;
        if let Ok(init) = instance.get_typed_func::<(), ()>(&store, "_initialize") { init.call(&mut store, ()).map_err(|x| Error::Engine(format!("{x}")))?; }
        let start = instance.get_typed_func::<i32, i32>(&store, "sx_init").map_err(|_| e("sx_init"))?;
        let mut w = Wasmi {
            alloc: instance.get_typed_func(&store, "sx_alloc").map_err(|_| e("sx_alloc"))?,
            free: instance.get_typed_func(&store, "sx_free").map_err(|_| e("sx_free"))?,
            eval: instance.get_typed_func(&store, "sx_eval").map_err(|_| e("sx_eval"))?,
            module: instance.get_typed_func(&store, "sx_module").map_err(|_| e("sx_module"))?,
            ready: instance.get_typed_func(&store, "sx_modules_ready").map_err(|_| e("sx_modules_ready"))?,
            dispatch: instance.get_typed_func(&store, "sx_dispatch").map_err(|_| e("sx_dispatch"))?,
            store, memory,
        };
        if start.call(&mut w.store, limits.heap_bytes as i32).map_err(|x| Error::Engine(format!("{x}")))? != 0 { return Err(e("init")); }
        Self::start(Vm::Wasmi(w), limits)
    }

    pub fn page(&self) -> &P { self.vm.page() }
    pub fn page_mut(&mut self) -> &mut P { self.vm.page_mut() }

    fn put(&mut self, bytes: &[u8]) -> Result<i32, Error> {
        let p = self.vm.alloc(bytes.len().max(1) as i32)?;
        if p == 0 { return Err(Error::Trapped(String::from("out of memory"))); }
        self.vm.write(p as u32 as usize, bytes)?;
        Ok(p)
    }

    /// Runs a classic script. `name` shows in error stacks.
    pub fn eval(&mut self, source: &str, name: &str) -> Result<(), Error> {
        let src = self.put(source.as_bytes())?;
        let nm = self.put(name.as_bytes())?;
        let r = self.vm.eval((src, source.len() as i32, nm, name.len() as i32, self.load_budget_ms));
        self.vm.free(src);
        self.vm.free(nm);
        let r = match r { Ok(0) => Ok(()), Ok(_) => Err(Error::Thrown), Err(x) => Err(x) };
        // Drawing the script queued reaches the page even if it threw afterwards.
        if !matches!(r, Err(Error::Trapped(_))) { self.dispatch("flush")?; }
        r
    }

    /// Runs a module script whose address is `name` (the base for its imports). Every
    /// module it imports must be loaded first: see [`Script::module_needs`].
    pub fn eval_module(&mut self, source: &str, name: &str) -> Result<(), Error> {
        let r = self.module_call(source, name, false);
        if !matches!(r, Err(Error::Trapped(_))) { self.dispatch("flush")?; }
        r
    }

    /// The addresses of modules that the module script `source` (at `name`) imports,
    /// directly or through others, that the page has not been given yet
    /// ([`Page::module_source`]). Runs none of it. Load these, then ask again, until
    /// nothing is missing.
    /// (Modules a held `import()` wants, not yet taken, come along too.)
    pub fn module_needs(&mut self, source: &str, name: &str) -> Result<Vec<String>, Error> {
        let r = self.module_call(source, name, true);
        let wanted = self.vm.page_mut().take_wanted_modules();
        match r { Err(Error::Trapped(x)) => Err(Error::Trapped(x)), _ => Ok(wanted) }
    }

    /// The modules that `import()` calls are waiting for (the page noted them while a
    /// script ran). Give the page each one's source, then call [`Script::modules_ready`].
    pub fn modules_wanted(&mut self) -> Vec<String> { self.vm.page_mut().take_wanted_modules() }

    /// Runs the waiting `import()` calls whose module graphs are now all given (asking
    /// for more where they import further modules). Returns how many still wait.
    pub fn modules_ready(&mut self) -> Result<usize, Error> {
        let r = self.vm.ready(self.budget_ms)?;
        self.dispatch("flush")?;
        Ok(r.max(0) as usize)
    }

    fn module_call(&mut self, source: &str, name: &str, discover: bool) -> Result<(), Error> {
        let src = self.put(source.as_bytes())?;
        let nm = self.put(name.as_bytes())?;
        let r = self.vm.module((src, source.len() as i32, nm, name.len() as i32, self.load_budget_ms, discover as i32));
        self.vm.free(src);
        self.vm.free(nm);
        match r { Ok(0) => Ok(()), Ok(_) => Err(Error::Thrown), Err(x) => Err(x) }
    }

    /// Calls the prelude's `__sx_dispatch` with `message`: `tick`, `click\tNODE`,
    /// `key\tTYPE\tKEY`, `load`. Returns its result (-1 if it threw).
    pub fn dispatch(&mut self, message: &str) -> Result<i32, Error> {
        let m = self.put(message.as_bytes())?;
        // A fetch's arrival may run a script the page inserted (a player's multi-megabyte
        // bundle): that gets the load budget, as the page's own scripts do.
        let budget = if message.starts_with("fetched\t") { self.load_budget_ms.max(self.budget_ms) } else { self.budget_ms };
        let r = self.vm.dispatch((m, message.len() as i32, budget));
        self.vm.free(m);
        r
    }

    /// Enables/disables the experimental tier without changing the interpreter.
    #[cfg(feature = "baseline-jit")]
    pub fn set_jit_enabled(&mut self, enabled: bool) {
        match &mut self.vm {
            Vm::Wasmi(w) => w.store.data_mut().jit.enabled = enabled,
            #[cfg(sexjs_native)] Vm::Native(v) => v.set_jit_enabled(enabled),
        }
    }
    #[cfg(feature = "baseline-jit")]
    pub fn jit_stats(&self) -> baseline::Stats {
        match &self.vm {
            Vm::Wasmi(w) => w.store.data().jit.stats,
            #[cfg(sexjs_native)] Vm::Native(v) => v.jit_stats(),
        }
    }

    /// Bytes of linear memory the engine holds.
    pub fn memory_bytes(&self) -> usize { self.vm.memory_bytes() }
}

fn link<P: Page + 'static>(linker: &mut Linker<State<P>>) -> Result<(), wasmi::errors::LinkerError> {
    type C<'a, P> = Caller<'a, State<P>>;
    fn mem<P>(c: &C<'_, P>) -> Option<Memory> { match c.get_export("memory") { Some(Extern::Memory(m)) => Some(m), _ => None } }
    linker.func_wrap("dom", "call", |mut c: C<'_, P>, op: i32, ptr: i32, len: i32| -> i32 {
        let Some(m) = mem(&c) else { return -1 };
        let (data, state) = m.data_and_store_mut(&mut c);
        let Some(args) = args_of(data, ptr, len) else { return -1 };
        let r = if op as u32 >= BYTE_OPS { state.page.call_bytes(op as u32, &args) } else {
            let Some(text) = args.iter().map(|a| core::str::from_utf8(a).ok()).collect::<Option<Vec<&str>>>() else { return -1 };
            state.page.call(op as u32, &text).map(String::into_bytes)
        };
        match r {
            Some(r) => { let n = r.len() as i32; state.result = r; n }
            None => -1,
        }
    })?;
    linker.func_wrap("dom", "result", |mut c: C<'_, P>, out: i32| {
        let Some(m) = mem(&c) else { return };
        let (data, state) = m.data_and_store_mut(&mut c);
        let r = core::mem::take(&mut state.result);
        let at = out as u32 as usize;
        if let Some(dst) = data.get_mut(at..at + r.len()) { dst.copy_from_slice(&r); }
    })?;
    linker.func_wrap("dom", "jit_available", |_: C<'_, P>| -> i32 { cfg!(feature = "baseline-jit") as i32 })?;
    linker.func_wrap("dom", "jit", |mut c: C<'_, P>, pc: i32, len: i32, argv: i32, argc: i32, out: i32| -> i32 {
        #[cfg(feature = "baseline-jit")]
        {
            let Some(m) = mem(&c) else { return 0 };
            let (data, state) = m.data_and_store_mut(&mut c);
            return state.jit.run(data, pc as u32, len as u32, argv as u32, argc as u32, out as u32) as i32;
        }
        #[cfg(not(feature = "baseline-jit"))]
        { let _ = (&mut c, pc, len, argv, argc, out); 0 }
    })?;
    const W: &str = "wasi_snapshot_preview1";
    linker.func_wrap(W, "clock_time_get", |mut c: C<'_, P>, _id: i32, _p: i64, at: i32| -> i32 {
        let now = c.data_mut().page.now_ns();
        let Some(m) = mem(&c) else { return 21 };
        if m.write(&mut c, at as u32 as usize, &now.to_le_bytes()).is_err() { return 21; }
        0
    })?;
    linker.func_wrap(W, "random_get", |mut c: C<'_, P>, ptr: i32, len: i32| -> i32 {
        let mut s = c.data_mut().page.now_ns() ^ 0x9E37_79B9_7F4A_7C15;
        let Some(m) = mem(&c) else { return 21 };
        let data = m.data_mut(&mut c);
        let at = ptr as u32 as usize;
        let Some(out) = data.get_mut(at..at + len as u32 as usize) else { return 21 };
        for b in out { s ^= s << 13; s ^= s >> 7; s ^= s << 17; *b = s as u8; }
        0
    })?;
    // Console output goes through op::LOG; libc's own stdio has nowhere to write.
    linker.func_wrap(W, "fd_write", |mut c: C<'_, P>, _fd: i32, iovs: i32, count: i32, written: i32| -> i32 {
        let Some(m) = mem(&c) else { return 21 };
        let data = m.data_mut(&mut c);
        let mut total = 0u32;
        for i in 0..count.max(0) as usize {
            let at = iovs as u32 as usize + i * 8;
            let Some(b) = data.get(at + 4..at + 8) else { return 21 };
            total += u32::from_le_bytes(b.try_into().unwrap());
        }
        let at = written as u32 as usize;
        if let Some(o) = data.get_mut(at..at + 4) { o.copy_from_slice(&total.to_le_bytes()); }
        0
    })?;
    linker.func_wrap(W, "fd_close", |_: C<'_, P>, _fd: i32| -> i32 { 8 })?;
    linker.func_wrap(W, "fd_fdstat_get", |_: C<'_, P>, _fd: i32, _at: i32| -> i32 { 8 })?;
    linker.func_wrap(W, "fd_seek", |_: C<'_, P>, _fd: i32, _o: i64, _w: i32, _at: i32| -> i32 { 8 })?;
    Ok(())
}
