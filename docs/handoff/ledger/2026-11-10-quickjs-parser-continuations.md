# QuickJS bounded parser continuations: proposal and evidence

Status: continuation implementation and host differential pass; native/coherent
HTTPS qualification pending. No playback acceptance.

The genuine Google YouTube demo now creates a 720×405 iframe and sends an actual
`onReady` API payload. It then reaches a separate existing browser failure:
`native trap 10`. This trap also appears in the earlier `bcdb`, `44f` and final
`fb0` HTTPS receipts. Earlier absence of reported JavaScript exceptions did not
establish trap-free execution. API handshake does not establish video playback.

Private combined source `e8aadd302eae4d22261b0172b48ad8f79790adfa`
(150 `fb0` + viewport 152 `b5e` + percentage 153 `8cd`) showed real child DOM growth
to 397 nodes/194 connected, then the poisoned engine's existing empty-document
fallback. Active job counters reported zero 10,000-job cap hits. The iframe box
and viewport stayed 720×405. This was separately pinned diagnostic-program and
ABI-identical `fb0` boot provenance, not a coherent release-image claim.

Private prologue source `fd0569162e78c1a3aaae1c035ffb6897e871b579`
retained the existing 1024 logical-Wasm-call guard. Its genuine 90.7-second replay
recorded exhaustion at depth 1025, entry 0, high-water 1025, normal-return imbalance 0.
The first frames entered `sx_eval`, `__JS_EvalInternal`, runtime calls, then later
the last 64 frames contained 39 binary-expression parser calls, 8 logical-expression
calls, 5 unary calls, 5 postfix calls and the final token read. The omitted middle
frames were not logged; these counts are the retained tail, not the entire stack.
Native frame-distance 158,736 bytes was measured with frame-address instrumentation
and is diagnostic only. Actual copied WABT header/build configuration maps trap 10
to EXHAUSTION, depth counting enabled, guard 1024 unchanged.

External local evidence:

- `/Users/xmacbookm5/Projects/sexos-evidence/embed-combined-readonly-e8/bounded-analysis.json`;
  serial SHA256 `81acae2be581e1dc543397ddbeb3b1ac28fdd305ee4b8f5e0c99d7cb7239ed57`.
- `/Users/xmacbookm5/Projects/sexos-evidence/embed-native-depth-fd05/bounded-analysis.json`;
  serial SHA256 `0fd64b2e72cc0ec69ce8689945cdfa2dcc7c69ad637f3d61a81919ec7f87414f`.
- Diagnostic Web SHA256 `beaa5182e5bafaabfdaece449f4772b8ce425c2e150ae56f8344e99ead898f49`;
  unchanged Wasm `273036b267ab359e04df0ca000f14c57141f0d391991917f739ce8ec93ecf273`.
- Fresh public TH resource capture SHA256
  `9ff04782d6d33f797f7dff35d1bbbf2ba94be706ff64addaa1124c21593c4e4b`, 62839 bytes.
  Its matching guest length is not proof of guest-response byte identity. Keep the
  downloaded script outside the repository and use its hash as an external fixture.

## Proposed source ownership and invariants

Start from current master `39a862eb3a75757fbb69f973a60d0a767262402c` in isolated
`codex/quickjs-parser-continuations`. No kernel, public ABI, native runtime guard,
Promise provenance, scheduler, instruction/time budgets, website-source replacement
or header-spoofing changes.

Add one narrowly applied QuickJS patch and integrate it in
`scripts/build_quickjs_wasm.sh`. Only `js_parse_expr_binary` changes: its existing
bounded precedence 0..8 recursion becomes an explicit 9-slot continuation array.
Retain token-consumption/source-location/opcode-emission order, left associativity,
private-brand `#x in` behavior and atom cleanup on every failed parse. Unary,
exponentiation, short-circuit and assignment parsers retain their own behavior.
Nested expressions still consume bounded native/linear stack and the existing
1024 runtime guard remains active. This removes fixed precedence-ladder frames
without increasing a resource budget.

Add a host qualification gate and fixtures. Compile pinned untouched and patched
QuickJS tooling; compare serialized bytecode byte-for-byte under identical source
names/options, actual results/exceptions and coercion side-effect order. Cover all
precedence levels, associativity, private brands, `in`/no-in, syntax failures,
deep syntax and bounded runtime recursion. Run the full DOM suite and native/JIT
gates. Preserve failing controls as well as passing results.

Final genuine HTTPS qualification needs a fresh coherent current-source image
including viewport 152 and slim integration. Private ABI-matched historical boot
pairs remain diagnostic evidence only. Do not claim player/media acceptance from
an API payload, resource length, parser success or a frame box.

## Initial implementation evidence

The explicit continuation patch is applied after the existing import/JIT/Promise
patches. Pinned wasi-sdk 25 regeneration produced Wasm SHA256
`cfe274380d954d585f36a03f1c030d586410a3e72cb05a4a2a70792fd5bfb62f`.
No native runtime guard, stack/time/instruction budget or kernel change is included.

Paired pinned upstream tools, compiled sequentially with identical Apple Clang
inputs, passed 21 fixtures: serialized raw bytecode is identical, and actual
results/exceptions/coercion logs agree. One fixture generates 128 deterministic
typed/coercive expressions directly into emitted syntax. Others cover private
brands, failed private RHS cleanup, no-in, associativity, all binary precedence
levels, 32-deep syntax, bounded runtime recursion and the separately hashed fresh
public TH capture's initial compilation. The TH fixture is external evidence and
its evaluation without a browser is not an embed/playback claim.

Proof: `/Users/xmacbookm5/Projects/sexos-evidence/parser-host-qualified/result.json`.
The first harness invocation used obsolete `qjsc -c` and failed as a CLI error;
the pinned tool uses `-b -C` for raw classic-script bytecode. That failed run is
preserved at `parser-host-first`. Later harness-only fixes reused its two compiled
tools only after every archived source file matched, and retain their build logs.
The initial precedence fixture also exposed its own JSON BigInt serialization
exception; it now converts that result explicitly and requires successful runtime
completion for every ordinary fixture. Earlier matching-error evidence remains.

On the ARM host, `SEXJS_NATIVE=0 cargo test --locked --release -p sex-js --features
baseline-jit -- --test-threads=1 --nocapture` passes all 3 compiler unit tests,
all 63 current DOM tests and all 3 new parser tests. Native x86 generated-code
execution remains a distinct CI/native qualification.

The 96-deep arithmetic fixture fails on deliberately substituted production
`fb0` Wasm `dbdb0f6c…` with `Trapped("call stack exhausted")` in 0.12 seconds.
Immediately restoring candidate `cfe27438…` makes it return 7. The candidate also
passes repeated failing nested private RHS followed by valid evaluation, and
retains bounded runtime recursion. Negative and positive logs:
`/tmp/sexos-parser-old-wasm-negative.log`, `/tmp/sexos-parser-full-wasmi.log`;
candidate/control inputs `/Users/xmacbookm5/Projects/sexos-evidence/parser-wasm-controls/`.
No old binary remains in the branch. Coherent current-source native and genuine
HTTPS acceptance remains pending.
