# Ribbon Seal launch-owner runtime qualification

The ordinary-parent launch regression passed in QEMU against exact ec2cc405e
Files and Shell binaries. `LAUNCH_TICKET_OWNER_RUNTIME_2026_11_15.json` records
all raw result, serial, disk, screenshot and runner digests. The source/ELF/image
receipt is under `target/launch-owner-runtime/provenance-v4.json`; the complete
passing run is `target/launch-owner-proof-v4` in the `sexos-launch-owner` worktree.

The developer-only Ribbon Seal fixture is not installed by the production build.
The gate installs it into a fresh copy of the unchanged blank seed. The image is
a separately named DEVELOPMENT graft of the preserved v9 normal ISO: only the
ec2 Files and Shell ELFs change. Eighty-two other production ELFs remain identical.
Boot metadata was replayed and independently extracted/compared. The canonical
helper's exact SHA/length compiled into Files matches its extracted artifact.
This is not sealed-cohort, installer or restricted-helper qualification.

Actual checks:

- A valid SDK main launch inherits the exact marker argument and private-file
  bytes. Its authenticated Started and successful Done identify a real child.
- A second live parent has only the shared rendezvous-file grant. Both parents
  are launched before preparing the victim ticket, because Files has one global
  prepared-ticket slot. That foreign parent receives Refused/NOT_FOUND and the
  exact Files denial record. Its argument bytes remain unchanged; private reads
  and delegation remain refused.
- The rightful owner subsequently consumes that SAME ticket successfully. Its
  child reads the private bytes. Replaying the consumed ticket is refused.
- After real retirement, the owner's actual row42 is reused with a different
  main task. Its stale token is refused, old private grant is absent, and a fresh
  authorized launch has exact new arguments without a private-file grant.
- Child and parent identities are joined to real private retirement. The final
  empty-session durable save completes before stopping QEMU.

The row-reuse result is deliberately composite: cleanup and Spindle's subsequent
PREPARE/global-slot replacement also invalidate the old token. It does not isolate
only the owner.task comparison. The existing compiled-production-method host
regression isolates that predicate separately.

## Worker provenance limitation and preserved failures

The kernel intentionally stamps a live same-domain worker as its run's main task
in `kernel/src/ipc/sender.rs` (the worker normalization branch). Therefore exact
main-task comparisons in userspace cannot independently distinguish that worker.
The diagnostic v3 boot records worker1073741825 under main1073741824: authorized
private read succeeds, and PREPARE returns Ok(6), contradicting the fixture's
initial Err(DENIED) assumption. That counterexample remains intact, with its own
source, ELF, disk, serial and diagnosis receipt. No kernel/source fix is applied.
This proof makes no worker launch exclusion, general worker-membership or
stale-worker-I/O isolation claim. “Main” here is the kernel's live-run identity.

The first fixture also expected DENIED for an ungranted read. Files' actual
confine path returns exact NAME before I/O, so that fixture contract was corrected
without accepting arbitrary errors. PREPARE refusal remains exact DENIED; launch
refusals remain exact NOT_FOUND, never resource-busy acceptance. All v1/v2/v3 raw
failures remain preserved. Fresh v4 validates the identified fixture corrections;
there were no retries of unchanged failed logic.

Host checks pass: the bounded record corruption/truncation test and sixteen
actual-source ticket-owner/shared-focus regressions. The final guest fixture,
Shell and Files PIC/PIE builds pass unchanged forbidden-byte scanning at seed0.
No loader/scanner exception, kernel edit, production stage, external artifact
mutation or new restricted-role protocol is involved.
