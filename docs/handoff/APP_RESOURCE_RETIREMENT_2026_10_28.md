# Boot app resource retirement — 2026-10-28

This implementation retains one GLOBAL_VAS and the immutable BOOT_DAG. The kernel records ownership and performs authorized transitions; sext receives and reuses the retired private bytes. No runtime frame allocator, capability mint, or per-app page table was introduced.

## Enrolled resources and retained identities

Boot enrolls Quil PD9 (content156), Spindle PD12 (primary content154), and Kaleido PD15 (content300). Each bounded manifest records every loaded PT_LOAD page, including ELF-owned heap/BSS, and the 64KiB user stack. Up to512 distinct physical pages per app are supported. Boot creates a contiguous sext alias over the potentially noncontiguous backing; it does not zero the still-live app through that alias. This deliberately trusts sext with the recorded backing. Canonical capabilities authorize retirement; the alias is not an additional client mapping capability.

Kernel Task/PD objects, capability tables, kernel stacks, queue backing, registry pointers, and page-table infrastructure remain identity tombstones. Existing static references make freeing them unsafe. Queue contents and the reply slot are drained after quiescence, but their backing is retained. There is no respawn or identity recycling. DMA drivers, shared service caches, and unrelated surfaces are outside these app manifests.

The ELF final-permission update now preserves the owning pkey. Previously the underlying page-table update replaced the flags and erased those pkey bits. Manifest capture verifies the final own-pkey leaves.

## Authority and order

Slots38/39/40 hold separate immutable lifecycle capabilities for the three targets. The shell, display, files, net, and Bell receive QUERY+their own single ACK role; sext receives QUERY+RETIRE. Roles are1/2/4/8/16, required mask31. Full-width slot/task arguments and actual capability identity are checked. Syscall63 returns either immutable catalog metadata or authoritative dead status. Syscall64 acknowledges only the holder's assigned role. Syscall65 returns the retired sext pool base, byte count, and exact pinned task identity.

Dead status requires the exact boot main-task pointer/ID, exited state, Dead ownership phase with no CPU owner, and an inactive Constructed/Dead trampoline with no owner. Before first private leaf mutation, retirement requires all five ACKs and closure of any target demand-paging range, then preflights every recorded leaf (including PA, permissions and pkey; only hardware A/D bits may differ). Duplicate VA/leaf/PA ownership is rejected. Every leaf is cleared and locally invalidated before the pool is published. A cached repeated retirement returns the same pool without touching reused bytes. This remains the BSP/AP-parked execution scope; it is not an SMP shootdown proof.

The IPC route fences make a terminal boot app unreachable for new Domain/IPC calls, signals and replies. Per-service exit hints are advisory. The kernel retries each unacknowledged role and sext at a bounded16-tick cadence, including after an earlier hint was accepted: nested service waits may discard a notice. Service status attestation remains mandatory. Sext therefore receives retries until final retirement succeeds, even after the final role ACK and without ordinary traffic.

ServiceCustody sets its local terminal fence before executing cleanup, runs cleanup once, and retries the ACK. RAMFS revokes the dead subject's grants, closes sole-owned handles, preserves saved bytes/names, and retains a handle that another live subject can open. The backend's owner shortcut still relies on the service's dead-caller fence; the backend alone does not establish caller liveness.

## Actual private-byte reuse

Sext validates and zeros the complete retired pool, verifies zeroing, then uses it for a rotating pager journal. This is real reuse by the authorized pager, not a buddy free or quarantine. It does not expand the existing demand FramePool capability or redistribute these private frames to another client. Minimal immutable identity metadata remains allocated as described above.

## Kaleido transport correction

Warmup, DNS, text-length/chunks, and body-length/chunks now each complete an authenticated caller13 reply exchange before sending the next request. The Domain call's enqueue result is never treated as the service response. This avoids the single retained reply slot dropping concurrent replies or feeding a stale NET reply into later FILES proof traffic. Cooperative close is checked in the app loop and during these exchanges. Bell NOTIFY is one-way; the following SUBSCRIBE reply is the FIFO publication barrier, so no nonexistent NOTIFY reply is awaited. The resource proof retains its saved8-byte RAMFS content and visible-surface interval. After the first guest run exposed that a128-tick delay could expire before registration/rendering, all runs now await an authenticated shell registration reply300 and actual display presentation reply256 (sequence1) before entering the proof or input loop. The128-tick interval follows those acknowledgements and is not used as presentation evidence. Every display create/text/position enqueue and each reply-producing request retries only ERR_SERVICE_NOT_READY with the identical payload, polling an authenticated close and yielding between attempts. Other errors remain failures; a queue-full create cannot be silently skipped ahead of presentation.

## Verification and limits

- `scripts/app_resources_host_gate.sh`:9/9 actual state/runtime-module tests. Includes no-effect live/owned denial, role/task identity, hint retries, demand/ACK barriers, all-leaf preflight, pkey preservation, queue drain and cached retirement. Runtime tests mock registry/mapper/TLB; they do not prove hardware invalidation.
- `scripts/reply_sender_host_gate.sh`:6/6, including no enqueue/wake for terminal reply/signal targets.
- Existing task-exit watch tests:4/4 with the new independent lifecycle poll hook.
- `scripts/service_custody_host_gate.sh`:11/11, using the actual ServiceCustody and RAMFS implementations. Includes forged/live/mismatched attestation, QUERY-only observer denial, fence-before-effect, exactly-once cleanup across ACK retries, shared-live grants and saved-byte preservation. A host unwind test observes ordering only; no kernel unwind behavior is claimed.
- `scripts/kaleido_service_call_gate.sh`:8/8 actual transport-helper tests, including request serialization, sender/type authentication, send failure, close during an exchange, shell/display feedback sequencing with a stale shell reply, and full-queue exact-payload retries with no premature wait or duplicate acceptance.
- The same gate also runs6/6 bounded preview-layout tests added on29 October.
- `cargo check -p kaleidoscope --target x86_64-sex.json -Z build-std=core,alloc,compiler_builtins -Z build-std-features=compiler-builtins-mem`:PASS with existing warnings.

Sealed image builds, guest service/UI cleanup evidence, private-pool guest zero/reuse markers, and the combined source-bound report are recorded in the [29 October integration report](APP_RESOURCES_AND_DESKTOP_2026_10_29.md). Host results alone do not establish GUI presentation, device behavior, or hostile-code containment under optional PKU.
