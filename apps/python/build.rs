// SPDX-License-Identifier: GPL-3.0-only
//! Compiles CPython 3.13 (vendor/cpython-3.13.18, PSF license) against musl's headers
//! for the ordinary-program target, the way apps/lua builds Lua: no configure run, a
//! hand-written `pyconfig.h` (port/), the built-in module table (port/config.c), and
//! SexOS's `main` and path setup (port/sexos_main.c) in place of Programs/python.c
//! and Modules/getpath.c. The pure-Python standard library is frozen into the binary:
//! port/../frozen holds marshalled code written by scripts/python_freeze.py, and this
//! script turns it into CPython's frozen-module tables (Python/frozen.c's role).
//! Without musl headers (or on the host) the program still builds and says so:
//! `cfg(c_program)` marks a build that has the C part.
use std::{env, fs, path::{Path, PathBuf}, process::Command, sync::Mutex};

const OBJECTS: &[&str] = &[
    "abstract", "boolobject", "bytes_methods", "bytearrayobject", "bytesobject", "call", "capsule",
    "cellobject", "classobject", "codeobject", "complexobject", "descrobject", "enumobject",
    "exceptions", "genericaliasobject", "genobject", "fileobject", "floatobject", "frameobject",
    "funcobject", "iterobject", "listobject", "longobject", "dictobject", "odictobject",
    "memoryobject", "methodobject", "moduleobject", "namespaceobject", "object", "obmalloc",
    "picklebufobject", "rangeobject", "setobject", "sliceobject", "structseq", "tupleobject",
    "typeobject", "typevarobject", "unicodeobject", "unicodectype", "unionobject", "weakrefobject",
];
const PYTHON: &[&str] = &[
    "_warnings", "Python-ast", "Python-tokenize", "asdl", "assemble", "ast", "ast_opt", "ast_unparse",
    "bltinmodule", "brc", "ceval", "codecs", "compile", "context", "critical_section", "crossinterp",
    "dynamic_annotations", "errors", "flowgraph", "frame", "future", "gc", "gc_free_threading",
    "gc_gil", "getargs", "getcompiler", "getcopyright", "getplatform", "getversion", "ceval_gil",
    "hamt", "hashtable", "import", "importdl", "initconfig", "interpconfig", "instrumentation",
    "instruction_sequence", "intrinsics", "jit", "legacy_tracing", "lock", "marshal", "modsupport",
    "mysnprintf", "mystrtoul", "object_stack", "optimizer", "optimizer_analysis", "optimizer_symbols",
    "parking_lot", "pathconfig", "preconfig", "pyarena", "pyctype", "pyfpe", "pyhash", "pylifecycle",
    "pymath", "pystate", "pythonrun", "pytime", "qsbr", "bootstrap_hash", "specialize", "structmember",
    "symtable", "sysmodule", "thread", "traceback", "tracemalloc", "getopt", "pystrcmp", "pystrtod",
    "pystrhex", "dtoa", "formatter_unicode", "fileutils", "suggestions", "perf_trampoline",
    "perf_jit_trampoline", "dynload_stub",
];
const PARSER: &[&str] = &[
    "token", "pegen", "pegen_errors", "action_helpers", "parser", "string_parser", "peg_api",
    "lexer/buffer", "lexer/lexer", "lexer/state", "tokenizer/file_tokenizer",
    "tokenizer/readline_tokenizer", "tokenizer/string_tokenizer", "tokenizer/utf8_tokenizer",
    "tokenizer/helpers", "myreadline",
];
/// The built-in modules of port/config.c, plus the interpreter's own (main, gc).
const MODULES: &[&str] = &[
    "main", "gcmodule", "getbuildinfo", "atexitmodule", "faulthandler", "posixmodule",
    "signalmodule", "_tracemalloc", "_suggestions", "_codecsmodule", "_collectionsmodule",
    "errnomodule", "_io/_iomodule", "_io/iobase", "_io/fileio", "_io/bytesio", "_io/bufferedio",
    "_io/textio", "_io/stringio", "itertoolsmodule", "_sre/sre", "_sysconfig", "_threadmodule",
    "timemodule", "_typingmodule", "_weakref", "_abc", "_functoolsmodule", "_operator", "_stat",
    "symtablemodule", "_opcode", "mathmodule", "cmathmodule", "_struct", "_randommodule", "_json",
    "_bisectmodule", "_heapqmodule", "binascii", "arraymodule", "_datetimemodule",
    "_contextvarsmodule", "_queuemodule", "_statisticsmodule", "sha2module", "_hacl/Hacl_Hash_SHA2",
];

