# Intel Wi-Fi for SexOS (2026-11-09)

Thread "Bring up Intel Wi-Fi". Follows priority P2 of
`docs/handoff/DRIVERS_AND_X17_R1_PLAN_2026_11_09.md` (PR #80): Intel AX200, AX201,
AX210 and AX211 (and the Killer AX1650/AX1675/AX1690 modules built on them), ported
from OpenBSD `iwx(4)` as a SexOS driver, WPA2 and WPA3, hooked into the network
service. The Alienware x17 R1 has a Killer AX1675 (AX210, PCI 8086:2725, Killer
subsystems 1673/1674), so this is the chip that matters for next week.

## 1. Short version

- **Done in this PR (stage W1): the whole hardware-independent driver core**, as a new
  `no_std`, allocation-free crate `crates/sex-wifi`, with 24 host tests:
  - chip table: every AX200/AX201/AX210/AX211 PCI id, the Killer subsystem table, and the
    AX201 revision rules, giving the firmware file each chip needs (the x17's AX1675x gets
    `iwlwifi-ty-a0-gf-a0-77.ucode` plus `iwlwifi-ty-a0-gf-a0.pnvm`);
  - firmware file parser (Intel's TLV `.ucode` format) and the split into LMAC, UMAC and
    paging sections; tested on the **real linux-firmware images** as well as synthetic ones;
  - the DMA layout and the "context info" tables the chip boots from (both the 22000-series
    layout and the AX210 "gen3" layout, byte for byte as OpenBSD writes them);
  - the power-up sequence (prepare, persistence bit, software reset, power gating for
    integrated parts, clock wait, RF-kill check, context info, image loader, LTR, CPU start,
    wait for ALIVE) as a step-at-a-time state machine, tested on a register model;
  - host command framing (wide headers, TFDs, write pointer) and RX packet parsing, ALIVE;
  - 802.11 frames: scan results (SSID, channel, security incl. WPA3 transition and
    hash-to-element), open and SAE authentication, association, deauth, and data frames
    to and from Ethernet II so sexnet's IP stack runs on Wi-Fi unchanged;
  - **WPA2-Personal**: passphrase hash, PTK, the station side of the 4-way and group-key
    handshakes, GTK and IGTK (protected management frames), replay and downgrade checks;
  - **WPA3-Personal (SAE)** on P-256: hunting-and-pecking *and* hash-to-element, commit,
    confirm, reflection check, then the 4-way handshake with AES-CMAC MICs and SHA-256 keys;
  - **CCMP** (AES-CCM) for data frames.
- **Proof:** IEEE 802.11 annex vectors (passphrase, PRF, CCMP M.6.4, SAE J.10 for both
  methods), RFC 3394 key wrap, RFC 3610 CCM, RFC 4493 CMAC; full WPA2 and WPA3 handshakes
  against an access point played by the test; wrong password, downgrade, tampering and
  replay all refused. `scripts/wifi_host_gate.sh`, now a CI step.
- **Not yet:** the device is not leased to sexnet and nothing in the running system calls
  this code. QEMU has no Intel Wi-Fi model, so the radio itself can only be proven on real
  hardware. Next stage (W2) makes the x17 say `[wifi.alive]` with the firmware version in
  its boot log, which is the first thing we can check next week.

## 2. Why a separate crate, and how it fits the design

The sex-manual puts drivers in user-space servers that get a device by capability and
never share an address space with the kernel. The Wi-Fi driver follows that:

- The **kernel** will only lease the PCI device (BAR0 view, command bits, one DMA region)
  to the network service, exactly like the Intel/Realtek wired NICs (PR #79). It knows
  nothing about Wi-Fi.
- The **network service** (`sexnet`) owns the lease, feeds this crate register reads and
  writes and DMA memory, and plugs the result into its Ethernet path. Wired and wireless
  then share one IP stack, DNS, TLS and socket layer.
- The **crate** holds everything that can be checked without hardware. It never touches
  memory it was not handed, never sleeps (every wait is a step the service calls again),
  and has no unbounded loops, which is the same discipline as `sexnet/src/nic.rs`.

Where this differs from Linux and from the manual's original plan, and why:

| Plan / Linux | Here | Why |
|---|---|---|
| Linux `iwlwifi` + `mac80211` + `wpa_supplicant` (three components, two in the kernel) | one Rust crate inside the network organ | GPL-2.0-only code can't be used (SexOS is GPL-3.0-only); OpenBSD's `iwx` is ISC and much smaller, and doing the supplicant in the same organ removes the netlink-style control protocol entirely |
| Manual: a driver server per device | Wi-Fi lives in the network service for now | the IP stack is there; a separate Wi-Fi server would copy every packet across one more boundary. When MSI-X and IOMMU domains exist (framework stages D3/D4) the Wi-Fi worker can move to its own PKU organ ("Network" organ, separate key) without changing the crate |
| Firmware loaded by the kernel (`request_firmware`) | the service reads the file itself from the firmware store | the kernel should not parse files; stage D8 (firmware store) gives a read-only capability to `/lib/firmware`-style blobs |
| Hardware crypto only | hardware crypto after the key is installed, software CCMP for the frames before that and as the reference | the same code checks the vectors and can drive any future chip without crypto offload |

## 3. Firmware

Intel's blobs are redistributable (`LICENCE.iwlwifi_firmware`) but not open. They are data
files: never linked, loaded by the driver at start. `scripts/fetch_wifi_firmware.sh`
downloads the 13 files any supported chip may ask for from linux-firmware, pinned to one
commit and checked by SHA-256 (about 15 MB in total; the x17 needs two files, 1.6 MB).
They are not committed to the repository. W2 puts the ones for the machine's chip on the
boot image / disk.

The driver speaks firmware API 77 (89 for the 0x7e40 "Ma" parts), the same as OpenBSD.

## 4. Stages

| Stage | What | Proof |
|---|---|---|
| **W1** (#83) | driver core crate: chips, firmware files, DMA layout, power-up, commands, 802.11, WPA2/WPA3, CCMP | host tests with IEEE/RFC vectors, real firmware images |
| **W2** (section 7) | kernel leases Intel Wi-Fi (class 02:80, ids from `sex_wifi::devices`) to sexnet with a 4 MiB DMA region; firmware files on the boot image; sexnet powers the chip up and waits for ALIVE; boot log `[wifi.chip]`, `[wifi.alive] fw=…` or the exact failure step | host gate on the lease; **x17 boot log next week** |
| **W3** (section 8) | command queue; platform NVM for the reported SKU; firmware init (INIT_COMPLETE); NVM read (MAC address, antennas, bands, channels); antenna, BT coexistence, SoC, DQA, thermal, power, regulatory (MCC), scan and beacon-filter configuration | host gate on a firmware model and the real x17 files; **x17: `[wifi.ready]`** |
| **W4** (section 9) | scanning: one UMAC scan of every allowed channel right after `[wifi.ready]`, beacons and probe responses read into a network list, `[wifi.network]` lines in the boot log | host gate on the firmware model; **x17 lists nearby networks** |
| **W4b** (section 10) | the network list for people: `wifi` and `wifi scan` in Spindle (a desktop query to sexnet, `sex_pdx::wifi_status`), rescans; the status bar indicator follows | host test of the words; **x17: `wifi` lists networks** |
| **W5a** (section 11) | join: PHY context, RLC, MAC context, binding, station, management TX queue, session protection (`iwx_auth`); `wifi join N` in Spindle | host gate on the firmware model and the real x17 file's command versions; **x17: `[wifi.joined]`** |
| **W5b** (section 12) | open-system auth and association frames on TX queue 1 (`iwx_tx`, TX command v10/gen3 or gen2), then ADD_STA update and MAC context with the AID (`iwx_run`) | host gate: TFD, TX command, byte counts and frames on the firmware model, retries and refusals; **x17: `[wifi.associated]`** |
| **W5c** (section 13) | WPA2: the 4-way handshake over EAPOL data frames on queue 1, pairwise and group keys installed (SEC_KEY_CMD on the x17, ADD_STA_KEY on older firmware); `wifi join N PASSPHRASE` | host gate: the AP's half of the handshake on the firmware model, key command bytes, wrong passphrase; **x17: `[wifi.connected]`** |
| W5d | WPA3: SAE commit/confirm in authentication frames, PMF | x17 joins a WPA3 network |
| **W5e** (section 14) | the connection as sexnet's Ethernet link: data frames out (firmware CCMP) and in (decrypt status, PN replay check), the IP stack moves to Wi-Fi while connected | host gate: frames both ways on the firmware model; **x17: `[sexnet.wifi.transport] up=1`, browser over Wi-Fi** |
| W6 | speed and comfort: HT/VHT/HE capabilities, aggregation, power save, roaming, rekeying, 6 GHz | throughput test against the wired NIC |
| W7 | MSI-X interrupts and an IOMMU domain (framework D3/D4); Wi-Fi worker in its own organ | fault injection: a wild DMA is stopped |
| W8 | Bluetooth (Intel USB Bluetooth, firmware over USB) | pair a mouse or headset |

## 5. Changes to name

- **No ABI change** in this PR (`sex-pdx` untouched, `abi_version_hash` unchanged).
- **Build:** the service build already compiles `aes` with `--cfg aes_force_soft` (sexos_build_spec.toml), which the crate needs on the `x86_64-sex` target.
- **New dependency:** `sha1` 0.10 (RustCrypto, MIT/Apache), needed by WPA2 key derivation.
  `aes`, `hmac`, `sha2`, `p256` were already in the lock file.

## 6. Sources and licences

- OpenBSD `sys/dev/pci/if_iwx.c`, `if_iwxreg.h`, `if_iwxvar.h` (ISC): register map, init
  order, device tables. Re-implemented in Rust, not copied.
- IEEE Std 802.11-2020 (frames, RSN, SAE, key hierarchy) and its test-vector annexes;
  hostap's `wlantest/test_vectors.c` and `src/common/common_module_tests.c` (BSD) for the
  vector values.
- linux-firmware `intel/iwlwifi/*` (Intel redistributable licence): firmware data only.
- No Linux kernel (GPL-2.0-only) code is used.

## 7. Stage W2: the chip starts its firmware (2026-11-10)

What the x17 R1 should print at its first boot, in order:

```
[hw.device] 3b:00.0 8086:2725 class=028000 Intel Wi-Fi 6E AX210 / Killer AX1675x service=sexnet support=in_progress
[devmgr.lease] 3b:00.0 8086:2725 service=sexnet pd=… slot=22 kind=Wifi nth=1
[boot.class.command] … kind=Wifi … dma_region=0x4400 dma_bytes=4194304
[boot.wifi.firmware.region] pd=… bytes=13795328
[wifi.chip] vendor=0x8086 device=0x2725 subsystem=…:1674 hw_rev=… rf_id=… bar_bytes=…
[wifi.chip.known] name="Killer Wi-Fi 6E AX1675x (AX210)" family=Ax210 firmware=iwlwifi-ty-a0-gf-a0-77.ucode integrated=0
[wifi.firmware] file=iwlwifi-ty-a0-gf-a0-77.ucode bytes=1594276 version=… api=77 …
[wifi.prepared] pool=4194304 used=… rx_pages=64 bus=…
[wifi.step] from=Ready to=Reset … (then Clock, Configure, NicLock, WaitAlive)
[wifi.alive] fw=… umac=… notifications=1 …
```

Any other ending is a `[wifi.failed] step=… reason=…` line naming the step, with the
interrupt cause, `GP_CNTRL` and the receive ring position, so the log alone says what to
fix. `step=Failed(RfKill)` means the radio switch (or Fn key) is off.

How it is wired:

- **Kernel (mechanism only, ~45 lines).** The NET manifest in `sex-hwdb` claims Intel
  class 02:80 as `Lease::Wifi` at slot 22. `device_lease::Kind::Wifi` is the class lease
  HD Audio and AHCI already use: BAR0 (at least 8 KiB of registers), memory and
  bus-master bits, and a DMA pool, here 4 MiB at its own slot 23 so the wired NIC keeps
  its pool at slot 32. The boot module `firmware/wifi.pack` becomes a read-only region at
  slot 24 of the service holding the chip; the kernel never parses it.
  `SYS_IDENTITY` gained `QUERY_SUBSYSTEM` (the module maker's ids; the x17's Killer module is subsystem device 1674).
- **Firmware pack.** `scripts/make_wifi_firmware_pack.py` packs the fetched files (name,
  offset, length, SHA-256 each; format in `sex_wifi::pack`). The build fetches the files
  if they are missing and packs whatever it has; an offline build gets an empty pack and
  the chip logs `reason=Missing`. The installed system carries the pack too (the boot
  partition image grows from 12 to 32 MiB, its full slot, when firmware is present).
- **Driver crate.** `sex_wifi::rx` (the receive ring for both families: free ring, used
  ring, status word, page recycling, the 0x1c80 write index) and `sex_wifi::bringup`
  (firmware into DMA, pages posted, the free ring made visible before the CPU starts,
  power-up, then the receive ring read until ALIVE).
- **Network service.** `servers/sexnet/src/wifi.rs` maps the lease, the pool and the pack
  onto the crate's traits and steps it from sexnet's main loop, beside the NIC.

Why the driver is still inside sexnet and not its own server (the guardrails' default):
a new server needs its own boot module, protection domain and PKU organ, and the radio's
packets would then cross one more boundary to reach the IP stack. Until MSI-X and IOMMU
domains land (W7), a separate server would add a copy without adding isolation the
hardware enforces. The crate does not depend on sexnet, so moving it later is a matter of
a new `Service` in the manifest.

Proof: `scripts/wifi_host_gate.sh` adds the receive ring on both families, ALIVE through
the ring (good and bad status), refusal of a page the driver never posted, the pack's
checksum and lookup, and the real pack starting the x17's chip layout in a 4 MiB pool.
QEMU has no Intel Wi-Fi, so the boot gates prove only that nothing else changed.

ABI: new `sex_pdx::pci_command` constants (`SLOT_WIFI`, `SLOT_WIFI_DMA`,
`SLOT_WIFI_FIRMWARE`, `WIFI_DMA_REGION_ID` 0x4400, `WIFI_FIRMWARE_REGION_ID` 0x4401,
`QUERY_SUBSYSTEM`), `sex_pdx::device_subsystem`, and `kernel/src/pci_command.rs` (DMA slot
per lease kind, subsystem query). `abi_version_hash` recomputed.

Next, W3: the command queue and the init commands (PNVM, PHY and antenna configuration,
regulatory domain, MAC context), ending in `[wifi.ready]`.

## 8. Stage W3: the firmware is told how to run (2026-11-10)

After `[wifi.alive]` the x17 R1 should go on to print:

```
[wifi.alive] fw=… umac=… notifications=1 … sku=0x….0x0.0x0
[wifi.pnvm] source=File sections=2 bytes=… version=… file=iwlwifi-ty-a0-gf-a0.pnvm
[wifi.init] step=WaitPnvm … (then ExtendedConfig, NvmAccessComplete, WaitInitComplete,
            NvmInfo, MacAddress, TxAntennas, BtCoex, SocConfig, Dqa, Thermal, Power, Mcc,
            ScanConfig, BeaconFilter)
[wifi.nvm] version=… sku_flags=… 2ghz=1 5ghz=1 ax=1 tx_chains=0x3 rx_chains=0x3 lar=1 channels=…
[wifi.ready] mac=…:…:…:…:…:… country=ZZ channels=… commands=11 ticks=…
```

Any other ending is `[wifi.failed] step=init reason=Failed(…)`: `Timeout { group, opcode }`
(no answer to that command within a second), `Refused` (the firmware's "failed" flag),
`Firmware(…)` (a REPLY_ERROR, with its error code and command), `Crashed { cause }` (the
software or hardware error interrupt), `NoNotification(PnvmLoaded | InitComplete)`,
`NoMacAddress` or `Unsupported(…)`.

What happens, in the order of iwx(4) (`iwx_load_pnvm`, `iwx_run_init_mvm_ucode`,
`iwx_init_hw`), as one bounded step per call:

1. **Platform NVM (AX210 family).** ALIVE v6/v7 carries a SKU id. The driver takes the
   chunk for that SKU and this radio (MAC type from `HW_REV`, RF type from `HW_RF_ID`)
   from the firmware file if it embeds one, else from the chip's `.pnvm` file in the boot
   pack, copies its sections into the DMA pool (one block, or a table of page-aligned
   segments when the firmware claims capability 32) and points the peripheral scratch at
   it. Then it rings the UMAC doorbell and waits for `PNVM_INIT_COMPLETE`. With no file
   or no matching chunk it rings anyway, so the firmware goes on without it, as iwx does.
   The x17's file has four SKUs, all for its radio (MAC 0x42, RF 0x10d).
2. **Firmware init.** `INIT_EXTENDED_CFG` (NVM access), `NVM_ACCESS_COMPLETE`, then wait
   for `INIT_COMPLETE`.
3. **NVM.** `NVM_GET_INFO` (v4, 468 bytes): bands, 802.11n/ac/ax, antennas,
   location-aware regulatory, the channels allowed. The MAC address is read from the
   fuses (strap, then OTP registers at 0x380) under the NIC lock, which is then held to
   the end, as `iwx_init_hw` holds it.
4. **Configuration.** TX antennas (firmware PHY config masked by the NVM), Bluetooth
   coexistence (Wi-Fi only until W8), SoC latency, DQA (only when the firmware claims it;
   the x17's does not), an empty thermal threshold list (thermal shutdown handed to the
   firmware), device power (no power saving yet), `MCC_UPDATE "ZZ"` when the NVM says the
   regulatory domain is location-aware (the answer names the country and channel count),
   the reduced scan configuration (v5 on the x17: no broadcast station id) and beacon
   filtering off.

One divergence from iwx: the SoC command's LTR delay is put in bits 2-3 (its field per
the command's definition). iwx masks the unshifted value, which always sends 0. It
matters only for integrated (CNVi) chips; the x17's AX210 card is discrete and sends
flag 1 with no delay either way.

Code: `sex_wifi::cmdq` (the command queue: buffers, TFDs, the write pointer that counts
past the ring on AX210, answer matching), `sex_wifi::init` (the state machine above),
`ucode::pnvm_select`/`pnvm_image`, `Layout::write_pnvm`, `hcmd` (ALIVE v6 SKU, REPLY_ERROR,
command ids), and `Bringup::receive`, which now hands every packet to a visitor and reads
only the first packet of a page on AX210 (the rest of the page is left over from earlier).

Proof: `scripts/wifi_host_gate.sh` (now 26 driver tests) runs the whole sequence against a
firmware model that reads the commands back out of the ring (order, payloads, the PNVM
the chip was pointed at), the failure paths (refusal, REPLY_ERROR, silence, no
INIT_COMPLETE, no MAC address, error interrupt), the command ring wrapping on both
families, and every SKU of the real x17 `.pnvm` file against the real firmware. QEMU has no
Intel Wi-Fi; the x17 boot log is the real test.

No ABI or kernel change: W3 is all in `sex-wifi` and sexnet.

Next, W4: scanning.

## 9. Stage W4: finding networks (2026-11-10)

Right after `[wifi.ready]` the x17 R1 should print:

```
[wifi.scan.start] channels=… ticks=…
[wifi.network] ssid="…" bssid=…:…:…:…:…:… channel=6 rssi=-48 security=wpa2 beacons=3
…one line per network…
[wifi.scan.done] networks=… frames=… dropped=0 channels=… ticks=…
```

or `[wifi.failed] step=scan reason=…` (`Refused`, `NoAnswer`, `Timeout` after 15 s,
`Status(2)` for an aborted scan, `Crashed { cause }`).

How it works (`sex_wifi::scan`, after iwx(4) `iwx_umac_scan_v14`, `iwx_fill_probe_req`,
`iwx_rx_mpdu_mq`):

- **Channels.** The NVM's channel flags, replaced by the regulatory answer's when MCC
  returns a list (as `iwx_init_channel_map` does): every valid 2.4 GHz channel, and the
  5 GHz ones when the SKU has the band, up to the firmware's limit (67). 6 GHz waits for W6.
- **Request.** One `SCAN_REQ_UMAC` (version 14; the x17's firmware declares 15, which has
  the same layout and is what iwx sends it): passive (no probe requests until the user
  names a network), adaptive dwell, every frame passed up, a notice per iteration. The
  probe template (our address, rates per band) is filled in for later directed scans.
- **Results.** Beacons and probe responses arrive as RX MPDU notifications: a 48-byte
  (22000) or 64-byte (AX210) descriptor, then the frame, with two pad bytes after the
  header when the firmware says so. Frames with a bad CRC are dropped. The existing
  `ieee80211::parse_bss` reads SSID, channel and security (open, WPA2, WPA3, transition);
  each BSSID is kept once with its strongest signal; 32 networks at most, the weakest
  giving way.

The scan runs on the command queue and receive ring that W3 left running; no PHY or MAC
context is needed to scan (iwx scans before it adds one).

Proof: four new host tests: the request's bytes (channels, bands, flags, probe template,
the firmware's channel limit), MPDU parsing with and without padding on both families,
a whole scan against the firmware model after the init sequence (two networks, the
strongest beacon kept, security read, an aborted scan reported), and the list when full.

No ABI or kernel change.

Next, W4b: the list on screen (status bar, `wifi scan` in Spindle), then W5: connecting.

## 10. Stage W4b: networks on screen (2026-11-10)

Spindle gains `wifi`:

```
> wifi
Wi-Fi: 7 networks heard on 67 channels (scan 1).
  home-5g  -48 dBm  channel 36  WPA2/WPA3  aa:bb:cc:dd:ee:01
  …
> wifi scan
Wi-Fi: scanning 67 channels (0 networks so far). Type wifi to see them.
```

Under QEMU (no Intel chip) it says so: `Wi-Fi: no Intel Wi-Fi chip here, or its firmware is
missing.`

- **Protocol** (`sex_pdx::wifi_status`, op `0x2E8` to sexnet): `STATE`, `SCAN`, and
  `NETWORK index part`. Every answer is one reply word (top byte `0x57`, or `0x58` for a
  refusal or no such network), like the sound mixer's. A network takes up to seven words:
  its facts (channel, signal, security, SSID length, beacons), its BSSID, and the SSID in
  7-byte parts, asked one after another. No shared memory or new kernel object is needed.
- **Who may ask:** Shell, SilkBar and Spindle rows read; Shell and Spindle may start a scan.
  A network list tells where the machine is, so programs are refused (`[wifi.status.denied]`).
- **Rescans:** `SCAN` starts a fresh scan when the chip is ready and the last scan has
  ended; the list then fills again as beacons arrive. Each scan logs `[wifi.scan.start]
  … scan=N` and ends with the same `[wifi.network]` and `[wifi.scan.done]` lines as W4.

Proof: host tests of the words (round trips, a 32-byte SSID through its five parts, other
sexnet replies never mistaken for Wi-Fi ones), run by `scripts/wifi_host_gate.sh`; sexnet
and Spindle build for the service target. The list itself needs the chip: on the x17,
type `wifi`.

**ABI:** a new PDX op and module (`sex_pdx::wifi_status`, op `0x2E8`); `sex-pdx/src/lib.rs`
gains `pub mod wifi_status`, so `abi_version_hash` is recomputed. No kernel change.

Next: the status bar's Wi-Fi indicator (SilkBar reads `STATE`), then W5: connecting.

## 11. Stage W5: connecting, and W5a: joining the channel (2026-11-10)

Connecting is split in five parts (table in section 4), each provable on its own. They
follow iwx(4)'s state machine (`iwx_newstate`): AUTH runs `iwx_auth` (this stage), the
authentication and association frames go out on the management queue (W5b), RUN
(`iwx_run`) marks the MAC and station associated and starts rate control, and the
4-way handshake (W5c) or SAE (W5d) install the keys (`iwx_add_sta_key`).

The x17's firmware (-77) declares the versions this driver sends: PHY_CONTEXT v4 with
the 6 GHz-sized channel field (capa 48), RLC_CONFIG v2 (which carries the receive
chains), MAC_CONTEXT, BINDING v2, ADD_STA (v10 layout), SCD_QUEUE_CONFIG v3,
SESSION_PROTECTION v1. It also claims the newer "MLD" API (capa 110), which iwx uses only
on MA-type MACs (AX211 variants) and the BZ family; the x17's TY MAC (0x42) keeps the
commands above. MA chips are refused for now with `Unsupported("mld_api")`.

W5a (`sex_wifi::join`), in iwx_auth's order:

1. PHY context 0 on the AP's channel, 20 MHz (wider channels are W6), on LMAC 0, or
   LMAC 1 for a 5 GHz channel when the firmware is dual-band (CDB, capa 40; the binding
   names the same LMAC).
2. RLC: the receive antennas the firmware allows, narrowed by the NVM's mask
   (`Init::valid_rx_antennas`), one idle and one active chain.
3. MAC context 0: our address, the AP's, ACK rates from the AP's basic rates
   (`iwx_ack_rates`, now read from the beacon's rate elements), 802.11 default EDCA per
   FIFO, beacons let through until associated.
4. Binding of MAC 0 to PHY 0 (status must be 0).
5. Station 0 for the AP (status must be ADD_STA_SUCCESS).
6. The management queue: a new 256-entry TFD ring, byte-count table and 32 frame
   buffers in the DMA region (`Layout::tx_ring`, `tx_byte_counts`, `tx_buffers`); the
   firmware must answer queue 1, write pointer 0.
7. Session protection for nine beacon intervals.

In Spindle, `wifi` numbers the networks and `wifi join N` starts the join. The boot log
shows `[wifi.join.start]`, a `[wifi.join] step=…` line per command, then `[wifi.joined]
ssid=… bssid=… channel=… phy=4 rlc=2 queue=3`, or `[wifi.failed] step=join reason=…`.
While joined, `wifi scan` is refused (leaving the channel comes with W5b).

Proof: three host tests on the firmware model (the order and bytes of all seven
commands after the x17 init; a refused station and a wrong queue number fail cleanly;
MLD-only and old PHY firmware refused), one on the command layouts (ACK rates, the
older PHY layout, MAC context after association, EDCA per FIFO), one on beacon rates and
DTIM, and the real x17 firmware's versions when the files are fetched. The firmware
model checks every payload size.

No ABI change: the `wifi_status` words gain `JOIN` and the phases `JOINING`/`JOINED`
(sex-pdx `wifi_status.rs` is not an ABI input; `lib.rs` is unchanged). No kernel change.

Next: W5b, the first frames on the air.

## 12. Stage W5b: authentication and association (2026-11-10)

`wifi join N` now goes on after the join (section 11): `sex_wifi::assoc` sends an
open-system authentication frame, then an association request (SSID, rates, and our
RSN element on a WPA2 network), and waits 300 ms for each answer, three tries each.
Once the AP accepts, the station entry is updated and the MAC context is modified
with the AP's association ID, as `iwx_run` does. Beacons keep passing the filter until
a DTIM period is known, as in iwx.

Frames go out through `sex_wifi::tx` (iwx's `iwx_tx`) on the management queue the join
created. The 4-byte command header, the TX command and the 802.11 header sit in one of
32 frame buffers. On the AX210 family (the x17) the command is `iwx_tx_cmd_gen3`
(TX_CMD v10); before that family it is gen2. The body follows the header in the same
buffer. TB0 is the first 20 bytes, TB1 is the rest of the command, and TB2 is the body.
The byte-count table entry counts bytes on AX210 and dwords before it. Management frames
use a fixed rate: the AP's lowest basic rate, on antenna A, in rate_n_flags v2 (both
firmwares report TX_CMD notification v7). The TX response's SSN frees the ring.

Boot log: `[wifi.assoc.start] security=…`, then `[wifi.assoc] step=…` lines, then
`[wifi.associated] ssid=… aid=…`, or `[wifi.failed] step=assoc reason=Refused(Status
{ frame: "auth", status: N })` (NoAnswer, NotSent, Disconnected). The desktop sees
the phases ASSOCIATING (7) and ASSOCIATED (8).

Not yet:
- WPA3-only networks use SAE (W5d, section 17); `Assoc::new` alone still refuses them (`wpa3_sae`).
- WPA2's 4-way handshake follows association (section 13).
- No HT/VHT capabilities are sent, so association is legacy-rate only (W6).
- `iwx_run`'s smart-fifo, multicast and power commands, and TLC, come with the data
  path (W5e).

## 13. Stage W5c: WPA2 keys (2026-11-10)

`wifi join N PASSPHRASE` (Spindle) sends the passphrase to sexnet in 7-byte parts
(`wifi_status::PASSWORD`, 8 to 63 characters), then the join. A WPA2 network joined
without one is answered `NEEDS_PASSWORD`. sexnet derives the PMK (PBKDF2) when the
association succeeds, then wipes the passphrase. It also wipes it on any failure.
Spindle keeps no copy, and its saved history drops the passphrase.

`sex_wifi::rsna` runs `eapol::Supplicant` (already tested against the authenticator
helpers) over the air:
- EAPOL-Key frames from the AP are from-DS data frames with LLC/SNAP 0x888e. Message 1
  often comes right behind the association response, so `assoc` keeps the last one
  it hears (`Assoc::early`).
- Replies go out on queue 1 as to-DS data frames, unencrypted, with high priority and at
  the management rate, as `iwx_tx_fill_cmd` sends EAPOL. Message 4 goes out before its
  keys are installed.
- Keys are installed with `iwx_mld_set_sta_key_cmd`'s SEC_KEY_CMD (DATA_PATH 0x18, 80
  bytes) when the firmware lists it, as the x17's -77 firmware does. Otherwise they go
  with ADD_STA_KEY v2/v3 (76 bytes, status checked). Pairwise is key 0. The group key
  uses its KDE id with the multicast flag. The MFP flag is set when the AP offers MFP.
  An IGTK goes in only with SEC_KEY_CMD.
- The phases are HANDSHAKE (9), then CONNECTED (10) once the pairwise and group keys
  are in.

Boot log: `[wifi.handshake.start] sec_key_cmd=1 early=1`, then `[wifi.handshake]
step=… messages=… keys=(pairwise, group, igtk)`, then `[wifi.connected] ssid=…
security=WPA2`, or `[wifi.failed] step=handshake reason=…`. A wrong passphrase shows
up as `Refused(Timeout { messages: 1 })`: the AP never sends message 3.

The SNonce comes from sexnet's entropy service (the same source TLS seeds from).
Next: data frames into sexnet's Ethernet path (W5e, with TLC and the `iwx_run`
leftovers), and SAE for WPA3 (W5d).

## 14. Stage W5e: traffic over Wi-Fi (2026-11-10)

`sex_wifi::link` turns the connection into an Ethernet link:
- **Out.** An Ethernet II frame becomes a to-DS data frame (LLC/SNAP) on queue 1. On WPA2
  the Protected bit is set and ENCRYPT_DIS is clear, so the firmware adds the CCMP
  header and MIC with the pairwise key (`iwx_tx`).
- **In.** Data frames from our AP are kept only if the firmware decrypted them
  (CCM, DEC_DONE, MIC_OK: `iwx_rx_hwdecrypt`). Their packet number must be newer
  than the last for that key and TID (`iwx_ccmp_decap`; the pairwise and group keys
  count apart). The CCMP header is removed, and the frame is cut to the length its IPv4,
  IPv6 or ARP packet declares, so a MIC some firmware leaves at the end never reaches
  the stack (a full 1500-byte packet would otherwise not fit). Plain data on a
  WPA2 link, our own broadcasts relayed back, and frames from other BSSs are dropped.
  RX MPDU padding is now removed after any header, as `iwx_rx_mpdu_mq` does, not
  only after management headers.
- **Rate.** Fixed at OFDM 12 Mb/s on antenna A until TLC rate control is configured
  (next step, together with `iwx_run`'s smart-fifo, multicast and power commands).
  This is slow but dependable for the first hardware test.

sexnet: once the link is up, the IP stack (`sex_net::Engine`) is rebuilt on the Wi-Fi
MAC and fed from the link, one frame per turn from a 32-frame receive queue (the air
delivers bursts). The wired port is set aside while Wi-Fi is up, and when the link goes
down (deauthentication, firmware error) the stack is rebuilt at once on a running wired
port. Each switch resets the web service's flows and sockets, which belonged to the
old stack; programs see their flows fail and reconnect. The
browser's open-internet side gets its DHCP lease on the Wi-Fi network, as it does on a
wired one.

Boot log: `[wifi.link.up] security=… rate=…`, `[sexnet.wifi.transport] up=1 mac=…`,
`[wifi.link.tx]` / `[wifi.link.rx] frames=… dropped=… last_drop=…` (sampled),
`[wifi.link.down] reason=…`.

Not yet:
- group-key rekeying (EAPOL after the handshake is dropped, so the AP eventually
  deauthenticates us, typically after an hour);
- TLC;
- QoS/WMM queues;
- power save;
- roaming;
- saved networks.

## 15. Stage W5f: group key rekeying (2026-11-10)

APs replace the group key every hour or so (some daily), and some also renew the
pairwise key. Without an answer they deauthenticate the station, so the x17 test would
drop after the first rekey. The handshake's supplicant now lives on in the link
(`rsna::Rekey`, handed over by `Rsna::into_link`):

- EAPOL-Key frames from the AP after the handshake arrive protected like other data.
  The link sends them to the supplicant instead of the IP stack. The supplicant
  checks the MIC and the replay counter, then unwraps the new GTK (and IGTK).
- The answer (group message 2) goes back protected, under the current keys, before
  the new key is installed.
- `rsna::Keys` installs keys, one command at a time, with SEC_KEY_CMD or ADD_STA_KEY.
  It is now shared by the handshake and the link. Once a key is in, its packet-number
  counters start over.
- A key the supplicant already installed is never installed again (KRACK). A repeated
  frame is dropped as a CCMP replay; a repeated key message as an EAPOL replay.

Boot log: `[wifi.rekey] messages=… installed=… error=…`.

## 16. Stage W5g: firmware rate control (2026-11-10)

Data used to go at a fixed 12 Mb/s. Once the link runs, it now sends
TLC_MNG_CONFIG_CMD (DATA_PATH 0x0f, v3 or v4 by the firmware's version table, 28 bytes
either way; `iwx_rs_init`) with the legacy rates both sides have: station 0, 20 MHz,
non-HT, chain A, max MPDU 2312. After the answer, data frames go without CMD_RATE and
with rate 0, and the firmware picks each frame's rate (`iwx_tx_fill_cmd`). Management
and EAPOL-before-keys frames keep the lowest basic rate. TLC_MNG_UPDATE_NOTIF (0xf7)
reports the chosen rate. If the command is refused or not answered within 1 s, the fixed
rate stays.

`Bss::rates` (and `Target::rates`) now record every rate the AP names, not only the
basic ones. The station still associates as a legacy station, so the top rate is
54 Mb/s. HT/VHT (HT capabilities in the association request, an HT PHY context, and
HT rates in this command) is the next step for speed.

Boot log: `[wifi.rate] control=… rate=…`.

## 17. Stage W5d: WPA3-Personal with SAE (2026-11-10)

On a WPA3-only network, sexnet builds SAE's password element for the AP and the x17.
It uses hash-to-element when the AP advertises it (RSNXE), and hunting and pecking
otherwise. Random values come from the entropy service. `Assoc::with_sae` then
authenticates with SAE instead of open system:

- **Commit** (algorithm 3, transaction 1): group 19, scalar and element. Status 126
  is used with hash-to-element, 0 otherwise.
- **Anti-clogging token.** If the AP asks for one (status 76), the commit goes again
  at once with the token: bare after the group, or in its container element (255/93)
  at the end with hash-to-element.
- **AP commit.** The AP's commit gives the KCK, PMK and PMKID (`sae::Sae`), and our
  confirm goes at once.
- **Confirm.** The AP's confirm (transaction 2) must check out, else
  `Refusal::Sae(BadConfirm)`. With a wrong password the AP drops our confirm, so the
  failure shows as `NoAnswer("sae_confirm")`.

The association request then carries our SAE RSN element: AKM 00-0f-ac-8, MFPC and
MFPR (management frame protection required), and BIP-CMAC as the group management
cipher. With hash-to-element it also carries the RSNXE. The 4-way handshake runs with
`Akm::Sae` and the PMK from SAE (the passphrase is wiped then). The IGTK is installed
with SEC_KEY_CMD.

Transition networks (WPA2/WPA3) keep using WPA2 for now. Only group 19 is offered.

Boot log: `[wifi.assoc.start] security=Wpa3Personal step=SaeCommit`,
`[wifi.handshake.start] akm=Sae …`.
