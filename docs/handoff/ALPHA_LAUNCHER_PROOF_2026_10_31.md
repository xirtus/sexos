# Current launcher exhaustion proof for alpha sessions

The first new compact sustained session completed 84 active cycles over
1800.1757 seconds, then failed its final third-Quil launcher check. It is **not
accepted**: its registry remains incomplete, full terminal cleanup did not
finish, and fresh read-only replay rejects it. The complete failed attempt is
retained with its original source/image cohort.

After exact retirement and pool zeroing of Quil 18, the launcher reported
`CLOSED / RESERVE EXHAUSTED`. The runner waited for that status on a legacy
`shell.palette.item` render diagnostic. Opening the palette supplies a separate
atomic item name/action and a status diagnostic; it does not guarantee that
particular render-only record. The old check depended on unrelated rendering.

The corrected probe opens the launcher and sends a real `j` to select Quil's
row. It requires a fresh atomic selection and closed/exhausted row after that
input. An immutable capture then uses the existing full launcher glyph checker,
including positive text Applied, exact selected title and terminal status.
Read-only replay joins this capture to the registry, recorded offsets and exact
third-incarnation cleanup. Earlier rows, legacy status text, flags and a
different subject cannot substitute for these facts.

## Completed qualification

The new frozen runner cohort has **574 production inputs identical** to the
qualified repaint image and 569 runner inputs. No guest code, ABI, capability,
source sampler or address-space behavior changes.

- All 42 registered host/static checks pass without source drift, with 2,630
  passing Rust test executions. All 910 Python tests pass.
- Compact and wide full native rehearsals pass with **42 immutable verified
  captures**, including the two current exhausted-launcher views.
- Fresh independent processes replay all captures, current selection/row
  offsets, lifecycle receipts and terminal durable documents.
- Both short rehearsals have `acceptance=false`. Confirmed Quil Close observes
  save completion; actual pending-I/O terminal teardown remains unproved.
- The original failed compact attempt is a retained negative control:
  read-only replay exits 1 with `failed alpha attempt remains diagnostic`.

[Machine evidence](assets/sexos-alpha-launcher-proof-2026-10-31-evidence.json)
links the [raw archive](assets/sexos-alpha-launcher-proof-2026-10-31-raw.tar.gz),
containing both frozen cohorts, the failed original attempt, qualified new
rehearsals, immutable frames/prefixes/disks, host logs and independent replay.
All archive members are size/hash verified. Neither live long-session tree is
included as terminal evidence.

The unchanged optimized image is
`9ad6768dcc70a0554999b5b2f57ceebcc0cd66e32500c1ceee79deb1e9ff8aca`;
its kernel is
`7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.
The [repaint qualification](CIVIL_CLOCK_REPAINT_2026_10_31.md) supplies its sealed
build, native regressions and static ELF scans.

## Continuing work

A fresh corrected compact 1800-second session is running on this optimized
image. The older wide session has since [completed and passed fresh replay](ALPHA_WIDE_ACCEPTANCE_2026_10_31.md)
with 85 cycles and 206 captures on its original frozen image/checker.
Its outcome retains that scope. No completed active duration alone supplies
sustained acceptance. Require actual terminal cleanup and fresh replay, then
qualify the corrected wide profile on the optimized image.

The next concrete lifecycle fixture is
[confirmed Quil Close during device I/O](QUIL_PENDING_IO_CLOSE_PLAN_2026_10_31.md).
It uses a bounded host storage profile and same-trace device/UART ordering to
distinguish an outstanding write from an already completed reply in transit.
That plan is not an implementation or a passed gate.

Inventory remains **2 Open / 28 Partial / System Proven false**. GLOBAL_VAS is
addressing, BOOT sanctions capability genesis, sex-pdx supplies authority and
PKU enforces it. The OS retains its SASOS design; physical/storage/reserve and
global epoch release work remains separate.
