# Executable architectural obligations

`ARCHITECTURE.md` remains constitutional authority. These six JSON `.sexspec`
files inventory its requirements, permitted transition contracts, source
anchors, dependency graph, scoped checks and missing evidence. Schema version 1
accepts only `open` and `partial` evidence statuses. No obligation is silently
promoted to a whole-system proof because a related test passes.

```sh
python3 scripts/check_architectural_truth.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p 'test_*.py' -v
python3 scripts/check_architectural_truth.py --run-host --evidence-dir /tmp/sexos-truth-host
```

The first command validates the inventory, pinned constitution hash, live source
anchors, check bindings and acyclic dependencies. It does not execute kernel
behavior. The third command runs registered host/static gates, retains per-check
logs and SHA-256 source bindings, and propagates failure. It never runs the
registered runtime gates. Image builds remain sealed to
`scripts/entrypoint_build.sh`; host test harnesses do not package an OS image.

Transition preconditions/effects are requirements in structured prose, not a
complete formal semantics. The scheduler ownership reference model is explicit
about its abstraction; production helper tests are separate. DiskFS host tests
execute extracted production functions with fixture descriptor I/O. Runtime
checks must retain their own platform, image and source provenance. The inventory
is a first executable evidence layer, not a mathematical proof of the OS.

When architecture changes, review the affected obligations and then update the
constitution hash. When code moves, update source anchors. A source anchor's
presence proves only that the binding exists. New claims need tests that exercise
the actual behavior and evidence whose scope matches the claim.

See `docs/handoff/SEXOS_ARCHITECTURAL_TRUTH_2026_10_28.md` for the audit,
remediation dependencies and observed execution results.
