# Repeatable page benchmark; selector index kept across mutations; Map hashing

Codex's QEMU profiles (#274, #318, #331) left timer-driven start-up as the
dominant, unattributed cost, and live HTTPS runs varied too much to show a total
win. This change adds a host benchmark that replays saved pages, uses it with
`perf` to attribute the cost, and fixes the two largest causes.

## Benchmark: `crates/sex-web/examples/bench.rs`

```
SEXJS_NATIVE=1 cargo build --release -p sex-web --example bench [--features profile]
SEXWEB_FETCH=1 target/release/examples/bench SNAP https://en.wikipedia.org/wiki/Microkernel   # records
target/release/examples/bench SNAP https://en.wikipedia.org/wiki/Microkernel                  # replays
```

It runs the page the way apps/web does (parser scripts, deferred and module
scripts, inserted classic scripts, fetches, timers) on the native QuickJS tier,
with a virtual clock: timer waits are skipped, so each run does the same work and
measures CPU only. Run to run spread is a few percent. `--features profile` adds
the existing jit-profile host-operation and selector attribution. Function names
for `perf` come from a build of quickjs.wasm with `--strip-debug` instead of
`--strip-all` (same function indices; `w2c_sexjs_fN` is `func[N]`).

Host native tier needs `wasm2c` (apt `wabt`); without it the build silently stays
on wasmi, which is about 4x slower and profiles as interpreter handlers.

## Findings (Wikipedia Microkernel, master)

- Selector queries: about 40% of the run. 514 queries cost 228 ms; `html` alone
  221 calls / 67 ms and `*` 102 / 73 ms. Every DOM mutation (including any
  set_attr) dropped the whole index, so almost every query rebuilt it.
- `js_strict_eq2` was the top guest function (12%), under `js_map_get`. QuickJS's
  Map hash leaves the low bits zero for small integers (low word of the double)
  and for 8-byte-aligned object pointers, and buckets are picked by low bits. The
  prelude's `nodes` Map (node id -> wrapper) put thousands of keys in one chain.

## Changes

- `sex-html` query index split into keys (id/class/tag lists, updated in place by
  set_attr/remove_attr/create/add) and tree order (rebuilt lazily after moves).
  A query with at most 64 candidates checks ancestry and sorts by tree position
  instead of rebuilding the order; a key-less selector on a detached subtree
  walks it. Direct arena edits still call `invalidate_queries()`.
- `guest/quickjs-map-hash.patch`: a full avalanche on every Map/Set/WeakMap key
  hash. Guest SHA256 15e5dee11f5c212d94b3d98b15b47c2c3b1aa45cc46277041f04e766a93816e4
  (was 0fd3f83c...).

## Results (host, native tier, 5 interleaved runs each, medians, ms)

| Page | master total | new total | timer ticks master -> new |
|---|---|---|---|
| Wikipedia Microkernel | 560 (514-568) | 234 (223-238) | 398 -> 96 |
| GitHub xirtus/sexos | 2315 (2262-2401) | 2219 (2162-2310) | 17 -> 13 |

GitHub is almost entirely script compilation (next_token, JS_NewAtomLen,
js_create_function, resolve_labels...), so it gains little here. After this
change Wikipedia is also led by parsing. The next lever is a bytecode cache
(compile once per script URL + content hash, keep the bytecode), not the JIT.

These are host CPU numbers, not QEMU VM time; QEMU TCG is roughly 10x slower
but the ratios should carry over. Visible YouTube playback stays an x17 check.

## Follow-up in the same PR: compiled-code cache

Profiling GitHub showed module scripts compiled about twice per load: finding a
module graph (`module_needs`, apps/web) compiles every module in a throwaway
context, and shared chunks again for each entry module, before the real run
compiles them once more. Discovery was about 1.8 s of 2.5 s.

`sex_js::CodeCache` (crates/sex-js/src/code_cache.rs) keeps QuickJS bytecode keyed
by SHA-256 of kind, provenance (muted or not), address and exact source. The guest
(`compile()` in guest/sexjs.c) asks with op CODE_GET (75) before compiling and
sends bytecode with CODE_PUT (76) after a miss it should write. Modules are written
at once (the graph search guarantees reuse); classic scripts on their second
sighting, since writing costs about a third of compiling. Read-back functions take
provenance from the current compile origin (guest/quickjs-code-cache.patch), as
compiled ones do. The browser keeps one 48 MiB cache for the session and gives it
to every page and frame. Page script cannot reach the two ops (the prelude keeps
`__host` private). Guest SHA256 3f6420d1446b5f9bb6604cf79b6f7574d2f3a8c327a2239461fa59b3fb160de9.

| Page (5 runs, median ms) | master | 1st visit | 2nd | 3rd |
|---|---|---|---|---|
| Wikipedia Microkernel | 544 | 243 | 241 | 122 |
| GitHub xirtus/sexos | 2232 | 1225 | 864 | 835 |

Wikipedia's second visit writes its classic scripts (no saving yet); the third reads
them. The cache lives for the browser session only; keeping it on disk is next.
