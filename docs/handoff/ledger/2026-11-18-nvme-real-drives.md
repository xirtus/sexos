# 2026-11-18: NVMe drives as real laptops have them

- SexDrive reads the drive's maximum transfer size (Identify Controller MDTS) and cuts
  every larger request into commands the drive accepts. Before, every command could be
  128 KiB, which a drive with a 64 KiB limit fails. Both drives of an Intel RAID set
  follow their own limit.
- A namespace whose sectors are not 512 bytes, or that carries per-sector metadata or
  protection information, is refused before any read or write:
  `[sexdrive.nvme.namespace.refused] nsid=1 reason=sector_size lba_bytes=4096 ...
  fix=nvme_format_lbaf_512`. Before, the drive was served anyway, and a 4 KiB-sector
  read would DMA 4 KiB into a 512-byte buffer. A namespace that cannot be identified is
  refused too (it used to fall back to the first 1 MiB).
- Admin commands wait up to 10 s of wall time after their poll budget (the first
  Identify used to give up after 1M polls, a few milliseconds on bare metal).
- A controller that reads all ones, lacks 4 KiB pages or the NVM command set is refused
  with one line instead of timing out step by step.
- The boot log names the drive: `[sexdrive.nvme.drive] vendor=144d model="..."
  firmware="..." mdts=9 max_transfer_kib=128 namespaces=1`.
- Tests: `tests/sexdrive_identify.rs` (in `scripts/sexdrive_dma_host_gate.sh`) and the
  QEMU gate `scripts/nvme_real_drive_gate.py` (MDTS 4 drive formats, writes and mounts
  again; a 4 KiB-sector drive is refused and left byte-for-byte unchanged). Gates take
  `SEXOS_GATE_NVME_OPTS` for extra NVMe device properties.
