# App resources and desktop responsiveness — 29 October 2026

Integration record for the work following task-exit demand-memory cleanup. This is a single-address-space operating system: one GLOBAL_VAS, immutable BOOT_DAG capabilities, and sex-pdx as runtime authority. No Unix process model, per-process page tables, runtime capability mint, or kernel frame-allocation policy was added. `ARCHITECTURE.md` remains unchanged.

Astra agents handled lifecycle/authority, shell interaction and compositor work, then independently reviewed the integration. Final builds, native input/pixel checks and artifact restoration were coordinated in the parent workspace.

This report records checkpoint `593c6632`. Later launcher work and remaining relaunch requirements are tracked in the [incarnation/launcher follow-up](INCARNATION_AUTHORITY_AND_LAUNCHER_2026_10_29.md).

## Delivered behavior

Quil, Spindle, and Kaleidoscope have boot-recorded app identities and private ELF/user-stack manifests. Native Close requests cooperative task finish through fixed shell capabilities. Enqueueing a request never establishes death: the shell retries until the kernel attests the original task is dead and no CPU owns its execution or stack.

The shell, display, files, network, and Bell each attest the target through their own immutable lifecycle grant. They retire their own references and acknowledge their own role. The kernel requires all five roles, demand-range cleanup where applicable, and an exact preflight of every private leaf before clearing and invalidating mappings. Sext then zeroes every returned byte before using the backing for its bounded journal. Exact replay cannot clear or zero a replacement owner.

This includes shell frame/tab/focus/drag/hover/collar state; display surfaces, text, fills, appearance latches and cursor save-under state; RamFS subject leases and disk selection; Bell events, callbacks, mute/policy/spam state; and stale request admission. Saved file contents, valid leases held by other subjects, and shared network caches remain intact. Linen's existing Spindle list/open/search route is preserved, with a QUERY-only lifecycle observer fencing terminal callers.

The actual native associations are Quil PD9/content156/frame201, Spindle PD12/content154 plus160/161/162/frame153, and Kaleidoscope PD15/content300. Unrelated prototypes202/205 are not silently assigned to those apps. Catalog IDs remain reserved after death. A foreign create can no longer allocate a duplicate active surface ID.

## Desktop changes

- Linen refresh no longer runs nested receive loops. One outstanding request serializes rescan, metadata, name and open replies; complete staged snapshots publish once while retaining selection changes made during the fetch. The last complete view stays visible while storage works.
- Linen retains shell replies across mailbox backpressure. Native registration and display presentation also retain their replies. Kaleidoscope serializes NET requests and retries exact display enqueues rather than consuming the enqueue result as a response.
- Linen text uses one immutable active batch and one replaceable latest repaint, bounded to 512 bytes each. Four checked send attempts per shell pass retain exact clear/chunk operations on a full display mailbox. Receipts mean complete admission; only the framebuffer checker establishes visibility.
- Kaleidoscope now has its missing BOOT display grant and registers its real surface with the shell. It waits for actual shell registration and an acknowledgement tied to a completed compositor pass before startup completes.
- Wider app surfaces use readable 2× glyphs, with width-aware reflow. Narrow overlays retain their existing density. Window titles render all letters, and text changes mark the scene dirty.
- Linen submits actual committed filenames, keyboard help and a selected-row marker. A bounded five-row window follows selection, sanitizes names and renders filename punctuation. The redundant startup sample rectangle and spin delay are removed.
- Quil content and Spindle's four panes fit their shell frame during tiling, movement and resize. The compositor preserves parent chrome and app content colors; minimizing a frame hides its bound child surfaces while retaining their text. Exact BOOT catalog associations determine membership.
- Kaleidoscope lays out a bounded title, honest cached/sample status and body preview using explicit lines. Network text is sanitized; the current service does not attest a live HTTP response.
- Main desktop geometry/hide operations retain the latest state on backpressure, revalidate current bounds and app custody before retrying, and restore chrome before Linen text. The palette closes independently of Bell. Linen suspends text submission while hidden and repaints the committed view on restore.
- Normal images no longer inject synthetic Enter, drag or pointer actions. Synthetic input requires an explicit sealed-stage test declaration. This prevents startup test input from minimizing a newly visible app.

## Verification

The final visible-app and normal profiles each passed a sealed build and full 50-second master runtime window in `/tmp/sexos-app-ui-final-20261029/`. The strict resource checker passed: actual shell registration and correlated display presentation preceded exit, all five service roles acknowledged, four demand mappings closed, and 43 private pages (176,128 bytes) detached, fully zeroed and reused by Sext. A desktop clock event followed reuse. The test held an actual RamFS handle with eight saved bytes and a Bell event until exit; files and Bell each retired one resource.

Normal paging observed nine recoveries, four releases, three cross-client reuses and one self-reuse. The older self-finish and quota-exit profiles plus restored normal each passed a further sealed 50-second runtime window in `/tmp/sexos-app-ui-paging-regression-20261029/`. Compiled scheduler stack-release, timer PKRU and optimized client memory-helper checks passed, along with observed memory authority and scheduler ownership checks.

The strengthened desktop run in `/tmp/sexos-app-ui-presentation-final-20261029/` passed on that exact normal ISO. It waited for the app's correlated presentation acknowledgement, then matched Kaleido title/status glyphs at the shell's tiled bounds. Real QMP keyboard input closed Kaleido, Quil and Spindle; each completed all five service roles. Their 43 + 37 + 79 private pages total **159 pages / 651,264 bytes**, fully zeroed and reused. Linen still handled selection during a storage refresh and preserved that selection on publication. The observed host key-to-action interval was about 0.10 seconds with 100 ms polling; this establishes functional overlap, not a hard latency or visible-frame bound.

