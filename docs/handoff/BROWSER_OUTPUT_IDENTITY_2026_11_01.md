# Round 149: Browser-owned output identity

Browser startup now uses its selected held surface for creation, preview writes,
position, registration, presentation and the atomic text outbox. Registration's
app identity comes from the held target. The family title resource remains 300;
it names the Browser title and supplies no surface authority.

The bitmap outbox requires an immutable startup Identity. Begin, Chunk, Commit
and Clear retain its surface through Full/retry, desired-image coalescing and
late receipts. An equal sequence from another surface cannot complete the batch.
Historical accepted Commit/Clear metadata and terminal Close behavior remain
intact; there is no default or first-message destination binding.

Both legacy TCP/DNS status-paint helpers also require a constructor surface.
They retain it while draining immutable 64-byte status replacements, and reject
zero surfaces or invalid/overflowing offsets without sending. The active App
uses its composed text outbox; these legacy helpers are not instantiated there.
The shared Spindle wire-probe state machine is unchanged.

Actual Browser remains **target 15/task 15/surface 300**. Display's bitmap store
still admits 15/300 and has its existing task-authentication gap. This client
change adds neither a successor reservation nor a new Display grant. The next
step is [Display's one-time held bitmap binding](BROWSER_DISPLAY_BITMAP_PLAN_2026_11_01.md),
then captured Shell input and distinct BOOT successor grants.

## Verification

Source commit: `2b0758e471a97c876302d7a90622772456f18530`.
All **43 registered frozen-source checks** pass, with **2,768 logged Rust test
executions plus 8 extracted lifecycle cases**, and **1,042 Python tests**.
The output gate rejects two compiled fixed-surface mutants and five actual-main
wiring counterexamples. Binding cases include distinct host task namespaces and
future host surface 343; they establish no native future incarnation.

The initial Python suite exposed a pixel-oracle literal `Outbox::atomic(300)`
assumption. Before the source commit, its contract was updated to recognize exact
held wiring, hash the identity/catalog/lifecycle dependencies, reject rewiring,
and retain archival literal-root support. Native geometry remains surface 300.

Eight boots and **112 registry-verified captures** pass: the sealed probe, both
full eight-incarnation desktop rehearsals and their fresh independent replays,
four connected-desktop boots across both sizes, and an auxiliary last-writer
cold boot. Connected checks exercise TLS/image presentation and storage cleanup.
Both rehearsals close/exhaust all actual Spindle and Quil reservations. The wide
parent's exact target 29/task 29 journal generation 55, 761 bytes recovers into fresh
Spindle 12; Up recalls `echo alpha29`. This auxiliary recovery is bound to the
replayed parent and has no standalone artifact-replay claim.

All 577 production and 604 runner entries, 1,181 distinct files, match the committed
Git blobs, frozen copies and source binding. Nine exact ISO-extracted ELF scans
inspect 324,945 decoded instructions with zero inspected vector/FP matches. This
static scan does not prove runtime register or cache erasure.

ISO SHA256:
`a91d0805fb574de88dd6e7761930998ebc11c400b7bed9643cfd381db9f255d2`.
Kernel SHA256 remains:
`08bd29a9099ad90a8fa91698b973b18c1d9f6aa5e65dadbf6291f9e0f2064b82`.

The [evidence](assets/sexos-browser-output-identity-2026-11-01-evidence.json)
and [raw archive](assets/sexos-browser-output-identity-2026-11-01-raw.tar.gz)
retain frozen inputs, native artifacts, replays, controls and diagnostics.
All 1,554 archive members were reopened and checked by size and SHA256.
Archive 16,249,220 bytes; SHA256:
`6c792477b74589758a20b38363ddfefd318e8e59c082b66d97e8145ec8f7188b`.

GLOBAL_VAS remains addressing only; BOOT alone creates capabilities; typed
sex-pdx authorizes and PKU enforces. There are no process address spaces or
runtime capability mints. This new image has no 1800-second acceptance claim;
[Round 150](READABLE_SPINDLE_ALPHA_ACCEPTANCE_2026_11_01.md) separately qualifies
the older readable Spindle image. Inventory remains **2 Open, 28 Partial, System
Proven false**; Files deferred-task authentication and a native unequal-task
allocator remain open.
