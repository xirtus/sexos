#!/usr/bin/env bash
# Host gate for fault recovery & automatic client reconnect:
# - Stale handoff purge & custody retirement for Queued+Exited tasks
# - IPC v2 endpoint detach & waiter wakeup with E_RESTARTED on server restart
# - Full restart notification (answers_every_call, MAX_TOLD >= 32) & hang_limit coverage
# - Automatic client reconnect (retry_on_peer_restart_with)
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
T="$(mktemp -d "${TMPDIR:-/tmp}/sexos-fault-recovery.XXXXXX")"
trap 'rm -rf "$T"' EXIT

rustc --edition=2021 -O --test "$ROOT_DIR/tests/fault_recovery.rs" -o "$T/fault_recovery"
"$T/fault_recovery" --test-threads=1

echo '[fault_recovery.host.gate] PASS scope=handoff_purge_ipcv2_teardown_restart_notify_client_reconnect'
