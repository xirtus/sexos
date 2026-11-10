# Hardware preflight: truthful host inventory

The existing `scripts/real_hardware_preflight.sh` was stale relative to master
`17a1a530`: it said SexOS did not use an IOMMU, labelled VMX/SVM as IOMMU support,
and preferred unrelated target JSON files over the repository's actual spec.
It could also report Secure Boot disabled when `mokutil` failed.

This change keeps the script read-only and distinguishes CPU virtualization,
reported CPU PKU support, exposed DMAR/IVRS firmware-table paths and later actual
SexOS enforcement. Missing PKU on an inspectable target or a non-x86-64 target
fails this daily-driver inventory. A VMX/SVM flag never establishes DMA isolation.
The checked-in `x86_64-sex.json` is required; an unrelated home/sibling file cannot
mask its absence. Failed/unknown Secure Boot queries remain unknown. A host ttyS0
or USB-serial device is not evidence that SexOS can use it for boot logging.

The kernel's actual `[iommu.unit.enable]` and per-device translation receipts must
be retained after boot. `iommu=off` remains an explicitly unprotected diagnostic
path. Signed update payloads do not imply Secure Boot support. This inventory
cannot qualify hardware, prove radio operation or substitute for the x17 workload
and recovery gates.

Validation:

- `bash -n scripts/real_hardware_preflight.sh`.
- Seven full-script Bash subprocess tests in
  `scripts/tests/test_hardware_preflight.py`: Intel/AMD table inventory,
  VMX without a table, missing PKU, false PKU substring, a missing repository target
  despite a sibling target, and failed Secure Boot inspection. Fixture paths use
  task-specific overrides; the user's home environment is preserved.
- Every fixture run uses explicit `SEXOS_PREFLIGHT_{REPO,PROC,SYS}_ROOT` overrides
  and prints a fixture/override warning. These are synthetic host-inventory tests,
  not a claim of real hardware results. The production script defaults to its own
  repository and real `/proc`/`/sys`.
- `git diff --check`.

No kernel, device driver, bootloader, partition or firmware setting changes.
