# Native browser gates need a visible fixture viewport (2026-11-10)

- **Observed failure:** the normal desktop tiles Web beside the other running
  programs, leaving a 566x177/181-pixel frame. Web paints a fixed 410x330 image.
  Canvas compared zero wrong pixels but only 3772 samples, below its coverage
  threshold; the pointer gate's link click landed at y=793 beyond its frame's
  bottom y=792; CSS animation's hover point sampled clipped bottom chrome.
- **Gate preparation:** a shared helper checks the actual tile. If it clips the
  fixture, real keyboard input selects Shell's source-bound Zoom Toggle command,
  records the resulting geometry and requires the full fixture to fit. Canvas,
  WebGL, three.js, Web APIs, pointer and CSS-animation gates use it. Animation
  expands only after the existing initial animation/timing assertions. Pixel
  coverage, pixel tolerances, input-event and animation assertions are retained.
- **Coherent native inputs:** source `ade1890e815b246b53ecacfeb769f8112a846351`;
  CI normal ISO SHA-256
  `fa177e2bf579fea7d681edb2093869b9d0f7511d4c803a21742ad6dcf9e5d891`;
  default-native FFmpeg Web extracted from that ISO, SHA-256
  `b4c0b2d2ece121c24a4a0abbd1dc0d7b4a96471053cbdf9ce7c0a6577c2106dc`.
  Experimental baseline-JIT feature is off in this Web binary.
- **Guest validation:** frames, Canvas 2D, WebGL, three.js, Web APIs, pointer and
  CSS animation all pass. Canvas/WebGL/three.js compare 9020 pixels in each of two
  states with no differences; APIs compare 9020 with no differences. Animation
  records 122 frames, 23 in-between animation samples and six in-between hover
  samples. Frames verifies actual Referer, postMessage and click colors; pointer
  follows its link. Durable inputs, raw logs, screenshots and result/runner hashes:
  `/Users/xmacbookm5/Projects/sexos-evidence/browser-native/ade1890e/`.
- **Limits:** these are bounded fixture gates on QEMU, not general web/graphics
  compatibility or hardware proof. The separate MSE gate currently fails its
  live-audio continuity check (~200 ms gap after the first second); all decoded
  frames play eventually. That remains an independent investigation.
