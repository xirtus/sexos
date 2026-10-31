#!/usr/bin/env bash
# The BOOT app incarnation catalog against the lifecycle constants and the
# kernel's actual boot plan; data only, never authority.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-app-catalog.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/app_catalog.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
echo '[app.catalog.result] PASS scope=catalog_equals_lifecycle_constants_and_kernel_boot_plan_not_native'
