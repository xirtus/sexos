# Restricted helper policy wire

Proposed userspace protocol; no handlers or permissions below are active yet.
The existing image pin and staging guard are separate prerequisites.

## Existing authority and insertion points

| Provider | Held route from Shell | Guard before | Persistent/non-IPC work |
| --- | --- | --- | --- |
| Files (11) | program_store::SHELL_ROUTE_SLOT (256) | trampoline deferred replay dispatch, before config/FS/package/relay/recording handling | pending replies, writes, image staging, and Region258 scrub |
| Display (2) | existing Shell Display Domain | main intake before text/pixel/window/input branches | retained scenes and periodic publication; text/pixel views are READ-only |
| Bell (12) | existing Shell Bell Domain | intake before sound/notices | periodic sound ring consumption, microphone capture and Region260 scrub |
| Net (13) | web::SHELL_SLOT | main intake before jobs/bind/source handling | retained flows/responses and whole row Net-region scrub |
| Shell (3) | own coordinator state | ordinary launch/window/clipboard/autofill/open/kill dispatch | generic program activation and rebirth poll |

Route slots are capabilities already held by the provider/coordinator. A numeric
PD or packet field cannot create one. Provider wrappers must query their held
Shell route using crash_log::peer_state, require Life::Live and exact caller
PD3/task3, and bind the observed epoch. Shell similarly snapshots each held
provider route before and after each phase. App catalog/lifecycle grants attest
reserved row, exact task and retirement; metadata in a message does not.

No provider executes a policy operation received from a parent or plugin. Files
alone accepts a parent job proposal through an existing explicit file grant;
Shell obtains the resulting immutable single-use ticket from Files. Shell derives
the target from program_free_row, never a parent-supplied target. The first
quarantine phase is admitted only for that catalogued, fully retired reservation.

## Bounded transcript

Use a dedicated command opcode and a distinct reply opcode, allocated after a
repository collision scan. Each command is exactly three u64 words. All providers
permit only one in-progress transcript; backpressure retains the exact command
and reply, with bounded deadline and retry, rather than advancing on enqueue.

- Word0: phase8 | generation56. Generation is persistent, increasing and never
  wraps. Exhaustion refuses restricted launches.
- Quarantine: word1 reserved target32|previous task32; word2 expected provider
  epoch. Validate exact held catalog and terminal custody, durably fence the
  entire row, stop old work, clear provider-owned regions, then ACK.
- Owner: word1 parent32|parent task32; word2 cookie32 with upper bits zero.
- Nonce: word1/word2 are the complete 128-bit request nonce.
- Bind: word1 newly loaded target32|task32; word2 immutable Files job ticket.
  This task comes from synchronous load_program(kind3), while still dormant.
- Seal: word1 new target/task; word2 expected provider epoch. Requires the exact
  completed transcript and fresh matching Shell/provider epochs. Files installs
  only its resolved bounded job rights; other remote providers remain deny-all.
- Revoke: word1 exact target/task; word2 expected epoch. Narrowing only; it cannot
  remove the row tombstone or restore normal access.

Every positive ACK echoes all three accepted words under the distinct reply
opcode. Shell requires exact provider subject, live epoch, command and transcript
position. This binds every Owner/Nonce/Bind field without treating a lossy hash as
a capability. Duplicate identical retransmission repeats the original ACK; a
same-phase different payload or wrong order quarantines the transaction. A new
provider epoch invalidates every old ACK. Queued old commands cannot match the
new expected epoch, even where service restart preserves the same PD/task.

Shell records its local deny fence as the fifth participant. It explicitly holds
both initial activation and program_rebirth_poll while the policy transcript is
incomplete. Existing lifecycle cleanup ACKs and these policy ACKs are different
contracts; neither substitutes for the other. No timeout or failed ACK activates
a normal helper as fallback. The existing inability to unload an aborted dormant
rebirth consumes that reservation until reboot, as documented separately.

## Persistent deny fences

Before any deferred message replay or ordinary requests on server entry, register
one private KEEP page (at most four pages if layout requires) using existing
SYS_CRASH_LOG::OP_KEEP. Follow the existing recording journal's validated integer,
checksum/commit-last pattern: no Rust pointers, capability objects or enums are
resurrected. Store per-row target, latest exact task, generation, Shell epoch and
quarantine state; job rights are revalidated, never restored as positive authority.

Only epoch1 plus an entirely zero cold page may initialize an empty ledger.
Missing/invalid keep, checksum, version, duplicate row, impossible task or unknown
restart state means all Program rows denied, before any request or periodic ring
inspection. A recovered restricted row stays denied regardless of saved phase;
recovery never silently changes it to ordinary. Partial writes therefore lose
availability, never restrictions. A persistent Shell ledger similarly retains
reservations/tombstones before its normal event loop.

Ordinary reuse of a formerly restricted row needs a separate explicit normal-run
bind: newer generation, real old-run retirement, different exact catalog task,
provider scrub and fresh epochs before activation. A normal bind cannot downgrade
a still-live restricted task. A missing ledger cannot be repaired by a plugin's
normal-looking registration or its own guessed generation.

## Trusted bootstrap and ready challenge

Files hashes the exact canonical helper image under the central writer/lease
mutex. All image writers, including load_unbound/recording, pass through try_load.
Hold the lease until synchronous kind3 private copy and matching Files epoch.
Never release on a stale completion. The canonical build binds the helper ELF
SHA/length to the resulting Files ELF; parent-supplied hashes are not accepted.

The trusted helper's first action after heap/stack initialization clears its own
permanent text/pixel buffers before reading a module or branching to its code.
Other providers have already stopped in-flight writes and cleared Files, sound
and Net regions before their quarantine ACK. Display cannot perform the text/pixel
scrub itself because its view is read-only. The helper then asks Shell Ready with
its exact run/generation/full nonce; Shell checks its completed transcript and
fresh four-provider epochs. Foreign, stale, interrupted or unavailable readiness
fails without plugin entry. Native plugin code can later bypass its own helper,
so every server's policy fence remains mandatory for the entire run.

Files resolves grants to actual volume/object identities once. Reject ambient
relative paths, aliases/dot segments, output==input/plugin, existing output,
rename/link/delete, picker, PREPARE, descendants, package/relay and unbounded I/O.
Bind immutable plugin/input snapshots or revalidated hashes and a new output with
server-enforced per-file/total/offset caps. A nonce is a stale-result fence, not
file authentication against another folder grantee.

## Required negatives and claims

Compile actual policy predicates and journal transitions in host tests: malformed
and out-of-order packets, same-phase changed payload, foreign caller, epoch change,
queue full, missing ACK, partial journal write/restart and normal downgrade fail
closed. Guest probes must test raw Files/Net/Bell/Display/Shell routes, parent
canary read/write, unauthorized device/lifecycle syscalls, shared-key row reuse,
late ACKs/results and provider restart. Keep these distinct from the already
proven offline processing/crash/timeout/reuse cases.

The role aims at file/network/parent-memory confidentiality and integrity. Own
heap/code/worker syscalls and public metadata/serial remain callable; bounded
process timeout does not establish global CPU/memory/disk DoS isolation. Current
plugin host is not a completed hostile-plugin sandbox or live insert.
