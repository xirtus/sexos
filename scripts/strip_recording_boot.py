#!/usr/bin/env python3
"""Opt-in staged module compaction; kernel and original target ELFs stay intact."""
import argparse,hashlib,json,os,shutil,struct,subprocess,sys
from pathlib import Path
SECTION_FIELDS=((40,48),(58,64))
def digest(data):return hashlib.sha256(data).hexdigest()
def masked(data,offset=0):
    data=bytearray(data)
    for lo,hi in SECTION_FIELDS:
        for at in range(max(lo,offset),min(hi,offset+len(data))):data[at-offset]=0
    return bytes(data)
def validate(before,after):
    for blob in (before,after):
        assert len(blob)>=64 and blob[:7]==b'\x7fELF\x02\x01\x01','requires ELF64 little endian'
        assert struct.unpack_from('<H',blob,18)[0]==62,'requires x86-64'
    assert masked(before[:64])==masked(after[:64]),'runtime ELF header changed'
    offset=struct.unpack_from('<Q',before,32)[0];size,count=struct.unpack_from('<HH',before,54)
    assert size==56 and 0<count<=128 and offset+size*count<=min(len(before),len(after)),'invalid program table'
    assert before[offset:offset+size*count]==after[offset:offset+size*count],'program header changed'
    loaded=[]
    for index in range(count):
        row=struct.unpack_from('<IIQQQQQQ',before,offset+index*size)
        kind,flags,start,va,pa,length,memory,align=row
        if not length:continue
        assert start+length<=min(len(before),len(after)),'truncated file-backed program segment'
        original=masked(before[start:start+length],start)
        assert original==masked(after[start:start+length],start),'file-backed program segment changed'
        if kind!=1:continue
        loaded.append(dict(va=va,file_bytes=length,memory_bytes=memory,flags=flags,masked_sha256=digest(original)))
    assert loaded,'no load segments'
    return loaded
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--root',type=Path,required=True);parser.add_argument('--receipt',type=Path,required=True);args=parser.parse_args()
    assert os.environ.get('SEXOS_RECORDER')=='1','only explicit recorder profile may compact staging'
    record=args.root/'programs/record';assert record.is_file()
    paths=sorted((args.root/'servers').iterdir())+sorted((args.root/'apps').iterdir())+[record]
    receipt=dict(status='incomplete',scope='staged server/app/record ELFs only; kernel, original targets and all firmware preserved',modules=[])
    def save():args.receipt.write_text(json.dumps(receipt,indent=2)+'\n')
    save()
    try:
        for path in paths:
            if not path.is_file():continue
            before=path.read_bytes()
            if before[:4]!=b'\x7fELF':continue
            temporary=path.with_name(path.name+'.stripped.tmp');shutil.copy2(path,temporary)
            try:
                subprocess.run(['llvm-strip','--strip-all',str(temporary)],check=True)
                after=temporary.read_bytes();loads=validate(before,after)
                if path==record:
                    subprocess.run([sys.executable,str(Path(__file__).with_name('key_bytes.py')),str(temporary)],check=True)
                temporary.replace(path)
                receipt['modules'].append(dict(path=str(path.relative_to(args.root)),before_sha256=digest(before),after_sha256=digest(after),before_bytes=len(before),after_bytes=len(after),load_segments=loads))
                save()
            finally:
                temporary.unlink(missing_ok=True)
        receipt['status']='verified';save()
    except BaseException as error:
        receipt['error']=repr(error);save();raise
if __name__=='__main__':main()
