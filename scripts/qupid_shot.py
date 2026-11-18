"""Screenshots for the Qupid gates: dump until the accent shows or the screen settles."""
import collections,time
from PIL import Image

ACCENT=(0xDB,0x4A,0x7E)

def accent_px(img):
    px=img.load();r0,g0,b0=ACCENT
    return sum(1 for y in range(img.height) for x in range(img.width)
               if abs(px[x,y][0]-r0)<8 and abs(px[x,y][1]-g0)<8 and abs(px[x,y][2]-b0)<8)

def accent_shot(q,out,name,want,tries=12):
    """Dumps the screen up to `tries` times, 0.5 s apart, until at least `want`
    accent pixels show or two dumps match. Saves name.png; returns (count, notes)."""
    path=out/f'{name}.ppm';last=None;count=0
    for _ in range(tries):
        q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
        img=Image.open(path).convert('RGB');count=accent_px(img)
        if count>=want:break
        raw=img.tobytes()
        if raw==last:break
        last=raw
    img.save(out/f'{name}.png')
    top=collections.Counter(img.getdata()).most_common(6)
    return count,f'size={img.size} top_colors={[("#%02x%02x%02x"%c,n) for c,n in top]}'
