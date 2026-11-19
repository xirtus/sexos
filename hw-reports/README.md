# Hardware reports from real machines

SexOS has no driver lab. What real hardware does comes from the machines the
project owner boots SexOS on, the Alienware x17 R1 first. Each report lands here,
one folder per machine, with a verdict beside it.

## Sending one

1. Boot SexOS (the CI ISO on a USB stick, see
   `docs/handoff/X17_BARE_METAL_RUNBOOK_2026_11_18.md`).
2. Plug in a second FAT or exFAT stick. In Spindle: `ls /media`, `cd /media/NAME`,
   `hw save`, then `unmount NAME`. That writes `hw-report.txt`: every hardware line of
   the boot log plus its newest 200 lines.
3. On the Mac, copy it to `~/Projects/Sex/sexwork/hw-reports/` (or upload it to the
   project chat). A thread files it:

   ```
   python3 scripts/hw_report.py hw-report.txt --machine x17r1 --file --seeds fuzz/drivers/seeds
   ```

   which writes `hw-reports/x17r1/DATE-N.txt` and `DATE-N.md` (the verdict, and what
   changed since the previous report), and turns device data in it (EDID) into fuzz seeds.
4. The pull request that adds it runs `hw_report.py --check-all hw-reports` in CI
   (Driver fuzzing workflow), so the verdict shows in the checks.

## What the verdict checks

One row per question of the runbook: devices without a driver, VMD, the SSD (format,
interrupts), RAID, Windows volumes, IOMMU, Intel display and panel width, the NVIDIA GPU,
Wi-Fi, sound, USB, touchpad, ACPI, battery, and whether anything crashed. Each row is
`ok`, `BAD` (with what it means) or `?` (the report has no line that answers it). The
checks live in `CHECKS` in `scripts/hw_report.py`; add a row when a driver gains a
`[tag]` line that answers a new question.
