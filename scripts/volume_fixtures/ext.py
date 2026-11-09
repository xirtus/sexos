import os,struct,subprocess,shutil,sys
def pattern(seed,n):
    x=(seed*0x9E3779B9 & 0xffffffff)|1; out=bytearray()
    while len(out)<n:
        x^=(x<<13)&0xffffffff; x^=x>>17; x^=(x<<5)&0xffffffff
        out+=struct.pack('<I',x)
    return bytes(out[:n])
def put(path,chunks,size):
    os.makedirs(os.path.dirname(path),exist_ok=True)
    with open(path,'wb') as f:
        for at,data in chunks: f.seek(at); f.write(data)
        f.truncate(size)
root=sys.argv[1]; repo='/home/user/sexos'
shutil.rmtree(root,ignore_errors=True)
# ext4
s=f'{root}/a'
put(f'{s}/hello.txt',[(0,b'Hello from Linux\n')],17); os.utime(f'{s}/hello.txt',(1700000000,1700000000))
put(f'{s}/empty',[],0)
put(f'{s}/docs/notes/deep.txt',[(0,pattern(1,5000))],5000)
put(f'{s}/héllo-世界.txt',[(0,b'unicode\n')],8)
os.symlink('docs/notes/deep.txt',f'{s}/link')
put(f'{s}/holey.bin',[(0,pattern(2,102400)),(204800,pattern(3,102400))],307200)
put(f'{s}/frag.bin',[(i*8192,pattern(10+i,4096)) for i in range(10)],81920)
put(f'{s}/tail.bin',[(0,pattern(4,70000))],70000)
for i in range(150): put(f'{s}/many/a-rather-long-file-name-number-{i:03}',[(0,b'file %03d\n'%i)],9)
# ext2, 1K blocks, no filetype
s=f'{root}/b'
put(f'{s}/big.bin',[(0,pattern(20,3000)),(200*1024,pattern(21,2000)),(1<<20,pattern(22,1500)),((66<<20)+123,pattern(23,2500))],70<<20)
put(f'{s}/small.txt',[(0,b'ext2 says hi\n')],13)
put(f'{s}/sub/dir/x.txt',[(0,pattern(24,1500))],1500)
# ext4: inline data, 64bit, meta_bg, 1K blocks, deep extent tree
s=f'{root}/c'
put(f'{s}/tiny.txt',[(0,b'tiny inline file\n')],17)
put(f'{s}/small.txt',[(0,pattern(30,100))],100)
put(f'{s}/smalldir/a',[(0,b'a\n')],2); put(f'{s}/smalldir/b',[(0,b'bb\n')],3)
put(f'{s}/deep.bin',[(i*2048,struct.pack('<II',0xD00D,i)) for i in range(400)],399*2048+8)
def mk(cmd,src,img,size):
    if os.path.exists(img): os.remove(img)
    subprocess.check_call(cmd+['-d',src,img,size])
    subprocess.call(['e2fsck','-fyD',img],stdout=subprocess.DEVNULL)  # index big directories into htrees
    subprocess.check_call(['python3',f'{repo}/scripts/make_volume_fixture.py',img,f'{repo}/tests/volumes/'+os.path.basename(img).replace('.img','.sparse')])
mk(['mkfs.ext4','-q','-F','-b','4096','-O','dir_index','-L','LINUXDATA','-E','lazy_itable_init=0,lazy_journal_init=0'],f'{root}/a',f'{root}/ext4.img','16M')
mk(['mkfs.ext2','-q','-F','-b','1024','-O','^filetype','-L','OLDDISK'],f'{root}/b',f'{root}/ext2.img','4M')
mk(['mkfs.ext4','-q','-F','-b','1024','-I','256','-O','inline_data,64bit,meta_bg,^resize_inode','-L','INLINE','-E','lazy_itable_init=0,lazy_journal_init=0'],f'{root}/c',f'{root}/ext4inline.img','8M')
