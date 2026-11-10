# Promise adoption privacy correction (draft PR 150)

The first draft's bytecode/producer tags did not cover every native callback
boundary. On the unmerged candidate Wasm `adbd4f365f8ce50342b616a6144685d333ae08c096f31a403763a9622ae118b6`,
an opaque ordinary thenable could throw after its bytecode frame had popped;
QuickJS's C catch then rejected an owned wrapper without the callback's origin.
The same issue affected pending reaction chains, native executor/finally wrappers,
async adoption and throwing then/constructor accessors. This was caught before
approval; it is not a claim that shipped master exposed this new event API.

Preserved negative controls:
`/Users/xmacbookm5/Projects/sexos-evidence/pr150-privacy-controls/`.
`provenance.json` pins the actual unchanged candidate Wasm, test source and logs.
The adoption matrix emitted 22 forbidden notifications plus one expected fulfilled
result; the separate native-bound/finally control emitted four forbidden events.

## Correction

- Thenable jobs inherit the actual callback function before invocation. Native
  Promise executors inherit their actual executor function before invocation.
- Ordinary property descriptors/prototype chains are inspected without executing
  getters, traps or exotic lookups a second time. Opaque getter functions taint
  the target before access. Proxies, exotic or over-deep lookups fail closed after
  opaque code. Async/async-generator adoption also protects constructor lookup.
- Reaction jobs carry a private source bit captured at settlement, including
  reactions registered before an opaque pending Promise settles. Scoped per-VM
  ambient provenance spans callback execution and C catch/settlement; it restores
  the previous state on normal and JavaScript-exception returns. A terminal Wasm
  trap remains governed by the existing poisoned-VM/drop path.
- Bound and native data closures retain creation/captured callback provenance.
  This covers native bound rejection functions and QuickJS's internal finally
  wrappers. No whole-realm muting replaces the positive own-source controls.
- Unknown native contexts after opaque code conservatively stay muted. Generated
  code, ambiguous proxy/accessor/bound/native closure lineages can under-report
  rejections; these are documented privacy limits, not a full HTML execution-
  context implementation. Known ordinary owned functions, thenables, accessors,
  executors and async adoption after opaque code are explicitly tested.

There is no native Promise replacement, new job, reordered job, raised 10,000-job
cap, page-visible provenance field, networking policy change or kernel change.

## Current qualification

Pinned WASI SDK 25 clang 19.1.5 and QuickJS-NG
`3c9afc9943323ee9c7dbd123c0cd991448f4b6c2` rebuild Wasm SHA256
`afabd82aeabd4d6a71044a58ac0a0ffb6b557339c011a7ae9772d3a874e035ff`;
a second build matches byte-for-byte. All 60 host DOM tests pass, including the
formerly failing controls, positive owned-after-opaque cases, ignored second
resolution, and exactly-once getter/proxy trap observation.

The native two-origin gate now includes the negative adoption/wrapper cases and
positive ordinary owned thenable/getter cases. Corrected native image/replays and
exact-head CI are still pending. All earlier `bcdb309e`/`44f0a0ab` native/HTTPS
receipts remain valid only as explicitly pinned earlier controls. They do not
qualify these changed Wasm/program bytes. PR 150 stays draft until corrected native
proof and review finish. No player rendering/playback claim.
