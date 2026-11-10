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
- 2026-11-09: Y3b: sexnet makes open-internet flows on demand (each with its own 64 KiB window and TLS session, numbered per caller; memory from a growing BOOT arena). `sex_app::net::Exchange` reads a request a step at a time, and the browser runs up to 16 script requests side by side (twelve 1.5 s requests take 1.7 s). Keep-alive and ALPN are still to do.
- 2026-11-09: Y3c: `Exchange` sets up a step at a time too (look up, connect, handshake, send: one call per poll), with a per-program cache of looked-up names, so a site that never answers no longer holds up the others (gate: a request beside one to an unreachable address lands in 1.2 s).
- 2026-11-09: Y3d: keep-alive. Requests no longer say `Connection: close`; when a reply's framing ends exactly at the received bytes and the server didn't ask to close, `sex_app::net` keeps the flow idle (per host, port and TLS; up to 32, for 30 s) and the next request to that site goes out on it, skipping the connect and TLS handshake. A kept flow that turns out dead before any reply byte is retried on a fresh one. The open-web TLS session is now full duplex (`split()` reader/writer), so one session carries many requests; host test against an OpenSSL peer: 3 requests on 1 connection. Gate: 17 requests over 13 connections on the page's server. ALPN is still to do.
- 2026-11-09: Y3e: ALPN. The open-web TLS handshake names `http/1.1`, as browsers do; the host test's OpenSSL peer offers `h2` and `http/1.1` and must pick `http/1.1` (rustls in sex-tls sets it; the test proves it on the wire). Y3 is done apart from bigger windows (64 KiB now) and HTTP/2, which wait for measurements on real video.
- 2026-11-10: Y5 (fMP4): `MediaSource`, `SourceBuffer` and `SourceBufferList` in the browser. `sex_web::mse::Stream` reads the init segment and each moof's times for `buffered`; each SourceBuffer is a growing `sex_media::Source` that FFmpeg reads in order, one fragment at a time, and separate video and audio SourceBuffers play on one clock (`Player::pair`). Playback starts before `endOfStream`. Gate `scripts/program_web_mse_gate.py`: Range-fetched segments play 100 pictures with both tones heard; a 12 s stream appended in 16 KiB slices every 500 ms plays all 300 pictures and its tone unbroken. Still to do: WebM SourceBuffers, seeking inside MSE, `timestampOffset`. `sex_fetch::pieces` (Range piece bookkeeping) is in for the media thread's seekable progressive `src`.
- 2026-11-10: Y5 done: WebM SourceBuffers (VP9, AV1, Opus, Vorbis), `timestampOffset`, `abort`, `changeType`, seeking, and quality switches. Each SourceBuffer keeps its segments by time; the decoder reads an init segment and then the segments in time order from where playback is. A seek waits (`seeking`) until the page has appended the segments that hold the new time, then starts over there; a new init segment mid-stream (a quality switch) opens the decoder again where its segments begin, keeping the clock; a feed that runs dry carries on at the next segment. `remove()` lets segments go; a full buffer (160 MiB) lets go of what was played. Gate `program_web_mse_gate.py` covers all of it (a 320x240 to 160x120 switch at 4 s plays 200 pictures; a seek to 6 s made before its segments exist waits, then plays; WebM plays 100 pictures with its tones).
- 2026-11-10: Y6: the web APIs players and frameworks reach for. A working `MutationObserver` (childList, attributes with filter and old values, characterData, subtree; records arrive in a microtask), custom elements now hear `attributeChangedCallback` and `disconnectedCallback`, and shadow DOM, flattened: `attachShadow` makes a real `display: contents` element that is the host's first child in the browser's tree and is hidden from the host's children in the page's view, so it draws in place; `<slot>` (`assignedNodes`), `getRootNode`, `composedPath` and event paths cross it. Its styles apply page-wide and slotted children draw after it, which is the flattening's cost. Also `postMessage`, `MessageChannel`, `BroadcastChannel`, `Intl` (QuickJS has none: numbers, compact numbers, currency, dates and times in UTC, plurals, lists, relative times, collation, for `en-US`), `DOMParser`, `XMLSerializer`, `DOMException` with names and codes, `DOMRect`, `Range`, Web Animations (`element.animate`, `finished`, fill), `navigator.mediaCapabilities.decodingInfo` (answers agree with `MediaSource.isTypeSupported`), `sendBeacon`, `cookieEnabled`, `document.currentScript`, `<style>.sheet.insertRule` (CSS-in-JS), `window.onerror`, and fullscreen and picture-in-picture saying no. Host tests in `sex-js/tests/dom.rs`; gate `scripts/program_web_apis_gate.py` builds a page from them and matches the host render pixel for pixel. Not done: `ReadableStream` bodies, IndexedDB, WebAssembly (pages test for these and take their other path).
- 2026-11-10: Y8 begins: scripts a page inserts now run (loaders, analytics and players add their bundles that way), and a script that arrives by fetch runs with the 120 s load budget instead of the 10 s event budget.
