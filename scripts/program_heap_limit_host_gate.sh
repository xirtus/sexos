#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-heap-limit.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import json,sys
root,out=map(Path,sys.argv[1:])
(out/'Cargo.lock').write_bytes((root/'Cargo.lock').read_bytes())
(out/'Cargo.toml').write_text('[package]\nname="sexos-heap-limit-tests"\nversion="0.0.0"\nedition="2021"\n[features]\nheap=[]\n[dependencies]\ntalc={version="=5.1.1",default-features=false}\n[[test]]\nname="heap_limit"\npath='+json.dumps(str(root/'tests/program_heap_limit.rs'))+'\n')
PY
cargo test --offline --manifest-path "$TEST_DIR/Cargo.toml" --target-dir "$TEST_DIR/target" -- --nocapture

python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY_CONTROLS'
from pathlib import Path
import json,os,subprocess,sys
root,out=map(Path,sys.argv[1:]);module=root/'crates/sex-app/src/heap.rs';source=module.read_text();fixture=root/'tests/program_heap_limit.rs'
assert 'HEAP.lock().source.set_limit(bytes)' in source
controls=[('forget-started-attempt','if self.started {','if false {'),('forget-total-charge','self.pages.checked_add(pages)? > self.max_pages','pages > self.max_pages'),('recover-after-invalid','if self.failed { return None; }','if false { return None; }'),('forget-query-anchor','before.base != self.base','false')]
for name,old,new in controls:
 assert source.count(old)==1,name+': mutation anchor missing';tree=out/name;tree.mkdir();changed=tree/'heap.rs';changed.write_text(source.replace(old,new,1));runner=tree/'runner.rs';runner.write_text(fixture.read_text().replace('../crates/sex-app/src/heap.rs',str(changed)))
 (tree/'Cargo.lock').write_bytes((root/'Cargo.lock').read_bytes());(tree/'Cargo.toml').write_text('[package]\nname="sexos-heap-limit-control"\nversion="0.0.0"\nedition="2021"\n[features]\nheap=[]\n[dependencies]\ntalc={version="=5.1.1",default-features=false}\n[[test]]\nname="heap_limit"\npath='+json.dumps(str(runner))+'\n')
 command=['cargo','test','--offline','--manifest-path',str(tree/'Cargo.toml'),'--target-dir',str(out/'target')]
 compiled=subprocess.run(command+['--no-run'],capture_output=True,text=True);(tree/'compile.log').write_text(compiled.stdout+compiled.stderr)
 if compiled.returncode:raise SystemExit(name+': compilation failure is not a rejected control\n'+compiled.stdout+compiled.stderr)
 result=subprocess.run(command+['--','--test-threads=1'],capture_output=True,text=True);(tree/'result.log').write_text(result.stdout+result.stderr)
 if result.returncode==0:raise SystemExit(name+': expected runtime test failure')
 print('[program.heap.limit] rejected='+name)
PY_CONTROLS
