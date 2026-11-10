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
selector behavior. All 42 DOM tests pass on the ARM interpreter fallback.
No kernel, ABI, Wasm guest binary or runtime resource-policy change.

The previous real HTTPS guest probe fetched and evaluated `www-widgetapi.js` but
did not verify a player or playback. This small compatibility fix does not change
that evidence boundary. A combined-resource replay is still needed after #131.
