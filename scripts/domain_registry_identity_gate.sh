#!/usr/bin/env bash
# Native tests of the actual registry implementation; no kernel/image build.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-domain-registry.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$ROOT_DIR"
rustc --edition=2021 --test tests/domain_registry_identity.rs -o "$TEST_DIR/registry-tests"
"$TEST_DIR/registry-tests"
echo '[domain_registry.identity.gate.result] PASS'
