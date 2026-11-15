# Restricted helper policy wire

Proposed userspace protocol; no handlers or permissions below are active yet.
The existing image pin and staging guard are separate prerequisites.

## Existing authority and insertion points

| Provider | Held route from Shell | Guard before | Persistent/non-IPC work |
| --- | --- | --- | --- |
| Files (11) | program_store::SHELL_ROUTE_SLOT (256) | trampoline deferred replay dispatch, before config/FS/package/relay/recording handling | pending replies, writes, image staging, and Region258 scrub |
| Display (1) | existing Shell Display Domain | main intake before text/pixel/window/input branches | retained scenes and periodic publication; text/pixel views are READ-only |
| Bell (10; route12) | existing Shell Bell Domain | intake before sound/notices | periodic sound ring consumption, microphone capture and Region260 scrub |
| Net (13) | web::SHELL_SLOT | main intake before jobs/bind/source handling | retained flows/responses and whole row Net-region scrub |
| Shell (3) | own coordinator state | ordinary launch/window/clipboard/autofill/open/kill dispatch | generic program activation and rebirth poll |

Shell holds provider Domain routes and brackets each exchange with fresh live
peer epochs. Display and Bell also hold Shell Domain routes. **Files and Net do
not hold a reverse Shell Domain route on this cohort.** They cannot query Shell's
epoch or send a full3word OP_ACK; do not invent slot6 there. All providers use
existing SYS29 authenticated one-word replies, including a fresh provider-private
128-bit challenge after Quarantine. Provider-own epoch comes from checked
OP_RESTARTS(0), never a packet field. Freshness is the current provider epoch,
fresh challenge and persistent Shell generation, not a claim of remote Shell
crash observation. Kernel sender metadata must still be exact Shell PD3/task3.
App catalog/lifecycle grants attest the reserved inactive row (never activated or
fully retired), exact task and retirement; payload metadata does not.

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
- EchoChallenge: both words echo the fresh128-bit provider-private challenge.
  CPU RDSEED/RDRAND availability and bounded retries fail closed; never log it.
- Coordinator: word1 trusted Shell's declared self epoch, word2 provider epoch.
  The declaration is correlation, not a provider-observed peer snapshot.
- Owner: word1 parent32|parent task32; word2 cookie32 with upper bits zero.
- Nonce: word1/word2 are the complete 128-bit request nonce.
- PluginHashLo/Hi and InputHashLo/Hi bind all256 bits of each exact immutable
  admission buffer's SHA256. Each word carries64 bits, with no lossy substitute.
- Bind: word1 newly loaded target32|task32; word2 immutable Files job ticket.
  This task comes from synchronous load_program(kind3), while still dormant.
- Seal: word1 new target/task; word2 expected provider epoch. Requires the exact
  completed transcript and fresh matching Shell/provider epochs. Files installs
  only its resolved bounded job rights; other remote providers remain deny-all.
- Revoke: word1 exact target/task; word2 expected epoch. Narrowing only; it cannot
  remove the row tombstone or restore normal access.

After accepting/persisting a command, the provider retains all three words and
returns word0 via SYS29. Only then may Shell request word1/word2 using OP_FETCH:
`[exact phase/generation word0, selector1..2, expected provider epoch]`. For
Quarantine only, selectors3/4 then return the fresh challenge halves. Every fetch
must be in order; exact last-selector retry is idempotent. Advance a credit only
once the kernel admits the actual reply. The provider must not start the next
phase until required credits complete. Shell verifies each echoed word and the
kernel-authenticated provider subject, with one outstanding reply request.

Unique accepted generation/phase ACK0 is emitted only after the corresponding
operation really completed; payload credits alone cannot certify acceptance.
A stale selector/phase/epoch, changed same-phase payload, missing credit, queue
full or timeout keeps the row quarantined. Provider restart discards all positive
transcripts/challenges before replay while retaining deny tombstones. Old initial
commands name the wrong own epoch; later commands have no accepted transcript.
Never log challenge, job nonce, hashes or input payload, including Debug/fatal
paths. Packet Debug prints only generation/phase.

