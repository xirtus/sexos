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
- Real-page v3 coverage and timings are still being measured. No speedup claimed.
- CI, including the aggregate build and safety gates, must pass on the exact head
  before merge. Visible YouTube playback remains reserved for x17.