The same run matched every final visible Linen filename and selection glyph against the actual framebuffer, including blank trailing cells. F8 then removed the text from the framebuffer; F8 restore reproduced the complete names with selected ID3 unchanged. `startup-layout-pixels.json`, `filename-pixels.json`, `minimize-restore.json` and `result.json` all passed.

All four USB lanes passed in `/tmp/sxui-usb29/`: tablet/contact, absent-device discovery, keyboard and boot mouse. The two-boot NVMe gate passed in `/tmp/sxui-disk29/`: a 242-byte document reopened twice with fingerprint `0xdbc6d3de7f43121f`. Its final native Quil viewport contained 12,576 text-color pixels. That occupancy check does not identify document glyphs; matching saved/loaded transport fingerprints establish document identity.

The final source-bound host inventory passes 36 registered host/static gates and 407 Rust test executions, with 315 file hashes. The Python suite passes 219 tests. Evidence is retained in `/tmp/sexos-app-ui-complete-host-20261029/` and `/tmp/sexos-app-ui-verified-python-20261029.log`. Host authority/mapper/TLB/PDX substitutes are explicitly scoped; none is presented as hardware or whole-system proof. The inventory still records seven open and 22 partial architectural obligations.

At that checkpoint, the root normal ISO was restored and byte-identical to the artifact used by the native desktop, USB and persistence runs. Its SHA-256 is `861e657bcab6a7beb585cce37a3cb5337e0146dbf737d6bf77196a701b6bfb33`; kernel SHA-256 is `2dea8fbfacd7b02c7b90dd8733d30b5025a9c9d7f148314d02957fb26d39e53b`. Tracked changes and untracked production Rust sources match the sealed normal build. The [evidence manifest](assets/sexos-app-desktop-2026-10-29-evidence.json) retains artifact/checker/source hashes. No emulator remains running.

### Failures that changed the implementation

The first app profile passed the general boot gate but failed the strict checker: a fixed 128-tick delay expired before shell/display startup consumed the app requests. This prompted registration and real presentation barriers. Checked enqueues then exposed the missing display capability. A subsequent run correctly waited for presentation but normal startup's synthetic Enter minimized the app; moving synthetic input to an explicit profile resolved that behavior. Failed logs and images remain in `/tmp/sexos-app-exit-20261028/`, `-v2-20261028/` and `-v3-20261028/`. They are not counted as successful UI evidence.

The first new-layout integration in `/tmp/sexos-app-ui-profiles-20261029/` passed both sealed runtime windows, strict resource cleanup, normal paging and all three native Close operations. Its filename framebuffer check failed: larger best-effort text batches lost chunks under display mailbox pressure, leaving a previous selection and partial labels. A command-palette close was also incorrectly guarded by the existence of a Bell frame. Those screenshots are retained as failures. This led to bounded, nonblocking retained text admission and an independent palette-close correction. The glyph checker itself was strengthened against stale longer names that share the expected prefix.

Review also corrected the startup screenshot checkpoint. The display's ordinary first-winning-pixel marker can precede the app's correlated presentation request, so that early capture did not establish completed startup layout. The final gate waits for the app's actual acknowledgement and checks tiled Kaleido glyphs and complete Linen names in the same framebuffer. Display producers use one FIFO; these early images were not evidence of cross-sender message reordering.

## Native captures

These are guest framebuffer captures converted losslessly from PPM to PNG:
[startup apps](assets/sexos-app-desktop-2026-10-29.png),
[Linen restored after all three app closes](assets/sexos-linen-restored-2026-10-29.png),
and [the document reopened after reboot](assets/sexos-quil-reopened-2026-10-29.png).
They show a functioning prototype with more readable content, not a finished visual design. Decorative fill operations outside the retained text/state paths remain best effort.

## Limits and next gains

Kernel PD/task objects, capability tables, kernel stacks, page-table infrastructure and queue backing remain permanent terminal identities. Their contents are drained, but existing static references make freeing the control objects unsafe. Closed boot apps cannot restart in this implementation. Relaunch requires an explicit incarnation design and preprovisioned authority; replaying a tombstone or recycling a domain number is not safe.

Private bytes are reused by Sext's journal, not freed to a buddy allocator or added to the existing demand FramePool. The implementation enrolls three fixed apps, assumes the BSP with APs parked, and uses local TLB invalidation. It does not establish SMP shootdown, forced termination of an unresponsive app, hostile-code containment under optional PKU, or complete UiIR epoch transport.

Next desktop gains should be independently reviewable: explicit app incarnation/relaunch authority, an honest closed/loading/error state in the app launcher, shared palette/spacing and clearer window labels, useful document editing and reopening, and measured rendering/input budgets. Each must preserve the single-address-space authority model and retain a real guest acceptance test.

For detailed ownership and ABI ordering, see [the retirement implementation note](APP_RESOURCE_RETIREMENT_2026_10_28.md) and `sexos_contract.toml`. The reusable `scripts/app_resource_profiles_gate.sh` builds a declared visible-app profile, restores and verifies normal, then drives three native Close operations using real QMP keyboard input. Its orchestration has separate failure/interruption tests.
