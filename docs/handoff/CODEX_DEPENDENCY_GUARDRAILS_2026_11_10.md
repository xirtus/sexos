# Dependency guardrail handoff (2026-11-10)

## Scope and ownership

Andreas asked Codex to find useful work while Claude was rate limited, without
colliding with its ongoing work. Base: `origin/master` at `91f00d83`.
Branch: `codex/bridge-ledger-check` in a separate worktree. The main local
checkout has pre-existing changes and was left alone.

The open PRs cover networking (#69), installer (#71), desktop (#72), display
(#77), diagnostics (#78), CI (#81), NVIDIA (#82), Wi-Fi (#83), RAID (#84),
IOMMU (#86), ACPI extraction (#88), latency (#89), browser (#90), and media
(#53, #62). None was merged or edited as part of this work.

## Completed

Added `scripts/tests/test_guardrail_dependency_identity.py`. The existing
architecture gate compares Cargo dependency keys; those keys can be aliases
for a different package. Its bridge check also misses target-specific,
build-time and workspace-inherited dependencies, and accepts a component path
mentioned anywhere in the bridge section as proof of a row.

The complementary check resolves Cargo package identities across dependency
tables, including optional and workspace-inherited dependencies. It rejects
kernel dependencies on the existing compatibility denylist and requires an
exact component cell in the active bridge table for Tuxedo/GPL-2-only
components. Inherited package licences and source SPDX markers are covered.
Malformed TOML or missing workspace inheritance causes failure, not a skip.

Nine new tests include synthetic cases for aliases, conditional tables,
workspace inheritance and misleading ledger mentions. They run automatically
through CI's existing unittest discovery command. No build, ABI, kernel or
runtime changes.

## Integration

This deliberately adds a new file instead of editing
`scripts/tests/test_kernel_design_guardrails.py`, which PR #88 also changes.
It imports that file's `COMPAT_CRATES` so the denylist has one source.
The only other addition is this handoff. No feature ledger edits, avoiding
the shared ledger touched by most ongoing PRs.

This remains a mechanical direct-dependency check. It does not prove the
absence of transitive compatibility layers, inspect executable behaviour,
validate a replacement plan's quality, or decide whether a licence expression
permits a particular integration. Those remain review obligations.

## Validation

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p 'test_guardrail_dependency_identity.py' -v`: 9 tests passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p 'test_*.py'`: all 1,148 tests passed in 73.409 seconds.
- `git diff --check`: passed.
- Reviewed PR #88's change to the imported module: it adds an AML check and
  preserves `COMPAT_CRATES`. No overlapping edited paths.

No guest build or QEMU run was performed for this host-only check.
