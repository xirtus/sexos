# Own-property cache: experimental follow-up to PR274

Branch `codex/jit-object-string` starts at PR274's exact head 0f7561a1.
The rejected scalar-admission experiments are absent. PR274 is unchanged and
still awaiting all CI checks, including aggregate build; do not merge early.

A bounded 256-entry own-property lookup cache runs inside the QuickJS guest,
only with the experimental tier enabled. This is guest-side property caching,
not additional native-compiled bytecode coverage. Ordinary own data properties
only: class/exotic guards, current shape identity, slot bounds, atom and descriptor
are checked; values are read afresh and duplicated with QuickJS ownership rules.
Saved shape identities are weak and never dereferenced. Descriptor validation
also covers in-place mutation and shape-address reuse. Prototype walks, accessors,
proxies and other exotic behavior retain the normal path. No object pointer is
passed to generated native code. Storage is bounded to 3 KiB on wasm32.

Guest entry points refresh the opt-in state from the host, so forced interpreter
mode skips both the cache and native import attempts during the workload.
Read-only cache counters are reported separately from native JIT counts.
The one cache hit seen in disabled runs occurred during initialization before
host disable; targeted tests prove counters stop advancing after disable.

## Verification

Six targeted differential tests pass on wasmi. The new test covers warm reads,
fresh value/reference lifetime, deletion, prototype fallback, getters, data
redefinition, slot reuse, proxies/revocation, symbol identity and shape churn.
It asserts real cache activity and checks disabling after warming.

Native QEMU off/on outputs match, including fixed expected property/getter/proxy
results. Cache hits: 1 off, 15,051 on. Native bytecode coverage is unchanged:
12 compiled / 13,415 executed / 3 bailouts / 0 seal refusals, actual domain seals.
See `target/browser-evidence/object-cache-native/result.json`.

The pinned QuickJS source and WASI SDK temporary copies had lost files. Restored
QuickJS commit 3c9afc9943323ee9c7dbd123c0cd991448f4b6c2 and official WASI SDK25
under ignored evidence paths; no third-party source imported into the repository.
Guest SHA256 17e4bbf0161dedb4e979da84b532c052311f8bf307a5741bbd13e6cc3c48b5ec.
Browser SHA256 95c6c6b8de1a5bf737af4c118040b6c93e3cdaf7fcbe7f7649a5c907bc404256.

## Live diagnostic pair, on then off

Same binary, frozen ISO, viewport, URLs and scripted physical interactions.
Cumulative VM seconds include host calls and exclude waits outside VM execution.

| Site | Off seconds | On seconds | On property-cache hits/misses | Native compiled/executed |
|---|---:|---:|---:|---:|
| Wikipedia Microkernel | 17.878 | 15.314 | 205076 / 43615 | 2 / 1473 |
| GitHub xirtus/sexos | 29.878 | 27.560 | 212045 / 83186 | 3 / 142 |

Both runs completed their text-find/scroll tasks and the separate local webmail
fixture. The latter is excluded from the table. No level-2 console errors were
observed in the on run. Inspected screenshots show Wikipedia appearance controls
at the sampled scroll position and GitHub repository navigation; do not claim
complete reading/navigation workflows or complete site compatibility from them.

Live bytes, asynchronous work and host scheduling are uncontrolled. This single
pair does not establish a speedup or attribution to the property cache. Native
JIT coverage remains unchanged; do not add cache hits to native execution counts.
String concatenation/comparison specialization and native object-bytecode lowering
are not implemented. All local VMs/build sessions completed and were reaped.
The source, inputs, raw logs and screenshots are sealed by
`target/browser-evidence/object-cache-measurements.json`. x17 playback remains pending.