fn main() {
    println!("cargo:rustc-link-arg-bins=-zstack-size=8388608");
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = dir.join("../../vendor/cpython-3.13.18");
    let port = dir.join("port");
    let frozen = dir.join("frozen");
    for d in [&src, &port, &frozen] {
        println!("cargo:rerun-if-changed={}", d.display());
    }
    println!("cargo:rerun-if-env-changed=MUSL_INCLUDE");
    println!("cargo::rustc-check-cfg=cfg(c_program)");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("hermit") { return; }
    let musl = env::var("MUSL_INCLUDE").map(PathBuf::from).ok().or_else(|| {
        ["/usr/lib/musl/include", "/usr/include/x86_64-linux-musl", "/usr/local/musl/include"].iter().map(PathBuf::from).find(|d| d.join("stdio.h").exists())
    });
    let Some(musl) = musl else {
        println!("cargo:warning=musl headers not found: set MUSL_INCLUDE (apt install musl-dev)");
        return;
    };
    println!("cargo:rustc-cfg=c_program");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let resource = String::from_utf8(Command::new("clang").arg("-print-resource-dir").output().expect("clang").stdout).unwrap();
    let resource = resource.trim().to_string();

    let (frozen_c, frozen_s) = frozen_tables(&frozen, &out);
    // (source, extra flags): the core is Py_BUILD_CORE, built-in modules Py_BUILD_CORE_BUILTIN.
    let mut jobs: Vec<(PathBuf, &[&str])> = Vec::new();
    const CORE: &[&str] = &["-DPy_BUILD_CORE"];
    const BUILTIN: &[&str] = &["-DPy_BUILD_CORE_BUILTIN"];
    for n in OBJECTS { jobs.push((src.join(format!("Objects/{n}.c")), CORE)); }
    for n in PYTHON { jobs.push((src.join(format!("Python/{n}.c")), CORE)); }
    for n in PARSER { jobs.push((src.join(format!("Parser/{n}.c")), CORE)); }
    for n in MODULES {
        let flags: &[&str] = if matches!(*n, "main" | "gcmodule" | "getbuildinfo") { CORE } else { BUILTIN };
        jobs.push((src.join(format!("Modules/{n}.c")), flags));
    }
    jobs.push((port.join("config.c"), CORE));
    jobs.push((port.join("sexos_main.c"), CORE));
    jobs.push((frozen_c, CORE));
    jobs.push((frozen_s, &[]));

    let objs = Mutex::new(Vec::new());
    let next = Mutex::new(0usize);
    let failed = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = { let mut n = next.lock().unwrap(); let i = *n; *n += 1; i };
                let Some((file, flags)) = jobs.get(i) else { break };
                let name = file.strip_prefix(&src).or_else(|_| file.strip_prefix(&port)).or_else(|_| file.strip_prefix(&out)).unwrap();
                let obj = out.join(name.to_string_lossy().replace(['/', '.'], "_") + ".o");
                let output = Command::new("clang")
                    .args(["--target=x86_64-unknown-none-elf", "-nostdinc", "-std=gnu11", "-O2", "-fPIC", "-ffunction-sections", "-fdata-sections"])
                    .args(["-msse2", "-mno-avx", "-mno-red-zone", "-fno-stack-protector", "-ftls-model=local-exec"])
                    .args(["-fwrapv", "-fno-strict-overflow", "-fvisibility=hidden", "-DNDEBUG", "-w"])
                    .args(["-DPLATFORM=\"sexos\"", "-DSOABI=\"cpython-313-sexos\""])
                    .arg("-isystem").arg(format!("{resource}/include"))
                    .arg("-isystem").arg(&musl)
                    .arg("-I").arg(&port)
                    .arg("-I").arg(src.join("Include"))
                    .arg("-I").arg(src.join("Include/internal"))
                    .arg("-I").arg(src.join("Include/internal/mimalloc"))
                    .arg("-I").arg(src.join("Modules/_hacl/include"))
                    .arg("-I").arg(src.join("Modules/_hacl"))
                    .arg("-I").arg(src.join("Python"))
                    .args(*flags)
                    .arg("-c").arg(file).arg("-o").arg(&obj)
                    .output().expect("clang");
                if output.status.success() {
                    objs.lock().unwrap().push(obj);
                } else {
                    failed.lock().unwrap().push(format!("{}:\n{}", file.display(), String::from_utf8_lossy(&output.stderr)));
                }
            });
        }
    });
    let failed = failed.into_inner().unwrap();
    if !failed.is_empty() {
        for f in &failed { eprintln!("{f}"); }
        panic!("{} CPython sources failed to compile", failed.len());
    }
    let mut objs = objs.into_inner().unwrap();
    objs.sort();
    let lib = out.join("libpython.a");
    let _ = fs::remove_file(&lib);
    assert!(Command::new("llvm-ar").arg("crs").arg(&lib).args(&objs).status().unwrap().success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=python");
}

