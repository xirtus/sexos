# Timed-wait source review: raw merged 23090514

Reviewed actual merged master `23090514b18ed8813c30ca7b03a840f75d55edf8`
against reviewed `c20934ae11a5a8cbb609825acc52adc4baa4d9b8`.
[The receipt](SELF_WAIT_DENIM_REVIEW.json) records every before/after kernel Rust
source hash, every named causal source, and the auxiliary service/SDK hashes.
This is source qualification, not a runtime result or general kernel audit.

The scheduler, self-wait state machine, interrupt clock/expiry poller, program
record deadline writers, task ownership, SMP code and PD creation are unchanged.
The six named deadline-reference owner paths are unchanged. Two owners change:
`program_workers.rs` adds only the SPAWN RECALL notification/RETRY branch;
`syscalls/mod.rs` adds code RECALL and Denim-only syscall88. Neither new branch
arms or consumes a deadline, parks Net, changes its saved frame, or changes ticks.

The other changes are heap page/accounting and BOOT setup:

- `app_resource_state.rs` admits exact nonpresent compressed-page markers and
  supplies leaf take/restore helpers.
- `app_resources.rs` tracks compressed pages, prohibits worker/code admission
  until recall, and scans/moves/restores only a ready, live, grown application
  row under the existing application lock. Net13 has no eligible application row.
- New `denim.rs` copies through Denim's keyed window and authenticates syscall88
  to PD51. Its post path enqueues only to that domain's ring and unparks only its
  main task; ordinary unpark cancels that recipient's existing deadline.
- `ipc/pagefault.rs` routes exact compressed-page faults through the existing
  prepare/publish/claim/complete fault-token lifecycle. Completion uses existing
  fault ownership and task identity checks, not a new timed-wait wake path.
- `init.rs` creates optional BOOT51/key14 before the unchanged inventory seal,
  reserves its service memory and labels its ordinary post-seal handoff.
  `lib.rs` and low-level SDKlib only declare the new module.

The Net13 interrupt-disabled cancel/arm/park ordering, nonblocked cancellation,
two-tick Ring3 clock envelope, and unique UP expiry/return requirements remain
unchanged. The new admission pins the **exact raw** 140-file kernel tree and the
entire matching named causal source set together. Individually approved older
SDK/logging variants cannot be combined with it. Older source cohorts stay
available; unknown new modules, altered writers, SMP or missing causal evidence
remain rejected. No kernel source was edited for this qualification.

Denim's own main task sleeps64 ticks while idle and8 while retrying page-in.
Its optional auxiliary profile is separate from the Net inference. It requires
exact service, SDK, init and raw causal sources, observed UP topology, one exact
`kernel.spawn.denim` identity51/key14/memory before the seal, one exact BOOT
handoff51 after it, and sampled waits only after handoff. Only task51 intervals8
or64 and non-early task51 expiries are admitted. Every task51 record must be
complete through newline, use canonical bounded u64 values and a positive count;
park deadlines must be at least their interval and expiry deadlines positive. A task51 expiry never counts as
a Net timer join. Missing, duplicated, wrong-role, mutated or mixed source
receipts cannot enable this profile. Sampling does not require every expiry to
have a sampled park; the existing independent Net join remains mandatory.

No raw23090514 QEMU timer trace is claimed here. Historical ba8/other images and
immutable evidence keep their original provenance; they are not master proof.
