# Browser SexTime snapshot panel

31 October 2026. Implementation and focused host checks are complete on
`codex/browser-sextime`, based on `af53182f`. Native qualification is pending.
This milestone has no accepted round number yet: Claude owns rounds 120–123
in the primary checkout. Their compact soak, causal ordering, Quil replace and
dirty-close work are kept separate from this worktree.

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

Focused preflight results (not native acceptance):

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

Next: after Claude's live compact soak finishes, verify the then-current base,
build/freeze the complete source, run native Clock modes at both desktop sizes
and the connected-session regressions, independently replay the raw evidence,
record the actual remaining release gates, then commit and push the qualified
change. No new release gate is claimed closed by a calendar display.

GLOBAL_VAS supplies addressing only, BOOT sanctions capability genesis,
sex-pdx supplies runtime authority and PKU enforces held authority. SexOS is
its own SASOS microkernel; this change adds no Unix/Linux/POSIX compatibility.
