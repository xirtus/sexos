# Compiled scripts kept across browser runs, per site

Thread "Speed up JavaScript in Kaleidoscope" (2026-11-19), the round after
2026-11-19-id-index-small-subtree-queries.md.

## Finding

After the DOM fixes, the largest cost left on a Wikipedia article is compiling
its 1.1 MB `load.php` bundle and the modules it pulls in (~120 ms, no single
hotspot in the parser). The in-memory `CodeCache` already skips that on a second
visit, but it started empty every time Kaleidoscope started, so the first visit
of each session always paid in full.

## Changes

- `sex_js::code_cache` (now public):
  - Keys carry a partition, the top-level page's origin (`set_partition`), so one
    site never finds or times bytecode from another site's visit.
  - `persist` notes new entries and first sightings (`take_fresh`, `take_seen`).
  - `encode` / `decode`: magic, engine build (SHA-256 of the guest wasm), key,
    SHA-256 of the bytecode. Anything else (another build, another key, a cut or
    damaged file) is refused, and the app deletes it.
  - `insert_loaded`, `insert_seen`, `clear`.
- `apps/web/src/code_store.rs`: `Kaleidoscope/code/<site>/`, `<site>` a hash of
  the origin (the folder list is not a readable history). One file per entry plus
  `seen`. Only top-level http(s) pages, 24 MiB per site, 64 sites, writes batched
  2 s after a page's burst and on quit. Forget-site removes the folder and empties
  memory.
- Bytecode is still only used for byte-identical source under the same address,
  kind and provenance, so it is never trusted more than the source.

## Results (host, native tier, Wikipedia Rust article replay, ms)

| visit | total | inserted scripts |
| --- | --- | --- |
| cold | 408 | 148 |
| second (written) | 340 | 140 |
| warm | 206 | 15 |

The warm row now holds after a restart too, from the third visit a site ever
gets (classic scripts are written on their second sighting, across runs).

## Tests

- `code_cache::tests::{saved_entries_are_checked, names_round_trip_and_partitions_differ}`
- `code_cache_saved_entries_come_back_after_a_restart_for_the_same_site_only`
