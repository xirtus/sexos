# Selector queries and direct element matching

The existing selector matcher already runs right to left. Document queries now
retain up to 64 parsed selectors (input at most 4096 bytes each), and Dom lazily
indexes id, class and tag candidates. Preorder intervals preserve subtree scope,
document order and duplicate IDs after reparenting, including detached trees.
The smallest rightmost-compound candidate list is filtered by the existing full
matcher. Results are never cached, so hover/focus and other live predicates remain
current. Index bounds are 32768 nodes, 131072 tokens and 2 MiB of input key bytes;
larger trees use the old scan. Mutation methods invalidate indexes; the parser's
direct attribute writes also invalidate. Direct users of the public node arena
must call `invalidate_queries()` as documented; browser mutation sites were audited.

Indexing alone did not remove the main cost. `Element.matches()` previously
queried its parent's entire subtree and sometimes the whole document, then
searched the result array. `closest()` repeated that for each ancestor. A new
host operation applies the existing matcher directly to the element. Detached
element matching now works, and `closest()` coerces its selector once.

Two indexed-versus-scan tests cover attributes, reordering, duplicate IDs, detached
roots, fresh hover/focus state, text/HTML replacement and bounded fallback.
Targeted JS tests cover cache eviction, static results, mutations, direct matches,
detached matches, closest coercion and invalid selectors. All passed. The profiled
native browser built and the genuine Wikipedia/GitHub profile-only runs passed.
Broad CI and a complete current-ISO integration run remain pending.

## Measurements, not a controlled speedup claim

Same Wikipedia URL and 31 physical Page Down inputs, opt-in instrumentation:

| Candidate | VM elapsed | Query elapsed | Query calls |
|---|---:|---:|---:|
| Original profile | 14.371 s | 5.113 s | 402 |
| Parsed cache + indexes | 14.828 s | 4.836 s | 402 |
| More detailed query attribution | 18.440 s | 6.290 s | 1003 |
| Direct element matching too | 8.267 s | 1.688 s | 578 |

The detailed run identified 128 queries for MediaWiki's long link-hover selector
as 4.470 seconds of query work. With direct matching, 288 MATCHES calls consumed
0.020 seconds. The visible fixture checks still found 81 text matches, 2291 paint
items, 778 links and document height 22112. Async workload counts changed, live
resource bytes were not frozen, clocks have 1 ms resolution and include preemption
and instrumentation overhead. These observations support the identified redundant
scans; they do not establish a controlled overall speedup ratio or a JIT speedup.

GitHub's separate profile-only check found 21 matches, 1476 items and 1017 links,
with no level-2 console errors observed. VM elapsed 29.812 s, host calls 0.909 s,
JIT admission/compilation/execution 0.619 s. No GitHub speedup is claimed.

`--profile-only` explicitly omits auxiliary webmail and reports
`full_site_gate: false`. The default site gate and its assertions are unchanged.
This avoids replaying the known frozen-Shell/current-harness palette mismatch;
it is not a replacement for full CI. All these VMs ended and were reaped.

Evidence: `target/browser-evidence/wiki-selector-profile`, `wiki-selector-detail`,
`wiki-selector-matches` and `github-selector-matches`; result/serial/input receipts
remain separate. Final browser SHA256
b36fb969f43c6526c8affff0dd98e1a72bb3926f32efce7992296b9723765c29.
Wikipedia final raw serial SHA256
166386e6213ae0b761bd80315e3ebe410acfe355a5805938c22b66b6f2f29eec.
The source patch receipt is supplemented by the new query module's hash because
that module was untracked when the pre-build patch was saved.

PR274 remains the separate interpreter-cache candidate. Native object/string JIT
lowering is still unfinished; visible YouTube playback remains an x17 check.
