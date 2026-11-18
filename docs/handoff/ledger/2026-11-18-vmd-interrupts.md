# 2026-11-18: NVMe interrupts behind Intel VMD ("RAID On")

- Behind VMD a drive's MSI-X write does not reach a CPU: VMD catches it and raises one
  of its own MSI-X vectors. Before, devices behind VMD got no vectors and sexdrive
  polled every command.
- Kernel (`vmd.rs`): for each VMD domain, VMD's own MSI-X table is mapped, every entry
  masked, MSI-X enabled and VMCONFIG's MSI-remap bit set:
  `[kernel.vmd.msi] remap=1 table_size= child_vectors= bir= offset= vmconfig=`.
  `vmd::route` aims VMD vectors (from 1; vector 0 is VMD's own on client parts, as in
  Linux) at the CPU vectors `device_irq` gives the drive.
- `device_irq::record`: a device behind VMD gets MSI-X entries with address
  `0xFEE00000 | vmd_vector << 12`, data 0 (`sex_vmd::child_msi`); the driver masks and
  unmasks its own table as before. Plain MSI is not routed behind VMD (that device
  polls). `[device.irq.msix] ... vmd_vector=N` (-1 when not behind VMD).
- Safe start leaves VMD's table off (`[kernel.vmd.msi] remap=0 reason=safe_start`).
- sexdrive: if 64 sleeps pass with no interrupt at all, it says
  `[sexdrive.nvme.irq.silent] waits=64 wakes=0 polling=1` and polls from then on, so a
  route that does not work on some machine costs speed for a moment, never the disk.
- Tests: `crates/sex-vmd/tests/vmd.rs` (child address, vector hand-out; in
  `scripts/raid_host_gate.sh`). QEMU has no VMD, so the route itself is first exercised
  on the Alienware x17 R1 (runbook row "Interrupts").
