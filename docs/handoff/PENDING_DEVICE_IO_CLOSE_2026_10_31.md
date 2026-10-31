# Native pending device-I/O Close and cold recovery

The normal optimized image now has a qualified virtual-device lifetime test
at both desktop sizes. A journal WRITE remains uncompleted through confirmed
Quil Close and exact task-9 finish. The same command completes before the first
private-pool-ready UART character. Five-role cleanup, successor recovery and
an independent cold boot then pass. This supplies the device-I/O edge that
the sustained workflows' completed-save Close did not observe.

No guest code, ABI, capability grant or architecture changes. The 574 production
inputs, ISO and kernel are identical to the qualified civil-clock repaint image.
The new runner cohort contains 575 separately bound checker/test inputs.

## Actual input and device ordering

Quil builds the baseline through normal input: six ASCII characters, eleven
completed clipboard doublings, two Backspaces, Save As and Linen reopen.
The baseline is 12,286 bytes; a distinct `q` edit makes the immutable save
snapshot 12,287 bytes. Unsaved `z` brings the editor to its real 12,288-byte
limit. The backing journal is a separate 65,536-byte two-bank container.

The ordinary Close is declined with an admitted save pending. The fixture waits
for a unique outstanding WRITE within the prepared journal's actual extents,
takes one screenshot, and immediately confirms Close. It verifies those retained
guard pixels afterward. A separate QMP connection restores a two-writes/second
limit; an independent absolute eight-second fallback can kill only the owned
guest even if QMP or the controlling thread stalls. Neither final run fires it.

The installed QEMU 11.0.2 log trace supplies one continuous stream containing
UART bytes, I/O commands, WRITE geometry and completion events. Reconstruction
requires complete exact Close/finish/pool records, command identity, namespace
and queue identity, valid block counts, journal extents and successful completion.
The live provisional observation never substitutes for final evidence.

| Profile | WRITE CID | Task-finish final event | WRITE completion event | Pool-ready first event | Throttle duration |
| --- | ---: | ---: | ---: | ---: | ---: |
| Compact 1024×768/e1000 | 1481 | 21985 | 32385 | 33221 | 1.328 s |
| Wide 1280×800/e1000e | 1481 | 21233 | 30600 | 32123 | 1.045 s |

Indices belong to each separate trace. Host timestamps are retained, never
sorted into a new event order. The readiness receipt reports 286,720 bytes
zeroed and reused by SEXT in each profile. Source-bound retirement checks retain
the five service ACK prerequisite and exact-incarnation lease/resource cleanup.
This is an observed readiness/reuse fence, not a whole-machine memory-erasure
or hostile-code-containment proof.

## Recovery and verification

Both terminal disks retain the complete 12,286-byte baseline, generation 1,
bank 1, identity 393217 and hash `0xce4d8088bbf0afbd`. A BOOT-reserved successor
loads it through ordinary Linen input before retirement. A separate normal-image
cold boot then starts from the exact terminal disk bytes and reopens it in fresh
Quil 9. The provider recovery receipt, native load, final disk metadata and full
Quil/civil-clock pixels all agree. Cancellation permits an old or fully committed
new immutable snapshot; it does not promise rollback. Unsaved `z` and partial
payloads are rejected.

The final F cohort passes **four native boots / 26 verified captures**, fresh
independent replay, all 42 registered host/static checks with 2,630 passing Rust
executions, and all **964 Python tests**. Parser/controller/cold-recovery tests
include malformed or reused command IDs, wrong extents, early/missing completion
or pool receipts, snapshot races, row-ID confusion and metadata changes despite
identical payload bytes. Read-only checks launch no guest.

[Machine evidence](assets/sexos-pending-device-io-close-2026-10-31-evidence.json)
links the [raw archive](assets/sexos-pending-device-io-close-2026-10-31-raw.tar.gz).
It retains exact sources/image/checkers, device/UART traces and byte mappings,
QMP profiles, immutable PPM/prefixes, seed/terminal disks, cold recovery, host
logs and fresh replay. Every member is size/hash verified. Original prototype
cohorts remain separately named; their boots never add to the final count.

- ISO: `9ad6768dcc70a0554999b5b2f57ceebcc0cd66e32500c1ceee79deb1e9ff8aca`.
- Kernel: `7f2bb264bce9bf74945fd0a7df07efd338ad8c437e68859a5bdeb738297a5740`.

Earlier attempts exposed checker defects: the installed timestamp format,
missing survivor input, incomplete cold transaction/metadata joins and a
two-read serial snapshot race. Their original evidence is retained. The final
cohort includes all corrections and is qualified independently.

## Continuing the OS goal

The corrected compact sustained workflow has passed separately; its optimized
wide repeat remains live. BOOT-catalog reserve expansion is being developed
independently in the primary checkout. Integrating that committed work requires
fresh source/image qualification. Storage torn-write/power-loss testing and the
broader physical/reserve/epoch plan remain separate work.

Inventory remains **2 Open / 28 Partial / System Proven false**. GLOBAL_VAS is
addressing only; BOOT sanctions capability genesis, sex-pdx supplies runtime
authority and PKU enforces it. This remains the SexOS SASOS design.
