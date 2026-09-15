# MASTER_RUNTIME_GATE_V1

- date: 2026-07-22T14:15:10-07:00
- git commit: 
- log_path: /Users/xmacbookm5/Projects/Sex/.gate_master/serial.log
- probe_seconds: 25
- nvme_enabled: 1
- nvme_img: /Users/xmacbookm5/Projects/Sex/.gate_master/nvme.img
- qemu: qemu-system-x86_64 -M q35 -m 512M -cpu max,+pku -cdrom sexos-v1.0.0.iso -device nec-usb-xhci,id=xhci -device usb-tablet,bus=xhci.0 -drive if=none,id=nvm,file=/Users/xmacbookm5/Projects/Sex/.gate_master/nvme.img,format=raw -device nvme,serial=sexos01,drive=nvm -serial file:$LOG -display none -no-reboot -no-shutdown

## Gate Results

| Gate | Status |
|------|--------|
| BUILD_GATE | PASS |
| SPAWN_GATE | PASS |
| FAULT_GATE | PASS |
| BOOTGRAPH_GATE | PASS |
| BOOTGRAPH_CLOCK_GATE | PASS |
| CAP_GRANT_GATE | PASS |
| ORDER_GATE | FAIL |
| SEXFILES_GATE (non-scoring qualifier) | PASS |
| CLOCK_GATE (legacy visible clock check) | PASS |
| SCHED_GATE (advisory) | PASS (ADVISORY) |
| **FINAL_SCORE** | **RED_MASTER** |

## Marker Counts

- Spawned PDs: 14
- Clock ticks (silkbar.clock.send): 13
- task.running total: 87
- Fault/panic hits: 0

## PD Spawn Details

- v sexdisplay: spawned
- v sexdrive: spawned
- v silk-shell: spawned
- v sexinput: spawned
- v silkbar: spawned
- v linen: spawned

- sexfiles: SEXFILES_GATE=PASS ready=1 kspawn=1

- sexdisplay (PD 1): task.running 37x
- sexdrive (PD 2): task.running 7x
- silk-shell (PD 3): task.running 7x
- sexinput (PD 4): task.running 6x
- silkbar (PD 6): task.running 6x
- linen (PD 7): task.running 6x

- sexfiles (PD 11): task.running 6x

## Clock Liveness

- silkbar.clock.send ticks: 13
- Minimum required: 2

## FINAL_SCORE Criteria (exact)

Score uses these gates:

1. **Build prerequisite**: BUILD_GATE is PASS or SKIP
2. **Hard scoring gates**: SPAWN_GATE, FAULT_GATE, BOOTGRAPH_GATE, CAP_GRANT_GATE, ORDER_GATE, BOOTGRAPH_CLOCK_GATE

Outcome logic:

- **GREEN_MASTER**: all hard scoring gates PASS and SEXFILES_GATE is PASS or SKIP
- **YELLOW_MASTER**: all hard scoring gates except BOOTGRAPH_CLOCK_GATE PASS, or hard gates PASS with SEXFILES_GATE not PASS/SKIP
- **RED_MASTER**: any other case (including BUILD_GATE FAIL)

Advisory/non-scoring rows:

- **SCHED_GATE** is advisory only and does not currently change FINAL_SCORE
- **CLOCK_GATE** is legacy visible clock check; FINAL_SCORE uses BOOTGRAPH_CLOCK_GATE
- **SEXFILES_GATE** is a non-scoring qualifier used only to distinguish GREEN vs YELLOW when hard gates pass

## Fail Criteria

- Any hard scoring gate fails
- QEMU fails to boot within probe window
- ISO missing when --skip-build used

## Run Command

```
./scripts/master_runtime_gate.sh
```

### Variants

```
# Skip rebuild (use existing ISO)
./scripts/master_runtime_gate.sh --skip-build

# Custom probe duration (e.g., 30 seconds)
./scripts/master_runtime_gate.sh --probe 30

# Keep serial log after run
./scripts/master_runtime_gate.sh --keep-log
```

## Known Notes

- sexusb (PD 5) is also present and running but excluded from the 6-PD gate requirement.
- purple-scanout module is loaded but is cosmetic/diagnostic only.
- QEMU is run headless (-display none); no window appears unless --display is overridden.
- Serial log is captured to /Users/xmacbookm5/Projects/Sex/.gate_master/serial.log and preserved only with --keep-log.
