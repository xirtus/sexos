# Incarnation-owned Spindle history — 29 October 2026

This slice replaces the legacy handle 0/fire-and-forget history path with bounded, receipt-backed snapshots in SexFiles RamFS. It builds on the [BOOT-reserved Spindle relaunch](BOOT_RESERVED_SPINDLE_RELAUNCH_2026_10_29.md). The existing uncommitted relaunch changes are preserved; no commit/push was requested in this slice. No agents were used.

## Authority and identity

The only subjects are the already BOOT-enrolled Spindle domain/task 12 and domain/task 17. They invoke their existing storage edges. SexFiles checks its exact lifecycle catalog grant (slot 39 or 50, task, content surface, coordinator 3) and LIVE status before each history request. The authenticated caller selects the private object name, `spindle-history-12` or `spindle-history-17`; the caller cannot select another owner. Ordinary RamFS owner checks also apply. The successor starts without an existing history object, without inheriting the predecessor's history bytes or handle.

Each transaction obtains the actual nonzero handle returned by OPEN and supplies it to every subsequent operation. Closing and reopening yields a fresh handle. These are service-owned file leases under existing sex-pdx authority, not new kernel capabilities. BOOT remains the only capability genesis, GLOBAL_VAS remains the single address space, and hardware key12 is never used to select history authority. No new syscall, per-process mapping, runtime capability mint, pointer IPC or Unix process model was added.

## Exchange, snapshot and recovery

The canonical register-only wire module is `crates/sex-pdx/src/spindle_history.rs`, opcode `0x4B`. It is included in the sealed ABI input list. A nonwrapping 24-bit sequence and explicit success/error tag separate receipts from payload, including data with high bits set. Spindle accepts these receipts only from actual sender SexFiles 11 and the one admitted sequence. Queue backpressure retains the exact request. SexFiles retains an undelivered receipt per subject, replays exact duplicate requests without repeating effects, and rejects changed/reversed sequence requests. The other subject and ordinary service clients can still progress.

A saved snapshot contains at most 4096 bytes: a 24-byte version/owner/count/length/checksum header and complete length-prefixed commands. It keeps the newest complete entries that fit, up to 128 entries of 256 bytes each; 256-byte commands fit 15 entries. The live history ring remains 128 entries. The checksum detects malformed/corrupt snapshots; it is not a cryptographic authentication claim.

Writes first fill an unpublished service buffer. COMMIT validates the complete snapshot, then `RamFs::replace_owned` prepares a replacement allocation and swaps the entire file. Invalid input or allocation failure preserves the previous snapshot. Shorter and empty snapshots remove trailing old content. The UI reports that `save` is queued; a Saved receipt is emitted only after matching COMMIT and CLOSE acknowledgements. Normal commands request autosave, coalescing later edits into a subsequent complete snapshot. Save/load control commands do not overwrite the saved history merely by being invoked.

Load opens a fresh owned handle, obtains the exact length, and reads into private staging. It validates version, owner, bounds, count, complete entries, reserved fields and checksum before publishing anything. Only a successful CLOSE and an unchanged local history revision permit the live ring replacement. Edits during a pending load preserve the newer local history instead. History I/O advances in the ordinary receive loop; it does not consume or drop HID inside a synchronous reply wait. Legacy synchronous disk commands refuse to drain storage replies while history I/O is active.

On attested app exit, SexFiles scrubs history staging and discards pending receipts before retiring RamFS leases and issuing its existing files-role ACK. The last committed bytes remain owner-private. An interrupted save cannot publish partial history or confer authority on the other incarnation.

## Verification

The normal sealed image passed the 90-second master gate (`GREEN_MASTER`) and normal two-client paging/reclamation regression. All **38 host gates**, **463 Rust test executions** and **242 Python tests** passed. The 17 new Rust tests exercise production codec, ring, client, service and RamFS code with mocked transport/lifecycle queries and host locks. They cover private ownership, fresh handles, stale/corrupt replies, exact replay, full reply custody, interrupted save/retirement, whole-file replacement, load conflicts, coalescing, bounds and permanent transport failure. Six new Python fixtures reject missing commit/close, wrong owner/fingerprint, reused handles, missing private startup/cleanup and reversed/terminal service records.

