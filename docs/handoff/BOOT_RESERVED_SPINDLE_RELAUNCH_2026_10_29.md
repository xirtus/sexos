# BOOT-reserved Spindle relaunch — 29 October 2026

Historical checkpoint: the [incarnation-owned history follow-up](SPINDLE_HISTORY_AUTHORITY_2026_10_29.md) supersedes this report's history limitation.

The verified Quil slice was committed and pushed as `33817053` to `origin/master` before this work began. This slice adds a separate finite Spindle reserve. No Astra agent was needed; it reuses the reviewed activation mechanism.

## Authority and identity

BOOT reserves domain/task 17 at layout 17 (`0x50000000`), with distinct ELF, user/kernel stacks and permanent control objects. Its hardware key is 12, separate from layout/domain identity17. Successor leaves remain non-present and delivery remains denied until predecessor12 has released execution custody, all five services acknowledge, and its private mappings retire. The existing syscall66 performs the exact leaf preflight and one-shot BSP activation. Neither predecessor is resurrected; no runtime capability mint, ELF load, frame selection, per-process page table or Unix process model was introduced.

Spindle's independent activation grant is slot 49. Its lifecycle is50, close51, call52 and self-catalog QUERY48. All five lifecycle subjects are provisioned before sealing. The second grant does not consume or alter Quil's slot 44 receipt. Host tests exercise independent readiness, activation, replay and exhaustion for both pairs, including attempts to resolve Quil's tuple as Spindle.

## Native surface and service paths

The app reads its immutable self catalog instead of assuming every instance is domain/task 12. Fresh content surfaces302–305 form the existing 2×2 grid, clipped into shell-owned frame 306. Registration presents primary 302; the shell derives frame 306 from the exact catalog association. Frame geometry, focus, close, hit testing and display retry custody include the fresh frame. The app cannot create or own frame 306. Old content154/160/161/162 and frame 153 stay tombstoned.

Fresh-frame input goes to the reserved route and native app. It does not run the predecessor's shell-local YARN dispatcher or inherit that model's text/history. Private command history starts empty. Its legacy handle0 history persistence is explicitly reported unavailable for the successor; no private predecessor handle is transferred. The app has its own BOOT display/storage/shell/Linen/Bell edges.

Linen has a QUERY-only observer for task 17/surface 302 at slot 50. It checks the exact caller/task/surface/coordinator and live status for each queued request. The original task 12 observer remains separate. Legacy Spindle replies are best effort and never take custody of the permanent shell peer's response slot.

The first guest attempt exposed a real startup bug: a full display queue dropped primary 302's create while the dirty cache assumed it existed. Successor display operations now retain the exact create, clear, text or hide packet across transient backpressure, yielding this app's execution while the display and shell progress. Cache publication follows full text admission. Permanent denial ends the task rather than claiming success. This is not a hard real-time or failed-display recovery claim.

The next guest reached native presentation and exposed punctuation being consumed as shell geometry actions. The shared focused-Spindle filter now admits every printable character in the native US key map except grave, which remains the launcher shortcut. The host test extracts both production functions and also checks that function keys remain available to the desktop. The acceptance types `linen-list` through physical key events, including the minus key.

After those fixes, native commands and four-grid pixels passed an independent check, but the automated acceptance had relied on a desktop-wide geometry diagnostic whose boot budget was exhausted. The checker now requires all four unbudgeted target 17 viewport records, verifies their exact 2×2 relationship, and compares the production-font pixels inside those actual bounds. A complete fresh acceptance follows this harness correction; no guest shortcut substitutes for the remaining Close/exhaustion checks.

The harness also waits for the native editor's exact length/cursor receipt per typed character: inserting a space may leave the padded grid's pixels unchanged. Final framebuffer matching remains mandatory. Bell acceptance requires the explicit notify request (`caller_pd=17`, `requested=1`) and a subsequent accepted event, rather than accepting the automatic command-completion notification (`requested=0`) as that evidence.

