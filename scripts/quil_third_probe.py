"""Actual third Quil activation, inherited workspace recovery, input and cleanup."""
import re
import time
from check_quil_ui_pixels import verify

def run(c,refresh,close):
    key,wait,text,q,focus=(c[k]for k in ('key','wait','text','q','focus'))
    start=len(text())
    key('grave_accent',r'\[shell.palette.item\] idx=0')
    key('j',r'\[shell.palette.select\] old=0 new=1')
    key('ret',r'\[shell.app.relaunch\] predecessor=16 successor=18 ')
    wait(r'\[app.activation.committed\] logical=9 incarnation=3 predecessor=16 successor=18 .*runtime_mint=0',start)
    wait(r'\[app.surface.presented\] target=18 surface=307 .*ok=1',start)
    refresh()
    loaded=[]
    for expected in [c['first'],c['second']]:
        focus(200)
        for _ in range(20):
            key('j',r'\[shell.action.select_next_linen\] id=\d+')
            offset=len(text());q.key('ret');end=time.monotonic()+1
            while time.monotonic()<end and '[linen.quil.disk_doc.intent]'not in text()[offset:]:time.sleep(.05)
            m=re.search(r'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id=(\d+) ok=1',text()[offset:])
            if m and int(m[1])==expected['object']:
                m=wait(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(expected['object']),offset)
                if (int(m[1]),m[2])!=(expected['bytes'],expected['hash']):raise RuntimeError('third incarnation document changed')
                loaded.append(expected['object']);break
            focus(200)
        else:raise RuntimeError('third incarnation selection unavailable')
    focus(307)
    key('t',r'\[quil.document.edit\]')
    # Validate actual glyphs from this exact new subject/surface, not an old view.
    errors=['not painted'];end=time.monotonic()+90
    while time.monotonic()<end:
        q.cmd('screendump',{'filename':str(c['out']/'third-quil-edit.ppm')})
        errors=verify(text(),(c['out']/'third-quil-edit.ppm').read_bytes(),c['font'],'t#',target=18,resolution=c['resolution'])
        if not errors:break
        time.sleep(.15)
    if errors:raise RuntimeError('third Quil pixels: '+str(errors))
    # Loaded notes start at the end; persist a useful edit through PD18's
    # own session and route. Existing expected bytes remain the disk oracle.
    second=c['second'];second['content']+='t'
    offset=len(text());c['ctrl']('s',r'\[quil.document.saved\].*dirty=0')
    m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(second['object']),offset)
    second.update(bytes=int(m[1]),hash=m[2])
    close(18,307)
    # Final visible status must report finite exhaustion rather than offer the
    # dead subject again. No command or diagnostic manufactures a fresh grant.
    key('grave_accent',r'\[shell.palette.item\] idx=0')
    wait(r'\[shell.palette.item\] idx=1 .*status=CLOSED / RESERVE EXHAUSTED')
    key('esc',r'\[command_palette.close\]')
    return dict(target=18,surface=307,logical=9,incarnation=3,documents=loaded,
                edited_and_saved=True,pixel_errors=errors,terminal=True,reserve_exhausted=True)
