# Untethered x17: Wi-Fi 6, Bluetooth audio, Thunderbolt 4 (plan, 2026-11-19)

Goal: the Alienware x17 R1 runs with no cables: fast Wi-Fi, a Bluetooth headset
for music and calls, and the Thunderbolt 4 port for docks, displays and eGPUs.
Owner: thread "Add Wi-Fi 6, Bluetooth audio and Thunderbolt". The hot-plug thread
("Support hot-plug docks, monitors and eGPUs") owns what appears behind a
Thunderbolt tunnel (PCIe rescan below a downstream port, monitors, eGPU binding).

## Where things stood

| Area | State before this plan |
|---|---|
| Wi-Fi (`crates/sex-wifi`, sexnet) | W1-W6a: AX200/201/210/211 bring-up, scan, WPA2/WPA3-SAE, rekey, rejoin. Legacy 802.11a/g only: 20 MHz, 54 Mb/s, no QoS, no aggregation. |
| Bluetooth (`crates/sex-bt`, Cufflinks) | Intel USB controllers, LE + classic HID, pairing panel, A2DP (own SBC encoder) from Bell's mix, AVRCP. No HFP (calls/mic), no SCO, no AAC. |
| Thunderbolt / USB4 | Nothing. |

## Stages

| Stage | What | Proof |
|---|---|---|
| **X1** Wi-Fi 4/5/6 rates | HT/VHT/HE elements, 40/80/160 MHz PHY context, ADD_STA HT flags, STA_HE_CTXT_CMD, TLC MCS per stream, QoS data with DSCP priority, A-MSDU subframe PNs. | host tests `crates/sex-wifi/tests/ax.rs`; x17 log `wifi=6 width=80` |
| **X2** Block ack | ADDBA request/response, firmware RX BA sessions (ADD_STA add_immediate_ba or RX_BAID_ALLOCATION_CONFIG v2), receive reorder buffer from the descriptor's reorder word and frame-release notifications, TX aggregation queue per TID (ADDBA to the AP, SCD queue per TID). | host tests with recorded frames; x17 throughput |
| **X3** Roaming, 6 GHz, saved networks | Background scans when the signal drops, reassociation to a better AP of the same network (PMK reuse for SAE), 6 GHz channels on AX210/AX211 (WPA3 only), networks remembered across boots in a cap-held store. | host tests; x17 walk test |
| **B1** HFP calls | SCO/eSCO links over the USB isochronous endpoint (needs the x17 thread's isochronous transfers), HFP AG (AT commands, codec negotiation CVSD/mSBC), headset mic into Bell's record path, Bell's output to the headset's SCO. | host tests of the AT/SCO/mSBC code; x17 with a headset |
| **B2** Bell without a card | Tick-clocked Bell mix so a headset plays with no HDA card running; AAC (if a free encoder fits). | host + QEMU sound gates |
| **T1** Thunderbolt NHI | Tiger Lake integrated TBT4 NHI (8086:9a1b/9a1d TGL-LP, 9a1f/9a21 TGL-H): MMIO, TX/RX ring 0, frame CRC, ICM firmware connection manager (driver ready, device connected/disconnected events, security level, approve device, boot ACL). | host tests against a fake NHI ring model |
| **T2** Approval UI + tunnels | Desktop prompt to approve a new Thunderbolt device (security level "user"), remembered approvals, tunnel up/down events to the hot-plug thread (rescan below the root port). | host tests; x17 with a dock |
| **T3** USB4 software connection manager | For machines whose firmware leaves the router to the OS (newer USB4 hosts); path setup for PCIe/DP/USB3 tunnels ourselves. | later |

Each stage is its own PR; long gates run on GitHub CI.

## Design notes

- Wi-Fi stays inside sexnet (as W1-W6), Bluetooth inside Cufflinks. Thunderbolt
  gets its own server (a PD of its own, like sexusb), leasing the NHI by PCI ID.
- Firmware commands follow OpenBSD iwx(4) (ISC). Where iwx has no 802.11ax, the
  HE command layouts come from iwlwifi's `fw/api` headers, which are dual-licensed
  GPL-2.0 or BSD-3-Clause. Thunderbolt follows the USB4 and Thunderbolt 3 specs
  with Linux's `drivers/thunderbolt` (GPL-2.0) as a reference only: our code is
  written fresh and is GPL-3.0.
- None of this needs a new syscall: Wi-Fi uses the existing lease, Cufflinks its
  regions, and the Thunderbolt server the driver/MSI-X syscall (86).