`/tmp/sexos-history-desktop-trial/` passed the complete native desktop gate through real QMP keyboard events:

- Spindle 12 saved `echo predecessor12` as a 44-byte private snapshot through actual handle 4. The save completed before its five-role retirement. Spindle 17 then reported its own history missing, rather than inheriting that object or handle.
- Three native commands in Spindle 17 produced a 70-byte snapshot with fingerprint `0xe25f8b5c7174f926`. Explicit `save` completed through handle 8; `load` opened fresh handle 9 and recovered exactly the same bytes, count and fingerprint.
- Real Up recalled `echo spindle17`. The frozen guest framebuffer matched the exact `> echo spindle17` prompt and all four complete native-grid glyph payloads using the production font. The report screenshot shows the saved/loaded status and recalled prompt.
- Every observed Saved receipt joined to the exact service-owned OPEN, matching atomic COMMIT and subsequent CLOSE; Loaded joined to its own OPEN/CLOSE and a prior exact saved fingerprint. History staging and pending replies were discarded before each files-role ACK and private retirement.
- Close 17 completed all five service ACKs and Sext scrubbing. Both Spindle identities each retired 84 pages; targets 9,12,15,16,17 together retired **285 pages / 1167360 bytes**. Both exhausted launcher rows rejected another relaunch.
- Quil's exact 242-byte document recovery and 243-byte dirty input/pixel regression also passed. Surviving Linen accepted selection during a real refresh on the first attempt (about 108 ms host-observed action latency), retained selection, displayed complete filename glyphs and passed visible F8 hide/restore. This is observed functional overlap, not a hard timing guarantee.

[History loaded and recalled](assets/sexos-spindle-history-successor-2026-10-29.png) · [Exhausted launcher](assets/sexos-spindle-history-exhausted-2026-10-29.png) · [Surviving Linen](assets/sexos-spindle-history-linen-restored-2026-10-29.png) · [Source/artifact evidence](assets/sexos-spindle-history-relaunch-2026-10-29-evidence.json) · [Selected native trace](assets/sexos-spindle-history-relaunch-2026-10-29-trace.json).

Normal ISO SHA-256: `83888f29907ad6846394c3c7be303b97713b0134b9a8f098feb6aba96826540d`.
Kernel SHA-256: `a71234f796db8662c39748ef00dd8ede1bf11a6dfd52423c020c6c388954aa49`.

The root ISO and kernel match the saved tested artifacts. Compiled Rust and all bound test/checker sources remain unchanged from the accepted build/host run. Raw build, host, Python and guest evidence hashes are saved in the JSON. No emulator remains running. The changes remain uncommitted.

Host fixtures alone do not establish guest persistence or pixels. The architecture inventory remains 30 obligations: 7 open and 23 partial; no complete-system or epoch-replay proof is claimed.

## Limits and next gain

This is **same-boot RamFS persistence**, not cross-reboot durability or implicit history transfer to another incarnation. Saved bytes belonging to a terminal identity do not become another identity's authority. An admitted request whose service stops replying has no operation deadline; the app still receives input and Close, and exit cleanup fences its pending work. Sequence exhaustion fails closed rather than wrapping.

The next persistence gain is a durable history object with an explicit BOOT-sanctioned recovery relationship and crash-tested publication. Pending registration/presentation progress and cancellation remain a separate desktop improvement. Both require concrete receipts and authority bindings; terminal identities, file handles and Sext frame journals are not substitutes for those grants.

## Subsequent work

This report preserves the same-boot checkpoint and its original evidence. That checkpoint was committed and pushed as `25dadc1766fbdb530e4b7182a32798a71e068b34`. The [durable recovery follow-up](SPINDLE_DURABLE_RECOVERY_2026_10_29.md) records the later BOOT-granted journal and separate reboot evidence.
