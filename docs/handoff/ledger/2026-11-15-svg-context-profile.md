# Post-selector Wikipedia profile: SVG ancestor classification

After #318, a fresh JIT-disabled Wikipedia Microkernel run attributed 5.142 s
of 7.134 s VM elapsed to timer dispatch. That is an inclusive phase, not an
identified single function. Private prelude instrumentation measured roughly
0.8–1.0 s in SVG classification; getElementsByTagName took 0.604 s inclusive
and was not selected as the larger target. Other interpreter time remains
unattributed; this does not claim SVG classification is the largest function
in the whole engine.

For names shared by HTML/SVG, wrapping an element previously called PARENT and
INFO separately for each ancestor (up to 64). SVG_CONTEXT now performs that same
bounded walk in the DOM-owning browser host. It stops at foreignObject or a
non-element, recognizes a nearer nested SVG, and reads current ancestry without
adding a cache. Explicit createElementNS markings and SVG-only tag handling are
unchanged. Kernel, guest memory permissions and JIT machinery are unchanged.

Interleaved genuine HTTPS runs, same binary instrumentation, ISO, 1280x800,
fresh disk, 31 physical Page Down events, JIT off:

| Pair | SVG time at 2,944 classifications before → after | Total VM elapsed before → after |
|---|---|---|
| 1 | 0.785 → 0.129 s | 5.925 → 7.056 s |
| 2 | 0.710 → 0.077 s | 6.244 → 5.636 s |

INFO/PARENT host calls dropped from 16,389/14,779 to 3,067–3,077/1,301–1,451,
with 852 SVG_CONTEXT calls. Initial page checks remained 81 matches, 2,291 paint
items, 778 links and height 22,112. The targeted cost reduction repeated, but
total VM time is mixed: no reliable overall speedup or JIT speedup is established.
Live resources, asynchronous work, scheduling and later paint counts differ.
VM elapsed includes host calls and preemption, not network wait or total page-load
wall time. JavaScript timing uses millisecond clocks and adds overhead to both.

Private profiling source 5a9d75b84 (before) / 66a92af37 (after); production code
56fcf198a excludes that diagnostic instrumentation. Complete JS host suites
(8+79+3+2 tests), sex-html/sex-web suites, existing SVG class tests and a new
foreignObject/nested SVG/depth/mutation test pass. Native profiled Wikipedia
runs pass. Production native classic-script and promise gates pass, using a
separately loaded browser on the previously verified ISO; this is not a fresh
whole-image/hardware qualification. Full site/webmail result is recorded in
target/browser-evidence/local-331/sites.log.

Evidence: target/browser-evidence/next-cost/{detail,after-1,before-2,after-2},
measurements.json (input/serial hashes, phase attribution), and local-331 host,
build and native gate logs. Production browser SHA256:
e6246710c6ebc6832995f843e13b4ec1bcc378d707ccf5fbbdd5eb1f26b4063b.
Runtime ISO SHA256:
fb0da1cff251750973bc6ec87fbca60b60958caa40a8ed7210f00a1928a7ffab.

#325 stays separate/draft. No native object/string speedup is established;
Wikipedia previously exercised zero heap paths, GitHub only limited property
loads. Visible YouTube playback remains an x17 check.
