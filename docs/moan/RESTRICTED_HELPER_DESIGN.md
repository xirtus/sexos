# Restricted helper: design for review, not implemented

Inventory inspected at c84dc5f8 (plus the separate8b070e96 exit predicate), not a
new kernel grant proposal. Current offline jobs prove ordinary crash containment.
No restriction below is claimed to exist until implemented and guest-tested.

## Existing entitlement inventory

`kernel/src/pd/create.rs` calls `CapEngine::grant_initial_rights`: initial VFS Node,
self Domain and self-signal/Interrupt entries. `kernel/src/init.rs:936` replaces
program storage slot1 with Files and gives Display5 and Shell6. Lines1033 onward
also give every program Net18 and Bell12. A network route exists even without a
network grant: Net's exact active Shell binding is the policy boundary. Program
rows do not inherit Spindle's Linen route, device/DMA leases, controller rights,
Browser entropy/clock/trust grants or Shell's lifecycle/activation grants.

| Existing surface | Current scope | Restricted-role policy required |
| --- | --- | --- |
| Files Domain slot1 + fs Region258 | own persistent `/data/<program>`, explicit mutable @ grants, picker, PREPARE, packages if granted | deny ambient paths, picker, offers/DnD, package/relay/prepare; immutable job-only read plugin/input, bounded create/write output |
| Display Domain5 + text/pixel regions | own text/pixel surface, compositor messages | deny all publication/window/input-provider operations for restricted rows, even direct calls; own region writes cannot publish |
| Shell Domain6 | window/focus, clipboard, autofill, launch/kill/exit, other app operations | permit only exact child OP_EXIT and read-only bootstrap status; deny all delegation and unrelated opcodes before generic handlers |
| Net Domain18 + per-row Net region | usable only for Shell-bound active network run | never bind restricted run; deny every request for quarantine/restricted subjects; clear stale row state before activation |
| Bell Domain12 + per-row sound Region260 | own playback ring, mic after consent, tones/notices | deny requests and exclude row from periodic ring consumption/capture, not just OP_PLAY; clear old run consent/cursors |
| Self Domain/own inbox + self-signal | own message queue/task | no provider/caller identity may be derived from payload; raw self-IPC cannot acquire server grants |
| Self catalog AppLifecycle slot48 | QUERY of exact own subject | read-only identity remains usable; does not grant ACK/RETIRE/control over others |
| SYS81 heap and SYS82 code | current program's own window; W^X/code scan | required for source-built native modules; retained self-only. Does not bound total system resource exhaustion |
| SYS83 workers, own finish/wait/ticks | workers/current run and scheduling mechanisms | native code can invoke these directly; SDK conventions cannot prohibit them. Resource/DoS scope must be measured separately |
| Public SYS_STATS87/serial/bridge info | metadata/own serial payload, typed validation on buffers | not a secret-grant channel; native code can disclose its granted audio to serial. No claim that child output is confidential from console observers |
| Other cap-checked syscalls | require typed capabilities not held by ordinary program | guest negatives must prove raw device, DMA, lifecycle/control and foreign-region requests refused |

`kernel/src/boot_memory.rs` creates permanent per-row Files/Net/text/pixel/sound
regions. They are not parent-child channels. Child and providers share only that
row's region, with no parent project pointers. Writable own sound/FS/Net memory
makes provider policy necessary: rejecting one IPC verb alone is insufficient.
The program PKU/tenancy mechanisms provide the separate private-address boundary;
a real parent-canary probe remains required. Shared-key rows are time-multiplexed
private translations, not a claim that every child has a unique hardware key.

Current generic Files launch can mint `+launch` until separately reviewed repair
2f85fff7 lands; this design must not assume it has shipped. Even that repair does
not deny ordinary descendants. Parent exit currently does not cascade-kill them.

## Fail-closed userspace handshake proposal

