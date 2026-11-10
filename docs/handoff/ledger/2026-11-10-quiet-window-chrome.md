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

- **Measured candidate `ed359638`:** sealed build and 55 compositor/reference checks,
  six guardrails, real 1024x768/1280x800 captures, native 2560x1600 scale 1->2->1,
  float/resize/close and keyboard/workspace/layout/scratchpad gates passed. Clock and
  scale-1 text pixel proofs passed. No geometry or font interpretation changed.
- **Timing scope:** QMP Super+M layout request through Shell receipt to an observed guest
  pixel, 20 transitions per trace, three alternating traces per image. Exclusive-guest
  A/A p95 wall ratio was 1.0187, QEMU process CPU ratio 1.0270. Corrected candidate versus
  baseline ratios were 0.9975 wall and 0.9915 CPU, within the 5% budget; no speedup claim.
  An earlier contended run failed at +18.2% wall p95 and remains retained. Process CPU
  observation uses Darwin rusage with the measured Mach timebase; this is not hardware
  latency or a full-frame publication timestamp.
- **Evidence:** local `sexos-evidence/quiet-window-chrome/` holds captures, source/ISO
  hashes, raw samples and reproduction scripts under `captures`, `latency-aa`,
  `latency-isolated` and `latency-contended`. The superseded runtime-toolbar-color
  candidate is retained separately as `quiet-window-chrome-provisional-token`.
- **Integration receipt:** refreshed onto master `fab6598f` (#128 wallpaper/night light,
  plus current JIT/quiet boot changes) after these measurements. The final combined
  source still needs current-head CI and functional guest/capture checks; pre-refresh
  images do not establish wallpaper/night-light integration. Keep the PR draft until
  those checks complete. Future slim geometry follows its own shared-F2 plan and gates.
