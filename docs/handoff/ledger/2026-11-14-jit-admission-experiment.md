# JIT admission experiment — withheld from production

PR274 head 0f7561a1 fixes Term's ignored relay denial and retains bounded IPv6
peer traces for its next CI failure. Run 43643275198 is pending; no merge is
permitted until every exact-head check and the aggregate build pass. Shared
IPv6/terminal repairs are also in open PRs245,293,295,296; do not duplicate or
silently import their larger stacks. YouTube visible playback stays with x17.

This separate branch explores cold-code cache starvation. Previously 64 hot
unsupported functions could occupy the cache while interleaved new useful and
cold functions repeatedly evicted one another. The regression fails before the
change and passes after it. A bounded 256-slot fingerprint counter now tracks
warm-up before allocating a bytecode entry. Fingerprints affect admission timing
only; full bytecode and scalar shape still identify cached code, compilation
still validates all operations, and executable entries are never evicted. Hash
collisions can delay warm-up; this is not a correctness or isolation authority.

Nine library and five targeted DOM tests pass. Native differential testing
matches forced interpreter output with 12 compiles / 13,415 executions / three
fallbacks / zero seal refusals, actual domain seals. Artifact-specific proof:
`target/browser-evidence/admission-warm-native/result.json`. Source/binary/ISO
hashes are in `admission-warm-inputs.json` in the same evidence directory.

## Real-page measurements

Cumulative elapsed VM execution, seconds (includes host calls, not full page
load or host CPU). Same runner and frozen ISO, sequential live HTTPS runs.

| Candidate | Wikipedia | GitHub |
|---|---:|---:|
| PR274 browser baseline | 11.975 | 22.305 |
| Guest scalar filter + failed-entry eviction | 13.422 | 28.131 |
| Guest scalar filter only | 14.317 | 27.074 |
| Host warm-up tracking, original guest | 11.213 | 25.755 |

The guest filter candidate was rejected and its source/wasm restored. Its
Wikipedia admitted workload matches baseline (1,473 native executions), but the
sample was slower. Rejected artifacts, logs and guest patch remain in evidence.

Host warm-up tracking eliminated observed cache-full events (Wikipedia 309→0,
GitHub 753→0), but compiled functions remain 2 and 3. Executions differ: Wikipedia
1,473→139, GitHub142→118. These live inputs and asynchronous work are uncontrolled;
this does not establish a speedup or improved real-page coverage. GitHub is
slower in this sample. Keep this candidate experimental, not part of PR274 or a
production performance claim. Do not infer a benefit from the synthetic
starvation regression alone.

`target/browser-evidence/admission-comparison.json` seals the inputs, results and
raw serial for all four runs. Local webmail remains a separate fixture and is
excluded from the table. All native VMs were reaped; no playback proof claimed.
