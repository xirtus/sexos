# Native Web viewport qualification

Production source d336252bda7ef39054d5b0ecfc4fb2ade71d8bba compiled in a fresh
Rust target with decoder-only FFmpeg and actual native wasm2c QuickJS (baseline
JIT disabled). Optimized build and executable-byte loader scan passed. Web ELF
SHA256: 2ef7302b68f60a89ffeb6f3092ae6279e894c16e3b6329c3357350fde6150365.
Later changes through 68f822b9 affect fixtures only; apps/crates/kernel/spec remain
byte-unchanged from that production checkpoint.

Native tests used this newly linked ordinary Web program with sealed master
3e075b59 services, not an all-source candidate ISO. Service ISO SHA256:
0e68ed67ef7126e745b75a0771b927e60cbb7d2b661de0024980398fe41e3683.
Build/compiler/decoder hashes, raw serial, logs and actual pixels are retained in
/Users/xmacbookm5/Projects/sexos-evidence/web-live-viewport/.

Passed:

- Three optimized production-helper bounds/clipping tests with overflow checks.
- All 45 DOM tests, including host-supplied viewport dimensions before resize
  listeners/property handlers and refusal to take dimensions from message fields.
- Tiny renderer surfaces from 0x0 and 1x1 through 64x32, with unchanged guard words.
- Four geometry-parser cases, including negative page-console impersonation of
  paint/tile/zoom/restore receipts. All 1,166 Python regressions passed before the
  added fourth parser case; the final complete suite remains a CI obligation.
- Native repeated real Shell maximize/restore: initial 550x137, maximum 1264x706,
  restored exactly, with script resize counts 0 through 6. The independent RGB
  oracle checked every document pixel: 58,850 per tiled view and 854,464 per
  maximized view, 2,798,792 across all seven states. The real cursor was outside
  the document. No masks or right/bottom crop exemption; toolbar is separately
  covered by the host/guest same-page oracle.
- Existing JavaScript loaded/clicked/key states: 59,708 full-canvas samples each,
  maximum error zero. Existing pointer coordinates, clicks, menu, wheel and link
  navigation passed.
- Existing cross-origin iframe loading, genuine Referer, postMessage filtering
  and green-to-clicked-blue pixels passed. A stronger JS-responsive half-width
  frame then received child resize counts 1 and 2 across maximize/restore and kept
  its clicked blue state. This tests actual child reflow/event/render integration.

Negative evidence is retained. The first viewport oracle assumed a large initial
window; actual tiling correctly supplied only 550x137. Its sampled pixels matched,
but its sample-count assertion failed. The final oracle covers every pixel and
requires the large dimensions at actual maximize, never at startup.

A separate real CSS layout gap was exposed by an iframe styled width:50%: no box
was produced, so no frame loaded. Atomic replaced-element sizing in
crates/sex-html/src/layout.rs resolves percentage dimensions against zero. The
responsive integration fixture uses a normal page resize listener to assign an
explicit half-innerWidth pixel size; this is not a product CSS fix or a claim that
percentage sizing works. The failed trace is retained as frames-responsive; the
successful callback case is frames-responsive-js. Percentage replaced sizing is a
separate next topic. No website source, Referer or User-Agent was spoofed.

All owned guests stopped after the checks. Final current-master integration,
current-head CI, genuine third-party embedding and hardware remain separate gates.
