#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-task-ids.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import json,re,subprocess,sys
root,out=map(Path,sys.argv[1:]);source=root/'kernel/src/app_resources.rs';text=source.read_text();fixture=root/'tests/app_resources_runtime.rs'
cases=['actual_counter_exhaustion_is_permanent_and_concurrent_ids_are_unique','actual_rebirth_exhaustion_preserves_original_task_and_resource_state','actual_worker_exhaustion_preserves_run_and_counter']
assert text.count('next_run_task()?')==2
rebirth=text.split('fn rebirth(',1)[1].split('pub fn force_end(',1)[0]
assert rebirth.index('next_run_task()?')<rebirth.index('app.generation=')<rebirth.index('load_program(app,')

def run(name,production,expect):
 tree=out/name;tree.mkdir();module=tree/'app_resources.rs'
 includes='\n'.join('include!('+json.dumps(str(root/'tests'/case))+');' for case in ['app_source_pin_cases.rs','program_task_id_cases.rs'])
 module.write_text(production+'\n#[cfg(test)] mod task_id_cases { '+includes+' }\n')
 runner=re.sub(r'#\[path="([^"]+)"\]',lambda m:'#[path='+json.dumps(str((fixture.parent/m.group(1)).resolve()))+']',fixture.read_text()).replace(str(source),str(module))
 path=tree/'runner.rs';path.write_text(runner)
 compiled=subprocess.run(['rustc','--edition=2021','--test','-A','warnings',str(path),'-o',str(tree/'test')],capture_output=True,text=True)
 if compiled.returncode:raise SystemExit(name+': control must compile\n'+compiled.stderr)
 results=[];failures=[]
 for case in cases:
  result=subprocess.run([str(tree/'test'),'--exact','app_resources::task_id_cases::'+case,'--nocapture'],capture_output=True,text=True)
  (tree/(case+'.log')).write_text(result.stdout+result.stderr);results.append(result.returncode==0)
  if result.returncode: failures.append(case+'\n'+result.stdout+result.stderr)
  elif expect: print('[program.task.ids] case='+case+' passed=1')
 if expect and not all(results):raise SystemExit(name+': failing cases '+str(results)+'\n'+'\n'.join(failures))
 if not expect and all(results):raise SystemExit(name+': invalid control passed')
 print('[program.task.ids] '+('PASS actual_counter_rebirth_worker=1' if expect else 'rejected='+name))
run('actual',text,True)
old='NEXT_RUN_TASK.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|id|id.checked_add(1).filter(|next|id>=0x4000_0000&&*next<=RUN_TASK_END)).map_err(|_|DENIED)';assert text.count(old)==1
run('wrapping-counter',text.replace(old,'{ let id=NEXT_RUN_TASK.fetch_add(1,Ordering::Relaxed); if id==u32::MAX { Err(DENIED) } else { Ok(id) } }'),False)
old_range='filter(|next|id>=0x4000_0000&&*next<=RUN_TASK_END)';assert text.count(old_range)==1
run('namespace-bypass',text.replace(old_range,'filter(|_|true)'),False)
old='        let task=next_run_task()?;\n        app.generation=generation;';assert text.count(old)==1
run('half-rearm-generation',text.replace(old,'        app.generation=generation;\n        let task=next_run_task()?;'),False)
PY
