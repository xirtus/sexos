# Native Promise rejection events

## Cause and behavior

A source-instrumented diagnostic of the real Google-hosted YouTube demo traced
`_.Iea` resolving before the inline `writeEmbed` call, while its initializer ran
later through a timer-based Promise polyfill. The captured SDK's actual selection
condition retained QuickJS's native Promise only when `PromiseRejectionEvent`
existed. This is a compatibility cause, not a playback acceptance result.

Implement the event together with QuickJS's actual host rejection tracker. Native
Promise remains unchanged. Constructor `promise` is a required WebIDL `object`,
including ordinary objects/functions; it is not restricted to a native Promise.
Promise/reason preserve identity and have branded read-only getters. Constructed
events are untrusted; engine notifications are trusted. Public dispatch removes
trust. Canceling `unhandledrejection` suppresses its default log.

A C-held, one-time private callback owns per-VM pending/outstanding state. After a
complete microtask checkpoint, notification is a separate private FIFO DOM task (page timer IDs cannot cancel it). Synchronous
and same-checkpoint catches suppress notification. A later catch queues
`rejectionhandled`; a catch inside notification creates no outstanding record.
Pending native jobs defer tasks, and a tracker-created task after a tick updates
the browser's next wakeup. Outstanding records use a weak map. Dropping the Script
VM on frame/document destruction discards its handler, queues, records and heap.
There is no Promise replacement, page-visible native callback, job cap increase,
source rewrite, networking policy relaxation or kernel change.

## Source privacy boundary

Private QuickJS bytecode metadata records the host's source policy. Promise
producers, settlement, adoption, async results and callback lineage inherit it.
Bound functions retain their target's policy; opaque async generators stay opaque.
The tracker never exposes a muted Promise or its reason to page callbacks/logs.

Known own-origin initial scripts and ordinary own-origin functions remain readable,
including delayed and async calls after an opaque script ran. The private dynamic
classic-script path may explicitly unmute a resource only after its existing CORS
validation. Initial foreign classic scripts and modules without a validated CORS
policy remain conservatively muted. Requested module URLs retain the fetched final
URL policy so redirects cannot erase provenance. The map is bounded at 128 entries;
unknown entries fail closed on overflow. Maps and metadata belong to one document VM.

Conservative limits: generated eval/Function code after an opaque source has entered
the VM, unknown proxy callbacks and transformed Promise lineages containing an
opaque producer/callback may stay muted even when their final rejection comes from
own-origin code. This deliberately under-reports ambiguous rejections rather than
exposing opaque reasons. It is not a complete HTML execution-context model.

## Qualification

- Pinned QuickJS-NG 3c9afc9943323ee9c7dbd123c0cd991448f4b6c2, WASI SDK 25 clang
  19.1.5; rebuilt checked-in Wasm SHA256
  `adbd4f365f8ce50342b616a6144685d333ae08c096f31a403763a9622ae118b6`.
- `cargo test -p sex-js`: all host tests pass, including 55 DOM tests. New cases
  cover dictionary conversion order/descriptors/brands, actual SDK native retention,
  rejected initializer ordering/identity, cancellation/default logging, late/same
  checkpoint catch, pending jobs/wakeup and private task cancellation, source muting (bound/proxy/eval/Function,
  async/async generator), redirects/map bounds and separate/replacement VM lifetimes.
- `scripts/program_web_promise_gate.py`: added to ordinary native runtime CI. Two
  genuine HTTP origins exercise native events, ordered initializers, own/opaque/CORS
  resources, frame identity and document navigation. Native receipt pending.
- Genuine HTTPS embed rerun pending. No YouTube readiness or playback claim.
- The previously proved finite 50k-job incomplete checkpoint is separate: its
  10,000-job cap is unchanged and the real embed had zero cap receipts.

Primary contracts: [HTML PromiseRejectionEvent and rejection notification](https://html.spec.whatwg.org/multipage/webappapis.html#the-promiserejectionevent-interface),
[WebIDL dictionary conversion](https://webidl.spec.whatwg.org/#es-dictionary).
