#!/usr/bin/env python3
"""Compile Shell's actual status receiver with held-query transport doubles.

The exact function uses production custody/catalog/status types. Four compiled
mutations must fail behavioral tests; no native independent-task claim is made.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile
from check_app_startup_identity import function, once_replace


def compile_run(root, out, name, helper):
    source = out / (name + ".rs")
    source.write_text(helper + "\n")
    binary = out / name
    env = {**os.environ, "SHELL_SPINDLE_STATUS_POLICY": str(source)}
    compiled = subprocess.run(["rustc", "--edition=2021", "--test", str(root / "tests/shell_spindle_status.rs"),
                               "-o", str(binary)], env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (out / (name + ".compile.log")).write_text(compiled.stdout)
    if compiled.returncode:
        raise RuntimeError("production/mutant must compile: " + name + "\n" + compiled.stdout)
    tested = subprocess.run([str(binary), "--test-threads=1"], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
    (out / (name + ".test.log")).write_text(tested.stdout)
    return tested


def run(root, out):
    source = (root / "servers/silk-shell/src/main.rs").read_text()
    expected = ("static mut SPINDLE_HISTORY_STATUS:[Option<sex_pdx::spindle_status::Receiver>;"
                "app_catalog::SPINDLE_COUNT]=[const{None};app_catalog::SPINDLE_COUNT];")
    if re.sub(r"\s+", "", expected) not in re.sub(r"\s+", "", source):
        raise ValueError("Shell status receivers must start unbound and follow catalog count")
    extracted = function(source, "spindle_history_status_receive")
    # function() returns from fn; preserve the actual unsafe qualifier verbatim.
    helper = "unsafe " + extracted
    if helper not in source:
        raise ValueError("receiver extraction changed production text")
    result = compile_run(root, out, "actual", helper)
    if result.returncode:
        raise RuntimeError("actual Shell receiver failed:\n" + result.stdout)
    print(result.stdout, end="")
    catalog_guard = ("matches!(sex_pdx::app_lifecycle::catalog(e.slot),Ok(c) if "
                     "(c.target_pd,c.task_id,c.surface_id,c.coordinator_pd)==(e.target,e.task,e.surface,3))")
    live_guard = "sex_pdx::app_lifecycle::info(e.slot)==Err(sex_pdx::app_lifecycle::LIVE)"
    mutants = [
        ("domain-as-task", once_replace(helper, "Receiver::new(e.target,e.task)", "Receiver::new(e.target,e.target)"),
         "exact_held_unequal_tasks_bind_both_actual_incarnations"),
        ("sender-task-bypass", once_replace(helper, "||msg.caller_task!=e.task", ""),
         "wrong_actual_sender_task_cannot_forge_the_expected_packed_tuple"),
        ("catalog-bypass", once_replace(helper, catalog_guard, "true"),
         "held_catalog_tuple_mismatch_or_absence_cannot_bind"),
        ("live-bypass", once_replace(helper, live_guard, "true"),
         "nonlive_contended_and_terminal_held_status_cannot_bind"),
    ]
    for name, changed, required_failure in mutants:
        result = compile_run(root, out, name, changed)
        if result.returncode != 101 or "test " + required_failure + " ... FAILED" not in result.stdout:
            raise RuntimeError("mutant did not fail intended behavioral test: " + name + "\n" + result.stdout)
        print(f"[shell.spindle.status.control] mutant={name} compiled=1 rejected=1")
    print("[shell.spindle.status] PASS actual_receiver=1 tests=8 compiled_mutants_rejected=4 scope=host_fresh_ipc_not_native_or_deferred_task_auth")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--out", type=Path, help="fresh directory retaining sources, binaries and logs")
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    if args.out is not None:
        out = args.out.resolve()
        out.mkdir(parents=True, exist_ok=True)
        if any(out.iterdir()): raise ValueError("output directory must be empty")
        run(root, out)
    else:
        with tempfile.TemporaryDirectory(prefix="sexos-shell-spindle-status-") as directory:
            run(root, Path(directory))


if __name__ == "__main__":
    main()
