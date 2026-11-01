# Round 155: catalog-driven BOOT Browser source grants

The current Browser's entropy, clock and trust grants now come from a descriptive
catalog plan. Production still contains exactly **one consumer, Browser 15**, with
Net-local slots **60/61/62**. The plan contains no task identity or capability.
The provider is PD 13; 13 is not a grant count. Future triples 70/71/72 and 73/74/75
exist only as descriptive candidates and explicit host fixtures.

BOOT resolves actual registered holder/consumer main tasks and checks their
context-to-PD association. It preflights every planned full-width slot for
uniqueness and vacancy before the first insertion, constructs the typed grants,
then reads back the actual inserted CapabilityData variants. Typed pure INFO
mechanisms and their own decoders validate consumer/profile/source/root/version;
BOOT neither impersonates Net's current task nor invokes a userspace syscall
wrapper. Entropy INFO uses actual instruction-support availability without
sampling; clock INFO does not sample CMOS. A late readback repeats before sealing
to catch later genesis overwrites.

Net startup separately executes its actual held lifecycle and typed INFO syscalls.
An immutable binding requires the exact queried slots, canonical Browser plan,
held task and all three consumer tuples. Missing, crossed, foreign or malformed
metadata fails closed without slot fallback or consumer rewriting. Unavailable
RDSEED remains valid grant metadata and keeps the existing Offline behavior.
Fresh source requests select by their authenticated owner; entropy advances
through the retained active owner's slot, even when another client requests work.
There is still one entropy, time and trust machine. Global clock high-water,
rollback poison and ordinals, source denial isolation and original lease behavior
remain intact.

## Verification

Source commit: `2501ebb8d9280d8a60f78390234bd41993d6d107`.
All **43 frozen checks** pass: **3,030 logged Rust executions plus 20 nested cases**
(**3,050 total**) and **1,051 Python tests**. The actual and host-future cohorts
execute 13+13 plan cases, 27+27 binding cases and 53+55 dispatch cases, including
reused SDK/mechanism tests. Ten invalid const-plan controls, private-field forgery,
three compiled binding mutants, nine dispatch mutant forms, nine dispatch wiring
controls and three catalog controls are rejected.

Host-future 15/150, 30/300 and 31/310 use separate slot triples. Tests require actual
query labels, deny bad INFO without provider effects, retain entropy 30/300 on
slot 70 across intervening 31/310 requests, then select 73 for 31. Clock 61→71→74
supports equal/forward samples and poisons all consumers on rollback, retaining
poison across retirement. These are host metadata/mechanism tests, not actual
future BOOT grants or a native unequal-task allocator.

Eight boots and **112 registry-verified captures** pass both desktop sizes,
full 8 app chains, fresh rehearsal replays, connected TLS image/document workflows,
cleanup and parent-bound cold recovery. In every boot, the late actual-table
readback precedes sealing and Net records its checked runtime binding. All five
source-authority record lines—entropy, clock, trust, SilkBar clock and TLS—are
**byte-identical to the preceding Round 154 image**, with the same order and values.
Ordinary source-backed TLS still works and direct raw source queries stay denied.
Writer 29/task 29 generation 55, 761 bytes recovers into fresh Spindle 12 with Up recalling
`echo alpha29`; auxiliary cold evidence has no standalone replay claim.

All 581 production and 616 runner entries, 1,197 distinct files, match committed Git
blobs, frozen copies and source binding. Nine ISO-extracted ELF scans inspect
327,421 decoded instructions with zero inspected vector/FP matches. This static
scan establishes no runtime register/cache erasure proof.

The new SDK plan is the 66th ordered ABI source input. Hash:
`73a2f41294dcf613de359a477955f678c66d79c469bd6eceec612016d5ff701c`.
Existing protocol layouts remain unchanged.
ISO SHA256:
`832d2697eb66e4877ad14f69a4bf2372e1a925d38da073008f518dd89245ac15`.
Kernel SHA256:
`3cba88b80c1685cba6f0b0b265d32d5f5cf4e3f1826fa84c2c3c5ceac91bdef8`.

The [evidence](assets/sexos-browser-source-catalog-2026-11-01-evidence.json)
and [raw archive](assets/sexos-browser-source-catalog-2026-11-01-raw.tar.gz)
retain qualified inputs, artifacts and controls. All 1,648 archive members were
reopened and checked by size and SHA256. Archive 19,845,875 bytes; SHA256:
`bee02dfefa3a47d4be78714817e5c60c3f852001341fc90bbe578897e680a69b`.

Next 156 supplies distinct static Display bitmap stores and retained tombstones
per catalog Browser. Actual successor rows, their six additional source grants,
Net routes, activation and complete selected-consumer evidence follow separately.
GLOBAL_VAS remains addressing only, BOOT alone creates capabilities, typed
sex-pdx authorizes and PKU enforces. No process VAS or runtime capability minting
is introduced. Inventory remains **2 Open, 28 Partial, System Proven false**;
broader network liveness, physical/SMP containment and new-image 1800s acceptance
remain outside this qualification.