## Verification

The sealed normal build passed its 90-second master window (`GREEN_MASTER`) and normal two-client paging regression: nine recovered faults, four observed leaf reclamations, three cross-client reuses and one self-reuse. All **38 host gates**, **446 Rust test executions** and **236 Python tests** passed. Host tests cover both activation grants, original and fresh group ownership, four-quadrant clipping, independent Linen authority, permanent retirement and exact display retries. Synthetic pixel fixtures reject missing/old subjects, blank output, wrong geometry and malformed grid payloads; they do not establish real guest visibility. The architecture inventory remains 30 obligations: 7 open and 23 partial; this is not a complete-system proof.

`/tmp/sexos-spindle-desktop-complete/` passed the full native acceptance through real QMP keyboard events:

- Quil saved document2 at 242 bytes / `0xdbc6d3de7f43121f`. Its successor16 recovered the exact document, accepted real input, displayed 243 dirty bytes, and closed cleanly.
- Spindle 12 completed all five ACKs and private retirement before 17 activated. Early delivery/activation was denied; the old route stayed denied after activation; a duplicate activation returned the same receipt. Exactly one fresh task 17 start was observed.
- Primary 302 received authenticated native presentation. Real `linen-list` input, including the minus key, reached the native editor. Linen independently authorized task 17 through observer 50. Explicit `notify spindle17` reached Bell with `requested=1` and produced an accepted event.
- `echo spindle17` executed in task 17. Complete production-font glyphs, including blank cells, matched all four actual content viewports302–305 in the guest framebuffer. Each quadrant contained visible glyph evidence.
- Close 17 used slot 51, finished the exact task and obtained all five ACKs before private retirement. Its 80 pages /327680 bytes were fully scrubbed by Sext. Across targets9,12,15,16,17, retirement covered **277 pages /1134592 bytes**. Terminal identities and control metadata remain retained.
- Both exhausted launcher rows rejected Enter. Actual launcher glyphs showed the exhausted state. Surviving Linen accepted selection during a real refresh on the first attempt (host-observed action latency about 104 ms), retained selection, displayed complete filename glyphs, and passed visible F8 hide/restore. This observation establishes functional overlap, not a hard timing guarantee.

[Native Spindle successor](assets/sexos-spindle-successor-2026-10-29.png) · [Exhausted launcher](assets/sexos-spindle-exhausted-2026-10-29.png) · [Surviving Linen](assets/sexos-spindle-linen-restored-2026-10-29.png) · [Source/artifact evidence](assets/sexos-spindle-relaunch-2026-10-29-evidence.json) · [Selected native trace](assets/sexos-spindle-relaunch-2026-10-29-trace.json).

Normal ISO SHA-256: `0789135862040aaa73700adf8cd25e192326bbad4dc7bfb93cd622663c32ddef`.
Kernel SHA-256: `af0e1974a55546cf7338707f3501ee52ba586eb3d9b3877b17a38626bc4b6526`.

The root ISO/kernel match the saved tested artifacts. Compiled Rust inputs remained unchanged after the sealed build; later changes corrected only the acceptance harness and documented the existing spec limits. Raw build, host and guest evidence hashes are recorded in the JSON. No emulator remains running after acceptance. The Spindle changes remain local for review after the initial Quil commit/push.

## Limits and next gains

There is one reserved successor each for Quil and Spindle. Closing the second instance exhausts that app's reserve until reboot. Activation remains BSP-only and rejects more than one initialized execution core; it does not claim cross-core invalidation. `ARCHITECTURE.md` remains unchanged.

The next useful slice is incarnation-owned history persistence with actual handle receipts, followed by reliable app registration retry/cancellation. Replenishing reserves requires explicit BOOT-sanctioned backing/image authority; terminal identities and Sext journal frames are not an implicit source of launch authority.
