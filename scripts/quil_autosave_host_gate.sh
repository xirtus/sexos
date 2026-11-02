#!/usr/bin/env bash
# Host tests of Quil's idle autosave policy (servers/quil/src/autosave.rs).
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TEST_DIR="$(mktemp -d)"; trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/servers/quil/src/autosave.rs" -o "$TEST_DIR/autosave-tests"
"$TEST_DIR/autosave-tests"
