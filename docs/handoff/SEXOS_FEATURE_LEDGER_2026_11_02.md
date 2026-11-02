# SexOS feature ledger (from 2026-11-02)

User-noticeable daily-driver features, logged here so they don't collide with
the round numbering in `SEXOS_DEVELOPMENT_ROUNDS_109_PLUS_2026_10_31.md`.

## F1. Browser reads HTML pages as text (2026-11-02)

**What changed.** The Browser (Kaleidoscope) can fetch a fixed `text/html`
page from the BOOT-controlled HTTPS endpoint and show it as readable text in
the existing reader. Press **W** in the Browser. Headings, paragraphs, line
breaks, lists, simple tables, character entities and links are rendered; links
keep their text with a numbered reference and the targets are listed at the
end. `script`, `style`, `head`, `template`, `noscript`, `svg` and comments
produce nothing. Output is bounded by the 12 KiB document limit and a cut is
marked `[page truncated]`.

**SexOS rules kept.** Selecting the page is fixed resource data on the same
BOOT endpoint, trust profile and held route (`sex_http::Resource::PageHtml`,
`/page.html`, `Accept: text/html`); it selects no host, port, route or
capability. No script runs. The converter (`apps/kaleidoscope/src/html_text.rs`)
is application presentation code, handed to the exchange as a plain function;
`tls_exchange` holds no HTML policy. `sex_tls::Document::rewrite` replaces the
ready body with its text form and erases the source bytes. No ABI input
changed.

**Evidence** (`docs/handoff/assets/browser-html-reader-2026-11-02/`):
- Host: `tests/browser_html_text.rs` (9 tests, in `sexnet_jobs_host_gate.sh`),
  `sex-http` codec (40, incl. text/html MIME acceptance and refusal),
  `sex-tls` crate tests (all suites green, incl. `Document::rewrite`), Python
  suite 1132 OK.
- Sealed build `make full` green; master runtime gate GREEN (the first attempt
  was YELLOW from a split SilkBar civil serial line, unrelated; rerun green).
- Native `scripts/https_html_gate.py` at 1280x800/e1000e and 1024x768/e1000:
  exact decrypted request, `[browser.html.converted] source_bytes=672
  text_bytes=342 links=2 truncated=0`, released body hash equals the converted
  text, and reader layout plus glyph pixels match the expected text.
- Regression: `https_document_gate.py` (`/welcome`, ready) still passes.

**Limits.** One fixed page path; no navigation to links yet, no CSS, images,
forms or JavaScript. The reader uses its existing fixed-width pane. The
expected text in the native gate comes from compiling the same converter on the
host; the converter's behaviour is pinned independently by the host tests.

**Next.** Follow links within the controlled endpoint (bounded path set),
wider reader pane, then a bounded script interpreter design that fits held
authority (no ambient DOM/network).

## F2. Browser follows page links (2026-11-02)

**What changed.** While an HTML page is open in the reader, number keys
**1–9** follow the matching link `[n]`. A plain absolute path (`/guide`) is
fetched from the same BOOT-controlled HTTPS endpoint as `text/html` and opened
in the reader, whose header shows the new path. Anything else (another host,
a relative or query URL, a cut link list) is refused locally with
`[browser.html.link.refused]` and causes no DNS or TLS work.

**SexOS rules kept.** The path is request data validated by
`sex_http::Path::from_location` (no `..`, `//`, query, host or port). It goes
through `sex_http::request_https_page` and `sex_tls::Session::page` on the
same held route, profile and expected name; it creates no endpoint or
authority. Link ranges come from the converter's own output
(`Converted::targets`), not from re-parsing page text, so page content cannot
forge a target. No ABI input changed.

**Evidence** (`docs/handoff/assets/browser-html-links-2026-11-02/`): host
html_text 11 tests, sex-http 41, sex-tls suites green, sexnet host gate and
Python suite OK; sealed build and master gate GREEN; native
`https_html_gate.py` at 1280x800/e1000e and 1024x768/e1000: page pixels,
link 3 (`https://elsewhere.example/x`) refused with no network work, link 1
followed with the exact second request `GET /guide`, guide text pixels match;
`/welcome` document gate regression passes.

**Limits.** Only links 1–9; no back/history key yet; fixed endpoint only.

## Release gates noted while doing F1–F2

- The reader frame is bounded by the 512-byte Linen text delivery shared with
  Display (47 columns, ~10 rows at 1280x800). A full-window reader needs a
  larger delivery region, which is a Display/Shell protocol change; deferred
  so it does not collide with the concurrent Display text work.
