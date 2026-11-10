#!/usr/bin/env bash
# Actual activation/rebirth module plus explicit finite host mapper/frames/registry.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-source-pin.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import hashlib,json,re,subprocess,sys
root,out=map(Path,sys.argv[1:]);source=root/'kernel/src/app_resources.rs';text=source.read_text()
fixture=root/'tests/app_resources_runtime.rs';case=root/'tests/app_source_pin_cases.rs'
method='app_resources::source_pin_cases::actual_first_activation_and_rebirth_reject_source_before_mutation'
# Bind the authority check to every executable replacement path, before mutation.
assert text.count('load_program(')==3
activate=text.split('pub fn activation(',1)[1].split('fn checked_program_image(',1)[0]
assert activate.index('checked_program_image(len,g.image_source)?')<activate.index('load_program(&mut apps[ni],&copy,task)?')
rebirth=text.split('fn rebirth(',1)[1].split('pub fn force_end(',1)[0]
assert rebirth.index('checked_program_image(len,source)')<rebirth.index('app.generation=')<rebirth.index('NEXT_RUN_TASK.fetch_add')
loader=text.split('fn load_program(',1)[1].split('pub fn grow(',1)[0]
assert 'program_image(' not in loader,'the loader must use exactly its verified private copy'
# Each prepared module is the full production module, with a child test appended
# so observers can inspect private state without adding production test hooks.
def run(name,production,expect_pass):
    tree=out/name;tree.mkdir()
    module=tree/'app_resources.rs'
    module.write_text(production+'\n#[cfg(test)] mod source_pin_cases { include!('+json.dumps(str(case))+'); }\n')
    runner=re.sub(r'#\[path="([^"]+)"\]',lambda m:'#[path='+json.dumps(str((fixture.parent/m.group(1)).resolve()))+']',fixture.read_text())
    runner=runner.replace(str(source),str(module))
    (tree/'runner.rs').write_text(runner)
    subprocess.run(['rustc','--edition=2021','--test','-A','warnings',str(tree/'runner.rs'),'-o',str(tree/'test')],check=True)
    result=subprocess.run([str(tree/'test'),'--exact',method,'--nocapture'],capture_output=True,text=True)
    (tree/'result.log').write_text(result.stdout+result.stderr)
    if (result.returncode==0)!=expect_pass:
        raise SystemExit(name+': unexpected result\n'+result.stdout+result.stderr)
    print('[program.source.pin] '+('PASS actual_activation_rebirth=1' if expect_pass else 'rejected='+name))
run('actual',text,True)
old='if source.is_some_and(|approved|!approved.matches(&copy)){return Err(DENIED)}'
assert text.count(old)==1
run('source-bypass',text.replace(old,'if false{return Err(DENIED)}'),False)
copy='        let copy=image.map(|len|checked_program_image(len,source)).transpose()?;\n'
generation='        app.generation=app.generation.checked_add(1).ok_or(DENIED)?;\n'
assert text.count(copy+generation)==1
run('generation-before-proof',text.replace(copy+generation,generation+copy),False)
(out/'inputs.json').write_text(json.dumps({str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [source,root/'kernel/src/app_resource_state.rs',fixture,case]},indent=2)+'\n')
PY
