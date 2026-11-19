# getElementById from the id index; small subtrees walked instead of re-ordering

Thread "Speed up JavaScript in Kaleidoscope" (2026-11-19), the round after
2026-11-18-dom-position-and-tag-queries.md. Same page bench, same Wikipedia replay.

## Finding

`--features profile` timed host operations with the bench's virtual clock (one
microsecond per reading), so its milliseconds were meaningless. The page trait has
a new `profile_ns()` (default `now_ns()`); `Document::profile_clock` lets the bench
keep page time virtual and profile in real time. With real numbers:

- BY_ID (getElementById): 29 calls, 14 ms. Each one listed all ~20,000 nodes.
- QUERY `*` under a just-created (detached) element: 53 calls, 22 ms, and `a`
  under one element between edits: 240 calls, 17 ms. Any tree change drops the
  document's tree order, and these queries rebuilt it for the whole document
  to search a handful of nodes.

## Changes

- `Dom::by_id` (sex-html): the id index, filtered to the root's tree and put in
  tree order; a walk only when the index is off. BY_ID uses it.
- `sex-html` query: with no tree order, a query under any element other than the
  document walks the subtree when it has at most 1,024 nodes, before falling back
  to rebuilding the order.
- Test: `id_lookup_and_small_subtree_queries_after_changes`.

## Results (host, native tier, 2 x 7 interleaved runs, medians, ms)

| Wikipedia Rust article | master (42ea008c) | new |
|---|---|---|
| total | 394 | 332 |
| timer ticks | 209 | 155 |
| host query time (profile) | 53 + 14 | 19 + ~0 |

Left: compiling the large load.php bundles (~120 ms on a first visit; the session
code cache already takes it to ~18 ms on a revisit) and interpreter time spread
thinly over jQuery and MediaWiki code.
