# Saved seeds

`seeds/<target>/` holds inputs both runners start from: the built-in mutator
(`run.sh`) adds them to its pool and `run_libfuzzer.sh` copies them into the
target's corpus. `scripts/hw_report.py REPORT --seeds fuzz/drivers/seeds` writes
device blobs from real machines here (EDID -> `drv_display_intel`).
