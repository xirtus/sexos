# YouTube in the SexOS browser: gap report and plan (2026-11-09)

Goal (Andreas, 2026-11-09): youtube.com loads and plays in `run +net web`, as a
priority, with something playing before bare-metal testing in the week of 2026-11-16.

## Ground rules

- We get there the way any browser does: run YouTube's own pages and player script
  on a faithful web platform. SexOS will **not** pretend to be a YouTube mobile or TV
  app to pull stream addresses out of YouTube's private API. That would sidestep
  YouTube's access checks (and its terms), it breaks every few weeks, and it teaches
  the browser nothing. Everything below is general web platform work that every
  other site benefits from too.
- The cloud build containers cannot reach youtube.com (the egress proxy refuses it),
  so every step is tested against local fixtures that behave like the parts of
  YouTube that matter (chunked + gzip replies, cookies, Range requests, MSE
  segment feeding). The first real youtube.com test is on Andreas's machine or the
  bare-metal box.

## What YouTube needs, and what SexOS has today

| Area | YouTube needs | SexOS today (master 21170cf) | Gap |
|---|---|---|---|
| TLS | TLS 1.3 or 1.2, SNI, ECDSA/RSA chains to public roots | sexnet: embedded-tls TLS 1.3, AES-128-GCM only, webpki + webpki-roots (`open-web`) | Small. 1.3 + AES-128-GCM is accepted by Google front ends. ChaCha20 and TLS 1.2 matter for other sites, not YouTube. |
| HTTP | HTTP/1.1 is enough (HTTP/2 and QUIC are optional). GET and POST with JSON bodies, request headers, `Range`, chunked replies, gzip/brotli | `sex_app::net::fetch`: HTTP/1.0 GET only, fixed headers, 900-byte request, no request body, no chunked decoding, no content decoding, whole reply into one preallocated buffer | **Large.** No POST means no `youtubei/v1/*` calls at all. |
| Connections | 6+ parallel flows to www.youtube.com, *.googlevideo.com, i.ytimg.com, fonts.gstatic.com, keep-alive | One TCP flow for the whole machine (sexnet `web::Service`, one owner at a time), 64 KiB RX, 16 KiB TX, one request per TLS session | **Large** for speed, workable for a first play (sequential). |
| Cookies | `Set-Cookie` / `Cookie` (VISITOR_INFO1_LIVE, YSC, consent cookies in the EU), `document.cookie` | `document.cookie` returns `''`; no jar | Medium. |
| Storage | localStorage, sessionStorage, IndexedDB (player settings, ad state) | in-memory localStorage/sessionStorage, no IndexedDB | Medium (IndexedDB can start as an in-memory shim). |
| JavaScript | ES2021+, ~3 MB player (`base.js`) for embeds, ~10 MB more for the full desktop site (Polymer/Lit), BotGuard VM | QuickJS-NG compiled to Wasm and run on wasmi (an interpreter inside an interpreter); load budget 120 s | **Large** for speed. The JIT thread (V8, stages J1-J8) is the long-term answer; QuickJS speedups in "Speed up browser games" help now. |
| DOM / web APIs | custom elements v1, shadow DOM, MutationObserver, Intersection/ResizeObserver, XHR, fetch with streams, Promise microtasks, TextEncoder/Decoder, crypto.getRandomValues, performance.now, Intl, requestIdleCallback | prelude.js DOM over `__host`; fetch (GET only, whole body) from PR #45; no XHR, no custom elements or shadow DOM, no observers | **Large.** Shared with the web platform thread. |
| CSS / layout | flex, grid, custom properties, calc, transforms, media queries, position sticky/fixed | block/inline/table/flex, integer layout | Large for looks, small for "plays". |
| Fonts | Roboto/YouTube Sans via woff2 (brotli) | Geist only | Small (fall back to Geist). |
| Images | JPEG/WebP thumbnails from i.ytimg.com, inline SVG icons | PNG/JPEG | Medium (WebP decoder, inline SVG paths). |
| Media element | `<video>` with currentTime, play/pause, events (timeupdate, canplay, waiting), A/V sync | none in the browser; Play app plays files with FFmpeg + Bell sound rings (PR #49) | **Large.** Owned by "Play images and video with FFmpeg". |
| Media Source Extensions | MediaSource, SourceBuffer (appendBuffer, remove, buffered, timestampOffset), `isTypeSupported`, fMP4 and WebM byte streams | none | **Large.** This thread, on top of the video element. |
| Codecs | VP9 or AV1 or H.264 video, Opus or AAC audio | FFmpeg n7.1.2 with H.264, HEVC, VP8, VP9, AAC, Opus, Vorbis (PR #49); no AV1 decoder (needs dav1d) | Small for H.264/VP9 + AAC/Opus. AV1 later. |
| Streaming protocol | The current web player streams through its own protocol over POST (SABR/UMP) or classic DASH ranges, all driven by player JS | n/a | None of our own: it is just fetch/XHR + MSE once those exist. |
| Encrypted media | Not needed for ordinary videos (only films/paid content use Widevine) | none | None for the goal. |

## Fallback paths (in order)

1. **Embed player first.** `youtube.com/embed/<id>` (and youtube-nocookie.com) is
   the player without the site around it: the same media needs, far less layout
   and script than the desktop watch page. This is the first real target.
2. **Light front-end sites.** Sites that serve a plain `<video src=...>` with a
   progressive MP4 (for example self-hosted Invidious/Piped instances, which do
   their own YouTube access) work with generic features alone: HTTP/1.1, chunked,
   Range streaming and a network `<video>`. SexOS adds no YouTube-specific code for
   this; the person chooses whether to use such a site.
3. **Full youtube.com watch page** once the JIT and the DOM/CSS work land.

## Plan

| Stage | Work | Owner |
|---|---|---|
| Y1 | HTTP/1.1 client: methods, request headers and bodies, chunked decoding, gzip/deflate, Range, redirects; a pure `sex-fetch` crate (host-tested) used by `sex_app::net::request`; browser loads pages and `fetch()` through it | this thread |
| Y2 | Cookie jar (Set-Cookie parsing, domain/path/secure/expiry, `Cookie` header, `document.cookie`), XHR on the same host op, `fetch(init)` method/headers/body, real `Response.headers` | this thread (small prelude diff, coordinated with "Speed up browser games") |
| Y3 | sexnet: several flows per program and several programs at once, 256 KiB windows, keep-alive TLS sessions, ALPN `http/1.1` | this thread (wave-1 networking had no owner yet) |
| Y4 | Network `<video>`: stream a `src` in Range pieces into FFmpeg (AVIO read callback over fed bytes) | video element: "Play images and video with FFmpeg"; network source: this thread |
| Y5 | Media Source Extensions on that element: fMP4 and WebM SourceBuffers through FFmpeg demuxers, `buffered`, `remove`, `timestampOffset`, `isTypeSupported` | this thread |
| Y6 | Web APIs the embed player touches: custom elements, shadow DOM (flattened), MutationObserver, observers, TextDecoder, crypto, inline SVG, WebP | split with "Speed up browser games" |
| Y7 | JS speed for multi-megabyte bundles | "Port a JIT engine to the browser" |
| Y8 | youtube.com/embed plays; then the watch page | this thread, tested on real hardware |

## Test fixtures

- `scripts/web_site` grows HTTP/1.1 behaviour (chunked, gzip, Set-Cookie, Range) in
  the local gate server, so the QEMU gates prove the same paths YouTube uses.
- A local "tube" page that drives MSE the way the YouTube player does (fetch
  segments with Range, append to SourceBuffers, seek) becomes the MSE gate.

## Progress log

- 2026-11-09: report written. Y1 and most of Y2 done: `sex-fetch` crate, `sex_app::net::request`, browser loads through it with a cookie jar, `fetch(init)`, `XMLHttpRequest`, `document.cookie`; gate `scripts/program_web_http11_gate.py`. Measured about 110 KB/s for a 1.3 MB body in QEMU, so Y3 (windows, several flows) matters for video.
- Also missing for cookies: programs have no civil clock, so the browser counts cookie time from 2026-11-09 plus uptime. A real clock for programs is a small kernel/SDK item.
