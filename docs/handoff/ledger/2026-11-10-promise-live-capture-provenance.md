# Live captured callback provenance

An additional negative control extends the earlier adoption correction. A native
finally wrapper registered before opaque code captured an owned Proxy. After an
opaque script changed that Proxy's trap, the intermediate `afabd82a` candidate's
creation-only snapshot missed the now-unknown callback and emitted one forbidden
notification. The exact intermediate Wasm and failing test/log are retained in
`/Users/xmacbookm5/Projects/sexos-evidence/pr150-captured-proxy-control/`.
This remains an unmerged draft-candidate finding, not a shipped-master API claim.

Source classification now reads captured callback/Promise metadata at use time.
The shared traversal budget permits at most 64 object visits, follows bound/native
closure captures and resolver Promise tags, and never invokes a JS getter, Proxy
trap or callback. Cyclic/deep ambiguous graphs conservatively remain muted after
opaque code. Native closure execution uses that live classification. Creation
origin remains retained as well; known ordinary owned captures remain observable.
No queue order/cap, native Promise identity, networking policy or kernel change.

Pinned QuickJS/WASI SDK rebuild produces Wasm SHA256
`dbdb0f6c7459b586b834b2e80639e603442d04167699c50d062d5124503e30e2`;
a second rebuild matches byte-for-byte. All 61 host DOM tests pass, including the
new mutable-capture negative control and the positive owned-origin controls. The
native two-origin gate includes the same pending-captured-Proxy scenario, alongside
throwing thenable/executor/getter/proxy, native-bound/finally, async adoption and
positive owned thenable/getter cases.

Corrected native build/replays, current-master integration and exact-head CI are
pending. Earlier `adbd4f36` and `afabd82a` results retain their exact scopes; neither
qualifies these new bytes. PR 150 stays draft pending native proof/review. Genuine
player rendering/playback remains unqualified.
