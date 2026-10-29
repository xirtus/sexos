# Durable Spindle history and BOOT recovery — 29 October 2026

The previous relaunch and incarnation-private history checkpoint was committed and pushed to `origin/master` as **`25dadc1766fbdb530e4b7182a32798a71e068b34`** (`feat: add Spindle relaunch and owned history`). This follow-up adds durable command history and explicit recovery authority. No agents were used. The new changes remain uncommitted.

## Authority stays with BOOT

SexOS retains one `GLOBAL_VAS`, BOOT-only capability genesis and `sex-pdx` authority. There are no per-process address spaces, inherited file descriptors, Unix process semantics or runtime capability minting. Hardware key12 is an enforcement detail, never recovery authority.

Before sealing capability tables, BOOT installs one typed `HistoryRecovery` grant in **SexFiles 11, slot 53**. It binds logical history object 12, original domain/task 12, reserved successor domain/task 17 and sectors **1808–1825**. Read-only syscall 67 returns that immutable metadata only to the exact capability holder; full-width slot validation prevents truncation aliases. The two query halves cannot mutate or mint authority.

This grant explicitly permits saved **data** to move 12→17 during the reserved relaunch and 17→12 after a new boot. It does not resurrect the terminal task, transfer a handle, inherit a capability or enlarge the finite activation reserve. Each current incarnation still needs its exact lifecycle catalog and LIVE status on the existing storage edge. Private names remain caller-derived; recovered bytes acquire a new current-owner snapshot header and a fresh owner-checked RamFS lease. The old owner and old handle remain unusable by the successor.

## Durable publication

`servers/sexfiles/src/history_journal.rs` implements two fixed banks, each one 512-byte commit header and eight payload sectors. A complete snapshot is at most 4096 bytes. Metadata binds version, logical object, source domain/task, nonwrapping generation, length and checksums. Recovery validates both metadata and complete command payload, including owner, reserved fields, entry boundaries and final-sector padding, then chooses the highest complete generation. A damaged newer bank can fall back to the previous complete bank. Unknown/corrupt storage without a valid bank and conflicting equal generations fail closed. There is no automatic format or repair that discards an unknown journal.

A save prepares the private RAM replacement before disk I/O, then performs:

1. Invalidate the inactive bank header and flush.
2. Write the complete new payload and flush.
3. Write its commit header and flush.
4. Read back and validate the committed bank.
5. Publish the prepared current-owner RAM snapshot; return matching COMMIT and CLOSE receipts before the app reports Saved.

The previous committed bank remains untouched. Errors preserve the previous RAM snapshot; exact request retries replay the original receipt without repeating disk effects. A crash after a durable write but before its receipt may recover that complete, unacknowledged generation. It cannot justify an earlier Saved message. Generation exhaustion fails closed.

The disk adapter uses the existing BOOT-backed block buffer and storage edge. SexFiles accepts block completion only from actual sender SexDrive 2. `BLOCK_SYNC` now issues the real NVMe FLUSH command and requires its matching successful completion; the old unconditional unsupported-device stub is removed. A flush timeout stops uncertain I/O. No flush failure is converted into success.

The reservation is outside the V4 pool and other named disk regions. Because it lies within the broad legacy v0 envelope, v0 allocation, specific-block allocation, extent validation and data writes explicitly fence it. Any overlapping external V4 descriptor is rejected both when read and when rebuilding allocation ownership. Whole legacy blocks touching either journal edge are unavailable to that allocator.

The real reboot check exposed two display delivery gaps. Both original 12 and successor 17 now retain exact surface/text calls under transient backpressure; a full queue cannot update the local rendered-text cache without admission. Shell focus has a bounded latest-desired outbox: retries wait for pending create/chrome admission, discard superseded or non-live targets, preserve a real clear-focus request and stop on permanent denial. Input remains under the shell's existing authority. Host fixtures cover both app identities and focus supersession, lifecycle fencing, geometry ordering, clear and denial.

Retirement still discards service staging and pending receipts before retiring file leases and acknowledging cleanup. Durable command data survives; task authority and resources do not.

## Host and normal image verification

All **38 host gates**, **488 Rust test executions** and **251 Python tests** pass. New production-function tests cover immutable recovery queries, exact holder/task binding, full-width slots, missing/misbound grants, every modeled save I/O interruption, torn-sector writes, corrupt-bank fallback, full 4096-byte snapshots, shorter replacements and zero tails, failed flushes, service publication failure, duplicate failure receipts, fresh private recovery leases and disk-region fences. Python fixtures reject fabricated barriers/readback, wrong recovery grants/source fingerprints, ambiguous disk generations and incorrect reboot surface/prompt pixels.

Crash tests use a sector/cache/barrier model with injected I/O failures and torn writes. They establish scoped publication behavior under that model. Checksums detect corruption; they are not cryptographic authentication against a malicious disk. Physical-controller power-loss behavior remains to be validated.

