# 2026-11-21 Kaleidoscope isolated worlds for extensions (workstream N7d)

- sex-js runs several script contexts over one document, as Chrome's isolated worlds:
  world 0 is the page's own; each extension's content scripts (and
  `chrome.scripting.executeScript`) run in that extension's world (1..15).
- Guest (`crates/sex-js/guest/sexjs.c`): `sx_world(id)` selects or makes a world;
  `sx_dispatch` sends input, timers, page progress and request answers to every
  world (any world may cancel input; the next timer is the soonest of all worlds).
  Mutation records cross worlds by node id (`__sx_world_post`), so each world's
  MutationObservers hear the others' changes. Promise rejection handlers are per world.
- Prelude: isolated worlds leave default actions (focus, click activation, typing)
  to world 0. extensions.js tags each message with its world.
- Host: `Script::in_world` / `has_world`; extension messages to a page go to the
  extension's world (`Realm::Page(owner, world)`), and a page world may speak only for
  the extension that owns it. The page's own world no longer gets the extension key
  on web pages, so page scripts can't reach `chrome` at all.
- quickjs.wasm rebuilt (sha256 6da26a84…).
- Limits: events a page dispatches itself (`el.dispatchEvent`) reach only its world;
  media/MSE notifications go to world 0.
- Tests: `crates/sex-js/tests/worlds.rs`; gate: program_web_extensions_gate.py
  (page and content script can't see each other's globals; cross-world observers).
