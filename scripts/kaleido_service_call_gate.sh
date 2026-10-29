#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
output=$(mktemp -d "${TMPDIR:-/tmp}/kaleido-service-call.XXXXXX")
trap 'rm -rf "$output"' EXIT
rustc --edition=2021 --test tests/kaleido_service_call.rs -o "$output/tests"
"$output/tests"
rustc --edition=2021 --test tests/kaleido_body_text.rs -o "$output/body-tests"
"$output/body-tests"
