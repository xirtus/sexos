# Intel Wi-Fi handoff (2026-11-10)

This note is for whoever picks up the native Intel Wi-Fi driver (`crates/sex-wifi`,
`servers/sexnet/src/wifi.rs`). The design is in `INTEL_WIFI_2026_11_09.md`
(sections 1–17). Per-stage evidence is in `SEXOS_FEATURE_LEDGER_2026_11_02.md`.

## State

| Stage | What | Where |
|-------|------|-------|
| W1–W4b | chip, firmware, init, scan, Spindle `wifi`/`wifi scan` | merged |
| W5a–c | join, auth/assoc, WPA2 4-way, key install | merged (#108) |
| W5e | data path: sexnet's IP stack moves onto Wi-Fi while connected | merged (#116) |
| W5f | group/pairwise rekeying while connected | PR #120 |
| W5g | firmware rate control (TLC), legacy rates up to 54 Mb/s | PR #120 |
| W5d | WPA3-Personal: SAE (hunting-and-pecking + H2E, anti-clogging tokens) | PR #120 |

Everything is tested only against the firmware model in `crates/sex-wifi/tests/driver.rs`.
No radio has run yet. First real test: the Alienware x17 R1 (Killer AX1675x,
8086:2725, firmware `iwlwifi-ty-a0-gf-a0-77`).

## How to check

- `bash scripts/wifi_host_gate.sh` runs 50 driver tests and the Spindle command tests.
- Build sexnet with the RUSTFLAGS line in `scripts/entrypoint_build.sh`
  (`-Z build-std`, `--target x86_64-sex.json`).
- On hardware, from Spindle: `wifi`, `wifi scan`, then `wifi join N PASSPHRASE`.
  Check the serial log, in order:
  - `[wifi.joined]`
  - `[wifi.associated]`
  - `[wifi.handshake]`
  - `[wifi.connected]`
  - `[wifi.link.up]`
  - `[sexnet.wifi.transport] up=1`
  - `[wifi.rate]`
  - `[wifi.rekey]` (after about an hour)
- On failure, the `[wifi.failed] step=… reason=…` line names the stage.

## Next steps, in priority order

1. **Hardware bring-up on the x17.** Fix whatever the real firmware disagrees with.
   Likely spots:
   - TX command flags and offload
   - SEC_KEY_CMD vs ADD_STA_KEY (picked from the firmware's command version table)
   - RX MPDU status bits
   - whether the firmware leaves the CCMP MIC in RX frames (handled either way)

   Compare against OpenBSD `sys/dev/pci/if_iwx.c` (ISC). Never use Linux iwlwifi code
   (GPL-2-only).
2. **HT/VHT rates.** Add HT capabilities to the association request
   (`ieee80211::assoc_request`), set the PHY context to the AP's width, set the HT/VHT
   bits in the MAC context, and pass HT/VHT rates and the width in
   `tlc::config` (`iwx_rs_init_v3/v4`). Legacy tops out at 54 Mb/s.
3. **WPA3 transition networks via SAE.** They use WPA2 today. Choose SAE when the
   network offers it (`Assoc::with_sae`).
4. **`iwx_run` leftovers.**
   - smart FIFO (SF_CONFIG)
   - multicast filter (MCAST_FILTER_CMD, so ARP/IPv6 multicast is received reliably)
   - power save
   - beacon-loss / missed-beacon notification, to take the link down and rejoin
5. **Reconnect.** Rejoin after a deauthentication or a link loss, and remember the
   last network (passphrase storage needs a design decision: keep it in a capability-held
   secret, never in plain files).
6. **Status bar Wi-Fi chip.** SilkBar has no SLOT_NET route; it needs a kernel init grant.

## Rules

- Licensing: code we write is GPL-3.0-only. iwx(4) is ISC and fine to adapt. Linux
  iwlwifi is GPL-2-only, so never copy from it.
- Each stage gets a ledger entry and a doc section. Any ABI change must be named and
  `abi_version_hash` recomputed (none so far for Wi-Fi).
- Never skip or disable tests to get CI green.
