#!/usr/bin/env bash
# The BOOT app incarnation catalog against the lifecycle constants and the
# kernel's actual boot plan; data only, never authority.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-app-catalog.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test "$ROOT_DIR/tests/app_catalog.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/boot_app_catalog.rs" -o "$TEST_DIR/actual-grants"
python3 "$ROOT_DIR/scripts/check_catalog_capacity.py"
"$TEST_DIR/actual-grants"
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import sys
root=Path(sys.argv[1]);out=Path(sys.argv[2]);init=(root/'kernel/src/init.rs').read_text();resources=(root/'kernel/src/app_resources.rs').read_text()
assert init.index('crate::app_resources::audit_boot_catalog(silkshell_id);')<init.index('crate::boot_memory::seal_all();')
audit=resources.split('pub fn audit_boot_catalog(',1)[1].split('fn grant(',1)[0]
for anchor in ('boot();','APPS.lock()','shell.find_capability','pd.find_capability','pd.main_task.load','task.context.pd_id','task.context.pd_ptr','app.identity','DORMANT[index]','crate::app_catalog_state::validate(row,&actual,coordinator)','[boot.app.catalog.complete]'):
 assert anchor in audit,'missing actual BOOT catalog extraction/check: '+anchor
source=(root/'kernel/src/app_catalog_state.rs').read_text();guard='    if actual.close_target != row.target { return Err(Error::CloseTarget); }'
assert source.count(guard)==1
(out/'state-mutant.rs').write_text(source.replace(guard,'',1))
fixture=(root/'tests/boot_app_catalog.rs').read_text()
for name in ('../crates/sex-pdx/src/app_catalog.rs','../kernel/src/app_resource_state.rs'):
 fixture=fixture.replace(name,str(root/name.removeprefix('../')))
fixture=fixture.replace('../kernel/src/app_catalog_state.rs',str(out/'state-mutant.rs'))
(out/'mutant-test.rs').write_text(fixture)
PY
rustc --edition=2021 --test "$TEST_DIR/mutant-test.rs" -o "$TEST_DIR/guard-mutant"
if "$TEST_DIR/guard-mutant" --exact domain_routes_cannot_target_another_incarnation; then
    echo '[app.catalog.negative] FAIL wrong-route mutant survived' >&2
    exit 1
fi
echo '[app.catalog.result] PASS scope=actual_pure_grant_validator_and_boot_hook_not_native_grant_observation wrong_route_guard_removed_rejected=1'
