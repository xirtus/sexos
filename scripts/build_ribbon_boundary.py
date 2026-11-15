#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Build original developer-only freestanding CLAP boundary modules, scan every ELF."""
import argparse,hashlib,json,os,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args();a.out.mkdir(parents=True,exist_ok=False)
 clang=os.environ.get('CLANG','clang');ld=os.environ.get('LD_LLD','ld.lld')
 resource=Path(subprocess.check_output([clang,'--print-resource-dir'],text=True).strip())/'include'
 source=ROOT/'fixtures/ribbon-boundary/plugin.c';headers=ROOT/'vendor/clap-fixture/include'
 inputs={str(f):sha(f)for f in[source,Path(__file__),*sorted(headers.rglob('*.h'))]}
 tools={v:subprocess.check_output([v,'--version'],text=True).splitlines()[0] for v in(clang,ld)}
 for mode,name in enumerate(('swap','crash','hang','parent-read','parent-write')):
  obj=a.out/(name+'.o');elf=a.out/(name+'.clap')
  subprocess.run([clang,'--target=x86_64-unknown-none-elf','-std=c11','-fPIC','-O2','-ffreestanding','-fno-stack-protector','-mno-red-zone','-msse2','-mno-avx','-nostdinc','-isystem',str(resource),'-I',str(headers),'-DRIBBON_MODE='+str(mode),'-c',str(source),'-o',str(obj)],check=True)
  subprocess.run([ld,'-shared','--hash-style=sysv','-z','max-page-size=4096','-o',str(elf),str(obj)],check=True)
  subprocess.run(['python3',str(ROOT/'scripts/key_bytes.py'),str(elf)],check=True)
 assert all(sha(Path(f))==v for f,v in inputs.items()),'source changed during build'
 (a.out/'receipt.json').write_text(json.dumps(dict(inputs=inputs,tools=tools,elfs={p.name:sha(p)for p in a.out.glob('*.clap')}),indent=2)+'\n')
if __name__=='__main__':main()
