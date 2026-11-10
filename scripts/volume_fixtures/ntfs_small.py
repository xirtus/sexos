import os,sys
m=sys.argv[1]
def gen(seed,n):
    return bytes(((((i*2654435761)&0xffffffff)^seed)>>13)&0xff for i in range(n))
open(os.path.join(m,'hello.txt'),'wb').write(b'small clusters\n')
os.mkdir(os.path.join(m,'Dir'))
for i in range(40): open(os.path.join(m,'Dir','name %02d.txt'%i),'wb').write(b'%d\n'%i)
open(os.path.join(m,'Dir','data.bin'),'wb').write(gen(5,5000))
