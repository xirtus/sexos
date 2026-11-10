#!/usr/bin/env bash
# Re-pin spec/abi_inputs.sha256 after a deliberate ABI change: one line per ABI
# input, so unrelated ABI edits in parallel branches merge without conflicts.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
mapfile -t inputs < <(sed -n '/^ABI_INPUTS=(/,/^)/{/^ *[a-z]/p}' scripts/entrypoint_build.sh | tr -d ' ')
sha256sum "${inputs[@]}" | sort -k2 > spec/abi_inputs.sha256
echo "pinned ${#inputs[@]} ABI inputs in spec/abi_inputs.sha256"
