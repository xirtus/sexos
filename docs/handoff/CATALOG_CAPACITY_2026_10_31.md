# Round 134: catalog-derived reservation bounds

Source `21e7a960` removes independent six-app lifecycle storage and three-client
document sizing. Kernel resource arrays, Display lifecycle storage, lifecycle
slots, Quil client targets and document-provider slots now derive their bounds
from the same descriptive BOOT incarnation catalog.

Display's presentation receipts use a 64-bit mask instead of an eight-bit mask.
Each incarnation retains a distinct bit, including index 63. An explicit
compile-time ceiling rejects a catalog above 64 entries rather than permitting
shift overflow or aliased presentation receipts. This ceiling describes receipt
storage; it does not establish a 64-app BOOT memory/surface/task budget.

Existing document identities 6/7/8 remain unchanged. Further document clients
use a separate BOOT region namespace beginning at `0x2000`, avoiding UiIR's
permanent identities 9/10. Provider slots and BOOT selftest arrays follow the
catalogued client count; each client is checked against every foreign provider
slot. Additional identities are labels, never runtime authority.

The registered catalog gate compiles the actual shared, kernel, Display and
document consumers with synthetic catalogs of 7 and 64 Quil entries. It checks
all derived lengths/slots/targets, distinct presentation bits and noncolliding
document identities. A 65-entry variant must fail the explicit compile-time
bound. These are capacity controls, not actual app reservations or authority
genesis. The production catalog still contains **six** entries.

Qualification uses the exact frozen source: 576 production plus 579 runner
inputs, all matching committed Git blobs. It passes a sealed GREEN build and
35-second probe, all 43 registered host checks with 2,663 passing Rust executions,
and 965 Python tests. The host source manifest stays unchanged throughout.
Before/after comparison finds 63 identical BOOT grant/resource/successor/catalog
records against the previous integrated image.

Native checks cover both desktop sizes: connected live/cold sessions, plus
find/replace/undo, durable save, cleanup, successor recovery and launcher
exhaustion rehearsals. Six workflow boots produce 76 verified captures; the
sealed probe adds a seventh boot. Every boot carries all six catalog audit
receipts before sealing. Fresh connected and app-workflow replays pass.
Their final results are retained in the
[machine evidence](assets/sexos-catalog-capacity-2026-10-31-evidence.json) and
[raw archive](assets/sexos-catalog-capacity-2026-10-31-raw.tar.gz), with immutable
frames, serial prefixes, disk snapshots and fresh app-workflow replays. Archive
members are size/hash verified. Earlier pending-device and sustained-image
qualifications keep their own bindings; this round does not relabel them.

ISO: `f854239c5053a2724f1d986361f97c59674273f7ec133a6cc634ce18fa0b8660`.
Kernel: `bd2a3fec9fc36bb16d28c1f9773b8e29cbe6f7ed17506363ebf33949a1dfba54`.
Five ISO-extracted app/service ELFs scan 225,473 decoded instructions with no
inspected vector/FP matches; this is a static instruction scan only.

Larger actual reserves remain unfinished. Next migrate the remaining
incarnation-specific app self-checks, service admission/save/history policies,
Shell delivery capacities and task/surface budgets. Browser successors also
need exact BOOT-held network/clock/trust routes and per-consumer broker state;
adding a row alone would fail its current exact PD15/surface300 identity check.
Then reserve additional private images and prove repeated close/reopen and
explicit exhaustion without new runtime authority or a process address space.

GLOBAL_VAS remains addressing, BOOT remains capability genesis, typed held
grants authorize operations and PKU enforces access. Inventory remains
**2 Open / 28 Partial / System Proven false**. The daily-driver goal stays active.
