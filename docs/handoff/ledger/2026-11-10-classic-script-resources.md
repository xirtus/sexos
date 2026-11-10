## Dynamically inserted classic scripts are private resources

The real HTTPS Google YouTube demo probe followed its genuine redirect to
`developers-dot-devsite-v2-prod.appspot.com` and loaded YouTube's `iframe_api`.
The requested `www-widgetapi.js` returned HTTP 200, then Web incorrectly rejected
it for missing ACAO: the prelude implemented inserted `<script src>` with public
`fetch()`. No player success was claimed, and no request headers were spoofed.

The fix uses a private classic-resource queue. The hidden host operation takes a
connected script node, derives its source/type/CORS setting from the actual DOM,
and binds it to a unique Document owner. Rust fetches and evaluates the source
in that owner, sets currentScript and delivers load/error completion. Source
bytes never enter fetch response storage or DOM text and do not pass through
replaceable global fetch/eval hooks. Inline inserted scripts capture intrinsic
eval too. Removed/changed nodes and a new document (even at the same URL) cannot
consume an old queued resource; navigation cancels remaining old-page work.

Default classic loads follow resource semantics, with real Referer and cookies.
Ordinary fetch/XHR retain the existing CORS checks. `crossorigin=anonymous`
retains CORS; credentialed cross-origin script mode is refused until its complete
ACAC policy is supported. Responses honor MIME/nosniff and CORP; cross-origin
loads require a JavaScript MIME type. CORP same-site uses the Public Suffix List
and HTTPS protection, with IP literals matched exactly. These are bounded
compatibility checks, not a claim of complete CSP/COEP/browser conformance.

References: [HTML classic script fetching](https://html.spec.whatwg.org/multipage/webappapis.html#fetch-a-classic-script),
[Fetch CORP](https://fetch.spec.whatwg.org/#cross-origin-resource-policy-check).

Local evidence:
- 15 sex-fetch policy/codec tests and 40 sex-js DOM tests pass, including owner,
  removal/src/type mutation, same-URL document replacement, private bodies and
  replaced eval/fetch hooks.
- The deterministic two-origin `program_web_classic_gate.py` passes: a genuine
  external JS resource executes without ACAO, the same URL via fetch is denied,
  HTML/nosniff/CORP/anonymous-CORS cases are denied, removed/changed and old-page
  sources are not requested, and navigation/close/retirement complete. The server
  verifies actual Origin/Referer headers.
- Separate boot provenance: immutable source 55ae0526 media ISO, ABI manifest
  identical to master dddf3bc0, SHA256
  `a47d5294d1810f3626d5a45c1cd2a95a0835a1be0c75e0bb0a433893f2daea16`.
  Program: this topic's master-based wasm-interpreter Web, SHA256
  `b8c07cdcb6d8c696010c1c217508ef05b08285ae360d75cbbbce41029f2548d1`.
  JSON/logs are outside Git in `Projects/sexos-evidence/classic-script-local-gate`.

CI runs the deterministic guest gate. Native-AOT and the real HTTPS embed are
retested after integrating the engine work; YouTube playback is still unverified.
No kernel or PDX ABI changes.
