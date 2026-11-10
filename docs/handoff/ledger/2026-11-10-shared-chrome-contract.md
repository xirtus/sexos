# Shared chrome contract, first implementation slice (2026-11-10)

Andreas explicitly approved the slim desktop concept and shared-F2 plan in the chat
on 2026-11-10 and requested implementation and integration. This starts that work;
`2026-11-10-slim-chrome-shared-f2-plan.md` remains the approved design contract.

The SDK now owns pure 28px total bar, 26px title, 24px control target, 13px UI font,
8px resize band and 6/8px gap metrics. Bounded logical/device conversion, exclusive
control boxes, workarea and content insets share one implementation. Empty/tiny frames
cannot wrap dimensions or select content outside the frame. These future geometry
values are not switched into live painting/input in this first slice.

Display's live scale step/reset uses the common bounded function, preserving existing
1..3 behavior and BOOT reset. Adversarial i64 payloads cannot overflow. Independent
literal goldens exercise scale 1/2/3, adjacent control boundaries, tiny frames and
maximum integer values. Python host discovery compiles and runs the production SDK
against those goldens. ABI pins are regenerated; no wire or kernel change.

Next slice: authenticated, bounded applied-scale publication receipts and Shell state,
then coordinated Shell/Display/grouping/damage/client migration to these metrics and
actual thin bar geometry. The enqueue return is not an applied-scale receipt; do not
claim it is one. Current screenshots remain the old geometry until that switch.

Validation: four Rust host cases and Python wrapper pass; `git diff --check` passes.
No screenshot or performance claim is attached to this pure-contract slice.