The normal image built only through `scripts/entrypoint_build.sh` and passed the 90-second master gate (**GREEN_MASTER**) plus real two-client Sext paging/reclamation. The architecture constitution is unchanged (SHA-256 `4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`). The inventory still has 30 obligations: 7 open and 23 partial. No complete-system, SMP or epoch-replay proof is claimed.

## Native evidence

`/tmp/sexos-durable-desktop-final-a/` passed the full native desktop gate. Task 12 saved `echo predecessor12` (44 bytes, fingerprint `0x2cd38b4d230600a5`) and completed all five cleanup roles. BOOT-reserved task 17 recovered those command bytes under grant 53, received a new current-owner header, and opened its own handle 5. Its three native commands extended history to four entries.

The explicit task 17 save completed at generation 5 through handle 9: **90 bytes / four entries**, fingerprint **`0xb4cb473a16fe0b1`**. All five observed journal publications completed three actual NVMe flushes and readback: **15 successful flush completions**, with no false Saved receipt. Load used fresh handle 10 and recovered that exact snapshot. Real Up recalled `echo spindle17`; all four complete surface payloads, including the full `> echo spindle17` prompt, matched the framebuffer using the production font.

`/tmp/sexos-durable-desktop-final-b/` then booted the **same ISO** against a byte-for-byte copy of A's final disk, without truncation or formatting. The new BOOT's task 12 recovered source 17/task 17, generation 5, the same 90-byte snapshot and entry-body fingerprint **`0xff4921860d51692f`**. Only its current-owner header changed (new whole-snapshot fingerprint `0x3c973eeffb6fe629`). Its fresh own handle 1 loaded all four entries. Real keyboard input opened Spindle, selected the launcher's Zoom action, and recalled the last command with Up. The strict framebuffer check passed with the complete prompt visibly present. Independent parsing of both disks confirmed that read-only recovery left the committed journal unchanged.

The full A run also passed Quil's 242-byte recovery/243-byte input-pixel regression, both exhausted launcher rows, all five service acknowledgements and Sext scrubbing for each terminal app. Targets 9,12,15,16,17 retired **285 pages / 1167360 bytes**. Surviving Linen accepted input during refresh on the first attempt (about 107 ms host-observed action latency), retained selection and passed complete filename pixels plus visible F8 hide/restore. This is functional overlap evidence, not a hard timing guarantee.

[Recovered successor](assets/sexos-spindle-durable-successor-2026-10-29.png) · [Fresh-boot recovery and visible recall](assets/sexos-spindle-durable-reboot-2026-10-29.png) · [Surviving Linen](assets/sexos-spindle-durable-linen-restored-2026-10-29.png) · [Source/artifact evidence](assets/sexos-spindle-durable-2026-10-29-evidence.json) · [Selected records from both boots](assets/sexos-spindle-durable-2026-10-29-trace.json).

Normal ISO SHA-256: `2a151f5a4727cf6386b029c5644285b8893a3275e08ed0b75127d8df7f9682e5`.
Kernel SHA-256: `f5e7c029ba8bc81593f1d3e1b60cbf5333e679823aae10135f62a06cda7399e8`.

The root artifacts match the saved tested image and kernel. Both accepted native manifests bind to that exact ISO. Compiled Rust was frozen before that sealed build, and all host-bound sources match the final accepted host report. Raw logs, disk images, checks and screenshots have SHA-256 bindings in the evidence JSON. No emulator remains running.

During verification, the initial reboot pixel checker incorrectly allowed text that lay entirely below a short tile's clip. Its first result is explicitly rejected in `/tmp/sexos-durable-desktop-b/pixel-audit-rejected.json`; it is not accepted reboot pixel evidence. The checker now rejects clipped nonblank cells and requires ink for every nonempty grid. A negative blank-frame fixture reproduces that false-positive case. Reboot testing uses the real launcher's Zoom action before requiring the complete recalled prompt. The original desktop run also passes this stricter checker. This follow-up does not claim that all startup tiles already accommodate the complete terminal grid.

## Reproduction and next gain

Build/probe with `GATE_DIR=/tmp/sexos-new-build scripts/master_runtime_gate.sh --probe 90 --keep-log`. Run `python3 scripts/check_architectural_truth.py --run-host --evidence-dir /tmp/sexos-new-host` and `python3 -m unittest discover -s scripts/tests`. The native pair is `python3 scripts/desktop_close_gate.py --iso sexos-v1.0.0.iso --out /tmp/sexos-new-a`, followed by `python3 scripts/spindle_durable_reboot_gate.py --iso sexos-v1.0.0.iso --boot-a /tmp/sexos-new-a --out /tmp/sexos-new-b`. Each output directory must be fresh. These native harnesses never build an image.

The next desktop gain is a viewport-aware terminal that keeps the current prompt readable in small tiles, followed by cancellation/deadlines for stalled service work. Durable recovery remains limited to this fixed BOOT relationship and journal; general storage recovery, reserve replenishment, hardware power-loss validation and complete epoch replay remain open.
