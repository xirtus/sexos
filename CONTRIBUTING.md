# Contributing to SexOS

SexOS is an experimental kernel. Small, focused, verifiable changes are much
easier to review than broad rewrites.

## Before you start

- Read [ARCHITECTURE.md](ARCHITECTURE.md). The invariants there are hard
  constraints, not style preferences.
- Build through the sealed entrypoint: `make full` (which calls
  `scripts/entrypoint_build.sh`). Do not add a second build path.
- Run the image under QEMU with `./dev.sh run-nographic` or `./dev.sh run`.

## Pull requests

- One concern per PR. Include the exact commands you ran and the observed
  output.
- Add or update tests where the behavior is testable.
- Do not commit build artifacts, ISOs, `.sync_*.db`, `target/`, or generated
  binaries. Releases carry the binaries.
- New code is GPL-3.0; by contributing you agree to license it under the same
  terms.
- If you change an ABI, update the contract/spec and the relevant handoff note.

## Review bar

PRs that introduce ambient authority, bypass `sex-pdx`, add per-process page
tables, or weaken the determinism invariants will be rejected. If you think an
invariant is wrong, open a design discussion first with a concrete alternative
and the failure mode it fixes.
