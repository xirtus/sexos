# Guest string fast paths, not native object/string lowering

The object/string branch now has guarded string/string addition and comparisons
inside QuickJS's bytecode engine. These use its existing concatenation allocator,
UTF-16 comparison and reference-counting routines. Mixed types, objects, symbols
and coercion retain the ordinary path. Failed concatenation consumes its inputs;
the fast path clears both operand slots before exception unwinding, preventing a
double free. Neither string nor object pointers enter generated native JIT code.

These are opt-in guest fast paths, reported separately from native JIT counters.
They do not implement native object/string bytecode lowering. Do not describe
these counts as newly compiled functions or claim the requested native JIT work
is finished. The production default remains off.

## Verification and provenance

Eight targeted differential tests pass, including Latin-1/UTF-16, lone surrogates,
NUL, all eight comparisons, empty strings, retained references, mixed-type and
throwing coercion, and an 8 MiB heap concatenation failure followed by successful
execution. A separate default-feature-off evaluation test passes.

Native off/on results match independently computed UTF-16 concatenation sums and
comparison counts, plus property/getter/proxy effects. String fast-path counts
are 0/0 off and 67 concatenations / 582 comparisons on. Native code remains
12 compiled / 13,415 executed / 3 bailouts / 0 refusals with actual domain seals.
`target/browser-evidence/object-string-native/result.json` records exact inputs.

The preceding property-cache commit bf6f5e940 trimmed required patch context to
avoid whitespace warnings, making that patch fail to apply despite its previously
built/tested binary. This follow-up restores the context and successfully rebuilds
the guest. Preserve that failed build log; prior binary evidence remains specific
to the sealed binary, not proof that bf6f5e940 could rebuild it. Blank context
lines in the unified patch intentionally contain their required space prefix.

Guest SHA256 0fd3f83cd70c9f0caae4783c0eadc0439036275883a64c9af0ffd74d1fd2f197.
Browser SHA256 69e3e7b24f9b8c192e2caa7dcab1912403cd147412014f5d1843dd96b9d3cdaf.
Same frozen ISO c6eaec57eb55cbbfd4b9bd1afc20caa8b6531d342e73ba8629ad89040443356c.
No new hardware or visible YouTube playback qualification.

## Live diagnostic pair, off then on

| Site | Off VM seconds | On VM seconds | Property-cache hits | String concat / compare | Native compiled/executed |
|---|---:|---:|---:|---:|---:|
| Wikipedia Microkernel | 12.040 | 11.345 | 164789 | 1972 / 149954 | 2 / 154 |
| GitHub xirtus/sexos | 23.227 | 23.636 | 212867 | 1703 / 119389 | 3 / 142 |

Both site runs and the separately reported local webmail workflow completed.
Times are cumulative VM elapsed time including host calls, not CPU or page load.
Live bytes, asynchronous work and host load remain uncontrolled; results are
mixed and establish no speedup. Do not compare against a prior run with different
native execution counts as if it were a matched workload. Sealed source, inputs,
results, raw serial and screenshots: `target/browser-evidence/object-string-measurements.json`.
All local native runs were reaped.

PR274 separately advanced to 83a2b514 after its prior run passed nineteen jobs
but the daily-use gate stopped before session-save completion. Master already
contains the exact wait. Fresh run 43654690898 must pass all checks including
aggregate build before merge. This branch's object/string work remains excluded.
