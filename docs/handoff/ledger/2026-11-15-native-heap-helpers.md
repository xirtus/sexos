# Native object and string helpers: bounded coverage, no speedup claim

Separate from PR274's interpreter caches and stacked above the selector work.
The opt-in baseline tier recognizes complete own-property getter bodies and
complete two-string concat/comparison bodies. It uses three per-VM native
x86-64 templates after 32 admitted calls per operation kind. These are template
counts, not three arbitrary JavaScript functions compiled. General functions
containing object/string operations remain unsupported.

Own-property loads reuse QuickJS's weak shape/atom/slot cache. C checks normal
non-exotic objects and current slot bounds. Native code checks shape identity,
current atom and ordinary data flags before loading the current JSValue. QuickJS
duplicates the result reference. Prototype/accessor/proxy cases fall back.
String templates read actual validated guest ranges, support Latin-1 and UTF-16
(including lone surrogates and NUL), and compare at most 256 units per operand.
Concat writes at most 256 UTF-16 units to guest stack scratch; QuickJS allocates
the returned string and owns all references. Oversized/mixed-type operations
fall back before effects. Concat output/input overlap is refused. Host offsets,
widths and output spans are checked before a template executes. Scratch is not a
persistent pointer, and the synchronous native call cannot run JS or grow memory.

Existing CodeBuf ownership and RW-to-RX sealing are reused; executable pages are
per VM/domain, not global. Feature remains default-off. Protocol 4 negotiates the
new helper import, retaining the scalar jit_v3 payload protocol. Assembly source
and regeneration script accompany the relocation-free binary templates.

## Verification so far

Host fallback differential test passed for string helper outputs. Invalid ranges,
widths, oversized lengths and overlapping output tests passed, including no
writes/no compilation on rejected descriptors. Native x86 guest off/on gate
passed with identical outputs for mutable property values, retained object
references, deletion, prototype fallback, accessors, proxies, Unicode comparisons
and concatenation. It executed 470 native property loads, 993 comparisons and
225 concatenations; off counters were all zero. Three heap templates and 12
scalar functions were sealed/executed. Existing integer overflow/bailout outputs
matched. The browser gate checks actual per-domain kernel seal receipts.

That gate overlapped another workspace's VM; its timing is not performance proof.
Broad exact-head CI, Linux native guard tests and 1/4-core isolation/W^X suites
remain pending. No native helper merge is authorized by these narrow passes alone.
YouTube visible decoded playback remains an x17 check.

## Genuine-page observations

Both profile-only site checks passed on the same Wikipedia Microkernel and
GitHub xirtus/sexos URLs used for the selector candidate. These checks omit the
auxiliary webmail flow and are not full integration gates.

| Page | Previous selector VM elapsed | Native helper VM elapsed | Native heap executions |
|---|---:|---:|---|
| Wikipedia | 8.267 s | 8.299 s | 0 |
| GitHub | 29.812 s | 29.619 s | 203 property loads; 0 string operations |

These are live, unfrozen page inputs with asynchronous workload differences,
1 ms clocks and possible host contention (another workspace VM was observed
during GitHub). No speedup is established. Wikipedia host/JIT elapsed were
2.494/0.382 s; GitHub 1.061/0.673 s. Most VM time remains outside the measured
host calls. The helper shapes are too narrow for broad real-page coverage.
A profile-only bounded resident bytecode counter now records the most called
non-scalar rejected bodies among 64 retained entries; it is not CPU attribution
and may omit later bodies. It is intended to choose the next useful lowering.

Evidence under target/browser-evidence: native-heap-gate, wiki-native-heap,
github-native-heap, native-heap-receipt.json. Tested browser SHA256
 dec9a2f807a839539ef12c3632ed18d60bfdf9205a3c066a9e3b357cb74866dc;
guest SHA256 09eac8f7d4992bd0e342ae7a708b9796237eef9ee5b3fd6219233987aa633afa.
Frozen ISO c6eaec57eb55cbbfd4b9bd1afc20caa8b6531d342e73ba8629ad89040443356c.
The later diagnostic build adds resident bytecode counts and checked descriptor
end arithmetic; earlier runtime evidence is not relabeled as that exact build.

The detailed Wikipedia run retained a 16-byte predicate called 1208 times:
`o != null && o === o.<field>`. A subsequent lowering recognizes its exact
short-circuit bytecode and uses the same guarded property template for normal
object inputs. Null, primitive, missing-own-property and accessor cases retain
fallback. The loaded value is borrowed for object identity comparison, so it
needs no new reference; ordinary getters still duplicate their returned value.
Host differential checks passed after this addition. Its separate native gate
and real-page rerun are tracked as native-heap-identity-gate and
wiki-native-heap-identity; they were pending when this source checkpoint was
written. Earlier helper results above do not establish this pattern's execution.
