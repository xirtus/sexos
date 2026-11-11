# Bounded row candidate selection

Parent cc44566cc302cad42deb26aece7c46341266dfe4 passed native 1x/2x pixels
but still FAILS the matched three-pair performance gate against baseline763:
p95 ratio 1.128740112602617, CPU ratio 1.0792389604303416. Evidence remains
sealed in nativecc445/perf-ab/latency.json. Reported host load 6–9 versus prior
2–3 is context, not permission to relax the gate or claim a pass.

row_winners previously repeated authenticated focus normalization, group lookup,
rectangle/rounded coverage and canonical key calculation for every surface at
every row interval. It now fills at most MAX_SURFACES index/span/key candidates
once per row in existing Scene-owned scratch, selects by the unchanged canonical
key among containing spans, and resolves content through the unchanged binding.
Candidate iteration preserves slice order for ties. An explicit initialized count
excludes stale entries; clear also erases storage. No retained references, heap,
by-value scratch return or new local array. The bounded scratch size assertion
remains 12KiB; RowWinners remains at most 64 bytes. No kernel change.

The scalar winner/AA lower-owner traversal and busy-scratch fallback remain
unchanged. New actual row-versus-scalar controls cover scales 1–3, reversed slot
order, every surface focus including the authenticated bound child and an inactive
surface, clipped top/bar/bottom corners, overlay priority and exact content/chrome
selection. Existing independent shape/AA pixel controls remain authoritative.

Validation: git diff --check passes. Compilation, host tests, rebuilt-ELF stack
review, native and matched performance gates PENDING (recorder owns heavy slot).
No performance recovery or 64KiB stack safety claim follows from this source patch.
