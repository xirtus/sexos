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
The first matching entry is shown; Next matching entry cycles in the provider
window and resets focus to Deny. No match is refused. The consumer cannot choose another entry by guessing an index.
Origin scopes are nonempty printable ASCII, up to 512 bytes; use punycode for
international hostnames. Titles and labels are escaped to ASCII for presentation,
so newline/bidi/control characters cannot impersonate dialog labels. Oversized
consent context is refused, never silently truncated. Approval requires a client
area of at least800x600, with the full escaped scope in bounded64-column lines.
Small tiles disable Allow, and token/origin/geometry are rechecked on action and
resize. A550x144 actual Weft Tab/Enter regression prevents hidden approval. There is no credential copy
to the system clipboard, notification history or file-open side channel.

## Protocol and secret custody

`sex_pdx::autofill` defines operations 0x3E0..0x3EF; 0x3F0 is Files' descriptive
binding query, and0x3F1/0x3F2 acknowledge provider revocation/client cancellation.
It is register-only: no pointer or shared-memory exchange. Shell is
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

Chastity revalidates the chosen entry against the canonical exact-origin
candidates only on approval, then passes borrowed
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
drops the canonical vault. Local buffers wipe immediately; visible Lock completion
and unlock are fenced until the broker acknowledges the exact provider epoch/run.
UNREGISTER is retried on kernel queue Full; authenticated right-narrowing revocations
bypass native intake saturation and custody contention. Revocation takes effect
when the broker observes it or detects
lifecycle loss; it cannot retract credentials already delivered by an approved,
completed request. A restarted Shell fails closed until the provider is relaunched
and unlocked. No automatic provider relaunch or ambient credential discovery.

## Consumer integration

The SDK works for ordinary program windows using their own existing BOOT Shell
route. Poll it alongside the window, with one-tick waits while a transfer is active;
`Window::next()` waits only for UI events and must not be used as the sole pump.
Window preserves protocol messages for the explicit SDK consumer without adding a
new Event variant or changing other applications' widget input handling.

Multipass/Valentine do not yet call this transport. The optional restore adapter
was withdrawn because the legacy HTTP authentication path makes ordinary BasicAuth
copies. The reusable SDK and actual separate fixture app demonstrate the transport;
callers own all credential handling after approved handoff. No live browser form
injection, account-service restore or clipboard integration is claimed.

## Verification

`scripts/autofill_host_gate.sh` compiles the **unchanged production protocol, broker
and SDK** with only native catalog/route/inbox transport replaced by a host model.
Seventeen tests cover no data before approval, deny, lock, both focus checks,
provider/approval impersonation, stale epoch/replay, exact run reuse, cancellation
before READY, malformed padding/bounds, expiry/clock rollback, multiline/bidi
origin refusal, full task/part identity words, and an8192-byte transfer under
foreign status flooding within the transfer deadline. Production native intake
is also compiled unchanged: full intake plus unavailable custody cannot drop Lock;
SDK transport-admission Full retries until the exact epoch/run ACK. A tombstoned
Close intentionally cannot receive that ACK: explicit actual run retirement
revokes it and the native full-intake regression verifies no queued DATA survives. Canonical
vault/UI tests preserve the user's original19 plus preflight and consent tests.

The canonical std guest and separate probe build and pass the forbidden-instruction
scan. The canonical full two-boot QEMU vault gate passes using the updated real
Shell/Files image. The final live two-app gate passes on the same updated real Shell/Files v2 image.
A fresh Display focus receipt is required before capturing visible consent, and
Allow is clicked using the actual post-layout Weft control coordinates. No kernel edits. ABI inputs include
the new protocol file, guarded by the existing pin mechanism.

`scripts/chastity_autofill_gate.py` drives real separate Weft applications: creates
and saves fictional credentials, reboots, unlocks the persisted snapshot, refuses
provider impersonation, verifies no result before visible consent, exercises
Deny then Allow once, restarts the consumer into a new task requiring fresh
consent, and verifies locking makes the provider unavailable. Screenshots and
serial are retained by Programs CI. **Both final QEMU gates pass** at guest source b9241079; exact hashes and local
artifact paths are recorded in CHASTITY_RUNTIME_2026_11_14.md. Remote CI remains
required before merge.
