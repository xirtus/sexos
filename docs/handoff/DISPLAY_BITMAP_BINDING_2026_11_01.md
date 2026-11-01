# Round 151: held-task Display bitmap binding

Display's bitmap backing now starts statically reserved and unbound. During
startup, a successful held lifecycle query supplies a checked, immutable
slot/target/task/surface/coordinator binding. It must match the Browser catalog
row and the Applied protocol's surface range. Initialization installs only small
metadata into the existing backing, once, before message intake. Missing or
ambiguous startup ownership stops initialization; packets cannot bind or rebind
storage.

Each bitmap request must match the actual kernel-stamped sender task as well as
its PD and content surface. The exact cached lifecycle tuple must still be LIVE,
and the rendered surface must be active and owned. These checks precede staging,
coverage, sequence, poison, pixel and receipt effects. Sampling uses the bound
association, and cleanup retires the store only for the exact target/task from
verified lifecycle proof.

Live reset still erases pixels while preserving the sequence floor and any
historical publication receipt. Full retains the bound destination and accepted
sequence. Terminal retirement erases storage, drops pending receipts and remains
permanent; reset or initialization cannot revive it. Clipping, alpha blending and
render-before-cleanup-ACK ordering are preserved.

Actual Browser remains **15/task15/surface300**. This phase creates no successor
rows or grants. Each later Browser needs a distinct backing/receipt/tombstone
lifetime. Adjacent Display text/fill protocols retain their existing PD-based
checks; this bounded bitmap change is not generalized per-task authorization.

## Verification

Source commit: `b45c1bf604f80132b35be6dda970a88ca5f912ad`.
All **43 frozen-source checks** pass: **2,773 logged Rust test executions**,
**8 additional lifecycle cases**, **5 metadata plus 3 extracted actual LIVE-helper
cases**, totaling **2,789 Rust executions**, and **1,042 Python tests**.
The bitmap fixture retains its 12 original cases and adds 5, covering unbound
packets, unequal held task 15/150, wrong-task rejection at every opcode, Full
historical receipts, exact retirement and permanent no-rebind behavior.
Two compiled binding mutants and three main-wiring counterexamples are rejected.
The compositor host fixture was adapted to explicit initialization before the
source commit; it restores no production default binding.

Eight boots and **112 registry-verified captures** pass: sealed probe, both full
eight-incarnation rehearsals with fresh read-only replays, four connected-desktop
boots and auxiliary cold recovery. Every boot records the exact held Display
bitmap tuple. Connected checks include actual TLS image presentation, document
storage and cleanup. The final writer 29/task 29 generation 55, 761-byte history
recovers into fresh Spindle 12, with Up recalling `echo alpha29`. The auxiliary
cold observation is bound to its independently replayed parent rather than a
standalone cold-artifact replay.

All 578 production and 606 runner entries, 1,184 distinct files, match committed
Git blobs, frozen copies and their source binding. Nine exact ISO-extracted ELFs
contain 325,243 decoded instructions with zero inspected vector/FP matches. The
scan establishes no runtime register/cache erasure proof.

ISO SHA256:
`ffecba591509876e41178e4063c0dd874c600dd50466ed5ba61b8e8e0c7364d6`.
Kernel SHA256:
`08bd29a9099ad90a8fa91698b973b18c1d9f6aa5e65dadbf6291f9e0f2064b82`.

The [evidence](assets/sexos-display-bitmap-binding-2026-11-01-evidence.json)
and [raw archive](assets/sexos-display-bitmap-binding-2026-11-01-raw.tar.gz)
retain qualified inputs, artifacts, controls and diagnostics. All 1,530 archive
members were reopened and verified by byte size and SHA256. Size 12,976,084 bytes;
SHA256 `f0a7c9a291ed803c97fd690fbd2a8ffa37d5dfcc07dd5c3c279b299711103940`.

The [next input plan](BROWSER_INPUT_BINDING_PLAN_2026_11_01.md) preserves queued
destinations while migrating Browser classification and stamped-task viewport
admission. Preparation also identified an upstream Shell custody task==PD
assumption, to replace with actual held-task authentication. Individual BOOT
source grants and additional actual Browser reservations follow those checks.

GLOBAL_VAS remains addressing only, BOOT alone creates capabilities, typed
sex-pdx authorizes and PKU enforces. There is no process VAS or runtime capability
minting. Inventory remains **2 Open, 28 Partial, System Proven false**. A native
unequal-task allocator, broader protocol task checks, Files deferred-task
authentication and new-image 1800s acceptance remain outside this qualification.
