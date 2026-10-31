#!/usr/bin/env python3
"""Compile exact app admission helpers; prove mutations fail independent tests.

Host metadata tests do not establish held capabilities or native unequal task
IDs. Separate startup wiring checks preserve the held SELF_CATALOG prerequisite.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

APPS = {"quil": "servers/quil/src/main.rs", "spindle": "apps/spindle/src/main.rs"}


def function(source, name):
    # Mask comments/literals without changing offsets; brace depth then extracts
    # the real function, not a regex ending at the first nested closing brace.
    masked = re.sub(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\\n])\'',
                    lambda m: " " * len(m[0]), source, flags=re.S)
    matches = list(re.finditer(r"\bfn\s+" + re.escape(name) + r"\s*\(", masked))
    if len(matches) != 1:
        raise ValueError(f"missing/duplicate production function {name}")
    start = matches[0].start()
    opening = masked.index("{", start)
    depth = 0
    for index in range(opening, len(masked)):
        if masked[index] == "{": depth += 1
        elif masked[index] == "}":
            depth -= 1
            if depth == 0: return source[start:index+1]
    raise ValueError("unterminated production function " + name)


def startup_wiring(source, app):
    start = function(source, "_start")
    body = start[start.index("{")+1:]
    expected = (
        f'let catalog=sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).expect("{app.upper()}_SELF_AUTHORITY");'
        f'let incarnation=startup_incarnation(catalog).expect("{app.upper()}_CATALOG_SUBJECT");'
    )
    # Require these as the very first statements: metadata lookup cannot replace
    # the held query, nor can app identity state be published before validation.
    compact = re.sub(r"\s+", "", body)
    if not compact.startswith(re.sub(r"\s+", "", expected)):
        raise ValueError(app + ": held query and helper must precede startup effects")
    if compact.count("startup_incarnation(catalog)") != 1:
        raise ValueError(app + ": ambiguous startup helper invocation")
    state = "QUIL_TASK" if app == "quil" else "INSTANCE_TASK"
    if state + ".store(catalog.task_id,core::sync::atomic::Ordering::Release);" not in compact:
        raise ValueError(app + ": startup must retain held task identity")


def policy(functions):
    result = []
    for app, helper in functions.items():
        result.append("mod " + app + " {\n" + helper + "\n"
                      "pub fn resolve(catalog:sex_pdx::app_lifecycle::CatalogInfo)"
                      "->Option<&'static sex_pdx::app_catalog::Incarnation>{startup_incarnation(catalog)}\n}\n")
    return "\n".join(result)


def compile_run(root, out, name, functions):
    path = out / (name + ".rs")
    path.write_text(policy(functions))
    binary = out / name
    env = {**os.environ, "APP_STARTUP_POLICY": str(path)}
    compiled = subprocess.run(["rustc", "--edition=2021", "--test", str(root / "tests/app_startup_identity.rs"),
                               "-o", str(binary)], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (out / (name + ".compile.log")).write_text(compiled.stdout)
    if compiled.returncode:
        raise RuntimeError("production/mutant must compile: " + name + "\n" + compiled.stdout)
    tested = subprocess.run([str(binary), "--test-threads=1"], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
    (out / (name + ".test.log")).write_text(tested.stdout)
    return tested


def once_replace(source, old, new):
    if source.count(old) != 1:
        raise ValueError("mutation no longer matches exactly one production guard: " + old)
    return source.replace(old, new, 1)


def run(root, out):
    functions = {}
    for app, path in APPS.items():
        source = (root / path).read_text()
        startup_wiring(source, app)
        functions[app] = function(source, "startup_incarnation")
    result = compile_run(root, out, "actual", functions)
    if result.returncode:
        raise RuntimeError("actual startup helpers failed:\n" + result.stdout)
    print(result.stdout, end="")
    mutants = []
    for app in APPS:
        changed = dict(functions)
        changed[app] = once_replace(changed[app], "Some(row)", "if catalog.task_id!=row.target{return None;} Some(row)")
        mutants.append((app + "-task-equality", changed, [app + "_accepts_independent_task_namespace"]))
    family = {app: once_replace(helper, f"row.app!=sex_pdx::app_catalog::App::{app.capitalize()} || ", "")
              for app, helper in functions.items()}
    mutants.append(("family-guards-removed", family, [app + "_rejects_other_families" for app in APPS]))
    subject = {app: once_replace(helper, " || !catalog.is_subject(row.target,row.surface,3)", "")
               for app, helper in functions.items()}
    mutants.append(("subject-guards-removed", subject, [app + "_rejects_invalid_identity" for app in APPS]))
    for name, changed, required_failures in mutants:
        result = compile_run(root, out, name, changed)
        if result.returncode != 101 or any("test " + test + " ... FAILED" not in result.stdout for test in required_failures):
            raise RuntimeError("mutant did not fail the intended independent tests: " + name + "\n" + result.stdout)
        print(f"[app.startup.control] mutant={name} compiled=1 rejected=1")
    print("[app.startup.identity] PASS actual_functions=2 tests=6 compiled_mutants_rejected=4 held_query_first=1 scope=host_metadata_not_native_unequal_boot")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--out", type=Path, help="fresh directory retaining extracted sources, binaries and logs")
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    if args.out is not None:
        out = args.out.resolve()
        out.mkdir(parents=True, exist_ok=True)
        if any(out.iterdir()): raise ValueError("output directory must be empty")
        run(root, out)
    else:
        with tempfile.TemporaryDirectory(prefix="sexos-app-startup-") as directory:
            run(root, Path(directory))


if __name__ == "__main__":
    main()
