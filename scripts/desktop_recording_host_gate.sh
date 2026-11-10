#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-recording.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
for test in desktop_recording_wire recording_role_custody recording_image_custody recording_image_restart; do
    rustc --test --edition=2021 "$ROOT_DIR/tests/$test.rs" -o "$TEST_DIR/$test"
    "$TEST_DIR/$test"
done

SEXOS_RECORDER=1 rustc --test --edition=2021 "$ROOT_DIR/tests/recording_spindle.rs" -o "$TEST_DIR/spindle"
"$TEST_DIR/spindle"

python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY_PATHS'
from pathlib import Path
import sys
root,out=map(Path,sys.argv[1:]);text=(root/'servers/sexfiles/src/fs_service.rs').read_text();wire=(root/'crates/sex-pdx/src/fs.rs').read_text()
parts=['mod w { '+'\n'.join(next(line for line in wire.splitlines()if line.startswith('pub const '+name+':'))for name in ['PATH_BYTES','ARGS_MAX'])+' }']
for prefix in ['const ROOT_MAX:', 'const PICKED_MAX:', 'struct Binding {']:
 parts.append(next(line for line in text.splitlines()if line.startswith(prefix)))
for name in ['recording_binding','confine']:
 a=text.index('fn '+name+'(');i=text.index('{',a)+1;depth=1
 while depth:depth+=(text[i]=='{')-(text[i]=='}');i+=1
 part=text[a:i]
 if name=='recording_binding': assert 'LOADED' not in part and 'PREPARED' not in part
 parts.append(part)
(out/'files_paths.rs').write_text('\n'.join(parts))
PY_PATHS
RECORDING_FILES_PATH_SOURCE="$TEST_DIR/files_paths.rs" rustc --test --edition=2021 "$ROOT_DIR/tests/recording_files_paths.rs" -o "$TEST_DIR/paths"
"$TEST_DIR/paths"

python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY_IMAGE'
from pathlib import Path
import sys
root,out=map(Path,sys.argv[1:]);text=(root/'servers/sexfiles/src/fs_service.rs').read_text();parts=[]
for prefix in ['static mut LOADED:', 'static mut LOADED_ARGS:', 'static mut PREPARED:', 'static mut PREPARED_ARGS:', 'pub enum Program ']:
 parts.append(next(line for line in text.splitlines()if line.startswith(prefix)))
for name in ['loaded','read_program','read_named','read_named_unbound','read_ticket']:
 a=text.index('fn '+name+'(');a=text.rfind('\n',0,a)+1;i=text.index('{',a)+1;depth=1
 while depth:depth+=(text[i]=='{')-(text[i]=='}');i+=1
 parts.append(text[a:i])
(out/'image_loader.rs').write_text('\n'.join(parts))
PY_IMAGE
RECORDING_IMAGE_LOADER_SOURCE="$TEST_DIR/image_loader.rs" rustc --test --edition=2021 "$ROOT_DIR/tests/recording_image_loader.rs" -o "$TEST_DIR/image-loader"
"$TEST_DIR/image-loader" --test-threads=1

python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY_RUNTIME'
from pathlib import Path
import json,re,sys
root,out=map(Path,sys.argv[1:]);production=root/'servers/sexfiles/src/recording_runtime.rs'
source=production.read_text()+'''
#[cfg(test)]pub fn cold_reset_for_test(){assert!(!ENTERED.load(Ordering::Acquire));for index in 0..512{JOURNAL.fault_word(index,0);}}
#[cfg(test)]pub fn journal_for_test()->Result<crate::recording_journal::Fence,crate::recording_journal::Error>{JOURNAL.load()}
'''
module=out/'runtime.rs';module.write_text(source)
fixture=root/'tests/recording_runtime.rs';runner=re.sub(r'#\[path\s*=\s*"([^"]+)"\]',lambda m:'#[path='+json.dumps(str((fixture.parent/m.group(1)).resolve()))+']',fixture.read_text()).replace(str(production),str(module))
(out/'runtime_runner.rs').write_text(runner)
text=(root/'servers/sexfiles/src/backends/diskfs.rs').read_text();a=text.index('pub fn diskfs_block_call(');i=text.index('{',a)+1;depth=1
while depth:depth+=(text[i]=='{')-(text[i]=='}');i+=1
nested=text[a:i]
for forbidden in ['recording_runtime::','fs_service::replies_pending','fs_service::poll_replies','trampoline_main']:
 assert forbidden not in nested,forbidden
assert 'defer_stash(msg.type_id, msg.caller_pd, msg.caller_task, msg.arg0, msg.arg1, msg.arg2)' in nested
(out/'nested_disk.rs').write_text('impl DiskFs {\n'+nested+'\n}')
text=(root/'servers/sexfiles/src/trampoline.rs').read_text();a=text.index('if msg.type_id==sex_pdx::program_store::OP{');i=text.index('{',a)+1;depth=1
while depth:depth+=(text[i]=='{')-(text[i]=='}');i+=1
ordinary=text[a:i];assert ordinary.count('continue;')==1
assert ordinary.index('recording_runtime::image_busy()')<ordinary.index('program_store::load(')
(out/'ordinary.rs').write_text('{\n'+ordinary.replace('continue;','return;')+'\n}')
PY_RUNTIME
SEXOS_RECORDER=1 RECORDING_RUNTIME_CASES="$ROOT_DIR/tests/recording_runtime_cases.rs" RECORDING_NESTED_DISK_SOURCE="$TEST_DIR/nested_disk.rs" RECORDING_ORDINARY_DISPATCH_SOURCE="$TEST_DIR/ordinary.rs" rustc --test --edition=2021 "$TEST_DIR/runtime_runner.rs" -o "$TEST_DIR/runtime"
"$TEST_DIR/runtime" --test-threads=1
