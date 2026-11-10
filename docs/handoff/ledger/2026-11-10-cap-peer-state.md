# Cap-scoped service epoch and observed liveness

The existing crash-log OP_RESTARTS keeps its legacy arg0=0 self count. A nonzero
canonical held Domain capability slot, with reserved buf=0, asks for that peer's
known epoch (restart count + 1, first boot 1) and typed lifecycle state. It accepts
no raw PD identifier, unheld/wrong-kind capability or truncated selector. Missing
or contended restart metadata fails closed. Unknown/epoch 0 results are never
converted into a successful live observation by the SDK.

The peer snapshot holds the same service mutex as rearm. It checks the registered
main task's identity/context and atomic ownership state, a non-consuming end
request bit and gave-up state. Starting, ending, exited, dead, gave-up and
fault-blocked are distinct nonlive states. Blocked ordinary IPC wait can still be
live. This is an observation, not a promise of progress or an unchanging lease.

A related legacy race is fixed: poll_one admits a rearmed service before dropping
its restart mutex. The old self-count lookup tried that mutex and could return 0,
misidentifying a restarted service's startup as first boot under SMP. BOOT now
publishes immutable PD/index association in a 24-byte atomic array; self lookup
reads the existing atomic count without taking that mutex. The count increment
already precedes activation. Unknown caller 0/unregistered counts keep the old 0
behavior; the typed peer operation instead returns an error for an unknown peer.

Exact kernel growth: 32 nonblank lines, no new syscall, recorder policy or runtime
capability minting. Current default BOOT edges are unchanged. Display already
holds Shell slot 6; Program rows hold Display 5/Files 1/Shell 6; Shell's explicit Files
route is program_store::SHELL_ROUTE_SLOT 256. Files lacks Shell slot 6: a later
recorder-role integration needs a separately disclosed BOOT edge there, not an
ambient peer lookup. Two newly relevant kernel files join the existing ABI pins,
so this snapshot pins 82 inputs.

Spindle's `service-state shell|display|files|network` provides a real diagnostic
through its held routes. The existing BOOT restart receipt now includes the actual
initial task id so the native probe can compare observed task ids before/after,
rather than infer task identity from a PD number. No UI geometry changes.

Host qualification compiles the full production restart module and the actual
syscall prefix/OP_RESTARTS arm, with explicit host registry/mapper/scheduler
boundaries. Cases cover canonical/held/wrong-kind/high-bit/raw-PD/reserved-buffer
selectors, unknown callers, pointer/context mismatches, end-bit preservation,
all typed states, contention, BOOT identity/count association and actual poll_one
rearm with unchanged task 3003 and epoch 1->2. A deterministic test holds the restart
mutex while the legacy self count still returns its actual nonzero value.
Seven compiled mutants are rejected, including the original false-zero lookup.
Guardrails and optimized kernel/Spindle target builds pass.

Native four-core `service_peer_gate.py` is wired after the crash-probe build. It
uses actual QMP keyboard commands, first observes live epoch 1, triggers the normal
probe-only Shell fault, checks actual same-task restart and Shell's own count 1,
then observes live epoch 2. Only documented RETRY contention is retried; successful
wrong epochs/states and malformed complete records fail. Native execution and
candidate hashes will be recorded in the PR body/evidence once available. This
ledger does not claim recorder authority from model or diagnostic results.

Recorder admission policy remains separate: bracket frame copy/Ready and Files
ACK acceptance with matching live epochs. A snapshot cannot prevent an SMP change
immediately after it; define admission at the successful observation. An already
admitted private frame may finish encoding/draining after Stop, but no new frame
may be admitted after Display processes revocation. Providers must revalidate
origin Shell epoch/live and actual recorder task before every retained positive
ACK retry. Request carries full origin epoch; epoch-prefixed nonce ranges prevent
fresh same-task Shell requests colliding with old pending positives. Prepared old
indicator frames cannot rearm a revoked session. These controls must accompany
actual recorder integration before any capture authority uses this observation.
