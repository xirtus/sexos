# Place fixed descendants discovered inside absolute subtrees

Real HTTPS embed diagnostics on private browser `10fdf87d` found a concrete
layout omission: the absolute body and player both had 720×405 boxes, but its
fixed `player-controls` descendant had no box. Initial/classic evaluations all
returned successfully (17 child evaluations); no engine trap or playback was
observed. The child had three painted rectangles and no native media frames.
The private observation is retained separately in
`sexos-evidence/private-live-embed/real-https-10fdf`; it is not release evidence.

The root drained the deferred-positioning list once before placing the absolute
body. Fixed descendants discovered during that placement were bubbled back to
the root after its drain and never processed. The root now incorporates these
newly discovered descendants into its work stack before continuing to the next
sibling, preserving tree paint order. Each loop is bounded by the number of DOM
nodes. Non-root containing-block selection, viewport sizing, overflow handling,
positioned layer ordering and existing placement routines remain in use.

Three focused tests fail on unchanged master `75e4811c` and pass with this fix:
fixed controls under an absolute/overflow-hidden body use the complete viewport;
newly discovered fixed descendants paint before later siblings; and nested
fixed versus absolute descendants use their respective viewport/container boxes.
Before/after logs are retained under `sexos-evidence/private-live-embed` as
`fixed-descendants-before.log` and `fixed-descendants-after.log`.

This restores missing browser UI boxes; it does not establish real player UI,
decoded media or YouTube playback. Real HTTPS replay and CI remain pending at
commit time. No kernel, guest engine, source policy, request headers, guard,
budget, executable pages or JIT permissions change.
