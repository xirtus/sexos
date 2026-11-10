# Private late JIT-cache diagnostic (never merge)

Continues private `codex/private-embed-live-state` from `a08816cf` and production
parser `dd914fd4`; this is not a production PR. No new compiled/native evidence
exists for the cache snapshot at this checkpoint.

The final native snapshot now copies existing JIT cache entries through the
read-only Script API for both Wasmi and native wasm2c. There are at most 64
entries and each already-admitted bytecode key has at most 128 bytes. It reports
arity, the existing saturating u16 call count, and whether an executable exists.
Keys are bytecode-plus-arity, not JavaScript function identities. Counts cover
only calls reaching the host cache after existing C and host guards. Empty cache
or zero seal refusals cannot establish whole-site function coverage or compiler
eligibility. `Stats.refused` is labelled `seal_refused`; unsupported bytecode is
not counted there. This adds no guest C change, per-call counters/logging,
compilation attempt, executable-code mutation, callback, getter or evaluation.

External local tools under `sexos-evidence/private-live-embed` prepare the next
observation:

- `decode_cache.py` reconstructs bounded owner/frame/entry records, rejects
  incomplete/duplicate/noncanonical fragments, and reports byte offsets and
  opcode names outside the baseline's current subset. A supported opcode list
  is not a claim that the complete function compiles or that runtime guards pass.
- `opcode-table.json` is metadata from the pinned local QuickJS-NG opcode header;
  all baseline constants match it. The header/source is not imported into SexOS.
- `test_decode_cache.py` passes four Python tests including two positive cases
  and nine malformed-input controls; exact commands, sources and output hashes
  are retained in `source-checks.json`. These are not guest execution proof.
- `probe-live-state.py` preserves the genuine expanded HTTPS runner's 180-second
  observation, then sends ordinary native Ctrl+Shift+Y. It requires the snapshot
  completion to identify the actual launched target and task, retains before
  and after screenshots, and checks frozen ISO/program metadata and hashes.
  It has only been syntax-checked; it has not run. No site source, request
  header, guard or budget is changed.

Next: obtain the coordinated build/native slot, compile and scan the private
opt-in Browser, freeze it alongside the already-frozen production ISO, replay
the same genuine HTTPS page, and decode the final parent/child state. Only then
choose a layout/runtime fix or a useful baseline extension from actual evidence.
Any performance comparison must separately account for diagnostic overhead.

Private opt-in Web compilation and forbidden-byte scanning now pass. The first
compile caught two shared-reference calls to mutable `Doc::get`; the diagnostic
uses an immutable match and existing `Script::page` instead. Failed compile log
is retained separately. No native diagnostic result exists at commit time.