No new domain, kernel capability, kernel syscall, app slot or shared-memory API.
Reuse held Shell->Files/Display/Bell/Net routes. New messages would be narrowly
source-authenticated policy commands, not a new generic delegation protocol.

1. Parent asks Files PREPARE for an **explicit restricted job**, with plugin/input
   read paths and output-create path, nonce, cookie, byte caps and render metadata.
   Files checks exact parent domain+task and every grant. Normalize once using the
   real filesystem resolver: reject dot segments, repeated separators, NUL, path
   aliases and input/output identity collisions. No child argument can mint this
   policy. Store an immutable single-use ticket bound to exact parent run and job.
2. Shell consumes that ticket once, refuses RUN_NETWORK and reserves a free,
   already-retired row. It **quarantines the entire target row before activation**.
   Files, Display, Bell and Net each ACK quarantine for the same launch generation.
   Missing/foreign/duplicate/stale ACK, queue full, deadline or peer restart cancels
   launch before the kernel activation. Native code cannot execute before every
   provider has a deny-by-default row fence. Existing parent grant does not change.
3. Only after quarantine ACKs does Shell use existing activation/record machinery.
   Kernel then establishes the actual new task. Shell queries its authenticated
   custody and sends an exact target+task+parent+cookie+nonce/generation binding.
   Until all exact-task ACKs, the row remains fully denied. A task guessed by user
   payload cannot unquarantine it. Trusted bootstrap may poll read-only readiness;
   plugin entry is deferred until the same binding is ready everywhere.
4. Files allows only exact new-run plugin/input reads and output create/write,
   offset+length/file/total byte caps, truncate only within budget, and completion
   sync. No relative own folder or inherited @ prefix is ambient authority. No
   picker, rename/link/delete, output replacing an input, path expansion, package,
   relay or PREPARE. Input integrity against concurrent folder grantees requires
   an explicit immutable snapshot/hash binding or source-recheck policy; nonce
   alone does not authenticate files. This remains an unresolved design choice.
5. Shell allows exact task OP_EXIT only. Net and Display remain deny-all; Bell
   ignores shared-ring state as well as IPC. Only Files permits narrowly bound
   job requests. A raw BOOT route invoke must encounter the same provider fence.
6. On timeout, cancellation, crash, parent close or provider/parent restart,
   quarantine first, revoke active file permissions and output publication, then
   force-end the exact child run and wait for actual retirement. Preserve the
   row tombstone/generation until a later fully acknowledged launch. Old task
   requests, output or ACKs cannot authorize a reused row. Parent import requires
   exact successful retirement, expected full nonce and bounded validated result.

Every provider must start with program rows denied until an authenticated Shell
binding; a provider restart must not silently restore normal-mode defaults for
an already-running restricted child. Policy ACKs need existing peer-state epoch
fences. A Shell restart similarly cannot forget restrictions and accept normal
operations. This is a real multi-provider policy change needing review, not a
voluntary SDK `restricted=true` boolean or trust in a helper executable's name.
Trusted helper image digest/ABI still needs verification before its bootstrap is
trusted to load arbitrary modules. Restrictions must hold even if that bootstrap
is bypassed by native code.

## Required proof before hostile-boundary claim

Actual guest probes must attempt foreign parent canary read/write, direct Files
outside job paths and writes to read-only inputs, output overflow/alias, network
without grant, picker/relay/package, direct Display/Bell requests and ring writes,
raw typed device/DMA/lifecycle calls, launch descendants, queued stale-task exits,
old ACK/data after retirement and same-PD new-task reuse. Include provider/Shell
restart and partial-ACK/queue-full negatives. Keep Moan responsive, show visible
failure, and prove original project/audio unchanged while a child is killed.

Confidentiality/integrity and crash containment are separate from DoS. Existing
self memory growth, workers and serial remain callable by native code. No hard
CPU/memory/disk/global resource sandbox is claimed without independent mechanisms
and measured limits. Offline timeout is recovery policy, not hostile containment.