/// CPython's frozen-module tables (what Python/frozen.c holds in a normal build) for
/// the marshalled modules in `frozen/`, whose bytes the assembler includes directly.
fn frozen_tables(frozen: &Path, out: &Path) -> (PathBuf, PathBuf) {
    let manifest = fs::read_to_string(frozen.join("MANIFEST")).expect("apps/python/frozen/MANIFEST");
    let mut asm = String::from(".section .rodata\n");
    let mut decls = String::new();
    let (mut bootstrap, mut stdlib) = (String::new(), String::new());
    for line in manifest.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (name, kind) = line.split_once(' ').unwrap();
        let file = frozen.join(format!("{name}.bin"));
        let size = fs::metadata(&file).expect("frozen module").len();
        let sym = format!("_Py_M__{}", name.replace('.', "_"));
        asm += &format!(".globl {sym}\n.hidden {sym}\n{sym}:\n.incbin \"{}\"\n", file.display());
        decls += &format!("extern const unsigned char {sym}[];\n");
        let package = if kind == "package" { "true" } else { "false" };
        let entry = |as_name: &str| format!("    {{\"{as_name}\", {sym}, {size}, {package}}},\n");
        match name {
            "importlib._bootstrap" => bootstrap += &entry("_frozen_importlib"),
            "importlib._bootstrap_external" => bootstrap += &entry("_frozen_importlib_external"),
            "zipimport" => bootstrap += &entry("zipimport"),
            "posixpath" => { stdlib += &entry("posixpath"); stdlib += &entry("os.path"); }
            _ => stdlib += &entry(name),
        }
    }
    let c = format!(r#"/* Generated by apps/python/build.rs from apps/python/frozen. */
#include "Python.h"
#include "pycore_import.h"
#include <stdbool.h>
{decls}
static const struct _frozen bootstrap_modules[] = {{
{bootstrap}    {{0, 0, 0}}
}};
static const struct _frozen stdlib_modules[] = {{
{stdlib}    {{0, 0, 0}}
}};
static const struct _frozen test_modules[] = {{
    {{0, 0, 0}}
}};
const struct _frozen *_PyImport_FrozenBootstrap = bootstrap_modules;
const struct _frozen *_PyImport_FrozenStdlib = stdlib_modules;
const struct _frozen *_PyImport_FrozenTest = test_modules;
static const struct _module_alias aliases[] = {{
    {{"_frozen_importlib", "importlib._bootstrap"}},
    {{"_frozen_importlib_external", "importlib._bootstrap_external"}},
    {{"os.path", "posixpath"}},
    {{0, 0}}
}};
const struct _module_alias *_PyImport_FrozenAliases = aliases;
const struct _frozen *PyImport_FrozenModules = NULL;
"#);
    let (c_path, s_path) = (out.join("frozen.c"), out.join("frozen_data.S"));
    fs::write(&c_path, c).unwrap();
    fs::write(&s_path, asm).unwrap();
    (c_path, s_path)
}
