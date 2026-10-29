# Incarnation authority and launcher — 29 October 2026

The previous verified checkpoint is committed and pushed as `593c6632` on `origin/master`. This follow-up uses one short Astra authority audit; implementation and verification stay in the parent to conserve the remaining allowance.

## Delivered

The launcher resolves an **existing BOOT incarnation** through the shell's lifecycle capability. It compares the exact domain, original task, primary surface and coordinator with its pinned catalog. Live status alone cannot override mismatched catalog data or a cached terminal identity. Retry, closing, malformed and unavailable evidence all disable execution. A dead instance is visibly closed even before its service cleanup finishes.

Quil and Spindle availability comes from those queries, replacing unconditional ready labels. Enter on a closed instance returns `no_boot_successor`, performs no focus/launch mutation, and keeps the launcher open. Both normal and input-drain keyboard paths enforce that result. Restore availability now reflects whether a minimized target exists. Statuses are snapshots refreshed on rendering/navigation and rechecked at execution.

The launcher now draws real app/action names, human-readable status lines, keyboard help and selection. Five visible rows scroll with selection across all ten commands. A fixed 512-byte active/latest text outbox retries exact chunks under backpressure without blocking HID. Showing a fresh palette cancels stale text and waits for geometry/chrome admission before repaint. This adds no capability or kernel ABI.

## Relaunch remains unprovisioned

**This is not fresh app relaunch support.** The current BOOT_DAG has one instance each of Quil9, Spindle12 and Kaleido15, and no successor task, private image, stack, activation grant or route. Retired private backing already belongs to Sext's workspace. Original PD/task/capability/control objects remain tombstones. Clearing their death flags, reusing their domain numbers, or loading code through their stale routes would break the retirement guarantees.

The UI therefore says “Closed / reboot to reopen.” A successor descriptor or a larger generation number cannot grant launch authority. Kernel and service lifecycle fences remain unchanged.

## Next executable relaunch slice

Start with one BOOT-reserved successor for one app, then generalize after a real guest proof:

1. Reserve a distinct subject/task identity, fresh surface, image and user/kernel-stack backing, dormant task state, leaf infrastructure and canonical VA ownership transition during BOOT. Plan the hardware pkey separately from domain identity; keys cannot grow with domain IDs.
2. Bind a stable logical app and increasing incarnation to the exact predecessor/successor identities and immutable activation, service, lifecycle and paging grants. Capacity for successor service records must also be reserved at BOOT.
3. Activate only through a separately authorized one-shot mechanism after predecessor execution custody, all five service ACKs and private-leaf retirement complete. Preflight every new mapping and task transition before publication. Preserve all predecessor tombstones; use fresh backing, not Sext's already-reused journal bytes.
4. Publish the new current-instance route only after successful activation. Saved documents remain service-owned; stale replies, callbacks, input and surfaces must never become new-incarnation requests. Duplicate activation replays its receipt without requeueing the task.
5. Guest acceptance: close old with live resources, activate the reserved successor, verify actual new text/input and document access, reject stale predecessor traffic and duplicate activation, close the successor, then report reserve exhaustion honestly. Continue with one GLOBAL_VAS and the BSP scope; no per-process page tables, runtime capability mint, Unix process model or invented SMP guarantee.

## Verification

The focused host gate runs ten policy/text tests and five tests using the extracted production capability-query adapter. The complete host inventory passes 37 gates / 423 Rust test executions; the Python suite passes 224 tests. Host substitutes do not prove kernel activation or framebuffer visibility.

The final sealed normal build passed its complete 50-second master window and normal two-client paging check in `/tmp/sexos-launcher-overlay-final-20261029/`. The native run in `/tmp/sexos-launcher-overlay-close-20261029/` passed startup layout, all three native closes and 159 private pages zeroed/reused, rejected Enter on closed Quil, every visible launcher name/status glyph, input during Linen refresh, and Linen minimize/restore pixels. The normal ISO remains the tested artifact.

The first guest run in `/tmp/sexos-launcher-close-20261029/` correctly rejected Enter but failed the pixel check because the focused app occluded the launcher. The fix gives the shell-owned surface152 modal priority without changing the action target focus; foreign callers cannot create that reserved ID. Actual compositor tests cover shell ownership, hide behavior, glyphs above app focus and absence of false app-presentation bits.

[Native closed-app launcher capture](assets/sexos-launcher-closed-2026-10-29.png). The prior checkpoint's [resource/desktop report](APP_RESOURCES_AND_DESKTOP_2026_10_29.md) remains historical evidence for the earlier image.

The fresh two-boot persistence regression also passed in `/tmp/sxlaunch-disk29/`: live Quil opened through the launcher, and the same 242-byte document loaded twice with fingerprint `0xdbc6d3de7f43121f`. Text-color occupancy in its native viewport was 9,384 pixels; document identity is established by transport fingerprints. The launcher verification covers keyboard operation.

Current normal ISO SHA-256: `142e39a61e254fb8cd1dd906ce2e861972e36288818f43127144e44e5f452bff`. [Source and artifact evidence](assets/sexos-launcher-2026-10-29-evidence.json). Normal kernel/input/app flags remain unchanged; no emulator remains running.
