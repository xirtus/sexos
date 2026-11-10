#!/usr/bin/env bash
# Re-pin spec/abi_inputs.sha256 after a deliberate ABI change: one line per ABI
# input, so unrelated ABI edits in parallel branches merge without conflicts.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
mapfile -t inputs < <(sed -n '/^ABI_INPUTS=(/,/^)/{/^ *[a-z]/p}' scripts/entrypoint_build.sh | tr -d ' ')
# A blank line between pins keeps edits to neighbouring files in separate
# merge hunks, so they merge cleanly; edits to the same file still conflict.
sha256sum "${inputs[@]}" | sort -k2 | sed 's/$/\n/' > spec/abi_inputs.sha256
echo "pinned ${#inputs[@]} ABI inputs in spec/abi_inputs.sha256"
