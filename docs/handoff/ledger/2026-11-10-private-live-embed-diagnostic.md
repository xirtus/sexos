# Private live embed snapshot (never merge)

Base: production parser head `dd914fd4ec3e56265b80f50d485eb9c675b4660b`.
Branch: `codex/private-embed-live-state` in the isolated parser worktree.
Status: source prepared; no private binary or runtime result yet.

Production parser 157 is merged separately. Its frozen default and opt-in Google
HTTPS observations both run 180.1 seconds without the old TH exhaustion, but show
only a black 720×405 child region and no decoded-media evidence. Initial-script
JIT snapshots show 0; there are no observed code seals. This establishes no
whole-run entry/fallback count or useful real-page speedup.

This private candidate changes only a native snapshot module/key handler and
read-only Rust Document accessors. Wasm remains `cfe27438…`, ABI `cbf69f94…`.
It must not be described as the production binary or a coherent release image.

- Cache selected computed styles only during real relayout. Key by immutable
  document resource-owner token and actual parent/child slot. Final stale/missing
  caches report unavailable; no recomputation or synthetic values. Additional
  detached nodes are accepted only after bounded native ancestry checks establish
  that they cannot change the connected layout and the document is not dirty.
- At most 4 native Ctrl+Shift+Y snapshots; at most 5 owner slots, 48 selected nodes
  per slot, 4,096 scanned nodes and 64 ancestor steps. Long URLs do not receive a
  truncated-input policy classification. Runtime C flags remain unavailable;
  origin-policy lookups retain recorded/derived/unknown distinctions.
- Read actual parent/child viewport, page/item/box counts, scroll/timers, retained
  resource queues, raw DOM attributes/parents/boxes and native Media fields. JS
  media properties and engine containing-block IDs are explicitly unavailable.
- Existing per-VM Script::jit_stats reports late host-cache compilation, actual
  entries, bailouts and refusals. C metadata skips/hot-function identities and
  pending-JS-job counts remain unavailable. No guest C change in this phase.

No JS evaluation/getters, job drain, forced layout, pixel publication, website
source replacement, fake headers or guard/budget change. No per-call logging.
The same real HTTPS URL/viewport is the first planned replay; a taller viewport
may later expose ordinary website Play controls, without constructing a player.
Private compilation/native execution waits for the coordinated UX window.
