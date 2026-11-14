#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --locked -p chastity-core
cargo test --locked -p chastity --no-default-features
