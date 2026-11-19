# Real-page compatibility and scalar-closure JIT

## Compatibility

Wikipedia Popups excludes descendant links using complex selectors in `:not`.
Implement full selector alternatives for `:not`, `:is` and `:where`, including
correct specificity and a 32-level nesting bound. Its observed exception is gone
in the native live run. This does not qualify all Wikipedia layout behavior.

GitHub's popover initialization spreads `adoptedStyleSheets`. Constructed sheets
now enter the real cascade, are scoped to their document/shadow tree, remain
separate from DOM children/styleSheets, and update all adopters after mutation.
Adoption is bounded to 256 sheets per root and 2 MiB of published CSS per page.
Array mutation, replacement, removal and disabling update rendered styles.
This is not full CSSOM/shadow-DOM conformance; existing flattened shadow-tree
limitations and other CSS features remain. CSSOM reference:
https://drafts.csswg.org/cssom-1/#extensions-to-the-documentorshadowroot-interface-mixin

Follow-up real GitHub failures identified missing document.domain and fragment
cloning. The getter returns the actual hostname; origin relaxation is refused.
Fragment clones now preserve child nodes, fixing template-based copy controls.
The subsequent live profile run contained no level=2 console records. Visible-text
checks alone are not proof of every interactive GitHub workflow.

## JIT protocol 3

Bounded profiling found real closure-based helpers and cold-cache saturation.
Snapshot up to eight declared arguments plus captured values inside QuickJS,
on every call. The host accepts only int32/boolean scalars; heap references,
undefined/TDZ, strings, doubles and other values retain interpreter execution.
Typed cache identity includes closure count and boolean tags. Writes, calls,
heap operations, unsupported opcodes and invalid control flow are never compiled.
Extra unused arguments and unused local declarations no longer exclude otherwise
verified code. Older protocol imports fail closed to interpreter fallback.
Cold uncompiled entries can be replaced; executable pages stay domain-owned,
bounded and W^X, using the unchanged seal/release mechanism.

Guest SHA256: f12de79556fbcac8d9d14e2c00f8a55e2211f074606c65f03f92ebacf5132241

## Verification so far

- 73 DOM and 8 library tests pass on the ARM host (no native x86 execution claim).
- Native differential gate passed off/on with 12 compiled functions, 13,415
  executions, 3 fallbacks, 0 seal refusals, actual per-domain seals and matching
  numeric/boolean/closure outputs. Raw receipt:
  `target/browser-evidence/native-jit-v3/result.json`.
- The bounded private profiler is retained only under target/browser-evidence;
  profiling hooks are absent from the shipped guest and source patch.
- Real-page v3 compiles/executes Wikipedia (2 / 1,157) and GitHub (3 / 142).
  Both measured runs report zero compiled captured helpers: the closure path is
  native-fixture demonstrated, not yet demonstrated on these real pages.
- One sequential interpreter/JIT live pair measured cumulative elapsed time inside
  VM execution: Wikipedia 5.365 / 5.466 s; GitHub 30.157 / 29.679 s; Hacker News
  0.287 / 0.277 s (no JIT execution). This includes host calls/compilation, excludes
  network waits outside VM calls, and is neither host CPU nor full page-load time.
  Live inputs/scheduling were uncontrolled; no speedup is established or claimed.
  Exact inputs/raw logs are sealed in target/browser-evidence/real-page-measurements.json.
- Screenshots exposed a further Wikipedia failure: root height 114 despite article
  boxes extending beyond 21,000 px. Visible descendant overflow must contribute to
  document scroll height; fixed subtrees and clipping boundaries must not.
  The generic extent fix passes all 55 sex-html integration tests. Native off/on
  runs report height 22,112 and accept real Page Down input; inspected screenshots
  show article paragraphs and links. The earlier text-find pass alone did not
  establish this. Both runs also complete the separate local webmail workflow.
- Corrected-build Wikipedia measurement, same binary/ISO and 31 Page Down inputs:
  interpreter 11.705 s, JIT 12.018 s cumulative VM time. JIT compiled 2 functions
  and executed 154 times (captured helpers 0). This single live pair is slower;
  asynchronous workload variation is visible in execution counts across runs.
  No acceleration claim. Inputs, results, serial and screenshots are sealed in
  target/browser-evidence/wiki-scroll-measurements.json. The earlier table above
  describes the prior no-scroll revision, not this workload.
- CI, including the aggregate build and safety gates, must pass on the exact head
  before merge. Visible YouTube playback remains reserved for x17.
