# Round 135: app and service policies follow BOOT identities

Source `b21ca26b` migrates the remaining fixed Quil/Spindle activation selection,
app startup identity checks, Shell geometry/input/view delivery, document service
participants, and Linen/Browser file-service admission to catalog metadata.
The production roster remains six BOOT-reserved incarnations.

Quil and Spindle first query their held self-lifecycle capability and validate
its target, task, surface and coordinator against the correct catalog family.
Successor startup and presentation depend on the validated ordinal. Shell walks
the ordered activation grants, checks each returned tuple against actual custody,
stops on grant errors and selects routes only from an active matching receipt.
Reserve counts reject impossible ordering. Child panes, viewport storage,
Spindle status receivers and Quil toolbar routing use the same associations.

The file service isolates sessions by catalog participant index, preserves
Browser's create-new-only object/generation restriction, and keeps pending
receipts bound to their exact caller. The snapshot resolver still queries the
actual held Region capability and checks immutable peer/rights metadata; the
catalog only selects its expected slot. Linen retains its actual lifecycle
query before admitting Spindle requests. Catalog lookup creates no authority.

A compiled expanded-policy fixture adds hypothetical Quil100, Spindle101 and
Browser102 rows. It runs actual policy/service modules with explicit kernel,
transport and backend doubles. Tests prove the fourth Quil's exact grant/route,
denial without skipping, future grid/geometry associations, distinct reply
sessions, and Browser102's create-only isolation. Compiled regressions that stop
at the previous chain end or restrict only Browser15 fail the tests. This is
host policy evidence, not a nine-incarnation BOOT or grant-creation result.

Review caught an original-Spindle startup regression before publication: its
Shell frame has geometry before app registration. The final helper preserves
that original path while successor frames use registered custody bounds. A
focused regression checks the distinction; native workflows exercise startup.

Qualification uses two immutable checker snapshots with **identical production
inputs, ISO/kernel, native scripts and Python inputs**. B binds committed source
`b21ca26b`; C binds `add0c94e`. Their sole runner difference is
`scripts/launcher_host_gate.sh`: its stale target17/frame306 assertion now checks
the catalog-selected tuple. B's failed host check is retained; C supplies the
complete corrected host run. Original snapshots and bindings are not rewritten.
Every snapshot's 576 production and 581 runner inputs matches its Git blobs.
The production BOOT authority comparison preserves 63 exact records.

The corrected host cohort passes all 43 registered checks with 2,667 passing
Rust executions; the unchanged Python inputs pass 965 tests. The production
image passes sealed GREEN build/probe plus 18 workflow boots and 138 verified
captures across both desktop sizes. Clock states, connected live/cold desktop,
TLS responsiveness, pending-device Close/cold recovery and app/find/relaunch
rehearsals pass, with fresh pending-device and app replays. All six catalog
receipts precede sealing in every workflow boot and the probe: 19 boots total.
Five exact app/service ELFs scan 226,072 decoded instructions with no inspected
vector/FP matches; this is a static scan, not runtime register/cache erasure.
These results are recorded in the
[machine evidence](assets/sexos-incarnation-policy-2026-10-31-evidence.json).
The [raw archive](assets/sexos-incarnation-policy-2026-10-31-raw.tar.gz) retains
both source/checker snapshots, immutable frames/prefixes, terminal disks, host
results, replay records, expanded controls and original diagnostics. Each member
is size/hash verified. The evidence states the exact counts and checker scope;
it does not claim sustained acceptance or physical-machine qualification.

ISO: `501b52d0b4c48f78badd939533f700872ee893e087c8053a0fa2eef31f2d41ae`.
Kernel: `867d6ec0657cd5b0d1c379b7296a5ec63f88583ebe355f2537c573c72f557210`.

Next add actual Quil reservations and verify the entire longer close/reopen
chain, including the additional document backing, end-of-reserve launcher state,
durable recovery and cleanup. Remaining Spindle journal/recovery ownership and
Browser's exact self-identity/network/clock/trust brokers require further work
before those reserves can grow. Task/surface/private-page budgets must be checked
against actual BOOT objects when rows are added.

GLOBAL_VAS remains addressing; BOOT creates capabilities; typed held grants
authorize runtime operations; PKU enforces access. No process VAS or POSIX layer
is added. Inventory stays **2 Open / 28 Partial / System Proven false** and the
daily-driver goal remains active.
