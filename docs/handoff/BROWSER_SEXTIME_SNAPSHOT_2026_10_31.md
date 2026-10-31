# Browser SexTime snapshot panel

31 October 2026. The Clock panel is qualified on `codex/browser-sextime`, now
including committed primary base `e5b95eb0`: launcher deadline handling,
Quil's dirty-close guard and the first BOOT-granted UiIR stream. Native checks
also restore the Clock to its original small tile without another query.
This independent milestone has no round number; Claude maintains the numbered
primary ledger. Later primary commits need their own integration verification.
The initial `bf5de8db` qualification and its original evidence remain below.

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

## Initial checkpoint `bf5de8db` — primary base `f8058b31`

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

### Initial evidence and diagnostics

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

### Initial scope

This snapshot's architectural inventory remains **4 Open / 26 Partial** and
**System Proven false**. These are this branch's counts, not a report on the
newer primary checkout. The work qualifies a last-query civil-time display
through existing held routes. A continuously live/authenticated physical or
network clock, newest-primary sustained alpha, public-web trust, physical
hardware, SMP, crash/power-loss and hostile containment remain separate work.

## Integrated qualification — primary base `e5b95eb0`

One fresh E cohort binds the combined production source, all checkers and this
sealed GREEN_MASTER image. Its 572 production inputs and 558 runner inputs
match the final source bytes. No C/D input-equivalence substitution is needed.

- ISO SHA-256: `49adf544ae131a823308d22a76cd870bcc4a9893c3a9aa27c64ce9d201ff00f8`.
- Kernel SHA-256: `0b6a11ff799598046d4880f921fe9c216b8a4a292c1414137185748de9780011`.
- All **41** registered host/static checks pass without source drift:
  **2,590** passing Rust test executions. The Python suite passes **854** tests.
- Both retained compiled calendar mutations are rejected and all five of their
  recorded source inputs are byte-identical to this cohort's inputs.
- The actual ISO-extracted Browser, Shell, Display and Quil ELFs contain
  **222,704** decoded instructions with zero inspected vector/FP matches.
  This remains a four-ELF static inspection, not whole-image or runtime erasure.

Each normal/invalid-year Clock run now captures T, Q, fresh T and a real
ZoomToggle restore. The final frame must use the exact original held-shell
tile, a newer viewport and the same second snapshot. Reusing a full-screen
frame, a foreign tile, an old viewport or an incomplete/missing restore action
fails. No extra query is issued to make a retained date appear fresh. All four
restores pass at the two desktop sizes, with all eight calendar/provenance rows
readable in the default tiles. Tiny fallback remains actual-function host scope.

Six Clock runs provide 16 verified captures and two observational Close
screenshots. The connected live/cold regressions provide four boots and 34
verified captures. Fresh independent replay accepts all eight cohorts:
**10 boots, 50 pixel-verified captures, 52 raw screenshots**. BOOT's UiIR
pairing, successful producer/consumer attachments and sampled committed focus
records are observed in all ten boot logs. They establish no complete stream
capture, rendering authority or global epoch protocol.

[Integrated machine evidence](assets/sexos-browser-sextime-integrated-2026-10-31-evidence.json)
links the [integrated raw archive](assets/sexos-browser-sextime-integrated-2026-10-31-raw.tar.gz).
All **1,420** archive members were verified by size and SHA-256. Original
bindings, build, wire, serial prefixes, PPMs, compressed disks, host/Python logs,
ELFs/disassembly and replay coordinators are retained. An initial external
replay wrapper used the wrong connected-result key/path; its diagnostic is
preserved alongside the corrected fresh wrapper. A packager report-key typo
was corrected before archive creation. Native evidence was unchanged.

The actual restored [compact desktop](assets/sexos-browser-sextime-integrated-2026-10-31-clock-tile-compact.png)
and [wide desktop](assets/sexos-browser-sextime-integrated-2026-10-31-clock-tile-wide.png)
are pixel-identical exports of the native PPMs, with no cursor masks or edited
guest pixels. The status-bar DEMO clock remains separate and synthetic.

This image's inventory is **2 Open / 28 Partial / System Proven false**.
`DETERMINISTIC_REPLAY` and global `EPOCH_COMMIT` remain Open. The adopted UiIR
and per-stream causal work have Partial scope; the Clock panel does not close
those obligations. Newest-image sustained alpha, physical/SMP, public-web/CA,
authenticated time, crash/power-loss and hostile containment remain separate.

Next independent UI work: replace the status-bar DEMO with validated time
through explicit BOOT-held routes and bounded nonblocking probes, preserving
unknown/source/precision states. Continue the main physical-profile, storage
recovery and lifetime/reserve plan alongside Claude's numbered work.

GLOBAL_VAS supplies addressing only, BOOT sanctions capability genesis,
sex-pdx supplies runtime authority and PKU enforces held authority. SexOS is
its own SASOS microkernel; this change adds no Unix/Linux/POSIX compatibility.
