#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --locked --manifest-path tests/autofill-host/Cargo.toml -- --test-threads=1
