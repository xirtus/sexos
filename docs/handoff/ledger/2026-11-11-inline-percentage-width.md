# Percentage widths of inline blocks and buttons

Atomic inline preparation used `width.resolve(0)` despite receiving the actual
containing-block width. A 50% inline block inside a 360px container collapsed to
zero; a percentage button fell back to its text width. Resolve against `cw`,
keeping existing content/border-box handling and auto intrinsic sizing.

Three focused tests fail on master d5420e0e (observed widths 0, 38, 0 instead of
180, 180, 90) and pass after the change. They cover an inline block, a bordered
clickable button, and nested inline percentages. Candidate targeted run took
0.47 seconds including build; broad regression belongs to GitHub CI.

A genuine browser snapshot had a central clickable control of zero width, which
motivated inspecting this path. Its exact computed width is not yet captured;
this generic bug/fix does not establish that site's remaining cause, playback
or decoded media. No kernel, JavaScript, budget or network-policy changes.
Evidence: sexos-evidence/jit-forward-boolean/inline-percent-before.log and
inline-percent-after.log. All local processes exited before the next UI build.
