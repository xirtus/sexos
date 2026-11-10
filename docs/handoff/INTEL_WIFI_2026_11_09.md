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
| **W1** (this PR) | driver core crate: chips, firmware files, DMA layout, power-up, commands, 802.11, WPA2/WPA3, CCMP | host tests with IEEE/RFC vectors, real firmware images |
| **W2** | kernel leases Intel Wi-Fi (class 02:80, ids from `sex_wifi::devices`) to sexnet with a 4 MiB DMA region; firmware files on the boot image; sexnet powers the chip up and waits for ALIVE; boot log `[wifi.chip]`, `[wifi.alive] fw=…` or the exact failure step | host gate on the lease; **x17 boot log next week** |
| W3 | RX ring and command queue processing; the init command set (PNVM load, PHY/antenna config, BT coexistence, regulatory MCC, MAC and station contexts) | x17: "init complete" |
| W4 | scanning (UMAC scan), a network list in the status bar and `wifi scan` in Spindle | x17 lists nearby networks |
| W5 | connect: open/SAE authentication, association, the handshake from this crate, key install, data queues into sexnet's Ethernet path; passphrase prompt; saved networks | x17 browses the web over Wi-Fi |
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
