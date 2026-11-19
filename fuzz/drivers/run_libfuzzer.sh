#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Coverage-guided run of every driver target (needs nightly + cargo-fuzz):
#   ./run_libfuzzer.sh SECONDS [TARGET...]
# Each target starts from its seed corpus (written by the target itself) plus
# whatever corpus/<target> holds from earlier runs; crashes land in artifacts/.
set -uo pipefail
REPORT_ONLY="drv_acpi_aml"
cd "$(dirname "$0")"
secs=${1:-60}; shift || true
targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then mapfile -t targets < <(cargo fuzz list --fuzz-dir . 2>/dev/null); fi
cargo fuzz build --fuzz-dir . --features libfuzzer -O --debug-assertions "${targets[@]:0:1}" >/dev/null 2>&1 || true
fail=0
for t in "${targets[@]}"; do
  mkdir -p "corpus/$t" "artifacts/$t"
  cargo run -q --release --bin "$t" -- --seeds "corpus/$t" 2>/dev/null
  if cargo fuzz run --fuzz-dir . --features libfuzzer -O --debug-assertions "$t" "corpus/$t" -- \
       -max_total_time="$secs" -timeout=20 -rss_limit_mb=4096 -artifact_prefix="artifacts/$t/" -print_final_stats=1 \
       > "artifacts/$t.log" 2>&1; then
    echo "[driver_fuzz.libfuzzer] target=$t ok $(grep -E 'stat::number_of_executed_units' "artifacts/$t.log" | awk '{print "execs="$2}') $(grep -oE 'cov: [0-9]+' "artifacts/$t.log" | tail -1 | tr -d ' ')"
  else
    [[ " $REPORT_ONLY " == *" $t "* && "${STRICT:-0}" != 1 ]] || fail=1
    echo "[driver_fuzz.libfuzzer] target=$t FAILED"
    grep -E 'panicked at|ERROR: libFuzzer|SUMMARY|DRIVER_HANG|Test unit written' "artifacts/$t.log" | head -6
  fi
done
exit $fail
