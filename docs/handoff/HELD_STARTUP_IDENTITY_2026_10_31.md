# Round 142: app startup preserves held task identities

Source `366cb211` removes domain/task equality assumptions from Quil and Spindle
startup, Spindle status construction, Linen admission and Shell status binding.
A domain identifies a protection subject; its registered main-task identity is
kept as the exact value returned by held lifecycle authority.

Both apps still require a successful SELF_CATALOG query before any identity-state
write. Their actual startup helpers select the expected family/surface and use
`CatalogInfo::is_subject` to validate coordinator and nonzero held task. They
preserve that task unchanged in workspace, history/status, view, close and serial
state. No metadata lookup creates a capability or substitutes for the query.

Spindle's publisher/receiver accept a nonzero held task independently of the
catalog domain and retain exact packed tuples, sequence/state checks, Full
retention and retirement. Linen selects the expected catalog association, then
checks its held lifecycle subject and LIVE result without reconstructing the
task from the domain. Its opcode and domain admission behavior is unchanged.

Shell starts each status receiver unbound. It binds once from actual custody
only after the held catalog tuple, LIVE state and fresh IPC `caller_task` match.
Caller-selected packed arguments cannot choose the binding. Wrong sender task,
wrong catalog, nonlive/dead/closing custody and stale/malformed notices are
dropped; terminal receivers remain retired. This strengthens fresh status IPC.
The separate file-service deferred caller-task propagation gap remains open.

Host checks compile exact production startup and Shell receiver functions with
actual SDK/catalog/custody types. Six startup tests and four compiled controls
exercise every current app family row with independent domain/task IDs; seven
status tests and two controls cover exact tuples and Full; five Linen tests and
two controls retain held liveness; eight Shell tests and four controls reject
domain-as-task reconstruction, forged IPC task, catalog bypass and LIVE bypass.
Kernel-query transport and UI effects in these fixtures are explicit doubles.

The initial A cohort passed its narrower startup/status checks, but review found
Linen and Shell still synthesizing tasks. It is retained as superseded evidence.
The final B cohort completes the path before publication. Native BOOT still uses
coinciding task/domain numbers; this result does **not** claim a runtime allocator
with unequal IDs or universal task-namespace independence.

Final qualification passes sealed GREEN build/probe, all 43 registered host
checks / 2,716 Rust executions, all 989 Python tests and both complete eight-Quil/
two-Spindle relaunch sessions with fresh independent replay. Three boots produce
50 registry-verified captures: compact 24, wide 26. Both Spindle12 and17 statuses
are accepted through the stricter fresh-sender check. Every boot audits the held
recovery roster, all eleven catalog subjects and pre-seal ordering. Eight
ISO-extracted ELFs, including both changed apps, Shell and Linen, scan 290,390
instructions with no inspected vector/FP matches; this is static inspection.

All 576 production and 593 runner inputs, 1,169 distinct files, match committed
Git blobs. [Machine evidence](assets/sexos-held-startup-identity-2026-10-31-evidence.json)
binds the [raw archive](assets/sexos-held-startup-identity-2026-10-31-raw.tar.gz):
2,721 members / 21,255,109 bytes, each independently size/hash checked. It retains
both source cohorts, final immutable frames/prefixes and terminal disks, host
controls, source/layout audits and fresh replays with their exact scope.

ISO: `425139ab2c9dcd7c43f7b2c89a70610d306578791a929f74165f21a22fc7561e`.
Kernel: `c86f67462a081cc92255f17d648da2dfd6ffa6d4bf7cb13e1dbb9c903152b434`.

The actual reserve remains eight Quils and two Spindles. Browser's independent
task policy and file/deferred request-task authentication are not changed.
The older recovery-roster image's sustained jobs remain separately bound. Next
add one third BOOT-reserved Spindle after checking all route/authority/storage
budgets; prove 12→17→24, exact history/probe/network cleanup, cold recovery and
exhaustion before increasing further. GLOBAL_VAS remains addressing, BOOT alone
creates capabilities, typed sex-pdx supplies authority and PKU enforces it.
No process VAS, runtime mint or Unix/POSIX mechanism is added. Native unequal-task
allocation, new-image sustained acceptance, physical/power-loss, SMP, hostile
containment, global epoch/replay and unrestricted daily-driver readiness remain
outside this result. Inventory stays **2 Open / 28 Partial / System Proven false**.