A live immutable job may continue through an unobserved Shell crash until its
hard120-second provider deadline. Pending policy exchanges expire after5seconds.
No immediate remote Shell-crash revocation is claimed. On Shell recovery, revoke
and terminate retained jobs before admitting normal launch work. Restrictions
remain enforced throughout: retained rights are only to immutable job buffers,
and every provider restart removes those positive rights before replay.

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

### Files immutable job admission (kind 30)

The parent sends an FS request with kind 30, empty path, offset 1 and a bounded
payload in DATA. `MOANJOB2` is 48 bytes followed by three explicit `@/` paths:
format byte at 8 (CLAP=0, VST3=1), zero bytes 9..12, cookie u32 at 12,
nonce 16 bytes at 16, three u16 path lengths at 32, and zero bytes 38..48.
Paths are at most 192 bytes each; controls, spaces, empty/dot components, aliases,
relative paths and trailing bytes are refused. All three paths must already be
held by the exact live parent main task. This first storage implementation admits
only the System sexfs volume; mounted/home backends are explicitly refused.

Files copies both inputs, computes their full SHA256 privately, reserves output
space fallibly, and holds at most two jobs for at most 120 seconds. Job IDs never
repeat within the Files epoch. A same-sequence retry must match all three request
words and every copied payload byte. SYS29 Full retains the exact pending reply;
actual delivery checks Files epoch and the current parent task again. Provider
restart discards all positive jobs; the separate deny journal remains deny-only.

This admission creates no child or positive file rights by itself. Shell CLAIM/DESCRIBE controls (0x4d505010..11) are dispatched only for stamped
PD3/task3. LOAD/COPIED (0x4d505012..13) remain unavailable.
Only a subsequent fully authenticated five-provider transaction can associate a
job with an attested dormant helper. No generic PREPARE ticket is reused.

CLAIM words are `[ticket, exact_parent_subject_word, cookie]`; Files checks the
live parent, immutable job, cookie and own current epoch. ACK0 is `0xd1 << 56 |
ticket << 32`, a descriptive receipt, not a capability. Before issuing another
phase Shell must receive ACK0, then fetch all 22 descriptor words in order via
DESCRIBE `[ticket, selector, expected_Files_epoch]`. Only successfully admitted
SYS29 replies advance the selector; exact last-selector retry is idempotent.
Changed/stale/out-of-order selectors and an early fetch never release a word.
The stream expires after five seconds and is fenced to current parent lifecycle
and Files epoch on every send. Shell independently brackets the actual held
Files epoch, so the short ACK does not substitute for a full epoch check.

The 176-byte `MOANINF2` descriptor is: magic8, ticket u32, frames u32, Files
epoch u64, nonce16, plugin SHA256, input SHA256, output SHA256 (zero until actual
publication), parent subject u64, child subject u64, generation u64, format u32,
cookie u32, reserved8 zero. Subject words encode PD in high32 and main task low32.
The descriptor is an immutable descriptive snapshot; only subsequent policy
transcript admission conveys rights. Startup and epoch changes never restore it.

### Exact helper copy receipt

LOAD `[ticket, generation, previousSubject]` requires the completed descriptor
exchange and the fully echoed InputHashHi scope. Files matches the claimed job's
full owner/cookie/nonce/hashes before loading the fixed name `moanplug`; a name
selects bytes only. The actual candidate must match the compiled SHA256/length
pin and the exclusive image lease. BOOT's image grant100 has peer=None, region
0x4000, exactly32MiB, Files RW; it is not a Shell shared-memory grant. Shell asks
the kernel to synchronously copy those retained BOOT frames via kind3.

The LOAD reply is `0xd2 << 56 | ticket << 32 | length`. After its admitted reply,
DESCRIBE selector22 (same exact ticket/Files epoch) yields the full lease serial.
Only then can COPIED `[ticket, generation, serial]` release the staging fence;
the provider uses its saved full scope/coordinator epoch, not a request-supplied
epoch. Exact retries do not release twice, and no phase advances on SYS29 Full.
The final receipt is `0xd3 << 56 | ticket << 32`. The coordinator is trusted to
send COPIED only after its synchronous kind3 returns. A missed/unknown copy does
not time out the staging fence open: this may quarantine image loading until
reboot. COPY success alone grants no file rights and does not activate a helper.
