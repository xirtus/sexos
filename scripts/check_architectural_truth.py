#!/usr/bin/env python3
"""Validate the architectural obligation inventory; optionally run host checks.

A valid inventory is not a proof of the OS. Open/partial obligations deliberately
remain visible, including when every registered check succeeds.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


SUBSYSTEMS = {"architecture", "pdx", "scheduler", "memory", "diskfs", "uiir"}
CORE_INVARIANTS = {
    "GLOBAL_VAS_ADDRESSING", "PDX_SOLE_AUTHORITY", "PKU_ENFORCEMENT_ONLY",
    "CAPABILITY_GENESIS", "NO_AMBIENT_AUTHORITY", "DETERMINISTIC_REPLAY",
    "EPOCH_COMMIT", "NO_PER_PROCESS_TABLES", "USERLAND_MEMORY_AUTHORITY",
    "CAUSAL_DOMAIN_ORDER",
}


class InvalidSpec(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise InvalidSpec(message)


def nonempty(value):
    return isinstance(value, str) and bool(value.strip())


def strings(value, label, allow_empty=False):
    require(isinstance(value, list), f"{label}: expected list")
    require(allow_empty or bool(value), f"{label}: empty")
    require(all(nonempty(item) for item in value), f"{label}: invalid string")
    require(len(set(value)) == len(value), f"{label}: duplicates")


def keys(value, expected, label):
    require(isinstance(value, dict), f"{label}: expected object")
    require(set(value) == set(expected), f"{label}: wrong fields")


def local_file(root, relative):
    require(nonempty(relative), "empty source/check path")
    path = Path(relative)
    require(not path.is_absolute() and ".." not in path.parts,
            f"path must be repository relative: {relative}")
    target = (root / path).resolve()
    require(target.is_relative_to(root.resolve()), f"path escapes repository: {relative}")
    require(target.is_file(), f"missing file: {relative}")
    return target


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON field: {key}")
        result[key] = value
    return result


def load_specs(root):
    root = Path(root).resolve()
    paths = sorted((root / "spec").glob("*.sexspec"))
    require({p.stem for p in paths} == SUBSYSTEMS, "expected exactly six subsystem specs")
    docs = {}
    bindings = set()
    for path in paths:
        doc = json.loads(path.read_text(), object_pairs_hook=unique_object)
        expected = {"schema_version", "subsystem", "invariants", "transitions"}
        if path.stem == "architecture":
            expected |= {"constitution", "state_model", "checks"}
        keys(doc, expected, str(path))
        require(type(doc["schema_version"]) is int and doc["schema_version"] == 1,
                f"{path.name}: unsupported schema")
        require(doc["subsystem"] == path.stem, f"{path.name}: subsystem mismatch")
        require(isinstance(doc["invariants"], list) and doc["invariants"],
                f"{path.name}: missing obligations")
        require(isinstance(doc["transitions"], list), f"{path.name}: transitions not a list")
        docs[path.stem] = doc
        bindings.add(path.relative_to(root).as_posix())

    arch = docs["architecture"]
    keys(arch["constitution"], {"path", "sha256"}, "constitution")
    require(arch["constitution"]["path"] == "ARCHITECTURE.md", "wrong constitutional authority")
    constitution = local_file(root, "ARCHITECTURE.md")
    require(hashlib.sha256(constitution.read_bytes()).hexdigest() == arch["constitution"]["sha256"],
            "constitution changed: review obligations before updating its pinned SHA-256")
    bindings.add("ARCHITECTURE.md")
    keys(arch["state_model"], {"equation", "state", "input", "capability_graph", "boundary"}, "state model")
    require(arch["state_model"]["equation"] == "S[n+1] = T(S[n], I[n], C)", "transition equation changed")
    require(arch["state_model"]["boundary"] == ["snapshot", "transform", "swap", "commit"],
            "epoch boundary order changed")
    for key in ("state", "input", "capability_graph"):
        require(nonempty(arch["state_model"][key]), f"state model: missing {key}")

    require(isinstance(arch["checks"], list) and arch["checks"], "missing check registry")
    checks = {}
    for check in arch["checks"]:
        keys(check, {"id", "kind", "argv", "scope"}, "check")
        require(nonempty(check["id"]) and check["id"] not in checks, "missing/duplicate check ID")
        require(re.fullmatch(r"[a-z][a-z0-9_]*", check["id"]) is not None,
                "check ID must be a safe lowercase identifier")
        require(check["kind"] in ("static", "host", "runtime"), "invalid check kind")
        require(nonempty(check["scope"]), "check must describe evidence limits")
        argv = check["argv"]
        require(isinstance(argv, list) and len(argv) == 2 and argv[0] == "bash",
                "check must be a literal bash script invocation")
        local_file(root, argv[1])
        require(argv[1].startswith("scripts/") and argv[1].endswith(".sh"), "invalid gate path")
        bindings.add(argv[1])
        checks[check["id"]] = check

    invariants = {}
    references = []
    used_checks = set()
    for name, doc in docs.items():
        for item in doc["invariants"]:
            keys(item, {"id", "requirement", "status", "sources", "checks", "gaps", "depends_on"}, "invariant")
            identity = item["id"]
            require(nonempty(identity) and identity not in invariants, "missing/duplicate invariant ID")
            require(nonempty(item["requirement"]), f"{identity}: missing requirement")
            require(item["status"] in ("open", "partial"), f"{identity}: unsupported proof claim")
            strings(item["checks"], identity + " checks", allow_empty=True)
            require((item["status"] == "partial") == bool(item["checks"]),
                    f"{identity}: status/check coverage mismatch")
            strings(item["gaps"], identity + " gaps")
            strings(item["depends_on"], identity + " dependencies", allow_empty=True)
            require(isinstance(item["sources"], list) and item["sources"], f"{identity}: no source anchors")
            for source in item["sources"]:
                keys(source, {"path", "anchor"}, "source")
                target = local_file(root, source["path"])
                require(nonempty(source["anchor"]), f"{identity}: empty source anchor")
                require(source["anchor"] in target.read_text(),
                        f"{identity}: missing source anchor {source['path']} :: {source['anchor']}")
                bindings.add(source["path"])
            invariants[identity] = item
            references.extend((identity, dep) for dep in item["depends_on"])
            used_checks.update(item["checks"])
        events = set()
        for transition in doc["transitions"]:
            keys(transition, {"event", "preconditions", "effects", "forbidden", "checks"}, "transition")
            require(nonempty(transition["event"]) and transition["event"] not in events,
                    f"{name}: missing/duplicate transition")
            events.add(transition["event"])
            for field in ("preconditions", "effects", "forbidden"):
                strings(transition[field], name + ":" + transition["event"] + " " + field)
            strings(transition["checks"], "transition checks", allow_empty=True)
            used_checks.update(transition["checks"])

    require(CORE_INVARIANTS <= {i["id"] for i in arch["invariants"]}, "missing constitutional obligations")
    require(used_checks <= checks.keys(), f"unknown check IDs: {sorted(used_checks - checks.keys())}")
    require(checks.keys() <= used_checks, f"orphaned check IDs: {sorted(checks.keys() - used_checks)}")
    for owner, dep in references:
        require(dep in invariants and dep != owner, f"{owner}: invalid dependency {dep}")
    visiting, visited = set(), set()

    def visit(identity):
        require(identity not in visiting, f"dependency cycle at {identity}")
        if identity in visited:
            return
        visiting.add(identity)
        for dep in invariants[identity]["depends_on"]:
            visit(dep)
        visiting.remove(identity)
        visited.add(identity)

    for identity in invariants:
        visit(identity)
    return docs, checks, invariants, bindings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--run-host", action="store_true", help="execute registered host/static checks, never runtime gates")
    parser.add_argument("--evidence-dir", type=Path, help="required for --run-host; retained check logs and SHA-256 bindings")
    args = parser.parse_args(argv)
    args.root = args.root.resolve()
    if args.run_host and args.evidence_dir is None:
        parser.error("--run-host requires --evidence-dir")
    evidence = args.evidence_dir.resolve() if args.evidence_dir is not None else None
    report = dict(inventory_valid=False, system_proven=False, checks=[],
                  runtime_checks_executed=False, run_status="started")

    def save_report():
        if evidence is not None:
            evidence.mkdir(parents=True, exist_ok=True)
            temporary = evidence / ".report.json.tmp"
            temporary.write_text(json.dumps(report, indent=2) + "\n")
            temporary.replace(evidence / "report.json")

    # Replace any prior success before validation or launching a check. An
    # interruption now leaves started/running evidence, never an old PASS.
    save_report()
    try:
        docs, checks, invariants, bindings = load_specs(args.root)
    except (InvalidSpec, OSError, ValueError, TypeError, KeyError) as exc:
        report.update(run_status="inventory_failed", error=str(exc))
        save_report()
        print(f"[architectural_truth.inventory] FAIL: {exc}", file=sys.stderr)
        return 1
    report.update(inventory_valid=True,
                  obligations={key: value["status"] for key, value in sorted(invariants.items())})
    exit_status = 0
    if args.run_host:
        # Include checker/harness sources as well as the explicitly bound paths.
        files = set(bindings)
        files.add("scripts/check_architectural_truth.py")
        files.add("limine.cfg")
        files.update(p.relative_to(args.root).as_posix() for p in (args.root / "scripts").glob("*.py"))
        for directory in ("kernel/src", "tests", "scripts/tests", "crates/sex-pdx/src", "servers/sexfiles/src", "servers/quil/src",
                          "sex-rt/src", "apps/sexdrive/src", "servers/sexusb/src", "servers/sexnet/src",
                          "servers/sext/src", "apps/kaleidoscope/src", "apps/spindle/src", "servers/sexbell/src",
                          "servers/silk-shell/src", "servers/linen/src", "servers/sexdisplay/src"):
            for path in (args.root / directory).rglob("*"):
                if path.is_file() and path.suffix in (".rs", ".py"):
                    files.add(path.relative_to(args.root).as_posix())
        report["source_sha256"] = {
            name: hashlib.sha256((args.root / name).read_bytes()).hexdigest() for name in sorted(files)
        }
        for identity, check in checks.items():
            if check["kind"] == "runtime":
                continue
            report.update(run_status="running", active_check=identity)
            save_report()
            launch_error = None
            try:
                result = subprocess.run(check["argv"], cwd=args.root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                output, returncode = result.stdout, result.returncode
            except OSError as exc:
                launch_error = str(exc)
                output, returncode = (launch_error + "\n").encode(), 127
            log = evidence / f"{identity}.log"
            log.write_bytes(output)
            row = dict(id=identity, kind=check["kind"], argv=check["argv"],
                       exit_code=returncode, log=str(log), scope=check["scope"])
            if launch_error is not None:
                row["launch_error"] = launch_error
            report["checks"].append(row)
            save_report()
            print(f"[architectural_truth.check] {identity} {'PASS' if returncode == 0 else 'FAIL'}")
            if returncode != 0:
                exit_status = 1
                print(output.decode(errors="replace")[-6000:], file=sys.stderr)
    report.pop("active_check", None)
    report.update(run_status="complete", checks_passed=(exit_status == 0) if args.run_host else None)
    save_report()
    counts = {state: sum(i["status"] == state for i in invariants.values()) for state in ("open", "partial")}
    print(f"[architectural_truth.inventory] PASS obligations={len(invariants)} open={counts['open']} partial={counts['partial']}")
    print("[architectural_truth.system_proof] NOT_ESTABLISHED (inventory/host checks are scoped evidence)")
    return exit_status


if __name__ == "__main__":
    sys.exit(main())
