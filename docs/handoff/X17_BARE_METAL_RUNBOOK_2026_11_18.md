# Alienware x17 R1: first bare-metal start (runbook, 2026-11-18)

What to set, what to bring, and what to send back after the first start of SexOS on
Andreas's Alienware x17 R1. The background (parts, PCI ids, driver plan) is in
[DRIVERS_AND_X17_R1_PLAN_2026_11_09.md](DRIVERS_AND_X17_R1_PLAN_2026_11_09.md).

## 1. Before starting

1. **Back up** anything on the internal disks you care about. SexOS refuses to write to
   a disk that holds another system (`[sexdrive.disk.foreign] writes=refused`) and serves
   an Intel RAID volume read only, but a first start on new hardware is still a first start.
2. **BIOS (F2 at the Alienware logo):**
   - Secure Boot: **off** (Limine is not signed).
   - Storage: leave it as it is. "RAID On" works: SexOS opens Intel VMD itself and reads a
     RAID 0/1 volume read only. "AHCI/NVMe" gives SexOS the drives directly. Switching
     modes breaks an existing RAID volume, so do not switch just for this test.
   - Graphics: **Hybrid** (Optimus). The Intel GPU drives the panel; SexOS powers the
     RTX 3070 down.
3. **The stick:** the `sexos-v1.0.0.iso` from CI (the `sexos-image` artifact of a green
   master run), written to a USB stick with `dd` or balenaEtcher. The ISO already holds
   the Wi-Fi firmware for the Killer AX1675.
4. **Bring:** a USB keyboard and mouse (the internal keyboard is PS/2 and should work,
   the touchpad may not yet), and an Ethernet cable for the Killer E3100G port as a
   fallback for Wi-Fi.
5. Start from the stick with **F12** (one-time boot menu), UEFI entry for the stick.

## 2. What to look at, and what to send back

The x17 has **no serial port**. Three places show what happened:

| When | Where | What to do |
|---|---|---|
| During start, until the desktop | Boot log on screen (kernel lines, white on black) | If it stops, photograph the last screen |
| If the kernel stops | Red "SexOS stopped" screen | Photograph it; it restarts after 30 s |
| Once the desktop is up | **Spindle: `hw`** (Scroll Lock shows Spindle) | Photograph each screen of output (scroll the terminal back for earlier lines) |

`hw` reads the kernel's copy of the boot log's first 256 KiB and shows only the
hardware lines: the PCI device census, VMD, the NVMe drives (model, firmware, transfer
limit, sector format), RAID, IOMMU, Intel display, GPU power, sound, network cards, USB,
ACPI and Wi-Fi. `hw nvme`, `hw wifi`, `hw display` show every line containing that text;
`hw all` the newest lines of the whole head. `crashes` lists anything that stopped.

Also useful from Spindle: `df` (did the disk mount), `wifi` and `wifi scan` (radio state
and networks), `wifi join N PASSWORD`, `beep` (sound).

## 3. Lines that answer the open questions

| Question | Line to look for | Good | Bad, and what it means |
|---|---|---|---|
| Which devices are there | `[hw.device] bb:dd.f vvvv:dddd ... support=` and `[hw.census]` | every row | `[hw.warning]` rows name what is missing |
| RAID mode (VMD) | `[kernel.vmd.found]`, `[kernel.vmd.device] ... class=01:08:02`, `[kernel.vmd.done] added=N` | `added` = number of SSDs | `[kernel.vmd.skip] reason=...` or `added=0`: VMD not opened |
| The SSD | `[sexdrive.nvme.drive] vendor=144d model="..." mdts=.. max_transfer_kib=..` | Samsung = vendor 144d | `[sexdrive.nvme.refused] reason=...` |
| Its format | `[sexdrive.nvme.namespace] sectors=N lba_shift=9` | lba_shift=9 | `[sexdrive.nvme.namespace.refused] reason=sector_size lba_bytes=4096 fix=nvme_format_lbaf_512`: the drive uses 4 KiB sectors (not supported yet; reformatting erases it) |
| Second SSD / RAID volume | `[sexdrive.nvme2.drive]`, `[sexdrive.raid.serve] ok=1 ... writes=0` | | `[sexdrive.nvme2.err] reason=...` |
| Whose disk | `[sexdrive.disk.layout] kind=...` | `foreign` is expected with Windows on it: nothing is written | |
| Interrupts | `[sexdrive.nvme.irq] mode=msix polling=0` | msix | `polling=1` is fine behind VMD (no MSI-X there yet) |
| DMA isolation | `[iommu.enabled] vendor=intel ...` | | `[iommu.absent] reason=...`; if the start hangs, pick "SexOS without DMA protection" in the boot menu |
| Panel | `[display.intel] device=9a60 ...`, `[display.intel.edid] ...` | | |
| dGPU off | `[gpu.found] ... vendor=10de`, `[gpu.power] ...` | | |
| Wi-Fi | `[wifi.chip] ...`, `[wifi.firmware] ...`, `[wifi.alive]`, `[wifi.ready] mac=... country=..` | ready | `[wifi.failed] step=... reason=...` |
| Sound | `[bell.hda.ready] vendor=0x10ec0..` | Realtek codec | |

## 4. If it goes wrong

- **Black screen right after the boot menu:** try the boot menu entry "SexOS without DMA
  protection"; if that starts, the IOMMU setup is the cause.
- **Stops at a driver line:** the last line on the boot log is where it hung; photograph it.
- **No internal disk at all, RAID On:** `hw vmd` and `hw nvme` say whether VMD opened and
  whether a drive answered.
- **No Wi-Fi:** `hw wifi`; Ethernet on the Killer port is the fallback.

Send the photos (or the `hw` screens) to the project; they settle every "to confirm" in
the driver plan.
