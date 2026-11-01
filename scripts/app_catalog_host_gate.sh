#!/usr/bin/env bash
# The BOOT app incarnation catalog against the lifecycle constants and the
# kernel's actual boot plan; data only, never authority.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${APP_CATALOG_GATE_OUT:-}" ]]; then
    TEST_DIR="$APP_CATALOG_GATE_OUT"
    mkdir -p "$TEST_DIR"
else
    TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-app-catalog.XXXXXX")"
    trap 'rm -rf "$TEST_DIR"' EXIT
fi
rustc --edition=2021 --test "$ROOT_DIR/tests/app_catalog.rs" -o "$TEST_DIR/tests"
"$TEST_DIR/tests"
rustc --edition=2021 --test "$ROOT_DIR/tests/boot_app_catalog.rs" -o "$TEST_DIR/actual-grants"
python3 "$ROOT_DIR/scripts/check_catalog_capacity.py"
python3 "$ROOT_DIR/scripts/check_incarnation_policy.py"
python3 "$ROOT_DIR/scripts/check_app_startup_identity.py"
"$TEST_DIR/actual-grants"
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY_CONTROLS'
from pathlib import Path
import re, subprocess, sys
root=Path(sys.argv[1]).resolve();out=Path(sys.argv[2]).resolve();init=(root/'kernel/src/init.rs').read_text();resources=(root/'kernel/src/app_resources.rs').read_text()
assert init.index('crate::app_resources::audit_boot_catalog(silkshell_id);')<init.index('crate::boot_memory::seal_all();')
audit=resources.split('pub fn audit_boot_catalog(',1)[1].split('fn grant(',1)[0]
for anchor in ('boot();','APPS.lock()','shell.find_capability','pd.find_capability','pd.main_task.load','task.context.pd_id','task.context.pd_ptr','app.identity','DORMANT[index]','crate::app_catalog_state::validate(row,&actual,coordinator)','[boot.app.catalog.complete]',
               'DOMAIN_REGISTRY.get(app.predecessor)','predecessor.main_task.load','predecessor_main as usize==predecessor_app.main','predecessor_task.context.pd_ptr,predecessor as *const _','assert_eq!(predecessor_app.task,predecessor_task.id',
               'predecessor:app.predecessor,predecessor_registered_task'):
 assert anchor in audit,'missing actual BOOT catalog extraction/check: '+anchor

def fixture(path, replacement=None):
 text=(root/path).read_text()
 # Compile the actual modules by absolute path; substitute only the selected
 # production module for a mutant, never a handwritten capture/validator model.
 def module(m):
  target=(root/'tests'/m[1]).resolve()
  if replacement and target==replacement[0]: target=replacement[1]
  return '#[path="'+str(target)+'"]'
 return re.sub(r'#\[path\s*=\s*"([^"]+)"\]',module,text)

def compiled_case(name,text,test,passes):
 source=out/(name+'.rs');binary=out/name;source.write_text(text)
 compile_result=subprocess.run(['rustc','--edition=2021','--test',str(source),'-o',str(binary)],capture_output=True,text=True)
 (out/(name+'-compile.log')).write_text(compile_result.stdout+compile_result.stderr)
 assert compile_result.returncode==0, name+': compile failure is not a rejected mutant'
 result=subprocess.run([str(binary),'--exact',test,'--test-threads=1','--nocapture'],capture_output=True,text=True)
 log=result.stdout+result.stderr;(out/(name+'.log')).write_text(log)
 assert 'running 1 test' in log,name+': missing selected runtime test'
 if passes:
  assert result.returncode==0,name+': actual production regression failed; see '+str(out/(name+'.log'))
 else:
  assert result.returncode!=0 and f'{test} ... FAILED' in log,name+': mutation survived or failed outside selected assertion'
 print('[app.catalog.control] PASS name='+name+' scope=compiled_actual_module_host_doubles')

source=(root/'kernel/src/app_catalog_state.rs').read_text()
unequal='unequal_registered_browser_tasks_validate_original_and_both_successors'
state_controls=[
 ('wrong-route','    if actual.close_target != row.target { return Err(Error::CloseTarget); }','', 'domain_routes_cannot_target_another_incarnation'),
 ('task-is-pd','if actual.task == 0','if actual.task != row.target',unequal),
 ('lifecycle-task-is-pd','if grant.task_id != actual.task','if grant.task_id != row.target',unequal),
 ('self-task-is-pd','if own.task_id != actual.task','if own.task_id != row.target',unequal),
 ('successor-task-is-pd','if activation.successor_task != actual.task','if activation.successor_task != row.target',unequal),
 ('predecessor-task-is-pd','if activation.predecessor_task != actual.predecessor_registered_task','if activation.predecessor_task != predecessor',unequal),
 ('missing-pointer-proof','    if !actual.task_domain_pointer_matches { return Err(Error::TaskDomainPointer); }','', 'unequal_registered_task_still_requires_exact_pointer_domain_and_grant_tasks'),
]
for name,old,new,test in state_controls:
 assert source.count(old)==1,name+': mutation anchor missing/ambiguous'
 changed=out/(name+'-module.rs');changed.write_text(source.replace(old,new,1))
 compiled_case(name,fixture(Path('tests/boot_app_catalog.rs'),(root/'kernel/src/app_catalog_state.rs',changed)),test,False)

capture_test='actual_boot_capture_joins_independent_registered_predecessor_tasks'
compiled_case('actual-capture',fixture(Path('tests/app_resources_runtime.rs')),capture_test,True)
# Inference from a Domain ID fails the unequal positive. Inference from the
# activation grant falsely accepts a forged predecessor task; the negative
# capture case must detect that independently of the grant under audit.
old='            predecessor_task.id\n'
assert resources.count(old)==1,'registered predecessor capture anchor missing/ambiguous'
for name,value in [('capture-infers-pd','app.predecessor'),('capture-trusts-grant','activation.expect("mutant activation").predecessor_task')]:
 changed=out/(name+'-module.rs');changed.write_text(resources.replace(old,'            '+value+'\n',1))
 compiled_case(name,fixture(Path('tests/app_resources_runtime.rs'),(root/'kernel/src/app_resources.rs',changed)),capture_test,False)
PY_CONTROLS
PYTHONDONTWRITEBYTECODE=1 python3 "$ROOT_DIR/scripts/check_browser_startup_identity.py"
PYTHONDONTWRITEBYTECODE=1 python3 "$ROOT_DIR/scripts/check_browser_sources.py" --root "$ROOT_DIR"
echo '[app.catalog.result] PASS scope=actual_validator_and_capture_with_host_registry_mapper_tlb_not_native_observation compiled_mutants_rejected=9 apps=19 registered_mains=31'
