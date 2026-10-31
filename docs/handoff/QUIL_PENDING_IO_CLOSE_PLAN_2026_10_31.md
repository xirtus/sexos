# Next lifecycle proof: confirmed Quil Close during device I/O

This plan has now been [implemented and qualified at both desktop sizes](PENDING_DEVICE_IO_CLOSE_2026_10_31.md)
on one exact normal-image cohort, including independent cold recovery. The
procedural design below is retained. The earlier sustained
alpha workflow proves an ordinary pending-save decline followed by a confirmed
Close after save completion. Application `pending=1` alone would still not prove
that device I/O remained outstanding: a completed reply could be in transit.

Keep the existing long sessions and frozen cohorts untouched. Implement a
separate short normal-image fixture using ordinary BOOT-held document routes,
real keyboard input and the existing dirty-close guard. No guest proof-only
branch, new authority or runtime capability mint is needed.

## Preparation and limits

Quil and `document::MAX` allow 12,288 logical text bytes. Prepare and successfully
save 12,286 ASCII bytes, append a distinct saved edit `q` to form a 12,287-byte
immutable snapshot, then start Ctrl+S and append unsaved `z`. The final editor
buffer is exactly 12,288 bytes. Never attempt to load 65,536 text bytes.

Build the baseline through real Quil input: New, type six `x` characters,
then eleven completed Select All / Copy / EOF / Paste doublings to 12,288
bytes, followed by two Backspaces and Save As `pendclose`. Await actual
clipboard publication/paste and durable-save receipts, then reopen through
Linen. Spindle `filldoc`/`filldocx` are unsuitable: they write binary patterns
without the QP01/QJ02 document wrapper, and Quil rejects their control/UTF-8 data.

The larger number is physical journal storage: two 32,768-byte banks in a
65,536-byte container, each with a 512-byte header and a payload bounded by
`document::MAX`. A successful ordinary warm save grows that backing container.
Resolve its stable extents from the real prepared disk before the critical
phase. See `servers/sexfiles/src/document_journal.rs` and `vfs.rs`.

Boot/setup remain unthrottled. Installed QEMU 11.0.2 supports runtime
`block_set_io_throttle`; use the actual backend identity reported by
`query-block`. The current runner names that backend `nvm`:

```json
{"execute":"block_set_io_throttle","arguments":{"device":"nvm","bps":0,"bps_rd":0,"bps_wr":0,"iops":0,"iops_rd":0,"iops_wr":2,"iops_size":0}}
```

The command's `device` alias is deprecated; a verified guest-device QOM `id`
can replace it. Keep reads/total limits unlimited and omit bursts. Two writes
per second should delay subsequent commands roughly 400–500 ms, while each
command remains inside sexdrive's 1,000-tick completion budget. One IOPS or low
byte-rate limits risk a controller timeout. Guest ticks are not a host-time
guarantee; fail on timeout/quarantine and preserve the attempt.

Bound the armed interval to eight host seconds. Restore all throttle limits to
zero and disable tracing in `finally`, including every exception path. Bound
trace size, cleanup time and trial count. Each trial gets a fresh output and
owned guest; an observation miss remains diagnostic.

## Critical input and evidence

Require the actual immutable save admission, newer unsaved edit and ordinary
Close decline with `dirty=1 pending=1`. Wait for the actual guard frame, take
one screenshot and retain its exact pre-confirm serial prefix. Immediately
send the second F11; verify the retained screenshot afterward. The existing
full pixel-verification loop before confirmation consumes the pending window.

Arm these installed QEMU trace events only for the short critical phase:

- `serial_write(addr,value)`: reconstruct UART data-register bytes at address 0.
- `pci_nvme_io_cmd(cid,nsid,sqid,opcode,opname)`: identify WRITE opcode 1,
  namespace 1, submission queue 1.
- `pci_nvme_write(cid,verb,nsid,nlb,count,lba)`: bind the target write to the
  prepared document's journal extents.
- `pci_nvme_enqueue_req_completion(cid,cqid,dw0,dw1,status)`: identify the same
  command's completion on queue 1.

Use one timestamped trace stream and require this order:

1. Target document WRITE submission and its exact CID/LBA.
2. Reconstructed `quil.document.close pending=1` for the selected incarnation.
3. Preferably the exact reconstructed `task.finish pd=9 task=9`.
4. Matching WRITE completion enqueue, with no ambiguous CID reuse.

The implemented oracle also requires matching completion before the first exact
pool-ready UART character. Then require exact-incarnation cleanup, private-frame zeroing,
lease release and BOOT-reserved successor checks. Do not manufacture a draw
receipt or treat a pending application flag as a device-completion observation.

## Recovery oracle

After the owned guest exits, preserve/hash the terminal disk and cold-boot the
same image. Recovery may return the complete 12,286-byte baseline or complete
12,287-byte immutable snapshot, depending on completed effects. Validate object
identity, generation and checksums. Reject unsaved `z`, partial payloads,
malformed banks and stale-incarnation delivery. Cancellation does not promise
rollback of already completed storage effects.

Retain the QMP command/results, exact source/image/trace bindings, UART parser,
raw device trace, captured prefix/PPM, cleanup receipts and cold-recovery bytes.
Replay must reconstruct these facts independently. This fixture addresses one
pending-I/O lifetime edge; physical power loss, global epoch replay and general
daily-driver acceptance remain separate release gates.

Primary references: [QMP block throttling](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html#command-block_set_io_throttle)
and [QEMU tracing](https://www.qemu.org/docs/master/devel/tracing.html).
