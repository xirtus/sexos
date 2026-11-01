# Round 159: Close with an allocated Browser TLS future

The new `allocated-close` evidence mode keeps the local TLS future live through
F11 for every BOOT-reserved Browser, 15 → 30 → 31. Quiesced mode retains its Q-
then-Close behavior. Explicit schema2 and mode fields must agree in the requested
command, manifest, result and replay; old cleared evidence cannot be relabeled
as allocated teardown.

Each allocated phase requires fresh preparation, positively Applied complete
HelloPending pixels and a live nonzero resource snapshot. There can be no Q,
cancellation, timeout, transfer, earlier erase or reprepare before Close. The
exact held-incarnation request must lead to nonzero payload erasure, `pending=1`,
provider/clipboard closure, a zero heap/payload receipt and task finish. All five
service ACKs, private retirement and zeroed pool readiness must precede the next
sanctioned activation. `pending=1` alone is insufficient: it can describe a phase
before the future allocation exists.

Accounting stays explicit. Preparation reports task bytes +8,248 private bytes;
8,192 are the two queue arrays and56 are Pipe metadata. Owned payload erasure
counts task bytes +8,192. It does not include Pipe metadata, allocator/Rc/RefCell
overhead, registers, caches or stack forensics. Native runs observed task13,064,
private21,312 and erased owned payload21,256 bytes per allocated Close.

Actual F11 down/up packets, input make-before-release edges, raw serial cut hashes
and the exact typed Close request are joined per incarnation. A request that
predates its referenced key cannot pass. Host `time.monotonic_ns` observations
must be ordered integers from the same run; replay derives rather than trusts
summary intervals. The recorded ranges were5.57–35.44ms key-to-request and
403.34–1,034.67ms key-to-pool. These include QMP, serial transport and polling;
they are host observations, not guest latency or hard realtime guarantees.

No production cleanup code changed. The scoped source audit found the existing
future destructor and queue erasure synchronous before the terminal reports.
All582 production-source blobs and the exact kernel match round158. Original
30,000-tick deadlines remain unchanged; expiry before Close fails diagnostically.
Seed RELEASE already precedes preparation, so this proves allocated local-future
teardown, not pending seed/network/device I/O cancellation or lease expiry.

## Evidence

Final source: `c78007be91dc6d4d4e4c64348278bfea40e09ac7`.
ABI remains66 inputs/hash
`12d218c65235f1600843213a8cdce2cb5f93c0476db09080903171099fe0720f`.
ISO: `1a83f932de5dfbd21fb2a8669555b4f47047be554454d4dd2709e2a32a90421c`.
Kernel: `93c692bc680a7231355f836520435caba97b8e40749416b151587c15a09252b3`.
Architecture remains unchanged,
`4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`.

- Five final boots pass: one sealed35-second boot and four mode/profile boots
  (allocated/quiesced × compact e1000/wide e1000e). All four fresh read-only
  replays pass, with40 qualified captures:36 TLS panels and4 launcher captures.
  Six allocated Closes have exact payload, pending1, zero-resource and retirement
  evidence. Both modes retain source inventory/global stream checks, startup
  Echo PCAP, per-incarnation resize/restore pixels and finite exhaustion.
- All43 frozen host checks pass without source drift:3,077 logged Rust executions
  plus27 nested =3,104. All1,109 frozen Python cases pass. Counterexamples cover
  preallocation, prior clear/timeout, wrong accounting, crossed or stale resources,
  early reuse/activation, old schema/mode substitution, reversed F11 edges,
  preexisting requests, invalid timestamps and changed raw prefixes.
- All1,209 frozen source blobs (582 production +627 runner,11,944,327 bytes) match
  pinned Git content and manifests. Nine ISO ELFs contain331,077 decoded
  instructions with zero FP/vector scan matches; static scanning does not prove
  runtime register/cache erasure.

[Raw evidence archive](assets/sexos-browser-allocated-close-2026-11-01-raw.tar.gz):
10,544,629 bytes, SHA-256
`0f9890dc84ed33301a02866fd40d945a94f91f7995a91ff1972a70bafb6159aa`.
All1,430 members were reopened and checked against per-file size/SHA records.
The archive includes the exact production-equivalence comparison, raw mode
captures/actions/timings, terminal compressed disks, source/image and replay.
The first attempt is separately UNQUALIFIED: its checker missed an atomic
resource record that interrupted an older non-atomic serial print. The corrected
parser permits that prefix placement while requiring a complete exact payload;
it rejects malformed and incomplete records.

[Round158 desktop evidence](BROWSER_TLS_PANELS_2026_11_01.md) keeps its original
image and binding. This round does not relabel it as a new desktop run or claim
current sustained daily-driver acceptance. Inventory remains **2 Open /28 Partial /
System Proven false**. GLOBAL_VAS, BOOT-only authority genesis, typed sex-pdx and
PKU remain the design; this adds no Unix process or runtime authority model.

## Next

160: replace the BOOT main-inventory’s hardcoded32 limit with one explicitly
bounded64-entry capacity. Keep the actual app catalog unchanged, reject overflow
and postseal registration, and preserve exact pointer/cancellation/wake ownership.
Qualify capacity before adding reserves in a separate round; raising a number
alone is not runtime task-admission evidence. Then address reserve image/stack/
reply-credit budgets and run a sustained unified desktop/cold-recovery session.
