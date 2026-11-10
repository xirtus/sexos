# Literal DOM tag lookup after the real HTTPS widget probe

The real Google-hosted YouTube demo now fetches its classic widget resource, but
privileged offline replay identified the next exception: the widget calls
`getElementsByTagName("yt:player")`, which Web passed into its CSS selector parser.
A colon in a literal qualified tag name must not become a CSS pseudo-class.

`getElementsByTagName` now filters descendant elements by their literal lowercase
local name. The wildcard still returns every descendant. CSS `querySelector` and
`querySelectorAll` retain their selector parsing. This preserves the existing
snapshot-array return shape; live HTMLCollection behavior and namespace-aware
XML casing are not implemented by this change.

Host regression covers qualified names, HTML case folding, document and subtree
scope, wildcard, root exclusion, empty/nonmatching literals and unchanged CSS
selector behavior. All 44 DOM tests pass on the ARM interpreter fallback after merging #131.
No kernel, ABI, Wasm guest binary or runtime resource-policy change.

The previous real HTTPS guest probe fetched and evaluated `www-widgetapi.js` but
did not verify a player or playback. This small compatibility fix does not change
that evidence boundary. A combined native-resource replay has now run on detached source `fa1c2e9a`
(#131 `a474238a` plus this literal-name fix `a71fd456`). FFmpeg Web SHA256
`b41766db53d1fe39c4a5b65d3c9dc4be4e4ae25b5f889e008fa1ae9e1560ac08`
passes executable key-byte scanning. The separate boot ISO is source 55ae0526,
SHA256 `a47d5294d1810f3626d5a45c1cd2a95a0835a1be0c75e0bb0a433893f2daea16`;
its ABI-input manifest matches this source (`cd551118355cb882...`).

The unmodified real HTTPS Google demo now evaluates the widget, creates the
YouTube embed iframe, and loads its HTML, CSS, embed-client and 1.9 MB player JS
with HTTP 200. The frame reports a genuine parent Referer. It stops at
`ReferenceError: writeEmbed is not defined`; player/playback remain unverified.
Receipts are in `Projects/sexos-evidence/widget-combined-real-https-embed`.
A separate source-cached privileged host diagnosis also creates the iframe with
no script exceptions (`widget-offline-literal-cached`); it is not guest evidence.
