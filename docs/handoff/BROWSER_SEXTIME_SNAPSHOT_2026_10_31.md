# Browser SexTime snapshot panel

31 October 2026. The Clock panel is qualified on `codex/browser-sextime`,
including committed primary base `f8058b31`. This independent milestone has no
round number: Claude owns rounds 120–123 and subsequent primary development.
Later primary commits, including the first UiIR stream, are outside this tested
image. Integration requires a fresh source binding and verification.

The Browser T action now has a dedicated viewport-aware Clock panel. One
validated last-query snapshot produces Gregorian UTC, SexTime seconds and
Maya Long Count with an explicit UTC-day/GMT584283 policy. The panel names the
q35 CMOS profile, one-second precision and snapshot-only behavior. It adds no
clock source, background sampling, scheduler time, certificate trust or wire
format. The existing source still returns its existing epoch; the adapter into
`sex_time::SexTimestamp` performs the explicit checked conversion at display.

Ready values must remain within the source's existing 2020–2099 range. Pending,
Unavailable, Cancelled, Uncertain, Rollback and invalid values show no prior
calendar values. A complete panel needs 20 columns and eight glyph rows;
smaller tiles show a nonnumeric state and controls. The 63-column measure cap
allows all eight rows to fit the existing 512-byte text transaction at full
screen, rather than asking a full-screen window to enlarge again. Every frame
is bounded, allocation-free and padded across its complete row geometry.

Reader rendering retains precedence over Clock rendering, matching the actual
Reader selection/Copy guard. Returning to retained HTTP with V cannot leave a
hidden document selectable beneath Clock content. The existing exact held
Display route carries each Clock frame. Its immutable receipt metadata joins
whole-frame FNV, viewport, row count and acknowledged sequence; it does not
relabel a historical receipt as a new sample or grant authority.

Focused controls:

- 19 actual calendar/geometry/source tests pass, including both tile sizes,
  full-screen sizing, UTC day/leap boundaries, source bounds and stale-value
  replacement. Two compiled production mutations fail: off-by-one input time
  and restoring the too-wide measure.
- 20 Python checks pass, including an independent datetime/ordinal Long Count
  model, full-body foreground/erasure checks, exact Applied metadata,
  glyph-height versus trailing-spacing bounds, small-tile fallback and a
  read-only replay CLI that cannot start a guest, complete UTF-8 capture prefixes
  and required source/checker dependency coverage.
- The existing actual certificate source/broker/client fixture passes31 tests
  with the shared calendar crate and text metrics linked.
- Offline locked Cargo metadata resolves the new Browser path dependency.

The native checker now records immutable screenshot serial prefixes, complete
raw file hashes and compressed seed/terminal disks. `--check` independently
replays source/service/client receipt joins, original wire traffic, exact
calendar glyphs, old ink across the Browser body and per-capture real pointer
parking. Failed attempts remain diagnostic. Existing normal, invalid-year and
Close modes are retained; the normal mode samples, clears and samples again.

## Qualified image and verification

Sealed GREEN_MASTER, with no production source drift during the build:

- ISO SHA-256: `ab70c71599ff3000d5965e5c4b78c94c3af65e216a26f06a8992eff48c92cf0d`.
- Kernel SHA-256: `57f9d80fd934c4e1e9eb1dde9c411926a34c65b549c62ccf3dc561eadb1dcef1`.
- 571 production inputs and 556 runner inputs are preserved in each binding.
- All 40 registered host/static checks pass without source drift: 2,581 passing
  Rust test executions across their logs. The Python suite passes 851 tests.
- The actual Browser ELF scan decodes 92,261 instructions with zero inspected
  vector/FP matches. This is a scoped Browser inspection, not a whole-image
  or runtime register/cache-erasure proof.

The six Clock native runs cover normal, invalid-year and Close modes at
1280×800/e1000e and 1024×768/e1000. They produce 12 pixel-verified captures and
two observational Close screenshots. Ready, clear and fresh-sample frames have
exact complete calendar foreground and old-ink checks. Close joins the existing
five-holder retirement/custody proof; its raw screenshot is not a pixel-absence
claim. Clock native views are full screen at both desktop sizes; default/tiny
tile geometry is covered by actual-function host tests.

The connected desktop regression passes at both sizes: four live/cold boots
and 34 verified captures, including input during held TLS, image controls,
Reader wheel/Copy, exact clipboard/document Save, history and disk recovery.
Independent fresh processes replay all eight native cohorts from raw evidence:
**10 boots, 46 verified captures, 48 raw screenshots**.

Clock captures retain their original C binding; final host/connected checks use
D. The complete input comparison shows identical production, native runner,
helper, Python and Rust bytes. Only `scripts/sexnet_jobs_host_gate.sh` differs:
its two tracked 0644 child scripts are invoked through Bash in fresh checkouts.
Original manifests and result files are preserved, with the explicit C/D input
equivalence record; no evidence is relabelled under a replacement binding.

## Evidence and diagnostics

[Machine evidence and scope](assets/sexos-browser-sextime-2026-10-31-evidence.json)
links the [raw archive](assets/sexos-browser-sextime-2026-10-31-qualified-raw.tar.gz).
All 3,802 archive members were checked by size and SHA-256. It includes frozen
sources/checkers, build logs and image, serial prefixes, original PPMs, wire
traffic, compressed seed/terminal disks, host/Python logs, compiled controls,
Browser disassembly and independent replay coordinators. The ignored Limine
bootloader inputs and their recorded revision/hashes are included explicitly.

The [wide Clock](assets/sexos-browser-sextime-2026-10-31-clock-wide.png),
[compact Clock](assets/sexos-browser-sextime-2026-10-31-clock-compact.png) and
[cleared panel](assets/sexos-browser-sextime-2026-10-31-clock-cleared-wide.png)
are pixel-identical PNG exports of actual native PPMs. The separate status-bar
DEMO clock is synthetic and is not the Clock panel's civil-time source.

Failed preliminary attempts remain diagnostic in the archive: missing ignored
bootloader inputs before build, two non-executable child-script invocations,
an external coordinator syntax error, and a direct replay helper's `/tmp`
versus `/private/tmp` binding-path mismatch. A preliminary connected run failed
its exact extra-flow check while another guest was starting. Its own capture
contains the three expected complete flows; the foreign source is not uniquely
attributed. The final runs retain the same strict flow check and pass. Neither
the other guest nor Claude's checkout was modified or stopped.

## Remaining scope

This snapshot's architectural inventory remains **4 Open / 26 Partial** and
**System Proven false**. These are this branch's counts, not a report on the
newer primary checkout. The work qualifies a last-query civil-time display
through existing held routes. A continuously live/authenticated physical or
network clock, newest-primary sustained alpha, public-web trust, physical
hardware, SMP, crash/power-loss and hostile containment remain separate work.

Next: integrate the newer primary commits in an isolated checkout, rebind and
verify the resulting image, then add native Clock tile/resize transitions.
Continue physical-profile, storage recovery and lifetime/reserve work from the
main plan without assigning Claude's in-flight round numbers to this milestone.

GLOBAL_VAS supplies addressing only, BOOT sanctions capability genesis,
sex-pdx supplies runtime authority and PKU enforces held authority. SexOS is
its own SASOS microkernel; this change adds no Unix/Linux/POSIX compatibility.
