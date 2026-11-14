# Chastity live autofill — round 179

The earlier library-only boundary is now a live, capability-authenticated route.
A separate program uses `sex_app::autofill::Client` to request an exact account
scope; Chastity shows a default-Deny per-request dialog and releases a credential
only after explicit approval. No kernel code or capability layout changes.

## Authority and identity

All traffic uses existing BOOT Domain routes: programs → Shell, Shell → each
program's held close route, and Shell → Files. Native ingress trusts only actual
kernel-stamped caller PD/task and Shell's held lifecycle catalog/LIVE result.
Shell refreshes **both** consumer and provider before every admission, including
queued credential chunks; close, task replacement and retirement revoke the flow.
Scalar PD/task, request tokens, filenames and epochs are correlation data, never
capability handles or authority to read a different vault.

Files answers a narrow identity query from authenticated Shell using its exact
current `Binding(task, root)`. Six three-byte name fragments carry the full run task
ID and fragment index. Shell matches the outstanding exact task/part and rechecks
its own catalog before accepting the descriptive label. This works for prepared
file tickets as well as bare program-store launches and avoids a global transient
"last requested name" as an identity source. Only the live loaded `chastity` run
can register as provider, and only one provider is active. That name check grants
no file or secret access; Chastity can still open only its existing Files folder,
and the consumer still needs visible consent for a particular entry and scope.
Installed executable integrity remains the existing loader/package trust model.

The request and approval messages must come from the currently focused consumer
and provider respectively. The dialog identifies the canonical consumer label and
exact domain/run, the exact requested origin/account scope, and the matching entry.
The currently selected matching entry is used, otherwise the first exact match;
no match is refused. The consumer cannot choose another entry by guessing an index.
Origin scopes are nonempty printable ASCII, up to 512 bytes; use punycode for
international hostnames. Titles and labels are escaped to ASCII for presentation,
so newline/bidi/control characters cannot impersonate dialog labels. Oversized
consent context is refused, never silently truncated. There is no credential copy
to the system clipboard, notification history or file-open side channel.

## Protocol and secret custody

`sex_pdx::autofill` defines operations 0x3E0..0x3EF; 0x3F0 is Files' descriptive
binding query. It is register-only: no pointer or shared-memory exchange. Shell is
a **trusted broker and sees credential registers**; this is not an end-to-end
confidential channel hiding passwords from the desktop's trusted Shell.

A random nonzero provider epoch changes on every unlock. Every origin/credential
chunk carries the epoch, broker token, exact offset, count and canonical padding.
READY also echoes the exact recipient run; stale replies cannot bootstrap a client
in a reused row. Each request has one consent decision and sequential stop-and-wait
transfer. The broker allows one outstanding request, at most 512 origin bytes,
4096 username bytes and 4096 password bytes. Consent expires after two minutes;
transfer after thirty seconds. Clock regression is a refusal. A cancellation before
READY uses the exact authenticated client and its monotonically allocated request
sequence; it cannot leave an orphan prompt or cancel another run's request.

Chastity consumes its core one-use grant only on approval, then passes borrowed
credentials into the provider SDK's preallocated zeroizing buffer. The client
returns a non-Debug/non-Clone `Credentials` only after complete, valid UTF-8 data
and FINISH. `Credentials::with` borrows the strings; dropping it wipes the buffer.
Callers must handle credentials responsibly inside their approved domain. Cancel
on user cancellation/Close; dropping a run is also fenced by native lifecycle.
The SDK supports one main-task autofill role/session per program. It is not a
concurrent broker for multiple SDK clients in one inbox.

The outbox is a bounded ring with reserved active-flow capacity; foreign status
traffic cannot consume the valid flow's forward/ACK slots. Actual static packet
storage is volatile-erased before setting Option slots empty, including intake and
SDK mailbox storage. There are no packet Debug/Clone implementations, and the
legacy first-eight-message raw PDX diagnostic explicitly excludes autofill traffic.
This is best-effort owned-buffer hygiene, not a claim about every CPU register or
compiler-generated temporary. Kernel IPC queues already erase dequeued backing and
are drained before a row is reused; those mechanisms are unchanged.

Lock closes the provider, wipes its outgoing state, clears the visible prompt and
revokes core grants. Revocation takes effect when the broker observes it or detects
lifecycle loss; it cannot retract credentials already delivered by an approved,
completed request. A restarted Shell fails closed until the provider is relaunched
and unlocked. No automatic provider relaunch or ambient credential discovery.

## Consumer integration

The SDK works for ordinary program windows using their own existing BOOT Shell
route. Poll it alongside the window, with one-tick waits while a transfer is active;
`Window::next()` waits only for UI events and must not be used as the sole pump.
Window preserves protocol messages for the explicit SDK consumer without adding a
new Event variant or changing other applications' widget input handling.

`sex-multipass` exposes optional feature `sexos` and `vault::Restore`: from a focused
sign-in action, request an exact validated DAV base URL, wait for Chastity consent,
then feed borrowed credentials to the existing `add_server_account` authentication
path. Multipass retains its session in Shibari; credentials are never added to the
stamp file. This adapter does not invent a missing Multipass server/window or alter
Valentine's policy of receiving an already-authenticated mail flow. Their owners
can call the adapter or the reusable SDK when their runtime UI is present.

## Verification

`scripts/autofill_host_gate.sh` compiles the **unchanged production protocol, broker
and SDK** with only native catalog/route/inbox transport replaced by a host model.
Fourteen tests cover no data before approval, deny, lock, both focus checks,
provider/approval impersonation, stale epoch/replay, exact run reuse, cancellation
before READY, malformed padding/bounds, expiry/clock rollback, multiline/bidi
origin refusal, full task/part identity words, and an 8192-byte transfer under
foreign status flooding within the transfer deadline. Chastity UI adds a separate
consent-label spoofing/oversize regression; the prior 13 vault-core tests remain.

The Chastity and separate `chastity-probe` guest binaries build and pass the
forbidden-instruction scan. Shell and Files build for the actual no-SSE server
target (Files uses the existing `aes_force_soft` setting); the optional Multipass
adapter also builds for the guest. ABI inputs include the new protocol file and
pins are regenerated with the existing guard tool, with no kernel hash changes.

`scripts/chastity_autofill_gate.py` drives real separate Weft applications: creates
and saves fictional credentials, reboots, unlocks the persisted snapshot, refuses
provider impersonation, verifies no result before visible consent, exercises
Deny then Allow once, restarts the consumer into a new task requiring fresh
consent, and verifies locking makes the provider unavailable. Screenshots and
serial are retained by Programs CI. **QEMU runtime proof remains pending until
this gate passes**; host models and guest compilation are not that proof.
