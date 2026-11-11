# Daily-driver checkpoint: integrated source `17a1a530`

Snapshot: 2026-11-10, 20:54 UTC. This fragment updates the evidence status of
[the original capability checklist](2026-11-10-daily-driver-capability-gates.md);
it does not replace that checklist or claim a daily-driver release.

Integrated master is `17a1a530078b9916027399c5e13af81a94d71d19`.
The original #77/#94/#110/#111/#118/#119/#120 backlog and subsequent #121–#156
are merged. PRs #157 and #158 remain drafts at this snapshot. Each integration
was admitted using its exact tested head; the union of historical results below
is **not** a claim that one final image has passed every hardware/workload gate.

## Outcome changes since the planning snapshot

| Outcome | Integrated evidence | Acceptance still outstanding |
| --- | --- | --- |
| Safe JavaScript acceleration | #125 adds opt-in hot-bytecode x86 execution, interpreter fallback, and one/four-core W^X/revocation checks. See [JIT evidence](2026-11-10-baseline-bytecode-jit.md). Narrow warm arithmetic was about 2.48× faster. | Broader workloads, cold-start cost and real website compatibility; no whole-browser speed claim or default enablement. |
| Real browser window sizing | #152 uses authenticated Window size/grant bounds, updates DOM before resize, reflows frames and waits for actual full-frame scanout in its gate. Final exact head `40be3818b703e17edbc7934c4eff43943a813ad9` passed all 14 checks. | This is physical viewport sizing, not CSS device-pixel-ratio or system-wide Web font scaling. |
| Percentage-sized embeds and native Promises | #153 resolves replaced-element percentages using the containing block. #150 keeps native Promise identity with actual rejection events and per-function source provenance; opaque graphs fail closed. | Selected everyday navigation/auth/forms/download/storage tasks and genuine YouTube playback. |
| Readable, consistent desktop | #146 integrates the approved 28-pixel bar, 26-pixel titles, 24-pixel controls and Geist Sans UI/Mono editor typography. QEMU layout, scaling, recovery and capture evidence is in [the geometry ledger](2026-11-10-slim-geometry-implementation.md). | Rounded-panel #158, natural proportional application labels, Settings/accessibility/keyboard gaps and hardware display qualification. |
| Native screenshots and video encoding | #147 saves actual desktop PNGs. #148 adds an optional caller-domain FFV1/BGR0 Matroska encoder, with decoded guest-frame/PTS checks. See [capture](2026-11-10-native-desktop-capture.md) and [encoder](2026-11-10-lossless-encoder.md). | A complete recorder application, visible committed recording indicator, stop/restart/failure handling and cross-domain capture denial. The encoder alone is not screen recording. |
| Reliable recorder prerequisites | #149 pins exact BOOT-approved image bytes; #151 observes held-capability service epochs; #154 adds explicit pre-allocation heap limits; #155 preserves actual task stamps through Files deferral; #156 bounds run/worker task IDs without wrapping. | These generic mechanisms confer no recorder BOOT role or capture authority. Provider/controller/app integration and native failure gates remain necessary. Heap limits exclude image/static/stack memory. #156 does not prove every independent ID namespace safe. |
| Signed updates and quieter boot | #111/#122 integrate the trust anchor and signed-update path; #123 quiets successful boot checkpoints while retaining network/radio diagnostics. | Owner-controlled offline release-key backup, final candidate rollback/power-cut evidence and boot-chain limits. Do not infer Secure Boot support from signed update payloads. |
| Wireless connectivity | #120's rekey/rate/WPA3 implementation is integrated. The [Intel handoff](../INTEL_WIFI_HANDOFF_2026_11_10.md) remains the hardware bring-up guide. | No radio has been qualified by QEMU. x17 association/DHCP/HTTPS, replay-safe rekey, actual rate changes, WPA3 failures/reconnect and fallback paths require hardware receipts. Use OpenBSD `if_iwx.c`, never Linux `iwlwifi`, as the reference. |

## Final combined viewport receipt

The [exact-head runtime job](https://github.com/xirtus/sexos/actions/runs/42891395722/job/130093161830)
for #152 tested the slim desktop together with the integrated Promise and percentage
layout changes. Its ISO SHA-256 was
`9b5aeb1b45a655ed64ed7c0b1775ff92cf801a54f76af4164e85b1fdea2dfe99`;
Web ELF SHA-256 was
`0942fbed3e1975b3ba5d1ccb42cf70a7e4126d6930abf309ac107d05aa9701eb`.
Seven states covered tiled `550×144` and maximized `1264×730` surfaces,
2,905,200 document pixels, resize counts zero through six, and three JS-render
checks of 61,732 samples each with zero error. The gate retains failed scanout
attempts and waits within its bound; an admission log is not treated as painted pixels.

For #155 and #156, root independently verified each exact-head native artifact's
archive/raw-serial hashes and ordered lifecycle receipts: row 42 ran `stdhello`
14/14 twice, retired, then rebirthed from task 42 to task 1073741830, with Files
acknowledging cleanup/binding before commit and the second run exiting cleanly.
This proves ordinary reload compatibility for those candidates, not recorder
authority or all possible concurrent exhaustion schedules.

## Current browser blocker and draft work

Genuine HTTPS YouTube embedding now yields the actual serialized `onReady` event.
It subsequently fails while compiling Google's TH script: native trap 10 is
call-depth exhaustion at depth 1025 with the production guard still 1024.
The same trap was present in older receipts; an absence of reported JS exceptions
did not establish trap-free execution. Playback and `/watch` remain unverified.

#157, head `dd914fd4ec3e56265b80f50d485eb9c675b4660b`, includes this master.
It replaces only the fixed binary-precedence parser recursion with nine local
continuation slots. Paired pinned compilers preserve byte-for-byte emitted code
and results/errors in the covered fixtures. An old-engine deep-expression
negative exhausts the call stack; the candidate evaluates it successfully.
Final coherent-image, native and genuine HTTPS checks remain pending. Guard,
time/instruction limits, provenance and kernel policy remain unchanged.

For generic JIT logs, `blocked:69` means the refused byte offset of **one**
forbidden sequence. It does not mean 69 different instructions or test cases.

#158, head `441362dd6281afaed6b2ef31d9d8dadcaf2a7f38`, adds a shared rounded
mask and next-owner antialiasing without changing desktop geometry. Initial
1280/1024 QEMU captures pass Geist/clock checks. Corner/control interaction,
complete stack headroom and fresh matched performance evidence are not complete.
Its independent frozen baseline is `5f946c33`; do not compare different profiles
or silently substitute an advancing master when measuring it.

## Hardware and release decision

The outcome checklist's outstanding storage durability, foreign-volume integrity,
touchpad, battery/thermal/lid/suspend, audio, real networking, service-failure
soak and full-day workload gates remain open. Linux capability parity remains
the full outcome backlog, not an inference from the number of merged PRs.

The existing `scripts/real_hardware_preflight.sh` also contains stale assertions:
it says SexOS does not use an IOMMU, conflates VMX/SVM with DMA remapping, and
looks for a target JSON outside the repository before the actual checked-in file.
Correct those before relying on that script as an x17 test guide. A host inventory
can report available evidence; it cannot attest the later SexOS boot's isolation.

Continue preserving Rust SASOS mechanisms, user-space policy, PDX/PKU/W^X and the
unchanged 21,500-line kernel budget. Keep private signing material outside evidence
bundles and Git. Only a fully scoped, candidate-bound hardware/workload receipt can
promote an outcome from QEMU-qualified to hardware-qualified.
