# Restricted-parent normal packaging checkpoint

Source `dfb03d9375ba0ba44e18b975a307a700a26094ad` passes the normal sealed build.
The complete 1,981-file production source manifest is identical before/after;
all 84 production ELFs and the built ISO are preserved under
`target/restricted-parent-build-only-dfb03d937`. The ISO SHA-256 is
`655d76f2f66ad497ebdf31f148259759020069c55167ade6c90522d10ae7a210`.
`MOAN_RESTRICTED_PARENT_BUILD_2026_11_15.json` binds the result and consumed-input
receipt: Files compiled against the exact preserved 126,912-byte helper with
SHA-256 `1da037d2100cb6e4bebf3113debe22ad75d98f421f0180b28f6567bee45dd1ae`.

Local validation passes 1,376 Python regressions, 60 Moan host tests and 29 plugin
host tests. The prior full-suite failure is preserved: outdated Term owner-shape
checks, a stale allocation-failure transport fixture, and fake-process startup
racing the test-only observation clock were repaired with focused regressions.
Production runtime deadlines and the original Term denial predicates remain intact.

This is packaging evidence only. No new QEMU boot, crash/restart, isolation or
installer claim is made. Automatic security review stopped the runtime-test agent
both for memory-access probes and the reduced crash/restart scope. Those tasks
were stopped, with no retries through another route. Only ordinary compilation,
source review, host regressions and packaging continued.

Merging remains held. PR296's synthetic merge contains the newer unreviewed
master kernel and correctly fails its source contract. A separate runtime leaf
also reaches the author-owned Fuzz launch-during-session-save race covered by
PR295. No copies of that author's fix, kernel changes, source-check weakening or
unchanged runtime retries are included here. Moan runtime qualification and actual
current-master reconciliation remain outstanding; UI/MIDI/JIT follow-up is not
represented as completed by this build.
