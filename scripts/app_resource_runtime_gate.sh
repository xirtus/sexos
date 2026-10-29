#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${APP_RESOURCE_LOG:?set APP_RESOURCE_LOG to the declared app_resource_cleanup_proof guest serial log}"
python3 "$ROOT_DIR/scripts/check_app_resource_log.py" "$APP_RESOURCE_LOG"
