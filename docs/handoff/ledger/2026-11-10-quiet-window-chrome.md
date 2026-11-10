## Quiet resize bands, desktop polish stage 1a (2026-11-10)

- **Plan first:** this implements the first resize-band slice of the merged
  `2026-11-10-desktop-ux-plan.md`. Geist defaults/caption correction are the prerequisite
  (PR #126); this topic does not change fonts, toolbar/tab/control geometry or input.
- **What changes:** side and bottom resize bands keep their full 8px hit area. Their
  inner fill uses the existing navy native-canvas token; only the outside pixel carries
  the currently applied rim tint. The rim stops varying with the frame counter.
  Palette tokens, glass sampling count, scene ownership and content bounds stay fixed.
- **Why:** the thick luminous bands compete with content. A quiet inset and stable
  hairline borrow the supplied Metaspace reference's restrained edge treatment without
  adding blur, shadows, rounded-corner hit masks or animation work.
- **Verification required before acceptance:** actual compositor/reference regression,
  guardrails, sealed build, real before/after captures at 1024x768 and 1280x800;
  native scale and window move/resize/close/keyboard flows; comparable repeated guest
  input-to-visible traces. Do not infer a guest frame-time improvement from removing
  the pulse arithmetic. A new focus-ring/geometry pass remains separate.
- **Test maintenance:** resize gate used a pre-E1 zero-gap expectation. Its independent
  goldens now retain the established 8px outer/6px inner gaps while checking the same
  real corner drag, size delta, floating position and subsequent window-open behavior.
- **ABI / kernel:** none. No new server buffers or per-pixel allocation.
