#!/usr/bin/env bash
# Actual screenshot custody/client/PNG code, scripted IPC, independent PNG decode.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-capture.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
python3 - "$ROOT_DIR" <<'PYBINDING'
from pathlib import Path
import sys
root=Path(sys.argv[1]);s=(root/'servers/sexdisplay/src/main.rs').read_text();body=s[s.index('fn render_prepared('):s.index('fn render(fb:')]
assert body.index('if !preparation.commit(')<body.index('if let Some(front)=store.front_pixels()')<body.index('SCREEN_CAPTURE.published(')
assert body.count('SCREEN_CAPTURE.published(')==1
boot=(root/'kernel/src/boot_memory.rs').read_text();assert 'paired_at(display,c::CLIENT,c::SLOT,c::SLOT,c::REGION_ID,c::BYTES as u64,false)' in boot
print('[capture.source.binding] PASS committed_front_and_readonly_boot_peer=1')
PYBINDING
rustc --edition=2021 --test -O -C overflow-checks=on "$ROOT_DIR/scripts/desktop_capture_host.rs" -o "$TEST_DIR/tests"
SEXOS_CAPTURE_HOST_PNG="$TEST_DIR/screen.png" "$TEST_DIR/tests" --test-threads=1
python3 - "$TEST_DIR/screen.png" <<'PY'
import binascii,struct,sys,zlib
b=open(sys.argv[1],'rb').read();assert b[:8]==b'\x89PNG\r\n\x1a\n';at=8;parts=[];types=[]
while at<len(b):
 n=struct.unpack('>I',b[at:at+4])[0];kind=b[at+4:at+8];body=b[at+8:at+8+n]
 assert len(body)==n and binascii.crc32(kind+body)==struct.unpack('>I',b[at+8+n:at+12+n])[0]
 types.append(kind)
 if kind==b'IHDR':assert struct.unpack('>IIBBBBB',body)==(129,173,8,2,0,0,0)
 if kind==b'IDAT':parts.append(body)
 at+=12+n
assert at==len(b) and types==[b'IHDR',b'IDAT',b'IEND']
raw=zlib.decompress(b''.join(parts));assert len(raw)==173*(129*3+1)
for y in range(173):
 row=raw[y*388:(y+1)*388];assert row[0]==0
 for x in range(129):assert row[1+x*3:4+x*3]==bytes(((x*17)&255,(y*29)&255,(x^y)&255))
print('[capture.png.decode] PASS pixels=22317 independent_crc_zlib_rgb=1')
PY
# A negative control must compile and then fail the matching production-code test.
python3 - "$ROOT_DIR" "$TEST_DIR" <<'PYMUTANT'
import pathlib,re,shutil,subprocess,sys
root,out=map(pathlib.Path,sys.argv[1:]);runner=pathlib.Path('scripts/desktop_capture_host.rs')
paths=[runner]+[(runner.parent/pathlib.Path(p)).resolve().relative_to(root.resolve()) for p in re.findall(r'#\[path\s*=\s*"([^"]+)"\]',(root/runner).read_text())]
cases=[
 ('wrong-owner','servers/sexdisplay/src/screen_capture.rs','msg.caller_pd != c::CLIENT','false','authority_bounds_and_custody'),
 ('stale-service','servers/sexdisplay/src/screen_capture.rs','msg.arg2 != self.epoch','false','same_task_display_restart_rejects_old_epoch_requests'),
 ('retained-pixels','crates/sex-pdx/src/screen_capture.rs','.store(0, core::sync::atomic::Ordering::SeqCst)','.fetch_or(0, core::sync::atomic::Ordering::SeqCst);','authority_bounds_and_custody'),
 ('file-buffer-reuse','apps/spindle/src/capture_command.rs','if self.pending && self.to_files {','if false {','cancellation_scrubs_promptly_but_fences_pending_file_buffer'),
]
for name,file,old,new,test in cases:
 tree=out/name
 for p in paths:
  target=tree/p;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(root/p,target)
 target=tree/file;text=target.read_text();assert text.count(old)==1,(name,'mutation binding');target.write_text(text.replace(old,new,1))
 binary=tree/'test';subprocess.run(['rustc','--edition=2021','--test','-O','-A','warnings',str(tree/runner),'-o',str(binary)],check=True)
 result=subprocess.run([str(binary),test,'--exact','--test-threads=1'],capture_output=True,text=True)
 assert result.returncode==101 and 'FAILED' in result.stdout,(name,'survived or did not execute',result.stdout,result.stderr)
 print('[capture.negative] rejected='+name)
PYMUTANT
echo '[desktop.capture.host] PASS scripted_transport_not_guest_file_proof=1'
