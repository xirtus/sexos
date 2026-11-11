# Private real-page boolean JIT and fixed controls proof

Private source f0458e1f combines read-only diagnostics, layout PR162 and JIT
PR165. Native Web SHA e293f463bcf390ca51e830f1a596e5caef9128a39b4bc7a708d663900c5e97b8
was loaded into frozen production157 ISO
1b825ecdd030fe64fcad38a946f773037d37ba45da23778725f31dc48bb0bee9.
This is not a new all-source image. Both inputs remained unchanged during
181.3 seconds including the final native diagnostic chord.

Ordinary HTTPS Google developer embed, without header overrides, produced one
actual task42 code seal. Child owner6/frame972 compiled one cache key and entered
native code 51 times, with zero bailouts or seal refusals. Entry20's 26-byte
SHA40e164a0df31773df710365abc4fb3dec0784127f044d218be3ee97de1ec69b3
matches the previously observed uncompiled function exactly. Its 82 admitted
calls are not a performance comparison with the earlier network run's 1,109.
Parent JIT counters remain zero. No whole-page speed claim.

The fixed controls container now has actual box [0,0,720,405]. Screenshot shows
YouTube title, channel and Watch on YouTube overlay where these were previously
missing. Playback remains unverified: both media nodes Empty, no source or
decoded frames. A central clickable node82 has [324,166,0,72], so zero width is
a better next lead than forcing the video wrapper's auto height.

Evidence: sexos-evidence/jit-forward-boolean/real-https-f045, including the raw
serial SHA4f69aebbb0e935a0c01a747a70a35063857ab44db06f5a7da2809331124cf599,
unchanged-input result, final PPM/PNG and bounded cache-decoded-v2.json.
Private generated C/object/header/Wasm/program hashes: private-f045-inputs.json.

The accompanying private-only selector change includes semantic buttons,
role=button and SVG in the existing bounded 48-node computed-style snapshot.
It does not invoke getters, evaluate JavaScript, recompute layout, change engine
budgets or prove the cause of node82's zero width. This selector change has not
yet been rebuilt or run; the evidence above belongs strictly to f0458e1f.
