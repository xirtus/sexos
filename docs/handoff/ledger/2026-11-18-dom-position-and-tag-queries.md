# Native compareDocumentPosition and contains; tag lookups matched by the host

Thread "Speed up JavaScript in Kaleidoscope" (2026-11-18). Measured with the page
bench (`crates/sex-web/examples/bench.rs`, see 2026-11-15-page-bench-index-maphash.md)
on a replay of https://en.wikipedia.org/wiki/Rust_(programming_language).

## Finding

A guest-side sampling profile (QuickJS interrupt handler printing `new Error().stack`,
a local hack, not committed) put about 80% of script samples under jQuery's
`uniqueSort` -> `sortOrder` -> our `compareDocumentPosition`. The prelude answered
every comparison by listing the whole document (`querySelectorAll('*')`, about
10,000 wrapped elements) and two `indexOf`s, so each jQuery sort was quadratic
in the page. Next was `getElementsByTagName(tag)` (Sizzle's fast path for
`$(el).find('a')`), which also wrapped every descendant and filtered by name in JS.

## Changes

- New host op POSITION (38): `compareDocumentPosition` bits computed from the
  ancestor chains in sex-js `Document::position`. A shadow root is the top of its
  own tree, as its `parentNode` is null to pages. `Node.contains` uses it too
  (was one PARENT call and wrapper per ancestor).
- `getElementsByTagName` with a plain name (or `*`) is a host type-selector query;
  other names (`yt:player`) are still compared literally.
- `sex-html` query: when an indexed candidate list is longer than the queried
  subtree, scan the subtree in tree order instead (a `.find('a')` under one element
  no longer checks every link in the document).
- Test: `document_position_in_one_host_call` (crates/sex-js/tests/dom.rs).

## Results (host, native tier, 9 runs each, ms)

| Wikipedia Rust article | master | new |
|---|---|---|
| total, median (min) | 632 (560) | 322 (296) |
| timer ticks, median | 484 | 170 |
| inserted scripts, median | 100 | 100 |

What is left is mostly script compilation of the large load.php bundles and
interpreter time, then selector queries.
