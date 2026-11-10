#!/usr/bin/env bash
# Full production restart module and actual OP_RESTARTS dispatch/prefix; host boundaries declared.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-peer-state.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PY'
from pathlib import Path
import hashlib,json,os,re,subprocess,sys
root,out=map(Path,sys.argv[1:]);restart=root/'kernel/src/service_restart.rs';source=restart.read_text();handler=root/'kernel/src/crash_log.rs';dispatch=handler.read_text();case=root/'tests/service_peer_cases.rs';fixture=root/'tests/service_peer_runtime.rs'
start=dispatch.index('pub fn syscall(');match=dispatch.index('    match op {',start);prefix=dispatch[start:match]
assert 'current_pd()' in prefix and 'regs.rsi = 0;' in prefix and 'regs.rdx = 0;' in prefix
arm=dispatch.index('        wire::OP_RESTARTS => {',match);brace=dispatch.index('{',arm);depth=1;end=brace+1
while depth:
    depth+=(dispatch[end]=='{')-(dispatch[end]=='}');end+=1
body=dispatch[brace:end]
(out/'envelope.rs').write_text('use crate::wire;\n'+prefix+'match op { wire::OP_RESTARTS => '+body+', _=>wire::INVALID }\n}\n')
def run(name,text,expected):
    tree=out/name;tree.mkdir();module=tree/'service_restart.rs'
    module.write_text(text+'\n#[cfg(test)] mod peer_cases { include!('+json.dumps(str(case))+'); }\n')
    runner=re.sub(r'#\[path="([^"]+)"\]',lambda m:'#[path='+json.dumps(str((fixture.parent/m.group(1)).resolve()))+']',fixture.read_text()).replace(str(restart),str(module))
    (tree/'runner.rs').write_text(runner)
    env=dict(os.environ,SEXOS_PEER_ENVELOPE_SOURCE=str(out/'envelope.rs'))
    compiled=subprocess.run(['rustc','--edition=2021','--test',str(tree/'runner.rs'),'-o',str(tree/'test')],env=env,capture_output=True,text=True)
    (tree/'compile.log').write_text(compiled.stdout+compiled.stderr)
    if compiled.returncode:raise SystemExit(name+': compile failure is not a rejected mutant\n'+compiled.stdout+compiled.stderr)
    result=subprocess.run([str(tree/'test'),'--test-threads=1','--nocapture'],capture_output=True,text=True)
    (tree/'result.log').write_text(result.stdout+result.stderr)
    if (result.returncode==0)!=expected:raise SystemExit(name+': unexpected result\n'+result.stdout+result.stderr)
    print('[service.peer.host] '+('PASS full_restart_and_syscall_envelope=1' if expected else 'rejected='+name))
run('actual',source,True)
controls=[
 ('raw-pd-fallback','Some(crate::capability::CapabilityData::Domain(pd))=>pd,_=>return Err(w::DENIED)','Some(crate::capability::CapabilityData::Domain(pd))=>pd,_=>slot'),
 ('slot-truncation','decode_capability_slot(slot)','decode_capability_slot(slot as u32 as u64)'),
 ('contention-is-live','Err(if contended{w::RETRY}else{w::NOT_FOUND})','Ok(PeerState{epoch:1,life:L::Live})'),
 ('end-query-consumes','let ending=target<64 && END_REQUESTS.load(Ordering::Acquire)&(1u64<<target)!=0;','let ending=end_requested(target);'),
 ('legacy-self-lock-zero','SERVICE_IDS.iter().position(|id|pd!=0&&id.load(Ordering::Acquire)==pd)','(0..SERVICES).find(|&i|SERVICE[i].try_lock().is_some_and(|s|s.pd==pd&&pd!=0))'),
 ('zero-first-epoch','(RESTARTS[index].load(Ordering::Acquire) as u64).checked_add(1)','(RESTARTS[index].load(Ordering::Acquire) as u64).checked_add(0)'),
 ('capture-id-alias','SERVICE_IDS[index].store(pd_id,Ordering::Release)','SERVICE_IDS[0].store(pd_id,Ordering::Release)'),
]
for name,old,new in controls:
    assert source.count(old)==1,name+': missing/ambiguous anchor';run(name,source.replace(old,new,1),False)
(out/'inputs.json').write_text(json.dumps({str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [restart,handler,root/'crates/sex-pdx/src/crash_log.rs',fixture,case]},indent=2)+'\n')
PY
