# Bounded serial captures and complete clock receipts (2026-11-10)

- **Failure:** PR #77's app-resource proof, CI run 42816615777, stopped QEMU at its
  bounded probe end while SilkBar was writing its next sample. The raw normal log
  ends in `[silkbar.civil.sample] seque`: 92 tag starts, 91 complete samples. This
  caused `CLOCK_GATE CIVIL: malformed civil sample`, despite earlier valid proof.
- **Capture boundary:** the checker validates clock receipt joins only through the
  last newline. It reports the ignored trailing byte count, raw size and SHA-256;
  it never changes the raw serial artifact. Raw partial CIVIL evidence still selects
  the CIVIL lane, so trimming cannot fall back to old DEMO receipts. All other gates,
  especially fault detection, retain the entire raw capture including the tail.
- **Proof strength:** malformed newline-terminated/interior records still fail, and
  missing minimum source/admission/draw evidence still fails. Only the interrupted
  final transport fragment is ineligible to be a clock receipt. The runtime harness
  preserves `bootgraph.log` beside `serial.log` and prints capture diagnostics.
- **Original artifact replay:** 556271 raw bytes,
  SHA-256 `0b9e42ecb16b921a5de509f4848d50670fea8ec396cb1f8b84bc85a0336788e5`.
  The checker ignores 28 final bytes for clock joins, retains 47 admitted/completed
  snapshots, and all BootGraph gates pass. This is a replay of the original capture,
  not a new native run or clock-pixel claim.
- **Validation:** 12 civil-clock tests pass, including CLI regressions for the actual
  cut tail, complete malformed tail/interior records, insufficient clock proof with
  legacy receipts, a partial fault that must still fail, raw hash reporting and exact
  raw artifact preservation. All 1146 Python host tests pass, as does shell syntax
  validation. CI on the exact PR head remains required.
