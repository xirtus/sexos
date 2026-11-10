# Native Promise events: native proof and genuine HTTPS boundary

This supersedes the pending local-qualification fields in the initial feature
entry. Runtime source is **bcdb309e400ec1d541923589dc020ed3cccc390d**, before the
subsequent merge of master 3e075b59 (recording profile and dormant image loader).
Do not describe these as post-refresh kernel/program receipts.

## Exact coherent inputs

Fresh normal sealed image from the feature's own worktree/target; default native
QuickJS, JIT feature off, FFmpeg decoding libraries and real musl C-hello.
Homebrew clang 23.1.5; pinned Limine ca3a5b20d2501e3819983c5cf838c70c30ee532a.
Pinned QuickJS Wasm reproduces byte-for-byte with WASI SDK25 clang19.1.5.

Immutable bundle:
`/Users/xmacbookm5/Projects/sexos-evidence/pr150-bcdb-inputs/`.
Its `provenance.json` records source status, compilers and individual input hashes;
`receipt-index.json` binds results, serial logs and the actual private probe runners.

- Normal ISO: `53218b59a4550780a09e855975982847ac58b9ba96f59232d5b6f1477bf94f22`.
- Web: `088c5a3d69c0725157c8734b93cbcfa063f486fe31d1dc65b54f0a918771e3f5`.
- ABI manifest: `156e966b96cb8b453043789e1cd11603d9644e7a5fd31d6ad3625b55dc012ff7`.
- QuickJS Wasm: `adbd4f365f8ce50342b616a6144685d333ae08c096f31a403763a9622ae118b6`.

## Results

`cargo test -p sex-js` passes all55 DOM cases at this exact source. New coverage
includes a final-URL redirect alias regression: an earlier requested-URL policy
must not classify a later final response with the same spelling. Actual final
origins are now classified independently of requested aliases.

Native two-origin gate **PASS**:
`/Users/xmacbookm5/Projects/sexos-evidence/pr150-bcdb-native-promise/result.json`.
It observes native Promise identity and initialization before a following inline
script; trusted/cancelable rejection/default logging and late-handled events;
opaque immediate/timer/async reasons suppressed; CORS-readable resource reasons
visible; actual Origin/Referer headers; independent frame identity; frame removal
cancels its delayed rejection; navigation replaces the document/queue cleanly.

Existing native classic resource/error/CORS/MIME/CORP/cancellation gate **PASS**:
`/Users/xmacbookm5/Projects/sexos-evidence/pr150-bcdb-classic/result.json`.
Both app runs close and private domains retire; guests stop normally.

Genuine unmodified Google-hosted HTTPS demo: two exploratory180s runs, both
`player_success_verified=false`. No Referer/User-Agent overrides, source cache,
page-source replacement, Promise substitution or forced player construction.
Results and screenshots are under `pr150-bcdb-real-https/` and
`pr150-bcdb-real-https-expanded/` in the same evidence directory.

The former `writeEmbed` ReferenceError is absent. Actual widget, embed client and
1.915MB player source load; the expanded run also fetches a later YouTube module
and Google TH script. This is bootstrap progress, not playback acceptance. No
reported new JS exception or media success receipt establishes player readiness.
The child initially reports one paint item and layout height0.

The second run launches a local Browser seed page, maximizes through actual Shell
ZoomToggle, then types the real HTTPS address through Browser's address UI.
The Shell frame is1280×750, **but current Web still paints410×330** and explicitly
ignores authenticated Resize events. Consequently it does not establish a larger
document viewport. Parent and child style/DOM/frame/publication metadata remain a
separate diagnostic task; production dynamic Web viewport is a separate topic.

## Refresh boundary

The feature branch subsequently merged master3e075b59 without source conflicts.
Post-refresh sealed image, native replay and exact-head CI remain pending; preserve
the pre-refresh receipts above. If the refreshed Web is byte-identical, the real
HTTPS observations continue to apply to those exact program bytes, with boot
provenance stated separately. No YouTube rendering/playback or broad JIT claim.
